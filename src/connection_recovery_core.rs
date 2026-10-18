//! The existing private original-session repair exchange, shared by platforms.
//! Platform adapters supply current qualification and data resource permission.
use futures_util::FutureExt;
use iroh::{
    Endpoint, EndpointAddr, TransportAddr,
    endpoint::{ConnectOptions, Connection, ControlExchange, ControlMessageFormat},
};
use n0_future::time::{Instant, sleep, timeout};
use std::{io, time::Duration};

use crate::control_protocol::AuthenticatedControl;

/// Actual platform owners share the repair algorithm without widening a
/// restricted endpoint or the control capability it returns.
pub(crate) trait RecoveryEndpoint: Sync {
    type Control: AuthenticatedControl;
    fn connect_repair_control(
        &self,
        contact: EndpointAddr,
        options: ConnectOptions,
    ) -> impl Future<Output = io::Result<Self::Control>> + Send;
    fn original_control(&self, original: &Connection) -> iroh::endpoint::ControlHandle;
}
impl RecoveryEndpoint for Endpoint {
    type Control = Connection;
    async fn connect_repair_control(
        &self,
        contact: EndpointAddr,
        options: ConnectOptions,
    ) -> io::Result<Connection> {
        self.connect_with_opts(contact, CONNECTION_RECOVERY_ALPN, options)
            .await
            .map_err(io::Error::other)?
            .await
            .map_err(io::Error::other)
    }
    fn original_control(&self, original: &Connection) -> iroh::endpoint::ControlHandle {
        self.control_handle(original)
    }
}
impl RecoveryEndpoint for iroh::endpoint::RestrictedEndpoint {
    type Control = std::sync::Arc<iroh::endpoint::ControlConnection>;
    async fn connect_repair_control(
        &self,
        contact: EndpointAddr,
        options: ConnectOptions,
    ) -> io::Result<Self::Control> {
        self.connect_control_with_opts(contact, CONNECTION_RECOVERY_ALPN, options)
            .await
            .map_err(io::Error::other)?
            .await
            .map(std::sync::Arc::new)
            .map_err(io::Error::other)
    }
    fn original_control(&self, original: &Connection) -> iroh::endpoint::ControlHandle {
        self.control_handle(original)
    }
}
async fn send_message(exchange: &ControlExchange, value: &impl serde::Serialize) -> io::Result<()> {
    exchange
        .send_message(
            ControlMessageFormat::EndTerminated {
                maximum: MAX_MESSAGE,
            },
            &encode_message(value)?,
        )
        .await
}
async fn recv_message<T: serde::de::DeserializeOwned>(exchange: &ControlExchange) -> io::Result<T> {
    serde_json::from_slice(
        &exchange
            .recv_message(ControlMessageFormat::EndTerminated {
                maximum: MAX_MESSAGE,
            })
            .await?,
    )
    .map_err(io::Error::other)
}

pub const CONNECTION_RECOVERY_ALPN: &[u8] = b"miopunch/connection-path-recovery/1";
pub(crate) const MAX_MESSAGE: usize = 4096;
pub(crate) const MAX_CANDIDATES: usize = 8;
pub(crate) const REPAIR_TIMEOUT: Duration = Duration::from_secs(3);
const BINDING_LABEL: &[u8] = b"miopunch-existing-connection-path-recovery/1";

#[derive(Debug, thiserror::Error)]
pub enum ConnectionPathRecoveryError {
    #[error("original connection {stable_id} terminated; a later contact must be a new connection")]
    Terminal { stable_id: usize },
    #[error("path repair on connection {stable_id} failed while it remains alive: {cause}")]
    Retryable { stable_id: usize, cause: String },
}
#[derive(Debug)]
pub struct ConnectionPathRecovery {
    pub stable_id: usize,
    pub path: TransportAddr,
    pub path_id: String,
    pub probe_watermark: u64,
    pub control_stable_id: usize,
    pub peer_confirmed: bool,
}
// Never format the exporter or this private envelope for diagnostics.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct RepairRequest {
    pub binding: [u8; 32],
    pub offer: EndpointAddr,
    #[serde(default, skip_serializing_if = "is_false")]
    pub require_confirmation: bool,
}
fn is_false(value: &bool) -> bool {
    !value
}
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct RepairConfirmation {
    pub path: TransportAddr,
    pub fresh_ack: bool,
    pub probe_watermark: u64,
}
pub(crate) fn encode_message(value: &impl serde::Serialize) -> io::Result<Vec<u8>> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    if bytes.len() > MAX_MESSAGE {
        return Err(io::Error::other("path recovery message exceeds bound"));
    }
    Ok(bytes)
}
pub(crate) fn binding(
    connection: &(impl crate::control_protocol::AuthenticatedControl + ?Sized),
) -> io::Result<[u8; 32]> {
    let mut binding = [0; 32];
    connection
        .export_keying_material(&mut binding, BINDING_LABEL, &[])
        .map_err(|_| io::Error::other("original TLS connection binding unavailable"))?;
    Ok(binding)
}
pub(crate) fn failed(connection: &Connection, cause: impl ToString) -> ConnectionPathRecoveryError {
    if connection.close_reason().is_some() {
        ConnectionPathRecoveryError::Terminal {
            stable_id: connection.stable_id(),
        }
    } else {
        ConnectionPathRecoveryError::Retryable {
            stable_id: connection.stable_id(),
            cause: cause.to_string(),
        }
    }
}
struct CloseControl<C: AuthenticatedControl>(C);
impl<C: AuthenticatedControl> Drop for CloseControl<C> {
    fn drop(&mut self) {
        self.0.close(0u32.into(), b"path recovery exchange ended");
    }
}
struct CancelContact(tokio_util::sync::CancellationToken);
impl Drop for CancelContact {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

pub(crate) struct ValidatedPath {
    pub address: TransportAddr,
    pub id: String,
    pub watermark: u64,
}
pub(crate) async fn validate_candidates<C: AuthenticatedControl + ?Sized>(
    endpoint: &(impl RecoveryEndpoint + ?Sized),
    original: &Connection,
    control: &C,
    candidate: EndpointAddr,
    permits: &impl Fn(&TransportAddr) -> bool,
    check: &impl Fn(&Connection, &C) -> io::Result<()>,
) -> io::Result<ValidatedPath> {
    if candidate.id != original.remote_id() || candidate.addrs.len() > MAX_CANDIDATES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid data path offer",
        ));
    }
    let paths: Vec<_> = candidate.addrs.into_iter().filter(permits).collect();
    if paths.is_empty() {
        return Err(io::Error::other("no mutually permitted data candidate"));
    }
    if original.side().is_client() {
        for path in &paths {
            loop {
                check(original, control)?;
                let opened = endpoint
                    .original_control(original)
                    .open_data_transport_path(path.clone())
                    .await;
                match opened {
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        sleep(Duration::from_millis(50)).await
                    }
                    // A candidate-local refusal cannot rule out another
                    // permitted carrier. Fatal original/control qualification
                    // is checked independently before opening and probing.
                    _ => break,
                }
            }
        }
    }
    let mut probes = Vec::new();
    loop {
        check(original, control)?;
        for path in original
            .paths()
            .iter()
            .filter(|path| paths.contains(path.remote_addr()))
        {
            if probes.iter().any(|(id, _, _, _)| *id == path.id()) {
                continue;
            }
            if let Ok(probe) = path.start_probe() {
                let watermark = probe.watermark();
                probes.push((path.id(), path.remote_addr().clone(), watermark, probe));
            }
        }
        if let Some((id, address, watermark, _)) = probes
            .iter()
            .find(|(_, _, _, probe)| matches!(probe.is_acknowledged(), Ok(true)))
        {
            check(original, control)?;
            if !permits(address) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "data permission changed during repair",
                ));
            }
            return Ok(ValidatedPath {
                address: address.clone(),
                id: id.to_string(),
                watermark: *watermark,
            });
        }
        sleep(Duration::from_millis(20)).await;
    }
}

/// One existing coordinator operation: protected contact, original-M path probe,
/// and cleanup all share the caller clock, with the historical three-second cap.
pub(crate) async fn recover<E: RecoveryEndpoint>(
    endpoint: &E,
    original: &Connection,
    contact: EndpointAddr,
    mut offer: EndpointAddr,
    caller_deadline: Instant,
    cancel: tokio_util::sync::CancellationToken,
    require_confirmation: bool,
    permits: impl Fn(&TransportAddr) -> bool,
    check_original: impl Fn(&Connection) -> io::Result<()>,
    check: impl Fn(&Connection, &E::Control) -> io::Result<()>,
) -> Result<ConnectionPathRecovery, ConnectionPathRecoveryError> {
    if original.close_reason().is_some() {
        return Err(failed(original, "original connection closed"));
    }
    if contact.id != original.remote_id() {
        return Err(failed(original, "repair target differs from original peer"));
    }
    check_original(original).map_err(|error| failed(original, error))?;
    let deadline = caller_deadline.min(Instant::now() + REPAIR_TIMEOUT);
    let _cancel = CancelContact(cancel.clone());
    offer.addrs.retain(&permits);
    offer.addrs = offer.addrs.into_iter().take(MAX_CANDIDATES).collect();
    let work = async {
        let request = RepairRequest {
            binding: binding(original)?,
            offer,
            require_confirmation,
        };
        let options = ConnectOptions::default()
            .with_caller_deadline(deadline)
            .with_attempt_cancellation(cancel.clone());
        let control = CloseControl(endpoint.connect_repair_control(contact, options).await?);
        check(original, &control.0)?;
        let exchange = control.0.open_control_exchange().await?;
        send_message(&exchange, &request).await?;
        let candidate = recv_message(&exchange).await?;
        let path = validate_candidates(endpoint, original, &control.0, candidate, &permits, &check)
            .await?;
        check(original, &control.0)?;
        if require_confirmation {
            let confirmation_exchange = control.0.accept_control_exchange().await?;
            let confirmation: RepairConfirmation = recv_message(&confirmation_exchange).await?;
            // The peer validates OUR advertised receive address. Its remote
            // address is not our remote address (including distinct relays).
            if !request.offer.addrs.contains(&confirmation.path)
                || !permits(&confirmation.path)
                || !confirmation.fresh_ack
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "peer did not validate the original replacement path",
                ));
            }
            check(original, &control.0)?;
            confirmation_exchange
                .send_message(ControlMessageFormat::Fixed { length: 1 }, &[1])
                .await?;
            confirmation_exchange.finish_send_confirmed().await?;
        }
        Ok::<_, io::Error>(ConnectionPathRecovery {
            stable_id: original.stable_id(),
            path: path.address,
            path_id: path.id,
            probe_watermark: path.watermark,
            control_stable_id: control.0.stable_id(),
            peer_confirmed: require_confirmation,
        })
    }
    .fuse();
    let closed = original.closed().fuse();
    let cancelled = cancel.cancelled().fuse();
    futures_util::pin_mut!(work, closed, cancelled);
    let result = timeout(deadline.saturating_duration_since(Instant::now()), async {
        futures_util::select_biased! {
            _ = closed => Err(io::Error::new(io::ErrorKind::NotConnected, "original connection closed")),
            _ = cancelled => Err(io::Error::new(io::ErrorKind::Interrupted, "path repair cancelled")),
            result = work => result,
        }
    }).await.map_err(|_| failed(original, "bounded path repair caller deadline exceeded"))?;
    if original.close_reason().is_some() {
        return Err(failed(original, "original connection closed"));
    }
    result.map_err(|error| failed(original, error))
}
