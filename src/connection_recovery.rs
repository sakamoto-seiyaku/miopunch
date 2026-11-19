//! Repair a data carrier while the original authenticated connection still lives.
//! A separate restricted exchange coordinates candidates; it never carries business.
use crate::control_protocol::AuthenticatedControl;
use crate::{
    admission_gate::{AdmissionGate, ProtocolScope},
    connection_coordinator::AttemptContext,
    native_network::NativeNetwork,
    resources::{ContactClass, ResourceEndpoint},
    session_maintenance::{Completion, Reservation, matches_path},
};
use iroh::endpoint::{
    ConnectOptions, ConnectWithOptsError, ConnectingError, ControlConnection, ControlHandle,
    ControlMessageFormat, DataCandidatePath, MemberRepairContacts,
};
use iroh::{EndpointAddr, TransportAddr, endpoint::Connection};
use std::{io, sync::Arc};
use tokio_util::sync::CancellationToken;

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
        let deadline = deadline.min(n0_future::time::Instant::now() + REPAIR_TIMEOUT);
        if contact.id != original.remote_id() || !self.gate.allows(original, ProtocolScope::Member)
        {
            return Err(core::failed(
                original,
                "invalid peer or original member session",
            ));
        }
        let attempt = AttemptContext::new(deadline, tokio_util::sync::CancellationToken::new());
        let registry = self.gate.maintenance();
        let reservation_error =
            |cause| core::failed(original, format!("repair reservation: {cause:?}"));
        let mut lease = match registry
            .acquire_or_wait(original, &attempt, ())
            .map_err(reservation_error)?
        {
            Reservation::Owned(lease) => lease,
            Reservation::Pending(wait) => {
                if let Some(Completion::Legacy(result)) =
                    wait.wait().await.map_err(reservation_error)?
                    && self.gate.allows(original, ProtocolScope::Member)
                    && endpoint.allows_data_path(&result.path)
                    && matches_path(original, result.stable_id, &result.path_id, &result.path)
                {
                    return Ok(result);
                }
                registry
                    .acquire(original, &attempt, ())
                    .map_err(reservation_error)?
            }
        };
        let quota = endpoint
            .begin_contact_attempt_until(original.remote_id(), ContactClass::Foreground, deadline)
            .map_err(|error| core::failed(original, error))?;
        let result = lease
            .run_ref(core::recover(
                endpoint.endpoint(),
                original,
                contact,
                data_offer(endpoint),
                deadline,
                attempt.cancellation.clone(),
                confirm,
                |path| endpoint.allows_data_path(path),
                |original| check_original(&self.gate, original),
                |original, control| check_repair_membership(&self.gate, original, control),
                || Ok(()),
            ))
            .await
            .map_err(reservation_error)?
            .map_err(|error| core::failed(original, error))?;
        endpoint.retire_replaced_rtc(original);
        quota.succeeded();
        // Only the confirmed requester owns the actual legacy response fields.
        if result.peer_confirmed {
            lease.completed(Completion::Legacy(result.clone()));
        }
        Ok(result)
    }

    /// Router handler for a restricted, proved current member. The opaque TLS
    /// binding must identify a live Member connection to this exact peer.
    /// Discovery hints or a surviving control session cannot resurrect it.
    pub async fn serve_connection_path_recovery(
        &self,
        endpoint: &ResourceEndpoint,
        control: &(impl AuthenticatedControl + ?Sized),
    ) -> io::Result<()> {
        serve_path_recovery(&self.gate, endpoint, control, false).await
    }
}

fn check_original(gate: &AdmissionGate, original: &Connection) -> io::Result<()> {
    if original.close_reason().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "original connection closed",
        ));
    }
    if !gate.allows(original, ProtocolScope::Member) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "original membership changed during path repair",
        ));
    }
    Ok(())
}

fn check_repair_membership(
    gate: &AdmissionGate,
    original: &Connection,
    control: &(impl AuthenticatedControl + ?Sized),
) -> io::Result<()> {
    check_original(gate, original)?;
    if control.close_reason().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "restricted repair control closed",
        ));
    }
    if !gate.allows_member_path_upgrade(control) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "control membership changed during path repair",
        ));
    }
    Ok(())
}

struct FusedRecovery<'a> {
    endpoint: &'a iroh::endpoint::RestrictedEndpoint,
    contact: Option<(TransportAddr, n0_future::time::Instant)>,
}

impl core::RecoveryEndpoint for FusedRecovery<'_> {
    type Control = Arc<ControlConnection>;

    async fn connect_repair_control(
        &self,
        contact: EndpointAddr,
        options: ConnectOptions,
    ) -> io::Result<Self::Control> {
        let options = match &self.contact {
            Some((route, reach_by)) => options
                .with_bootstrap_route(route.clone())
                .with_caller_deadline(*reach_by),
            None => options,
        };
        self.endpoint
            .connect_control_with_opts(contact, CONNECTION_RECOVERY_ALPN, options)
            .await
            .map_err(|error| {
                let kind = match &error {
                    ConnectWithOptsError::LocallyRejected { .. } => io::ErrorKind::PermissionDenied,
                    ConnectWithOptsError::AttemptTimedOut { .. } => io::ErrorKind::TimedOut,
                    ConnectWithOptsError::AttemptCancelled { .. } => io::ErrorKind::Interrupted,
                    _ => io::ErrorKind::Other,
                };
                io::Error::new(kind, error)
            })?
            .await
            .map(Arc::new)
            .map_err(|error| {
                let kind = match &error {
                    ConnectingError::LocallyRejected { .. } => io::ErrorKind::PermissionDenied,
                    ConnectingError::AdmissionFailed { kind, .. } => *kind,
                    ConnectingError::AttemptTimedOut { .. } => io::ErrorKind::TimedOut,
                    ConnectingError::AttemptCancelled { .. } => io::ErrorKind::Interrupted,
                    _ => io::ErrorKind::Other,
                };
                io::Error::new(kind, error)
            })
    }

    fn original_control(&self, original: &Connection) -> ControlHandle {
        self.endpoint.control_handle(original)
    }

    async fn open_repair_candidate(
        &self,
        original: &Connection,
        address: TransportAddr,
    ) -> io::Result<Option<DataCandidatePath>> {
        self.original_control(original)
            .open_data_transport_candidate(address)
            .await
    }
}

pub(crate) async fn recover_member_path(
    gate: &AdmissionGate,
    endpoint: &ResourceEndpoint,
    original: &Connection,
    contacts: MemberRepairContacts,
    deadline: n0_future::time::Instant,
    cancel: CancellationToken,
) -> Result<ConnectionPathRecovery, ConnectionPathRecoveryError> {
    check_original(gate, original).map_err(|error| core::failed(original, error))?;
    let quota = endpoint
        .begin_contact_attempt_until(original.remote_id(), ContactClass::Foreground, deadline)
        .map_err(|error| core::failed(original, error))?;
    let attempt = AttemptContext::new(deadline, cancel);
    let lease = gate
        .maintenance()
        .acquire_outgoing(original, &attempt, ())
        .map_err(|error| core::failed(original, format!("repair reservation: {error:?}")))?;
    let network = endpoint.endpoint().udp_network_generation();
    let result = contacts
        .run(|contact, reach_by| {
            let network = &network;
            let lease = &lease;
            let cancel = attempt.cancellation.child_token();
            async move {
                let Some(TransportAddr::Custom(route)) = contact.addrs.first() else {
                    return Err(io::ErrorKind::InvalidInput.into());
                };
                if !endpoint.allows_restricted_contact(route) {
                    return Err(io::Error::new(
                        io::ErrorKind::NotConnected,
                        "repair contact facility unavailable",
                    ));
                }
                let check_network = || {
                    if !network.is_current() {
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "network changed during path repair",
                        ));
                    }
                    Ok(())
                };
                let contact_endpoint = FusedRecovery {
                    endpoint: endpoint.endpoint(),
                    contact: Some((TransportAddr::Custom(route.clone()), reach_by)),
                };
                core::recover(
                    &contact_endpoint,
                    original,
                    contact,
                    data_offer(endpoint),
                    deadline,
                    cancel,
                    true,
                    |path| endpoint.allows_data_path(path),
                    |original| {
                        check_network()?;
                        check_original(gate, original)
                    },
                    |original, control| {
                        check_network()?;
                        check_repair_membership(gate, original, control)
                    },
                    || lease.seal_outgoing_hello(),
                )
                .await
            }
        })
        .await
        .map_err(|error| core::failed(original, error))?;
    quota.succeeded();
    Ok(result)
}

pub(crate) async fn serve_path_recovery(
    gate: &AdmissionGate,
    endpoint: &ResourceEndpoint,
    control: &(impl AuthenticatedControl + ?Sized),
    fused: bool,
) -> io::Result<()> {
    let deadline = n0_future::time::Instant::now() + REPAIR_TIMEOUT;
    let attempt = AttemptContext::new(deadline, CancellationToken::new());
    let network = endpoint.endpoint().udp_network_generation();
    let serve = async {
        let restricted_carrier = || {
            control.paths().iter().any(|path| {
                matches!(path.remote_addr(), TransportAddr::Custom(route)
                if path.is_selected() && endpoint.allows_restricted_contact(route))
            })
        };
        if control.alpn() != CONNECTION_RECOVERY_ALPN
            || !gate.allows_member_path_upgrade(control)
            || (fused && !restricted_carrier())
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "unqualified path recovery control",
            ));
        }
        let exchange = control.accept_control_exchange().await?;
        let bytes = exchange
            .recv_message(ControlMessageFormat::EndTerminated {
                maximum: MAX_MESSAGE,
            })
            .await?;
        let request: RepairRequest = serde_json::from_slice(&bytes).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "invalid path repair request")
        })?;
        if request.offer.id != control.remote_id() || request.offer.addrs.len() > MAX_CANDIDATES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid requester data offer",
            ));
        }
        let original = gate
            .member_connection_for_binding(control.remote_id(), &request.binding)
            .filter(|original| {
                !fused
                    || (original.alpn() == crate::native_network::FUSED_MEMBER_ALPN
                        && request.require_confirmation)
            })
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotConnected,
                    "original member connection is closed or unknown",
                )
            })?;
        let check =
            |original: &Connection, control: &_| check_repair_membership(gate, original, control);
        let permits = |path: &TransportAddr| {
            endpoint.allows_data_path(path) && (!fused || network.is_current())
        };
        check(&original, control)?;
        let registry = gate.maintenance();
        let reservation_error =
            |cause| io::Error::other(format!("incoming repair reservation: {cause:?}"));
        let lease = if fused {
            match registry
                .acquire_incoming(&original, &attempt)
                .map_err(reservation_error)?
            {
                Reservation::Owned(lease) => lease,
                Reservation::Pending(wait) => {
                    wait.wait().await.map_err(reservation_error)?;
                    registry
                        .acquire(&original, &attempt, ())
                        .map_err(reservation_error)?
                }
            }
        } else {
            registry
                .acquire(&original, &attempt, ())
                .map_err(reservation_error)?
        };
        let fused_endpoint = FusedRecovery {
            endpoint: endpoint.endpoint(),
            contact: None,
        };
        let validate = |candidate| async {
            if fused {
                core::validate_candidates(
                    &fused_endpoint,
                    &original,
                    control,
                    candidate,
                    &permits,
                    &check,
                )
                .await
            } else {
                core::validate_candidates(
                    endpoint.endpoint(),
                    &original,
                    control,
                    candidate,
                    &permits,
                    &check,
                )
                .await
            }
        };
        lease
            .run(async {
                if !fused && original.side().is_client() {
                    validate(request.offer.clone()).await?;
                }
                let bytes = encode_message(&data_offer(endpoint))?;
                exchange
                    .send_message(
                        ControlMessageFormat::EndTerminated {
                            maximum: MAX_MESSAGE,
                        },
                        &bytes,
                    )
                    .await?;
                if request.require_confirmation {
                    let mut path = validate(request.offer).await?;
                    check(&original, control)?;
                    // A real authenticated replacement-path ACK precedes retirement
                    // of the old carrier; closing a sole path cannot kill repair.
                    endpoint.retire_replaced_rtc(&original);
                    let confirmation = control.open_control_exchange().await?;
                    confirmation
                        .send_message(
                            ControlMessageFormat::EndTerminated {
                                maximum: MAX_MESSAGE,
                            },
                            &encode_message(&core::RepairConfirmation {
                                path: path.address.clone(),
                                fresh_ack: true,
                                probe_watermark: path.watermark,
                            })?,
                        )
                        .await?;
                    let confirmed = confirmation
                        .recv_message(ControlMessageFormat::Fixed { length: 1 })
                        .await?;
                    if confirmed != [1] {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "repair confirmation refused",
                        ));
                    }
                    if fused {
                        check(&original, control)?;
                        if !permits(&path.address) {
                            return Err(io::Error::new(
                                io::ErrorKind::PermissionDenied,
                                "data permission changed during repair",
                            ));
                        }
                        path.retain_candidate();
                    }
                }
                // Keep responder authentication until the requester finishes its
                // original-path validation and closes the private exchange.
                let _ = control.revocation_handle().closed().await;
                Ok(())
            })
            .await
            .map_err(|cause| io::Error::other(format!("incoming repair: {cause:?}")))?
    };
    tokio::time::timeout_at(deadline, serve)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "path recovery control timed out"))?
}
