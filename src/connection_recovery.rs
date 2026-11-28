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
    self as core, MAX_MESSAGE, REPAIR_TIMEOUT, RepairRequest, encode_message,
};
pub use crate::connection_recovery_core::{
    CONNECTION_RECOVERY_ALPN, ConnectionPathRecovery, ConnectionPathRecoveryError,
};

fn data_offer(endpoint: &ResourceEndpoint) -> EndpointAddr {
    let mut offer = endpoint.addr();
    offer.addrs.retain(|path| endpoint.allows_data_path(path));
    offer
}

impl NativeNetwork {
    /// Coordinate permitted candidates over a new protected control contact,
    /// then validate a carrier on the original Connection. This convenience
    /// entry gives the complete operation three seconds. The explicit deadline
    /// entry can give slow signaling the caller's remaining lifetime.
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

    /// Require a peer-confirmed replacement path. Non-negotiating v1 peers keep
    /// their bounded single offer and optional confirmation exchange.
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
    const VALIDATE_RETURN_PATH: bool = true;
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
                    ConnectingError::AdmissionFailed { reason, .. } => reason.kind(),
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
    let attempt = AttemptContext::new(deadline, cancel);
    let registry = gate.maintenance();
    let reservation_error = |error| core::failed(original, format!("repair reservation: {error:?}"));
    let lease = loop {
        match registry
            .acquire_outgoing_or_wait(original, &attempt, ())
            .map_err(reservation_error)?
        {
            Reservation::Owned(lease) => break lease,
            Reservation::Pending(wait) => {
                wait.wait().await.map_err(reservation_error)?;
                check_original(gate, original).map_err(|error| core::failed(original, error))?;
            }
        }
    };
    let quota = endpoint
        .begin_contact_attempt_until(original.remote_id(), ContactClass::Foreground, deadline)
        .map_err(|error| core::failed(original, error))?;
    let result = contacts
        .run(|contact, reach_by| {
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
                let route_deadline = core::contact_deadline(&contact, deadline);
                let contact_endpoint = FusedRecovery {
                    endpoint: endpoint.endpoint(),
                    contact: Some((
                        TransportAddr::Custom(route.clone()),
                        reach_by.min(route_deadline),
                    )),
                };
                if endpoint
                    .member_fusion
                    .as_ref()
                    .is_some_and(|(fusion, _)| fusion.supports_direct_repair(original))
                {
                    match recover_member_tcp(
                        gate,
                        endpoint,
                        &contact_endpoint,
                        original,
                        contact.clone(),
                        route_deadline,
                        cancel.clone(),
                        || lease.seal_outgoing_hello(),
                    )
                    .await
                    {
                        Err(error) if error.kind() == io::ErrorKind::Unsupported => {}
                        result => return result,
                    }
                }
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
                        check_original(gate, original)
                    },
                    |original, control| {
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

async fn recover_member_tcp(
    gate: &AdmissionGate,
    endpoint: &ResourceEndpoint,
    contact_endpoint: &FusedRecovery<'_>,
    original: &Connection,
    contact: EndpointAddr,
    deadline: n0_future::time::Instant,
    cancel: CancellationToken,
    begin_validation: impl FnOnce() -> io::Result<()>,
) -> io::Result<ConnectionPathRecovery> {
    use core::RecoveryEndpoint;
    let work = async {
        let control = contact_endpoint
            .connect_repair_control(
                contact,
                ConnectOptions::default()
                    .with_caller_deadline(deadline)
                    .with_attempt_cancellation(cancel.clone()),
            )
            .await?;
        let result = async {
            check_repair_membership(gate, original, &control)?;
            let exchange = control.open_control_exchange().await?;
            exchange
                .send_message(
                    ControlMessageFormat::EndTerminated {
                        maximum: MAX_MESSAGE,
                    },
                    &encode_message(&RepairRequest {
                        binding: binding(original)?,
                        offer: EndpointAddr::new(endpoint.endpoint().id()),
                        require_confirmation: true,
                        traverse_tcp: true,
                        batched: false,
                        budget_ms: Some(
                            deadline
                                .saturating_duration_since(n0_future::time::Instant::now())
                                .as_millis() as u64,
                        ),
                    })?,
                )
                .await?;
            if exchange
                .recv_message(ControlMessageFormat::Fixed { length: 1 })
                .await?
                != [1]
            {
                return Err(io::ErrorKind::Unsupported.into());
            }
            begin_validation()?;
            let (fusion, _) = endpoint
                .member_fusion
                .as_ref()
                .ok_or(io::ErrorKind::Unsupported)?;
            let traversal = control.open_control_exchange().await?;
            let (remote, watermark) = fusion
                .repair_direct_until(original, traversal, deadline, &cancel)
                .await?;
            check_repair_membership(gate, original, &control)?;
            let paths = original.paths();
            let path = paths
                .iter()
                .find(|path| path.remote_addr() == &remote)
                .ok_or(io::ErrorKind::NotConnected)?;
            Ok(ConnectionPathRecovery {
                stable_id: original.stable_id(),
                path: remote,
                path_id: path.id().to_string(),
                probe_watermark: watermark,
                control_stable_id: control.stable_id(),
                peer_confirmed: true,
            })
        }
        .await;
        control.close(0u32.into(), b"member TCP repair ended");
        result
    };
    tokio::select! { biased;
        _ = original.closed() => Err(io::ErrorKind::NotConnected.into()),
        _ = cancel.cancelled() => Err(io::ErrorKind::Interrupted.into()),
        _ = tokio::time::sleep_until(deadline) => Err(io::ErrorKind::TimedOut.into()),
        result = work => result,
    }
}

pub(crate) async fn serve_path_recovery(
    gate: &AdmissionGate,
    endpoint: &ResourceEndpoint,
    control: &(impl AuthenticatedControl + ?Sized),
    fused: bool,
) -> io::Result<()> {
    let started = n0_future::time::Instant::now();
    let contact = EndpointAddr::new(control.remote_id()).with_addrs(
        control
            .paths()
            .iter()
            .filter(|path| path.is_selected())
            .map(|path| path.remote_addr().clone()),
    );
    let maximum = core::contact_deadline(&contact, endpoint.contact_deadline());
    let mut deadline = maximum;
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
        if request.offer.id != control.remote_id() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid requester data offer",
            ));
        }
        let requested = request
            .budget_ms
            .map(|ms| {
                started
                    + std::time::Duration::from_millis(ms)
                        .min(maximum.saturating_duration_since(started))
            })
            .unwrap_or(started + REPAIR_TIMEOUT);
        deadline = maximum.min(requested);
        let attempt = AttemptContext::new(deadline, CancellationToken::new());
        let operation = async {
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
            let check = |original: &Connection, control: &_| {
                check_repair_membership(gate, original, control)
            };
            let permits = |path: &TransportAddr| {
                endpoint.allows_data_path(path)
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
            if fused && request.traverse_tcp {
                let (fusion, _) = endpoint
                    .member_fusion
                    .as_ref()
                    .ok_or(io::ErrorKind::Unsupported)?;
                let supported =
                    request.require_confirmation && fusion.supports_direct_repair(&original);
                exchange
                    .send_message(
                        ControlMessageFormat::Fixed { length: 1 },
                        &[u8::from(supported)],
                    )
                    .await?;
                if !supported {
                    return Err(io::ErrorKind::Unsupported.into());
                }
                lease
                    .run(async {
                        let traversal = control.accept_control_exchange().await?;
                        fusion
                            .repair_direct_until(
                                &original,
                                traversal,
                                deadline,
                                &attempt.cancellation,
                            )
                            .await?;
                        check(&original, control)?;
                        // The requester retires this control only after consuming
                        // our final Ready, just like the existing repair exchange.
                        let _ = control.revocation_handle().closed().await;
                        Ok::<_, io::Error>(())
                    })
                    .await
                    .map_err(|cause| {
                        io::Error::other(format!("incoming TCP repair: {cause:?}"))
                    })??;
                return Ok(());
            }
            if request.batched {
                let response = core::batches::Offer {
                    offer: EndpointAddr::new(endpoint.endpoint().id()),
                    batched: true,
                };
                exchange
                    .send_message(
                        ControlMessageFormat::EndTerminated {
                            maximum: MAX_MESSAGE,
                        },
                        &encode_message(&response)?,
                    )
                    .await?;
                let frames = control.accept_control_exchange().await?;
                let result = tokio::time::timeout_at(
                    deadline,
                    lease.run(async {
                        if fused {
                            core::batches::exchange(
                                &fused_endpoint,
                                &original,
                                control,
                                &frames,
                                data_offer(endpoint),
                                request.offer,
                                &permits,
                                &check,
                                &|original| check_original(gate, original),
                            )
                            .await
                        } else {
                            core::batches::exchange(
                                endpoint.endpoint(),
                                &original,
                                control,
                                &frames,
                                data_offer(endpoint),
                                request.offer,
                                &permits,
                                &check,
                                &|original| check_original(gate, original),
                            )
                            .await
                        }
                    }),
                )
                .await
                .map_err(|_| io::ErrorKind::TimedOut)?
                .map_err(|cause| io::Error::other(format!("incoming repair: {cause:?}")))??;
                let _ = result;
                return Ok(());
            }
            lease
                .run(async {
                    if !fused && original.side().is_client() {
                        let mut path = validate(request.offer.clone()).await?;
                        path.retain_candidate()?;
                    }
                    // A peer without the batch extension still consumes the
                    // historical single bounded offer. Negotiated peers receive all.
                    let offer = core::batches::offers(&data_offer(endpoint))?.remove(0);
                    let bytes = encode_message(&offer)?;
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
                        check(&original, control)?;
                        if !permits(&path.address) {
                            return Err(io::Error::new(
                                io::ErrorKind::PermissionDenied,
                                "data permission changed during repair",
                            ));
                        }
                        path.retain_candidate()?;
                    }
                    // Keep responder authentication until the requester finishes its
                    // original-path validation and closes the private exchange.
                    let _ = control.revocation_handle().closed().await;
                    Ok(())
                })
                .await
                .map_err(|cause| io::Error::other(format!("incoming repair: {cause:?}")))?
        };
        tokio::time::timeout_at(deadline, operation)
            .await
            .map_err(|_| io::ErrorKind::TimedOut)?
    };
    tokio::time::timeout_at(maximum, serve)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "path recovery control timed out"))?
}

/// The member owner dispatches this exchange directly from the existing session.
pub(crate) async fn recover_original_exchange(
    gate: &AdmissionGate,
    endpoint: &ResourceEndpoint,
    original: &Connection,
    exchange: &iroh::endpoint::ControlExchange,
    initiator: bool,
    deadline: n0_future::time::Instant,
    cancel: CancellationToken,
) -> io::Result<ConnectionPathRecovery> {
    let work = async {
        check_original(gate, original)?;
        let offer = data_offer(endpoint);
        let started = n0_future::time::Instant::now();
        let mut exchange_deadline = deadline;
        let first = if initiator {
            let request = RepairRequest {
                binding: binding(original)?,
                offer: EndpointAddr::new(offer.id),
                require_confirmation: true,
                traverse_tcp: false,
                batched: true,
                budget_ms: Some(
                    deadline
                        .saturating_duration_since(n0_future::time::Instant::now())
                        .as_millis() as u64,
                ),
            };
            exchange
                .send_message(core::batches::FORMAT, &encode_message(&request)?)
                .await?;
            let response: core::batches::Offer =
                serde_json::from_slice(&exchange.recv_message(core::batches::FORMAT).await?)
                    .map_err(io::Error::other)?;
            if !response.batched {
                return Err(io::ErrorKind::Unsupported.into());
            }
            response.offer
        } else {
            let request: RepairRequest =
                serde_json::from_slice(&exchange.recv_message(core::batches::FORMAT).await?)
                    .map_err(io::Error::other)?;
            if request.binding != binding(original)?
                || request.offer.id != original.remote_id()
                || !request.batched
                || !request.require_confirmation
                || request.traverse_tcp
            {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
            let remaining = request.budget_ms.ok_or(io::ErrorKind::InvalidData)?;
            exchange_deadline = deadline.min(
                started
                    + std::time::Duration::from_millis(remaining)
                        .min(deadline.saturating_duration_since(started)),
            );
            exchange
                .send_message(
                    core::batches::FORMAT,
                    &encode_message(&core::batches::Offer {
                        offer: EndpointAddr::new(offer.id),
                        batched: true,
                    })?,
                )
                .await?;
            request.offer
        };
        let attempt = AttemptContext::new(exchange_deadline, cancel.clone());
        let registry = gate.maintenance();
        let reservation = if initiator {
            Reservation::Owned(registry.acquire_outgoing(original, &attempt, ()).map_err(
                |cause| io::Error::other(format!("original repair reservation: {cause:?}")),
            )?)
        } else {
            registry
                .acquire_incoming(original, &attempt)
                .map_err(|cause| {
                    io::Error::other(format!("original repair reservation: {cause:?}"))
                })?
        };
        let lease = match reservation {
            Reservation::Owned(lease) => lease,
            Reservation::Pending(_) => return Err(io::ErrorKind::WouldBlock.into()),
        };
        lease.seal_outgoing_hello()?;
        let adapter = FusedRecovery {
            endpoint: endpoint.endpoint(),
            contact: None,
        };
        tokio::time::timeout_at(
            exchange_deadline,
            lease.run(core::batches::exchange(
                &adapter,
                original,
                original,
                exchange,
                offer,
                first,
                &|address| endpoint.allows_data_path(address),
                &|original, _| check_original(gate, original),
                &|original| check_original(gate, original),
            )),
        )
        .await
        .map_err(|_| io::ErrorKind::TimedOut)?
        .map_err(|cause| io::Error::other(format!("original repair: {cause:?}")))?
    };
    tokio::select! { biased;
        _ = original.closed() => Err(io::ErrorKind::NotConnected.into()),
        _ = cancel.cancelled() => Err(io::ErrorKind::Interrupted.into()),
        result = tokio::time::timeout_at(deadline, work) => result.map_err(|_| io::ErrorKind::TimedOut)?,
    }
}
