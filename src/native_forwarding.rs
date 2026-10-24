//! Real native sockets behind the existing identity, admission and service ACL.
use crate::{
    data_stream::DataStream,
    forwarding::{self, Protocol},
    native_client::{ClientError, NativeClient},
    service_policy::ServiceAccess,
};
use iroh::{EndpointAddr, endpoint::Connection};
use p2panda_core::VerifyingKey;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
    task::{JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;

const MAX_LISTENERS: usize = 8;
const MAX_FLOWS: usize = 16;
const IDLE: Duration = Duration::from_secs(60);
// Receive the complete ordinary UDP payload before applying our smaller wire
// bound. Windows reports WSAEMSGSIZE instead of Linux's truncation behavior.
const UDP_RECEIVE_BYTES: usize = 65_536;

struct Flow {
    peer: VerifyingKey,
    protocol: Protocol,
    provider: bool,
    target: SocketAddr,
    cancel: CancellationToken,
    connection: Option<Connection>,
    dropped_datagrams: u64,
}
struct Receiver {
    peer: VerifyingKey,
    protocol: Protocol,
    target: Option<SocketAddr>,
    cancel: CancellationToken,
    connection: Connection,
}
struct Listener {
    peer: VerifyingKey,
    protocol: Protocol,
    target: SocketAddr,
    listen: SocketAddr,
    cancel: CancellationToken,
    join: JoinHandle<()>,
    counters: Arc<Mutex<ListenerCounters>>,
}
#[derive(Default, serde::Serialize)]
struct ListenerCounters {
    accepted: u64,
    completed: u64,
    failed: u64,
    dropped_datagrams: u64,
    last_error: Option<&'static str>,
}
#[derive(Default)]
struct State {
    access: Option<ServiceAccess>,
    scopes: BTreeMap<Protocol, BTreeSet<SocketAddr>>,
    listeners: BTreeMap<u64, Listener>,
    flows: BTreeMap<u64, Flow>,
    receivers: BTreeMap<u64, Receiver>,
    next: u64,
    closed: bool,
}
pub(crate) struct NativeForwarding {
    state: Mutex<State>,
    setup: tokio::sync::Mutex<()>,
    gate: crate::admission_gate::AdmissionGate,
    joined: tokio::sync::Notify,
    receiver_tasks: tokio::sync::Mutex<JoinSet<()>>,
}
pub(crate) struct ForwardingOwner(pub Arc<NativeForwarding>);
impl Drop for ForwardingOwner {
    fn drop(&mut self) {
        self.0.cancel_all();
    }
}
struct FlowOwner {
    runtime: Arc<NativeForwarding>,
    id: u64,
    cancel: CancellationToken,
}
impl FlowOwner {
    fn datagram_dropped(&self) {
        if let Some(flow) = self.runtime.state.lock().unwrap().flows.get_mut(&self.id) {
            flow.dropped_datagrams = flow.dropped_datagrams.saturating_add(1);
        }
    }
}
impl Drop for FlowOwner {
    fn drop(&mut self) {
        let flow = self.runtime.state.lock().unwrap().flows.remove(&self.id);
        self.cancel.cancel();
        if let Some(flow) = flow {
            if let Some(connection) = flow.connection {
                connection.close(0u32.into(), b"forward flow ended");
            }
        }
        self.runtime.joined.notify_waiters();
    }
}
struct ReceiverOwner {
    runtime: Arc<NativeForwarding>,
    id: u64,
    cancel: CancellationToken,
}
impl Drop for ReceiverOwner {
    fn drop(&mut self) {
        let receiver = self
            .runtime
            .state
            .lock()
            .unwrap()
            .receivers
            .remove(&self.id);
        if let Some(receiver) = receiver {
            receiver
                .connection
                .close(0u32.into(), b"forward receiver ended");
        }
        self.runtime.joined.notify_waiters();
    }
}
impl NativeForwarding {
    pub fn new(gate: crate::admission_gate::AdmissionGate) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State::default()),
            setup: tokio::sync::Mutex::new(()),
            gate,
            joined: Default::default(),
            receiver_tasks: Default::default(),
        })
    }
    pub fn enforce_access(&self, access: &ServiceAccess) {
        for protocol in [Protocol::Tcp, Protocol::Udp] {
            self.gate.update_service_policy(
                protocol.alpn(),
                access.guest_pairs_for(protocol.service()),
                Default::default(),
            );
        }
        let mut state = self.state.lock().unwrap();
        for receiver in state.receivers.values() {
            if !Self::provides(&state, access, receiver.peer, receiver.protocol)
                || receiver.target.is_some_and(|target| {
                    !state
                        .scopes
                        .get(&receiver.protocol)
                        .is_some_and(|targets| targets.contains(&target))
                })
            {
                receiver.cancel.cancel();
                receiver
                    .connection
                    .close(1u32.into(), b"forward receiver authorization ended");
            }
        }
        for flow in state.flows.values() {
            if !Self::permitted(&state, access, flow) {
                flow.cancel.cancel();
                if let Some(connection) = &flow.connection {
                    connection.close(1u32.into(), b"forward authorization ended");
                }
            }
        }
        for listener in state.listeners.values() {
            if !forwarding::peer_permitted(access, listener.peer, listener.protocol) {
                listener.cancel.cancel();
            }
        }
        state.access = Some(access.clone());
    }
    fn permitted(state: &State, access: &ServiceAccess, flow: &Flow) -> bool {
        forwarding::peer_permitted(access, flow.peer, flow.protocol)
            && (!flow.provider
                || Self::provides(state, access, flow.peer, flow.protocol)
                    && state
                        .scopes
                        .get(&flow.protocol)
                        .is_some_and(|targets| targets.contains(&flow.target)))
    }
    fn provides(
        state: &State,
        access: &ServiceAccess,
        peer: VerifyingKey,
        protocol: Protocol,
    ) -> bool {
        !state.closed
            && state.scopes.contains_key(&protocol)
            && access.member(access.local)
            && access.permits_service(protocol.service(), peer)
    }
    /// Transfer a long-lived service flow out of the generic ingress handler.
    /// Full business capacity must leave normal policy/control receipt usable.
    pub async fn receive(
        self: &Arc<Self>,
        connection: Connection,
        protocol: Protocol,
    ) -> Result<(), ClientError> {
        let peer = VerifyingKey::from_bytes(connection.remote_id().as_bytes())?;
        let mut tasks = self.receiver_tasks.lock().await;
        while tasks.try_join_next().is_some() {}
        let owner = {
            let mut state = self.state.lock().unwrap();
            if state.receivers.len() >= MAX_FLOWS
                || !state
                    .access
                    .as_ref()
                    .is_some_and(|access| Self::provides(&state, access, peer, protocol))
            {
                connection.close(1u32.into(), b"forward receiver unavailable");
                return Err("forward receiver unavailable".into());
            }
            state.next = state
                .next
                .checked_add(1)
                .ok_or("forward identifier exhausted")?;
            let id = state.next;
            let cancel = CancellationToken::new();
            state.receivers.insert(
                id,
                Receiver {
                    peer,
                    protocol,
                    target: None,
                    cancel: cancel.clone(),
                    connection: connection.clone(),
                },
            );
            ReceiverOwner {
                runtime: self.clone(),
                id,
                cancel,
            }
        };
        let runtime = self.clone();
        tasks.spawn(async move {
            let work = runtime.serve(connection, protocol, &owner);
            tokio::select! { biased; _ = owner.cancel.cancelled() => {}, _ = work => {} }
        });
        Ok(())
    }
    fn claim(
        self: &Arc<Self>,
        peer: VerifyingKey,
        protocol: Protocol,
        target: SocketAddr,
        provider: bool,
        cancel: CancellationToken,
    ) -> Result<FlowOwner, ClientError> {
        let mut state = self.state.lock().unwrap();
        let flow = Flow {
            peer,
            protocol,
            target,
            provider,
            cancel: cancel.clone(),
            connection: None,
            dropped_datagrams: 0,
        };
        if state.closed
            || cancel.is_cancelled()
            || state.flows.len() >= MAX_FLOWS
            || !state
                .access
                .as_ref()
                .is_some_and(|access| Self::permitted(&state, access, &flow))
        {
            return Err("forward unavailable or denied".into());
        }
        state.next = state
            .next
            .checked_add(1)
            .ok_or("forward identifier exhausted")?;
        let id = state.next;
        state.flows.insert(id, flow);
        Ok(FlowOwner {
            runtime: self.clone(),
            id,
            cancel,
        })
    }
    fn attach(&self, owner: &FlowOwner, connection: Connection) -> Result<(), ClientError> {
        let mut state = self.state.lock().unwrap();
        let flow = state.flows.get_mut(&owner.id).ok_or("forward ended")?;
        if flow.cancel.is_cancelled() {
            connection.close(0u32.into(), b"forward stopped");
            return Err("forward stopped".into());
        }
        flow.connection = Some(connection);
        Ok(())
    }
    fn cancel_all(&self) {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        for listener in state.listeners.values() {
            listener.cancel.cancel();
        }
        for flow in state.flows.values() {
            flow.cancel.cancel();
            if let Some(connection) = &flow.connection {
                connection.close(0u32.into(), b"forward stopped");
            }
        }
        for receiver in state.receivers.values() {
            receiver.cancel.cancel();
            receiver
                .connection
                .close(0u32.into(), b"forward receiver stopped");
        }
    }
    pub async fn close(&self) -> Result<(), ClientError> {
        self.cancel_all();
        let _setup = self.setup.lock().await;
        let listeners = std::mem::take(&mut self.state.lock().unwrap().listeners);
        let mut failed = false;
        for (_, listener) in listeners {
            failed |= listener.join.await.is_err();
        }
        let mut tasks = self.receiver_tasks.lock().await;
        while let Some(result) = tasks.join_next().await {
            failed |= result.is_err();
        }
        if failed {
            return Err("forward listener failed".into());
        }
        Ok(())
    }
    async fn join_retired(&self, protocol: Protocol) -> Result<(), ClientError> {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let done = self.joined.notified();
                let pending = {
                    let state = self.state.lock().unwrap();
                    state.receivers.values().any(|receiver| {
                        receiver.protocol == protocol && receiver.cancel.is_cancelled()
                    }) || state.flows.values().any(|flow| {
                        flow.provider && flow.protocol == protocol && flow.cancel.is_cancelled()
                    })
                };
                if !pending {
                    break;
                }
                done.await;
            }
        })
        .await?;
        Ok(())
    }
    pub fn status(&self) -> Value {
        let state = self.state.lock().unwrap();
        json!({"native_listener":true,"provider":true,"max_listeners":MAX_LISTENERS,"max_flows":MAX_FLOWS,
            "active_flows":state.flows.len(),"provider_workers":state.receivers.len(),"max_provider_workers":MAX_FLOWS,"services":([Protocol::Tcp,Protocol::Udp].map(|protocol|json!({"protocol":protocol,
                "enabled":state.scopes.contains_key(&protocol),"targets":state.scopes.get(&protocol)}))),
            "flows":state.flows.iter().map(|(id,flow)|json!({"id":id.to_string(),"protocol":flow.protocol,"peer":flow.peer,
                "target":flow.target,"provider":flow.provider,"dropped_datagrams":flow.dropped_datagrams,"transport":flow.connection.as_ref().map(crate::native_client::path_status)})).collect::<Vec<_>>(),
            "listeners":state.listeners.iter().map(|(id,l)|json!({"id":id.to_string(),"protocol":l.protocol,"peer":l.peer,
                "target":l.target,"listen":l.listen,"running":!l.join.is_finished() && !l.cancel.is_cancelled(),
                "counters":*l.counters.lock().unwrap()})).collect::<Vec<_>>()})
    }
    async fn serve(
        self: &Arc<Self>,
        connection: Connection,
        protocol: Protocol,
        receiver: &ReceiverOwner,
    ) -> Result<(), ClientError> {
        // The guard closes rejected transports as well as accepted flows.
        let peer = VerifyingKey::from_bytes(connection.remote_id().as_bytes())?;
        let result = async {
            let mut stream = tokio::time::timeout(Duration::from_secs(10), DataStream::accept(&connection)).await??;
            let open: forwarding::Open = tokio::time::timeout(Duration::from_secs(10), stream.read_control()).await??;
            if let Some(current) = self.state.lock().unwrap().receivers.get_mut(&receiver.id) { current.target = Some(open.target); }
            let owner = self.claim(peer, protocol, open.target, true, receiver.cancel.child_token())?;
            self.attach(&owner, connection.clone())?;
            tokio::select! {
                biased;
                _ = owner.cancel.cancelled() => Err("forward stopped".into()),
                result = async {
                    match protocol {
                        Protocol::Tcp => {
                            let socket = tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(open.target)).await??;
                            stream.write_control(&forwarding::Ready { connected: true }).await?;
                            tcp_bridge(socket, stream).await?;
                            // The consumer retires its connection only after
                            // reading the full response and stream FIN. A
                            // provider close here could discard its queued
                            // final bytes, even after SendStream::finish().
                            // Keep the FlowOwner alive until that peer close;
                            // policy/host cancellation still interrupts this
                            // wait and an absent peer has the usual idle bound.
                            tokio::time::timeout(IDLE, connection.closed()).await?;
                            Ok(())
                        },
                        Protocol::Udp => {
                            let socket = UdpSocket::bind(wildcard(open.target)).await?;
                            socket.connect(open.target).await?;
                            stream.write_control(&forwarding::Ready { connected: true }).await?;
                            udp_bridge(socket, stream, &owner).await
                        }
                    }
                } => result,
            }
        }.await;
        connection.close(0u32.into(), b"forward provider ended");
        result
    }
}

fn wildcard(target: SocketAddr) -> SocketAddr {
    SocketAddr::new(
        if target.is_ipv4() {
            "0.0.0.0".parse().unwrap()
        } else {
            "::".parse().unwrap()
        },
        0,
    )
}

async fn tcp_bridge(socket: TcpStream, stream: DataStream) -> Result<(), ClientError> {
    let (mut local_read, mut local_write) = socket.into_split();
    let (mut remote_write, mut remote_read) = stream.split();
    let upload = async {
        let mut buffer = vec![0u8; crate::data_stream::CHUNK_BYTES];
        loop {
            let count = tokio::time::timeout(IDLE, local_read.read(&mut buffer)).await??;
            if count == 0 {
                remote_write.finish()?;
                return Ok::<_, ClientError>(());
            }
            remote_write.write_chunk(&buffer[..count]).await?;
        }
    };
    let download = async {
        loop {
            let bytes = match remote_read.read_chunk_or_end().await? {
                Some(bytes) => bytes,
                None => {
                    local_write.shutdown().await?;
                    return Ok::<_, ClientError>(());
                }
            };
            tokio::time::timeout(IDLE, local_write.write_all(&bytes)).await??;
        }
    };
    tokio::try_join!(upload, download)?;
    Ok(())
}
/// None means one unsupported host datagram was consumed. Other receive errors
/// remain errors; they must not be silently converted into a packet drop.
async fn receive_udp_datagram(
    socket: &UdpSocket,
    bytes: &mut [u8],
) -> std::io::Result<Option<(usize, SocketAddr)>> {
    match socket.recv_from(bytes).await {
        Ok((count, _)) if count > forwarding::MAX_DATAGRAM => Ok(None),
        Ok(value) => Ok(Some(value)),
        // Windows consumes the oversized message and returns WSAEMSGSIZE.
        // This also protects the listener if an OS supports a larger payload
        // than the fixed full ordinary-UDP buffer above.
        Err(error) if error.raw_os_error() == Some(10040) => Ok(None),
        Err(error) => Err(error),
    }
}

async fn udp_bridge(socket: UdpSocket, stream: DataStream, owner: &FlowOwner) -> Result<(), ClientError> {
    let (mut write, mut read) = stream.split();
    let upload = async {
        loop {
            let bytes = read.read_chunk().await?;
            if bytes.len() > forwarding::MAX_DATAGRAM {
                return Err("UDP datagram too large".into());
            }
            tokio::time::timeout(IDLE, socket.send(&bytes)).await??;
        }
        #[allow(unreachable_code)]
        Ok::<_, ClientError>(())
    };
    let download = async {
        let mut bytes = vec![0u8; UDP_RECEIVE_BYTES];
        loop {
            let Some((count, _)) = tokio::time::timeout(IDLE, receive_udp_datagram(&socket, &mut bytes)).await?? else {
                owner.datagram_dropped();
                continue;
            };
            write.write_chunk(&bytes[..count]).await?;
        }
        #[allow(unreachable_code)]
        Ok::<_, ClientError>(())
    };
    tokio::try_join!(upload, download)?;
    Ok(())
}

async fn udp_consumer(
    socket: Arc<UdpSocket>,
    source: SocketAddr,
    first: Vec<u8>,
    mut pending: tokio::sync::mpsc::Receiver<Vec<u8>>,
    stream: DataStream,
) -> Result<(), ClientError> {
    let (mut write, mut read) = stream.split();
    let upload = async {
        write.write_chunk(&first).await?;
        while let Some(bytes) = tokio::time::timeout(IDLE, pending.recv()).await? {
            write.write_chunk(&bytes).await?;
        }
        Ok::<_, ClientError>(())
    };
    let download = async {
        loop {
            let bytes = read.read_chunk().await?;
            if bytes.len() > forwarding::MAX_DATAGRAM {
                return Err("UDP datagram too large".into());
            }
            tokio::time::timeout(IDLE, socket.send_to(&bytes, source)).await??;
        }
        #[allow(unreachable_code)]
        Ok::<_, ClientError>(())
    };
    tokio::try_join!(upload, download)?;
    Ok(())
}

impl NativeClient {
    /// One bounded request/response, using the same provider and wire as the
    /// browser operation. A local application listener is optional for SDK use.
    pub async fn forward_exchange(
        &self,
        address: EndpointAddr,
        protocol: Protocol,
        target: SocketAddr,
        payload: &[u8],
        response_bytes: usize,
    ) -> Result<Vec<u8>, ClientError> {
        self.require_ready()?;
        if payload.len() > forwarding::MAX_EXCHANGE || response_bytes > forwarding::MAX_EXCHANGE {
            return Err("forward exchange too large".into());
        }
        let peer = VerifyingKey::from_bytes(address.id.as_bytes())?;
        let owner =
            self.forwarding
                .claim(peer, protocol, target, false, CancellationToken::new())?;
        tokio::select! {
            biased;
            _ = owner.cancel.cancelled() => Err("forward stopped".into()),
            result = async {
                let connection = self.resources.connect(address, protocol.alpn()).await?;
                self.forwarding.attach(&owner, connection.clone())?;
                let stream = forwarding::open(&connection, target).await?;
                Ok(forwarding::exchange(stream, protocol, payload, response_bytes).await?)
            } => result,
        }
    }
    /// Cancel a concrete flow observed through forwarding_status and await its
    /// owner, including a transport setup which has actually returned Pending.
    pub async fn cancel_forwarding_flow(&self, id: &str) -> Result<(), ClientError> {
        let id: u64 = id.parse()?;
        let (cancel, connection) = {
            let state = self.forwarding.state.lock().unwrap();
            let flow = state.flows.get(&id).ok_or("unknown forward flow")?;
            (flow.cancel.clone(), flow.connection.clone())
        };
        cancel.cancel();
        if let Some(connection) = connection {
            connection.close(0u32.into(), b"forward flow cancelled");
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let done = self.forwarding.joined.notified();
                if !self
                    .forwarding
                    .state
                    .lock()
                    .unwrap()
                    .flows
                    .contains_key(&id)
                {
                    break;
                }
                done.await;
            }
        })
        .await?;
        Ok(())
    }
    pub(crate) fn forwarding_enabled(&self, protocol: Protocol) -> bool {
        self.forwarding
            .state
            .lock()
            .unwrap()
            .scopes
            .contains_key(&protocol)
    }
    pub async fn enable_forwarding(
        &self,
        protocol: Protocol,
        targets: Vec<SocketAddr>,
    ) -> Result<Value, ClientError> {
        self.require_ready()?;
        if targets.is_empty()
            || targets.len() > 32
            || targets
                .iter()
                .any(|a| a.port() == 0 || a.ip().is_unspecified() || a.ip().is_multicast())
        {
            return Err("invalid forwarding targets".into());
        }
        let _setup = self.forwarding.setup.lock().await;
        let node = self.node.lock().await;
        let mut services = self.services.lock().await;
        let access = services.access(&node).await?;
        if !access.member(access.local) {
            return Err("forward provider requires membership".into());
        }
        if let Some(change) = access.declaration_change(protocol.service(), true, true) {
            services.apply(&node, &self.files, change).await?;
        }
        self.forwarding
            .state
            .lock()
            .unwrap()
            .scopes
            .insert(protocol, targets.into_iter().collect());
        self.forwarding
            .enforce_access(&services.access(&node).await?);
        self.forwarding.join_retired(protocol).await?;
        Ok(self.forwarding.status())
    }
    pub async fn disable_forwarding(&self, protocol: Protocol) -> Result<Value, ClientError> {
        let _setup = self.forwarding.setup.lock().await;
        self.forwarding
            .state
            .lock()
            .unwrap()
            .scopes
            .remove(&protocol);
        let node = self.node.lock().await;
        let mut services = self.services.lock().await;
        let access = services.access(&node).await?;
        self.forwarding.enforce_access(&access);
        if let Some(change) = access.declaration_change(protocol.service(), true, false) {
            services.apply(&node, &self.files, change).await?;
        }
        self.forwarding.join_retired(protocol).await?;
        Ok(self.forwarding.status())
    }
    pub fn forwarding_status(&self) -> Value {
        self.forwarding.status()
    }
    pub async fn stop_forwarding(&self, id: &str) -> Result<Value, ClientError> {
        let _setup = self.forwarding.setup.lock().await;
        let listener = self
            .forwarding
            .state
            .lock()
            .unwrap()
            .listeners
            .remove(&id.parse()?)
            .ok_or("unknown forward")?;
        listener.cancel.cancel();
        listener.join.await?;
        Ok(self.forwarding.status())
    }
    pub async fn start_forwarding(
        &self,
        address: EndpointAddr,
        protocol: Protocol,
        target: SocketAddr,
        bind: SocketAddr,
    ) -> Result<Value, ClientError> {
        self.require_ready()?;
        if !bind.ip().is_loopback() {
            return Err("forward listener must use loopback".into());
        }
        let _setup = self.forwarding.setup.lock().await;
        let peer = VerifyingKey::from_bytes(address.id.as_bytes())?;
        {
            let state = self.forwarding.state.lock().unwrap();
            if state.closed
                || state.listeners.len() >= MAX_LISTENERS
                || !state
                    .access
                    .as_ref()
                    .is_some_and(|a| forwarding::peer_permitted(a, peer, protocol))
            {
                return Err("forward unavailable or denied".into());
            }
        }
        let cancel = CancellationToken::new();
        let socket = match protocol {
            Protocol::Tcp => BoundSocket::Tcp(TcpListener::bind(bind).await?),
            Protocol::Udp => BoundSocket::Udp(Arc::new(UdpSocket::bind(bind).await?)),
        };
        let listen = socket.local_addr()?;
        let resources = self.resources.clone();
        let runtime = self.forwarding.clone();
        let child_cancel = cancel.clone();
        let counters = Arc::new(Mutex::new(ListenerCounters::default()));
        let worker_counters = counters.clone();
        let join = tokio::spawn(async move {
            let mut tasks = JoinSet::new();
            let mut udp_peers = BTreeMap::<SocketAddr, tokio::sync::mpsc::Sender<Vec<u8>>>::new();
            loop {
                tokio::select! {
                    biased;
                    _ = child_cancel.cancelled() => break,
                    result = tasks.join_next(), if !tasks.is_empty() => { if let Some(Ok(Some(peer))) = result { udp_peers.remove(&peer); } },
                    result = socket.accept() => {
                        let Ok(input) = result else { break };
                        let mut pending = None;
                        let input = match input {
                            Input::DroppedDatagram => {
                                worker_counters.lock().unwrap().dropped_datagrams += 1;
                                continue;
                            },
                            Input::Udp(socket, source, payload) => {
                                if let Some(sender) = udp_peers.get(&source) {
                                    if sender.try_send(payload).is_err() { worker_counters.lock().unwrap().dropped_datagrams += 1; }
                                    continue;
                                }
                                if tasks.len() >= MAX_FLOWS { worker_counters.lock().unwrap().dropped_datagrams += 1; continue; }
                                let (sender, receiver) = tokio::sync::mpsc::channel(16);
                                udp_peers.insert(source, sender);
                                pending = Some(receiver);
                                Input::Udp(socket, source, payload)
                            },
                            Input::Tcp(socket) => {
                                if tasks.len() >= MAX_FLOWS { worker_counters.lock().unwrap().failed += 1; continue; }
                                Input::Tcp(socket)
                            },
                        };
                        worker_counters.lock().unwrap().accepted += 1;
                        let runtime = runtime.clone(); let resources = resources.clone(); let address = address.clone(); let cancel = child_cancel.child_token();
                        let counters = worker_counters.clone();
                        tasks.spawn(async move {
                            let source = match &input { Input::Udp(_, source, _) => Some(*source), _ => None };
                            let result = async {
                                let owner = runtime.claim(peer, protocol, target, false, cancel.clone())?;
                                tokio::select! {
                                    biased;
                                    _ = cancel.cancelled() => Err("forward stopped".into()),
                                    result = async {
                                        let connection = resources.connect(address, protocol.alpn()).await?;
                                        runtime.attach(&owner, connection.clone())?;
                                        let stream = forwarding::open(&connection, target).await?;
                                        match input {
                                            Input::Tcp(socket) => tcp_bridge(socket, stream).await,
                                            Input::DroppedDatagram => Err("oversized datagram reached a flow".into()),
                                            Input::Udp(socket, source, payload) => {
                                                udp_consumer(socket, source, payload, pending.ok_or("missing datagram queue")?, stream).await
                                            }
                                        }
                                    } => result,
                                }
                            }.await;
                            let mut counters = counters.lock().unwrap();
                            if result.is_err() { counters.failed += 1; counters.last_error = Some("forward_flow_ended"); }
                            else { counters.completed += 1; }
                            source
                        });
                    }
                }
            }
            child_cancel.cancel();
            while tasks.join_next().await.is_some() {}
        });
        let mut state = self.forwarding.state.lock().unwrap();
        state.next = state
            .next
            .checked_add(1)
            .ok_or("forward identifier exhausted")?;
        let id = state.next;
        state.listeners.insert(
            id,
            Listener {
                peer,
                protocol,
                target,
                listen,
                cancel,
                join,
                counters,
            },
        );
        Ok(json!({"id":id.to_string(),"listen":listen,"protocol":protocol,"target":target}))
    }
}
enum BoundSocket {
    Tcp(TcpListener),
    Udp(Arc<UdpSocket>),
}
enum Input {
    Tcp(TcpStream),
    Udp(Arc<UdpSocket>, SocketAddr, Vec<u8>),
    DroppedDatagram,
}
impl BoundSocket {
    fn local_addr(&self) -> std::io::Result<SocketAddr> {
        match self {
            Self::Tcp(s) => s.local_addr(),
            Self::Udp(s) => s.local_addr(),
        }
    }
    async fn accept(&self) -> std::io::Result<Input> {
        match self {
            Self::Tcp(s) => Ok(Input::Tcp(s.accept().await?.0)),
            Self::Udp(s) => {
                let mut payload = vec![0u8; UDP_RECEIVE_BYTES];
                let Some((count, peer)) = receive_udp_datagram(s, &mut payload).await? else {
                    return Ok(Input::DroppedDatagram);
                };
                payload.truncate(count);
                Ok(Input::Udp(s.clone(), peer, payload))
            }
        }
    }
}
