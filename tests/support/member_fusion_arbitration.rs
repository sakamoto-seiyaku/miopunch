//! The frozen contention input through normal fused member admission and streams.

use std::{
    io::ErrorKind,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
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
    member_path_lab::{DEADLINE, FLOOR, delayed_relay::delay_relay, observe_stun},
};

#[path = "udp_blackhole.rs"]
mod udp_blackhole;

#[path = "relay_contact_broker.rs"]
#[allow(dead_code)]
mod broker;

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
#[ignore = "requires a disposable network namespace for actual interface changes"]
async fn ipv6_prefix_change_preserves_ipv4_observation_and_original_stream() {
    use std::process::Command;
    fn ip(arguments: &[&str]) {
        assert!(Command::new("ip").args(arguments).status().unwrap().success());
    }
    ip(&["link", "add", "mio205", "type", "dummy"]);
    ip(&["link", "set", "mio205", "up"]);
    ip(&["-6", "addr", "add", "fd00:205::1/64", "dev", "mio205", "nodad"]);
    let pair = Pair::bind_with(
        MaintenancePolicy { quality_interval_ms: 30_000, ..MaintenancePolicy::active() },
        false,
        |builder| builder.bind_addr((Ipv6Addr::UNSPECIFIED, 0)).unwrap(),
        None,
        false,
    ).await;
    let ids = pair.connections.each_ref().map(MemberConnection::stable_id);
    let mut stream = open_flow(&pair.connections[0], &pair.connections[1]).await;
    exchange(&mut stream, 205).await;
    let plan = pair.connections[0].coordinate_udp_plan(
        Instant::now() + Duration::from_secs(8), &CancellationToken::new(),
    ).await.unwrap();
    assert!(plan.local_samples().iter().all(|sample| sample.mapped.is_ipv4()));
    ip(&["-6", "addr", "del", "fd00:205::1/64", "dev", "mio205"]);
    ip(&["-6", "addr", "add", "fd00:205:1::1/64", "dev", "mio205", "nodad"]);
    tokio::join!(pair.endpoints[0].network_change(), pair.endpoints[1].network_change());
    time::sleep(Duration::from_millis(400)).await;
    assert_eq!(pair.connections[0].udp_plan().map(|plan| plan.round), Some(plan.round),
        "the actual IPv4 binding and its observation survive an IPv6-only prefix change");
    exchange(&mut stream, 206).await;
    assert_eq!(pair.connections.each_ref().map(MemberConnection::stable_id), ids);
    pair.close().await;
}

#[tokio::test]
#[ignore = "requires a disposable network namespace for IPv6 withdrawal and UDP filters"]
async fn last_ipv6_candidate_is_withdrawn_while_ipv4_carries_the_original_stream() {
    fn ip(args: &[&str]) {
        assert!(
            std::process::Command::new("ip")
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    let ipv6: Ipv6Addr = "2001:db8:205::1".parse().unwrap();
    ip(&["link", "add", "mio205", "type", "dummy"]);
    ip(&["link", "set", "mio205", "addrgenmode", "none"]);
    ip(&["link", "set", "mio205", "up"]);
    ip(&["addr", "add", "192.0.2.1/24", "dev", "mio205"]);
    ip(&[
        "-6",
        "addr",
        "add",
        "2001:db8:205::1/64",
        "dev",
        "mio205",
        "nodad",
    ]);
    // Advertise both families on the relay before IPv4 becomes healthy.
    let mut pair = Pair::bind_with(
        MaintenancePolicy::active(),
        true,
        |builder| builder.bind_addr((Ipv6Addr::UNSPECIFIED, 0)).unwrap(),
        Some(&[]),
        true,
    )
    .await;
    let client = usize::from(!pair.connections[0].side().is_client());
    let server = 1 - client;
    let peer = &pair.connections[client];
    let advertised = *pair.endpoints[server]
        .addr()
        .ip_addrs()
        .find(|addr| addr.ip() == IpAddr::V6(ipv6))
        .unwrap();
    let mut flow = open_flow(peer, &pair.connections[server]).await;
    exchange(&mut flow, 1).await;
    time::timeout(DEADLINE, async {
        while !peer
            .test_remote_qnt_candidates()
            .unwrap()
            .contains(&advertised)
        {
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("peer received the server's IPv6 candidate over the original QUIC connection");

    drop(pair._ipv4_blackhole.take());
    let paths = time::timeout(Duration::from_secs(8), async {
        loop {
            let current = pair.connections.each_ref().map(|conn| {
                conn.paths()
                    .iter()
                    .find(|path| {
                        path.is_selected()
                            && matches!(path.remote_addr(), TransportAddr::Ip(addr) if addr.is_ipv4())
                            && path.stats().acknowledgment.is_some()
                    })
                    .map(|path| path.id())
            });
            if let [Some(left), Some(right)] = current {
                break [left, right];
            }
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("both ends selected a verified IPv4 path");
    let ipv4 = *pair.endpoints[server]
        .addr()
        .ip_addrs()
        .find(|addr| addr.is_ipv4())
        .unwrap();
    let before = peer.test_remote_qnt_candidates().unwrap();
    assert!(before.contains(&ipv4) && before.contains(&advertised));
    let removed_before = pair.connections[server].stats().frame_tx.remove_address;
    ip(&["-6", "addr", "del", "2001:db8:205::1/64", "dev", "mio205"]);
    time::timeout(DEADLINE, async {
        while pair.endpoints[server]
            .addr()
            .ip_addrs()
            .any(SocketAddr::is_ipv6)
        {
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("server withdrew its last local IPv6 candidate");
    let rx = [
        peer.test_observe_stream_receive(paths[client], &flow.0)
            .unwrap(),
        pair.connections[server]
            .test_observe_stream_receive(paths[server], &flow.1)
            .unwrap(),
    ];
    time::timeout(DEADLINE, exchange(&mut flow, 2))
        .await
        .unwrap();
    assert!(rx.into_iter().all(|bytes| bytes.confirm()));
    time::timeout(DEADLINE, async {
        while peer.test_remote_qnt_candidates().unwrap().contains(&advertised) {
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "peer retained {advertised} while the original IPv4 stream works; server REMOVE_ADDRESS frames {removed_before} -> {}, peer candidates {:?}",
            pair.connections[server].stats().frame_tx.remove_address,
            peer.test_remote_qnt_candidates().unwrap(),
        )
    });
    assert!(pair.connections[server].stats().frame_tx.remove_address > removed_before);
    assert!(peer.test_remote_qnt_candidates().unwrap().contains(&ipv4));
    pair.close().await;
}

#[tokio::test]
#[ignore = "requires a disposable network namespace for a late IPv6 interface address"]
async fn later_ipv6_window_reaches_the_original_member_with_healthy_ipv4() {
    fn ip(args: &[&str]) {
        assert!(std::process::Command::new("ip").args(args).status().unwrap().success());
    }
    ip(&["link", "add", "late205", "type", "dummy"]);
    ip(&["link", "set", "late205", "addrgenmode", "none"]);
    ip(&["link", "set", "late205", "up"]);
    ip(&["addr", "add", "192.0.2.1/24", "dev", "late205"]);
    let pair = Pair::bind_with(
        MaintenancePolicy::active(), false,
        |builder| {
            // The only reachable IPv6 address will sort after the first window.
            let mut builder = builder.transport_config(iroh::endpoint::QuicTransportConfig::builder()
                .max_remote_nat_traversal_addresses(8).build()).bind_addr((Ipv6Addr::UNSPECIFIED, 0)).unwrap();
            for port in 10000..10008 {
                builder = builder.external_addr(SocketAddr::from((Ipv6Addr::LOCALHOST, port)));
            }
            builder
        },
        Some(&[]), false,
    ).await;
    let client = usize::from(!pair.connections[0].side().is_client());
    let server = 1 - client;
    let original = &pair.connections[client];
    let mut flow = time::timeout(DEADLINE, open_flow(original, &pair.connections[server])).await.expect("open original IPv4 stream");
    time::timeout(DEADLINE, exchange(&mut flow, 1)).await.expect("initial IPv4 round trip");
    let v4 = original.paths().iter().find(|path| path.is_selected()
        && matches!(path.remote_addr(), TransportAddr::Ip(addr) if addr.is_ipv4())
        && path.stats().acknowledgment.is_some()).unwrap().id();
    ip(&["-6", "addr", "add", "2001:db8:205::1/64", "dev", "late205", "nodad"]);
    let ip: IpAddr = "2001:db8:205::1".parse().unwrap();
    let reachable = time::timeout(DEADLINE, async {
        loop {
            if let Some(addr) = pair.endpoints[server].addr().ip_addrs().find(|addr| addr.ip() == ip) {
                break *addr;
            }
            time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    time::timeout(Duration::from_secs(14), async {
        let mut announced = false;
        loop {
            announced |= original.test_remote_qnt_candidates().unwrap().contains(&reachable);
            if announced && original.paths().iter().any(|path| {
                path.remote_addr() == &TransportAddr::Ip(reachable) && path.stats().acknowledgment.is_some()
            }) { break; }
            time::sleep(Duration::from_millis(15)).await;
        }
    }).await.unwrap_or_else(|_| panic!("later IPv6 candidate {reachable} must be advertised and ACKed; peer={:?}, paths={:?}", original.test_remote_qnt_candidates().unwrap(), original.paths()));
    assert!(original.paths().get(v4).is_some());
    time::timeout(DEADLINE, exchange(&mut flow, 2)).await.unwrap();
    pair.close().await;
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
    for endpoint in &pair.endpoints {
        endpoint.test_disable_background_udp();
    }
    let [left, right] = &pair.connections;
    let originals = pair.connections.each_ref().map(MemberConnection::stable_id);
    let leader = usize::from(pair.endpoints[0].id() > pair.endpoints[1].id());
    let dry = CancellationToken::new();
    let dry_deadline = Instant::now() + DEADLINE;
    let winner = &pair.connections[leader];
    let yielder = &pair.connections[1 - leader];
    let barrier = Arc::new(Barrier::new(3));
    let reserved = pair.endpoints.each_ref().map(|endpoint| {
        endpoint.test_coordinate_udp_reservation(barrier.clone())
    });
    let (dry_cancelled, dry_yielded, (mut a, mut b)) = time::timeout_at(dry_deadline, async {
        let conflicting = CancellationToken::new();
        let (winner, yielder, flows) = tokio::join!(
            winner.coordinate_udp_traversal(dry_deadline, &dry),
            async {
                let result = yielder
                    .coordinate_udp_traversal(dry_deadline, &conflicting)
                    .await;
                dry.cancel();
                result
            },
            async {
                // Hold the actual manual reservations before business activity can
                // start an automatic improvement round on the same originals.
                while !reserved.iter().all(|reached| reached.load(Relaxed)) {
                    time::sleep(Duration::from_millis(5)).await;
                }
                let mut a = open_flow(left, right).await;
                let mut b = open_flow(right, left).await;
                tokio::join!(exchange(&mut a, 1), exchange(&mut b, 1));
                assert_eq!(pair.requests(), [0, 0]);
                barrier.wait().await;
                (a, b)
            },
        );
        (winner.unwrap_err().kind(), yielder.unwrap_err().kind(), flows)
    })
    .await
    .unwrap();
    let streams = [
        u64::from(a.0.streams().0.id()),
        u64::from(b.0.streams().0.id()),
    ];
    assert_ne!(streams[0], streams[1]);
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
    let work = time::timeout_at(deadline + Duration::from_millis(100), async {
        let ((), plans) = tokio::join!(
            async {
                let error = yielder
                    .coordinate_udp_traversal(deadline, &cancel)
                    .await
                    .unwrap_err();
                assert_ne!(error.kind(), ErrorKind::WouldBlock, "the manual round lost its slot");
            },
            plans(&pair.connections),
        );
        while !pair.connections.iter().all(|connection| {
            connection
                .udp_traversal()
                .is_some_and(|report| report.outcome.is_some())
        }) {
            time::sleep(Duration::from_millis(5)).await;
        }
        plans
    })
    .await
    .expect("crossed requests and the worked round fit the original three seconds");
    let settled = started.elapsed();
    stop.cancel();
    let (mut a, mut b, sequences) = time::timeout(FLOOR, pulse).await.unwrap().unwrap();
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
    enhanced_leaves_other_family_probing(false).await;
}

#[tokio::test]
#[ignore = "requires an isolated network namespace for dual-family UDP faults"]
async fn enhanced_ipv6_leaves_late_ipv4_probing_on_the_original_idle_connection() {
    enhanced_leaves_other_family_probing(true).await;
}

async fn enhanced_leaves_other_family_probing(enhanced_v6: bool) {
    use iroh_relay::ObservationFamily;
    let family = if enhanced_v6 {
        ObservationFamily::Ipv6
    } else {
        ObservationFamily::Ipv4
    };
    let primary_ip = if enhanced_v6 {
        IpAddr::V6(Ipv6Addr::LOCALHOST)
    } else {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    };
    let other_ip = if enhanced_v6 {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    } else {
        IpAddr::V6(Ipv6Addr::LOCALHOST)
    };
    let mut observers = JoinSet::new();
    let resources = if enhanced_v6 {
        Some(observe_ipv6_stun(&mut observers, Arc::default()).await)
    } else {
        None
    };
    // Existing STUN responders return actual mappings. Kernel faults hold the
    // primary IPv4 path while the enhanced round executes on the original.
    let mut pair = Pair::bind_with(
        MaintenancePolicy {
            contact_timeout_ms: DEADLINE.as_millis() as u64,
            ..MaintenancePolicy::active()
        },
        true,
        |builder| builder.bind_addr((Ipv6Addr::LOCALHOST, 0)).unwrap(),
        resources.as_deref(),
        true,
    )
    .await;
    let primary_addresses = pair.endpoints.each_ref().map(|endpoint| {
        *endpoint
            .addr()
            .ip_addrs()
            .find(|addr| addr.is_ipv6() == enhanced_v6)
            .unwrap()
    });
    let _received_primary_blocked = udp_blackhole::UdpBlackhole::on_input(primary_addresses);
    if enhanced_v6 {
        drop(pair.ipv6_blackhole.take());
    } else {
        drop(pair._ipv4_blackhole.take());
    }
    let [a, b] = &pair.connections;
    let ids = [a.stable_id(), b.stable_id()];
    let mut stream = open_flow(a, b).await;
    exchange(&mut stream, 0).await;
    assert!(
        pair.connections
            .iter()
            .all(|conn| matches!(conn.initial_data_path(), TransportAddr::Relay(_)))
    );
    let primary_probe = UdpSocket::bind((primary_ip, 0)).await.unwrap();
    let primary_target = primary_probe.local_addr().unwrap();
    let first_primary = Instant::now();
    for (endpoint, connection) in pair.endpoints.iter().zip(&pair.connections) {
        endpoint
            .learn_ip_candidates(
                EndpointAddr::new(connection.remote_id()).with_ip_addr(primary_target),
            )
            .await;
    }
    time::timeout(DEADLINE, async {
        let mut sources = std::collections::BTreeSet::new();
        while sources.len() < 2 {
            sources.insert(primary_probe.recv_from(&mut [0; 1500]).await.unwrap().1);
        }
    })
    .await
    .expect("both originals queued an ordinary enhanced-family probe before the enhanced pause");
    time::timeout(Duration::from_secs(3), async {
        while !pair
            .connections
            .iter()
            .all(|connection| connection.test_nat_traversal_probes_paused(family))
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
                .map(|connection| connection.udp_traversal_for_family(family))
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
            .all(|connection| connection.test_nat_traversal_probes_paused(family))
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
    while primary_probe.try_recv_from(&mut [0; 1500]).is_ok() {}
    assert!(
        pair.connections.iter().all(|connection| {
            connection
                .paths()
                .iter()
                .any(|path| path.is_selected() && path.is_relay())
        }),
        "both address families remain unavailable while the original relay carries enhanced control"
    );

    let old_other_addresses = pair.endpoints.each_ref().map(|endpoint| {
        *endpoint
            .addr()
            .ip_addrs()
            .find(|addr| addr.is_ipv6() != enhanced_v6)
            .unwrap()
    });
    // New real external UDP ports represent a later mapping without changing
    // interfaces or cancelling the unrelated IPv4 round's network generation.
    let mapped_left = UdpSocket::bind((other_ip, 0)).await.unwrap();
    let mapped_right = UdpSocket::bind((other_ip, 0)).await.unwrap();
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
    .expect("late other-family starts a new QNT round before IPv4 enhanced finishes");
    assert!(
        pair.connections
            .iter()
            .all(|connection| connection.test_nat_traversal_probes_paused(family))
    );
    assert_eq!(
        primary_probe.try_recv_from(&mut probe).unwrap_err().kind(),
        ErrorKind::WouldBlock
    );
    eprintln!(
        "LATE_OTHER_FAMILY_QNT_DURING_ENHANCED original={ids:?} enhanced={family:?} candidates={addresses:?}"
    );
    let forwarding = tokio::spawn(async move {
        let mut left_packet = [0; 65536];
        let mut right_packet = [0; 65536];
        loop {
            tokio::select! {
                received = mapped_left.recv_from(&mut left_packet) => {
                    let (length, _) = received.unwrap();
                    mapped_right.send_to(&left_packet[..length], old_other_addresses[0]).await.unwrap();
                }
                received = mapped_right.recv_from(&mut right_packet) => {
                    let (length, _) = received.unwrap();
                    mapped_left.send_to(&right_packet[..length], old_other_addresses[1]).await.unwrap();
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
    .expect("the other family must qualify and be selected on the idle original during enhanced paused-family");
    assert_eq!(
        pair.connections
            .each_ref()
            .map(|connection| connection.stats().frame_tx.remove_address),
        removed,
        "temporarily paused enhanced-family candidates remain announced"
    );
    eprintln!("OTHER_FAMILY_READY original={ids:?} enhanced={family:?} candidates={addresses:?}");
    time::timeout_at(release_deadline, async {
        while pair
            .connections
            .iter()
            .any(|connection| connection.test_nat_traversal_probes_paused(family))
        {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the original enhanced round releases its pause by its own fixed deadline");
    for connection in &pair.connections {
        assert_eq!(
            connection.udp_traversal_for_family(family).unwrap().round,
            round
        );
    }
    eprintln!(
        "ORDINARY_ENHANCED_FAMILY_PROBE_WINDOW target={primary_target} first={first_primary:?} resumed_after_ms={}",
        first_primary.elapsed().as_millis()
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
    observers.shutdown().await;
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

#[tokio::test]
#[ignore = "requires an isolated network namespace for a receive-side UDP fault"]
async fn busy_peer_releases_the_unsampled_exit_for_another_member() {
    time::timeout(Duration::from_secs(25), async {
        let root = tempfile::tempdir().unwrap();
        let mut a = NativeNetwork::create(root.path().join("a")).await.unwrap();
        let mut b = join(&mut a, &root.path().join("b")).await;
        let mut c = join(&mut a, &root.path().join("c")).await;
        let mut d = join(&mut a, &root.path().join("d")).await;
        for node in [&mut b, &mut c, &mut d] {
            for record in a.management_history().await.unwrap() {
                node.ingest_management_record(record.header(), record.body()).await.unwrap();
            }
        }
        let nodes = [a,b,c,d];
        let mut relay_config = iroh_relay::server::ServerConfig::default();
        relay_config.relay = Some(iroh_relay::server::RelayConfig::new((Ipv4Addr::LOCALHOST, 0)));
        let relay = iroh_relay::server::Server::spawn(relay_config).await.unwrap();
        let url = relay.http_url().unwrap();
        let counts: [Arc<AtomicUsize>; 4] = std::array::from_fn(|_| Arc::default());
        let mut observers = JoinSet::new();
        let mut endpoints = Vec::new();
        for (node, count) in nodes.iter().zip(&counts) {
            let mut config = node.resource_configuration().unwrap();
            let mut resources = observe_stun(&mut observers, count).await;
            resources.insert(0, Resource { url: "stun://[::1]:9/".into(), uses: [ResourceUse::Observation].into(), limitation: "wrong-family first observer must not suppress usable IPv4 observers".into() });
            resources.push(Resource { url: url.to_string(), uses: [ResourceUse::DataRelay].into(), limitation: "owned per-interface physical occupancy witness".into() });
            config.replacement = Some(resources);
            config.tcp_direct_disabled = true;
            config.maintenance.quality_interval_ms = 30_000;
            config.maintenance.contact_timeout_ms = 5_000;
            node.save_resource_configuration(&config).unwrap();
            endpoints.push(node.bind_member_fusion_endpoint_with(Endpoint::builder(presets::Minimal).clear_ip_transports().bind_addr((Ipv4Addr::LOCALHOST,0)).unwrap(), Duration::from_secs(5)).await.unwrap());
        }
        async fn connect(left: &MemberFusionEndpoint, right: &MemberFusionEndpoint, url: &iroh::RelayUrl) -> [MemberConnection;2] {
            let (a,b) = tokio::join!(left.connect(EndpointAddr::new(right.id()).with_relay_url(url.clone())), right.accept());
            [a.unwrap(), b.unwrap().unwrap()]
        }
        let ab = connect(&endpoints[0], &endpoints[1], &url).await;
        let bc = connect(&endpoints[1], &endpoints[2], &url).await;
        let ad = connect(&endpoints[0], &endpoints[3], &url).await;
        while endpoints.iter().any(|endpoint| !endpoint.test_building_peers().is_empty()) {
            time::sleep(Duration::from_millis(5)).await;
        }
        let barrier = Arc::new(Barrier::new(2));
        let reserved = endpoints[1].test_coordinate_udp_reservation(barrier);
        let cancel = CancellationToken::new();
        let mut held = tokio::spawn({ let connection = bc[0].clone(); let cancel=cancel.clone(); async move { connection.coordinate_udp_plan(Instant::now()+Duration::from_secs(8), &cancel).await } });
        tokio::select! {
            result = &mut held => panic!("B-C exited before the reservation barrier: {result:?}"),
            result = time::timeout(Duration::from_secs(3), async {
                while !reserved.load(std::sync::atomic::Ordering::Acquire) { time::sleep(Duration::from_millis(5)).await; }
            }) => result.expect("B-C reaches the unsampled reservation barrier"),
        }
        let before: Vec<_> = counts.iter().map(|count| count.load(Relaxed)).collect();
        let blocked = ab[0].coordinate_udp_plan(Instant::now()+Duration::from_secs(2), &CancellationToken::new()).await.unwrap_err();
        assert_eq!(blocked.kind(), ErrorKind::WouldBlock);
        assert_eq!(counts[0].load(Relaxed),before[0]);
        assert_eq!(counts[1].load(Relaxed),before[1]);
        let started = Instant::now();
        let plan = ad[0].coordinate_udp_plan(Instant::now()+Duration::from_secs(3), &CancellationToken::new()).await.unwrap();
        assert_eq!(plan.local_samples().len(),2);
        assert_eq!(counts[0].load(Relaxed)-before[0],2);
        assert_eq!(counts[3].load(Relaxed)-before[3],2);
        assert!(started.elapsed()<Duration::from_secs(3));
        cancel.cancel();
        assert_eq!(held.await.unwrap().unwrap_err().kind(),ErrorKind::Interrupted);
        let addresses=[&endpoints[0],&endpoints[3]].map(|endpoint| *endpoint.addr().ip_addrs().find(|address| address.is_ipv4()).unwrap());
        let raw_blocked=udp_blackhole::UdpBlackhole::on_input(addresses);
        let next_paths = ad.each_ref().map(|connection| connection.paths().iter().map(|path| path.id()).max().unwrap_or(noq_proto::PathId::ZERO).saturating_add(1u8));
        for (endpoint, connection) in [&endpoints[0], &endpoints[3]].into_iter().zip(&ad) {
            endpoint.test_limit_transport_paths(connection, vec![connection.initial_data_path().clone(), TransportAddr::Ip(addresses[0]), TransportAddr::Ip(addresses[1])]).await.unwrap();
            for path in connection.paths().iter().filter(|path| !path.is_relay()) { path.close().unwrap(); }
        }
        while !ad.iter().all(|connection| connection.paths().iter().any(|path| path.is_relay() && path.is_selected())) {
            time::sleep(Duration::from_millis(5)).await;
        }
        let mut stream=open_flow(&ad[0],&ad[1]).await;
        exchange(&mut stream,201).await;
        while endpoints.iter().any(|endpoint| !endpoint.test_building_peers().is_empty()) {
            time::sleep(Duration::from_millis(5)).await;
        }
        let originals = ad.each_ref().map(MemberConnection::stable_id);
        let suppressed = Arc::new(std::sync::atomic::AtomicBool::new(true));
        for (connection, next) in ad.iter().zip(next_paths) {
            connection.test_suppress_path_acks(next, suppressed.clone());
        }
        let mut traversal = tokio::spawn({ let connection=ad[0].clone(); async move { connection.coordinate_udp_traversal(Instant::now()+Duration::from_secs(6),&CancellationToken::new()).await } });
        tokio::select! {
            result = &mut traversal => panic!("A-D traversal ended before raw probes: {result:?}"),
            result = time::timeout(Duration::from_secs(2), async {
                while !ad.iter().all(|connection| connection.udp_traversal().is_some_and(|report| report.trace.probes>0 && report.physical_released.is_none())) { time::sleep(Duration::from_millis(5)).await; }
            }) => result.expect("both peers sampled and started real raw probes on the retained interface"),
        }
        let samples_while_running=counts[0].load(Relaxed);
        let waiting=ab[0].coordinate_udp_plan(Instant::now()+Duration::from_secs(1),&CancellationToken::new()).await.unwrap_err();
        assert_eq!(waiting.kind(),ErrorKind::WouldBlock);
        assert_eq!(counts[0].load(Relaxed),samples_while_running);
        drop(raw_blocked);
        time::timeout(Duration::from_secs(3),async {
            while !ad.iter().all(|connection| connection.udp_traversal().is_some_and(|report| report.physical_released.is_some())) { time::sleep(Duration::from_millis(5)).await; }
        }).await.expect("both peers retain a raw bidirectional winner before QUIC acknowledgment");
        assert!(ad.iter().all(|connection| connection.udp_traversal().unwrap().validated.is_none()));
        let sampled_before = counts[0].load(Relaxed);
        let next = ab[0].coordinate_udp_plan(Instant::now()+Duration::from_secs(2),&CancellationToken::new()).await.unwrap();
        assert_eq!(next.local_samples().len(),2);
        assert_eq!(counts[0].load(Relaxed)-sampled_before,2);
        assert!(ad.iter().all(|connection| connection.udp_traversal().unwrap().validated.is_none()));
        suppressed.store(false,std::sync::atomic::Ordering::Release);
        time::timeout(Duration::from_secs(3),async {
            while !ad.iter().all(|connection| connection.udp_traversal().is_some_and(|report| report.validated.is_some())) { time::sleep(Duration::from_millis(5)).await; }
        }).await.expect("original QUIC validates the retained winner after the next peer started");
        time::timeout(Duration::from_secs(2),async {
            while !ad.iter().all(|connection| {
                let report=connection.udp_traversal().unwrap();
                connection.paths().iter().any(|path| Some(path.id()) == report.validated.map(|(id,_)| id) && path.is_selected())
            }) { time::sleep(Duration::from_millis(5)).await; }
        }).await.expect("both original QUICs select the exact acknowledged winner before business confirmation");
        let round=ad[0].udp_traversal().unwrap().round;
        let left=&ad[0]; let right=&ad[1];
        let (a,b)=tokio::join!(
            left.confirm_udp_business(round,&mut stream.0,async |stream| {
                let (send,recv)=stream.streams(); send.write_all(b"after201").await.map_err(std::io::Error::other)?;
                let mut bytes=[0;8]; recv.read_exact(&mut bytes).await.map_err(std::io::Error::other)?; assert_eq!(&bytes,b"back-201"); Ok(())
            }),
            right.confirm_udp_business(round,&mut stream.1,async |stream| {
                let (send,recv)=stream.streams(); let mut bytes=[0;8]; recv.read_exact(&mut bytes).await.map_err(std::io::Error::other)?; assert_eq!(&bytes,b"after201"); send.write_all(b"back-201").await.map_err(std::io::Error::other)?; Ok(())
            })
        ); a.unwrap();b.unwrap();
        traversal.await.unwrap().unwrap();
        assert_eq!(ad.each_ref().map(MemberConnection::stable_id),originals);
        eprintln!("PHYSICAL_BUSY_YIELDED originals={originals:?} another_peer_sampled=true samples=2 next_sampled_before_quic_ack=true original_stream_continues=true");
        drop(stream);
        for endpoint in endpoints { endpoint.close().await; }
        for node in nodes { node.close().await; }
        observers.shutdown().await;
        relay.shutdown().await.unwrap();
    }).await.expect("busy unsampled peer leaves the interface available to a ready member");
}

async fn observe_ipv6_stun(
    observers: &mut JoinSet<()>,
    received: Arc<AtomicUsize>,
) -> Vec<Resource> {
    let mut resources = Vec::new();
    for _ in 0..2 {
        let socket = UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
        resources.push(Resource {
            url: format!("stun://{}/", socket.local_addr().unwrap()),
            uses: [ResourceUse::Observation].into(),
            limitation: "real IPv6 mapping observer".into(),
        });
        let received = received.clone();
        observers.spawn(async move {
            loop {
                let mut request = [0; 1500];
                let (length, source) = socket.recv_from(&mut request).await.unwrap();
                assert_eq!(length, 20);
                received.fetch_add(1, Relaxed);
                let SocketAddr::V6(source) = source else {
                    panic!("IPv6 request")
                };
                let mut reply = vec![1, 1, 0, 24, 0x21, 0x12, 0xa4, 0x42];
                reply.extend_from_slice(&request[8..20]);
                reply.extend_from_slice(&[0, 0x20, 0, 20, 0, 2]);
                reply.extend_from_slice(&(source.port() ^ 0x2112).to_be_bytes());
                for (address, mask) in source.ip().octets().iter().zip(&request[4..20]) {
                    reply.push(address ^ mask);
                }
                socket.send_to(&reply, source).await.unwrap();
            }
        });
    }
    resources
}

#[tokio::test]
#[ignore = "requires an isolated network namespace for a receive-side IPv6 fault"]
async fn ipv6_enhanced_uses_the_original_member_binding_and_stream() {
    time::timeout(Duration::from_secs(20), async {
        let mut observers = JoinSet::new();
        let resources = observe_ipv6_stun(&mut observers, Arc::default()).await;
        let pair = Pair::bind_with(
            MaintenancePolicy {
                quality_interval_ms: 30_000,
                ..MaintenancePolicy::active()
            },
            true,
            |builder| {
                builder
                    .clear_ip_transports()
                    .bind_addr((Ipv6Addr::LOCALHOST, 0))
                    .unwrap()
            },
            Some(&resources),
            false,
        )
        .await;
        let plan = pair.connections[0]
            .coordinate_udp_plan(
                Instant::now() + Duration::from_secs(4),
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(plan.local_samples().len(), 2);
        assert!(
            plan.local_samples()
                .iter()
                .all(|sample| sample.destination.is_ipv6() && sample.mapped.is_ipv6())
        );
        let ids = pair.connections.each_ref().map(MemberConnection::stable_id);
        let mut stream = open_flow(&pair.connections[0], &pair.connections[1]).await;
        exchange(&mut stream, 202).await;
        while pair
            .endpoints
            .iter()
            .any(|endpoint| !endpoint.test_building_peers().is_empty())
        {
            time::sleep(Duration::from_millis(5)).await;
        }
        let addresses = pair.endpoints.each_ref().map(|endpoint| {
            *endpoint
                .addr()
                .ip_addrs()
                .find(|addr| addr.is_ipv6())
                .unwrap()
        });
        let fault = udp_blackhole::UdpBlackhole::on_input(addresses);
        let suppressed = Arc::new(std::sync::atomic::AtomicBool::new(true));
        for (endpoint, connection) in pair.endpoints.iter().zip(&pair.connections) {
            let next = connection
                .paths()
                .iter()
                .map(|path| path.id())
                .max()
                .unwrap()
                .saturating_add(1u8);
            connection.test_suppress_path_acks(next, suppressed.clone());
            endpoint
                .test_limit_transport_paths(
                    connection,
                    vec![
                        connection.initial_data_path().clone(),
                        TransportAddr::Ip(addresses[0]),
                        TransportAddr::Ip(addresses[1]),
                    ],
                )
                .await
                .unwrap();
            for path in connection.paths().iter().filter(|path| !path.is_relay()) {
                path.close().unwrap();
            }
        }
        let task = tokio::spawn({
            let connection = pair.connections[0].clone();
            async move {
                connection
                    .coordinate_udp_traversal(
                        Instant::now() + Duration::from_secs(8),
                        &CancellationToken::new(),
                    )
                    .await
            }
        });
        time::timeout(Duration::from_secs(3), async {
            while !pair.connections.iter().all(|connection| {
                connection.udp_traversal().is_some_and(|report| {
                    report.trace.probes > 0 && report.physical_released.is_none()
                })
            }) {
                time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("both real IPv6 raw phases start while primary receive is blocked");
        exchange(&mut stream, 203).await;
        drop(fault);
        time::timeout(Duration::from_secs(3), async {
            while !pair.connections.iter().all(|connection| {
                connection
                    .udp_traversal()
                    .is_some_and(|report| report.physical_released.is_some())
            }) {
                time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("both retain the actual raw winner before original QUIC acknowledgment");
        assert!(
            pair.connections.iter().all(|connection| connection
                .udp_traversal()
                .unwrap()
                .validated
                .is_none())
        );
        suppressed.store(false, std::sync::atomic::Ordering::Release);
        time::timeout(Duration::from_secs(3), async {
            while !pair.connections.iter().all(|connection| {
                connection.udp_traversal().is_some_and(|report| {
                    report.validated.is_some_and(|(id, _)| {
                        connection
                            .paths()
                            .get(id)
                            .is_some_and(|path| path.is_selected())
                    })
                })
            }) {
                time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("both original QUICs acknowledge and select their exact raw IPv6 winner");
        let round = pair.connections[0].udp_traversal().unwrap().round;
        let (left, right) = (&pair.connections[0], &pair.connections[1]);
        let (a, b) = tokio::join!(
            left.confirm_udp_business(round, &mut stream.0, async |stream| {
                let (send, recv) = stream.streams();
                send.write_all(b"ipv6-202")
                    .await
                    .map_err(std::io::Error::other)?;
                let mut bytes = [0; 8];
                recv.read_exact(&mut bytes)
                    .await
                    .map_err(std::io::Error::other)?;
                assert_eq!(&bytes, b"back-202");
                Ok(())
            }),
            right.confirm_udp_business(round, &mut stream.1, async |stream| {
                let (send, recv) = stream.streams();
                let mut bytes = [0; 8];
                recv.read_exact(&mut bytes)
                    .await
                    .map_err(std::io::Error::other)?;
                assert_eq!(&bytes, b"ipv6-202");
                send.write_all(b"back-202")
                    .await
                    .map_err(std::io::Error::other)?;
                Ok(())
            })
        );
        a.unwrap();
        b.unwrap();
        let executed = task.await.unwrap().unwrap();
        time::timeout_at(executed.deadline, async {
            while !pair.connections.iter().all(|connection| {
                connection
                    .udp_traversal()
                    .is_some_and(|report| report.finished.is_some())
            }) {
                time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("both original round reports settle within their existing deadline");
        for (side, connection) in pair.connections.iter().enumerate() {
            let report = connection.udp_traversal().unwrap();
            let phase = if side == 0 {
                &executed.local_phase
            } else {
                &executed.peer_phase
            };
            assert!(
                report.trace.probes <= phase.peer_datagram_bound(report.trace.listener_bound + 1)
            );
            assert!(report.trace.listener_attempted <= 256);
            assert!(report.finished.unwrap() <= report.deadline.into_std());
        }
        assert_eq!(
            pair.connections.each_ref().map(MemberConnection::stable_id),
            ids
        );
        eprintln!("IPV6_ENHANCED_ORIGINAL ids={ids:?} samples=2 original_stream=true");
        drop(stream);
        pair.close().await;
        observers.shutdown().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "requires an isolated network namespace for a receive-side IPv6 fault"]
async fn restricted_contact_qualifies_its_original_with_enhanced_ipv6() {
    use iroh::endpoint::MemberDialOptions;
    use miopunch_native_probe::mqtt_transport::MqttTransport;
    time::timeout(Duration::from_secs(20), async {
        let directory = tempfile::tempdir().unwrap();
        let mut left = NativeNetwork::create(directory.path().join("left")).await.unwrap();
        let right = join(&mut left, &directory.path().join("right")).await;
        let nodes=[left,right];
        let broker=broker::Broker::new().await;
        let mut observers=JoinSet::new();
        let received=Arc::new(AtomicUsize::new(0));
        let mut resources=observe_ipv6_stun(&mut observers,received.clone()).await;
        resources.push(Resource { url:format!("mqtt://{}/",broker.address), uses:[ResourceUse::Signaling].into(), limitation:"restricted control only".into() });
        let mut endpoints=Vec::new();
        let mut carriers=Vec::new();
        for node in &nodes {
            let mut config=node.resource_configuration().unwrap();
            config.replacement=Some(resources.clone());
            config.tcp_direct_disabled=true;
            config.udp_direct_disabled=false;
            config.member_data_relay_disabled=true;
            config.maintenance.contact_timeout_ms=12_000;
            node.save_resource_configuration(&config).unwrap();
            let carrier=Arc::new(MqttTransport::new(broker.address,Arc::default()));
            let endpoint=node.bind_member_fusion_endpoint_with(Endpoint::builder(presets::Minimal).clear_ip_transports().bind_addr((Ipv6Addr::LOCALHOST,0)).unwrap().add_custom_transport(carrier.clone()),Duration::from_secs(12)).await.unwrap();
            carrier.wait_ready().await.unwrap();
            endpoints.push(endpoint);carriers.push(carrier);
        }
        let addresses=std::array::from_fn(|side| *endpoints[side].addr().ip_addrs().find(|addr| addr.is_ipv6()).unwrap());
        let fault=udp_blackhole::UdpBlackhole::on_input(addresses);
        let release=tokio::spawn(async move {
            time::timeout(Duration::from_secs(8),async { while received.load(Relaxed)<4 { time::sleep(Duration::from_millis(5)).await; } }).await.expect("both restricted originals observe their actual IPv6 binding before ordinary data is available");
            drop(fault);
        });
        let identity=Arc::new(AtomicUsize::new(0));
        let recorded=identity.clone();
        let contact=MqttTransport::address(&carriers[1].route().unwrap()).unwrap();
        let options=MemberDialOptions::default().with_restricted_contacts(vec![contact]).with_route_diagnostics(|_,_| ((1,1), |_|{})).with_identity_diagnostics(move |_,_,id| { recorded.store(id,Relaxed); });
        let deadline=Instant::now()+Duration::from_secs(12);
        let cancel = CancellationToken::new();
        let (left,right)=tokio::join!(endpoints[0].connect_with_routes_until(EndpointAddr::new(endpoints[1].id()), options,deadline,&cancel), endpoints[1].accept());
        let connections=[left.unwrap(),right.unwrap().unwrap()];
        release.await.unwrap();
        assert_eq!(connections[0].stable_id(),identity.load(Relaxed),"first business retains its authenticated restricted QUIC original");
        for connection in &connections {
            let report=connection.udp_traversal_for_family(iroh_relay::ObservationFamily::Ipv6).expect("enhanced IPv6 qualified the restricted original");
            assert!(report.physical_released.is_some());
            let (id,_)=report.validated.unwrap();
            let winner=report.winner.unwrap();
            assert!(winner.remote.is_ipv6() && winner.local_addr.is_ipv6());
            assert!(connection.paths().get(id).is_some_and(|path| path.is_selected() && path.remote_addr()==&TransportAddr::Ip(winner.remote) && path.local_socket_id()==Some(winner.local_socket_id)));
            assert!(matches!(connection.initial_data_path(),TransportAddr::Ip(addr) if addr.is_ipv6()));
        }
        let mut stream=open_flow(&connections[0],&connections[1]).await;
        exchange(&mut stream,202_001).await;
        eprintln!("RESTRICTED_ENHANCED_IPV6 original={} first_business=true",identity.load(Relaxed));
        drop(stream);
        for endpoint in endpoints { endpoint.close().await; }
        for node in nodes { node.close().await; }
        observers.shutdown().await;
        broker.close().await;
    }).await.expect("bounded restricted first data through enhanced IPv6");
}
