use std::{
    future,
    io::ErrorKind,
    net::Ipv4Addr,
    path::Path,
    process,
    sync::{Arc, Mutex},
    time::Duration,
};

use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayUrl, TransportAddr,
    endpoint::{
        BeforeConnectOutcome, ConnectingError, ConnectionError, EndpointHooks, MemberConnection,
        MemberDialOptions, MemberFusionEndpoint, presets,
    },
};
use miopunch_native_probe::{
    connection_coordinator::connection_diagnostic::monotonic_ms,
    daily_management::ManagementAction,
    native_network::{NativeNetwork, member_fusion_attempt_id},
    resources::{ResourceConfig, ResourceTable},
};
use p2panda_core::VerifyingKey;
use serde_json::{Value, json};
use tokio::{
    process::Command,
    sync::Notify,
    time::{self, Instant},
};
use tokio_util::sync::CancellationToken;

use super::{admit, bind, relay, resource};

#[path = "delayed_relay.rs"]
mod delayed_relay;

const SETUP: Duration = Duration::from_secs(5);
const REQUEST: &[u8] = b"fusion ping 143";
const REPLY: &[u8] = b"fusion pong 143";
const IDLE: Duration = Duration::from_millis(600);
const REVERSE_BUDGET: Duration = Duration::from_secs(2);

#[derive(Debug)]
struct HoldOutgoing(Arc<Notify>);

impl EndpointHooks for HoldOutgoing {
    async fn before_connect(&self, _target: &EndpointAddr, _alpn: &[u8]) -> BeforeConnectOutcome {
        self.0.notify_one();
        future::pending().await
    }
}

async fn admitted_pair(dir: &Path, url: &RelayUrl) -> (NativeNetwork, EndpointId) {
    let mut admin = NativeNetwork::create(dir.join("admin")).await.unwrap();
    let mut joining = NativeNetwork::join(dir.join("joining"), admin.manual_invitation().unwrap())
        .await
        .unwrap();
    let mut config = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![resource(url)],
    });
    config.udp_direct_disabled = true;
    config.tcp_direct_disabled = true;
    for node in [&admin, &joining] {
        node.save_resource_configuration(&config).unwrap();
    }
    let admin_bootstrap = bind(&admin).await;
    let join_bootstrap = bind(&joining).await;
    admit(
        &mut admin,
        &mut joining,
        &admin_bootstrap,
        &join_bootstrap,
        url,
    )
    .await;
    let member_id = EndpointId::from_bytes(joining.status().node_id.as_bytes()).unwrap();
    tokio::join!(admin_bootstrap.close(), join_bootstrap.close());
    joining.close().await;
    (admin, member_id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn route_stagger_uses_remaining_time_and_releases_losing_relay_slots() {
    #[derive(Debug)]
    enum Event {
        Started(Instant, Instant),
        Cancelled,
        Finished(Result<usize, ErrorKind>),
    }

    struct RouteReport {
        timeline: Arc<Mutex<Vec<(RelayUrl, Event)>>>,
        url: RelayUrl,
        finished: bool,
    }

    impl Drop for RouteReport {
        fn drop(&mut self) {
            if !self.finished {
                self.timeline
                    .lock()
                    .unwrap()
                    .push((self.url.clone(), Event::Cancelled));
            }
        }
    }

    let relay = relay().await;
    let healthy = relay.http_url().unwrap();
    let backend = (Ipv4Addr::LOCALHOST, healthy.port().unwrap()).into();
    let (silent_first, stop_first) = delayed_relay::delay_relay(backend, SETUP * 3).await;
    let (silent_second, stop_second) = delayed_relay::delay_relay(backend, SETUP * 3).await;
    let (silent_tail, stop_tail) = delayed_relay::delay_relay(backend, SETUP * 3).await;
    let dir = tempfile::tempdir().unwrap();
    let (admin, member_id) = admitted_pair(dir.path(), &healthy).await;
    let member = NativeNetwork::open(dir.path().join("joining"))
        .await
        .unwrap();
    let mut config = member.resource_configuration().unwrap();
    config.replacement = Some(
        [&healthy, &silent_first, &silent_second, &silent_tail]
            .into_iter()
            .map(resource)
            .collect(),
    );
    member.save_resource_configuration(&config).unwrap();
    let server = admin.bind_member_fusion_endpoint(SETUP).await.unwrap();
    time::timeout(SETUP, server.online()).await.unwrap();

    for (budget, candidates) in [
        (Duration::from_millis(900), vec![healthy.clone()]),
        (
            Duration::from_secs(4),
            vec![silent_second.clone(), healthy.clone(), silent_tail.clone()],
        ),
    ] {
        let client = member.bind_member_fusion_endpoint(SETUP).await.unwrap();
        let timeline = Arc::new(Mutex::new(Vec::new()));
        let attempt_timeline = timeline.clone();
        let options = MemberDialOptions::new(Vec::new(), Vec::new(), candidates.clone())
            .with_route_diagnostics(move |route, deadline| {
                let TransportAddr::Relay(url) = route else {
                    panic!("expected a relay route");
                };
                let mut timeline = attempt_timeline.lock().unwrap();
                timeline.push((url.clone(), Event::Started(Instant::now(), deadline)));
                let ids = (1, timeline.len());
                let mut diagnostic = RouteReport {
                    timeline: attempt_timeline.clone(),
                    url: url.clone(),
                    finished: false,
                };
                (ids, move |outcome| {
                    diagnostic.finished = true;
                    diagnostic
                        .timeline
                        .lock()
                        .unwrap()
                        .push((diagnostic.url.clone(), Event::Finished(outcome)));
                })
            });
        let deadline = Instant::now() + budget;
        let cancel = CancellationToken::new();
        let (connection, _) = time::timeout(SETUP, async {
            tokio::join!(
                async {
                    let target =
                        EndpointAddr::new(server.id()).with_relay_url(silent_first.clone());
                    let connection = client
                        .connect_with_routes_until(target, options, deadline, &cancel)
                        .await
                        .unwrap();
                    assert_eq!(connection.remote_id(), server.id());
                    assert_eq!(
                        connection.initial_data_path(),
                        &TransportAddr::Relay(healthy.clone())
                    );
                    request(&connection).await;
                    connection
                },
                async {
                    let connection = server.accept().await.unwrap().unwrap();
                    assert_eq!(connection.remote_id(), member_id);
                    let mut stream = connection.accept_bi().await.unwrap();
                    let (send, recv) = stream.streams();
                    assert_eq!(recv.read_to_end(64).await.unwrap(), REQUEST);
                    send.write_all(REPLY).await.unwrap();
                    send.finish().unwrap();
                    assert_eq!(send.stopped().await.unwrap(), None);
                }
            )
        })
        .await
        .unwrap();
        {
            let timeline = timeline.lock().unwrap();
            let mut pending = 0;
            let mut starts = Vec::new();
            for (url, event) in timeline.iter() {
                match event {
                    Event::Started(at, route_deadline) => {
                        pending += 1;
                        assert!(pending <= 2);
                        assert_eq!(*route_deadline, deadline);
                        starts.push((url, *at));
                    }
                    Event::Finished(Ok(stable_id)) => {
                        pending -= 1;
                        assert_eq!(*stable_id, connection.stable_id());
                        assert_eq!(*url, healthy);
                    }
                    Event::Cancelled => pending -= 1,
                    Event::Finished(Err(error)) => panic!("unexpected failure from {url}: {error}"),
                }
            }
            assert_eq!(
                pending, 0,
                "all relay attempts resolved or cancelled: {timeline:?}"
            );
            assert_eq!(
                starts.iter().map(|(route, _)| *route).collect::<Vec<_>>(),
                std::iter::once(&silent_first)
                    .chain(candidates.iter())
                    .collect::<Vec<_>>(),
            );
            let actual = starts[1].1.duration_since(starts[0].1);
            let expected = (deadline.saturating_duration_since(starts[0].1) / 4)
                .min(Duration::from_millis(250));
            assert!(
                actual.abs_diff(expected) <= Duration::from_millis(30),
                "expected {expected:?}, saw {actual:?}"
            );
            println!(
                "relay-race budget={budget:?} expected={expected:?} actual={actual:?} {timeline:?}"
            );
        }
        client.close().await;
    }
    server.close().await;
    member.close().await;
    admin.close().await;
    for stop in [stop_first, stop_second, stop_tail] {
        stop.cancel();
    }
    relay.shutdown().await.unwrap();
}

#[tokio::test]
async fn approved_member_gets_first_relay_reply_and_revoked_member_cannot_open_a_stream() {
    let service = relay().await;
    let url = service.http_url().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let join_directory = dir.path().join("joining");
    let (mut admin, member_id) = admitted_pair(dir.path(), &url).await;

    let server = admin.bind_member_fusion_endpoint(SETUP).await.unwrap();
    tokio::time::timeout(SETUP, server.online()).await.unwrap();
    let started = monotonic_ms();
    let (dialer, responder) = tokio::time::timeout(SETUP * 2, async {
        tokio::join!(
            worker(&join_directory, server.id(), &url, "approved"),
            async {
                let connection = server.accept().await.unwrap().unwrap();
                assert_eq!(connection.remote_id(), member_id);
                let stable_id = connection.stable_id();
                for _ in 0..3 {
                    let mut stream = connection.accept_bi().await.unwrap();
                    let (send, recv) = stream.streams();
                    assert_eq!(recv.read_to_end(64).await.unwrap(), REQUEST);
                    send.write_all(REPLY).await.unwrap();
                    send.finish().unwrap();
                    assert_eq!(send.stopped().await.unwrap(), None);
                }
                assert_eq!(connection.stable_id(), stable_id);
                let report = reply_record(&connection, &url, "responder", started);
                eprintln!("{report}");
                report
            },
        )
    })
    .await
    .unwrap();
    assert_eq!(admission_starts(&records(&dialer)).len(), 1);
    let dialer = output_record(&dialer, "fusion-first-reply", "initiator");
    assert_eq!(dialer["attemptId"], responder["attemptId"]);
    assert_eq!(dialer["round"], responder["round"]);
    assert_ne!(dialer["pid"], responder["pid"]);
    assert_eq!(dialer["peer"], server.id().to_string());

    let mut manager = admin.begin_management_session();
    admin
        .manage(
            &mut manager,
            ManagementAction::RemoveMember(VerifyingKey::from_bytes(member_id.as_bytes()).unwrap()),
        )
        .await
        .unwrap();
    assert!(
        !admin
            .status()
            .members
            .iter()
            .any(|(id, _)| id.as_bytes() == member_id.as_bytes())
    );
    let (dialer, denied) = tokio::time::timeout(SETUP * 2, async {
        tokio::join!(
            worker(&join_directory, server.id(), &url, "revoked"),
            async { server.accept().await.unwrap() },
        )
    })
    .await
    .unwrap();
    assert!(
        matches!(
            denied,
            Err(ConnectingError::AdmissionFailed {
                kind: ErrorKind::PermissionDenied,
                ..
            })
        ),
        "responder must reach admission and deny business: {denied:?}"
    );
    let rejection = output_record(&dialer, "connection-timeline", "initiator");
    assert_ne!(rejection["attemptId"], responder["attemptId"]);
    assert_eq!(rejection["round"], 1);
    server.close().await;
    admin.close().await;
    service.shutdown().await.unwrap();
}

async fn worker(
    directory: &Path,
    peer: EndpointId,
    relay: &RelayUrl,
    expected: &str,
) -> process::Output {
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "member_fusion::dialer_process",
            "--nocapture",
        ])
        .env("MIOPUNCH_FUSION_DIRECTORY", directory)
        .env("MIOPUNCH_FUSION_PEER", peer.to_string())
        .env("MIOPUNCH_FUSION_RELAY", relay.as_str())
        .env("MIOPUNCH_FUSION_EXPECTED", expected)
        .kill_on_drop(true)
        .output()
        .await
        .unwrap();
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    assert!(
        output.status.success(),
        "{expected}: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    output
}

#[tokio::test]
#[ignore = "launched as a separate native process by the relay acceptance test"]
async fn dialer_process() {
    let directory = std::env::var_os("MIOPUNCH_FUSION_DIRECTORY").unwrap();
    let peer: EndpointId = std::env::var("MIOPUNCH_FUSION_PEER")
        .unwrap()
        .parse()
        .unwrap();
    let relay: RelayUrl = std::env::var("MIOPUNCH_FUSION_RELAY")
        .unwrap()
        .parse()
        .unwrap();
    let expected = std::env::var("MIOPUNCH_FUSION_EXPECTED").unwrap();
    let directory = Path::new(&directory);
    let node = NativeNetwork::open(directory).await.unwrap();
    assert_eq!(
        node.admission_stage(),
        miopunch_native_probe::manual_admission::AdmissionStage::Complete
    );
    if expected.starts_with("idle") {
        let idle_budget = if expected == "idle" { IDLE } else { IDLE * 3 };
        let mut config = node.resource_configuration().unwrap();
        config.maintenance.exploration_interval_ms = idle_budget.as_millis().try_into().unwrap();
        node.save_resource_configuration(&config).unwrap();
    }
    let held_outgoing = if expected == "stalled-hold" {
        Some(Arc::new(Notify::new()))
    } else {
        None
    };
    let endpoint = if let Some(held) = &held_outgoing {
        node.bind_member_fusion_endpoint_with(
            Endpoint::builder(presets::Minimal).hooks(HoldOutgoing(held.clone())),
            SETUP,
        )
        .await
        .unwrap()
    } else {
        node.bind_member_fusion_endpoint(SETUP).await.unwrap()
    };
    let started = monotonic_ms();
    let target = EndpointAddr::new(peer).with_relay_url(relay.clone());
    if expected == "approved" {
        let cancellation = CancellationToken::new();
        let timeout_only = CancellationToken::new();
        let deadline = Instant::now() + SETUP;
        let (cancelled, timed_out, first, second, ()) = tokio::join!(
            endpoint.connect_until(target.clone(), deadline, &cancellation),
            endpoint.connect_until(
                target.clone(),
                Instant::now() + Duration::from_millis(15),
                &timeout_only,
            ),
            endpoint.connect(target.clone()),
            endpoint.connect(target.clone()),
            async {
                tokio::task::yield_now().await;
                cancellation.cancel();
            },
        );
        assert_eq!(cancelled.unwrap_err().kind(), ErrorKind::Interrupted);
        assert_eq!(timed_out.unwrap_err().kind(), ErrorKind::TimedOut);
        let connection = first.unwrap();
        let second = second.unwrap();
        assert_eq!(connection.stable_id(), second.stable_id());
        assert_eq!(connection.remote_id(), peer);
        let stable_id = connection.stable_id();
        tokio::join!(request(&connection), request(&second));
        let reused = endpoint.connect(target).await.unwrap();
        assert_eq!(stable_id, reused.stable_id());
        request(&reused).await;
        assert_eq!(connection.stable_id(), stable_id);
        eprintln!(
            "{}",
            reply_record(&connection, &relay, "initiator", started)
        );
    } else if expected == "revoked" {
        let result = endpoint.connect(target).await;
        assert!(result.is_err(), "revoked member received a business handle");
    } else {
        wire_process(
            directory.parent().unwrap(),
            &endpoint,
            target,
            &relay,
            &expected,
            held_outgoing,
        )
        .await;
    }
    endpoint.close().await;
    node.close().await;
}

#[tokio::test]
async fn one_way_uses_the_existing_admission_round_and_crossed_dials_choose_one_connection() {
    let service = relay().await;
    let url = service.http_url().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let (admin, joining) = admitted_pair(dir.path(), &url).await;
    let admin_id = EndpointId::from_bytes(admin.status().node_id.as_bytes()).unwrap();
    admin.close().await;
    let admin_path = dir.path().join("admin");
    let joining_path = dir.path().join("joining");
    let (low, low_path, high, high_path) = if admin_id.as_bytes() < joining.as_bytes() {
        (admin_id, &admin_path, joining, &joining_path)
    } else {
        (joining, &joining_path, admin_id, &admin_path)
    };

    for (low_mode, high_mode) in [
        ("low-dial", "low-serve"),
        ("high-serve", "high-dial"),
        ("stalled-hold", "stalled-reverse"),
        ("cross", "cross"),
        ("stale", "stale"),
    ] {
        let (low_output, high_output) = time::timeout(SETUP * 2, async {
            tokio::join!(
                worker(low_path, high, &url, low_mode),
                worker(high_path, low, &url, high_mode)
            )
        })
        .await
        .unwrap();
        let low_records = records(&low_output);
        let high_records = records(&high_output);
        let low_report = wire_report(&low_records);
        let high_report = wire_report(&high_records);
        assert_ne!(low_report["pid"], high_report["pid"]);
        assert_eq!(low_report["attemptId"], high_report["attemptId"]);
        assert_eq!(low_report["peer"], high.to_string());
        assert_eq!(high_report["peer"], low.to_string());
        let client_is_low = matches!(low_mode, "low-dial" | "cross" | "stale");
        assert_eq!(
            low_report["client"], client_is_low,
            "{low_mode}: expected the preferred candidate if both are qualified"
        );
        assert_eq!(high_report["client"], !client_is_low);
        let winner = low_report["attemptId"].as_str().unwrap();

        // SCRAM opens one stream from the QUIC client. Just one additional
        // control stream, opened by the larger ID, carries READY and COMMIT.
        assert_eq!(low_report["firstStreamIndex"], u64::from(client_is_low));
        assert_eq!(
            high_report["firstStreamIndex"],
            1 + u64::from(!client_is_low)
        );
        assert_eq!(
            stage(&low_records, winner),
            ["ready-received", "commit-sent"]
        );
        assert_eq!(
            stage(&high_records, winner),
            ["ready-sent", "commit-received"]
        );
        assert_local_delivery(&low_records, low_report);
        assert_local_delivery(&high_records, high_report);

        let low_starts = admission_starts(&low_records);
        let high_starts = admission_starts(&high_records);
        if low_mode == "stalled-hold" {
            assert!(
                low_records
                    .iter()
                    .any(|record| record["event"] == "fusion-outgoing-held")
            );
        }
        if matches!(low_mode, "cross" | "stale") {
            assert_eq!(low_starts.len(), 2);
            assert_eq!(high_starts.len(), 2);
            assert_overlap(&low_records, &low_starts);
            assert_overlap(&high_records, &high_starts);
            let loser = low_starts
                .iter()
                .find_map(|record| {
                    let id = record["attemptId"].as_str().unwrap();
                    (id != winner).then_some(id)
                })
                .unwrap();
            assert!(
                high_starts
                    .iter()
                    .any(|record| record["attemptId"] == loser)
            );
            let ready = timeline_position(
                &low_records,
                loser,
                "member-fusion-admission-stage",
                "ready-received",
            );
            let remote_ready = timeline_position(
                &high_records,
                loser,
                "member-fusion-admission-stage",
                "ready-sent",
            );
            assert!(timeline_position(&low_records, winner, "data-relay", "qualified") < ready);
            if low_mode == "stale" {
                assert_eq!(
                    stage(&low_records, loser),
                    [
                        "ready-received",
                        "qualification-requested",
                        "qualification-confirmed",
                        "superseded"
                    ]
                );
                assert_eq!(
                    stage(&high_records, loser),
                    ["ready-sent", "qualification-confirmed", "superseded"]
                );
                let qualified = high_records
                    .iter()
                    .position(|record| record["event"] == "fusion-preferred-qualified")
                    .unwrap();
                let preferred_ready = timeline_position(
                    &high_records,
                    winner,
                    "member-fusion-admission-stage",
                    "ready-sent",
                );
                let confirmation = timeline_position(
                    &high_records,
                    loser,
                    "member-fusion-admission-stage",
                    "qualification-confirmed",
                );
                assert!(remote_ready < qualified && qualified < confirmation);
                assert!(qualified < preferred_ready);
            } else {
                assert_eq!(stage(&low_records, loser), ["ready-received", "superseded"]);
                assert_eq!(stage(&high_records, loser), ["ready-sent", "superseded"]);
                assert!(
                    timeline_position(&high_records, winner, "data-relay", "qualified")
                        < remote_ready
                );
            }
            let committed = timeline_position(
                &low_records,
                winner,
                "member-fusion-admission-stage",
                "commit-sent",
            );
            assert!(
                ready < committed,
                "both candidates must be qualified before selection"
            );
        } else {
            assert_eq!(low_starts.len(), 1);
            assert_eq!(high_starts.len(), 1);
        }
    }
    let admin = NativeNetwork::open(&admin_path).await.unwrap();
    let member = NativeNetwork::open(&joining_path).await.unwrap();
    let fused = admin.bind_member_fusion_endpoint(SETUP).await.unwrap();
    let separate = bind(&member).await;
    time::timeout(SETUP, async {
        tokio::join!(separate.online(), fused.online())
    })
    .await
    .unwrap();
    for alpn in [
        miopunch_native_probe::connection_coordinator::REPLICA_ALPN,
        miopunch_native_probe::terminal::TERMINAL_ALPN,
        miopunch_native_probe::admission_gate::ADMISSION_REQUEST_ALPN,
        miopunch_native_probe::genesis_recovery::RECOVERY_ALPN,
    ] {
        let target = EndpointAddr::new(fused.id()).with_relay_url(url.clone());
        let rejected = time::timeout(SETUP, separate.connect(target, alpn))
            .await
            .unwrap();
        assert!(
            rejected.is_err(),
            "dedicated protocol obtained a fused member connection: {alpn:?}"
        );
    }
    tokio::join!(fused.close(), separate.close());
    tokio::join!(admin.close(), member.close());
    service.shutdown().await.unwrap();
}

async fn wire_process(
    root: &Path,
    endpoint: &MemberFusionEndpoint,
    target: EndpointAddr,
    relay: &RelayUrl,
    mode: &str,
    held_outgoing: Option<Arc<Notify>>,
) {
    let started = monotonic_ms();
    let local = endpoint.id();
    let remote = target.id;
    let fixture = mode.split('-').next().unwrap();
    let stale_gate = (mode == "stale").then(|| endpoint.test_coordinate_stale_readiness());
    if mode == "cross" {
        endpoint.test_coordinate_qualified_candidates();
    }
    time::timeout(SETUP, endpoint.online()).await.unwrap();
    tokio::fs::write(root.join(format!("{fixture}-bound-{local}")), [])
        .await
        .unwrap();
    let remote_is_ready = root.join(format!("{fixture}-bound-{remote}"));
    while !remote_is_ready.exists() {
        time::sleep(Duration::from_millis(5)).await;
    }
    let dialing = matches!(
        mode,
        "low-dial" | "high-dial" | "cross" | "stale" | "idle" | "stalled-hold" | "stalled-reverse"
    );
    let stalled = root.join("stalled-outgoing-entered");
    let reverse_deadline = if mode == "stalled-reverse" {
        while !stalled.exists() {
            time::sleep(Duration::from_millis(5)).await;
        }
        Some(Instant::now() + REVERSE_BUDGET)
    } else {
        None
    };
    let connection =
        if let Some((preferred_qualified, release_ready, reverse_response)) = stale_gate {
            let (connection, ()) = tokio::join!(endpoint.connect(target), async {
                let remote_qualification = root.join(format!("stale-preferred-qualified-{remote}"));
                let reverse = root.join("stale-reverse-response");
                preferred_qualified.notified().await;
                eprintln!("{}", json!({"event": "fusion-preferred-qualified"}));
                tokio::fs::write(root.join(format!("stale-preferred-qualified-{local}")), [])
                    .await
                    .unwrap();
                if local.as_bytes() < remote.as_bytes() {
                    while !remote_qualification.exists() {
                        time::sleep(Duration::from_millis(5)).await;
                    }
                    release_ready.notify_one();
                    reverse_response.notified().await;
                    tokio::fs::write(reverse, []).await.unwrap();
                } else {
                    while !reverse.exists() {
                        time::sleep(Duration::from_millis(5)).await;
                    }
                    release_ready.notify_one();
                }
            });
            connection.unwrap()
        } else if let Some(held_outgoing) = held_outgoing {
            let (connection, ()) = tokio::join!(endpoint.connect(target), async {
                held_outgoing.notified().await;
                eprintln!("{}", json!({"event": "fusion-outgoing-held"}));
                tokio::fs::write(stalled, []).await.unwrap();
            });
            connection.unwrap()
        } else if let Some(deadline) = reverse_deadline {
            endpoint
                .connect_until(target, deadline, &CancellationToken::new())
                .await
                .expect("the sole qualified reverse candidate must meet the caller's deadline")
        } else if dialing {
            endpoint.connect(target).await.unwrap()
        } else {
            endpoint.accept().await.unwrap().unwrap()
        };
    assert_eq!(connection.remote_id(), remote);
    if matches!(mode, "cross" | "stale" | "stalled-hold") && connection.side().is_server() {
        assert_eq!(
            endpoint.accept().await.unwrap().unwrap().stable_id(),
            connection.stable_id()
        );
    }
    if fixture == "idle" {
        idle_streams(&connection, dialing).await;
        return;
    }
    let role = if connection.side().is_client() {
        "initiator"
    } else {
        "responder"
    };
    let mut report = reply_record(&connection, relay, role, started);
    report["event"] = json!("fusion-wire");
    report["client"] = json!(connection.side().is_client());
    let (index, ()) = tokio::join!(
        async {
            let mut stream = connection.open_bi().await.unwrap();
            let (send, recv) = stream.streams();
            let request = format!("request 1 from {local}");
            send.write_all(request.as_bytes()).await.unwrap();
            send.finish().unwrap();
            let reply = format!("reply 1 from {remote}");
            assert_eq!(recv.read_to_end(256).await.unwrap(), reply.as_bytes());
            send.id().index()
        },
        async {
            let mut stream = connection.accept_bi().await.unwrap();
            let (send, recv) = stream.streams();
            let request = format!("request 1 from {remote}");
            assert_eq!(recv.read_to_end(256).await.unwrap(), request.as_bytes());
            let reply = format!("reply 1 from {local}");
            send.write_all(reply.as_bytes()).await.unwrap();
            send.finish().unwrap();
            assert_eq!(send.stopped().await.unwrap(), None);
        },
    );
    assert_eq!(connection.stable_id(), report["stableId"]);
    report["firstStreamIndex"] = json!(index);
    report["firstReplyMs"] = json!(monotonic_ms());
    eprintln!("{report}");
    if let Some(deadline) = reverse_deadline {
        assert!(
            Instant::now() < deadline,
            "bidirectional replies must fit within the reverse caller budget"
        );
    }
    assert!(
        time::timeout(Duration::from_millis(250), endpoint.accept())
            .await
            .is_err()
    );
}

fn wire_report(records: &[Value]) -> &Value {
    records
        .iter()
        .find(|record| record["event"] == "fusion-wire")
        .unwrap()
}

fn admission_starts(records: &[Value]) -> Vec<&Value> {
    records
        .iter()
        .filter(|record| record["phase"] == "member-admission" && record["result"] == "started")
        .collect()
}

fn stage<'a>(records: &'a [Value], attempt: &str) -> Vec<&'a str> {
    records
        .iter()
        .filter_map(|record| {
            (record["attemptId"] == attempt && record["phase"] == "member-fusion-admission-stage")
                .then(|| record["result"].as_str().unwrap())
        })
        .collect()
}

fn timeline_position(records: &[Value], attempt: &str, phase: &str, result: &str) -> usize {
    records
        .iter()
        .position(|record| {
            record["attemptId"] == attempt && record["phase"] == phase && record["result"] == result
        })
        .unwrap()
}

fn assert_local_delivery(records: &[Value], report: &Value) {
    let attempt = report["attemptId"].as_str().unwrap();
    let admitted = timeline_position(records, attempt, "member-admission", "succeeded");
    let qualified = timeline_position(records, attempt, "data-relay", "qualified");
    let stage_ready = if stage(records, attempt)[0] == "ready-sent" {
        "ready-sent"
    } else {
        "ready-received"
    };
    let ready = timeline_position(
        records,
        attempt,
        "member-fusion-admission-stage",
        stage_ready,
    );
    assert!(admitted < qualified && qualified < ready);
    assert_eq!(records[ready]["stableId"], report["stableId"]);
    assert!(
        records[ready]["monotonicMs"].as_u64().unwrap() <= report["firstReplyMs"].as_u64().unwrap()
    );
}

fn assert_overlap(records: &[Value], starts: &[&Value]) {
    let first = starts[0]["attemptId"].as_str().unwrap();
    let second_start = timeline_position(
        records,
        starts[1]["attemptId"].as_str().unwrap(),
        "member-admission",
        "started",
    );
    let first_end = timeline_position(records, first, "member-admission", "succeeded");
    assert!(
        second_start < first_end,
        "the second TLS connection must enter admission while the first is active"
    );
}

#[tokio::test]
async fn an_independent_stream_failure_keeps_business_alive_and_periodic_ping_does_not_extend_idle()
{
    let service = relay().await;
    let url = service.http_url().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let (admin, joining) = admitted_pair(dir.path(), &url).await;
    let admin_id = EndpointId::from_bytes(admin.status().node_id.as_bytes()).unwrap();
    admin.close().await;
    let admin_path = dir.path().join("admin");
    let joining_path = dir.path().join("joining");
    let (active, server) = time::timeout(SETUP * 2, async {
        tokio::join!(
            worker(&joining_path, admin_id, &url, "idle"),
            worker(&admin_path, joining, &url, "idle-serve"),
        )
    })
    .await
    .unwrap();
    assert_eq!(admission_starts(&records(&active)).len(), 1);
    assert_eq!(admission_starts(&records(&server)).len(), 1);
    let initiator = output_record(&active, "fusion-idle", "initiator");
    let responder = output_record(&server, "fusion-idle", "responder");
    assert_ne!(initiator["pid"], responder["pid"]);
    assert_eq!(initiator["attemptId"], responder["attemptId"]);
    assert_eq!(initiator["ownerClosed"], true);
    assert_eq!(responder["ownerClosed"], false);
    service.shutdown().await.unwrap();
}

async fn idle_streams(connection: &MemberConnection, initiator: bool) {
    const PREFIX: &[u8] = b"leased before idle";
    const SUFFIX: &[u8] = b"still open after another stream was reset";
    const RESET: &[u8] = b"reset this stream";
    const PING: &[u8] = b"maintenance ping";
    const PONG: &[u8] = b"maintenance pong";

    let released = if initiator {
        let mut business = connection.open_bi().await.unwrap();
        let (send, recv) = business.streams();
        send.write_all(PREFIX).await.unwrap();
        let mut doomed = connection.open_bi().await.unwrap();
        let (doomed_send, doomed_recv) = doomed.streams();
        doomed_send.write_all(RESET).await.unwrap();
        assert_eq!(doomed_recv.read_to_end(64).await.unwrap(), RESET);
        doomed_send.reset(7u32.into()).unwrap();
        drop(doomed);

        time::sleep(IDLE * 2).await;
        send.write_all(SUFFIX).await.unwrap();
        send.finish().unwrap();
        assert_eq!(recv.read_to_end(64).await.unwrap(), SUFFIX);
        let released = Instant::now();
        drop(business);
        released
    } else {
        let mut business = connection.accept_bi().await.unwrap();
        let (send, recv) = business.streams();
        let mut leading = [0; PREFIX.len()];
        recv.read_exact(&mut leading).await.unwrap();
        assert_eq!(leading, PREFIX);
        let mut doomed = connection.accept_bi().await.unwrap();
        let (doomed_send, doomed_recv) = doomed.streams();
        let mut message = [0; RESET.len()];
        doomed_recv.read_exact(&mut message).await.unwrap();
        assert_eq!(message, RESET);
        doomed_send.write_all(RESET).await.unwrap();
        doomed_send.finish().unwrap();
        assert!(doomed_recv.read_to_end(64).await.is_err());
        drop(doomed);

        assert_eq!(recv.read_to_end(64).await.unwrap(), SUFFIX);
        send.write_all(SUFFIX).await.unwrap();
        send.finish().unwrap();
        assert_eq!(send.stopped().await.unwrap(), None);
        let released = Instant::now();
        drop(business);
        released
    };
    let (sent, received, closed) = time::timeout(IDLE * 2, async {
        tokio::join!(
            async {
                let mut count = 0;
                let mut ticks = time::interval(IDLE / 10);
                loop {
                    ticks.tick().await;
                    let Ok(mut stream) = connection.open_maintenance_bi().await else {
                        break;
                    };
                    let (send, recv) = stream.streams();
                    if send.write_all(PING).await.is_err() || send.finish().is_err() {
                        break;
                    }
                    let Ok(reply) = recv.read_to_end(64).await else {
                        break;
                    };
                    assert_eq!(reply, PONG);
                    count += 1;
                }
                count
            },
            async {
                let mut count = 0;
                while let Ok(mut stream) = connection.accept_bi().await {
                    let (send, recv) = stream.streams();
                    let Ok(request) = recv.read_to_end(64).await else {
                        break;
                    };
                    assert_eq!(request, PING);
                    if send.write_all(PONG).await.is_err() || send.finish().is_err() {
                        break;
                    }
                    count += 1;
                }
                count
            },
            connection.closed(),
        )
    })
    .await
    .expect("periodic ping must not retain the idle connection");
    assert!(
        sent >= 2 && received >= 2,
        "actual maintenance traffic in both directions: {sent}/{received}"
    );
    let elapsed = released.elapsed();
    assert_eq!(matches!(closed, ConnectionError::LocallyClosed), initiator);
    if initiator {
        assert!(
            elapsed >= IDLE && elapsed < IDLE * 3 / 2,
            "idle budget elapsed: {elapsed:?}"
        );
    } else {
        assert!(
            matches!(closed, ConnectionError::ApplicationClosed(ref reason) if reason.reason.as_ref() == b"member connection idle")
        );
    }
    eprintln!(
        "{}",
        json!({
            "event": "fusion-idle", "role": if initiator { "initiator" } else { "responder" },
            "pid": process::id(), "attemptId": hex::encode(member_fusion_attempt_id(connection).unwrap()),
            "stableId": connection.stable_id(), "idleMs": elapsed.as_millis(),
            "pingsSent": sent, "pingsReceived": received, "ownerClosed": initiator,
        })
    );
}

async fn request(connection: &MemberConnection) {
    let mut stream = connection.open_bi().await.unwrap();
    let (send, recv) = stream.streams();
    send.write_all(REQUEST).await.unwrap();
    send.finish().unwrap();
    assert_eq!(recv.read_to_end(64).await.unwrap(), REPLY);
}

fn reply_record(
    connection: &MemberConnection,
    relay: &RelayUrl,
    role: &str,
    started: u64,
) -> Value {
    let relay_path = TransportAddr::Relay(relay.clone());
    assert!(
        connection
            .paths()
            .iter()
            .any(|path| path.is_selected() && path.remote_addr() == &relay_path)
    );
    let finished = monotonic_ms();
    json!({
        "event": "fusion-first-reply",
        "attemptId": hex::encode(member_fusion_attempt_id(connection).unwrap()),
        "round": 1,
        "pid": process::id(),
        "stableId": connection.stable_id(),
        "monotonicMs": finished,
        "durationMs": finished - started,
        "role": role,
        "peer": connection.remote_id().to_string(),
        "relay": relay.as_str(),
    })
}

fn output_record(output: &process::Output, event: &str, role: &str) -> Value {
    records(output)
        .into_iter()
        .find(|record| record["event"] == event && record["role"] == role)
        .unwrap()
}

fn records(output: &process::Output) -> Vec<Value> {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect()
}
