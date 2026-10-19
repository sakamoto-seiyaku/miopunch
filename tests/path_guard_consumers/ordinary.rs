// Fixed public consumer: identical bytes on published iroh and the fork.
// The custom carrier implements only the original required transport methods.
use iroh::{
    Endpoint, EndpointAddr, RelayMode, TransportAddr,
    endpoint::{
        PortmapperConfig, presets,
        transports::{CustomEndpoint, CustomSender, CustomTransport, RecvInfo, Transmit},
    },
};
use iroh_base::CustomAddr;
use noq::{AsyncUdpSocket, Runtime, UdpSender};
use std::{
    io::{self, IoSliceMut},
    net::{Ipv4Addr, SocketAddr},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};
const ID: u64 = 0x6d103;
const ALPN: &[u8] = b"m103/ordinary/1";
#[derive(Debug, Default)]
struct Counts {
    valid: AtomicU64,
    polls: AtomicU64,
    sends: AtomicU64,
    send_bytes: AtomicU64,
    receives: AtomicU64,
    receive_bytes: AtomicU64,
}
fn custom(addr: SocketAddr) -> CustomAddr {
    CustomAddr::from_parts(ID, addr.to_string().as_bytes())
}
fn physical(addr: &CustomAddr) -> Option<SocketAddr> {
    (addr.id() == ID)
        .then(|| std::str::from_utf8(addr.data()).ok()?.parse().ok())
        .flatten()
}
#[derive(Debug)]
struct LegacyCarrier {
    socket: Mutex<Option<Box<dyn AsyncUdpSocket>>>,
    address: SocketAddr,
    counts: Arc<Counts>,
}
impl LegacyCarrier {
    fn new() -> Arc<Self> {
        let socket = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        socket.set_nonblocking(true).unwrap();
        let address = socket.local_addr().unwrap();
        Arc::new(Self {
            socket: Mutex::new(Some(noq::TokioRuntime.wrap_udp_socket(socket).unwrap())),
            address,
            counts: Arc::new(Counts::default()),
        })
    }
}
#[derive(Debug)]
struct LegacyEndpoint {
    socket: Box<dyn AsyncUdpSocket>,
    address: CustomAddr,
    watch: n0_watcher::Watchable<Vec<CustomAddr>>,
    counts: Arc<Counts>,
}
#[derive(Debug)]
struct LegacySender {
    sender: Mutex<Pin<Box<dyn UdpSender>>>,
    counts: Arc<Counts>,
}
impl CustomTransport for LegacyCarrier {
    fn bind(&self) -> io::Result<Box<dyn CustomEndpoint>> {
        let address = custom(self.address);
        Ok(Box::new(LegacyEndpoint {
            socket: self.socket.lock().unwrap().take().expect("one bind"),
            watch: n0_watcher::Watchable::new(vec![address.clone()]),
            address,
            counts: self.counts.clone(),
        }))
    }
}
impl CustomEndpoint for LegacyEndpoint {
    fn watch_local_addrs(&self) -> n0_watcher::Direct<Vec<CustomAddr>> {
        self.watch.watch()
    }
    fn create_sender(&self) -> Arc<dyn CustomSender> {
        Arc::new(LegacySender {
            sender: Mutex::new(self.socket.create_sender()),
            counts: self.counts.clone(),
        })
    }
    fn poll_recv(
        &mut self,
        cx: &mut Context<'_>,
        bufs: &mut [IoSliceMut<'_>],
        metas: &mut [noq::udp::RecvMeta],
        infos: &mut [RecvInfo],
    ) -> Poll<io::Result<usize>> {
        match self.socket.poll_recv(cx, bufs, metas) {
            Poll::Ready(Ok(n)) => {
                for i in 0..n {
                    infos[i] = RecvInfo::new(custom(metas[i].addr), Some(self.address.clone()));
                    self.counts.receives.fetch_add(1, Ordering::SeqCst);
                    self.counts
                        .receive_bytes
                        .fetch_add(metas[i].len as u64, Ordering::SeqCst);
                }
                Poll::Ready(Ok(n))
            }
            other => other,
        }
    }
}
impl CustomSender for LegacySender {
    fn is_valid_send_addr(&self, addr: &CustomAddr) -> bool {
        self.counts.valid.fetch_add(1, Ordering::SeqCst);
        physical(addr).is_some()
    }
    fn poll_send(
        &self,
        cx: &mut Context<'_>,
        dst: &CustomAddr,
        _src: Option<&CustomAddr>,
        transmit: &Transmit<'_>,
    ) -> Poll<io::Result<()>> {
        self.counts.polls.fetch_add(1, Ordering::SeqCst);
        let destination = physical(dst).ok_or(io::ErrorKind::InvalidInput)?;
        let packet = noq::udp::Transmit {
            destination,
            ecn: None,
            contents: transmit.contents,
            segment_size: transmit.segment_size,
            src_ip: None,
        };
        let result = self.sender.lock().unwrap().as_mut().poll_send(&packet, cx);
        if matches!(result, Poll::Ready(Ok(()))) {
            self.counts.sends.fetch_add(1, Ordering::SeqCst);
            self.counts
                .send_bytes
                .fetch_add(packet.contents.len() as u64, Ordering::SeqCst);
        }
        result
    }
}
async fn exchange(case: &str) {
    let left = LegacyCarrier::new();
    let right = LegacyCarrier::new();
    let mut relay = None;
    if case == "relay" {
        let mut config = iroh_relay::server::ServerConfig::default();
        config.relay = Some(iroh_relay::server::RelayConfig::new((
            Ipv4Addr::LOCALHOST,
            0,
        )));
        relay = Some(iroh_relay::server::Server::spawn(config).await.unwrap());
    }
    let builder = || {
        Endpoint::builder(presets::Minimal)
            .portmapper_config(PortmapperConfig::Disabled)
            .clear_ip_transports()
            .relay_mode(RelayMode::Disabled)
    };
    let mut a = builder();
    let mut b = builder().alpns(vec![ALPN.to_vec()]);
    match case {
        "ip" => {
            a = a.bind_addr((Ipv4Addr::LOCALHOST, 0)).unwrap();
            b = b.bind_addr((Ipv4Addr::LOCALHOST, 0)).unwrap();
        }
        "relay" => {
            let map = iroh::RelayMap::from_iter([relay.as_ref().unwrap().http_url().unwrap()]);
            a = a.relay_mode(RelayMode::Custom(map.clone()));
            b = b.relay_mode(RelayMode::Custom(map));
        }
        "legacy" => {
            a = a.add_custom_transport(left.clone());
            b = b.add_custom_transport(right.clone());
        }
        _ => panic!("unknown case"),
    }
    let client = a.bind().await.unwrap();
    let server = b.bind().await.unwrap();
    if case == "relay" {
        tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(client.online(), server.online());
        })
        .await
        .unwrap();
    }
    let target = if case == "legacy" {
        EndpointAddr::new(server.id()).with_addrs([TransportAddr::Custom(custom(right.address))])
    } else {
        server.addr()
    };
    let started = Instant::now();
    let (connection, peer) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(client.connect(target, ALPN), async {
            server.accept().await.unwrap().await
        })
    })
    .await
    .unwrap();
    let connection = connection.unwrap();
    let peer = peer.unwrap();
    let snapshot = |c: &Counts| {
        [
            c.valid.load(Ordering::SeqCst),
            c.polls.load(Ordering::SeqCst),
            c.sends.load(Ordering::SeqCst),
            c.send_bytes.load(Ordering::SeqCst),
            c.receives.load(Ordering::SeqCst),
            c.receive_bytes.load(Ordering::SeqCst),
        ]
    };
    let before = [snapshot(&left.counts), snapshot(&right.counts)];
    let payload: Vec<u8> = (0..32768).map(|i| (i % 251) as u8).collect();
    let work = tokio::time::timeout(Duration::from_secs(10), async {
        for _ in 0..3 {
            tokio::join!(
                async {
                    let (mut send, mut recv) = connection.open_bi().await.unwrap();
                    send.write_all(&payload).await.unwrap();
                    send.finish().unwrap();
                    assert_eq!(recv.read_to_end(65536).await.unwrap(), payload);
                    send.stopped().await.unwrap();
                },
                async {
                    let (mut send, mut recv) = peer.accept_bi().await.unwrap();
                    assert_eq!(recv.read_to_end(65536).await.unwrap(), payload);
                    send.write_all(&payload).await.unwrap();
                    send.finish().unwrap();
                    send.stopped().await.unwrap();
                }
            );
        }
    })
    .await;
    let after_business = [snapshot(&left.counts), snapshot(&right.counts)];
    connection.close(0u32.into(), b"ordinary complete");
    let closed = tokio::time::timeout(Duration::from_secs(3), peer.closed()).await;
    tokio::join!(client.close(), server.close());
    drop(connection);
    drop(peer);
    drop(client);
    drop(server);
    let mut relay_closed = true;
    if let Some(relay) = relay {
        let addr = relay.http_addr().unwrap();
        relay.shutdown().await.unwrap();
        relay_closed = std::net::TcpListener::bind(addr).is_ok();
    }
    work.expect("all bytes and stream ACKs complete");
    closed.expect("peer observes close");
    assert!(relay_closed);
    let counts = |c: &Counts| {
        [
            c.valid.load(Ordering::SeqCst),
            c.polls.load(Ordering::SeqCst),
            c.sends.load(Ordering::SeqCst),
            c.send_bytes.load(Ordering::SeqCst),
            c.receives.load(Ordering::SeqCst),
            c.receive_bytes.load(Ordering::SeqCst),
        ]
    };
    let lc = counts(&left.counts);
    let rc = counts(&right.counts);
    if case == "legacy" {
        assert!(lc[0] > 0 && rc[0] > 0 && lc[2] > 0 && rc[2] > 0 && lc[4] > 0 && rc[4] > 0);
    } else {
        assert_eq!(lc, [0; 6]);
        assert_eq!(rc, [0; 6]);
    }
    let addresses = [left.address, right.address];
    drop(left);
    drop(right);
    for address in addresses {
        let socket = std::net::UdpSocket::bind(address).expect("custom UDP ownership released");
        drop(socket);
    }
    println!(
        "{{\"case\":\"{case}\",\"streams\":3,\"payloadBytesEachWayPerStream\":32768,\"elapsedMs\":{},\"legacyCounterUnits\":[\"is_valid_send_addr_calls\",\"poll_send_calls\",\"successful_send_calls\",\"sent_ciphertext_bytes\",\"recv_buffers\",\"received_ciphertext_bytes\"],\"beforeBusinessCounters\":{:?},\"afterBusinessCounters\":{:?},\"left\":{:?},\"right\":{:?},\"normalClose\":true,\"ownedUdpPortsRebound\":true,\"relayClosedAndPortRebound\":{relay_closed}}}",
        started.elapsed().as_millis(),
        before,
        after_business,
        lc,
        rc
    );
}
fn main() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let args: Vec<_> = std::env::args().skip(1).collect();
            let cases: Vec<&str> = if args.is_empty() {
                vec!["ip", "relay", "legacy"]
            } else {
                args.iter().map(String::as_str).collect()
            };
            for case in cases {
                exchange(case).await;
            }
        });
}
