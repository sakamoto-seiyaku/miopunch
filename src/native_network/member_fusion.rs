use std::{io, time::Duration};

use iroh::{
    Endpoint, RelayMode, SecretKey,
    endpoint::{
        Builder, Connection, ControlConfirmation, ControlConnection, ExportKeyingMaterialError,
        MemberConnection, MemberFusionAdmission, MemberFusionEndpoint, MemberFusionEvent,
        PortmapperConfig, RelayResourcePolicy, RelayResourceUse, RestrictedBuilder,
        RestrictedEndpoint, presets,
    },
};
use serde_json::json;

use super::{NativeNetwork, NodeError};
use crate::{
    admission_gate::{AdmissionGate, ProtocolScope},
    connection_coordinator::{AttemptRole, connection_diagnostic::AttemptLog},
    resources::{ResourceConfig, ResourceUse},
};

/// Ordinary admitted business over the explicitly selected native fusion endpoint.
pub const FUSED_MEMBER_ALPN: &[u8] = b"miopunch/session/member-fusion/2";

impl NativeNetwork {
    /// Bind the node's ordinary member endpoint using the persisted identity,
    /// membership and contact resource permissions.
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
        let builder =
            builder.default_connection_path_selector(resources.path_selection.selector()?);
        let builder = if resources.udp_direct_disabled {
            builder.clear_ip_transports()
        } else {
            builder
        };
        let relay_resources = resources.clone();
        let endpoint = builder
            .secret_key(SecretKey::from_bytes(self.node_key.as_bytes()))
            .portmapper_config(PortmapperConfig::Disabled)
            .relay_mode(RelayMode::Custom(relays))
            .relay_resource_policy(RelayResourcePolicy::new(move |url, usage| {
                let use_ = match usage {
                    RelayResourceUse::DataRelay => ResourceUse::DataRelay,
                    RelayResourceUse::Signaling => ResourceUse::Signaling,
                    _ => return false,
                };
                relay_resources.allows(url.as_str(), use_)
            }))
            .member_fusion(
                FUSED_MEMBER_ALPN,
                Authorization {
                    gate: self.gate.with_message_bootstrap(),
                    resources,
                    allow_custom: false,
                },
            )
            .admission_timeout(setup_budget)
            .bind(idle_budget)
            .await?;
        Ok(endpoint)
    }

    pub(super) async fn bind_shared_member_fusion(
        &self,
        builder: RestrictedBuilder,
        resources: &ResourceConfig,
    ) -> Result<(RestrictedEndpoint, MemberFusionEndpoint), iroh::endpoint::BindError> {
        self.gate
            .register_protocol(FUSED_MEMBER_ALPN, ProtocolScope::Member);
        builder
            .bind_with_member_fusion(
                FUSED_MEMBER_ALPN,
                Authorization {
                    gate: self.gate.with_message_bootstrap(),
                    resources: resources.clone(),
                    allow_custom: true,
                },
                Duration::from_millis(resources.maintenance.exploration_interval_ms),
            )
            .await
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
    if let Some((attempt, route)) = connection.diagnostic_dial() {
        log.local_id = attempt;
        log.route_phase_id = Some(route);
    }
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
struct Authorization {
    gate: AdmissionGate,
    resources: ResourceConfig,
    allow_custom: bool,
}

impl MemberFusionAdmission for Authorization {
    async fn authorize(&self, connection: &ControlConnection) -> io::Result<ControlConfirmation> {
        let log = attempt_log(connection);
        let admission = log.phase("member-admission", json!({"peer": connection.remote_id()}));
        if let Err(error) = self.gate.authenticate_fusion_member(connection).await {
            if error.kind() == io::ErrorKind::PermissionDenied {
                admission.finish("rejected");
            } else {
                admission.finish_with("failed", json!({"kind": format!("{:?}", error.kind())}));
            }
            return Err(error);
        }
        admission.finish("succeeded");

        let gate = self.gate.clone();
        Ok(ControlConfirmation::with_control(move |connection| {
            Box::pin(async move {
                if !gate.allows(connection, ProtocolScope::Member) {
                    return Err(io::Error::new(
                        if connection.close_reason().is_some() {
                            io::ErrorKind::ConnectionAborted
                        } else {
                            io::ErrorKind::PermissionDenied
                        },
                        "member admission changed",
                    ));
                }
                let path = if connection
                    .paths()
                    .iter()
                    .any(|path| path.is_selected() && path.is_relay())
                {
                    "data-relay"
                } else {
                    "data-path"
                };
                log.event(path, "qualified", serde_json::Value::Null);
                Ok(())
            })
        }))
    }

    fn is_current(&self, connection: &ControlConnection) -> bool {
        self.gate.allows(connection, ProtocolScope::Member)
    }

    fn business_released(&self, connection: &Connection) {
        self.gate.publish_business(connection);
    }

    fn permits_data_path(&self, remote: &iroh::TransportAddr) -> bool {
        match remote {
            iroh::TransportAddr::Relay(_) => true,
            iroh::TransportAddr::Ip(_) => !self.resources.udp_direct_disabled,
            iroh::TransportAddr::Custom(addr) => {
                self.allow_custom && self.resources.allows_custom(addr)
            }
            _ => false,
        }
    }

    fn custom_data_transports(&self) -> Vec<u64> {
        if self.allow_custom {
            self.gate
                .allowed_data_carriers()
                .into_iter()
                .filter(|id| !ResourceConfig::is_restricted_contact_carrier(*id))
                .collect()
        } else {
            Vec::new()
        }
    }

    fn permits_restricted_contact(&self, address: &iroh_base::CustomAddr) -> bool {
        self.resources.allows_member_contact_carrier(address)
    }

    fn selection_event(&self, connection: &ControlConnection, event: MemberFusionEvent) {
        let result = match event {
            MemberFusionEvent::ReadySent => "ready-sent",
            MemberFusionEvent::ReadyReceived => "ready-received",
            MemberFusionEvent::QualificationRequested => "qualification-requested",
            MemberFusionEvent::QualificationConfirmed => "qualification-confirmed",
            MemberFusionEvent::QualificationUnavailable => "qualification-unavailable",
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
