#[cfg(not(wasm_browser))]
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    collections::{BTreeSet, HashSet, VecDeque},
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::Poll,
};

use iroh_base::{EndpointId, TransportAddr};
use n0_error::StackResultExt;
use n0_future::{
    FuturesUnordered, MaybeFuture, MergeUnbounded, Stream, StreamExt,
    boxed::BoxStream,
    future::now_or_never,
    task::JoinSet,
    time::{self, Duration, Instant},
};
use n0_watcher::Watcher;
use noq::{Closed, PathStats, PathStatus, WeakConnectionHandle};
use noq_proto::{PathError, PathEvent as NoqPathEvent, PathId, n0_nat_traversal};
use rustc_hash::FxHashMap;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, Level, Span, debug, error, event, info_span, instrument, trace, warn};

use self::path_state::RemotePathState;
pub(crate) use self::path_watcher::PathStateReceiver;
pub use self::{
    path_watcher::{Path, PathEvent, PathEventStream, PathList, PathListIter, PathListStream},
    remote_info::{RemoteInfo, TransportAddrInfo, TransportAddrUsage},
};
use super::{MappedAddrs, Source};
use crate::{
    address_lookup::{AddressLookupFailed, AddressLookupServices, Item as AddressLookupItem},
    endpoint::DirectAddr,
    socket::{
        Metrics as SocketMetrics, RELAY_PATH_MAX_IDLE_TIMEOUT,
        remote_map::remote_state::path_watcher::PathStateSender,
        transports::{self, OwnedTransmit, TransportsSender},
    },
};

mod path_state;
mod path_watcher;
mod remote_info;

/// How often to attempt holepunching.
///
/// If there have been no changes to the NAT address candidates, holepunching will not be
/// attempted more frequently than at this interval.
const HOLEPUNCH_ATTEMPTS_INTERVAL: Duration = Duration::from_secs(5);
const MEMBER_REEVALUATION_INTERVAL: Duration = Duration::from_secs(1);
const MEMBER_DIRECT_ACK_STALL_MIN: Duration = Duration::from_secs(1);
const LOCAL_PROBE_GRACE: Duration = Duration::from_millis(250);

/// A connection-specific retry is needed after noq issues further path IDs.
pub(crate) const PATH_IDENTIFIERS_UNAVAILABLE: &str =
    "path identifiers unavailable; retry explicit path";

/// The latency at or under which we don't try to upgrade to a better path.
const GOOD_ENOUGH_LATENCY: Duration = Duration::from_millis(10);

// TODO: use this
// /// How long since the last activity we try to keep an established endpoint peering alive.
// ///
// /// It's also the idle time at which we stop doing QAD queries to keep NAT mappings alive.
// pub(super) const SESSION_ACTIVE_TIMEOUT: Duration = Duration::from_secs(45);

/// How often we try to upgrade to a better path.
///
/// Even if we have some non-relay route that works.
const UPGRADE_INTERVAL: Duration = Duration::from_secs(60);

/// The time after which an idle [`RemoteStateActor`] stops.
///
/// The actor only enters the idle state if no connections are active and no inbox senders exist
/// apart from the one stored in the endpoint map. Stopping and restarting the actor in this state
/// is not an issue; a timeout here serves the purpose of not stopping-and-recreating actors
/// in a high frequency, and to keep data about previous path around for subsequent connections.
const ACTOR_MAX_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// A stream of events from all paths for all connections.
///
/// The connection is identified using [`ConnId`].  The event `Err` variant happens when the
/// actor has lagged processing the events, which is rather critical for us.
type PathEvents = MergeUnbounded<
    Pin<Box<dyn Stream<Item = (ConnId, Result<NoqPathEvent, noq::Lagged>)> + Send + Sync>>,
>;

/// A stream of events of announced NAT traversal candidate addresses for all connections.
///
/// The connection is identified using [`ConnId`].
type AddrEvents = MergeUnbounded<
    Pin<
        Box<
            dyn Stream<Item = (ConnId, Result<n0_nat_traversal::Event, noq::Lagged>)> + Send + Sync,
        >,
    >,
>;

type MemberActivity = MergeUnbounded<Pin<Box<dyn Stream<Item = ConnId> + Send + Sync>>>;

#[derive(Clone, derive_more::Debug)]
pub(crate) struct MemberPathPolicy {
    // Bootstrap is active until release. Afterward retained standby or streams
    // grant path work, while business activity can seek additional paths.
    activity: Option<watch::Receiver<bool>>,
    business_activity: Option<watch::Receiver<bool>>,
    concurrent_business: Option<watch::Receiver<bool>>,
    acked_custom_transports: [Option<u64>; 2],
    ack_native_relay: bool,
    repair: Option<watch::Sender<bool>>,
    direct_health: Option<watch::Sender<bool>>,
    #[cfg(not(wasm_browser))]
    building: Option<Arc<crate::endpoint::member_fusion::building::Building>>,
    #[cfg(not(wasm_browser))]
    physical_endpoint: std::sync::Weak<crate::socket::EndpointInner>,
    #[cfg(not(wasm_browser))]
    enhanced: Option<Arc<MemberUdpPathWork>>,
    #[debug(skip)]
    permit: Arc<dyn Fn(&TransportAddr) -> bool + Send + Sync>,
}

impl MemberPathPolicy {
    #[cfg(not(wasm_browser))]
    pub(crate) fn new(
        activity: Option<watch::Receiver<bool>>,
        permit: impl Fn(&TransportAddr) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            activity,
            business_activity: None,
            concurrent_business: None,
            acked_custom_transports: [None; 2],
            ack_native_relay: false,
            repair: None,
            direct_health: None,
            #[cfg(not(wasm_browser))]
            building: None,
            physical_endpoint: Default::default(),
            enhanced: None,
            permit: Arc::new(permit),
        }
    }

    #[cfg(not(wasm_browser))]
    pub(crate) fn with_business_activity(
        mut self,
        activity: watch::Receiver<bool>,
        concurrent: watch::Receiver<bool>,
    ) -> Self {
        self.business_activity = Some(activity);
        self.concurrent_business = Some(concurrent);
        self
    }

    #[cfg(not(wasm_browser))]
    pub(crate) fn with_required_path_acks(
        mut self,
        custom: [Option<u64>; 2],
        native_relay: bool,
    ) -> Self {
        self.acked_custom_transports = custom;
        self.ack_native_relay = native_relay;
        self
    }

    #[cfg(not(wasm_browser))]
    pub(crate) fn with_repair_health(mut self, repair: Option<watch::Sender<bool>>) -> Self {
        self.repair = repair;
        self
    }

    #[cfg(not(wasm_browser))]
    pub(crate) fn with_direct_health(mut self, health: watch::Sender<bool>) -> Self {
        self.direct_health = Some(health);
        self
    }

    #[cfg(not(wasm_browser))]
    pub(crate) fn with_enhanced_udp(mut self, work: Option<Arc<MemberUdpPathWork>>) -> Self {
        self.enhanced = work;
        self
    }

    #[cfg(not(wasm_browser))]
    pub(crate) fn with_building(
        mut self,
        building: Option<Arc<crate::endpoint::member_fusion::building::Building>>,
    ) -> Self {
        self.building = building;
        self
    }

    #[cfg(not(wasm_browser))]
    pub(crate) fn with_physical_endpoint(
        mut self,
        endpoint: std::sync::Weak<crate::socket::EndpointInner>,
    ) -> Self {
        self.physical_endpoint = endpoint;
        self
    }

    fn checks_enhanced_udp(&self) -> bool {
        #[cfg(not(wasm_browser))]
        return self.enhanced.is_some();
        #[cfg(wasm_browser)]
        false
    }

    fn needs_enhanced_udp(&self, needed: [bool; 2]) {
        #[cfg(not(wasm_browser))]
        if let Some(work) = &self.enhanced {
            for (signal, needed) in work.needed.iter().zip(needed) {
                signal.send_if_modified(|current| {
                    let changed = *current != needed;
                    *current = needed;
                    changed
                });
            }
        }
        #[cfg(wasm_browser)]
        let _ = needed;
    }

    fn permits(&self, address: transports::Addr) -> bool {
        match &address {
            transports::Addr::Custom(_) if !self.requires_path_ack(&address) => true,
            _ => (self.permit)(&address.into()),
        }
    }

    fn requires_path_ack(&self, address: &transports::Addr) -> bool {
        match address {
            transports::Addr::Custom(custom) => {
                self.acked_custom_transports.contains(&Some(custom.id()))
            }
            transports::Addr::Relay(..) => self.ack_native_relay,
            _ => false,
        }
    }

    fn active(&self) -> bool {
        self.activity
            .as_ref()
            .is_none_or(|activity| *activity.borrow())
    }

    fn business_active(&self) -> bool {
        self.business_activity
            .as_ref()
            .is_none_or(|activity| *activity.borrow())
    }
}

/// Signals between the member path actor and the enhanced UDP round owner.
#[cfg(not(wasm_browser))]
#[derive(Debug, Default)]
pub(crate) struct MemberUdpPathWork {
    pub(crate) needed: [watch::Sender<bool>; 2],
    pub(crate) running: [std::sync::Mutex<usize>; 2],
}

#[derive(Debug)]
struct MemberPaths {
    policy: MemberPathPolicy,
    #[cfg(not(wasm_browser))]
    building: Option<crate::endpoint::member_fusion::building::BuildPermit>,
    #[cfg(not(wasm_browser))]
    physical: Arc<
        std::sync::Mutex<
            std::collections::HashMap<
                crate::endpoint::member_fusion::physical::Scope,
                crate::endpoint::member_fusion::physical::MemberPunching,
            >,
        >,
    >,
    #[cfg(not(wasm_browser))]
    building_events: tokio_util::sync::CancellationToken,
    #[cfg(not(wasm_browser))]
    building_subscribed: bool,
    #[cfg(not(wasm_browser))]
    restricted_contact: Option<Arc<AtomicBool>>,
    known_probes: FxHashMap<SocketAddr, Instant>,
    response_deadlines: FxHashMap<SocketAddr, Instant>,
    // One extra target for the current period of concurrent business.
    additional_direct: Option<SocketAddr>,
    last_exchange: [Option<HolepunchAttempt>; 2],
    candidate_cursor: usize,
    candidate_rotated_at: Option<Instant>,
    network_changed_at: Option<Instant>,
    new_path_probes: FxHashMap<PathId, (Instant, noq::PathProbe)>,
}

impl MemberPaths {
    fn new(policy: MemberPathPolicy) -> Self {
        Self {
            policy,
            #[cfg(not(wasm_browser))]
            building: None,
            #[cfg(not(wasm_browser))]
            physical: Default::default(),
            #[cfg(not(wasm_browser))]
            building_events: Default::default(),
            #[cfg(not(wasm_browser))]
            building_subscribed: false,
            #[cfg(not(wasm_browser))]
            restricted_contact: None,
            known_probes: FxHashMap::default(),
            response_deadlines: FxHashMap::default(),
            additional_direct: None,
            last_exchange: [None, None],
            candidate_cursor: 0,
            candidate_rotated_at: None,
            network_changed_at: None,
            new_path_probes: FxHashMap::default(),
        }
    }

    #[cfg(not(wasm_browser))]
    fn subscribe_building(
        &mut self,
        id: ConnId,
    ) -> Option<Pin<Box<dyn Stream<Item = ConnId> + Send + Sync>>> {
        let building = self.policy.building.as_ref()?;
        if self.building_subscribed {
            return None;
        }
        self.building_subscribed = true;
        let events = n0_future::stream::unfold(
            (building.subscribe(), self.building_events.clone()),
            move |(mut changed, cancelled)| async move {
                tokio::select! { biased;
                    _ = cancelled.cancelled() => None,
                    result = changed.changed() => { result.ok()?; Some((id, (changed, cancelled))) },
                }
            },
        );
        Some(Box::pin(events))
    }

    fn allow_probes(&mut self, conn: &noq::Connection, peer: EndpointId) -> bool {
        #[cfg(not(wasm_browser))]
        if let Some(building) = &self.policy.building {
            if self.building.is_none() {
                self.building = building.try_acquire(peer);
            }
            self.gate_physical_probes(conn, peer);
            conn.set_nat_traversal_probes_permitted(self.building.is_some());
            return self.building.is_some();
        }
        let _ = (conn, peer);
        true
    }

    #[cfg(not(wasm_browser))]
    fn gate_physical_probes(&self, conn: &noq::Connection, peer: EndpointId) {
        let Some(building) = &self.policy.building else {
            return;
        };
        let endpoint = self.policy.physical_endpoint.clone();
        let physical = self.physical.clone();
        let building = building.clone();
        conn.set_nat_traversal_probe_gate(Some(Arc::new(move |remote| {
            let Some(endpoint) = endpoint.upgrade() else {
                return false;
            };
            let Ok(scope) = endpoint.udp_punching_scope_for(remote) else {
                return false;
            };
            let mut leases = physical.lock().expect("ordinary probe physical leases");
            if leases.contains_key(&scope) {
                return true;
            }
            if let Some(permit) = building.physical.try_acquire(scope.clone(), peer) {
                leases.insert(scope, permit);
                true
            } else {
                false
            }
        })));
    }

    fn has_current_ack(&self, stats: &PathStats) -> bool {
        stats.acknowledgment.is_some_and(|ack| {
            self.network_changed_at
                .is_none_or(|changed| Instant::from(ack.sent_at) > changed)
        })
    }
}

#[cfg(not(wasm_browser))]
impl Drop for MemberPaths {
    fn drop(&mut self) {
        self.building_events.cancel();
        self.physical
            .lock()
            .expect("ordinary physical leases")
            .clear();
    }
}

/// The state we need to know about a single remote endpoint.
///
/// This actor manages all connections to the remote endpoint.  It will trigger holepunching
/// and select the best path etc.
pub(super) struct RemoteStateActor {
    /// All connections we have to this remote endpoint.
    connections: FxHashMap<ConnId, ConnectionState>,
    member_activity: MemberActivity,
    #[cfg(feature = "miopunch-test-hooks")]
    force_path_id_retry_once: Option<ConnId>,
    /// State of the actor and hooks into the rest of the remote endpoint.
    ///
    /// This is on a separate struct so that we can have parallel mutable borrows to
    /// `connections` and `state`.
    state: State,
}

/// State of the [`RemoteStateActor`] and hooks into the rest of the remote endpoint.
struct State {
    /// The endpoint ID of the remote endpoint.
    endpoint_id: EndpointId,

    // Hooks into the rest of the Socket.
    //
    /// Metrics.
    metrics: Arc<SocketMetrics>,
    /// Our local addresses.
    ///
    /// These are our local addresses and any reflexive transport addresses.
    local_direct_addrs: n0_watcher::Direct<BTreeSet<DirectAddr>>,
    /// The mapped addresses for this endpoint.
    mapped_addrs: MappedAddrs,
    /// Address lookup service, cloned from the socket.
    address_lookup: AddressLookupServices,

    // Internal state - Noq Connections we are managing.
    //
    /// Notifications when connections are closed.
    connections_close: FuturesUnordered<OnClosed>,
    /// Events emitted by Noq about path changes, for all paths, all connections.
    path_events: PathEvents,
    /// A stream of events of announced NAT traversal candidate addresses for all connections.
    addr_events: AddrEvents,

    // Internal state - Holepunching and path state.
    //
    /// All possible paths we are aware of.
    ///
    /// These paths might be entirely impossible to use, since they are added by Address Lookup
    /// mechanisms.  The are only potentially usable.
    paths: RemotePathState,
    /// Information about the last holepunching attempt.
    last_holepunch: Option<HolepunchAttempt>,

    /// The path we currently consider the preferred path to the remote endpoint.
    ///
    /// **We expect this path to work.** If we become aware this path is broken then it is
    /// set back to `None`.  Having a selected path does not mean we may not be able to get
    /// a better path: e.g. when the selected path is a relay path we still need to trigger
    /// holepunching regularly.
    ///
    /// We only select a path once the path is functional in Noq.
    selected_path: Option<transports::PathBinding>,
    /// Time at which we should schedule the next holepunch attempt.
    scheduled_holepunch: Option<Instant>,
    /// When to next attempt opening paths in [`Self::pending_open_paths`].
    scheduled_open_path: Option<Instant>,
    /// Paths which we still need to open.
    ///
    /// They failed to open because we did not have enough CIDs issued by the remote.
    pending_open_paths: VecDeque<transports::PathBinding>,

    // Internal state - address lookup
    //
    /// Stream of Address Lookup results, or always pending if Address Lookup is not running.
    address_lookup_stream: Option<BoxStream<Result<AddressLookupItem, AddressLookupFailed>>>,

    /// The path selector used to pick the preferred path among the candidates.
    path_selector: Arc<dyn PathSelector>,
    connection_path_selector: Option<Arc<dyn ConnectionPathSelector>>,
    reevaluation_interval: Option<Duration>,
    uses_path_health: bool,
}

impl RemoteStateActor {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        endpoint_id: EndpointId,
        local_direct_addrs: n0_watcher::Direct<BTreeSet<DirectAddr>>,
        mapped_addrs: MappedAddrs,
        metrics: Arc<SocketMetrics>,
        address_lookup: AddressLookupServices,
        path_selector: Arc<dyn PathSelector>,
        connection_path_selector: Option<Arc<dyn ConnectionPathSelector>>,
    ) -> Self {
        let reevaluation_interval = connection_path_selector
            .as_ref()
            .map_or_else(
                || path_selector.reevaluation_interval(),
                |selector| selector.reevaluation_interval(),
            )
            .map(|interval| interval.clamp(Duration::from_millis(250), Duration::from_secs(30)));
        let uses_path_health = connection_path_selector
            .as_ref()
            .is_some_and(|selector| selector.uses_path_health());
        Self {
            connections: FxHashMap::default(),
            member_activity: MemberActivity::default(),
            #[cfg(feature = "miopunch-test-hooks")]
            force_path_id_retry_once: None,
            state: State {
                endpoint_id,
                metrics: metrics.clone(),
                local_direct_addrs,
                mapped_addrs,
                address_lookup,
                connections_close: Default::default(),
                path_events: Default::default(),
                addr_events: Default::default(),
                paths: RemotePathState::new(metrics),
                last_holepunch: None,
                selected_path: Default::default(),
                scheduled_holepunch: None,
                scheduled_open_path: None,
                pending_open_paths: VecDeque::new(),
                address_lookup_stream: None,
                path_selector,
                connection_path_selector,
                reevaluation_interval,
                uses_path_health,
            },
        }
    }

    pub(super) fn start(
        self,
        initial_msgs: Vec<RemoteStateMessage>,
        tasks: &mut JoinSet<(EndpointId, Vec<RemoteStateMessage>)>,
        shutdown_token: CancellationToken,
        parent_span: Span,
    ) -> mpsc::Sender<RemoteStateMessage> {
        let (tx, rx) = mpsc::channel(16);
        let endpoint_id = self.state.endpoint_id;

        // Ideally we'd use the endpoint span as parent.  We'd have to plug that span into
        // here somehow.  Instead we have no parent and explicitly set the me attribute.  If
        // we don't explicitly set a span we get the spans from whatever call happens to
        // first create the actor, which is often very confusing as it then keeps those
        // spans for all logging of the actor.
        tasks.spawn(
            self.run(initial_msgs, rx, shutdown_token)
                .instrument(info_span!(
                    parent: parent_span,
                    "RemoteStateActor",
                    remote = %endpoint_id.fmt_short(),
                )),
        );
        tx
    }

    /// Runs the main loop of the actor.
    ///
    /// Note that the actor uses async handlers for tasks from the main loop.  The actor is
    /// not processing items from the inbox while waiting on any async calls.  So some
    /// discipline is needed to not turn pending for a long time.
    async fn run(
        mut self,
        initial_msgs: Vec<RemoteStateMessage>,
        mut inbox: mpsc::Receiver<RemoteStateMessage>,
        shutdown_token: CancellationToken,
    ) -> (EndpointId, Vec<RemoteStateMessage>) {
        trace!("actor started");
        for msg in initial_msgs {
            self.handle_message(msg).await;
        }
        let idle_timeout = time::sleep(ACTOR_MAX_IDLE_TIMEOUT);
        n0_future::pin!(idle_timeout);

        let check_connections = time::interval(UPGRADE_INTERVAL);
        n0_future::pin!(check_connections);

        let mut next_reevaluation = None;
        loop {
            let reevaluation_interval = self.state.reevaluation_interval.or_else(|| {
                self.connections
                    .values()
                    .any(|state| state.member_paths.is_some())
                    .then_some(MEMBER_REEVALUATION_INTERVAL)
            });
            if let Some(interval) = reevaluation_interval {
                next_reevaluation.get_or_insert_with(|| Instant::now() + interval);
            } else {
                next_reevaluation = None;
            }
            let reevaluation = match next_reevaluation {
                Some(when) => MaybeFuture::Some(time::sleep_until(when)),
                None => MaybeFuture::None,
            };
            n0_future::pin!(reevaluation);
            let scheduled_path_open = match self.state.scheduled_open_path {
                Some(when) => MaybeFuture::Some(time::sleep_until(when)),
                None => MaybeFuture::None,
            };
            n0_future::pin!(scheduled_path_open);
            let scheduled_hp = match self.state.scheduled_holepunch {
                Some(when) => MaybeFuture::Some(time::sleep_until(when)),
                None => MaybeFuture::None,
            };
            n0_future::pin!(scheduled_hp);
            if !self.is_idle(&inbox) {
                idle_timeout
                    .as_mut()
                    .reset(Instant::now() + ACTOR_MAX_IDLE_TIMEOUT);
            }

            tokio::select! {
                biased;

                _ = shutdown_token.cancelled() => {
                    trace!("actor cancelled");
                    break;
                }
                msg = inbox.recv() => {
                    match msg {
                        Some(msg) => self.handle_message(msg).await,
                        None => break,
                    }
                }
                Some((id, evt)) = self.state.path_events.next() => {
                    self.handle_path_event(id, evt);
                }
                Some((id, evt)) = self.state.addr_events.next() => {
                    trace!(?id, ?evt, "remote addrs updated, triggering holepunching");
                    if self.connections.get(&id).is_some_and(|state| state.member_paths.is_some()) {
                        self.update_member_udp(id, true);
                    } else {
                        self.trigger_holepunching();
                    }
                }
                Some(id) = self.member_activity.next() => {
                    self.update_member_udp(id, false);
                }
                Some((conn_id, closed)) = self.state.connections_close.next(), if !self.state.connections_close.is_empty() => {
                    self.handle_connection_close(conn_id, closed);
                }
                res = self.state.local_direct_addrs.updated() => {
                    if let Err(n0_watcher::Disconnected) = res {
                        trace!("direct address watcher disconnected, shutting down");
                        break;
                    }
                    self.update_local_direct_address();
                    self.update_active_member_udp();
                    trace!("local addrs updated, triggering holepunching");
                    self.trigger_holepunching();
                }
                _ = &mut scheduled_path_open => {
                    trace!("triggering scheduled path_open");
                    self.state.scheduled_open_path = None;
                    let mut addrs = std::mem::take(&mut self.state.pending_open_paths);
                    while let Some(addr) = addrs.pop_front() {
                        self.open_path_on_all_conns(&addr);
                    }
                }
                _ = &mut scheduled_hp => {
                    trace!("triggering scheduled holepunching");
                    self.state.scheduled_holepunch = None;
                    self.trigger_holepunching();
                }
                Some(item) = maybe_next(self.state.address_lookup_stream.as_mut()), if self.state.address_lookup_stream.is_some() => {
                    self.state.handle_address_lookup_item(item);
                    self.update_active_member_udp();
                }
                _ = check_connections.tick() => {
                    self.check_connections();
                }
                _ = &mut reevaluation => {
                    // Reschedule from now: a busy actor never catches up with a
                    // burst of delayed maintenance. No discovery or redial here.
                    next_reevaluation = reevaluation_interval.map(|interval| Instant::now() + interval);
                    self.reevaluate_paths();
                    self.update_active_member_udp();
                }
                _ = &mut idle_timeout => {
                    if self.is_idle(&inbox) {
                        trace!("idle timeout expired and still idle: terminate actor");
                        break;
                    } else {
                        // Seems like we weren't really idle, so we reset
                        idle_timeout.as_mut().reset(Instant::now() + ACTOR_MAX_IDLE_TIMEOUT);
                    }
                }
            }
        }

        inbox.close();
        // There might be a race between checking `inbox.is_empty()` and `inbox.close()`,
        // so we pull out all messages that are left over.
        let mut leftover_msgs = Vec::with_capacity(inbox.len());
        inbox.recv_many(&mut leftover_msgs, inbox.len()).await;

        trace!("actor terminating");
        (self.state.endpoint_id, leftover_msgs)
    }

    /// Returns `true` if the actor is fully idle.
    fn is_idle(&self, inbox: &mpsc::Receiver<RemoteStateMessage>) -> bool {
        self.connections.is_empty()
            && inbox.is_empty()
            && self.state.paths.resolve_requests_is_empty()
    }

    /// Handles an actor message.
    ///
    /// Error returns are fatal and kill the actor.
    #[instrument(skip(self))]
    async fn handle_message(&mut self, msg: RemoteStateMessage) {
        // trace!("handling message");
        match msg {
            RemoteStateMessage::SendDatagram(sender, transmit) => {
                self.state.handle_msg_send_datagram(sender, transmit).await;
            }
            RemoteStateMessage::AddConnection(handle, restricted, tx) => {
                self.handle_msg_add_connection(handle, restricted, tx);
            }
            RemoteStateMessage::AuthorizeCandidates {
                conn_id,
                allowed_data_custom_ids,
                member,
                tx,
            } => {
                let result =
                    self.authorize_candidates(ConnId(conn_id), allowed_data_custom_ids, member);
                let _ = tx.send(result);
            }
            #[cfg(not(wasm_browser))]
            RemoteStateMessage::PrepareMemberUdp {
                conn_id,
                policy,
                tx,
            } => {
                let result = self.prepare_member_udp(ConnId(conn_id), policy);
                let _ = tx.send(result);
            }
            RemoteStateMessage::OpenAdmittedDataPath {
                conn_id,
                remote,
                own_candidate,
                tx,
            } => {
                if own_candidate && tx.is_closed() {
                    return;
                }
                let result = if let Some(state) = self.connections.get(&ConnId(conn_id)) {
                    let fused_bootstrap = state.purpose == ConnectionPurpose::Control
                        && state.member_paths.as_ref().is_some_and(|paths| matches!(&remote, TransportAddr::Custom(address)
                            if paths.policy.requires_path_ack(&transports::Addr::Custom(address.clone()))
                                && paths.policy.permits(transports::Addr::Custom(address.clone()))));
                    if state.purpose != ConnectionPurpose::Ordinary && !fused_bootstrap {
                        Err(
                            "data carrier opening requires an admitted or pending member connection",
                        )
                    } else if let Some(conn) = state.handle.upgrade() {
                        let address = match remote {
                            TransportAddr::Ip(address) => transports::Addr::Ip(address),
                            TransportAddr::Relay(url) => {
                                transports::Addr::Relay(url, self.state.endpoint_id)
                            }
                            TransportAddr::Custom(address) => transports::Addr::Custom(address),
                            _ => {
                                let _ = tx.send(Err("unsupported data carrier"));
                                return;
                            }
                        };
                        let path = transports::PathBinding::from_remote(address);
                        self.state.open_path_inner(
                            ConnId(conn_id),
                            state,
                            &conn,
                            &path,
                            true,
                            own_candidate,
                        )
                    } else {
                        Err("connection closed")
                    }
                } else {
                    Err("connection not registered")
                };
                let _ = tx.send(result);
            }
            RemoteStateMessage::RetainAdmittedDataPath {
                conn_id,
                remote,
                tx,
            } => {
                let result = if let Some(state) = self.connections.get(&ConnId(conn_id)) {
                    let address = transports::PathBinding::from_remote(transports::Addr::Custom(
                        remote.clone(),
                    ));
                    if state.purpose != ConnectionPurpose::Ordinary || !state.may_select(&address) {
                        Err("data retention requires current connection-owned qualification")
                    } else if let Some(conn) = state
                        .handle
                        .upgrade()
                        .filter(|conn| conn.close_reason().is_none())
                    {
                        if let Some((id, _)) = state.paths.iter().find(|(_, path)| matches!(&path.address, transports::FourTuple::Custom { remote: actual, .. } if actual == &remote)) {
                            if let Some(path) = conn.path(*id) { let _ = path.ping(); }
                            Ok(())
                        } else { Err("qualified data path not established on original connection") }
                    } else {
                        Err("original connection closed")
                    }
                } else {
                    Err("connection not registered")
                };
                if result.is_ok() {
                    self.select_path();
                }
                let _ = tx.send(result);
            }
            RemoteStateMessage::RetainControlDataPath {
                conn_id,
                remote,
                tx,
            } => {
                let result = if let Some(state) = self.connections.get_mut(&ConnId(conn_id)) {
                    if state.purpose != ConnectionPurpose::Control
                        || state.retained_control_custom.is_some()
                    {
                        Err("data route retention requires an unsealed restricted connection")
                    } else if state.retired_custom.contains(&remote)
                        || !state
                            .mapped_addrs
                            .qualifies_connection_owned_custom(&remote, conn_id)
                    {
                        Err("control retention requires current connection-owned qualification")
                    } else if let Some(conn) = state
                        .handle
                        .upgrade()
                        .filter(|conn| conn.close_reason().is_none())
                    {
                        if state.paths.iter().any(|(id, path)| matches!(&path.address, transports::FourTuple::Custom { remote: actual, .. } if actual == &remote) && conn.path(*id).is_some()) {
                            state.retained_control_custom = Some(remote);
                            Ok(())
                        } else { Err("qualified control data route not established on original connection") }
                    } else {
                        Err("original connection closed")
                    }
                } else {
                    Err("connection not registered")
                };
                if result.is_ok() {
                    self.select_path();
                }
                let _ = tx.send(result);
            }
            RemoteStateMessage::HasRetainedControlDataPath {
                conn_id,
                remote,
                tx,
            } => {
                let current = self
                    .connections
                    .get(&ConnId(conn_id))
                    .is_some_and(|state| state.retained_control_data_is_current(&remote));
                let _ = tx.send(current);
            }
            #[cfg(feature = "miopunch-test-hooks")]
            RemoteStateMessage::TestLimitTransportPaths {
                conn_id,
                allowed,
                tx,
            } => {
                let result = if allowed.len() > 8 {
                    Err("fixture path mask exceeds bound")
                } else if let Some(conn) = self
                    .connections
                    .get(&ConnId(conn_id))
                    .and_then(|state| state.handle.upgrade())
                {
                    let mapped = self.state.mapped_addrs.clone();
                    conn.restrict_paths(move |address| {
                        mapped.to_transport_addr(address).is_some_and(|path| {
                            let path: TransportAddr = path.into();
                            allowed.contains(&path)
                        })
                    });
                    Ok(())
                } else {
                    Err("original connection closed")
                };
                let _ = tx.send(result);
            }
            RemoteStateMessage::OpenDataPath {
                conn_id,
                remote,
                #[cfg(not(wasm_browser))]
                local,
                keep_spare,
                tx,
            } => {
                if tx.is_closed() {
                    return;
                }
                #[cfg(not(wasm_browser))]
                eprintln!(
                    "[DEBUG-udp-handoff] actor-open-request stable_id={} local={local:?} remote={remote}",
                    conn_id
                );
                let result = if let Some(state) = self.connections.get(&ConnId(conn_id)) {
                    if let Some(conn) = state.handle.upgrade() {
                        if conn.side().is_client() {
                            #[cfg(feature = "miopunch-test-hooks")]
                            if self.force_path_id_retry_once == Some(ConnId(conn_id)) {
                                self.force_path_id_retry_once = None;
                                let _ = tx.send(Err(PATH_IDENTIFIERS_UNAVAILABLE));
                                return;
                            }
                            #[cfg(not(wasm_browser))]
                            if let Some(binding) = local.filter(|binding| !binding.id.is_primary())
                                && !state.udp_sockets.iter().any(|socket| {
                                    socket.id == binding.id
                                        && socket.is_live()
                                        && socket.binding() == binding
                                })
                            {
                                let _ = tx.send(Err(
                                    "UDP binding does not belong to original connection",
                                ));
                                return;
                            }
                            let path = transports::PathBinding::new(
                                transports::FourTuple::Ip {
                                    remote,
                                    #[cfg(not(wasm_browser))]
                                    local: local.map(|binding| binding.local_addr.ip()),
                                    #[cfg(wasm_browser)]
                                    local: None,
                                },
                                {
                                    #[cfg(not(wasm_browser))]
                                    {
                                        local.and_then(|binding| binding.id.to_noq())
                                    }
                                    #[cfg(wasm_browser)]
                                    {
                                        None
                                    }
                                },
                            );
                            self.state.open_path_inner(
                                ConnId(conn_id),
                                state,
                                &conn,
                                &path,
                                true,
                                keep_spare,
                            )
                        } else {
                            Err("only the dialing side can open a data path")
                        }
                    } else {
                        Err("connection closed")
                    }
                } else {
                    Err("connection not registered")
                };
                eprintln!(
                    "[DEBUG-udp-handoff] actor-open-result stable_id={} result={:?}",
                    conn_id,
                    result
                        .as_ref()
                        .map(|path| path.as_ref().map(|path| path.id()))
                );
                let _ = tx.send(result);
            }
            #[cfg(not(wasm_browser))]
            RemoteStateMessage::RetainUdpSocket {
                conn_id,
                socket,
                live,
                tx,
            } => {
                let valid_round = live.lock().expect("traversal lease");
                let result = match self.connections.get_mut(&ConnId(conn_id)) {
                    Some(state)
                        if *valid_round
                            && state.permits_udp_traversal()
                            && socket.connection_id == conn_id
                            && socket.remote == self.state.endpoint_id
                            && socket.is_live() =>
                    {
                        state.udp_sockets.retain(|socket| socket.is_live());
                        state.udp_sockets.push(ConnectionSocket(socket));
                        Ok(())
                    }
                    _ => Err("winner binding requires its live authorized original connection"),
                };
                let _ = tx.send(result);
            }
            RemoteStateMessage::AuthorizeUdpProbe { conn_id, tx } => {
                let result = match self.connections.get(&ConnId(conn_id)) {
                    Some(state) if state.permits_udp_traversal() => Ok(()),
                    _ => {
                        Err("UDP probe requires a live restricted or qualified original connection")
                    }
                };
                let _ = tx.send(result);
            }
            #[cfg(feature = "miopunch-test-hooks")]
            RemoteStateMessage::TestForcePathIdRetry { conn_id, tx } => {
                let result = match self.connections.get(&ConnId(conn_id)) {
                    Some(state) if state.purpose == ConnectionPurpose::Control => {
                        if state
                            .handle
                            .upgrade()
                            .is_some_and(|conn| conn.side().is_client())
                        {
                            self.force_path_id_retry_once = Some(ConnId(conn_id));
                            Ok(())
                        } else {
                            Err("test retry requires a live dialing connection")
                        }
                    }
                    _ => Err("test retry requires a restricted control connection"),
                };
                let _ = tx.send(result);
            }
            RemoteStateMessage::OpenControlPath {
                conn_id,
                remote,
                tx,
            } => {
                let result = if let Some(state) = self.connections.get(&ConnId(conn_id)) {
                    if state.purpose != ConnectionPurpose::Control {
                        Err("control path requires a restricted control connection")
                    } else if let Some(conn) = state.handle.upgrade() {
                        if conn.side().is_client() {
                            let remote = match remote {
                                TransportAddr::Custom(address) => transports::Addr::Custom(address),
                                TransportAddr::Relay(url) => {
                                    transports::Addr::Relay(url, self.state.endpoint_id)
                                }
                                _ => {
                                    let _ = tx.send(Err("unsupported control carrier"));
                                    return;
                                }
                            };
                            let path = transports::PathBinding::from_remote(remote);
                            self.state
                                .open_path_on_conn(ConnId(conn_id), state, &conn, &path, true)
                        } else {
                            Err("only the dialing side can open a control path")
                        }
                    } else {
                        Err("connection closed")
                    }
                } else {
                    Err("connection not registered")
                };
                let _ = tx.send(result);
            }
            #[cfg(feature = "miopunch-test-hooks")]
            RemoteStateMessage::TestReopenControlPath {
                conn_id,
                remote,
                tx,
            } => {
                let result = if let Some(state) = self.connections.get(&ConnId(conn_id)) {
                    if state.purpose != ConnectionPurpose::Control {
                        Err("test peer must still be in control phase")
                    } else if let Some(conn) = state.handle.upgrade() {
                        for (id, addr) in &state.paths {
                            if addr.is_custom() {
                                if let Some(path) = conn.path(*id) {
                                    let _ = path.close();
                                }
                            }
                        }
                        let tuple =
                            transports::PathBinding::from_remote(transports::Addr::Custom(remote));
                        let mapped = self.state.mapped_addrs.to_mapped_tuple(&tuple);
                        let attempt = conn.open_path_ensure(mapped, PathStatus::Available);
                        if attempt.path_id().is_some() {
                            Ok(())
                        } else {
                            Err("test path did not open")
                        }
                    } else {
                        Err("connection closed")
                    }
                } else {
                    Err("connection not registered")
                };
                let _ = tx.send(result);
            }
            RemoteStateMessage::CommitDataPath { conn_id, tx } => {
                let result = self.commit_data_path(ConnId(conn_id));
                let _ = tx.send(result);
            }
            RemoteStateMessage::RetireControlPath {
                conn_id,
                remote,
                tx,
            } => {
                let result = if let Some(state) = self.connections.get_mut(&ConnId(conn_id)) {
                    if state.purpose != ConnectionPurpose::Control {
                        Err("candidate retirement requires restricted original connection")
                    } else if let Some(conn) = state.handle.upgrade() {
                        state.retired_custom.insert(remote.clone());
                        let retired = state.retired_custom.clone();
                        let mapped = self.state.mapped_addrs.clone();
                        conn.restrict_paths(move |addr| !matches!(mapped.to_transport_addr(addr),Some(transports::Addr::Custom(custom)) if retired.contains(&custom)));
                        for (id, tuple) in conn.network_paths() {
                            if matches!(self.state.mapped_addrs.to_transport_addr(tuple.remote()),Some(transports::Addr::Custom(custom)) if custom==remote)
                                && let Some(path) = conn.path(id)
                            {
                                let _ = path.close();
                            }
                        }
                        Ok(())
                    } else {
                        Err("connection closed")
                    }
                } else {
                    Err("connection not registered")
                };
                let _ = tx.send(result);
            }
            RemoteStateMessage::ResolveRemote(addrs, tx) => {
                self.state.handle_msg_resolve_remote(addrs, tx);
                self.update_active_member_udp();
            }
            RemoteStateMessage::RemoteInfo(tx) => {
                let addrs = self.state.paths.to_remote_addrs();
                let info = RemoteInfo {
                    endpoint_id: self.state.endpoint_id,
                    addrs,
                };
                tx.send(info).ok();
            }
            RemoteStateMessage::NetworkChange { is_major } => {
                self.handle_msg_network_change(is_major);
            }
        }
    }

    /// Handles [`RemoteStateMessage::AddConnection`].
    ///
    /// Error returns are fatal and kill the actor.
    fn handle_msg_add_connection(
        &mut self,
        conn: noq::Connection,
        restricted: bool,
        tx: oneshot::Sender<PathStateReceiver>,
    ) {
        let (path_state_sender, path_state_receiver) = PathStateSender::new();
        self.state.metrics.num_conns_opened.inc();
        // Remove any conflicting stable_ids from the local state.
        let conn_id = ConnId(conn.stable_id());
        let scoped = self.state.mapped_addrs.clone();
        conn.restrict_paths(move |addr| match scoped.to_transport_addr(addr) {
            Some(transports::Addr::Custom(custom)) => {
                scoped.permits_custom_connection(&custom, conn_id.0)
            }
            Some(_) => true,
            None => false,
        });
        let data = self.state.mapped_addrs.clone();
        conn.restrict_application_data_paths(move |path| {
            match data.to_transport_addr(path.remote()) {
                Some(transports::Addr::Custom(custom)) => {
                    data.permits_custom_application_data(&custom, conn_id.0)
                }
                Some(_) => true,
                None => false,
            }
        });
        self.connections.remove(&conn_id);

        #[cfg(not(wasm_browser))]
        {
            let pool = self.state.mapped_addrs.udp_socket_pool.clone();
            let remote = self.state.endpoint_id;
            conn.restrict_network_paths(move |path| {
                path.local_socket_id()
                    .map(transports::LocalSocketId::from_noq)
                    .is_none_or(|id| {
                        pool.get(id).is_some_and(|socket| {
                            socket.remote == remote
                                && socket.connection_id == conn_id.0
                                && socket.is_pinned()
                        })
                    })
            });
        }

        // Hook up paths, NAT addresses and connection closed event streams.
        self.state
            .path_events
            .push(Box::pin(conn.path_events().map(move |evt| (conn_id, evt))));
        self.state.addr_events.push(Box::pin(
            conn.nat_traversal_updates().map(move |evt| (conn_id, evt)),
        ));
        self.state.connections_close.push(OnClosed::new(&conn));

        // Add local addrs to the connection
        if !restricted {
            let local_addrs = self.state.local_candidates();
            update_qnt_candidates(&conn, &local_addrs);
        }

        // Store the connection
        let conn_state = self
            .connections
            .entry(conn_id)
            .insert_entry(ConnectionState {
                connection_id: conn_id.0,
                mapped_addrs: self.state.mapped_addrs.clone(),
                handle: conn.weak_handle(),
                path_state: path_state_sender,
                paths: Default::default(),
                suspect_paths: Default::default(),
                pending_stream_handoff: None,
                has_been_direct: false,
                purpose: if restricted {
                    ConnectionPurpose::Control
                } else {
                    ConnectionPurpose::Ordinary
                },
                allowed_data_custom_ids: BTreeSet::new(),
                custom_data_policy: false,
                member_paths: None,
                retained_control_custom: None,
                retired_custom: HashSet::new(),
                #[cfg(not(wasm_browser))]
                udp_sockets: Vec::new(),
            })
            .into_mut();

        // Store PathId(0), set path_status and select best path, check if holepunching
        // is needed.
        if let Some(path) = conn.path(PathId::ZERO) {
            let path_remote = self
                .state
                .register_and_configure_path(conn_id, conn_state, &path);

            if !restricted
                && let Some(path_remote) = path_remote
                && !path_remote.is_relay()
                && conn.side().is_client()
            {
                // We may have raced this with a relay address.  Try and add any
                // relay addresses we have back.
                let relays = self
                    .state
                    .paths
                    .addrs()
                    .filter(|addr| addr.is_relay())
                    .map(|addr| transports::PathBinding::from_remote(addr.clone()))
                    .collect::<Vec<_>>();
                for open_addr in relays {
                    let _ = self
                        .state
                        .open_path_on_conn(conn_id, conn_state, &conn, &open_addr, false);
                }
            }
        }
        if !restricted {
            self.trigger_holepunching();
        }
        self.select_path();
        tx.send(path_state_receiver).ok();
    }

    /// Atomically forbids custom packet I/O on this connection before any
    /// application handle is allowed to send business traffic. The noq gate
    /// also covers packets queued by QUIC and remote path-open races.
    fn commit_data_path(&mut self, conn_id: ConnId) -> Result<(), &'static str> {
        let state = self
            .connections
            .get_mut(&conn_id)
            .ok_or("connection not registered")?;
        let conn = state.handle.upgrade().ok_or("connection closed")?;
        if conn.close_reason().is_some() {
            return Err("connection closed");
        }
        if state.purpose == ConnectionPurpose::Data {
            return Ok(());
        }
        if state.purpose != ConnectionPurpose::Control {
            return Err("only a restricted control connection can become data-only");
        }
        let Some((data_path_id, _)) = state
            .paths
            .iter()
            .find(|(id, addr)| addr.is_ip() && conn.path(**id).is_some())
        else {
            return Err("no established UDP data path");
        };
        let data_path_id = *data_path_id;
        let mapped = self.state.mapped_addrs.clone();
        conn.restrict_paths(move |addr| {
            matches!(
                mapped.to_transport_addr(addr),
                Some(transports::Addr::Ip(_))
            )
        });
        state.purpose = ConnectionPurpose::Data;
        if let Some(path) = conn.path(data_path_id) {
            let _ = path.set_status(PathStatus::Available);
        }
        for (id, addr) in &state.paths {
            if addr.is_custom() {
                if let Some(path) = conn.path(*id) {
                    match path.close() {
                        Ok(()) | Err(noq_proto::ClosePathError::ClosedPath) => {
                            // The peer may have committed first and abandoned
                            // this exact control path. Local closure is then
                            // already complete; preserve the validated IP path.
                        }
                        Err(_) => {
                            conn.close(0u32.into(), b"control path close failed");
                            return Err("could not close control path; connection closed");
                        }
                    }
                }
            }
        }
        let local_candidates = self.state.local_candidates();
        update_qnt_candidates(&conn, &local_candidates);
        self.select_path();
        Ok(())
    }

    /// Use the member QNT owner during restricted admission, holding the
    /// original contact open for protected control while UDP is verified.
    #[cfg(not(wasm_browser))]
    fn prepare_member_udp(
        &mut self,
        conn_id: ConnId,
        policy: MemberPathPolicy,
    ) -> Result<(), &'static str> {
        let state = self
            .connections
            .get_mut(&conn_id)
            .ok_or("connection not registered")?;
        let conn = state.handle.upgrade().ok_or("connection closed")?;
        let initial = conn
            .path(PathId::ZERO)
            .and_then(|path| path.network_path().ok())
            .ok_or("contact path lost")?;
        let bootstrap = initial.remote();
        let mapped = self.state.mapped_addrs.clone();
        let packet_policy = policy.clone();
        let restricted_contact = Arc::new(AtomicBool::new(true));
        let before_release = restricted_contact.clone();
        conn.restrict_paths(move |address| {
            !before_release.load(Ordering::Acquire)
                || address == bootstrap
                || mapped.to_transport_addr(address).is_some_and(|address| {
                    (matches!(address, transports::Addr::Ip(_))
                        || packet_policy.requires_path_ack(&address))
                        && packet_policy.permits(address)
                })
        });
        let mut paths = MemberPaths::new(policy);
        paths.gate_physical_probes(&conn, self.state.endpoint_id);
        if let Some(events) = paths.subscribe_building(conn_id) {
            conn.set_nat_traversal_probes_permitted(false);
            self.member_activity.push(events);
        }
        paths.restricted_contact = Some(restricted_contact);
        state.member_paths = Some(paths);
        self.update_member_udp(conn_id, false);
        Ok(())
    }

    /// Release ordinary carriers only after the full admission decision.
    fn authorize_candidates(
        &mut self,
        conn_id: ConnId,
        allowed_data_custom_ids: Vec<u64>,
        member: Option<MemberPathPolicy>,
    ) -> Result<(), &'static str> {
        let state = self
            .connections
            .get_mut(&conn_id)
            .ok_or("connection not registered")?;
        if !matches!(
            state.purpose,
            ConnectionPurpose::Control | ConnectionPurpose::Data
        ) {
            return Err("connection is not restricted");
        }
        let conn = state.handle.upgrade().ok_or("connection closed")?;
        if conn.close_reason().is_some() {
            return Err("connection closed");
        }
        // The actor processes this after the hook future completes. Recheck
        // live transport qualification here, never trust its earlier snapshot.
        let exact = state.retained_control_custom.clone();
        if let Some(address) = exact.as_ref()
            && !state.retained_control_data_is_current(address)
        {
            return Err("retained data route lost current qualification before release");
        }
        let allowed: BTreeSet<_> = allowed_data_custom_ids.into_iter().collect();
        let mapped = self.state.mapped_addrs.clone();
        let packet_allowed = allowed.clone();
        let retired = state.retired_custom.clone();
        conn.restrict_paths(move |addr| match mapped.to_transport_addr(addr) {
            Some(transports::Addr::Custom(custom)) => {
                !retired.contains(&custom)
                    && (packet_allowed.contains(&custom.id())
                        || (exact.as_ref() == Some(&custom)
                            && mapped.qualifies_connection_owned_custom(&custom, conn_id.0)))
            }
            Some(_) => true,
            None => false,
        });
        if let Some(policy) = member {
            // A wildcard first dial learns its source IP during the handshake.
            // Complete that initial binding before exact member-path checks.
            for (id, binding) in &mut state.paths {
                if !matches!(
                    binding.address,
                    transports::FourTuple::Ip { local: None, .. }
                ) {
                    continue;
                }
                let Some(path) = conn.path(*id) else { continue };
                let Some(actual) = path
                    .network_path()
                    .ok()
                    .and_then(|live| self.state.mapped_addrs.to_transport_tuple(&live))
                else {
                    continue;
                };
                if actual.remote() == binding.remote()
                    && actual.local_socket == binding.local_socket
                    && actual != *binding
                {
                    *binding = actual.clone();
                    state.path_state.record_opened(path.weak_handle(), actual);
                }
            }
            // The admitted member planner decides which newly advertised
            // addresses to probe, even if a bootstrap round is still retrying.
            conn.defer_probes_for_new_nat_addresses();
            let packet_policy = policy.clone();
            let mapped = self.state.mapped_addrs.clone();
            conn.restrict_paths(move |addr| {
                mapped
                    .to_transport_addr(addr)
                    .is_some_and(|address| packet_policy.permits(address))
            });
            if policy.acked_custom_transports.iter().any(Option::is_some) || policy.ack_native_relay {
                let mapped = self.state.mapped_addrs.clone();
                let packet_policy = policy.clone();
                conn.require_application_path_ack(move |path| {
                    mapped.to_transport_addr(path.remote())
                        .is_some_and(|address| packet_policy.requires_path_ack(&address))
                });
            }
            for activity in [policy.activity.clone(), policy.concurrent_business.clone()]
                .into_iter()
                .flatten()
            {
                let events = n0_future::stream::unfold(activity, move |mut activity| async move {
                    activity.changed().await.ok()?;
                    Some((conn_id, activity))
                });
                self.member_activity.push(Box::pin(events));
            }
            if let Some(paths) = &mut state.member_paths {
                paths.policy = policy;
                #[cfg(not(wasm_browser))]
                if let Some(contact) = paths.restricted_contact.take() {
                    contact.store(false, Ordering::Release);
                }
            } else {
                state.member_paths = Some(MemberPaths::new(policy));
            }
            #[cfg(not(wasm_browser))]
            if let Some(paths) = &state.member_paths {
                paths.gate_physical_probes(&conn, self.state.endpoint_id);
            }
            #[cfg(not(wasm_browser))]
            if let Some(events) = state
                .member_paths
                .as_mut()
                .and_then(|paths| paths.subscribe_building(conn_id))
            {
                conn.set_nat_traversal_probes_permitted(false);
                self.member_activity.push(events);
            }
        }
        state.allowed_data_custom_ids = allowed;
        state.custom_data_policy = true;
        for (id, path_addr) in &state.paths {
            if state.forbids(path_addr)
                && let Some(path) = conn.path(*id)
            {
                match path.close() {
                    Ok(()) | Err(noq_proto::ClosePathError::ClosedPath) => {}
                    Err(_) => {
                        conn.close(0u32.into(), b"signaling path close failed");
                        return Err("could not close signaling path");
                    }
                }
            }
        }
        state.purpose = ConnectionPurpose::Ordinary;
        if state.member_paths.is_none() {
            let local_candidates = self.state.local_candidates();
            update_qnt_candidates(&conn, &local_candidates);
            self.trigger_holepunching();
        }
        self.select_path();
        self.update_member_udp(conn_id, false);
        Ok(())
    }

    /// Handles [`RemoteStateMessage::NetworkChange`].
    fn handle_msg_network_change(&mut self, is_major: bool) {
        // Ping all the paths so loss-detection starts ASAP.
        for conn in self.connections.values() {
            if let Some(noq_conn) = conn.handle.upgrade() {
                for (path_id, addr) in &conn.paths {
                    if let Some(path) = noq_conn.path(*path_id) {
                        // Ping the current path
                        if let Err(err) = path.ping() {
                            warn!(%err, %path_id, ?addr, "failed to ping path");
                        }
                    }
                }
            }
        }

        if is_major {
            let now = Instant::now();
            for state in self.connections.values_mut() {
                if let Some(member) = &mut state.member_paths {
                    member.known_probes.clear();
                    member.response_deadlines.clear();
                    member.additional_direct = None;
                    member.last_exchange = [None, None];
                    member.network_changed_at = Some(now);
                    member.new_path_probes.clear();
                }
            }
            self.trigger_holepunching();
            self.select_path();
        }
    }

    fn update_active_member_udp(&mut self) {
        let active: Vec<_> = self
            .connections
            .iter()
            .filter(|(_, state)| {
                state.member_paths.as_ref().is_some_and(|member| {
                    member.policy.active() || member.policy.checks_enhanced_udp()
                })
            })
            .map(|(id, _)| *id)
            .collect();
        for id in active {
            self.update_member_udp(id, false);
        }
    }

    /// Retained standby and business both activate QNT on the original. A
    /// server advertisement also asks the client's existing role to participate.
    fn update_member_udp(&mut self, id: ConnId, peer_advertised: bool) {
        let Some(state) = self.connections.get_mut(&id) else {
            return;
        };
        let Some(member) = &state.member_paths else {
            return;
        };
        let trace_ipv6 = std::env::var_os("MIOPUNCH_TRACE_PEER_IPV6").is_some();
        let activity = member.policy.active();
        let active = activity || peer_advertised;
        let enhanced = member.policy.checks_enhanced_udp();
        if trace_ipv6 {
            eprintln!(
                "PEER_IPV6 member-update stable_id={} purpose={:?} activity={activity} peer_advertised={peer_advertised} enhanced={enhanced}",
                id.0, state.purpose,
            );
        }
        if !active && !enhanced {
            #[cfg(not(wasm_browser))]
            if let Some(member) = &mut state.member_paths {
                if let Some(conn) = state.handle.upgrade() {
                    if member.policy.building.is_some() { conn.stop_nat_traversal_probes(); }
                    conn.set_nat_traversal_probes_permitted(member.policy.building.is_none());
                }
                member.building = None;
                member
                    .physical
                    .lock()
                    .expect("ordinary physical leases")
                    .clear();
            }
            return;
        }
        let Some(conn) = state
            .handle
            .upgrade()
            .filter(|conn| conn.close_reason().is_none())
        else {
            return;
        };
        let now = Instant::now();
        let mut good_direct = false;
        let mut healthy_direct = Vec::new();
        let mut verified = BTreeSet::new();
        let mut healthy_tcp = false;
        for (path_id, binding) in &state.paths {
            let ip = match binding.remote() {
                transports::Addr::Ip(remote) => Some(remote),
                transports::Addr::Custom(addr)
                    if member.policy.acked_custom_transports[0] == Some(addr.id()) =>
                {
                    None
                }
                _ => continue,
            };
            if !state.path_is_qualified(*path_id, &conn) {
                continue;
            }
            let Some(stats) = conn.path_stats(*path_id) else {
                continue;
            };
            if !member.has_current_ack(&stats) {
                continue;
            }
            let healthy = !state.suspect_paths.contains_key(path_id);
            if let Some(remote) = ip {
                verified.insert(remote);
                good_direct |= healthy && stats.rtt <= GOOD_ENOUGH_LATENCY;
            } else {
                healthy_tcp |= healthy;
            }
            if healthy && !healthy_direct.contains(&binding) {
                healthy_direct.push(binding);
            }
        }
        let member = state
            .member_paths
            .as_mut()
            .expect("checked member connection");
        #[cfg(not(wasm_browser))]
        if member.policy.building.is_some() {
            member
                .response_deadlines
                .retain(|remote, deadline| now < *deadline && !verified.contains(remote));
            let waiting_for_peer = !member.response_deadlines.is_empty()
                || member.last_exchange.iter().flatten().any(|last| {
                    last.remote_candidates.is_empty()
                        && now - last.when < HOLEPUNCH_ATTEMPTS_INTERVAL
                });
            for (_, network) in conn.network_paths() {
                if let Some(binding) = self.state.mapped_addrs.to_transport_tuple(&network)
                    && let transports::Addr::Ip(remote) = binding.remote()
                {
                    member.response_deadlines.remove(&remote);
                }
            }
            let validating = member
                .new_path_probes
                .values()
                .any(|(deadline, _)| now < *deadline)
                || conn.network_paths().iter().any(|(path_id, network)| {
                    !state.paths.contains_key(path_id)
                        && conn
                            .path(*path_id)
                            .is_some_and(|path| path.status().is_ok())
                        && self
                            .state
                            .mapped_addrs
                            .to_transport_tuple(network)
                            .is_some_and(|binding| binding.is_ip())
                });
            member
                .physical
                .lock()
                .expect("ordinary physical leases")
                .retain(|scope, _| {
                    !conn.nat_traversal_probes_paused(scope.is_ipv6())
                        && conn.nat_traversal_probes_pending_for_family(scope.is_ipv6())
                });
            let in_flight = conn.nat_traversal_probes_pending() || waiting_for_peer || validating;
            if !in_flight && member.building.is_some() {
                conn.stop_nat_traversal_probes();
                conn.set_nat_traversal_probes_permitted(false);
                member.building = None;
                member
                    .physical
                    .lock()
                    .expect("ordinary physical leases")
                    .clear();
            }
            if in_flight && !validating {
                member.allow_probes(&conn, self.state.endpoint_id);
            }
            conn.set_nat_traversal_probes_permitted(member.building.is_some());
        }
        let ready_direct = healthy_direct.len();
        let concurrent = member
            .policy
            .concurrent_business
            .as_ref()
            .map(|demand| *demand.borrow());
        if concurrent == Some(false) {
            member.additional_direct = None;
        }
        let balancing = ready_direct != 0 && concurrent.is_some();
        if ready_direct != 0 && (!member.policy.business_active() || balancing) {
            member.policy.needs_enhanced_udp([false; 2]);
            if ready_direct != 1 || concurrent != Some(true) {
                #[cfg(not(wasm_browser))]
                {
                    if member.policy.building.is_some() {
                        conn.stop_nat_traversal_probes();
                    }
                    conn.set_nat_traversal_probes_permitted(member.policy.building.is_none());
                    member.building = None;
                    member
                        .physical
                        .lock()
                        .expect("ordinary physical leases")
                        .clear();
                }
                return;
            }
        }
        let needs_enhanced = !healthy_tcp && verified.is_empty();
        if !active {
            if trace_ipv6 {
                eprintln!(
                    "PEER_IPV6 member-qnt-skipped stable_id={} reason=inactive",
                    id.0
                );
            }
            member.policy.needs_enhanced_udp([needs_enhanced; 2]);
            return;
        }
        let ipv4_paused = conn.ipv4_nat_traversal_probes_paused();
        let ipv6_paused = conn.nat_traversal_probes_paused(true);
        let can_probe = |addr: &SocketAddr| {
            !healthy_tcp
                && if addr.ip().to_canonical().is_ipv6() {
                    !ipv6_paused
                } else {
                    !ipv4_paused
                }
        };
        let all_candidates = self.state.local_candidates();
        let limit = conn.local_nat_traversal_address_limit().unwrap_or(0);
        if all_candidates.len() > limit
            && ready_direct == 0
            && member
                .candidate_rotated_at
                .is_none_or(|last| now - last >= HOLEPUNCH_ATTEMPTS_INTERVAL)
        {
            if member.candidate_rotated_at.is_some() {
                member.candidate_cursor = member.candidate_cursor.wrapping_add((limit / 2).max(1));
            }
            member.candidate_rotated_at = Some(now);
        }
        let local_candidates =
            member_candidate_window(&all_candidates, limit, member.candidate_cursor);
        if local_candidates.is_empty() {
            if trace_ipv6 {
                eprintln!(
                    "PEER_IPV6 member-qnt-skipped stable_id={} reason=no-local-candidates",
                    id.0
                );
            }
            update_qnt_candidates(&conn, &local_candidates);
            member.policy.needs_enhanced_udp([needs_enhanced; 2]);
            return;
        }
        if balancing && conn.side().is_server() {
            // Advertisements let the QUIC client own the extra probe in
            // both business directions. Server probes could open a
            // different binding through the client's reciprocal challenge.
            let advertise = local_candidates.into_iter().filter(can_probe).collect();
            update_qnt_candidates(&conn, &advertise);
            return;
        }
        let advertised_remote = conn.get_remote_nat_traversal_addresses();
        let explicit: BTreeSet<_> = self
            .state
            .paths
            .addrs()
            .filter_map(|addr| match addr {
                transports::Addr::Ip(remote)
                    if member.policy.permits(addr.clone()) && !verified.contains(remote) =>
                {
                    Some(*remote)
                }
                _ => None,
            })
            .collect();
        let known: BTreeSet<_> = explicit
            .iter()
            .copied()
            .chain(
                advertised_remote
                    .as_ref()
                    .into_iter()
                    .flatten()
                    .copied()
                    .filter(|remote| {
                        member.policy.permits(transports::Addr::Ip(*remote))
                            && !verified.contains(remote)
                    }),
            )
            .collect();
        if trace_ipv6 {
            let local_ipv6: Vec<_> = local_candidates
                .iter()
                .filter(|addr| addr.is_ipv6())
                .collect();
            let known_ipv6: Vec<_> = known.iter().filter(|addr| addr.is_ipv6()).collect();
            let peer_ipv6: Vec<_> = conn
                .get_remote_nat_traversal_addresses()
                .unwrap_or_default()
                .into_iter()
                .filter(SocketAddr::is_ipv6)
                .collect();
            eprintln!(
                "PEER_IPV6 member-candidates stable_id={} side={:?} ipv4_paused={ipv4_paused} local={local_ipv6:?} known={known_ipv6:?} peer_announced={peer_ipv6:?} verified={verified:?}",
                id.0,
                conn.side(),
            );
        }
        member
            .known_probes
            .retain(|remote, _| known.contains(remote));
        let additional = if balancing {
            member
                .additional_direct
                .or_else(|| explicit.iter().chain(&known).copied().find(can_probe))
        } else {
            None
        };
        let may_retry = !good_direct || (balancing && member.additional_direct.is_none());
        for remote in known.iter().filter(|address| {
            can_probe(address)
                && if balancing {
                    Some(**address) == additional
                } else {
                    explicit.contains(*address)
                        || ((ipv4_paused || ipv6_paused) && can_probe(address))
                }
        }) {
            if member
                .known_probes
                .get(remote)
                .is_none_or(|last| may_retry && now - *last >= HOLEPUNCH_ATTEMPTS_INTERVAL)
                && member.allow_probes(&conn, self.state.endpoint_id)
                && conn.probe_known_nat_traversal_address(*remote).is_ok()
            {
                if trace_ipv6 {
                    eprintln!(
                        "PEER_IPV6 member-known-probe stable_id={} address={remote} at={now:?}",
                        id.0,
                    );
                }
                member.known_probes.insert(*remote, now);
                member
                    .response_deadlines
                    .insert(*remote, now + HOLEPUNCH_ATTEMPTS_INTERVAL);
                if balancing {
                    member.additional_direct = Some(*remote);
                }
            }
        }
        if balancing {
            return;
        }
        let fallback = std::array::from_fn(|family| {
            let ipv6 = family == 1;
            let probing = member
                .known_probes
                .iter()
                .any(|(remote, sent)| remote.is_ipv6() == ipv6 && now - *sent < LOCAL_PROBE_GRACE);
            let recent_exchange = member.last_exchange[family]
                .as_ref()
                .is_some_and(|last| now - last.when < LOCAL_PROBE_GRACE);
            needs_enhanced && !probing && !recent_exchange
        });
        if conn.side().is_server() {
            // At first activation prefer local probes. Only advertise when a
            // local path could not be verified or none can be attempted here.
            // IPv6 advertisements use their own verification state.
            if let Ok(announced) = conn.get_local_nat_traversal_addresses() {
                let have_ipv6 = verified
                    .iter()
                    .any(|addr| addr.ip().to_canonical().is_ipv6());
                let advertise = local_candidates
                    .into_iter()
                    .filter(|addr| {
                        announced.contains(addr)
                            || (!member.known_probes.iter().any(|(remote, sent)| {
                                remote.is_ipv6() == addr.is_ipv6()
                                    && now - *sent < LOCAL_PROBE_GRACE
                            }) && can_probe(addr)
                                && (verified.is_empty()
                                    || (addr.ip().to_canonical().is_ipv6() && !have_ipv6)))
                    })
                    .collect();
                update_qnt_candidates(&conn, &advertise);
            }
            member.policy.needs_enhanced_udp(fallback);
            return;
        }
        let Ok(remote) = advertised_remote else {
            member.policy.needs_enhanced_udp(fallback);
            return;
        };
        let announced = conn.get_local_nat_traversal_addresses().unwrap_or_default();
        let advertise = local_candidates
            .iter()
            .copied()
            .filter(|address| can_probe(address) || announced.contains(address))
            .collect();
        update_qnt_candidates(&conn, &advertise);
        let mut started = [false; 2];
        for ipv6 in [false, true] {
            if if ipv6 { ipv6_paused } else { ipv4_paused } {
                continue;
            }
            let family = usize::from(ipv6);
            let local: BTreeSet<_> = local_candidates
                .iter()
                .copied()
                .filter(|addr| addr.is_ipv6() == ipv6 && can_probe(addr))
                .collect();
            if local.is_empty() {
                continue;
            }
            let remote: BTreeSet<_> = remote
                .iter()
                .copied()
                .filter(|address| {
                    address.is_ipv6() == ipv6
                        && member.policy.permits(transports::Addr::Ip(*address))
                })
                .collect();
            let probing_family = member.known_probes.iter().any(|(remote, sent)| {
                remote.is_ipv6() == ipv6 && can_probe(remote) && now - *sent < LOCAL_PROBE_GRACE
            });
            if probing_family
                || (!verified.is_empty()
                    && remote.is_subset(&verified)
                    && !known.iter().any(|addr| addr.is_ipv6() == ipv6))
            {
                continue;
            }
            if member.last_exchange[family].as_ref().is_some_and(|last| {
                local.is_subset(&last.local_candidates)
                    && remote.is_subset(&last.remote_candidates)
                    && (good_direct || now - last.when < HOLEPUNCH_ATTEMPTS_INTERVAL)
            }) {
                continue;
            }
            if !member.allow_probes(&conn, self.state.endpoint_id) {
                continue;
            }
            let exchange = conn.initiate_nat_traversal_round_for_family(ipv6, |addr| {
                addr.is_ipv6() == ipv6
                    && can_probe(&addr)
                    && remote.contains(&addr)
                    && !verified.contains(&addr)
            });
            if trace_ipv6 && ipv6 {
                eprintln!(
                    "PEER_IPV6 member-qnt-exchange stable_id={} result={exchange:?} at={now:?}",
                    id.0
                );
            }
            if exchange.is_ok() {
                started[family] = true;
                self.state.metrics.holepunch_attempts.inc();
                member.last_exchange[family] = Some(HolepunchAttempt {
                    when: now,
                    local_candidates: local,
                    remote_candidates: remote,
                });
                trace!(
                    stable_id = id.0,
                    ipv6, "member QNT candidate exchange on original connection"
                );
            }
        }
        member
            .policy
            .needs_enhanced_udp(std::array::from_fn(|family| {
                !started[family] && fallback[family]
            }));
    }

    fn handle_connection_close(&mut self, conn_id: ConnId, closed: Closed) {
        event!(
            target: "iroh::_events::conn::closed",
            Level::DEBUG,
            %conn_id,
            remote_id = %self.state.endpoint_id.fmt_short(),
            reason=?closed.reason,
        );

        if let Some(conn_state) = self.connections.remove(&conn_id) {
            self.state.metrics.num_conns_closed.inc();
            conn_state.path_state.close(closed);
        }
        // A per-connection winner can disappear while another M is still open.
        // Recompute peer hints instead of retaining that retired association.
        self.select_path();
    }

    /// Updates the local [`DirectAddr`]s to all connections.
    ///
    /// Each connection needs to have the local direct addresses to use as QNT address
    /// candidates.
    fn update_local_direct_address(&mut self) {
        let local_addrs = self.state.local_candidates();
        for state in self.connections.values().filter(|state| {
            state.purpose != ConnectionPurpose::Control && state.member_paths.is_none()
        }) {
            if let Some(conn) = state.handle.upgrade() {
                update_qnt_candidates(&conn, &local_addrs);
            }
        }
        // todo: trace
    }

    /// Triggers holepunching to the remote endpoint.
    ///
    /// This will manage the entire process of holepunching with the remote endpoint.
    ///
    /// - Holepunching happens on the Connection with the lowest [`ConnId`] which is a
    ///   client.
    ///   - Both endpoints may initiate holepunching if both have a client connection.
    ///   - Any opened paths are opened on all other connections without holepunching.
    /// - If there are no changes in local or remote candidate addresses since the
    ///   last attempt **and** there was a recent attempt, a trigger_holepunching call
    ///   will be scheduled instead.
    fn trigger_holepunching(&mut self) {
        if self.connections.is_empty() {
            trace!("not holepunching: no connections");
            return;
        }

        let Some(conn) = self
            .connections
            .iter()
            .filter(|(_, state)| {
                state.purpose != ConnectionPurpose::Control && state.member_paths.is_none()
            })
            .filter_map(|(id, state)| state.handle.upgrade().map(|conn| (*id, conn)))
            .filter(|(_, conn)| conn.side().is_client())
            .min_by_key(|(id, _)| *id)
            .map(|(_, conn)| conn)
        else {
            trace!("not holepunching: no client connection");
            return;
        };
        let remote_candidates = match conn.get_remote_nat_traversal_addresses() {
            Ok(addrs) => BTreeSet::from_iter(addrs),
            Err(err) => {
                warn!("failed to get nat candidate addresses: {err:#}");
                return;
            }
        };
        let local_candidates = self.state.local_candidates();
        let new_candidates = self
            .state
            .last_holepunch
            .as_ref()
            .map(|last_hp| {
                // Addrs are allowed to disappear, but if there are new ones we need to
                // holepunch again.
                trace!(
                    ?last_hp,
                    ?local_candidates,
                    ?remote_candidates,
                    "candidates to holepunch?"
                );
                !remote_candidates.is_subset(&last_hp.remote_candidates)
                    || !local_candidates.is_subset(&last_hp.local_candidates)
            })
            .unwrap_or(true);
        if !new_candidates && let Some(ref last_hp) = self.state.last_holepunch {
            let next_hp = last_hp.when + HOLEPUNCH_ATTEMPTS_INTERVAL;
            let now = Instant::now();
            if next_hp > now {
                trace!(scheduled_in = ?(next_hp - now), "not holepunching: no new addresses");
                self.state.scheduled_holepunch = Some(next_hp);
                return;
            }
        }

        self.state.do_holepunching(conn);
    }

    #[instrument(skip(self))]
    fn handle_path_event(&mut self, conn_id: ConnId, event: Result<NoqPathEvent, noq::Lagged>) {
        let Ok(event) = event else {
            warn!("missed a PathEvent, RemoteStateActor lagging");
            // TODO: Is it possible to recover using the sync APIs to figure out what the
            //    state of the connection and it's paths are?
            return;
        };
        let Some(conn_state) = self.connections.get_mut(&conn_id) else {
            trace!("event for removed connection");
            return;
        };
        let Some(conn) = conn_state.handle.upgrade() else {
            trace!("event for closed connection");
            return;
        };
        trace!("path event");
        if matches!(
            &event,
            NoqPathEvent::Established { .. }
                | NoqPathEvent::Abandoned { .. }
                | NoqPathEvent::Discarded { .. }
        ) {
            eprintln!(
                "[DEBUG-udp-handoff] actor-path-event stable_id={} event={event:?}",
                conn_id.0
            );
        }
        match event {
            NoqPathEvent::Established { id: path_id, .. } => {
                let Some(path) = conn.path(path_id) else {
                    trace!("path open event for unknown path");
                    return;
                };

                let binding = self
                    .state
                    .register_and_configure_path(conn_id, conn_state, &path);
                if std::env::var_os("MIOPUNCH_TRACE_PEER_IPV6").is_some()
                    && let Some(binding) = &binding
                    && matches!(binding.remote(), transports::Addr::Ip(addr) if addr.ip().to_canonical().is_ipv6())
                {
                    eprintln!(
                        "PEER_IPV6 actor-path-established stable_id={} purpose={:?} path_id={path_id} binding={binding:?}",
                        conn_id.0, conn_state.purpose,
                    );
                }
                let member_return_path = binding
                    .as_ref()
                    .is_some_and(|binding| conn_state.is_native_member_path(binding));
                if let (Some(member), Some(_)) = (&mut conn_state.member_paths, binding) {
                    if member_return_path {
                        // Keep the new carrier's qualification ACKs on its own binding.
                        // Either side can release when its new path becomes Available.
                        if let Ok(probe) = path.start_return_path_probe() {
                            let interval = self
                                .state
                                .reevaluation_interval
                                .unwrap_or(MEMBER_REEVALUATION_INTERVAL);
                            let budget = 2 * interval.max(path.stats().probe_timeout);
                            member
                                .new_path_probes
                                .insert(path_id, (Instant::now() + budget, probe));
                        }
                    } else {
                        let _ = path.start_probe();
                    }
                }
                self.select_path();
            }
            NoqPathEvent::Abandoned { id, reason, .. } => {
                if let Some(member) = &mut conn_state.member_paths {
                    for (path_id, network) in conn
                        .network_paths()
                        .into_iter()
                        .filter(|(path_id, _)| *path_id == id)
                    {
                        let _ = path_id;
                        if let Some(binding) = self.state.mapped_addrs.to_transport_tuple(&network)
                            && let transports::Addr::Ip(remote) = binding.remote()
                        {
                            member.response_deadlines.remove(&remote);
                        }
                    }
                }
                // Remove abandoned path from the conn state.
                let Some(network_path) = conn_state.remove_path(&id, &conn) else {
                    debug!(%id, "path not in path_id_map");
                    return;
                };

                // We track all known remote addresses for the peer in `State::paths`. The
                // paths are tracked by remote address only (we ignore the local
                // IP). Therefore, we mark a remote addr as abandoned in the remote-global
                // state only once no connections have any path to that remote addr.
                if !conn_state
                    .paths
                    .values()
                    .any(|tuple| tuple.remote() == network_path.remote())
                {
                    self.state.paths.abandoned_path(&network_path.remote());
                }

                event!(
                    target: "iroh::_events::path::abandoned",
                    Level::DEBUG,
                    remote = %self.state.endpoint_id.fmt_short(),
                    %conn_id,
                    path_id = %id,
                    %network_path,
                    ?reason
                );

                // If the remote closed our selected path, select a new one.
                self.select_path();
            }
            NoqPathEvent::Discarded { id, path_stats, .. } => {
                trace!(%id, ?path_stats, "path discarded");
                #[cfg(not(wasm_browser))]
                {
                    let network_paths = conn.network_paths();
                    let live_bindings = network_paths
                        .into_iter()
                        .filter_map(|(_, path)| {
                            path.local_socket_id()
                                .map(transports::LocalSocketId::from_noq)
                        })
                        .collect::<BTreeSet<_>>();
                    conn_state.udp_sockets.retain(|socket| {
                        !socket.has_used_path() || live_bindings.contains(&socket.id)
                    });
                }
            }
            NoqPathEvent::ReturnConfirmed { .. } => self.select_path(),
            NoqPathEvent::RemoteStatus { .. } | NoqPathEvent::ObservedAddr { .. } => {
                // Nothing to do for these events.
            }
            _ => {
                // We expect to keep noq and iroh in sync in all test setups, but in production it's totally possible
                // that iroh itself is linked against a newer version of noq with additional events we don't yet
                // know how to handle.
                #[cfg(test)]
                panic!("Unhandled path event: {event:?}");
            }
        }
    }

    /// Refresh only live, qualified paths in existing connections. QUIC retains
    /// its own path/connection timeouts and reports final failure to callers.
    fn reevaluate_paths(&mut self) {
        let health_checks = self.state.uses_path_health;
        for state in self
            .connections
            .values()
            .filter(|state| {
                state.purpose != ConnectionPurpose::Control
                    && (self.state.reevaluation_interval.is_some() || state.member_paths.is_some())
            })
        {
            if let Some(conn) = state
                .handle
                .upgrade()
                .filter(|conn| conn.close_reason().is_none())
            {
                for (id, address) in &state.paths {
                    if state.may_select(address)
                        && let Some(path) = conn.path(*id)
                    {
                        if state.is_native_member_path(address)
                            || !(health_checks || state.member_paths.is_some())
                        {
                            let _ = path.ping();
                        } else {
                            let _ = path.start_probe();
                        }
                    }
                }
            }
        }
        self.select_path();
    }

    /// A locally withdrawn data carrier cannot remain a scored candidate or
    /// be reopened by the peer. Preserve the admission allowlist and retire
    /// only this Connection's exact addresses. Raw and restricted consumers
    /// retain their existing lifecycle; the transport still owns its shutdown.
    fn retire_unavailable_custom_paths(&mut self) {
        let Some(senders) = &self.state.mapped_addrs.custom_route_senders else {
            return;
        };
        for state in self.connections.values_mut().filter(|state| {
            state.custom_data_policy && state.purpose == ConnectionPurpose::Ordinary
        }) {
            let mut changed = false;
            for path in state.paths.values() {
                if let transports::FourTuple::Custom { remote, .. } = &path.address
                    && !senders
                        .iter()
                        .any(|sender| sender.is_valid_send_addr(remote))
                {
                    changed |= state.retired_custom.insert(remote.clone());
                }
            }
            if !changed {
                continue;
            }
            let Some(conn) = state
                .handle
                .upgrade()
                .filter(|conn| conn.close_reason().is_none())
            else {
                continue;
            };
            let mapped = self.state.mapped_addrs.clone();
            let allowed = state.allowed_data_custom_ids.clone();
            let retired = state.retired_custom.clone();
            conn.restrict_paths(move |addr| match mapped.to_transport_addr(addr) {
                Some(transports::Addr::Custom(custom)) => {
                    allowed.contains(&custom.id()) && !retired.contains(&custom)
                }
                Some(_) => true,
                None => false,
            });
            for (id, address) in &state.paths {
                if state.forbids(address)
                    && let Some(path) = conn.path(*id)
                {
                    let _ = path.close();
                }
            }
        }
    }

    /// Selects the preferred path by invoking the configured [`PathSelector`].
    ///
    /// Legacy selectors run once over the peer aggregate. Explicit connection
    /// selectors run later, once for each live connection's qualified paths.
    #[instrument(skip_all)]
    fn select_path(&mut self) {
        self.retire_unavailable_custom_paths();
        let current_path = self.state.selected_path.as_ref();
        let selected_addr = {
            let ctx = PathSelectionContext::new(current_path, &self.connections);
            self.state.path_selector.select(&ctx).selected().cloned()
        };

        if let Some(addr) = selected_addr
            && self.state.selected_path.as_ref() != Some(&addr)
        {
            let prev_remote = self.state.selected_path.replace(addr.clone());
            event!(
                target: "iroh::_events::path::selected",
                Level::DEBUG,
                remote = %self.state.endpoint_id.fmt_short(),
                network_path = %addr,
                prev_network_path = %prev_remote.map(|p| format!("{p}")).unwrap_or("None".to_string()),
            );
        } else if !self.connections.values().any(|state| {
            state.purpose != ConnectionPurpose::Control
                && state.paths.values().any(|path| {
                    Some(path) == self.state.selected_path.as_ref() && state.may_select(path)
                })
        }) {
            self.state.selected_path = None;
        }

        self.apply_selected_path();
    }

    /// Applies connection-local selection to noq, retaining a qualified peer
    /// candidate only as a hint for ordinary data connections.
    ///
    /// Iterates over all connections and applies the selected path as follows:
    /// - Closes non-selected IP paths (but keeps one IP path open still)
    /// - Sets all non-selected paths to [`PathStatus::Backup`]
    /// - Opens the selected path if it does not exist on the connection
    /// - Sets the selected path to [`PathStatus::Available`]
    fn apply_selected_path(&mut self) {
        let now = Instant::now();
        let health_interval = self
            .state
            .reevaluation_interval
            .filter(|_| self.state.uses_path_health);
        for (conn_id, conn_state) in self.connections.iter_mut() {
            let Some(conn) = conn_state
                .handle
                .upgrade()
                .filter(|conn| conn.close_reason().is_none())
            else {
                continue;
            };
            // Share only a qualified ordinary data candidate. A restricted
            // exchange must keep its own paths while it negotiates admission.
            if conn_state.purpose == ConnectionPurpose::Ordinary
                && let Some(selected) = self.state.selected_path.clone()
                && conn_state.may_select(&selected)
                && !(conn_state.member_paths.is_some() && selected.is_ip())
                && !conn_state.requires_path_ack(&selected)
            {
                let _ = self
                    .state
                    .open_path_on_conn(*conn_id, conn_state, &conn, &selected, false);
            }
            if self.state.connection_path_selector.is_none()
                && conn_state.purpose == ConnectionPurpose::Ordinary
                && self.state.selected_path.is_none()
                && conn_state.member_paths.is_none()
            {
                // The legacy empty result does not create a selection.
                continue;
            }
            let current_id = conn_state.path_state.selected_id();
            let health_interval = health_interval.or_else(|| {
                conn_state.member_paths.as_ref().map(|_| {
                    self.state
                        .reevaluation_interval
                        .unwrap_or(MEMBER_REEVALUATION_INTERVAL)
                })
            });
            if let Some(interval) = health_interval
                && conn_state.purpose != ConnectionPurpose::Control
            {
                conn_state.update_path_health(&conn, now, interval);
                conn_state.confirm_member_backups(&conn, current_id, now, interval);
            }
            let current = current_id.filter(|id| conn_state.path_is_qualified(*id, &conn));
            let context = PathSelectionContext::for_connection(
                current.and_then(|id| conn_state.paths.get(&id)),
                conn_state,
                health_interval.map(|interval| (now, interval)),
            );
            if let Some(member) = &conn_state.member_paths
                && let Some(health) = &member.policy.direct_health
            {
                let healthy = context.paths().any(|path| {
                    !path.is_suspect()
                        && (path.network_path().is_ip()
                            || matches!(path.network_path(), transports::FourTuple::Custom { remote, .. }
                                if member.policy.acked_custom_transports[0] == Some(remote.id())))
                        && path.stats().is_some_and(|stats| stats.acknowledgment.is_some())
                });
                health.send_if_modified(|previous| {
                    let changed = *previous != healthy;
                    *previous = healthy;
                    changed
                });
            }
            let fused = context.is_member_fusion();
            let mut stream_paths = Vec::new();
            let selected_for_conn = if conn_state.purpose == ConnectionPurpose::Control
                && let Some(exact) = conn_state.retained_control_custom.as_ref()
            {
                // Confirmation must traverse the newly qualified exact route,
                // even when the signaling path has a lower measured latency.
                conn_state.paths.iter().find_map(|(id, path)| (matches!(&path.address, transports::FourTuple::Custom { remote, .. } if remote == exact) && conn_state.may_select(path)).then_some(*id))
            } else if let Some(selector) = &self.state.connection_path_selector {
                let decision = selector.select(&context);
                let selected = decision.for_connection(conn_state, &conn).or(current);
                stream_paths = decision.stream_paths;
                selected
            } else if conn_state.purpose == ConnectionPurpose::Ordinary
                && conn_state.member_paths.is_none()
                && let Some(selected) = self.state.selected_path.as_ref().and_then(|selected| {
                    conn_state.paths.iter().find_map(|(id, path)| {
                        (selected == path && conn_state.path_is_qualified(*id, &conn))
                            .then_some(*id)
                    })
                })
            {
                // Apply the peer decision without invoking the legacy callback
                // again. A hint never transfers another connection's capability.
                Some(selected)
            } else {
                // Restricted paths, or a peer preference that this connection
                // cannot own, use the same built-in scorer on qualified paths.
                let decision =
                    super::super::biased_rtt_path_selector::BiasedRttPathSelector::default()
                        .select(&context);
                let selected = decision.for_connection(conn_state, &conn).or(current);
                stream_paths = decision.stream_paths;
                selected
            };

            let selected_binding = selected_for_conn.and_then(|id| conn_state.paths.get(&id));
            let standby_on_direct = conn_state.member_paths.as_ref().is_some_and(|member| {
                !member.policy.business_active()
                    && selected_binding.is_some_and(|binding| binding.is_ip())
                    && selected_for_conn.is_some_and(|id| {
                        !conn_state.suspect_paths.contains_key(&id)
                            && conn.path_stats(id).is_some_and(|stats| {
                                member.has_current_ack(&stats) && context.has_fresh_ack(&stats)
                            })
                    })
            });
            if fused {
                let mut seen_bindings = Vec::new();
                let mut business_paths = Vec::new();
                // Prefer the current binding's exact path ID if more than one
                // QUIC path names that same binding.
                if let Some(id) = selected_for_conn
                    && stream_paths.contains(&(conn_state.connection_id, id))
                {
                    seen_bindings.extend(conn_state.paths.get(&id));
                    business_paths.push(id);
                }
                for (owner, id) in stream_paths {
                    if owner == conn_state.connection_id
                        && conn_state.path_is_qualified(id, &conn)
                        && let Some(binding) = conn_state.paths.get(&id)
                        && !seen_bindings.contains(&binding)
                    {
                        seen_bindings.push(binding);
                        business_paths.push(id);
                    }
                }
                conn.set_business_paths(selected_for_conn, &business_paths);
            }

            let is_selected = |id, binding| {
                if fused {
                    selected_for_conn == Some(id)
                } else {
                    selected_binding == Some(binding)
                }
            };
            let failed_current = current_id.filter(|id| {
                if fused {
                    Some(*id) != selected_for_conn
                        && (current.is_none() || conn_state.suspect_paths.contains_key(id))
                } else {
                    conn_state.suspect_paths.contains_key(id)
                        && current.is_some()
                        && conn_state.paths.get(id) != selected_binding
                }
            });
            let handoff = conn_state.pending_stream_handoff.or(failed_current);
            if let Some(repair) = conn_state
                .member_paths
                .as_ref()
                .and_then(|member| member.policy.repair.as_ref())
            {
                let failed =
                    selected_for_conn.is_none_or(|id| conn_state.suspect_paths.contains_key(&id));
                repair.send_if_modified(|current| {
                    if *current == failed {
                        return false;
                    }
                    *current = failed;
                    true
                });
            }
            let mut activated = false;
            for (path_id, path_fourtuple) in conn_state.paths.iter() {
                let Some(path) = conn.path(*path_id) else {
                    continue;
                };
                if (conn_state.purpose == ConnectionPurpose::Data && path_fourtuple.is_custom())
                    || conn_state.forbids(path_fourtuple)
                {
                    let _ = path.close();
                    continue;
                }

                // Keep paths with business or validation work. A healthy
                // direct standby needs only its selected IP binding.
                let keep_member_path = conn_state.member_paths.as_ref().is_some_and(|member| {
                    conn.business_on_path(*path_id)
                        || member.new_path_probes.get(path_id).is_some_and(|(deadline, _)| now < *deadline)
                        || (!standby_on_direct
                            && selected_binding != Some(path_fourtuple)
                            && !conn_state.suspect_paths.contains_key(path_id)
                            && context.has_fresh_ack(&path.stats()))
                });
                // Keep a health-demoted original binding until it proves recovery
                // or QUIC retires it.
                //
                // Relay and custom paths are kept open. Only the client closes paths,
                // to avoid the client and server independently closing different paths
                // and racing to abandon the last one.
                if conn.side().is_client()
                    && path_fourtuple.is_ip()
                    && !keep_member_path
                    && handoff != Some(*path_id)
                    && !conn_state
                        .suspect_paths
                        .get(path_id)
                        .is_some_and(|suspect| suspect.retained_for_recovery)
                    && selected_for_conn.is_some()
                    && !is_selected(*path_id, path_fourtuple)
                    && conn_state.paths.values().filter(|a| a.is_ip()).count() > 1
                {
                    trace!(?path_fourtuple, %conn_id, %path_id, "closing direct path");
                    if std::env::var_os("MIOPUNCH_TRACE_PEER_IPV6").is_some()
                        && matches!(path_fourtuple.remote(), transports::Addr::Ip(addr) if addr.ip().to_canonical().is_ipv6())
                    {
                        let stats = path.stats();
                        eprintln!(
                            "PEER_IPV6 actor-path-close stable_id={} path_id={path_id} selected={selected_for_conn:?} rtt={:?} has_ack={}",
                            conn_id.0, stats.rtt, stats.acknowledgment.is_some(),
                        );
                    }
                    match path.close() {
                        Err(noq_proto::ClosePathError::MultipathNotNegotiated) => {
                            error!("multipath not negotiated");
                        }
                        Err(noq_proto::ClosePathError::LastOpenPath) => {
                            error!("could not close last open path");
                        }
                        Err(noq_proto::ClosePathError::ClosedPath) => {
                            // We already closed this.
                        }
                        Ok(()) => {}
                    }
                    continue;
                }

                // Set path status: The selected path becomes Available, all other paths
                // become Backup.
                let status = if is_selected(*path_id, path_fourtuple) {
                    PathStatus::Available
                } else {
                    PathStatus::Backup
                };
                if path.set_status(status).is_ok() && status == PathStatus::Available {
                    activated = true;
                }
            }

            if activated {
                conn_state.pending_stream_handoff = None;
                if let Some(id) = handoff.filter(|id| Some(*id) != selected_for_conn)
                    && let Some(path) = conn.path(id)
                {
                    // PTO probes the old carrier. Once a replacement is Available,
                    // let ordinary scheduling send the old path's outstanding data.
                    path.requeue_unacknowledged_stream_frames();
                    if let Some(suspect) = conn_state.suspect_paths.get_mut(&id) {
                        suspect.retained_for_recovery = true;
                    }
                }
                for (&id, suspect) in &mut conn_state.suspect_paths {
                    if Some(id) != selected_for_conn
                        && conn.business_on_path(id)
                        && let Some(path) = conn.path(id)
                    {
                        // A split stream can lose its backup without a
                        // connection-level selection change.
                        path.requeue_unacknowledged_stream_frames();
                        suspect.retained_for_recovery = true;
                    }
                }
            } else if fused {
                // Selection can become empty while the replacement is being
                // qualified. Preserve the old carrier across that gap.
                conn_state.pending_stream_handoff = handoff;
            }

            if let Some(member) = &mut conn_state.member_paths {
                member.new_path_probes.retain(|id, (deadline, probe)| {
                    now < *deadline
                        && !(activated && selected_for_conn == Some(*id))
                        && probe.is_acknowledged().is_ok()
                });
            }

            // Record the new selected path in the path watcher.
            if fused || selected_for_conn.is_some() {
                if fused && current_id != selected_for_conn
                    && std::env::var_os("MIOPUNCH_TRACE_PEER_IPV6").is_some()
                {
                    eprintln!(
                        "PEER_IPV6 member-selected stable_id={} before={current_id:?} after={selected_for_conn:?} binding={selected_binding:?}",
                        conn_id.0,
                    );
                }
                conn_state.path_state.record_selected(selected_for_conn);
            }
        }
    }

    fn open_path_on_all_conns(&mut self, open_addr: &transports::PathBinding) {
        for (conn_id, conn_state) in self.connections.iter() {
            if (conn_state.member_paths.is_some() && open_addr.is_ip())
                || conn_state.requires_path_ack(open_addr)
            {
                continue;
            }
            let Some(conn) = conn_state.handle.upgrade() else {
                continue;
            };
            let _ = self
                .state
                .open_path_on_conn(*conn_id, conn_state, &conn, open_addr, false);
        }
    }

    /// Handles regularly checking if any paths need hole punching currently
    ///
    /// Currently we need to have 1 IP path, with a good enough latency.
    fn check_connections(&mut self) {
        let mut is_goodenough = true;
        for conn_state in self.connections.values().filter(|state| state.member_paths.is_none()) {
            let mut is_conn_goodenough = false;
            if let Some(conn) = conn_state.handle.upgrade() {
                let min_ip_rtt = conn_state
                    .paths
                    .iter()
                    .filter_map(|(path_id, addr)| {
                        if addr.is_ip() {
                            conn.path_stats(*path_id).map(|stats| stats.rtt)
                        } else {
                            None
                        }
                    })
                    .min();

                if let Some(min_ip_rtt) = min_ip_rtt {
                    let is_latency_goodenough = min_ip_rtt <= GOOD_ENOUGH_LATENCY;
                    is_conn_goodenough = is_latency_goodenough;
                } else {
                    // No IP transport found
                    is_conn_goodenough = false;
                }
            }
            is_goodenough &= is_conn_goodenough;
        }

        if !is_goodenough {
            debug!("connections are not good enough, triggering holepunching");
            self.trigger_holepunching();
        }
    }
}

impl State {
    fn may_bootstrap_on(&self, path: &transports::PathBinding) -> bool {
        !path.is_connection_bound_udp()
            && match &path.address {
                transports::FourTuple::Custom { remote, .. } => {
                    self.mapped_addrs.permits_custom_bootstrap(remote)
                }
                _ => true,
            }
    }

    /// Handles [`RemoteStateMessage::SendDatagram`].
    async fn handle_msg_send_datagram(
        &mut self,
        mut sender: Box<TransportsSender>,
        transmit: OwnedTransmit,
    ) {
        // Sending datagrams might fail, e.g. because we don't have the right transports set
        // up to handle sending this owned transmit to.
        // After all, we try every single path that we know (relay URL, IP address), even
        // though we might not have a relay transport or ip-capable transport set up.
        // So these errors must not be fatal for this actor (or even this operation).

        // Endpoint-mapped bootstrap packets have no authenticated connection
        // identity yet. A physical traversal winner belongs to an existing
        // connection and cannot be reused to bootstrap another one.
        if let Some(addr) = self
            .selected_path
            .as_ref()
            .filter(|path| self.may_bootstrap_on(path))
        {
            trace!(?addr, "sending datagram to selected path");

            // TODO(Frando): We might want to include a local IP here in the future, if we confidently
            // know that it is the correct one.
            // See https://github.com/n0-computer/iroh/issues/4280.
            let four_tuple = transports::PathBinding::from_remote(addr.remote());
            if let Err(err) = send_datagram(&mut sender, four_tuple, transmit).await {
                debug!(?addr, "failed to send datagram on selected_path: {err:#}");
            }
        } else {
            trace!(
                paths = ?self.paths.addrs().collect::<Vec<_>>(),
                "sending datagram to all known paths",
            );
            if self.paths.is_empty() {
                warn!("Cannot send datagrams: No paths to remote endpoint known");
            }

            for addr in self.paths.addrs() {
                if let transports::Addr::Custom(custom) = addr
                    && !self.mapped_addrs.permits_custom_bootstrap(custom)
                {
                    continue;
                }
                // We never want to send to our local addresses.
                // The local address set is updated in the main loop so we can use `peek` here.
                if let transports::Addr::Ip(sockaddr) = addr
                    && self
                        .local_direct_addrs
                        .peek()
                        .iter()
                        .any(|a| a.addr == *sockaddr)
                {
                    trace!(%sockaddr, "not sending datagram to our own address");

                // TODO(Frando): We might want to include a local IP here in the future, if we confidently
                // know that it is the correct one.
                // See https://github.com/n0-computer/iroh/issues/4280.
                } else if let Err(err) = send_datagram(
                    &mut sender,
                    transports::PathBinding::from_remote(addr.clone()),
                    transmit.clone(),
                )
                .await
                {
                    debug!(?addr, "failed to send datagram: {err:#}");
                }
            }
            // This message is received *before* a connection is added.  So we do
            // not yet have a connection to holepunch.  Instead we trigger
            // holepunching when AddConnection is received.
        }
    }

    /// Handles [`RemoteStateMessage::ResolveRemote`].
    fn handle_msg_resolve_remote(
        &mut self,
        addrs: BTreeSet<TransportAddr>,
        tx: oneshot::Sender<Result<(), AddressLookupFailed>>,
    ) {
        let addrs = to_transports_addr(self.endpoint_id, addrs);
        self.paths.insert_multiple(addrs, Source::App);
        self.paths.resolve_remote(tx);
        // Start Address Lookup if we have no selected path.
        self.trigger_address_lookup();
    }

    /// Triggers Address Lookup for the remote endpoint, if needed.
    ///
    /// Does not start Address Lookup if we have a selected path or if Address Lookup is
    /// currently running.
    fn trigger_address_lookup(&mut self) {
        let reusable_selected = self
            .selected_path
            .as_ref()
            .is_some_and(|path| self.may_bootstrap_on(path));
        if reusable_selected || self.address_lookup_stream.is_some() {
            return;
        }
        let stream = self.address_lookup.resolve(self.endpoint_id);
        let stream = stream.filter_map(|item| match item {
            // We don't care about errors from individual services, we just continue.
            // Individual errors are buffered into the final error by `AddressLookupServices::resolve`,
            // and if the lookup fails we return them upstream with the final `AddressLookupFailed` error.
            Ok(Err(_err)) => None,
            Ok(Ok(item)) => Some(Ok(item)),
            Err(err) => Some(Err(err)),
        });
        self.address_lookup_stream = Some(Box::pin(stream));
    }

    /// Handles an address lookup result.
    ///
    /// All address lookup results end up being sent here. It takes care of updating the
    /// [`RemotePathState`] with the results.
    fn handle_address_lookup_item(
        &mut self,
        item: Option<Result<AddressLookupItem, AddressLookupFailed>>,
    ) {
        match item {
            None => {
                self.paths.address_lookup_finished(Ok(()));
                self.address_lookup_stream = None;
            }
            Some(Err(err)) => {
                if let AddressLookupFailed::NoServiceConfigured { .. } = err {
                    trace!("Address Lookup not configured");
                } else {
                    debug!("Address Lookup failed: {err:#}");
                }
                self.paths.address_lookup_finished(Err(err));
                self.address_lookup_stream = None;
            }
            Some(Ok(item)) => {
                if item.endpoint_id() != self.endpoint_id {
                    warn!(
                        ?item,
                        "Address Lookup emitted item for wrong remote endpoint"
                    );
                } else {
                    let source = Source::AddressLookup {
                        name: item.provenance().to_string(),
                    };
                    let addrs =
                        to_transports_addr(self.endpoint_id, item.into_endpoint_addr().addrs);
                    self.paths.insert_multiple(addrs, source);
                }
            }
        }
    }

    /// Unconditionally perform holepunching.
    #[instrument(skip_all)]
    fn do_holepunching(&mut self, conn: noq::Connection) {
        self.metrics.holepunch_attempts.inc();
        let local_candidates = self.local_candidates();
        match conn.initiate_nat_traversal_round() {
            Ok(remote_candidates) => {
                let remote_candidates = remote_candidates
                    .iter()
                    .map(|addr| SocketAddr::new(addr.ip().to_canonical(), addr.port()))
                    .collect();
                event!(
                    target: "iroh::_events::qnt::init",
                    Level::DEBUG,
                    remote = %self.endpoint_id.fmt_short(),
                    ?local_candidates,
                    ?remote_candidates,
                );
                self.last_holepunch = Some(HolepunchAttempt {
                    when: Instant::now(),
                    local_candidates,
                    remote_candidates,
                });
            }
            Err(err) => {
                debug!("failed to initiate NAT traversal: {err:#}");
                use noq_proto::n0_nat_traversal::Error;
                match err {
                    Error::Closed
                    | Error::TooManyAddresses
                    | Error::WrongConnectionSide
                    | Error::ExtensionNotNegotiated => {
                        // Fatal, no need to retry for now
                    }
                    Error::Multipath(_) | Error::NotEnoughAddresses => {
                        // Retry in a bit
                        let now = Instant::now();
                        let next_hp = now + Duration::from_millis(100);
                        trace!(scheduled_in = ?(next_hp - now), "holepunching retry");
                        self.scheduled_holepunch = Some(next_hp);
                    }
                }
            }
        }
    }

    /// Register a path with our state and configure path-specific settings.
    ///
    /// This inserts the path in the [`ConnectionState`] and [`Self::paths`].
    ///
    /// It configures the path with the correct path status (see [`Self::set_path_status`]),
    /// and applies path-type-specific settings:
    /// Relay paths get a longer idle timeout to accommodate transparent reconnection
    /// by the relay actor (see [`RELAY_PATH_MAX_IDLE_TIMEOUT`]).
    fn register_and_configure_path(
        &mut self,
        conn_id: ConnId,
        conn_state: &mut ConnectionState,
        path: &noq::Path,
    ) -> Option<transports::PathBinding> {
        let network_path = self
            .mapped_addrs
            .to_transport_tuple(&path.network_path().ok()?)?;
        event!(
            target: "iroh::_events::path::open",
            Level::DEBUG,
            remote = %self.endpoint_id.fmt_short(),
            %conn_id,
            path_id=%path.id(),
            %network_path,
        );
        conn_state.add_open_path(network_path.clone(), path.id(), &self.metrics);
        if network_path.is_relay()
            && let Err(e) = path.set_max_idle_timeout(Some(RELAY_PATH_MAX_IDLE_TIMEOUT))
        {
            debug!(?e, "failed to set relay path idle timeout");
        }

        if conn_state.member_paths.is_none() {
            self.set_path_status(conn_id, path, &network_path);
        }
        // A traversal pool socket is owned by this connection. Its remote
        // destination cannot become a peer-wide bootstrap candidate: a new
        // connection would be rejected by the remote socket's owner filter.
        if !network_path.is_connection_bound_udp() {
            self.paths
                .insert_open_path(network_path.remote(), Source::Connection);
        }
        Some(network_path)
    }

    fn set_path_status(
        &mut self,
        conn_id: ConnId,
        path: &noq::Path,
        network_path: &transports::PathBinding,
    ) {
        let status = self.path_status_for_addr(network_path);
        match path.set_status(status) {
            Err(error) => warn!(?error, ?network_path, ?status, "set_status failed"),
            Ok(prev_status) if prev_status != status => {
                event!(
                    target: "iroh::_events::path::set_status",
                    Level::DEBUG,
                    remote = %self.endpoint_id.fmt_short(),
                    %conn_id,
                    path_id=%path.id(),
                    %network_path,
                    ?status,
                    ?prev_status,
                );
            }
            Ok(_) => {}
        }
    }

    fn open_path_on_conn(
        &mut self,
        conn_id: ConnId,
        conn_state: &ConnectionState,
        conn: &noq::Connection,
        open_4tuple: &transports::PathBinding,
        explicit_request: bool,
    ) -> Result<(), &'static str> {
        self.open_path_inner(
            conn_id,
            conn_state,
            conn,
            open_4tuple,
            explicit_request,
            false,
        )
        .map(|_| ())
    }

    fn open_path_inner(
        &mut self,
        conn_id: ConnId,
        conn_state: &ConnectionState,
        conn: &noq::Connection,
        open_4tuple: &transports::PathBinding,
        explicit_request: bool,
        keep_spare: bool,
    ) -> Result<Option<crate::endpoint::UdpCandidatePath>, &'static str> {
        if conn.close_reason().is_some() {
            return Err("connection closed");
        }
        if !conn_state.owns_udp_binding(open_4tuple) {
            return Err("UDP binding does not belong to original connection");
        }
        if conn_state.purpose == ConnectionPurpose::Control
            && !open_4tuple.is_custom()
            && !explicit_request
        {
            return Ok(None);
        }
        if conn_state.purpose == ConnectionPurpose::Data && open_4tuple.is_custom() {
            return Err("control path forbidden on data connection");
        }
        if conn_state.forbids(open_4tuple) {
            return Err("signaling-only path forbidden on admitted connection");
        }
        // Only the client opens paths; the server receives them via
        // QUIC frames and reacts to PathOpened events.
        if conn.side().is_server() {
            return if explicit_request {
                Err("only the dialing side can open a path")
            } else {
                Ok(None)
            };
        }
        // Already open on this connection; nothing to do.
        if conn_state
            .paths
            .iter()
            .any(|(id, addr)| addr == open_4tuple && conn.path(*id).is_some())
        {
            return Ok(None);
        }

        let mapped_4tuple = self.mapped_addrs.to_mapped_tuple(open_4tuple);
        let path_status = if conn_state.requires_path_ack(open_4tuple)
            || (conn_state.member_paths.is_some() && explicit_request)
        {
            PathStatus::Backup
        } else {
            self.path_status_for_addr(open_4tuple)
        };

        let (fut, owned) = if keep_spare {
            conn.open_path_ensure_with_spare(mapped_4tuple, path_status)
        } else {
            let fut = match open_4tuple.local_socket {
                Some(socket) => conn
                    .open_path_ensure_on_socket(socket, mapped_4tuple, path_status)
                    .map_err(|_| "local UDP socket unavailable")?,
                None => conn.open_path_ensure(mapped_4tuple, path_status),
            };
            (fut, false)
        };
        match fut.path_id() {
            Some(path_id) => {
                if explicit_request {
                    eprintln!(
                        "[DEBUG-udp-handoff] actor-path-created stable_id={} path_id={} binding={open_4tuple:?} status={path_status:?}",
                        conn_id.0, path_id
                    );
                }
                #[cfg(not(wasm_browser))]
                if let Some(id) = open_4tuple
                    .local_socket
                    .map(transports::LocalSocketId::from_noq)
                    && let Some(socket) =
                        conn_state.udp_sockets.iter().find(|socket| socket.id == id)
                {
                    socket.mark_path_used();
                }
                trace!(%conn_id, %path_id, ?path_status, "opening new path");
                Ok(if owned {
                    conn.path(path_id)
                        .map(|path| crate::endpoint::DataCandidatePath(Some(path)))
                } else {
                    None
                })
            }
            None => {
                let ret = now_or_never(fut);
                match ret {
                    Some(Err(error @ PathError::RemoteCidsExhausted))
                    | Some(Err(error @ PathError::MaxPathIdReached)) => {
                        if explicit_request {
                            eprintln!(
                                "[DEBUG-udp-handoff] actor-path-resource-blocked stable_id={} binding={open_4tuple:?} reason={error:?}",
                                conn_id.0
                            );
                            // A peer-wide retry queue has no connection/attempt binding.
                            // The caller must retry this explicit request instead.
                            return Err(PATH_IDENTIFIERS_UNAVAILABLE);
                        }
                        self.scheduled_open_path =
                            Some(Instant::now() + Duration::from_millis(333));
                        self.pending_open_paths.push_back(open_4tuple.clone());
                        trace!(?open_4tuple, ?ret, "scheduling open_path");
                        Ok(None)
                    }
                    Some(Ok(_)) => Ok(None),
                    _ => {
                        warn!(?ret, "Opening path failed");
                        Err("opening path failed")
                    }
                }
            }
        }
    }

    /// Returns the [`PathStatus`] for `addr`.
    ///
    /// Returns [`PathStatus::Available`] if `addr` is the currently-selected path,
    /// or [`PathStatus::Backup`] otherwise.
    fn path_status_for_addr(&self, addr: &transports::PathBinding) -> PathStatus {
        if Some(addr) == self.selected_path.as_ref() {
            PathStatus::Available
        } else {
            PathStatus::Backup
        }
    }

    /// Returns the current set of local direct addresses.
    fn local_candidates(&mut self) -> BTreeSet<SocketAddr> {
        self.local_direct_addrs
            .get()
            .iter()
            .map(|d| d.addr)
            .collect()
    }
}

/// Give both families space in QNT's negotiated window, rotating excess candidates on retry.
fn member_candidate_window(
    candidates: &BTreeSet<SocketAddr>,
    limit: usize,
    cursor: usize,
) -> BTreeSet<SocketAddr> {
    if candidates.len() <= limit {
        return candidates.clone();
    }
    let v4: Vec<_> = candidates
        .iter()
        .copied()
        .filter(SocketAddr::is_ipv4)
        .collect();
    let v6: Vec<_> = candidates
        .iter()
        .copied()
        .filter(SocketAddr::is_ipv6)
        .collect();
    let mut ordered = Vec::with_capacity(candidates.len());
    for index in 0..v4.len().max(v6.len()) {
        for family in [&v4, &v6] {
            if index < family.len() {
                ordered.push(family[(cursor % family.len() + index) % family.len()]);
            }
        }
    }
    ordered.into_iter().take(limit).collect()
}

/// Updates QNT's candidate addresses to be the current set of direct addresses.
///
/// `direct_addrs` must be a set of addresses extracted from the endpoint's current
/// [`DirectAddr`]s.
fn update_qnt_candidates(conn: &noq::Connection, direct_addrs: &BTreeSet<SocketAddr>) {
    let noq_candidates = match conn.get_local_nat_traversal_addresses() {
        Ok(addrs) => BTreeSet::from_iter(addrs),
        Err(err) => {
            warn!("failed to get local nat candidates: {err:#}");
            return;
        }
    };
    for addr in noq_candidates.difference(direct_addrs) {
        if let Err(err) = conn.remove_nat_traversal_address(*addr) {
            warn!("failed removing local addr: {err:#}");
        }
    }
    for addr in direct_addrs.difference(&noq_candidates) {
        let result = conn.add_nat_traversal_address(*addr);
        if addr.is_ipv6() && std::env::var_os("MIOPUNCH_TRACE_PEER_IPV6").is_some() {
            eprintln!(
                "PEER_IPV6 qnt-announce stable_id={} address={addr} result={result:?}",
                conn.stable_id(),
            );
        }
        if let Err(err) = result {
            warn!("failed adding local addr: {err:#}",);
        }
    }
    trace!(?direct_addrs, "updated local QNT addresses");
}

fn send_datagram<'a>(
    sender: &'a mut TransportsSender,
    addr: transports::PathBinding,
    owned_transmit: OwnedTransmit,
) -> impl Future<Output = n0_error::Result<()>> + 'a {
    std::future::poll_fn(move |cx| {
        let transmit = transports::Transmit {
            ecn: owned_transmit.ecn,
            contents: owned_transmit.contents.as_ref(),
            segment_size: owned_transmit.segment_size,
        };

        Pin::new(&mut *sender)
            .poll_send(cx, &addr, &transmit)
            .map(|res| res.with_context(|_| format!("failed to send datagram to {:?}", addr)))
    })
}

/// Messages to send to the [`RemoteStateActor`].
#[derive(derive_more::Debug)]
pub(crate) enum RemoteStateMessage {
    /// Sends a datagram to all known paths.
    ///
    /// Used to send QUIC Initial packets.  If there is no working direct path this will
    /// trigger holepunching.
    ///
    /// This is not acceptable to use on the normal send path, as it is an async send
    /// operation with a bunch more copying.  So it should only be used for sending QUIC
    /// Initial packets.
    #[debug("SendDatagram(..)")]
    SendDatagram(Box<TransportsSender>, OwnedTransmit),
    /// Adds an active connection to this remote endpoint.
    ///
    /// The actor will downgrade the connection to a [`noq::WeakConnectionHandle`] as soon
    /// as it processes the message. It will keep hold of the weak handle until it closes,
    /// but only update to a strong [`noq::Connection`] for brief moments.
    ///
    /// The actor will actively manage paths on the connection and start holepunching as needed.
    #[debug("AddConnection({})", _0.stable_id())]
    AddConnection(noq::Connection, bool, oneshot::Sender<PathStateReceiver>),
    /// Enable candidate exchange after a full admission decision.
    AuthorizeCandidates {
        conn_id: usize,
        allowed_data_custom_ids: Vec<u64>,
        member: Option<MemberPathPolicy>,
        tx: oneshot::Sender<Result<(), &'static str>>,
    },
    /// Start the same member UDP owner before business handoff.
    #[cfg(not(wasm_browser))]
    PrepareMemberUdp {
        conn_id: usize,
        policy: MemberPathPolicy,
        tx: oneshot::Sender<Result<(), &'static str>>,
    },
    /// A permitted UDP, relay or custom data carrier on an admitted connection.
    OpenAdmittedDataPath {
        conn_id: usize,
        remote: TransportAddr,
        own_candidate: bool,
        tx: oneshot::Sender<Result<Option<crate::endpoint::DataCandidatePath>, &'static str>>,
    },
    /// Retain an already established, freshly qualified connection-owned carrier.
    RetainAdmittedDataPath {
        conn_id: usize,
        remote: iroh_base::CustomAddr,
        tx: oneshot::Sender<Result<(), &'static str>>,
    },
    /// Seal a fresh, owned data path while withholding the normal handle.
    RetainControlDataPath {
        conn_id: usize,
        remote: iroh_base::CustomAddr,
        tx: oneshot::Sender<Result<(), &'static str>>,
    },
    /// Read the current exact connection-owned seal without changing admission.
    HasRetainedControlDataPath {
        conn_id: usize,
        remote: iroh_base::CustomAddr,
        tx: oneshot::Sender<bool>,
    },
    #[cfg(feature = "miopunch-test-hooks")]
    TestLimitTransportPaths {
        conn_id: usize,
        allowed: Vec<TransportAddr>,
        tx: oneshot::Sender<Result<(), &'static str>>,
    },
    /// Opens a known UDP path on an already authenticated connection.
    OpenDataPath {
        conn_id: usize,
        remote: SocketAddr,
        #[cfg(not(wasm_browser))]
        local: Option<transports::LocalSocketBinding>,
        keep_spare: bool,
        tx: oneshot::Sender<Result<Option<crate::endpoint::UdpCandidatePath>, &'static str>>,
    },
    #[cfg(not(wasm_browser))]
    RetainUdpSocket {
        conn_id: usize,
        socket: Arc<transports::PoolSocket>,
        live: Arc<std::sync::Mutex<bool>>,
        tx: oneshot::Sender<Result<(), &'static str>>,
    },
    /// Permit a same-socket UDP NAT probe while this connection is restricted.
    AuthorizeUdpProbe {
        conn_id: usize,
        tx: oneshot::Sender<Result<(), &'static str>>,
    },
    #[cfg(feature = "miopunch-test-hooks")]
    TestForcePathIdRetry {
        conn_id: usize,
        tx: oneshot::Sender<Result<(), &'static str>>,
    },
    /// Attempts to open an additional control route on a restricted connection.
    OpenControlPath {
        conn_id: usize,
        remote: TransportAddr,
        tx: oneshot::Sender<Result<(), &'static str>>,
    },
    RetireControlPath {
        conn_id: usize,
        remote: iroh_base::CustomAddr,
        tx: oneshot::Sender<Result<(), &'static str>>,
    },
    #[cfg(feature = "miopunch-test-hooks")]
    TestReopenControlPath {
        conn_id: usize,
        remote: iroh_base::CustomAddr,
        tx: oneshot::Sender<Result<(), &'static str>>,
    },
    /// Removes control-only paths after a data path has been established.
    CommitDataPath {
        conn_id: usize,
        tx: oneshot::Sender<Result<(), &'static str>>,
    },
    /// Asks if there is any possible path that could be used.
    ///
    /// This adds the provided transport addresses to the list of potential paths for this
    /// remote and starts Address Lookup if needed.
    ///
    /// Sends back `Ok` immediately if the provided address list is non-empy or we have are
    /// other known paths.  Otherwise sends back `Ok` once Address Lookup produces a result,
    /// or the Address Lookup error if Address Lookup fails or produces no results,
    #[debug("ResolveRemote(..)")]
    ResolveRemote(
        BTreeSet<TransportAddr>,
        oneshot::Sender<Result<(), AddressLookupFailed>>,
    ),
    /// Returns information about the remote.
    ///
    /// This currently only includes a list of all known transport addresses for the remote.
    RemoteInfo(oneshot::Sender<RemoteInfo>),
    /// The network status has changed in some way
    NetworkChange { is_major: bool },
}

/// Information about a holepunch attempt.
///
/// Addresses are always stored in canonical form.
#[derive(Debug)]
struct HolepunchAttempt {
    when: Instant,
    /// The set of local addresses which could take part in holepunching.
    ///
    /// This does not mean every address here participated in the holepunching.  E.g. we
    /// could have tried only a sub-set of the addresses because a previous attempt already
    /// covered part of the range.
    ///
    /// We do not store this as a [`DirectAddr`] because this is checked for equality and we
    /// do not want to compare the sources of these addresses.
    local_candidates: BTreeSet<SocketAddr>,
    /// The set of remote addresses which could take part in holepunching.
    ///
    /// Like [`Self::local_candidates`] we may not have used them.
    remote_candidates: BTreeSet<SocketAddr>,
}

/// Newtype to track Connections.
///
/// The wrapped value is the [`noq::Connection::stable_id`] value, and is thus only valid
/// for active connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, derive_more::Display)]
#[display("{_0}")]
struct ConnId(usize);

/// A connection managed by the [`RemoteStateActor`].
///
/// - A handle to the connection.
/// - The paths we know about.
/// - Some stuff to make observers happy.
#[derive(Debug)]
struct ConnectionState {
    connection_id: usize,
    mapped_addrs: super::MappedAddrs,
    /// Weak handle to the connection.
    handle: WeakConnectionHandle,
    /// Writer-side handle for the connection's path observation state.
    ///
    /// The matching [`PathStateReceiver`] is held by the [`Connection`].
    ///
    /// [`Connection`]: crate::endpoint::Connection
    path_state: PathStateSender,
    /// The open paths that exist on this connection.
    ///
    /// This might be lagging from the connection state inside noq itself as this can only
    /// be updated once we received the event from noq.
    ///
    /// IP paths *should* have the local IP address filled in by the time we receive the
    /// event. The noq established event is only emitted once at least one datagram is
    /// received from the peer on the path.
    paths: FxHashMap<PathId, transports::PathBinding>,
    suspect_paths: FxHashMap<PathId, SuspectPath>,
    pending_stream_handoff: Option<PathId>,
    /// Whether this connection has ever had a direct path.
    ///
    /// Used for recording metrics.
    has_been_direct: bool,
    purpose: ConnectionPurpose,
    allowed_data_custom_ids: BTreeSet<u64>,
    custom_data_policy: bool,
    member_paths: Option<MemberPaths>,
    /// Exact grant sealed by a restricted hook; never a type-wide permission.
    retained_control_custom: Option<iroh_base::CustomAddr>,
    retired_custom: HashSet<iroh_base::CustomAddr>,
    #[cfg(not(wasm_browser))]
    udp_sockets: Vec<ConnectionSocket>,
}

#[derive(Debug)]
struct SuspectPath {
    /// The episode ends once a packet sent later on this path is acknowledged.
    detected_at: Instant,
    retained_for_recovery: bool,
}

impl SuspectPath {
    fn new(detected_at: Instant) -> Self {
        Self {
            detected_at,
            retained_for_recovery: false,
        }
    }
}

#[cfg(not(wasm_browser))]
#[derive(Debug)]
struct ConnectionSocket(Arc<transports::PoolSocket>);
#[cfg(not(wasm_browser))]
impl std::ops::Deref for ConnectionSocket {
    type Target = transports::PoolSocket;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
#[cfg(not(wasm_browser))]
impl Drop for ConnectionSocket {
    fn drop(&mut self) {
        self.0.close();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConnectionPurpose {
    Ordinary,
    /// Only a control ALPN is available; no automatic candidates or data.
    Control,
    /// Custom paths are forbidden by the per-connection packet gate.
    Data,
}

impl ConnectionState {
    fn is_native_member_path(&self, binding: &transports::PathBinding) -> bool {
        self.member_paths.as_ref().is_some_and(|member| {
            binding.is_ip()
                || binding.is_relay()
                || matches!(binding.remote(), transports::Addr::Custom(address)
                    if member.policy.acked_custom_transports[0] == Some(address.id()))
        })
    }

    fn binding_matches(&self, binding: &transports::PathBinding, live: noq::FourTuple) -> bool {
        self.mapped_addrs.to_transport_tuple(&live).as_ref() == Some(binding)
    }

    fn has_recent_return(
        &self,
        binding: &transports::PathBinding,
        path: &noq::Path,
        now: Instant,
        window: Duration,
    ) -> bool {
        path.return_evidence().is_some_and(|(live, received)| {
            self.binding_matches(binding, live)
                && now.saturating_duration_since(received.into()) <= window
        })
    }

    fn confirm_member_backups(
        &self,
        conn: &noq::Connection,
        selected: Option<PathId>,
        now: Instant,
        interval: Duration,
    ) {
        let Some(selected) = selected else { return };
        if self.member_paths.is_none()
            || self.purpose != ConnectionPurpose::Ordinary
            || (!self.suspect_paths.contains_key(&selected)
                && conn
                    .path_stats(selected)
                    .is_none_or(|stats| stats.pto_count == 0))
        {
            return;
        }
        for (id, binding) in &self.paths {
            if *id != selected
                && self.is_native_member_path(binding)
                && self.path_is_qualified(*id, conn)
                && let Some(path) = conn.path(*id)
                && let Ok(live) = path.network_path()
                && self.binding_matches(binding, live)
            {
                let budget = 2 * interval.max(path.stats().probe_timeout);
                let tcp_suspect = binding.is_custom() && self.suspect_paths.contains_key(id);
                if tcp_suspect || !self.has_recent_return(binding, &path, now, budget) {
                    let _ = path.start_confirmation(live, budget);
                }
            }
        }
    }

    fn update_path_health(&mut self, conn: &noq::Connection, now: Instant, interval: Duration) {
        // Continuous sends can keep moving the actual PTO's deadline. Age the
        // absence of ACK progress independently while packets are in flight.
        let paths: Vec<_> = self
            .paths
            .iter()
            .filter_map(|(id, binding)| {
                if self.may_select(binding)
                    && let Some(path) = conn.path(*id)
                {
                    let stats = path.stats();
                    let member_path = self.is_native_member_path(binding);
                    let member_direct = member_path && !binding.is_relay();
                    let progress = member_path
                        && path.confirmation().is_some_and(|(live, received)| {
                            self.binding_matches(binding, live)
                                && now.saturating_duration_since(received.into())
                                    < interval.max(2 * stats.probe_timeout)
                                && stats
                                    .ack_pending_since
                                    .is_none_or(|pending| pending <= received)
                        });
                    let stall_interval = if member_direct {
                        interval.max(MEMBER_DIRECT_ACK_STALL_MIN)
                    } else {
                        interval
                    };
                    let stalled = stats.ack_pending_since.is_some_and(|since| {
                        now.saturating_duration_since(since.into())
                            >= stall_interval.max(2 * stats.probe_timeout)
                    });
                    Some((*id, stats, progress, member_direct, stalled))
                } else {
                    None
                }
            })
            .collect();
        for (id, stats, progress, member_direct, _) in &paths {
            if *progress {
                self.suspect_paths.remove(id);
                continue;
            }
            if self.suspect_paths.get(id).is_some_and(|suspect| {
                stats
                    .acknowledgment
                    .is_some_and(|ack| Instant::from(ack.sent_at) > suspect.detected_at)
            }) {
                self.suspect_paths.remove(id);
            }
            // On member direct paths, PTOs can prepare backups while the
            // current sender still has time to make progress.
            if !member_direct && stats.pto_count >= 2 {
                self.suspect_paths
                    .entry(*id)
                    .or_insert_with(|| SuspectPath::new(now));
            }
        }

        if self.member_paths.is_some()
            || paths.iter().any(|(id, stats, _, _, stalled)| {
                !self.suspect_paths.contains_key(id)
                    && !stalled
                    && has_fresh_ack(stats, now, interval)
            })
        {
            for (id, _, progress, _, stalled) in paths {
                if !progress && stalled {
                    self.suspect_paths
                        .entry(id)
                        .or_insert_with(|| SuspectPath::new(now));
                }
            }
        }
    }

    /// Initial preparation uses restricted control. Active maintenance may use
    /// the exact original after its admission data policy has been installed;
    /// an unrestricted ordinary connection has no such authority.
    fn permits_udp_traversal(&self) -> bool {
        (self.purpose == ConnectionPurpose::Control
            || (self.purpose == ConnectionPurpose::Ordinary && self.custom_data_policy))
            && self
                .handle
                .upgrade()
                .is_some_and(|connection| connection.close_reason().is_none())
    }

    fn retained_control_data_is_current(&self, address: &iroh_base::CustomAddr) -> bool {
        self.retained_control_custom.as_ref() == Some(address)
            && !self.retired_custom.contains(address)
            && self.mapped_addrs.qualifies_connection_owned_custom(address, self.connection_id)
            && self.handle.upgrade().is_some_and(|conn| {
                conn.close_reason().is_none()
                    && self.paths.iter().any(|(id, path)| {
                        matches!(&path.address, transports::FourTuple::Custom { remote, .. } if remote == address)
                            && conn.path(*id).is_some()
                    })
            })
    }

    /// Physical local sockets belong to one exact original Connection.
    /// Global peer preferences must never copy another connection's binding.
    fn owns_udp_binding(&self, path: &transports::PathBinding) -> bool {
        #[cfg(not(wasm_browser))]
        if let Some(id) = path.local_socket.map(transports::LocalSocketId::from_noq) {
            return self
                .udp_sockets
                .iter()
                .any(|socket| socket.id == id && socket.is_live() && socket.is_pinned());
        }
        true
    }
    fn path_is_qualified(&self, id: PathId, conn: &noq::Connection) -> bool {
        self.paths.get(&id).is_some_and(|path| {
            self.may_select(path)
                && conn.path(id).is_some_and(|live| {
                    !self.is_native_member_path(path)
                        || live
                            .network_path()
                            .is_ok_and(|live| self.binding_matches(path, live))
                })
                && (!self.requires_path_ack(path)
                    || conn
                        .path_stats(id)
                        .is_some_and(|stats| stats.acknowledgment.is_some()))
        })
    }

    fn requires_path_ack(&self, path: &transports::PathBinding) -> bool {
        self.member_paths.as_ref()
            .is_some_and(|member| member.policy.requires_path_ack(&path.remote()))
    }

    fn may_select(&self, path: &transports::PathBinding) -> bool {
        self.owns_udp_binding(path)
            && !self.forbids(path)
            && match &path.address {
                transports::FourTuple::Custom { remote, .. } => self
                    .mapped_addrs
                    .qualifies_custom_data(remote, self.connection_id),
                _ => true,
            }
            && (self.purpose != ConnectionPurpose::Data || path.is_ip())
    }

    fn forbids(&self, path: &transports::PathBinding) -> bool {
        self.member_paths
            .as_ref()
            .is_some_and(|member| !member.policy.permits(path.remote()))
            || matches!(&path.address, transports::FourTuple::Custom { remote, .. }
            if !self.mapped_addrs.permits_custom_connection(remote, self.connection_id)
                || self.retired_custom.contains(remote) || (self.custom_data_policy && !self.allowed_data_custom_ids.contains(&remote.id()) && self.retained_control_custom.as_ref() != Some(remote)))
    }

    /// Tracks an open path for the connection.
    fn add_open_path(
        &mut self,
        network_path: transports::PathBinding,
        path_id: PathId,
        metrics: &Arc<SocketMetrics>,
    ) {
        #[cfg(not(wasm_browser))]
        if let Some(id) = network_path
            .local_socket
            .map(transports::LocalSocketId::from_noq)
            && let Some(socket) = self.udp_sockets.iter().find(|socket| socket.id == id)
        {
            socket.mark_path_used();
        }
        match &network_path.address {
            transports::FourTuple::Ip { .. } => metrics.paths_direct.inc(),
            transports::FourTuple::Relay { .. } => metrics.paths_relay.inc(),
            transports::FourTuple::Custom { .. } => metrics.paths_custom.inc(),
        };
        if !self.has_been_direct && network_path.is_ip() {
            self.has_been_direct = true;
            metrics.num_conns_direct.inc();
        }

        self.paths.insert(path_id, network_path.clone());
        if let Some(conn) = self.handle.upgrade()
            && let Some(path) = conn.path(path_id)
        {
            let handle = path.weak_handle();
            self.path_state.record_opened(handle, network_path);
        }
    }

    /// Removes a path from this connection.
    fn remove_path(
        &mut self,
        path_id: &PathId,
        conn: &noq::Connection,
    ) -> Option<transports::PathBinding> {
        let addr = self.paths.remove(path_id)?;
        self.suspect_paths.remove(path_id);
        if let Some(member) = &mut self.member_paths {
            member.new_path_probes.remove(path_id);
        }
        self.path_state.record_abandoned(*path_id, conn);
        Some(addr)
    }
}

/// State of the endpoint relevant for path selection.
///
/// Constructed by the endpoint and passed to [`PathSelector::select`].  Borrows from
/// the endpoint's internal data.
#[derive(Debug)]
#[cfg_attr(not(feature = "unstable-custom-transports"), allow(unreachable_pub))]
pub struct PathSelectionContext<'a> {
    current: Option<&'a transports::PathBinding>,
    source: PathsSource<'a>,
    health_window: Option<(Instant, Duration)>,
}

/// Either a reference to live connection state, or a synthesized list of paths
/// (for unit-testing selectors).
#[derive(Debug)]
enum PathsSource<'a> {
    Live(&'a FxHashMap<ConnId, ConnectionState>),
    Connection(&'a ConnectionState),
    #[cfg(any(test, feature = "miopunch-test-hooks"))]
    Test {
        current: Option<&'a transports::FourTuple>,
        paths: Vec<PathSelectionData<'a>>,
    },
}

#[cfg_attr(not(feature = "unstable-custom-transports"), allow(unreachable_pub))]
impl<'a> PathSelectionContext<'a> {
    fn new(
        current: Option<&'a transports::PathBinding>,
        connections: &'a FxHashMap<ConnId, ConnectionState>,
    ) -> Self {
        Self {
            current,
            source: PathsSource::Live(connections),
            health_window: None,
        }
    }

    fn for_connection(
        current: Option<&'a transports::PathBinding>,
        state: &'a ConnectionState,
        health_window: Option<(Instant, Duration)>,
    ) -> Self {
        Self {
            current,
            source: PathsSource::Connection(state),
            health_window,
        }
    }

    /// Constructs a context with synthetic path data for testing.
    #[cfg(any(test, feature = "miopunch-test-hooks"))]
    pub(crate) fn for_test(
        current: Option<&'a transports::FourTuple>,
        paths: Vec<PathSelectionData<'a>>,
    ) -> Self {
        Self {
            current: None,
            source: PathsSource::Test { current, paths },
            health_window: None,
        }
    }

    /// Exercise the real scorer with deterministic observations, without
    /// constructing a connection or granting permission to a path.
    #[cfg(feature = "miopunch-test-hooks")]
    #[doc(hidden)]
    pub fn test_select_biased_rtt_with_tcp(
        current: Option<&'a transports::FourTuple>,
        paths: Vec<(&'a transports::FourTuple, Option<PathStats>)>,
        tier: impl Fn(&transports::FourTuple) -> Option<u8>,
        tcp_address: impl Fn(&transports::FourTuple) -> Option<SocketAddr>,
    ) -> Option<transports::FourTuple> {
        let paths = paths.into_iter().map(|(path, stats)| PathSelectionData::for_test(path, stats)).collect();
        let context = Self::for_test(current, paths);
        context.select_biased_rtt_with_tcp(tier, tcp_address)
            .selected().map(|path| path.address.clone())
    }

    /// The original connection being evaluated, for connection-scoped callbacks.
    /// Peer callbacks return `None`. The ID is valid only while the connection lives.
    pub fn connection_id(&self) -> Option<usize> {
        match &self.source {
            PathsSource::Connection(state) => Some(state.connection_id),
            _ => None,
        }
    }

    /// Whether this context is evaluating an admitted shared member connection.
    pub fn is_member_fusion(&self) -> bool {
        matches!(&self.source, PathsSource::Connection(state) if state.member_paths.is_some())
    }

    /// Select using iroh's biased RTT and hysteresis, with optional application
    /// transport tiers. A lower tier number takes precedence.
    pub fn select_biased_rtt(
        &self,
        tier: impl Fn(&transports::FourTuple) -> Option<u8>,
    ) -> PathSelection {
        super::super::biased_rtt_path_selector::BiasedRttPathSelector::default()
            .select_with_tier(self, tier)
    }

    /// Preserve network-family bias for an application's TCP carrier and use
    /// TCP to break exact ties when no current path has a hysteresis claim.
    pub fn select_biased_rtt_with_tcp(
        &self,
        tier: impl Fn(&transports::FourTuple) -> Option<u8>,
        tcp_address: impl Fn(&transports::FourTuple) -> Option<SocketAddr>,
    ) -> PathSelection {
        super::super::biased_rtt_path_selector::BiasedRttPathSelector::default().select_with_tcp(
            self,
            tier,
            tcp_address,
        )
    }

    /// The path currently considered the preferred path to the remote endpoint, if any.
    pub fn current(&self) -> Option<&transports::FourTuple> {
        #[cfg(any(test, feature = "miopunch-test-hooks"))]
        if let PathsSource::Test { current, .. } = &self.source {
            return *current;
        }
        self.current.map(|path| &path.address)
    }

    /// Whether this candidate is the current path, including its local socket.
    pub fn is_current(&self, candidate: &PathSelectionData<'_>) -> bool {
        if let PathsSource::Connection(state) = &self.source
            && state.member_paths.is_some()
            && let StatsSource::Live { path_id, .. } = &candidate.source
        {
            return self.current.is_some() && state.path_state.selected_id() == Some(*path_id);
        }
        self.current.is_some_and(|path| {
            path.address == *candidate.network_path && path.local_socket == candidate.local_socket
        }) || {
            #[cfg(any(test, feature = "miopunch-test-hooks"))]
            {
                matches!(&self.source, PathsSource::Test { current: Some(current), .. }
                    if *current == candidate.network_path)
            }
            #[cfg(not(any(test, feature = "miopunch-test-hooks")))]
            {
                false
            }
        }
    }

    /// Iterator over candidate paths.
    ///
    /// Connection-local evaluation includes only that connection's qualified
    /// paths. Peer-hint evaluation can include the same address on multiple
    /// ordinary connections; selectors can aggregate these as appropriate.
    pub fn paths(&self) -> Box<dyn Iterator<Item = PathSelectionData<'a>> + '_> {
        match &self.source {
            PathsSource::Live(connections) => Box::new(
                connections
                    .values()
                    .filter(|state| state.purpose != ConnectionPurpose::Control)
                    .filter_map(|state| {
                        state
                            .handle
                            .upgrade()
                            .filter(|conn| conn.close_reason().is_none())
                            .map(|conn| (state, conn))
                    })
                    .flat_map(|(state, conn)| {
                        state
                            .paths
                            .iter()
                            .filter({
                                let conn = conn.clone();
                                move |(id, addr)| {
                                    state.may_select(addr)
                                        && (!state.requires_path_ack(addr)
                                            || conn.path_stats(**id).is_some_and(|stats| {
                                                stats.acknowledgment.is_some()
                                            }))
                                }
                            })
                            .map(move |(path_id, addr)| {
                                PathSelectionData::live(addr, *path_id, conn.clone(), false)
                            })
                    }),
            ),
            PathsSource::Connection(state) => Box::new(
                state
                    .handle
                    .upgrade()
                    .filter(|conn| conn.close_reason().is_none())
                    .into_iter()
                    .flat_map(move |conn| {
                        state.paths.iter().filter_map(move |(id, addr)| {
                            let suspect = state.suspect_paths.contains_key(id);
                            if let Some(member) = &state.member_paths {
                                let current = state.path_state.selected_id() == Some(*id);
                                let current_suspect =
                                    state.path_state.selected_id().is_none_or(|current| {
                                        state.suspect_paths.contains_key(&current)
                                    });
                                // A fresh same-binding response proves a backup can carry
                                // this direction even when its ordinary ACKs use the failed path.
                                let needs_response = !current
                                    && current_suspect
                                    && state.path_state.selected_id().is_some()
                                    && state.purpose == ConnectionPurpose::Ordinary
                                    && state.is_native_member_path(addr);
                                // A response remains evidence after a healthy relay
                                // takes over while the backup confirmation is in flight.
                                let may_use_tcp_confirmation = addr.is_custom()
                                    && state.is_native_member_path(addr);
                                let confirmed = (needs_response || may_use_tcp_confirmation)
                                    && (!may_use_tcp_confirmation || !suspect)
                                    && self.health_window.is_some_and(|(now, interval)| {
                                        conn.path(*id).is_some_and(|path| {
                                            let stats = path.stats();
                                            let window =
                                                2 * interval.max(stats.probe_timeout);
                                            let proof = if needs_response {
                                                path.return_evidence()
                                            } else {
                                                path.confirmation()
                                            };
                                            proof.is_some_and(|(live, received)| {
                                                state.binding_matches(addr, live)
                                                    && now
                                                        .saturating_duration_since(received.into())
                                                        <= window
                                                    && (!may_use_tcp_confirmation
                                                        || member.network_changed_at.is_none_or(
                                                            |changed| {
                                                                Instant::from(received) > changed
                                                            },
                                                        ))
                                            })
                                        })
                                    });
                                if (needs_response && !confirmed)
                                    || (suspect && !confirmed)
                                    || (addr.is_ip()
                                        && member.network_changed_at.is_some_and(|changed| {
                                            !conn
                                                .path_stats(*id)
                                                .and_then(|stats| stats.acknowledgment)
                                                .is_some_and(|ack| {
                                                    Instant::from(ack.sent_at) > changed
                                                })
                                        }))
                                    || (!confirmed
                                        && (state.requires_path_ack(addr)
                                            || (!current && (current_suspect || addr.is_ip())))
                                        && !conn
                                            .path_stats(*id)
                                            .is_some_and(|stats| self.has_fresh_ack(&stats)))
                                {
                                    return None;
                                }
                            }
                            (state.may_select(addr) && conn.path(*id).is_some())
                                .then(|| PathSelectionData::live(addr, *id, conn.clone(), suspect))
                        })
                    }),
            ),
            #[cfg(any(test, feature = "miopunch-test-hooks"))]
            PathsSource::Test { paths, .. } => Box::new(paths.iter().cloned()),
        }
    }

    /// Whether this connection recently received an ACK for this binding.
    pub fn has_fresh_ack(&self, stats: &PathStats) -> bool {
        self.health_window
            .is_some_and(|(now, interval)| has_fresh_ack(stats, now, interval))
    }
}

fn has_fresh_ack(stats: &PathStats, now: Instant, interval: Duration) -> bool {
    stats.acknowledgment.is_some_and(|ack| {
        now.saturating_duration_since(ack.received_at.into())
            <= 2 * interval.max(stats.probe_timeout)
    })
}

/// Data the selector sees about one candidate path.
//
// In production this borrows from a live connection and looks up stats from noq on
// demand. Test builds can also wrap synthesized stats so selectors
// can be unit-tested without standing up real connections.
#[cfg_attr(not(feature = "unstable-custom-transports"), allow(unreachable_pub))]
#[derive(derive_more::Debug, Clone)]
pub struct PathSelectionData<'a> {
    network_path: &'a transports::FourTuple,
    local_socket: Option<noq::LocalSocketId>,
    #[debug(skip)]
    source: StatsSource,
    suspect: bool,
}

#[derive(Clone)]
enum StatsSource {
    Live {
        path_id: PathId,
        conn: noq::Connection,
    },
    /// Boxed so `PathStats` doesn't inflate the enum in production, where
    /// only the `Live` variant is ever constructed.
    #[cfg(any(test, feature = "miopunch-test-hooks"))]
    Test(Option<Box<PathStats>>),
}

#[cfg_attr(not(feature = "unstable-custom-transports"), allow(unreachable_pub))]
impl<'a> PathSelectionData<'a> {
    pub(crate) fn original_id(&self) -> Option<(usize, PathId)> {
        match &self.source {
            StatsSource::Live { path_id, conn } => Some((conn.stable_id(), *path_id)),
            _ => None,
        }
    }

    fn live(
        network_path: &'a transports::PathBinding,
        path_id: PathId,
        conn: noq::Connection,
        suspect: bool,
    ) -> Self {
        Self {
            network_path: &network_path.address,
            local_socket: network_path.local_socket,
            source: StatsSource::Live { path_id, conn },
            suspect,
        }
    }

    /// Constructs a [`PathSelectionData`] with synthetic stats for testing.
    ///
    /// `PathStats` is `#[non_exhaustive]` so callers build it via
    /// `let mut s = PathStats::default(); s.rtt = ...;`.
    #[cfg(any(test, feature = "miopunch-test-hooks"))]
    pub(crate) fn for_test(
        network_path: &'a transports::FourTuple,
        stats: Option<PathStats>,
    ) -> Self {
        Self {
            network_path,
            local_socket: None,
            source: StatsSource::Test(stats.map(Box::new)),
            suspect: false,
        }
    }

    /// The network path of the candidate path.
    pub fn network_path(&self) -> &transports::FourTuple {
        self.network_path
    }

    /// Physical local socket of this candidate, separate from its real address.
    /// `None` denotes ordinary endpoint I/O, not permission to use another socket.
    #[cfg(not(wasm_browser))]
    pub fn local_socket_id(&self) -> Option<transports::LocalSocketId> {
        self.local_socket.map(transports::LocalSocketId::from_noq)
    }

    /// Returns path statistics if available.
    pub fn stats(&self) -> Option<PathStats> {
        match &self.source {
            StatsSource::Live { path_id, conn } => conn.path_stats(*path_id),
            #[cfg(any(test, feature = "miopunch-test-hooks"))]
            StatsSource::Test(stats) => stats.as_deref().copied(),
        }
    }

    /// The binding has unacknowledged traffic and has not proved recovery.
    pub fn is_suspect(&self) -> bool {
        self.suspect
    }
}

/// Trait to configure path selection.
///
/// Most users do not need to provide their own selector.
#[cfg_attr(not(feature = "unstable-custom-transports"), allow(unreachable_pub))]
pub trait PathSelector: Send + Sync + std::fmt::Debug + 'static {
    /// Optional bounded reevaluation of live path quality. The actor clamps
    /// this to 250 milliseconds through 30 seconds and skips catch-up bursts.
    /// None preserves selection only on path lifecycle events.
    fn reevaluation_interval(&self) -> Option<Duration> {
        None
    }

    /// Pick the selected path to carry application data among the currently
    /// open network paths to the remote endpoint.
    ///
    /// Build the result by starting from [`PathSelection::none`] and calling
    /// [`PathSelection::set`] for the path the selector wants active.
    ///
    /// Returning an empty [`PathSelection`] keeps the current selection unchanged.
    fn select(&self, ctx: &PathSelectionContext<'_>) -> PathSelection;
}

/// Explicit policy for one original connection's qualified paths.
///
/// Register with [`crate::endpoint::Builder::connection_path_selector`]. Unlike
/// [`PathSelector`], this callback never receives the peer aggregate. Returning
/// no selection preserves the connection's current qualified path. Retained
/// selections are checked again against current ownership and live paths.
#[cfg_attr(not(feature = "unstable-custom-transports"), allow(unreachable_pub))]
pub trait ConnectionPathSelector: Send + Sync + std::fmt::Debug + 'static {
    /// Optional reevaluation, bounded by the actor to 250ms through 30s.
    fn reevaluation_interval(&self) -> Option<Duration> {
        None
    }

    /// Enable connection-local ACK freshness, suspect detection and stream failover.
    fn uses_path_health(&self) -> bool {
        false
    }

    /// Pick a path using [`PathSelection::none`] and [`PathSelection::set`].
    fn select(&self, ctx: &PathSelectionContext<'_>) -> PathSelection;
}

/// The set of paths a [`PathSelector`] has chosen.
///
/// Today this holds at most one path.  Build via [`PathSelection::none`] +
/// [`PathSelection::set`].
#[derive(Debug, Clone)]
#[cfg_attr(not(feature = "unstable-custom-transports"), allow(unreachable_pub))]
pub struct PathSelection {
    selection: Option<transports::PathBinding>,
    original: Option<(usize, PathId)>,
    pub(crate) stream_paths: Vec<(usize, PathId)>,
}

#[cfg_attr(not(feature = "unstable-custom-transports"), allow(unreachable_pub))]
impl PathSelection {
    /// An empty selection.
    pub fn none() -> Self {
        Self {
            selection: None,
            original: None,
            stream_paths: Vec::new(),
        }
    }

    /// Sets the path as the selected path.
    ///
    /// This discards any previously selected path and sets this one as a single selected
    /// path.
    pub fn set(&mut self, path: &PathSelectionData<'_>) {
        if self.selection.is_some() {
            tracing::warn!(
                path = %path.network_path(),
                "PathSelection already contains a path; ignoring additional path"
            );
            return;
        }
        self.selection = Some(transports::PathBinding::new(
            path.network_path.clone(),
            path.local_socket,
        ));
        if let StatsSource::Live { path_id, conn } = &path.source {
            self.original = Some((conn.stable_id(), *path_id));
        }
    }

    /// The selected path: the one data should be sent on. This is not public so
    /// we can later allow for selecting multiple paths without changing the
    /// public API of `PathSelection`.
    ///
    /// Returns `None` when nothing has been selected.
    pub(crate) fn selected(&self) -> Option<&transports::PathBinding> {
        self.selection.as_ref()
    }

    fn for_connection(&self, state: &ConnectionState, conn: &noq::Connection) -> Option<PathId> {
        if state.member_paths.is_none() {
            let selected = self.selected()?;
            return state.paths.iter().find_map(|(id, path)| {
                (path == selected && state.path_is_qualified(*id, conn)).then_some(*id)
            });
        }
        let (owner, id) = self.original?;
        (owner == state.connection_id
            && state.paths.get(&id) == self.selected()
            && state.path_is_qualified(id, conn))
        .then_some(id)
    }
}

/// Future that resolves to the `conn_id` once a connection is closed.
///
/// This uses [`noq::Connection::on_closed`], which does not keep the connection alive
/// while awaiting the future.
struct OnClosed {
    conn_id: ConnId,
    inner: noq::OnClosed,
}

impl OnClosed {
    fn new(conn: &noq::Connection) -> Self {
        Self {
            conn_id: ConnId(conn.stable_id()),
            inner: conn.on_closed(),
        }
    }
}

impl Future for OnClosed {
    type Output = (ConnId, Closed);

    fn poll(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let closed = std::task::ready!(Pin::new(&mut self.inner).poll(cx));
        Poll::Ready((self.conn_id, closed))
    }
}

/// Converts an iterator of [`TransportAddr'] into an iterator of [`transports::Addr`].
fn to_transports_addr(
    endpoint_id: EndpointId,
    addrs: impl IntoIterator<Item = TransportAddr>,
) -> impl Iterator<Item = transports::Addr> {
    addrs.into_iter().filter_map(move |addr| match addr {
        TransportAddr::Relay(relay_url) => Some(transports::Addr::from((relay_url, endpoint_id))),
        TransportAddr::Ip(sockaddr) => Some(transports::Addr::from(sockaddr)),
        TransportAddr::Custom(custom_addr) => Some(transports::Addr::from(custom_addr)),
        _ => {
            warn!(?addr, "Unsupported TransportAddr");
            None
        }
    })
}

/// Returns the next item if `maybe_stream` is `Some`, or `None` otherwise.
async fn maybe_next<S: Stream + Unpin>(maybe_stream: Option<&mut S>) -> Option<Option<S::Item>> {
    match maybe_stream {
        None => None,
        Some(s) => Some(s.next().await),
    }
}
