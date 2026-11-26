//! The frozen contention input through normal fused member admission and streams.

use std::{
    io::ErrorKind,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
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
    net::UdpSocket,
    sync::Barrier,
    task::JoinSet,
    time::{self, Instant},
};
use tokio_util::sync::CancellationToken;

use super::{
    join,
    path_contention_baseline::{DEADLINE, FLOOR, delayed_relay::delay_relay, observe_stun},
};

#[path = "udp_blackhole.rs"]
mod udp_blackhole;

struct Pair {
    _directory: tempfile::TempDir,
    nodes: [NativeNetwork; 2],
    endpoints: [MemberFusionEndpoint; 2],
    connections: [MemberConnection; 2],
    observers: JoinSet<()>,
    requests: [Arc<AtomicUsize>; 2],
    relay: iroh_relay::server::Server,
    relay_stop: CancellationToken,
    _ipv4_blackhole: Option<udp_blackhole::UdpBlackhole>,
    ipv6_blackhole: Option<udp_blackhole::UdpBlackhole>,
}

impl Pair {
    async fn bind(policy: MaintenancePolicy, relay_only: bool) -> Self {
        let pair = Self::bind_with(policy, relay_only, |builder| builder, None, false).await;
        if relay_only {
            for (endpoint, connection) in pair.endpoints.iter().zip(&pair.connections) {
                let path = connection.initial_data_path().clone();
                assert!(matches!(path, TransportAddr::Relay(_)));
                endpoint
                    .test_limit_transport_paths(connection, vec![path])
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
        pair
    }

    async fn bind_with(
        policy: MaintenancePolicy,
        relay_only: bool,
        configure: impl Fn(iroh::endpoint::Builder) -> iroh::endpoint::Builder,
        observations: Option<&[Resource]>,
        hold_direct: bool,
    ) -> Self {
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
            let mut resources = match observations {
                Some(resources) => resources.to_vec(),
                None => observe_stun(&mut observers, count).await,
            };
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
                node.bind_member_fusion_endpoint_with(configure(builder), DEADLINE)
                    .await
                    .unwrap(),
            );
        }
        let endpoints: [MemberFusionEndpoint; 2] = endpoints.try_into().ok().unwrap();
        let ipv4_blackhole = hold_direct.then(|| {
            udp_blackhole::UdpBlackhole::new(endpoints.each_ref().map(|endpoint| {
                *endpoint
                    .addr()
                    .ip_addrs()
                    .find(|address| address.is_ipv4())
                    .unwrap()
            }))
        });
        let ipv6_blackhole = hold_direct.then(|| {
            udp_blackhole::UdpBlackhole::new(endpoints.each_ref().map(|endpoint| {
                *endpoint
                    .addr()
                    .ip_addrs()
                    .find(|address| address.is_ipv6())
                    .unwrap()
            }))
        });
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
        Self {
            _directory: directory,
            nodes,
            endpoints,
            connections,
            observers,
            requests,
            relay,
            relay_stop,
            _ipv4_blackhole: ipv4_blackhole,
            ipv6_blackhole,
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

#[tokio::test]
async fn one_standby_slot_preserves_business_across_background_contacts() {
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("owner"))
        .await
        .unwrap();
    let b = join(&mut admin, &dir.path().join("b")).await;
    let c = join(&mut admin, &dir.path().join("c")).await;
    let mut endpoints = Vec::new();
    for node in [&admin, &b, &c] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(vec![]);
        config.maintenance.standby_member_limit = Some(1);
        node.save_resource_configuration(&config).unwrap();
        endpoints.push(
            node.bind_member_fusion_endpoint_with(
                Endpoint::builder(presets::Minimal)
                    .clear_ip_transports()
                    .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
                    .unwrap(),
                DEADLINE,
            )
            .await
            .unwrap(),
        );
    }
    let [owner, b, c]: [MemberFusionEndpoint; 3] = endpoints.try_into().unwrap();
    async fn connect(
        owner: &MemberFusionEndpoint,
        peer: &MemberFusionEndpoint,
    ) -> (MemberConnection, MemberConnection) {
        let (local, remote) = tokio::join!(owner.connect(peer.addr()), peer.accept());
        (local.unwrap(), remote.unwrap().unwrap())
    }
    time::timeout(Duration::from_secs(12), async {
        let (ab, ba) = connect(&owner, &b).await;
        let mut file = open_flow(&ab, &ba).await;
        let mut terminal = open_flow(&ab, &ba).await;
        exchange(&mut file, 1).await;
        drop(file);
        exchange(&mut terminal, 2).await;
        drop(terminal);

        let (ac, ca) = connect(&owner, &c).await;
        maintenance(&ac, &ca, 3).await;
        ac.closed().await;
        assert!(ab.close_reason().is_none());
        let (ac, ca) = connect(&owner, &c).await;
        let (check, other) = tokio::join!(ac.open_maintenance_bi(), ca.accept_bi());
        let mut checks = (check.unwrap(), other.unwrap());
        exchange(&mut checks, 4).await;
        let mut business = open_flow(&ac, &ca).await;
        drop(checks);
        exchange(&mut business, 5).await;
        assert!(ab.close_reason().is_none());
        drop(business);
        ab.closed().await;
        let reused = owner.connect(c.addr()).await.unwrap();
        assert_eq!(reused.stable_id(), ac.stable_id());
        exchange(&mut open_flow(&ac, &ca).await, 6).await;
    })
    .await
    .expect("multi-peer standby and in-flight stream eviction");
    tokio::join!(owner.close(), b.close(), c.close());
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
async fn standby_udp_uses_the_round_budget_without_a_ping() {
    for interval in [1_000, 30_000] {
        let pair = Pair::bind(
            MaintenancePolicy {
                exploration_interval_ms: interval,
                quality_interval_ms: 250,
                path_retry_base_ms: 250,
                contact_timeout_ms: 3_000,
                enhanced_udp_cooldown_ms: 6_000,
                ..MaintenancePolicy::active()
            },
            true,
        )
        .await;
        let [a, b] = &pair.connections;
        let started = Instant::now();
        let reports = time::timeout(Duration::from_secs(4), async {
            loop {
                if let [Some(a), Some(b)] = pair
                    .connections
                    .each_ref()
                    .map(MemberConnection::udp_traversal)
                {
                    break [a, b];
                }
                time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("standby without a ping must start a round");
        assert_eq!(reports[0].round, reports[1].round);
        for report in reports {
            assert!(report.deadline > started + Duration::from_secs(1));
            assert!(report.deadline <= Instant::now() + Duration::from_secs(3));
        }
        time::sleep_until(started + Duration::from_secs(5)).await;
        assert_eq!(pair.requests(), [2, 2]);
        time::timeout_at(started + Duration::from_secs(9), async {
            while !pair.requests().into_iter().all(|samples| samples >= 4) {
                time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the second physical round follows the configured cooldown in both profiles");
        if interval == 30_000 {
            tokio::join!(
                pair.endpoints[0].network_change(),
                pair.endpoints[1].network_change()
            );
            time::timeout(Duration::from_secs(4), async {
                while !pair.requests().into_iter().all(|samples| samples >= 6) {
                    time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("a real network change wakes standby before the previous six-second cooldown");
        }
        let mut flow = open_flow(a, b).await;
        exchange(&mut flow, 7).await;
        drop(flow);
        pair.close().await;
    }
}

#[tokio::test]
async fn business_preempts_unspent_idle_maintenance_and_cancel_releases_the_original_round() {
    let pair = Pair::bind(
        MaintenancePolicy {
            quality_interval_ms: 30_000,
            ..MaintenancePolicy::active()
        },
        true,
    )
    .await;
    let [a, b] = &pair.connections;
    // The pending idle worker reaches its actual reservation and waits without starting I/O.
    let reserved = pair.endpoints[0].test_coordinate_udp_reservation(Arc::new(Barrier::new(2)));
    maintenance(a, b, 1).await;
    time::timeout(DEADLINE, async {
        while !reserved.load(std::sync::atomic::Ordering::Acquire) {
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the idle worker holds its actual reservation before business arrives");
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
            .all(MemberConnection::test_ipv4_nat_traversal_probes_paused)
    );
    drop(stream);
    time::timeout(Duration::from_millis(600), async {
        loop {
            if pair.connections.iter().all(|conn| {
                !conn.test_ipv4_nat_traversal_probes_paused()
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
            "ipv4QntPausedDuring":[true,true], "ipv4QntPausedAfter":[false,false], "packets":packets,
            "observationRequests":pair.requests(),
        })
    );
    pair.close().await;
}

#[tokio::test]
#[ignore = "requires a disposable network namespace with IPv4 UDP packet filters"]
async fn enhanced_ipv4_leaves_ipv6_probing_on_the_original_idle_connection() {
    // Existing STUN responders return actual mappings. Kernel faults hold the
    // primary IPv4 path while the enhanced round executes on the original.
    let mut pair = Pair::bind_with(
        MaintenancePolicy {
            contact_timeout_ms: DEADLINE.as_millis() as u64,
            ..MaintenancePolicy::active()
        },
        true,
        |builder| builder.bind_addr((Ipv6Addr::LOCALHOST, 0)).unwrap(),
        None,
        true,
    )
    .await;
    let primary_v4 = pair.endpoints.each_ref().map(|endpoint| {
        *endpoint
            .addr()
            .ip_addrs()
            .find(|addr| addr.is_ipv4())
            .unwrap()
    });
    let _received_v4_blocked = udp_blackhole::UdpBlackhole::on_input(primary_v4);
    drop(pair._ipv4_blackhole.take());
    let [a, b] = &pair.connections;
    let ids = [a.stable_id(), b.stable_id()];
    let mut stream = open_flow(a, b).await;
    exchange(&mut stream, 0).await;
    assert!(
        pair.connections
            .iter()
            .all(|conn| matches!(conn.initial_data_path(), TransportAddr::Relay(_)))
    );
    let ipv4_probe = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let ipv4_target = ipv4_probe.local_addr().unwrap();
    let first_ipv4 = Instant::now();
    for (endpoint, connection) in pair.endpoints.iter().zip(&pair.connections) {
        endpoint
            .learn_ip_candidates(
                EndpointAddr::new(connection.remote_id()).with_ip_addr(ipv4_target),
            )
            .await;
    }
    time::timeout(DEADLINE, async {
        let mut sources = std::collections::BTreeSet::new();
        while sources.len() < 2 {
            sources.insert(ipv4_probe.recv_from(&mut [0; 1500]).await.unwrap().1);
        }
    })
    .await
    .expect("both originals queued an ordinary IPv4 probe before the enhanced pause");
    time::timeout(Duration::from_secs(3), async {
        while pair.requests().contains(&0) {
            time::sleep(Duration::from_millis(5)).await;
        }
        while !pair
            .connections
            .iter()
            .all(MemberConnection::test_ipv4_nat_traversal_probes_paused)
        {
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("both peers committed the enhanced round and an observer received real IPv4 traffic");
    let reports = time::timeout(DEADLINE, async {
        loop {
            if let [Some(left), Some(right)] = pair
                .connections
                .each_ref()
                .map(MemberConnection::udp_traversal)
                && left.round == right.round
                && [
                    left.trace.probes + left.trace.responses,
                    right.trace.probes + right.trace.responses,
                ]
                .into_iter()
                .all(|packets| packets > 0)
            {
                break [left, right];
            }
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("both originals started the same actual enhanced execution");
    let round = reports[0].round;
    eprintln!("ENHANCED_ACTUAL_ROUND reports={reports:?}");
    let release_deadline =
        reports.iter().map(|report| report.deadline).max().unwrap() + Duration::from_millis(100);
    assert!(
        pair.connections
            .iter()
            .all(MemberConnection::test_ipv4_nat_traversal_probes_paused)
    );
    exchange(&mut stream, 1).await;
    let reach_out = pair
        .connections
        .each_ref()
        .map(|connection| connection.stats().frame_tx.reach_out);
    let removed = pair
        .connections
        .each_ref()
        .map(|connection| connection.stats().frame_tx.remove_address);
    while ipv4_probe.try_recv_from(&mut [0; 1500]).is_ok() {}
    assert!(
        pair.connections.iter().all(|connection| {
            connection
                .paths()
                .iter()
                .any(|path| path.is_selected() && path.is_relay())
        }),
        "both address families remain unavailable while the original relay carries enhanced control"
    );

    let old_addresses = pair.endpoints.each_ref().map(|endpoint| {
        *endpoint
            .addr()
            .ip_addrs()
            .find(|addr| addr.is_ipv6())
            .unwrap()
    });
    // New real external UDP ports represent a later mapping without changing
    // interfaces or cancelling the unrelated IPv4 round's network generation.
    let mapped_left = UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
    let mapped_right = UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
    let addresses = [
        mapped_left.local_addr().unwrap(),
        mapped_right.local_addr().unwrap(),
    ];
    for (endpoint, mapped) in pair.endpoints.iter().zip(addresses) {
        endpoint.test_add_external_addr(mapped).await;
    }

    let mut probe = [0; 1200];
    time::timeout(Duration::from_secs(2), async {
        loop {
            if pair
                .connections
                .iter()
                .enumerate()
                .any(|(side, connection)| connection.stats().frame_tx.reach_out > reach_out[side])
            {
                break;
            }
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("late IPv6 starts a new QNT round before IPv4 enhanced finishes");
    assert!(
        pair.connections
            .iter()
            .all(MemberConnection::test_ipv4_nat_traversal_probes_paused)
    );
    assert_eq!(
        ipv4_probe.try_recv_from(&mut probe).unwrap_err().kind(),
        ErrorKind::WouldBlock
    );
    eprintln!("LATE_IPV6_QNT_DURING_IPV4_PAUSE original={ids:?} ipv6={addresses:?}");
    let forwarding = tokio::spawn(async move {
        let mut left_packet = [0; 65536];
        let mut right_packet = [0; 65536];
        loop {
            tokio::select! {
                received = mapped_left.recv_from(&mut left_packet) => {
                    let (length, _) = received.unwrap();
                    mapped_right.send_to(&left_packet[..length], old_addresses[0]).await.unwrap();
                }
                received = mapped_right.recv_from(&mut right_packet) => {
                    let (length, _) = received.unwrap();
                    mapped_left.send_to(&right_packet[..length], old_addresses[1]).await.unwrap();
                }
            }
        }
    });
    time::timeout(Duration::from_secs(2), async {
        loop {
            if pair.connections.iter().enumerate().all(|(side, conn)| {
                conn.paths().iter().any(|path| {
                    path.is_selected()
                        && path.remote_addr() == &TransportAddr::Ip(addresses[1 - side])
                        && path.stats().acknowledgment.is_some()
                })
            }) {
                break;
            }
            time::sleep(Duration::from_millis(15)).await;
        }
    })
    .await
    .expect("IPv6 must qualify and be selected on the idle original during enhanced IPv4");
    assert_eq!(
        pair.connections
            .each_ref()
            .map(|connection| connection.stats().frame_tx.remove_address),
        removed,
        "temporarily paused IPv4 candidates remain announced"
    );
    eprintln!("IDLE_IPV6_ENHANCED_READY original={ids:?} ipv6={addresses:?}");
    time::timeout_at(release_deadline, async {
        while pair
            .connections
            .iter()
            .any(MemberConnection::test_ipv4_nat_traversal_probes_paused)
        {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the original enhanced round releases its pause by its own fixed deadline");
    for connection in &pair.connections {
        assert_eq!(connection.udp_traversal().unwrap().round, round);
    }
    eprintln!(
        "ORDINARY_IPV4_PROBE_WINDOW target={ipv4_target} first={first_ipv4:?} resumed_after_ms={}",
        first_ipv4.elapsed().as_millis()
    );
    exchange(&mut stream, 2).await;
    time::sleep(Duration::from_millis(200)).await;
    assert!(
        pair.connections
            .iter()
            .enumerate()
            .all(|(side, connection)| {
                connection.paths().iter().any(|path| {
                    path.is_selected()
                        && path.remote_addr() == &TransportAddr::Ip(addresses[1 - side])
                })
            })
    );
    exchange(&mut stream, 1000).await;
    assert_eq!([a.stable_id(), b.stable_id()], ids);
    pair.close().await;
    forwarding.abort();
    let _ = forwarding.await;
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
