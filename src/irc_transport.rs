//! Bounded IRC channel envelopes carrying opaque, signaling-only QUIC datagrams.
//! Public delivery tokens and nicknames confer neither identity nor permission.
use futures_channel::mpsc;
use futures_util::{Sink, StreamExt};
use iroh::endpoint::transports::{
    CustomEndpoint, CustomSender, CustomTransport, RecvInfo, Transmit,
};
use iroh_base::CustomAddr;
use n0_watcher::{Direct, Watchable};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
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
    io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    net::TcpStream,
    sync::watch,
};
use tokio_util::codec::{FramedRead, LinesCodec};

pub use crate::irc_datagram::TRANSPORT_ID;
use crate::irc_datagram::{
    self as wire, AssemblyReport, IrcFacility, MAX_DATAGRAM, Profile, QUEUE_DEPTH, Qualification,
    Reassembly, Route, Service, address, route_from_addr,
};
pub const CHANNEL: &str = "#miopunch-v1";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Starting,
    Ready,
    Closed,
}
/// Each binding owns its observations, so a retiring driver cannot stamp a new route.
#[derive(Debug, Default, Serialize)]
struct ProtocolReport {
    qualification: Qualification,
    assembly: AssemblyReport,
    ready_ms: Option<u64>,
    withdraw_ms: Option<u64>,
    failure: Option<String>,
    queue_pending: u64,
    queue_peak: u64,
    queue_age_ms: u64,
    pong_sent: u64,
    part_sent: bool,
    quit_sent: bool,
    tls: Option<serde_json::Value>,
    max_line_bytes: usize,
    server_prefix: String,
    fragments: Vec<FragmentEvent>,
    accepted_bytes: u64,
    accepted_datagrams: Vec<DatagramEvent>,
    received_bytes: u64,
}
#[derive(Debug, Serialize)]
struct DatagramEvent {
    bytes: usize,
    first_five_bytes: String,
    ciphertext_sha256: String,
}
#[derive(Debug, Serialize)]
struct FragmentEvent {
    at_ms: u64,
    line_bytes: usize,
    prefix: String,
    fragment: String,
}
/// Bounded adapter observations; these are not a cryptographic audit.
#[derive(Debug, Default)]
pub struct Traffic {
    sent: AtomicU64,
    received: AtomicU64,
    fragments: AtomicU64,
    rejected: AtomicU64,
    diagnostics: Mutex<Arc<Mutex<ProtocolReport>>>,
    driver_active: AtomicU64,
}
impl Traffic {
    pub fn sent_packets(&self) -> u64 {
        self.sent.load(Ordering::Relaxed)
    }
    pub fn received_packets(&self) -> u64 {
        self.received.load(Ordering::Relaxed)
    }
    pub fn pending_assemblies(&self) -> u64 {
        let report = self.diagnostics.lock().unwrap().clone();
        let pending = report.lock().unwrap().assembly.pending;
        pending as u64
    }
    pub fn expired_assemblies(&self) -> u64 {
        let report = self.diagnostics.lock().unwrap().clone();
        let expired = report.lock().unwrap().assembly.expired;
        expired
    }
    pub fn snapshot(&self) -> serde_json::Value {
        let report = self.diagnostics.lock().unwrap().clone();
        let report = report.lock().unwrap();
        serde_json::json!({"sent":self.sent_packets(),"received":self.received_packets(),"fragments":self.fragments.load(Ordering::Relaxed),"rejected":self.rejected.load(Ordering::Relaxed),"expired":report.assembly.expired,"pending":report.assembly.pending,"driver_active":self.driver_active.load(Ordering::Relaxed),"protocol":&*report})
    }
    pub fn report(&self) -> String {
        format!(
            "IRC_TRAFFIC sent={} received={} fragments={} rejected={} expired={} pending={} max_datagram={MAX_DATAGRAM} assemblies=32 queue={QUEUE_DEPTH}",
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
    binding: Mutex<Option<Binding>>,
    traffic: Arc<Traffic>,
    channel: String,
    profile: Profile,
    tls_ca: Option<Vec<u8>>,
    bounded_attempt: bool,
}
#[derive(Debug)]
struct Binding {
    status: watch::Sender<Status>,
    shutdown: watch::Sender<bool>,
    addresses: Watchable<Vec<CustomAddr>>,
    report: Arc<Mutex<ProtocolReport>>,
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
            channel: CHANNEL.into(),
            profile: Profile::default(),
            tls_ca: None,
            bounded_attempt: false,
        })
    }
    pub fn from_facility(
        facility: &IrcFacility,
        disabled: &[String],
        profile: Profile,
        tls_ca: Option<Vec<u8>>,
    ) -> io::Result<Self> {
        facility.check_allowed(disabled)?;
        profile.validate()?;
        Ok(Self {
            resource: facility.native_url.parse().map_err(io::Error::other)?,
            service: facility.service_tag()?,
            binding: Mutex::new(None),
            traffic: Arc::new(Traffic::default()),
            channel: facility.channel.clone(),
            profile,
            tls_ca,
            bounded_attempt: true,
        })
    }
    pub fn bind_datagrams(&self) -> io::Result<IrcDatagram> {
        Ok(IrcDatagram {
            endpoint: self.bind_endpoint()?,
        })
    }
    /// A configured node keeps its permitted contact entry alive independently
    /// of one caller's deadline. Explicit close still retires the binding.
    pub fn from_maintained_facility(
        facility: &IrcFacility,
        disabled: &[String],
        profile: Profile,
        tls_ca: Option<Vec<u8>>,
    ) -> io::Result<Self> {
        let mut transport = Self::from_facility(facility, disabled, profile, tls_ca)?;
        transport.bounded_attempt = false;
        Ok(transport)
    }
    pub fn traffic(&self) -> &Arc<Traffic> {
        &self.traffic
    }
    /// Retire this binding immediately, before any QUIC graceful-drain wait.
    /// Readiness publication shares the report lock and cannot revive it.
    pub fn close(&self) {
        if let Some(binding) = self.binding.lock().unwrap().as_ref() {
            let mut report = binding.report.lock().unwrap();
            binding.shutdown.send_replace(true);
            binding.addresses.set(Vec::new()).ok();
            binding.status.send_replace(Status::Closed);
            report.qualification.retire();
        }
    }
    pub async fn wait_ready(&self) -> io::Result<()> {
        let mut status = self
            .binding
            .lock()
            .unwrap()
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "IRC not bound"))?
            .status
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
    shutdown: watch::Sender<bool>,
    report: Arc<Mutex<ProtocolReport>>,
}
struct DriverBinding {
    route: Route,
    service: Service,
    addresses: Watchable<Vec<CustomAddr>>,
    status: watch::Sender<Status>,
    traffic: Arc<Traffic>,
    channel: String,
    profile: Profile,
    started: Instant,
    report: Arc<Mutex<ProtocolReport>>,
}
#[derive(Debug)]
struct IrcEndpoint {
    shared: Arc<Shared>,
    receiver: tokio::sync::mpsc::Receiver<Packet>,
    addresses: Watchable<Vec<CustomAddr>>,
    status: watch::Sender<Status>,
    _driver: tokio::task::JoinHandle<()>,
}
impl Drop for IrcEndpoint {
    fn drop(&mut self) {
        // Serialize retirement with driver-side address publication.
        let mut report = self.shared.report.lock().unwrap();
        self.shared.shutdown.send_replace(true);
        self.addresses.set(Vec::new()).ok();
        self.status.send_replace(Status::Closed);
        report.qualification.retire();
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
        Ok(Box::new(self.bind_endpoint()?))
    }
}
impl IrcTransport {
    fn bind_endpoint(&self) -> io::Result<IrcEndpoint> {
        let (status, _) = watch::channel(Status::Starting);
        let (shutdown, stop) = watch::channel(false);
        let addresses = Watchable::new(Vec::new());
        let report = Arc::new(Mutex::new(ProtocolReport::default()));
        {
            let mut binding = self.binding.lock().unwrap();
            if binding
                .as_ref()
                .is_some_and(|binding| *binding.status.borrow() != Status::Closed)
            {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "IRC already bound",
                ));
            }
            *binding = Some(Binding {
                status: status.clone(),
                shutdown: shutdown.clone(),
                addresses: addresses.clone(),
                report: report.clone(),
            });
        }
        let route = rand::random();
        let (outbound, queue) = mpsc::channel(QUEUE_DEPTH);
        let (incoming, receiver) = tokio::sync::mpsc::channel(QUEUE_DEPTH);
        *self.traffic.diagnostics.lock().unwrap() = report.clone();
        let shared = Arc::new(Shared {
            route,
            service: self.service,
            outbound: Mutex::new(outbound),
            shutdown,
            report: report.clone(),
        });
        let resource = self.resource.clone();
        let binding = DriverBinding {
            route,
            service: self.service,
            addresses: addresses.clone(),
            status: status.clone(),
            traffic: self.traffic.clone(),
            channel: self.channel.clone(),
            profile: self.profile.clone(),
            started: Instant::now(),
            report,
        };
        let tls_ca = self.tls_ca.clone();
        let bounded = self.bounded_attempt;
        self.traffic.driver_active.fetch_add(1, Ordering::Relaxed);
        let driver = tokio::spawn(async move {
            // Retain RX until route retirement, so a closed receiver cannot race ready.
            let retained = incoming.clone();
            let result = if bounded {
                tokio::time::timeout_at(
                    tokio::time::Instant::from_std(binding.started)
                        + Duration::from_millis(binding.profile.attempt_ms),
                    drive(resource, &binding, queue, incoming, stop, tls_ca),
                )
                .await
                .unwrap_or_else(|_| {
                    Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "IRC whole attempt deadline",
                    ))
                })
            } else {
                drive(resource, &binding, queue, incoming, stop, tls_ca).await
            };
            binding.addresses.set(Vec::new()).ok();
            binding.status.send_replace(Status::Closed);
            let mut report = binding.report.lock().unwrap();
            report.failure = result.err().map(|error| error.to_string());
            report.queue_pending = 0;
            report.assembly.pending = 0;
            report.qualification.retire();
            report.withdraw_ms = Some(binding.started.elapsed().as_millis() as u64);
            binding
                .traffic
                .driver_active
                .fetch_sub(1, Ordering::Relaxed);
            drop(retained);
        });
        Ok(IrcEndpoint {
            shared,
            receiver,
            addresses,
            status,
            _driver: driver,
        })
    }
}

trait IrcIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> IrcIo for T {}
type Stream = Box<dyn IrcIo>;
async fn drive(
    resource: iroh::RelayUrl,
    binding: &DriverBinding,
    queue: mpsc::Receiver<Outbound>,
    incoming: tokio::sync::mpsc::Sender<Packet>,
    stop: watch::Receiver<bool>,
    ca: Option<Vec<u8>>,
) -> io::Result<()> {
    let mut stop = stop;
    let mut stream = tokio::select! {biased;
        _=stop.changed()=>return Ok(()),
        stream=connect_stream(&resource,binding,ca)=>stream?,
    };
    let nick = format!("mio{}", hex::encode(rand::random::<[u8; 8]>()));
    tokio::select! {biased;
        _=stop.changed()=>return Ok(()),
        result=async {
            write_line(&mut stream,&format!("NICK {nick}\r\n")).await?;
            write_line(&mut stream,"USER mio 0 * :miopunch\r\n").await
        }=>result?,
    }
    let (read, write) = tokio::io::split(stream);
    let mut lines = FramedRead::new(BufReader::new(read), LinesCodec::new_with_max_length(510));
    let (control, commands) = tokio::sync::mpsc::channel(8);
    let reader = read_loop(&mut lines, &nick, binding, control, incoming);
    let writer = write_loop(write, binding, queue, commands, stop);
    tokio::select! {result=reader=>result,result=writer=>result}
}
async fn connect_stream(
    resource: &iroh::RelayUrl,
    binding: &DriverBinding,
    ca: Option<Vec<u8>>,
) -> io::Result<Stream> {
    let host = resource
        .host_str()
        .unwrap()
        .trim_start_matches('[')
        .trim_end_matches(']');
    let tcp = tokio::time::timeout(
        Duration::from_secs(5),
        TcpStream::connect((
            host,
            resource.port().unwrap_or(if resource.scheme() == "ircs" {
                6697
            } else {
                6667
            }),
        )),
    )
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "IRC TCP timeout"))??;
    let stream: Stream = if resource.scheme() == "ircs" {
        let mut roots = rustls::RootCertStore::empty();
        if let Some(ca) = ca {
            for cert in rustls_pemfile::certs(&mut ca.as_slice()) {
                roots.add(cert?).map_err(io::Error::other)?;
            }
        } else {
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        }
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let name =
            rustls::pki_types::ServerName::try_from(host.to_owned()).map_err(io::Error::other)?;
        let tls = tokio::time::timeout(
            Duration::from_secs(5),
            tokio_rustls::TlsConnector::from(Arc::new(config)).connect(name, tcp),
        )
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "IRC TLS timeout"))??;
        let connection = tls.get_ref().1;
        binding.report.lock().unwrap().tls = Some(serde_json::json!({
            "verified":true,
            "hostname":host,
            "version":connection.protocol_version().map(|value|format!("{value:?}")),
            "cipher":connection.negotiated_cipher_suite().map(|value|format!("{:?}",value.suite())),
            "leaf_der_sha256":connection.peer_certificates().and_then(|certs|certs.first()).map(|cert|hex::encode(Sha256::digest(cert.as_ref())))
        }));
        Box::new(tls)
    } else {
        Box::new(tcp)
    };
    Ok(stream)
}

async fn write_line<W: AsyncWrite + Unpin>(write: &mut W, line: &str) -> io::Result<()> {
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
async fn write_loop<W: AsyncWrite + Unpin>(
    mut write: W,
    binding: &DriverBinding,
    mut queue: mpsc::Receiver<Outbound>,
    mut commands: tokio::sync::mpsc::Receiver<String>,
    mut stop: watch::Receiver<bool>,
) -> io::Result<()> {
    let mut ready = binding.status.subscribe();
    let traffic = &binding.traffic;
    let mut active: std::collections::VecDeque<String> = Default::default();
    let mut next = tokio::time::Instant::now();
    loop {
        tokio::select! {biased;
            _=stop.changed()=>{
                return tokio::time::timeout(Duration::from_secs(2),async {
                    write_line(&mut write,&format!("PART {} :experiment complete\r\nQUIT :experiment complete\r\n",binding.channel)).await?;
                    {let mut report=binding.report.lock().unwrap();report.part_sent=true;report.quit_sent=true;}
                    write.shutdown().await
                }).await.map_err(|_|io::Error::new(io::ErrorKind::TimedOut,"IRC cleanup write deadline"))?;
            }
            command=commands.recv()=>{
                let Some(command)=command else{return Ok(())};write_line(&mut write,&command).await?;
                if command.starts_with("PONG "){binding.report.lock().unwrap().pong_sent+=1;}
            }
            _=tokio::time::sleep_until(next),if !active.is_empty()=>{
                let part=active.pop_front().unwrap();write_line(&mut write,&format!("PRIVMSG {} :{part}\r\n",binding.channel)).await?;
                traffic.fragments.fetch_add(1,Ordering::Relaxed);next=tokio::time::Instant::now()+Duration::from_millis(binding.profile.pacing_ms);
                if active.is_empty(){traffic.sent.fetch_add(1,Ordering::Relaxed);let mut report=binding.report.lock().unwrap();report.queue_pending=report.queue_pending.saturating_sub(1);}
            }
            changed=ready.changed(),if *ready.borrow()!=Status::Ready=>{changed.map_err(io::Error::other)?;}
            item=queue.next(),if *ready.borrow()==Status::Ready&&active.is_empty()=>{
                let Some(item)=item else{return Ok(())};let age=item.queued.elapsed().as_millis() as u64;
                binding.report.lock().unwrap().queue_age_ms=age;
                if age>binding.profile.queue_ttl_ms{traffic.rejected.fetch_add(1,Ordering::Relaxed);let mut report=binding.report.lock().unwrap();report.queue_pending=report.queue_pending.saturating_sub(1);continue;}
                active=wire::fragments(item.target,binding.route,rand::random(),&item.bytes)?.into();
            }
        }
    }
}
async fn read_loop<R: AsyncRead + Unpin>(
    lines: &mut FramedRead<BufReader<R>, LinesCodec>,
    nick: &str,
    binding: &DriverBinding,
    control: tokio::sync::mpsc::Sender<String>,
    incoming: tokio::sync::mpsc::Sender<Packet>,
) -> io::Result<()> {
    let traffic = &binding.traffic;
    let mut qualification = Qualification::default();
    let mut reassembly = Reassembly::new(binding.profile.clone())?;
    let mut expiry = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            _=expiry.tick()=>{
                let now=binding.started.elapsed().as_millis() as u64;reassembly.expire(now);
                binding.report.lock().unwrap().assembly=reassembly.snapshot();
                if *binding.status.borrow()==Status::Starting&&now>7000{return Err(io::Error::new(io::ErrorKind::TimedOut,"IRC registration or JOIN deadline"));}
            }
            line=lines.next()=>{
                let line=line.ok_or_else(||io::Error::new(io::ErrorKind::UnexpectedEof,"IRC disconnected"))?.map_err(io::Error::other)?;let parsed=wire::parse_line(&line)?;
                for command in qualification.receive(&parsed,nick,&binding.channel)?{control.try_send(command).map_err(io::Error::other)?;}
                let now=binding.started.elapsed().as_millis() as u64;
                let is_packet=qualification.ready&&*binding.status.borrow()==Status::Ready&&parsed.command=="PRIVMSG"&&parsed.args.first()==Some(&binding.channel.as_str());
                let packet=if is_packet{reassembly.receive(parsed.trailing,binding.route,now)}else{None};
                {
                let mut report=binding.report.lock().unwrap();
                if *binding.status.borrow()==Status::Closed { qualification.retire(); }
                report.qualification=qualification.clone();
                    report.assembly=reassembly.snapshot();report.max_line_bytes=report.max_line_bytes.max(line.len()+2);
                    if report.server_prefix.is_empty()&&!parsed.prefix.is_empty(){report.server_prefix=parsed.prefix.into();}
                if qualification.ready&&*binding.status.borrow()==Status::Starting {
                    report.ready_ms=Some(now);
                    binding.addresses.set(vec![address(binding.service,binding.route)]).ok();
                    binding.status.send_replace(Status::Ready);
                }
                    if is_packet&&parsed.trailing.starts_with("MIR1 ")&&report.fragments.len()<256{
                        report.fragments.push(FragmentEvent{at_ms:now,line_bytes:line.len()+2,prefix:parsed.prefix.into(),fragment:parsed.trailing.split(' ').nth(4).unwrap_or("").into()});
                    }
                }
                if let Some((source,bytes))=packet{
                    binding.report.lock().unwrap().received_bytes += bytes.len() as u64;
                    traffic.received.fetch_add(1,Ordering::Relaxed);
                    if incoming.try_send(Packet{source,bytes}).is_err(){traffic.received.fetch_sub(1,Ordering::Relaxed);traffic.rejected.fetch_add(1,Ordering::Relaxed);}
                }
            }
        }
    }
}

pub struct IrcDatagram {
    endpoint: IrcEndpoint,
}
impl IrcDatagram {
    pub fn local_addresses(&self) -> Vec<CustomAddr> {
        self.endpoint.addresses.get()
    }
    pub async fn send(&self, target: &CustomAddr, bytes: &[u8]) -> io::Result<()> {
        futures_util::future::poll_fn(|cx| self.endpoint.shared.poll_datagram(cx, target, bytes))
            .await
    }
    pub async fn receive(&mut self) -> io::Result<(CustomAddr, Vec<u8>)> {
        let p = self
            .endpoint
            .receiver
            .recv()
            .await
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "IRC carrier closed"))?;
        Ok((address(self.endpoint.shared.service, p.source), p.bytes))
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
        self.0.poll_datagram(cx, dst, transmit.contents)
    }
}

impl Shared {
    fn poll_datagram(
        &self,
        cx: &mut Context,
        dst: &CustomAddr,
        bytes: &[u8],
    ) -> Poll<io::Result<()>> {
        if *self.shutdown.borrow() {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "IRC carrier retired",
            )));
        }
        let target = match route_from_addr(dst, self.service) {
            Ok(route) => route,
            Err(error) => return Poll::Ready(Err(error)),
        };
        if bytes.is_empty() || bytes.len() > MAX_DATAGRAM {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid IRC datagram size",
            )));
        }
        let mut queue = self.outbound.lock().unwrap();
        match Pin::new(&mut *queue).poll_ready(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(_)) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "IRC carrier closed",
            ))),
            Poll::Ready(Ok(())) => {
                {
                    let mut r = self.report.lock().unwrap();
                    r.queue_pending += 1;
                    r.queue_peak = r.queue_peak.max(r.queue_pending);
                }
                let result = Pin::new(&mut *queue)
                    .start_send(Outbound {
                        target,
                        bytes: bytes.to_vec(),
                        queued: Instant::now(),
                    })
                    .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "IRC carrier closed"));
                if result.is_err() {
                    let mut r = self.report.lock().unwrap();
                    r.queue_pending = r.queue_pending.saturating_sub(1);
                } else {
                    let mut r = self.report.lock().unwrap();
                    r.accepted_bytes += bytes.len() as u64;
                    if r.accepted_datagrams.len() < 256 {
                        r.accepted_datagrams.push(DatagramEvent {
                            bytes: bytes.len(),
                            first_five_bytes: hex::encode(&bytes[..bytes.len().min(5)]),
                            ciphertext_sha256: hex::encode(Sha256::digest(bytes)),
                        });
                    }
                }
                Poll::Ready(result)
            }
        }
    }
}
