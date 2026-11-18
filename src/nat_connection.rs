//! Node-owned automatic UDP traversal on an admitted original Connection.
//!
//! Each new round observes the real Endpoint and recommends once. The shared
//! selector and maintenance allowance survive rounds and connections; no NAT
//! label, mode override, or experiment environment is consulted here.

use crate::{
    admission_gate::AdmissionGate,
    control_session::{CommitError, ControlSession, DataSession},
    nat_mapping::{NodeRoundBudget, RoundPermit},
    nat_punch::{self, Attempt, ExecutionStatus, MappingSnapshot},
    nat_selector::Analyzer,
    network_maintenance::MaintenancePolicy,
    session_maintenance::{LocalNatBudgetError, MaintenanceDeferral},
};
use iroh::{RelayMap, Watcher as _, endpoint::ObservationValidity};
use std::{
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::time::Instant as TokioInstant;

fn upgrade_deadline_error() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "unified caller deadline exceeded")
}

fn upgrade_cancelled_error() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "UDP upgrade canceled")
}

fn round_interval(policy: &MaintenancePolicy) -> Duration {
    Duration::from_millis(policy.exploration_interval_ms) * 32
}

/// Local permission and maintenance ceilings. They never shrink a legacy plan.
#[derive(Clone, Debug)]
pub struct UdpPolicy {
    pub enabled: bool,
    pub additional_observations: usize,
    pub peer_datagrams: usize,
    pub max_rounds: u8,
}

impl Default for UdpPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            additional_observations: 64,
            peer_datagrams: crate::nat_scheduler::DEFAULT_LEGACY_PEER_ALLOWANCE,
            max_rounds: 1,
        }
    }
}

/// Reuse one connector per node to share learning and enhanced round admission.
#[derive(Clone)]
pub struct NatConnector {
    observers: Option<RelayMap>,
    stun_observers: Vec<iroh::RelayUrl>,
    analyzer: Arc<Mutex<Analyzer>>,
    budget: NodeRoundBudget,
    enabled: Arc<AtomicBool>,
    resource_enabled: bool,
    max_rounds: u8,
    round_interval: Duration,
}

/// Direct success has no punching recommendation and cannot train its score.
#[derive(Debug)]
pub struct UdpUpgrade {
    pub data: DataSession,
    pub attempt: Option<Attempt>,
    pub direct: Option<std::net::SocketAddr>,
    direct_scope: Option<crate::nat_direct::DirectPath>,
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

impl NatConnector {
    pub fn new(observers: Option<RelayMap>, policy: UdpPolicy) -> Self {
        Self {
            observers,
            stun_observers: Vec::new(),
            analyzer: Arc::new(Mutex::new(Analyzer::new(Instant::now()))),
            budget: NodeRoundBudget::new(
                policy
                    .additional_observations
                    .min(nat_punch::MAX_OBSERVATIONS),
                policy.peer_datagrams,
            ),
            enabled: Arc::new(AtomicBool::new(policy.enabled)),
            resource_enabled: true,
            max_rounds: policy.max_rounds.min(32),
            round_interval: round_interval(&MaintenancePolicy::default()),
        }
    }

    /// Use the node's validated exploration interval for this connector.
    pub fn with_maintenance_policy(mut self, policy: &MaintenancePolicy) -> io::Result<Self> {
        policy.validate()?;
        self.round_interval = round_interval(policy);
        Ok(self)
    }

    pub fn set_udp_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Release);
    }

    /// An endpoint keeps its resource startup snapshot, while node learning,
    /// runtime maintenance permission and round admission survive new binds.
    pub(crate) fn for_endpoint(
        &self,
        observers: Option<RelayMap>,
        stun_observers: Vec<iroh::RelayUrl>,
        enabled: bool,
        maintenance: &MaintenancePolicy,
    ) -> io::Result<Self> {
        Self {
            observers,
            stun_observers,
            resource_enabled: enabled,
            ..self.clone()
        }
        .with_maintenance_policy(maintenance)
    }

    pub(crate) fn udp_enabled(&self) -> bool {
        self.resource_enabled && self.enabled.load(Ordering::Acquire)
    }

    pub(crate) fn round_wait_until(
        &self,
        now: n0_future::time::Instant,
    ) -> Option<n0_future::time::Instant> {
        self.budget.waiting_until(now)
    }

    fn check(&self, control: &ControlSession, gate: &AdmissionGate) -> io::Result<()> {
        if !self.udp_enabled() {
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
                || control.exchange_control().close_reason().is_some()
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "NAT connection scope changed",
                ));
            }
            Ok(())
        };
        let polling = async {
            loop {
                check()?;
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            #[allow(unreachable_code)]
            Err::<T, io::Error>(io::Error::other("scope guard stopped"))
        };
        check()?;
        tokio::select! {
            result = operation => { let value = result?; check()?; Ok(value) },
            result = polling => result,
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
            .observers
            .as_ref()
            .map(|map| {
                RelayMap::from_iter(
                    map.relays::<Vec<_>>()
                        .into_iter()
                        .filter(|relay| allowed(&relay.url)),
                )
            })
            .filter(|map| map.len() >= 2);
        let stun = self
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
        let result = if self.max_rounds == 0 {
            Err(LocalNatBudgetError::Rounds.into())
        } else {
            self.budget.acquire(
                generation,
                TokioInstant::now,
                deadline,
                self.round_interval,
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
                "peerLimit":self.budget.peer_limit(),
                "occupied":self.budget.is_occupied(),
                "nextDueInMs":next_due.map(|due|due.saturating_duration_since(now).as_millis())
            }),
        );
        result
    }

    /// Acquire a fresh ordered mapping sequence only after current admission.
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
        let qad = if let Some(observers) = &observers.qad {
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
                    let snapshot = Self::mapping_snapshot(sequence);
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
                    let snapshot = Self::mapping_snapshot(sequence);
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
        let snapshot = snapshot.with_assisted(endpoint.udp_local_ipv4_candidates())?;
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
    ) -> io::Result<MappingSnapshot> {
        let snapshot = MappingSnapshot::from_sequence(sequence.clone())?;
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
            let agreement = nat_punch::begin_round(control, gate, true).await?;
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
        let permit = snapshot.claim_round(&self.budget, &generation, || {
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
        let operation =
            nat_punch::coordinate_round(control, gate, snapshot, &self.analyzer, agreement, permit);
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
        let mut permit = attempt.take_round(&self.budget)?;
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
        log.phase("punch", serde_json::json!({"datagramLimit":bound}))
            .run(async {
                let status = attempt
                    .execute_checked(control, gate, &allowance, &enabled)
                    .await?;
                if status != ExecutionStatus::CompleteExecution {
                    return Err(io::Error::other(format!("NAT plan {status:?}")));
                }
                Ok(())
            })
            .await?;
        self.check(control, gate)?;
        self.guard(control, gate, attempt.prepare_data_path(control, gate))
            .await?;
        self.check(control, gate)
    }

    /// Bounded retries create fresh rounds and re-Analyze. Retryable errors
    /// return the original restricted session with its queued business intact.
    pub async fn upgrade(
        &self,
        control: ControlSession,
        gate: &AdmissionGate,
    ) -> Result<UdpUpgrade, CommitError> {
        let deadline = crate::connection_budget::deadline(control.endpoint().caller_deadline());
        let cancellation = control.endpoint().attempt_cancellation();
        let cancelled = || async {
            match &cancellation {
                Some(token) => token.cancelled().await,
                None => std::future::pending().await,
            }
        };
        let prepared = tokio::select! {
            biased;
            _ = cancelled() => Err(upgrade_cancelled_error()),
            result = self.prepare_upgrade(&control, gate, deadline) => result,
        };
        let prepared = match prepared {
            Ok(prepared) => prepared,
            Err(error) if control.is_closed() => return Err(CommitError::Terminal { error }),
            Err(error) => {
                return Err(CommitError::Retryable {
                    error,
                    session: control,
                });
            }
        };
        if TokioInstant::now() >= TokioInstant::from(deadline) {
            return Err(CommitError::Retryable {
                error: upgrade_deadline_error(),
                session: control,
            });
        }
        // Once commitment starts, the handoff owns the restricted session and
        // its queue. Dropping that work closes the original through its guard.
        tokio::select! {
            biased;
            _ = cancelled() => Err(CommitError::Terminal {
                error: upgrade_cancelled_error(),
            }),
            _ = tokio::time::sleep_until(deadline.into()) => Err(CommitError::Terminal {
                error: upgrade_deadline_error(),
            }),
            result = self.commit_prepared(control, gate, prepared) => result,
        }
    }

    /// Preserve the existing commit boundary while a common coordinator owns
    /// capability negotiation, fresh path proof and bilateral readiness.
    async fn commit_prepared(
        &self,
        control: ControlSession,
        gate: &AdmissionGate,
        prepared: PreparedUdpPath,
    ) -> Result<UdpUpgrade, CommitError> {
        match prepared {
            PreparedUdpPath::Punching(attempt) => {
                let data = self.commit_checked(control, gate, Some(&attempt)).await?;
                Ok(UdpUpgrade {
                    data,
                    attempt: Some(attempt),
                    direct: None,
                    direct_scope: None,
                })
            }
            PreparedUdpPath::Direct(mut direct) => {
                let data = self.commit_checked(control, gate, None).await?;
                if !self.udp_enabled()
                    || !direct.generation.is_current()
                    || gate.nat_learning_scope(data.connection()) != Some(direct.scope.clone())
                    || data.connection().stable_id() != direct.connection_id
                {
                    data.connection().close(
                        iroh::endpoint::VarInt::from_u32(0),
                        b"direct commitment invalidated",
                    );
                    return Err(CommitError::Terminal {
                        error: io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "direct commitment invalidated",
                        ),
                    });
                }
                direct.retain();
                Ok(UdpUpgrade {
                    data,
                    attempt: None,
                    direct: Some(direct.remote),
                    direct_scope: Some(direct),
                })
            }
        }
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

    async fn commit_checked(
        &self,
        control: ControlSession,
        gate: &AdmissionGate,
        attempt: Option<&Attempt>,
    ) -> Result<DataSession, CommitError> {
        let connection = control.scope_connection();
        let scope = control.nat_learning_scope(gate);
        let generation = control.endpoint().udp_network_generation();
        let mut cancellation = control.cancellation_guard();
        let permitted = || {
            if !self.udp_enabled()
                || !generation.is_current()
                || scope.is_none()
                || gate.nat_learning_scope(&connection) != scope
                || connection.close_reason().is_some()
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "NAT commitment scope changed",
                ));
            }
            Ok(())
        };
        let polling = async {
            loop {
                if let Err(error) = permitted() {
                    return Err(CommitError::Terminal { error });
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        };
        let operation = async {
            if let Some(attempt) = attempt {
                attempt.commit_winner_path(control, gate).await
            } else {
                control.commit_data_path().await
            }
        };
        let result =
            tokio::select! { result = operation => result, result = polling => return result };
        match result {
            Ok(data) => {
                if let Err(error) = permitted() {
                    return Err(CommitError::Terminal { error });
                }
                cancellation.disarm();
                Ok(data)
            }
            Err(error) => {
                cancellation.disarm();
                Err(error)
            }
        }
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
            if self.max_rounds > 0 {
                "available"
            } else {
                "denied"
            },
            serde_json::json!({"resource":"punch_rounds","limit":self.max_rounds}),
        );
        if self.max_rounds == 0 {
            return Err(LocalNatBudgetError::Rounds.into());
        }
        // Direct QUIC validation has a separate bounded maintenance envelope.
        let direct_allowed = self.budget.peer_limit() >= 64;
        log.event(
            "probe-budget",
            if direct_allowed { "reserved" } else { "denied" },
            serde_json::json!({"resource":"direct_datagrams","requested":64}),
        );
        if !direct_allowed {
            return Err(LocalNatBudgetError::Direct.into());
        }
        let direct = self
            .guard(
                control,
                gate,
                crate::nat_direct::attempt(control, gate, deadline),
            )
            .await?;
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
        for round in 0..self.max_rounds {
            println!(
                "NAT_CONNECTION_ROUND number={} stable_id={}",
                round + 1,
                control.stable_id()
            );
            let mut attempt = self.prepare_round(control, gate, deadline).await?;
            match self.execute(&mut attempt, control, gate).await {
                Ok(()) => return Ok(PreparedUdpPath::Punching(attempt)),
                Err(error) => {
                    if error.kind() != io::ErrorKind::TimedOut || round + 1 == self.max_rounds {
                        return Err(error);
                    }
                }
            }
        }
        Err(io::Error::other("NAT round allowance exhausted"))
    }

    fn confirm_success(
        &self,
        attempt: &mut Attempt,
        data: &DataSession,
        gate: &AdmissionGate,
    ) -> bool {
        self.udp_enabled() && attempt.confirm_success(data, gate, &self.analyzer)
    }

    /// Real authorized bytes and an exact echo over the committed original
    /// data path precede the one-time score update and retained-winner ownership.
    pub async fn exchange_business(
        &self,
        attempt: &mut Attempt,
        data: &DataSession,
        gate: &AdmissionGate,
        payload: &[u8],
    ) -> io::Result<Vec<u8>> {
        // Check before polling either IO future; select's polling order must
        // not create a stream when permission was already withdrawn.
        if !self.udp_enabled() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "UDP disabled during business confirmation",
            ));
        }
        let disabled = async {
            loop {
                if !self.udp_enabled() {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "UDP disabled during business confirmation",
                    ));
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        };
        let exchange =
            attempt.guard_committed(data, gate, async { exchange_payload(data, payload).await });
        let received = tokio::select! { result = exchange => result, result = disabled => result }?;
        let learned = self.confirm_success(attempt, data, gate);
        println!(
            "LEGACY_RESULT outcome=confirmed_business learned={learned} score_delta={} stable_id={}",
            if learned { 2 } else { 0 },
            data.connection().stable_id()
        );
        Ok(received)
    }
    /// Direct and punching results both confirm bytes on their captured
    /// original scope; direct results never receive a legacy score update.
    pub async fn exchange_upgrade_business(
        &self,
        upgrade: &mut UdpUpgrade,
        gate: &AdmissionGate,
        payload: &[u8],
    ) -> io::Result<Vec<u8>> {
        if let Some(attempt) = upgrade.attempt.as_mut() {
            return self
                .exchange_business(attempt, &upgrade.data, gate, payload)
                .await;
        }
        let direct = upgrade
            .direct_scope
            .as_ref()
            .ok_or_else(|| io::Error::other("missing direct scope"))?;
        let check = || {
            if !self.udp_enabled()
                || !direct.generation.is_current()
                || upgrade.data.connection().stable_id() != direct.connection_id
                || gate.nat_learning_scope(upgrade.data.connection()).as_ref()
                    != Some(&direct.scope)
                || upgrade.data.connection().close_reason().is_some()
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "direct business scope changed",
                ));
            }
            Ok(())
        };
        let polling = async {
            loop {
                check()?;
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            #[allow(unreachable_code)]
            Err::<Vec<u8>, io::Error>(io::Error::other("direct guard stopped"))
        };
        check()?;
        let result = tokio::select! {
            result = tokio::time::timeout(std::time::Duration::from_secs(5), exchange_payload(&upgrade.data, payload)) =>
                result.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "direct business confirmation elapsed"))?,
            result = polling => result,
        }?;
        check()?;
        Ok(result)
    }
}

async fn exchange_payload(data: &DataSession, payload: &[u8]) -> io::Result<Vec<u8>> {
    if payload.is_empty() || payload.len() > 16 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid business payload",
        ));
    }
    let outbound = async {
        let (mut send, mut recv) = data.connection().open_bi().await?;
        send.write_all(payload).await?;
        send.finish()?;
        if recv
            .read_to_end(16 * 1024)
            .await
            .map_err(io::Error::other)?
            != payload
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "business echo mismatch",
            ));
        }
        Ok::<(), io::Error>(())
    };
    let inbound = async {
        let (mut send, mut recv) = data.connection().accept_bi().await?;
        let received = recv
            .read_to_end(16 * 1024)
            .await
            .map_err(io::Error::other)?;
        if received.is_empty() {
            return Err(io::Error::other("empty peer business"));
        }
        send.write_all(&received).await?;
        send.finish()?;
        Ok::<_, io::Error>(received)
    };
    let (_, received) = tokio::try_join!(outbound, inbound)?;
    Ok(received)
}
