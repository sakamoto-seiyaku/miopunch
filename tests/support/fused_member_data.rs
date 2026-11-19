use std::{
    net::Ipv4Addr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering::Relaxed},
    },
    time::{Duration, Instant},
};

use iroh::{
    Endpoint, EndpointAddr, RelayUrl, TransportAddr,
    endpoint::{
        BeforeConnectOutcome, EndpointHooks, LocalTransportAddr, MemberConnection,
        MemberDialOptions, MemberFusionEndpoint, MemberStream, RestrictedEndpoint, presets,
    },
};
use iroh_relay::server::{RelayConfig, Server, ServerConfig};
use miopunch_native_probe::{
    native_network::member_fusion_attempt_id,
    resources::{Resource, ResourceUse},
};
use tokio::time::{self, timeout};
use tokio_util::sync::CancellationToken;

use super::{
    BUSINESS, LEASE, MemberDataRelay, MemberDataTransport, NativeNetwork, SignalingLease, connect,
    endpoint, nodes,
};

#[path = "delayed_relay.rs"]
mod delayed_relay;
#[cfg(target_os = "linux")]
#[path = "udp_blackhole.rs"]
mod udp_blackhole;

const FUSION_BUDGET: Duration = Duration::from_secs(8);
const RTT_MARGIN: Duration = Duration::from_millis(5);

#[derive(Clone, Debug, Default)]
struct FusedDials(Arc<AtomicUsize>);

impl EndpointHooks for FusedDials {
    async fn before_connect(&self, _: &EndpointAddr, _: &[u8]) -> BeforeConnectOutcome {
        self.0.fetch_add(1, Relaxed);
        BeforeConnectOutcome::Accept
    }
}

struct FusedRelay {
    _nodes: (
        tempfile::TempDir,
        NativeNetwork,
        NativeNetwork,
        NativeNetwork,
    ),
    endpoints: [MemberFusionEndpoint; 2],
    addresses: [TransportAddr; 2],
    links: [RelayLink; 2],
    hub: RestrictedEndpoint,
    forwarder: MemberDataRelay,
    dials: [FusedDials; 2],
}

struct RelayLink {
    endpoint: RestrictedEndpoint,
    #[cfg(feature = "nat-lab-test-hooks")]
    outer: (iroh::endpoint::Connection, iroh::endpoint::Connection),
    #[cfg(feature = "nat-lab-test-hooks")]
    spares: std::vec::IntoIter<Arc<MemberDataTransport>>,
    leases: [SignalingLease; 2],
}

impl FusedRelay {
    async fn bind(relays: &[RelayUrl], udp_direct: bool) -> Self {
        Self::bind_with_registrations(relays, udp_direct, [1, 1]).await
    }

    async fn bind_with_registrations(
        relays: &[RelayUrl],
        udp_direct: bool,
        registrations: [usize; 2],
    ) -> Self {
        let nodes = nodes().await;
        let (_, a, b, c) = &nodes;
        let hub = endpoint(b, None).await;
        let forwarder = MemberDataRelay::new(b.admission_gate());
        forwarder.set_enabled(true);
        let dials = [FusedDials::default(), FusedDials::default()];
        let bind = |node, dials, registrations| {
            Self::bind_side(
                node,
                &hub,
                &forwarder,
                dials,
                relays,
                udp_direct,
                registrations,
            )
        };
        let (left, right) = (
            bind(a, dials[0].clone(), registrations[0]).await,
            bind(c, dials[1].clone(), registrations[1]).await,
        );
        let (a, a_address, a_link) = left;
        let (c, c_address, c_link) = right;
        assert_eq!(forwarder.active_links(), 2);
        Self {
            _nodes: nodes,
            endpoints: [a, c],
            addresses: [a_address, c_address],
            links: [a_link, c_link],
            hub,
            forwarder,
            dials,
        }
    }

    async fn bind_side(
        node: &NativeNetwork,
        hub: &RestrictedEndpoint,
        forwarder: &MemberDataRelay,
        dials: FusedDials,
        relays: &[RelayUrl],
        udp_direct: bool,
        registrations: usize,
    ) -> (MemberFusionEndpoint, TransportAddr, RelayLink) {
        let link = endpoint(node, None).await;
        let (device, hub_link) = connect(&link, hub, BUSINESS).await;
        assert_eq!(device.remote_id(), hub.id());
        assert_eq!(hub_link.remote_id(), link.id());
        let adapters: Vec<_> = (0..registrations)
            .map(|_| Arc::new(MemberDataTransport::new(hub.id(), node.admission_gate())))
            .collect();
        let mut config = node.resource_configuration().unwrap();
        config.tcp_direct_disabled = true;
        config.udp_direct_disabled = !udp_direct;
        config.replacement = Some(
            relays
                .iter()
                .map(|url| Resource {
                    url: url.to_string(),
                    uses: [ResourceUse::DataRelay].into(),
                    limitation: "owned loopback relay for both admitted members".into(),
                })
                .collect(),
        );
        node.save_resource_configuration(&config).unwrap();
        let mut builder = Endpoint::builder(presets::Minimal)
            .clear_ip_transports()
            .bind_addr((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .hooks(dials);
        for adapter in &adapters {
            builder = builder.add_custom_transport(adapter.clone());
        }
        let fused = node
            .bind_member_fusion_endpoint_with(builder, FUSION_BUDGET)
            .await
            .unwrap();
        let mut adapters = adapters.into_iter();
        let (address, leases) = attach_member_relay(
            &fused,
            forwarder,
            &device,
            &hub_link,
            adapters.next().unwrap(),
        )
        .await;
        let link = RelayLink {
            endpoint: link,
            #[cfg(feature = "nat-lab-test-hooks")]
            outer: (device, hub_link),
            #[cfg(feature = "nat-lab-test-hooks")]
            spares: adapters,
            leases,
        };
        (fused, address, link)
    }

    #[cfg(feature = "nat-lab-test-hooks")]
    async fn rotate_registration(&mut self, side: usize) {
        let link = &mut self.links[side];
        let [device, forwarder] = &mut link.leases;
        tokio::join!(device.close(), forwarder.close());
        assert_eq!(self.forwarder.active_links(), 1);
        let previous = self.addresses[side].clone();
        let (address, leases) = attach_member_relay(
            &self.endpoints[side],
            &self.forwarder,
            &link.outer.0,
            &link.outer.1,
            link.spares.next().unwrap(),
        )
        .await;
        assert_ne!(address, previous);
        assert!(!self.endpoints[side].addr().addrs.contains(&previous));
        assert_eq!(self.forwarder.active_links(), 2);
        link.leases = leases;
        self.addresses[side] = address;
    }

    async fn connect(&self, route: &TransportAddr) -> [MemberConnection; 2] {
        let [client, server] = &self.endpoints;
        let target = EndpointAddr::new(server.id()).with_addrs([route.clone()]);
        let deadline = tokio::time::Instant::now() + FUSION_BUDGET;
        let cancel = CancellationToken::new();
        let (outgoing, incoming) = tokio::time::timeout_at(deadline, async {
            tokio::try_join!(
                client.connect_with_routes_until(target, Default::default(), deadline, &cancel),
                async {
                    server
                        .accept()
                        .await
                        .unwrap()
                        .map_err(std::io::Error::other)
                },
            )
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(outgoing.remote_id(), server.id());
        assert_eq!(incoming.remote_id(), client.id());
        assert_eq!(
            member_fusion_attempt_id(&outgoing).unwrap(),
            member_fusion_attempt_id(&incoming).unwrap()
        );
        [outgoing, incoming]
    }

    async fn reuse(&self, side: usize, candidate: &TransportAddr, original: &MemberConnection) {
        let target = EndpointAddr::new(original.remote_id()).with_addrs([candidate.clone()]);
        let reused = self.endpoints[side].connect(target).await.unwrap();
        assert_eq!(reused.stable_id(), original.stable_id());
    }

    async fn online(&self) {
        timeout(FUSION_BUDGET, async {
            tokio::join!(self.endpoints[0].online(), self.endpoints[1].online())
        })
        .await
        .unwrap();
    }

    async fn close(self) {
        assert_eq!(
            self.dials.each_ref().map(|dials| dials.0.load(Relaxed)),
            [1, 0]
        );
        assert_eq!(self.forwarder.active_links(), 2);
        let [left, right] = self.links;
        drop([left.leases, right.leases]);
        tokio::join!(
            self.endpoints[0].close(),
            self.endpoints[1].close(),
            self.hub.close(),
            left.endpoint.close(),
            right.endpoint.close()
        );
    }
}

async fn attach_member_relay(
    fused: &MemberFusionEndpoint,
    forwarder: &MemberDataRelay,
    device: &iroh::endpoint::Connection,
    hub: &iroh::endpoint::Connection,
    adapter: Arc<MemberDataTransport>,
) -> (TransportAddr, [SignalingLease; 2]) {
    let (client, server) = tokio::join!(
        adapter.attach(device, LEASE),
        forwarder.accept_link(hub, LEASE)
    );
    let leases = [client.unwrap(), server.unwrap()];
    let address = TransportAddr::Custom(adapter.address().unwrap());
    timeout(FUSION_BUDGET, async {
        while !fused.addr().addrs.contains(&address) {
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("attached member data carrier is available to announce to its peer");
    (address, leases)
}

async fn fused_exchange(connections: &[MemberConnection; 2]) {
    let [client, server] = connections;
    tokio::join!(
        async {
            let mut stream = client.open_bi().await.unwrap();
            let (send, recv) = stream.streams();
            send.write_all(b"private A-C business").await.unwrap();
            send.finish().unwrap();
            assert_eq!(recv.read_to_end(64).await.unwrap(), b"private ACK");
        },
        async {
            let mut stream = server.accept_bi().await.unwrap();
            let (send, recv) = stream.streams();
            assert_eq!(recv.read_to_end(64).await.unwrap(), b"private A-C business");
            send.write_all(b"private ACK").await.unwrap();
            send.finish().unwrap();
            send.stopped().await.unwrap();
        },
    );
}

#[tokio::test]
async fn fused_member_data_relay_carries_admitted_business() {
    let topology = FusedRelay::bind(&[], false).await;
    let connections = topology.connect(&topology.addresses[1]).await;
    let attempt = member_fusion_attempt_id(&connections[0]).unwrap();
    #[cfg(feature = "member-signaling-test-hooks")]
    let forwarded_before = topology.forwarder.test_counts().0;
    let started = std::time::Instant::now();
    tokio::time::timeout(FUSION_BUDGET, fused_exchange(&connections))
        .await
        .unwrap();
    for (side, connection) in connections.iter().enumerate() {
        let remote = &topology.addresses[1 - side];
        let TransportAddr::Custom(local) = &topology.addresses[side] else {
            unreachable!()
        };
        assert_eq!(connection.initial_data_path(), remote);
        let selected = connection.paths();
        let path = selected.iter().find(|path| path.is_selected()).unwrap();
        assert_eq!(path.remote_addr(), remote);
        if side == 1 {
            assert_eq!(
                path.local_addr(),
                &LocalTransportAddr::Custom(Some(local.clone()))
            );
        }
        assert!(path.stats().acknowledgment.is_some());
        assert!(path.stats().frame_rx.stream > 0);
        eprintln!(
            "MIO153 member bootstrap side={side} stable={} attempt={attempt:02x?} elapsed_ms={} transport={} stream_frames={} received_bytes={}",
            connection.stable_id(),
            started.elapsed().as_millis(),
            local.id(),
            path.stats().frame_rx.stream,
            path.stats().udp_rx.bytes,
        );
    }
    #[cfg(feature = "member-signaling-test-hooks")]
    {
        let forwarded = topology.forwarder.test_counts().0 - forwarded_before;
        assert!(forwarded > 0);
        eprintln!(
            "MIO153 forwarder={} forwarded_during_business={forwarded}",
            topology.hub.id()
        );
    }
    topology.close().await;
}

async fn native_relay() -> Server {
    let mut config = ServerConfig::default();
    config.relay = Some(RelayConfig::new((Ipv4Addr::LOCALHOST, 0)));
    Server::spawn(config).await.unwrap()
}

#[tokio::test]
async fn fused_member_data_can_win_against_a_slower_native_relay_on_the_original_stream() {
    let native = native_relay().await;
    let (url, proxy) =
        delayed_relay::delay_relay(native.http_addr().unwrap(), Duration::from_millis(65)).await;
    let relay = TransportAddr::Relay(url.clone());
    let topology = FusedRelay::bind(std::slice::from_ref(&url), false).await;
    topology.online().await;
    let connections = topology.connect(&relay).await;
    assert!(
        connections
            .iter()
            .all(|connection| connection.initial_data_path() == &relay)
    );
    let ids = connections.each_ref().map(MemberConnection::stable_id);
    timeout(FUSION_BUDGET, async {
        let mut stream = OriginalStream::open(&connections[0], &connections[1]).await;
        #[cfg(feature = "nat-lab-test-hooks")]
        let suppressed = {
            let suppressed = Arc::new(std::sync::atomic::AtomicBool::new(true));
            for connection in &connections {
                connection
                    .test_suppress_path_acks(noq_proto::PathId::from(1u8), suppressed.clone());
            }
            suppressed
        };
        topology
            .reuse(0, &topology.addresses[1], &connections[0])
            .await;
        #[cfg(feature = "nat-lab-test-hooks")]
        {
            let path_id = noq_proto::PathId::from(1u8);
            while !connections.iter().all(|connection| {
                connection
                    .paths()
                    .get(path_id)
                    .is_some_and(|path| path.stats().frame_rx.path_response > 0)
            }) {
                stream.round_trip().await;
            }
            stream.round_trip().await;
            for (side, connection) in connections.iter().enumerate() {
                let paths = connection.paths();
                let candidate = paths.get(path_id).unwrap();
                assert_eq!(candidate.remote_addr(), &topology.addresses[1 - side]);
                assert!(!candidate.is_selected());
                assert!(candidate.stats().acknowledgment.is_none());
                assert_eq!(candidate.stats().frame_rx.stream, 0);
                assert_eq!(candidate.stats().frame_tx.stream, 0);
                assert!(
                    paths
                        .iter()
                        .any(|path| path.is_selected() && path.remote_addr() == &relay)
                );
            }
        }
        let since = Instant::now();
        #[cfg(feature = "nat-lab-test-hooks")]
        suppressed.store(false, Relaxed);
        let member = topology.addresses.each_ref();
        verify_selection(
            &connections,
            &mut stream,
            [member[1], member[0]],
            [&relay, &relay],
            since,
            true,
        )
        .await;
    })
    .await
    .expect("confirmed member backup wins on the original stream");
    assert_eq!(connections.each_ref().map(MemberConnection::stable_id), ids);
    topology.close().await;
    proxy.cancel();
    native.shutdown().await.unwrap();
}

#[tokio::test]
async fn fused_native_relay_can_win_against_a_slower_member_data_backup() {
    let native = native_relay().await;
    let url = native.http_url().unwrap();
    let relay = TransportAddr::Relay(url.clone());
    let topology = FusedRelay::bind(std::slice::from_ref(&url), false).await;
    topology.online().await;
    let connections = topology.connect(&topology.addresses[1]).await;
    let member = topology.addresses.each_ref();
    assert_eq!(connections[0].initial_data_path(), member[1]);
    assert_eq!(connections[1].initial_data_path(), member[0]);
    let ids = connections.each_ref().map(MemberConnection::stable_id);
    timeout(FUSION_BUDGET, async {
        let mut stream = OriginalStream::open(&connections[0], &connections[1]).await;
        let since = Instant::now();
        topology.reuse(0, &relay, &connections[0]).await;
        verify_selection(
            &connections,
            &mut stream,
            [&relay, &relay],
            [member[1], member[0]],
            since,
            true,
        )
        .await;
    })
    .await
    .expect("faster native relay wins on the original member relay stream");
    assert_eq!(connections.each_ref().map(MemberConnection::stable_id), ids);
    topology.close().await;
    native.shutdown().await.unwrap();
}

#[tokio::test]
async fn accepting_side_can_supply_the_member_relay_on_the_original_stream() {
    let native = native_relay().await;
    let (url, proxy) =
        delayed_relay::delay_relay(native.http_addr().unwrap(), Duration::from_millis(65)).await;
    let relay = TransportAddr::Relay(url.clone());
    let topology = FusedRelay::bind(std::slice::from_ref(&url), false).await;
    topology.online().await;
    let connections = topology.connect(&relay).await;
    let ids = connections.each_ref().map(MemberConnection::stable_id);
    timeout(FUSION_BUDGET, async {
        let mut stream = OriginalStream::open(&connections[1], &connections[0]).await;
        let since = Instant::now();
        let member = topology.addresses.each_ref();
        topology.reuse(1, member[0], &connections[1]).await;
        verify_selection(
            &connections,
            &mut stream,
            [member[1], member[0]],
            [&relay, &relay],
            since,
            true,
        )
        .await;
    })
    .await
    .expect("accepting peer's contact hint enables the original connection to use member relay");
    assert_eq!(connections.each_ref().map(MemberConnection::stable_id), ids);
    topology.close().await;
    proxy.cancel();
    native.shutdown().await.unwrap();
}

#[tokio::test]
async fn reaches_a_third_native_relay_after_member_relay_bootstrap() {
    let native = native_relay().await;
    let url: RelayUrl = format!("http://localhost:{}/", native.http_addr().unwrap().port())
        .parse()
        .unwrap();
    let refused = [(); 2].map(|_| {
        let socket = tokio::net::TcpSocket::new_v4().unwrap();
        socket.bind((Ipv4Addr::LOCALHOST, 0).into()).unwrap();
        socket
    });
    let relay_urls: [RelayUrl; 3] = [
        format!("http://{}/", refused[0].local_addr().unwrap())
            .parse()
            .unwrap(),
        format!("http://{}/", refused[1].local_addr().unwrap())
            .parse()
            .unwrap(),
        url.clone(),
    ];
    let relay = TransportAddr::Relay(url);
    let ordered = EndpointAddr::new(iroh::SecretKey::from_bytes(&[1; 32]).public())
        .with_addrs(relay_urls.iter().cloned().map(TransportAddr::Relay));
    assert_eq!(ordered.addrs.last(), Some(&relay));
    for socket in &refused {
        assert_eq!(
            tokio::net::TcpStream::connect(socket.local_addr().unwrap())
                .await
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::ConnectionRefused
        );
    }
    let topology = FusedRelay::bind(&relay_urls, false).await;
    topology.online().await;
    let connections = topology.connect(&topology.addresses[1]).await;
    let member = topology.addresses.each_ref();
    assert_eq!(connections[0].initial_data_path(), member[1]);
    assert_eq!(connections[1].initial_data_path(), member[0]);
    let ids = connections.each_ref().map(MemberConnection::stable_id);
    timeout(FUSION_BUDGET, async {
        let since = Instant::now();
        let deadline = time::Instant::now() + FUSION_BUDGET;
        let reused = topology.endpoints[0]
            .connect_with_routes_until(
                EndpointAddr::new(connections[0].remote_id()),
                MemberDialOptions::new(Vec::new(), Vec::new(), relay_urls.into()),
                deadline,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(reused.stable_id(), ids[0]);
        let mut stream = OriginalStream::open(&connections[0], &connections[1]).await;
        verify_selection(
            &connections,
            &mut stream,
            [&relay, &relay],
            [member[1], member[0]],
            since,
            true,
        )
        .await;
    })
    .await
    .expect("only the third configured native relay can qualify and take the original stream");
    assert_eq!(connections.each_ref().map(MemberConnection::stable_id), ids);
    topology.close().await;
    native.shutdown().await.unwrap();
}

#[cfg(feature = "nat-lab-test-hooks")]
#[tokio::test]
async fn third_registration_supplied_by_accepting_side_still_qualifies() {
    let native = native_relay().await;
    let (url, proxy) =
        delayed_relay::delay_relay(native.http_addr().unwrap(), Duration::from_millis(65)).await;
    let relay = TransportAddr::Relay(url.clone());
    let mut topology =
        FusedRelay::bind_with_registrations(std::slice::from_ref(&url), false, [2, 3]).await;
    topology.online().await;
    let connections = topology.connect(&relay).await;
    let ids = connections.each_ref().map(MemberConnection::stable_id);
    let suppressed = Arc::new(std::sync::atomic::AtomicBool::new(true));
    timeout(FUSION_BUDGET * 2, async {
        let mut stream = OriginalStream::open(&connections[1], &connections[0]).await;
        for candidate_id in 1u8..=2 {
            let id = noq_proto::PathId::from(candidate_id);
            for connection in &connections {
                connection.test_suppress_path_acks(id, suppressed.clone());
            }
            topology
                .reuse(1, &topology.addresses[0], &connections[1])
                .await;
            loop {
                stream.round_trip().await;
                if connections.iter().enumerate().all(|(side, connection)| {
                    connection.paths().get(id).is_some_and(|path| {
                        assert_eq!(path.remote_addr(), &topology.addresses[1 - side]);
                        assert!(path.stats().acknowledgment.is_none());
                        assert_eq!(path.stats().frame_rx.stream, 0);
                        assert!(!path.is_selected());
                        path.stats().frame_rx.path_response > 0
                    })
                }) {
                    break;
                }
            }
            if candidate_id == 1 {
                topology.rotate_registration(0).await;
            }
            topology.rotate_registration(1).await;
        }
        // For the third offer, only the accepting side's true registration
        // changed. It receives the same exact remote address as before.
        let since = Instant::now();
        let member = topology.addresses.each_ref();
        topology.reuse(1, member[0], &connections[1]).await;
        verify_selection(
            &connections,
            &mut stream,
            [member[1], member[0]],
            [&relay, &relay],
            since,
            true,
        )
        .await;
    })
    .await
    .expect("two retired registrations cannot consume the current route's validation chance");
    assert_eq!(connections.each_ref().map(MemberConnection::stable_id), ids);
    topology.close().await;
    proxy.cancel();
    native.shutdown().await.unwrap();
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "requires a disposable namespace with UDP filtering and 20ms loopback UDP delay"]
async fn fused_direct_path_wins_with_both_data_relays_available() {
    let native = native_relay().await;
    let url = native.http_url().unwrap();
    let relay = TransportAddr::Relay(url.clone());
    let topology = FusedRelay::bind(std::slice::from_ref(&url), true).await;
    topology.online().await;
    let sockets = topology.endpoints.each_ref().map(|endpoint| {
        *endpoint
            .addr()
            .ip_addrs()
            .find(|address| address.ip().is_loopback())
            .unwrap()
    });
    let block = udp_blackhole::UdpBlackhole::new(sockets);
    let connections = topology.connect(&relay).await;
    let ids = connections.each_ref().map(MemberConnection::stable_id);
    timeout(FUSION_BUDGET, async {
        let mut stream = OriginalStream::open(&connections[0], &connections[1]).await;
        let member = topology.addresses.each_ref();
        let since = Instant::now();
        topology.reuse(0, member[1], &connections[0]).await;
        // Both relays have original-connection ACKs before restoring direct.
        verify_selection(
            &connections,
            &mut stream,
            [&relay, &relay],
            [member[1], member[0]],
            since,
            true,
        )
        .await;
        let direct = sockets.map(TransportAddr::Ip);
        let since = Instant::now();
        drop(block);
        topology.reuse(0, &direct[1], &connections[0]).await;
        verify_selection(
            &connections,
            &mut stream,
            [&direct[1], &direct[0]],
            [&relay, &relay],
            since,
            false,
        )
        .await;
        for (side, connection) in connections.iter().enumerate() {
            assert!(
                connection
                    .paths()
                    .iter()
                    .any(|path| path.remote_addr() == member[1 - side]
                        && path.stats().acknowledgment.is_some())
            );
        }
    })
    .await
    .expect("primary UDP wins while both real data relays remain available");
    assert_eq!(connections.each_ref().map(MemberConnection::stable_id), ids);
    topology.close().await;
    native.shutdown().await.unwrap();
}

async fn verify_selection(
    connections: &[MemberConnection; 2],
    stream: &mut OriginalStream,
    selected: [&TransportAddr; 2],
    alternate: [&TransportAddr; 2],
    since: Instant,
    same_tier: bool,
) {
    let observations = loop {
        stream.round_trip().await;
        let observations = std::array::from_fn::<_, 2, _>(|side| {
            let paths = connections[side].paths();
            let winner = paths
                .iter()
                .find(|path| path.is_selected() && path.remote_addr() == selected[side])?;
            let backup = paths
                .iter()
                .find(|path| !path.is_selected() && path.remote_addr() == alternate[side])?;
            let (winner, backup) = (winner.stats(), backup.stats());
            let (winner_ack, backup_ack) = (winner.acknowledgment?, backup.acknowledgment?);
            (winner_ack.sent_at >= since && backup_ack.sent_at >= since).then_some((winner, backup))
        });
        if let [Some(left), Some(right)] = observations {
            break [left, right];
        }
        time::sleep(Duration::from_millis(15)).await;
    };
    for _ in 0..4 {
        stream.round_trip().await;
    }
    for (side, (selected_before, backup)) in observations.into_iter().enumerate() {
        let connection = &connections[side];
        let paths = connection.paths();
        let winner = paths
            .iter()
            .find(|path| path.is_selected() && path.remote_addr() == selected[side])
            .unwrap();
        let stats = winner.stats();
        assert!(stats.frame_rx.stream > selected_before.frame_rx.stream);
        assert!(stats.udp_rx.bytes > selected_before.udp_rx.bytes);
        if same_tier {
            assert!(
                selected_before.rtt + RTT_MARGIN <= backup.rtt,
                "same-tier measured improvement: {:?} vs {:?}",
                selected_before.rtt,
                backup.rtt
            );
        } else {
            assert!(
                selected_before.rtt + RTT_MARGIN > backup.rtt,
                "primary must win without requiring a same-tier RTT improvement"
            );
        }
        eprintln!(
            "MIO153 same_tier={same_tier} side={side} stable={} stream={} path={} selected_rtt={:?} backup_rtt={:?} stream_frames={} bytes={}",
            connection.stable_id(),
            stream.id,
            winner.id(),
            selected_before.rtt,
            backup.rtt,
            stats.frame_rx.stream - selected_before.frame_rx.stream,
            stats.udp_rx.bytes - selected_before.udp_rx.bytes,
        );
    }
}

struct OriginalStream {
    outgoing: MemberStream,
    incoming: MemberStream,
    id: noq_proto::StreamId,
    sequence: u64,
}

impl OriginalStream {
    async fn open(sender: &MemberConnection, receiver: &MemberConnection) -> Self {
        let mut outgoing = sender.open_bi().await.unwrap();
        let (send, _) = outgoing.streams();
        let id = send.id();
        send.write_all(&0u64.to_be_bytes()).await.unwrap();
        let incoming = receiver.accept_bi().await.unwrap();
        let mut stream = Self {
            outgoing,
            incoming,
            id,
            sequence: 0,
        };
        stream.echo().await;
        stream
    }

    async fn round_trip(&mut self) {
        self.sequence += 1;
        let (send, _) = self.outgoing.streams();
        send.write_all(&self.sequence.to_be_bytes()).await.unwrap();
        self.echo().await;
    }

    async fn echo(&mut self) {
        let (send, recv) = self.outgoing.streams();
        let (reply, request) = self.incoming.streams();
        assert!(
            [send.id(), recv.id(), reply.id(), request.id()]
                .into_iter()
                .all(|id| id == self.id)
        );
        let mut data = [0; 8];
        request.read_exact(&mut data).await.unwrap();
        assert_eq!(data, self.sequence.to_be_bytes());
        reply.write_all(&data).await.unwrap();
        recv.read_exact(&mut data).await.unwrap();
        assert_eq!(data, self.sequence.to_be_bytes());
    }
}
