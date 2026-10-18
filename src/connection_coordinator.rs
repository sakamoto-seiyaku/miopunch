//! Shared strict member connection coordination. Platform adapters own their
//! gates and candidate resources; this module owns negotiation and release.
use crate::control_protocol::AuthenticatedControl;
use futures_util::FutureExt;
use iroh::{
    Endpoint, TransportAddr,
    endpoint::{Connection, ControlExchange, ControlMessageFormat},
};
use std::{future::Future, io, pin::Pin};

pub const MEMBER_ALPN: &[u8] = b"miopunch/session/member/1";
pub const ADMISSION_REQUEST_ALPN: &[u8] = b"miopunch/session/admission/1";
pub const PRECODE_REQUEST_ALPN: &[u8] = b"miopunch/session/precode/1";
/// Restricted control for improvement or repair of an already living ordinary
/// member session. Both operations use the same binding/probe/commit exchange;
/// this control session never returns an ordinary handle.
pub const ACTIVE_IMPROVEMENT_ALPN: &[u8] = b"miopunch/session/improve/1";
pub const REPLICA_ALPN: &[u8] = b"miopunch/session/replica/1";
const FRAME_LEN: usize = 28;
pub(crate) const UDP: u8 = 1;
pub(crate) const TCP: u8 = 2;
pub(crate) const RELAY: u8 = 4;
pub(crate) const RTC: u8 = 8;
const CAPABILITIES: u8 = UDP | TCP | RELAY | RTC;

/// Start real fresh QUIC probes on this operation's existing permitted DATA
/// paths. The returned check requires the same still-open path, current
/// permission and its post-entry ACK; a listed path or old ACK is insufficient.
/// Probe progress belongs to the existing connection driver, with no task/timer.
pub(crate) fn probe_existing_data_survival(
    original: &Connection,
    permits: impl Fn(&TransportAddr) -> bool,
) -> impl Fn(&Connection, &dyn Fn(&TransportAddr) -> bool) -> bool {
    let original_id = original.stable_id();
    let original_peer = original.remote_id();
    let probes: Vec<_> = original
        .paths()
        .iter()
        .filter(|path| permits(path.remote_addr()))
        .filter_map(|path| {
            path.start_probe()
                .ok()
                .map(|probe| (path.id().to_string(), path.remote_addr().clone(), probe))
        })
        .collect();
    move |original, permits| {
        original.stable_id() == original_id
            && original.remote_id() == original_peer
            && original.close_reason().is_none()
            && probes.iter().any(|(id, address, probe)| {
                permits(address)
                    && matches!(probe.is_acknowledged(), Ok(true))
                    && original
                        .paths()
                        .iter()
                        .any(|path| path.id().to_string() == *id && path.remote_addr() == address)
            })
    }
}

/// A fast control rejection can precede an existing path's new ACK. Reuse the
/// established bounded path validator for the remainder of this same operation;
/// contact addresses are never added to its DATA offer.
pub(crate) async fn validate_existing_data_survival(
    endpoint: &(impl crate::connection_recovery_core::RecoveryEndpoint + ?Sized),
    original: &Connection,
    deadline: n0_future::time::Instant,
    permits: &impl Fn(&TransportAddr) -> bool,
    check: &impl Fn(&Connection) -> io::Result<()>,
) -> bool {
    if deadline <= n0_future::time::Instant::now() || original.close_reason().is_some() {
        return false;
    }
    let mut candidate = iroh::EndpointAddr::new(original.remote_id());
    candidate.addrs.extend(
        original
            .paths()
            .iter()
            .map(|path| path.remote_addr().clone())
            .filter(permits)
            .take(crate::connection_recovery_core::MAX_CANDIDATES),
    );
    n0_future::time::timeout(
        deadline.saturating_duration_since(n0_future::time::Instant::now()),
        crate::connection_recovery_core::validate_candidates(
            endpoint,
            original,
            original,
            candidate,
            permits,
            &|original, _| check(original),
        ),
    )
    .await
    .is_ok_and(|result| result.is_ok())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SessionPurpose {
    MemberBusiness = 1,
    AdmissionRequest = 2,
    Replica = 3,
    ActiveImprovement = 4,
}
impl SessionPurpose {
    pub fn alpn(self) -> &'static [u8] {
        match self {
            Self::MemberBusiness => MEMBER_ALPN,
            Self::AdmissionRequest => ADMISSION_REQUEST_ALPN,
            Self::Replica => REPLICA_ALPN,
            Self::ActiveImprovement => ACTIVE_IMPROVEMENT_ALPN,
        }
    }
    pub fn from_alpn(alpn: &[u8]) -> Option<Self> {
        match alpn {
            MEMBER_ALPN => Some(Self::MemberBusiness),
            ADMISSION_REQUEST_ALPN | PRECODE_REQUEST_ALPN => Some(Self::AdmissionRequest),
            REPLICA_ALPN => Some(Self::Replica),
            ACTIVE_IMPROVEMENT_ALPN => Some(Self::ActiveImprovement),
            _ => None,
        }
    }
    pub(crate) fn requires_data(self) -> bool {
        !matches!(self, Self::AdmissionRequest | Self::ActiveImprovement)
    }
    fn label(self) -> &'static str {
        match self {
            Self::MemberBusiness => "member-business",
            Self::AdmissionRequest => "admission-request",
            Self::Replica => "replica",
            Self::ActiveImprovement => "active-improvement",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum Platform {
    Native = 1,
    Browser = 2,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum AttemptRole {
    Initiator = 1,
    Responder = 2,
}
impl AttemptRole {
    pub(crate) fn initial(connection: &(impl AuthenticatedControl + ?Sized)) -> Self {
        if connection.side().is_client() {
            Self::Initiator
        } else {
            Self::Responder
        }
    }
    fn opposite(self) -> Self {
        match self {
            Self::Initiator => Self::Responder,
            Self::Responder => Self::Initiator,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum RtcRole {
    None = 0,
    Offerer = 1,
    Answerer = 2,
}
impl RtcRole {
    pub(crate) fn for_attempt(role: AttemptRole, supported: bool) -> Self {
        if !supported {
            Self::None
        } else {
            match role {
                AttemptRole::Initiator => Self::Offerer,
                AttemptRole::Responder => Self::Answerer,
            }
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct RuntimeCapabilities {
    pub runtime: Platform,
    pub supported: u8,
    pub permitted: u8,
    pub rtc_role: RtcRole,
}
impl RuntimeCapabilities {
    fn validate(self, role: AttemptRole) -> io::Result<()> {
        if self.supported & !CAPABILITIES != 0
            || self.permitted & !self.supported != 0
            || (self.supported & RTC == 0 && self.rtc_role != RtcRole::None)
            || (self.supported & RTC != 0 && self.rtc_role != RtcRole::for_attempt(role, true))
            || (self.runtime == Platform::Browser && self.supported & (UDP | TCP) != 0)
        {
            return Err(control_error("InvalidControl"));
        }
        Ok(())
    }
}

/// Private negotiation identity for one attempt on one original M. Initial
/// connect uses its initiator; future improvements can pass another role.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ProtectedAttempt {
    id: [u8; 16],
    pub role: AttemptRole,
    pub rtc_role: RtcRole,
    pub local_runtime: Platform,
    pub peer_runtime: Platform,
    pub mutual: u8,
}
impl ProtectedAttempt {
    pub(crate) fn id_hex(self) -> String {
        hex::encode(self.id)
    }
    pub(crate) fn matches(self, id: &str) -> bool {
        id == self.id_hex()
    }
    pub(crate) fn quic_dialer(
        self,
        endpoint: &Endpoint,
        connection: &Connection,
    ) -> iroh_base::EndpointId {
        self.quic_dialer_for(endpoint.id(), connection)
    }
    pub(crate) fn quic_dialer_for(
        self,
        local: iroh_base::EndpointId,
        connection: &(impl AuthenticatedControl + ?Sized),
    ) -> iroh_base::EndpointId {
        if connection.side().is_client() {
            local
        } else {
            connection.remote_id()
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum PathPlan {
    ExistingDirect,
    ExistingRelay,
    Rtc,
    Relay,
    Native { tcp: bool, udp: bool },
}
impl ProtectedAttempt {
    fn plans(
        self,
        connection: &(impl AuthenticatedControl + ?Sized),
        direct: Option<&(TransportAddr, u8)>,
        relay: Option<&TransportAddr>,
    ) -> io::Result<Vec<PathPlan>> {
        let _ = connection;
        if direct.is_some_and(|(_, capability)| self.mutual & capability != 0) {
            return Ok(vec![PathPlan::ExistingDirect]);
        }
        if self.mutual & RELAY != 0 && relay.is_some() {
            return Ok(vec![PathPlan::ExistingRelay]);
        }
        let mut plans = Vec::new();
        if self.mutual & RELAY != 0 {
            plans.push(PathPlan::Relay);
        }
        if self.local_runtime == Platform::Native && self.peer_runtime == Platform::Native {
            if self.mutual & (UDP | TCP) != 0 {
                plans.push(PathPlan::Native {
                    tcp: self.mutual & TCP != 0,
                    udp: self.mutual & UDP != 0,
                });
            }
        } else if self.mutual & RTC != 0 {
            plans.push(PathPlan::Rtc);
        }
        if plans.is_empty() {
            Err(control_error("DataPathUnavailable"))
        } else {
            Ok(plans)
        }
    }
}

/// Platform adapter seam. Capability and authorization checks do not grant
/// membership. Preparation returns an exact, connection-owned candidate.
pub(crate) trait MemberPathAdapter<C: AuthenticatedControl + ?Sized = Connection> {
    fn capabilities(&self, role: AttemptRole) -> RuntimeCapabilities;
    /// Installed one-hop relay clients can be learned from this exact M,
    /// without advertising an uninstalled transport as a node capability.
    fn capabilities_for(&self, _connection: &C, role: AttemptRole) -> RuntimeCapabilities {
        self.capabilities(role)
    }
    fn existing_relay(&self, connection: &C) -> Option<TransportAddr> {
        connection
            .paths()
            .iter()
            .filter(|path| path.is_relay())
            .max_by_key(|path| path.is_selected())
            .map(|path| path.remote_addr().clone())
    }
    fn authorization_revision(&self, connection: &C, purpose: SessionPurpose) -> io::Result<u64>;
    fn committed(&self, _connection: &C, _report: serde_json::Value) {}
    // Bounded local diagnostics only; this callback cannot influence commit.
    fn active_diagnostic(&self, _original: &C, _control: &C, _stage: &'static str) {}
    /// An optional Native fixture yield after real authenticated Ready input.
    /// Default adapters introduce no hold and no change to the wire protocol.
    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    fn after_actual_ready_input_fixture<'a>(
        &'a self,
        _original: usize,
        _control: usize,
        _stream: String,
        _frame: &'a [u8],
        _deadline: n0_future::time::Instant,
    ) -> impl Future<Output = io::Result<()>> + Send + 'a {
        async { Ok(()) }
    }
    /// Register a successfully released ordinary M with the existing
    /// maintenance owner.  Implementations store only a weak handle.
    fn register_original(&self, _connection: &C) {}
    fn existing_direct(&self, connection: &C) -> Option<(TransportAddr, u8)> {
        let paths = connection.paths();
        paths
            .iter()
            .filter(|path| path.is_ip())
            .max_by_key(|path| path.is_selected())
            .map(|path| (path.remote_addr().clone(), UDP))
    }
    fn check<'a>(
        &'a self,
        connection: &'a C,
        purpose: SessionPurpose,
    ) -> impl Future<Output = io::Result<()>> + Send + 'a;
    fn prepare<'a>(
        &'a self,
        endpoint: Option<&'a Endpoint>,
        connection: &'a C,
        attempt: ProtectedAttempt,
        plan: PathPlan,
    ) -> impl Future<Output = io::Result<PreparedPath>> + Send + 'a;

    /// Prepare a candidate on an existing M while signalling on the distinct
    /// restricted C.  Native/browser adapters override this for RTC; the
    /// default keeps future direct adapters on the same exact-M seam.
    fn prepare_active<'a>(
        &'a self,
        endpoint: Option<&'a Endpoint>,
        original: &'a C,
        control: &'a C,
        attempt: ProtectedAttempt,
        plan: PathPlan,
    ) -> impl Future<Output = io::Result<PreparedPath>> + Send + 'a {
        let _ = (endpoint, original, control, attempt, plan);
        async { Err(control_error("DataPathUnavailable")) }
    }
}

type Retain = Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = io::Result<()>> + Send>> + Send>;
/// A live exact route and fresh original-M proof, with candidate-only rollback.
/// Native preparation may defer its retain until bilateral readiness. RTC
/// adapters retain their existing lease behavior and return their sealed probe.
pub(crate) struct PreparedPath {
    stable_id: usize,
    route: TransportAddr,
    path_id: Option<String>,
    watermark: u64,
    acknowledged: Option<Box<dyn Fn() -> io::Result<bool> + Send + Sync>>,
    current: Box<dyn Fn() -> io::Result<()> + Send + Sync>,
    retain: Option<Retain>,
    rollback: Option<Box<dyn FnOnce() + Send>>,
}
impl PreparedPath {
    pub(crate) fn candidate(
        connection: &(impl AuthenticatedControl + ?Sized),
        route: TransportAddr,
        current: impl Fn() -> io::Result<()> + Send + Sync + 'static,
    ) -> Self {
        Self {
            stable_id: connection.stable_id(),
            route,
            path_id: None,
            watermark: 0,
            acknowledged: None,
            current: Box::new(current),
            retain: None,
            rollback: None,
        }
    }
    pub(crate) fn with_proof(
        mut self,
        path_id: String,
        watermark: u64,
        acknowledged: impl Fn() -> io::Result<bool> + Send + Sync + 'static,
    ) -> Self {
        self.path_id = Some(path_id);
        self.watermark = watermark;
        self.acknowledged = Some(Box::new(acknowledged));
        self
    }
    pub(crate) fn with_exact_path_id(mut self, path_id: String) -> Self {
        self.path_id = Some(path_id);
        self
    }
    pub(crate) fn with_retain(
        mut self,
        retain: impl FnOnce() -> Pin<Box<dyn Future<Output = io::Result<()>> + Send>> + Send + 'static,
    ) -> Self {
        self.retain = Some(Box::new(retain));
        self
    }
    pub(crate) fn with_rollback(mut self, rollback: impl FnOnce() + Send + 'static) -> Self {
        self.rollback = Some(Box::new(rollback));
        self
    }
    fn check(
        &self,
        connection: &(impl AuthenticatedControl + ?Sized),
        require_ack: bool,
    ) -> io::Result<()> {
        if connection.stable_id() != self.stable_id || connection.close_reason().is_some() {
            return Err(control_error("DataPathUnavailable"));
        }
        (self.current)()?;
        if !connection.paths().iter().any(|path| {
            path.remote_addr() == &self.route
                && self
                    .path_id
                    .as_ref()
                    .is_none_or(|id| path.id().to_string() == *id)
        }) {
            return Err(control_error("DataPathUnavailable"));
        }
        if require_ack && !matches!(self.acknowledged.as_ref().map(|ack| ack()), Some(Ok(true))) {
            return Err(control_error("DataPathUnavailable"));
        }
        Ok(())
    }
}
impl Drop for PreparedPath {
    fn drop(&mut self) {
        if let Some(rollback) = self.rollback.take() {
            rollback();
        }
    }
}

#[derive(Clone, Debug)]
pub struct AttemptContext {
    pub deadline: n0_future::time::Instant,
    pub cancellation: tokio_util::sync::CancellationToken,
}
impl AttemptContext {
    pub fn new(
        deadline: n0_future::time::Instant,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Self {
        Self {
            deadline,
            cancellation,
        }
    }
    async fn run<T>(
        &self,
        work: impl Future<Output = Result<T, ConnectError>>,
    ) -> Result<T, ConnectError> {
        let cancelled = self.cancellation.cancelled().fuse();
        let work = work.fuse();
        futures_util::pin_mut!(cancelled, work);
        n0_future::time::timeout(self.deadline.saturating_duration_since(n0_future::time::Instant::now()),async {
            futures_util::select_biased! { _=cancelled => Err(ConnectError::Cancelled), result=work=>result }
        }).await.unwrap_or(Err(ConnectError::DeadlineExceeded))
    }
}
#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error("identity or admission rejected")]
    IdentityOrAdmissionRejected,
    #[error("unified connection protocol unsupported")]
    ProtocolUnsupported,
    #[error("invalid protected connection control")]
    InvalidControl,
    #[error("no currently permitted data path")]
    DataPathUnavailable,
    #[error("connection deadline exceeded")]
    DeadlineExceeded,
    #[error("connection attempt cancelled")]
    Cancelled,
    #[error("connection transport failed: {0}")]
    Transport(String),
}
impl ConnectError {
    fn from_connecting(error: iroh::endpoint::ConnectingError) -> Self {
        use iroh::endpoint::ConnectingError;
        match error {
            ConnectingError::AttemptCancelled { .. } => Self::Cancelled,
            ConnectingError::AttemptTimedOut { .. } => Self::DeadlineExceeded,
            ConnectingError::ConnectionError {
                source: iroh::endpoint::ConnectionError::ApplicationClosed(_),
                ..
            } => Self::DataPathUnavailable,
            ConnectingError::LocallyRejected { .. } => Self::IdentityOrAdmissionRejected,
            ConnectingError::AdmissionFailed { reason, kind, .. } => {
                if kind == io::ErrorKind::TimedOut {
                    Self::DeadlineExceeded
                } else if kind == io::ErrorKind::Interrupted {
                    Self::Cancelled
                } else if reason.starts_with("unified/ProtocolUnsupported") {
                    Self::ProtocolUnsupported
                } else if reason.starts_with("unified/InvalidControl") {
                    Self::InvalidControl
                } else if kind == io::ErrorKind::PermissionDenied
                    || reason.starts_with("unified/IdentityOrAdmissionRejected")
                {
                    Self::IdentityOrAdmissionRejected
                } else {
                    Self::DataPathUnavailable
                }
            }
            ConnectingError::DataPathUnavailable { reason, .. } => {
                if reason == "unified caller deadline exceeded" {
                    Self::DeadlineExceeded
                } else if reason.starts_with("unified/ProtocolUnsupported") {
                    Self::ProtocolUnsupported
                } else if reason.starts_with("unified/InvalidControl") {
                    Self::InvalidControl
                } else if reason.starts_with("unified/IdentityOrAdmissionRejected") {
                    Self::IdentityOrAdmissionRejected
                } else {
                    Self::DataPathUnavailable
                }
            }
            _ => Self::Transport(error.to_string()),
        }
    }
}
enum ContactDialFailure {
    Unavailable(ConnectError),
    Terminal(ConnectError),
}
// The two platform owners share contact selection without widening the
// restricted endpoint into an ordinary endpoint.
#[derive(Clone)]
enum EndpointAccess {
    Ordinary(Endpoint),
    Restricted(iroh::endpoint::RestrictedEndpoint),
}
impl EndpointAccess {
    fn custom_addrs(&self) -> Vec<iroh_base::CustomAddr> {
        match self {
            Self::Ordinary(e) => e.custom_addrs(),
            Self::Restricted(e) => e.custom_addrs(),
        }
    }
    async fn remote_info(&self, id: iroh::EndpointId) -> Option<iroh::endpoint::RemoteInfo> {
        match self {
            Self::Ordinary(e) => e.remote_info(id).await,
            Self::Restricted(e) => e.remote_info(id).await,
        }
    }
}

pub(crate) enum ConnectionDelivery {
    Business(Connection),
    Control(crate::control_protocol::ControlOwner),
}
impl ConnectionDelivery {
    pub(crate) fn business(self) -> Result<Connection, ConnectError> {
        match self {
            Self::Business(c) => Ok(c),
            Self::Control(c) => {
                c.close(0u32.into(), b"business delivery required");
                Err(ConnectError::InvalidControl)
            }
        }
    }
    pub(crate) fn control(self) -> Result<crate::control_protocol::ControlOwner, ConnectError> {
        match self {
            Self::Control(c) => Ok(c),
            Self::Business(c) => {
                c.close(0u32.into(), b"control delivery required");
                Err(ConnectError::InvalidControl)
            }
        }
    }
    fn alpn(&self) -> &[u8] {
        match self {
            Self::Business(c) => c.alpn(),
            Self::Control(c) => c.alpn(),
        }
    }
    fn close(&self, reason: &[u8]) {
        match self {
            Self::Business(c) => c.close(0u32.into(), reason),
            Self::Control(c) => c.close(0u32.into(), reason),
        }
    }
}

/// An incoming attempt from either platform owner, preserving its capability.
/// Neither variant exposes an early connection or converts between endpoints.
pub enum CoordinatedIncoming {
    Ordinary(iroh::endpoint::Incoming),
    Restricted(iroh::endpoint::RestrictedIncoming),
}
impl From<iroh::endpoint::Incoming> for CoordinatedIncoming {
    fn from(v: iroh::endpoint::Incoming) -> Self {
        Self::Ordinary(v)
    }
}
impl From<iroh::endpoint::RestrictedIncoming> for CoordinatedIncoming {
    fn from(v: iroh::endpoint::RestrictedIncoming) -> Self {
        Self::Restricted(v)
    }
}
impl CoordinatedIncoming {
    fn accept(self) -> Result<CoordinatedAccepting, iroh::endpoint::ConnectionError> {
        match self {
            Self::Ordinary(i) => i.accept().map(CoordinatedAccepting::Ordinary),
            Self::Restricted(i) => i.accept().map(CoordinatedAccepting::Restricted),
        }
    }
}
/// TLS classification preserves the original incoming deadline and capability.
pub enum CoordinatedAccepting {
    Ordinary(iroh::endpoint::Accepting),
    Restricted(iroh::endpoint::RestrictedAccepting),
}
impl From<iroh::endpoint::Accepting> for CoordinatedAccepting {
    fn from(v: iroh::endpoint::Accepting) -> Self {
        Self::Ordinary(v)
    }
}
impl From<iroh::endpoint::RestrictedAccepting> for CoordinatedAccepting {
    fn from(v: iroh::endpoint::RestrictedAccepting) -> Self {
        Self::Restricted(v)
    }
}
impl CoordinatedAccepting {
    fn with_attempt(self, attempt: &AttemptContext) -> Self {
        match self {
            Self::Ordinary(a) => Self::Ordinary(
                a.with_caller_deadline(attempt.deadline)
                    .with_attempt_cancellation(attempt.cancellation.clone()),
            ),
            Self::Restricted(a) => Self::Restricted(
                a.with_caller_deadline(attempt.deadline)
                    .with_attempt_cancellation(attempt.cancellation.clone()),
            ),
        }
    }
    async fn alpn(&mut self) -> Result<Vec<u8>, iroh::endpoint::AlpnError> {
        match self {
            Self::Ordinary(a) => a.alpn().await,
            Self::Restricted(a) => a.alpn().await,
        }
    }
    async fn deliver(
        self,
        control_only: bool,
    ) -> Result<ConnectionDelivery, iroh::endpoint::ConnectingError> {
        use crate::control_protocol::OwnedControl;
        match self {
            Self::Ordinary(a) => a.await.map(|c| {
                if control_only {
                    ConnectionDelivery::Control(c.retain_control())
                } else {
                    ConnectionDelivery::Business(c)
                }
            }),
            Self::Restricted(a) if control_only => a
                .into_control()
                .await
                .map(|c| ConnectionDelivery::Control(std::sync::Arc::new(c).retain_control())),
            Self::Restricted(a) => a.await.map(ConnectionDelivery::Business),
        }
    }
}

#[derive(Clone)]
pub struct ConnectionCoordinator {
    endpoint: EndpointAccess,
    permitted: u8,
    contact_ids: Vec<u64>,
    member_contact_id: Option<u64>,
    member_data_id: Option<u64>,
}
impl ConnectionCoordinator {
    pub(crate) fn new(endpoint: Endpoint, permitted: u8, contact_ids: Vec<u64>) -> Self {
        Self {
            endpoint: EndpointAccess::Ordinary(endpoint),
            permitted,
            contact_ids,
            member_contact_id: None,
            member_data_id: None,
        }
    }
    pub(crate) fn new_restricted(
        endpoint: iroh::endpoint::RestrictedEndpoint,
        permitted: u8,
        contact_ids: Vec<u64>,
    ) -> Self {
        Self {
            endpoint: EndpointAccess::Restricted(endpoint),
            permitted,
            contact_ids,
            member_contact_id: None,
            member_data_id: None,
        }
    }
    /// The native resource owner supplies the installed member contact carrier.
    /// Browser callers have no member adapter and retain None.
    pub(crate) fn with_member_contact(mut self, id: u64) -> Self {
        self.member_contact_id = Some(id);
        self
    }
    /// Separate from member signaling: only an installed, currently
    /// advertised data adapter may enter the relay tier.
    pub(crate) fn with_member_data(mut self, id: u64) -> Self {
        if self.permitted & RELAY != 0
            && self
                .endpoint
                .custom_addrs()
                .iter()
                .any(|address| address.id() == id)
        {
            self.member_data_id = Some(id);
            if !self.contact_ids.contains(&id) {
                self.contact_ids.push(id);
            }
        }
        self
    }
    async fn dial(
        &self,
        target: iroh::EndpointAddr,
        purpose: SessionPurpose,
        attempt: &AttemptContext,
        wire_alpn: &[u8],
        control_only: bool,
    ) -> Result<ConnectionDelivery, ContactDialFailure> {
        if SessionPurpose::from_alpn(wire_alpn) != Some(purpose) {
            return Err(ContactDialFailure::Terminal(
                ConnectError::ProtocolUnsupported,
            ));
        }
        use iroh::endpoint::{ConnectWithOptsError, ConnectingError};
        if attempt.cancellation.is_cancelled() {
            return Err(ContactDialFailure::Terminal(ConnectError::Cancelled));
        }
        if n0_future::time::Instant::now() >= attempt.deadline {
            return Err(ContactDialFailure::Terminal(ConnectError::DeadlineExceeded));
        }
        let mut options = iroh::endpoint::ConnectOptions::new()
            .with_caller_deadline(attempt.deadline)
            .with_attempt_cancellation(attempt.cancellation.clone());
        if let Some(route) = target.addrs.iter().next()
            && target.addrs.len() == 1
        {
            options = options.with_bootstrap_route(route.clone());
        }
        use crate::control_protocol::OwnedControl;
        let setup_error = |error: ConnectWithOptsError| match error {
            ConnectWithOptsError::LocallyRejected { .. } => {
                ContactDialFailure::Terminal(ConnectError::IdentityOrAdmissionRejected)
            }
            ConnectWithOptsError::NoAddress { .. } => {
                ContactDialFailure::Unavailable(ConnectError::Transport(error.to_string()))
            }
            _ => ContactDialFailure::Terminal(ConnectError::Transport(error.to_string())),
        };
        let delivered = match &self.endpoint {
            EndpointAccess::Ordinary(endpoint) => endpoint
                .connect_with_opts(target, wire_alpn, options)
                .await
                .map_err(setup_error)?
                .await
                .map(|c| {
                    if control_only {
                        ConnectionDelivery::Control(c.retain_control())
                    } else {
                        ConnectionDelivery::Business(c)
                    }
                }),
            EndpointAccess::Restricted(endpoint) if control_only => endpoint
                .connect_control_with_opts(target, wire_alpn, options)
                .await
                .map_err(setup_error)?
                .await
                .map(|c| ConnectionDelivery::Control(std::sync::Arc::new(c).retain_control())),
            EndpointAccess::Restricted(endpoint) => endpoint
                .connect_with_opts(target, wire_alpn, options)
                .await
                .map_err(setup_error)?
                .await
                .map(ConnectionDelivery::Business),
        };
        delivered.map_err(|error| {
            // Only QUIC setup reachability failures may move to another contact.
            // Protected gate/control/data failures have an authenticated M and
            // must never be turned into a new membership attempt.
            let unavailable = matches!(&error,
                ConnectingError::ConnectionError { source, .. }
                if matches!(source, iroh::endpoint::ConnectionError::TimedOut
                    | iroh::endpoint::ConnectionError::Reset));
            let error = ConnectError::from_connecting(error);
            if unavailable {
                ContactDialFailure::Unavailable(error)
            } else {
                ContactDialFailure::Terminal(error)
            }
        })
    }
    pub async fn connect(
        &self,
        target: iroh::EndpointAddr,
        purpose: SessionPurpose,
        attempt: AttemptContext,
    ) -> Result<Connection, ConnectError> {
        self.connect_alpn(target, purpose.alpn(), attempt).await
    }
    /// Establish only the constrained control scope for this purpose.
    pub async fn connect_control(
        &self,
        target: iroh::EndpointAddr,
        purpose: SessionPurpose,
        attempt: AttemptContext,
    ) -> Result<crate::control_protocol::ControlOwner, ConnectError> {
        self.connect_delivery(target, purpose.alpn(), attempt, true)
            .await?
            .control()
    }
    pub(crate) async fn connect_control_alpn(
        &self,
        target: iroh::EndpointAddr,
        alpn: &[u8],
        attempt: AttemptContext,
    ) -> Result<crate::control_protocol::ControlOwner, ConnectError> {
        self.connect_delivery(target, alpn, attempt, true)
            .await?
            .control()
    }
    pub(crate) async fn connect_alpn(
        &self,
        target: iroh::EndpointAddr,
        wire_alpn: &[u8],
        attempt: AttemptContext,
    ) -> Result<Connection, ConnectError> {
        self.connect_delivery(target, wire_alpn, attempt, false)
            .await?
            .business()
    }
    pub(crate) async fn connect_delivery(
        &self,
        mut target: iroh::EndpointAddr,
        wire_alpn: &[u8],
        attempt: AttemptContext,
        control_only: bool,
    ) -> Result<ConnectionDelivery, ConnectError> {
        let purpose =
            SessionPurpose::from_alpn(wire_alpn).ok_or(ConnectError::ProtocolUnsupported)?;
        attempt
            .run(async {
                if purpose.requires_data() && self.permitted == 0 {
                    return Err(ConnectError::DataPathUnavailable);
                }
                target.addrs.retain(|address| match address {
                    TransportAddr::Ip(_) => self.permitted & UDP != 0,
                    TransportAddr::Relay(_) => self.permitted & RELAY != 0,
                    TransportAddr::Custom(address) => self.contact_ids.contains(&address.id()),
                    _ => false,
                });
                if target.addrs.is_empty() {
                    return Err(ConnectError::DataPathUnavailable);
                }
                let mut active_ips = Vec::new();
                if let Some(info) = self.endpoint.remote_info(target.id).await {
                    for entry in info.addrs() {
                        if matches!(entry.usage(), iroh::endpoint::TransportAddrUsage::Active)
                            && let TransportAddr::Ip(address) = entry.addr()
                        {
                            active_ips.push(TransportAddr::Ip(*address));
                        }
                    }
                }
                let is_active_ip = |address: &TransportAddr| {
                    matches!(address, TransportAddr::Ip(_))
                        && active_ips.iter().any(|active| active == address)
                };
                let mut last_unavailable = None;
                // A currently open IP path is the strongest direct hint. Relay
                // routes remain eligible on first contact, even without a
                // cached peer path. Unverified or inactive IP announcements
                // are held until member/public contact has had its turn.
                let active_routes = target.addrs.iter().filter(|address| is_active_ip(address));
                let relay_routes = target.addrs.iter().filter(|address| {
                    matches!(address, TransportAddr::Relay(_))
                        || matches!(address, TransportAddr::Custom(route)
                            if Some(route.id()) == self.member_data_id)
                });
                for route in active_routes.chain(relay_routes) {
                    let data = iroh::EndpointAddr::new(target.id).with_addrs([route.clone()]);
                    match self
                        .dial(data, purpose, &attempt, wire_alpn, control_only)
                        .await
                    {
                        Ok(connection) => return Ok(connection),
                        Err(ContactDialFailure::Terminal(error)) => return Err(error),
                        Err(ContactDialFailure::Unavailable(error)) => {
                            last_unavailable = Some(error)
                        }
                    }
                }
                let mut public_contact = target.clone();
                public_contact.addrs.retain(|address| {
                    !matches!(address, TransportAddr::Ip(_) | TransportAddr::Relay(_))
                        && !matches!(address, TransportAddr::Custom(route)
                            if Some(route.id()) == self.member_data_id)
                });
                if let Some(member_id) = self.member_contact_id {
                    public_contact.addrs.retain(|address| {
                        !matches!(address,
                        TransportAddr::Custom(route) if route.id() == member_id)
                    });
                    for route in target.addrs.iter().filter_map(|address| match address {
                        TransportAddr::Custom(route) if route.id() == member_id => Some(route),
                        _ => None,
                    }) {
                        // A withdrawn local member registration is actual
                        // unavailability, including an endpoint/link retirement
                        // since this coordinator was constructed.
                        if !self
                            .endpoint
                            .custom_addrs()
                            .iter()
                            .any(|local| local.id() == member_id)
                        {
                            break;
                        }
                        let member = iroh::EndpointAddr::new(target.id)
                            .with_addrs([TransportAddr::Custom(route.clone())]);
                        match self
                            .dial(member, purpose, &attempt, wire_alpn, control_only)
                            .await
                        {
                            Ok(connection) => return Ok(connection),
                            Err(ContactDialFailure::Terminal(error)) => return Err(error),
                            Err(ContactDialFailure::Unavailable(error)) => {
                                last_unavailable = Some(error)
                            }
                        }
                    }
                }
                // Cold direct-only targets still need their announced IP
                // fallback. When a contact alternative exists, this comes
                // after member signaling and configured public contact.
                let mut inactive_ips = target.addrs.iter().filter(|address| {
                    matches!(address, TransportAddr::Ip(_)) && !is_active_ip(address)
                });
                if public_contact.addrs.is_empty() {
                    for route in inactive_ips.by_ref() {
                        let data = iroh::EndpointAddr::new(target.id).with_addrs([route.clone()]);
                        match self
                            .dial(data, purpose, &attempt, wire_alpn, control_only)
                            .await
                        {
                            Ok(connection) => return Ok(connection),
                            Err(ContactDialFailure::Terminal(error)) => return Err(error),
                            Err(ContactDialFailure::Unavailable(error)) => {
                                last_unavailable = Some(error)
                            }
                        }
                    }
                    return Err(last_unavailable.unwrap_or(ConnectError::DataPathUnavailable));
                }
                match self
                    .dial(public_contact, purpose, &attempt, wire_alpn, control_only)
                    .await
                {
                    Ok(connection) => return Ok(connection),
                    Err(ContactDialFailure::Terminal(error)) => return Err(error),
                    Err(ContactDialFailure::Unavailable(error)) => last_unavailable = Some(error),
                }
                for route in inactive_ips {
                    let data = iroh::EndpointAddr::new(target.id).with_addrs([route.clone()]);
                    match self
                        .dial(data, purpose, &attempt, wire_alpn, control_only)
                        .await
                    {
                        Ok(connection) => return Ok(connection),
                        Err(ContactDialFailure::Terminal(error)) => return Err(error),
                        Err(ContactDialFailure::Unavailable(error)) => {
                            last_unavailable = Some(error)
                        }
                    }
                }
                Err(last_unavailable.unwrap_or(ConnectError::DataPathUnavailable))
            })
            .await
    }
    pub async fn accept(
        &self,
        incoming: impl Into<CoordinatedIncoming>,
        purpose: SessionPurpose,
        attempt: AttemptContext,
    ) -> Result<Connection, ConnectError> {
        self.accept_delivery(incoming.into(), purpose, attempt, false)
            .await?
            .business()
    }
    /// Receive a control-only admission without an ordinary handle.
    pub async fn accept_control(
        &self,
        incoming: impl Into<CoordinatedIncoming>,
        purpose: SessionPurpose,
        attempt: AttemptContext,
    ) -> Result<crate::control_protocol::ControlOwner, ConnectError> {
        self.accept_delivery(incoming.into(), purpose, attempt, true)
            .await?
            .control()
    }
    async fn accept_delivery(
        &self,
        incoming: CoordinatedIncoming,
        purpose: SessionPurpose,
        attempt: AttemptContext,
        control_only: bool,
    ) -> Result<ConnectionDelivery, ConnectError> {
        attempt
            .run(async {
                let accepting = incoming
                    .accept()
                    .map_err(|e| ConnectError::Transport(e.to_string()))?;
                self.accept_inner(accepting, purpose, &attempt, control_only)
                    .await
            })
            .await
    }
    /// Route a pending TLS attempt under its original clock and cancellation.
    pub async fn accept_accepting(
        &self,
        accepting: impl Into<CoordinatedAccepting>,
        purpose: SessionPurpose,
        attempt: AttemptContext,
    ) -> Result<Connection, ConnectError> {
        attempt
            .run(self.accept_inner(accepting.into(), purpose, &attempt, false))
            .await?
            .business()
    }
    pub async fn accept_control_accepting(
        &self,
        accepting: impl Into<CoordinatedAccepting>,
        purpose: SessionPurpose,
        attempt: AttemptContext,
    ) -> Result<crate::control_protocol::ControlOwner, ConnectError> {
        attempt
            .run(self.accept_inner(accepting.into(), purpose, &attempt, true))
            .await?
            .control()
    }
    async fn accept_inner(
        &self,
        accepting: CoordinatedAccepting,
        purpose: SessionPurpose,
        attempt: &AttemptContext,
        control_only: bool,
    ) -> Result<ConnectionDelivery, ConnectError> {
        let mut accepting = accepting.with_attempt(attempt);
        let alpn = accepting
            .alpn()
            .await
            .map_err(|e| ConnectError::Transport(e.to_string()))?;
        if SessionPurpose::from_alpn(&alpn) != Some(purpose) {
            return Err(ConnectError::ProtocolUnsupported);
        }
        let connection = accepting
            .deliver(control_only)
            .await
            .map_err(ConnectError::from_connecting)?;
        if SessionPurpose::from_alpn(connection.alpn()) != Some(purpose) {
            connection.close(b"unified protocol required");
            return Err(ConnectError::ProtocolUnsupported);
        }
        Ok(connection)
    }
}

pub(crate) fn control_error(kind: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("unified/{kind}"))
}
struct PendingMember(Option<iroh::endpoint::WeakControlConnection>);
impl Drop for PendingMember {
    fn drop(&mut self) {
        if let Some(connection) = self.0.take() {
            connection.close(0u32.into(), b"unified pending member retired");
        }
    }
}

/// One shared protected protocol and release path on both platforms. The
/// endpoint's original admission hook still owns identity/proof verification.
pub(crate) struct ControlOffer {
    pub(crate) peer: [u8; FRAME_LEN],
    pub(crate) attempt: ProtectedAttempt,
    exchange: ControlExchange,
    nonce: [u8; 16],
}
impl ControlOffer {
    pub(crate) async fn confirm(self) -> io::Result<()> {
        let mut ready = [0; 17];
        ready[0] = 1;
        ready[1..].copy_from_slice(&self.peer[12..]);
        let format = ControlMessageFormat::EndTerminated { maximum: 17 };
        self.exchange.send_message(format, &ready).await?;
        let ack = self.exchange.recv_message(format).await?;
        if ack.len() != 17 || ack[0] != 1 || ack[1..] != self.nonce {
            return Err(control_error("InvalidControl"));
        }
        Ok(())
    }
}

pub(crate) async fn prepare_control_offer(
    connection: &(impl AuthenticatedControl + ?Sized),
    purpose: SessionPurpose,
    local: RuntimeCapabilities,
) -> io::Result<ControlOffer> {
    let role = AttemptRole::initial(connection);
    local.validate(role)?;
    let exchange = if connection.side().is_client() {
        connection.open_control_exchange().await?
    } else {
        connection.accept_control_exchange().await?
    };
    let nonce: [u8; 16] = rand::random();
    let mut frame = [0; FRAME_LEN];
    frame[..4].copy_from_slice(b"MUC1");
    frame[4..11].copy_from_slice(&[
        1,
        purpose as u8,
        local.runtime as u8,
        role as u8,
        local.supported,
        local.permitted,
        local.rtc_role as u8,
    ]);
    frame[12..].copy_from_slice(&nonce);
    exchange
        .send_message(
            ControlMessageFormat::LengthPrefixed16 { maximum: FRAME_LEN },
            &frame,
        )
        .await?;
    let length = exchange
        .recv_message(ControlMessageFormat::Fixed { length: 2 })
        .await?;
    if usize::from(u16::from_be_bytes(length.try_into().unwrap())) != FRAME_LEN {
        return Err(control_error("InvalidControl"));
    }
    let peer: [u8; FRAME_LEN] = exchange
        .recv_message(ControlMessageFormat::Fixed { length: FRAME_LEN })
        .await?
        .try_into()
        .unwrap();
    if &peer[..4] != b"MUC1" || peer[4] != 1 {
        return Err(control_error("ProtocolUnsupported"));
    }
    if peer[5] != frame[5] || peer[7] != role.opposite() as u8 || peer[11] != 0 {
        return Err(control_error("InvalidControl"));
    }
    let peer_runtime = match peer[6] {
        1 => Platform::Native,
        2 => Platform::Browser,
        _ => return Err(control_error("InvalidControl")),
    };
    let peer_rtc = match peer[10] {
        0 => RtcRole::None,
        1 => RtcRole::Offerer,
        2 => RtcRole::Answerer,
        _ => return Err(control_error("InvalidControl")),
    };
    RuntimeCapabilities {
        runtime: peer_runtime,
        supported: peer[8],
        permitted: peer[9],
        rtc_role: peer_rtc,
    }
    .validate(role.opposite())?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"miopunch/unified-attempt/1");
    if role == AttemptRole::Initiator {
        hasher.update(&nonce);
        hasher.update(&peer[12..]);
    } else {
        hasher.update(&peer[12..]);
        hasher.update(&nonce);
    }
    let id: [u8; 16] = hasher.finalize().as_bytes()[..16].try_into().unwrap();
    Ok(ControlOffer {
        exchange,
        nonce,
        peer,
        attempt: ProtectedAttempt {
            id,
            role,
            rtc_role: local.rtc_role,
            local_runtime: local.runtime,
            peer_runtime,
            mutual: local.permitted & peer[9],
        },
    })
}

pub(crate) struct PreparedAdmission {
    purpose: SessionPurpose,
    original: usize,
    original_binding: [u8; 32],
    revision: u64,
    local: RuntimeCapabilities,
    offer: ControlOffer,
    prepared: Option<PreparedPath>,
    cancellation: PendingMember,
}
impl PreparedAdmission {
    pub(crate) fn path_id(
        &self,
        connection: &(impl AuthenticatedControl + ?Sized),
    ) -> io::Result<iroh::endpoint::PathId> {
        let prepared = self
            .prepared
            .as_ref()
            .ok_or_else(|| control_error("DataPathUnavailable"))?;
        connection
            .paths()
            .iter()
            .find(|path| {
                path.remote_addr() == &prepared.route
                    && prepared
                        .path_id
                        .as_ref()
                        .is_none_or(|id| path.id().to_string() == *id)
            })
            .map(|path| path.id())
            .ok_or_else(|| control_error("DataPathUnavailable"))
    }
    pub(crate) async fn confirm<C: AuthenticatedControl + ?Sized, A: MemberPathAdapter<C>>(
        mut self,
        connection: &C,
        adapter: &A,
    ) -> io::Result<()> {
        let Self {
            purpose,
            original,
            original_binding,
            revision,
            local,
            offer,
            ..
        } = &self;
        let (purpose, original, original_binding, revision, local) =
            (*purpose, *original, *original_binding, *revision, *local);
        let peer = offer.peer;
        let attempt = offer.attempt;
        let peer_runtime = attempt.peer_runtime;
        let role = attempt.role;
        adapter.check(connection, purpose).await?;
        if adapter.authorization_revision(connection, purpose)? != revision
            || connection.stable_id() != original
            || crate::connection_recovery_core::binding(connection)? != original_binding
        {
            return Err(control_error("IdentityOrAdmissionRejected"));
        }
        let offer = self.offer;
        if purpose == SessionPurpose::AdmissionRequest {
            offer.confirm().await?;
            adapter.check(connection, purpose).await?;
            if adapter.authorization_revision(connection, purpose)? != revision
                || connection.stable_id() != original
                || crate::connection_recovery_core::binding(connection)? != original_binding
            {
                return Err(control_error("IdentityOrAdmissionRejected"));
            }
            let report = serde_json::json!({"event":"unified-control-ready","version":1,
                "purpose":purpose.label(),"purposeCode":purpose as u8,"stableId":original,"restricted":true,
                "role":if role==AttemptRole::Initiator{"initiator"}else{"responder"},
                "runtime":format!("{:?}",local.runtime),"peerRuntime":format!("{:?}",peer_runtime),
                "supported":local.supported,"permitted":local.permitted,"peerSupported":peer[8],"peerPermitted":peer[9],
                "ordinaryHandle":false,"dataReleased":false,"freshAck":null,
                "stableUnchanged":true,"bindingUnchanged":true,"sourceRevision":revision});
            adapter.committed(connection, report.clone());
            println!("{report}");
            self.cancellation.0 = None;
            return Ok(());
        }
        let mut prepared = self
            .prepared
            .take()
            .ok_or_else(|| control_error("DataPathUnavailable"))?;
        offer.confirm().await?;
        adapter.check(connection, purpose).await?;
        if adapter.authorization_revision(connection, purpose)? != revision {
            return Err(control_error("IdentityOrAdmissionRejected"));
        }
        prepared.check(connection, true)?;
        if let Some(retain) = prepared.retain.take() {
            retain().await?;
        }
        adapter.check(connection, purpose).await?;
        if adapter.authorization_revision(connection, purpose)? != revision {
            return Err(control_error("IdentityOrAdmissionRejected"));
        }
        prepared.check(connection, true)?;
        let paths = connection.paths();
        let report = serde_json::json!({"event":"unified-path-ready","version":1,"purpose":purpose.label(),"purposeCode":purpose as u8,"stableId":original,
            "stableUnchanged":connection.stable_id()==original,"bindingUnchanged":crate::connection_recovery_core::binding(connection)?==original_binding,
            "runtime":format!("{:?}",local.runtime),"peerRuntime":format!("{:?}",peer_runtime),
            "supported":local.supported,"permitted":local.permitted,"peerSupported":peer[8],"peerPermitted":peer[9],
            "role":if role==AttemptRole::Initiator{"initiator"}else{"responder"},"rtcRole":format!("{:?}",attempt.rtc_role),
            "freshAck":true,"route":prepared.route.to_string(),"pathId":prepared.path_id,
            "selected":paths.iter().any(|path|path.remote_addr()==&prepared.route && path.is_selected()),
            "selectedRoute":paths.iter().find(|path|path.is_selected()).map(|path|path.remote_addr().to_string()),
            "probeWatermark":prepared.watermark,"authorizationRevision":revision});
        adapter.committed(connection, report.clone());
        println!("{report}");
        prepared.rollback = None;
        self.cancellation.0 = None;
        Ok(())
    }
}
pub(crate) async fn prepare_control_session<
    C: AuthenticatedControl + ?Sized,
    A: MemberPathAdapter<C>,
>(
    endpoint: Option<&Endpoint>,
    connection: &C,
    adapter: &A,
) -> io::Result<PreparedAdmission> {
    let purpose = SessionPurpose::from_alpn(connection.alpn())
        .ok_or_else(|| control_error("ProtocolUnsupported"))?;
    if purpose != SessionPurpose::MemberBusiness && connection.caller_deadline().is_none() {
        return Err(control_error("InvalidControl"));
    }
    let original = connection.stable_id();
    let original_binding = crate::connection_recovery_core::binding(connection)?;
    let cancellation = PendingMember(Some(connection.revocation_handle()));
    adapter.check(connection, purpose).await?;
    let revision = adapter.authorization_revision(connection, purpose)?;
    let role = AttemptRole::initial(connection);
    let mut local = adapter.capabilities_for(connection, role);
    if purpose == SessionPurpose::AdmissionRequest {
        // Actual support remains visible, but A permits no data carrier.
        local.permitted = 0;
    }
    let offer = prepare_control_offer(connection, purpose, local).await?;
    let peer = offer.peer;
    let attempt = offer.attempt;
    if purpose == SessionPurpose::AdmissionRequest {
        if peer[9] != 0 {
            return Err(control_error("InvalidControl"));
        }
        return Ok(PreparedAdmission {
            purpose,
            original,
            original_binding,
            revision,
            local,
            offer,
            prepared: None,
            cancellation,
        });
    }
    let direct = adapter.existing_direct(connection);
    let relay = adapter.existing_relay(connection);
    let plans = attempt.plans(connection, direct.as_ref(), relay.as_ref())?;
    let mut prepared = None;
    for plan in plans {
        let candidate = match plan {
            PathPlan::ExistingDirect => Ok(PreparedPath::candidate(
                connection,
                direct
                    .as_ref()
                    .ok_or_else(|| control_error("DataPathUnavailable"))?
                    .0
                    .clone(),
                || Ok(()),
            )),
            _ => adapter.prepare(endpoint, connection, attempt, plan).await,
        };
        match candidate {
            Ok(candidate) => {
                prepared = Some(candidate);
                break;
            }
            Err(error) => {
                adapter.check(connection, purpose).await?;
                if adapter.authorization_revision(connection, purpose)? != revision {
                    return Err(control_error("IdentityOrAdmissionRejected"));
                }
                if error.to_string().starts_with("unified/InvalidControl")
                    || error.to_string().starts_with("unified/ProtocolUnsupported")
                    || error
                        .to_string()
                        .starts_with("unified/IdentityOrAdmissionRejected")
                {
                    return Err(error);
                }
                // The original absolute context remains in force for the next
                // permitted candidate; failed RTC does not mark its node offline.
                eprintln!("UNIFIED_CANDIDATE_FAILED plan={plan:?} error={error}");
            }
        }
    }
    let mut prepared = prepared.ok_or_else(|| control_error("DataPathUnavailable"))?;
    prepared.check(connection, false)?;
    if prepared.acknowledged.is_none() {
        let paths = connection.paths();
        let path = paths
            .iter()
            .find(|path| {
                path.remote_addr() == &prepared.route
                    && prepared
                        .path_id
                        .as_ref()
                        .is_none_or(|id| path.id().to_string() == *id)
            })
            .ok_or_else(|| control_error("DataPathUnavailable"))?;
        let probe = path.start_probe()?;
        prepared.path_id = Some(path.id().to_string());
        prepared.watermark = probe.watermark();
        prepared.acknowledged = Some(Box::new(move || {
            probe.is_acknowledged().map_err(io::Error::other)
        }));
    }
    loop {
        adapter.check(connection, purpose).await?;
        if adapter.authorization_revision(connection, purpose)? != revision {
            return Err(control_error("IdentityOrAdmissionRejected"));
        }
        prepared.check(connection, false)?;
        if matches!(
            prepared.acknowledged.as_ref().map(|ack| ack()),
            Some(Ok(true))
        ) {
            break;
        }
        n0_future::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    Ok(PreparedAdmission {
        purpose,
        original,
        original_binding,
        revision,
        local,
        offer,
        prepared: Some(prepared),
        cancellation,
    })
}
pub(crate) async fn coordinate_session<A: MemberPathAdapter>(
    endpoint: Option<&Endpoint>,
    connection: &Connection,
    adapter: &A,
) -> io::Result<()> {
    let Some(purpose) = SessionPurpose::from_alpn(connection.alpn()) else {
        return Ok(());
    };
    let operation = async {
        prepare_control_session(endpoint, connection, adapter)
            .await?
            .confirm(connection, adapter)
            .await
    };
    let result = if let Some(deadline) = connection.caller_deadline() {
        n0_future::time::timeout(
            deadline.saturating_duration_since(n0_future::time::Instant::now()),
            operation,
        )
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "unified caller deadline exceeded"))?
    } else {
        operation.await
    };
    if result.is_ok() && purpose == SessionPurpose::MemberBusiness {
        adapter.register_original(connection);
    }
    result
}

const ACTIVE_MAGIC: &[u8; 4] = b"MIA1";
const ACTIVE_VERSION: u8 = 1;
const ACTIVE_HELLO_KIND: u8 = 1;
const ACTIVE_READY_KIND: u8 = 2;
const ACTIVE_COMMITTED_KIND: u8 = 3;
// magic4 + version1 + kind1 + role1 + runtime1 + supported1 + permitted1
// + rtc-role1 + nonce16 + exporter binding32.
const ACTIVE_HELLO_LEN: usize = 59;
const ACTIVE_READY_LEN: usize = 30;
const ACTIVE_MAX_FRAME: usize = 96;

#[derive(Clone, Copy)]
struct ActiveHello {
    role: AttemptRole,
    runtime: Platform,
    supported: u8,
    permitted: u8,
    rtc_role: RtcRole,
    nonce: [u8; 16],
    binding: [u8; 32],
}

fn active_hello(
    capabilities: RuntimeCapabilities,
    role: AttemptRole,
    binding: [u8; 32],
) -> ActiveHello {
    ActiveHello {
        role,
        runtime: capabilities.runtime,
        supported: capabilities.supported,
        permitted: capabilities.permitted,
        rtc_role: capabilities.rtc_role,
        nonce: rand::random(),
        binding,
    }
}

fn encode_active_hello(hello: ActiveHello) -> [u8; ACTIVE_HELLO_LEN] {
    let mut frame = [0; ACTIVE_HELLO_LEN];
    frame[..4].copy_from_slice(ACTIVE_MAGIC);
    frame[4] = ACTIVE_VERSION;
    frame[5] = ACTIVE_HELLO_KIND;
    frame[6] = hello.role as u8;
    frame[7] = hello.runtime as u8;
    frame[8] = hello.supported;
    frame[9] = hello.permitted;
    frame[10] = hello.rtc_role as u8;
    frame[11..27].copy_from_slice(&hello.nonce);
    frame[27..59].copy_from_slice(&hello.binding);
    frame
}

fn decode_active_hello(frame: &[u8]) -> io::Result<ActiveHello> {
    if frame.len() != ACTIVE_HELLO_LEN
        || &frame[..4] != ACTIVE_MAGIC
        || frame[4] != ACTIVE_VERSION
        || frame[5] != ACTIVE_HELLO_KIND
    {
        return Err(control_error("ProtocolUnsupported"));
    }
    let role = match frame[6] {
        1 => AttemptRole::Initiator,
        2 => AttemptRole::Responder,
        _ => return Err(control_error("InvalidControl")),
    };
    let runtime = match frame[7] {
        1 => Platform::Native,
        2 => Platform::Browser,
        _ => return Err(control_error("InvalidControl")),
    };
    let rtc_role = match frame[10] {
        0 => RtcRole::None,
        1 => RtcRole::Offerer,
        2 => RtcRole::Answerer,
        _ => return Err(control_error("InvalidControl")),
    };
    Ok(ActiveHello {
        role,
        runtime,
        supported: frame[8],
        permitted: frame[9],
        rtc_role,
        nonce: frame[11..27].try_into().unwrap(),
        binding: frame[27..59].try_into().unwrap(),
    })
}

#[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
fn observe_actual_active_frame(
    stage: &'static str,
    original: usize,
    control: usize,
    stream: String,
    frame: &[u8],
) {
    // Owned fixture stdout is PRIVATE. This observes SDK bytes; it never gates IO.
    println!(
        "{}",
        serde_json::json!({"event":"actual-common-active-frame",
        "stage":stage,"originalStableId":original,"controlStableId":control,
        "streamId":stream,"kind":frame[5],"frameLength":frame.len(),
        "frameBlake3":blake3::hash(frame).to_hex().to_string(),
        "attemptBlake3":blake3::hash(&frame[6..22]).to_hex().to_string(),
        "monotonicMs":crate::rtc_transport::diagnostic_monotonic_ms()})
    );
}

async fn write_active_frame(
    send: &ControlExchange,
    kind: u8,
    attempt: [u8; 16],
    watermark: u64,
    _original_id: usize,
    _control_id: usize,
) -> io::Result<()> {
    let mut frame = [0; ACTIVE_READY_LEN];
    frame[..4].copy_from_slice(ACTIVE_MAGIC);
    frame[4] = ACTIVE_VERSION;
    frame[5] = kind;
    frame[6..22].copy_from_slice(&attempt);
    frame[22..].copy_from_slice(&watermark.to_be_bytes());
    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    observe_actual_active_frame(
        "sdk-frame-generated",
        _original_id,
        _control_id,
        send.id().to_string(),
        &frame,
    );
    send.send_message(
        ControlMessageFormat::LengthPrefixed16 {
            maximum: frame.len(),
        },
        &frame,
    )
    .await?;
    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    observe_actual_active_frame(
        "quic-stream-write-accepted",
        _original_id,
        _control_id,
        send.id().to_string(),
        &frame,
    );
    Ok(())
}

async fn read_active_frame<C: AuthenticatedControl + ?Sized, A: MemberPathAdapter<C>>(
    recv: &ControlExchange,
    expected_kind: u8,
    attempt: [u8; 16],
    _original_id: usize,
    _control_id: usize,
    _adapter: &A,
    _deadline: n0_future::time::Instant,
) -> io::Result<u64> {
    let frame = read_active_message(recv, ACTIVE_READY_LEN).await?;
    if &frame[..4] != ACTIVE_MAGIC
        || frame[4] != ACTIVE_VERSION
        || frame[5] != expected_kind
        || frame[6..22] != attempt
    {
        return Err(control_error("InvalidControl"));
    }
    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    if expected_kind == ACTIVE_READY_KIND {
        _adapter
            .after_actual_ready_input_fixture(
                _original_id,
                _control_id,
                recv.id().to_string(),
                &frame,
                _deadline,
            )
            .await?;
    }
    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    observe_actual_active_frame(
        "peer-frame-parsed-and-attempt-validated",
        _original_id,
        _control_id,
        recv.id().to_string(),
        &frame,
    );
    Ok(u64::from_be_bytes(frame[22..].try_into().unwrap()))
}

async fn write_active_hello(send: &ControlExchange, hello: ActiveHello) -> io::Result<()> {
    let frame = encode_active_hello(hello);
    send.send_message(
        ControlMessageFormat::LengthPrefixed16 {
            maximum: frame.len(),
        },
        &frame,
    )
    .await?;
    send.finish_send().await?;
    Ok(())
}

async fn read_active_message(exchange: &ControlExchange, length: usize) -> io::Result<Vec<u8>> {
    let header = exchange
        .recv_message(ControlMessageFormat::Fixed { length: 2 })
        .await?;
    if usize::from(u16::from_be_bytes(header.try_into().unwrap())) != length {
        return Err(control_error("InvalidControl"));
    }
    exchange
        .recv_message(ControlMessageFormat::Fixed { length })
        .await
}
async fn read_active_hello(recv: &ControlExchange) -> io::Result<ActiveHello> {
    decode_active_hello(&read_active_message(recv, ACTIVE_HELLO_LEN).await?)
}

impl ProtectedAttempt {
    fn improvement_plans(self) -> Vec<PathPlan> {
        if self.local_runtime == Platform::Native && self.peer_runtime == Platform::Native {
            if self.mutual & (UDP | TCP) != 0 {
                vec![PathPlan::Native {
                    tcp: self.mutual & TCP != 0,
                    udp: self.mutual & UDP != 0,
                }]
            } else {
                // Native↔native uses its accepted UDP/TCP path family here;
                // RTC is reserved for mixed runtime pairs in this slice.
                Vec::new()
            }
        } else if self.mutual & RTC != 0 {
            vec![PathPlan::Rtc]
        } else {
            Vec::new()
        }
    }
}

async fn open_active_stream(
    control: &(impl AuthenticatedControl + ?Sized),
) -> io::Result<ControlExchange> {
    if control.side().is_client() {
        control.open_control_exchange().await
    } else {
        control.accept_control_exchange().await
    }
}

/// The original business owner can narrow M while restricted C stays confined.
pub(crate) trait ActiveEndpoint: Sync {
    type Session: AuthenticatedControl + Clone;
    fn active_session(&self, original: &Connection) -> Self::Session;
    fn ordinary_endpoint(&self) -> Option<&Endpoint>;
}
impl ActiveEndpoint for Endpoint {
    type Session = Connection;
    fn active_session(&self, original: &Connection) -> Connection {
        original.clone()
    }
    fn ordinary_endpoint(&self) -> Option<&Endpoint> {
        Some(self)
    }
}
impl ActiveEndpoint for iroh::endpoint::RestrictedEndpoint {
    type Session = iroh::endpoint::ControlHandle;
    fn active_session(&self, original: &Connection) -> Self::Session {
        self.control_handle(original)
    }
    fn ordinary_endpoint(&self) -> Option<&Endpoint> {
        None
    }
}

async fn active_session<C: AuthenticatedControl + Clone, A: MemberPathAdapter<C>>(
    endpoint: Option<&Endpoint>,
    original: C,
    control: &C,
    adapter: &A,
    attempt: AttemptContext,
    local: ActiveHello,
    peer_hello: ActiveHello,
) -> io::Result<()> {
    // The active control initiator/responder owns RTC offer/answer roles;
    // `ProtectedAttempt::quic_dialer` still derives the original M's QUIC
    // dialing side independently.
    let role = AttemptRole::initial(control);
    let original_binding = crate::connection_recovery_core::binding(&original)?;
    if peer_hello.role != role.opposite()
        || local.role != role
        || local.binding != original_binding
        || peer_hello.binding != original_binding
        || control.remote_id() != original.remote_id()
    {
        return Err(control_error("IdentityOrAdmissionRejected"));
    }
    RuntimeCapabilities {
        runtime: local.runtime,
        supported: local.supported,
        permitted: local.permitted,
        rtc_role: local.rtc_role,
    }
    .validate(role)?;
    RuntimeCapabilities {
        runtime: peer_hello.runtime,
        supported: peer_hello.supported,
        permitted: peer_hello.permitted,
        rtc_role: peer_hello.rtc_role,
    }
    .validate(peer_hello.role)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"miopunch/active-improvement/1");
    if role == AttemptRole::Initiator {
        hasher.update(&local.nonce);
        hasher.update(&peer_hello.nonce);
    } else {
        hasher.update(&peer_hello.nonce);
        hasher.update(&local.nonce);
    }
    let id: [u8; 16] = hasher.finalize().as_bytes()[..16].try_into().unwrap();
    let protected = ProtectedAttempt {
        id,
        role,
        rtc_role: local.rtc_role,
        local_runtime: local.runtime,
        peer_runtime: peer_hello.runtime,
        mutual: local.permitted & peer_hello.permitted,
    };
    adapter
        .check(&original, SessionPurpose::MemberBusiness)
        .await?;
    adapter
        .check(control, SessionPurpose::ActiveImprovement)
        .await?;
    let revision = adapter.authorization_revision(&original, SessionPurpose::MemberBusiness)?;
    if adapter.authorization_revision(&original, SessionPurpose::MemberBusiness)? != revision {
        return Err(control_error("IdentityOrAdmissionRejected"));
    }
    let mut prepared = None;
    for plan in protected.improvement_plans() {
        match adapter
            .prepare_active(endpoint, &original, control, protected, plan)
            .await
        {
            Ok(candidate) => {
                prepared = Some(candidate);
                break;
            }
            Err(error) => {
                adapter
                    .check(&original, SessionPurpose::MemberBusiness)
                    .await?;
                adapter
                    .check(control, SessionPurpose::ActiveImprovement)
                    .await?;
                if error
                    .to_string()
                    .starts_with("unified/IdentityOrAdmissionRejected")
                    || error.to_string().starts_with("unified/InvalidControl")
                {
                    return Err(error);
                }
            }
        }
    }
    let mut prepared = prepared.ok_or_else(|| control_error("DataPathUnavailable"))?;
    prepared.check(&original, false)?;

    // The first C stream is owned by the adapter's bounded candidate protocol.
    // A second strictly framed stream carries the bilateral commit barrier.
    let ready = n0_future::time::timeout(
        attempt
            .deadline
            .saturating_duration_since(n0_future::time::Instant::now()),
        open_active_stream(control),
    )
    .await
    .map_err(|_| control_error("DeadlineExceeded"))??;
    write_active_frame(
        &ready,
        ACTIVE_READY_KIND,
        protected.id,
        prepared.watermark,
        original.stable_id(),
        control.stable_id(),
    )
    .await?;
    let _peer_watermark = n0_future::time::timeout(
        attempt
            .deadline
            .saturating_duration_since(n0_future::time::Instant::now()),
        read_active_frame::<C, A>(
            &ready,
            ACTIVE_READY_KIND,
            protected.id,
            original.stable_id(),
            control.stable_id(),
            adapter,
            attempt.deadline,
        ),
    )
    .await
    .map_err(|_| control_error("DeadlineExceeded"))??;
    adapter
        .check(&original, SessionPurpose::MemberBusiness)
        .await?;
    adapter
        .check(control, SessionPurpose::ActiveImprovement)
        .await?;
    if adapter.authorization_revision(&original, SessionPurpose::MemberBusiness)? != revision {
        return Err(control_error("IdentityOrAdmissionRejected"));
    }
    prepared.check(&original, true)?;
    if let Some(retain) = prepared.retain.take() {
        retain().await?;
    }
    prepared.check(&original, true)?;
    write_active_frame(
        &ready,
        ACTIVE_COMMITTED_KIND,
        protected.id,
        prepared.watermark,
        original.stable_id(),
        control.stable_id(),
    )
    .await?;
    let _peer_final = n0_future::time::timeout(
        attempt
            .deadline
            .saturating_duration_since(n0_future::time::Instant::now()),
        read_active_frame::<C, A>(
            &ready,
            ACTIVE_COMMITTED_KIND,
            protected.id,
            original.stable_id(),
            control.stable_id(),
            adapter,
            attempt.deadline,
        ),
    )
    .await
    .map_err(|_| control_error("DeadlineExceeded"))??;
    adapter.active_diagnostic(&original, control, "peer-committed-observed");
    adapter
        .check(&original, SessionPurpose::MemberBusiness)
        .await?;
    adapter.active_diagnostic(&original, control, "post-committed-original-checked");
    // Peer COMMITTED completes C's protocol role. Its peer may now close C;
    // only current original-M authorization and candidate proof govern commit.
    if adapter.authorization_revision(&original, SessionPurpose::MemberBusiness)? != revision {
        return Err(control_error("IdentityOrAdmissionRejected"));
    }
    prepared.check(&original, true)?;
    prepared.rollback = None;
    adapter.active_diagnostic(&original, control, "rollback-disarmed");
    // Terminal stream cleanup is best effort after bilateral qualified commit.
    // A completed peer C cannot retire the retained original-M candidate.
    if ready.finish_send().await.is_err() {
        adapter.active_diagnostic(&original, control, "post-committed-send-finish-failed");
    }
    adapter.committed(
        &original,
        serde_json::json!({"event":"active-improvement-committed","version":1,
            "stableId":original.stable_id(),"controlStableId":control.stable_id(),
            "freshAck":true,"bothReady":true,
            "authorizationRevision":revision}),
    );
    Ok(())
}

/// Run the active improvement protocol on an already connected restricted C.
/// The original M is retained independently; C carries only bounded control.
pub(crate) async fn improve_existing<E: ActiveEndpoint, A: MemberPathAdapter<E::Session>>(
    endpoint: &E,
    original: &Connection,
    control: &E::Session,
    adapter: &A,
    attempt: AttemptContext,
) -> io::Result<()> {
    if original.remote_id() != control.remote_id() || original.close_reason().is_some() {
        return Err(control_error("DataPathUnavailable"));
    }
    let binding = crate::connection_recovery_core::binding(original)?;
    let role = AttemptRole::initial(control);
    let original = endpoint.active_session(original);
    let local = active_hello(adapter.capabilities_for(&original, role), role, binding);
    let exchange = n0_future::time::timeout(
        attempt
            .deadline
            .saturating_duration_since(n0_future::time::Instant::now()),
        open_active_stream(control),
    )
    .await
    .map_err(|_| control_error("DeadlineExceeded"))??;
    write_active_hello(&exchange, local).await?;
    let peer = n0_future::time::timeout(
        attempt
            .deadline
            .saturating_duration_since(n0_future::time::Instant::now()),
        read_active_hello(&exchange),
    )
    .await
    .map_err(|_| control_error("DeadlineExceeded"))??;
    active_session(
        endpoint.ordinary_endpoint(),
        original,
        control,
        adapter,
        attempt,
        local,
        peer,
    )
    .await
}

/// Fault fixture using the real active hello framing on an authenticated C.
/// A deliberately different original exporter must fail before candidate setup.
#[cfg(feature = "rtc-probe")]
pub(crate) async fn wrong_active_binding_fixture(
    original: &Connection,
    control: &(impl AuthenticatedControl + ?Sized),
    caps: RuntimeCapabilities,
    deadline: n0_future::time::Instant,
) -> io::Result<io::Result<()>> {
    if original.close_reason().is_some()
        || control.close_reason().is_some()
        || control.remote_id() != original.remote_id()
        || control.alpn() != ACTIVE_IMPROVEMENT_ALPN
    {
        return Err(control_error("InvalidControl"));
    }
    let role = AttemptRole::initial(control);
    let mut binding = crate::connection_recovery_core::binding(original)?;
    binding[0] ^= 1;
    n0_future::time::timeout(
        deadline.saturating_duration_since(n0_future::time::Instant::now()),
        async {
            let exchange = open_active_stream(control).await?;
            write_active_hello(&exchange, active_hello(caps, role, binding)).await?;
            // Outer success proves the exact faulty hello was accepted by the
            // actual QUIC stream write; the inner result observes the response.
            Ok(read_active_hello(&exchange).await.map(|_| ()))
        },
    )
    .await
    .map_err(|_| control_error("DeadlineExceeded"))?
}

/// Server-side active-control entry. The hello's exporter binding selects the
/// exact original M from the existing weak registry/gate lookup.
pub(crate) async fn serve_active_session<E, A, F>(
    endpoint: &E,
    control: &E::Session,
    adapter: &A,
    registry: &crate::session_maintenance::SessionMaintenance,
    responder_deadline: n0_future::time::Instant,
    lookup: F,
) -> io::Result<()>
where
    E: ActiveEndpoint,
    A: MemberPathAdapter<E::Session>,
    F: Fn(iroh::EndpointId, &[u8; 32]) -> Option<Connection>,
{
    let deadline = control
        .caller_deadline()
        .map(|deadline| deadline.min(responder_deadline))
        .unwrap_or(responder_deadline);
    let exchange = n0_future::time::timeout(
        deadline.saturating_duration_since(n0_future::time::Instant::now()),
        open_active_stream(control),
    )
    .await
    .map_err(|_| control_error("DeadlineExceeded"))??;
    let peer = n0_future::time::timeout(
        deadline.saturating_duration_since(n0_future::time::Instant::now()),
        read_active_hello(&exchange),
    )
    .await
    .map_err(|_| control_error("DeadlineExceeded"))??;
    let original = lookup(control.remote_id(), &peer.binding)
        .ok_or_else(|| control_error("DataPathUnavailable"))?;
    let remaining = deadline;
    let attempt = AttemptContext::new(remaining, tokio_util::sync::CancellationToken::new());
    let role = AttemptRole::initial(control);
    let local = active_hello(
        adapter.capabilities_for(&endpoint.active_session(&original), role),
        role,
        peer.binding,
    );
    write_active_hello(&exchange, local).await?;
    let lease = registry
        .acquire(&original, &attempt, ())
        .map_err(|error| control_error(&format!("active/{error:?}")))?;
    lease
        .run(active_session(
            endpoint.ordinary_endpoint(),
            endpoint.active_session(&original),
            control,
            adapter,
            attempt,
            local,
            peer,
        ))
        .await
        .map_err(|error| control_error(&format!("active/{error:?}")))?
}

#[cfg(test)]
mod active_wire_tests {
    use super::*;

    #[test]
    fn bounded_active_hello_round_trips_and_rejects_mutation() {
        let hello = active_hello(
            RuntimeCapabilities {
                runtime: Platform::Browser,
                supported: RELAY | RTC,
                permitted: RTC,
                rtc_role: RtcRole::Offerer,
            },
            AttemptRole::Initiator,
            [7; 32],
        );
        let encoded = encode_active_hello(hello);
        assert_eq!(encoded.len(), ACTIVE_HELLO_LEN);
        let decoded = decode_active_hello(&encoded).unwrap();
        assert_eq!(decoded.role, hello.role);
        assert_eq!(decoded.runtime, hello.runtime);
        assert_eq!(decoded.supported, hello.supported);
        assert_eq!(decoded.permitted, hello.permitted);
        assert_eq!(decoded.rtc_role, hello.rtc_role);
        assert_eq!(decoded.nonce, hello.nonce);
        assert_eq!(decoded.binding, hello.binding);

        let mut wrong_kind = encoded;
        wrong_kind[5] = ACTIVE_READY_KIND;
        assert!(decode_active_hello(&wrong_kind).is_err());
        assert!(decode_active_hello(&wrong_kind[..ACTIVE_HELLO_LEN - 1]).is_err());
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[path = "connection_coordinator_native.rs"]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::{
    AdmissionBeforeConnect, CoordinatorHook, NativeAdmission, NativeControlAdapter,
};

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "connection_coordinator_active_tests.rs"]
mod active_commit_tests;
