//! A connection-owned RTC datagram bridge. This module never constructs an Endpoint.
//! RTC actors own the sockets/channel; this lease owns routing, bounded queues and
//! fresh original-path qualification. Limits are experimental application budgets.
use crate::control_protocol::AuthenticatedControl;
use crate::rtc_association::{DatagramQueue, QUEUE_BYTES};
use futures_util::{FutureExt, future::poll_fn, task::AtomicWaker};
use iroh::endpoint::{
    Connection,
    transports::{
        CustomEndpoint, CustomEndpointShutdown, CustomSender, CustomTransport, RecvInfo, Transmit,
    },
};
use iroh_base::{CustomAddr, TransportAddr};
use n0_watcher::{Direct, Watchable, Watcher};
use std::{
    collections::BTreeMap,
    future::Future,
    io,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};

pub const TRANSPORT_ID: u64 = 0x6d696f727463; // "miortc", data only after a fresh original-M ACK.
pub const MAX_DATAGRAM: usize = 2048;
const MAX_ASSOCIATIONS: usize = 32;

#[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
pub(crate) fn diagnostic_monotonic_ms() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    u64::try_from(
        START
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}
fn check_address(address: &CustomAddr) -> io::Result<()> {
    if address.id() != TRANSPORT_ID || address.data().len() != 16 {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid RTC route",
        ))
    } else {
        Ok(())
    }
}

struct Registry {
    ice_profile: crate::rtc_ice_profile::RtcIceProfile,
    routes: Mutex<BTreeMap<usize, Arc<Route>>>,
    addresses: Watchable<Vec<CustomAddr>>,
    retired: AtomicBool,
    receiver: AtomicWaker,
    resource_allowed: AtomicBool,
    resource_revision: AtomicU64,
    active_pumps: AtomicUsize,
    #[cfg(feature = "rtc-probe")]
    control_fixture: Mutex<Option<Arc<crate::rtc_invalidation_fixture::ControlFixture>>>,
    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    actual_dequeue_fixture:
        Mutex<Option<Arc<crate::rtc_actual_dequeue_fixture::ActualDequeueBarrier>>>,
    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    ready_input_fixture:
        Mutex<Option<Arc<crate::rtc_invalidation_fixture::CommonReadyInputFixture>>>,
}
impl Registry {
    fn refresh(&self) {
        let addresses = self
            .routes
            .lock()
            .unwrap()
            .values()
            .filter(|route| {
                route.ready.load(Ordering::Acquire) && !route.retired.load(Ordering::Acquire)
            })
            .map(|route| route.local.clone())
            .collect();
        let _ = self.addresses.set(addresses);
        self.receiver.wake();
    }
    fn matching(&self, remote: &CustomAddr) -> Option<Arc<Route>> {
        let routes: Vec<_> = self.routes.lock().unwrap().values().cloned().collect();
        routes
            .into_iter()
            .find(|route| route.remote.lock().unwrap().as_ref() == Some(remote))
    }
    fn close(&self) {
        self.retired.store(true, Ordering::Release);
        #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
        if let Some(fixture) = self.ready_input_fixture.lock().unwrap().take() {
            fixture.release();
        }
        let routes: Vec<_> = self.routes.lock().unwrap().values().cloned().collect();
        for route in routes {
            route.close_with_reason("registry-shutdown");
        }
        self.refresh();
    }
}

struct Route {
    registry: Weak<Registry>,
    original: iroh::endpoint::WeakControlConnection,
    original_peer: iroh_base::EndpointId,
    connection_id: usize,
    local: CustomAddr,
    remote: Mutex<Option<CustomAddr>>,
    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    /// A weak diagnostic handle for this route's exact RTC association. It
    /// never keeps the peer connection alive or owns its cleanup.
    peer_connection: Mutex<Option<Weak<dyn webrtc::peer_connection::PeerConnection>>>,
    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    pump_observation: Mutex<Option<Arc<dyn Fn() -> serde_json::Value + Send + Sync>>>,
    current: Box<dyn Fn() -> bool + Send + Sync>,
    ready: AtomicBool,
    qualified: AtomicBool,
    retired: AtomicBool,
    tx: Arc<DatagramQueue>,
    rx: Arc<DatagramQueue>,
    sent_packets: AtomicU64,
    sent_bytes: AtomicU64,
    received_packets: AtomicU64,
    received_bytes: AtomicU64,
    resource_revision: u64,
    pumps: AtomicUsize,
    retirement: AtomicWaker,
    qualification_updates: Option<Direct<u64>>,
    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    diagnostic_retirement: Mutex<Option<(&'static str, u64)>>,
}
impl Route {
    fn current(&self) -> bool {
        if self.retired.load(Ordering::Acquire) {
            return false;
        }
        let resource_current = self.registry.upgrade().is_some_and(|registry| {
            registry.resource_allowed.load(Ordering::Acquire)
                && registry.resource_revision.load(Ordering::Acquire) == self.resource_revision
        });
        if !resource_current {
            self.close_with_reason("resource-or-registry-invalid");
            return false;
        }
        if !(self.current)() {
            self.close_with_reason("qualification-or-attempt-invalid");
            return false;
        }
        if !self.original.is_open() {
            self.close_with_reason("original-M-closed");
            return false;
        }
        true
    }
    fn active(&self) -> bool {
        self.current() && self.ready.load(Ordering::Acquire)
    }

    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    async fn selected_pair(
        &self,
        budget: crate::rtc_association::RtcDeadline,
    ) -> io::Result<serde_json::Value> {
        if !self.active() || !self.qualified.load(Ordering::Acquire) {
            return Err(invalid("RTC selected route is not qualified"));
        }
        let peer = self
            .peer_connection
            .lock()
            .unwrap()
            .as_ref()
            .and_then(Weak::upgrade)
            .ok_or_else(|| invalid("RTC peer connection is no longer live"))?;
        if !self.active() || !self.qualified.load(Ordering::Acquire) {
            return Err(invalid("RTC selected route is not qualified"));
        }
        let pair = tokio::time::timeout(budget.remaining()?, async {
            peer.sctp()
                .await
                .ok_or_else(|| invalid("RTC SCTP transport is unavailable"))?
                .transport()
                .ice_transport()
                .get_selected_candidate_pair()
                .await
                .map_err(io::Error::other)?
                .ok_or_else(|| invalid("RTC selected candidate pair unavailable"))
        })
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "RTC selected-pair observation deadline exceeded",
            )
        })??;
        if !self.active() || !self.qualified.load(Ordering::Acquire) {
            return Err(invalid("RTC selected route changed during observation"));
        }
        Ok(serde_json::json!({"local":pair.local(),"remote":pair.remote()}))
    }
    fn close(&self) {
        self.close_with_reason("explicit-route-close");
    }
    fn close_with_reason(&self, reason: &'static str) {
        if self.retired.swap(true, Ordering::AcqRel) {
            return;
        }
        #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
        {
            *self.diagnostic_retirement.lock().unwrap() = Some((reason, diagnostic_monotonic_ms()));
        }
        #[cfg(not(all(feature = "rtc-probe", not(target_arch = "wasm32"))))]
        let _ = reason;
        self.ready.store(false, Ordering::Release);
        self.qualified.store(false, Ordering::Release);
        #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
        self.peer_connection.lock().unwrap().take();
        #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
        {
            let observer = self.pump_observation.lock().unwrap().take();
            drop(observer);
        }
        if let Some(registry) = self.registry.upgrade() {
            // Withdraw this exact route and publish the new address set before
            // queue/driver retirement wakes. No registry lock spans refresh.
            registry.routes.lock().unwrap().remove(&self.connection_id);
            registry.refresh();
        }
        self.tx.close();
        self.rx.close();
        self.retirement.wake();
    }
}

/// Install this factory on the existing endpoint before creating its member M.
#[derive(Clone)]
pub struct RtcTransport(Arc<Registry>);
impl std::fmt::Debug for RtcTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RtcTransport").finish_non_exhaustive()
    }
}
impl Default for RtcTransport {
    fn default() -> Self {
        Self::new()
    }
}
impl RtcTransport {
    pub fn new() -> Self {
        Self::with_ice_profile(crate::rtc_ice_profile::RtcIceProfile::default())
            .expect("empty RTC ICE profile is valid")
    }
    /// Install one validated immutable profile before the endpoint or its M exist.
    pub fn with_ice_profile(profile: crate::rtc_ice_profile::RtcIceProfile) -> io::Result<Self> {
        profile.validate()?;
        Ok(Self(Arc::new(Registry {
            ice_profile: profile,
            routes: Mutex::new(BTreeMap::new()),
            addresses: Watchable::new(Vec::new()),
            retired: AtomicBool::new(false),
            receiver: AtomicWaker::new(),
            resource_allowed: AtomicBool::new(true),
            resource_revision: AtomicU64::new(0),
            active_pumps: AtomicUsize::new(0),
            #[cfg(feature = "rtc-probe")]
            control_fixture: Mutex::new(None),
            #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
            actual_dequeue_fixture: Mutex::new(None),
            #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
            ready_input_fixture: Mutex::new(None),
        })))
    }
    pub fn ice_profile(&self) -> &crate::rtc_ice_profile::RtcIceProfile {
        &self.0.ice_profile
    }
    /// Native fixture only: delay completion of one actual, matching protected
    /// Common Ready read for this M. No candidate or carrier bytes are created.
    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    pub fn arm_common_ready_input_fixture(&self, original: usize) -> io::Result<()> {
        if self.0.retired.load(Ordering::Acquire) || !self.resource_allowed() {
            return Err(invalid("RTC fixture transport unavailable"));
        }
        let mut slot = self.0.ready_input_fixture.lock().unwrap();
        if slot.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Common Ready fixture already owned",
            ));
        }
        *slot = Some(Arc::new(
            crate::rtc_invalidation_fixture::CommonReadyInputFixture::new(original),
        ));
        Ok(())
    }
    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    pub fn common_ready_input_fixture_snapshot(&self) -> Option<serde_json::Value> {
        let fixture = self.0.ready_input_fixture.lock().unwrap().clone();
        fixture.and_then(|fixture| fixture.snapshot())
    }
    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    pub fn release_common_ready_input_fixture(&self) {
        let fixture = self.0.ready_input_fixture.lock().unwrap().clone();
        if let Some(fixture) = fixture {
            fixture.release();
        }
    }
    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    pub fn clear_common_ready_input_fixture(&self) {
        let fixture = self.0.ready_input_fixture.lock().unwrap().take();
        if let Some(fixture) = fixture {
            fixture.release();
        }
    }
    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    pub(crate) async fn after_actual_common_ready_input_fixture(
        &self,
        original: usize,
        control: usize,
        stream: String,
        actual_frame: &[u8],
        deadline: n0_future::time::Instant,
    ) -> io::Result<()> {
        let fixture = self.0.ready_input_fixture.lock().unwrap().clone();
        if let Some(fixture) = fixture {
            fixture
                .after_actual_ready_input(original, control, stream, actual_frame, deadline)
                .await?;
        }
        Ok(())
    }
    /// Reserve exactly one association for this original M. `current` must
    /// compare live membership/secret/resource/attempt generations, not a
    /// one-time admission snapshot. It runs under the QUIC connection lock and
    /// must use independent generation/permission tokens, never re-enter M.
    /// The first false result retires this lease.
    pub fn reserve(
        &self,
        connection: &(impl AuthenticatedControl + ?Sized),
        current: impl Fn() -> bool + Send + Sync + 'static,
    ) -> io::Result<RtcRouteLease> {
        self.reserve_inner(connection, current, None)
    }
    /// In addition to the lock-free packet predicate, observe qualification
    /// revision notifications while the channel is backpressured or idle.
    pub fn reserve_with_updates(
        &self,
        connection: &(impl AuthenticatedControl + ?Sized),
        current: impl Fn() -> bool + Send + Sync + 'static,
        updates: Direct<u64>,
    ) -> io::Result<RtcRouteLease> {
        self.reserve_inner(connection, current, Some(updates))
    }
    fn reserve_inner(
        &self,
        connection: &(impl AuthenticatedControl + ?Sized),
        current: impl Fn() -> bool + Send + Sync + 'static,
        qualification_updates: Option<Direct<u64>>,
    ) -> io::Result<RtcRouteLease> {
        if self.0.retired.load(Ordering::Acquire)
            || !self.0.resource_allowed.load(Ordering::Acquire)
            || connection.close_reason().is_some()
            || !current()
        {
            return Err(invalid("RTC original M or qualification is unavailable"));
        }
        let mut routes = self.0.routes.lock().unwrap();
        if routes.contains_key(&connection.stable_id()) || routes.len() >= MAX_ASSOCIATIONS {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "RTC association already owned by M or registry full",
            ));
        }
        let route = Arc::new(Route {
            registry: Arc::downgrade(&self.0),
            original: connection.revocation_handle(),
            original_peer: connection.remote_id(),
            connection_id: connection.stable_id(),
            local: CustomAddr::from_parts(TRANSPORT_ID, &rand::random::<[u8; 16]>()),
            remote: Mutex::new(None),
            #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
            peer_connection: Mutex::new(None),
            #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
            pump_observation: Mutex::new(None),
            current: Box::new(current),
            ready: AtomicBool::new(false),
            qualified: AtomicBool::new(false),
            retired: AtomicBool::new(false),
            tx: Arc::new(DatagramQueue::new(QUEUE_BYTES, MAX_DATAGRAM)),
            rx: Arc::new(DatagramQueue::new(QUEUE_BYTES, MAX_DATAGRAM)),
            sent_packets: AtomicU64::new(0),
            sent_bytes: AtomicU64::new(0),
            received_packets: AtomicU64::new(0),
            received_bytes: AtomicU64::new(0),
            resource_revision: self.0.resource_revision.load(Ordering::Acquire),
            pumps: AtomicUsize::new(0),
            retirement: AtomicWaker::new(),
            qualification_updates,
            #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
            diagnostic_retirement: Mutex::new(None),
        });
        routes.insert(connection.stable_id(), route.clone());
        Ok(RtcRouteLease(route))
    }
    pub fn active_pumps(&self) -> usize {
        self.0.active_pumps.load(Ordering::Acquire)
    }
    /// Retire only this original M's association when its existing maintenance
    /// operation replaces the failed carrier. Other Ms remain independent.
    pub fn retire_connection(&self, connection: &Connection) -> bool {
        let route = self
            .0
            .routes
            .lock()
            .unwrap()
            .get(&connection.stable_id())
            .cloned();
        if let Some(route) = route {
            if route.original.is_open() && connection.close_reason().is_none() {
                route.close_with_reason("connection-route-replaced");
                return true;
            }
        }
        false
    }
    pub fn active_associations(&self) -> usize {
        self.0.routes.lock().unwrap().len()
    }

    /// Read the selected ICE pair from the live peer connection attached to
    /// this exact original-M route. The weak handle is bounded by the route
    /// lifetime and the getter never changes transport state.
    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    pub async fn selected_pair(
        &self,
        connection_id: usize,
        budget: crate::rtc_association::RtcDeadline,
    ) -> io::Result<serde_json::Value> {
        let route = self
            .0
            .routes
            .lock()
            .unwrap()
            .get(&connection_id)
            .cloned()
            .ok_or_else(|| invalid("RTC association unavailable"))?;
        route.selected_pair(budget).await
    }
    /// Runtime permission for this experimental direct carrier. A change
    /// retires existing leases; re-enabling permits only new attempts.
    pub fn set_resource_allowed(&self, allowed: bool) {
        if self.0.resource_allowed.swap(allowed, Ordering::AcqRel) != allowed {
            self.0
                .resource_revision
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |revision| {
                    revision.checked_add(1)
                })
                .expect("RTC resource revision exhausted");
            let routes: Vec<_> = self.0.routes.lock().unwrap().values().cloned().collect();
            for route in routes {
                route.close_with_reason("resource-permission-changed");
            }
        }
    }
    pub fn resource_revision(&self) -> u64 {
        self.0.resource_revision.load(Ordering::Acquire)
    }

    pub fn resource_allowed(&self) -> bool {
        self.0.resource_allowed.load(Ordering::Acquire)
    }

    /// Arm only this actual selected, qualified original-M RTC route. The
    /// matcher must read bounded PRIVATE actual encoder diagnostics only;
    /// never capture a strong M/PC/lease/transport or guess ciphertext class.
    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    pub fn arm_actual_dequeue_fixture(
        &self,
        original: &Connection,
        selected_path_id: &str,
        remaining_deadline: n0_future::time::Instant,
        matcher: Arc<dyn Fn(&[u8]) -> bool + Send + Sync>,
    ) -> io::Result<()> {
        let route = self
            .0
            .routes
            .lock()
            .unwrap()
            .get(&original.stable_id())
            .cloned()
            .ok_or_else(|| invalid("actual dequeue fixture original route unavailable"))?;
        let remote = route
            .remote
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| invalid("actual dequeue fixture remote route unavailable"))?;
        if original.close_reason().is_some()
            || !route.active()
            || !route.qualified.load(Ordering::Acquire)
            || !(route.original.is_open()
                && route.connection_id == original.stable_id()
                && route.original_peer == original.remote_id())
            || !original.paths().iter().any(|path| {
                path.id().to_string() == selected_path_id
                    && path.is_selected()
                    && path.remote_addr() == &TransportAddr::Custom(remote.clone())
            })
        {
            return Err(invalid(
                "actual dequeue fixture requires selected qualified exact original M",
            ));
        }
        let fixture = Arc::new(
            crate::rtc_actual_dequeue_fixture::ActualDequeueBarrier::new(
                original.stable_id(),
                route.local.clone(),
                route.resource_revision,
                selected_path_id.to_owned(),
                remaining_deadline,
                matcher,
            )?,
        );
        {
            let mut slot = self.0.actual_dequeue_fixture.lock().unwrap();
            if slot.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "owned actual dequeue fixture already armed",
                ));
            }
            *slot = Some(fixture.clone());
        }
        // No registry/fixture lock spans qualification or endpoint operations.
        if original.close_reason().is_some()
            || !route.active()
            || !route.qualified.load(Ordering::Acquire)
        {
            {
                let mut slot = self.0.actual_dequeue_fixture.lock().unwrap();
                if slot
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &fixture))
                {
                    slot.take();
                }
            }
            fixture.release_or_cleanup();
            return Err(invalid("actual dequeue fixture route retired during arm"));
        }
        Ok(())
    }

    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    pub fn actual_dequeue_fixture_snapshot(&self) -> Option<serde_json::Value> {
        let fixture = self.0.actual_dequeue_fixture.lock().unwrap().clone()?;
        Some(fixture.snapshot())
    }

    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    pub fn release_actual_dequeue_fixture(&self) -> io::Result<()> {
        let fixture = self
            .0
            .actual_dequeue_fixture
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "actual dequeue fixture not armed")
            })?;
        fixture.release_or_cleanup();
        Ok(())
    }

    /// Signal scheduler release outside locks; keep waiting-state diagnostics
    /// until actual wait Drop is observed. Cleanup neither revives nor sends.
    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    pub fn clear_actual_dequeue_fixture(&self) -> io::Result<Option<serde_json::Value>> {
        let fixture = self.0.actual_dequeue_fixture.lock().unwrap().clone();
        let Some(fixture) = fixture else {
            return Ok(None);
        };
        fixture.release_or_cleanup();
        if fixture.is_waiting() {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "actual dequeue wait Drop remains unobserved; snapshot before cleanup",
            ));
        }
        let snapshot = fixture.snapshot();
        {
            let mut slot = self.0.actual_dequeue_fixture.lock().unwrap();
            if slot
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &fixture))
            {
                slot.take();
            }
        }
        Ok(Some(snapshot))
    }

    /// Optional one-shot hold for this owned test transport, not runtime policy.
    #[cfg(feature = "rtc-probe")]
    pub fn arm_control_fixture(
        &self,
        stage: crate::rtc_invalidation_fixture::ControlCheckpointStage,
    ) -> io::Result<()> {
        self.arm_control_fixture_inner(stage, false)
    }

    /// Owned cleanup scheduling fault; the actual PC closes after explicit
    /// release or the original setup deadline. Existing QUIC IO retires first.
    #[cfg(feature = "rtc-probe")]
    pub fn arm_control_fixture_with_cleanup_hold(
        &self,
        stage: crate::rtc_invalidation_fixture::ControlCheckpointStage,
    ) -> io::Result<()> {
        self.arm_control_fixture_inner(stage, true)
    }

    #[cfg(feature = "rtc-probe")]
    fn arm_control_fixture_inner(
        &self,
        stage: crate::rtc_invalidation_fixture::ControlCheckpointStage,
        hold_cleanup: bool,
    ) -> io::Result<()> {
        let mut fixture = self.0.control_fixture.lock().unwrap();
        if fixture.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "owned Control fixture already armed",
            ));
        }
        *fixture = Some(Arc::new(
            crate::rtc_invalidation_fixture::ControlFixture::new(stage, hold_cleanup),
        ));
        Ok(())
    }

    #[cfg(feature = "rtc-probe")]
    pub fn control_fixture_snapshot(&self) -> Option<serde_json::Value> {
        let fixture = self.0.control_fixture.lock().unwrap().clone()?;
        let mut snapshot = fixture.snapshot()?;
        snapshot["publishedRoutes"] = serde_json::json!(self.published_routes_fixture());
        Some(snapshot)
    }

    #[cfg(feature = "rtc-probe")]
    pub fn published_routes_fixture(&self) -> Vec<String> {
        self.0
            .addresses
            .get()
            .iter()
            .map(|address| hex::encode(address.data()))
            .collect()
    }

    /// Read-only owned Native pump facts. Callback cloning and invocation are
    /// separated from route/registry locks; raw route IDs stay private.
    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    pub fn pump_observations_fixture(&self) -> Vec<serde_json::Value> {
        let routes: Vec<_> = self.0.routes.lock().unwrap().values().cloned().collect();
        let mut values = Vec::new();
        for route in routes {
            let observer = route.pump_observation.lock().unwrap().clone();
            if let Some(observer) = observer {
                let observation = observer();
                values.push(serde_json::json!({
                    "originalStableId":route.connection_id,
                    "localRoute":hex::encode(route.local.data()),
                    "originalOpen":route.original.is_open(),
                    "routeReady":route.ready.load(Ordering::Acquire),
                    "routeRetired":route.retired.load(Ordering::Acquire),
                    "capturedResourceRevision":route.resource_revision,
                    "currentResourceRevision":self.0.resource_revision.load(Ordering::Acquire),
                    "resourceAllowed":self.0.resource_allowed.load(Ordering::Acquire),
                    "observation":observation,
                }));
            }
        }
        values
    }

    #[cfg(feature = "rtc-probe")]
    pub fn release_control_fixture(&self, attempt: &str) -> io::Result<()> {
        let fixture = self
            .0
            .control_fixture
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| invalid("owned Control fixture absent"))?;
        fixture.release(attempt)
    }

    #[cfg(feature = "rtc-probe")]
    pub fn clear_control_fixture(&self) {
        if let Some(fixture) = self.0.control_fixture.lock().unwrap().take() {
            fixture.clear();
        }
    }

    #[cfg(feature = "rtc-probe")]
    pub async fn before_pc_close_fixture(
        &self,
        original: usize,
        budget: crate::rtc_association::RtcDeadline,
    ) {
        let fixture = self.0.control_fixture.lock().unwrap().clone();
        if let Some(fixture) = fixture {
            fixture.before_pc_close(original, budget).await;
        }
    }

    #[cfg(feature = "rtc-probe")]
    pub fn release_cleanup_fixture(&self, attempt: &str) -> io::Result<()> {
        let fixture = self
            .0
            .control_fixture
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| invalid("owned cleanup fixture absent"))?;
        fixture.release_cleanup(attempt)
    }

    #[cfg(feature = "rtc-probe")]
    pub fn holds_cleanup_fixture(&self) -> bool {
        self.0
            .control_fixture
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|fixture| fixture.holds_cleanup())
    }

    #[cfg(feature = "rtc-probe")]
    pub fn record_cleanup_details_fixture(&self, original: usize, details: serde_json::Value) {
        let fixture = self.0.control_fixture.lock().unwrap().clone();
        if let Some(fixture) = fixture {
            fixture.record_cleanup_details(original, details);
        }
    }

    #[cfg(feature = "rtc-probe")]
    pub async fn control_fixture_checkpoint(
        &self,
        stage: crate::rtc_invalidation_fixture::ControlCheckpointStage,
        original: &(impl AuthenticatedControl + ?Sized),
        lease: &RtcRouteLease,
        budget: crate::rtc_association::RtcDeadline,
        metadata: serde_json::Value,
        actual_sends: Arc<dyn Fn() -> serde_json::Value + Send + Sync>,
    ) -> io::Result<()> {
        let fixture = self.0.control_fixture.lock().unwrap().clone();
        if let Some(fixture) = fixture {
            fixture
                .checkpoint(stage, original, lease, budget, metadata, actual_sends)
                .await?;
        }
        Ok(())
    }
}

/// RAII ownership remains with the original member's RTC actor. Dropping it
/// closes both queues and withdraws the exact route; it does not close another M.
pub struct RtcRouteLease(Arc<Route>);
impl RtcRouteLease {
    pub fn local_addr(&self) -> CustomAddr {
        self.0.local.clone()
    }
    pub fn remote_addr(&self) -> io::Result<CustomAddr> {
        self.0
            .remote
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| invalid("RTC peer route not bound"))
    }
    pub fn set_remote(&self, remote: CustomAddr) -> io::Result<()> {
        check_address(&remote)?;
        if !self.0.current() {
            return Err(invalid("RTC lease retired"));
        }
        let mut address = self.0.remote.lock().unwrap();
        if address.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "RTC peer route already bound",
            ));
        }
        *address = Some(remote);
        Ok(())
    }
    /// Called only after the actor verifies the negotiated profile and channel open.
    pub fn activate(&self) -> io::Result<()> {
        if !self.0.current() || self.remote_addr().is_err() {
            return Err(invalid("RTC lease not current and bound"));
        }
        self.0.tx.open().map_err(io::Error::other)?;
        self.0.rx.open().map_err(io::Error::other)?;
        self.0.ready.store(true, Ordering::Release);
        if let Some(registry) = self.0.registry.upgrade() {
            registry.refresh();
        }
        Ok(())
    }
    pub fn io(&self) -> RtcDatagramIo {
        RtcDatagramIo(self.0.clone())
    }
    pub fn close(&self) {
        self.0.close();
    }

    /// Attach a weak handle only after the caller's cancellation guard owns a
    /// strong peer connection. This keeps observation tied to this route and
    /// cannot extend the PC lifetime.
    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    pub fn attach_peer_connection(
        &self,
        peer: &Arc<dyn webrtc::peer_connection::PeerConnection>,
    ) -> io::Result<()> {
        if !self.0.active() {
            return Err(invalid("RTC association unavailable"));
        }
        let mut attached = self.0.peer_connection.lock().unwrap();
        if attached.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "RTC peer connection already attached",
            ));
        }
        *attached = Some(Arc::downgrade(peer));
        Ok(())
    }
    /// Attach one read-only Native production-pump diagnostic. The callback
    /// must capture only pump statistics, never a strong M/PC/transport/lease.
    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    pub fn attach_pump_observation(
        &self,
        observer: Arc<dyn Fn() -> serde_json::Value + Send + Sync>,
    ) -> io::Result<()> {
        if !self.0.active() {
            return Err(invalid("RTC association unavailable for pump observation"));
        }
        let mut attached = self.0.pump_observation.lock().unwrap();
        if self.0.retired.load(Ordering::Acquire) {
            return Err(invalid("RTC association retired before pump observation"));
        }
        if attached.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "RTC pump observation already attached",
            ));
        }
        *attached = Some(observer);
        Ok(())
    }

    /// Start a fresh QUIC PING on the exact address of this original M.
    /// A DataChannel open event or an old path snapshot cannot construct this proof.
    pub fn start_probe(
        &self,
        connection: &(impl AuthenticatedControl + ?Sized),
    ) -> io::Result<RtcPathProbe> {
        if connection.stable_id() != self.0.connection_id || !self.0.active() {
            return Err(invalid("RTC probe does not belong to this original M"));
        }
        let remote = TransportAddr::Custom(self.remote_addr()?);
        let paths = connection.paths();
        let path = paths
            .iter()
            .find(|path| path.remote_addr() == &remote)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::WouldBlock, "RTC QUIC path not established")
            })?;
        let path_id = path.id().to_string();
        let probe = path.start_probe()?;
        let watermark = probe.watermark();
        Ok(RtcPathProbe {
            route: self.0.clone(),
            path_id,
            watermark,
            acknowledged: Box::new(move || probe.is_acknowledged().map_err(io::Error::other)),
        })
    }
    /// Only this sealed fresh probe, still on the same live path/attempt, can
    /// grant ordinary data. Packet admission repeats the live qualification.
    pub fn retain(
        &self,
        connection: &(impl AuthenticatedControl + ?Sized),
        probe: &RtcPathProbe,
    ) -> io::Result<()> {
        if connection.stable_id() != self.0.connection_id
            || !Arc::ptr_eq(&self.0, &probe.route)
            || !self.0.active()
            || !probe.is_acknowledged()?
            || !connection.paths().iter().any(|path| {
                path.id().to_string() == probe.path_id
                    && path.remote_addr() == &TransportAddr::Custom(self.remote_addr().unwrap())
            })
        {
            return Err(invalid(
                "RTC path lacks a fresh ACK for this current original M lease",
            ));
        }
        self.0.qualified.store(true, Ordering::Release);
        Ok(())
    }
    pub fn report(&self) -> RtcRouteReport {
        self.io().report()
    }

    /// Renew exact original-M liveness after the final asynchronous private
    /// confirmation, before ordinary release. An earlier ACK can outlive the
    /// peer's current credential/source; it cannot satisfy this new watermark.
    pub async fn validate_release(
        &self,
        connection: &(impl AuthenticatedControl + ?Sized),
        budget: crate::rtc_association::RtcDeadline,
    ) -> io::Result<RtcPathProbe> {
        let probe = self.start_probe(connection)?;
        #[cfg(feature = "rtc-probe")]
        let fixture = self
            .0
            .registry
            .upgrade()
            .and_then(|registry| registry.control_fixture.lock().unwrap().clone());
        #[cfg(feature = "rtc-probe")]
        if let Some(fixture) = &fixture {
            fixture.record_release_probe(connection.stable_id(), serde_json::json!({
                "pathId":probe.path_id(),"watermark":probe.watermark(),"startedMs":budget.elapsed_ms(),"acknowledged":false,
            }));
        }
        let wait = async {
            while !probe.is_acknowledged()? {
                budget.remaining()?;
                n0_future::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            Ok::<_, io::Error>(())
        };
        n0_future::time::timeout(budget.remaining()?, self.io().while_active(wait))
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "original RTC release probe deadline",
                )
            })???;
        #[cfg(feature = "rtc-probe")]
        if let Some(fixture) = &fixture {
            fixture.record_release_probe(connection.stable_id(), serde_json::json!({
                "pathId":probe.path_id(),"watermark":probe.watermark(),"acknowledgedMs":budget.elapsed_ms(),"acknowledged":true,
            }));
        }
        Ok(probe)
    }
}
impl Drop for RtcRouteLease {
    fn drop(&mut self) {
        self.close();
    }
}

pub struct RtcPathProbe {
    route: Arc<Route>,
    path_id: String,
    watermark: u64,
    acknowledged: Box<dyn Fn() -> io::Result<bool> + Send + Sync>,
}
impl RtcPathProbe {
    pub fn path_id(&self) -> &str {
        &self.path_id
    }
    pub fn watermark(&self) -> u64 {
        self.watermark
    }
    pub fn is_acknowledged(&self) -> io::Result<bool> {
        if !self.route.active() {
            return Err(invalid("RTC probe lease retired"));
        }
        (self.acknowledged)()
    }
}

/// Cloneable queue handles for the single RTC send/event pump. They cannot
/// activate or qualify a route and do not extend the original M's lifetime.
#[derive(Clone)]
pub struct RtcDatagramIo(Arc<Route>);
/// One real driver task owns this guard. Every task exit retires its exact
/// route before coordination can retain or select it, including early errors.
pub struct RtcPumpGuard {
    io: RtcDatagramIo,
    registry: Option<Arc<Registry>>,
}
impl Drop for RtcPumpGuard {
    fn drop(&mut self) {
        self.io.retire_with_reason("pump-guard-exit");
        self.io.0.pumps.fetch_sub(1, Ordering::AcqRel);
        if let Some(registry) = &self.registry {
            registry.active_pumps.fetch_sub(1, Ordering::AcqRel);
        }
    }
}
impl RtcDatagramIo {
    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    fn actual_dequeue_fixture(
        &self,
    ) -> Option<Arc<crate::rtc_actual_dequeue_fixture::ActualDequeueBarrier>> {
        let registry = self.0.registry.upgrade()?;
        let fixture = registry.actual_dequeue_fixture.lock().unwrap().clone();
        fixture
    }

    /// Called only AFTER the genuine packet occupies production pending,
    /// INSIDE existing while_active. No nested route watcher or fake queue.
    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    pub(crate) async fn after_actual_pending_fixture(&self, packet: &[u8]) -> io::Result<()> {
        let Some(fixture) = self.actual_dequeue_fixture() else {
            return Ok(());
        };
        let remote = self.0.remote.lock().unwrap().clone();
        let selected = remote.is_some_and(|remote| {
            self.0
                .original
                .path_is_selected(fixture.path_id(), &TransportAddr::Custom(remote))
        });
        if !selected {
            return Ok(());
        }
        fixture
            .after_actual_pending(self, &self.0.local, packet)
            .await
    }

    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    pub(crate) fn observe_actual_pending_work_end(&self, packet: Option<&[u8]>) {
        if let Some(fixture) = self.actual_dequeue_fixture() {
            fixture.observe_work_ended_pending(self, &self.0.local, packet);
        }
    }

    /// Cancel a driver's held work before it can use newly available channel
    /// capacity after qualification/resource retirement or original-M closure.
    pub async fn while_active<F: Future>(&self, work: F) -> io::Result<F::Output> {
        self.while_live(work, true).await
    }

    /// Guard Reserved preparation before handing the one retirement watcher to
    /// the active pump. This grants no path or application-data qualification.
    pub async fn while_current<F: Future>(&self, work: F) -> io::Result<F::Output> {
        self.while_live(work, false).await
    }

    async fn while_live<F: Future>(&self, work: F, require_ready: bool) -> io::Result<F::Output> {
        let live = || self.0.current() && (!require_ready || self.0.ready.load(Ordering::Acquire));
        let retired = poll_fn(|cx| {
            self.0.retirement.register(cx.waker());
            if live() {
                Poll::Pending
            } else {
                Poll::Ready(())
            }
        })
        .fuse();
        let closed = self.0.original.closed().fuse();
        let qualification = async {
            if let Some(mut updates) = self.0.qualification_updates.clone() {
                loop {
                    if !live() {
                        return;
                    }
                    if updates.updated().await.is_err() {
                        self.retire();
                        return;
                    }
                }
            } else {
                std::future::pending::<()>().await;
            }
        }
        .fuse();
        let work = work.fuse();
        futures_util::pin_mut!(retired, closed, qualification, work);
        futures_util::select_biased! {
            _ = retired => Err(invalid("RTC datagram route retired")),
            _ = closed => { self.retire(); Err(invalid("RTC original M closed")) },
            _ = qualification => { self.retire(); Err(invalid("RTC qualification retired")) },
            result = work => Ok(result),
        }
    }
    pub fn pump_guard(&self) -> RtcPumpGuard {
        let registry = self.0.registry.upgrade();
        self.0.pumps.fetch_add(1, Ordering::AcqRel);
        if let Some(registry) = &registry {
            registry.active_pumps.fetch_add(1, Ordering::AcqRel);
        }
        RtcPumpGuard {
            io: self.clone(),
            registry,
        }
    }
    pub fn poll_outbound(&self, cx: &mut Context<'_>) -> Poll<io::Result<Vec<u8>>> {
        if !self.0.active() {
            return Poll::Ready(Err(invalid("RTC association unavailable")));
        }
        self.0.tx.poll_pop(cx).map_err(io::Error::other)
    }
    pub fn receive(&self, packet: Vec<u8>) -> io::Result<()> {
        if !self.0.active() {
            return Err(invalid("RTC receive association unavailable"));
        }
        let length = packet.len();
        self.0.rx.try_push(packet).map_err(io::Error::other)?;
        self.0.received_packets.fetch_add(1, Ordering::Relaxed);
        self.0
            .received_bytes
            .fetch_add(length as u64, Ordering::Relaxed);
        if let Some(registry) = self.0.registry.upgrade() {
            registry.receiver.wake();
        }
        Ok(())
    }
    pub fn is_active(&self) -> bool {
        self.0.active()
    }
    pub fn retire(&self) {
        self.0.close();
    }
    pub(crate) fn retire_with_reason(&self, reason: &'static str) {
        self.0.close_with_reason(reason);
    }
    #[cfg(all(feature = "rtc-probe", not(target_arch = "wasm32")))]
    pub(crate) fn retirement_diagnostic(&self) -> serde_json::Value {
        match *self.0.diagnostic_retirement.lock().unwrap() {
            Some((reason, at)) => serde_json::json!({"reason":reason,"monotonicMs":at}),
            None => serde_json::Value::Null,
        }
    }
    pub fn report(&self) -> RtcRouteReport {
        RtcRouteReport {
            original_stable_id: self.0.connection_id,
            active: self.0.active(),
            qualified: self.0.qualified.load(Ordering::Acquire),
            sent_packets: self.0.sent_packets.load(Ordering::Relaxed),
            sent_bytes: self.0.sent_bytes.load(Ordering::Relaxed),
            received_packets: self.0.received_packets.load(Ordering::Relaxed),
            received_bytes: self.0.received_bytes.load(Ordering::Relaxed),
            tx_bytes: self.0.tx.bytes(),
            rx_bytes: self.0.rx.bytes(),
            tx_peak_bytes: self.0.tx.peak_bytes(),
            rx_peak_bytes: self.0.rx.peak_bytes(),
            resource_revision: self.0.resource_revision,
            owned_pump_tasks: self.0.pumps.load(Ordering::Acquire),
        }
    }
}
#[derive(Debug, serde::Serialize)]
pub struct RtcRouteReport {
    pub original_stable_id: usize,
    pub active: bool,
    pub qualified: bool,
    pub sent_packets: u64,
    pub sent_bytes: u64,
    pub received_packets: u64,
    pub received_bytes: u64,
    pub tx_bytes: usize,
    pub rx_bytes: usize,
    pub tx_peak_bytes: usize,
    pub rx_peak_bytes: usize,
    pub resource_revision: u64,
    pub owned_pump_tasks: usize,
}

impl CustomTransport for RtcTransport {
    fn bind(&self) -> io::Result<Box<dyn CustomEndpoint>> {
        Ok(Box::new(self.clone()))
    }
}
impl CustomEndpoint for RtcTransport {
    fn shutdown_handler(&self) -> Option<Arc<dyn CustomEndpointShutdown>> {
        Some(Arc::new(self.clone()))
    }
    fn watch_local_addrs(&self) -> Direct<Vec<CustomAddr>> {
        self.0.addresses.watch()
    }
    fn create_sender(&self) -> Arc<dyn CustomSender> {
        Arc::new(self.clone())
    }
    fn poll_recv(
        &mut self,
        cx: &mut Context<'_>,
        buffers: &mut [io::IoSliceMut<'_>],
        metas: &mut [noq_udp::RecvMeta],
        infos: &mut [RecvInfo],
    ) -> Poll<io::Result<usize>> {
        self.0.receiver.register(cx.waker());
        if buffers.is_empty() {
            return Poll::Ready(Ok(0));
        }
        let routes: Vec<_> = self.0.routes.lock().unwrap().values().cloned().collect();
        for route in routes {
            if !route.active() {
                continue;
            }
            if let Ok(Some(packet)) = route.rx.try_pop() {
                if packet.len() > buffers[0].len() {
                    route.close_with_reason("receive-buffer-too-small");
                    continue;
                }
                buffers[0][..packet.len()].copy_from_slice(&packet);
                metas[0].len = packet.len();
                metas[0].stride = packet.len();
                infos[0] = RecvInfo::new(
                    route.remote.lock().unwrap().clone().unwrap(),
                    Some(route.local.clone()),
                );
                return Poll::Ready(Ok(1));
            }
        }
        Poll::Pending
    }
}
impl CustomSender for RtcTransport {
    fn is_valid_send_addr(&self, address: &CustomAddr) -> bool {
        self.0.matching(address).is_some_and(|route| route.active())
    }
    fn guards_application_data(&self, address: &CustomAddr) -> bool {
        address.id() == TRANSPORT_ID
    }
    fn can_bootstrap_connection(&self, _address: &CustomAddr) -> bool {
        false
    }
    fn is_valid_connection_addr(&self, address: &CustomAddr, connection_id: usize) -> bool {
        self.0
            .matching(address)
            .is_some_and(|route| route.connection_id == connection_id && route.active())
    }
    fn is_data_path_qualified(&self, address: &CustomAddr, connection_id: usize) -> bool {
        self.0.matching(address).is_some_and(|route| {
            route.connection_id == connection_id
                && route.active()
                && route.qualified.load(Ordering::Acquire)
        })
    }
    fn poll_send(
        &self,
        cx: &mut Context<'_>,
        destination: &CustomAddr,
        source: Option<&CustomAddr>,
        transmit: &Transmit<'_>,
    ) -> Poll<io::Result<()>> {
        let Some(route) = self
            .0
            .matching(destination)
            .filter(|route| route.active() && source.is_none_or(|source| source == &route.local))
        else {
            return Poll::Ready(Err(invalid("RTC send route unavailable")));
        };
        if transmit.segment_size.is_some() {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "RTC bridge rejects GSO",
            )));
        }
        match route.tx.poll_capacity(cx, transmit.contents.len()) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => Poll::Ready(Err(io::Error::other(error))),
            Poll::Ready(Ok(())) => {
                let result = route
                    .tx
                    .try_push(transmit.contents.to_vec())
                    .map_err(io::Error::other);
                if result.is_ok() {
                    route.sent_packets.fetch_add(1, Ordering::Relaxed);
                    route
                        .sent_bytes
                        .fetch_add(transmit.contents.len() as u64, Ordering::Relaxed);
                }
                Poll::Ready(result)
            }
        }
    }
}
impl CustomEndpointShutdown for RtcTransport {
    fn close(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        self.0.close();
        Box::pin(std::future::ready(()))
    }
}
