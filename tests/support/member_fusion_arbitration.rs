//! The frozen contention input through normal fused member admission and streams.

use std::{
    io::ErrorKind,
    net::{Ipv4Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering::Relaxed},
    },
    time::Duration,
};

use iroh::{
    Endpoint, EndpointAddr, TransportAddr,
    endpoint::{MemberConnection, MemberFusionEndpoint, MemberStream, MemberUdpPlan, presets},
};
use miopunch_native_probe::{
    native_network::{NativeNetwork, member_fusion_attempt_id},
    network_maintenance::MaintenancePolicy,
    resources::{Resource, ResourceUse},
};
use serde_json::json;
use tokio::{
    sync::Barrier,
    task::JoinSet,
    time::{self, Instant},
};
use tokio_util::sync::CancellationToken;

use super::{
    join,
    path_contention_baseline::{DEADLINE, FLOOR, delayed_relay::delay_relay, observe_stun},
};

struct Pair {
    _directory: tempfile::TempDir,
    nodes: [NativeNetwork; 2],
    endpoints: [MemberFusionEndpoint; 2],
    connections: [MemberConnection; 2],
    observers: JoinSet<()>,
    requests: [Arc<AtomicUsize>; 2],
    relay: iroh_relay::server::Server,
    relay_stop: CancellationToken,
}

impl Pair {
    async fn bind(policy: MaintenancePolicy, relay_only: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut left = NativeNetwork::create(directory.path().join("left"))
            .await
            .unwrap();
        let right = join(&mut left, &directory.path().join("right")).await;
        let mut config = iroh_relay::server::ServerConfig::default();
        config.relay = Some(iroh_relay::server::RelayConfig::new((
            Ipv4Addr::LOCALHOST,
            0,
        )));
        let relay = iroh_relay::server::Server::spawn(config).await.unwrap();
        let (url, relay_stop) =
            delay_relay(relay.http_addr().unwrap(), Duration::from_millis(35)).await;
        let path = TransportAddr::Relay(url.clone());
        let requests: [Arc<AtomicUsize>; 2] = std::array::from_fn(|_| Arc::default());
        let mut observers = JoinSet::new();
        let nodes = [left, right];
        let mut endpoints = Vec::new();
        for (node, count) in nodes.iter().zip(&requests) {
            let mut config = node.resource_configuration().unwrap();
            let mut resources = observe_stun(&mut observers, count).await;
            resources.push(Resource {
                url: url.to_string(),
                uses: [ResourceUse::DataRelay].into(),
                limitation: "isolated business relay".into(),
            });
            config.replacement = Some(resources);
            config.udp_direct_disabled = false;
            config.tcp_direct_disabled = true;
            config.member_data_relay_disabled = true;
            config.maintenance = policy.clone();
            node.save_resource_configuration(&config).unwrap();
            let builder = Endpoint::builder(presets::Minimal)
                .clear_ip_transports()
                .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
                .unwrap();
            endpoints.push(
                node.bind_member_fusion_endpoint_with(builder, DEADLINE)
                    .await
                    .unwrap(),
            );
        }
        let endpoints: [MemberFusionEndpoint; 2] = endpoints.try_into().ok().unwrap();
        let (left, right) = time::timeout(DEADLINE, async {
            tokio::join!(
                async {
                    endpoints[0].online().await;
                    let peer = if relay_only {
                        EndpointAddr::new(endpoints[1].id()).with_addrs([path.clone()])
                    } else {
                        let direct = *endpoints[1]
                            .addr()
                            .ip_addrs()
                            .find(|addr| addr.is_ipv4())
                            .unwrap();
                        EndpointAddr::new(endpoints[1].id()).with_ip_addr(direct)
                    };
                    endpoints[0].connect(peer).await
                },
                async {
                    endpoints[1].online().await;
                    endpoints[1].accept().await.unwrap()
                },
            )
        })
        .await
        .unwrap();
        let connections = [left.unwrap(), right.unwrap()];
        assert_eq!(connections[0].remote_id(), endpoints[1].id());
        assert_eq!(connections[1].remote_id(), endpoints[0].id());
        assert_eq!(
            member_fusion_attempt_id(&connections[0]),
            member_fusion_attempt_id(&connections[1])
        );
        if relay_only {
            for (endpoint, connection) in endpoints.iter().zip(&connections) {
                endpoint
                    .test_limit_transport_paths(connection, vec![path.clone()])
                    .await
                    .unwrap();
                for local in connection.paths().iter().filter(|path| !path.is_relay()) {
                    local.close().unwrap();
                }
                assert!(
                    connection
                        .paths()
                        .iter()
                        .any(|path| path.is_relay() && path.is_selected())
                );
            }
        }
        Self {
            _directory: directory,
            nodes,
            endpoints,
            connections,
            observers,
            requests,
            relay,
            relay_stop,
        }
    }

    fn requests(&self) -> [usize; 2] {
        self.requests
            .each_ref()
            .map(|requests| requests.load(Relaxed))
    }

    async fn close(mut self) {
        tokio::join!(self.endpoints[0].close(), self.endpoints[1].close());
        let [left, right] = self.nodes;
        tokio::join!(left.close(), right.close());
        self.relay_stop.cancel();
        self.relay.shutdown().await.unwrap();
        self.observers.shutdown().await;
    }
}

async fn open_flow(from: &MemberConnection, to: &MemberConnection) -> (MemberStream, MemberStream) {
    let (from, to) = tokio::join!(from.open_bi(), to.accept_bi());
    let (from, to) = (from.unwrap(), to.unwrap());
    assert!(from.is_business() && to.is_business());
    (from, to)
}

async fn exchange(flow: &mut (MemberStream, MemberStream), sequence: u64) {
    let (from, to) = flow;
    let (send_a, recv_a) = from.streams();
    let (send_b, recv_b) = to.streams();
    assert_eq!(send_a.id(), send_b.id());
    let left = sequence.to_be_bytes();
    let right = (!sequence).to_be_bytes();
    let mut received_left = [0; 8];
    let mut received_right = [0; 8];
    let (send_left, send_right, read_left, read_right) = tokio::join!(
        send_a.write_all(&left),
        send_b.write_all(&right),
        recv_b.read_exact(&mut received_left),
        recv_a.read_exact(&mut received_right),
    );
    send_left.unwrap();
    send_right.unwrap();
    read_left.unwrap();
    read_right.unwrap();
    assert_eq!((received_left, received_right), (left, right));
}

async fn maintenance(from: &MemberConnection, to: &MemberConnection, sequence: u64) {
    let (sent, received) = tokio::join!(from.open_maintenance_bi(), to.accept_bi());
    let mut stream = (sent.unwrap(), received.unwrap());
    assert!(!stream.0.is_business() && !stream.1.is_business());
    exchange(&mut stream, sequence).await;
}

async fn plans(connections: &[MemberConnection; 2]) -> [MemberUdpPlan; 2] {
    loop {
        if let [Some(a), Some(b)] = connections.each_ref().map(MemberConnection::udp_plan) {
            assert_eq!(a.round, b.round);
            return [a, b];
        }
        time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn crossed_fused_offers_share_one_physical_round_and_preserve_both_business_streams() {
    let pair = Pair::bind(
        MaintenancePolicy {
            exploration_interval_ms: FLOOR.as_millis() as u64,
            ..MaintenancePolicy::conservative()
        },
        true,
    )
    .await;
    let [left, right] = &pair.connections;
    let mut a = open_flow(left, right).await;
    let mut b = open_flow(right, left).await;
    tokio::join!(exchange(&mut a, 1), exchange(&mut b, 1));
    assert_eq!(pair.requests(), [0, 0]);
    let originals = pair.connections.each_ref().map(MemberConnection::stable_id);
    let streams = [
        u64::from(a.0.streams().0.id()),
        u64::from(b.0.streams().0.id()),
    ];
    assert_ne!(streams[0], streams[1]);
    let leader = usize::from(pair.endpoints[0].id() > pair.endpoints[1].id());
    let dry = CancellationToken::new();
    let dry_deadline = Instant::now() + DEADLINE;
    let winner = &pair.connections[leader];
    let yielder = &pair.connections[1 - leader];
    let barrier = Arc::new(Barrier::new(2));
    for endpoint in &pair.endpoints {
        endpoint.test_coordinate_udp_reservation(barrier.clone());
    }
    let (dry_cancelled, dry_yielded) = time::timeout_at(dry_deadline, async {
        let conflicting = CancellationToken::new();
        let (winner, yielder) =
            tokio::join!(winner.coordinate_udp_traversal(dry_deadline, &dry), async {
                let result = yielder
                    .coordinate_udp_traversal(dry_deadline, &conflicting)
                    .await;
                dry.cancel();
                result
            },);
        (winner.unwrap_err().kind(), yielder.unwrap_err().kind())
    })
    .await
    .unwrap();
    assert_eq!(dry_cancelled, ErrorKind::Interrupted);
    assert_eq!(dry_yielded, ErrorKind::WouldBlock);
    let dry_requests = pair.requests();
    assert_eq!(dry_requests, [0, 0]);
    assert!(
        pair.connections
            .iter()
            .all(|connection| connection.udp_traversal().is_none())
    );
    let dry_finished = Instant::now();
    tokio::join!(exchange(&mut a, 2), exchange(&mut b, 2));
    // A protected stream's STOP travels on the same relay as these original bytes.
    // Give the responding executor time to see it, still within the policy floor.
    time::sleep_until(dry_finished + Duration::from_millis(150)).await;
    let started = Instant::now();
    assert!(started - dry_finished < FLOOR);
    let deadline = started + DEADLINE;
    let cancel = CancellationToken::new();
    let stop = CancellationToken::new();
    let pulse = tokio::spawn({
        let stop = stop.clone();
        async move {
            let mut last_sequence = 2;
            let mut ticks = time::interval(Duration::from_millis(75));
            loop {
                tokio::select! { biased;
                    _ = stop.cancelled() => break (a,b,last_sequence),
                    _ = ticks.tick() => {
                        last_sequence += 1;
                        tokio::join!(exchange(&mut a, last_sequence), exchange(&mut b, last_sequence));
                    }
                }
            }
        }
    });
    let (result, work) = time::timeout_at(deadline + Duration::from_millis(100), async {
        let (outcome, plans) = tokio::join!(
            yielder.coordinate_udp_traversal(deadline, &cancel),
            plans(&pair.connections),
        );
        while !pair.connections.iter().all(|connection| {
            connection
                .udp_traversal()
                .is_some_and(|report| report.outcome.is_some())
        }) {
            time::sleep(Duration::from_millis(5)).await;
        }
        (outcome, plans)
    })
    .await
    .expect("crossed requests and the worked round fit the original three seconds");
    let settled = started.elapsed();
    stop.cancel();
    let (mut a, mut b, sequences) = time::timeout(FLOOR, pulse).await.unwrap().unwrap();
    assert_ne!(result.unwrap_err().kind(), ErrorKind::WouldBlock);
    assert_eq!(
        work.each_ref().map(|plan| plan.coordinator),
        [leader == 0, leader == 1]
    );
    let retry = Instant::now() + DEADLINE;
    let mut counts = [0; 2];
    let mut limits = [0; 2];
    let mut samples = [0; 2];
    let mut matches = [false; 2];
    for side in 0..2 {
        let connection = &pair.connections[side];
        let plan = &work[side];
        let report = connection.udp_traversal().expect("worked report retained");
        assert_eq!(report.round, plan.round);
        assert_eq!(report.connection_id, originals[side]);
        assert!(report.deadline <= deadline + Duration::from_millis(100));
        assert!(report.outcome.is_some_and(|outcome| outcome.is_err()));
        assert!(report.validated.is_none() && !report.learned);
        samples[side] = plan.local_samples().len();
        matches[side] = report.first_match.is_some();
        counts[side] = report.trace.probes + report.trace.responses;
        limits[side] = plan
            .local_phase
            .peer_datagram_bound(usize::from(plan.local_phase.behavior.listen_random) + 1);
        assert_eq!(samples[side], 2);
        assert!(matches[side] && counts[side] > 0 && counts[side] <= limits[side]);
        let defer = connection
            .coordinate_udp_traversal(retry, &cancel)
            .await
            .unwrap_err();
        assert_eq!(defer.kind(), ErrorKind::WouldBlock);
        assert_eq!(
            defer.to_string(),
            "member UDP round cooling down",
            "the worked round released its slot"
        );
    }
    assert_eq!(pair.requests(), samples);
    assert!(sequences > 3);
    tokio::join!(
        exchange(&mut a, sequences + 1),
        exchange(&mut b, sequences + 1)
    );
    assert_eq!(
        pair.connections.each_ref().map(MemberConnection::stable_id),
        originals
    );
    assert_eq!(
        [
            u64::from(a.0.streams().0.id()),
            u64::from(b.0.streams().0.id())
        ],
        streams
    );
    eprintln!(
        "{}",
        json!({
            "event":"fused-contention-fixture", "originals":originals, "streams":streams,
            "attempt":hex::encode(member_fusion_attempt_id(left).unwrap()),
            "round":hex::encode(work[0].round), "leader":leader, "yielded":1-leader,
            "stunRequests":pair.requests(), "samples":samples, "packets":counts, "limits":limits,
        "matched":matches, "businessSequences":[sequences + 1,sequences + 1], "workedCooldown":true,
        "dryRequests":dry_requests, "dryCancelled":format!("{dry_cancelled:?}"),
        "dryYielded":format!("{dry_yielded:?}"), "followUpMs":(started-dry_finished).as_millis(),
        "deadlineMs":DEADLINE.as_millis(), "settledMs":settled.as_millis(),
        })
    );
    pair.close().await;
}

#[tokio::test]
async fn enhanced_maintenance_uses_each_nodes_policy_and_preserves_the_original_idle_deadline() {
    for (profile, policy) in [
        ("mobile", MaintenancePolicy::conservative()),
        ("always-on", MaintenancePolicy::active()),
    ] {
        let idle = Duration::from_millis(policy.exploration_interval_ms);
        let pair = Pair::bind(policy, true).await;
        let [a, b] = &pair.connections;
        let mut flow = open_flow(a, b).await;
        exchange(&mut flow, 1).await;
        assert_eq!(pair.requests(), [0, 0]);
        drop(flow);
        let released = Instant::now();
        let cutoff = released + idle;
        let mut checks = 0;
        time::timeout_at(cutoff + Duration::from_millis(350), async {
            let pings = async {
                loop {
                    maintenance(a, b, checks + 1).await;
                    checks += 1;
                    time::sleep(idle / 7).await;
                }
            };
            tokio::select! {
                biased;
                _ = a.closed() => {}
                _ = b.closed() => {}
                _ = pings => {}
            }
        })
        .await
        .expect("periodic maintenance must let the original idle clock close the connection");
        assert!(Instant::now() >= cutoff - Duration::from_millis(100));
        assert!(checks >= 2, "{profile}: {checks} actual checks");
        time::timeout(Duration::from_millis(350), async {
            tokio::join!(a.closed(), b.closed());
            while pair.connections.iter().any(|conn| {
                conn.udp_traversal()
                    .is_some_and(|report| report.outcome.is_none())
            }) {
                time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("original idle closure cancels both maintenance workers");
        assert_eq!(
            pair.requests(),
            [2, 2],
            "{profile}: at most one real observation round while idle"
        );
        let reports = pair.connections.each_ref().map(|conn| {
            conn.udp_traversal()
                .expect("both ends entered the bounded maintenance round")
        });
        assert_eq!(reports[0].round, reports[1].round);
        for report in &reports {
            assert!(report.deadline <= cutoff + Duration::from_millis(50));
            assert!(report.finished.unwrap() <= (cutoff + Duration::from_millis(150)).into_std());
        }
        eprintln!(
            "{}",
            json!({
                "event":"fusion-maintenance-fixture", "profile":profile, "checks":checks,
                "observationRequests":pair.requests(), "idleMs":idle.as_millis(),
                "elapsedMs":released.elapsed().as_millis(),
                "packets":reports.map(|report| report.trace.probes + report.trace.responses),
            })
        );
        pair.close().await;
    }
}

#[tokio::test]
async fn business_preempts_unspent_idle_maintenance_and_cancel_releases_the_original_round() {
    let pair = Pair::bind(MaintenancePolicy::active(), true).await;
    let [a, b] = &pair.connections;
    // The pending idle worker reaches its actual reservation and waits without starting I/O.
    pair.endpoints[0].test_coordinate_udp_reservation(Arc::new(Barrier::new(2)));
    maintenance(a, b, 1).await;
    assert_eq!(pair.requests(), [0, 0]);
    let mut stream = open_flow(a, b).await;
    let work = time::timeout(Duration::from_secs(3), async {
        let mut seq = 2;
        loop {
            exchange(&mut stream, seq).await;
            if let [Some(left), Some(right)] = pair
                .connections
                .each_ref()
                .map(MemberConnection::udp_traversal)
                && [
                    left.trace.probes + left.trace.responses,
                    right.trace.probes + right.trace.responses,
                ]
                .into_iter()
                .all(|packets| packets > 0)
            {
                assert_eq!(left.round, right.round);
                break left.round;
            }
            seq += 1;
            time::sleep(Duration::from_millis(35)).await;
        }
    })
    .await;
    let round = work.expect("foreground used the slot held by the idle check");
    assert_eq!(pair.requests(), [2, 2]);
    assert!(
        pair.connections
            .iter()
            .all(MemberConnection::test_nat_traversal_probes_paused)
    );
    drop(stream);
    time::timeout(Duration::from_millis(600), async {
        loop {
            if pair.connections.iter().all(|conn| {
                !conn.test_nat_traversal_probes_paused()
                    && conn.udp_traversal().is_some_and(|report| {
                        report.round == round && report.outcome == Some(Err(ErrorKind::Interrupted))
                    })
            }) {
                break;
            }
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("dropping both real business lifetimes stops both ends before the round deadline");
    assert!(a.close_reason().is_none() && b.close_reason().is_none());
    let packets = pair
        .connections
        .each_ref()
        .map(|conn| conn.udp_traversal().unwrap().trace.probes);
    time::sleep(Duration::from_millis(250)).await;
    assert_eq!(pair.requests(), [2, 2]);
    assert_eq!(
        pair.connections
            .each_ref()
            .map(|conn| conn.udp_traversal().unwrap().trace.probes),
        packets
    );
    eprintln!(
        "{}",
        json!({
            "event":"fusion-enhanced-cancellation", "round":hex::encode(round),
            "qntPausedDuring":[true,true], "qntPausedAfter":[false,false], "packets":packets,
            "observationRequests":pair.requests(),
        })
    );
    pair.close().await;
}

#[tokio::test]
async fn a_live_direct_original_keeps_irohs_path_policy_without_enhanced_observation() {
    let pair = Pair::bind(MaintenancePolicy::active(), false).await;
    let [a, b] = &pair.connections;
    let mut flow = open_flow(a, b).await;
    for seq in 1..=3 {
        exchange(&mut flow, seq).await;
        assert!(pair.connections.iter().all(|conn| {
            conn.paths()
                .iter()
                .any(|path| path.is_selected() && !path.is_relay())
        }));
        time::sleep(Duration::from_millis(800)).await;
    }
    assert_eq!(pair.requests(), [0, 0]);
    assert!(
        pair.connections
            .iter()
            .all(|conn| conn.udp_traversal().is_none())
    );
    pair.close().await;
}
