//! A bounded, single-hop TCP packet carrier for iroh's custom transport seam.
//! The six-byte preface is the sender's IPv4 listening address; each packet is
//! a two-byte big-endian length followed by one complete QUIC datagram.

use std::collections::HashMap;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use futures_channel::mpsc;
use futures_util::{Sink, StreamExt};
use iroh::endpoint::transports::{
    CustomEndpoint, CustomSender, CustomTransport, FourTuple, PathSelection, PathSelectionContext,
    PathSelector, RecvInfo, Transmit,
};
use iroh_base::CustomAddr;
use n0_watcher::{Direct, Watchable};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const TRANSPORT_ID: u64 = 0x6d696f746370; // "miotcp", local to this experiment.
const MAX_FRAME: usize = 2048;
const QUEUE_DEPTH: usize = 32;
const MAX_CIRCUITS: usize = 64;

#[derive(Debug)]
pub struct TcpOnly;

impl PathSelector for TcpOnly {
    fn select(&self, context: &PathSelectionContext<'_>) -> PathSelection {
        let mut selection = PathSelection::none();
        if let Some(path) = context.paths().find(|path| {
            matches!(path.network_path(), FourTuple::Custom { remote, .. } if remote.id() == TRANSPORT_ID)
        }) {
            selection.set(&path);
        }
        selection
    }
}

#[derive(Debug, Default)]
pub struct Traffic {
    pub sent_packets: AtomicU64,
    pub sent_bytes: AtomicU64,
    pub received_packets: AtomicU64,
    pub received_bytes: AtomicU64,
    pub closed_circuits: AtomicU64,
    pub failed_circuits: AtomicU64,
    pub cancelled_circuits: AtomicU64,
    pub outbound_outstanding: AtomicU64,
    pub outbound_peak: AtomicU64,
    pub inbound_outstanding: AtomicU64,
    pub inbound_peak: AtomicU64,
}

impl Traffic {
    pub fn report(&self) -> String {
        format!(
            "TCP_TRAFFIC sent_packets={} sent_bytes={} received_packets={} received_bytes={} closed={} failed={} cancelled={} outbound_outstanding={} outbound_peak={} inbound_outstanding={} inbound_peak={} queue_depth={} max_frame={} max_circuits={}",
            self.sent_packets.load(Ordering::Relaxed),
            self.sent_bytes.load(Ordering::Relaxed),
            self.received_packets.load(Ordering::Relaxed),
            self.received_bytes.load(Ordering::Relaxed),
            self.closed_circuits.load(Ordering::Relaxed),
            self.failed_circuits.load(Ordering::Relaxed),
            self.cancelled_circuits.load(Ordering::Relaxed),
            self.outbound_outstanding.load(Ordering::Relaxed),
            self.outbound_peak.load(Ordering::Relaxed),
            self.inbound_outstanding.load(Ordering::Relaxed),
            self.inbound_peak.load(Ordering::Relaxed),
            QUEUE_DEPTH,
            MAX_FRAME,
            MAX_CIRCUITS,
        )
    }
}

#[derive(Debug)]
pub struct TcpTransport {
    bind_ip: Ipv4Addr,
    traffic: Arc<Traffic>,
    bound_addr: Arc<Mutex<Option<SocketAddrV4>>>,
}

impl TcpTransport {
    pub fn new(bind_ip: Ipv4Addr, traffic: Arc<Traffic>) -> Self {
        Self {
            bind_ip,
            traffic,
            bound_addr: Arc::new(Mutex::new(None)),
        }
    }

    pub fn address(addr: SocketAddrV4) -> CustomAddr {
        CustomAddr::from_parts(TRANSPORT_ID, &encode(addr))
    }

    pub fn bound_addr(&self) -> Option<SocketAddrV4> {
        *self.bound_addr.lock().unwrap()
    }
}

fn encode(addr: SocketAddrV4) -> [u8; 6] {
    let mut bytes = [0; 6];
    bytes[..4].copy_from_slice(&addr.ip().octets());
    bytes[4..].copy_from_slice(&addr.port().to_be_bytes());
    bytes
}

fn decode(bytes: &[u8]) -> io::Result<SocketAddrV4> {
    let [a, b, c, d, hi, lo] = bytes else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid TCP carrier address",
        ));
    };
    let port = u16::from_be_bytes([*hi, *lo]);
    if port == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "zero TCP carrier port",
        ));
    }
    Ok(SocketAddrV4::new(Ipv4Addr::new(*a, *b, *c, *d), port))
}

fn decode_custom(addr: &CustomAddr) -> io::Result<SocketAddrV4> {
    if addr.id() != TRANSPORT_ID {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "wrong transport ID",
        ));
    }
    decode(addr.data())
}

#[derive(Debug)]
struct Packet {
    source: SocketAddrV4,
    bytes: Vec<u8>,
    traffic: Arc<Traffic>,
}

impl Packet {
    fn new(source: SocketAddrV4, bytes: Vec<u8>, traffic: Arc<Traffic>) -> Self {
        let current = traffic.inbound_outstanding.fetch_add(1, Ordering::Relaxed) + 1;
        traffic.inbound_peak.fetch_max(current, Ordering::Relaxed);
        Self {
            source,
            bytes,
            traffic,
        }
    }
}

impl Drop for Packet {
    fn drop(&mut self) {
        self.traffic
            .inbound_outstanding
            .fetch_sub(1, Ordering::Relaxed);
    }
}

#[derive(Debug)]
struct Channel {
    sender: mpsc::Sender<QueuedPacket>,
}

#[derive(Debug)]
struct OutboundCircuit {
    peer: SocketAddrV4,
    queue: mpsc::Receiver<QueuedPacket>,
    channel: Arc<Mutex<Channel>>,
}

#[derive(Debug)]
struct QueuedPacket {
    bytes: Vec<u8>,
    traffic: Arc<Traffic>,
}

impl QueuedPacket {
    fn new(bytes: Vec<u8>, traffic: Arc<Traffic>) -> Self {
        let current = traffic.outbound_outstanding.fetch_add(1, Ordering::Relaxed) + 1;
        traffic.outbound_peak.fetch_max(current, Ordering::Relaxed);
        Self { bytes, traffic }
    }
}

impl Drop for QueuedPacket {
    fn drop(&mut self) {
        self.traffic
            .outbound_outstanding
            .fetch_sub(1, Ordering::Relaxed);
    }
}

#[derive(Debug)]
struct Shared {
    local: SocketAddrV4,
    channels: Mutex<HashMap<SocketAddrV4, Arc<Mutex<Channel>>>>,
    incoming: tokio::sync::mpsc::Sender<Packet>,
    inbound_limit: Arc<tokio::sync::Semaphore>,
    shutdown: tokio::sync::watch::Sender<bool>,
    traffic: Arc<Traffic>,
}

#[derive(Debug)]
struct TcpEndpoint {
    shared: Arc<Shared>,
    receiver: tokio::sync::mpsc::Receiver<Packet>,
    addresses: Watchable<Vec<CustomAddr>>,
    accept_task: tokio::task::JoinHandle<()>,
}

impl Drop for TcpEndpoint {
    fn drop(&mut self) {
        self.shared.shutdown.send_replace(true);
        self.accept_task.abort();
        self.shared.channels.lock().unwrap().clear();
    }
}

impl CustomTransport for TcpTransport {
    fn bind(&self) -> io::Result<Box<dyn CustomEndpoint>> {
        let socket =
            std::net::TcpListener::bind(SocketAddr::V4(SocketAddrV4::new(self.bind_ip, 0)))?;
        socket.set_nonblocking(true)?;
        let local = match socket.local_addr()? {
            SocketAddr::V4(addr) => addr,
            _ => unreachable!(),
        };
        *self.bound_addr.lock().unwrap() = Some(local);
        let listener = TcpListener::from_std(socket)?;
        let (incoming, receiver) = tokio::sync::mpsc::channel(QUEUE_DEPTH);
        let (shutdown, _) = tokio::sync::watch::channel(false);
        let shared = Arc::new(Shared {
            local,
            channels: Mutex::new(HashMap::new()),
            incoming,
            inbound_limit: Arc::new(tokio::sync::Semaphore::new(MAX_CIRCUITS)),
            shutdown,
            traffic: self.traffic.clone(),
        });
        let weak = Arc::downgrade(&shared);
        let accept_task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let Some(shared) = weak.upgrade() else { break };
                if let Ok(permit) = shared.inbound_limit.clone().try_acquire_owned() {
                    tokio::spawn(async move {
                        let _permit = permit;
                        run_stream(shared, stream, None).await;
                    });
                } else {
                    shared
                        .traffic
                        .failed_circuits
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
        });
        Ok(Box::new(TcpEndpoint {
            shared,
            receiver,
            addresses: Watchable::new(vec![TcpTransport::address(local)]),
            accept_task,
        }))
    }
}

impl CustomEndpoint for TcpEndpoint {
    fn watch_local_addrs(&self) -> Direct<Vec<CustomAddr>> {
        self.addresses.watch()
    }

    fn create_sender(&self) -> Arc<dyn CustomSender> {
        Arc::new(TcpSender(self.shared.clone()))
    }

    fn poll_recv(
        &mut self,
        cx: &mut Context,
        bufs: &mut [io::IoSliceMut<'_>],
        metas: &mut [noq_udp::RecvMeta],
        infos: &mut [RecvInfo],
    ) -> Poll<io::Result<usize>> {
        if bufs.is_empty() {
            return Poll::Ready(Ok(0));
        }
        loop {
            let packet = match self.receiver.poll_recv(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(packet)) => packet,
                Poll::Ready(None) => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "TCP carrier closed",
                    )));
                }
            };
            if packet.bytes.len() > bufs[0].len() {
                continue;
            }
            bufs[0][..packet.bytes.len()].copy_from_slice(&packet.bytes);
            metas[0].len = packet.bytes.len();
            metas[0].stride = packet.bytes.len();
            infos[0] = RecvInfo::new(
                TcpTransport::address(packet.source),
                Some(TcpTransport::address(self.shared.local)),
            );
            return Poll::Ready(Ok(1));
        }
    }
}

#[derive(Debug)]
struct TcpSender(Arc<Shared>);

impl CustomSender for TcpSender {
    fn is_valid_send_addr(&self, addr: &CustomAddr) -> bool {
        decode_custom(addr).is_ok()
    }

    fn poll_send(
        &self,
        cx: &mut Context,
        dst: &CustomAddr,
        _src: Option<&CustomAddr>,
        transmit: &Transmit<'_>,
    ) -> Poll<io::Result<()>> {
        let target = match decode_custom(dst) {
            Ok(addr) => addr,
            Err(err) => return Poll::Ready(Err(err)),
        };
        if transmit.contents.is_empty() || transmit.contents.len() > MAX_FRAME {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "oversized QUIC packet",
            )));
        }
        let channel = {
            let mut channels = self.0.channels.lock().unwrap();
            if !channels.contains_key(&target) && channels.len() >= MAX_CIRCUITS {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::OutOfMemory,
                    "too many TCP circuits",
                )));
            }
            channels
                .entry(target)
                .or_insert_with(|| {
                    let (sender, receiver) = mpsc::channel(QUEUE_DEPTH);
                    let channel = Arc::new(Mutex::new(Channel { sender }));
                    let shared = self.0.clone();
                    let task_channel = channel.clone();
                    tokio::spawn(async move {
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(5),
                            TcpStream::connect(target),
                        )
                        .await
                        {
                            Ok(Ok(stream)) => {
                                run_stream(
                                    shared,
                                    stream,
                                    Some(OutboundCircuit {
                                        peer: target,
                                        queue: receiver,
                                        channel: task_channel,
                                    }),
                                )
                                .await
                            }
                            result => {
                                let err = match result {
                                    Ok(Err(err)) => err,
                                    Err(_) => io::Error::new(
                                        io::ErrorKind::TimedOut,
                                        "TCP connect timeout",
                                    ),
                                    Ok(Ok(_)) => unreachable!(),
                                };
                                shared
                                    .traffic
                                    .failed_circuits
                                    .fetch_add(1, Ordering::Relaxed);
                                eprintln!("TCP_CLOSE peer={target} result={err}");
                                remove_channel(&shared, target, &task_channel);
                            }
                        }
                    });
                    channel
                })
                .clone()
        };
        let mut channel = channel.lock().unwrap();
        match Pin::new(&mut channel.sender).poll_ready(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(_)) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "TCP carrier closed",
            ))),
            Poll::Ready(Ok(())) => {
                let packet = QueuedPacket::new(transmit.contents.to_vec(), self.0.traffic.clone());
                match Pin::new(&mut channel.sender).start_send(packet) {
                    Ok(()) => Poll::Ready(Ok(())),
                    Err(_) => Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "TCP carrier closed",
                    ))),
                }
            }
        }
    }
}

fn remove_channel(shared: &Shared, peer: SocketAddrV4, channel: &Arc<Mutex<Channel>>) {
    let mut channels = shared.channels.lock().unwrap();
    if channels
        .get(&peer)
        .is_some_and(|current| Arc::ptr_eq(current, channel))
    {
        channels.remove(&peer);
    }
}

async fn run_stream(shared: Arc<Shared>, mut stream: TcpStream, outbound: Option<OutboundCircuit>) {
    let outbound_cleanup = outbound
        .as_ref()
        .map(|circuit| (circuit.peer, circuit.channel.clone()));
    let mut shutdown = shared.shutdown.subscribe();
    let result = async {
        if *shutdown.borrow() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "TCP endpoint closed"));
        }
        let mut preface = [0; 6];
        tokio::select! {
            result = async {
                tokio::time::timeout(std::time::Duration::from_secs(5), stream.write_all(&encode(shared.local)))
                    .await.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "TCP preface write timeout"))??;
                tokio::time::timeout(std::time::Duration::from_secs(5), stream.read_exact(&mut preface))
                    .await.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "TCP preface read timeout"))??;
                Ok::<(), io::Error>(())
            } => result?,
            _ = shutdown.changed() => return Err(io::Error::new(io::ErrorKind::Interrupted, "TCP endpoint closed")),
        }
        let remote = decode(&preface)?;
        let (peer, mut queue, channel) = if let Some(circuit) = outbound {
            if circuit.peer != remote {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "TCP peer address mismatch",
                ));
            }
            (circuit.peer, circuit.queue, circuit.channel)
        } else {
            let (sender, receiver) = mpsc::channel(QUEUE_DEPTH);
            let channel = Arc::new(Mutex::new(Channel { sender }));
            let mut channels = shared.channels.lock().unwrap();
            if channels.len() >= MAX_CIRCUITS || channels.contains_key(&remote) {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "TCP peer circuit unavailable",
                ));
            }
            channels.insert(remote, channel.clone());
            drop(channels);
            (remote, receiver, channel)
        };
        let (mut reader, mut writer) = stream.into_split();
        let traffic = shared.traffic.clone();
        let mut writer_task = tokio::spawn(async move {
            while let Some(packet) = queue.next().await {
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    writer
                        .write_all(&(packet.bytes.len() as u16).to_be_bytes())
                        .await?;
                    writer.write_all(&packet.bytes).await
                })
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "TCP write timeout"))??;
                traffic.sent_packets.fetch_add(1, Ordering::Relaxed);
                traffic
                    .sent_bytes
                    .fetch_add(packet.bytes.len() as u64, Ordering::Relaxed);
            }
            Ok::<(), io::Error>(())
        });
        let read_packets = async {
            loop {
                let mut len_buf = [0; 2];
                match tokio::time::timeout(
                    std::time::Duration::from_secs(15),
                    reader.read(&mut len_buf[..1]),
                )
                .await
                {
                    Ok(Ok(0)) => break Ok(()),
                    Ok(Ok(_)) => {}
                    Ok(Err(err)) => break Err(err),
                    Err(_) => {
                        break Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "TCP frame header timeout",
                        ));
                    }
                }
                match tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    reader.read_exact(&mut len_buf[1..]),
                )
                .await
                {
                    Ok(Ok(_)) => {}
                    Ok(Err(err)) => break Err(err),
                    Err(_) => {
                        break Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "TCP frame header timeout",
                        ));
                    }
                }
                let len = u16::from_be_bytes(len_buf) as usize;
                if len == 0 || len > MAX_FRAME {
                    break Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "invalid TCP frame length",
                    ));
                }
                let mut bytes = vec![0; len];
                match tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    reader.read_exact(&mut bytes),
                )
                .await
                {
                    Ok(Ok(_)) => {}
                    Ok(Err(err)) => break Err(err),
                    Err(_) => {
                        break Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "TCP frame body timeout",
                        ));
                    }
                }
                shared
                    .traffic
                    .received_packets
                    .fetch_add(1, Ordering::Relaxed);
                shared
                    .traffic
                    .received_bytes
                    .fetch_add(len as u64, Ordering::Relaxed);
                match tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    shared
                        .incoming
                        .send(Packet::new(peer, bytes, shared.traffic.clone())),
                )
                .await
                {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) => {
                        break Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            "iroh receiver closed",
                        ));
                    }
                    Err(_) => {
                        break Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "iroh receiver backpressure timeout",
                        ));
                    }
                }
            }
        };
        let read_result = tokio::select! {
            result = read_packets => result,
            result = &mut writer_task => result.map_err(io::Error::other)?,
            _ = shutdown.changed() => Err(io::Error::new(io::ErrorKind::Interrupted, "TCP endpoint closed")),
        };
        writer_task.abort();
        remove_channel(&shared, peer, &channel);
        read_result
    }
    .await;
    if let Some((peer, channel)) = outbound_cleanup {
        remove_channel(&shared, peer, &channel);
    }
    match result {
        Ok(()) => {
            shared
                .traffic
                .closed_circuits
                .fetch_add(1, Ordering::Relaxed);
        }
        Err(err) if err.kind() == io::ErrorKind::Interrupted => {
            shared
                .traffic
                .cancelled_circuits
                .fetch_add(1, Ordering::Relaxed);
            eprintln!("TCP_CLOSE result=cancelled");
        }
        Err(err) => {
            shared
                .traffic
                .failed_circuits
                .fetch_add(1, Ordering::Relaxed);
            eprintln!("TCP_CLOSE result={err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn open_carrier() -> (Box<dyn CustomEndpoint>, TcpStream) {
        let transport = TcpTransport::new(Ipv4Addr::LOCALHOST, Arc::new(Traffic::default()));
        let endpoint = transport.bind().unwrap();
        let mut stream = TcpStream::connect(transport.bound_addr().unwrap())
            .await
            .unwrap();
        let mut preface = [0; 6];
        stream.read_exact(&mut preface).await.unwrap();
        stream
            .write_all(&encode(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 41000)))
            .await
            .unwrap();
        (endpoint, stream)
    }

    #[tokio::test]
    async fn dropping_endpoint_cancels_an_open_tcp_carrier() {
        let (endpoint, mut stream) = open_carrier().await;
        drop(endpoint);
        let mut byte = [0];
        let read = tokio::time::timeout(std::time::Duration::from_secs(1), stream.read(&mut byte))
            .await
            .unwrap();
        match read {
            Ok(0) => {}
            Err(err) if err.kind() == io::ErrorKind::ConnectionReset => {}
            other => panic!("unexpected TCP close after endpoint drop: {other:?}"),
        }
    }

    #[tokio::test]
    async fn unread_inbound_queue_closes_in_bounded_time() {
        let (_endpoint, mut stream) = open_carrier().await;
        let frame = vec![0; MAX_FRAME];
        for _ in 0..=QUEUE_DEPTH {
            stream
                .write_all(&(MAX_FRAME as u16).to_be_bytes())
                .await
                .unwrap();
            stream.write_all(&frame).await.unwrap();
        }
        let mut byte = [0];
        let read = tokio::time::timeout(std::time::Duration::from_secs(7), stream.read(&mut byte))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(read, 0);
    }
}
