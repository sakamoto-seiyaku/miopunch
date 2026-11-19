#[path = "udp_blackhole.rs"]
mod udp_blackhole;

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMode, RelayUrl, TransportAddr,
    endpoint::{
        BeforeConnectOutcome, ControlConfirmation, ControlConnection, EndpointHooks,
        MemberConnection, MemberFusionAdmission, MemberFusionEndpoint, MemberStream,
        PortmapperConfig, presets,
    },
};
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;

use super::{admitted_pair, delayed_relay::delay_relay, member_fusion_attempt_id, relay, resource};
use miopunch_native_probe::{native_network::NativeNetwork, path_selection::PathPreference};
use udp_blackhole::UdpBlackhole;

const BUSINESS_LIMIT: Duration = Duration::from_secs(6);
const SMALL_REQUEST_LIMIT: Duration = Duration::from_secs(5);
#[cfg(target_os = "linux")]
const QNT_CONTROL_LIMIT: Duration = Duration::from_secs(6);
const SEND_INTERVAL: Duration = Duration::from_millis(25);
const LOW_RELAY_DELAY: Duration = Duration::from_millis(25);
const HIGH_RELAY_DELAY: Duration = Duration::from_millis(350);
const SETUP: Duration = Duration::from_secs(30);
const LOOPBACK: [IpAddr; 2] = [IpAddr::V4(Ipv4Addr::LOCALHOST); 2];

#[derive(Debug, Clone, Default)]
pub(super) struct Dials(pub(super) Arc<AtomicUsize>);

impl EndpointHooks for Dials {
    async fn before_connect(&self, _: &EndpointAddr, _: &[u8]) -> BeforeConnectOutcome {
        self.0.fetch_add(1, Ordering::Relaxed);
        BeforeConnectOutcome::Accept
    }
}

struct Pair {
    connections: [MemberConnection; 2],
    endpoints: [MemberFusionEndpoint; 2],
    nodes: [NativeNetwork; 2],
    relay: iroh_relay::server::Server,
    delay: CancellationToken,
    url: RelayUrl,
    dials: [Dials; 2],
    ips: [IpAddr; 2],
    _root: tempfile::TempDir,
}

impl Pair {
    async fn new(ips: [IpAddr; 2], delay: Duration, preference: PathPreference) -> Self {
        let relay = relay().await;
        let url = relay.http_url().unwrap();
        let root = tempfile::tempdir().unwrap();
        let (admin, _) = admitted_pair(root.path(), &url).await;
        let joining = NativeNetwork::open(root.path().join("joining"))
            .await
            .unwrap();
        let (url, delay) = delay_relay(relay.http_addr().unwrap(), delay).await;
        let nodes = [joining, admin];
        let dials = [Dials::default(), Dials::default()];
        for node in &nodes {
            let mut config = node.resource_configuration().unwrap();
            config.replacement = Some(vec![resource(&url)]);
            config.udp_direct_disabled = false;
            config.path_selection.preference = preference;
            node.save_resource_configuration(&config).unwrap();
        }
        let builder = |side: usize| {
            Endpoint::builder(presets::Minimal)
                .clear_ip_transports()
                .bind_addr(SocketAddr::new(ips[side], 0))
                .unwrap()
                .hooks(dials[side].clone())
        };
        let client = nodes[0]
            .bind_member_fusion_endpoint_with(builder(0), SETUP)
            .await
            .unwrap();
        let server = nodes[1]
            .bind_member_fusion_endpoint_with(builder(1), SETUP)
            .await
            .unwrap();
        let connections = on_relay(&client, &server, &url).await;
        assert_eq!(
            member_fusion_attempt_id(&connections[0]),
            member_fusion_attempt_id(&connections[1])
        );
        Self {
            connections,
            endpoints: [client, server],
            nodes,
            relay,
            delay,
            url,
            dials,
            ips,
            _root: root,
        }
    }

    fn addresses(&self) -> [SocketAddr; 2] {
        std::array::from_fn(|side| {
            *self.endpoints[side]
                .addr()
                .ip_addrs()
                .find(|addr| addr.ip() == self.ips[side])
                .expect("actual bound UDP address")
        })
    }

    fn assert_one_dial(&self) {
        assert_eq!(
            [
                self.dials[0].0.load(Ordering::Relaxed),
                self.dials[1].0.load(Ordering::Relaxed)
            ],
            [1, 0]
        );
    }

    async fn close(self) {
        self.assert_one_dial();
        let [client, server] = self.endpoints;
        let [joining, admin] = self.nodes;
        tokio::join!(
            client.close(),
            server.close(),
            joining.close(),
            admin.close()
        );
        self.delay.cancel();
        self.relay.shutdown().await.unwrap();
    }
}

pub(super) async fn on_relay(
    client: &MemberFusionEndpoint,
    server: &MemberFusionEndpoint,
    url: &RelayUrl,
) -> [MemberConnection; 2] {
    time::timeout(SETUP, async {
        tokio::join!(client.online(), server.online())
    })
    .await
    .unwrap();
    let (client, server) = time::timeout(SETUP, async {
        tokio::join!(
            client.connect(EndpointAddr::new(server.id()).with_relay_url(url.clone())),
            server.accept()
        )
    })
    .await
    .unwrap();
    let connections = [client.unwrap(), server.unwrap().unwrap()];
    for connection in &connections {
        assert_eq!(
            connection.initial_data_path(),
            &TransportAddr::Relay(url.clone())
        );
        assert!(
            connection
                .paths()
                .iter()
                .any(|path| path.is_selected() && path.is_relay())
        );
    }
    connections
}

pub(super) struct OriginalStream {
    outgoing: MemberStream,
    incoming: MemberStream,
    pub(super) stream_id: noq::StreamId,
}

impl OriginalStream {
    pub(super) async fn open(
        sender: &MemberConnection,
        receiver: &MemberConnection,
        first: u64,
    ) -> Self {
        let mut outgoing = sender.open_bi().await.unwrap();
        let (send, _) = outgoing.streams();
        let stream_id = send.id();
        send.write_all(&first.to_be_bytes()).await.unwrap();
        let mut incoming = receiver.accept_bi().await.unwrap();
        let mut pair = Self {
            outgoing,
            incoming,
            stream_id,
        };
        pair.echo(first).await;
        pair
    }

    async fn echo(&mut self, expected: u64) {
        let (send, recv) = self.outgoing.streams();
        let (reply, request) = self.incoming.streams();
        assert!(
            [send.id(), recv.id(), reply.id(), request.id()]
                .into_iter()
                .all(|id| id == self.stream_id)
        );
        let mut data = [0; 8];
        request.read_exact(&mut data).await.unwrap();
        assert_eq!(data, expected.to_be_bytes());
        reply.write_all(&data).await.unwrap();
        recv.read_exact(&mut data).await.unwrap();
        assert_eq!(data, expected.to_be_bytes());
    }

    pub(super) async fn round_trip(&mut self, data: u64) {
        self.outgoing
            .streams()
            .0
            .write_all(&data.to_be_bytes())
            .await
            .unwrap();
        self.echo(data).await;
    }
}

async fn wait_for(label: &str, limit: Duration, mut condition: impl FnMut() -> bool) {
    time::timeout(limit, async {
        while !condition() {
            time::sleep(Duration::from_millis(15)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{label} within {limit:?}"));
}

async fn qualified_udp(
    connections: &[MemberConnection; 2],
    addresses: [SocketAddr; 2],
    after: Instant,
    select: bool,
) -> [noq_proto::PathId; 2] {
    let mut found = [None; 2];
    wait_for(
        "both physical UDP tuples have fresh original-connection acknowledgments",
        2 * SMALL_REQUEST_LIMIT,
        || {
            for side in 0..2 {
                found[side] = connections[side]
                    .paths()
                    .iter()
                    .find(|path| {
                        (!select || path.is_selected())
                            && path.remote_addr() == &TransportAddr::Ip(addresses[1 - side])
                            && path.local_addr()
                                == &iroh::endpoint::LocalTransportAddr::Ip(Some(
                                    addresses[side].ip(),
                                ))
                            && path
                                .stats()
                                .acknowledgment
                                .is_some_and(|ack| ack.sent_at >= after.into_std())
                    })
                    .map(|path| path.id());
            }
            found.iter().all(Option::is_some)
        },
    )
    .await;
    found.map(Option::unwrap)
}

fn relay_is_selected(connection: &MemberConnection, url: &RelayUrl) -> bool {
    connection
        .paths()
        .iter()
        .any(|path| path.is_selected() && path.remote_addr() == &TransportAddr::Relay(url.clone()))
}

fn exchanges(connections: &[MemberConnection; 2]) -> [u64; 2] {
    [
        connections[0].stats().frame_tx.reach_out,
        connections[1].stats().frame_tx.add_address,
    ]
}

#[tokio::test]
async fn original_relay_stream_migrates_to_ipv4() {
    let pair = Pair::new(LOOPBACK, LOW_RELAY_DELAY, PathPreference::Quality).await;
    let ids = pair.connections.each_ref().map(MemberConnection::stable_id);
    let after = Instant::now();
    let mut stream = time::timeout(
        BUSINESS_LIMIT,
        OriginalStream::open(&pair.connections[0], &pair.connections[1], 0),
    )
    .await
    .unwrap();
    qualified_udp(&pair.connections, pair.addresses(), after, true).await;
    time::timeout(BUSINESS_LIMIT, stream.round_trip(1))
        .await
        .unwrap();
    assert_eq!(
        pair.connections.each_ref().map(MemberConnection::stable_id),
        ids
    );
    pair.close().await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "requires the lab's disposable network namespace and its assigned UDP addresses"]
async fn lan_ipv4_and_ipv6_keep_two_way_original_streams() {
    for (addresses, direction) in [
        (["192.168.147.1", "192.168.147.2"], 1),
        (["198.18.147.1", "198.18.147.2"], 0),
        (["fd00:147::1", "fd00:147::2"], 1),
    ] {
        let pair = Pair::new(
            addresses.map(|ip| ip.parse().unwrap()),
            LOW_RELAY_DELAY,
            PathPreference::Quality,
        )
        .await;
        let sockets = pair.addresses();
        let initial_blackhole = UdpBlackhole::new(sockets);
        let ids = pair.connections.each_ref().map(MemberConnection::stable_id);
        let first = Instant::now();
        let mut stream = time::timeout(
            BUSINESS_LIMIT,
            OriginalStream::open(
                &pair.connections[direction],
                &pair.connections[1 - direction],
                0,
            ),
        )
        .await
        .unwrap();
        assert!(first.elapsed() >= 4 * LOW_RELAY_DELAY);
        for connection in &pair.connections {
            assert!(relay_is_selected(connection, &pair.url));
        }
        let release = Instant::now();
        drop(initial_blackhole);
        let migrated = AtomicBool::new(false);
        let (paths, next) = tokio::join!(
            async {
                let paths = qualified_udp(&pair.connections, sockets, release, true).await;
                migrated.store(true, Ordering::Release);
                paths
            },
            async {
                let mut sequence = 1;
                while !migrated.load(Ordering::Acquire) {
                    time::timeout(BUSINESS_LIMIT, stream.round_trip(sequence))
                        .await
                        .unwrap();
                    sequence += 1;
                }
                sequence
            },
        );
        assert!(next > 1, "the original stream advanced during migration");
        let before = pair.connections.each_ref().map(|connection| {
            connection
                .paths()
                .iter()
                .find(|path| path.is_selected())
                .unwrap()
                .stats()
                .udp_rx
                .bytes
        });
        time::timeout(BUSINESS_LIMIT, stream.round_trip(next))
            .await
            .unwrap();
        assert_eq!(
            pair.connections.each_ref().map(MemberConnection::stable_id),
            ids
        );
        for (side, connection) in pair.connections.iter().enumerate() {
            let current = connection.paths();
            let path = current
                .iter()
                .find(|path| path.id() == paths[side])
                .unwrap();
            assert!(path.is_selected());
            assert!(path.stats().udp_rx.bytes > before[side]);
            eprintln!(
                "MIO147 migration side={side} stable={} stream={} local={:?} remote={} path={} ack_after_release={:?}",
                ids[side],
                stream.stream_id,
                path.local_addr(),
                path.remote_addr(),
                path.id(),
                path.stats()
                    .acknowledgment
                    .unwrap()
                    .received_at
                    .duration_since(release.into_std())
            );
        }
        assert!(
            exchanges(&pair.connections)
                .into_iter()
                .all(|frames| frames > 0)
        );
        pair.close().await;
    }
}

#[tokio::test]
async fn accepting_side_can_probe_a_locally_known_address_and_reuse_it() {
    let pair = Pair::new(LOOPBACK, LOW_RELAY_DELAY, PathPreference::Quality).await;
    let sockets = pair.addresses();
    pair.endpoints[1]
        .learn_ip_candidates(EndpointAddr::new(pair.endpoints[0].id()).with_ip_addr(sockets[0]))
        .await;
    let origin = &pair.connections[1];
    let initial = Instant::now();
    let mut outgoing = origin.open_bi().await.unwrap();
    outgoing
        .streams()
        .0
        .write_all(b"server-initiated")
        .await
        .unwrap();
    let paths = qualified_udp(&pair.connections, sockets, initial, true).await;
    assert_eq!(
        exchanges(&pair.connections),
        [0, 0],
        "the passive dialer did not have to exchange candidates"
    );
    let mut incoming = pair.connections[0].accept_bi().await.unwrap();
    assert_eq!(incoming.streams().0.id(), outgoing.streams().0.id());
    let mut bytes = [0; 16];
    incoming.streams().1.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"server-initiated");
    let before = pair
        .connections
        .each_ref()
        .map(|connection| connection.stats().frame_tx.path_challenge);
    for side in 0..2 {
        pair.endpoints[side]
            .learn_ip_candidates(
                EndpointAddr::new(pair.endpoints[1 - side].id()).with_ip_addr(sockets[1 - side]),
            )
            .await;
    }
    incoming.streams().0.write_all(b"reply").await.unwrap();
    let mut bytes = [0; 5];
    time::timeout(
        SMALL_REQUEST_LIMIT,
        outgoing.streams().1.read_exact(&mut bytes),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(&bytes, b"reply");
    time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(exchanges(&pair.connections), [0, 0]);
    assert_eq!(
        pair.connections
            .each_ref()
            .map(|connection| connection.stats().frame_tx.path_challenge),
        before
    );
    assert_eq!(
        qualified_udp(&pair.connections, sockets, initial, true).await,
        paths
    );
    pair.close().await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "requires the lab's disposable network namespace and UDP packet filters"]
async fn matching_probe_without_quic_path_validation_keeps_healthy_relay() {
    let pair = Pair::new(LOOPBACK, LOW_RELAY_DELAY, PathPreference::Quality).await;
    let sockets = pair.addresses();
    let padded_validation = UdpBlackhole::block_padded_validation(sockets[0], sockets[1]);
    let client = &pair.connections[0];
    let before_responses = client.stats().frame_rx.path_response;
    let before_opens = client.stats().frame_tx.path_challenge;
    let mut stream = time::timeout(
        BUSINESS_LIMIT,
        OriginalStream::open(client, &pair.connections[1], 0),
    )
    .await
    .unwrap();
    wait_for(
        "matched off-path PATH_RESPONSE and attempted path validation",
        SMALL_REQUEST_LIMIT,
        || {
            client.stats().frame_rx.path_response > before_responses
                && client.stats().frame_tx.path_challenge >= before_opens + 2
        },
    )
    .await;
    let until = Instant::now() + Duration::from_secs(2);
    while Instant::now() < until {
        time::timeout(BUSINESS_LIMIT, stream.round_trip(1))
            .await
            .unwrap();
        for connection in &pair.connections {
            assert!(relay_is_selected(connection, &pair.url));
            assert!(connection.paths().iter().all(|path| !path.is_ip()));
        }
    }
    let released = Instant::now();
    drop(padded_validation);
    qualified_udp(&pair.connections, sockets, released, true).await;
    time::timeout(BUSINESS_LIMIT, stream.round_trip(2))
        .await
        .unwrap();
    pair.close().await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "requires the lab's disposable network namespace and UDP packet filters"]
async fn udp_blackhole_recovers_the_fused_original_with_continuous_and_single_writes() {
    for (delay, continuous) in [
        (LOW_RELAY_DELAY, true),
        (LOW_RELAY_DELAY, false),
        (HIGH_RELAY_DELAY, true),
        (HIGH_RELAY_DELAY, false),
    ] {
        let pair = Pair::new(LOOPBACK, delay, PathPreference::PreferDirect).await;
        let before = Instant::now();
        let mut stream = time::timeout(
            BUSINESS_LIMIT,
            OriginalStream::open(&pair.connections[0], &pair.connections[1], 0),
        )
        .await
        .unwrap();
        let sockets = pair.addresses();
        let direct = qualified_udp(&pair.connections, sockets, before, true).await;
        wait_for("healthy relay backup before fault", BUSINESS_LIMIT, || {
            pair.connections.iter().all(|connection| {
                connection
                    .paths()
                    .iter()
                    .any(|path| path.is_relay() && path.stats().acknowledgment.is_some())
            })
        })
        .await;
        let blackhole = UdpBlackhole::new(sockets);
        let cut = Instant::now();
        let deadline = cut + BUSINESS_LIMIT;
        let last_sequence = if continuous {
            (BUSINESS_LIMIT.as_millis() / SEND_INTERVAL.as_millis()) as u64
        } else {
            1
        };
        let (send, recv) = stream.outgoing.streams();
        let (reply, request) = stream.incoming.streams();
        assert_eq!(send.id(), stream.stream_id);
        assert_eq!(request.id(), stream.stream_id);
        let (_, first_byte, ()) = tokio::join!(
            async {
                let mut cadence = time::interval_at(cut, SEND_INTERVAL);
                cadence.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
                for sequence in 1..=last_sequence {
                    cadence.tick().await;
                    send.write_all(&sequence.to_be_bytes()).await.unwrap();
                }
            },
            async {
                let mut data = [0; 8];
                time::timeout_at(deadline, recv.read_exact(&mut data))
                    .await
                    .expect("original bytes must recover before the fixed six-second deadline")
                    .unwrap();
                let first_byte = Instant::now();
                assert_eq!(data, 1u64.to_be_bytes());
                for sequence in 2..=last_sequence {
                    time::timeout_at(deadline + BUSINESS_LIMIT, recv.read_exact(&mut data))
                        .await
                        .unwrap()
                        .unwrap();
                    assert_eq!(data, sequence.to_be_bytes());
                }
                first_byte
            },
            async {
                let mut data = [0; 8];
                for sequence in 1..=last_sequence {
                    time::timeout_at(deadline + BUSINESS_LIMIT, request.read_exact(&mut data))
                        .await
                        .unwrap()
                        .unwrap();
                    assert_eq!(data, sequence.to_be_bytes());
                    reply.write_all(&data).await.unwrap();
                }
            }
        );
        let until = Instant::now().max(first_byte + Duration::from_secs(3));
        loop {
            for (side, connection) in pair.connections.iter().enumerate() {
                assert!(relay_is_selected(connection, &pair.url));
                let current = connection.paths();
                let path = current
                    .iter()
                    .find(|path| path.id() == direct[side])
                    .expect("retained original direct binding");
                assert!(!path.is_selected());
                assert!(path.stats().acknowledgment.unwrap().sent_at < cut.into_std());
            }
            if Instant::now() >= until {
                break;
            }
            time::sleep(Duration::from_millis(20)).await;
        }
        let release = Instant::now();
        drop(blackhole);
        assert_eq!(
            qualified_udp(&pair.connections, sockets, release, true).await,
            direct
        );
        time::timeout(BUSINESS_LIMIT, stream.round_trip(last_sequence + 1))
            .await
            .unwrap();
        eprintln!(
            "MIO147 blackhole relay_leg={delay:?} continuous={continuous} first_original_reply={:?} stream={} direct={direct:?}",
            first_byte - cut,
            stream.stream_id
        );
        pair.close().await;
    }
}

#[tokio::test]
async fn explicit_relay_preference_preserves_both_admitted_paths() {
    let pair = Pair::new(LOOPBACK, LOW_RELAY_DELAY, PathPreference::PreferRelay).await;
    let after = Instant::now();
    let mut stream = time::timeout(
        BUSINESS_LIMIT,
        OriginalStream::open(&pair.connections[0], &pair.connections[1], 0),
    )
    .await
    .unwrap();
    qualified_udp(&pair.connections, pair.addresses(), after, false).await;
    time::timeout(BUSINESS_LIMIT, stream.round_trip(7))
        .await
        .unwrap();
    assert!(
        pair.connections
            .iter()
            .all(|connection| relay_is_selected(connection, &pair.url))
    );
    pair.close().await;
}

#[derive(Debug)]
struct RevocableUdp {
    peer: EndpointId,
    peer_address: Arc<OnceLock<SocketAddr>>,
    allowed: Arc<AtomicBool>,
}

impl MemberFusionAdmission for RevocableUdp {
    async fn authorize(
        &self,
        connection: &ControlConnection,
    ) -> std::io::Result<ControlConfirmation> {
        if !self.is_current(connection) {
            return Err(std::io::ErrorKind::PermissionDenied.into());
        }
        Ok(ControlConfirmation::with_control(|_| {
            Box::pin(async { Ok(()) })
        }))
    }

    fn is_current(&self, connection: &ControlConnection) -> bool {
        connection.remote_id() == self.peer
    }

    fn permits_data_path(&self, path: &TransportAddr) -> bool {
        path.is_relay()
            || (self.allowed.load(Ordering::Acquire)
                && matches!(path, TransportAddr::Ip(address) if self.peer_address.get() == Some(address)))
    }
}

#[tokio::test]
async fn withdrawing_udp_permission_retires_the_path_and_keeps_original_bytes() {
    let relay = relay().await;
    let url = relay.http_url().unwrap();
    let relay_mode = RelayMode::Custom(iroh::RelayMap::from_iter([url.clone()]));
    let client_secret = iroh::SecretKey::generate();
    let server_secret = iroh::SecretKey::generate();
    let allowed = Arc::new(AtomicBool::new(true));
    let udp_addresses = [Arc::new(OnceLock::new()), Arc::new(OnceLock::new())];
    let dials = [Dials::default(), Dials::default()];
    let builder = |secret, peer, side: usize| {
        Endpoint::builder(presets::Minimal)
            .clear_ip_transports()
            .bind_addr((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .portmapper_config(PortmapperConfig::Disabled)
            .relay_mode(relay_mode.clone())
            .secret_key(secret)
            .hooks(dials[side].clone())
            .member_fusion(
                b"miopunch/test-udp-policy/1",
                RevocableUdp {
                    peer,
                    peer_address: udp_addresses[1 - side].clone(),
                    allowed: allowed.clone(),
                },
            )
    };
    let client_id = client_secret.public();
    let server_id = server_secret.public();
    let client = builder(client_secret, server_id, 0)
        .bind(SETUP)
        .await
        .unwrap();
    let server = builder(server_secret, client_id, 1)
        .bind(SETUP)
        .await
        .unwrap();
    let endpoints = [client, server];
    let addresses = endpoints.each_ref().map(|endpoint| {
        *endpoint
            .addr()
            .ip_addrs()
            .find(|address| address.ip().is_loopback())
            .unwrap()
    });
    assert_ne!(addresses[0], addresses[1]);
    for (address, observed) in addresses.iter().zip(&udp_addresses) {
        observed.set(*address).unwrap();
    }
    let connections = on_relay(&endpoints[0], &endpoints[1], &url).await;
    let ids = connections.each_ref().map(MemberConnection::stable_id);
    let after = Instant::now();
    let mut stream = time::timeout(
        BUSINESS_LIMIT,
        OriginalStream::open(&connections[0], &connections[1], 0),
    )
    .await
    .unwrap();
    let old_paths = qualified_udp(&connections, addresses, after, true).await;
    allowed.store(false, Ordering::Release);
    wait_for(
        "revoked bindings leave both connection path lists",
        BUSINESS_LIMIT,
        || {
            connections.iter().all(|connection| {
                relay_is_selected(connection, &url)
                    && connection.paths().iter().all(|path| !path.is_ip())
            })
        },
    )
    .await;
    for side in 0..2 {
        endpoints[side]
            .learn_ip_candidates(
                EndpointAddr::new(endpoints[1 - side].id()).with_ip_addr(addresses[1 - side]),
            )
            .await;
    }
    time::timeout(BUSINESS_LIMIT, stream.round_trip(9))
        .await
        .unwrap();
    assert_eq!(connections.each_ref().map(MemberConnection::stable_id), ids);
    assert!(
        connections
            .iter()
            .all(|connection| connection.paths().iter().all(|path| !path.is_ip()))
    );
    assert_eq!(
        dials.each_ref().map(|d| d.0.load(Ordering::Relaxed)),
        [1, 0]
    );
    eprintln!(
        "MIO147 revoked original={ids:?} stream={} former_udp={old_paths:?}",
        stream.stream_id
    );
    tokio::join!(endpoints[0].close(), endpoints[1].close());
    relay.shutdown().await.unwrap();
}

#[cfg(target_os = "linux")]
async fn small_request(
    connections: &[MemberConnection; 2],
    nonce: u64,
) -> (OriginalStream, Duration) {
    let started = Instant::now();
    let stream = time::timeout(
        SMALL_REQUEST_LIMIT,
        OriginalStream::open(&connections[0], &connections[1], nonce),
    )
    .await
    .expect("same five-second small-request deadline as MIO145");
    (stream, started.elapsed())
}

#[cfg(target_os = "linux")]
async fn qnt_exchange(
    pair: &Pair,
    candidate: &tokio::net::UdpSocket,
) -> (Duration, Duration, Duration) {
    let relay = TransportAddr::Relay(pair.url.clone());
    let client_udp = pair.addresses()[0];
    let relay_control = || {
        [0, 1].map(|side| {
            let paths = pair.connections[side].paths();
            let stats = paths
                .iter()
                .find(|path| path.is_selected() && path.remote_addr() == &relay)
                .expect("original relay selected throughout the candidate exchange")
                .stats();
            if side == 0 {
                [stats.frame_rx.add_address, stats.frame_tx.reach_out]
            } else {
                [stats.frame_tx.add_address, stats.frame_rx.reach_out]
            }
        })
    };
    let before = relay_control();
    // Relay frames already in flight can still advance the receiver's counter.
    // Wait for it to reach the sender's new frame count in each direction.
    let new_advertisement = before[1][0] + 1;
    let started = Instant::now();
    let (timing, ()) = time::timeout_at(started + QNT_CONTROL_LIMIT, async {
        pair.endpoints[1]
            .test_add_external_addr(candidate.local_addr().unwrap())
            .await;
        tokio::join!(async {
        let mut reach_out_before = before[0][1];
        let mut advertised = None;
        let mut reach_out = None;
        loop {
            let now = relay_control();
            if now[0][0] >= new_advertisement && now[1][0] >= new_advertisement {
                let advertised = *advertised.get_or_insert_with(|| started.elapsed());
                if now[0][1] > reach_out_before {
                    let (sent_frames, sent) =
                        *reach_out.get_or_insert_with(|| (now[0][1], started.elapsed()));
                    if now[1][1] >= sent_frames {
                        let delivered = started.elapsed();
                        assert!(advertised >= 2 * HIGH_RELAY_DELAY);
                        assert!(delivered >= 4 * HIGH_RELAY_DELAY);
                        return (advertised, sent, delivered);
                    }
                }
            } else {
                reach_out_before = now[0][1];
            }
            time::sleep(Duration::from_millis(15)).await;
        }
        }, async {
            // This socket was introduced by this exchange. It receives the
            // dialing endpoint's short QNT probe even while both original UDP
            // directions are blackholed.
            let mut probe = [0; 1200];
            let (length, source) = candidate.recv_from(&mut probe).await.unwrap();
            assert_eq!(source, client_udp);
            assert!((1..probe.len()).contains(&length));
        })
    })
    .await
    .expect("both original-relay control frames and the new candidate's probe arrive within six seconds");
    timing
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "requires the lab's disposable network namespace and UDP packet filters"]
async fn qnt_and_small_request_meet_the_original_deadlines_during_large_relay_stream() {
    const LARGE: usize = 64 * 1024 * 1024;
    const BLOCK: usize = 64 * 1024;
    const FIRST_LOADED_SAMPLE: usize = 512 * 1024;
    const REPLY: [u8; 8] = *b"original";
    let pair = Pair::new(LOOPBACK, HIGH_RELAY_DELAY, PathPreference::Quality).await;
    let ids = pair.connections.each_ref().map(MemberConnection::stable_id);
    let sockets = pair.addresses();
    let blackhole = UdpBlackhole::new(sockets);
    let (solo_stream, solo) = small_request(&pair.connections, 0xa117).await;
    assert!(
        solo >= Duration::from_secs(1),
        "real delayed relay bytes: {solo:?}"
    );
    let baseline_candidate = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    // The completed small request keeps its stream lease while the QNT
    // baseline runs on the otherwise idle relay connection.
    let (advertised, reach_out, control) = qnt_exchange(&pair, &baseline_candidate).await;
    drop(solo_stream);
    eprintln!(
        "MIO147 solo relay_leg={HIGH_RELAY_DELAY:?} small_deadline={SMALL_REQUEST_LIMIT:?} control_deadline={QNT_CONTROL_LIMIT:?} small={solo:?} add_address_delivered={advertised:?} reach_out_sent={reach_out:?} reach_out_delivered={control:?}"
    );
    let mut stream = pair.connections[0].open_bi().await.unwrap();
    let original_stream_id = stream.streams().0.id();
    let (mut writer, mut answer, _lease) = stream.into_parts();
    let client_job = tokio::spawn(async move {
        let mut block = vec![0x5a; BLOCK];
        for number in 0..(LARGE / BLOCK) as u64 {
            block[..8].copy_from_slice(&number.to_be_bytes());
            writer.write_all(&block).await.unwrap();
        }
    });

    let mut incoming = pair.connections[1].accept_bi().await.unwrap();
    assert_eq!(incoming.streams().1.id(), original_stream_id);
    let bytes = Arc::new(AtomicUsize::new(0));
    let reporting_bytes = bytes.clone();
    let direct_reply = Arc::new(AtomicBool::new(false));
    let reply_after_migration = direct_reply.clone();
    let server_job = tokio::spawn(async move {
        let (reply, reader) = incoming.streams();
        let mut replied = false;
        let mut block = vec![0; BLOCK];
        for number in 0..(LARGE / BLOCK) as u64 {
            reader.read_exact(&mut block).await.unwrap();
            assert_eq!(block[..8], number.to_be_bytes());
            assert!(block[8..].iter().all(|byte| *byte == 0x5a));
            reporting_bytes.fetch_add(BLOCK, Ordering::Relaxed);
            if !replied && reply_after_migration.load(Ordering::Acquire) {
                reply.write_all(&REPLY).await.unwrap();
                replied = true;
            }
        }
    });
    wait_for(
        "512 KiB of real bytes before concurrent requests",
        SETUP,
        || bytes.load(Ordering::Relaxed) >= FIRST_LOADED_SAMPLE,
    )
    .await;
    let before = bytes.load(Ordering::Relaxed);
    assert!(before < LARGE);
    let loaded_candidate = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let ((loaded, small_progress), (advertised, reach_out, control)) = tokio::join!(
        async {
            let (_, elapsed) = small_request(&pair.connections, 0xa217).await;
            (elapsed, bytes.load(Ordering::Relaxed))
        },
        qnt_exchange(&pair, &loaded_candidate),
    );
    let progress = bytes.load(Ordering::Relaxed);
    assert!(
        progress >= before + 2 * BLOCK && progress < LARGE,
        "receiver progress while both original deadlines are running: {before} to {progress}"
    );
    assert!(loaded >= Duration::from_secs(1));
    assert!(
        small_progress >= before + 2 * BLOCK,
        "the large stream advanced during the small reply"
    );
    eprintln!(
        "MIO147 loaded relay_leg={HIGH_RELAY_DELAY:?} small_deadline={SMALL_REQUEST_LIMIT:?} control_deadline={QNT_CONTROL_LIMIT:?} small={loaded:?} add_address_delivered={advertised:?} reach_out_sent={reach_out:?} reach_out_delivered={control:?} progress={before}..{small_progress}..{progress}/{LARGE} original_stream={original_stream_id}"
    );
    let released = Instant::now();
    drop(blackhole);
    qualified_udp(&pair.connections, sockets, released, true).await;
    direct_reply.store(true, Ordering::Release);
    let mut reply = [0; REPLY.len()];
    time::timeout(SMALL_REQUEST_LIMIT, answer.read_exact(&mut reply))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reply, REPLY);
    assert_eq!(answer.id(), original_stream_id);
    assert_eq!(
        pair.connections.each_ref().map(MemberConnection::stable_id),
        ids
    );
    assert!(pair.connections.iter().all(|connection| {
        connection
            .paths()
            .iter()
            .any(|path| path.is_ip() && path.is_selected())
    }));
    client_job.abort();
    server_job.abort();
    pair.close().await;
}
