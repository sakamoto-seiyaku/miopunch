//! Enhanced UDP coordination on the protected, already admitted original.

use std::{
    io,
    sync::{Arc, Mutex},
    time::Instant as StdInstant,
};

use iroh_relay::RelayMap;
use n0_watcher::Watcher as _;
use tokio::sync::Semaphore;

use super::{
    CancellationToken, Connection, ControlExchange, Duration, Endpoint, Instant, StreamKind, time,
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
};

mod execute;
mod wire;
pub use execute::MemberUdpTraversalReport;
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
    slot: Semaphore,
}

impl Default for Planner {
    fn default() -> Self {
        Self {
            analyzer: Mutex::new(Analyzer::new(StdInstant::now())),
            slot: Semaphore::new(1),
        }
    }
}

#[derive(Debug)]
pub(super) struct Planning {
    endpoint: Endpoint,
    connection: Connection,
    config: MemberUdpConfig,
    owner: Arc<Planner>,
    latest: Mutex<Option<MemberUdpPlan>>,
    traversal: Mutex<Option<Arc<execute::Execution>>>,
}

impl Planning {
    pub(super) fn new(
        endpoint: &Endpoint,
        connection: &Connection,
        owner: Arc<Planner>,
    ) -> Option<Arc<Self>> {
        let config = endpoint
            .inner
            .static_config
            .admission
            .as_ref()?
            .member_udp_config()?
            .clone();
        Some(Arc::new(Self {
            endpoint: endpoint.clone(),
            connection: connection.clone(),
            config,
            owner,
            latest: Mutex::new(None),
            traversal: Mutex::new(None),
        }))
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
        let scope = self.round_scope(self.scope()?)?;
        let deadline = deadline.min(Instant::now() + self.config.round_timeout);
        let traverse = kind == StreamKind::UdpTraversal;
        let work = async {
            let _slot = self.owner.slot.try_acquire().map_err(|_| busy())?;
            let stream = self
                .guarded(deadline, &scope, async {
                    let (mut send, recv) =
                        self.connection.open_bi().await.map_err(io::Error::other)?;
                    send.write_all(&[kind as u8])
                        .await
                        .map_err(io::Error::other)?;
                    Ok(ControlExchange::from_streams(send, recv))
                })
                .await?;
            let result = async {
                let plan = self
                    .guarded(
                        deadline,
                        &scope,
                        self.offer(&stream, &scope, deadline, traverse),
                    )
                    .await?;
                self.publish(&plan);
                if traverse {
                    self.execute(&stream, &plan, None).await?;
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
            result = work => result,
        }
    }

    fn control_timeout(&self) -> Duration {
        CONTROL_WAIT.min(self.config.round_timeout)
    }

    pub(super) async fn respond(&self, stream: ControlExchange, traverse: bool) {
        let started = Instant::now();
        let work = async {
            let hello =
                Hello::decode(&time::timeout(self.control_timeout(), receive(&stream)).await??)?;
            let scope = self.scope()?;
            hello.check(&scope)?;
            let scope = self.round_scope(scope)?;
            let deadline = started + self.config.round_timeout.min(hello.budget);
            let _slot = self.owner.slot.try_acquire().map_err(|_| busy())?;
            let (plan, recommendation) = self
                .guarded(
                    deadline,
                    &scope,
                    self.choose(&stream, &scope, deadline, traverse),
                )
                .await?;
            self.publish(&plan);
            if traverse {
                self.execute(&stream, &plan, recommendation).await?;
            }
            Ok(())
        };
        if let Err(error) = work.await {
            let _ = time::timeout_at(
                started + self.config.round_timeout,
                report_error(&stream, &error),
            )
            .await;
        }
    }

    fn publish(&self, plan: &MemberUdpPlan) {
        *self.latest.lock().expect("member UDP plan") = Some(plan.clone());
    }

    async fn offer(
        &self,
        stream: &ControlExchange,
        scope: &RoundScope,
        deadline: Instant,
        traverse: bool,
    ) -> io::Result<MemberUdpPlan> {
        send(
            stream,
            &Hello::new(&scope.authorization, remaining(deadline)?)?.encode(),
        )
        .await?;
        let challenge = receive(stream).await?;
        let round = Round::decode(&challenge, wire::CHALLENGE)?;
        let (local, proposal) = tokio::try_join!(
            async {
                let local = self.observe(&scope.sources).await?;
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

    async fn choose(
        &self,
        stream: &ControlExchange,
        scope: &RoundScope,
        deadline: Instant,
        traverse: bool,
    ) -> io::Result<(MemberUdpPlan, Option<Recommendation>)> {
        let round = Round(rand::random());
        send(stream, &round.message(wire::CHALLENGE)).await?;
        // Dropping or cancelling the visitor's stream stops even a pending observation.
        let (local, remote) = tokio::try_join!(self.observe(&scope.sources), async {
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

    async fn observe(&self, sources: &Sources) -> io::Result<Evidence> {
        let endpoint = &self.endpoint;
        let sequence = match endpoint
            .observe_udp_mapping_round(&sources.qad, &sources.stun)
            .await
        {
            Ok(sequence) => Some(sequence),
            Err(error) if error.kind() == io::ErrorKind::Unsupported => {
                let ordered_qad = async {
                    if sources.qad.len() < 2 {
                        return Ok(None);
                    }
                    ordered(endpoint.observe_udp_mapping_sequence(&sources.qad).await)
                };
                match ordered_qad.await? {
                    Some(sequence) => Some(sequence),
                    None if !sources.stun.is_empty() => ordered(
                        endpoint
                            .observe_udp_stun_mapping_sequence(&sources.stun)
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
        Ok(Evidence {
            wire,
            sequence,
            observed,
        })
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
