//! Native gate/candidate adapter for the common member coordinator.
use super::{
    AttemptContext, AttemptRole, MemberPathAdapter, PathPlan, Platform, PreparedPath,
    ProtectedAttempt, RELAY, RTC, RtcRole, RuntimeCapabilities, SessionPurpose, TCP, UDP,
    control_error, coordinate_session,
};
use crate::{admission_gate::AdmissionGate, control_session::ControlSession};
use iroh::{Endpoint, TransportAddr, endpoint::Connection};
use std::{future::Future, io};

#[derive(Debug, Clone)]
pub(crate) struct CoordinatorHook {
    gate: AdmissionGate,
    udp_allowed: bool,
    relay_allowed: bool,
    resources: Option<crate::resources::ResourceConfig>,
    native: Option<crate::native_path_adapter::NativePathAdapter>,
    #[cfg(feature = "rtc-probe")]
    rtc: Option<crate::rtc_transport::RtcTransport>,
}

struct CloseActiveControl(Connection);
impl Drop for CloseActiveControl {
    fn drop(&mut self) {
        self.0
            .close(0u32.into(), b"active improvement exchange ended");
    }
}
struct CancelRepair(tokio_util::sync::CancellationToken);
impl Drop for CancelRepair {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
impl CoordinatorHook {
    pub(crate) fn new(gate: AdmissionGate, udp_allowed: bool) -> Self {
        Self {
            gate,
            udp_allowed,
            relay_allowed: false,
            resources: None,
            native: None,
            #[cfg(feature = "rtc-probe")]
            rtc: None,
        }
    }
    pub(crate) fn with_resources(mut self, resources: crate::resources::ResourceConfig) -> Self {
        self.resources = Some(resources);
        self
    }
    pub(crate) fn with_native(
        mut self,
        adapter: crate::native_path_adapter::NativePathAdapter,
    ) -> Self {
        self.native = Some(adapter);
        self
    }
    pub(crate) fn with_relay(mut self, allowed: bool) -> Self {
        self.relay_allowed = allowed;
        self
    }
    #[cfg(feature = "rtc-probe")]
    pub(crate) fn with_rtc(mut self, rtc: Option<crate::rtc_transport::RtcTransport>) -> Self {
        self.rtc = rtc;
        self
    }

    /// Open one restricted active C through an existing permitted contact and
    /// improve the exact original M. C never becomes an ordinary handle.
    pub(crate) async fn improve(
        &self,
        endpoint: &Endpoint,
        original: &Connection,
        target: iroh::EndpointAddr,
        attempt: AttemptContext,
    ) -> io::Result<()> {
        let role = AttemptRole::initial(original);
        let caps = self.capabilities_for(original, role);
        let contact_ids = target
            .addrs
            .iter()
            .filter_map(|address| match address {
                TransportAddr::Custom(address) => Some(address.id()),
                _ => None,
            })
            .collect();
        let control =
            super::ConnectionCoordinator::new(endpoint.clone(), caps.permitted, contact_ids)
                .with_member_data(crate::member_transport::DATA_TRANSPORT_ID)
                .connect_alpn(target, super::ACTIVE_IMPROVEMENT_ALPN, attempt.clone())
                .await
                .map_err(|error| io::Error::other(error.to_string()))?;
        let _control = CloseActiveControl(control.clone());
        super::improve_existing(endpoint, original, &control, self, attempt).await
    }

    pub(crate) async fn repair(
        &self,
        resources: &crate::resources::ResourceEndpoint,
        original: &Connection,
        mut contact: iroh::EndpointAddr,
        caller_deadline: n0_future::time::Instant,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<(), crate::connection_recovery_core::ConnectionPathRecoveryError> {
        use crate::connection_recovery_core as recovery;
        let started = n0_future::time::Instant::now();
        if original.close_reason().is_some() {
            return Err(recovery::failed(original, "original connection closed"));
        }
        if caller_deadline <= started
            || contact.id != original.remote_id()
            || contact.addrs.len() > recovery::MAX_CANDIDATES
            || !self.gate.allows_member_path_upgrade(original)
        {
            return Err(recovery::failed(
                original,
                "repair caller/peer/current member scope differs",
            ));
        }
        let deadline = caller_deadline.min(started + recovery::REPAIR_TIMEOUT);
        let quota = resources
            .begin_contact_attempt_until(original.remote_id(), deadline)
            .map_err(|cause| recovery::failed(original, cause))?;
        let _cancel = CancelRepair(cancel.clone());
        let attempt = AttemptContext::new(deadline, cancel);
        let lease = self
            .gate
            .maintenance()
            .acquire(original, &attempt, quota)
            .map_err(|cause| {
                recovery::failed(original, format!("repair reservation: {cause:?}"))
            })?;
        contact.addrs.extend(
            original
                .paths()
                .iter()
                .map(|path| path.remote_addr().clone())
                .take(8),
        );
        if let Some(known) = resources.contacts.get_endpoint_info(original.remote_id()) {
            contact.addrs.extend(known.into_endpoint_addr().addrs);
        }
        let permits =
            |address: &TransportAddr| self.permits_repair_survival(resources, original, address);
        let surviving = super::probe_existing_data_survival(original, &permits);
        let result = lease
            .run_ref(self.improve(resources.endpoint(), original, contact, attempt))
            .await
            .map_err(|cause| io::Error::other(format!("repair attempt: {cause:?}")))
            .and_then(|result| result);
        if result.is_ok() && original.close_reason().is_none() {
            resources.reset_contact(original.remote_id());
            return Ok(());
        }
        resources.contact_failed(original.remote_id());
        let check = |original: &Connection| {
            if self.gate.allows_member_path_upgrade(original) && original.close_reason().is_none() {
                Ok(())
            } else {
                Err(control_error("IdentityOrAdmissionRejected"))
            }
        };
        let maintained = surviving(original, &permits)
            || lease
                .run_ref(super::validate_existing_data_survival(
                    resources.endpoint(),
                    original,
                    deadline,
                    &permits,
                    &check,
                ))
                .await
                .unwrap_or(false)
            || surviving(original, &permits);
        if !maintained && original.close_reason().is_none() {
            original.close(
                0u32.into(),
                b"same-session repair has no surviving data path",
            );
            original.closed().await;
        }
        Err(recovery::failed(
            original,
            result
                .err()
                .map(|cause| cause.to_string())
                .unwrap_or_else(|| "original connection closed".into()),
        ))
    }

    fn permits_repair_survival(
        &self,
        resources: &crate::resources::ResourceEndpoint,
        original: &Connection,
        address: &TransportAddr,
    ) -> bool {
        if !self.gate.allows_member_path_upgrade(original) || original.close_reason().is_some() {
            return false;
        }
        #[cfg(feature = "rtc-probe")]
        if let TransportAddr::Custom(address) = address
            && address.id() == crate::rtc_transport::TRANSPORT_ID
        {
            use iroh::endpoint::transports::CustomSender;
            return self
                .rtc
                .as_ref()
                .is_some_and(|rtc| rtc.is_data_path_qualified(address, original.stable_id()));
        }
        resources.allows_data_path(address)
    }
}
impl MemberPathAdapter for CoordinatorHook {
    #[cfg(feature = "rtc-probe")]
    fn after_actual_ready_input_fixture<'a>(
        &'a self,
        original: usize,
        control: usize,
        stream: String,
        frame: &'a [u8],
        deadline: n0_future::time::Instant,
    ) -> impl Future<Output = io::Result<()>> + Send + 'a {
        async move {
            if let Some(rtc) = &self.rtc {
                rtc.after_actual_common_ready_input_fixture(
                    original, control, stream, frame, deadline,
                )
                .await?;
            }
            Ok(())
        }
    }
    fn committed(&self, connection: &Connection, report: serde_json::Value) {
        #[cfg(feature = "rtc-probe")]
        if report["event"] == "active-improvement-committed" {
            println!(
                "{}",
                serde_json::json!({"event":"unified-active-committed",
                "stage":"common-committed","monotonicMs":crate::rtc_transport::diagnostic_monotonic_ms(),
                "stableId":connection.stable_id(),"controlStableId":report["controlStableId"],
                "originalOpen":connection.close_reason().is_none(),
                "freshAck":report["freshAck"],"bothReady":report["bothReady"]})
            );
        }
        #[cfg(not(feature = "rtc-probe"))]
        let _ = (connection, report);
    }
    fn register_original(&self, connection: &Connection) {
        self.gate.maintenance().register(connection);
    }

    fn capabilities(&self, role: AttemptRole) -> RuntimeCapabilities {
        let (rtc_supported, rtc_allowed) = {
            #[cfg(feature = "rtc-probe")]
            {
                (
                    self.rtc.is_some(),
                    self.rtc.as_ref().is_some_and(|rtc| rtc.resource_allowed()),
                )
            }
            #[cfg(not(feature = "rtc-probe"))]
            {
                (false, false)
            }
        };
        RuntimeCapabilities {
            runtime: Platform::Native,
            supported: UDP
                | RELAY
                | if self
                    .native
                    .as_ref()
                    .is_some_and(|native| native.tcp_available())
                {
                    TCP
                } else {
                    0
                }
                | if rtc_supported { RTC } else { 0 },
            permitted: if self.udp_allowed { UDP } else { 0 }
                | if self
                    .native
                    .as_ref()
                    .is_some_and(|native| native.tcp_available())
                {
                    TCP
                } else {
                    0
                }
                | if self.relay_allowed { RELAY } else { 0 }
                | if rtc_allowed { RTC } else { 0 },
            rtc_role: RtcRole::for_attempt(role, rtc_supported),
        }
    }
    fn capabilities_for(&self, connection: &Connection, role: AttemptRole) -> RuntimeCapabilities {
        let mut caps = self.capabilities(role);
        if self.existing_relay(connection).is_some() {
            caps.permitted |= RELAY;
        }
        caps
    }
    fn existing_relay(&self, connection: &Connection) -> Option<TransportAddr> {
        let resources = self.resources.as_ref()?;
        connection
            .paths()
            .iter()
            .filter(|path| match path.remote_addr() {
                TransportAddr::Relay(url) => {
                    resources.allows(url.as_str(), crate::resources::ResourceUse::DataRelay)
                }
                TransportAddr::Custom(address) => {
                    address.id() == crate::member_transport::DATA_TRANSPORT_ID
                        && address.data().len() == 48
                        && resources.allows_custom(address)
                }
                _ => false,
            })
            .max_by_key(|path| path.is_selected())
            .map(|path| path.remote_addr().clone())
    }
    fn authorization_revision(
        &self,
        connection: &Connection,
        purpose: SessionPurpose,
    ) -> io::Result<u64> {
        // A normally closed M has already lost its ephemeral proof entry.
        // That retirement is a network outcome, not evidence of bad identity.
        if connection.close_reason().is_some() {
            return Err(control_error("DataPathUnavailable"));
        }
        if purpose == SessionPurpose::AdmissionRequest {
            return self
                .gate
                .request_revision(connection)
                .ok_or_else(|| control_error("IdentityOrAdmissionRejected"));
        }
        self.gate
            .path_qualification(connection)
            .map(|q| q.revision())
            .ok_or_else(|| control_error("IdentityOrAdmissionRejected"))
    }
    fn check<'a>(
        &'a self,
        connection: &'a Connection,
        purpose: SessionPurpose,
    ) -> impl Future<Output = io::Result<()>> + Send + 'a {
        async move {
            if connection.close_reason().is_some() {
                return Err(control_error("DataPathUnavailable"));
            }
            let allowed = if purpose == SessionPurpose::AdmissionRequest {
                self.gate.allows(
                    connection,
                    crate::admission_gate::ProtocolScope::AdmissionRequest,
                )
            } else if purpose == SessionPurpose::ActiveImprovement {
                self.gate.allows_member_path_upgrade(connection)
            } else {
                self.gate
                    .allows(connection, crate::admission_gate::ProtocolScope::Member)
            };
            if allowed {
                Ok(())
            } else {
                Err(control_error("IdentityOrAdmissionRejected"))
            }
        }
    }
    fn existing_direct(&self, connection: &Connection) -> Option<(TransportAddr, u8)> {
        let paths = connection.paths();
        paths
            .iter()
            .filter_map(|path| {
                let capability = match path.remote_addr() {
                    TransportAddr::Ip(_) if self.udp_allowed => UDP,
                    TransportAddr::Custom(address)
                        if address.id() == crate::tcp_transport::DATA_TRANSPORT_ID
                            && self
                                .native
                                .as_ref()
                                .is_some_and(|native| native.tcp_available()) =>
                    {
                        TCP
                    }
                    _ => return None,
                };
                Some((path.is_selected(), path.remote_addr().clone(), capability))
            })
            .max_by_key(|(selected, _, _)| *selected)
            .map(|(_, route, capability)| (route, capability))
    }
    fn prepare<'a>(
        &'a self,
        endpoint: &'a Endpoint,
        connection: &'a Connection,
        attempt: ProtectedAttempt,
        plan: PathPlan,
    ) -> impl Future<Output = io::Result<PreparedPath>> + Send + 'a {
        async move {
            match plan {
                #[cfg(feature = "rtc-probe")]
                PathPlan::Rtc => {
                    crate::rtc_member::bootstrap_control_unified(
                        endpoint,
                        connection,
                        &self.gate,
                        self.rtc
                            .as_ref()
                            .ok_or_else(|| control_error("DataPathUnavailable"))?,
                        attempt,
                    )
                    .await
                }
                PathPlan::Native { tcp, udp } => self
                    .native
                    .as_ref()
                    .ok_or_else(|| control_error("DataPathUnavailable"))?
                    .prepare_paths(endpoint, connection, &self.gate, tcp, udp)
                    .await
                    .map_err(|error| {
                        if error.kind() == io::ErrorKind::InvalidData {
                            control_error("InvalidControl")
                        } else if !self.gate.allows_member_path_upgrade(connection) {
                            control_error("IdentityOrAdmissionRejected")
                        } else {
                            control_error("DataPathUnavailable")
                        }
                    }),
                PathPlan::ExistingRelay => {
                    let route = self
                        .existing_relay(connection)
                        .ok_or_else(|| control_error("DataPathUnavailable"))?;
                    let gate = self.gate.clone();
                    let weak = connection.weak_handle();
                    Ok(PreparedPath::candidate(connection, route, move || {
                        if weak
                            .upgrade()
                            .is_some_and(|original| gate.allows_member_path_upgrade(&original))
                        {
                            Ok(())
                        } else {
                            Err(control_error("IdentityOrAdmissionRejected"))
                        }
                    }))
                }
                PathPlan::Relay => {
                    crate::message_bootstrap::prepare_relay(
                        endpoint,
                        connection,
                        &self.gate,
                        self.resources
                            .as_ref()
                            .ok_or_else(|| control_error("DataPathUnavailable"))?,
                    )
                    .await?;
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
                    let _ = attempt;
                    Err(control_error("DataPathUnavailable"))
                }
            }
        }
    }

    fn prepare_active<'a>(
        &'a self,
        endpoint: &'a Endpoint,
        original: &'a Connection,
        control: &'a Connection,
        attempt: ProtectedAttempt,
        plan: PathPlan,
    ) -> impl Future<Output = io::Result<PreparedPath>> + Send + 'a {
        async move {
            match plan {
                #[cfg(feature = "rtc-probe")]
                PathPlan::Rtc => {
                    crate::rtc_member::bootstrap_control_active(
                        endpoint,
                        original,
                        control,
                        &self.gate,
                        self.rtc
                            .as_ref()
                            .ok_or_else(|| control_error("DataPathUnavailable"))?,
                        attempt,
                    )
                    .await
                }
                // Native punching still owns its historical ControlSession
                // stream on M; keep it out of Active C until that adapter has
                // a C-aware exchange rather than silently reusing M.
                _ => Err(control_error("DataPathUnavailable")),
            }
        }
    }
}
impl iroh::endpoint::EndpointHooks for CoordinatorHook {
    async fn after_handshake(
        &self,
        connection: &Connection,
    ) -> iroh::endpoint::AfterHandshakeOutcome {
        if SessionPurpose::from_alpn(connection.alpn()) == Some(SessionPurpose::ActiveImprovement) {
            return iroh::endpoint::AfterHandshakeOutcome::AcceptRestricted;
        }
        if SessionPurpose::from_alpn(connection.alpn()) != Some(SessionPurpose::AdmissionRequest) {
            return iroh::endpoint::AfterHandshakeOutcome::Accept;
        }
        if coordinate_session(None, connection, self).await.is_err() {
            return iroh::endpoint::AfterHandshakeOutcome::Reject {
                error_code: 0u32.into(),
                reason: b"unified restricted purpose rejected".to_vec(),
            };
        }
        iroh::endpoint::AfterHandshakeOutcome::AcceptRestricted
    }
    async fn before_data_release(
        &self,
        endpoint: &Endpoint,
        connection: &Connection,
    ) -> io::Result<()> {
        if matches!(
            SessionPurpose::from_alpn(connection.alpn()),
            Some(SessionPurpose::AdmissionRequest | SessionPurpose::ActiveImprovement)
        ) {
            return Err(control_error("InvalidControl"));
        }
        coordinate_session(Some(endpoint), connection, self).await
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
