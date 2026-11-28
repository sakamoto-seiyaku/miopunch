//! Native gate/candidate adapter for the common member coordinator.
use crate::connection_coordinator::ControlFailure;
use super::{
    AttemptContext, AttemptRole, BackgroundRelayRace, MemberPathAdapter, PathPlan, Platform,
    PreparedPath, ProtectedAttempt, RELAY, RTC, RepairFailure, RtcRole, RuntimeCapabilities,
    SessionPurpose, TCP, UDP,
    connection_diagnostic::{self, AttemptLog},
};
use crate::admission_gate::AdmissionGate;
use crate::control_protocol::AuthenticatedControl;
use crate::resources::ContactClass;
use crate::session_maintenance::{CommittedPath, Completion, Reservation};
use iroh::{Endpoint, TransportAddr, endpoint::Connection};
use serde_json::json;
use std::io;

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

struct CloseActiveControl(crate::control_protocol::ControlOwner);
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

    pub(crate) fn improvement_capabilities(&self) -> u8 {
        let caps = self.capabilities(AttemptRole::Initiator);
        (caps.permitted & RTC)
            | if self
                .native
                .as_ref()
                .is_some_and(|native| native.udp_available())
            {
                UDP
            } else {
                0
            }
            | if self
                .native
                .as_ref()
                .is_some_and(|native| native.tcp_improvement_available())
            {
                TCP
            } else {
                0
            }
    }

    pub(crate) fn udp_round_wait_until(
        &self,
        now: n0_future::time::Instant,
    ) -> Option<n0_future::time::Instant> {
        self.native.as_ref()?.udp_round_wait_until(now)
    }

    pub(crate) fn has_qualified_direct(
        &self,
        resources: &crate::resources::ResourceEndpoint,
        original: &Connection,
    ) -> bool {
        original.paths().iter().any(|path| {
            let direct = match path.remote_addr() {
                TransportAddr::Ip(_) => true,
                #[cfg(feature = "rtc-probe")]
                TransportAddr::Custom(address)
                    if address.id() == crate::rtc_transport::TRANSPORT_ID =>
                {
                    true
                }
                TransportAddr::Custom(address) => {
                    address.id() == crate::tcp_transport::DATA_TRANSPORT_ID
                        && self.native.as_ref().is_some_and(|native| {
                            native.tcp_path_qualified(address, original.stable_id())
                        })
                }
                _ => false,
            };
            direct && self.permits_repair_survival(resources, original, path.remote_addr())
        })
    }

    /// Open one restricted active C through an existing permitted contact and
    /// improve the exact original M. C never becomes an ordinary handle.
    pub(crate) fn improve<'a>(
        &'a self,
        endpoint: &'a iroh::endpoint::RestrictedEndpoint,
        original: &'a Connection,
        target: iroh::EndpointAddr,
        attempt: AttemptContext,
        seal_hello: impl FnOnce() -> io::Result<()> + 'a,
    ) -> impl Future<Output = io::Result<CommittedPath>> + 'a {
        let mut log = AttemptLog::new(AttemptRole::Initiator, "maintenance_improvement");
        log.original = Some(original.stable_id());
        let preferred = original
            .paths()
            .iter()
            .find(|path| {
                path.is_selected() && path.is_relay() && target.addrs.contains(path.remote_addr())
            })
            .map(|path| path.remote_addr().clone());
        self.improve_logged(
            endpoint,
            original,
            target,
            attempt,
            log,
            Some(BackgroundRelayRace { preferred }),
            seal_hello,
        )
    }

    async fn improve_logged(
        &self,
        endpoint: &iroh::endpoint::RestrictedEndpoint,
        original: &Connection,
        target: iroh::EndpointAddr,
        attempt: AttemptContext,
        log: AttemptLog,
        race: Option<BackgroundRelayRace>,
        seal_hello: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<CommittedPath> {
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
        let control = super::ConnectionCoordinator::new_restricted(
            endpoint.clone(),
            caps.permitted,
            contact_ids,
        )
        .with_member_data(crate::member_transport::DATA_TRANSPORT_ID)
        .with_background_relay_race(race)
        .connect_active_control(target, attempt.clone(), log)
        .await
        .map_err(|error| io::Error::other(error.to_string()))?;
        let _control = CloseActiveControl(control.clone());
        super::improve_existing(
            endpoint,
            original,
            &control
                .restricted_handle()
                .ok_or_else(|| ControlFailure::InvalidControl.into_io())?,
            &NativeControlAdapter(self.clone()),
            attempt,
            log,
            seal_hello,
        )
        .await
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
        let mut log = AttemptLog::new(AttemptRole::Initiator, "foreground_repair");
        log.original = Some(original.stable_id());
        if let Some(reason) = original.close_reason() {
            log.connection_ended(&reason, None);
            return Err(recovery::failed(original, "original connection closed"));
        }
        if caller_deadline <= started
            || contact.id != original.remote_id()
            || !self.gate.allows_member_path_upgrade(original)
        {
            return Err(recovery::failed(
                original,
                "repair caller/peer/current member scope differs",
            ));
        }
        let deadline = caller_deadline;
        let _cancel = CancelRepair(cancel.clone());
        let attempt = AttemptContext::new(deadline, cancel);
        let phase = log.phase(
            "repair",
            json!({"budgetMs":deadline.duration_since(started).as_millis() as u64}),
        );
        let registry = self.gate.maintenance();
        let permits =
            |address: &TransportAddr| self.permits_repair_survival(resources, original, address);
        let reservation_error =
            |cause| recovery::failed(original, format!("repair reservation: {cause:?}"));
        let mut lease = match registry
            .acquire_or_wait(original, &attempt, ())
            .map_err(reservation_error)?
        {
            Reservation::Owned(lease) => lease,
            Reservation::Pending(wait) => {
                if let Some(Completion::Active(path)) =
                    wait.wait().await.map_err(reservation_error)?
                    && path.is_open(original)
                    && permits(&path.address)
                    && self
                        .authorization_revision(original, SessionPurpose::MemberBusiness)
                        .is_ok_and(|revision| revision == path.revision)
                {
                    phase.finish("reused");
                    return Ok(());
                }
                registry
                    .acquire(original, &attempt, ())
                    .map_err(reservation_error)?
            }
        };
        let quota = resources
            .begin_contact_attempt_until(original.remote_id(), ContactClass::Foreground, deadline)
            .map_err(|cause| recovery::failed(original, cause))?;
        contact.addrs.extend(
            original
                .paths()
                .iter()
                .map(|path| path.remote_addr().clone()),
        );
        if let Some(known) = resources.contacts.get_endpoint_info(original.remote_id()) {
            contact.addrs.extend(known.into_endpoint_addr().addrs);
        }
        let result = lease
            .run_ref(async {
                let mut failure = io::ErrorKind::NotConnected.into();
                for target in recovery::contact_targets(&contact) {
                    let end = recovery::contact_deadline(&target, deadline);
                    let context = AttemptContext::new(end, attempt.cancellation.child_token());
                    let _cancel = CancelRepair(context.cancellation.clone());
                    match self
                        .improve_logged(
                            resources.endpoint(),
                            original,
                            target,
                            context,
                            log,
                            None,
                            || Ok(()),
                        )
                        .await
                    {
                        Ok(path) => return Ok(path),
                        Err(error)
                            if matches!(
                                error.kind(),
                                io::ErrorKind::PermissionDenied | io::ErrorKind::Interrupted
                            ) =>
                        {
                            return Err(error);
                        }
                        Err(error) => failure = error,
                    }
                    if n0_future::time::Instant::now() >= deadline {
                        break;
                    }
                }
                Err(failure)
            })
            .await
            .map_err(|cause| io::Error::other(format!("repair attempt: {cause:?}")))
            .and_then(|result| result);
        let failure = match result {
            Ok(path) if original.close_reason().is_none() => {
                quota.succeeded();
                lease.completed(Completion::Active(path));
                phase.finish("succeeded");
                return Ok(());
            }
            Ok(_) => io::Error::other("original connection closed"),
            Err(cause) => cause,
        };
        let outcome = super::settle_failed_repair(
            original,
            &attempt,
            &failure,
            permits,
            log,
            "native_sdk_repair",
        );
        if matches!(outcome, RepairFailure::Deferred) {
            phase.finish("deferred");
        } else {
            phase.finish_with(
                "failed",
                json!({"originalOpen":original.close_reason().is_none()}),
            );
        }
        Err(recovery::failed(original, failure))
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
        if !resources.allows_data_path(address) {
            return false;
        }
        if let TransportAddr::Custom(remote) = address
            && remote.id() == crate::member_transport::DATA_TRANSPORT_ID
        {
            // MID1's published local address exists only during an attached
            // forwarding lease and names the same forwarder as the peer route.
            return resources.endpoint().custom_addrs().iter().any(|local| {
                crate::member_transport::MemberDataTransport::shares_forwarder(local, remote)
            });
        }
        true
    }

    #[cfg(feature = "rtc-probe")]
    pub(crate) fn settle_rtc_resource_withdrawal(
        &self,
        resources: &crate::resources::ResourceEndpoint,
        originals: &[usize],
    ) {
        for original in self.gate.maintenance().live() {
            if originals.contains(&original.stable_id()) {
                super::settle_path_withdrawal(
                    &original,
                    |address| self.permits_repair_survival(resources, &original, address),
                    "native_sdk_path_policy",
                    "rtc_resource_withdrawn",
                );
            }
        }
    }
}

impl CoordinatorHook {
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
            permitted: if self
                .native
                .as_ref()
                .map_or(self.udp_allowed, |native| native.udp_available())
            {
                UDP
            } else {
                0
            } | if self
                .native
                .as_ref()
                .is_some_and(|native| native.tcp_available())
            {
                TCP
            } else {
                0
            } | if self.relay_allowed { RELAY } else { 0 }
                | if rtc_allowed { RTC } else { 0 },
            rtc_role: RtcRole::for_attempt(role, rtc_supported),
        }
    }
    fn capabilities_for(
        &self,
        connection: &(impl crate::control_protocol::AuthenticatedControl + ?Sized),
        role: AttemptRole,
    ) -> RuntimeCapabilities {
        let mut caps = self.capabilities(role);
        if self.existing_relay(connection).is_some() {
            caps.permitted |= RELAY;
        }
        if self
            .existing_direct(connection)
            .is_some_and(|(_, carrier)| carrier == TCP)
        {
            caps.supported |= TCP;
            caps.permitted |= TCP;
        }
        caps
    }
    fn existing_relay(
        &self,
        connection: &(impl crate::control_protocol::AuthenticatedControl + ?Sized),
    ) -> Option<TransportAddr> {
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
        connection: &(impl crate::control_protocol::AuthenticatedControl + ?Sized),
        purpose: SessionPurpose,
    ) -> io::Result<u64> {
        // A normally closed M has already lost its ephemeral proof entry.
        // That retirement is a network outcome, not evidence of bad identity.
        if connection.close_reason().is_some() {
            return Err(ControlFailure::DataPathUnavailable.into_io());
        }
        if purpose == SessionPurpose::AdmissionRequest {
            return self
                .gate
                .request_revision(connection)
                .ok_or_else(|| ControlFailure::IdentityOrAdmissionRejected.into_io());
        }
        self.gate
            .path_qualification(connection)
            .map(|q| q.revision())
            .ok_or_else(|| ControlFailure::IdentityOrAdmissionRejected.into_io())
    }
    fn check<'a>(
        &'a self,
        connection: &'a (impl crate::control_protocol::AuthenticatedControl + ?Sized),
        purpose: SessionPurpose,
    ) -> impl Future<Output = io::Result<()>> + Send + 'a {
        async move {
            if connection.close_reason().is_some() {
                return Err(ControlFailure::DataPathUnavailable.into_io());
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
                Err(ControlFailure::IdentityOrAdmissionRejected.into_io())
            }
        }
    }
    fn existing_direct(
        &self,
        connection: &(impl crate::control_protocol::AuthenticatedControl + ?Sized),
    ) -> Option<(TransportAddr, u8)> {
        let paths = connection.paths();
        paths
            .iter()
            .filter_map(|path| {
                let capability = match path.remote_addr() {
                    TransportAddr::Ip(_) if self.udp_allowed => UDP,
                    TransportAddr::Custom(address)
                        if address.id() == crate::tcp_transport::DATA_TRANSPORT_ID
                            && (self
                                .native
                                .as_ref()
                                .is_some_and(|native| native.tcp_available())
                                || self
                                    .resources
                                    .as_ref()
                                    .is_some_and(|resources| resources.allows_custom(address))) =>
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
    fn committed(
        &self,
        connection: &(impl crate::control_protocol::AuthenticatedControl + ?Sized),
        report: serde_json::Value,
    ) {
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
        if report["event"] == "active-improvement-committed" {
            let mut report = report;
            report["originalOpen"] = serde_json::json!(connection.close_reason().is_none());
            println!("{report}");
        }
    }
}
/// Typed Native admission while retaining the existing application wire.
#[derive(Debug, Clone)]
pub(crate) struct NativeAdmission {
    gate: AdmissionGate,
    coordinator: CoordinatorHook,
    tcp: Option<crate::tcp_punch::TcpPunch>,
}
impl NativeAdmission {
    pub(crate) fn new(
        gate: AdmissionGate,
        coordinator: CoordinatorHook,
        tcp: Option<crate::tcp_punch::TcpPunch>,
    ) -> Self {
        Self {
            gate,
            coordinator,
            tcp,
        }
    }

    fn for_gate(&self, gate: AdmissionGate) -> Self {
        let mut coordinator = self.coordinator.clone();
        coordinator.gate = gate.clone();
        Self::new(gate, coordinator, self.tcp.clone())
    }

    async fn admit_registered_member(
        &self,
        connection: &iroh::endpoint::ControlConnection,
    ) -> io::Result<iroh::endpoint::AdmissionOutcome> {
        if let Some(tcp) = &self.tcp
            && connection.paths().iter().any(|path| {
                matches!(path.remote_addr(), TransportAddr::Custom(address)
                    if address.id() == crate::tcp_transport::DATA_TRANSPORT_ID)
            })
        {
            let control =
                crate::control_session::ControlSession::from_control(connection.control_handle());
            let mut cancellation = control.cancellation_guard();
            tcp.retain_existing_control(connection).await?;
            if !self.gate.allows_member_path_upgrade(connection) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "TCP membership changed before data release",
                ));
            }
            cancellation.disarm();
        }
        self.gate
            .admit_member_control(connection, |remote| match remote {
                TransportAddr::Ip(_) => self.coordinator.udp_allowed,
                TransportAddr::Relay(url) => {
                    self.coordinator.relay_allowed
                        && self
                            .coordinator
                            .resources
                            .as_ref()
                            .is_some_and(|resources| {
                                resources
                                    .allows(url.as_str(), crate::resources::ResourceUse::DataRelay)
                            })
                }
                TransportAddr::Custom(address) => self
                    .coordinator
                    .resources
                    .as_ref()
                    .is_some_and(|resources| resources.allows_custom(address)),
                _ => false,
            })
            .await
    }
}
/// The restricted adapter uses the shared negotiation and fresh-probe logic.
pub(crate) struct NativeControlAdapter(pub(crate) CoordinatorHook);
impl MemberPathAdapter<iroh::endpoint::ControlHandle> for NativeControlAdapter {
    fn capabilities(&self, role: AttemptRole) -> RuntimeCapabilities {
        self.0.capabilities(role)
    }
    fn capabilities_for(
        &self,
        connection: &iroh::endpoint::ControlHandle,
        role: AttemptRole,
    ) -> RuntimeCapabilities {
        self.0.capabilities_for(connection, role)
    }
    fn existing_direct(
        &self,
        connection: &iroh::endpoint::ControlHandle,
    ) -> Option<(TransportAddr, u8)> {
        self.0.existing_direct(connection)
    }
    fn existing_relay(&self, connection: &iroh::endpoint::ControlHandle) -> Option<TransportAddr> {
        self.0.existing_relay(connection)
    }
    fn authorization_revision(
        &self,
        connection: &iroh::endpoint::ControlHandle,
        purpose: SessionPurpose,
    ) -> io::Result<u64> {
        self.0.authorization_revision(connection, purpose)
    }
    async fn check(
        &self,
        connection: &iroh::endpoint::ControlHandle,
        purpose: SessionPurpose,
    ) -> io::Result<()> {
        self.0.check(connection, purpose).await
    }
    fn committed(&self, connection: &iroh::endpoint::ControlHandle, report: serde_json::Value) {
        self.0.committed(connection, report)
    }
    fn active_diagnostic(
        &self,
        original: &iroh::endpoint::ControlHandle,
        control: &iroh::endpoint::ControlHandle,
        stage: &'static str,
    ) {
        println!(
            "[DEBUG-udp-handoff] {}",
            serde_json::json!({"stage":stage,
            "stableId":original.stable_id(),"controlStableId":control.stable_id(),
            "originalClosed":connection_diagnostic::close_reason(original.close_reason().as_ref()),
            "controlClosed":connection_diagnostic::close_reason(control.close_reason().as_ref())})
        );
    }
    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    async fn after_actual_ready_input_fixture(
        &self,
        original: usize,
        control: usize,
        stream: String,
        frame: &[u8],
        deadline: n0_future::time::Instant,
    ) -> io::Result<()> {
        if let Some(rtc) = &self.0.rtc {
            rtc.after_actual_common_ready_input_fixture(original, control, stream, frame, deadline)
                .await?;
        }
        Ok(())
    }
    async fn prepare(
        &self,
        _endpoint: Option<&Endpoint>,
        connection: &iroh::endpoint::ControlHandle,
        attempt: ProtectedAttempt,
        plan: PathPlan,
    ) -> io::Result<PreparedPath> {
        // Keep the transport negotiations off the polling thread's stack. A relay
        // candidate uses the same outer future as a full RTC or native traversal.
        match plan {
            #[cfg(feature = "rtc-probe")]
            PathPlan::Rtc => {
                Box::pin(crate::rtc_member::bootstrap_control_unified_handle(
                    connection,
                    &self.0.gate,
                    self.0
                        .rtc
                        .as_ref()
                        .ok_or_else(|| ControlFailure::DataPathUnavailable.into_io())?,
                    attempt,
                ))
                .await
            }
            PathPlan::ExistingRelay => {
                let route = self
                    .0
                    .existing_relay(connection)
                    .ok_or_else(|| ControlFailure::DataPathUnavailable.into_io())?;
                let qualification = self
                    .0
                    .gate
                    .path_qualification(connection)
                    .ok_or_else(|| ControlFailure::IdentityOrAdmissionRejected.into_io())?;
                let weak = connection.weak_handle();
                Ok(PreparedPath::candidate(connection, route, move || {
                    if weak.is_open() && qualification.is_current() {
                        Ok(())
                    } else {
                        Err(ControlFailure::IdentityOrAdmissionRejected.into_io())
                    }
                }))
            }
            PathPlan::Relay => {
                crate::message_bootstrap::prepare_relay_control(
                    connection,
                    &self.0.gate,
                    self.0
                        .resources
                        .as_ref()
                        .ok_or_else(|| ControlFailure::DataPathUnavailable.into_io())?,
                    attempt.log,
                )
                .await?;
                let route = self
                    .0
                    .existing_relay(connection)
                    .ok_or_else(|| ControlFailure::DataPathUnavailable.into_io())?;
                let qualification = self
                    .0
                    .gate
                    .path_qualification(connection)
                    .ok_or_else(|| ControlFailure::IdentityOrAdmissionRejected.into_io())?;
                let weak = connection.weak_handle();
                Ok(PreparedPath::candidate(connection, route, move || {
                    if weak.is_open() && qualification.is_current() {
                        Ok(())
                    } else {
                        Err(ControlFailure::IdentityOrAdmissionRejected.into_io())
                    }
                }))
            }
            PathPlan::Native { tcp, udp } => {
                let native = self.0.native.as_ref()
                    .ok_or_else(|| ControlFailure::DataPathUnavailable.into_io())?;
                Box::pin(native.prepare_control_paths(connection, &self.0.gate, tcp, udp, attempt))
                    .await
            }
            _ => Err(ControlFailure::DataPathUnavailable.into_io()),
        }
    }
    async fn prepare_active(
        &self,
        _endpoint: Option<&Endpoint>,
        original: &iroh::endpoint::ControlHandle,
        control: &iroh::endpoint::ControlHandle,
        attempt: ProtectedAttempt,
        plan: PathPlan,
    ) -> io::Result<PreparedPath> {
        self.0.gate.with_direct_candidates(original, |progress| {
            progress.active_enhanced_only = Some(false);
        });
        match plan {
            PathPlan::Native { tcp, udp } => {
                println!(
                    "{}",
                    serde_json::json!({"event":"unified-native-active-start",
                    "stableId":original.stable_id(),"controlStableId":control.stable_id(),
                    "udp":udp,"tcp":tcp})
                );
                let native = self.0.native.as_ref()
                    .ok_or_else(|| ControlFailure::DataPathUnavailable.into_io())?;
                let mut prepared = Box::pin(native.prepare_active_paths(
                    original, control, &self.0.gate, tcp, udp, attempt,
                )).await?;
                prepared.log = Some(attempt.log);
                attempt
                    .log
                    .phase("fresh-ack", json!({"pathId":prepared.path_id}))
                    .run(async {
                        let paths = original.paths();
                        self.active_diagnostic(original, control, "native-candidate-prepared");
                        let path = paths
                            .iter()
                            .find(|path| {
                                path.remote_addr() == &prepared.route
                                    && prepared
                                        .path_id
                                        .as_ref()
                                        .is_none_or(|id| path.id().to_string() == *id)
                            })
                            .ok_or_else(|| ControlFailure::DataPathUnavailable.into_io())?;
                        let probe = path.start_scoped_probe()?;
                        self.active_diagnostic(original, control, "native-fresh-probe-started");
                        prepared = prepared.with_proof(
                            path.id().to_string(),
                            probe.watermark(),
                            move || probe.is_acknowledged().map_err(io::Error::other),
                        );
                        // The surrounding active attempt owns the deadline/cancellation.
                        // READY may describe only a freshly acknowledged path on M.
                        while !prepared.acknowledged.as_ref().unwrap()()? {
                            prepared.check(original, false)?;
                            self.check(control, SessionPurpose::ActiveImprovement)
                                .await?;
                            n0_future::time::sleep(std::time::Duration::from_millis(1)).await;
                        }
                        self.active_diagnostic(
                            original,
                            control,
                            "native-fresh-probe-acknowledged",
                        );
                        Ok(prepared)
                    })
                    .await
            }
            #[cfg(feature = "rtc-probe")]
            PathPlan::Rtc => {
                Box::pin(crate::rtc_member::bootstrap_control_active_handles(
                    original,
                    control,
                    &self.0.gate,
                    self.0
                        .rtc
                        .as_ref()
                        .ok_or_else(|| ControlFailure::DataPathUnavailable.into_io())?,
                    attempt,
                ))
                .await
            }
            _ => Err(ControlFailure::DataPathUnavailable.into_io()),
        }
    }
}
impl NativeAdmission {
    async fn admit_connection(
        &self,
        connection: &iroh::endpoint::ControlConnection,
    ) -> io::Result<iroh::endpoint::AdmissionOutcome> {
        let purpose = SessionPurpose::from_alpn(connection.alpn());
        let admission = (purpose == Some(SessionPurpose::ActiveImprovement)).then(|| {
            let handle = connection.control_handle();
            let mut log =
                AttemptLog::new(AttemptRole::initial(&handle), "active_control_admission");
            log.control = Some(handle.stable_id());
            if let Some((attempt, route)) = handle.diagnostic_dial() {
                log.local_id = attempt;
                log.route_phase_id = Some(route);
            }
            log.phase("control-admission", serde_json::Value::Null)
        });
        let scope = match self.gate.authenticate(connection).await {
            Ok(scope) => scope,
            Err(()) => {
                if let Some(phase) = admission {
                    phase.finish("rejected");
                }
                return Err(ControlFailure::IdentityOrAdmissionRejected.into_io());
            }
        };
        if purpose.is_none() && scope == crate::admission_gate::ProtocolScope::Member {
            return self.admit_registered_member(connection).await;
        }
        if purpose == Some(SessionPurpose::ActiveImprovement)
            || purpose.is_none() && scope != crate::admission_gate::ProtocolScope::Member
        {
            // Authentication already completed the established bilateral stage
            // exchange. These protocol handlers continue on bounded control;
            // adding another readiness frame would change their existing wire.
            let gate = self.gate.clone();
            return Ok(
                iroh::endpoint::AdmissionOutcome::ControlOnlyWithConfirmation(
                    iroh::endpoint::ControlConfirmation::with_control(move |control| {
                        Box::pin(async move {
                            let allowed = gate.allows(control, scope);
                            if let Some(phase) = admission {
                                phase.finish(if allowed { "succeeded" } else { "rejected" });
                            }
                            if allowed {
                                Ok(())
                            } else {
                                Err(ControlFailure::IdentityOrAdmissionRejected.into_io())
                            }
                        })
                    }),
                ),
            );
        }
        let purpose = SessionPurpose::from_alpn(connection.alpn())
            .ok_or_else(|| ControlFailure::ProtocolUnsupported.into_io())?;
        if !matches!(
            (scope, purpose),
            (
                crate::admission_gate::ProtocolScope::AdmissionRequest,
                SessionPurpose::AdmissionRequest
            ) | (
                crate::admission_gate::ProtocolScope::Member,
                SessionPurpose::MemberBusiness | SessionPurpose::Replica
            )
        ) {
            return Err(ControlFailure::ProtocolUnsupported.into_io());
        }
        let adapter = NativeControlAdapter(self.coordinator.clone());
        let handle = connection.control_handle();
        let prepared = super::prepare_control_session(None, &handle, &adapter).await?;
        prepared.into_outcome(connection, || self.gate.allowed_data_carriers(), |prepared| {
            iroh::endpoint::ControlConfirmation::with_control(move |connection| {
                Box::pin(async move {
                    prepared.confirm(&connection.control_handle(), &adapter).await
                })
            })
        }).await
    }
}

/// One outgoing operation in the other network uses the current endpoint's
/// physical sockets and relay registration while authenticating in its own scope.
#[derive(Debug)]
pub(crate) struct MigrationWireContext {
    peer: iroh::EndpointId,
    alpn: &'static [u8],
    gate: AdmissionGate,
}

impl MigrationWireContext {
    pub(crate) fn new(peer: iroh::EndpointId, alpn: &'static [u8], gate: &AdmissionGate) -> Self {
        Self {
            peer,
            alpn,
            gate: gate.with_request_coordinator().with_message_bootstrap(),
        }
    }

    fn matches(&self, peer: iroh::EndpointId, alpn: &[u8]) -> bool {
        self.peer == peer && self.alpn == alpn
    }
}

impl iroh::endpoint::EndpointAdmission for NativeAdmission {
    async fn admit(
        &self,
        connection: &iroh::endpoint::ControlConnection,
    ) -> io::Result<iroh::endpoint::AdmissionOutcome> {
        #[cfg(feature = "nat-lab-test-hooks")]
        eprintln!("NATIVE_CONTROL_IDENTITY {}", json!({
            "alpn": String::from_utf8_lossy(connection.alpn()),
            "side": format!("{:?}", connection.side()),
            "local": connection.id().to_string(),
            "peer": connection.remote_id().to_string(),
            "stable": connection.stable_id(),
        }));
        if let Some(context) = connection.application_context::<MigrationWireContext>() {
            if !connection.side().is_client() || !context.matches(connection.remote_id(), connection.alpn()) {
                return Err(ControlFailure::IdentityOrAdmissionRejected.into_io());
            }
            return self.for_gate(context.gate.clone()).admit_connection(connection).await;
        }
        self.admit_connection(connection).await
    }

    fn is_current(&self, connection: &iroh::endpoint::ControlConnection) -> bool {
        let gate = connection.application_context::<MigrationWireContext>()
            .map_or(&self.gate, |context| &context.gate);
        gate.authenticated_control_current(connection)
    }
    fn business_released(&self, connection: &Connection) {
        let gate = connection.application_context::<MigrationWireContext>()
            .map_or(&self.gate, |context| &context.gate);
        gate.publish_business(connection);
        if connection.alpn() == super::MEMBER_ALPN {
            gate.maintenance().register(connection);
        }
    }
}

/// Preserve the original pre-connect veto without installing a full-connection
/// admission hook on a restricted endpoint.
#[derive(Debug, Clone)]
pub(crate) struct AdmissionBeforeConnect(pub(crate) AdmissionGate);
impl iroh::endpoint::EndpointHooks for AdmissionBeforeConnect {
    async fn before_connect(
        &self,
        remote: &iroh::EndpointAddr,
        alpn: &[u8],
    ) -> iroh::endpoint::BeforeConnectOutcome {
        self.0.before_connect(remote, alpn)
    }

    async fn before_connect_with_context(
        &self,
        remote: &iroh::EndpointAddr,
        alpn: &[u8],
        context: Option<&(dyn std::any::Any + Send + Sync)>,
    ) -> iroh::endpoint::BeforeConnectOutcome {
        let Some(context) = context.and_then(|context| context.downcast_ref::<MigrationWireContext>()) else {
            return self.0.before_connect(remote, alpn);
        };
        if !context.matches(remote.id, alpn) {
            return iroh::endpoint::BeforeConnectOutcome::Reject;
        }
        context.gate.before_connect(remote, alpn)
    }
}
