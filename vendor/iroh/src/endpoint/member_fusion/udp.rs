//! Enhanced UDP coordination on the protected, already admitted original.

use std::{
    future::pending,
    io,
    sync::{Arc, Mutex},
    time::Instant as StdInstant,
};

use iroh_relay::RelayMap;
use n0_watcher::Watcher as _;
use tokio::sync::watch;

use super::{
    Activity, CancellationToken, Connection, ControlExchange, Duration, Endpoint, Instant, PathId,
    StreamKind, time,
};
use crate::{
    RelayUrl,
    endpoint::{
        ControlMessageFormat, ObservationValidity, UdpMappingSample, UdpMappingSequence,
        nat_plan::{MAX_OBSERVATIONS, PhasePlan, SNAPSHOT_LIFETIME, SnapshotWire, make_plan},
        nat_selector::{
            Analyzer, BehaviorId, LearningScope, Recommendation, behaviors, initial_scores,
        },
    },
    socket::remote_map::MemberUdpPathWork,
};

mod execute;
mod slot;
mod wire;
pub use execute::MemberUdpTraversalReport;
use slot::{Priority, Reservation, Slot};
use wire::{Hello, Round};

const WRAP: ControlMessageFormat = ControlMessageFormat::LengthPrefixed16 { maximum: 2048 };
const CONTROL_WAIT: Duration = Duration::from_secs(5);

/// Native resources and ceilings for a single coordinated UDP traversal round.
#[derive(Debug, Clone)]
pub struct MemberUdpConfig {
    /// Allow enhanced observation and coordination for this endpoint.
    pub enabled: bool,
    /// Configured QUIC address-discovery observers.
    pub qad: RelayMap,
    /// Configured UDP STUN observers.
    pub stun: Vec<RelayUrl>,
    /// Maximum number of observation requests reserved for this round.
    pub observations: usize,
    /// Maximum datagrams the local phase may send.
    pub peer_datagrams: usize,
    /// Local maximum for the entire round, including observation and coordination.
    pub round_timeout: Duration,
    /// Minimum separation between physical rounds on this node after one starts work.
    pub round_interval: Duration,
    /// Local pacing for business and periodic maintenance requests.
    pub quality_interval: Duration,
}

/// The single agreed frp plan and the local phase on the admitted original.
#[derive(Debug, Clone)]
pub struct MemberUdpPlan {
    /// Protected round identifier, the same at both ends.
    pub round: [u8; 16],
    /// The mode and index selected by the receiving coordinator.
    pub id: BehaviorId,
    /// Whether either peer could only provide candidate evidence.
    pub fallback: bool,
    /// Whether neither peer supplied a usable candidate.
    pub disabled: bool,
    /// Local role, targets, send delay and phase limits.
    pub local_phase: PhasePlan,
    /// Validated packet and phase limits of the peer.
    pub peer_phase: PhasePlan,
    /// Agreed maximum runtime for the two phases, bounded by the original round.
    pub execution_budget: Duration,
    /// Local absolute deadline for this plan on the original connection.
    pub deadline: Instant,
    /// Side of the protected planning stream that selected the behavior.
    pub coordinator: bool,
    evidence: Evidence,
    scope: RoundScope,
}

impl MemberUdpPlan {
    /// Valid samples used on this endpoint, in arrival order.
    pub fn local_samples(&self) -> &[UdpMappingSample] {
        self.evidence
            .sequence
            .as_ref()
            .map_or(&[], |value| &value.samples)
    }
}

#[derive(Debug)]
pub(super) struct Planner {
    analyzer: Mutex<Analyzer>,
    slot: Slot,
    standby_turn: tokio::sync::Mutex<()>,
    #[cfg(feature = "miopunch-test-hooks")]
    reservation_barrier: Mutex<Option<Arc<tokio::sync::Barrier>>>,
}

impl Default for Planner {
    fn default() -> Self {
        Self {
            analyzer: Mutex::new(Analyzer::new(StdInstant::now())),
            slot: Slot::default(),
            standby_turn: Default::default(),
            #[cfg(feature = "miopunch-test-hooks")]
            reservation_barrier: Mutex::default(),
        }
    }
}

impl Planner {
    pub(super) fn foreground(&self) -> watch::Receiver<bool> {
        self.slot.foreground()
    }

    pub(super) fn business_changed(&self, active: bool) {
        self.slot.business_changed(active);
    }

    #[cfg(feature = "miopunch-test-hooks")]
    pub(super) fn test_coordinate_reservation(&self, barrier: Arc<tokio::sync::Barrier>) {
        *self
            .reservation_barrier
            .lock()
            .expect("UDP reservation barrier") = Some(barrier);
    }
}

#[derive(Debug)]
pub(in super::super) struct Planning {
    endpoint: Endpoint,
    connection: Connection,
    config: MemberUdpConfig,
    owner: Arc<Planner>,
    retention: super::MemberRetention,
    retry_base: Duration,
    path_work: Arc<MemberUdpPathWork>,
    maintenance: watch::Sender<Option<Instant>>,
    latest: Mutex<Option<MemberUdpPlan>>,
    traversal: Mutex<Option<Arc<execute::Execution>>>,
}

impl Planning {
    pub(super) fn new(
        endpoint: &Endpoint,
        connection: &Connection,
        owner: Arc<Planner>,
        retention: super::MemberRetention,
    ) -> Option<Arc<Self>> {
        let policy = endpoint.inner.static_config.admission.as_ref()?;
        let config = policy.member_udp_config()?.clone();
        Some(Arc::new(Self {
            endpoint: endpoint.clone(),
            connection: connection.clone(),
            config,
            owner,
            retry_base: policy.member_path_retry_base(),
            retention,
            path_work: Arc::default(),
            maintenance: watch::Sender::default(),
            latest: Mutex::new(None),
            traversal: Mutex::new(None),
        }))
    }

    pub(in super::super) fn path_work(&self) -> Option<Arc<MemberUdpPathWork>> {
        self.can_maintain().then(|| self.path_work.clone())
    }

    fn can_maintain(&self) -> bool {
        self.config.enabled && (!self.config.qad.is_empty() || !self.config.stun.is_empty())
    }

    pub(super) async fn bootstrap_requested(&self) -> bool {
        self.can_maintain()
            && self.scope().is_ok()
            && self.sources().is_ok()
            && self
                .path_work
                .needed
                .subscribe()
                .wait_for(|needed| *needed)
                .await
                .is_ok()
    }

    pub(super) async fn bootstrap(
        &self,
        exchange: ControlExchange,
        deadline: Instant,
        cancel: &CancellationToken,
    ) -> io::Result<PathId> {
        // Reuse the same physical round as online improvement. Both sides need
        // its actual selected path, but cannot confirm business during admission.
        let kind = StreamKind::UdpImprovement;
        if self.connection.side().is_client() {
            self.initiate_on(deadline, cancel, kind, Some(exchange))
                .await?;
        } else {
            self.respond_until(exchange, None, kind, deadline).await?;
        }
        self.traversal()
            .and_then(|report| report.validated.map(|(path, _)| path))
            .ok_or_else(|| io::ErrorKind::NotConnected.into())
    }

    pub(super) fn request_maintenance(&self) {
        if self.can_maintain() {
            self.maintenance.send_replace(Some(Instant::now()));
        }
    }

    pub(super) async fn maintain(&self, activity: &Activity) {
        if !self.can_maintain() {
            return pending().await;
        }
        let mut needed = self.path_work.needed.subscribe();
        let mut active = activity.subscribe();
        let mut maintenance = self.maintenance.subscribe();
        let mut permitted = activity.path_activity.subscribe();
        let cancel = CancellationToken::new();
        let interval = self.config.quality_interval;
        // First eligible active work shares the current search opportunity.
        // Subsequent rounds keep the configured quality pacing below.
        let mut active_at = Instant::now();
        let mut maintenance_at = Instant::now();
        let mut automatic_at = Instant::now() + interval;
        let mut failures = 0u32;
        let mut last_check = None;
        let standby = matches!(self.retention, super::MemberRetention::Standby(_));
        let mut generation = self.endpoint.udp_network_generation();
        loop {
            if standby && !generation.is_current() {
                generation = self.endpoint.udp_network_generation();
                failures = 0;
                automatic_at = Instant::now();
                maintenance_at = automatic_at;
            }
            let idle_since = activity.idle_since();
            let request = *maintenance.borrow_and_update();
            let eligible = *needed.borrow_and_update();
            let (due, deadline, kind) = if let Some(since) = idle_since {
                let fresh_check = request.is_some_and(|at| at >= since && last_check != request);
                let (deadline, due) = match self.retention {
                    super::MemberRetention::Standby(_) => {
                        let ready = *permitted.borrow_and_update();
                        let due = if fresh_check {
                            Some(maintenance_at)
                        } else if self.connection.side().is_client() {
                            Some(automatic_at)
                        } else {
                            None
                        };
                        (
                            Instant::now() + self.config.round_timeout,
                            due.filter(|_| ready),
                        )
                    }
                    super::MemberRetention::IdleTimeout(timeout) => {
                        (since + timeout, fresh_check.then_some(maintenance_at))
                    }
                };
                (
                    due.filter(|_| eligible),
                    deadline,
                    StreamKind::UdpMaintenance,
                )
            } else {
                automatic_at = Instant::now() + interval;
                last_check = request;
                (
                    eligible.then_some(active_at),
                    Instant::now() + self.config.round_timeout,
                    StreamKind::UdpImprovement,
                )
            };
            if let Some(due) = due.filter(|due| *due <= Instant::now())
                && due < deadline
                && deadline > Instant::now()
            {
                if kind == StreamKind::UdpMaintenance {
                    maintenance_at = Instant::now() + interval;
                    last_check = request;
                    let result = if standby {
                        let turn = async {
                            let turn = self.owner.standby_turn.lock().await;
                            time::sleep_until(self.owner.slot.next_physical_round()).await;
                            turn
                        };
                        let _turn = tokio::select! { biased;
                            _ = active.wait_for(|yes| *yes) => continue,
                            _ = permitted.wait_for(|yes| !*yes) => continue,
                            _ = needed.wait_for(|yes| !*yes) => continue,
                            _ = self.endpoint.inner.wait_network_generation_change(&generation) => continue,
                            turn = turn => turn,
                        };
                        self.initiate(Instant::now() + self.config.round_timeout, &cancel, kind)
                            .await
                    } else {
                        self.initiate(deadline, &cancel, kind).await
                    };
                    match result {
                        Ok(_) => failures = 0,
                        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                        Err(_) => failures = (failures + 1).min(5),
                    }
                    automatic_at = Instant::now() + self.retry_base * (1 << failures);
                    if standby {
                        maintenance_at = automatic_at;
                    }
                } else {
                    active_at = Instant::now() + interval;
                    let inactive = active.wait_for(|active| !*active);
                    tokio::select! { biased;
                        _ = inactive => {},
                        _ = self.initiate(deadline, &cancel, kind) => {},
                    }
                }
                continue;
            }
            tokio::select! {
                _ = self.endpoint.inner.wait_network_generation_change(&generation), if standby => {},
                _ = active.changed() => {},
                _ = needed.changed() => {},
                _ = permitted.changed() => {},
                _ = maintenance.changed() => {},
                _ = async {
                    match due {
                        Some(due) if standby || (due < deadline && deadline > Instant::now()) => time::sleep_until(due).await,
                        _ => pending().await,
                    }
                } => {}
            }
        }
    }

    pub(super) fn current(&self) -> Option<MemberUdpPlan> {
        let plan = self.latest.lock().expect("member UDP plan").clone()?;
        (Instant::now() < plan.deadline
            && self.check_scope(&plan.scope).is_ok()
            && plan.evidence.check().is_ok())
        .then_some(plan)
    }

    fn scope(&self) -> io::Result<LearningScope> {
        if !self.config.enabled {
            return Err(denied("enhanced UDP observation denied"));
        }
        if self.connection.close_reason().is_some() {
            return Err(io::ErrorKind::NotConnected.into());
        }
        self.endpoint
            .inner
            .static_config
            .admission
            .as_ref()
            .and_then(|policy| policy.member_udp_scope(&self.connection))
            .ok_or_else(|| denied("current member authorization required for enhanced UDP"))
    }

    fn round_scope(&self, authorization: LearningScope) -> io::Result<RoundScope> {
        Ok(RoundScope {
            authorization,
            generation: self.endpoint.udp_network_generation(),
            sources: self.sources()?,
        })
    }

    fn check_scope(&self, scope: &RoundScope) -> io::Result<()> {
        if !scope.generation.is_current() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "UDP network changed",
            ));
        }
        if scope.authorization != self.scope()? {
            return Err(denied("member scope changed during UDP coordination"));
        }
        if scope
            .sources
            .qad
            .relays::<Vec<_>>()
            .iter()
            .any(|relay| !self.allows(&relay.url))
            || scope.sources.stun.iter().any(|url| !self.allows(url))
        {
            return Err(denied("UDP observation permission changed"));
        }
        Ok(())
    }

    async fn guarded<T>(
        &self,
        deadline: Instant,
        scope: &RoundScope,
        work: impl Future<Output = io::Result<T>>,
    ) -> io::Result<T> {
        let scope_changed = async {
            loop {
                self.check_scope(scope)?;
                time::sleep(Duration::from_millis(10)).await;
            }
        };
        tokio::select! { biased;
            _ = time::sleep_until(deadline) => Err(io::ErrorKind::TimedOut.into()),
            result = scope_changed => result,
            result = work => {
                self.check_scope(scope)?;
                result
            }
        }
    }

    pub(super) async fn initiate(
        &self,
        deadline: Instant,
        cancel: &CancellationToken,
        kind: StreamKind,
    ) -> io::Result<MemberUdpPlan> {
        self.initiate_on(deadline, cancel, kind, None).await
    }

    async fn initiate_on(
        &self,
        deadline: Instant,
        cancel: &CancellationToken,
        kind: StreamKind,
        exchange: Option<ControlExchange>,
    ) -> io::Result<MemberUdpPlan> {
        let scope = self.round_scope(self.scope()?)?;
        let deadline = deadline.min(Instant::now() + self.config.round_timeout);
        let traverse = kind != StreamKind::UdpPlan;
        let priority = if kind == StreamKind::UdpMaintenance {
            Priority::Maintenance
        } else {
            Priority::Business
        };
        if cancel.is_cancelled() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let slot = self.owner.slot.reserve(
            &scope.authorization,
            self.connection.stable_id(),
            true,
            priority,
            deadline,
            traverse,
        )?;
        let work = async {
            let mut running = None;
            #[cfg(feature = "miopunch-test-hooks")]
            {
                let barrier = self
                    .owner
                    .reservation_barrier
                    .lock()
                    .expect("UDP reservation barrier")
                    .take();
                if let Some(barrier) = barrier {
                    self.guarded(deadline, &scope, async {
                        barrier.wait().await;
                        Ok(())
                    })
                    .await?;
                }
            }
            let stream = if let Some(exchange) = exchange {
                exchange
            } else {
                self.guarded(deadline, &scope, async {
                    let (mut send, recv) =
                        self.connection.open_bi().await.map_err(io::Error::other)?;
                    send.write_all(&[kind as u8])
                        .await
                        .map_err(io::Error::other)?;
                    Ok(ControlExchange::from_streams(send, recv))
                })
                .await?
            };
            let result = async {
                let plan = self
                    .guarded(
                        deadline,
                        &scope,
                        self.offer(&stream, &scope, deadline, kind, &slot, &mut running),
                    )
                    .await?;
                self.publish(&plan);
                if traverse {
                    self.execute(&stream, &plan, None, &slot, kind).await?;
                }
                Ok(plan)
            }
            .await;
            if let Err(error) = &result {
                let _ = time::timeout_at(deadline, report_error(&stream, error)).await;
            }
            result
        };
        tokio::select! { biased;
            _ = cancel.cancelled() => Err(io::ErrorKind::Interrupted.into()),
            _ = slot.yielded().cancelled() => Err(busy()),
            result = work => result,
        }
    }

    fn control_timeout(&self) -> Duration {
        CONTROL_WAIT.min(self.config.round_timeout)
    }

    pub(super) async fn respond(
        &self,
        stream: ControlExchange,
        activity: &Activity,
        kind: StreamKind,
    ) {
        let _ = self
            .respond_until(
                stream,
                Some(activity),
                kind,
                Instant::now() + self.config.round_timeout,
            )
            .await;
    }

    async fn respond_until(
        &self,
        stream: ControlExchange,
        activity: Option<&Activity>,
        kind: StreamKind,
        deadline: Instant,
    ) -> io::Result<()> {
        let started = Instant::now();
        let deadline = deadline.min(started + self.config.round_timeout);
        let traverse = kind != StreamKind::UdpPlan;
        let priority = if kind == StreamKind::UdpMaintenance {
            Priority::Maintenance
        } else {
            Priority::Business
        };
        let work = async {
            let hello = Hello::decode(
                &time::timeout_at(
                    deadline.min(started + self.control_timeout()),
                    receive(&stream),
                )
                .await??,
            )?;
            let scope = self.scope()?;
            hello.check(&scope)?;
            let scope = self.round_scope(scope)?;
            let mut deadline = deadline.min(started + hello.budget);
            if priority == Priority::Maintenance {
                let since = activity.and_then(Activity::idle_since).ok_or_else(busy)?;
                match self.retention {
                    super::MemberRetention::IdleTimeout(timeout) => {
                        deadline = deadline.min(since + timeout)
                    }
                    super::MemberRetention::Standby(_)
                        if !activity.is_some_and(|a| *a.path_activity.borrow()) =>
                    {
                        return Err(busy());
                    }
                    _ => {}
                }
            }
            if kind.automatically_improves_path()
                && (!self.can_maintain() || !*self.path_work.needed.borrow())
            {
                return Err(busy());
            }
            let slot = self.owner.slot.reserve(
                &scope.authorization,
                self.connection.stable_id(),
                false,
                priority,
                deadline,
                traverse,
            )?;
            let work = async {
                let mut running = None;
                let (plan, recommendation) = self
                    .guarded(
                        deadline,
                        &scope,
                        self.choose(&stream, &scope, deadline, kind, &slot, &mut running),
                    )
                    .await?;
                self.publish(&plan);
                if traverse {
                    self.execute(&stream, &plan, recommendation, &slot, kind)
                        .await?;
                }
                Ok(())
            };
            tokio::select! { biased;
                _ = slot.yielded().cancelled() => Err(busy()),
                result = work => result,
            }
        };
        let result = work.await;
        if let Err(error) = &result {
            let _ = time::timeout_at(deadline, report_error(&stream, error)).await;
        }
        result
    }

    fn start_probes(&self, kind: StreamKind) -> io::Result<Option<RunningPath<'_>>> {
        if kind == StreamKind::UdpPlan {
            return Ok(None);
        }
        // The protected OBSERVE agreement has already committed this round.
        // QNT may clear its request while refreshing candidates or finding TCP;
        // that changes eligibility for a new round, not this round's lifetime.
        let mut running = self.path_work.running.lock().expect("member UDP path work");
        if *running == 0 {
            self.connection.set_nat_traversal_probes_paused(true);
        }
        *running += 1;
        Ok(Some(RunningPath {
            work: &self.path_work,
            connection: &self.connection,
        }))
    }

    fn publish(&self, plan: &MemberUdpPlan) {
        *self.latest.lock().expect("member UDP plan") = Some(plan.clone());
    }

    async fn offer<'a>(
        &'a self,
        stream: &ControlExchange,
        scope: &RoundScope,
        deadline: Instant,
        kind: StreamKind,
        slot: &Reservation<'_>,
        running: &mut Option<RunningPath<'a>>,
    ) -> io::Result<MemberUdpPlan> {
        send(
            stream,
            &Hello::new(&scope.authorization, remaining(deadline)?)?.encode(),
        )
        .await?;
        let challenge = receive(stream).await?;
        let round = Round::decode(&challenge, wire::CHALLENGE)?;
        slot.commit()?;
        send(stream, &round.message(wire::OBSERVE)).await?;
        round.check(&receive(stream).await?, wire::OBSERVE)?;
        *running = self.start_probes(kind)?;
        let traverse = kind != StreamKind::UdpPlan;
        let (local, proposal) = tokio::try_join!(
            async {
                let local = self.observe(scope, traverse, slot).await?;
                local.check()?;
                self.check_scope(scope)?;
                send(stream, &round.offer(&local.wire)).await?;
                Ok::<_, io::Error>(local)
            },
            async { round.proposal(&receive(stream).await?) },
        )?;
        let (local_phase, peer_phase, fallback, disabled) =
            phases(proposal.id, &proposal.evidence, &local.wire, false)?;
        if proposal.packet_limits != packet_limits(&peer_phase, &local_phase) {
            return Err(invalid("inconsistent UDP packet limits"));
        }
        if proposal.budget > phase_window([&local_phase, &peer_phase]) {
            return Err(invalid("UDP plan exceeds the legacy phase window"));
        }
        self.check_limits(&local_phase)?;
        let bound = deadline.min(local.expires_at());
        let accepted =
            Duration::from_millis(proposal.budget.min(remaining(bound)?).as_millis() as u64);
        check_phase_time(accepted, [&local_phase, &peer_phase])?;
        local.check()?;
        self.check_scope(scope)?;
        send(stream, &round.accept(accepted)?).await?;
        // Send FIN without resetting bytes the peer has yet to read.
        if !traverse {
            stream.finish_send().await?;
        }
        round.check(&receive(stream).await?, wire::COMMITTED)?;
        check_phase_time(accepted.min(remaining(bound)?), [&local_phase, &peer_phase])?;
        local.check()?;
        Ok(MemberUdpPlan {
            round: round.0,
            id: proposal.id,
            fallback,
            disabled,
            local_phase,
            peer_phase,
            execution_budget: accepted,
            deadline: bound.min(Instant::now() + accepted),
            coordinator: false,
            evidence: local,
            scope: scope.clone(),
        })
    }

    async fn choose<'a>(
        &'a self,
        stream: &ControlExchange,
        scope: &RoundScope,
        deadline: Instant,
        kind: StreamKind,
        slot: &Reservation<'_>,
        running: &mut Option<RunningPath<'a>>,
    ) -> io::Result<(MemberUdpPlan, Option<Recommendation>)> {
        let round = Round(rand::random());
        send(stream, &round.message(wire::CHALLENGE)).await?;
        round.check(&receive(stream).await?, wire::OBSERVE)?;
        slot.commit()?;
        *running = self.start_probes(kind)?;
        send(stream, &round.message(wire::OBSERVE)).await?;
        let traverse = kind != StreamKind::UdpPlan;
        // Dropping or cancelling the visitor's stream stops even a pending observation.
        let (local, remote) = tokio::try_join!(self.observe(scope, traverse, slot), async {
            round.evidence(&receive(stream).await?)
        },)?;
        local.check()?;
        self.check_scope(scope)?;
        let (plan, recommendation) = make_plan(
            &mut self.owner.analyzer.lock().expect("member UDP analyzer"),
            scope.authorization.clone(),
            &local.wire,
            &remote,
        );
        let (local_phase, peer_phase, fallback, disabled) =
            phases(plan.id, &local.wire, &remote, true)?;
        self.check_limits(&local_phase)?;
        let bound = deadline.min(local.expires_at());
        let budget = remaining(bound)?.min(phase_window([&local_phase, &peer_phase]));
        check_phase_time(budget, [&local_phase, &peer_phase])?;
        send(
            stream,
            &round.plan(
                &local.wire,
                plan.id,
                packet_limits(&local_phase, &peer_phase),
                budget,
            )?,
        )
        .await?;
        let accepted = round.accepted(&receive(stream).await?)?;
        if accepted > budget {
            return Err(invalid("UDP plan acknowledgement exceeds proposed time"));
        }
        check_phase_time(accepted.min(remaining(bound)?), [&local_phase, &peer_phase])?;
        local.check()?;
        self.check_scope(scope)?;
        let deadline = bound.min(Instant::now() + accepted);
        send(stream, &round.message(wire::COMMITTED)).await?;
        if !traverse {
            stream.finish_send().await?;
        }
        Ok((
            MemberUdpPlan {
                round: round.0,
                id: plan.id,
                fallback,
                disabled,
                local_phase,
                peer_phase,
                execution_budget: accepted,
                deadline,
                coordinator: true,
                evidence: local,
                scope: scope.clone(),
            },
            recommendation,
        ))
    }

    fn check_limits(&self, phase: &PhasePlan) -> io::Result<()> {
        if packet_limit(phase) > self.config.peer_datagrams {
            return Err(busy());
        }
        Ok(())
    }

    fn allows(&self, url: &RelayUrl) -> bool {
        self.endpoint
            .inner
            .static_config
            .relay_resource_policy
            .allows(url, crate::endpoint::RelayResourceUse::Observation)
    }

    fn sources(&self) -> io::Result<Sources> {
        let qad = RelayMap::from_iter(
            self.config
                .qad
                .relays::<Vec<_>>()
                .into_iter()
                .filter(|relay| self.allows(&relay.url)),
        );
        let stun: Vec<_> = self
            .config
            .stun
            .iter()
            .filter(|url| self.allows(url))
            .cloned()
            .collect();
        let count = qad.len() + stun.len();
        if count == 0 && (!self.config.qad.is_empty() || !self.config.stun.is_empty()) {
            return Err(denied("no configured UDP observation source is permitted"));
        }
        if count > self.config.observations.min(MAX_OBSERVATIONS) || self.config.peer_datagrams == 0
        {
            return Err(busy());
        }
        Ok(Sources { qad, stun })
    }

    async fn observe(
        &self,
        scope: &RoundScope,
        traverse: bool,
        slot: &Reservation<'_>,
    ) -> io::Result<Evidence> {
        let sources = &scope.sources;
        let on_work = || {
            if traverse {
                slot.spend(&scope.generation, self.config.round_interval);
            }
        };
        let qad_submission = (traverse && !sources.qad.is_empty())
            .then(|| slot.qad_submission(scope.generation.clone(), self.config.round_interval));
        let endpoint = &self.endpoint;
        let sequence = match endpoint
            .inner
            .observe_udp_mapping_round(
                &sources.qad,
                &sources.stun,
                &on_work,
                qad_submission.as_ref(),
            )
            .await
        {
            Ok(sequence) => Some(sequence),
            Err(error) if error.kind() == io::ErrorKind::Unsupported => {
                let ordered_qad = async {
                    if sources.qad.len() < 2 {
                        return Ok(None);
                    }
                    ordered(
                        endpoint
                            .inner
                            .observe_udp_mapping_sequence(&sources.qad, qad_submission.as_ref())
                            .await,
                    )
                };
                match ordered_qad.await? {
                    Some(sequence) => Some(sequence),
                    None if !sources.stun.is_empty() => ordered(
                        endpoint
                            .inner
                            .observe_udp_stun_mapping_sequence(&sources.stun, &on_work)
                            .await,
                    )?,
                    None => None,
                }
            }
            Err(error) => ordered(Err(error))?,
        };
        let (mut wire, observed) = match sequence.as_ref() {
            Some(sequence) => SnapshotWire::from_samples(sequence, 1)?,
            None => (
                SnapshotWire::existing(endpoint.udp_observation().get())?,
                StdInstant::now(),
            ),
        };
        wire.assisted = endpoint.udp_local_ipv4_candidates();
        wire.validate()?;
        tracing::debug!(
            stable_id = self.connection.stable_id(),
            samples = sequence
                .as_ref()
                .map_or(0, |sequence| sequence.samples.len()),
            "member UDP observation finished"
        );
        Ok(Evidence {
            wire,
            sequence,
            observed,
        })
    }
}

struct RunningPath<'a> {
    work: &'a MemberUdpPathWork,
    connection: &'a Connection,
}

impl Drop for RunningPath<'_> {
    fn drop(&mut self) {
        let mut running = self.work.running.lock().expect("member UDP path work");
        *running -= 1;
        if *running == 0 {
            self.connection.set_nat_traversal_probes_paused(false);
        }
    }
}

fn ordered(result: io::Result<UdpMappingSequence>) -> io::Result<Option<UdpMappingSequence>> {
    match result {
        Ok(sequence) => Ok(Some(sequence)),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::PermissionDenied | io::ErrorKind::Interrupted
            ) =>
        {
            Err(error)
        }
        Err(_) => Ok(None),
    }
}

#[derive(Debug, Clone)]
struct RoundScope {
    authorization: LearningScope,
    generation: ObservationValidity,
    sources: Sources,
}

#[derive(Debug, Clone)]
struct Sources {
    qad: RelayMap,
    stun: Vec<RelayUrl>,
}

#[derive(Debug, Clone)]
struct Evidence {
    wire: SnapshotWire,
    sequence: Option<UdpMappingSequence>,
    observed: StdInstant,
}

impl Evidence {
    fn expires_at(&self) -> Instant {
        let end = self.observed + SNAPSHOT_LIFETIME;
        self.sequence
            .as_ref()
            .and_then(UdpMappingSequence::traversal_deadline)
            .map_or(end, |sample_end| end.min(sample_end.into()))
            .into()
    }

    fn check(&self) -> io::Result<()> {
        remaining(self.expires_at())?;
        if self
            .sequence
            .as_ref()
            .is_some_and(|sequence| !sequence.is_current_generation())
        {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "UDP observations changed",
            ));
        }
        Ok(())
    }
}

fn phases(
    id: BehaviorId,
    c_wire: &SnapshotWire,
    v_wire: &SnapshotWire,
    coordinator: bool,
) -> io::Result<(PhasePlan, PhasePlan, bool, bool)> {
    let c_feature = c_wire.view().map(|view| view.feature());
    let v_feature = v_wire.view().map(|view| view.feature());
    let (c, v, fallback, disabled) = match (c_feature, v_feature) {
        (Some(c), Some(v)) => {
            if !initial_scores(c, v).iter().any(|entry| entry.id == id) {
                return Err(invalid(
                    "UDP mode and index do not apply to the round observations",
                ));
            }
            let (c, v) = behaviors(id, c, v).ok_or_else(|| invalid("unknown UDP behavior"))?;
            (c, v, false, false)
        }
        _ => {
            let expected = crate::endpoint::nat_plan::basic_plan(c_wire, v_wire);
            if id != expected.id {
                return Err(invalid(
                    "UDP mode and index do not apply to candidate-only evidence",
                ));
            }
            (expected.c, expected.v, true, expected.disabled)
        }
    };
    let c = PhasePlan::new(
        c,
        v.send_delay_ms,
        &v_wire.mapped,
        &v_wire.assisted,
        v_feature.map_or(0, |f| f.ports_difference),
    );
    let v = PhasePlan::new(
        v,
        c.behavior.send_delay_ms,
        &c_wire.mapped,
        &c_wire.assisted,
        c_feature.map_or(0, |f| f.ports_difference),
    );
    let (local, peer) = if coordinator { (c, v) } else { (v, c) };
    Ok((local, peer, fallback, disabled))
}

fn check_phase_time(budget: Duration, phases: [&PhasePlan; 2]) -> io::Result<()> {
    if phases
        .iter()
        .any(|phase| budget <= Duration::from_millis(phase.behavior.send_delay_ms.into()))
    {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "UDP plan delay exceeds remaining round",
        ));
    }
    Ok(())
}

fn phase_window([local, peer]: [&PhasePlan; 2]) -> Duration {
    Duration::from_millis(local.total_budget_ms.max(peer.total_budget_ms).into())
}

fn packet_limit(phase: &PhasePlan) -> usize {
    phase.peer_datagram_bound(usize::from(phase.behavior.listen_random) + 1)
}

fn packet_limits(c: &PhasePlan, v: &PhasePlan) -> [usize; 2] {
    [packet_limit(c), packet_limit(v)]
}

fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| io::ErrorKind::TimedOut.into())
}

async fn send(stream: &ControlExchange, bytes: &[u8]) -> io::Result<()> {
    stream.send_message(WRAP, bytes).await
}

async fn receive(stream: &ControlExchange) -> io::Result<Vec<u8>> {
    let bytes = stream.recv_message(WRAP).await?;
    wire::check_error(&bytes)?;
    Ok(bytes)
}

async fn report_error(stream: &ControlExchange, error: &io::Error) {
    if send(stream, &wire::error(error.kind())).await.is_ok() {
        let _ = stream.finish_send().await;
    }
}

fn denied(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn busy() -> io::Error {
    io::Error::new(
        io::ErrorKind::WouldBlock,
        "enhanced UDP round allowance unavailable",
    )
}
