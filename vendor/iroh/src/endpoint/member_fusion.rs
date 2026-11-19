//! Connection ownership for the native, ordinary member ALPN.

use std::{
    collections::HashMap,
    io,
    sync::{Arc, Mutex, Weak},
};

use tokio::{
    sync::{mpsc, watch},
    time::{self, Duration, Instant},
};
use tokio_util::sync::CancellationToken;

use super::{
    BindError, Builder, ConnectError, ConnectOptions, ConnectingError, Connection, ConnectionError,
    ControlConfirmation, ControlConnection, ControlMessageFormat, EndpointAddr, EndpointId,
    ExportKeyingMaterialError, PathList, RecvStream, RelayResourceUse, RestrictedBuilder,
    RestrictedEndpoint, SendStream, Side, TransportAddr,
    restricted::{AdmissionOutcome, BoxFuture, DynEndpointAdmission},
};

/// Application authorization for the native member ALPN.
pub trait MemberFusionAdmission: std::fmt::Debug + Send + Sync {
    /// Authenticate the peer as a member of this network. The confirmation
    /// checks local readiness after the endpoint proves the data path.
    fn authorize<'a>(
        &'a self,
        connection: &'a ControlConnection,
    ) -> impl Future<Output = io::Result<ControlConfirmation>> + Send + 'a;

    /// Check current authorization again before handing off the selected connection.
    fn is_current(&self, connection: &ControlConnection) -> bool;

    /// Register the selected connection with the application's revocation policy.
    fn business_released(&self, _connection: &Connection) {}

    /// Observe messages of the existing bilateral admission stage.
    fn selection_event(&self, _connection: &ControlConnection, _event: MemberFusionEvent) {}
}

/// Admission-stage messages on one TLS-authenticated candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberFusionEvent {
    /// The larger-ID peer sent readiness on its locally qualified candidate.
    ReadySent,
    /// The smaller-ID peer received readiness on its locally qualified candidate.
    ReadyReceived,
    /// The smaller-ID peer committed this connection for the pair.
    Committed,
    /// The larger-ID peer received the committed connection.
    CommitReceived,
    /// The candidate was superseded by the selected connection.
    Superseded,
}

#[derive(Debug)]
struct Admission<A> {
    inner: A,
    fusion: Arc<Fusion>,
}

impl<A: MemberFusionAdmission> DynEndpointAdmission for Admission<A> {
    fn admit<'a>(
        &'a self,
        connection: &'a ControlConnection,
    ) -> BoxFuture<'a, io::Result<AdmissionOutcome>> {
        Box::pin(async move {
            let confirmation = self.inner.authorize(connection).await?;
            let path = connection
                .paths()
                .iter()
                .filter(|path| match path.remote_addr() {
                    TransportAddr::Relay(url) => connection
                        .endpoint
                        .inner
                        .static_config
                        .relay_resource_policy
                        .allows(url, RelayResourceUse::DataRelay),
                    _ => false,
                })
                .max_by_key(|path| path.is_selected())
                .map(|path| path.id())
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotConnected, "no permitted data relay")
                })?;
            let permit = connection.qualify_data_path(path, Vec::new()).await?;
            Ok(AdmissionOutcome::AcceptWithConfirmation {
                permit,
                confirmation,
            })
        })
    }

    fn is_current(&self, connection: &ControlConnection) -> bool {
        self.inner.is_current(connection)
    }

    fn business_released(&self, connection: &Connection) {
        self.inner.business_released(connection);
    }

    fn application_protocol(&self) -> bool {
        true
    }

    fn member_fusion(&self) -> Option<&Fusion> {
        Some(&self.fusion)
    }

    fn fusion_event(&self, connection: &ControlConnection, event: MemberFusionEvent) {
        self.inner.selection_event(connection, event);
    }
}

/// Configure the node owner for a single ordinary member ALPN.
#[derive(Debug)]
pub struct MemberFusionBuilder {
    inner: RestrictedBuilder,
    owner: Arc<Fusion>,
    alpn: Vec<u8>,
    setup: Duration,
}

impl MemberFusionBuilder {
    pub(super) fn new(
        mut builder: Builder,
        alpn: &[u8],
        admission: impl MemberFusionAdmission + 'static,
    ) -> Self {
        let owner = Arc::new(Fusion::default());
        builder.admission = Some(Arc::new(Admission {
            inner: admission,
            fusion: owner.clone(),
        }));
        Self {
            inner: RestrictedBuilder::new(builder.alpns(vec![alpn.to_vec()])),
            owner,
            alpn: alpn.to_vec(),
            setup: Duration::from_secs(60),
        }
    }

    /// Maximum original admission budget for an inbound attempt or shared dial.
    pub fn admission_timeout(mut self, timeout: Duration) -> Self {
        self.inner = self.inner.admission_timeout(timeout);
        self.setup = timeout;
        self
    }

    /// Bind the member endpoint with its local idle maintenance allowance.
    pub async fn bind(self, idle_timeout: Duration) -> Result<MemberFusionEndpoint, BindError> {
        self.owner
            .idle_timeout
            .set(idle_timeout)
            .expect("new owner");
        let endpoint = self.inner.bind().await?;
        let (incoming_tx, incoming) = mpsc::unbounded_channel();
        tokio::spawn(accept_connections(
            endpoint.clone(),
            self.owner.clone(),
            incoming_tx,
        ));
        Ok(MemberFusionEndpoint {
            inner: Arc::new(EndpointOwner {
                endpoint,
                fusion: self.owner,
                alpn: self.alpn,
                setup: self.setup,
                incoming: tokio::sync::Mutex::new(incoming),
            }),
        })
    }
}

/// The node-owned first connection and idle lifetime for ordinary member traffic.
#[derive(Debug, Clone)]
pub struct MemberFusionEndpoint {
    inner: Arc<EndpointOwner>,
}

#[derive(Debug)]
struct EndpointOwner {
    endpoint: RestrictedEndpoint,
    fusion: Arc<Fusion>,
    alpn: Vec<u8>,
    setup: Duration,
    incoming: tokio::sync::Mutex<mpsc::UnboundedReceiver<IncomingResult>>,
}

impl Drop for EndpointOwner {
    fn drop(&mut self) {
        self.fusion.shutdown.cancel();
    }
}

impl MemberFusionEndpoint {
    /// TLS identity of this member endpoint.
    pub fn id(&self) -> EndpointId {
        self.inner.endpoint.id()
    }

    /// Wait for this endpoint's contact transport to become online.
    pub async fn online(&self) {
        self.inner.endpoint.online().await;
    }

    /// Join the first dial or use the already selected connection.
    pub async fn connect(&self, target: EndpointAddr) -> io::Result<MemberConnection> {
        self.connect_until(
            target,
            Instant::now() + self.inner.setup,
            &CancellationToken::new(),
        )
        .await
    }

    /// Bound only this caller's wait. The shared dial has its own setup budget
    /// while another caller is still waiting for it.
    pub async fn connect_until(
        &self,
        target: EndpointAddr,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> io::Result<MemberConnection> {
        tokio::select! { biased;
            _ = self.inner.fusion.shutdown.cancelled() => Err(io::ErrorKind::NotConnected.into()),
            _ = cancellation.cancelled() => Err(io::ErrorKind::Interrupted.into()),
            _ = time::sleep_until(deadline) => Err(io::ErrorKind::TimedOut.into()),
            result = self.shared_connect(target) => result,
        }
    }

    async fn shared_connect(&self, target: EndpointAddr) -> io::Result<MemberConnection> {
        let owner = &self.inner;
        let peer = owner.fusion.peer(target.id);
        let waiter = {
            let mut state = peer.state.lock().expect("member peer lock");
            if let Some(connection) = state.connection() {
                return Ok(connection);
            }
            if let Some(waiter) = state.dial.as_ref().and_then(Dial::join) {
                waiter
            } else {
                let dial = Arc::new(Dial::new(owner.fusion.shutdown.child_token()));
                let waiter = dial.join().expect("new member dial");
                state.dial = Some(dial.clone());
                let endpoint = owner.endpoint.clone();
                let alpn = owner.alpn.clone();
                let peer = peer.clone();
                tokio::spawn(async move {
                    let result = async {
                        let opts = ConnectOptions::default()
                            .with_attempt_cancellation(dial.cancel.clone());
                        let connecting = endpoint.connect_with_opts(target, &alpn, opts).await?;
                        Ok::<_, ConnectError>(connecting.await?)
                    }
                    .await;
                    let mut state = peer.state.lock().expect("member peer lock");
                    dial.state.lock().expect("member dial lock").result =
                        Some(result.map(drop).map_err(Arc::new));
                    if state
                        .dial
                        .as_ref()
                        .is_some_and(|current| Arc::ptr_eq(current, &dial))
                    {
                        state.dial = None;
                    }
                    peer.changed.send_modify(|_| {});
                });
                waiter
            }
        };
        let mut changed = peer.changed.subscribe();
        loop {
            {
                let state = peer.state.lock().expect("member peer lock");
                if let Some(connection) = state.connection() {
                    waiter.0.satisfy();
                    return Ok(connection);
                }
                if state.incoming == 0 {
                    match &waiter.0.state.lock().expect("member dial lock").result {
                        Some(Err(error)) => return Err(io::Error::other(error.clone())),
                        Some(Ok(())) => return Err(io::ErrorKind::NotConnected.into()),
                        None => {}
                    }
                }
            }
            let _ = changed.changed().await;
        }
    }

    /// Receive an admitted peer's selected incoming connection or its setup error.
    /// Outbound selection is returned directly to its waiting dialers.
    pub async fn accept(&self) -> Option<Result<MemberConnection, ConnectingError>> {
        let mut incoming = self.inner.incoming.lock().await;
        loop {
            let next = tokio::select! { biased;
                _ = self.inner.fusion.shutdown.cancelled() => return None,
                next = incoming.recv() => next?,
            };
            let live = match next {
                Ok(live) => live.upgrade(),
                Err(error) => return Some(Err(error)),
            };
            if let Some(live) = live
                && live.connection.close_reason().is_none()
            {
                return Some(Ok(MemberConnection(live)));
            }
        }
    }

    /// Close the node owner and all of its member connections.
    pub async fn close(&self) {
        self.inner.fusion.shutdown.cancel();
        self.inner.endpoint.close().await;
    }
}

// A pending incoming notification cannot extend the connection's idle lifetime.
type IncomingResult = Result<Weak<Live>, ConnectingError>;

async fn accept_connections(
    endpoint: RestrictedEndpoint,
    fusion: Arc<Fusion>,
    accepted: mpsc::UnboundedSender<IncomingResult>,
) {
    let mut attempts = tokio::task::JoinSet::new();
    loop {
        tokio::select! { biased;
            _ = fusion.shutdown.cancelled() => break,
            _ = attempts.join_next(), if !attempts.is_empty() => {},
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { break };
                let fusion = fusion.clone();
                let accepted = accepted.clone();
                attempts.spawn(async move {
                    match incoming.await {
                        Ok(connection) => {
                            let peer = fusion.peer(connection.remote_id());
                            let state = peer.state.lock().expect("member peer lock");
                            if let Some(selected) = state.connection()
                                && selected.stable_id() == connection.stable_id()
                            {
                                let _ = accepted.send(Ok(Arc::downgrade(&selected.0)));
                            }
                        }
                        Err(error) if was_superseded(&error) => {},
                        Err(error) => { let _ = accepted.send(Err(error)); }
                    }
                });
            }
        }
    }
    endpoint.close().await;
}

#[derive(Debug, Default)]
pub(super) struct Fusion {
    peers: Mutex<HashMap<EndpointId, Weak<Peer>>>,
    shutdown: CancellationToken,
    idle_timeout: std::sync::OnceLock<Duration>,
}

impl Fusion {
    fn peer(&self, id: EndpointId) -> Arc<Peer> {
        let mut peers = self.peers.lock().expect("member peers lock");
        if let Some(peer) = peers.get(&id).and_then(Weak::upgrade) {
            return peer;
        }
        peers.retain(|_, peer| peer.strong_count() != 0);
        let peer = Arc::new(Peer::default());
        peers.insert(id, Arc::downgrade(&peer));
        peer
    }

    pub(super) fn candidate(&self, connection: &Connection) -> Candidate {
        let peer = self.peer(connection.remote_id());
        let incoming = connection.side().is_server();
        if incoming {
            peer.state.lock().expect("member peer lock").incoming += 1;
        }
        Candidate {
            peer,
            incoming,
            shutdown: self.shutdown.clone(),
            idle_timeout: *self.idle_timeout.get().expect("bound member idle timeout"),
        }
    }
}

#[derive(Debug, Default)]
struct Peer {
    state: Mutex<PeerState>,
    changed: watch::Sender<()>,
}

#[derive(Debug, Default)]
struct PeerState {
    dial: Option<Arc<Dial>>,
    incoming: usize,
    selection: Option<usize>,
    live: Option<MemberConnection>,
}

impl PeerState {
    fn connection(&self) -> Option<MemberConnection> {
        self.live
            .as_ref()
            .filter(|live| live.close_reason().is_none())
            .cloned()
    }
}

#[derive(Debug)]
struct Dial {
    state: Mutex<DialState>,
    cancel: CancellationToken,
}

#[derive(Debug, Default)]
struct DialState {
    waiters: usize,
    satisfied: bool,
    result: Option<Result<(), Arc<ConnectError>>>,
}

impl Dial {
    fn new(cancel: CancellationToken) -> Self {
        Self {
            state: Mutex::default(),
            cancel,
        }
    }

    fn join(self: &Arc<Self>) -> Option<Waiter> {
        let mut state = self.state.lock().expect("member dial lock");
        if self.cancel.is_cancelled() {
            return None;
        }
        state.waiters += 1;
        Some(Waiter(self.clone()))
    }

    fn satisfy(&self) {
        self.state.lock().expect("member dial lock").satisfied = true;
    }
}

struct Waiter(Arc<Dial>);

impl Drop for Waiter {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().expect("member dial lock");
        state.waiters -= 1;
        if state.waiters == 0 && !state.satisfied && state.result.is_none() {
            self.0.cancel.cancel();
        }
    }
}

pub(super) struct Candidate {
    peer: Arc<Peer>,
    incoming: bool,
    shutdown: CancellationToken,
    idle_timeout: Duration,
}

impl Drop for Candidate {
    fn drop(&mut self) {
        if self.incoming {
            self.peer.state.lock().expect("member peer lock").incoming -= 1;
            self.peer.changed.send_modify(|_| {});
        }
    }
}

const ADMISSION_STAGE: ControlMessageFormat = ControlMessageFormat::Fixed { length: 1 };
const READY: &[u8] = b"\x01";
const COMMIT: &[u8] = b"\x02";
const SUPERSEDED: &[u8] = b"\x03";
pub(super) const SUPERSEDED_REASON: &[u8] = b"member connection superseded";

impl Candidate {
    // Substitute READY/COMMIT for the ordinary bilateral stage exchange. The
    // larger authenticated ID opens the stream regardless of QUIC dial role.
    // Only the smaller ID can commit, so two crossed dials cannot each win.
    pub(super) async fn select(
        &self,
        control: &ControlConnection,
        policy: &dyn DynEndpointAdmission,
    ) -> io::Result<Selection> {
        let event = |event| policy.fusion_event(control, event);
        if control.id().as_bytes() > control.remote_id().as_bytes() {
            let exchange = control.open_control_exchange().await?;
            exchange.send_message(ADMISSION_STAGE, READY).await?;
            exchange.finish_send().await?;
            event(MemberFusionEvent::ReadySent);
            return match exchange.recv_message(ADMISSION_STAGE).await?.as_slice() {
                COMMIT => {
                    event(MemberFusionEvent::CommitReceived);
                    self.reserve(control.stable_id()).ok_or_else(superseded)
                }
                SUPERSEDED => {
                    event(MemberFusionEvent::Superseded);
                    Err(superseded())
                }
                _ => Err(io::ErrorKind::InvalidData.into()),
            };
        }

        let exchange = control.accept_control_exchange().await?;
        if exchange.recv_message(ADMISSION_STAGE).await? != READY {
            return Err(io::ErrorKind::InvalidData.into());
        }
        event(MemberFusionEvent::ReadyReceived);

        if self.incoming {
            self.wait_for_preferred_dial(control).await;
        }
        if !policy.is_current(control) {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        if let Some(selection) = self.reserve(control.stable_id()) {
            exchange.send_message(ADMISSION_STAGE, COMMIT).await?;
            exchange.finish_send().await?;
            event(MemberFusionEvent::Committed);
            return Ok(selection);
        }
        exchange.send_message(ADMISSION_STAGE, SUPERSEDED).await?;
        event(MemberFusionEvent::Superseded);
        let _ = exchange.finish_send_confirmed().await;
        Err(superseded())
    }

    async fn wait_for_preferred_dial(&self, control: &ControlConnection) {
        // Only a real local dial can defer an eligible incoming candidate.
        // Leave time to send the selection on the fallback's original budget.
        let rtt = control
            .paths()
            .iter()
            .find(|path| path.is_selected())
            .map(|path| path.rtt())
            .unwrap_or(Duration::from_millis(100));
        let Some(until) = control
            .caller_deadline()
            .and_then(|end| end.checked_sub(rtt * 2))
        else {
            return;
        };
        let mut changed = self.peer.changed.subscribe();
        loop {
            {
                let state = self.peer.state.lock().expect("member peer lock");
                if state.connection().is_some()
                    || state
                        .dial
                        .as_ref()
                        .is_none_or(|dial| dial.cancel.is_cancelled())
                {
                    return;
                }
            }
            if time::timeout_at(until, changed.changed()).await.is_err() {
                return;
            }
        }
    }

    fn reserve(&self, id: usize) -> Option<Selection> {
        let mut state = self.peer.state.lock().expect("member peer lock");
        if state.selection.is_some() || state.connection().is_some() {
            return None;
        }
        state.selection = Some(id);
        Some(Selection {
            peer: self.peer.clone(),
            id,
            idle_timeout: self.idle_timeout,
            shutdown: self.shutdown.clone(),
        })
    }
}

fn superseded() -> io::Error {
    io::ErrorKind::AlreadyExists.into()
}

fn was_superseded(error: &ConnectingError) -> bool {
    match error {
        ConnectingError::AdmissionFailed {
            kind: io::ErrorKind::AlreadyExists,
            ..
        } => true,
        ConnectingError::ConnectionError {
            source: ConnectionError::ApplicationClosed(closed),
            ..
        } => closed.reason.as_ref() == SUPERSEDED_REASON,
        _ => false,
    }
}

pub(super) struct Selection {
    peer: Arc<Peer>,
    id: usize,
    idle_timeout: Duration,
    shutdown: CancellationToken,
}

impl Selection {
    pub(super) fn publish(self, connection: Connection) {
        let live = MemberConnection(Arc::new(Live {
            connection,
            activity: Arc::new(Activity::default()),
        }));
        self.peer.state.lock().expect("member peer lock").live = Some(live.clone());
        let peer = self.peer.clone();
        let idle_timeout = self.idle_timeout;
        let shutdown = self.shutdown.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = live.0.connection.closed() => {},
                _ = shutdown.cancelled() => live.0.connection.close(0u32.into(), b"member owner ended"),
                _ = live.0.idle(idle_timeout) => {},
            }
            let mut state = peer.state.lock().expect("member peer lock");
            if state
                .live
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(&current.0, &live.0))
            {
                state.live = None;
            }
        });
    }
}

impl Drop for Selection {
    fn drop(&mut self) {
        let mut state = self.peer.state.lock().expect("member peer lock");
        if state.selection == Some(self.id) {
            state.selection = None;
            self.peer.changed.send_modify(|_| {});
        }
    }
}

/// An admitted connection reference. Only live business streams hold activity.
#[derive(Debug, Clone)]
pub struct MemberConnection(Arc<Live>);

impl MemberConnection {
    /// Authenticated member at the other end.
    pub fn remote_id(&self) -> EndpointId {
        self.0.connection.remote_id()
    }

    /// This process's stable identifier for the original connection.
    pub fn stable_id(&self) -> usize {
        self.0.connection.stable_id()
    }

    /// TLS side of the selected original connection.
    pub fn side(&self) -> Side {
        self.0.connection.side()
    }

    /// Paths currently in use by the original connection.
    pub fn paths(&self) -> PathList<'_> {
        self.0.connection.paths()
    }

    /// Export material bound to the selected original TLS connection.
    pub fn export_keying_material(
        &self,
        output: &mut [u8],
        label: &[u8],
        context: &[u8],
    ) -> Result<(), ExportKeyingMaterialError> {
        self.0
            .connection
            .export_keying_material(output, label, context)
    }

    /// The terminal reason, if this connection has closed.
    pub fn close_reason(&self) -> Option<ConnectionError> {
        self.0.connection.close_reason()
    }

    /// Wait for the original connection to close.
    pub async fn closed(&self) -> ConnectionError {
        self.0.connection.closed().await
    }

    /// Open an independent bidirectional business stream with a local activity lease.
    pub async fn open_bi(&self) -> io::Result<MemberStream> {
        self.open(StreamKind::Business).await
    }

    /// Open a periodic maintenance stream on this already selected connection.
    /// Neither side counts it as business activity or extends its idle lifetime.
    pub async fn open_maintenance_bi(&self) -> io::Result<MemberStream> {
        self.open(StreamKind::Maintenance).await
    }

    async fn open(&self, kind: StreamKind) -> io::Result<MemberStream> {
        let lease = self.0.activity.lease(kind);
        let (mut send, recv) = self
            .0
            .connection
            .open_bi()
            .await
            .map_err(io::Error::other)?;
        send.write_all(&[kind as u8])
            .await
            .map_err(io::Error::other)?;
        Ok(MemberStream {
            send,
            recv,
            _lease: lease,
        })
    }

    /// Accept the next independent stream; its wire prefix identifies activity.
    pub async fn accept_bi(&self) -> io::Result<MemberStream> {
        let (send, mut recv) = self
            .0
            .connection
            .accept_bi()
            .await
            .map_err(io::Error::other)?;
        let mut header = [0];
        recv.read_exact(&mut header)
            .await
            .map_err(io::Error::other)?;
        let kind = match header[0] {
            0 => StreamKind::Business,
            1 => StreamKind::Maintenance,
            _ => return Err(io::ErrorKind::InvalidData.into()),
        };
        let lease = self.0.activity.lease(kind);
        Ok(MemberStream {
            send,
            recv,
            _lease: lease,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum StreamKind {
    Business,
    Maintenance,
}

/// The two halves of one member stream and its local activity lease, if any.
#[derive(Debug)]
pub struct MemberStream {
    send: SendStream,
    recv: RecvStream,
    _lease: Option<ActivityLease>,
}

impl MemberStream {
    /// Access the original QUIC stream halves for this service.
    pub fn streams(&mut self) -> (&mut SendStream, &mut RecvStream) {
        (&mut self.send, &mut self.recv)
    }
}

#[derive(Debug)]
struct Live {
    connection: Connection,
    activity: Arc<Activity>,
}

impl Live {
    async fn idle(&self, budget: Duration) {
        let mut changed = self.activity.changed.subscribe();
        loop {
            let idle_since = self
                .activity
                .state
                .lock()
                .expect("member activity lock")
                .idle_since;
            if let Some(since) = idle_since {
                if time::timeout_at(since + budget, changed.changed())
                    .await
                    .is_err()
                {
                    let state = self.activity.state.lock().expect("member activity lock");
                    if state.idle_since == Some(since) {
                        self.connection
                            .close(0u32.into(), b"member connection idle");
                        return;
                    }
                }
            } else {
                let _ = changed.changed().await;
            }
        }
    }
}

#[derive(Debug)]
struct Activity {
    state: Mutex<ActivityState>,
    changed: watch::Sender<()>,
}

#[derive(Debug)]
struct ActivityState {
    idle_since: Option<Instant>,
    holders: usize,
}

impl Default for Activity {
    fn default() -> Self {
        Self {
            state: Mutex::new(ActivityState {
                idle_since: Some(Instant::now()),
                holders: 0,
            }),
            changed: watch::Sender::default(),
        }
    }
}

impl Activity {
    fn lease(self: &Arc<Self>, kind: StreamKind) -> Option<ActivityLease> {
        if kind == StreamKind::Maintenance {
            return None;
        }
        let mut state = self.state.lock().expect("member activity lock");
        if state.holders == 0 {
            state.idle_since = None;
            self.changed.send_modify(|_| {});
        }
        state.holders += 1;
        Some(ActivityLease(self.clone()))
    }
}

#[derive(Debug)]
struct ActivityLease(Arc<Activity>);

impl Drop for ActivityLease {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().expect("member activity lock");
        state.holders -= 1;
        if state.holders == 0 {
            state.idle_since = Some(Instant::now());
            self.0.changed.send_modify(|_| {});
        }
    }
}
