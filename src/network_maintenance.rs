//! One node-level member selector and durable sync coordinator. Endpoints own
//! identity-scoped contact attempts; iroh owns the paths of each living session.
use crate::{
    admission_gate::{AdmissionGate, ProtocolScope},
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
    sync::{Notify, Semaphore, mpsc, watch},
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
            contact_timeout_ms: 2_000,
        }
    }
    pub fn active() -> Self {
        Self {
            exploration_interval_ms: 1_000,
            discovery_interval_ms: 5_000,
            max_concurrent_contacts: 4,
            max_neighbors: 4,
            quality_interval_ms: 1_000,
            contact_timeout_ms: 2_000,
        }
    }
    pub(crate) fn validate(&self) -> std::io::Result<()> {
        if !(250..=60_000).contains(&self.exploration_interval_ms)
            || !(1_000..=120_000).contains(&self.discovery_interval_ms)
            || !(1..=8).contains(&self.max_concurrent_contacts)
            || !(1..=8).contains(&self.max_neighbors)
            || !(250..=30_000).contains(&self.quality_interval_ms)
            || !(100..=30_000).contains(&self.contact_timeout_ms)
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
}
pub struct MaintainedNetwork {
    resources: Arc<ResourceEndpoint>,
    updates: watch::Receiver<MaintenanceStatus>,
    commands: mpsc::Sender<Command>,
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
    pub fn node_status(&self) -> NodeStatus {
        self.updates.borrow().node.clone()
    }
    pub fn maintenance_status(&self) -> MaintenanceStatus {
        let mut status = self.updates.borrow().clone();
        if self.activity.count.load(Ordering::Acquire) > 0 {
            status.state = MaintenanceState::BusinessPriority;
        }
        status
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
struct Coordinator {
    node: NativeNetwork,
    resources: Arc<ResourceEndpoint>,
    policy: MaintenancePolicy,
    candidates: BTreeMap<EndpointId, Candidate>,
    neighbors: BTreeMap<EndpointId, Neighbor>,
    sampler: PeerSampler,
    backoff: DiscoveryBackoff,
    discover_at: Instant,
    explore_at: Instant,
    last_attempt_at: Option<Instant>,
    status: MaintenanceStatus,
    published: watch::Sender<MaintenanceStatus>,
    provider: watch::Sender<ReplicaProvider>,
    activity: Arc<Activity>,
}
impl Coordinator {
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
    async fn command(&mut self, command: Command) -> Result<(), NodeError> {
        match command {
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
        if self.activity.count.load(Ordering::Acquire) > 0 {
            self.status.state = MaintenanceState::BusinessPriority;
        } else if !self.neighbors.is_empty() {
            self.status.state = MaintenanceState::Connected;
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
                let result = resources.connect(candidate.address.clone(), alpn).await;
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
                    self.status.cause = Some(error.to_string());
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
        if self.neighbors.is_empty() && self.status.last_sync.is_none() {
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
        cancel: CancellationToken,
        router: Router,
    ) {
        loop {
            let activity = self.activity.clone();
            let active = activity.count.load(Ordering::Acquire) > 0;
            tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                command = commands.recv() => {
                    let Some(command) = command else { break; };
                    if let Err(error) = self.command(command).await { self.status.cause = Some(error.to_string()); }
                }
                _ = activity.changed.notified() => { self.explore_at = self.explore_at.max(Instant::now()); }
                _ = tokio::time::sleep_until(self.explore_at), if !active => {
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => break,
                        result = self.step() => {
                            if let Err(error) = result { self.status.cause = Some(error.to_string()); self.status.state = MaintenanceState::TemporarilyUnreachable; self.explore_at = Instant::now() + self.floor(); }
                        }
                    }
                }
            }
            self.publish().await;
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
        let config = self.resource_configuration()?;
        self.gate
            .register_protocol(REPLICA_ALPN, ProtocolScope::Member);
        let policy = config.maintenance.clone();
        policy.validate()?;
        let mut path_policy = config.path_selection.clone();
        path_policy.reevaluation_interval_ms = path_policy
            .reevaluation_interval_ms
            .max(policy.quality_interval_ms);
        let builder = Endpoint::builder(iroh::endpoint::presets::Minimal)
            .bind_addr(SocketAddr::from((bind_ip, 0)))
            .map_err(|_| NodeError::InvalidData)?
            .path_selector(path_policy.selector()?);
        let resources = self
            .bind_configured_endpoint_with(builder)
            .await?
            .with_contact_timeout(Duration::from_millis(policy.contact_timeout_ms))?;
        if resources.recovery_origins().is_empty() {
            resources
                .endpoint()
                .address_lookup()
                .map_err(std::io::Error::other)?
                .add(resources.contacts.clone());
        }
        let resources = Arc::new(resources);
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
        let router = routes(
            Router::builder(resources.endpoint().clone())
                .accept(REPLICA_ALPN, handler.clone())
                .accept(RECOVERY_REPLICA_ALPN, handler),
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
        };
        let (published, updates) = watch::channel(status.clone());
        let (commands, incoming) = mpsc::channel(16);
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
            discover_at: Instant::now(),
            explore_at: Instant::now(),
            last_attempt_at: None,
            status,
            published,
            provider: provider_tx,
            activity: activity.clone(),
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
        let task = tokio::spawn(coordinator.run(incoming, cancel.clone(), router));
        Ok(MaintainedNetwork {
            resources,
            updates,
            commands,
            activity,
            cancel,
            task: Some(task),
        })
    }
}
