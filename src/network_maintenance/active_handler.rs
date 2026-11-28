use super::{AdmissionGate, NativeNetwork, ResourceEndpoint};
use iroh::protocol::ControlProtocolHandler;
use std::sync::Arc;

#[derive(Clone)]
struct ActiveImprovementHandler {
    endpoint: iroh::endpoint::RestrictedEndpoint,
    gate: AdmissionGate,
    adapter: crate::connection_coordinator::CoordinatorHook,
    _resources: Arc<ResourceEndpoint>,
}
impl std::fmt::Debug for ActiveImprovementHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ActiveImprovementHandler")
    }
}
impl ControlProtocolHandler for ActiveImprovementHandler {
    async fn accept(
        &self,
        control: iroh::endpoint::ControlConnection,
    ) -> Result<(), iroh::protocol::AcceptError> {
        if control.alpn() != crate::connection_coordinator::ACTIVE_IMPROVEMENT_ALPN
            || !self.gate.allows_member_path_upgrade(&control)
        {
            control.close(0u32.into(), b"active improvement rejected");
            return Ok(());
        }
        let endpoint = self.endpoint.clone();
        let gate = self.gate.clone();
        let maintenance = gate.maintenance();
        let lookup_maintenance = maintenance.clone();
        let lookup_gate = gate.clone();
        let adapter = self.adapter.clone();
        let Some(responder_deadline) = control.caller_deadline() else {
            control.close(0u32.into(), b"active improvement clock missing");
            return Ok(());
        };
        let result = tokio::time::timeout(
            responder_deadline.saturating_duration_since(n0_future::time::Instant::now()),
            crate::connection_coordinator::serve_active_session(
                &endpoint,
                &control.control_handle(),
                &crate::connection_coordinator::NativeControlAdapter(adapter),
                &maintenance,
                responder_deadline,
                move |peer, binding| {
                    lookup_maintenance
                        .for_binding(peer, binding)
                        .or_else(|| lookup_gate.member_connection_for_binding(peer, binding))
                },
            ),
        )
        .await;
        let result = result.unwrap_or_else(|_| {
            Err(crate::connection_coordinator::control_error(
                "DeadlineExceeded",
            ))
        });
        #[cfg(feature = "rtc-probe")]
        println!(
            "{}",
            serde_json::json!({"event":"unified-active-result",
            "monotonicMs":crate::rtc_transport::diagnostic_monotonic_ms(),
            "controlStableId":control.stable_id(),"ok":result.is_ok(),"controlOpen":control.close_reason().is_none(),
            "failureClass":result.as_ref().err().map(|error| match error.kind() {
                std::io::ErrorKind::TimedOut => "timed-out",
                std::io::ErrorKind::PermissionDenied => "permission-denied",
                std::io::ErrorKind::InvalidData => "invalid-data",
                _ => "other",
            })})
        );
        if let Err(error) = result
            && control.close_reason().is_none()
        {
            crate::connection_coordinator::close_active_failure(&control, &error);
        }
        Ok(())
    }
}

impl NativeNetwork {
    /// Install the existing active-path control handler in an application-owned
    /// restricted Router. Use the ResourceEndpoint bound by this same node and
    /// register it with `accept_control(ACTIVE_IMPROVEMENT_ALPN, handler)`.
    /// The handler keeps the resource owner alive and resolves the exact live
    /// member session through this node's authorization and exporter binding.
    /// It never hands an ordinary connection to the control protocol caller.
    pub fn active_improvement_handler(
        &self,
        resources: Arc<ResourceEndpoint>,
    ) -> impl ControlProtocolHandler + Clone + use<> {
        ActiveImprovementHandler {
            endpoint: resources.endpoint().clone(),
            gate: self.gate.clone(),
            adapter: resources.active_coordinator(self.gate.clone()),
            _resources: resources,
        }
    }
}
