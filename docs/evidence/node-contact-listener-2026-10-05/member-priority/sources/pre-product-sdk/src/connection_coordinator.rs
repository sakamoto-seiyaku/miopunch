//! Shared strict member connection coordination. Platform adapters own their
//! gates and candidate resources; this module owns negotiation and release.
use futures_util::FutureExt;
use iroh::{Endpoint, TransportAddr, endpoint::Connection};
use std::{future::Future, io, pin::Pin};

pub const MEMBER_ALPN: &[u8] = b"miopunch/session/member/1";
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
}
impl SessionPurpose {
    fn alpn(self) -> &'static [u8] {
        match self {
            Self::MemberBusiness => MEMBER_ALPN,
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
    fn authorization_revision(&self, connection: &Connection) -> io::Result<u64>;
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
#[derive(Clone)]
pub struct ConnectionCoordinator {
    endpoint: Endpoint,
    permitted: u8,
    contact_ids: Vec<u64>,
}
impl ConnectionCoordinator {
    pub(crate) fn new(endpoint: Endpoint, permitted: u8, contact_ids: Vec<u64>) -> Self {
        Self {
            endpoint,
            permitted,
            contact_ids,
        }
    }
    pub async fn connect(
        &self,
        mut target: iroh::EndpointAddr,
        purpose: SessionPurpose,
        attempt: AttemptContext,
    ) -> Result<Connection, ConnectError> {
        attempt
            .run(async {
                if self.permitted == 0 {
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
                let connecting = self
                    .endpoint
                    .connect_with_opts(
                        target,
                        purpose.alpn(),
                        iroh::endpoint::ConnectOptions::new()
                            .with_caller_deadline(attempt.deadline)
                            .with_attempt_cancellation(attempt.cancellation.clone()),
                    )
                    .await
                    .map_err(|error| match error {
                        iroh::endpoint::ConnectWithOptsError::LocallyRejected { .. } => {
                            ConnectError::IdentityOrAdmissionRejected
                        }
                        _ => ConnectError::Transport(error.to_string()),
                    })?;
                connecting.await.map_err(ConnectError::from_connecting)
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
                let connection = incoming
                    .with_caller_deadline(attempt.deadline)
                    .accept()
                    .map_err(|e| ConnectError::Transport(e.to_string()))?
                    .await
                    .map_err(ConnectError::from_connecting)?;
                if connection.alpn() != purpose.alpn() {
                    connection.close(0u32.into(), b"unified protocol required");
                    return Err(ConnectError::ProtocolUnsupported);
                }
                Ok(connection)
            })
            .await
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
pub(crate) async fn coordinate_member<A: MemberPathAdapter>(
    endpoint: &Endpoint,
    connection: &Connection,
    adapter: &A,
) -> io::Result<()> {
    if connection.alpn() != MEMBER_ALPN {
        return Ok(());
    }
    let mut cancellation = PendingMember(Some(connection.clone()));
    let operation = async {
        adapter.check(connection).await?;
        let revision = adapter.authorization_revision(connection)?;
        let role = AttemptRole::initial(connection);
        let local = adapter.capabilities(role);
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
            SessionPurpose::MemberBusiness as u8,
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
                _ => adapter.prepare(endpoint, connection, attempt, plan).await,
            };
            match candidate {
                Ok(candidate) => {
                    prepared = Some(candidate);
                    break;
                }
                Err(error) => {
                    adapter.check(connection).await?;
                    if adapter.authorization_revision(connection)? != revision {
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
            adapter.check(connection).await?;
            if adapter.authorization_revision(connection)? != revision {
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
        let mut ready = [0; 17];
        ready[0] = 1;
        ready[1..].copy_from_slice(&peer[12..]);
        send.write_all(&ready).await?;
        send.finish()?;
        let ack = recv.read_to_end(17).await.map_err(io::Error::other)?;
        if ack.len() != 17 || ack[0] != 1 || ack[1..] != nonce {
            return Err(control_error("InvalidControl"));
        }
        adapter.check(connection).await?;
        if adapter.authorization_revision(connection)? != revision {
            return Err(control_error("IdentityOrAdmissionRejected"));
        }
        prepared.check(connection, true)?;
        if let Some(retain) = prepared.retain.take() {
            retain().await?;
        }
        adapter.check(connection).await?;
        if adapter.authorization_revision(connection)? != revision {
            return Err(control_error("IdentityOrAdmissionRejected"));
        }
        prepared.check(connection, true)?;
        let paths = connection.paths();
        let report = serde_json::json!({"event":"unified-path-ready","version":1,
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
