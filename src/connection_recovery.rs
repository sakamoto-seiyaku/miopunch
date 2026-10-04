//! Repair a data carrier while the original authenticated connection still lives.
//! A separate restricted exchange coordinates candidates; it never carries business.
use crate::{
    admission_gate::ProtocolScope, native_network::NativeNetwork, resources::ResourceEndpoint,
};
use iroh::{EndpointAddr, TransportAddr, endpoint::Connection};
use std::io;

pub(crate) use crate::connection_recovery_core::binding;
use crate::connection_recovery_core::{
    self as core, MAX_CANDIDATES, MAX_MESSAGE, REPAIR_TIMEOUT, RepairRequest, encode_message,
};
pub use crate::connection_recovery_core::{
    CONNECTION_RECOVERY_ALPN, ConnectionPathRecovery, ConnectionPathRecoveryError,
};

fn data_offer(endpoint: &ResourceEndpoint) -> EndpointAddr {
    let mut offer = endpoint.addr();
    offer.addrs.retain(|path| endpoint.allows_data_path(path));
    offer.addrs = offer.addrs.into_iter().take(MAX_CANDIDATES).collect();
    offer
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
        self.recover_connection_path_until(
            endpoint,
            original,
            contact,
            n0_future::time::Instant::now() + REPAIR_TIMEOUT,
        )
        .await
    }

    /// The caller's absolute clock also bounds the restricted contact and probe.
    pub async fn recover_connection_path_until(
        &self,
        endpoint: &ResourceEndpoint,
        original: &Connection,
        contact: EndpointAddr,
        deadline: n0_future::time::Instant,
    ) -> Result<ConnectionPathRecovery, ConnectionPathRecoveryError> {
        self.recover_with_confirmation(endpoint, original, contact, deadline, false)
            .await
    }

    /// Opt into the finite original-path confirmation extension. Legacy repair
    /// callers continue sending the unchanged two-field request.
    pub async fn recover_connection_path_confirmed_until(
        &self,
        endpoint: &ResourceEndpoint,
        original: &Connection,
        contact: EndpointAddr,
        deadline: n0_future::time::Instant,
    ) -> Result<ConnectionPathRecovery, ConnectionPathRecoveryError> {
        self.recover_with_confirmation(endpoint, original, contact, deadline, true)
            .await
    }

    async fn recover_with_confirmation(
        &self,
        endpoint: &ResourceEndpoint,
        original: &Connection,
        contact: EndpointAddr,
        deadline: n0_future::time::Instant,
        confirm: bool,
    ) -> Result<ConnectionPathRecovery, ConnectionPathRecoveryError> {
        let result = core::recover(
            endpoint.endpoint(),
            original,
            contact,
            data_offer(endpoint),
            deadline,
            tokio_util::sync::CancellationToken::new(),
            confirm,
            |path| endpoint.allows_data_path(path),
            |original| {
                if self.gate.allows(original, ProtocolScope::Member) {
                    Ok(())
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "original is not a current member session",
                    ))
                }
            },
            |original, control| self.check_repair_membership(original, control),
        )
        .await?;
        endpoint.retire_replaced_rtc(original);
        Ok(result)
    }

    async fn validate_connection_candidates(
        &self,
        endpoint: &ResourceEndpoint,
        original: &Connection,
        control: &Connection,
        candidate: EndpointAddr,
    ) -> io::Result<TransportAddr> {
        core::validate_candidates(
            endpoint.endpoint(),
            original,
            control,
            candidate,
            &|path| endpoint.allows_data_path(path),
            &|original, control| self.check_repair_membership(original, control),
        )
        .await
        .map(|path| path.address)
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
        if !self.gate.allows(original, ProtocolScope::Member) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "original membership changed during path repair",
            ));
        }
        if control.close_reason().is_some() {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "restricted repair control closed",
            ));
        }
        if !self.gate.allows_member_path_upgrade(control) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "control membership changed during path repair",
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
                self.validate_connection_candidates(
                    endpoint,
                    &original,
                    control,
                    request.offer.clone(),
                )
                .await?;
            }
            let bytes = encode_message(&data_offer(endpoint))?;
            send.write_all(&bytes).await?;
            send.finish()?;
            if request.require_confirmation {
                let path = core::validate_candidates(
                    endpoint.endpoint(),
                    &original,
                    control,
                    request.offer,
                    &|path| endpoint.allows_data_path(path),
                    &|original, control| self.check_repair_membership(original, control),
                )
                .await?;
                self.check_repair_membership(&original, control)?;
                // A real authenticated replacement-path ACK precedes retirement
                // of the old carrier; closing a sole path cannot kill repair.
                endpoint.retire_replaced_rtc(&original);
                let (mut send, mut recv) = control.open_bi().await?;
                send.write_all(&encode_message(&core::RepairConfirmation {
                    path: path.address,
                    fresh_ack: true,
                    probe_watermark: path.watermark,
                })?)
                .await?;
                send.finish()?;
                let mut confirmed = [0];
                recv.read_exact(&mut confirmed)
                    .await
                    .map_err(io::Error::other)?;
                if confirmed != [1] {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "repair confirmation refused",
                    ));
                }
            }
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
