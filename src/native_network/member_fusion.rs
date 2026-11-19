use std::{io, time::Duration};

use iroh::{
    Endpoint, RelayMode, SecretKey,
    endpoint::{
        Connection, ControlConfirmation, ControlConnection, ExportKeyingMaterialError,
        MemberFusionAdmission, PortmapperConfig, RelayResourcePolicy, RelayResourceUse,
        RestrictedEndpoint, presets,
    },
};
use serde_json::json;

use super::{NativeNetwork, NodeError};
use crate::{
    admission_gate::{AdmissionGate, ProtocolScope},
    connection_coordinator::{AttemptRole, connection_diagnostic::AttemptLog},
    control_protocol::AuthenticatedControl,
    resources::ResourceUse,
};

/// Ordinary admitted business over the explicitly selected native fusion endpoint.
pub const FUSED_MEMBER_ALPN: &[u8] = b"miopunch/session/member-fusion/1";

impl NativeNetwork {
    /// Bind the first native fusion slice using saved data relay permissions and
    /// the durable member identity. Each connect/accept owns its setup budget.
    pub async fn bind_member_fusion_endpoint(
        &self,
        setup_budget: Duration,
    ) -> Result<RestrictedEndpoint, NodeError> {
        let resources = self.resource_configuration()?;
        let relays = resources.relay_map(ResourceUse::DataRelay)?;
        self.gate
            .register_protocol(FUSED_MEMBER_ALPN, ProtocolScope::Member);
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(SecretKey::from_bytes(self.node_key.as_bytes()))
            .alpns(vec![FUSED_MEMBER_ALPN.to_vec()])
            .clear_ip_transports()
            .portmapper_config(PortmapperConfig::Disabled)
            .relay_mode(RelayMode::Custom(relays))
            .relay_resource_policy(RelayResourcePolicy::new(move |url, usage| {
                matches!(usage, RelayResourceUse::DataRelay)
                    && resources.allows(url.as_str(), ResourceUse::DataRelay)
            }))
            .member_fusion(Authorization(self.gate.clone()))
            .admission_timeout(setup_budget)
            .bind()
            .await?;
        Ok(endpoint)
    }
}

/// Cross-peer diagnostic identifier derived from this authenticated QUIC attempt.
/// The TLS exporter label is reserved for diagnostics.
pub fn member_fusion_attempt_id(
    connection: &impl AuthenticatedControl,
) -> Result<[u8; 16], ExportKeyingMaterialError> {
    let mut id = [0; 16];
    connection.export_keying_material(
        &mut id,
        b"EXPORTER-miopunch-member-fusion-attempt-1",
        b"",
    )?;
    Ok(id)
}

#[derive(Debug)]
struct Authorization(AdmissionGate);

impl MemberFusionAdmission for Authorization {
    async fn authorize(&self, connection: &ControlConnection) -> io::Result<ControlConfirmation> {
        let mut log =
            AttemptLog::new(AttemptRole::initial(connection), "member_fusion").with_round(1);
        if let Ok(attempt) = member_fusion_attempt_id(connection) {
            log = log.with_shared(attempt);
        }
        log.original = Some(connection.stable_id());
        let admission = log.phase("member-admission", json!({"peer": connection.remote_id()}));
        if !matches!(
            self.0.authenticate(connection).await,
            Ok(ProtocolScope::Member)
        ) {
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
}
