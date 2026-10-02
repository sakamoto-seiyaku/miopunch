//! Bounded IRC channel envelopes carrying opaque, signaling-only QUIC datagrams.
//! Public delivery tokens and nicknames confer neither identity nor permission.
use futures_channel::mpsc;
use futures_util::{Sink, StreamExt};
use iroh::endpoint::transports::{
    CustomEndpoint, CustomSender, CustomTransport, RecvInfo, Transmit,
};
use iroh_base::CustomAddr;
use n0_watcher::{Direct, Watchable};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncWriteExt, BufReader},
    net::TcpStream,
    sync::watch,
};
use tokio_util::codec::{FramedRead, LinesCodec};

pub const TRANSPORT_ID: u64 = 0x6d696f697263; // "mioirc"
pub const CHANNEL: &str = "#miopunch-v1";
const MAX_DATAGRAM: usize = 2048;
const CHUNK: usize = 128;
const MAX_PARTS: usize = MAX_DATAGRAM.div_ceil(CHUNK);
const QUEUE_DEPTH: usize = 64;
const ASSEMBLIES: usize = 32;
const ASSEMBLY_TTL: Duration = Duration::from_secs(3);
const REPLAY_DEPTH: usize = 128;
const LINE_INTERVAL: Duration = Duration::from_millis(10);
const QUEUE_TTL: Duration = Duration::from_millis(500);
type Route = [u8; 16];
type Service = [u8; 16];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Starting,
    Ready,
    Closed,
}
fn address(service: Service, route: Route) -> CustomAddr {
    let mut data = [0; 32];
    data[..16].copy_from_slice(&service);
    data[16..].copy_from_slice(&route);
    CustomAddr::from_parts(TRANSPORT_ID, &data)
}
fn route_from_addr(addr: &CustomAddr, service: Service) -> io::Result<Route> {
    if addr.id() != TRANSPORT_ID || addr.data().len() != 32 || addr.data()[..16] != service {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "IRC route belongs to another service",
        ));
    }
    Ok(addr.data()[16..].try_into().unwrap())
}

/// Bounded adapter observations; these are not a cryptographic audit.
#[derive(Debug, Default)]
pub struct Traffic {
    sent: AtomicU64,
    received: AtomicU64,
    fragments: AtomicU64,
    rejected: AtomicU64,
    expired: AtomicU64,
    pending: AtomicU64,
}
impl Traffic {
    pub fn sent_packets(&self) -> u64 {
        self.sent.load(Ordering::Relaxed)
    }
    pub fn received_packets(&self) -> u64 {
        self.received.load(Ordering::Relaxed)
    }
    pub fn pending_assemblies(&self) -> u64 {
        self.pending.load(Ordering::Relaxed)
    }
    pub fn expired_assemblies(&self) -> u64 {
        self.expired.load(Ordering::Relaxed)
    }
    pub fn report(&self) -> String {
        format!(
            "IRC_TRAFFIC sent={} received={} fragments={} rejected={} expired={} pending={} max_datagram={MAX_DATAGRAM} assemblies={ASSEMBLIES} queue={QUEUE_DEPTH}",
            self.sent_packets(),
            self.received_packets(),
            self.fragments.load(Ordering::Relaxed),
            self.rejected.load(Ordering::Relaxed),
            self.expired_assemblies(),
            self.pending_assemblies()
        )
    }
}
#[derive(Debug)]
pub struct IrcTransport {
    resource: iroh::RelayUrl,
    service: Service,
    binding: Mutex<Option<watch::Sender<Status>>>,
    traffic: Arc<Traffic>,
}
impl IrcTransport {
    /// Only explicit plaintext IRC origins with no authentication, URL options
    /// or arbitrary channel selection are supported by this qualified adapter.
    pub fn from_resource(resource: iroh::RelayUrl) -> io::Result<Self> {
        if resource.scheme() != "irc"
            || resource.port().is_none_or(|p| p == 0)
            || resource.path() != "/"
            || resource.query().is_some()
            || resource.fragment().is_some()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported IRC service configuration",
            ));
        }
        let origin = crate::resources::resource_origin(resource.as_str())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid IRC resource"))?;
        let mut hash = Sha256::new();
        hash.update(b"miopunch/irc-service/1\0");
        hash.update(origin);
        Ok(Self {
            resource,
            service: hash.finalize()[..16].try_into().unwrap(),
            binding: Mutex::new(None),
            traffic: Arc::new(Traffic::default()),
        })
    }
    pub fn traffic(&self) -> &Arc<Traffic> {
        &self.traffic
    }
    pub async fn wait_ready(&self) -> io::Result<()> {
        let mut status = self
            .binding
            .lock()
            .unwrap()
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "IRC not bound"))?
            .subscribe();
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                match *status.borrow() {
                    Status::Ready => return Ok(()),
                    Status::Closed => {
                        return Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            "IRC carrier stopped",
                        ));
                    }
                    Status::Starting => {}
                }
                status.changed().await.map_err(|_| {
                    io::Error::new(io::ErrorKind::BrokenPipe, "IRC carrier stopped")
                })?;
            }
        })
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "IRC registration timeout"))?
    }
}
#[derive(Debug)]
struct Packet {
    source: Route,
    bytes: Vec<u8>,
}
#[derive(Debug)]
struct Outbound {
    target: Route,
    bytes: Vec<u8>,
    queued: Instant,
}
#[derive(Debug)]
struct Shared {
    route: Route,
    service: Service,
    outbound: Mutex<mpsc::Sender<Outbound>>,
}
struct DriverBinding {
    route: Route,
    service: Service,
    addresses: Watchable<Vec<CustomAddr>>,
    status: watch::Sender<Status>,
    traffic: Arc<Traffic>,
}
#[derive(Debug)]
struct IrcEndpoint {
    shared: Arc<Shared>,
    receiver: tokio::sync::mpsc::Receiver<Packet>,
    addresses: Watchable<Vec<CustomAddr>>,
    status: watch::Sender<Status>,
    driver: tokio::task::JoinHandle<()>,
}
impl Drop for IrcEndpoint {
    fn drop(&mut self) {
        self.driver.abort();
        self.addresses.set(Vec::new()).ok();
        self.status.send_replace(Status::Closed);
    }
}
impl CustomTransport for IrcTransport {
    fn resource(&self) -> Option<(iroh::RelayUrl, iroh::endpoint::RelayResourceUse)> {
        Some((
            self.resource.clone(),
            iroh::endpoint::RelayResourceUse::Signaling,
        ))
    }
    fn bind(&self) -> io::Result<Box<dyn CustomEndpoint>> {
        let (status, _) = watch::channel(Status::Starting);
        {
            let mut binding = self.binding.lock().unwrap();
            if binding
                .as_ref()
                .is_some_and(|s| *s.borrow() != Status::Closed)
            {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "IRC already bound",
                ));
            }
            *binding = Some(status.clone());
        }
        let route = rand::random();
        let (outbound, queue) = mpsc::channel(QUEUE_DEPTH);
        let (incoming, receiver) = tokio::sync::mpsc::channel(QUEUE_DEPTH);
        let shared = Arc::new(Shared {
            route,
            service: self.service,
            outbound: Mutex::new(outbound),
        });
        let addresses = Watchable::new(Vec::new());
        let resource = self.resource.clone();
        let binding = DriverBinding {
            route,
            service: self.service,
            addresses: addresses.clone(),
            status: status.clone(),
            traffic: self.traffic.clone(),
        };
        let driver = tokio::spawn(async move {
            if let Err(error) = drive(resource, &binding, queue, incoming).await {
                eprintln!("IRC_CLOSE error={error}");
            }
            binding.addresses.set(Vec::new()).ok();
            binding.status.send_replace(Status::Closed);
            binding.traffic.pending.store(0, Ordering::Relaxed);
        });
        Ok(Box::new(IrcEndpoint {
            shared,
            receiver,
            addresses,
            status,
            driver,
        }))
    }
}

/// Each envelope fits a traditional 512-byte line including a qualified
/// server's prefix. Fields are public routing metadata, followed by ciphertext.
fn fragments(target: Route, source: Route, bytes: &[u8]) -> Vec<String> {
    let id = rand::random::<u64>();
    let total = bytes.len().div_ceil(CHUNK);
    bytes
        .chunks(CHUNK)
        .enumerate()
        .map(|(index, chunk)| {
            format!(
                "MIR1 {} {} {id:016x} {index:02x} {total:02x} {}",
                hex::encode(target),
                hex::encode(source),
                hex::encode(chunk)
            )
        })
        .collect()
}
struct Assembly {
    created: Instant,
    parts: Vec<Option<Vec<u8>>>,
}
#[derive(Default)]
struct Reassembly {
    pending: BTreeMap<(Route, u64), Assembly>,
    completed: BTreeMap<(Route, u64), Instant>,
}
impl Reassembly {
    fn expire(&mut self, traffic: &Traffic) {
        let before = self.pending.len();
        self.pending
            .retain(|_, a| a.created.elapsed() < ASSEMBLY_TTL);
        traffic
            .expired
            .fetch_add((before - self.pending.len()) as u64, Ordering::Relaxed);
        self.completed.retain(|_, at| at.elapsed() < ASSEMBLY_TTL);
        traffic
            .pending
            .store(self.pending.len() as u64, Ordering::Relaxed);
    }
    fn receive(&mut self, text: &str, own: Route, traffic: &Traffic) -> Option<Packet> {
        self.expire(traffic);
        let fields = text.split(' ').collect::<Vec<_>>();
        if fields.len() != 7 || fields[0] != "MIR1" || fields[1] != hex::encode(own) {
            return None;
        }
        let parsed = (|| {
            let source: Route = hex::decode(fields[2]).ok()?.try_into().ok()?;
            if source == own
                || fields[3].len() != 16
                || fields[4].len() != 2
                || fields[5].len() != 2
            {
                return None;
            }
            let id = u64::from_str_radix(fields[3], 16).ok()?;
            let index = usize::from_str_radix(fields[4], 16).ok()?;
            let total = usize::from_str_radix(fields[5], 16).ok()?;
            if total == 0
                || total > MAX_PARTS
                || index >= total
                || fields[6].is_empty()
                || fields[6].len() > CHUNK * 2
            {
                return None;
            }
            let bytes = hex::decode(fields[6]).ok()?;
            if index + 1 < total && bytes.len() != CHUNK {
                return None;
            }
            Some((source, id, index, total, bytes))
        })();
        let Some((source, id, index, total, bytes)) = parsed else {
            traffic.rejected.fetch_add(1, Ordering::Relaxed);
            return None;
        };
        let key = (source, id);
        if self.completed.contains_key(&key) {
            return None;
        }
        if !self.pending.contains_key(&key) && self.pending.len() >= ASSEMBLIES {
            traffic.rejected.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        let assembly = self.pending.entry(key).or_insert_with(|| Assembly {
            created: Instant::now(),
            parts: vec![None; total],
        });
        if assembly.parts.len() != total
            || assembly.parts[index]
                .as_ref()
                .is_some_and(|old| old != &bytes)
        {
            self.pending.remove(&key);
            traffic.rejected.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        assembly.parts[index] = Some(bytes);
        traffic
            .pending
            .store(self.pending.len() as u64, Ordering::Relaxed);
        if !self.pending[&key].parts.iter().all(Option::is_some) {
            return None;
        }
        let assembly = self.pending.remove(&key).unwrap();
        let bytes = assembly
            .parts
            .into_iter()
            .flatten()
            .flatten()
            .collect::<Vec<_>>();
        if self.completed.len() >= REPLAY_DEPTH {
            if let Some(old) = self
                .completed
                .iter()
                .min_by_key(|(_, at)| *at)
                .map(|(key, _)| *key)
            {
                self.completed.remove(&old);
            }
        }
        self.completed.insert(key, Instant::now());
        traffic
            .pending
            .store(self.pending.len() as u64, Ordering::Relaxed);
        Some(Packet { source, bytes })
    }
}

async fn drive(
    resource: iroh::RelayUrl,
    binding: &DriverBinding,
    queue: mpsc::Receiver<Outbound>,
    incoming: tokio::sync::mpsc::Sender<Packet>,
) -> io::Result<()> {
    let host = resource
        .host_str()
        .unwrap()
        .trim_start_matches('[')
        .trim_end_matches(']');
    let stream = tokio::time::timeout(
        Duration::from_secs(5),
        TcpStream::connect((host, resource.port().unwrap())),
    )
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "IRC TCP connect timeout"))??;
    let (read, mut write) = stream.into_split();
    let nick = format!("mio{}", hex::encode(rand::random::<[u8; 8]>()));
    write_line(&mut write, &format!("NICK {nick}\r\n")).await?;
    write_line(&mut write, "USER mio 0 * :miopunch\r\n").await?;
    let mut lines = FramedRead::new(BufReader::new(read), LinesCodec::new_with_max_length(510));
    let (control, commands) = tokio::sync::mpsc::channel(8);
    // Futures stay in this driver: cancellation drops both halves immediately.
    // Receiving a partial IRC line never competes with an outbound frame read.
    let reader = read_loop(&mut lines, &nick, binding, control, incoming);
    let writer = write_loop(write, binding, queue, commands);
    tokio::select! {result=reader=>result,result=writer=>result}
}
async fn write_line(write: &mut tokio::net::tcp::OwnedWriteHalf, line: &str) -> io::Result<()> {
    if line.len() > 512 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "IRC outbound line too long",
        ));
    }
    tokio::time::timeout(Duration::from_secs(2), write.write_all(line.as_bytes()))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "IRC write stalled"))?
}
async fn write_loop(
    mut write: tokio::net::tcp::OwnedWriteHalf,
    binding: &DriverBinding,
    mut queue: mpsc::Receiver<Outbound>,
    mut commands: tokio::sync::mpsc::Receiver<String>,
) -> io::Result<()> {
    let mut ready = binding.status.subscribe();
    let traffic = &binding.traffic;
    loop {
        tokio::select! {
            command=commands.recv()=>{let Some(command)=command else{return Ok(())};write_line(&mut write,&command).await?;}
            changed=ready.changed(),if *ready.borrow()!=Status::Ready=>{changed.map_err(io::Error::other)?;}
            item=queue.next(),if *ready.borrow()==Status::Ready=>{
                let Some(item)=item else{return Ok(())};
                if item.queued.elapsed()>QUEUE_TTL {traffic.rejected.fetch_add(1,Ordering::Relaxed);continue;}
                for part in fragments(item.target,binding.route,&item.bytes) {
                    write_line(&mut write,&format!("PRIVMSG {CHANNEL} :{part}\r\n")).await?;
                    traffic.fragments.fetch_add(1,Ordering::Relaxed);
                    tokio::time::sleep(LINE_INTERVAL).await;
                }
                traffic.sent.fetch_add(1,Ordering::Relaxed);
            }
        }
    }
}
async fn read_loop(
    lines: &mut FramedRead<BufReader<tokio::net::tcp::OwnedReadHalf>, LinesCodec>,
    nick: &str,
    binding: &DriverBinding,
    control: tokio::sync::mpsc::Sender<String>,
    incoming: tokio::sync::mpsc::Sender<Packet>,
) -> io::Result<()> {
    let route = binding.route;
    let service = binding.service;
    let addresses = &binding.addresses;
    let status = &binding.status;
    let traffic = &binding.traffic;
    let started = Instant::now();
    let mut registered = false;
    let mut capability = false;
    let mut reassembly = Reassembly::default();
    let mut expiry = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            _=expiry.tick()=>{reassembly.expire(&traffic);if *status.borrow()==Status::Starting && started.elapsed()>Duration::from_secs(7) {return Err(io::Error::new(io::ErrorKind::TimedOut,"IRC registration or channel join timeout"));}}
            line=lines.next()=>{
                let line=line.ok_or_else(||io::Error::new(io::ErrorKind::UnexpectedEof,"IRC disconnected"))?.map_err(io::Error::other)?;
                let body=if line.starts_with(':') {line.split_once(' ').map(|(_,body)|body).unwrap_or("")} else {&line};
                let (head,trailing)=body.split_once(" :").unwrap_or((body,""));let args=head.split_whitespace().collect::<Vec<_>>();
                let Some(command)=args.first().copied() else{continue;};
                match command {
                    "PING"=>{control.try_send(format!("PONG :{trailing}\r\n")).map_err(io::Error::other)?;}
                    "001"=>{registered=true;control.try_send(format!("JOIN {CHANNEL}\r\n")).map_err(io::Error::other)?;}
                    "005"=>{capability=true;for arg in &args {if let Some(value)=arg.strip_prefix("NICKLEN=") {if value.parse::<usize>().unwrap_or(0)<nick.len(){return Err(io::Error::new(io::ErrorKind::Unsupported,"IRC nick limit"));}}}}
                    "366" if registered && capability && args.get(2)==Some(&CHANNEL)=>{addresses.set(vec![address(service,route)]).ok();status.send_replace(Status::Ready);}
                    "PRIVMSG" if *status.borrow()==Status::Ready && args.get(1)==Some(&CHANNEL)=>{
                        if let Some(packet)=reassembly.receive(trailing,route,&traffic) {if incoming.try_send(packet).is_ok(){traffic.received.fetch_add(1,Ordering::Relaxed);}else{traffic.rejected.fetch_add(1,Ordering::Relaxed);}}
                    }
                    "ERROR"|"403"|"404"|"405"|"407"|"421"|"433"|"439"|"471"|"473"|"474"|"475"|"477"|"479"=>{return Err(io::Error::new(io::ErrorKind::PermissionDenied,format!("IRC server rejected service: {command}")));}
                    "KICK" if args.get(1)==Some(&CHANNEL) && args.get(2)==Some(&nick)=>{return Err(io::Error::new(io::ErrorKind::PermissionDenied,"IRC channel permission withdrawn"));}
                    _=>{}
                }
            }
        }
    }
}

impl CustomEndpoint for IrcEndpoint {
    fn watch_local_addrs(&self) -> Direct<Vec<CustomAddr>> {
        self.addresses.watch()
    }
    fn create_sender(&self) -> Arc<dyn CustomSender> {
        Arc::new(IrcSender(self.shared.clone()))
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
                        "IRC carrier closed",
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
                address(self.shared.service, packet.source),
                Some(address(self.shared.service, self.shared.route)),
            );
            return Poll::Ready(Ok(1));
        }
    }
}
#[derive(Debug)]
struct IrcSender(Arc<Shared>);
impl CustomSender for IrcSender {
    fn is_valid_send_addr(&self, addr: &CustomAddr) -> bool {
        route_from_addr(addr, self.0.service).is_ok()
    }
    fn poll_send(
        &self,
        cx: &mut Context,
        dst: &CustomAddr,
        _src: Option<&CustomAddr>,
        transmit: &Transmit<'_>,
    ) -> Poll<io::Result<()>> {
        let target = match route_from_addr(dst, self.0.service) {
            Ok(route) => route,
            Err(error) => return Poll::Ready(Err(error)),
        };
        if transmit.contents.is_empty() || transmit.contents.len() > MAX_DATAGRAM {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid IRC datagram size",
            )));
        }
        let mut queue = self.0.outbound.lock().unwrap();
        match Pin::new(&mut *queue).poll_ready(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(_)) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "IRC carrier closed",
            ))),
            Poll::Ready(Ok(())) => Poll::Ready(
                Pin::new(&mut *queue)
                    .start_send(Outbound {
                        target,
                        bytes: transmit.contents.to_vec(),
                        queued: Instant::now(),
                    })
                    .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "IRC carrier closed")),
            ),
        }
    }
}
