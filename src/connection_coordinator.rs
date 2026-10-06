//! Shared strict member connection coordination. Platform adapters own their
//! gates and candidate resources; this module owns negotiation and release.
use futures_util::FutureExt;
use iroh::{Endpoint, TransportAddr, endpoint::Connection};
use std::{future::Future, io, pin::Pin};

pub const MEMBER_ALPN: &[u8] = b"miopunch/session/member/1";
pub const ADMISSION_REQUEST_ALPN: &[u8] = b"miopunch/session/admission/1";
pub const PRECODE_REQUEST_ALPN: &[u8] = b"miopunch/session/precode/1";
pub const REPLICA_ALPN: &[u8] = b"miopunch/session/replica/1";
const FRAME_LEN: usize = 28;
pub(crate) const UDP: u8 = 1;
pub(crate) const TCP: u8 = 2;
pub(crate) const RELAY: u8 = 4;
pub(crate) const RTC: u8 = 8;
const CAPABILITIES: u8 = UDP | TCP | RELAY | RTC;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SessionPurpose {
    MemberBusiness = 1,
    AdmissionRequest = 2,
    Replica = 3,
}
impl SessionPurpose {
    pub fn alpn(self) -> &'static [u8] {
        match self {
            Self::MemberBusiness => MEMBER_ALPN,
            Self::AdmissionRequest => ADMISSION_REQUEST_ALPN,
            Self::Replica => REPLICA_ALPN,
        }
    }
    pub fn from_alpn(alpn: &[u8]) -> Option<Self> {
        match alpn {
            MEMBER_ALPN => Some(Self::MemberBusiness),
            ADMISSION_REQUEST_ALPN | PRECODE_REQUEST_ALPN => Some(Self::AdmissionRequest),
            REPLICA_ALPN => Some(Self::Replica),
            _ => None,
        }
    }
    pub(crate) fn requires_data(self) -> bool {
        self != Self::AdmissionRequest
    }
    fn label(self) -> &'static str {
        match self {
            Self::MemberBusiness => "member-business",
            Self::AdmissionRequest => "admission-request",
            Self::Replica => "replica",
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
    pub(crate) fn initial(connection: &Connection) -> Self {
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
        if connection.side().is_client() {
            endpoint.id()
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
        connection: &Connection,
        direct: Option<&(TransportAddr, u8)>,
    ) -> io::Result<Vec<PathPlan>> {
        let paths = connection.paths();
        if direct.is_some_and(|(_, capability)| self.mutual & capability != 0) {
            return Ok(vec![PathPlan::ExistingDirect]);
        }
        if self.mutual & RELAY != 0 && paths.iter().any(|path| path.is_relay()) {
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
pub(crate) trait MemberPathAdapter {
    fn capabilities(&self, role: AttemptRole) -> RuntimeCapabilities;
    fn authorization_revision(
        &self,
        connection: &Connection,
        purpose: SessionPurpose,
    ) -> io::Result<u64>;
    fn committed(&self, _connection: &Connection, _report: serde_json::Value) {}
    fn existing_direct(&self, connection: &Connection) -> Option<(TransportAddr, u8)> {
        let paths = connection.paths();
        paths
            .iter()
            .filter(|path| path.is_ip())
            .max_by_key(|path| path.is_selected())
            .map(|path| (path.remote_addr().clone(), UDP))
    }
    fn check<'a>(
        &'a self,
        connection: &'a Connection,
        purpose: SessionPurpose,
    ) -> impl Future<Output = io::Result<()>> + Send + 'a;
    fn prepare<'a>(
        &'a self,
        endpoint: &'a Endpoint,
        connection: &'a Connection,
        attempt: ProtectedAttempt,
        plan: PathPlan,
    ) -> impl Future<Output = io::Result<PreparedPath>> + Send + 'a;
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
        connection: &Connection,
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
    fn check(&self, connection: &Connection, require_ack: bool) -> io::Result<()> {
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
            ConnectingError::LocallyRejected { .. } => Self::IdentityOrAdmissionRejected,
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
impl ContactDialFailure {
    fn into_error(self) -> ConnectError {
        match self {
            Self::Unavailable(error) | Self::Terminal(error) => error,
        }
    }
}
#[derive(Clone)]
pub struct ConnectionCoordinator {
    endpoint: Endpoint,
    permitted: u8,
    contact_ids: Vec<u64>,
    member_contact_id: Option<u64>,
}
impl ConnectionCoordinator {
    pub(crate) fn new(endpoint: Endpoint, permitted: u8, contact_ids: Vec<u64>) -> Self {
        Self {
            endpoint,
            permitted,
            contact_ids,
            member_contact_id: None,
        }
    }
    /// The native resource owner supplies the installed member contact carrier.
    /// Browser callers have no member adapter and retain None.
    pub(crate) fn with_member_contact(mut self, id: u64) -> Self {
        self.member_contact_id = Some(id);
        self
    }
    async fn dial(
        &self,
        target: iroh::EndpointAddr,
        purpose: SessionPurpose,
        attempt: &AttemptContext,
        wire_alpn: &[u8],
    ) -> Result<Connection, ContactDialFailure> {
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
        let connecting = self
            .endpoint
            .connect_with_opts(target, wire_alpn, options)
            .await
            .map_err(|error| match error {
                ConnectWithOptsError::LocallyRejected { .. } => {
                    ContactDialFailure::Terminal(ConnectError::IdentityOrAdmissionRejected)
                }
                ConnectWithOptsError::NoAddress { .. } => {
                    ContactDialFailure::Unavailable(ConnectError::Transport(error.to_string()))
                }
                _ => ContactDialFailure::Terminal(ConnectError::Transport(error.to_string())),
            })?;
        connecting.await.map_err(|error| {
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
    // Exact new selector, needed by the existing native precode entry.
    pub(crate) async fn connect_alpn(
        &self,
        mut target: iroh::EndpointAddr,
        wire_alpn: &[u8],
        attempt: AttemptContext,
    ) -> Result<Connection, ConnectError> {
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
                let mut last_unavailable = None;
                // A hint is not a usable data path. Trial each permitted data
                // route in this attempt's exact bootstrap scope; cached contact
                // routes cannot carry it before an actual setup failure.
                for route in target.addrs.iter().filter(|address| {
                    matches!(address, TransportAddr::Ip(_) | TransportAddr::Relay(_))
                }) {
                    let data = iroh::EndpointAddr::new(target.id).with_addrs([route.clone()]);
                    match self.dial(data, purpose, &attempt, wire_alpn).await {
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
                        match self.dial(member, purpose, &attempt, wire_alpn).await {
                            Ok(connection) => return Ok(connection),
                            Err(ContactDialFailure::Terminal(error)) => return Err(error),
                            Err(ContactDialFailure::Unavailable(error)) => {
                                last_unavailable = Some(error)
                            }
                        }
                    }
                }
                if public_contact.addrs.is_empty() {
                    return Err(last_unavailable.unwrap_or(ConnectError::DataPathUnavailable));
                }
                self.dial(public_contact, purpose, &attempt, wire_alpn)
                    .await
                    .map_err(ContactDialFailure::into_error)
            })
            .await
    }
    pub async fn accept(
        &self,
        incoming: iroh::endpoint::Incoming,
        purpose: SessionPurpose,
        attempt: AttemptContext,
    ) -> Result<Connection, ConnectError> {
        attempt
            .run(async {
                let accepting = incoming
                    .accept()
                    .map_err(|error| ConnectError::Transport(error.to_string()))?;
                self.accept_inner(accepting, purpose, &attempt).await
            })
            .await
    }

    /// Accept a pending connection already classified by negotiated ALPN. The
    /// same original context bounds metadata, hook execution and data release;
    /// classification does not grant identity, admission or a new budget.
    pub async fn accept_accepting(
        &self,
        accepting: iroh::endpoint::Accepting,
        purpose: SessionPurpose,
        attempt: AttemptContext,
    ) -> Result<Connection, ConnectError> {
        attempt
            .run(self.accept_inner(accepting, purpose, &attempt))
            .await
    }

    async fn accept_inner(
        &self,
        accepting: iroh::endpoint::Accepting,
        purpose: SessionPurpose,
        attempt: &AttemptContext,
    ) -> Result<Connection, ConnectError> {
        let mut accepting = accepting
            .with_caller_deadline(attempt.deadline)
            .with_attempt_cancellation(attempt.cancellation.clone());
        // ALPN inspection awaits the actual TLS negotiation without polling the
        // framework hook future. A mismatched purpose must allocate no candidate.
        let alpn = accepting
            .alpn()
            .await
            .map_err(|error| ConnectError::Transport(error.to_string()))?;
        if SessionPurpose::from_alpn(&alpn) != Some(purpose) {
            return Err(ConnectError::ProtocolUnsupported);
        }
        let connection = accepting.await.map_err(ConnectError::from_connecting)?;
        if SessionPurpose::from_alpn(connection.alpn()) != Some(purpose) {
            connection.close(0u32.into(), b"unified protocol required");
            return Err(ConnectError::ProtocolUnsupported);
        }
        Ok(connection)
    }
}

pub(crate) fn control_error(kind: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("unified/{kind}"))
}
struct PendingMember(Option<Connection>);
impl Drop for PendingMember {
    fn drop(&mut self) {
        if let Some(connection) = self.0.take() {
            connection.close(0u32.into(), b"unified pending member retired");
        }
    }
}

/// One shared protected protocol and release path on both platforms. The
/// endpoint's original admission hook still owns identity/proof verification.
async fn confirm_nonce(
    send: &mut iroh::endpoint::SendStream,
    recv: &mut iroh::endpoint::RecvStream,
    nonce: &[u8; 16],
    peer_nonce: &[u8],
) -> io::Result<()> {
    let mut ready = [0; 17];
    ready[0] = 1;
    ready[1..].copy_from_slice(peer_nonce);
    send.write_all(&ready).await?;
    send.finish()?;
    let ack = recv.read_to_end(17).await.map_err(io::Error::other)?;
    if ack.len() != 17 || ack[0] != 1 || &ack[1..] != nonce {
        return Err(control_error("InvalidControl"));
    }
    Ok(())
}

pub(crate) async fn coordinate_session<A: MemberPathAdapter>(
    endpoint: Option<&Endpoint>,
    connection: &Connection,
    adapter: &A,
) -> io::Result<()> {
    let Some(purpose) = SessionPurpose::from_alpn(connection.alpn()) else {
        return Ok(()); // unrelated, explicitly registered purposes remain separate
    };
    if purpose.requires_data() && endpoint.is_none() {
        return Err(control_error("InvalidControl"));
    }
    // Migrated callers supply this at their original public operation boundary.
    if purpose != SessionPurpose::MemberBusiness && connection.caller_deadline().is_none() {
        return Err(control_error("InvalidControl"));
    }
    let original = connection.stable_id();
    let original_binding = crate::connection_recovery_core::binding(connection)?;
    let mut cancellation = PendingMember(Some(connection.clone()));
    let operation = async {
        adapter.check(connection, purpose).await?;
        let revision = adapter.authorization_revision(connection, purpose)?;
        let role = AttemptRole::initial(connection);
        let mut local = adapter.capabilities(role);
        if purpose == SessionPurpose::AdmissionRequest {
            // Actual support remains visible, but A permits no data carrier.
            local.permitted = 0;
        }
        local.validate(role)?;
        let (mut send, mut recv) = if connection.side().is_client() {
            connection.open_bi().await?
        } else {
            connection.accept_bi().await?
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
        send.write_all(&(FRAME_LEN as u16).to_be_bytes()).await?;
        send.write_all(&frame).await?;
        let mut length = [0; 2];
        recv.read_exact(&mut length)
            .await
            .map_err(io::Error::other)?;
        if usize::from(u16::from_be_bytes(length)) != FRAME_LEN {
            return Err(control_error("InvalidControl"));
        }
        let mut peer = [0; FRAME_LEN];
        recv.read_exact(&mut peer).await.map_err(io::Error::other)?;
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
        let attempt = ProtectedAttempt {
            id,
            role,
            rtc_role: local.rtc_role,
            local_runtime: local.runtime,
            peer_runtime,
            mutual: local.permitted & peer[9],
        };
        if purpose == SessionPurpose::AdmissionRequest {
            if peer[9] != 0 {
                return Err(control_error("InvalidControl"));
            }
            confirm_nonce(&mut send, &mut recv, &nonce, &peer[12..]).await?;
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
            return Ok(());
        }
        let direct = adapter.existing_direct(connection);
        let plans = attempt.plans(connection, direct.as_ref())?;
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
                PathPlan::ExistingRelay => {
                    let paths = connection.paths();
                    let path = paths
                        .iter()
                        .filter(|path| path.is_relay())
                        .max_by_key(|path| path.is_selected())
                        .ok_or_else(|| control_error("DataPathUnavailable"))?;
                    Ok(PreparedPath::candidate(
                        connection,
                        path.remote_addr().clone(),
                        || Ok(()),
                    ))
                }
                _ => {
                    adapter
                        .prepare(
                            endpoint.ok_or_else(|| control_error("InvalidControl"))?,
                            connection,
                            attempt,
                            plan,
                        )
                        .await
                }
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
        confirm_nonce(&mut send, &mut recv, &nonce, &peer[12..]).await?;
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
        Ok(())
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
    if result.is_ok() {
        cancellation.0 = None;
    }
    result
}

#[cfg(not(target_arch = "wasm32"))]
#[path = "connection_coordinator_native.rs"]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::{CoordinatorHook, release_existing_data};
