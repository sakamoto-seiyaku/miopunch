use std::{
    io,
    net::{Ipv4Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed},
    },
    task::{Context, Poll},
    time::Duration,
};

use iroh::{
    Endpoint, EndpointAddr, RelayUrl, TransportAddr,
    endpoint::{
        LocalTransportAddr, MemberConnection, MemberFusionEndpoint, presets,
        transports::{
            CustomEndpoint, CustomEndpointShutdown, CustomSender, CustomTransport, RecvInfo,
        },
    },
};
use iroh_base::CustomAddr;
use miopunch_native_probe::tcp_transport::{DATA_TRANSPORT_ID, TcpTransport, Traffic};
use n0_watcher::{Direct, Watchable};
use tokio::net::{TcpSocket, TcpStream};
use tokio::time::{self, Instant};

use super::udp::{Dials, OriginalStream, on_relay};
use super::{NativeNetwork, admitted_pair, member_fusion_attempt_id, relay, resource};

const SETUP: Duration = Duration::from_secs(15);
const BUSINESS_LIMIT: Duration = Duration::from_secs(6);

struct Pair {
    nodes: [NativeNetwork; 2],
    endpoints: [MemberFusionEndpoint; 2],
    connections: [MemberConnection; 2],
    sockets: [SocketAddr; 2],
    traffic: [Arc<Traffic>; 2],
    dials: [Dials; 2],
    relay: iroh_relay::server::Server,
    url: RelayUrl,
    _root: tempfile::TempDir,
}

impl Pair {
    async fn new(
        disabled: [bool; 2],
        unreachable: Option<SocketAddr>,
        discarded: Option<Arc<AtomicUsize>>,
    ) -> Self {
        Self::bind(Ipv4Addr::LOCALHOST, disabled, unreachable, discarded).await
    }

    async fn bind(
        bind: Ipv4Addr,
        disabled: [bool; 2],
        unreachable: Option<SocketAddr>,
        discarded: Option<Arc<AtomicUsize>>,
    ) -> Self {
        let relay = relay().await;
        let url = relay.http_url().unwrap();
        let root = tempfile::tempdir().unwrap();
        let (admin, _) = admitted_pair(root.path(), &url).await;
        let joining = NativeNetwork::open(root.path().join("joining"))
            .await
            .unwrap();
        let nodes = [joining, admin];
        let dials = [Dials::default(), Dials::default()];
        let traffic = [(); 2].map(|_| Arc::new(Traffic::default()));
        let carriers = traffic
            .each_ref()
            .map(|traffic| TcpTransport::new(bind, traffic.clone()));
        for (side, node) in nodes.iter().enumerate() {
            let mut config = node.resource_configuration().unwrap();
            config.replacement = Some(vec![resource(&url)]);
            config.udp_direct_disabled = true;
            config.tcp_direct_disabled = disabled[side];
            node.save_resource_configuration(&config).unwrap();
        }
        let builder = |side: usize| {
            Endpoint::builder(presets::Minimal)
                .hooks(dials[side].clone())
                .add_custom_transport(Arc::new(TcpFixture {
                    real: carriers[side].clone(),
                    advertised: if side == 1 { unreachable } else { None },
                    discarded: if side == 0 { discarded.clone() } else { None },
                }))
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
            member_fusion_attempt_id(&connections[0]).unwrap(),
            member_fusion_attempt_id(&connections[1]).unwrap()
        );
        let sockets = carriers
            .each_ref()
            .map(|carrier| carrier.bound_socket_addr().unwrap());
        Self {
            nodes,
            endpoints: [client, server],
            connections,
            sockets,
            traffic,
            dials,
            relay,
            url,
            _root: root,
        }
    }

    fn assert_original(&self, ids: [usize; 2]) {
        assert_eq!(
            ids,
            self.connections.each_ref().map(MemberConnection::stable_id)
        );
        assert_eq!(
            self.dials.each_ref().map(|dials| dials.0.load(Relaxed)),
            [1, 0]
        );
    }

    async fn close(self) {
        let [client, server] = self.endpoints;
        let [joining, admin] = self.nodes;
        tokio::join!(
            client.close(),
            server.close(),
            joining.close(),
            admin.close()
        );
        self.relay.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn original_relay_stream_migrates_to_existing_tcp() {
    let pair = Pair::new([false; 2], None, None).await;
    let connections = &pair.connections;
    let ids = connections.each_ref().map(MemberConnection::stable_id);
    let addresses = pair.sockets.map(TcpTransport::address);
    assert!(
        pair.traffic
            .iter()
            .all(|traffic| traffic.sent_packets.load(Relaxed) == 0)
    );
    let start = Instant::now();
    time::timeout(BUSINESS_LIMIT, async {
        // Start activity on the accepting side. Relay was the original path
        // on both endpoints, and UDP is disabled on both of them.
        let mut stream = OriginalStream::open(&connections[1], &connections[0], 0).await;
        let mut next = 1;
        let paths = loop {
            stream.round_trip(next).await;
            next += 1;
            let paths = std::array::from_fn::<_, 2, _>(|side| {
                connections[side]
                    .paths()
                    .iter()
                    .find(|path| {
                        path.is_selected()
                            && path.remote_addr() == &TransportAddr::Custom(addresses[1 - side].clone())
                            && path.local_addr() == &LocalTransportAddr::Custom(Some(addresses[side].clone()))
                            && path
                                .stats()
                                .acknowledgment
                                .is_some_and(|ack| ack.sent_at >= start.into_std())
                    })
                    .map(|path| (path.id(), path.stats().udp_rx.bytes))
            });
            if let [Some(client), Some(server)] = paths {
                break [client, server];
            }
            time::sleep(Duration::from_millis(20)).await;
        };
        let tcp_before = pair.traffic.each_ref().map(|traffic| traffic.received_bytes.load(Relaxed));
        for sequence in next..next + 16 {
            stream.round_trip(sequence).await;
        }
        pair.assert_original(ids);
        for (side, (id, before)) in paths.into_iter().enumerate() {
            let paths = connections[side].paths();
            let path = paths.iter().find(|path| path.id() == id).unwrap();
            assert!(path.is_selected() && path.stats().udp_rx.bytes > before);
            assert!(pair.traffic[side].received_bytes.load(Relaxed) > tcp_before[side]);
            eprintln!(
                "MIO152 migrated side={side} stable={} stream={} local={} remote={} path={id} ack_ms={} received_tcp_bytes={}",
                ids[side], stream.stream_id, pair.sockets[side], pair.sockets[1 - side],
                path.stats().acknowledgment.unwrap().received_at.duration_since(start.into_std()).as_millis(),
                pair.traffic[side].received_bytes.load(Relaxed),
            );
        }
    })
    .await
    .expect("new TCP path gets a fresh ACK and carries bytes of the original stream");
    pair.close().await;
}

#[tokio::test]
async fn disabled_tcp_keeps_the_same_relay_stream() {
    // A low-level caller may install a transport factory. The saved disable
    // still wins on that endpoint, even with real listeners at both ends.
    let pair = Pair::new([false, true], None, None).await;
    keep_relay(&pair, "disabled", Duration::from_secs(1)).await;
    for traffic in &pair.traffic {
        assert_eq!(traffic.sent_packets.load(Relaxed), 0);
        assert_eq!(traffic.received_packets.load(Relaxed), 0);
    }
    pair.close().await;
}

#[tokio::test]
async fn tcp_path_response_alone_keeps_business_on_the_original_relay() {
    let pair = Pair::new([false; 2], None, None).await;
    let ids = pair.connections.each_ref().map(MemberConnection::stable_id);
    let path_id = noq_proto::PathId::from(1u8);
    let suppressed = Arc::new(AtomicBool::new(true));
    for connection in &pair.connections {
        connection.test_suppress_path_acks(path_id, suppressed.clone());
    }
    time::timeout(BUSINESS_LIMIT, async {
        let mut stream = OriginalStream::open(&pair.connections[0], &pair.connections[1], 0).await;
        while !pair.connections.iter().all(|connection| connection.paths().get(path_id).is_some()) {
            time::sleep(Duration::from_millis(15)).await;
        }
        for sequence in 1..=16 {
            stream.round_trip(sequence).await;
            for connection in &pair.connections {
                let paths = connection.paths();
                let tcp = paths.get(path_id).unwrap();
                assert!(matches!(tcp.remote_addr(), TransportAddr::Custom(addr) if addr.id() == DATA_TRANSPORT_ID));
                assert!(tcp.stats().frame_rx.path_response > 0);
                assert!(tcp.stats().acknowledgment.is_none());
                assert!(paths.iter().any(|path| path.is_relay() && path.is_selected()),
                    "an established TCP candidate without a fresh ACK cannot take relay business");
                assert_eq!(tcp.stats().frame_tx.stream, 0);
                assert_eq!(tcp.stats().frame_rx.stream, 0);
            }
        }
        let released = Instant::now();
        suppressed.store(false, Relaxed);
        wait_for_selected_tcp(&pair, released).await;
        stream.round_trip(17).await;
        for connection in &pair.connections {
            let paths = connection.paths();
            assert!(paths.get(path_id).unwrap().stats().frame_rx.stream > 0);
        }
        pair.assert_original(ids);
        eprintln!("MIO152 ACK released stable={ids:?} stream={} path={path_id}", stream.stream_id);
    })
    .await
    .unwrap();
    pair.close().await;
}

#[tokio::test]
async fn explicit_tcp_contact_upgrades_the_original_relay_on_wildcard_listeners() {
    let pair = Pair::bind(Ipv4Addr::UNSPECIFIED, [false; 2], None, None).await;
    assert!(
        pair.sockets
            .iter()
            .all(|socket| socket.ip().is_unspecified())
    );
    let ids = pair.connections.each_ref().map(MemberConnection::stable_id);
    time::timeout(BUSINESS_LIMIT, async {
        let mut stream = OriginalStream::open(&pair.connections[0], &pair.connections[1], 0).await;
        assert!(pair.connections.iter().all(|connection| {
            connection
                .paths()
                .iter()
                .any(|path| path.is_relay() && path.is_selected())
        }));
        let reachable = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), pair.sockets[1].port());
        let reachable = TransportAddr::Custom(TcpTransport::address(reachable));
        let start = Instant::now();
        let reused = pair.endpoints[0]
            .connect(EndpointAddr::new(pair.endpoints[1].id()).with_addrs([reachable.clone()]))
            .await
            .unwrap();
        assert_eq!(reused.stable_id(), ids[0]);
        wait_for_selected_tcp(&pair, start).await;
        for sequence in 1..=16 {
            stream.round_trip(sequence).await;
        }
        assert!(
            pair.traffic
                .iter()
                .all(|traffic| traffic.received_bytes.load(Relaxed) > 0)
        );
        assert!(
            reused
                .paths()
                .iter()
                .any(|p| p.is_selected() && p.remote_addr() == &reachable)
        );
        pair.assert_original(ids);
        eprintln!(
            "MIO152 wildcard stable={ids:?} stream={} route={reachable}",
            stream.stream_id
        );
    })
    .await
    .unwrap();
    pair.close().await;
}

async fn wait_for_selected_tcp(pair: &Pair, after: Instant) {
    while !pair.connections.iter().all(|connection| connection.paths().iter().any(|path| {
        path.is_selected()
            && matches!(path.remote_addr(), TransportAddr::Custom(addr) if addr.id() == DATA_TRANSPORT_ID)
            && path.stats().acknowledgment.is_some_and(|ack| ack.received_at >= after.into_std())
    })) {
        time::sleep(Duration::from_millis(15)).await;
    }
}

#[tokio::test]
async fn refused_tcp_keeps_the_same_relay_stream() {
    let unlistened = TcpSocket::new_v4().unwrap();
    unlistened.bind((Ipv4Addr::LOCALHOST, 0).into()).unwrap();
    let address = unlistened.local_addr().unwrap();
    assert_eq!(
        TcpStream::connect(address).await.unwrap_err().kind(),
        io::ErrorKind::ConnectionRefused
    );
    let pair = Pair::new([false; 2], Some(address), None).await;
    keep_relay(&pair, "refused", Duration::from_secs(7)).await;
    assert!(pair.traffic[0].failed_circuits.load(Relaxed) > 0);
    assert_eq!(pair.traffic[0].received_packets.load(Relaxed), 0);
    drop(unlistened);
    pair.close().await;
}

#[tokio::test]
async fn matching_tcp_preface_without_quic_ack_keeps_the_same_relay_stream() {
    let discarded = Arc::new(AtomicUsize::new(0));
    let pair = Pair::new([false; 2], None, Some(discarded.clone())).await;
    keep_relay(&pair, "preface-only", Duration::from_secs(7)).await;
    for traffic in &pair.traffic {
        assert!(traffic.sent_packets.load(Relaxed) > 0);
        assert!(traffic.received_packets.load(Relaxed) > 0);
    }
    // The two real TCP carriers exchanged and decoded route prefaces and
    // complete transport frames; iroh did not see the candidate's return.
    assert!(discarded.load(Relaxed) > 0);
    pair.close().await;
}

async fn keep_relay(pair: &Pair, scenario: &str, during: Duration) {
    let ids = pair.connections.each_ref().map(MemberConnection::stable_id);
    let start = Instant::now();
    let mut stream = time::timeout(
        BUSINESS_LIMIT,
        OriginalStream::open(&pair.connections[0], &pair.connections[1], 0),
    )
    .await
    .unwrap();
    let mut sequence = 1;
    while start.elapsed() < during || sequence == 1 {
        time::timeout(BUSINESS_LIMIT, stream.round_trip(sequence))
            .await
            .unwrap();
        sequence += 1;
        for connection in &pair.connections {
            let paths = connection.paths();
            assert!(paths.iter().any(|path| {
                path.is_selected() && path.remote_addr() == &TransportAddr::Relay(pair.url.clone())
            }));
            assert!(!paths.iter().any(|path| {
                matches!(path.remote_addr(), TransportAddr::Custom(addr) if addr.id() == DATA_TRANSPORT_ID)
                    && path.stats().acknowledgment.is_some_and(|ack| ack.sent_at >= start.into_std())
            }));
        }
        time::sleep(Duration::from_millis(40)).await;
    }
    pair.assert_original(ids);
    eprintln!(
        "MIO152 {scenario} stable={ids:?} stream={} relay_round_trips={sequence}",
        stream.stream_id
    );
}

/// Fixture faults are limited to advertising a refused socket or withholding
/// real TCP payloads from QUIC after the carrier has received them.
#[derive(Debug)]
struct TcpFixture {
    real: TcpTransport,
    advertised: Option<SocketAddr>,
    discarded: Option<Arc<AtomicUsize>>,
}

impl CustomTransport for TcpFixture {
    fn bind(&self) -> io::Result<Box<dyn CustomEndpoint>> {
        Ok(Box::new(TcpFixtureEndpoint {
            real: self.real.bind()?,
            advertised: self
                .advertised
                .map(|addr| Watchable::new(vec![TcpTransport::address(addr)])),
            discarded: self.discarded.clone(),
        }))
    }
}

#[derive(Debug)]
struct TcpFixtureEndpoint {
    real: Box<dyn CustomEndpoint>,
    advertised: Option<Watchable<Vec<CustomAddr>>>,
    discarded: Option<Arc<AtomicUsize>>,
}

impl CustomEndpoint for TcpFixtureEndpoint {
    fn shutdown_handler(&self) -> Option<Arc<dyn CustomEndpointShutdown>> {
        self.real.shutdown_handler()
    }

    fn watch_local_addrs(&self) -> Direct<Vec<CustomAddr>> {
        match &self.advertised {
            Some(addresses) => addresses.watch(),
            None => self.real.watch_local_addrs(),
        }
    }

    fn create_sender(&self) -> Arc<dyn CustomSender> {
        self.real.create_sender()
    }

    fn poll_recv(
        &mut self,
        cx: &mut Context<'_>,
        bufs: &mut [io::IoSliceMut<'_>],
        metas: &mut [noq_udp::RecvMeta],
        infos: &mut [RecvInfo],
    ) -> Poll<io::Result<usize>> {
        let result = self.real.poll_recv(cx, bufs, metas, infos);
        if let (Some(discarded), Poll::Ready(Ok(packets))) = (&self.discarded, &result) {
            discarded.fetch_add(*packets, Relaxed);
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        result
    }
}
