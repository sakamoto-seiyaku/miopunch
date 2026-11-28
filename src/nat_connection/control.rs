//! UDP traversal for a restricted control connection and its original data handoff.
mod upgrade;
pub use upgrade::UdpUpgrade;

use super::{NatConnector, UdpPolicy};
use crate::{
    admission_gate::AdmissionGate,
    control_session::ControlSession,
    nat_mapping::{NodeRoundBudget, RoundPermit},
    nat_punch::{self, Attempt, ExecutionStatus, MappingSnapshot},
    nat_selector::Analyzer,
    network_maintenance::MaintenancePolicy,
    session_maintenance::{LocalNatBudgetError, MaintenanceDeferral},
};
use iroh::{RelayMap, Watcher as _, endpoint::ObservationValidity};
use std::{
    io,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
use tokio::time::Instant as TokioInstant;

async fn wait_for_invalidation(check: impl Fn() -> io::Result<()>) -> io::Error {
    loop {
        if let Err(error) = check() {
            return error;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn upgrade_deadline_error() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "unified caller deadline exceeded")
}

fn upgrade_cancelled_error() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "UDP upgrade canceled")
}

/// Learning and enhanced-round admission are shared by every control endpoint on the node.
pub(crate) struct RoundState {
    analyzer: Arc<Mutex<Analyzer>>,
    budget: NodeRoundBudget,
}

/// Executes the control-session wire protocol. A direct path needs no enhanced-round state.
#[derive(Clone)]
pub struct ControlUdpTraversal {
    config: Arc<NatConnector>,
    rounds: Arc<OnceLock<RoundState>>,
}

#[derive(Debug)]
pub(crate) enum PreparedUdpPath {
    Direct(crate::nat_direct::DirectPath),
    Punching(Attempt),
}

struct ExecutingAttempt<'a> {
    attempt: &'a mut Attempt,
    completed: bool,
}

struct RoundObservers {
    qad: Option<RelayMap>,
    stun: Vec<iroh::RelayUrl>,
}

impl RoundObservers {
    fn cost(&self) -> usize {
        self.qad.as_ref().map_or(0, RelayMap::len) + self.stun.len()
    }
}

impl Drop for ExecutingAttempt<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.attempt.finish_failure();
        }
    }
}

impl ControlUdpTraversal {
    /// Standalone owner for a probe. Clones share its permit ledger and learning.
    pub fn new(observers: Option<RelayMap>, policy: UdpPolicy) -> Self {
        Self::for_node(
            Arc::new(NatConnector::new(observers, policy)),
            Arc::default(),
        )
    }

    pub(crate) fn for_node(config: Arc<NatConnector>, rounds: Arc<OnceLock<RoundState>>) -> Self {
        Self { config, rounds }
    }

    pub fn config(&self) -> &NatConnector {
        &self.config
    }

    pub fn with_maintenance_policy(mut self, policy: &MaintenancePolicy) -> io::Result<Self> {
        policy.validate()?;
        Arc::make_mut(&mut self.config).round_interval = super::round_interval(policy);
        Ok(self)
    }

    fn rounds(&self) -> &RoundState {
        self.rounds.get_or_init(|| RoundState {
            analyzer: Arc::new(Mutex::new(Analyzer::new(Instant::now()))),
            budget: NodeRoundBudget::new(self.config.observation_limit, self.config.peer_datagrams),
        })
    }

    pub(crate) fn round_wait_until(
        &self,
        now: n0_future::time::Instant,
    ) -> Option<n0_future::time::Instant> {
        self.rounds.get()?.budget.waiting_until(now)
    }

    fn check(&self, control: &ControlSession, gate: &AdmissionGate) -> io::Result<()> {
        if !self.config.udp_enabled() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "UDP resource disabled",
            ));
        }
        if !control.may_upgrade_member_path(gate) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "member path upgrade denied",
            ));
        }
        Ok(())
    }

    async fn guard<T>(
        &self,
        control: &ControlSession,
        gate: &AdmissionGate,
        operation: impl std::future::Future<Output = io::Result<T>>,
    ) -> io::Result<T> {
        let scope = control.nat_learning_scope(gate);
        let generation = control.endpoint().udp_network_generation();
        let check = || {
            self.check(control, gate)?;
            if !generation.is_current()
                || control.nat_learning_scope(gate) != scope
                || control.is_closed()
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "NAT connection scope changed",
                ));
            }
            Ok(())
        };
        check()?;
        tokio::select! {
            result = operation => { let value = result?; check()?; Ok(value) },
            error = wait_for_invalidation(check) => Err(error),
        }
    }

    async fn within_round<T>(
        &self,
        control: &ControlSession,
        deadline: TokioInstant,
        operation: impl std::future::Future<Output = io::Result<T>>,
    ) -> io::Result<T> {
        let cancellation = control.exchange_control().attempt_cancellation();
        let cancelled = async {
            match cancellation {
                Some(token) => token.cancelled().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            biased;
            _ = cancelled => Err(upgrade_cancelled_error()),
            _ = tokio::time::sleep_until(deadline) => Err(upgrade_deadline_error()),
            result = operation => result,
        }
    }

    fn round_observers(&self, control: &ControlSession) -> RoundObservers {
        let allowed = |url: &iroh::RelayUrl| control.endpoint().allows_udp_observation(url);
        let qad = self
            .config
            .observers
            .as_ref()
            .map(|map| {
                RelayMap::from_iter(
                    map.relays::<Vec<_>>()
                        .into_iter()
                        .filter(|relay| allowed(&relay.url)),
                )
            })
            .filter(|map| !map.is_empty());
        let stun = self
            .config
            .stun_observers
            .iter()
            .filter(|url| allowed(url))
            .cloned()
            .collect();
        RoundObservers { qad, stun }
    }

    fn check_round(
        &self,
        control: &ControlSession,
        gate: &AdmissionGate,
        permit: &RoundPermit,
    ) -> io::Result<()> {
        self.check(control, gate)?;
        permit.check_generation(&control.endpoint().udp_network_generation())
    }

    fn acquire_round(
        &self,
        control: &ControlSession,
        generation: ObservationValidity,
        deadline: TokioInstant,
        observation_cost: usize,
    ) -> io::Result<RoundPermit> {
        let now = TokioInstant::now();
        let result = if self.config.max_rounds == 0 {
            Err(LocalNatBudgetError::Rounds.into())
        } else {
            self.rounds().budget.acquire(
                generation,
                TokioInstant::now,
                deadline,
                self.config.round_interval,
                observation_cost,
            )
        };
        let (status, next_due) = match &result {
            Ok(permit) => ("reserved", Some(permit.next_due)),
            Err(error) => match MaintenanceDeferral::from_error(error) {
                Some(MaintenanceDeferral::LocalBudget { next_due }) => {
                    ("deferred", Some(*next_due))
                }
                _ => ("denied", None),
            },
        };
        control.diagnostic().event(
            "probe-budget",
            status,
            serde_json::json!({
                "resource":"enhanced_round",
                "observationRequested":observation_cost,
                "observationReserved":result.as_ref().ok().map(|permit|permit.observations),
                "peerLimit":self.config.peer_datagrams,
                "occupied":self.rounds.get().is_some_and(|state| state.budget.is_occupied()),
                "nextDueInMs":next_due.map(|due|due.saturating_duration_since(now).as_millis())
            }),
        );
        result
    }

    /// Acquire fresh mapping evidence only after current admission.
    pub async fn observe(
        &self,
        control: &ControlSession,
        gate: &AdmissionGate,
    ) -> io::Result<MappingSnapshot> {
        self.check(control, gate)?;
        let generation = control.endpoint().udp_network_generation();
        let deadline =
            crate::connection_budget::deadline(control.exchange_control().caller_deadline());
        let observers = self.round_observers(control);
        let permit = self.acquire_round(control, generation, deadline, observers.cost())?;
        let snapshot = self
            .within_round(
                control,
                deadline,
                self.guard(
                    control,
                    gate,
                    self.observe_round(control, gate, &permit, observers),
                ),
            )
            .await?;
        Ok(snapshot.with_round(permit))
    }

    async fn observe_round(
        &self,
        control: &ControlSession,
        gate: &AdmissionGate,
        permit: &RoundPermit,
        observers: RoundObservers,
    ) -> io::Result<MappingSnapshot> {
        let log = control.diagnostic();
        let endpoint = control.endpoint();
        let existing_mapping = || {
            let phase = log.phase(
                "observation",
                serde_json::json!({"method":"existing_mapping"}),
            );
            let result = MappingSnapshot::fallback_generation(
                endpoint.udp_network_generation(),
                endpoint.udp_observation().get(),
            );
            phase.finish(if result.is_ok() {
                "succeeded"
            } else {
                "failed"
            });
            result
        };
        self.check_round(control, gate, permit)?;
        let none = RelayMap::empty();
        let phase = log.phase("observation", serde_json::json!({"method":"mixed"}));
        match endpoint
            .observe_udp_mapping_round(observers.qad.as_ref().unwrap_or(&none), &observers.stun)
            .await
        {
            Ok(sequence) => {
                self.check_round(control, gate, permit)?;
                let samples = sequence.samples.len();
                let stun_samples = sequence
                    .samples
                    .iter()
                    .filter(|sample| sample.observer.starts_with("stun://"))
                    .count();
                let snapshot =
                    Self::mapping_snapshot(sequence, MappingSnapshot::from_observation_round);
                phase.finish_with(
                    if snapshot.is_ok() {
                        "succeeded"
                    } else {
                        "failed"
                    },
                    serde_json::json!({"samples": samples, "stunSamples": stun_samples,
                        "qadSamples": samples - stun_samples}),
                );
                return self.finish_observation(control, gate, snapshot?);
            }
            Err(error) if error.kind() == io::ErrorKind::Unsupported => {
                phase.finish("skipped");
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::PermissionDenied | io::ErrorKind::Interrupted
                ) =>
            {
                phase.finish("failed");
                return Err(error);
            }
            Err(_) => {
                phase.finish("unavailable");
                self.check_round(control, gate, permit)?;
                return self.finish_observation(control, gate, existing_mapping()?);
            }
        }
        let qad = if let Some(observers) = observers.qad.as_ref().filter(|map| map.len() >= 2) {
            log.event(
                "probe-budget",
                "consumed",
                serde_json::json!({
                    "resource":"qad_observation","requested":observers.len(),
                }),
            );
            let phase = log.phase("observation", serde_json::json!({"method":"qad"}));
            match endpoint.observe_udp_mapping_sequence(observers).await {
                Ok(sequence) => {
                    let samples = sequence.samples.len();
                    let snapshot = Self::mapping_snapshot(sequence, MappingSnapshot::from_sequence);
                    phase.finish_with(
                        if snapshot.is_ok() {
                            "succeeded"
                        } else {
                            "failed"
                        },
                        serde_json::json!({"samples":samples}),
                    );
                    snapshot.ok()
                }
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                    phase.finish("failed");
                    return Err(error);
                }
                Err(error) => {
                    phase.finish("failed");
                    eprintln!("QAD_SEQUENCE_UNAVAILABLE error_kind={:?}", error.kind());
                    None
                }
            }
        } else {
            None
        };
        self.check_round(control, gate, permit)?;
        let snapshot = if let Some(snapshot) = qad {
            snapshot
        } else if !observers.stun.is_empty() {
            log.event(
                "probe-budget",
                "consumed",
                serde_json::json!({
                    "resource":"stun_observation","requested":observers.stun.len(),
                }),
            );
            let phase = log.phase("observation", serde_json::json!({"method":"stun"}));
            match endpoint
                .observe_udp_stun_mapping_sequence(&observers.stun)
                .await
            {
                Ok(sequence) => {
                    let samples = sequence.samples.len();
                    let snapshot = Self::mapping_snapshot(sequence, MappingSnapshot::from_sequence);
                    phase.finish_with(
                        if snapshot.is_ok() {
                            "succeeded"
                        } else {
                            "failed"
                        },
                        serde_json::json!({"samples":samples}),
                    );
                    snapshot?
                }
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                    phase.finish("failed");
                    return Err(error);
                }
                Err(error) => {
                    phase.finish("failed");
                    eprintln!("STUN_SEQUENCE_UNAVAILABLE error_kind={:?}", error.kind());
                    existing_mapping()?
                }
            }
        } else {
            existing_mapping()?
        };
        self.finish_observation(control, gate, snapshot)
    }

    fn finish_observation(
        &self,
        control: &ControlSession,
        gate: &AdmissionGate,
        snapshot: MappingSnapshot,
    ) -> io::Result<MappingSnapshot> {
        let snapshot = snapshot.with_assisted(control.endpoint().udp_local_ipv4_candidates())?;
        self.check(control, gate)?;
        println!(
            "LEGACY_FEATURE feature={:?} socket_generation_current={}",
            snapshot.feature(),
            snapshot.is_current()
        );
        Ok(snapshot)
    }

    fn mapping_snapshot(
        sequence: iroh::endpoint::UdpMappingSequence,
        snapshot: impl FnOnce(iroh::endpoint::UdpMappingSequence) -> io::Result<MappingSnapshot>,
    ) -> io::Result<MappingSnapshot> {
        let snapshot = snapshot(sequence.clone())?;
        for (index, sample) in sequence.samples.iter().enumerate() {
            println!(
                "LEGACY_SAMPLE order={index} destination={} mapped={} observer={}",
                sample.destination, sample.mapped, sample.observer
            );
        }
        Ok(snapshot)
    }

    /// Exactly one Analyze and protected plan exchange for this fresh round.
    pub async fn prepare(
        &self,
        control: &ControlSession,
        gate: &AdmissionGate,
    ) -> io::Result<Attempt> {
        self.prepare_round(
            control,
            gate,
            crate::connection_budget::deadline(control.exchange_control().caller_deadline()),
        )
        .await
    }

    async fn prepare_round(
        &self,
        control: &ControlSession,
        gate: &AdmissionGate,
        deadline: TokioInstant,
    ) -> io::Result<Attempt> {
        self.check(control, gate)?;
        let generation = control.endpoint().udp_network_generation();
        let observers = self.round_observers(control);
        let permit = self.acquire_round(control, generation, deadline, observers.cost());
        let permit = self
            .settle_round_admission(control, gate, deadline, permit)
            .await?;
        let operation = async {
            self.check_round(control, gate, &permit)?;
            let agreement = control
                .diagnostic()
                .phase("punch-round-agreement", serde_json::Value::Null)
                .run(nat_punch::begin_round(control, gate, true))
                .await?;
            let snapshot = self
                .observe_round(control, gate, &permit, observers)
                .await?;
            self.coordinate_round(control, gate, &snapshot, agreement, permit)
                .await
        };
        self.within_round(control, deadline, self.guard(control, gate, operation))
            .await
    }

    async fn settle_round_admission(
        &self,
        control: &ControlSession,
        gate: &AdmissionGate,
        deadline: TokioInstant,
        result: io::Result<RoundPermit>,
    ) -> io::Result<RoundPermit> {
        let error = match result {
            Ok(permit) => return Ok(permit),
            Err(error) => error,
        };
        if MaintenanceDeferral::from_error(&error).is_some()
            || LocalNatBudgetError::is_local_limit(&error)
        {
            let notice = nat_punch::begin_round(control, gate, false);
            if let Err(protocol) = self
                .within_round(control, deadline, self.guard(control, gate, notice))
                .await
                && matches!(
                    protocol.kind(),
                    io::ErrorKind::InvalidData
                        | io::ErrorKind::PermissionDenied
                        | io::ErrorKind::Interrupted
                )
            {
                return Err(protocol);
            }
        }
        Err(error)
    }

    pub async fn coordinate(
        &self,
        control: &ControlSession,
        gate: &AdmissionGate,
        snapshot: &MappingSnapshot,
    ) -> io::Result<Attempt> {
        self.check(control, gate)?;
        let generation = control.endpoint().udp_network_generation();
        let caller =
            crate::connection_budget::deadline(control.exchange_control().caller_deadline());
        let permit = snapshot.claim_round(&self.rounds().budget, &generation, || {
            self.acquire_round(control, generation.clone(), caller, 0)
        });
        let mut permit = self
            .settle_round_admission(control, gate, caller, permit)
            .await?;
        permit.limit_deadline(caller.min(snapshot.expires_at()));
        let deadline = permit.deadline;
        let operation = async {
            self.check_round(control, gate, &permit)?;
            let agreement = nat_punch::begin_round(control, gate, true).await?;
            self.coordinate_round(control, gate, snapshot, agreement, permit)
                .await
        };
        self.within_round(control, deadline, self.guard(control, gate, operation))
            .await
    }

    async fn coordinate_round(
        &self,
        control: &ControlSession,
        gate: &AdmissionGate,
        snapshot: &MappingSnapshot,
        agreement: nat_punch::RoundAgreement,
        permit: RoundPermit,
    ) -> io::Result<Attempt> {
        self.check_round(control, gate, &permit)?;
        let operation = nat_punch::coordinate_round(
            control,
            gate,
            snapshot,
            &self.rounds().analyzer,
            agreement,
            permit,
        );
        let attempt = control
            .diagnostic()
            .phase("punch-plan", serde_json::Value::Null)
            .run(operation)
            .await?;
        println!(
            "LEGACY_SELECTED mode={} index={} c={:?} v={:?} scores_before={:?} round={} stable_id={} execution={:?} fallback={}",
            attempt.plan().id.mode,
            attempt.plan().id.index,
            attempt.plan().c,
            attempt.plan().v,
            attempt.plan().scores_before,
            attempt.round().encoded(),
            control.stable_id(),
            attempt.execution_status(),
            attempt.plan().fallback
        );
        println!(
            "LEGACY_CANDIDATES round={} stable_id={} assisted={:?} mapped={:?}",
            attempt.round().encoded(),
            control.stable_id(),
            attempt.assisted_candidates(),
            attempt.mapped_candidates()
        );
        Ok(attempt)
    }

    /// Execute the entire selected plan and validate its original physical path.
    pub async fn execute(
        &self,
        attempt: &mut Attempt,
        control: &ControlSession,
        gate: &AdmissionGate,
    ) -> io::Result<()> {
        attempt.check_execution(control)?;
        let mut permit = attempt.take_round(&self.rounds().budget)?;
        let mut execution = ExecutingAttempt {
            attempt,
            completed: false,
        };
        let deadline = permit.deadline.min(crate::connection_budget::deadline(
            control.exchange_control().caller_deadline(),
        ));
        let operation = self.execute_round(execution.attempt, control, gate, &mut permit);
        let result = self.within_round(control, deadline, operation).await;
        execution.completed = result.is_ok();
        result
    }

    async fn execute_round(
        &self,
        attempt: &mut Attempt,
        control: &ControlSession,
        gate: &AdmissionGate,
        permit: &mut RoundPermit,
    ) -> io::Result<()> {
        self.check_round(control, gate, permit)?;
        permit.resume()?;
        let log = control.diagnostic();
        let bound = attempt.peer_datagram_bound();
        let allowance = permit.reserve_peer_plan(bound)?;
        log.event(
            "probe-budget",
            "reserved",
            serde_json::json!({"resource":"punch_datagrams","requested":bound,"remaining":permit.peer_remaining()}),
        );
        let enabled = || self.check(control, gate);
        let phase = log.phase("punch", serde_json::json!({"datagramLimit":bound}));
        let result = async {
            let status = attempt
                .execute_checked(control, gate, &allowance, permit.deadline, &enabled)
                .await?;
            if status != ExecutionStatus::CompleteExecution {
                return Err(io::Error::other(format!("NAT plan {status:?}")));
            }
            Ok(())
        }
        .await;
        phase.finish_with(
            if result.is_ok() { "succeeded" } else { "failed" },
            serde_json::json!({"errorKind":result.as_ref().err().map(|error| format!("{:?}", error.kind()))}),
        );
        result?;
        self.check(control, gate)?;
        log.phase("punch-handoff", serde_json::Value::Null)
            .run(self.guard(control, gate, attempt.prepare_data_path(control, gate)))
            .await?;
        self.check(control, gate)
    }

    /// Retain a prepared path inside typed admission, without returning business.
    pub(crate) async fn retain_prepared_path(
        &self,
        control: &ControlSession,
        gate: &AdmissionGate,
        mut prepared: PreparedUdpPath,
        pending_admission: bool,
    ) -> io::Result<()> {
        let mut cancellation = pending_admission.then(|| control.cancellation_guard());
        self.guard(control, gate, async {
            match &prepared {
                PreparedUdpPath::Direct(path) => path.check(control, gate)?,
                PreparedUdpPath::Punching(attempt) => attempt.verify_winner_path(control, gate)?,
            }
            if pending_admission {
                control.commit_control_path().await?;
            }
            match &mut prepared {
                PreparedUdpPath::Direct(path) => path.check(control, gate)?,
                PreparedUdpPath::Punching(attempt) => attempt.retain_control_path(control, gate)?,
            }
            Ok(())
        })
        .await?;
        if let Some(cancellation) = &mut cancellation {
            cancellation.disarm();
        }
        Ok(())
    }

    pub(crate) async fn prepare_upgrade(
        &self,
        control: &ControlSession,
        gate: &AdmissionGate,
        deadline: n0_future::time::Instant,
    ) -> io::Result<PreparedUdpPath> {
        let deadline = TokioInstant::from(deadline);
        if TokioInstant::now() >= deadline {
            return Err(upgrade_deadline_error());
        }
        self.check(control, gate)?;
        let log = control.diagnostic();
        log.event(
            "probe-budget",
            if self.config.max_rounds > 0 {
                "available"
            } else {
                "denied"
            },
            serde_json::json!({"resource":"punch_rounds","limit":self.config.max_rounds}),
        );
        if self.config.max_rounds == 0 {
            return Err(LocalNatBudgetError::Rounds.into());
        }
        // Direct QUIC validation has a separate bounded maintenance envelope.
        let direct_allowed = self.config.peer_datagrams >= 64;
        log.event(
            "probe-budget",
            if direct_allowed { "reserved" } else { "denied" },
            serde_json::json!({"resource":"direct_datagrams","requested":64}),
        );
        if !direct_allowed {
            return Err(LocalNatBudgetError::Direct.into());
        }
        let enhanced_only = cfg!(feature = "nat-lab-test-hooks")
            && std::env::var("MIOPUNCH_NAT_LAB_ENHANCED_ONLY").as_deref() == Ok("1");
        let direct = if enhanced_only {
            println!("NAT_LAB_ENHANCED_ONLY stable_id={}", control.stable_id());
            crate::nat_direct::DirectAttempt::Unavailable {
                more_candidates: false,
            }
        } else {
            self.guard(
                control,
                gate,
                crate::nat_direct::attempt(control, gate, deadline),
            )
            .await?
        };
        let more_candidates = match direct {
            crate::nat_direct::DirectAttempt::Ready(direct) => {
                direct.check(control, gate)?;
                println!(
                    "NAT_DIRECT_SUCCESS stable_id={} target={} punching_recommendations=0",
                    control.stable_id(),
                    direct.remote
                );
                return Ok(PreparedUdpPath::Direct(direct));
            }
            crate::nat_direct::DirectAttempt::Unavailable { more_candidates } => more_candidates,
        };
        gate.with_direct_candidates(control.endpoint(), |progress| {
            progress.active_enhanced_only = Some(!more_candidates);
        });
        self.prepare_enhanced_path(control, gate, deadline)
            .await
            .map_err(|error| {
                if more_candidates
                    && matches!(
                        MaintenanceDeferral::from_error(&error),
                        Some(MaintenanceDeferral::LocalBudget { .. })
                    )
                {
                    MaintenanceDeferral::LocalAlternatives.into()
                } else {
                    error
                }
            })
    }

    async fn prepare_enhanced_path(
        &self,
        control: &ControlSession,
        gate: &AdmissionGate,
        deadline: TokioInstant,
    ) -> io::Result<PreparedUdpPath> {
        for round in 0..self.config.max_rounds {
            println!(
                "NAT_CONNECTION_ROUND number={} stable_id={}",
                round + 1,
                control.stable_id()
            );
            let mut attempt = self.prepare_round(control, gate, deadline).await?;
            match self.execute(&mut attempt, control, gate).await {
                Ok(()) => return Ok(PreparedUdpPath::Punching(attempt)),
                Err(error) => {
                    if error.kind() != io::ErrorKind::TimedOut
                        || round + 1 == self.config.max_rounds
                    {
                        return Err(error);
                    }
                }
            }
        }
        Err(io::Error::other("NAT round allowance exhausted"))
    }
}
