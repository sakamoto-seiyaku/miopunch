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
        MemberConnection, MemberFusionAdmission, MemberFusionEndpoint, MemberStream, MemberUdpPlan,
        PortmapperConfig, presets,
    },
};
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;

use super::{admitted_pair, delayed_relay::delay_relay, member_fusion_attempt_id, relay, resource};
use miopunch_native_probe::{
    native_network::NativeNetwork,
    path_selection::PathPreference,
    resources::{Resource, ResourceUse},
};
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
        Self::bind(ips, ips, delay, preference).await
    }

    async fn bind(
        ips: [IpAddr; 2],
        binds: [IpAddr; 2],
        delay: Duration,
        preference: PathPreference,
    ) -> Self {
        Self::bind_with_observers(ips, binds, delay, preference, &[], |builder| builder).await
    }

    async fn bind_with_observers(
        ips: [IpAddr; 2],
        binds: [IpAddr; 2],
        delay: Duration,
        preference: PathPreference,
        observers: &[Resource],
        configure: impl Fn(iroh::endpoint::Builder) -> iroh::endpoint::Builder,
    ) -> Self {
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
            config.replacement = Some(
                [resource(&url)]
                    .into_iter()
                    .chain(observers.iter().cloned())
                    .collect(),
            );
            config.udp_direct_disabled = false;
            config.path_selection.preference = preference;
            node.save_resource_configuration(&config).unwrap();
        }
        let builder = |side: usize| {
            configure(Endpoint::builder(presets::Minimal).clear_ip_transports())
                .bind_addr(SocketAddr::new(binds[side], 0))
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

fn observation_resource(url: String) -> Resource {
    Resource {
        url,
        uses: [ResourceUse::Observation].into(),
        limitation: "controlled loopback observer".into(),
    }
}

async fn reply_stun(socket: &tokio::net::UdpSocket, expected: [SocketAddr; 2]) {
    let mut seen = Vec::new();
    for _ in 0..2 {
        let mut request = [0; 1500];
        let (len, source) = socket.recv_from(&mut request).await.unwrap();
        assert_eq!(len, 20);
        assert_eq!(&request[..8], &[0, 1, 0, 0, 0x21, 0x12, 0xa4, 0x42]);
        let SocketAddr::V4(source) = source else {
            panic!("IPv4 observation required");
        };
        let mut reply = [0; 32];
        reply[..8].copy_from_slice(&[1, 1, 0, 12, 0x21, 0x12, 0xa4, 0x42]);
        reply[8..20].copy_from_slice(&request[8..20]);
        reply[20..26].copy_from_slice(&[0, 0x20, 0, 8, 0, 1]);
        reply[26..28].copy_from_slice(&(source.port() ^ 0x2112).to_be_bytes());
        reply[28..32].copy_from_slice(&(u32::from(*source.ip()) ^ 0x2112_a442).to_be_bytes());
        socket.send_to(&reply, source).await.unwrap();
        seen.push(SocketAddr::V4(source));
    }
    assert!(expected.iter().all(|address| seen.contains(address)));
}

#[tokio::test]
async fn enhanced_round_uses_two_samples_while_qad_is_waiting_on_the_original() {
    use tokio::net::UdpSocket;

    let qad = UdpSocket::bind((
        Ipv4Addr::new(127, 0, 0, 14),
        iroh_relay::defaults::DEFAULT_RELAY_QUIC_PORT,
    ))
    .await
    .unwrap();
    let stun_a = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let stun_b = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let pair = Pair::bind_with_observers(
        LOOPBACK,
        LOOPBACK,
        LOW_RELAY_DELAY,
        PathPreference::Quality,
        &[
            observation_resource("https://127.0.0.14/".into()),
            observation_resource(format!("stun://{}/", stun_a.local_addr().unwrap())),
            observation_resource(format!("stun://{}/", stun_b.local_addr().unwrap())),
        ],
        |builder| builder,
    )
    .await;
    let expected = pair.addresses();
    let ids = pair.connections.each_ref().map(MemberConnection::stable_id);
    assert!(
        pair.connections
            .iter()
            .all(|connection| connection.udp_plan().is_none())
    );
    let mut stream = time::timeout(
        BUSINESS_LIMIT,
        OriginalStream::open(&pair.connections[0], &pair.connections[1], 157),
    )
    .await
    .unwrap();
    for (initiator, selected_index) in [(0, 2), (1, 2), (0, 3)] {
        while qad.try_recv_from(&mut [0; 2048]).is_ok() {}
        let deadline = Instant::now() + Duration::from_secs(2);
        let collect = async {
            let mut sources = Vec::new();
            while sources.len() != 2 {
                let (len, source) = qad.recv_from(&mut [0; 2048]).await.unwrap();
                assert!(len >= 1200);
                if !sources.contains(&source) {
                    sources.push(source);
                }
            }
            assert!(expected.iter().all(|address| sources.contains(address)));
            tokio::join!(reply_stun(&stun_a, expected), reply_stun(&stun_b, expected));
        };
        let (plan, ()) = time::timeout_at(deadline, async {
            tokio::join!(coordinate_once(&pair, initiator, deadline), async {
                collect.await;
                stream.round_trip(158).await;
            })
        })
        .await
        .expect("two independent STUN replies must advance while real QAD is still waiting");
        let peer = paired_plan(&pair, initiator, &plan).await;
        assert_eq!((plan.id.mode, plan.id.index), (0, selected_index));
        assert!(!plan.fallback);
        for (side, agreed) in [(initiator, &plan), (1 - initiator, &peer)] {
            assert_eq!(agreed.local_samples().len(), 2);
            assert!(agreed.local_samples().iter().all(|sample| {
                sample.mapped == expected[side] && sample.observer.starts_with("stun://")
            }));
        }
        assert!(plan.execution_budget <= Duration::from_secs(2));
        eprintln!(
            "MIO157 mixed sources={expected:?} original={ids:?} initiator={initiator} round={:?} mode={} index={} c={:?} v={:?}",
            plan.round,
            plan.id.mode,
            plan.id.index,
            peer.local_phase.behavior.role,
            plan.local_phase.behavior.role
        );
        if initiator == 0 && selected_index == 2 {
            let after = Instant::now();
            for side in 0..2 {
                pair.endpoints[side]
                    .learn_ip_candidates(
                        EndpointAddr::new(pair.endpoints[1 - side].id())
                            .with_ip_addr(expected[1 - side]),
                    )
                    .await;
            }
            qualified_udp(&pair.connections, expected, after, true).await;
            time::timeout(BUSINESS_LIMIT, stream.round_trip(159))
                .await
                .unwrap();
        }
    }
    assert_eq!(
        pair.connections.each_ref().map(MemberConnection::stable_id),
        ids
    );
    pair.close().await;
}

async fn coordinate_once(pair: &Pair, initiator: usize, deadline: Instant) -> MemberUdpPlan {
    pair.connections[initiator]
        .coordinate_udp_plan(deadline, &CancellationToken::new())
        .await
        .expect("coordinate one protected enhancement on the admitted connection")
}

async fn paired_plan(pair: &Pair, initiator: usize, plan: &MemberUdpPlan) -> MemberUdpPlan {
    let peer = &pair.connections[1 - initiator];
    let their_plan = time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(their_plan) = peer.udp_plan().filter(|value| value.round == plan.round) {
                break their_plan;
            }
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("coordinator independently accepted the same protected plan");
    assert!(their_plan.coordinator && !plan.coordinator);
    assert_eq!(their_plan.id, plan.id);
    assert_eq!(their_plan.local_phase, plan.peer_phase);
    assert_eq!(their_plan.peer_phase, plan.local_phase);
    assert_eq!(their_plan.execution_budget, plan.execution_budget);
    their_plan
}

#[tokio::test]
async fn enhanced_round_deduplicates_qad_destinations_and_falls_back_for_one_sample() {
    use iroh_relay::{defaults::DEFAULT_RELAY_QUIC_PORT, server, tls::CaTlsConfig};

    let destination = (Ipv4Addr::LOCALHOST, DEFAULT_RELAY_QUIC_PORT).into();
    let (certs, tls) = server::testing::self_signed_tls_certs_and_config();
    let mut quic = server::QuicConfig::new(destination);
    quic.server_config = Some(tls);
    let mut config = server::ServerConfig::default();
    config.quic = Some(quic);
    let qad = server::Server::spawn(config).await.unwrap();
    let stun = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let stun_destination = stun.local_addr().unwrap();
    let pair = Pair::bind_with_observers(
        LOOPBACK,
        LOOPBACK,
        LOW_RELAY_DELAY,
        PathPreference::Quality,
        &[
            observation_resource("https://127.0.0.1:44341/".into()),
            observation_resource("https://127.0.0.1:44342/".into()),
            observation_resource(format!("stun://{stun_destination}/")),
        ],
        |builder| builder.ca_tls_config(CaTlsConfig::custom_roots(certs.clone())),
    )
    .await;
    let addresses = pair.addresses();
    // Two names use one QAD UDP destination. With the STUN server silent,
    // both endpoints may offer that QAD mapping as a basic candidate only.
    let fallback = coordinate_once(&pair, 0, Instant::now() + Duration::from_secs(5)).await;
    let peer = paired_plan(&pair, 0, &fallback).await;
    assert!(fallback.fallback && peer.fallback);
    assert_eq!((fallback.id.mode, fallback.id.index), (0, 6));
    for (side, plan) in [fallback, peer].iter().enumerate() {
        assert_eq!(plan.local_samples().len(), 1);
        assert_eq!(plan.local_samples()[0].destination, destination);
        assert_eq!(plan.local_samples()[0].mapped, addresses[side]);
    }
    while stun.try_recv_from(&mut [0; 2048]).is_ok() {}
    let (mixed, ()) = tokio::join!(
        coordinate_once(&pair, 1, Instant::now() + Duration::from_secs(2)),
        reply_stun(&stun, addresses),
    );
    let peer = paired_plan(&pair, 1, &mixed).await;
    assert!(!mixed.fallback && !peer.fallback);
    for (side, plan) in [(1, &mixed), (0, &peer)] {
        let samples = plan.local_samples();
        assert_eq!(samples.len(), 2);
        assert!(
            samples
                .iter()
                .all(|sample| sample.mapped == addresses[side])
        );
        for expected in [destination, stun_destination] {
            assert!(samples.iter().any(|sample| sample.destination == expected));
        }
    }
    eprintln!(
        "MIO157 distinct actual QAD={destination} STUN={stun_destination} same physical bindings={addresses:?}"
    );
    pair.close().await;
    qad.shutdown().await.unwrap();
}

#[tokio::test]
async fn enhanced_round_keeps_ordered_stun_for_an_unsupported_mixed_binding() {
    use iroh::endpoint::BindOpts;
    use tokio::net::UdpSocket;

    let mut observers = [
        UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap(),
        UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap(),
    ];
    observers.sort_by_key(|socket| socket.local_addr().unwrap().to_string());
    let [first, second] = &observers;
    let resources = observers
        .each_ref()
        .map(|socket| observation_resource(format!("stun://{}/", socket.local_addr().unwrap())));
    let extra = Ipv4Addr::new(127, 0, 0, 15);
    let pair = Pair::bind_with_observers(
        LOOPBACK,
        LOOPBACK,
        LOW_RELAY_DELAY,
        PathPreference::Quality,
        &resources,
        |builder| {
            builder
                .bind_addr_with_opts((extra, 0), BindOpts::default().set_prefix_len(32))
                .unwrap()
        },
    )
    .await;
    assert!(pair.endpoints.iter().all(|endpoint| {
        endpoint
            .addr()
            .ip_addrs()
            .any(|address| address.ip() == IpAddr::V4(extra))
    }));
    let expected = pair.addresses();
    let (plan, ()) = tokio::join!(
        coordinate_once(&pair, 0, Instant::now() + Duration::from_secs(3)),
        async {
            assert!(
                time::timeout(Duration::from_millis(250), second.recv_from(&mut [0; 2048]))
                    .await
                    .is_err(),
                "second ordered request must wait for a reply to the first"
            );
            reply_stun(first, expected).await;
            reply_stun(second, expected).await;
        },
    );
    let peer = paired_plan(&pair, 0, &plan).await;
    for outcome in [plan, peer] {
        assert!(!outcome.fallback);
        assert_eq!(
            outcome
                .local_samples()
                .iter()
                .map(|sample| sample.destination)
                .collect::<Vec<_>>(),
            observers
                .each_ref()
                .map(|socket| socket.local_addr().unwrap()),
        );
    }
    eprintln!("MIO157 unsupported mixed binding kept ordered STUN");
    pair.close().await;
}

#[tokio::test]
async fn enhanced_round_classifies_and_releases_interrupted_observation() {
    use std::io::ErrorKind;
    use tokio::net::UdpSocket;

    #[derive(Debug, Clone, Copy)]
    enum Stop {
        Cancel,
        Network,
        Revoke,
        Timeout,
    }
    for (stop, kind) in [
        (Stop::Cancel, ErrorKind::Interrupted),
        (Stop::Network, ErrorKind::Interrupted),
        (Stop::Revoke, ErrorKind::PermissionDenied),
        (Stop::Timeout, ErrorKind::TimedOut),
    ] {
        let stun = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let pair = Pair::bind_with_observers(
            LOOPBACK,
            LOOPBACK,
            LOW_RELAY_DELAY,
            PathPreference::Quality,
            &[observation_resource(format!(
                "stun://{}/",
                stun.local_addr().unwrap()
            ))],
            |builder| builder,
        )
        .await;
        let token = CancellationToken::new();
        let deadline = Instant::now() + Duration::from_millis(950);
        let (result, ()) = time::timeout_at(deadline + Duration::from_millis(100), async {
            tokio::join!(
                pair.connections[0].coordinate_udp_plan(deadline, &token),
                async {
                    let mut sources = [
                        stun.recv_from(&mut [0; 1500]).await.unwrap().1,
                        stun.recv_from(&mut [0; 1500]).await.unwrap().1,
                    ];
                    let mut expected = pair.addresses();
                    sources.sort();
                    expected.sort();
                    assert_eq!(sources, expected, "stop both active observation requests");
                    match stop {
                        Stop::Cancel => token.cancel(),
                        Stop::Network => pair.endpoints[0].network_change().await,
                        Stop::Revoke => pair.nodes[0].udp_connector().set_udp_enabled(false),
                        Stop::Timeout => {}
                    }
                }
            )
        })
        .await
        .expect("both observers see this round before it terminates");
        assert_eq!(result.unwrap_err().kind(), kind, "{stop:?}");
        while stun.try_recv_from(&mut [0; 1500]).is_ok() {}
        assert!(
            time::timeout(Duration::from_millis(650), stun.recv_from(&mut [0; 1500]))
                .await
                .is_err(),
            "{stop:?}: neither endpoint may retransmit after stopping"
        );
        assert!(
            pair.connections
                .iter()
                .all(|connection| connection.udp_plan().is_none())
        );
        assert!(
            pair.connections
                .iter()
                .all(|connection| relay_is_selected(connection, &pair.url))
        );
        if matches!(stop, Stop::Revoke) {
            let error = pair.connections[1]
                .coordinate_udp_plan(
                    Instant::now() + Duration::from_secs(2),
                    &CancellationToken::new(),
                )
                .await
                .unwrap_err();
            assert_eq!(error.kind(), ErrorKind::PermissionDenied);
            assert!(stun.try_recv_from(&mut [0; 1500]).is_err());
        }
        eprintln!(
            "MIO157 stopped={stop:?} classified={kind:?} retained original relay without STUN retries"
        );
        pair.close().await;
    }
}

#[tokio::test]
async fn enhanced_round_treats_silent_observation_as_unavailable_without_restarting_stun() {
    let stun = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let pair = Pair::bind_with_observers(
        LOOPBACK,
        LOOPBACK,
        LOW_RELAY_DELAY,
        PathPreference::Quality,
        &[observation_resource(format!(
            "stun://{}/",
            stun.local_addr().unwrap()
        ))],
        |builder| builder,
    )
    .await;
    let outcome = coordinate_once(&pair, 0, Instant::now() + Duration::from_secs(5)).await;
    let peer = paired_plan(&pair, 0, &outcome).await;
    assert!(outcome.fallback && peer.fallback);
    assert!(outcome.local_samples().is_empty() && peer.local_samples().is_empty());
    let mut transactions = std::collections::BTreeMap::new();
    let mut request = [0; 1500];
    while let Ok((length, source)) = stun.try_recv_from(&mut request) {
        assert_eq!(length, 20);
        let transaction = <[u8; 12]>::try_from(&request[8..20]).unwrap();
        transactions
            .entry(source)
            .or_insert_with(std::collections::BTreeSet::new)
            .insert(transaction);
    }
    for addr in pair.addresses() {
        assert_eq!(
            transactions[&addr].len(),
            1,
            "ordinary silence may only make one observation request"
        );
    }
    eprintln!("MIO157 ordinary no-response used basic candidate without ordered STUN retries");
    pair.close().await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "requires a disposable namespace with mio154a and its two configured IP addresses"]
async fn changed_interface_address_revalidates_the_original_udp_stream() {
    let old = IpAddr::V4(Ipv4Addr::new(10, 154, 0, 1));
    let new = IpAddr::V4(Ipv4Addr::new(10, 154, 0, 2));
    let mut pair = Pair::bind(
        [old; 2],
        [Ipv4Addr::UNSPECIFIED.into(); 2],
        LOW_RELAY_DELAY,
        PathPreference::PreferDirect,
    )
    .await;
    let ids = pair.connections.each_ref().map(MemberConnection::stable_id);
    let started = Instant::now();
    let mut stream = time::timeout(
        BUSINESS_LIMIT,
        OriginalStream::open(&pair.connections[0], &pair.connections[1], 1),
    )
    .await
    .unwrap();
    let addresses = pair.addresses();
    let old_paths = qualified_udp(&pair.connections, addresses, started, true).await;
    let adjust = |action: &str, address: IpAddr| {
        let address = format!("{address}/24");
        assert!(
            std::process::Command::new("ip")
                .args(["addr", action, &address, "dev", "mio154a"])
                .status()
                .unwrap()
                .success()
        );
    };
    let cut = Instant::now();
    adjust("del", old);
    time::timeout_at(cut + BUSINESS_LIMIT, stream.round_trip(2))
        .await
        .unwrap();
    wait_for(
        "original selects live relay after old address is removed",
        BUSINESS_LIMIT,
        || {
            pair.connections
                .iter()
                .all(|connection| relay_is_selected(connection, &pair.url))
        },
    )
    .await;
    time::timeout(BUSINESS_LIMIT, stream.round_trip(3))
        .await
        .unwrap();
    let added = Instant::now();
    adjust("add", new);
    wait_for(
        "endpoint advertises changed interface address",
        BUSINESS_LIMIT,
        || {
            pair.endpoints.iter().all(|endpoint| {
                let address = endpoint.addr();
                address.ip_addrs().any(|addr| addr.ip() == new)
                    && address.ip_addrs().all(|addr| addr.ip() != old)
            })
        },
    )
    .await;
    pair.ips = [new; 2];
    let new_sockets = pair.addresses();
    for (local, remote) in [(0, 1), (1, 0)] {
        pair.endpoints[local]
            .learn_ip_candidates(
                EndpointAddr::new(pair.endpoints[remote].id()).with_ip_addr(new_sockets[remote]),
            )
            .await;
    }
    let paths = qualified_udp(&pair.connections, new_sockets, added, true).await;
    assert_ne!(paths, old_paths);
    let stream_frames = |side: usize| {
        pair.connections[side]
            .paths()
            .iter()
            .find(|path| path.id() == paths[side])
            .unwrap()
            .stats()
            .frame_rx
            .stream
    };
    let before = [stream_frames(0), stream_frames(1)];
    time::timeout(BUSINESS_LIMIT, stream.round_trip(4))
        .await
        .unwrap();
    for (side, connection) in pair.connections.iter().enumerate() {
        assert_eq!(connection.stable_id(), ids[side]);
        assert!(stream_frames(side) > before[side]);
        assert!(connection.paths().iter().all(|path| {
            path.remote_addr() != &TransportAddr::Ip(addresses[1 - side]) || !path.is_selected()
        }));
    }
    eprintln!(
        "MIO154 interface old={addresses:?} new={new_sockets:?} original={ids:?} stream={} old_paths={old_paths:?} new_paths={paths:?} recovered={:?}",
        stream.stream_id,
        added.elapsed()
    );
    pair.close().await;
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
