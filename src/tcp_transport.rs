//! A bounded, single-hop TCP packet carrier for iroh's custom transport seam.
//! The preface is the sender's listening address: six bytes for IPv4 and
//! eighteen for unscoped IPv6. Each packet is a two-byte big-endian length
//! followed by one complete QUIC datagram. Inner TLS authenticates identity.

use std::collections::{BTreeSet, HashMap};
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll};

use futures_channel::mpsc;
use futures_util::{Sink, StreamExt};
use iroh::endpoint::transports::{
    CustomEndpoint, CustomEndpointShutdown, CustomSender, CustomTransport, FourTuple,
    PathSelection, PathSelectionContext, PathSelector, RecvInfo, Transmit,
};
use iroh_base::CustomAddr;
use n0_watcher::{Direct, Watchable};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpSocket, TcpStream};

const TRANSPORT_ID: u64 = 0x6d696f746370; // "miotcp", retained from MIOPUNCH-18.
/// Current custom carrier that is eligible for admitted member data.
pub const DATA_TRANSPORT_ID: u64 = TRANSPORT_ID;
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

#[derive(Debug, Clone)]
pub struct TcpTransport {
    bind_addr: SocketAddr,
    dual_stack: bool,
    traffic: Arc<Traffic>,
    bound_addr: Arc<Mutex<Option<SocketAddr>>>,
    nat_pmp_mapping: bool,
    bind_policy: Option<iroh::endpoint::RelayResourcePolicy>,
    punching: bool,
    reuse_local_port: bool,
    shared: Arc<Mutex<Weak<Shared>>>,
    observations: Arc<TcpObservations>,
}

#[derive(Debug, Default)]
struct TcpObservations {
    slots: Mutex<HashMap<String, TcpObservationRequest>>,
}

#[derive(Debug)]
struct TcpObservationRequest {
    request: tokio_util::sync::CancellationToken,
    /// Completed diagnostics retain their socket in the actual carrier's
    /// lifecycle, so a formal query can retire it before reusing the tuple.
    retained_stream: Option<Arc<TcpStream>>,
}

impl Drop for TcpObservationRequest {
    fn drop(&mut self) {
        self.request.cancel();
    }
}

impl TcpTransport {
    pub fn new(bind_ip: Ipv4Addr, traffic: Arc<Traffic>) -> Self {
        Self::listen(SocketAddr::V4(SocketAddrV4::new(bind_ip, 0)), traffic)
    }

    pub fn listen(bind_addr: SocketAddr, traffic: Arc<Traffic>) -> Self {
        Self {
            bind_addr,
            dual_stack: false,
            traffic,
            bound_addr: Arc::new(Mutex::new(None)),
            nat_pmp_mapping: false,
            bind_policy: None,
            punching: false,
            reuse_local_port: false,
            shared: Arc::new(Mutex::new(Weak::new())),
            observations: Default::default(),
        }
    }

    /// Bind each available IP family independently for the installed client.
    pub(crate) fn dual_stack(traffic: Arc<Traffic>) -> Self {
        let mut transport = Self::new(Ipv4Addr::UNSPECIFIED, traffic);
        transport.dual_stack = true;
        transport
    }

    /// Explicit local gateway consent. Each bound IPv4 listener owns a TCP
    /// NAT-PMP service, independently of the QUIC socket's UDP mapping.
    pub fn with_nat_pmp_mapping(mut self) -> Self {
        self.nat_pmp_mapping = true;
        self
    }

    pub(crate) fn with_punching(mut self) -> Self {
        self.punching =
            cfg!(any(target_os = "linux", target_os = "windows")) && self.bind_addr.is_ipv4();
        self.reuse_local_port = cfg!(any(
            target_os = "linux",
            target_os = "android",
            target_os = "windows"
        ));
        self
    }

    pub(crate) fn supports_punching(&self) -> bool {
        self.punching
    }

    /// Diagnostics and formal punching share supersession on this actual carrier.
    pub(crate) fn begin_observation(&self, origin: &str) -> tokio_util::sync::CancellationToken {
        let request = tokio_util::sync::CancellationToken::new();
        self.observations.slots.lock().unwrap().insert(
            origin.into(),
            TcpObservationRequest {
                request: request.clone(),
                retained_stream: None,
            },
        );
        request
    }

    pub(crate) fn retain_observation(
        &self,
        origin: &str,
        request: &tokio_util::sync::CancellationToken,
        stream: Arc<TcpStream>,
    ) -> io::Result<()> {
        let mut slots = self.observations.slots.lock().unwrap();
        let slot = slots
            .get_mut(origin)
            .filter(|_| !request.is_cancelled())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::Interrupted, "TCP STUN query superseded")
            })?;
        slot.retained_stream = Some(stream);
        Ok(())
    }

    pub(crate) fn bound_socket_addr_for(&self, remote: SocketAddr) -> io::Result<SocketAddr> {
        self.shared
            .lock()
            .unwrap()
            .upgrade()
            .ok_or_else(|| io::Error::other("TCP endpoint unavailable"))?
            .local_for(remote)
    }

    pub(crate) async fn observe(
        &self,
        observer: SocketAddr,
    ) -> io::Result<(SocketAddr, TcpStream)> {
        self.observe_from(observer, self.bound_socket_addr_for(observer)?)
            .await
    }

    pub(crate) async fn observe_from(
        &self,
        observer: SocketAddr,
        source: SocketAddr,
    ) -> io::Result<(SocketAddr, TcpStream)> {
        let shared = self
            .shared
            .lock()
            .unwrap()
            .upgrade()
            .ok_or_else(|| io::Error::other("TCP endpoint unavailable"))?;
        if !shared.reuse_local_port {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "TCP observation requires retained transport port reuse",
            ));
        }
        let stream = connect_from_listener(source, observer).await?;
        crate::tcp_observation::binding(stream).await
    }

    pub(crate) fn prime(&self, peer: SocketAddr) -> io::Result<TcpCircuitLease> {
        let shared = self
            .shared
            .lock()
            .unwrap()
            .upgrade()
            .ok_or_else(|| io::Error::other("TCP endpoint unavailable"))?;
        if !shared.punching || !peer.is_ipv4() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "TCP punching unavailable",
            ));
        }
        let channel = ensure_channel(&shared, peer)?;
        channel.lock().unwrap().owners += 1;
        Ok(TcpCircuitLease {
            shared,
            peer,
            channel,
        })
    }

    /// Validate this TCP path without selecting it for application data before
    /// the active protocol commits it on the original.
    pub(crate) fn prime_from(
        &self,
        peer: SocketAddr,
        source: SocketAddr,
    ) -> io::Result<TcpCircuitLease> {
        let shared = self
            .shared
            .lock()
            .unwrap()
            .upgrade()
            .ok_or_else(|| io::Error::other("TCP endpoint unavailable"))?;
        if !shared.punching || !peer.is_ipv4() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "TCP punching unavailable",
            ));
        }
        let channel = ensure_channel_from(&shared, peer, Some(source))?;
        channel.lock().unwrap().owners += 1;
        Ok(TcpCircuitLease {
            shared,
            peer,
            channel,
        })
    }

    /// Validate this TCP path without selecting it for application data before
    /// the active protocol commits it on the original.
    pub(crate) fn qualify_after_commit(
        &self,
        original: &iroh::endpoint::ControlHandle,
        peer: SocketAddr,
    ) -> io::Result<PendingTcpPath> {
        let shared = self
            .shared
            .lock()
            .unwrap()
            .upgrade()
            .ok_or_else(|| io::Error::other("TCP endpoint unavailable"))?;
        let connection_id = original.stable_id();
        let open_routes: BTreeSet<_> = original
            .paths()
            .iter()
            .filter_map(|path| {
                if let iroh::TransportAddr::Custom(address) = path.remote_addr() {
                    decode_custom(address).ok()
                } else {
                    None
                }
            })
            .collect();
        {
            let mut paths = shared.pending_paths.lock().unwrap();
            paths.retain(|_, state| state.original.is_open());
            paths
                .entry(connection_id)
                .and_modify(|state| state.qualified.retain(|route| open_routes.contains(route)))
                .or_insert_with(|| ActiveTcpRoutes {
                    original: original.weak_handle(),
                    qualified: open_routes,
                });
        }
        Ok(PendingTcpPath {
            shared,
            connection_id,
            peer,
            local_opens: original.side().is_client(),
        })
    }

    pub(crate) fn data_path_qualified(&self, address: &CustomAddr, connection_id: usize) -> bool {
        self.shared
            .lock()
            .unwrap()
            .upgrade()
            .is_some_and(|shared| shared.data_path_qualified(address, connection_id))
    }

    /// Retain the circuit already carrying an authenticated connection. This
    /// does not require simultaneous-open support or start a new TCP dial.
    pub(crate) fn retain_existing(&self, peer: SocketAddr) -> io::Result<TcpCircuitLease> {
        let shared = self
            .shared
            .lock()
            .unwrap()
            .upgrade()
            .ok_or_else(|| io::Error::other("TCP endpoint unavailable"))?;
        let channel = shared
            .channels
            .lock()
            .unwrap()
            .get(&peer)
            .cloned()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotConnected,
                    "existing TCP circuit unavailable",
                )
            })?;
        {
            let mut retained = channel.lock().unwrap();
            if !retained.active || *retained.cancel.borrow() {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "existing TCP circuit closed",
                ));
            }
            retained.owners += 1;
        }
        Ok(TcpCircuitLease {
            shared,
            peer,
            channel,
        })
    }

    pub fn address(addr: impl Into<SocketAddr>) -> CustomAddr {
        CustomAddr::from_parts(TRANSPORT_ID, &encode(addr.into()))
    }

    pub fn socket_addr(addr: &CustomAddr) -> io::Result<SocketAddr> {
        decode_custom(addr)
    }

    pub fn bound_addr(&self) -> Option<SocketAddrV4> {
        match self.bound_socket_addr()? {
            SocketAddr::V4(addr) => Some(addr),
            SocketAddr::V6(_) => None,
        }
    }

    pub fn bound_socket_addr(&self) -> Option<SocketAddr> {
        *self.bound_addr.lock().unwrap()
    }
}

fn encode(addr: SocketAddr) -> Vec<u8> {
    let mut bytes = match addr {
        SocketAddr::V4(addr) => addr.ip().octets().to_vec(),
        SocketAddr::V6(addr) => addr.ip().octets().to_vec(),
    };
    bytes.extend(addr.port().to_be_bytes());
    bytes
}

fn decode(bytes: &[u8]) -> io::Result<SocketAddr> {
    let addr = match bytes.len() {
        6 => SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::from(<[u8; 4]>::try_from(&bytes[..4]).unwrap()),
            u16::from_be_bytes(bytes[4..].try_into().unwrap()),
        )),
        18 => SocketAddr::V6(SocketAddrV6::new(
            Ipv6Addr::from(<[u8; 16]>::try_from(&bytes[..16]).unwrap()),
            u16::from_be_bytes(bytes[16..].try_into().unwrap()),
            0,
            0,
        )),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid TCP carrier address",
            ));
        }
    };
    if addr.port() == 0
        || matches!(addr, SocketAddr::V6(addr) if addr.ip().to_ipv4_mapped().is_some())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid TCP carrier port or mapped IPv6 address",
        ));
    }
    Ok(addr)
}

fn decode_custom(addr: &CustomAddr) -> io::Result<SocketAddr> {
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
    source: SocketAddr,
    local: SocketAddr,
    bytes: Vec<u8>,
    traffic: Arc<Traffic>,
}

impl Packet {
    fn new(source: SocketAddr, local: SocketAddr, bytes: Vec<u8>, traffic: Arc<Traffic>) -> Self {
        let current = traffic.inbound_outstanding.fetch_add(1, Ordering::Relaxed) + 1;
        traffic.inbound_peak.fetch_max(current, Ordering::Relaxed);
        Self {
            source,
            local,
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
    pending_queue: Option<mpsc::Receiver<QueuedPacket>>,
    active: bool,
    local_addr: Option<SocketAddr>,
    dial_cancel: tokio::sync::watch::Sender<bool>,
    cancel: tokio::sync::watch::Sender<bool>,
    owners: usize,
    established_data: bool,
}

/// A connection attempt owns its share of the real peer circuit. Cancelling
/// one attempt cannot close a circuit still retained by another live session.
pub(crate) struct TcpCircuitLease {
    shared: Arc<Shared>,
    peer: SocketAddr,
    channel: Arc<Mutex<Channel>>,
}

impl TcpCircuitLease {
    pub(crate) fn handshake_ready(&self) -> bool {
        let channel = self.channel.lock().unwrap();
        channel.active && !channel.sender.is_closed()
    }
    pub(crate) fn binding_is_live(&self, source: SocketAddr) -> bool {
        let channel = self.channel.lock().unwrap();
        channel.active && !channel.sender.is_closed() && channel.local_addr == Some(source)
    }
}

pub(crate) struct PendingTcpPath {
    shared: Arc<Shared>,
    connection_id: usize,
    peer: SocketAddr,
    local_opens: bool,
}

impl PendingTcpPath {
    pub(crate) fn commit(self) {
        if let Some(state) = self
            .shared
            .pending_paths
            .lock()
            .unwrap()
            .get_mut(&self.connection_id)
        {
            state.qualified.insert(self.peer);
        }
    }
}

impl Drop for PendingTcpPath {
    fn drop(&mut self) {
        // The opener can retire its exact allocation. The receiver may still
        // see a late peer probe, so keep its per-M allowlist. A failed address
        // adds no state and remains validation-only.
        if self.local_opens {
            self.shared
                .pending_paths
                .lock()
                .unwrap()
                .remove(&self.connection_id);
        }
    }
}

impl TcpCircuitLease {
    /// A committed circuit is now a normal transport channel. Its existing
    /// reader owns idle and shutdown cleanup, including when several Ms
    /// share the same physical circuit.
    pub(crate) fn retain_in_transport(self) {
        self.channel.lock().unwrap().established_data = true;
    }
}

impl Drop for TcpCircuitLease {
    fn drop(&mut self) {
        let last = {
            let mut channel = self.channel.lock().unwrap();
            channel.owners -= 1;
            if channel.owners == 0 && !channel.established_data {
                channel.cancel.send_replace(true);
                true
            } else {
                false
            }
        };
        if last {
            let mut channels = self.shared.channels.lock().unwrap();
            if let Some(current) = channels
                .get(&self.peer)
                .filter(|channel| Arc::ptr_eq(channel, &self.channel))
            {
                let current = current.lock().unwrap();
                if current.owners == 0 && !current.established_data {
                    current.cancel.send_replace(true);
                    drop(current);
                    channels.remove(&self.peer);
                }
            }
        }
    }
}

fn channel(
    sender: mpsc::Sender<QueuedPacket>,
    queue: mpsc::Receiver<QueuedPacket>,
) -> Arc<Mutex<Channel>> {
    let (cancel, _) = tokio::sync::watch::channel(false);
    let (dial_cancel, _) = tokio::sync::watch::channel(false);
    Arc::new(Mutex::new(Channel {
        sender,
        pending_queue: Some(queue),
        active: false,
        local_addr: None,
        dial_cancel,
        cancel,
        owners: 0,
        established_data: false,
    }))
}

#[derive(Debug)]
struct OutboundCircuit {
    peer: SocketAddr,
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
    locals: Vec<SocketAddr>,
    network_changed: Arc<tokio::sync::Notify>,
    channels: Mutex<HashMap<SocketAddr, Arc<Mutex<Channel>>>>,
    pending_paths: Mutex<HashMap<usize, ActiveTcpRoutes>>,
    incoming: tokio::sync::mpsc::Sender<Packet>,
    inbound_limit: Arc<tokio::sync::Semaphore>,
    shutdown: tokio::sync::watch::Sender<bool>,
    traffic: Arc<Traffic>,
    punching: bool,
    reuse_local_port: bool,
    observations: Arc<TcpObservations>,
}

#[derive(Debug)]
struct ActiveTcpRoutes {
    original: iroh::endpoint::WeakControlConnection,
    qualified: BTreeSet<SocketAddr>,
}

impl Shared {
    fn local_for(&self, peer: SocketAddr) -> io::Result<SocketAddr> {
        self.locals
            .iter()
            .copied()
            .find(|local| local.is_ipv4() == peer.is_ipv4())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::AddrNotAvailable,
                    "TCP address family unavailable",
                )
            })
    }

    fn data_path_qualified(&self, addr: &CustomAddr, connection_id: usize) -> bool {
        decode_custom(addr).is_ok_and(|peer| {
            self.pending_paths
                .lock()
                .unwrap()
                .get(&connection_id)
                .is_none_or(|state| state.qualified.contains(&peer))
        })
    }
}

#[derive(Debug)]
struct TcpEndpoint {
    shared: Arc<Shared>,
    receiver: tokio::sync::mpsc::Receiver<Packet>,
    addresses: Watchable<Vec<CustomAddr>>,
    accept_tasks: Vec<tokio::task::JoinHandle<()>>,
    address_task: tokio::task::JoinHandle<()>,
    mapping: Option<Arc<TcpMapping>>,
}

#[derive(Debug)]
struct TcpMapping(portmapper::Client);

impl CustomEndpointShutdown for TcpMapping {
    fn close(&self) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(self.0.deactivate_and_wait())
    }
}

#[derive(Debug)]
struct TcpCleanup {
    shared: Arc<Shared>,
    mapping: Option<Arc<TcpMapping>>,
}

impl CustomEndpointShutdown for TcpCleanup {
    fn close(&self) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async {
            self.shared.shutdown.send_replace(true);
            self.shared.observations.slots.lock().unwrap().clear();
            self.shared.channels.lock().unwrap().clear();
            if let Some(mapping) = &self.mapping {
                mapping.close().await;
            }
        })
    }
}

impl Drop for TcpEndpoint {
    fn drop(&mut self) {
        self.shared.shutdown.send_replace(true);
        self.shared.observations.slots.lock().unwrap().clear();
        for task in &self.accept_tasks {
            task.abort();
        }
        self.address_task.abort();
        self.shared.channels.lock().unwrap().clear();
        if let Some(mapping) = self.mapping.take() {
            // Abrupt receiver drop still gives the owned lease a best-effort
            // release; graceful Endpoint::close already awaited this handler.
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ =
                        tokio::time::timeout(std::time::Duration::from_secs(1), mapping.close())
                            .await;
                });
            }
        }
        eprintln!("{}", self.shared.traffic.report());
    }
}

fn unavailable_family(error: &io::Error) -> bool {
    if matches!(
        error.kind(),
        io::ErrorKind::AddrNotAvailable | io::ErrorKind::Unsupported
    ) {
        return true;
    }
    #[cfg(unix)]
    if error.raw_os_error() == Some(rustix::io::Errno::AFNOSUPPORT.raw_os_error()) {
        return true;
    }
    #[cfg(windows)]
    if error.raw_os_error() == Some(10047) {
        return true;
    } // WSAEAFNOSUPPORT
    false
}

fn bind_listener(address: SocketAddr, reuse: bool) -> io::Result<TcpListener> {
    let socket = socket2::Socket::new(
        if address.is_ipv4() {
            socket2::Domain::IPV4
        } else {
            socket2::Domain::IPV6
        },
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )?;
    if address.is_ipv6() {
        socket.set_only_v6(true)?;
    }
    if reuse {
        socket.set_reuse_address(true)?;
        #[cfg(any(target_os = "linux", target_os = "android"))]
        socket.set_reuse_port(true)?;
    }
    socket.bind(&address.into())?;
    socket.listen(64)?;
    socket.set_nonblocking(true)?;
    TcpListener::from_std(socket.into())
}

impl CustomTransport for TcpTransport {
    fn bind_with_portmapper_config(
        &self,
        config: &iroh::endpoint::PortmapperConfig,
        policy: &iroh::endpoint::RelayResourcePolicy,
    ) -> io::Result<Box<dyn CustomEndpoint>> {
        let mut transport = self.clone();
        transport.nat_pmp_mapping &= matches!(
            config,
            iroh::endpoint::PortmapperConfig::NatPmp
                | iroh::endpoint::PortmapperConfig::Enabled { .. }
        );
        transport.bind_policy = Some(policy.clone());
        transport.bind()
    }

    fn bind(&self) -> io::Result<Box<dyn CustomEndpoint>> {
        if matches!(self.bind_addr, SocketAddr::V6(addr) if addr.scope_id() != 0 || addr.flowinfo() != 0 || addr.ip().to_ipv4_mapped().is_some())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "TCP carrier requires an unscoped, unmapped IPv6 address",
            ));
        }
        let requested = if self.dual_stack {
            vec![
                SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
                SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)),
            ]
        } else {
            vec![self.bind_addr]
        };
        let mut listeners = Vec::new();
        let mut last_error = None;
        for address in requested {
            match bind_listener(address, self.reuse_local_port) {
                Ok(listener) => listeners.push(listener),
                Err(error) if self.dual_stack && unavailable_family(&error) => {
                    last_error = Some(error);
                }
                Err(error) => return Err(error),
            }
        }
        if listeners.is_empty() {
            return Err(last_error.unwrap());
        }
        let locals = listeners
            .iter()
            .map(TcpListener::local_addr)
            .collect::<io::Result<Vec<_>>>()?;
        let local = locals[0];
        *self.bound_addr.lock().unwrap() = Some(local);
        let (incoming, receiver) = tokio::sync::mpsc::channel(QUEUE_DEPTH);
        let (shutdown, _) = tokio::sync::watch::channel(false);
        let shared = Arc::new(Shared {
            locals: locals.clone(),
            network_changed: Arc::new(tokio::sync::Notify::new()),
            channels: Mutex::new(HashMap::new()),
            pending_paths: Mutex::new(HashMap::new()),
            incoming,
            inbound_limit: Arc::new(tokio::sync::Semaphore::new(MAX_CIRCUITS)),
            shutdown,
            traffic: self.traffic.clone(),
            punching: self.punching,
            reuse_local_port: self.reuse_local_port,
            observations: self.observations.clone(),
        });
        *self.shared.lock().unwrap() = Arc::downgrade(&shared);
        let accept_tasks = listeners
            .into_iter()
            .map(|listener| {
                let weak = Arc::downgrade(&shared);
                let mut accept_shutdown = shared.shutdown.subscribe();
                tokio::spawn(async move {
                    loop {
                        let accepted = tokio::select! {
                            accepted=listener.accept()=>accepted,
                            _=accept_shutdown.changed()=>break,
                        };
                        let Ok((stream, _)) = accepted else { break };
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
                })
            })
            .collect();
        let addresses = Watchable::new(locals.iter().copied().map(TcpTransport::address).collect());
        let mapping = if self.nat_pmp_mapping
            && local.is_ipv4()
            && self
                .bind_policy
                .as_ref()
                .is_none_or(|policy| policy.allows_custom(&TcpTransport::address(local)))
        {
            // NAT-PMP targets the default gateway's local address. A listener
            // bound only to some other interface cannot receive that mapping.
            let compatible = local.ip().is_unspecified()
                || netwatch::interfaces::HomeRouter::new()
                    .is_some_and(|router| router.my_ip == Some(local.ip()));
            compatible.then(|| {
                let client = portmapper::Client::new(portmapper::Config {
                    enable_upnp: false,
                    enable_pcp: false,
                    enable_nat_pmp: true,
                    protocol: portmapper::Protocol::Tcp,
                });
                client.update_local_port(local.port().try_into().unwrap());
                Arc::new(TcpMapping(client))
            })
        } else {
            None
        };
        let mut external = mapping
            .as_ref()
            .map(|mapping| mapping.0.watch_external_address());
        let announced = addresses.clone();
        let network_changed = shared.network_changed.clone();
        let address_task = tokio::spawn(async move {
            use n0_watcher::Watcher;
            let monitor = match netwatch::netmon::Monitor::new().await {
                Ok(monitor) => monitor,
                Err(error) => {
                    eprintln!("TCP interface monitor unavailable: {error}");
                    return;
                }
            };
            let mut interfaces = monitor.interface_state();
            loop {
                let state = interfaces.get();
                let mut candidates = BTreeSet::new();
                for local in &locals {
                    if local.ip().is_unspecified() {
                        for interface in state
                            .interfaces
                            .values()
                            .filter(|interface| interface.is_up())
                        {
                            for address in interface.addrs() {
                                let ip = address.addr();
                                if ip.is_ipv4() == local.is_ipv4()
                                    && !ip.is_unspecified()
                                    && !matches!(ip, std::net::IpAddr::V6(ip) if ip.is_unicast_link_local())
                                {
                                    candidates.insert(TcpTransport::address(SocketAddr::new(
                                        ip,
                                        local.port(),
                                    )));
                                }
                            }
                        }
                    } else {
                        candidates.insert(TcpTransport::address(*local));
                    }
                }
                if let Some(address) = external
                    .as_mut()
                    .and_then(|watch| *watch.borrow_and_update())
                {
                    candidates.insert(TcpTransport::address(address));
                }
                announced.set(candidates.into_iter().collect()).ok();
                tokio::select! {
                    _ = interfaces.updated() => {},
                    _ = network_changed.notified() => { let _ = monitor.network_change().await; },
                    _ = async { match external.as_mut() { Some(watch) => { let _ = watch.changed().await; }, None => std::future::pending().await } } => {},
                }
            }
        });
        Ok(Box::new(TcpEndpoint {
            shared,
            receiver,
            addresses,
            accept_tasks,
            address_task,
            mapping,
        }))
    }
}

impl CustomEndpoint for TcpEndpoint {
    fn shutdown_handler(&self) -> Option<Arc<dyn CustomEndpointShutdown>> {
        Some(Arc::new(TcpCleanup {
            shared: self.shared.clone(),
            mapping: self.mapping.clone(),
        }))
    }

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
                Some(TcpTransport::address(packet.local)),
            );
            return Poll::Ready(Ok(1));
        }
    }
}

#[derive(Debug)]
struct TcpSender(Arc<Shared>);

impl CustomSender for TcpSender {
    fn network_changed(&self) {
        self.0.network_changed.notify_one();
    }

    fn is_valid_send_addr(&self, addr: &CustomAddr) -> bool {
        decode_custom(addr).is_ok_and(|addr| self.0.local_for(addr).is_ok())
    }

    fn guards_application_data(&self, addr: &CustomAddr) -> bool {
        self.0.punching
            && addr.id() == DATA_TRANSPORT_ID
            && !self.0.pending_paths.lock().unwrap().is_empty()
    }

    fn is_data_path_qualified(&self, addr: &CustomAddr, connection_id: usize) -> bool {
        self.0.data_path_qualified(addr, connection_id)
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
        if self.0.local_for(target).is_err() {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "TCP carrier address family mismatch",
            )));
        }
        if transmit.contents.is_empty() || transmit.contents.len() > MAX_FRAME {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "oversized QUIC packet",
            )));
        }
        let channel = match ensure_channel(&self.0, target) {
            Ok(channel) => channel,
            Err(error) => return Poll::Ready(Err(error)),
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
                Pin::new(&mut channel.sender)
                    .start_send(packet)
                    .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "TCP carrier closed"))?;
                Poll::Ready(Ok(()))
            }
        }
    }
}

async fn connect_from_listener(local: SocketAddr, target: SocketAddr) -> io::Result<TcpStream> {
    let socket = if local.is_ipv4() {
        TcpSocket::new_v4()?
    } else {
        TcpSocket::new_v6()?
    };
    socket.set_reuseaddr(true)?;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    socket.set_reuseport(true)?;
    socket.bind(local)?;
    socket.connect(target).await
}

fn ensure_channel(shared: &Arc<Shared>, target: SocketAddr) -> io::Result<Arc<Mutex<Channel>>> {
    ensure_channel_from(shared, target, None)
}
fn ensure_channel_from(
    shared: &Arc<Shared>,
    target: SocketAddr,
    source: Option<SocketAddr>,
) -> io::Result<Arc<Mutex<Channel>>> {
    let channel = {
        let mut channels = shared.channels.lock().unwrap();
        if !channels.contains_key(&target) && channels.len() >= MAX_CIRCUITS {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                "too many TCP circuits",
            ));
        }
        channels
            .entry(target)
            .or_insert_with(|| {
                let (sender, receiver) = mpsc::channel(QUEUE_DEPTH);
                let channel = channel(sender,receiver);
                let shared = shared.clone();
                let task_channel = channel.clone();
                let mut cancel=channel.lock().unwrap().cancel.subscribe();
                let mut dial_cancel=channel.lock().unwrap().dial_cancel.subscribe();
                let mut shutdown=shared.shutdown.subscribe();
                tokio::spawn(async move {
                    let connecting=tokio::time::timeout(std::time::Duration::from_secs(5), async {
                        if shared.reuse_local_port && target.is_ipv4() {
                            connect_from_listener(source.unwrap_or(shared.local_for(target)?), target).await
                        } else {
                            TcpStream::connect(target).await
                        }
                    });
                    let result=tokio::select! {
                        result=connecting=>result,
                        _=cancel.changed()=>Ok(Err(io::Error::new(io::ErrorKind::Interrupted,"TCP attempt canceled"))),
                        _=shutdown.changed()=>Ok(Err(io::Error::new(io::ErrorKind::Interrupted,"TCP endpoint closed"))),
                        _=dial_cancel.changed()=>Ok(Err(io::Error::new(io::ErrorKind::Interrupted,"TCP incoming circuit won"))),
                    };
                    match result {
                        Ok(Ok(stream)) => {
                            run_stream(
                                shared,
                                stream,
                                Some(OutboundCircuit {
                                    peer: target,
                                    channel: task_channel,
                                }),
                            )
                            .await
                        }
                        result => {
                            let channel=task_channel.lock().unwrap();
                            if channel.active {return;}
                            drop(channel);
                            let err = match result {
                                Ok(Err(err)) => err,
                                Err(_) => {
                                    io::Error::new(io::ErrorKind::TimedOut, "TCP connect timeout")
                                }
                                Ok(Ok(_)) => unreachable!(),
                            };
                            shared
                                .traffic
                                .cancelled_circuits
                                .fetch_add(u64::from(err.kind()==io::ErrorKind::Interrupted), Ordering::Relaxed);
                            shared.traffic.failed_circuits
                                .fetch_add(u64::from(err.kind()!=io::ErrorKind::Interrupted), Ordering::Relaxed);
                            eprintln!("TCP_CLOSE peer={target} result={err}");
                            remove_channel(&shared, target, &task_channel);
                        }
                    }
                });
                channel
            })
            .clone()
    };
    Ok(channel)
}

fn remove_channel(shared: &Shared, peer: SocketAddr, channel: &Arc<Mutex<Channel>>) {
    let mut channels = shared.channels.lock().unwrap();
    if channels
        .get(&peer)
        .is_some_and(|current| Arc::ptr_eq(current, channel))
    {
        let mut prepared = channel.lock().unwrap();
        if prepared.owners != 0 && !prepared.established_data {
            // A failed physical dial still belongs to this preparation lease.
            // Preserve its queue so an incoming socket can still win. Neither
            // error exit may replace the cancellation owner before retirement.
            return;
        }
        prepared.pending_queue.take();
        drop(prepared);
        channels.remove(&peer);
    }
    shared
        .pending_paths
        .lock()
        .unwrap()
        .retain(|_, state| state.original.is_open());
}

async fn run_stream(shared: Arc<Shared>, mut stream: TcpStream, outbound: Option<OutboundCircuit>) {
    let outbound_cleanup = outbound
        .as_ref()
        .map(|circuit| (circuit.peer, circuit.channel.clone()));
    let mut shutdown = shared.shutdown.subscribe();
    let mut owns_channel = false;
    let mut early_cancel = outbound
        .as_ref()
        .map(|circuit| circuit.channel.lock().unwrap().cancel.subscribe());
    let result = async {
        if *shutdown.borrow() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "TCP endpoint closed"));
        }
        if early_cancel.as_ref().is_some_and(|cancel|*cancel.borrow()) {return Err(io::Error::new(io::ErrorKind::Interrupted,"TCP attempt canceled"));}
        let socket_local = stream.local_addr()?;
        let local = shared.local_for(socket_local)?;
        let advertised = SocketAddr::new(socket_local.ip(), local.port());
        let mut preface = vec![0; if local.is_ipv4() { 6 } else { 18 }];
        tokio::select! {
            result = async {
                tokio::time::timeout(std::time::Duration::from_secs(5), stream.write_all(&encode(advertised)))
                    .await.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "TCP preface write timeout"))??;
                tokio::time::timeout(std::time::Duration::from_secs(5), stream.read_exact(&mut preface))
                    .await.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "TCP preface read timeout"))??;
                Ok::<(), io::Error>(())
            } => result?,
            _ = shutdown.changed() => return Err(io::Error::new(io::ErrorKind::Interrupted, "TCP endpoint closed")),
            _ = async {match &mut early_cancel {Some(cancel)=>{let _=cancel.changed().await;},None=>std::future::pending::<()>().await}} => return Err(io::Error::new(io::ErrorKind::Interrupted, "TCP attempt canceled")),
        }
        let remote = decode(&preface)?;
        let (peer, channel) = if let Some(circuit) = outbound {
            // The dialed route may be a gateway mapping. Its private preface
            // is only an untrusted return hint, never endpoint authentication.
            // Keep responses on this circuit under the original dialed route;
            // inner iroh TLS authenticates the requested endpoint identity.
            (circuit.peer, circuit.channel)
        } else {
            let socket_peer=stream.peer_addr()?;
            // A coordinated same-port peer's actual connected tuple survives
            // NAT. The private preface remains a hint and cannot authenticate it.
            let peer=if shared.punching && (socket_peer.port()==remote.port() || shared.channels.lock().unwrap().contains_key(&socket_peer)) {socket_peer} else {remote};
            let mut channels = shared.channels.lock().unwrap();
            if channels.len() >= MAX_CIRCUITS && !channels.contains_key(&peer) {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "TCP peer circuit unavailable",
                ));
            }
            let channel=channels.entry(peer).or_insert_with(||{let (sender,receiver)=mpsc::channel(QUEUE_DEPTH);channel(sender,receiver)}).clone();
            drop(channels);
            (peer, channel)
        };
        let mut queue={
            let mut channel=channel.lock().unwrap();
            if *channel.cancel.borrow() {return Err(io::Error::new(io::ErrorKind::Interrupted,"TCP attempt canceled"));}
            let queue=channel.pending_queue.take().ok_or_else(||io::Error::new(io::ErrorKind::AlreadyExists,"TCP circuit already active"))?;
            channel.active=true;
            channel.local_addr=Some(socket_local);
            channel.dial_cancel.send_replace(true);
            queue
        };
        owns_channel=true;
        let mut circuit_cancel=channel.lock().unwrap().cancel.subscribe();
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
                        .send(Packet::new(peer, local, bytes, shared.traffic.clone())),
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
            _ = circuit_cancel.changed() => Err(io::Error::new(io::ErrorKind::Interrupted, "TCP attempt canceled")),
        };
        writer_task.abort();
        remove_channel(&shared, peer, &channel);
        read_result
    }
    .await;
    if let Some((peer, channel)) = outbound_cleanup {
        if owns_channel || !channel.lock().unwrap().active {
            remove_channel(&shared, peer, &channel);
        }
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
            .write_all(&encode(SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::LOCALHOST,
                41000,
            ))))
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

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[tokio::test]
    async fn incoming_circuit_can_win_after_outgoing_preface_failure() {
        let traffic = Arc::new(Traffic::default());
        let transport = TcpTransport::new(Ipv4Addr::LOCALHOST, traffic.clone()).with_punching();
        let mut endpoint = transport.bind().unwrap();
        let peer = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let peer_address = peer.local_addr().unwrap();
        let lease = transport.prime(peer_address).unwrap();
        let (mut failed, _) = peer.accept().await.unwrap();
        let mut preface = [0; 6];
        failed.read_exact(&mut preface).await.unwrap();
        drop(failed);
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while traffic.failed_circuits.load(Ordering::Relaxed) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the outgoing preface must fail while its preparation is still held");

        let mut incoming = TcpStream::connect(transport.bound_addr().unwrap()).await.unwrap();
        incoming.read_exact(&mut preface).await.unwrap();
        incoming.write_all(&encode(peer_address)).await.unwrap();
        incoming.write_all(b"\0\x07witness").await.unwrap();
        let mut packet = [0; 7];
        let mut bufs = [io::IoSliceMut::new(&mut packet)];
        let mut metas = [noq_udp::RecvMeta::default()];
        let mut infos = [RecvInfo::new(TcpTransport::address(peer_address), None)];
        let received = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            std::future::poll_fn(|cx| endpoint.poll_recv(cx, &mut bufs, &mut metas, &mut infos)),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(received, 1);
        assert_eq!(metas[0].len, 7);
        assert_eq!(&packet, b"witness");

        drop(lease);
        let closed = tokio::time::timeout(std::time::Duration::from_secs(1), incoming.read(&mut [0]))
            .await.unwrap();
        assert!(matches!(closed, Ok(0))
            || closed.is_err_and(|error| error.kind() == io::ErrorKind::ConnectionReset));
        drop(endpoint);
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
