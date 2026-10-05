//! Shared connection release seam. Adapters own transport preparation; this
//! module owns the decision to hand an ordinary connection to its caller.
use std::io;

use iroh::{Endpoint, TransportAddr, endpoint::Connection};

use crate::{admission_gate::AdmissionGate, control_session::ControlSession};

pub const MEMBER_ALPN: &[u8] = b"miopunch/session/member/1";
const FRAME_LEN: usize = 28;
const UDP: u8 = 1;

/// Existing member permission; more purposes migrate in separate slices.
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

/// One absolute caller budget, shared with the original iroh attempt.
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
        work: impl std::future::Future<Output = Result<T, ConnectError>>,
    ) -> Result<T, ConnectError> {
        tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => Err(ConnectError::Cancelled),
            result = tokio::time::timeout(self.deadline.saturating_duration_since(n0_future::time::Instant::now()), work) => result.unwrap_or(Err(ConnectError::DeadlineExceeded)),
        }
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
                if reason.starts_with("unified/ProtocolUnsupported") {
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

/// Formal SDK entry. The caller supplies identity/contact hints and purpose,
/// while this coordinator owns protocol and carrier selection. This first
/// tracer uses existing UDP only; other adapters migrate independently.
#[derive(Clone)]
pub struct ConnectionCoordinator {
    endpoint: Endpoint,
    udp_allowed: bool,
}
impl ConnectionCoordinator {
    pub(crate) fn new(endpoint: Endpoint, udp_allowed: bool) -> Self {
        Self {
            endpoint,
            udp_allowed,
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
                if !self.udp_allowed {
                    return Err(ConnectError::DataPathUnavailable);
                }
                // Signaling/custom contact hints never become ordinary data in this
                // tracer. No hidden legacy ALPN, relay or adapter fallback.
                target
                    .addrs
                    .retain(|address| matches!(address, TransportAddr::Ip(_)));
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
                    .map_err(|error| ConnectError::Transport(error.to_string()))?
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

/// Endpoint release hook: private control runs on the normal authenticated M,
/// before ordinary frames are enabled by iroh. No second handshake or timer.
#[derive(Debug, Clone)]
pub(crate) struct CoordinatorHook {
    gate: AdmissionGate,
    udp_allowed: bool,
}
impl CoordinatorHook {
    pub(crate) fn new(gate: AdmissionGate, udp_allowed: bool) -> Self {
        Self { gate, udp_allowed }
    }
    fn check(&self, connection: &Connection) -> io::Result<()> {
        if self
            .gate
            .allows(connection, crate::admission_gate::ProtocolScope::Member)
            && connection.close_reason().is_none()
        {
            Ok(())
        } else {
            Err(control_error("IdentityOrAdmissionRejected"))
        }
    }
}
fn control_error(kind: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("unified/{kind}"))
}

impl iroh::endpoint::EndpointHooks for CoordinatorHook {
    async fn before_data_release(
        &self,
        endpoint: &Endpoint,
        connection: &Connection,
    ) -> io::Result<()> {
        if connection.alpn() != MEMBER_ALPN {
            return Ok(());
        }
        self.check(connection)?;
        let qualification = self
            .gate
            .path_qualification(connection)
            .ok_or_else(|| control_error("IdentityOrAdmissionRejected"))?;
        let mut cancellation =
            ControlSession::from_connection(endpoint.clone(), connection.clone())
                .cancellation_guard();
        let (mut send, mut recv) = if connection.side().is_client() {
            connection.open_bi().await?
        } else {
            connection.accept_bi().await?
        };
        let nonce: [u8; 16] = rand::random();
        let mut frame = [0; FRAME_LEN];
        frame[..4].copy_from_slice(b"MUC1");
        frame[4] = 1; // strict version
        frame[5] = SessionPurpose::MemberBusiness as u8; // existing gate scope
        frame[6] = 1; // actual native runtime
        frame[7] = if connection.side().is_client() { 1 } else { 2 };
        frame[8] = UDP; // runtime supports UDP independently of consent
        frame[9] = if self.udp_allowed { UDP } else { 0 };
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
        if peer[5] != frame[5]
            || peer[6] != 1
            || peer[7] != 3 - frame[7]
            || peer[8] != UDP
            || peer[9] & !UDP != 0
            || peer[10..12] != [0, 0]
        {
            return Err(control_error("InvalidControl"));
        }
        if frame[9] & peer[9] & UDP == 0 {
            return Err(control_error("DataPathUnavailable"));
        }
        self.check(connection)?;
        // Pin a real, existing exact route; starting a fresh authenticated
        // probe prevents a stale path observation from releasing ordinary M.
        let paths = connection.paths();
        let path = paths
            .iter()
            .find(|path| path.is_ip() && path.is_selected())
            .ok_or_else(|| control_error("DataPathUnavailable"))?;
        let probe = path.start_probe()?;
        loop {
            self.check(connection)?;
            if !qualification.is_current() {
                return Err(control_error("IdentityOrAdmissionRejected"));
            }
            if probe.is_acknowledged().map_err(io::Error::other)? {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        release_existing_data(endpoint, connection, None, &self.gate).await?;
        let mut ready = [0; 17];
        ready[0] = 1;
        ready[1..].copy_from_slice(&peer[12..]);
        send.write_all(&ready).await?;
        send.finish()?;
        let ack = recv.read_to_end(17).await.map_err(io::Error::other)?;
        if ack.len() != 17 || ack[0] != 1 || ack[1..] != nonce {
            return Err(control_error("InvalidControl"));
        }
        self.check(connection)?;
        if !qualification.is_current() {
            return Err(control_error("IdentityOrAdmissionRejected"));
        }
        // Readiness may arrive over another restricted control route after
        // this exact probed path retired. A historical ACK cannot release M.
        // Selection may change while the permitted path stays open; keep the
        // existing selector's choice rather than requiring a stale selection.
        let live_paths = connection.paths();
        let live_path = live_paths
            .iter()
            .find(|current| {
                current.id() == path.id()
                    && current.remote_addr() == path.remote_addr()
                    && current.is_ip()
            })
            .ok_or_else(|| control_error("DataPathUnavailable"))?;
        if !matches!(probe.is_acknowledged(), Ok(true)) {
            return Err(control_error("DataPathUnavailable"));
        }
        println!(
            "{}",
            serde_json::json!({"event":"unified-path-ready","version":1,"role":if connection.side().is_client(){"initiator"}else{"responder"},"freshAck":true,"route":live_path.remote_addr().to_string(),"pathId":live_path.id().to_string(),"selected":live_path.is_selected(),
                "selectedRoute":live_paths.iter().find(|current|current.is_selected()).map(|current|current.remote_addr().to_string()),"probeWatermark":probe.watermark(),"authorizationRevision":qualification.revision()})
        );
        cancellation.disarm();
        Ok(())
    }
}

/// Existing direct-path release, shared by the native bootstrap and the
/// unified coordinator. Retention does not grant membership.
pub(crate) async fn release_existing_data(
    endpoint: &Endpoint,
    connection: &Connection,
    tcp: Option<&crate::tcp_punch::TcpPunch>,
    gate: &AdmissionGate,
) -> io::Result<()> {
    if let Some(tcp) = tcp
        && connection.paths().iter().any(|path| matches!(path.remote_addr(),
            TransportAddr::Custom(address) if address.id() == crate::tcp_transport::DATA_TRANSPORT_ID))
    {
        let control = ControlSession::from_connection(endpoint.clone(), connection.clone());
        let mut cancellation = control.cancellation_guard();
        tcp.retain_existing(endpoint, connection).await?;
        if !gate.allows_member_path_upgrade(connection) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied,
                "TCP membership changed before data release"));
        }
        cancellation.disarm();
    }
    Ok(())
}
