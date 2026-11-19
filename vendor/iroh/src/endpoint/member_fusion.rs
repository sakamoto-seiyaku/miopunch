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
    ControlConfirmation, ControlConnection, ControlExchange, ControlMessageFormat, DataPathPermit,
    EndpointAddr, EndpointId, ExportKeyingMaterialError, PathList, RecvStream, RelayResourceUse,
    RestrictedBuilder, RestrictedEndpoint, SendStream, Side, TransportAddr, WeakControlConnection,
    restricted::{AdmissionOutcome, BoxFuture, DynEndpointAdmission},
};

mod dial;
pub use dial::MemberDialOptions;

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

    /// Decide which already reachable carrier is eligible for member data.
    fn permits_data_path(&self, remote: &TransportAddr) -> bool {
        matches!(remote, TransportAddr::Relay(_))
    }

    /// Custom data carriers the application registered for current members.
    fn custom_data_transports(&self) -> Vec<u64> {
        Vec::new()
    }

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
    /// The chooser requested the other endpoint's current qualification of a candidate.
    QualificationRequested,
    /// Both endpoints' qualification of the requested candidate was confirmed.
    QualificationConfirmed,
    /// The requested candidate was unavailable and the reverse candidate was reserved.
    QualificationUnavailable,
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
    alternate: Option<(Vec<u8>, Arc<dyn DynEndpointAdmission>)>,
    custom_data_limit: Option<Vec<u64>>,
}

impl<A: MemberFusionAdmission> DynEndpointAdmission for Admission<A> {
    fn alternate_protocol(&self, alpn: &[u8]) -> Option<&dyn DynEndpointAdmission> {
        let (member, fallback) = self.alternate.as_ref()?;
        (member != alpn).then_some(fallback.as_ref())
    }

    fn admit<'a>(
        &'a self,
        connection: &'a ControlConnection,
    ) -> BoxFuture<'a, io::Result<AdmissionOutcome>> {
        Box::pin(async move {
            let confirmation = self.inner.authorize(connection).await?;
            let mut custom = self.inner.custom_data_transports();
            if let Some(allowed) = &self.custom_data_limit {
                custom.retain(|id| allowed.contains(id));
            }
            let path = connection
                .paths()
                .iter()
                .filter(|path| {
                    self.inner.permits_data_path(path.remote_addr())
                        && match path.remote_addr() {
                            TransportAddr::Relay(url) => connection
                                .endpoint
                                .inner
                                .static_config
                                .relay_resource_policy
                                .allows(url, RelayResourceUse::DataRelay),
                            TransportAddr::Ip(_) => true,
                            TransportAddr::Custom(addr) => custom.contains(&addr.id()),
                            _ => false,
                        }
                })
                .max_by_key(|path| path.is_selected())
                .map(|path| path.id())
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotConnected, "no permitted member data path")
                })?;
            let permit = connection.qualify_data_path(path, custom).await?;
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
        let custom_data_limit = builder.restricted_data_custom_transports.take();
        builder.admission = Some(Arc::new(Admission {
            inner: admission,
            fusion: owner.clone(),
            alternate: None,
            custom_data_limit,
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

pub(super) struct SharedOwner {
    fusion: Arc<Fusion>,
    incoming: mpsc::UnboundedReceiver<IncomingResult>,
}

pub(super) fn shared_policy(
    alpn: &[u8],
    admission: impl MemberFusionAdmission + 'static,
    fallback: Arc<dyn DynEndpointAdmission>,
    custom_data_limit: Option<Vec<u64>>,
    idle_timeout: Duration,
) -> (Arc<dyn DynEndpointAdmission>, SharedOwner) {
    let (selected, incoming) = mpsc::unbounded_channel();
    let fusion = Arc::new(Fusion {
        selected: Some(selected),
        idle_timeout: idle_timeout.into(),
        ..Default::default()
    });
    let policy = Admission {
        inner: admission,
        fusion: fusion.clone(),
        alternate: Some((alpn.to_vec(), fallback)),
        custom_data_limit,
    };
    (Arc::new(policy), SharedOwner { fusion, incoming })
}

impl SharedOwner {
    pub(super) fn attach(
        self,
        endpoint: RestrictedEndpoint,
        alpn: &[u8],
        setup: Duration,
    ) -> MemberFusionEndpoint {
        MemberFusionEndpoint {
            inner: Arc::new(EndpointOwner {
                endpoint,
                fusion: self.fusion,
                alpn: alpn.to_vec(),
                setup,
                incoming: tokio::sync::Mutex::new(self.incoming),
            }),
        }
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

    /// Synchronize two real candidates after they qualify at this test endpoint.
    #[cfg(feature = "miopunch-test-hooks")]
    #[doc(hidden)]
    pub fn test_coordinate_qualified_candidates(&self) {
        self.inner
            .fusion
            .qualification_gate
            .set(Arc::new(QualificationGate::default()))
            .expect("candidate coordination already enabled");
    }

    /// Expose the test barriers for preferred qualification, release of READY,
    /// and the reverse candidate's first response, in that order.
    #[cfg(feature = "miopunch-test-hooks")]
    #[doc(hidden)]
    pub fn test_coordinate_stale_readiness(
        &self,
    ) -> (
        Arc<tokio::sync::Notify>,
        Arc<tokio::sync::Notify>,
        Arc<tokio::sync::Notify>,
    ) {
        let stale = StaleReadiness::default();
        let signals = (
            stale.preferred_qualified.clone(),
            stale.release_ready.clone(),
            stale.reverse_response.clone(),
        );
        let gate = QualificationGate {
            stale: Some(stale),
            ..Default::default()
        };
        self.inner
            .fusion
            .qualification_gate
            .set(Arc::new(gate))
            .expect("candidate coordination already enabled");
        signals
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
        self.connect_inner(target, None, deadline, cancellation)
            .await
    }

    /// Establish one shared connection using the supplied current contact
    /// inputs. Only the caller that starts the shared attempt supplies routes
    /// and its absolute deadline. Later waiters keep their own wait limits.
    pub async fn connect_with_routes_until(
        &self,
        target: EndpointAddr,
        routes: MemberDialOptions,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> io::Result<MemberConnection> {
        self.connect_inner(target, Some(routes), deadline, cancellation)
            .await
    }

    async fn connect_inner(
        &self,
        target: EndpointAddr,
        routes: Option<MemberDialOptions>,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> io::Result<MemberConnection> {
        let shared_deadline = if routes.is_some() {
            deadline.min(Instant::now() + self.inner.setup)
        } else {
            Instant::now() + self.inner.setup
        };
        tokio::select! { biased;
            _ = self.inner.fusion.shutdown.cancelled() => Err(io::ErrorKind::NotConnected.into()),
            _ = cancellation.cancelled() => Err(io::ErrorKind::Interrupted.into()),
            _ = time::sleep_until(deadline) => Err(io::ErrorKind::TimedOut.into()),
            result = self.shared_connect(target, routes, shared_deadline) => result,
        }
    }

    /// Reuse a selected connection or await an existing attempt without dialing.
    pub async fn reuse_until(
        &self,
        id: EndpointId,
        deadline: Instant,
    ) -> io::Result<Option<MemberConnection>> {
        let peer = self
            .inner
            .fusion
            .peers
            .lock()
            .expect("member peers lock")
            .get(&id)
            .and_then(Weak::upgrade);
        let Some(peer) = peer else { return Ok(None) };
        let waiter = peer
            .state
            .lock()
            .expect("member peer lock")
            .dial
            .as_ref()
            .and_then(Dial::join);
        tokio::select! { biased;
            _ = self.inner.fusion.shutdown.cancelled() => Err(io::ErrorKind::NotConnected.into()),
            _ = time::sleep_until(deadline) => Err(io::ErrorKind::TimedOut.into()),
            result = await_selected(peer, waiter) => result,
        }
    }

    async fn shared_connect(
        &self,
        target: EndpointAddr,
        routes: Option<MemberDialOptions>,
        deadline: Instant,
    ) -> io::Result<MemberConnection> {
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
                let dial = Arc::new(Dial::new(
                    owner.fusion.shutdown.child_token(),
                    routes.is_some(),
                ));
                let waiter = dial.join().expect("new member dial");
                state.dial = Some(dial.clone());
                let endpoint = owner.endpoint.clone();
                let alpn = owner.alpn.clone();
                let peer = peer.clone();
                tokio::spawn(async move {
                    let work = dial::Dial {
                        endpoint: &endpoint,
                        alpn: &alpn,
                        deadline,
                        cancel: &dial.cancel,
                    };
                    let result = tokio::select! { biased;
                        _ = dial.cancel.cancelled() => Err(io::ErrorKind::Interrupted.into()),
                        _ = time::sleep_until(deadline) => Err(io::ErrorKind::TimedOut.into()),
                        result = work.run(target, routes) => result,
                    };
                    let mut state = peer.state.lock().expect("member peer lock");
                    dial.state.lock().expect("member dial lock").result =
                        Some(result.map_err(Arc::new));
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
        await_selected(peer, Some(waiter))
            .await?
            .ok_or_else(|| io::ErrorKind::NotConnected.into())
    }

    /// Receive an admitted peer's selected connection. A standalone endpoint
    /// reports incoming selection and setup errors. A shared endpoint reports
    /// successful selection in either direction exactly once.
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

async fn await_selected(
    peer: Arc<Peer>,
    waiter: Option<Waiter>,
) -> io::Result<Option<MemberConnection>> {
    let mut changed = peer.changed.subscribe();
    loop {
        {
            let state = peer.state.lock().expect("member peer lock");
            if let Some(connection) = state.connection() {
                if let Some(waiter) = &waiter {
                    waiter.0.satisfy();
                }
                return Ok(Some(connection));
            }
            let dial = waiter
                .as_ref()
                .map(|waiter| waiter.0.state.lock().expect("member dial lock"));
            match dial.as_ref().and_then(|dial| dial.result.as_ref()) {
                Some(Err(error))
                    if state.incoming == 0 || error.kind() == io::ErrorKind::PermissionDenied =>
                {
                    return Err(io::Error::new(error.kind(), error.clone()));
                }
                Some(Ok(())) if state.incoming == 0 => {
                    return Err(io::ErrorKind::NotConnected.into());
                }
                None if state.incoming == 0 && waiter.is_none() => return Ok(None),
                _ => {}
            }
        }
        let _ = changed.changed().await;
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
    selected: Option<mpsc::UnboundedSender<IncomingResult>>,
    #[cfg(feature = "miopunch-test-hooks")]
    qualification_gate: std::sync::OnceLock<Arc<QualificationGate>>,
}

impl Fusion {
    pub(super) fn publish(
        &self,
        selection: Selection,
        connection: Connection,
        data_path: TransportAddr,
    ) {
        let live = selection.publish(connection, data_path);
        if let Some(selected) = &self.selected {
            let _ = selected.send(Ok(live));
        }
    }
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
            qualified: None,
            shutdown: self.shutdown.clone(),
            idle_timeout: *self.idle_timeout.get().expect("bound member idle timeout"),
            #[cfg(feature = "miopunch-test-hooks")]
            qualification_gate: self.qualification_gate.get().cloned(),
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
    preferred: Vec<Qualification>,
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

    fn preferred_binding(&self) -> Option<CandidateBinding> {
        self.preferred
            .iter()
            .find(|candidate| candidate.is_current())
            .map(|candidate| candidate.binding)
    }

    fn preferred_at(&self, binding: CandidateBinding) -> Option<&Qualification> {
        self.preferred
            .iter()
            .find(|candidate| candidate.binding == binding && candidate.is_current())
    }
}

#[derive(Debug)]
struct Dial {
    state: Mutex<DialState>,
    cancel: CancellationToken,
    race_routes: bool,
}

#[derive(Debug, Default)]
struct DialState {
    waiters: usize,
    satisfied: bool,
    result: Option<Result<(), Arc<io::Error>>>,
}

impl Dial {
    fn new(cancel: CancellationToken, race_routes: bool) -> Self {
        Self {
            state: Mutex::default(),
            cancel,
            race_routes,
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
    qualified: Option<CandidateBinding>,
    shutdown: CancellationToken,
    idle_timeout: Duration,
    #[cfg(feature = "miopunch-test-hooks")]
    qualification_gate: Option<Arc<QualificationGate>>,
}

impl Drop for Candidate {
    fn drop(&mut self) {
        if self.incoming || self.qualified.is_some() {
            let mut state = self.peer.state.lock().expect("member peer lock");
            if self.incoming {
                state.incoming -= 1;
            }
            state
                .preferred
                .retain(|preferred| Some(preferred.binding) != self.qualified);
            self.peer.changed.send_modify(|_| {});
        }
    }
}

const BINDING_SIZE: usize = 16;
type CandidateBinding = [u8; BINDING_SIZE];
const STAGE: ControlMessageFormat = ControlMessageFormat::LengthPrefixed16 {
    maximum: BINDING_SIZE,
};
const QUALIFIED: &[u8] = b"\x01";
const UNAVAILABLE: &[u8] = b"\x00";
const COMMIT: &[u8] = b"\x02";
const SUPERSEDED: &[u8] = b"\x03";
pub(super) const SUPERSEDED_REASON: &[u8] = b"member connection superseded";
pub(super) const MEMBER_ADMISSION_DENIED: &[u8] = b"member fusion admission denied";

#[derive(Debug)]
struct Qualification {
    binding: CandidateBinding,
    connection: WeakControlConnection,
    deadline: Instant,
    cancellation: Option<CancellationToken>,
}

enum ReverseChoice {
    Reserve(Selection),
    Prefer(CandidateBinding),
    Confirm(CandidateBinding, Selection),
}

impl Qualification {
    fn is_current(&self) -> bool {
        Instant::now() < self.deadline
            && self.connection.is_open()
            && self
                .cancellation
                .as_ref()
                .is_none_or(|token| !token.is_cancelled())
    }
}

#[cfg(feature = "miopunch-test-hooks")]
#[derive(Debug)]
struct QualificationGate {
    both: tokio::sync::Barrier,
    reverse_ready: tokio::sync::Notify,
    stale: Option<StaleReadiness>,
}

#[cfg(feature = "miopunch-test-hooks")]
impl Default for QualificationGate {
    fn default() -> Self {
        Self {
            both: tokio::sync::Barrier::new(2),
            reverse_ready: tokio::sync::Notify::new(),
            stale: None,
        }
    }
}

#[cfg(feature = "miopunch-test-hooks")]
#[derive(Debug, Default)]
struct StaleReadiness {
    preferred_qualified: Arc<tokio::sync::Notify>,
    release_ready: Arc<tokio::sync::Notify>,
    reverse_response: Arc<tokio::sync::Notify>,
}

impl Candidate {
    // Substitute READY/COMMIT for the ordinary bilateral stage exchange. The
    // larger authenticated ID opens the stream regardless of QUIC dial role.
    // Only the smaller ID can commit, so two crossed dials cannot each win.
    pub(super) async fn select(
        &mut self,
        control: &ControlConnection,
        policy: &dyn DynEndpointAdmission,
        permit: &DataPathPermit,
    ) -> io::Result<Selection> {
        let local_is_larger = control.id().as_bytes() > control.remote_id().as_bytes();
        let preferred = self.incoming == local_is_larger;
        #[cfg(feature = "miopunch-test-hooks")]
        let qualification_gate = self.qualification_gate.clone();
        #[cfg(feature = "miopunch-test-hooks")]
        let stale = qualification_gate
            .as_ref()
            .and_then(|gate| gate.stale.as_ref().map(|stale| (gate, stale)));
        #[cfg(feature = "miopunch-test-hooks")]
        if let Some((gate, _)) = stale
            && local_is_larger
            && preferred
        {
            gate.reverse_ready.notified().await;
        }
        // The admission path calls select only after authorization and data-path
        // qualification. Before this point a dial cannot participate in selection.
        if preferred {
            self.qualify_preferred(control)?;
        }
        #[cfg(feature = "miopunch-test-hooks")]
        if let Some(gate) = &self.qualification_gate {
            if let Some(stale) = &gate.stale {
                if preferred {
                    stale.preferred_qualified.notify_one();
                }
                if preferred == local_is_larger {
                    stale.release_ready.notified().await;
                }
            } else {
                gate.both.wait().await;
                if !local_is_larger && preferred {
                    gate.reverse_ready.notified().await;
                }
            }
        }

        let event = |event| policy.fusion_event(control, event);
        if local_is_larger {
            let exchange = control.open_control_exchange().await?;
            // A reverse READY may name the exact other connection that is
            // already qualified here. This is evidence carried by the existing
            // stage message, even if its own READY is still in transit.
            let alternate = if preferred {
                None
            } else {
                self.preferred_binding()
            };
            let ready = alternate
                .as_ref()
                .map_or(&[][..], |binding| binding.as_slice());
            exchange.send_message(STAGE, ready).await?;
            event(MemberFusionEvent::ReadySent);
            #[cfg(feature = "miopunch-test-hooks")]
            if let Some((gate, _)) = stale
                && !preferred
            {
                gate.reverse_ready.notify_one();
            }
            return self
                .finish_larger(control.stable_id(), &exchange, &event, preferred)
                .await;
        }

        let exchange = control.accept_control_exchange().await?;
        let ready = exchange.recv_message(STAGE).await?;
        let alternate = match ready.as_slice() {
            [] => None,
            binding if self.incoming => {
                Some(CandidateBinding::try_from(binding).map_err(|_| io::ErrorKind::InvalidData)?)
            }
            _ => return Err(io::ErrorKind::InvalidData.into()),
        };
        event(MemberFusionEvent::ReadyReceived);

        #[cfg(feature = "miopunch-test-hooks")]
        if self.incoming
            && let Some(gate) = &self.qualification_gate
        {
            gate.reverse_ready.notify_one();
        }
        let selection = if self.incoming {
            self.choose_reverse(control, &exchange, &event, alternate)
                .await?
        } else {
            self.reserve(control.stable_id()).await
        };
        if let Some(selection) = selection {
            permit.check(control).await?;
            if !policy.is_current(control) {
                return Err(if control.close_reason().is_some() {
                    io::ErrorKind::ConnectionAborted
                } else {
                    io::ErrorKind::PermissionDenied
                }
                .into());
            }
            exchange.send_message(STAGE, COMMIT).await?;
            exchange.finish_send().await?;
            event(MemberFusionEvent::Committed);
            #[cfg(feature = "miopunch-test-hooks")]
            if self.incoming {
                self.signal_reverse_response();
            }
            return Ok(selection);
        }
        exchange.send_message(STAGE, SUPERSEDED).await?;
        event(MemberFusionEvent::Superseded);
        let _ = exchange.finish_send_confirmed().await;
        Err(superseded())
    }

    async fn finish_larger(
        &self,
        id: usize,
        exchange: &ControlExchange,
        event: &(impl Fn(MemberFusionEvent) + Sync),
        preferred: bool,
    ) -> io::Result<Selection> {
        let mut message = exchange.recv_message(STAGE).await?;
        let mut reserved = None;
        if !preferred && let Ok(binding) = CandidateBinding::try_from(message.as_slice()) {
            // The answer and reservation share the same lock as preferred
            // qualification. A later candidate cannot make a negative answer stale.
            reserved = self
                .when_unselected(|state| {
                    state
                        .preferred_at(binding)
                        .is_none()
                        .then(|| self.reserve_in(state, id))
                })
                .await
                .ok_or_else(superseded)?;
            let (reply, observed) = if reserved.is_some() {
                (UNAVAILABLE, MemberFusionEvent::QualificationUnavailable)
            } else {
                (QUALIFIED, MemberFusionEvent::QualificationConfirmed)
            };
            exchange.send_message(STAGE, reply).await?;
            event(observed);
            message = exchange.recv_message(STAGE).await?;
        }
        match message.as_slice() {
            COMMIT => {
                exchange.finish_send().await?;
                event(MemberFusionEvent::CommitReceived);
                if let Some(reserved) = reserved {
                    Ok(reserved)
                } else {
                    self.reserve(id).await.ok_or_else(superseded)
                }
            }
            SUPERSEDED => {
                event(MemberFusionEvent::Superseded);
                Err(superseded())
            }
            _ => Err(io::ErrorKind::InvalidData.into()),
        }
    }

    async fn choose_reverse(
        &self,
        control: &ControlConnection,
        exchange: &ControlExchange,
        event: &(impl Fn(MemberFusionEvent) + Sync),
        advertised: Option<CandidateBinding>,
    ) -> io::Result<Option<Selection>> {
        let id = control.stable_id();
        // The local qualifier and the fallback's choice share the same decision
        // point. Without a qualified local alternate, READY leads straight to COMMIT.
        let choice = self
            .when_unselected(|state| {
                if let Some(binding) =
                    advertised.filter(|binding| state.preferred_at(*binding).is_some())
                {
                    ReverseChoice::Prefer(binding)
                } else if let Some(binding) = state.preferred_binding() {
                    ReverseChoice::Confirm(binding, self.reserve_in(state, id))
                } else {
                    ReverseChoice::Reserve(self.reserve_in(state, id))
                }
            })
            .await;
        let binding = match choice {
            None => return Ok(None),
            Some(ReverseChoice::Reserve(selection)) => return Ok(Some(selection)),
            Some(ReverseChoice::Prefer(binding)) => binding,
            Some(ReverseChoice::Confirm(binding, reserved)) => {
                exchange.send_message(STAGE, &binding).await?;
                event(MemberFusionEvent::QualificationRequested);
                #[cfg(feature = "miopunch-test-hooks")]
                self.signal_reverse_response();
                match exchange.recv_message(STAGE).await?.as_slice() {
                    QUALIFIED => {
                        event(MemberFusionEvent::QualificationConfirmed);
                        drop(reserved);
                    }
                    UNAVAILABLE => {
                        event(MemberFusionEvent::QualificationUnavailable);
                        return Ok(Some(reserved));
                    }
                    _ => return Err(io::ErrorKind::InvalidData.into()),
                }
                binding
            }
        };
        self.wait_for_preferred_candidate(binding).await;
        Ok(self.reserve(id).await)
    }

    #[cfg(feature = "miopunch-test-hooks")]
    fn signal_reverse_response(&self) {
        if let Some(stale) = self
            .qualification_gate
            .as_ref()
            .and_then(|gate| gate.stale.as_ref())
        {
            stale.reverse_response.notify_one();
        }
    }

    fn qualify_preferred(&mut self, control: &ControlConnection) -> io::Result<()> {
        let mut binding = [0; BINDING_SIZE];
        control
            .export_keying_material(&mut binding, b"EXPORTER-iroh-member-fusion-qualified", b"")
            .map_err(|_| io::Error::other("could not bind the qualified candidate to TLS"))?;
        let qualification = Qualification {
            binding,
            connection: control.weak_handle(),
            deadline: control
                .caller_deadline()
                .expect("fusion uses bounded admission"),
            cancellation: control.attempt_cancellation(),
        };
        self.peer
            .state
            .lock()
            .expect("member peer lock")
            .preferred
            .push(qualification);
        self.qualified = Some(binding);
        Ok(())
    }

    fn preferred_binding(&self) -> Option<CandidateBinding> {
        self.peer
            .state
            .lock()
            .expect("member peer lock")
            .preferred_binding()
    }

    async fn wait_for_preferred_candidate(&self, binding: CandidateBinding) {
        // The surrounding admission bounds the reverse candidate's wait. It
        // can resume if the exact preferred qualification ceases to be current.
        let mut changed = self.peer.changed.subscribe();
        loop {
            let deadline = {
                let state = self.peer.state.lock().expect("member peer lock");
                let peer_choice = state.preferred_at(binding);
                let Some(candidate) = peer_choice else { return };
                if state.connection().is_some() {
                    return;
                }
                candidate.deadline
            };
            if time::timeout_at(deadline, changed.changed()).await.is_err() {
                return;
            }
        }
    }

    // Wait for an in-flight selection to publish or abandon its reservation.
    // The surrounding admission future bounds this by the original deadline.
    async fn when_unselected<T>(&self, choose: impl FnOnce(&mut PeerState) -> T) -> Option<T> {
        let mut changed = self.peer.changed.subscribe();
        loop {
            {
                let mut state = self.peer.state.lock().expect("member peer lock");
                if state.connection().is_some() {
                    return None;
                }
                if state.selection.is_none() {
                    return Some(choose(&mut state));
                }
            }
            let _ = changed.changed().await;
        }
    }

    async fn reserve(&self, id: usize) -> Option<Selection> {
        self.when_unselected(|state| self.reserve_in(state, id))
            .await
    }

    fn reserve_in(&self, state: &mut PeerState, id: usize) -> Selection {
        state.selection = Some(id);
        Selection {
            peer: self.peer.clone(),
            id,
            idle_timeout: self.idle_timeout,
            shutdown: self.shutdown.clone(),
        }
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
    fn publish(self, connection: Connection, data_path: TransportAddr) -> Weak<Live> {
        let live = MemberConnection(Arc::new(Live {
            connection,
            data_path,
            activity: Arc::new(Activity::default()),
        }));
        {
            let mut state = self.peer.state.lock().expect("member peer lock");
            state.live = Some(live.clone());
            if live.side().is_server()
                && let Some(dial) = &state.dial
                && dial.race_routes
            {
                // Stop unstaged routes and stagger timers. The single mapped
                // dial already retires through the bilateral selection stage.
                dial.cancel.cancel();
            }
        }
        let selected = Arc::downgrade(&live.0);
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
        selected
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

    /// Remote address of the original data path verified at business handoff.
    pub fn initial_data_path(&self) -> &TransportAddr {
        &self.0.data_path
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
        self.incoming_bi().await?.accept().await
    }

    /// Receive a stream independently from reading its activity header.
    pub async fn incoming_bi(&self) -> io::Result<IncomingMemberStream> {
        let (send, recv) = self
            .0
            .connection
            .accept_bi()
            .await
            .map_err(io::Error::other)?;
        Ok(IncomingMemberStream {
            send,
            recv,
            activity: self.0.activity.clone(),
        })
    }
}

/// One incoming stream waiting to declare its activity scope.
#[derive(Debug)]
pub struct IncomingMemberStream {
    send: SendStream,
    recv: RecvStream,
    activity: Arc<Activity>,
}

impl IncomingMemberStream {
    /// Read and apply this stream's activity scope.
    pub async fn accept(mut self) -> io::Result<MemberStream> {
        let mut header = [0];
        self.recv
            .read_exact(&mut header)
            .await
            .map_err(io::Error::other)?;
        let kind = match header[0] {
            0 => StreamKind::Business,
            1 => StreamKind::Maintenance,
            _ => return Err(io::ErrorKind::InvalidData.into()),
        };
        let lease = self.activity.lease(kind);
        Ok(MemberStream {
            send: self.send,
            recv: self.recv,
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
    /// Whether this stream carries a local business-activity lease.
    pub fn is_business(&self) -> bool {
        self._lease.is_some()
    }
    /// Independently cancel or observe only this stream.
    pub fn control(&self) -> noq::StreamControl {
        self.send.control()
    }

    /// Access the original QUIC stream halves for this service.
    pub fn streams(&mut self) -> (&mut SendStream, &mut RecvStream) {
        (&mut self.send, &mut self.recv)
    }

    /// Separate transport halves while retaining a stream activity owner.
    pub fn into_parts(self) -> (SendStream, RecvStream, MemberStreamLease) {
        (
            self.send,
            self.recv,
            MemberStreamLease {
                _lease: self._lease,
            },
        )
    }
}

/// Invokes the existing idle policy when the last owner of a business stream ends.
#[derive(Debug)]
pub struct MemberStreamLease {
    _lease: Option<ActivityLease>,
}

#[derive(Debug)]
struct Live {
    connection: Connection,
    data_path: TransportAddr,
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
