//! Repair a data carrier while the original authenticated connection still lives.
//! A separate restricted exchange coordinates candidates; it never carries business.
use crate::{
    admission_gate::ProtocolScope, native_network::NativeNetwork, resources::ResourceEndpoint,
};
use iroh::{EndpointAddr, TransportAddr, endpoint::Connection};
use std::{io, time::Duration};
use thiserror::Error;

pub const CONNECTION_RECOVERY_ALPN: &[u8] = b"miopunch/connection-path-recovery/1";
const MAX_MESSAGE: usize = 4096;
const MAX_CANDIDATES: usize = 8;
const REPAIR_TIMEOUT: Duration = Duration::from_secs(3);
const BINDING_LABEL: &[u8] = b"miopunch-existing-connection-path-recovery/1";

#[derive(Debug, Error)]
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
}

// Private wire envelope; the session binding is never formatted for diagnostics.
#[derive(serde::Serialize, serde::Deserialize)]
struct RepairRequest {
    binding: [u8; 32],
    offer: EndpointAddr,
}

fn data_offer(endpoint: &ResourceEndpoint) -> EndpointAddr {
    let mut offer = endpoint.addr();
    offer.addrs.retain(|path| endpoint.allows_data_path(path));
    offer.addrs = offer.addrs.into_iter().take(MAX_CANDIDATES).collect();
    offer
}

fn encode_message(message: &impl serde::Serialize) -> io::Result<Vec<u8>> {
    let bytes = serde_json::to_vec(message).map_err(io::Error::other)?;
    if bytes.len() > MAX_MESSAGE {
        return Err(io::Error::other("path recovery message exceeds bound"));
    }
    Ok(bytes)
}

pub(crate) fn binding(connection: &Connection) -> io::Result<[u8; 32]> {
    let mut binding = [0; 32];
    connection
        .export_keying_material(&mut binding, BINDING_LABEL, &[])
        .map_err(|_| io::Error::other("original TLS connection binding unavailable"))?;
    Ok(binding)
}
fn failed(connection: &Connection, cause: impl ToString) -> ConnectionPathRecoveryError {
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
struct CloseControl(Connection);
impl Drop for CloseControl {
    fn drop(&mut self) {
        self.0.close(0u32.into(), b"path recovery exchange ended");
    }
}

impl NativeNetwork {
    /// Coordinate permitted candidates over a new protected control contact,
    /// then validate a carrier on the original Connection. The three-second
    /// attempt is also cancelled by the original noq connection's actual close.
    /// This is an explicit maintenance operation, not an application redial or
    /// a second background scheduling loop.
    pub async fn recover_connection_path(
        &self,
        endpoint: &ResourceEndpoint,
        original: &Connection,
        contact: EndpointAddr,
    ) -> Result<ConnectionPathRecovery, ConnectionPathRecoveryError> {
        if original.close_reason().is_some() {
            return Err(failed(original, "original connection closed"));
        }
        if !self.gate.allows(original, ProtocolScope::Member) || original.remote_id() != contact.id
        {
            return Err(failed(
                original,
                "original connection is not a current member session for this target",
            ));
        }
        let repair = async {
            let request = RepairRequest {
                binding: binding(original)?,
                offer: data_offer(endpoint),
            };
            let control = endpoint
                .endpoint()
                .connect(contact, CONNECTION_RECOVERY_ALPN)
                .await
                .map_err(io::Error::other)?;
            let control = CloseControl(control);
            if !self.gate.allows_member_path_upgrade(&control.0) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "control peer is not a current member",
                ));
            }
            let (mut send, mut recv) = control.0.open_bi().await?;
            send.write_all(&encode_message(&request)?).await?;
            send.finish()?;
            let bytes = recv
                .read_to_end(MAX_MESSAGE)
                .await
                .map_err(io::Error::other)?;
            let candidate: EndpointAddr =
                serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            let path = self
                .validate_connection_candidates(endpoint, original, &control.0, candidate)
                .await?;
            Ok(ConnectionPathRecovery {
                stable_id: original.stable_id(),
                path,
            })
        };
        tokio::select! {
            _ = original.closed() => Err(failed(original, "original connection closed")),
            result = tokio::time::timeout(REPAIR_TIMEOUT, repair) => match result {
                Ok(Ok(recovered)) => {
                    if original.close_reason().is_some() { Err(failed(original, "original connection closed")) } else { Ok(recovered) }
                }
                Ok(Err(error)) => Err(failed(original, error)),
                Err(_) => Err(failed(original, "bounded path repair timed out")),
            }
        }
    }

    async fn validate_connection_candidates(
        &self,
        endpoint: &ResourceEndpoint,
        original: &Connection,
        control: &Connection,
        candidate: EndpointAddr,
    ) -> io::Result<TransportAddr> {
        if candidate.id != original.remote_id() || candidate.addrs.len() > MAX_CANDIDATES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid data path offer",
            ));
        }
        let paths = candidate
            .addrs
            .into_iter()
            .filter(|path| endpoint.allows_data_path(path))
            .collect::<Vec<_>>();
        if paths.is_empty() {
            return Err(io::Error::other("no mutually permitted data candidate"));
        }
        // Noq opens new paths only from the original dialing side. That peer
        // can be either requester or responder of this separate control exchange.
        if original.side().is_client() {
            for path in &paths {
                loop {
                    self.check_repair_membership(original, control)?;
                    match endpoint
                        .endpoint()
                        .open_data_transport_path(original, path.clone())
                        .await
                    {
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            tokio::time::sleep(Duration::from_millis(50)).await
                        }
                        _ => break,
                    }
                }
            }
        }
        loop {
            self.check_repair_membership(original, control)?;
            if let Some(path) = original
                .paths()
                .iter()
                .find(|path| paths.contains(path.remote_addr()))
                .map(|path| path.remote_addr().clone())
            {
                return Ok(path);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn check_repair_membership(
        &self,
        original: &Connection,
        control: &Connection,
    ) -> io::Result<()> {
        if original.close_reason().is_some() {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "original connection closed",
            ));
        }
        if !self.gate.allows(original, ProtocolScope::Member)
            || !self.gate.allows_member_path_upgrade(control)
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "membership changed during path repair",
            ));
        }
        Ok(())
    }

    /// Router handler for a restricted, proved current member. The opaque TLS
    /// binding must identify a live Member connection to this exact peer.
    /// Discovery hints or a surviving control session cannot resurrect it.
    pub async fn serve_connection_path_recovery(
        &self,
        endpoint: &ResourceEndpoint,
        control: &Connection,
    ) -> io::Result<()> {
        tokio::time::timeout(REPAIR_TIMEOUT, async {
            if control.alpn() != CONNECTION_RECOVERY_ALPN
                || !self.gate.allows_member_path_upgrade(control)
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "unqualified path recovery control",
                ));
            }
            let (mut send, mut recv) = control.accept_bi().await?;
            let bytes = recv
                .read_to_end(MAX_MESSAGE)
                .await
                .map_err(io::Error::other)?;
            let request: RepairRequest = serde_json::from_slice(&bytes).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid path repair request")
            })?;
            if request.offer.id != control.remote_id() || request.offer.addrs.len() > MAX_CANDIDATES
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid requester data offer",
                ));
            }
            let original = self
                .gate
                .member_connection_for_binding(control.remote_id(), &request.binding)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotConnected,
                        "original member connection is closed or unknown",
                    )
                })?;
            self.check_repair_membership(&original, control)?;
            if original.side().is_client() {
                self.validate_connection_candidates(endpoint, &original, control, request.offer)
                    .await?;
            }
            let bytes = encode_message(&data_offer(endpoint))?;
            send.write_all(&bytes).await?;
            send.finish()?;
            // The requester retains its authenticated control scope while it
            // validates the new carrier. Its owned guard closes this exchange
            // on completion, cancellation or failure; the serving wait is bound.
            let _ = control.closed().await;
            Ok(())
        })
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "path recovery control timed out"))?
    }
}
