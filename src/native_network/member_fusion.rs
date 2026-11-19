use std::{io, time::Duration};

use iroh::{
    Endpoint, RelayMode, SecretKey,
    endpoint::{
        Builder, Connection, ControlConfirmation, ControlConnection, ExportKeyingMaterialError,
        MemberConnection, MemberFusionAdmission, MemberFusionEndpoint, MemberFusionEvent,
        PortmapperConfig, RelayResourcePolicy, RelayResourceUse, presets,
    },
};
use serde_json::json;

use super::{NativeNetwork, NodeError};
use crate::{
    admission_gate::{AdmissionGate, ProtocolScope},
    connection_coordinator::{AttemptRole, connection_diagnostic::AttemptLog},
    resources::ResourceUse,
};

/// Ordinary admitted business over the explicitly selected native fusion endpoint.
pub const FUSED_MEMBER_ALPN: &[u8] = b"miopunch/session/member-fusion/2";

impl NativeNetwork {
    /// Bind the node's ordinary member endpoint using the persisted identity,
    /// membership and data relay permissions.
    pub async fn bind_member_fusion_endpoint(
        &self,
        setup_budget: Duration,
    ) -> Result<MemberFusionEndpoint, NodeError> {
        self.bind_member_fusion_endpoint_with(Endpoint::builder(presets::Minimal), setup_budget)
            .await
    }

    /// Bind with caller-supplied endpoint hooks and the node's fusion policy.
    pub async fn bind_member_fusion_endpoint_with(
        &self,
        builder: Builder,
        setup_budget: Duration,
    ) -> Result<MemberFusionEndpoint, NodeError> {
        let resources = self.resource_configuration()?;
        let idle_budget = Duration::from_millis(resources.maintenance.exploration_interval_ms);
        let relays = resources.relay_map(ResourceUse::DataRelay)?;
        self.gate
            .register_protocol(FUSED_MEMBER_ALPN, ProtocolScope::Member);
        let endpoint = builder
            .secret_key(SecretKey::from_bytes(self.node_key.as_bytes()))
            .clear_ip_transports()
            .portmapper_config(PortmapperConfig::Disabled)
            .relay_mode(RelayMode::Custom(relays))
            .relay_resource_policy(RelayResourcePolicy::new(move |url, usage| {
                matches!(usage, RelayResourceUse::DataRelay)
                    && resources.allows(url.as_str(), ResourceUse::DataRelay)
            }))
            .member_fusion(FUSED_MEMBER_ALPN, Authorization(self.gate.clone()))
            .admission_timeout(setup_budget)
            .bind(idle_budget)
            .await?;
        Ok(endpoint)
    }
}

/// Cross-peer diagnostic identifier derived from this authenticated QUIC attempt.
/// The TLS exporter label is reserved for diagnostics.
pub fn member_fusion_attempt_id(
    connection: &MemberConnection,
) -> Result<[u8; 16], ExportKeyingMaterialError> {
    let mut id = [0; 16];
    connection.export_keying_material(&mut id, ATTEMPT_EXPORTER_LABEL, b"")?;
    Ok(id)
}

const ATTEMPT_EXPORTER_LABEL: &[u8] = b"EXPORTER-miopunch-member-fusion-attempt-1";

fn attempt_log(connection: &ControlConnection) -> AttemptLog {
    let mut log = AttemptLog::new(AttemptRole::initial(connection), "member_fusion").with_round(1);
    let mut attempt = [0; 16];
    if connection
        .export_keying_material(&mut attempt, ATTEMPT_EXPORTER_LABEL, b"")
        .is_ok()
    {
        log = log.with_shared(attempt);
    }
    log.original = Some(connection.stable_id());
    log
}

#[derive(Debug)]
struct Authorization(AdmissionGate);

impl MemberFusionAdmission for Authorization {
    async fn authorize(&self, connection: &ControlConnection) -> io::Result<ControlConfirmation> {
        let log = attempt_log(connection);
        let admission = log.phase("member-admission", json!({"peer": connection.remote_id()}));
        if self.0.authenticate_fusion_member(connection).await.is_err() {
            admission.finish("rejected");
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "member admission rejected",
            ));
        }
        admission.finish("succeeded");

        let gate = self.0.clone();
        Ok(ControlConfirmation::with_control(move |connection| {
            Box::pin(async move {
                if !gate.allows(connection, ProtocolScope::Member) {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "member admission changed",
                    ));
                }
                log.event("data-relay", "qualified", serde_json::Value::Null);
                Ok(())
            })
        }))
    }

    fn is_current(&self, connection: &ControlConnection) -> bool {
        self.0.allows(connection, ProtocolScope::Member)
    }

    fn business_released(&self, connection: &Connection) {
        self.0.publish_business(connection);
    }

    fn selection_event(&self, connection: &ControlConnection, event: MemberFusionEvent) {
        let result = match event {
            MemberFusionEvent::ReadySent => "ready-sent",
            MemberFusionEvent::ReadyReceived => "ready-received",
            MemberFusionEvent::Committed => "commit-sent",
            MemberFusionEvent::CommitReceived => "commit-received",
            MemberFusionEvent::Superseded => "superseded",
        };
        attempt_log(connection).event(
            "member-fusion-admission-stage",
            result,
            serde_json::Value::Null,
        );
    }
}
