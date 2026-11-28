//! Common active-commit regression on admitted loopback Native SDK members.
//! Uses actual protected M/C and fresh Noq IP-path ACKs; no RTC/WAN claim.
use super::*;
use crate::{
    admission_gate::ProtocolScope, manual_admission::ManualExchange, native_network::NativeNetwork,
};
use iroh::EndpointAddr;
use std::{
    net::Ipv4Addr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

async fn admitted_pair() -> (tempfile::TempDir, NativeNetwork, NativeNetwork) {
    let directory = tempfile::tempdir().unwrap();
    let a_path = directory.path().join("a");
    let b_path = directory.path().join("b");
    let mut a = NativeNetwork::create(&a_path).await.unwrap();
    let mut resources = a.resource_configuration().unwrap();
    resources.replacement = Some(vec![]);
    resources.tcp_direct_disabled = true;
    a.save_resource_configuration(&resources).unwrap();
    let b = add_member(&mut a, &b_path).await;
    tokio::join!(a.close(), b.close());
    (
        directory,
        NativeNetwork::open(&a_path).await.unwrap(),
        NativeNetwork::open(&b_path).await.unwrap(),
    )
}

async fn add_member(a: &mut NativeNetwork, path: &std::path::Path) -> NativeNetwork {
    let mut b = NativeNetwork::join(path, a.manual_invitation().unwrap())
        .await
        .unwrap();
    let mut resources = b.resource_configuration().unwrap();
    resources.replacement = Some(vec![]);
    resources.tcp_direct_disabled = true;
    b.save_resource_configuration(&resources).unwrap();
    let ae = a.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let be = b.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let deadline = n0_future::time::Instant::now() + Duration::from_secs(5);
    let options = iroh::endpoint::ConnectOptions::new().with_caller_deadline(deadline);
    let (client, server) = tokio::join!(
        async {
            be.connect_control_with_opts(ae.addr(), ADMISSION_REQUEST_ALPN, options)
                .await
                .unwrap()
                .await
                .unwrap()
        },
        async {
            ae.accept()
                .await
                .unwrap()
                .with_caller_deadline(deadline)
                .accept_control()
                .unwrap()
                .await
                .unwrap()
        }
    );
    let (client, server) = (Arc::new(client), Arc::new(server));
    let (request, served) = tokio::join!(
        b.request_manual_admission(&client),
        a.serve_manual_admission(&server)
    );
    assert!(matches!(request.unwrap(), ManualExchange::AwaitingApproval));
    served.unwrap();
    let request = a.manual_requests().await.unwrap().pop().unwrap();
    a.approve_manual_admission(&request).await.unwrap();
    let receiver = async {
        let ManualExchange::Materials(material) =
            b.request_manual_admission(&client).await.unwrap()
        else {
            panic!("approved admission material absent")
        };
        b.complete_manual_admission(material).await.unwrap();
    };
    let (_, served) = tokio::join!(receiver, a.serve_manual_admission(&server));
    served.unwrap();
    client.close(0u32.into(), b"admission complete");
    server.close(0u32.into(), b"admission complete");
    tokio::join!(ae.close(), be.close());
    b
}

#[cfg(feature = "rtc-probe")]
#[ignore = "requires the owned MQTT broker and structured maintenance timeline"]
#[test]
fn busy_member_and_round_occupancy_leave_other_maintenance_eligible() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "connection_coordinator::active_commit_tests::maintenance_fairness_fixture",
            "--ignored",
            "--nocapture",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    println!("{stdout}");
    eprintln!("{stderr}");
    assert!(output.status.success());
    let (busy, eligible) = stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|record| record["event"] == "maintenance-fairness-fixture")
        .map(|record| {
            (
                record["busy"].as_u64().unwrap(),
                record["eligible"].as_u64().unwrap(),
            )
        })
        .unwrap();
    let actual = stderr
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|record| record["source"] == "maintenance_improvement")
        .collect::<Vec<_>>();
    let first = actual
        .iter()
        .find(|record| record["phase"] == "route-dial" && record["result"] == "started")
        .unwrap();
    assert_eq!(
        first["stableId"], eligible,
        "the ranked but reserved original cannot block the later M"
    );
    let permits = actual
        .iter()
        .filter(|record| {
            record["phase"] == "probe-budget"
                && record["result"] == "reserved"
                && record["detail"]["resource"] == "enhanced_round"
        })
        .map(|record| record["stableId"].as_u64().unwrap())
        .collect::<Vec<_>>();
    assert!(
        permits.len() >= 2,
        "expected two real enhanced refills, got {permits:?}"
    );
    assert_ne!(
        permits[0], permits[1],
        "two consecutive enhanced permits must reach different due Ms"
    );
    assert!(permits[..2].contains(&busy) && permits[..2].contains(&eligible));
}

#[cfg(feature = "rtc-probe")]
#[tokio::test]
#[ignore = "run through busy_member_and_round_occupancy_leave_other_maintenance_eligible"]
async fn maintenance_fairness_fixture() {
    use crate::{
        control_session::ControlSession,
        resources::{Resource, ResourceUse},
    };
    use iroh::protocol::{AcceptError, ProtocolHandler, RestrictedRouter};

    #[derive(Debug, Clone)]
    struct ObserveOriginal(tokio::sync::mpsc::Sender<Connection>);
    impl ProtocolHandler for ObserveOriginal {
        async fn accept(&self, original: Connection) -> Result<(), AcceptError> {
            self.0.send(original.clone()).await.unwrap();
            original.closed().await;
            Ok(())
        }
    }

    let broker = std::env::var("MIOPUNCH_MQTT_BROKER").expect("owned MQTT broker required");
    let (directory, mut caller, peer) = admitted_pair().await;
    let neighbor = add_member(&mut caller, &directory.path().join("neighbor")).await;
    let mut relay_config = iroh_relay::server::ServerConfig::default();
    relay_config.relay = Some(iroh_relay::server::RelayConfig::new((
        Ipv4Addr::LOCALHOST,
        0,
    )));
    let relay = iroh_relay::server::Server::spawn(relay_config)
        .await
        .unwrap();
    let relay_url = relay.http_url().unwrap();
    for node in [&caller, &peer, &neighbor] {
        let mut config = node.resource_configuration().unwrap();
        let mut resources = vec![Resource {
            url: relay_url.to_string(),
            uses: [ResourceUse::DataRelay].into(),
            limitation: "owned native relay".into(),
        }];
        if std::ptr::eq(node, &caller) {
            resources.push(Resource {
                url: format!("mqtt://{broker}/"),
                uses: [ResourceUse::Signaling, ResourceUse::Lookup].into(),
                limitation: "owned MQTT broker".into(),
            });
        }
        config.replacement = Some(resources);
        config.maintenance.exploration_interval_ms = 250;
        config.maintenance.path_retry_base_ms = 250;
        config.maintenance.enhanced_udp_cooldown_ms = 8_000;
        config.maintenance.discovery_interval_ms = 1_000;
        config.maintenance.contact_timeout_ms = 5_000;
        node.save_resource_configuration(&config).unwrap();
    }
    let gate = caller.admission_gate();
    let udp = caller
        .control_udp()
        .with_maintenance_policy(&caller.resource_configuration().unwrap().maintenance)
        .unwrap();
    let rtc = crate::rtc_transport::RtcTransport::new();
    rtc.set_resource_allowed(false);
    let caller = caller.maintain_network_with_rtc(Ipv4Addr::LOCALHOST, rtc, |b| b, |r| r).await.unwrap();
    udp.config().set_udp_enabled(false);
    let peer_endpoint = Arc::new(
        peer.bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap(),
    );
    let (incoming, mut accepted) = tokio::sync::mpsc::channel(2);
    let router = RestrictedRouter::builder(peer_endpoint.endpoint().clone())
        .accept(MEMBER_ALPN, ObserveOriginal(incoming))
        .accept_control(
            ACTIVE_IMPROVEMENT_ALPN,
            peer.active_improvement_handler(peer_endpoint.clone()),
        )
        .spawn();
    let neighbor = neighbor
        .maintain_network(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(45), async {
        tokio::join!(caller.endpoint().online(), peer_endpoint.endpoint().online(), neighbor.endpoint().online());
        let relay_path = TransportAddr::Relay(relay_url);
        let target = EndpointAddr::new(peer_endpoint.endpoint().id()).with_addrs([relay_path.clone()]);
        let (first, activity) = caller.connect(target.clone(), MEMBER_ALPN).await.unwrap();
        let first_remote = accepted.recv().await.unwrap();
        let (second, second_activity) = caller.connect(target, MEMBER_ALPN).await.unwrap();
        let second_remote = accepted.recv().await.unwrap();
        drop(second_activity);
        for (left, right) in [(&first, &first_remote), (&second, &second_remote)] {
            let left_control = caller.endpoint().control_handle(left);
            let right_control = peer_endpoint.endpoint().control_handle(right);
            tokio::try_join!(
                left_control.test_limit_transport_paths(vec![relay_path.clone()]),
                right_control.test_limit_transport_paths(vec![relay_path.clone()]),
            ).unwrap();
            for original in [left, right] {
                for path in original.paths().iter().filter(|path| path.is_ip()) {
                    path.close().unwrap();
                }
                assert!(original.paths().iter().any(|path| path.is_relay()));
                assert!(original.paths().iter().all(|path| path.is_relay()));
            }
        }
        let (mut send, mut recv) = first.open_bi().await.unwrap();
        send.write_all(b"start").await.unwrap();
        let (mut reply, mut inbound) = first_remote.accept_bi().await.unwrap();
        let mut bytes = [0; 5];
        inbound.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"start");
        let (head, other) = if first.stable_id() < second.stable_id() {
            (&first, &second)
        } else {
            (&second, &first)
        };
        let attempt = AttemptContext::new(
            n0_future::time::Instant::now() + Duration::from_secs(15),
            tokio_util::sync::CancellationToken::new(),
        );
        let busy = gate.maintenance().acquire(head, &attempt, ()).unwrap();
        let observing = ControlSession::from_control(caller.endpoint().control_handle(head));
        let before = caller.maintenance_status();
        assert_eq!(before.contact_attempts, 0);
        println!("{}", json!({"event":"maintenance-fairness-fixture","busy":head.stable_id(),"eligible":other.stable_id()}));
        udp.config().set_udp_enabled(true);
        let held = udp.observe(&observing, &gate).await.unwrap();
        let mut status = caller.watch_maintenance();
        tokio::time::timeout(Duration::from_secs(8), status.wait_for(|s| s.contact_attempts > 0))
            .await.expect("later M must start while the head's reservation is held").unwrap();
        assert!(held.is_current());
        assert_eq!(gate.maintenance().active_attempts(), 1);
        println!("{}", json!({"event":"maintenance-fairness-release","contactAttempts":status.borrow().contact_attempts}));
        drop((held, busy, activity));
        caller.offer_contact(neighbor.addr()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(12), status.wait_for(|s| {
            s.last_sync.as_ref().is_some_and(|sync| sync.source == neighbor.endpoint().id() && sync.complete())
                && s.discovery_rounds > before.discovery_rounds
        })).await.expect("sync and discovery while both relay Ms repeatedly become due").unwrap();
        // Observe two naturally refilled node allowances (F = 250ms, 32F = 8s).
        tokio::time::sleep(Duration::from_secs(23)).await;
        assert!(first.close_reason().is_none() && second.close_reason().is_none());
        assert!(caller.maintenance_status().discovery_rounds > before.discovery_rounds + 1);
        send.write_all(b"after").await.unwrap();
        inbound.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"after");
        reply.write_all(b"still").await.unwrap();
        recv.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"still");
    }).await.unwrap();
    tokio::join!(caller.close(), neighbor.close());
    router.shutdown().await.unwrap();
    peer_endpoint.endpoint().close().await;
    peer.close().await;
    relay.shutdown().await.unwrap();
}

#[cfg(feature = "rtc-probe")]
#[tokio::test]
async fn repair_window_expiry_preserves_the_original_relay_stream() {
    use crate::connection_recovery::ConnectionPathRecoveryError;
    use tokio::{
        net::{TcpListener, TcpStream},
        sync::watch,
        time::{Instant, timeout},
    };

    let (_directory, caller, responder) = admitted_pair().await;
    let mut config = iroh_relay::server::ServerConfig::default();
    config.relay = Some(iroh_relay::server::RelayConfig::new((
        Ipv4Addr::LOCALHOST,
        0,
    )));
    let relay = iroh_relay::server::Server::spawn(config).await.unwrap();
    let upstream = (
        Ipv4Addr::LOCALHOST,
        relay.http_url().unwrap().port().unwrap(),
    );
    let proxy = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let relay_url: iroh::RelayUrl = format!("http://{}/", proxy.local_addr().unwrap())
        .parse()
        .unwrap();
    let (paused, traffic) = watch::channel(false);
    let forward = tokio::spawn(async move {
        let mut circuits = tokio::task::JoinSet::new();
        while let Ok((mut incoming, _)) = proxy.accept().await {
            let mut traffic = traffic.clone();
            circuits.spawn(async move {
                let mut outgoing = TcpStream::connect(upstream).await.unwrap();
                let work = tokio::io::copy_bidirectional(&mut incoming, &mut outgoing);
                tokio::pin!(work);
                loop {
                    let running = !*traffic.borrow_and_update();
                    tokio::select! {
                        biased;
                        update = traffic.changed() => if update.is_err() { break; },
                        _ = &mut work, if running => break,
                    }
                }
            });
        }
    });
    for node in [&caller, &responder] {
        let mut config = node.resource_configuration().unwrap();
        config.udp_direct_disabled = true;
        config.tcp_direct_disabled = true;
        config.replacement = Some(vec![crate::resources::Resource {
            url: relay_url.to_string(),
            uses: [crate::resources::ResourceUse::DataRelay].into(),
            limitation: "owned, pausable relay".into(),
        }]);
        node.save_resource_configuration(&config).unwrap();
    }
    let responder_endpoint = responder
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    responder_endpoint
        .endpoint()
        .set_alpns(vec![MEMBER_ALPN.to_vec(), ACTIVE_IMPROVEMENT_ALPN.to_vec()]);
    let caller = caller
        .maintain_network_with_rtc(Ipv4Addr::LOCALHOST, crate::rtc_transport::RtcTransport::new(), |b| b, |router| router)
        .await
        .unwrap();
    timeout(Duration::from_secs(8), async {
        tokio::join!(
            caller.endpoint().online(),
            responder_endpoint.endpoint().online()
        );
    })
    .await
    .unwrap();
    let target = iroh::EndpointAddr::new(responder_endpoint.endpoint().id())
        .with_addrs([TransportAddr::Relay(relay_url)]);
    let connect = || async {
        let (local, peer) = timeout(Duration::from_secs(8), async {
            tokio::join!(caller.connect(target.clone(), MEMBER_ALPN), async {
                responder_endpoint.endpoint().accept().await.unwrap().await
            })
        })
        .await
        .unwrap();
        (local.unwrap(), peer.unwrap())
    };
    let ((original, _activity), peer) = connect().await;
    let ids = (original.stable_id(), peer.stable_id());
    for connection in [&original, &peer] {
        assert_eq!(connection.paths().iter().count(), 1);
        assert!(connection.paths().iter().all(|path| path.is_relay()));
    }
    let (mut send, mut receive) = original.open_bi().await.unwrap();
    send.write_all(b"before").await.unwrap();
    let (mut reply, mut incoming) = peer.accept_bi().await.unwrap();
    async fn expect_chunk(stream: &mut iroh::endpoint::RecvStream, bytes: &[u8]) {
        let mut data = vec![0; bytes.len()];
        timeout(Duration::from_secs(5), stream.read_exact(&mut data))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(data, bytes);
    }
    expect_chunk(&mut incoming, b"before").await;
    reply.write_all(b"before").await.unwrap();
    expect_chunk(&mut receive, b"before").await;

    let relay_rtt = || original.paths().iter().next().unwrap().rtt();
    if let Some(ready) = std::env::var_os("MIOPUNCH_REPAIR_NETEM_READY") {
        let ready = std::path::PathBuf::from(ready);
        eprintln!("MIO131_NETEM_READY");
        timeout(Duration::from_secs(5), async {
            while !ready.exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        // Measure the live relay RTT before starting the repair clock.
        timeout(Duration::from_secs(20), async {
            while relay_rtt() < Duration::from_secs(1) {
                send.write_all(b"latency").await.unwrap();
                reply.write_all(b"latency").await.unwrap();
                tokio::join!(
                    expect_chunk(&mut incoming, b"latency"),
                    expect_chunk(&mut receive, b"latency")
                );
            }
        })
        .await
        .expect("original relay should reach the representative RTT");
        assert!(
            relay_rtt() < Duration::from_secs(2),
            "relay RTT {:?}",
            relay_rtt()
        );
    }
    let before_pause_rtt = relay_rtt();
    paused.send(true).unwrap();
    let started = Instant::now();
    let result = caller
        .repair_member_connection_until(&original, target.clone(), started + Duration::from_secs(6))
        .await;
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_secs(3) && elapsed < Duration::from_secs(4),
        "repair {result:?} took {elapsed:?}"
    );
    assert!(
        matches!(result, Err(ConnectionPathRecoveryError::Retryable { stable_id, .. }) if stable_id == ids.0),
        "repair {result:?}; original close {:?}",
        original.close_reason()
    );
    assert!(original.close_reason().is_none() && peer.close_reason().is_none());
    paused.send(false).unwrap();
    send.write_all(b"after").await.unwrap();
    expect_chunk(&mut incoming, b"after").await;
    reply.write_all(b"after").await.unwrap();
    expect_chunk(&mut receive, b"after").await;
    assert_eq!((original.stable_id(), peer.stable_id()), ids);
    eprintln!(
        "MIO131_RELAY paused={elapsed:?} relay_rtt={before_pause_rtt:?} original={ids:?} same_stream_bytes=before,after"
    );

    tokio::join!(caller.close(), responder_endpoint.close());
    responder.close().await;
    forward.abort();
    let _ = forward.await;
}

#[cfg(feature = "rtc-probe")]
#[tokio::test]
async fn withdrawing_rtc_permission_closes_only_members_without_an_allowed_backup() {
    use crate::rtc_transport::{RtcDatagramIo, RtcTransport};
    use std::future::poll_fn;
    use tokio::time::timeout;

    let (_directory, caller, responder) = admitted_pair().await;
    let local_rtc = RtcTransport::new();
    let remote_rtc = RtcTransport::new();
    let peer_endpoint = responder
        .bind_configured_endpoint_with_rtc(
            Endpoint::builder(iroh::endpoint::presets::Minimal)
                .bind_addr((Ipv4Addr::LOCALHOST, 0))
                .unwrap(),
            remote_rtc.clone(),
        )
        .await
        .unwrap();
    peer_endpoint
        .endpoint()
        .set_alpns(vec![MEMBER_ALPN.to_vec()]);
    let caller = caller
        .maintain_network_with_rtc(Ipv4Addr::LOCALHOST, local_rtc.clone(), |b| b, |router| router)
        .await
        .unwrap();
    let target = iroh::EndpointAddr::new(peer_endpoint.endpoint().id()).with_addrs(
        peer_endpoint
            .addr()
            .ip_addrs()
            .copied()
            .map(TransportAddr::Ip),
    );
    let packet_pump = |from: RtcDatagramIo, to: RtcDatagramIo| {
        tokio::spawn(async move {
            while let Ok(packet) = poll_fn(|cx| from.poll_outbound(cx)).await {
                if to.receive(packet).is_err() {
                    break;
                }
            }
        })
    };

    let mut live = Vec::new();
    for keep_ip in [false, true] {
        let ((original, activity), peer) = timeout(Duration::from_secs(5), async {
            let (original, peer) =
                tokio::join!(caller.connect(target.clone(), MEMBER_ALPN), async {
                    peer_endpoint.endpoint().accept().await.unwrap().await
                });
            (original.unwrap(), peer.unwrap())
        })
        .await
        .unwrap();
        let local = local_rtc.reserve(&original, || true).unwrap();
        let remote = remote_rtc.reserve(&peer, || true).unwrap();
        local.set_remote(remote.local_addr()).unwrap();
        remote.set_remote(local.local_addr()).unwrap();
        local.activate().unwrap();
        remote.activate().unwrap();
        let pumps = (
            packet_pump(local.io(), remote.io()),
            packet_pump(remote.io(), local.io()),
        );
        let outgoing = caller.endpoint().control_handle(&original);
        let incoming = peer_endpoint.endpoint().control_handle(&peer);
        outgoing
            .open_data_transport_path(TransportAddr::Custom(remote.local_addr()))
            .await
            .unwrap();
        let (local_ack, remote_ack) = timeout(Duration::from_secs(4), async {
            loop {
                if let (Ok(local_ack), Ok(remote_ack)) =
                    (local.start_probe(&original), remote.start_probe(&peer))
                {
                    break (local_ack, remote_ack);
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        timeout(Duration::from_secs(4), async {
            while !local_ack.is_acknowledged().unwrap() || !remote_ack.is_acknowledged().unwrap() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        local.retain(&original, &local_ack).unwrap();
        remote.retain(&peer, &remote_ack).unwrap();
        outgoing
            .retain_data_transport_path(remote.local_addr())
            .await
            .unwrap();
        incoming
            .retain_data_transport_path(local.local_addr())
            .await
            .unwrap();
        if !keep_ip {
            for (control, connection, address) in [
                (&outgoing, &original, remote.local_addr()),
                (&incoming, &peer, local.local_addr()),
            ] {
                control
                    .test_limit_transport_paths(vec![TransportAddr::Custom(address)])
                    .await
                    .unwrap();
                for path in connection.paths().iter().filter(|path| path.is_ip()) {
                    let _ = path.close();
                }
                assert!(connection.paths().iter().all(|path| !path.is_ip()));
            }
        }
        let (mut send, _) = original.open_bi().await.unwrap();
        send.write_all(b"one").await.unwrap();
        let (_, mut recv) = timeout(Duration::from_secs(4), peer.accept_bi())
            .await
            .unwrap()
            .unwrap();
        let mut bytes = [0; 3];
        timeout(Duration::from_secs(4), recv.read_exact(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&bytes, b"one");
        live.push((original, peer, send, recv, local, remote, activity, pumps));
    }
    local_rtc.set_resource_allowed(false);
    let (sole_rtc, _, old_stream, ..) = &mut live[0];
    let closed = timeout(Duration::from_secs(1), sole_rtc.closed())
        .await
        .expect("revoked sole RTC should promptly close M");
    assert!(matches!(
        closed,
        iroh::endpoint::ConnectionError::LocallyClosed
    ));
    assert!(old_stream.write_all(b"old").await.is_err());
    let (with_backup, peer, send, recv, ..) = &mut live[1];
    assert!(with_backup.close_reason().is_none() && peer.close_reason().is_none());
    send.write_all(b"two").await.unwrap();
    let mut bytes = [0; 3];
    timeout(Duration::from_secs(4), recv.read_exact(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&bytes, b"two");
    assert!(
        with_backup
            .paths()
            .iter()
            .any(|path| path.is_ip() && path.is_selected())
    );
    eprintln!("MIO131_RTC withdrawn_sole=terminal allowed_backup=continued");
    tokio::join!(caller.close(), peer_endpoint.close());
    responder.close().await;
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Fault {
    CompletedControlClose,
    PreCommitControlClose,
    OriginalClose,
    CurrentProofInvalid,
}

struct ProbeAdapter {
    inner: NativeControlAdapter,
    gate: crate::admission_gate::AdmissionGate,
    fault: Option<Fault>,
    peer_committed: AtomicBool,
    reached: tokio::sync::Notify,
    post_check_entered: AtomicBool,
    control: Mutex<Option<iroh::endpoint::WeakControlConnection>>,
    node: Option<Arc<tokio::sync::Mutex<Option<NativeNetwork>>>>,
    rollback: Arc<AtomicBool>,
    real_ack: AtomicBool,
    report: Mutex<Option<serde_json::Value>>,
    stages: Mutex<Vec<&'static str>>,
    production_native: bool,
    expect_tcp: bool,
}
impl ProbeAdapter {
    fn new(
        gate: crate::admission_gate::AdmissionGate,
        fault: Option<Fault>,
        node: Option<Arc<tokio::sync::Mutex<Option<NativeNetwork>>>>,
    ) -> Self {
        Self {
            inner: NativeControlAdapter(CoordinatorHook::new(gate.clone(), true)),
            gate,
            fault,
            peer_committed: AtomicBool::new(false),
            reached: tokio::sync::Notify::new(),
            post_check_entered: AtomicBool::new(false),
            control: Mutex::new(None),
            node,
            rollback: Arc::new(AtomicBool::new(false)),
            real_ack: AtomicBool::new(false),
            report: Mutex::new(None),
            stages: Mutex::new(Vec::new()),
            production_native: false,
            expect_tcp: false,
        }
    }
}
impl MemberPathAdapter<iroh::endpoint::ControlHandle> for ProbeAdapter {
    fn capabilities(&self, role: AttemptRole) -> RuntimeCapabilities {
        self.inner.capabilities(role)
    }
    fn authorization_revision(
        &self,
        connection: &iroh::endpoint::ControlHandle,
        purpose: SessionPurpose,
    ) -> io::Result<u64> {
        self.inner.authorization_revision(connection, purpose)
    }
    fn committed(&self, _original: &iroh::endpoint::ControlHandle, report: serde_json::Value) {
        *self.report.lock().unwrap() = Some(report);
    }
    fn active_diagnostic(
        &self,
        _original: &iroh::endpoint::ControlHandle,
        _control: &iroh::endpoint::ControlHandle,
        stage: &'static str,
    ) {
        let mut stages = self.stages.lock().unwrap();
        if stages.len() < 16 {
            stages.push(stage);
        }
        drop(stages);
        if stage == "peer-committed-observed" {
            self.peer_committed.store(true, Ordering::Release);
            self.reached.notify_one();
        }
    }
    fn check<'a>(
        &'a self,
        connection: &'a iroh::endpoint::ControlHandle,
        purpose: SessionPurpose,
    ) -> impl Future<Output = io::Result<()>> + Send + 'a {
        async move {
            if purpose == SessionPurpose::MemberBusiness
                && self.peer_committed.load(Ordering::Acquire)
                && self.fault.is_some()
                && !self.post_check_entered.swap(true, Ordering::AcqRel)
            {
                // Wait for the actual peer's successful common protocol to return
                // and close its exact C. No timer, fake frame or ACK injection.
                let control = self.control.lock().unwrap().clone().unwrap();
                control.closed().await;
                match self.fault.unwrap() {
                    Fault::OriginalClose => connection.close(0u32.into(), b"original close fault"),
                    Fault::CurrentProofInvalid => {
                        // Existing SDK fixture publishes new durable credential material,
                        // invalidating the real proof/revision. It grants no membership.
                        #[cfg(feature = "rtc-probe")]
                        self.node
                            .as_ref()
                            .unwrap()
                            .lock()
                            .await
                            .as_mut()
                            .unwrap()
                            .invalidate_current_credential_fixture()
                            .await
                            .map_err(io::Error::other)?;
                        #[cfg(not(feature = "rtc-probe"))]
                        return Err(ControlFailure::InvalidControl.into_io());
                    }
                    _ => {}
                }
            }
            self.inner.check(connection, purpose).await
        }
    }
    fn prepare<'a>(
        &'a self,
        endpoint: Option<&'a Endpoint>,
        connection: &'a iroh::endpoint::ControlHandle,
        attempt: ProtectedAttempt,
        plan: PathPlan,
    ) -> impl Future<Output = io::Result<PreparedPath>> + Send + 'a {
        self.inner.prepare(endpoint, connection, attempt, plan)
    }
    fn prepare_active<'a>(
        &'a self,
        _endpoint: Option<&'a Endpoint>,
        original: &'a iroh::endpoint::ControlHandle,
        control: &'a iroh::endpoint::ControlHandle,
        _attempt: ProtectedAttempt,
        plan: PathPlan,
    ) -> impl Future<Output = io::Result<PreparedPath>> + Send + 'a {
        async move {
            assert!(matches!(plan, PathPlan::Native { udp: true, .. }));
            *self.control.lock().unwrap() = Some(control.weak_handle());
            if self.production_native {
                let mut candidate = self
                    .inner
                    .prepare_active(_endpoint, original, control, _attempt, plan)
                    .await?;
                candidate.check(original, true)?;
                if self.expect_tcp {
                    assert!(matches!(&candidate.route, TransportAddr::Custom(addr)
                        if addr.id() == crate::tcp_transport::DATA_TRANSPORT_ID));
                    assert!(
                        original
                            .paths()
                            .iter()
                            .any(|path| path.remote_addr() == &candidate.route
                                && !path.is_selected())
                    );
                }
                self.real_ack.store(true, Ordering::Release);
                let rollback = self.rollback.clone();
                let native_rollback = candidate.rollback.take();
                candidate.rollback = Some(Box::new(move || {
                    if let Some(native_rollback) = native_rollback {
                        native_rollback();
                    }
                    rollback.store(true, Ordering::Release);
                }));
                if self.fault == Some(Fault::PreCommitControlClose) {
                    control.close(0u32.into(), b"pre-commit control close fault");
                }
                return Ok(candidate);
            }
            let paths = original.paths();
            let path = paths
                .iter()
                .find(|path| path.is_ip() && path.is_selected())
                .ok_or_else(|| ControlFailure::DataPathUnavailable.into_io())?;
            let route = path.remote_addr().clone();
            let path_id = path.id().to_string();
            let probe = path.start_probe()?;
            let qualification = self
                .gate
                .path_qualification(original)
                .ok_or_else(|| ControlFailure::IdentityOrAdmissionRejected.into_io())?;
            while !probe.is_acknowledged().map_err(io::Error::other)? {
                self.inner
                    .check(original, SessionPurpose::MemberBusiness)
                    .await?;
                if n0_future::time::Instant::now() >= control.caller_deadline().unwrap() {
                    return Err(ControlFailure::DeadlineExceeded.into_io());
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            self.real_ack.store(true, Ordering::Release);
            let rollback = self.rollback.clone();
            // This path is already Available from the actual ordinary SDK dial.
            // No selector/retain emulation: exercise common commit ownership with
            // a genuine current connection-bound probe and a rollback observer.
            let candidate = PreparedPath::candidate(original, route, move || {
                if qualification.is_current() {
                    Ok(())
                } else {
                    Err(ControlFailure::IdentityOrAdmissionRejected.into_io())
                }
            })
            .with_proof(path_id, probe.watermark(), move || {
                probe.is_acknowledged().map_err(io::Error::other)
            })
            .with_rollback(move || {
                rollback.store(true, Ordering::Release);
            });
            if self.fault == Some(Fault::PreCommitControlClose) {
                control.close(0u32.into(), b"pre-commit control close fault");
            }
            Ok(candidate)
        }
    }
}

async fn ordinary_pair(
    a: &crate::resources::ResourceEndpoint,
    b: &crate::resources::ResourceEndpoint,
) -> (Connection, Connection) {
    member_pair(a, b, b.addr()).await
}

async fn member_pair(
    a: &crate::resources::ResourceEndpoint,
    b: &crate::resources::ResourceEndpoint,
    address: EndpointAddr,
) -> (Connection, Connection) {
    let deadline = n0_future::time::Instant::now() + Duration::from_secs(5);
    let attempt = AttemptContext::new(deadline, tokio_util::sync::CancellationToken::new());
    let (ac, bc) = (a.coordinator(), b.coordinator());
    let (client, server) = tokio::join!(
        ac.connect(address, SessionPurpose::MemberBusiness, attempt.clone()),
        async {
            bc.accept(
                b.endpoint().accept().await.unwrap(),
                SessionPurpose::MemberBusiness,
                attempt,
            )
            .await
        }
    );
    (client.unwrap(), server.unwrap())
}

#[tokio::test]
async fn foreground_dials_while_background_contact_is_busy_or_backed_off() {
    use crate::resources::ContactClass::{Background, Foreground};

    let (_directory, a, b) = admitted_pair().await;
    let ae = a
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let be = b
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let id = be.addr().id;
    let (_, previous) = ae.begin_contact_attempt(id, Foreground).unwrap();
    previous.failed("foreground failed".into());
    drop(previous);
    assert!(ae.connect(be.addr(), MEMBER_ALPN).await.is_err());

    let attempt = AttemptContext::new(
        n0_future::time::Instant::now() + Duration::from_secs(5),
        tokio_util::sync::CancellationToken::new(),
    );
    let responder = be.coordinator();
    let accept_member = || async {
        responder
            .accept(
                be.endpoint().accept().await.unwrap(),
                SessionPurpose::MemberBusiness,
                attempt.clone(),
            )
            .await
            .unwrap()
    };
    let (background, incoming) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(
            ae.connect_for_recovery(be.addr(), MEMBER_ALPN, Background),
            accept_member()
        )
    })
    .await
    .unwrap();
    let (background_member, background) = background.unwrap();
    background.succeeded(); // Clear the earlier foreground failure, preserving this guard.
    let (first, duplicate, inbound) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(
            ae.connect(be.addr(), MEMBER_ALPN),
            ae.connect(be.addr(), MEMBER_ALPN),
            accept_member()
        )
    })
    .await
    .unwrap();
    assert!(first.is_ok() && duplicate.is_err());
    assert!(ae.begin_contact_attempt(id, Background).is_err());
    background.failed("later background failure".into());
    drop(background);
    assert!(!ae.contact_ready(id, Background));

    let (foreground_control, inbound_control) =
        tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(
                ae.connect_control(be.addr(), ADMISSION_REQUEST_ALPN),
                async {
                    let control = be.endpoint().accept().await.unwrap();
                    responder
                        .accept_control(control, SessionPurpose::AdmissionRequest, attempt)
                        .await
                }
            )
        })
        .await
        .unwrap();
    foreground_control
        .unwrap()
        .close(0u32.into(), b"foreground control complete");
    inbound_control
        .unwrap()
        .close(0u32.into(), b"foreground control complete");
    for connection in [
        first.unwrap(),
        inbound,
        background_member.business().unwrap(),
        incoming,
    ] {
        connection.close(0u32.into(), b"contact class fixture complete");
    }
    tokio::join!(ae.close(), be.close());
    tokio::join!(a.close(), b.close());
}

async fn production_native_active_scenario(udp_enabled: bool, reverse: bool) {
    use crate::connection_recovery::ConnectionPathRecoveryError;
    use crate::resources::ContactClass;
    use crate::session_maintenance::{Completion, MaintenanceDeferral, Reservation};
    let (_directory, a, b) = admitted_pair().await;
    let ae = Arc::new(
        a.bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap(),
    );
    let be = Arc::new(
        b.bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap(),
    );
    let (original, remote) = ordinary_pair(&ae, &be).await;
    let (a, b, ae, be, original, remote) = if reverse {
        (b, a, be, ae, remote, original)
    } else {
        (a, b, ae, be, original, remote)
    };
    let stable = original.stable_id();
    let binding = crate::connection_recovery_core::binding(&original).unwrap();
    let (mut send, mut recv) = original.open_bi().await.unwrap();
    send.write_all(b"before").await.unwrap();
    let (mut peer_send, mut peer_recv) = remote.accept_bi().await.unwrap();
    let mut before = [0; 6];
    peer_recv.read_exact(&mut before).await.unwrap();
    assert_eq!(&before, b"before");

    // Match a real application: it keeps accepting new business streams while
    // maintenance negotiates. Native control bytes must never reach this loop.
    let application_connection = remote.clone();
    let application = tokio::spawn(async move {
        while let Ok((mut send, mut recv)) = application_connection.accept_bi().await {
            let Ok(bytes) = recv.read_to_end(8).await else {
                application_connection.close(0u32.into(), b"control reached business handler");
                break;
            };
            if bytes.len() != 8 {
                break;
            }
            if send.write_all(&bytes).await.is_err() {
                break;
            }
            let _ = send.finish();
        }
    });

    let handler = b.active_improvement_handler(be.clone());
    let router = iroh::protocol::RestrictedRouter::builder(be.endpoint().clone())
        .accept_control(ACTIVE_IMPROVEMENT_ALPN, handler)
        .spawn();
    a.udp_connector().set_udp_enabled(udp_enabled);
    b.udp_connector().set_udp_enabled(udp_enabled);
    // The live node's real enhanced-round ledger is already occupied. The
    // ordinary UDP route on this M must still reach both commit barriers.
    let held_udp = if udp_enabled {
        let observation = crate::control_session::ControlSession::from_control(
            ae.endpoint().control_handle(&original),
        );
        let snapshot = a
            .control_udp()
            .observe(&observation, &a.admission_gate())
            .await
            .unwrap();
        assert!(snapshot.is_current());
        Some(snapshot)
    } else {
        None
    };
    let coordinator = ae.active_coordinator(a.admission_gate());
    let local_registry = a.admission_gate().maintenance();
    let peer_registry = b.admission_gate().maintenance();
    let (deadline, quota) = ae
        .begin_contact_attempt(original.remote_id(), ContactClass::Background)
        .unwrap();
    let attempt = AttemptContext::new(
        deadline.min(n0_future::time::Instant::now() + Duration::from_secs(5)),
        tokio_util::sync::CancellationToken::new(),
    );
    let mut lease = local_registry.acquire(&original, &attempt, ()).unwrap();
    if udp_enabled {
        let peer_lease = peer_registry.acquire(&remote, &attempt, ()).unwrap();
        let paths: Vec<_> = remote.paths().iter().map(|path| path.id()).collect();
        let error = lease
            .run_ref(coordinator.improve(
                ae.endpoint(),
                &original,
                be.addr(),
                attempt.clone(),
                || Ok(()),
            ))
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(
            MaintenanceDeferral::from_error(&error),
            Some(&MaintenanceDeferral::Remote)
        );
        assert_eq!(
            remote
                .paths()
                .iter()
                .map(|path| path.id())
                .collect::<Vec<_>>(),
            paths
        );
        assert!(original.close_reason().is_none() && remote.close_reason().is_none());
        #[cfg(feature = "rtc-probe")]
        {
            let control = ae
                .endpoint()
                .connect_control_with_opts(
                    be.addr(),
                    ACTIVE_IMPROVEMENT_ALPN,
                    iroh::endpoint::ConnectOptions::new().with_caller_deadline(attempt.deadline),
                )
                .await
                .unwrap()
                .await
                .unwrap();
            let valid_caps =
                NativeControlAdapter(coordinator.clone()).capabilities(AttemptRole::Initiator);
            let refused =
                wrong_active_binding_fixture(&original, &control, valid_caps, attempt.deadline)
                    .await
                    .unwrap()
                    .unwrap_err();
            assert!(
                MaintenanceDeferral::from_error(&active_control_error(&control, refused)).is_none()
            );
            assert!(
                matches!(control.closed().await, iroh::endpoint::ConnectionError::ApplicationClosed(close) if close.error_code == 0u32.into())
            );
        }
        drop(peer_lease);
    }
    let (start, started) = tokio::sync::oneshot::channel();
    let improving = async {
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            started.await.unwrap();
            lease
                .run_ref(coordinator.improve(
                    ae.endpoint(),
                    &original,
                    be.addr(),
                    attempt.clone(),
                    || Ok(()),
                ))
                .await
        })
        .await;
        if let Ok(Ok(Ok(path))) = &result {
            quota.succeeded();
            lease.completed(Completion::Active(path.clone()));
        }
        drop(lease);
        result
    };
    let repairs = async {
        if !udp_enabled {
            start.send(()).unwrap();
            return;
        }
        let expired = coordinator
            .repair(
                &ae,
                &original,
                be.addr(),
                n0_future::time::Instant::now() + Duration::from_millis(30),
                tokio_util::sync::CancellationToken::new(),
            )
            .await;
        assert!(
            matches!(expired, Err(ConnectionPathRecoveryError::Retryable { cause, .. }) if cause.contains("DeadlineExceeded"))
        );
        let cancel = tokio_util::sync::CancellationToken::new();
        let cancelled =
            coordinator.repair(&ae, &original, be.addr(), attempt.deadline, cancel.clone());
        futures_util::pin_mut!(cancelled);
        assert!(futures_util::poll!(cancelled.as_mut()).is_pending());
        cancel.cancel();
        assert!(
            matches!(cancelled.await, Err(ConnectionPathRecoveryError::Retryable { cause, .. }) if cause.contains("Cancelled"))
        );
        assert!(original.close_reason().is_none());
        assert_eq!(local_registry.active_attempts(), 1);
        // A second C could not start while the foreground guard is occupied.
        let _foreground = ae
            .begin_contact_attempt(original.remote_id(), ContactClass::Foreground)
            .unwrap();
        let repaired = coordinator.repair(
            &ae,
            &original,
            be.addr(),
            attempt.deadline,
            tokio_util::sync::CancellationToken::new(),
        );
        futures_util::pin_mut!(repaired);
        assert!(futures_util::poll!(repaired.as_mut()).is_pending());
        start.send(()).unwrap();
        repaired.await.unwrap();
    };
    let peer_completion = async {
        if !udp_enabled {
            return;
        }
        let wait = n0_future::time::timeout(Duration::from_secs(3), async {
            loop {
                if peer_registry.active_attempts() != 0
                    && let Reservation::Pending(wait) = peer_registry
                        .acquire_or_wait(&remote, &attempt, ())
                        .unwrap()
                {
                    break wait;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            matches!(wait.wait().await, Ok(Some(Completion::Active(path))) if path.is_open(&remote))
        );
    };
    let business = tokio::time::timeout(Duration::from_secs(3), async {
        for _ in 0..8 {
            let (mut send, mut recv) = original.open_bi().await?;
            send.write_all(b"business").await?;
            send.finish()?;
            let bytes = recv.read_to_end(8).await.map_err(io::Error::other)?;
            if bytes != b"business" {
                return Err(io::Error::other("unexpected business reply"));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok::<_, io::Error>(())
    });
    let (result, business, (), ()) = tokio::join!(improving, business, repairs, peer_completion);
    let same_original = original.stable_id() == stable
        && crate::connection_recovery_core::binding(&original)
            .is_ok_and(|actual| actual == binding);
    let continued = tokio::time::timeout(Duration::from_secs(2), async {
        send.write_all(b"after").await?;
        let mut after = [0; 5];
        peer_recv
            .read_exact(&mut after)
            .await
            .map_err(io::Error::other)?;
        peer_send.write_all(&after).await?;
        recv.read_exact(&mut after)
            .await
            .map_err(io::Error::other)?;
        Ok::<_, io::Error>(after == *b"after")
    })
    .await;
    original.close(0u32.into(), b"native active regression complete");
    remote.close(0u32.into(), b"native active regression complete");
    application.await.unwrap();
    drop((
        send,
        recv,
        peer_send,
        peer_recv,
        original,
        remote,
        coordinator,
    ));
    router.shutdown().await.unwrap();
    drop(router);
    let ae = Arc::try_unwrap(ae)
        .ok()
        .expect("initiator resource owner released");
    let be = Arc::try_unwrap(be)
        .ok()
        .expect("responder handler released");
    tokio::join!(ae.close(), be.close());
    tokio::join!(a.close(), b.close());
    if udp_enabled {
        assert!(
            matches!(result, Ok(Ok(Ok(_)))),
            "production native active: {result:?}"
        );
        assert!(held_udp.unwrap().is_current());
    } else {
        assert!(
            matches!(result, Ok(Ok(Err(_)))),
            "disabled UDP active: {result:?}"
        );
    }
    assert!(same_original);
    assert!(
        matches!(business, Ok(Ok(()))),
        "concurrent business: {business:?}"
    );
    assert!(
        matches!(continued, Ok(Ok(true))),
        "original stream: {continued:?}"
    );
}

#[tokio::test]
async fn production_native_active_prepares_udp_on_the_original_member() {
    production_native_active_scenario(true, false).await;
}

#[tokio::test]
async fn production_native_active_respects_udp_disable_and_keeps_business() {
    production_native_active_scenario(false, false).await;
}

#[tokio::test]
async fn active_native_budget_deferral_preserves_another_members_original_stream() {
    use crate::{
        control_session::ControlSession,
        resources::{Resource, ResourceUse},
        session_maintenance::MaintenanceDeferral,
    };
    let (_directory, a, b) = admitted_pair().await;
    let mut config = iroh_relay::server::ServerConfig::default();
    config.relay = Some(iroh_relay::server::RelayConfig::new((
        Ipv4Addr::LOCALHOST,
        0,
    )));
    let relay = iroh_relay::server::Server::spawn(config).await.unwrap();
    let url = relay.http_url().unwrap();
    let relay_route = TransportAddr::Relay(url.clone());
    for node in [&a, &b] {
        let mut config = node.resource_configuration().unwrap();
        config.tcp_direct_disabled = true;
        config.replacement = Some(vec![Resource {
            url: url.to_string(),
            uses: [ResourceUse::DataRelay].into(),
            limitation: "owned relay".into(),
        }]);
        node.save_resource_configuration(&config).unwrap();
    }
    let ae = a
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let be = Arc::new(
        b.bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap(),
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(ae.endpoint().online(), be.endpoint().online());
    })
    .await
    .unwrap();
    let address = EndpointAddr::new(be.endpoint().id()).with_addrs([relay_route.clone()]);
    let (_other, held_remote) = member_pair(&ae, &be, address.clone()).await;
    let (original, remote) = member_pair(&ae, &be, address.clone()).await;
    let ids = (original.stable_id(), remote.stable_id());
    let a_original = ae.endpoint().control_handle(&original);
    let b_original = be.endpoint().control_handle(&remote);
    a_original.open_data_transport_path(relay_route.clone()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while ![&original, &remote].iter().all(|member|
            member.paths().iter().any(|path| path.is_relay() && path.stats().acknowledgment.is_some())
        ) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("the controlled relay path is usable before retiring direct paths");
    tokio::try_join!(
        a_original.test_limit_transport_paths(vec![relay_route.clone()]),
        b_original.test_limit_transport_paths(vec![relay_route]),
    )
    .unwrap();
    for member in [&original, &remote] {
        for path in member.paths().iter().filter(|path| !path.is_relay()) {
            path.close().unwrap();
        }
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        while ![&original, &remote].iter().all(|member| {
            let paths = member.paths();
            paths.iter().all(|path| path.is_relay()) && paths.iter().any(|path| path.is_selected())
        }) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("both originals are carried only by the controlled relay");
    let hold = ControlSession::from_control(be.endpoint().control_handle(&held_remote));
    let held = b
        .control_udp()
        .observe(&hold, &b.admission_gate())
        .await
        .unwrap();
    let previous_generation = be.endpoint().udp_network_generation();
    be.endpoint().network_change().await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while previous_generation.is_current() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        !held.is_current(),
        "only the outstanding owner can still block this round"
    );
    let (mut send, mut receive) = original.open_bi().await.unwrap();
    send.write_all(b"before").await.unwrap();
    let (mut reply, mut incoming) = remote.accept_bi().await.unwrap();
    let mut bytes = [0u8; 6];
    incoming.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"before");
    let router = iroh::protocol::RestrictedRouter::builder(be.endpoint().clone())
        .accept_control(
            ACTIVE_IMPROVEMENT_ALPN,
            b.active_improvement_handler(be.clone()),
        )
        .spawn();
    let deadline = n0_future::time::Instant::now() + Duration::from_secs(10);
    let control = ae
        .endpoint()
        .connect_control_with_opts(
            address,
            ACTIVE_IMPROVEMENT_ALPN,
            iroh::endpoint::ConnectOptions::new().with_caller_deadline(deadline),
        )
        .await
        .unwrap()
        .await
        .unwrap();
    let attempt = AttemptContext::new(deadline, tokio_util::sync::CancellationToken::new());
    let lease = a
        .admission_gate()
        .maintenance()
        .acquire(&original, &attempt, ())
        .unwrap();
    let error = lease
        .run(improve_existing(
            ae.endpoint(),
            &original,
            &control.control_handle(),
            &NativeControlAdapter(ae.active_coordinator(a.admission_gate())),
            attempt,
            AttemptLog::new(AttemptRole::Initiator, "active_native_budget_fixture"),
            || Ok(()),
        ))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(
        MaintenanceDeferral::from_error(&error),
        Some(&MaintenanceDeferral::Remote)
    );
    let reason = tokio::time::timeout(Duration::from_secs(2), control.control_handle().closed())
        .await
        .unwrap();
    assert!(
        matches!(reason, iroh::endpoint::ConnectionError::ApplicationClosed(close)
        if close.error_code == ACTIVE_DEFERRED_CODE.into())
    );
    assert!(!held.is_current());
    assert_eq!((original.stable_id(), remote.stable_id()), ids);
    send.write_all(b"after!").await.unwrap();
    incoming.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"after!");
    reply.write_all(&bytes).await.unwrap();
    receive.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"after!");
    drop(held);
    assert!(
        b.control_udp()
            .observe(&hold, &b.admission_gate())
            .await
            .unwrap()
            .is_current()
    );
    router.shutdown().await.unwrap();
    tokio::join!(ae.close(), be.endpoint().close());
    tokio::join!(a.close(), b.close());
}

#[tokio::test]
async fn native_active_wrong_udp_result_is_invalid_control_and_keeps_original() {
    scripted_native_udp_result(None, false).await;
}

#[tokio::test]
async fn native_active_local_budget_distinguishes_remaining_ordinary_udp_candidates() {
    for reverse in [false, true] {
        scripted_native_udp_result(Some(1), reverse).await;
        scripted_native_udp_result(Some(64), reverse).await;
    }
}

async fn scripted_native_udp_result(offer_count: Option<u16>, reverse: bool) {
    use crate::{
        control_session::ControlSession,
        resources::{Resource, ResourceUse},
        session_maintenance::MaintenanceDeferral,
    };
    let (_directory, a, b) = admitted_pair().await;
    let relay = if offer_count.is_some() {
        let mut config = iroh_relay::server::ServerConfig::default();
        config.relay = Some(iroh_relay::server::RelayConfig::new((
            Ipv4Addr::LOCALHOST,
            0,
        )));
        let relay = iroh_relay::server::Server::spawn(config).await.unwrap();
        for node in [&a, &b] {
            let mut config = node.resource_configuration().unwrap();
            config.replacement = Some(vec![Resource {
                url: relay.http_url().unwrap().to_string(),
                uses: [ResourceUse::DataRelay].into(),
                limitation: "owned relay for the direct candidate window".into(),
            }]);
            node.save_resource_configuration(&config).unwrap();
        }
        Some(relay)
    } else {
        None
    };
    let ae = a
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let be = b
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let (original, peer) = if let Some(relay) = &relay {
        let path = TransportAddr::Relay(relay.http_url().unwrap());
        tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(ae.endpoint().online(), be.endpoint().online());
        })
        .await
        .unwrap();
        let (original, peer) = member_pair(
            &ae,
            &be,
            EndpointAddr::new(be.endpoint().id()).with_addrs([path.clone()]),
        )
        .await;
        let original_path = ae.endpoint().control_handle(&original);
        let peer_path = be.endpoint().control_handle(&peer);
        tokio::try_join!(
            original_path.test_limit_transport_paths(vec![path.clone()]),
            peer_path.test_limit_transport_paths(vec![path]),
        )
        .unwrap();
        for connection in [&original, &peer] {
            for path in connection.paths().iter().filter(|path| path.is_ip()) {
                path.close().unwrap();
            }
            assert!(connection.paths().iter().any(|path| path.is_relay()));
            assert!(connection.paths().iter().all(|path| path.is_relay()));
        }
        (original, peer)
    } else {
        ordinary_pair(&ae, &be).await
    };
    let (a, b, ae, be, original, peer) = if reverse {
        (b, a, be, ae, peer, original)
    } else {
        (a, b, ae, be, original, peer)
    };
    be.endpoint().set_alpns(vec![ACTIVE_IMPROVEMENT_ALPN.to_vec()]);
    let ids = (original.stable_id(), peer.stable_id());
    let (mut send, mut receive) = original.open_bi().await.unwrap();
    send.write_all(b"before").await.unwrap();
    let (mut reply, mut incoming) = peer.accept_bi().await.unwrap();
    let mut before = [0; 6];
    incoming.read_exact(&mut before).await.unwrap();
    assert_eq!(&before, b"before");
    let _held = if offer_count.is_some() {
        let observing = ControlSession::from_control(ae.endpoint().control_handle(&original));
        Some(
            a.control_udp()
                .observe(&observing, &a.admission_gate())
                .await
                .unwrap(),
        )
    } else {
        None
    };

    let checks = if offer_count.is_some() { 2 } else { 1 };
    let mut previously_tried = 0;
    for round in 0..checks {
        let deadline = n0_future::time::Instant::now() + Duration::from_secs(5);
        let (control, remote_control) = tokio::join!(
            async {
                ae.endpoint()
                    .connect_control_with_opts(
                        be.addr(),
                        ACTIVE_IMPROVEMENT_ALPN,
                        iroh::endpoint::ConnectOptions::new().with_caller_deadline(deadline),
                    )
                    .await
                    .unwrap()
                    .await
                    .unwrap()
            },
            async {
                be.endpoint()
                    .accept()
                    .await
                    .unwrap()
                    .with_caller_deadline(deadline)
                    .accept_control()
                    .unwrap()
                    .await
                    .unwrap()
            },
        );
        let adapter = NativeControlAdapter(ae.active_coordinator(a.admission_gate()));
        let attempt = AttemptContext::new(deadline, tokio_util::sync::CancellationToken::new());
        let lease = a
            .admission_gate()
            .maintenance()
            .acquire(&original, &attempt, ())
            .unwrap();
        let scripted_peer = async {
            let stream = remote_control.accept_control_exchange().await.unwrap();
            let request = read_active_hello(&stream).await.unwrap();
            let mut caps = adapter.capabilities(AttemptRole::Responder);
            caps.permitted = UDP;
            let local = active_hello(caps, AttemptRole::Responder, request.binding);
            write_active_hello(&stream, local).await.unwrap();

            let control = ControlSession::from_active_control(
                be.endpoint().control_handle(&peer),
                remote_control.control_handle(),
            );
            let format = ControlMessageFormat::EndTerminated { maximum: 16 * 1024 };
            if reverse {
                let scope = control.nat_learning_scope(&b.admission_gate()).unwrap();
                let stream = control.open_control_exchange().await.unwrap();
                stream
                    .send_message(
                        format,
                        &serde_json::to_vec(&json!({"Hello":{
                            "network":scope.network,"generation":scope.secret_generation
                        }}))
                        .unwrap(),
                    )
                    .await
                    .unwrap();
                let offer: Value =
                    serde_json::from_slice(&stream.recv_message(format).await.unwrap()).unwrap();
                let nonce = offer["Offer"]["nonce"].clone();
                assert!(
                    offer["Offer"]["addresses"]
                        .as_array()
                        .is_some_and(|addresses| !addresses.is_empty())
                );
                let stream = control.open_control_exchange().await.unwrap();
                stream
                    .send_message(
                        format,
                        &serde_json::to_vec(&json!({"Offer":{"nonce":nonce,"addresses":[]}}))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                let ready: Value =
                    serde_json::from_slice(&stream.recv_message(format).await.unwrap()).unwrap();
                assert_eq!(ready["Ready"]["nonce"], nonce);
                let stream = control.open_control_exchange().await.unwrap();
                let mut result = json!({"Result":{"nonce":nonce,"available":false}});
                if offer_count == Some(64) {
                    result["Result"]["more_candidates"] = json!(true);
                }
                stream
                    .send_message(format, &serde_json::to_vec(&result).unwrap())
                    .await
                    .unwrap();
                let reply: Value =
                    serde_json::from_slice(&stream.recv_message(format).await.unwrap()).unwrap();
                assert_eq!(reply["Result"]["nonce"], nonce);
            } else {
                let nonce = [0u8; 16];
                let result_nonce = if offer_count.is_some() {
                    nonce
                } else {
                    [1u8; 16]
                };
                let addresses = (0..offer_count.unwrap_or(0))
                    .map(|offset| std::net::SocketAddr::from((Ipv4Addr::LOCALHOST, 47000 + offset)))
                    .collect::<Vec<_>>();
                for (expected, answer) in [
                    (
                        "Hello",
                        json!({"Offer":{"nonce":nonce,"addresses":addresses}}),
                    ),
                    ("Offer", json!({"Ready":{"nonce":nonce}})),
                    (
                        "Result",
                        json!({"Result":{"nonce":result_nonce,"available":false}}),
                    ),
                ] {
                    let stream = remote_control.accept_control_exchange().await.unwrap();
                    let input = stream.recv_message(format).await.unwrap();
                    let frame = serde_json::from_slice::<Value>(&input).unwrap();
                    assert!(frame.get(expected).is_some(), "{frame}");
                    stream
                        .send_message(format, &serde_json::to_vec(&answer).unwrap())
                        .await
                        .unwrap();
                }
            }
            if offer_count.is_some() {
                let error = crate::nat_punch::begin_round(&control, &b.admission_gate(), true)
                    .await
                    .err()
                    .expect("the unoccupied peer receives the occupied node's deferral");
                assert_eq!(
                    MaintenanceDeferral::from_error(&error),
                    Some(&MaintenanceDeferral::Remote)
                );
            }
        };
        let control_handle = control.control_handle();
        let (result, ()) = tokio::time::timeout_at(deadline, async {
            tokio::join!(
                lease.run(improve_existing(
                    ae.endpoint(),
                    &original,
                    &control_handle,
                    &adapter,
                    attempt,
                    AttemptLog::new(AttemptRole::Initiator, "test_active_udp_result"),
                    || Ok(()),
                )),
                scripted_peer
            )
        })
        .await
        .unwrap();
        let error = result.unwrap().unwrap_err();
        match offer_count {
            None => {
                assert_eq!(error.kind(), io::ErrorKind::InvalidData);
                assert!(error.to_string().starts_with("unified/InvalidControl"));
            }
            Some(64) => assert_eq!(
                MaintenanceDeferral::from_error(&error),
                Some(&MaintenanceDeferral::LocalAlternatives)
            ),
            Some(_) => assert!(
                matches!(
                    MaintenanceDeferral::from_error(&error),
                    Some(MaintenanceDeferral::LocalBudget { next_due }) if *next_due > n0_future::time::Instant::now()
                ),
                "{error:?}, reverse={reverse}, offer_count={offer_count:?}, round={round}"
            ),
        }
        if !reverse && let Some(offered) = offer_count {
            let original = ae.endpoint().control_handle(&original);
            let tried = a
                .admission_gate()
                .with_direct_candidates(&original, |progress| progress.tried_addresses())
                .unwrap();
            if offered == 1 {
                assert_eq!(tried, 1);
            } else {
                // The real PathID limit caps each window. It never constitutes
                // evidence that the remaining addresses were actually tried.
                assert!(previously_tried < tried && tried < usize::from(offered));
            }
            println!(
                "OBSERVED direct_offer_total={offered} cumulative_direct_launches={tried} window={round}"
            );
            previously_tried = tried;
        }
        control.close(0u32.into(), b"active UDP result fixture finished");
        remote_control.close(0u32.into(), b"active UDP result fixture finished");
    }
    assert_eq!((original.stable_id(), peer.stable_id()), ids);
    assert!(original.close_reason().is_none() && peer.close_reason().is_none());
    send.write_all(b"later!").await.unwrap();
    incoming.read_exact(&mut before).await.unwrap();
    assert_eq!(&before, b"later!");
    reply.write_all(&before).await.unwrap();
    receive.read_exact(&mut before).await.unwrap();
    assert_eq!(&before, b"later!");
    original.close(0u32.into(), b"active UDP result fixture finished");
    peer.close(0u32.into(), b"active UDP result fixture finished");
    tokio::join!(ae.close(), be.close());
    tokio::join!(a.close(), b.close());
    if let Some(relay) = relay {
        relay.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn unmarked_active_control_close_is_not_deferred() {
    let (_directory, a, b) = admitted_pair().await;
    let ae = a
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let be = b
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let (original, remote) = ordinary_pair(&ae, &be).await;
    be.endpoint()
        .set_alpns(vec![ACTIVE_IMPROVEMENT_ALPN.to_vec()]);
    let attempt = AttemptContext::new(
        n0_future::time::Instant::now() + Duration::from_secs(2),
        tokio_util::sync::CancellationToken::new(),
    );
    let coordinator = ae.active_coordinator(a.admission_gate());
    let refuse = async {
        let control = be
            .endpoint()
            .accept()
            .await
            .unwrap()
            .accept_control()
            .unwrap()
            .await
            .unwrap();
        let stream = control.accept_control_exchange().await.unwrap();
        read_active_hello(&stream).await.unwrap();
        control.close(0u32.into(), b"end without active result");
    };
    let (result, ()) = tokio::time::timeout_at(attempt.deadline, async {
        tokio::join!(
            coordinator.repair(
                &ae,
                &original,
                be.addr(),
                attempt.deadline,
                attempt.cancellation
            ),
            refuse
        )
    })
    .await
    .unwrap();
    assert!(
        matches!(result, Err(crate::connection_recovery::ConnectionPathRecoveryError::Retryable { cause, .. }) if !cause.contains("deferred"))
    );
    assert!(ae.contact_ready(
        original.remote_id(),
        crate::resources::ContactClass::Foreground
    ));
    assert!(original.close_reason().is_none() && remote.close_reason().is_none());
    original.close(0u32.into(), b"fixture ended");
    remote.close(0u32.into(), b"fixture ended");
    tokio::join!(ae.close(), be.close());
    tokio::join!(a.close(), b.close());
}

#[tokio::test]
async fn production_native_active_from_accepting_side_keeps_concurrent_business() {
    production_native_active_scenario(true, true).await;
}

#[tokio::test]
async fn background_relay_race_admits_backup_and_sends_one_hello() {
    use iroh::endpoint::{BeforeConnectOutcome, EndpointHooks, presets};

    #[derive(Debug, Clone)]
    struct StalledRoute {
        url: iroh::RelayUrl,
        reject: Arc<AtomicBool>,
        started: tokio::sync::mpsc::UnboundedSender<(iroh::RelayUrl, n0_future::time::Instant)>,
    }
    impl EndpointHooks for StalledRoute {
        async fn before_connect(&self, target: &EndpointAddr, alpn: &[u8]) -> BeforeConnectOutcome {
            if alpn == ACTIVE_IMPROVEMENT_ALPN {
                let url = target.relay_urls().next().unwrap();
                self.started
                    .send((url.clone(), n0_future::time::Instant::now()))
                    .unwrap();
                if url == &self.url {
                    if self.reject.load(Ordering::Acquire) {
                        return BeforeConnectOutcome::Reject;
                    }
                    return std::future::pending().await;
                }
            }
            BeforeConnectOutcome::Accept
        }
    }

    let (_directory, a, b) = admitted_pair().await;
    let mut config = iroh_relay::server::ServerConfig::default();
    config.relay = Some(iroh_relay::server::RelayConfig::new((
        Ipv4Addr::LOCALHOST,
        0,
    )));
    let relay = iroh_relay::server::Server::spawn(config).await.unwrap();
    let live = relay.http_url().unwrap();
    let stalled = format!("http://127.0.0.2:{}/", live.port().unwrap())
        .parse::<iroh::RelayUrl>()
        .unwrap();
    for node in [&a, &b] {
        let mut resources = node.resource_configuration().unwrap();
        resources.replacement = Some(
            [&live, &stalled]
                .into_iter()
                .map(|url| crate::resources::Resource {
                    url: url.to_string(),
                    uses: [crate::resources::ResourceUse::DataRelay].into(),
                    limitation: "owned active relay race".into(),
                })
                .collect(),
        );
        node.save_resource_configuration(&resources).unwrap();
    }
    let (started, mut starts) = tokio::sync::mpsc::unbounded_channel();
    let reject = Arc::new(AtomicBool::new(false));
    let ae = a
        .bind_configured_endpoint_with(
            Endpoint::builder(presets::Minimal)
                .bind_addr((Ipv4Addr::LOCALHOST, 0))
                .unwrap()
                .hooks(StalledRoute {
                    url: stalled.clone(),
                    reject: reject.clone(),
                    started,
                }),
        )
        .await
        .unwrap();
    let be = b
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(ae.endpoint().online(), be.endpoint().online());
    })
    .await
    .unwrap();
    let direct = be
        .endpoint()
        .bound_sockets()
        .into_iter()
        .find(std::net::SocketAddr::is_ipv4)
        .unwrap();
    let (original, remote) = member_pair(
        &ae,
        &be,
        EndpointAddr::new(be.addr().id).with_ip_addr(direct),
    )
    .await;
    let stable = original.stable_id();
    let (mut tx, mut rx) = original.open_bi().await.unwrap();
    tx.write_all(b"first").await.unwrap();
    let (mut echo, mut incoming) = remote.accept_bi().await.unwrap();
    let mut bytes = [0; 5];
    incoming.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"first");

    let coordinator = ConnectionCoordinator::new_restricted(ae.endpoint().clone(), RELAY, vec![])
        .with_background_relay_race(Some(BackgroundRelayRace {
            preferred: Some(TransportAddr::Relay(stalled.clone())),
        }));
    let live_target = EndpointAddr::new(be.addr().id).with_relay_url(live.clone());
    let target = live_target.clone().with_relay_url(stalled.clone());
    let attempt = || {
        AttemptContext::new(
            n0_future::time::Instant::now() + Duration::from_secs(5),
            tokio_util::sync::CancellationToken::new(),
        )
    };
    let log = AttemptLog::new(AttemptRole::Initiator, "background_relay_race_fixture");
    reject.store(true, Ordering::Release);
    assert!(matches!(
        coordinator
            .connect_active_control(target.clone(), attempt(), log)
            .await,
        Err(ConnectError::IdentityOrAdmissionRejected)
    ));
    assert_eq!(starts.try_recv().unwrap().0, stalled);
    assert!(starts.try_recv().is_err());
    reject.store(false, Ordering::Release);

    let (incoming_tx, mut controls) = tokio::sync::mpsc::unbounded_channel();
    let router = iroh::protocol::RestrictedRouter::builder(be.endpoint().clone())
        .accept_control(ACTIVE_IMPROVEMENT_ALPN, ObserveControl(incoming_tx))
        .spawn();
    let dialing = attempt();
    let control = coordinator
        .connect_active_control(target, dialing.clone(), log)
        .await
        .unwrap();
    let (first, first_at) = starts.try_recv().unwrap();
    let (second, second_at) = starts.try_recv().unwrap();
    assert_eq!((first, second), (stalled, live.clone()));
    assert!(second_at - first_at >= Duration::from_millis(240));
    assert!(
        control
            .paths()
            .iter()
            .any(|path| matches!(path.remote_addr(), TransportAddr::Relay(url) if url == &live))
    );
    let received = tokio::time::timeout(Duration::from_secs(2), controls.recv())
        .await
        .unwrap()
        .unwrap();
    let client = ProbeAdapter::new(a.admission_gate(), None, None);
    let server = ProbeAdapter::new(b.admission_gate(), None, None);
    let local_registry = client.gate.maintenance();
    let registry = server.gate.maintenance();
    let lease = local_registry
        .acquire_outgoing(&original, &dialing, ())
        .unwrap();
    let winning = control.restricted_handle().unwrap();
    let receiving = received.control_handle();
    let deadline = dialing.deadline;
    let (path, peer) = tokio::time::timeout_at(deadline, async {
        tokio::join!(
            lease.run_ref(improve_existing(
                ae.endpoint(),
                &original,
                &winning,
                &client,
                dialing,
                log,
                || lease.seal_outgoing_hello()
            )),
            serve_active_session(
                be.endpoint(),
                &receiving,
                &server,
                &registry,
                deadline,
                |peer, binding| registry.for_binding(peer, binding)
            ),
        )
    })
    .await
    .unwrap();
    assert!(path.unwrap().unwrap().is_open(&original));
    peer.unwrap();
    assert!(
        controls.try_recv().is_err(),
        "only the admitted winner reaches the peer control handler"
    );
    assert!(starts.try_recv().is_err());
    tx.write_all(b"after").await.unwrap();
    incoming.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"after");
    echo.write_all(b"reply").await.unwrap();
    rx.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"reply");
    assert_eq!(original.stable_id(), stable);
    assert!(remote.close_reason().is_none());
    drop(lease);

    // The same race owns both ready results. Qualify two actual controls first
    // so testing the tie doesn't depend on transport scheduling.
    let (first, second) = (attempt(), attempt());
    let (preferred, alternate) = tokio::join!(
        coordinator.connect_active_control(live_target.clone(), first.clone(), log),
        coordinator.connect_active_control(live_target, second.clone(), log),
    );
    let preferred = preferred.unwrap();
    let alternate = alternate.unwrap();
    let preferred_id = preferred.stable_id();
    let alternate_binding =
        crate::connection_recovery_core::binding(&alternate.restricted_handle().unwrap()).unwrap();
    let raced = race_contact_routes(
        (
            &first,
            std::future::ready(Ok(QualifiedRoute {
                delivery: ConnectionDelivery::Control(preferred),
                log,
            })),
        ),
        (
            &second,
            std::future::ready(Ok(QualifiedRoute {
                delivery: ConnectionDelivery::Control(alternate),
                log,
            })),
        ),
    )
    .await
    .map_err(|failure| failure.error().to_string())
    .unwrap();
    assert_eq!(raced.delivery.stable_id(), preferred_id);
    let (left, right) = (
        controls.recv().await.unwrap(),
        controls.recv().await.unwrap(),
    );
    let (winner, loser) = if crate::connection_recovery_core::binding(&left.control_handle())
        .unwrap()
        == alternate_binding
    {
        (right, left)
    } else {
        assert_eq!(
            crate::connection_recovery_core::binding(&right.control_handle()).unwrap(),
            alternate_binding
        );
        (left, right)
    };
    assert!(
        tokio::time::timeout(Duration::from_secs(2), loser.accept_control_exchange())
            .await
            .unwrap()
            .is_err(),
        "the qualified losing control is closed before Hello"
    );
    let winner = winner.control_handle();
    let request = raced.delivery.control().unwrap();
    let request = request.restricted_handle().unwrap();
    let lease = local_registry
        .acquire_outgoing(&original, &first, ())
        .unwrap();
    let deadline = first.deadline;
    let (path, peer) = tokio::time::timeout_at(deadline, async {
        tokio::join!(
            lease.run_ref(improve_existing(
                ae.endpoint(),
                &original,
                &request,
                &client,
                first,
                log,
                || lease.seal_outgoing_hello()
            )),
            serve_active_session(
                be.endpoint(),
                &winner,
                &server,
                &registry,
                deadline,
                |peer, binding| registry.for_binding(peer, binding)
            ),
        )
    })
    .await
    .unwrap();
    assert!(path.unwrap().unwrap().is_open(&original));
    peer.unwrap();
    tx.write_all(b"later").await.unwrap();
    incoming.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"later");
    drop(lease);
    router.shutdown().await.unwrap();
    tokio::join!(ae.close(), be.close());
    tokio::join!(a.close(), b.close());
    relay.shutdown().await.unwrap();
}

#[tokio::test]
async fn colliding_active_hello_hands_off_to_the_original_dialer() {
    use crate::session_maintenance::MaintenanceError;

    let (_directory, a, b) = admitted_pair().await;
    let ae = a
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let be = b
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let (original, remote) = ordinary_pair(&ae, &be).await;
    for endpoint in [&ae, &be] {
        endpoint.endpoint().set_alpns(vec![ACTIVE_IMPROVEMENT_ALPN.to_vec()]);
    }
    let ids = (original.stable_id(), remote.stable_id());
    assert!(original.side().is_client() && !remote.side().is_client());
    let (mut send, mut reply) = original.open_bi().await.unwrap();
    send.write_all(b"before").await.unwrap();
    let (mut echo, mut receive) = remote.accept_bi().await.unwrap();
    let mut bytes = [0; 6];
    receive.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"before");

    let deadline = n0_future::time::Instant::now() + Duration::from_secs(5);
    let dial = |from: &crate::resources::ResourceEndpoint,
                to: &crate::resources::ResourceEndpoint| {
        let (from, to, address) = (from.endpoint().clone(), to.endpoint().clone(), to.addr());
        async move {
            let options = iroh::endpoint::ConnectOptions::new().with_caller_deadline(deadline);
            tokio::join!(
                async {
                    from.connect_control_with_opts(address, ACTIVE_IMPROVEMENT_ALPN, options)
                        .await
                        .unwrap()
                        .await
                        .unwrap()
                },
                async {
                    to.accept()
                        .await
                        .unwrap()
                        .with_caller_deadline(deadline)
                        .accept_control()
                        .unwrap()
                        .await
                        .unwrap()
                }
            )
        }
    };
    let (winning, winning_incoming) = dial(&ae, &be).await;
    let (losing, losing_incoming) = dial(&be, &ae).await;
    let (winning, winning_incoming) = (winning.control_handle(), winning_incoming.control_handle());
    let (losing, losing_incoming) = (losing.control_handle(), losing_incoming.control_handle());
    let client = ProbeAdapter::new(a.admission_gate(), None, None);
    let server = ProbeAdapter::new(b.admission_gate(), None, None);
    let client_registry = client.gate.maintenance();
    let server_registry = server.gate.maintenance();
    let attempt = || AttemptContext::new(deadline, tokio_util::sync::CancellationToken::new());
    let (client_attempt, server_attempt) = (attempt(), attempt());
    let client_lease = client_registry
        .acquire_outgoing(&original, &client_attempt, ())
        .unwrap();
    let server_lease = server_registry
        .acquire_outgoing(&remote, &server_attempt, ())
        .unwrap();
    let server_outgoing = async {
        let result = server_lease
            .run(improve_existing(
                be.endpoint(),
                &remote,
                &losing,
                &server,
                server_attempt,
                AttemptLog::new(AttemptRole::Initiator, "colliding_hello_acceptor"),
                // Reaching this closure on the losing control would mean it received a reply.
                || panic!("losing Hello entered candidate validation"),
            ))
            .await;
        losing.close(0u32.into(), b"outgoing attempt yielded");
        result
    };
    let serve_winner = async {
        // Consume the losing request here without sending a reply. This forces
        // the original acceptor to be waiting for Hello when the winner arrives.
        let exchange = open_active_stream(&losing_incoming).await.unwrap();
        let hello = read_active_hello(&exchange).await.unwrap();
        assert_eq!(
            hello.binding,
            crate::connection_recovery_core::binding(&original).unwrap()
        );
        echo.write_all(b"during").await.unwrap();
        reply.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"during");
        serve_active_session(
            be.endpoint(),
            &winning_incoming,
            &server,
            &server_registry,
            deadline,
            |peer, binding| server_registry.for_binding(peer, binding),
        )
        .await
    };
    let client_outgoing = client_lease.run_ref(improve_existing(
        ae.endpoint(),
        &original,
        &winning,
        &client,
        client_attempt,
        AttemptLog::new(AttemptRole::Initiator, "colliding_hello_dialer"),
        || client_lease.seal_outgoing_hello(),
    ));
    let (won, served, yielded) = tokio::time::timeout_at(deadline, async {
        tokio::join!(client_outgoing, serve_winner, server_outgoing)
    })
    .await
    .unwrap();
    assert!(won.unwrap().unwrap().is_open(&original));
    served.unwrap();
    assert!(matches!(yielded, Err(MaintenanceError::Cancelled)));
    for (adapter, control) in [(&client, &winning), (&server, &winning_incoming)] {
        assert_eq!(
            adapter.report.lock().unwrap().as_ref().unwrap()["controlStableId"],
            json!(control.stable_id())
        );
        assert_eq!(
            adapter
                .stages
                .lock()
                .unwrap()
                .iter()
                .filter(|stage| **stage == "common-ready-begin")
                .count(),
            1
        );
    }
    send.write_all(b"after!").await.unwrap();
    receive.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"after!");
    assert_eq!((original.stable_id(), remote.stable_id()), ids);
    assert!(original.close_reason().is_none() && remote.close_reason().is_none());
    drop(client_lease);
    tokio::join!(ae.close(), be.close());
    tokio::join!(a.close(), b.close());
}

#[tokio::test]
async fn qualified_original_can_own_traversal_but_ordinary_connection_cannot() {
    let (_directory, a, b) = admitted_pair().await;
    let ae = a
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let be = b
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let (original, remote) = ordinary_pair(&ae, &be).await;
    let mut traversal = ae
        .endpoint()
        .control_handle(&original)
        .register_udp_traversal([61; 16])
        .await;
    let mut retained = None;
    if let Ok(lease) = &mut traversal {
        lease.listen_random_ports(1).await.unwrap();
        let local = lease
            .local_sockets()
            .into_iter()
            .find(|socket| !socket.id.is_primary())
            .unwrap();
        retained = Some(
            lease
                .retain_winner(
                    local.id,
                    std::net::SocketAddr::from((Ipv4Addr::LOCALHOST, local.local_addr.port())),
                )
                .await,
        );
    }
    let retained_ok = matches!(&retained, Some(Ok(Some(_))));
    if let Some(Ok(Some(winner))) = &mut retained {
        assert!(
            winner.commit(&remote).is_err(),
            "winner belongs only to its original M"
        );
    }

    let builder = || {
        iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
            .clear_relay_transports()
            .bind_addr(std::net::SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .unwrap()
            .alpns(vec![b"ordinary-traversal-boundary".to_vec()])
    };
    let plain_a = builder().bind().await.unwrap();
    let plain_b = builder().bind().await.unwrap();
    let (plain, peer) = tokio::join!(
        plain_a.connect(plain_b.addr(), b"ordinary-traversal-boundary"),
        async { plain_b.accept().await.unwrap().await },
    );
    let (plain, peer) = (plain.unwrap(), peer.unwrap());
    let denied = plain_a
        .register_udp_traversal(&plain, [62; 16])
        .await
        .is_err();
    plain.close(0u32.into(), b"boundary regression done");
    peer.close(0u32.into(), b"boundary regression done");
    original.close(0u32.into(), b"boundary regression done");
    remote.close(0u32.into(), b"boundary regression done");
    drop((retained, plain, peer, original, remote));
    let allowed = traversal.is_ok();
    drop(traversal);
    tokio::join!(plain_a.close(), plain_b.close(), ae.close(), be.close());
    tokio::join!(a.close(), b.close());
    assert!(
        denied,
        "ordinary unqualified endpoints cannot gain traversal authority"
    );
    assert!(
        allowed && retained_ok,
        "qualified M needs connection-owned traversal and winner retention"
    );
}

async fn scenario(fault: Fault) {
    scenario_with_native(fault, false, None).await;
}

async fn scenario_with_native(
    fault: Fault,
    production_native: bool,
    tcp_observer: Option<std::net::SocketAddr>,
) {
    let (_directory, a, b) = admitted_pair().await;
    if let Some(observer) = tcp_observer {
        for node in [&a, &b] {
            let mut config = node.resource_configuration().unwrap();
            config.replacement = Some(vec![crate::resources::Resource {
                url: format!("stun+tcp://{observer}/"),
                uses: [crate::resources::ResourceUse::Observation].into(),
                limitation: "owned TCP Binding observer".into(),
            }]);
            config.tcp_direct_disabled = false;
            config.tcp_listen = Some((Ipv4Addr::LOCALHOST, 0).into());
            config.path_selection.preference = crate::path_selection::PathPreference::PreferDirect;
            node.save_resource_configuration(&config).unwrap();
        }
    }
    let ae = a
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let be = b
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let contact = if tcp_observer.is_some() {
        EndpointAddr::new(be.endpoint().id()).with_ip_addr(be.endpoint().bound_sockets()[0])
    } else {
        be.addr()
    };
    let (original, remote) = member_pair(&ae, &be, contact.clone()).await;
    let (other, other_remote) = member_pair(&ae, &be, contact).await;
    assert!(a.admission_gate().allows(&original, ProtocolScope::Member));
    assert!(b.admission_gate().allows(&remote, ProtocolScope::Member));
    let original_stable = original.stable_id();
    let binding = crate::connection_recovery_core::binding(&original).unwrap();
    let (mut stream, mut reply) = other.open_bi().await.unwrap();
    stream.write_all(b"before").await.unwrap();
    let (mut response, mut request) = other_remote.accept_bi().await.unwrap();
    let mut before = [0; 6];
    request.read_exact(&mut before).await.unwrap();
    assert_eq!(&before, b"before");
    response.write_all(b"before").await.unwrap();
    reply.read_exact(&mut before).await.unwrap();
    assert_eq!(&before, b"before");

    let (mut original_stream, mut original_reply) = original.open_bi().await.unwrap();
    original_stream.write_all(b"before").await.unwrap();
    let (mut original_response, mut original_request) = remote.accept_bi().await.unwrap();
    original_request.read_exact(&mut before).await.unwrap();
    assert_eq!(&before, b"before");
    original_response.write_all(b"before").await.unwrap();
    original_reply.read_exact(&mut before).await.unwrap();
    assert_eq!(&before, b"before");

    let b_gate = b.admission_gate();
    let b = Arc::new(tokio::sync::Mutex::new(Some(b)));
    let mut initiator = ProbeAdapter::new(a.admission_gate(), None, None);
    let mut responder = ProbeAdapter::new(b_gate, Some(fault), Some(b.clone()));
    if production_native {
        initiator.inner = NativeControlAdapter(ae.active_coordinator(initiator.gate.clone()));
        responder.inner = NativeControlAdapter(be.active_coordinator(responder.gate.clone()));
        initiator.production_native = true;
        responder.production_native = true;
        initiator.expect_tcp = tcp_observer.is_some();
        responder.expect_tcp = tcp_observer.is_some();
    }
    be.endpoint().set_alpns(vec![ACTIVE_IMPROVEMENT_ALPN.to_vec()]);
    let deadline = n0_future::time::Instant::now() + Duration::from_secs(5);
    let cancellation = tokio_util::sync::CancellationToken::new();
    let options = iroh::endpoint::ConnectOptions::new()
        .with_caller_deadline(deadline)
        .with_attempt_cancellation(cancellation.clone());
    let (control_owner, peer_control_owner) = tokio::join!(
        async {
            ae.endpoint()
                .connect_control_with_opts(be.addr(), ACTIVE_IMPROVEMENT_ALPN, options)
                .await
                .unwrap()
                .await
                .unwrap()
        },
        async {
            be.endpoint()
                .accept()
                .await
                .unwrap()
                .with_caller_deadline(deadline)
                .with_attempt_cancellation(cancellation.clone())
                .accept_control()
                .unwrap()
                .await
                .unwrap()
        }
    );
    let control = control_owner.control_handle();
    let peer_control = peer_control_owner.control_handle();
    let registry = responder.gate.maintenance();
    let lookup = registry.clone();
    let attempt = AttemptContext::new(deadline, cancellation);
    let lease = initiator
        .gate
        .maintenance()
        .acquire(&original, &attempt, ())
        .unwrap();
    let (client_result, server_result) = tokio::time::timeout(
        deadline.saturating_duration_since(n0_future::time::Instant::now()),
        async {
            tokio::join!(
                async {
                    let result = lease
                        .run(improve_existing(
                            ae.endpoint(),
                            &original,
                            &control,
                            &initiator,
                            attempt,
                            connection_diagnostic::AttemptLog::new(
                                AttemptRole::Initiator,
                                "explicit_improvement",
                            ),
                            || Ok(()),
                        ))
                        .await;
                    if matches!(&result, Ok(Ok(_))) {
                        // A scheduler-only gate guarantees peer COMMITTED was
                        // read before the successful peer's real C cleanup.
                        responder.reached.notified().await;
                    }
                    control.close(0u32.into(), b"actual local active operation completed");
                    result
                },
                serve_active_session(
                    be.endpoint(),
                    &peer_control,
                    &responder,
                    &registry,
                    deadline,
                    move |peer, binding| lookup.for_binding(peer, binding)
                )
            )
        },
    )
    .await
    .expect("original active deadline bounds the real close gate");
    println!(
        "{}",
        serde_json::json!({"event":"common-active-commit-regression",
        "scope":"admitted Native SDK/IP probe common boundary only; not RTC/WAN/M77 acceptance",
        "fault":format!("{fault:?}"),"realFreshAck":responder.real_ack.load(Ordering::Acquire),
        "peerCommittedObserved":responder.peer_committed.load(Ordering::Acquire),
        "serverStages":*responder.stages.lock().unwrap(),
        "candidateRollbackObserved":responder.rollback.load(Ordering::Acquire),
        "serverOk":server_result.is_ok(),"serverError":server_result.as_ref().err().map(ToString::to_string)})
    );
    let real_ack = responder.real_ack.load(Ordering::Acquire);
    let rolled_back = responder.rollback.load(Ordering::Acquire);
    let responder_reported = responder.report.lock().unwrap().is_some();
    if tcp_observer.is_some() {
        for connection in [&original, &remote] {
            assert!(
                connection
                    .paths()
                    .iter()
                    .any(|path| path.is_ip() && path.is_selected())
            );
            assert!(connection.paths().iter().all(|path| {
                !matches!(path.remote_addr(), TransportAddr::Custom(addr)
                    if addr.id() == crate::tcp_transport::DATA_TRANSPORT_ID && path.is_selected())
            }));
        }
    }
    let both_reports_valid = [&initiator, &responder].iter().all(|adapter| {
        adapter
            .report
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|report| {
                report["event"] == "active-improvement-committed"
                    && report["freshAck"] == true
                    && report["bothReady"] == true
            })
    });
    if fault == Fault::CompletedControlClose {
        assert_eq!(original.stable_id(), original_stable);
        assert_eq!(
            crate::connection_recovery_core::binding(&original).unwrap(),
            binding
        );
        assert!(initiator.gate.allows(&original, ProtocolScope::Member));
        assert!(responder.gate.allows(&remote, ProtocolScope::Member));
    }
    if fault != Fault::PreCommitControlClose {
        assert!(responder.peer_committed.load(Ordering::Acquire));
        assert!(responder.post_check_entered.load(Ordering::Acquire));
    }
    if matches!(
        fault,
        Fault::CompletedControlClose | Fault::PreCommitControlClose
    ) {
        original_stream.write_all(b"after").await.unwrap();
        let mut after = [0; 5];
        original_request.read_exact(&mut after).await.unwrap();
        assert_eq!(&after, b"after");
        original_response.write_all(b"after").await.unwrap();
        original_reply.read_exact(&mut after).await.unwrap();
        assert_eq!(&after, b"after");
    }
    // Durable credential invalidation deliberately revokes every old proof on b;
    // continuity is required only while the unrelated M is still authorized.
    if fault != Fault::CurrentProofInvalid {
        stream.write_all(b"after").await.unwrap();
        let mut after = [0; 5];
        request.read_exact(&mut after).await.unwrap();
        assert_eq!(&after, b"after");
        response.write_all(b"after").await.unwrap();
        reply.read_exact(&mut after).await.unwrap();
        assert_eq!(&after, b"after");
        assert!(initiator.gate.allows(&other, ProtocolScope::Member));
        assert!(responder.gate.allows(&other_remote, ProtocolScope::Member));
    } else {
        assert!(!responder.gate.allows(&remote, ProtocolScope::Member));
    }
    control.close(0u32.into(), b"owned common commit regression finished");
    peer_control.close(0u32.into(), b"owned common commit regression finished");
    for connection in [&original, &remote, &other, &other_remote] {
        connection.close(0u32.into(), b"owned common commit regression finished");
    }
    drop((
        stream,
        reply,
        response,
        request,
        original_stream,
        original_reply,
        original_response,
        original_request,
        control,
        peer_control,
        original,
        remote,
        other,
        other_remote,
    ));
    drop((initiator, responder, registry));
    tokio::join!(ae.close(), be.close());
    let b = b.lock().await.take().unwrap();
    tokio::join!(a.close(), b.close());
    // The targeted pre-fix failure is asserted only after normal SDK cleanup.
    assert!(real_ack);
    if fault == Fault::CompletedControlClose {
        assert!(matches!(&client_result, Ok(Ok(_))), "{client_result:?}");
        assert!(
            server_result.is_ok(),
            "completed peer C cleanup cannot undo qualified commit: {server_result:?}"
        );
        assert!(!rolled_back);
        assert!(both_reports_valid);
    } else {
        assert!(server_result.is_err());
        assert!(rolled_back);
        assert!(!responder_reported);
    }
}

#[tokio::test]
async fn production_native_active_rollback_preserves_existing_udp_and_other_streams() {
    tokio::time::timeout(
        Duration::from_secs(12),
        scenario_with_native(Fault::PreCommitControlClose, true, None),
    )
    .await
    .expect("native rollback must preserve existing business paths");
}

#[cfg(target_os = "linux")]
#[path = "../tests/support/tcp_binding_observer.rs"]
mod tcp_binding_observer;

#[cfg(target_os = "linux")]
#[tokio::test]
async fn active_tcp_rollback_preserves_original_paths_and_streams() {
    let (observer, server) = tcp_binding_observer::tcp_binding_observer(None, 0).await;
    tokio::time::timeout(
        Duration::from_secs(12),
        scenario_with_native(Fault::PreCommitControlClose, true, Some(observer)),
    )
    .await
    .expect("native TCP rollback must preserve both original streams");
    server.abort();
}

#[tokio::test]
async fn completed_peer_control_close_preserves_common_commit_and_other_stream() {
    scenario(Fault::CompletedControlClose).await;
}
#[tokio::test]
async fn pre_commit_control_close_still_rolls_back_candidate_and_preserves_other_stream() {
    scenario(Fault::PreCommitControlClose).await;
}
#[tokio::test]
async fn original_close_after_peer_committed_still_rolls_back_candidate() {
    scenario(Fault::OriginalClose).await;
}
#[cfg(feature = "rtc-probe")]
#[tokio::test]
async fn current_proof_invalid_after_peer_committed_still_rolls_back_candidate() {
    scenario(Fault::CurrentProofInvalid).await;
}

async fn cold_admission_with_blackhole(include_bad_candidate: bool) -> &'static str {
    let directory = tempfile::tempdir().unwrap();
    let admin = NativeNetwork::create(directory.path().join("admin"))
        .await
        .unwrap();
    let member = NativeNetwork::join(
        directory.path().join("member"),
        admin.manual_invitation().unwrap(),
    )
    .await
    .unwrap();
    for node in [&admin, &member] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(vec![]);
        node.save_resource_configuration(&config).unwrap();
    }
    let server = admin
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let client = member
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    // Keep an owned UDP socket open so the first route silently drops QUIC
    // packets instead of returning an immediate closed-port error.
    let blackhole = tokio::net::UdpSocket::bind((Ipv4Addr::new(127, 0, 0, 0), 0))
        .await
        .unwrap();
    let bad = blackhole.local_addr().unwrap();
    let advertised = server.addr();
    let good = *advertised
        .ip_addrs()
        .find(|address| address.ip() == std::net::IpAddr::V4(Ipv4Addr::LOCALHOST))
        .expect("real server loopback route");
    let target = if include_bad_candidate {
        iroh::EndpointAddr::new(advertised.id)
            .with_addrs([iroh::TransportAddr::Ip(bad), iroh::TransportAddr::Ip(good)])
    } else {
        iroh::EndpointAddr::new(advertised.id).with_addrs([iroh::TransportAddr::Ip(good)])
    };
    if include_bad_candidate {
        assert_eq!(
            target.addrs.iter().next(),
            Some(&iroh::TransportAddr::Ip(bad))
        );
    }

    let deadline = n0_future::time::Instant::now() + Duration::from_secs(5);
    let cancellation = tokio_util::sync::CancellationToken::new();
    let caller = client.coordinator();
    let responder = server.coordinator();
    let result = tokio::time::timeout(Duration::from_secs(7), async {
        tokio::join!(
            caller.connect_control(
                target,
                SessionPurpose::AdmissionRequest,
                AttemptContext::new(deadline, cancellation.clone()),
            ),
            async {
                let incoming = server.endpoint().accept().await.unwrap();
                responder
                    .accept_control(
                        incoming,
                        SessionPurpose::AdmissionRequest,
                        AttemptContext::new(deadline, cancellation),
                    )
                    .await
            }
        )
    })
    .await;
    let outcome = match result {
        Ok((Ok(outgoing), Ok(incoming))) => {
            outgoing.close(0u32.into(), b"owned route test ended");
            incoming.close(0u32.into(), b"owned route test ended");
            "connected"
        }
        Ok((Err(_), _)) => "caller_failed",
        Ok((_, Err(_))) => "responder_failed",
        Err(_) => "scenario_timeout",
    };
    tokio::join!(client.close(), server.close());
    drop(blackhole);
    outcome
}

#[tokio::test]
async fn cold_admission_reaches_the_only_advertised_local_ip() {
    assert_eq!(cold_admission_with_blackhole(false).await, "connected");
}

#[tokio::test]
async fn cold_admission_uses_reachable_ip_after_a_blackholed_first_candidate() {
    assert_eq!(
        cold_admission_with_blackhole(true).await,
        "connected",
        "the working route must share the original deadline"
    );
}

#[derive(Clone, Debug)]
struct EchoMember(tokio::sync::mpsc::UnboundedSender<Connection>);

impl iroh::protocol::ProtocolHandler for EchoMember {
    async fn accept(&self, connection: Connection) -> Result<(), iroh::protocol::AcceptError> {
        self.0
            .send(connection.clone())
            .map_err(iroh::protocol::AcceptError::from_err)?;
        while let Ok((mut send, mut receive)) = connection.accept_bi().await {
            let bytes = receive
                .read_to_end(32)
                .await
                .map_err(iroh::protocol::AcceptError::from_err)?;
            send.write_all(&bytes)
                .await
                .map_err(iroh::protocol::AcceptError::from_err)?;
            send.finish()
                .map_err(iroh::protocol::AcceptError::from_err)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct ObserveControl(tokio::sync::mpsc::UnboundedSender<iroh::endpoint::ControlConnection>);

impl iroh::protocol::ControlProtocolHandler for ObserveControl {
    async fn accept(
        &self,
        connection: iroh::endpoint::ControlConnection,
    ) -> Result<(), iroh::protocol::AcceptError> {
        self.0
            .send(connection)
            .map_err(iroh::protocol::AcceptError::from_err)
    }
}

#[derive(Clone, Debug, Default)]
struct PauseProtectedFile {
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}

impl iroh::endpoint::EndpointHooks for PauseProtectedFile {
    async fn after_handshake(
        &self,
        connection: &Connection,
    ) -> iroh::endpoint::AfterHandshakeOutcome {
        if connection.alpn() == crate::file_transfer::FILE_ALPN {
            self.entered.notify_one();
            self.release.notified().await;
        }
        iroh::endpoint::AfterHandshakeOutcome::Accept
    }
}

async fn echo_member(connection: &Connection) {
    let (mut send, mut receive) = connection.open_bi().await.unwrap();
    send.write_all(b"member request").await.unwrap();
    send.finish().unwrap();
    assert_eq!(receive.read_to_end(32).await.unwrap(), b"member request");
}

#[test]
fn first_contact_relay_survives_a_blackholed_active_ip() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "connection_coordinator::active_commit_tests::first_contact_relay_fixture",
            "--ignored",
            "--nocapture",
        ])
        .output()
        .unwrap();
    println!("{}", String::from_utf8_lossy(&output.stdout));
    let stderr = String::from_utf8(output.stderr).unwrap();
    eprint!("{stderr}");
    assert!(output.status.success());

    let records: Vec<Value> = stderr
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    for (stage, winner) in [
        ("mixed-business", Some("relay")),
        ("mixed-control", Some("relay")),
        ("revoked-in-flight", None),
        ("mixed-cancelled", None),
        ("mixed-expired", None),
        ("foreground-dropped", None),
        ("direct-with-offline-relay", Some("ip")),
    ] {
        let start = records
            .iter()
            .position(|record| record["stage"] == stage)
            .unwrap();
        let attempt = &records[start + 1..];
        let end = attempt
            .iter()
            .position(|record| record["event"] == "first-contact-fixture")
            .unwrap();
        let events: Vec<_> = attempt[..end]
            .iter()
            .filter(|record| record["role"] == "initiator" && record["source"] == "connect")
            .collect();
        let route_events: Vec<_> = events
            .iter()
            .copied()
            .filter(|event| event["phase"] == "route-dial")
            .collect();

        // Both routes start under the one attempt before either ends, and each
        // settles exactly once. A one-route retry cannot satisfy this assertion.
        assert_eq!(route_events.len(), 4, "{stage}");
        let [ip, relay, ..] = route_events.as_slice() else {
            unreachable!()
        };
        assert_eq!(
            ip["detail"]["input"]["routeHints"][0]["kind"], "ip",
            "{stage}"
        );
        assert_eq!(
            relay["detail"]["input"]["routeHints"][0]["kind"], "relay",
            "{stage}"
        );
        assert_eq!(ip["localAttemptId"], relay["localAttemptId"], "{stage}");
        for started in [*ip, *relay] {
            assert_eq!(started["result"], "started", "{stage}");
            assert_eq!(
                started["detail"]["input"]["routeMode"], "explicit",
                "{stage}"
            );
            let phase_id = &started["detail"]["phaseId"];
            let attempt_id = &started["localAttemptId"];
            let route_kind = started["detail"]["input"]["routeHints"][0]["kind"].as_str();
            let ended: Vec<_> = route_events[2..]
                .iter()
                .filter(|event| {
                    &event["localAttemptId"] == attempt_id
                        && &event["detail"]["phaseId"] == phase_id
                })
                .collect();
            assert_eq!(ended.len(), 1, "{stage}");
            let outcome = &ended[0]["result"];
            if winner == route_kind {
                assert_eq!(outcome, "succeeded", "{stage}");
            } else if stage == "revoked-in-flight" && route_kind == Some("relay") {
                assert_eq!(outcome, "failed", "{stage}");
                assert_ne!(
                    ended[0]["detail"]["output"]["reason"], "deadline",
                    "{stage}"
                );
            } else if stage == "mixed-expired" {
                assert!(outcome == "failed" || outcome == "cancelled", "{stage}");
            } else {
                assert_eq!(outcome, "cancelled", "{stage}");
            }
        }
        let delivered: Vec<_> = events
            .iter()
            .filter(|event| event["phase"] == "dial-delivered")
            .collect();
        if let Some(winner) = winner {
            assert_eq!(delivered.len(), 1, "{stage}");
            let started = if winner == "ip" { ip } else { relay };
            let delivery = delivered[0];
            assert_eq!(
                delivery["localAttemptId"], started["localAttemptId"],
                "{stage}"
            );
            assert_eq!(
                delivery["routePhaseId"], started["detail"]["phaseId"],
                "{stage}"
            );
            assert!(
                events.iter().any(|event| event["phase"] == "hello"
                    && event["result"] == "succeeded"
                    && event["localAttemptId"] == delivery["localAttemptId"]
                    && event["routePhaseId"] == delivery["routePhaseId"]),
                "{stage}"
            );
        } else {
            assert!(delivered.is_empty(), "{stage}");
        }
    }
}

#[tokio::test]
#[ignore = "run through first_contact_relay_survives_a_blackholed_active_ip to assert the timeline"]
async fn first_contact_relay_fixture() {
    let netem_ready = std::env::var_os("MIOPUNCH_134_NETEM_READY").map(std::path::PathBuf::from);
    let window = if netem_ready.is_some() {
        Duration::from_secs(20)
    } else {
        Duration::from_secs(2)
    };

    let (_directory, caller, responder) = admitted_pair().await;
    let mut config = iroh_relay::server::ServerConfig::default();
    config.relay = Some(iroh_relay::server::RelayConfig::new((
        Ipv4Addr::LOCALHOST,
        0,
    )));
    let relay = iroh_relay::server::Server::spawn(config).await.unwrap();
    let relay_route = TransportAddr::Relay(relay.http_url().unwrap());
    for node in [&caller, &responder] {
        node.admission_gate()
            .register_protocol(crate::file_transfer::FILE_ALPN, ProtocolScope::Member);
        let mut config = node.resource_configuration().unwrap();
        config.maintenance.contact_timeout_ms = window.as_millis() as u64;
        config.tcp_direct_disabled = true;
        config.member_data_relay_disabled = true;
        config.replacement = Some(vec![crate::resources::Resource {
            url: relay.http_url().unwrap().to_string(),
            uses: [crate::resources::ResourceUse::DataRelay].into(),
            limitation: "owned first contact relay".into(),
        }]);
        node.save_resource_configuration(&config).unwrap();
    }

    let gate = caller.admission_gate();
    let pause = PauseProtectedFile::default();
    let (control_tx, mut controls) = tokio::sync::mpsc::unbounded_channel();
    let (member_tx, mut members) = tokio::sync::mpsc::unbounded_channel();
    let responder = responder
        .maintain_network_with(Ipv4Addr::LOCALHOST, |router| {
            router
                .accept(MEMBER_ALPN, EchoMember(member_tx.clone()))
                .accept(crate::file_transfer::FILE_ALPN, EchoMember(member_tx))
                .accept_control(ADMISSION_REQUEST_ALPN, ObserveControl(control_tx))
        })
        .await
        .unwrap();
    let caller = caller
        .maintain_network_with_endpoint(
            Ipv4Addr::LOCALHOST,
            |builder| builder.hooks(pause.clone()).transport_config(
                iroh::endpoint::QuicTransportConfig::builder().test_without_nat_traversal().build()
            ),
            |router| router,
        )
        .await
        .unwrap();
    let _responder_activity = responder.business_activity();
    let _caller_activity = caller.business_activity();
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(caller.endpoint().online(), responder.endpoint().online());
    })
    .await
    .expect("the owned relay must be online for both members");

    // An owned UDP router allows the same previously working address to drop
    // packets silently while the original connection still has an active path.
    let udp = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let direct = TransportAddr::Ip(udp.local_addr().unwrap());
    let responder_addr = responder
        .addr()
        .ip_addrs()
        .copied()
        .find(|a| a.ip() == Ipv4Addr::LOCALHOST)
        .unwrap();
    let blackhole = Arc::new(AtomicBool::new(false));
    let dropped = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let router = tokio::spawn({
        let blackhole = blackhole.clone();
        let dropped = dropped.clone();
        async move {
            let mut caller_addr = None;
            let mut packet = vec![0; 65536];
            loop {
                let (length, from) = udp.recv_from(&mut packet).await.unwrap();
                if blackhole.load(Ordering::Acquire) {
                    dropped.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                if from == responder_addr {
                    if let Some(caller_addr) = caller_addr {
                        udp.send_to(&packet[..length], caller_addr).await.unwrap();
                    }
                } else {
                    caller_addr = Some(from);
                    udp.send_to(&packet[..length], responder_addr)
                        .await
                        .unwrap();
                }
            }
        }
    });
    let peer = iroh::EndpointAddr::new(responder.endpoint().id());
    let direct_only = peer.clone().with_addrs([direct.clone()]);
    let relay_only = peer.clone().with_addrs([relay_route.clone()]);
    let mixed = peer.clone().with_addrs([direct.clone(), relay_route]);
    let attempt = || {
        AttemptContext::new(
            n0_future::time::Instant::now() + window,
            tokio_util::sync::CancellationToken::new(),
        )
    };
    let require_active = || async {
        let remotes = caller.endpoint().remote_info(peer.id).await.unwrap();
        assert!(
            remotes.addrs().any(|route| route.addr() == &direct
                && matches!(route.usage(), iroh::endpoint::TransportAddrUsage::Active)),
            "the blackholed IP must still be an active hint: {remotes:?}",
        );
    };
    let mark = |stage| {
        eprintln!(
            "{}",
            serde_json::json!({
                "event":"first-contact-fixture", "stage":stage,
                "monotonicMs":connection_diagnostic::monotonic_ms(),
                "droppedPackets":dropped.load(Ordering::Relaxed),
            })
        );
    };

    // Warm and blackhole the proxy as the only supplied route. Background
    // contact may already have saved the live relay used by the mixed cases.
    let proxy_only = caller.coordinator().with_member_contacts(None);
    let original = proxy_only
        .connect(
            direct_only.clone(),
            SessionPurpose::MemberBusiness,
            attempt(),
        )
        .await
        .unwrap();
    let caller_handle = caller.endpoint().control_handle(&original);
    let binding = crate::connection_recovery_core::binding(&caller_handle).unwrap();
    let original_remote = tokio::time::timeout(window, async {
        loop {
            let incoming = members.recv().await.unwrap();
            let handle = responder.endpoint().control_handle(&incoming);
            if crate::connection_recovery_core::binding(&handle).unwrap() == binding {
                break incoming;
            }
        }
    }).await.expect("the responder received the warmup connection");
    // Keep the warmup connection on the proxy rather than letting iroh
    // advertise and migrate to the responders' unproxied loopback sockets.
    let responder_handle = responder.endpoint().control_handle(&original_remote);
    let (caller_mask, responder_mask) = tokio::join!(
        caller_handle.test_limit_transport_paths(vec![direct.clone()]),
        responder_handle.test_limit_transport_paths(vec![direct.clone()]),
    );
    caller_mask.unwrap();
    responder_mask.unwrap();
    tokio::time::timeout(window, echo_member(&original))
        .await
        .unwrap_or_else(|_| panic!("warmup direct={direct:?} caller={:?} responder={:?} close={:?}/{:?}",
            original.paths(), original_remote.paths(), original.close_reason(), original_remote.close_reason()));
    require_active().await;
    if let Some(ready) = &netem_ready {
        // A disposable namespace adds latency after endpoint registration and
        // warming the original path. Relay-only and mixed retain one window.
        mark("netem-ready");
        tokio::time::timeout(Duration::from_secs(5), async {
            while !ready.exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("namespace latency was applied");
    } else {
        blackhole.store(true, Ordering::Release);

        mark("active-only");
        let result = proxy_only
            .connect(
                direct_only.clone(),
                SessionPurpose::MemberBusiness,
                attempt(),
            )
            .await;
        assert!(
            matches!(result, Err(ConnectError::DeadlineExceeded)),
            "{result:?}"
        );
        assert!(dropped.load(Ordering::Relaxed) > 0);
        require_active().await;
    }

    let report_relay_rtt = |connection: &Connection| {
        if netem_ready.is_none() {
            return;
        }
        let rtt = connection
            .paths()
            .iter()
            .find_map(|path| {
                if matches!(path.remote_addr(), TransportAddr::Relay(_)) {
                    connection.rtt(path.id())
                } else {
                    None
                }
            })
            .expect("a measured data relay path");
        assert!(
            (Duration::from_secs(1)..=Duration::from_secs(2)).contains(&rtt),
            "{rtt:?}"
        );
        eprintln!(
            "{}",
            json!({"event":"first-contact-relay-rtt","rttMs":rtt.as_millis()})
        );
    };

    mark("relay-only-business");
    tokio::time::timeout(window, async {
        let (connection, _activity) = caller
            .connect(relay_only.clone(), MEMBER_ALPN)
            .await
            .unwrap();
        echo_member(&connection).await;
        report_relay_rtt(&connection);
        connection.close(0u32.into(), b"relay-only fixture complete");
    })
    .await
    .unwrap();
    if netem_ready.is_some() {
        tokio::time::timeout(window, echo_member(&original))
            .await
            .unwrap();
        blackhole.store(true, Ordering::Release);
    } else {
        mark("relay-only-control");
        let control = caller
            .coordinator()
            .connect_control_alpn(relay_only.clone(), ADMISSION_REQUEST_ALPN, attempt())
            .await
            .unwrap();
        assert_eq!(control.remote_id(), peer.id);
        let remote = tokio::time::timeout(window, controls.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(remote.remote_id(), caller.endpoint().id());
        control.close(0u32.into(), b"relay-only fixture complete");
    }

    require_active().await;
    mark("mixed-business");
    let business = tokio::time::timeout(window, async {
        let (connection, _activity) = caller.connect(mixed.clone(), MEMBER_ALPN).await?;
        assert_ne!(connection.stable_id(), original.stable_id());
        echo_member(&connection).await;
        report_relay_rtt(&connection);
        connection.close(0u32.into(), b"mixed fixture complete");
        Ok::<_, crate::resources::ResourceContactError>(())
    })
    .await;
    if netem_ready.is_some() {
        mark("finished");
        assert!(
            matches!(business, Ok(Ok(()))),
            "delayed mixed member business: {business:?}"
        );
        assert!(dropped.load(Ordering::Relaxed) > 0);
        assert!(original.close_reason().is_none());
        original.close(0u32.into(), b"delayed fixture complete");
        router.abort();
        tokio::join!(caller.close(), responder.close());
        relay.shutdown().await.unwrap();
        return;
    }
    require_active().await;
    mark("mixed-control");
    let control = caller
        .coordinator()
        .connect_control_alpn(mixed.clone(), ADMISSION_REQUEST_ALPN, attempt())
        .await;
    if let Ok(control) = &control {
        assert_eq!(control.remote_id(), peer.id);
        let remote = tokio::time::timeout(window, controls.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(remote.remote_id(), caller.endpoint().id());
        control.close(0u32.into(), b"mixed fixture complete");
    }
    mark("mixed-finished");
    assert!(
        matches!(business, Ok(Ok(()))),
        "mixed member business: {business:?}; mixed restricted control: {control:?}"
    );
    assert!(control.is_ok(), "mixed restricted control: {control:?}");
    assert!(original.close_reason().is_none());

    mark("ready-tie");
    blackhole.store(false, Ordering::Release);
    let coordinator = caller.coordinator();
    let active_attempt = attempt();
    let relay_attempt = attempt();
    let log = AttemptLog::new(AttemptRole::Initiator, "first-contact-tie");
    let (active, relayed) = tokio::join!(
        Box::pin(coordinator.dial(
            direct_only,
            SessionPurpose::MemberBusiness,
            &active_attempt,
            MEMBER_ALPN,
            false,
            log
        )),
        Box::pin(coordinator.dial(
            relay_only,
            SessionPurpose::MemberBusiness,
            &relay_attempt,
            MEMBER_ALPN,
            false,
            log
        )),
    );
    let active = active.ok().expect("real active route qualified");
    let relayed = relayed.ok().expect("real relay route qualified");
    let ConnectionDelivery::Business(loser) = &relayed.delivery else {
        unreachable!()
    };
    let loser = loser.clone();
    let active_id = active.delivery.stable_id();
    // Supply both already qualified connections to exercise the same-poll tie
    // deterministically. The actual dials and data admission ran above.
    let winner = race_contact_routes(
        (&active_attempt, std::future::ready(Ok(active))),
        (&relay_attempt, std::future::ready(Ok(relayed))),
    )
    .await
    .ok()
    .expect("one route won");
    assert_eq!(winner.delivery.stable_id(), active_id);
    assert!(loser.close_reason().is_some());
    assert!(relay_attempt.cancellation.is_cancelled());
    assert!(!active_attempt.cancellation.is_cancelled());
    let ConnectionDelivery::Business(qualified) = &winner.delivery else {
        unreachable!()
    };
    let qualified = qualified.clone();
    tokio::time::timeout(window, echo_member(&qualified))
        .await
        .unwrap();

    // A ready success cannot hide a terminal authorization failure from the
    // other branch. The already qualified connection must also be retired.
    let rejected = race_contact_routes(
        (&active_attempt, std::future::ready(Ok(winner))),
        (
            &attempt(),
            std::future::ready(Err(ContactDialFailure::Terminal(
                ConnectError::IdentityOrAdmissionRejected,
            ))),
        ),
    )
    .await;
    assert!(matches!(
        rejected,
        Err(ContactDialFailure::Terminal(
            ConnectError::IdentityOrAdmissionRejected
        ))
    ));
    assert!(qualified.close_reason().is_some());

    // Revoke this application ALPN after the real relay has authenticated
    // and qualified its path, with the active-IP dial still pending.
    blackhole.store(true, Ordering::Release);
    require_active().await;
    mark("revoked-in-flight");
    let coordinator_result = tokio::time::timeout(window, async {
        tokio::join!(
            coordinator.connect_alpn(mixed.clone(), crate::file_transfer::FILE_ALPN, attempt()),
            async {
                pause.entered.notified().await;
                gate.restrict_lifecycle(Some(
                    [MEMBER_ALPN.to_vec(), ADMISSION_REQUEST_ALPN.to_vec()].into(),
                ));
                pause.release.notify_one();
            },
        )
    })
    .await
    .expect("the live relay reached the protected admission hook")
    .0;
    assert!(coordinator_result.is_err(), "{coordinator_result:?}");
    assert!(original.close_reason().is_none());

    relay.shutdown().await.unwrap();
    blackhole.store(true, Ordering::Release);
    require_active().await;
    mark("mixed-cancelled");
    let ctx = attempt();
    let token = ctx.cancellation.clone();
    let (result, ()) = tokio::join!(
        coordinator.connect_control_alpn(mixed.clone(), ADMISSION_REQUEST_ALPN, ctx),
        async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            token.cancel();
        },
    );
    assert!(matches!(result, Err(ConnectError::Cancelled)), "{result:?}");

    require_active().await;
    mark("mixed-expired");
    let result = coordinator
        .connect_control_alpn(mixed.clone(), ADMISSION_REQUEST_ALPN, attempt())
        .await;
    assert!(
        matches!(result, Err(ConnectError::DeadlineExceeded)),
        "{result:?}"
    );
    assert!(controls.try_recv().is_err());

    require_active().await;
    mark("foreground-dropped");
    let dropped_call = tokio::time::timeout(
        Duration::from_millis(100),
        caller.connect(mixed.clone(), MEMBER_ALPN),
    )
    .await;
    assert!(dropped_call.is_err());

    blackhole.store(false, Ordering::Release);
    require_active().await;
    mark("direct-with-offline-relay");
    tokio::time::timeout(window, async {
        let (connection, _activity) = caller.connect(mixed.clone(), MEMBER_ALPN).await.unwrap();
        echo_member(&connection).await;
        connection.close(0u32.into(), b"direct fixture complete");
    })
    .await
    .unwrap();
    tokio::time::timeout(window, echo_member(&original))
        .await
        .unwrap();
    while let Ok(remote) = members.try_recv() {
        tokio::time::timeout(window, remote.closed()).await.unwrap();
    }

    mark("revoked");
    gate.update_authorization([*caller.endpoint().id().as_bytes()], [*peer.id.as_bytes()]);
    let result = coordinator
        .connect(mixed, SessionPurpose::MemberBusiness, attempt())
        .await;
    assert!(
        matches!(result, Err(ConnectError::IdentityOrAdmissionRejected)),
        "{result:?}"
    );
    tokio::time::timeout(window, original.closed())
        .await
        .unwrap();

    mark("finished");
    router.abort();
    tokio::join!(caller.close(), responder.close());
}
