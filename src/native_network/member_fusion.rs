use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use iroh::{
    Endpoint, RelayMode, SecretKey,
    endpoint::{
        Builder, Connection, ControlConfirmation, ControlConnection, ControlExchange,
        ControlHandle, ExportKeyingMaterialError, MemberConnection, MemberDirectCandidate,
        MemberFusionAdmission, MemberFusionEndpoint, MemberFusionEvent, MemberUdpConfig,
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
pub const FUSED_MEMBER_ALPN: &[u8] = b"miopunch/session/member-fusion/5";

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
        let standby = iroh::endpoint::MemberRetention::Standby(
            resources
                .maintenance
                .standby_member_limit
                .unwrap_or(usize::MAX),
        );
        let relays = resources.relay_map(ResourceUse::DataRelay)?;
        self.gate
            .register_protocol(FUSED_MEMBER_ALPN, ProtocolScope::Member);
        if resources.allows_member_data_relay() {
            self.gate
                .register_data_carrier(crate::member_transport::DATA_TRANSPORT_ID);
        }
        let (builder, tcp) = resources.configure_with_tcp(builder)?;
        let tcp = tcp.map(|transport| {
            crate::tcp_punch::TcpPunch::new(
                transport,
                resources.tcp_stun_resources(),
                resources.clone(),
            )
        });
        let relay_resources = resources.clone();
        let custom_resources = resources.clone();
        let observation_resources = resources.clone();
        let udp_config = self.udp.fusion_config(&resources)?;
        let endpoint = builder
            .secret_key(SecretKey::from_bytes(self.node_key.as_bytes()))
            .portmapper_config(PortmapperConfig::Disabled)
            .relay_mode(RelayMode::Custom(relays))
            .observation_servers(resources.relay_map(ResourceUse::Observation)?)
            .relay_resource_policy(
                RelayResourcePolicy::new(move |url, usage| {
                    let use_ = match usage {
                        RelayResourceUse::DataRelay => ResourceUse::DataRelay,
                        RelayResourceUse::Signaling => ResourceUse::Signaling,
                        RelayResourceUse::Observation => ResourceUse::Observation,
                        _ => return false,
                    };
                    relay_resources.allows(url.as_str(), use_)
                })
                .with_custom_addresses(move |address| custom_resources.allows_custom(address))
                .with_observation_addresses(move |url| {
                    observation_resources.observation_dial_addresses(url)
                }),
            )
            .member_fusion(
                FUSED_MEMBER_ALPN,
                Authorization {
                    gate: self.gate.with_message_bootstrap(),
                    resources,
                    allow_custom: false,
                    udp: self.udp.fusion_permission(),
                    udp_config,
                    tcp,
                },
            )
            .admission_timeout(setup_budget)
            .bind_with_retention(standby)
            .await?;
        Ok(endpoint)
    }

    pub(super) async fn bind_shared_member_fusion(
        &self,
        builder: RestrictedBuilder,
        resources: &ResourceConfig,
        tcp: Option<crate::tcp_punch::TcpPunch>,
    ) -> Result<(RestrictedEndpoint, MemberFusionEndpoint), NodeError> {
        self.gate
            .register_protocol(FUSED_MEMBER_ALPN, ProtocolScope::Member);
        Ok(builder
            .bind_with_member_retention(
                FUSED_MEMBER_ALPN,
                Authorization {
                    gate: self.gate.with_message_bootstrap(),
                    resources: resources.clone(),
                    allow_custom: true,
                    udp: self.udp.fusion_permission(),
                    udp_config: self.udp.fusion_config(resources)?,
                    tcp,
                },
                iroh::endpoint::MemberRetention::Standby(
                    resources
                        .maintenance
                        .standby_member_limit
                        .unwrap_or(usize::MAX),
                ),
            )
            .await?)
    }
}

impl crate::resources::ResourceEndpoint {
    pub(crate) fn install_member_repair(self: &Arc<Self>) {
        let (fusion, gate) = self.member_fusion.as_ref().expect("fused endpoint").clone();
        let weak_resources = Arc::downgrade(self);
        fusion.set_repair_contact(move |original, contact, deadline, cancel| {
            let resources = weak_resources.clone();
            let gate = gate.clone();
            async move {
                if let Some(resources) = resources.upgrade() {
                    use crate::connection_coordinator::{
                        AttemptRole, connection_diagnostic::AttemptLog,
                    };
                    let mut log = AttemptLog::new(AttemptRole::Initiator, "member_fusion_repair");
                    log.original = Some(original.stable_id());
                    let phase = log.phase("repair", json!({"originalSide":format!("{:?}", original.side()),
                        "budgetMs":deadline.saturating_duration_since(tokio::time::Instant::now()).as_millis()}));
                    let result = crate::connection_recovery::recover_member_path(
                        &gate, &resources, &original, contact, deadline, cancel,
                    )
                    .await;
                    match result {
                        Ok(path) => phase.finish_with("succeeded", json!({"path":path.path,
                            "pathId":path.path_id,"probeWatermark":path.probe_watermark,
                            "controlStableId":path.control_stable_id,"peerConfirmed":path.peer_confirmed})),
                        Err(error) => phase.finish_with("failed", json!({"error":error.to_string()})),
                    }
                }
            }
        });
    }

    pub(crate) async fn serve_member_repair(&self, control: &ControlConnection) -> io::Result<()> {
        let (_, gate) = self.member_fusion.as_ref().expect("fused endpoint");
        let result =
            crate::connection_recovery::serve_path_recovery(gate, self, control, true).await;
        control.close(0u32.into(), b"member path repair ended");
        result
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
    udp: Arc<AtomicBool>,
    udp_config: MemberUdpConfig,
    tcp: Option<crate::tcp_punch::TcpPunch>,
}

impl Authorization {
    async fn prepare_tcp_candidate(
        &self,
        original: &ControlHandle,
        exchange: &ControlExchange,
        bootstrap: bool,
    ) -> io::Result<MemberDirectCandidate> {
        let tcp = self.tcp.as_ref().ok_or(io::ErrorKind::Unsupported)?;
        let mut log = AttemptLog::new(AttemptRole::initial(original), "member_fusion_tcp");
        log.original = Some(original.stable_id());
        let mut candidate = tcp
            .prepare_fused(original, exchange, &self.gate, log, bootstrap)
            .await?;
        let remote = candidate.route().clone();
        let original = original.clone();
        let gate = self.gate.clone();
        Ok(MemberDirectCandidate::new(remote, move || {
            if bootstrap {
                candidate.retain_control(&original, &gate)
            } else {
                candidate.check(&original, &gate)?;
                candidate
                    .active
                    .take()
                    .expect("prepared fused TCP")
                    .retain();
                Ok(())
            }
        }))
    }
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

    fn path_retry_base(&self) -> Duration {
        Duration::from_millis(self.resources.maintenance.path_retry_base_ms)
    }

    fn udp_config(&self) -> Option<&MemberUdpConfig> {
        Some(&self.udp_config)
    }

    fn udp_scope(&self, connection: &Connection) -> Option<crate::nat_selector::LearningScope> {
        if !self.udp.load(Ordering::Acquire) || !self.gate.allows(connection, ProtocolScope::Member)
        {
            return None;
        }
        self.gate.nat_learning_scope(connection)
    }

    fn business_released(&self, connection: &Connection) {
        self.gate.publish_business(connection);
    }

    fn permits_data_path(&self, remote: &iroh::TransportAddr) -> bool {
        match remote {
            iroh::TransportAddr::Relay(_) => true,
            iroh::TransportAddr::Ip(_) => !self.resources.udp_direct_disabled,
            iroh::TransportAddr::Custom(addr) => match addr.id() {
                crate::tcp_transport::DATA_TRANSPORT_ID
                | crate::member_transport::DATA_TRANSPORT_ID => self.resources.allows_custom(addr),
                id => self.allow_custom && !ResourceConfig::is_restricted_contact_carrier(id),
            },
            _ => false,
        }
    }

    fn custom_data_transports(&self) -> Vec<u64> {
        self.gate
            .allowed_data_carriers()
            .into_iter()
            .filter(|id| match *id {
                crate::tcp_transport::DATA_TRANSPORT_ID => !self.resources.tcp_direct_disabled,
                crate::member_transport::DATA_TRANSPORT_ID => {
                    self.resources.allows_member_data_relay()
                }
                id => self.allow_custom && !ResourceConfig::is_restricted_contact_carrier(id),
            })
            .collect()
    }

    fn member_direct_transport(&self) -> Option<u64> {
        (!self.resources.tcp_direct_disabled).then_some(crate::tcp_transport::DATA_TRANSPORT_ID)
    }

    fn member_direct_traversal(&self) -> Option<(Duration, Duration)> {
        self.tcp
            .as_ref()
            .filter(|tcp| tcp.transport.supports_punching() && !tcp.observers.is_empty())
            .map(|_| {
                let policy = &self.resources.maintenance;
                (
                    Duration::from_millis(policy.contact_timeout_ms),
                    Duration::from_millis(policy.path_retry_base_ms),
                )
            })
    }

    async fn prepare_member_direct(
        &self,
        original: &ControlHandle,
        exchange: &ControlExchange,
    ) -> io::Result<MemberDirectCandidate> {
        self.prepare_tcp_candidate(original, exchange, false).await
    }

    async fn prepare_member_bootstrap_direct(
        &self,
        original: &ControlHandle,
        exchange: &ControlExchange,
    ) -> io::Result<MemberDirectCandidate> {
        self.prepare_tcp_candidate(original, exchange, true).await
    }

    fn member_relay_transport(&self) -> Option<u64> {
        self.resources
            .allows_member_data_relay()
            .then_some(crate::member_transport::DATA_TRANSPORT_ID)
    }

    fn shares_member_relay(
        &self,
        local: &iroh_base::CustomAddr,
        remote: &iroh_base::CustomAddr,
    ) -> bool {
        crate::member_transport::MemberDataTransport::shares_forwarder(local, remote)
    }

    fn permits_direct_candidate(&self, address: &iroh_base::CustomAddr) -> bool {
        self.resources.allows_custom(address)
            && crate::tcp_transport::TcpTransport::socket_addr(address)
                .is_ok_and(|addr| !addr.ip().is_unspecified())
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
