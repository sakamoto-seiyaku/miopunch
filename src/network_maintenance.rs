//! One node-level member selector and durable sync coordinator. Endpoints own
//! identity-scoped contact attempts; iroh owns the paths of each living session.
use crate::{
    admission_gate::{AdmissionGate, ProtocolScope},
    contact_listener::{ContactListenerControl, ContactListenerPhase, ContactListenerStatus},
    native_network::{NativeNetwork, NodeError, NodeStatus},
    network_recovery::{RECOVERY_REPLICA_ALPN, commit_ip},
    offline_replica::{REPLICA_ALPN, ReplicaOutcome, ReplicaProvider},
    resources::{ResourceContactError, ResourceEndpoint},
};
use futures_util::{StreamExt, stream::FuturesUnordered};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, TransportAddr,
    endpoint::Connection,
    protocol::{ProtocolHandler, Router, RouterBuilder},
};
use p2panda_core::VerifyingKey;
use p2panda_discovery::{
    DiscoveryResult, DiscoveryStrategy,
    random_walk::{RandomWalker, RandomWalkerConfig},
};
use p2panda_net::{
    addrs::NodeInfo,
    discovery::{DiscoveryBackoff, DiscoveryBackoffConfig},
};
use p2panda_store::{SqliteStore, SqliteStoreBuilder, Transaction, address_book::AddressBookStore};
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    net::{Ipv4Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{Notify, Semaphore, mpsc, oneshot, watch},
    task::JoinHandle,
    time::Instant,
};
use tokio_util::sync::CancellationToken;
/// This node's local effort, independent of membership or resource permission.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MaintenancePolicy {
    pub exploration_interval_ms: u64,
    pub discovery_interval_ms: u64,
    pub max_concurrent_contacts: usize,
    pub max_neighbors: usize,
    pub quality_interval_ms: u64,
    pub contact_timeout_ms: u64,
}
impl Default for MaintenancePolicy {
    fn default() -> Self {
        Self::conservative()
    }
}
impl MaintenancePolicy {
    pub fn conservative() -> Self {
        Self {
            exploration_interval_ms: 5_000,
            discovery_interval_ms: 15_000,
            max_concurrent_contacts: 1,
            max_neighbors: 1,
            quality_interval_ms: 5_000,
            contact_timeout_ms: crate::connection_budget::DEFAULT_MS,
        }
    }
    pub fn active() -> Self {
        Self {
            exploration_interval_ms: 1_000,
            discovery_interval_ms: 5_000,
            max_concurrent_contacts: 4,
            max_neighbors: 4,
            quality_interval_ms: 1_000,
            contact_timeout_ms: crate::connection_budget::DEFAULT_MS,
        }
    }
    pub(crate) fn validate(&self) -> std::io::Result<()> {
        if !(250..=60_000).contains(&self.exploration_interval_ms)
            || !(1_000..=120_000).contains(&self.discovery_interval_ms)
            || !(1..=8).contains(&self.max_concurrent_contacts)
            || !(1..=8).contains(&self.max_neighbors)
            || !(250..=30_000).contains(&self.quality_interval_ms)
            || !(100..=crate::connection_budget::MAX_MS).contains(&self.contact_timeout_ms)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "node maintenance budget is outside bounded limits",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaintenanceState {
    Starting,
    Connected,
    TemporarilyUnreachable,
    WaitingForDependencies,
    BusinessPriority,
    Stopped,
}
#[derive(Clone, Debug)]
pub struct NeighborStatus {
    pub id: EndpointId,
    pub stable_id: usize,
    /// Actual QUIC PING frames, including the path selector's bounded probes.
    pub pings_sent: u64,
}
/// Secret-free diagnostics: sources are local observations, never global freshness.
#[derive(Clone, Debug)]
pub struct MaintenanceStatus {
    pub state: MaintenanceState,
    pub node: NodeStatus,
    pub neighbors: Vec<NeighborStatus>,
    pub last_sync: Option<ReplicaOutcome>,
    pub contact_attempts: u64,
    pub discovery_rounds: u64,
    pub max_in_flight: usize,
    pub retry_after: Duration,
    pub cause: Option<String>,
    /// Local facility generation only; readiness grants no membership/data path.
    pub contact_listener: Option<ContactListenerStatus>,
}
struct Activity {
    count: AtomicUsize,
    changed: Notify,
}
/// Hold while the application uses a business session. Dropping releases priority.
/// Existing QUIC paths remain maintained by iroh throughout this lease.
pub struct BusinessActivity(Arc<Activity>);
impl Drop for BusinessActivity {
    fn drop(&mut self) {
        self.0.count.fetch_sub(1, Ordering::AcqRel);
        self.0.changed.notify_one();
    }
}
enum Command {
    Contact(EndpointAddr),
    NetworkChanged,
    DisableContactSignaling(oneshot::Sender<Result<(), NodeError>>),
}
pub struct MaintainedNetwork {
    resources: Arc<ResourceEndpoint>,
    active: crate::connection_coordinator::CoordinatorHook,
    contact_observer: Option<Arc<crate::mqtt_transport::MqttTransport>>,
    updates: watch::Receiver<MaintenanceStatus>,
    commands: mpsc::Sender<Command>,
    disable_commands: mpsc::Sender<Command>,
    activity: Arc<Activity>,
    cancel: CancellationToken,
    task: Option<JoinHandle<()>>,
}
impl MaintainedNetwork {
    pub fn endpoint(&self) -> &Endpoint {
        self.resources.endpoint()
    }
    pub fn addr(&self) -> EndpointAddr {
        self.resources.addr()
    }
    /// The configured endpoint's existing common connection entry. This shares
    /// its current resource snapshot and does not create a second Node owner.
    pub fn coordinator(&self) -> crate::connection_coordinator::ConnectionCoordinator {
        self.resources.coordinator()
    }
    /// Repair the exact member Connection kept by this caller. Existing Node
    /// contact quota, original-M lease, Router and active protocol are shared;
    /// the caller remainder and historical three-second cap bound the operation.
    /// A failed repair with no fresh surviving DATA path terminates this M.
    pub async fn repair_member_connection_until(
        &self,
        original: &Connection,
        contact: EndpointAddr,
        deadline: n0_future::time::Instant,
    ) -> Result<(), crate::connection_recovery::ConnectionPathRecoveryError> {
        self.active
            .repair(
                &self.resources,
                original,
                contact,
                deadline,
                self.cancel.child_token(),
            )
            .await
    }
    pub fn node_status(&self) -> NodeStatus {
        self.updates.borrow().node.clone()
    }
    pub fn maintenance_status(&self) -> MaintenanceStatus {
        let mut status = self.updates.borrow().clone();
        status.contact_listener = self
            .resources
            .contact_listener()
            .map(|listener| listener.status());
        if self.activity.count.load(Ordering::Acquire) > 0 {
            status.state = MaintenanceState::BusinessPriority;
        }
        status
    }
    /// Actual traffic from the explicitly supplied contact carrier. A normal
    /// startup without that observer reports None, rather than invented counts.
    /// The saved resource policy still decides whether the carrier may bind.
    pub fn contact_listener_traffic(&self) -> Option<serde_json::Value> {
        self.contact_observer
            .as_ref()
            .map(|transport| transport.traffic().snapshot())
    }
    pub fn watch_maintenance(&self) -> watch::Receiver<MaintenanceStatus> {
        self.updates.clone()
    }
    /// A route hint never grants membership. Identical hints do not reset pacing.
    pub async fn offer_contact(&self, hint: EndpointAddr) -> Result<(), NodeError> {
        if hint.addrs.len() > 8 {
            return Err(NodeError::InvalidData);
        }
        self.commands
            .send(Command::Contact(hint))
            .await
            .map_err(|_| NodeError::InvalidData)
    }
    /// A client lifecycle/network event wakes the single selector; the frequency
    /// floor and same-identity in-flight guard still apply to repeated events.
    pub async fn network_changed(&self) -> Result<(), NodeError> {
        self.commands
            .send(Command::NetworkChanged)
            .await
            .map_err(|_| NodeError::InvalidData)
    }
    /// Disable this Node's selected MQTT signaling listener until a fresh startup.
    /// Saves only that facility's Signaling disable and waits for actual address
    /// withdrawal, observer retirement and carrier driver closure. Other resource
    /// uses, member permissions and existing data sessions are retained.
    /// A stopped owner, persistence failure or retirement failure returns an error.
    pub async fn disable_contact_signaling(&self) -> Result<(), NodeError> {
        let (completion, completed) = oneshot::channel();
        // A terminal listener needs no urgent interruption. Repeated disable
        // uses ordinary FIFO ordering, acknowledging preceding network events.
        let commands = if self
            .resources
            .contact_listener()
            .is_some_and(|listener| listener.status().phase == ContactListenerPhase::Stopped)
        {
            &self.commands
        } else {
            &self.disable_commands
        };
        commands
            .send(Command::DisableContactSignaling(completion))
            .await
            .map_err(|_| NodeError::InvalidData)?;
        completed.await.map_err(|_| NodeError::InvalidData)?
    }
    pub fn business_activity(&self) -> BusinessActivity {
        self.activity.count.fetch_add(1, Ordering::AcqRel);
        self.activity.changed.notify_one();
        BusinessActivity(self.activity.clone())
    }
    /// Business demand gets the same endpoint contact guard and resource snapshot.
    /// Keep the returned lease for the application's use of this Connection.
    pub async fn connect(
        &self,
        target: EndpointAddr,
        alpn: &[u8],
    ) -> Result<(Connection, BusinessActivity), ResourceContactError> {
        let activity = self.business_activity();
        let connection = self.resources.connect(target, alpn).await?;
        Ok((connection, activity))
    }
    pub async fn close(mut self) {
        self.cancel.cancel();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}
impl Drop for MaintainedNetwork {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[derive(Clone)]
struct Candidate {
    address: EndpointAddr,
    udp_hint: Option<SocketAddr>,
    verified: bool,
}
struct Neighbor {
    connection: Connection,
    sync_at: Instant,
}
struct CloseExchange(Option<Connection>);
impl Drop for CloseExchange {
    fn drop(&mut self) {
        if let Some(connection) = &self.0 {
            connection.close(0u32.into(), b"maintenance exchange ended");
        }
    }
}
/// The mature p2panda breadth-first sampler consumes only this bounded local
/// projection; no additional Discovery/Gossip actors or endpoint loops run.
struct PeerSampler {
    store: SqliteStore,
    walker: RandomWalker<ChaCha20Rng, SqliteStore, VerifyingKey, NodeInfo>,
    previous: Option<DiscoveryResult<VerifyingKey, NodeInfo>>,
}
impl PeerSampler {
    async fn new(id: VerifyingKey) -> Result<Self, NodeError> {
        let store = SqliteStoreBuilder::memory()
            .build()
            .await
            .map_err(std::io::Error::other)?;
        let walker = RandomWalker::from_config(
            id,
            store.clone(),
            ChaCha20Rng::from_rng(&mut rand::rng()),
            RandomWalkerConfig {
                reset_walk_probability: 0.02,
            },
        );
        Ok(Self {
            store,
            walker,
            previous: None,
        })
    }
    async fn insert(&mut self, candidate: &Candidate) -> Result<(), NodeError> {
        let mut info = NodeInfo::from(candidate.address.clone());
        if candidate.verified {
            info = info.bootstrap();
        }
        let permit = self.store.begin().await.map_err(std::io::Error::other)?;
        self.store
            .insert_node_info(info)
            .await
            .map_err(std::io::Error::other)?;
        self.store
            .commit(permit)
            .await
            .map_err(std::io::Error::other)?;
        self.previous = None;
        Ok(())
    }
    async fn next(&mut self) -> Result<Option<EndpointId>, NodeError> {
        let id = self
            .walker
            .next_node(self.previous.as_ref())
            .await
            .map_err(std::io::Error::other)?;
        if let Some(id) = id {
            self.previous = Some(DiscoveryResult::new(id));
        }
        id.map(|id| EndpointId::from_bytes(id.as_bytes()).map_err(|_| NodeError::InvalidData))
            .transpose()
    }
}

#[derive(Clone)]
struct HistoryHandler {
    provider: watch::Receiver<ReplicaProvider>,
    resources: Arc<ResourceEndpoint>,
    gate: AdmissionGate,
    permits: Arc<Semaphore>,
}
impl std::fmt::Debug for HistoryHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HistoryHandler")
    }
}
impl ProtocolHandler for HistoryHandler {
    async fn on_accepting(
        &self,
        mut accepting: iroh::endpoint::Accepting,
    ) -> Result<Connection, iroh::protocol::AcceptError> {
        // Retain the original receipt clock through Router ALPN classification.
        let deadline = self
            .resources
            .contact_deadline_from(accepting.received_at());
        let alpn = accepting
            .alpn()
            .await
            .map_err(iroh::protocol::AcceptError::from_err)?;
        if alpn != REPLICA_ALPN {
            // Recovery's historical acceptance remains separate and unchanged.
            return accepting.await.map_err(Into::into);
        }
        if deadline <= n0_future::time::Instant::now() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "replica admission deadline elapsed during classification",
            )
            .into());
        }
        tokio::time::timeout(
            deadline.saturating_duration_since(n0_future::time::Instant::now()),
            accepting.with_caller_deadline(deadline),
        )
        .await
        .map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "replica admission deadline exceeded",
            )
        })?
        .map_err(Into::into)
    }

    async fn accept(&self, connection: Connection) -> Result<(), iroh::protocol::AcceptError> {
        let Ok(_permit) = self.permits.clone().try_acquire_owned() else {
            connection.close(0u32.into(), b"maintenance serving budget busy");
            return Ok(());
        };
        if connection.alpn() == RECOVERY_REPLICA_ALPN {
            let result = async {
                if !self.resources.recovery_udp_allowed()
                    || !self.gate.allows_member_path_upgrade(&connection)
                {
                    return Err(());
                }
                commit_ip(self.resources.endpoint(), &connection)
                    .await
                    .map_err(|_| ())?;
                let provider = self.provider.borrow().clone();
                provider.serve(&connection).await.map_err(|_| ())?;
                tokio::time::timeout(Duration::from_secs(12), connection.closed())
                    .await
                    .map_err(|_| ())?;
                Ok::<_, ()>(())
            }
            .await;
            if result.is_err() {
                connection.close(0u32.into(), b"maintenance recovery rejected");
            }
        } else {
            while self.gate.allows(&connection, ProtocolScope::Member) {
                let Ok(streams) = connection.accept_bi().await else {
                    break;
                };
                // Read the newest applied snapshot at the start of each exchange.
                let provider = self.provider.borrow().clone();
                if provider.serve_stream(&connection, streams).await.is_err() {
                    break;
                }
            }
            connection.close(0u32.into(), b"maintenance member session ended");
        }
        Ok(())
    }
}

#[derive(Clone)]
struct ActiveImprovementHandler {
    endpoint: Endpoint,
    gate: AdmissionGate,
    adapter: crate::connection_coordinator::CoordinatorHook,
    resources: Arc<ResourceEndpoint>,
}
impl std::fmt::Debug for ActiveImprovementHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ActiveImprovementHandler")
    }
}
impl ProtocolHandler for ActiveImprovementHandler {
    async fn on_accepting(
        &self,
        accepting: iroh::endpoint::Accepting,
    ) -> Result<Connection, iroh::protocol::AcceptError> {
        // Router has identified this restricted ALPN; include its preceding
        // classification time without imposing a clock on unrelated protocols.
        let deadline = self
            .resources
            .contact_deadline_from(accepting.received_at());
        if deadline <= n0_future::time::Instant::now() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "active improvement deadline elapsed during classification",
            )
            .into());
        }
        tokio::time::timeout(
            deadline.saturating_duration_since(n0_future::time::Instant::now()),
            accepting.with_caller_deadline(deadline),
        )
        .await
        .map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "active improvement admission deadline exceeded",
            )
        })?
        .map_err(Into::into)
    }

    async fn accept(&self, control: Connection) -> Result<(), iroh::protocol::AcceptError> {
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
                &control,
                &adapter,
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
        if result.is_err() {
            control.close(0u32.into(), b"active improvement failed");
        }
        Ok(())
    }
}
struct Coordinator {
    node: NativeNetwork,
    resources: Arc<ResourceEndpoint>,
    policy: MaintenancePolicy,
    candidates: BTreeMap<EndpointId, Candidate>,
    neighbors: BTreeMap<EndpointId, Neighbor>,
    sampler: PeerSampler,
    backoff: DiscoveryBackoff,
    contact_listener: Option<ContactListenerControl>,
    listener_backoff: DiscoveryBackoff,
    listener_retry_at: Option<Instant>,
    listener_last_attempt: Option<Instant>,
    discover_at: Instant,
    explore_at: Instant,
    last_attempt_at: Option<Instant>,
    status: MaintenanceStatus,
    published: watch::Sender<MaintenanceStatus>,
    provider: watch::Sender<ReplicaProvider>,
    activity: Arc<Activity>,
    active: crate::connection_coordinator::CoordinatorHook,
}
impl Coordinator {
    fn listener_changed(&mut self) {
        let Some(listener) = &self.contact_listener else {
            return;
        };
        let status = listener.status();
        match status.phase {
            ContactListenerPhase::Ready => {
                self.listener_backoff.reset();
                self.listener_retry_at = None;
            }
            ContactListenerPhase::Unavailable => {
                if self.listener_retry_at.is_none() {
                    self.listener_retry_at =
                        Some(Instant::now() + self.floor().max(self.listener_backoff.duration()));
                }
            }
            _ => self.listener_retry_at = None,
        }
        self.status.contact_listener = Some(status);
    }
    fn restart_listener(&mut self) {
        let Some(listener) = &self.contact_listener else {
            return;
        };
        if listener.status().phase == ContactListenerPhase::Stopped {
            self.listener_retry_at = None;
            return;
        }
        self.listener_last_attempt = Some(Instant::now());
        if let Err(error) = listener.restart() {
            self.status.cause = Some(error.to_string());
            self.listener_backoff.increment();
            self.listener_retry_at =
                Some(Instant::now() + self.floor().max(self.listener_backoff.duration()));
        } else {
            self.listener_backoff.increment();
            self.listener_retry_at = None;
        }
        self.status.contact_listener = Some(listener.status());
    }
    fn floor(&self) -> Duration {
        Duration::from_millis(self.policy.exploration_interval_ms)
    }
    fn member(&self, id: EndpointId) -> bool {
        self.node
            .status()
            .members
            .iter()
            .any(|(key, _)| key.as_bytes() == id.as_bytes())
    }
    fn permitted(&self, candidate: &Candidate) -> bool {
        candidate.address.addrs.iter().any(|path| self.resources.allows_data_path(path)
            || (!candidate.verified && matches!(path, TransportAddr::Custom(addr) if addr.id() == crate::mqtt_transport::TRANSPORT_ID && self.resources.recovery_origins().iter().any(|origin| crate::mqtt_transport::service_tag(origin).is_ok_and(|tag| addr.data().len() == 32 && addr.data()[..16] == tag)))))
    }
    async fn insert(&mut self, candidate: Candidate) -> Result<bool, NodeError> {
        let id = candidate.address.id;
        if id == self.resources.endpoint().id()
            || !self.permitted(&candidate)
            || (self.candidates.len() >= 64 && !self.candidates.contains_key(&id))
        {
            return Ok(false);
        }
        if self.candidates.get(&id).is_some_and(|old| {
            old.address == candidate.address && old.udp_hint == candidate.udp_hint
        }) {
            return Ok(false);
        }
        self.resources.remember(candidate.address.clone());
        self.sampler.insert(&candidate).await?;
        self.candidates.insert(id, candidate);
        Ok(true)
    }
    fn wake(&mut self) {
        // New evidence can end a long failure delay, never the node's frequency floor.
        let earliest = self
            .last_attempt_at
            .map(|at| at + self.floor())
            .unwrap_or_else(Instant::now);
        self.explore_at = earliest.max(Instant::now());
        self.discover_at = self.explore_at;
        self.backoff.reset();
    }
    async fn disable_contact_signaling(&mut self) -> Result<(), NodeError> {
        let listener = self.contact_listener.as_ref().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no owned MQTT signaling listener",
            )
        })?;
        let config = self
            .resources
            .disabled_contact_configuration(self.node.resource_configuration()?)?;
        let saved = self.node.save_resource_configuration(&config);
        // Even a save error can occur after atomic replacement. Fail closed:
        // retire the running resource, but never acknowledge persistence success.
        self.listener_retry_at = None;
        let retired = listener.shutdown().await;
        let responders_retired = self
            .resources
            .retire_recovery_origin(&listener.resource_url())
            .await;
        self.status.contact_listener = Some(listener.status());
        saved?;
        retired?;
        responders_retired?;
        Ok(())
    }
    async fn command(&mut self, command: Command) -> Result<(), NodeError> {
        match command {
            Command::DisableContactSignaling(completion) => {
                let result = self.disable_contact_signaling().await;
                if let Err(error) = &result {
                    self.status.cause = Some(error.to_string());
                }
                // Publish before acknowledging, including real terminal status.
                self.publish().await;
                let _ = completion.send(result);
            }
            Command::Contact(address) => {
                if self
                    .insert(Candidate {
                        address,
                        udp_hint: None,
                        verified: false,
                    })
                    .await?
                {
                    self.wake();
                }
            }
            Command::NetworkChanged => {
                for id in self.candidates.keys() {
                    self.resources.reset_contact(*id);
                }
                self.wake();
                if let Some(listener) = &self.contact_listener {
                    if listener.status().phase == ContactListenerPhase::Unavailable {
                        let earliest = self
                            .listener_last_attempt
                            .map(|at| at + self.floor())
                            .unwrap_or_else(Instant::now);
                        self.listener_retry_at = Some(earliest.max(Instant::now()));
                    }
                }
            }
        }
        Ok(())
    }
    async fn publish(&mut self) {
        self.status.node = self.node.status();
        self.status.neighbors = self
            .neighbors
            .iter()
            .map(|(id, peer)| NeighborStatus {
                id: *id,
                stable_id: peer.connection.stable_id(),
                pings_sent: peer.connection.stats().frame_tx.ping,
            })
            .collect();
        self.status.retry_after = self.explore_at.saturating_duration_since(Instant::now());
        self.status.contact_listener = self
            .contact_listener
            .as_ref()
            .map(|listener| listener.status());
        if self.activity.count.load(Ordering::Acquire) > 0 {
            self.status.state = MaintenanceState::BusinessPriority;
        } else if !self.neighbors.is_empty() {
            self.status.state = MaintenanceState::Connected;
        } else if self.status.state == MaintenanceState::Connected {
            self.status.state = MaintenanceState::TemporarilyUnreachable;
        }
        self.published.send_replace(self.status.clone());
        match self.node.management_history().await {
            Ok(records) => {
                self.provider
                    .send_replace(ReplicaProvider::new(&self.node, records));
            }
            Err(error) => {
                self.status.cause = Some(error.to_string());
            }
        }
    }
    async fn persist_contacts(&self) -> Result<(), NodeError> {
        let contacts = self
            .candidates
            .values()
            .filter(|candidate| candidate.verified && self.member(candidate.address.id))
            .map(|candidate| candidate.address.clone())
            .collect::<Vec<_>>();
        self.node.save_maintenance_contacts(&contacts)
    }
    async fn synchronize(
        &mut self,
        candidate: Candidate,
        connection: Connection,
    ) -> Result<(), NodeError> {
        let id = connection.remote_id();
        let mut owned = CloseExchange(Some(connection));
        let connection = owned.0.as_ref().unwrap();
        let result = async {
            if connection.alpn() == RECOVERY_REPLICA_ALPN {
                if let Some(udp) = candidate.udp_hint {
                    self.resources
                        .endpoint()
                        .open_data_path(connection, udp)
                        .await
                        .map_err(std::io::Error::other)?;
                }
                commit_ip(self.resources.endpoint(), connection)
                    .await
                    .map_err(std::io::Error::other)?;
            }
            self.node
                .sync_replica(connection)
                .await
                .map_err(std::io::Error::other)
        }
        .await;
        match result {
            Ok(outcome) => {
                let complete =
                    outcome.complete() && self.node.gate.allows_member_path_upgrade(connection);
                self.status.last_sync = Some(outcome);
                if !complete {
                    self.status.state = MaintenanceState::WaitingForDependencies;
                    self.resources.contact_failed(id);
                    return Ok(());
                }
                let mut address = EndpointAddr::new(id);
                address.addrs.extend(
                    connection
                        .paths()
                        .iter()
                        .map(|path| path.remote_addr().clone())
                        .filter(|path| self.resources.allows_data_path(path))
                        .take(8),
                );
                let candidate = Candidate {
                    address,
                    udp_hint: None,
                    verified: true,
                };
                self.resources.remember(candidate.address.clone());
                self.sampler.insert(&candidate).await?;
                self.candidates.insert(id, candidate);
                self.persist_contacts().await?;
                self.resources.reset_contact(id);
                self.status.cause = None;
                self.status.state = MaintenanceState::Connected;
                if connection.alpn() == REPLICA_ALPN
                    && self.neighbors.len() < self.policy.max_neighbors
                {
                    let connection = owned.0.take().unwrap();
                    self.neighbors.insert(
                        id,
                        Neighbor {
                            connection,
                            sync_at: Instant::now() + self.floor(),
                        },
                    );
                }
            }
            Err(error) => {
                self.resources.contact_failed(id);
                self.status.cause = Some(error.to_string());
                self.status.state = MaintenanceState::TemporarilyUnreachable;
            }
        }
        Ok(())
    }
    async fn improve_live_member(&mut self) -> Result<bool, NodeError> {
        if self.activity.count.load(Ordering::Acquire) > 0 {
            return Ok(false);
        }
        let registry = self.node.gate.maintenance();
        let Some(original) = registry.live().into_iter().next() else {
            return Ok(false);
        };
        let id = original.remote_id();
        if !self.node.gate.allows_member_path_upgrade(&original)
            || !self.resources.contact_ready(id)
        {
            return Ok(false);
        }
        let mut target = EndpointAddr::new(id);
        for path in original
            .paths()
            .iter()
            .map(|path| path.remote_addr().clone())
            .take(8)
        {
            target.addrs.insert(path);
        }
        if target.addrs.is_empty() {
            if let Some(candidate) = self.candidates.get(&id) {
                target = candidate.address.clone();
            } else {
                return Ok(false);
            }
        }
        let (deadline, contact) = match self.resources.begin_contact_attempt(id) {
            Ok(value) => value,
            Err(_) => return Ok(false),
        };
        self.status.contact_attempts += 1;
        self.last_attempt_at = Some(Instant::now());
        let cancellation = CancellationToken::new();
        let attempt = crate::connection_coordinator::AttemptContext::new(deadline, cancellation);
        let lease = match registry.acquire(&original, &attempt, contact) {
            Ok(lease) => lease,
            Err(error) => {
                self.status.cause = Some(format!("active improvement reservation: {error:?}"));
                return Ok(false);
            }
        };
        let result = lease
            .run(
                self.active
                    .improve(self.resources.endpoint(), &original, target, attempt),
            )
            .await;
        match result {
            Ok(Ok(())) => {
                self.resources.reset_contact(id);
                self.status.cause = None;
                Ok(true)
            }
            Ok(Err(error)) => {
                self.resources.contact_failed(id);
                self.status.cause = Some(format!("active improvement: {error:?}"));
                self.status.state = MaintenanceState::TemporarilyUnreachable;
                Ok(true)
            }
            Err(error) => {
                self.resources.contact_failed(id);
                self.status.cause = Some(format!("active improvement: {error:?}"));
                self.status.state = MaintenanceState::TemporarilyUnreachable;
                Ok(true)
            }
        }
    }
    async fn step(&mut self) -> Result<(), NodeError> {
        self.neighbors.retain(|_, peer| {
            peer.connection.close_reason().is_none()
                && self
                    .node
                    .gate
                    .allows(&peer.connection, ProtocolScope::Member)
        });
        // Existing neighbor synchronization and new contacts share this coordinator.
        let due = self
            .neighbors
            .iter()
            .filter(|(_, peer)| peer.sync_at <= Instant::now())
            .map(|(id, peer)| (*id, peer.connection.clone()))
            .collect::<Vec<_>>();
        for (id, connection) in due {
            match self.node.sync_replica(&connection).await {
                Ok(outcome) => {
                    if outcome.complete() {
                        self.resources.reset_contact(id);
                    }
                    self.status.last_sync = Some(outcome);
                    if let Some(peer) = self.neighbors.get_mut(&id) {
                        peer.sync_at = Instant::now()
                            + Duration::from_millis(self.policy.exploration_interval_ms);
                    }
                }
                Err(error) => {
                    connection.close(0u32.into(), b"maintenance sync failed");
                    self.neighbors.remove(&id);
                    self.resources.contact_failed(id);
                    self.status.cause = Some(error.to_string());
                    self.status.state = MaintenanceState::TemporarilyUnreachable;
                }
            }
            if self.activity.count.load(Ordering::Acquire) > 0 {
                self.explore_at = Instant::now() + self.floor();
                return Ok(());
            }
        }
        if self.activity.count.load(Ordering::Acquire) > 0 {
            self.explore_at = Instant::now() + self.floor();
            return Ok(());
        }
        // Improvement belongs to this same due batch and consumes the same
        // contact guard/epoch reservation as ordinary maintenance contacts.
        if self.improve_live_member().await? {
            self.explore_at = Instant::now() + self.floor();
            return Ok(());
        }
        let mut chosen = BTreeMap::new();
        // A full permanent view still samples one other source for state
        // coverage. That bounded exchange closes after sync; it does not add
        // another permanent neighbor or turn partial connectivity into a mesh.
        let contact_limit = if self.neighbors.len() < self.policy.max_neighbors {
            self.policy
                .max_concurrent_contacts
                .min(self.policy.max_neighbors - self.neighbors.len())
        } else {
            1
        };
        {
            for _ in 0..self.candidates.len().saturating_mul(2) {
                if chosen.len() >= contact_limit {
                    break;
                }
                let Some(id) = self.sampler.next().await? else {
                    break;
                };
                if self.neighbors.contains_key(&id)
                    || chosen.contains_key(&id)
                    || !self.resources.contact_ready(id)
                {
                    continue;
                }
                let Some(candidate) = self.candidates.get(&id) else {
                    continue;
                };
                if candidate.verified && !self.member(id) {
                    continue;
                }
                if self.permitted(candidate) {
                    chosen.insert(id, candidate.clone());
                }
            }
        }
        let mut dials = FuturesUnordered::new();
        for candidate in chosen.into_values() {
            self.status.contact_attempts += 1;
            let resources = self.resources.clone();
            let alpn = if self.member(candidate.address.id) && candidate.udp_hint.is_none() {
                REPLICA_ALPN
            } else {
                RECOVERY_REPLICA_ALPN
            };
            dials.push(async move {
                let result = resources
                    .connect_for_recovery(candidate.address.clone(), alpn)
                    .await;
                (candidate, result)
            });
        }
        self.status.max_in_flight = self.status.max_in_flight.max(dials.len());
        if !dials.is_empty() {
            self.last_attempt_at = Some(Instant::now());
        }
        while let Some((candidate, result)) = dials.next().await {
            match result {
                Ok(connection) => self.synchronize(candidate, connection).await?,
                Err(error) => {
                    // Display reports retry pacing; retain the concrete dial failure too.
                    self.status.cause = Some(match &error {
                        crate::resources::ResourceContactError::TemporarilyUnreachable {
                            cause: Some(cause),
                            ..
                        } => format!("{error}: {cause}"),
                        _ => error.to_string(),
                    });
                    self.status.state = MaintenanceState::TemporarilyUnreachable;
                }
            }
            if self.activity.count.load(Ordering::Acquire) > 0 {
                self.explore_at = Instant::now() + self.floor();
                return Ok(());
            }
        }
        // Expand only after the known-contact batch, without waiting for every
        // stale address or running an independent per-peer recovery engine.
        if self.discover_at <= Instant::now()
            && !self.resources.recovery_origins().is_empty()
            && self.resources.recovery_udp_allowed()
        {
            self.last_attempt_at = Some(Instant::now());
            self.status.discovery_rounds += 1;
            let hints = self
                .node
                .discover_recovery_candidates(&self.resources)
                .await;
            let mut changed = false;
            for hint in hints {
                let candidate = if self
                    .candidates
                    .get(&hint.id)
                    .is_some_and(|candidate| candidate.verified)
                {
                    // A fresh physical address supplements successful contact history.
                    let address = EndpointAddr::new(hint.id).with_ip_addr(hint.udp_addr.into());
                    Candidate {
                        address,
                        udp_hint: None,
                        verified: false,
                    }
                } else {
                    let address = EndpointAddr::new(hint.id).with_addrs([TransportAddr::Custom(
                        iroh_base::CustomAddr::from_parts(
                            crate::mqtt_transport::TRANSPORT_ID,
                            &hint.route,
                        ),
                    )]);
                    Candidate {
                        address,
                        udp_hint: Some(hint.udp_addr.into()),
                        verified: false,
                    }
                };
                changed |= self.insert(candidate).await?;
            }
            if changed {
                self.backoff.reset();
            } else {
                self.backoff.increment();
            }
            let wait = Duration::from_millis(self.policy.discovery_interval_ms)
                .max(self.backoff.duration());
            self.discover_at = Instant::now() + wait;
        }
        self.explore_at = Instant::now() + self.floor();
        if self.neighbors.is_empty()
            && self.status.state != MaintenanceState::WaitingForDependencies
        {
            self.status.state = MaintenanceState::TemporarilyUnreachable;
            if self.candidates.is_empty() && self.resources.recovery_origins().is_empty() {
                self.status.cause = Some("no permitted member contact or discovery entry; waiting for a new hint or startup resource configuration".into());
            }
        }
        Ok(())
    }
    async fn run(
        mut self,
        mut commands: mpsc::Receiver<Command>,
        mut disable_commands: mpsc::Receiver<Command>,
        cancel: CancellationToken,
        router: Router,
    ) {
        let mut listener_updates = self
            .contact_listener
            .as_ref()
            .map(|listener| listener.watch());
        self.listener_changed();
        loop {
            let activity = self.activity.clone();
            let active = activity.count.load(Ordering::Acquire) > 0;
            let listener_retry = self.listener_retry_at.unwrap_or(self.explore_at);
            tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                command = disable_commands.recv() => {
                    let Some(command) = command else { break; };
                    if let Err(error) = self.command(command).await { self.status.cause = Some(error.to_string()); }
                }
                command = commands.recv() => {
                    let Some(command) = command else { break; };
                    if let Err(error) = self.command(command).await { self.status.cause = Some(error.to_string()); }
                }
                result = async { listener_updates.as_mut().unwrap().changed().await }, if listener_updates.is_some() => {
                    if result.is_err() { listener_updates = None; }
                    self.listener_changed();
                }
                _ = tokio::time::sleep_until(listener_retry), if self.listener_retry_at.is_some() => {
                    // This branch remains active during healthy relay/business
                    // use. It shares this Node's single selector and budget.
                    self.restart_listener();
                }
                _ = activity.changed.notified() => { self.explore_at = self.explore_at.max(Instant::now()); }
                _ = tokio::time::sleep_until(self.explore_at), if !active => {
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => break,
                        // Only explicit resource retirement interrupts a contact
                        // or sync batch. Ordinary hints/events keep their ordering.
                        command = disable_commands.recv() => {
                            let Some(command) = command else { break; };
                            if let Err(error) = self.command(command).await {
                                self.status.cause = Some(error.to_string());
                            }
                        }
                        result = self.step() => {
                            if let Err(error) = result { self.status.cause = Some(error.to_string()); self.status.state = MaintenanceState::TemporarilyUnreachable; self.explore_at = Instant::now() + self.floor(); }
                        }
                    }
                }
            }
            self.publish().await;
        }
        if let Some(listener) = &self.contact_listener {
            if let Err(error) = listener.shutdown().await {
                self.status.cause = Some(error.to_string());
            }
            self.status.contact_listener = Some(listener.status());
        }
        self.status.state = MaintenanceState::Stopped;
        self.published.send_replace(self.status.clone());
        let _ = router.shutdown().await;
        self.node.close().await;
    }
}
impl NativeNetwork {
    pub async fn maintain_network(self, bind_ip: Ipv4Addr) -> Result<MaintainedNetwork, NodeError> {
        self.maintain_network_with(bind_ip, |router| router).await
    }
    /// Add application protocol handlers to this same endpoint/router. Classify
    /// their ALPNs on admission_gate before startup as for ordinary endpoints.
    pub async fn maintain_network_with(
        self,
        bind_ip: Ipv4Addr,
        routes: impl FnOnce(RouterBuilder) -> RouterBuilder,
    ) -> Result<MaintainedNetwork, NodeError> {
        self.maintain_network_with_endpoint(bind_ip, |builder| builder, routes)
            .await
    }

    /// Install application-owned custom transports on the same configured
    /// endpoint. Resource policy and the existing maintenance owner still apply.
    pub async fn maintain_network_with_endpoint(
        self,
        bind_ip: Ipv4Addr,
        configure: impl FnOnce(iroh::endpoint::Builder) -> iroh::endpoint::Builder,
        routes: impl FnOnce(RouterBuilder) -> RouterBuilder,
    ) -> Result<MaintainedNetwork, NodeError> {
        self.maintain_network_inner(
            bind_ip,
            configure,
            routes,
            #[cfg(feature = "rtc-probe")]
            None,
            None,
        )
        .await
    }

    /// The installed RTC adapter is owned by this same Node lifecycle; contact
    /// selection and ordinary handlers still use the configured SDK endpoint.
    #[cfg(feature = "rtc-probe")]
    pub async fn maintain_network_with_rtc(
        self,
        bind_ip: Ipv4Addr,
        rtc: crate::rtc_transport::RtcTransport,
        routes: impl FnOnce(RouterBuilder) -> RouterBuilder,
    ) -> Result<MaintainedNetwork, NodeError> {
        self.maintain_network_inner(bind_ip, |builder| builder, routes, Some(rtc), None)
            .await
    }

    /// Use one explicit contact carrier on the same configured endpoint and
    /// NodeMaintenance lifecycle. The existing resource assembly validates its
    /// complete facility mapping and permission before binding any contact IO.
    pub async fn maintain_network_with_contact(
        self,
        bind_ip: Ipv4Addr,
        contact: Arc<crate::mqtt_transport::MqttTransport>,
        configure: impl FnOnce(iroh::endpoint::Builder) -> iroh::endpoint::Builder,
        routes: impl FnOnce(RouterBuilder) -> RouterBuilder,
    ) -> Result<MaintainedNetwork, NodeError> {
        self.maintain_network_inner(
            bind_ip,
            configure,
            routes,
            #[cfg(feature = "rtc-probe")]
            None,
            Some(contact),
        )
        .await
    }

    /// Compose the existing RTC and explicit contact assembly with application
    /// hooks and routes while retaining the single NodeMaintenance owner.
    #[cfg(feature = "rtc-probe")]
    pub async fn maintain_network_with_rtc_and_contact(
        self,
        bind_ip: Ipv4Addr,
        rtc: crate::rtc_transport::RtcTransport,
        contact: Arc<crate::mqtt_transport::MqttTransport>,
        configure: impl FnOnce(iroh::endpoint::Builder) -> iroh::endpoint::Builder,
        routes: impl FnOnce(RouterBuilder) -> RouterBuilder,
    ) -> Result<MaintainedNetwork, NodeError> {
        self.maintain_network_inner(bind_ip, configure, routes, Some(rtc), Some(contact))
            .await
    }

    async fn maintain_network_inner(
        self,
        bind_ip: Ipv4Addr,
        configure: impl FnOnce(iroh::endpoint::Builder) -> iroh::endpoint::Builder,
        routes: impl FnOnce(RouterBuilder) -> RouterBuilder,
        #[cfg(feature = "rtc-probe")] rtc: Option<crate::rtc_transport::RtcTransport>,
        contact: Option<Arc<crate::mqtt_transport::MqttTransport>>,
    ) -> Result<MaintainedNetwork, NodeError> {
        let config = self.resource_configuration()?;
        self.gate
            .register_protocol(REPLICA_ALPN, ProtocolScope::Member);
        let policy = config.maintenance.clone();
        policy.validate()?;
        let mut path_policy = config.path_selection.clone();
        path_policy.reevaluation_interval_ms = path_policy
            .reevaluation_interval_ms
            .max(policy.quality_interval_ms);
        #[cfg(feature = "rtc-probe")]
        let rtc_installed = rtc.is_some();
        #[cfg(not(feature = "rtc-probe"))]
        let rtc_installed = false;
        let builder = configure(
            Endpoint::builder(iroh::endpoint::presets::Minimal)
                .bind_addr(SocketAddr::from((bind_ip, 0)))
                .map_err(|_| NodeError::InvalidData)?
                .path_selector(path_policy.selector_with_native_rtc(rtc_installed)?),
        );
        let contact_observer = contact.clone();
        #[cfg(feature = "rtc-probe")]
        let resources = match (rtc, contact) {
            (Some(rtc), Some(contact)) => {
                self.bind_configured_endpoint_with_rtc_and_contact(builder, rtc, contact)
                    .await?
            }
            (Some(rtc), None) => self.bind_configured_endpoint_with_rtc(builder, rtc).await?,
            (None, Some(contact)) => {
                self.bind_configured_endpoint_with_contact(builder, contact)
                    .await?
            }
            (None, None) => self.bind_configured_endpoint_with(builder).await?,
        };
        #[cfg(not(feature = "rtc-probe"))]
        let resources = if let Some(contact) = contact {
            self.bind_configured_endpoint_with_contact(builder, contact)
                .await?
        } else {
            self.bind_configured_endpoint_with(builder).await?
        };
        let resources =
            resources.with_contact_timeout(Duration::from_millis(policy.contact_timeout_ms))?;
        if resources.recovery_origins().is_empty() {
            resources
                .endpoint()
                .address_lookup()
                .map_err(std::io::Error::other)?
                .add(resources.contacts.clone());
        }
        let resources = Arc::new(resources);
        let active_adapter = resources.active_coordinator(self.gate.clone());
        let provider = ReplicaProvider::new(&self, self.management_history().await?);
        let (provider_tx, provider_rx) = watch::channel(provider);
        let handler = HistoryHandler {
            provider: provider_rx,
            resources: resources.clone(),
            gate: self.gate.clone(),
            permits: Arc::new(Semaphore::new(
                policy.max_neighbors + policy.max_concurrent_contacts,
            )),
        };
        let active_handler = ActiveImprovementHandler {
            endpoint: resources.endpoint().clone(),
            gate: self.gate.clone(),
            adapter: active_adapter.clone(),
            resources: resources.clone(),
        };
        let router = routes(
            Router::builder(resources.endpoint().clone())
                .accept(REPLICA_ALPN, handler.clone())
                .accept(RECOVERY_REPLICA_ALPN, handler)
                .accept(
                    crate::connection_coordinator::ACTIVE_IMPROVEMENT_ALPN,
                    active_handler,
                ),
        )
        .spawn();
        let status = MaintenanceStatus {
            state: MaintenanceState::Starting,
            node: self.status(),
            neighbors: Vec::new(),
            last_sync: None,
            contact_attempts: 0,
            discovery_rounds: 0,
            max_in_flight: 0,
            retry_after: Duration::ZERO,
            cause: None,
            contact_listener: resources
                .contact_listener()
                .map(|listener| listener.status()),
        };
        let (published, updates) = watch::channel(status.clone());
        let (commands, incoming) = mpsc::channel(16);
        // A bounded urgent command queue shares the same owner/selector; it
        // does not add another actor, retry loop or timer.
        let (disable_commands, disable_incoming) = mpsc::channel(1);
        let activity = Arc::new(Activity {
            count: AtomicUsize::new(0),
            changed: Notify::new(),
        });
        let backoff = DiscoveryBackoff::new(
            DiscoveryBackoffConfig::bounded(
                Duration::ZERO,
                Duration::from_millis(policy.discovery_interval_ms),
                Duration::from_millis(policy.discovery_interval_ms + 100),
                Duration::from_secs(120),
            )?,
            ChaCha20Rng::from_rng(&mut rand::rng()),
        );
        let listener_backoff = DiscoveryBackoff::new(
            DiscoveryBackoffConfig::bounded(
                Duration::ZERO,
                Duration::from_millis(policy.discovery_interval_ms),
                Duration::from_millis(policy.discovery_interval_ms + 100),
                Duration::from_secs(120),
            )?,
            ChaCha20Rng::from_rng(&mut rand::rng()),
        );
        let sampler = PeerSampler::new(self.status().node_id).await?;
        let saved = self.saved_maintenance_contacts()?;
        let mut coordinator = Coordinator {
            node: self,
            resources: resources.clone(),
            policy,
            candidates: BTreeMap::new(),
            neighbors: BTreeMap::new(),
            sampler,
            backoff,
            contact_listener: resources.contact_listener().cloned(),
            listener_backoff,
            listener_retry_at: None,
            listener_last_attempt: None,
            discover_at: Instant::now(),
            explore_at: Instant::now(),
            last_attempt_at: None,
            status,
            published,
            provider: provider_tx,
            activity: activity.clone(),
            active: active_adapter.clone(),
        };
        for address in saved {
            if coordinator.member(address.id) {
                coordinator
                    .insert(Candidate {
                        address,
                        udp_hint: None,
                        verified: true,
                    })
                    .await?;
            }
        }
        let cancel = CancellationToken::new();
        let task =
            tokio::spawn(coordinator.run(incoming, disable_incoming, cancel.clone(), router));
        Ok(MaintainedNetwork {
            resources,
            active: active_adapter,
            contact_observer,
            updates,
            commands,
            disable_commands,
            activity,
            cancel,
            task: Some(task),
        })
    }
}
