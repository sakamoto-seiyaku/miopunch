//! One native startup recovery operation; candidate hints use the endpoint's
//! address book and contact budget, then the existing durable LogSync consumer.
use crate::{
    admission_gate::ProtocolScope,
    mqtt_discovery::{self, TaggedCandidate},
    mqtt_transport::{self, TRANSPORT_ID},
    native_network::{NativeNetwork, NodeError},
    offline_replica::{ReplicaError, ReplicaOutcome},
    resources::ResourceEndpoint,
};
use futures_util::StreamExt;
use iroh::{EndpointAddr, TransportAddr, endpoint::Connection};
use std::{
    net::{Ipv4Addr, SocketAddr},
    time::Duration,
};

pub const RECOVERY_REPLICA_ALPN: &[u8] = b"miopunch/recovery-replica/1";
const PATH_TIMEOUT: Duration = Duration::from_secs(12);

#[derive(Debug)]
pub enum RecoveryStatus {
    /// This source's required history has been verified, saved and applied;
    /// both endpoint identities are current members. No global freshness claim.
    Ready(ReplicaOutcome),
    /// No qualified member/path yet; retry the startup operation
    /// after backoff, new clues or a resource/network change.
    Waiting,
    /// Transport has finished and records are saved, but some source-scope
    /// records remain unapplied. A later exchange can provide causal dependencies.
    WaitingForDependencies(ReplicaOutcome),
}

pub struct NetworkRecovery {
    pub resources: ResourceEndpoint,
    pub status: RecoveryStatus,
}

async fn broker_address(resource: &iroh::RelayUrl) -> std::io::Result<SocketAddr> {
    let host = resource
        .host_str()
        .ok_or_else(|| std::io::Error::other("MQTT host absent"))?;
    let port = resource.port().unwrap_or(1883);
    tokio::time::timeout(
        Duration::from_secs(2),
        tokio::net::lookup_host((host, port)),
    )
    .await
    .map_err(|_| std::io::Error::other("MQTT host lookup timed out"))??
    .find(SocketAddr::is_ipv4)
    .ok_or_else(|| std::io::Error::other("MQTT IPv4 profile unavailable"))
}

pub(crate) async fn commit_ip(
    endpoint: &iroh::Endpoint,
    connection: &Connection,
) -> Result<(), ReplicaError> {
    tokio::time::timeout(PATH_TIMEOUT, async {
        loop {
            if connection.close_reason().is_some() {
                return Err(ReplicaError::Transport);
            }
            if connection
                .paths()
                .iter()
                .any(|path| matches!(path.remote_addr(), TransportAddr::Ip(_)))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        endpoint
            .commit_data_path(connection)
            .await
            .map_err(|_| ReplicaError::Transport)?;
        // Committing installs the packet gate synchronously, but the public
        // path snapshot follows QUIC's path events. Wait for that observation
        // before the replica consumer applies its IP-only history boundary.
        let mut paths = connection.paths_stream();
        while let Some(paths) = paths.next().await {
            if !paths.is_empty()
                && paths
                    .iter()
                    .all(|path| matches!(path.remote_addr(), TransportAddr::Ip(_)))
            {
                return Ok(());
            }
        }
        Err(ReplicaError::Transport)
    })
    .await
    .unwrap_or(Err(ReplicaError::Transport))
}

impl NativeNetwork {
    /// Start online candidate answering only from qualified saved resources.
    /// Each query rechecks the current credential/member view and live route.
    pub(crate) async fn start_recovery_discovery(&self, resources: &mut ResourceEndpoint) {
        if !resources.recovery_udp_allowed() {
            return;
        }
        let starts = resources.recovery_origins().into_iter().map(|resource| {
            let endpoint = resources.endpoint().clone();
            let gate = self.gate.clone();
            let scope = self.scope.clone();
            async move {
                let origin = crate::resources::resource_origin(resource.as_str())?;
                let service = mqtt_transport::service_tag(&resource).ok()?;
                let broker = broker_address(&resource).await.ok()?;
                let current_scope = scope.clone();
                let responder = mqtt_discovery::respond_live(broker, scope, move || {
                    if !gate.discovery_current(&current_scope) {
                        return None;
                    }
                    let address = endpoint.custom_addrs().into_iter().find(|address| {
                        address.id() == TRANSPORT_ID
                            && address.data().len() == 32
                            && address.data()[..16] == service
                    })?;
                    let udp_addr = endpoint.addr().ip_addrs().find_map(|addr| match addr {
                        SocketAddr::V4(addr) => Some(*addr),
                        _ => None,
                    })?;
                    Some(TaggedCandidate {
                        id: endpoint.id(),
                        route: address.data().try_into().ok()?,
                        udp_addr,
                    })
                })
                .await
                .ok()?;
                Some((origin, responder))
            }
        });
        resources.add_recovery_responders(
            futures_util::future::join_all(starts)
                .await
                .into_iter()
                .flatten(),
        );
    }

    /// Client startup entry. Recovery begins without a business request or a
    /// caller-supplied peer identity, broker topic, discovery loop or proof pack.
    pub async fn resume_network(
        &mut self,
        bind_ip: Ipv4Addr,
    ) -> Result<NetworkRecovery, NodeError> {
        let mut resources = self.bind_configured_endpoint(bind_ip).await?;
        let status = self.recover_network(&mut resources).await;
        Ok(NetworkRecovery { resources, status })
    }

    /// Retry on the same configured endpoint; saved consent and scoped contact
    /// backoff remain in force. Only a complete durable source scope is Ready.
    pub async fn recover_network(&mut self, resources: &mut ResourceEndpoint) -> RecoveryStatus {
        if resources.endpoint().id().as_bytes() != self.node_key.verifying_key().as_bytes()
            || self.secret.is_none()
            || !resources.recovery_udp_allowed()
        {
            return RecoveryStatus::Waiting;
        }
        let candidates = self.discover_recovery_candidates(resources).await;
        let mut pending = None;
        for candidate in candidates {
            let target = EndpointAddr::new(candidate.id).with_addrs([TransportAddr::Custom(
                iroh_base::CustomAddr::from_parts(TRANSPORT_ID, &candidate.route),
            )]);
            resources.remember(target.clone());
            let Ok(connection) = resources.connect(target, RECOVERY_REPLICA_ALPN).await else {
                continue;
            };
            let result = async {
                if !self.gate.allows(&connection, ProtocolScope::Bootstrap) {
                    return Err(ReplicaError::Rejected);
                }
                resources
                    .endpoint()
                    .open_data_path(&connection, candidate.udp_addr.into())
                    .await
                    .map_err(|_| ReplicaError::Transport)?;
                commit_ip(resources.endpoint(), &connection).await?;
                let outcome = self.sync_replica(&connection).await?;
                if outcome.complete() && !self.gate.allows_member_path_upgrade(&connection) {
                    return Err(ReplicaError::Rejected);
                }
                Ok(outcome)
            }
            .await;
            connection.close(0u32.into(), b"startup replica exchange ended");
            if let Ok(outcome) = result {
                if !outcome.complete() {
                    pending = Some(outcome);
                    continue;
                }
                // The authenticated physical path replaces temporary carrier
                // routing in the same endpoint address book for ordinary dials.
                resources.remember(
                    EndpointAddr::new(candidate.id).with_ip_addr(candidate.udp_addr.into()),
                );
                return RecoveryStatus::Ready(outcome);
            }
            resources.contact_failed(candidate.id);
        }
        pending
            .map(RecoveryStatus::WaitingForDependencies)
            .unwrap_or(RecoveryStatus::Waiting)
    }

    /// Qualified hints for the single node-level maintenance coordinator.
    pub(crate) async fn discover_recovery_candidates(
        &self,
        resources: &ResourceEndpoint,
    ) -> Vec<TaggedCandidate> {
        if !resources.recovery_udp_allowed() || self.secret.is_none() {
            return Vec::new();
        }
        let queries = resources.recovery_origins().into_iter().map(|resource| {
            let scope = self.scope.clone();
            async move {
                let service = mqtt_transport::service_tag(&resource).ok()?;
                let broker = broker_address(&resource).await.ok()?;
                let found = mqtt_discovery::discover_routes::<32>(broker, &scope)
                    .await
                    .ok()?;
                Some(
                    found
                        .into_iter()
                        .filter(|hint| hint.route[..16] == service)
                        .collect::<Vec<_>>(),
                )
            }
        });
        let mut candidates = Vec::new();
        for found in futures_util::future::join_all(queries)
            .await
            .into_iter()
            .flatten()
        {
            for hint in found {
                if hint.id != resources.endpoint().id()
                    && !candidates.contains(&hint)
                    && candidates.len() < 16
                {
                    candidates.push(hint);
                }
            }
        }
        candidates
    }

    /// The online member's router handler. Secret possession is restricted
    /// bootstrap access; a new/removed identity receives no history manifest.
    pub async fn serve_network_recovery(
        &self,
        resources: &ResourceEndpoint,
        connection: &Connection,
    ) -> Result<(), ReplicaError> {
        let records = self.management_history().await?;
        self.serve_recovery_records(resources, connection, &records)
            .await
    }

    #[cfg(feature = "replica-test-hooks")]
    pub async fn serve_network_recovery_fixture(
        &self,
        resources: &ResourceEndpoint,
        connection: &Connection,
        records: &[crate::management_store::ManagementRecord],
    ) -> Result<(), ReplicaError> {
        self.serve_recovery_records(resources, connection, records)
            .await
    }

    async fn serve_recovery_records(
        &self,
        resources: &ResourceEndpoint,
        connection: &Connection,
        records: &[crate::management_store::ManagementRecord],
    ) -> Result<(), ReplicaError> {
        let result = async {
            if resources.endpoint().id().as_bytes() != self.node_key.verifying_key().as_bytes()
                || connection.alpn() != RECOVERY_REPLICA_ALPN
                || !self.gate.allows_member_path_upgrade(connection)
                || !resources.recovery_udp_allowed()
            {
                return Err(ReplicaError::Rejected);
            }
            commit_ip(resources.endpoint(), connection).await?;
            self.serve_replica_records(connection, records).await?;
            // Retain the provider's handle until the receiver observes its final
            // acknowledgement and closes; dropping it earlier can discard ACK.
            tokio::time::timeout(PATH_TIMEOUT, connection.closed())
                .await
                .map_err(|_| ReplicaError::Transport)?;
            Ok(())
        }
        .await;
        if result.is_err() {
            connection.close(0u32.into(), b"startup recovery rejected");
        }
        result
    }
}
