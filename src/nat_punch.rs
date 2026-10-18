//! Protected legacy-plan exchange and guarded legacy mode0/1/3 execution.
//!
//! The coordinator owns the directed c/v selector: c is the accepting node,
//! v the visitor. One round chooses one behavior. Full legacy execution is a
//! separate adapter; a single prime never claims to implement a complete mode.

use std::{
    collections::HashSet,
    io,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use iroh::endpoint::{
    ObservationValidity, UdpMappingSequence, UdpObservation, UdpObservationFailure,
};
use serde::{Deserialize, Serialize};

use crate::{
    admission_gate::AdmissionGate,
    control_session::{CommitError, ControlSession, DataPathOpen, DataSession},
    nat_mapping::NodeProbeBudget,
    nat_selector::{
        Analyzer, Behavior, BehaviorId, BehaviorScore, FeatureView, LearningScope, Recommendation,
        behaviors, initial_scores,
    },
};

const FRAME_LIMIT: usize = 16 * 1024;
const EXCHANGE_WAIT: Duration = Duration::from_secs(5);
/// Supports the longest old 38-second plan, while bounding retained evidence.
pub const SNAPSHOT_LIFETIME: Duration = Duration::from_secs(60);
const MAX_OBSERVATIONS: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotWire {
    mapped: Vec<SocketAddr>,
    local_ips: Vec<IpAddr>,
    assisted: Vec<SocketAddr>,
}

impl SnapshotWire {
    fn validate(&self) -> io::Result<()> {
        if self.mapped.len() > MAX_OBSERVATIONS
            || self.local_ips.len() > MAX_OBSERVATIONS
            || self.assisted.len() > MAX_OBSERVATIONS
            || self
                .mapped
                .iter()
                .chain(&self.assisted)
                .any(|a| !a.is_ipv4() || a.port() == 0 || a.ip().is_unspecified())
        {
            return Err(invalid("invalid or oversized mapping snapshot"));
        }
        Ok(())
    }

    fn view(&self) -> Option<FeatureView> {
        FeatureView::new(self.mapped.clone(), &self.local_ips)
    }
    fn target(&self) -> Option<SocketAddr> {
        self.assisted
            .first()
            .copied()
            .or_else(|| self.mapped.last().copied())
    }
}

#[derive(Debug, Clone)]
enum Validity {
    Sequence(UdpMappingSequence),
    Fallback { generation: ObservationValidity },
}

/// Local validity is never replaced by a remote claim or a copied timestamp.
#[derive(Debug, Clone)]
pub struct MappingSnapshot {
    wire: SnapshotWire,
    validity: Validity,
    observed_at: Instant,
}

impl MappingSnapshot {
    pub fn from_sequence(sequence: UdpMappingSequence) -> io::Result<Self> {
        let now = Instant::now();
        let mut destinations = HashSet::new();
        if sequence.samples.len() < 2
            || sequence.samples.len() > MAX_OBSERVATIONS
            || !sequence.is_current_generation()
            || sequence.samples.iter().enumerate().any(|(i, s)| {
                !destinations.insert(s.destination)
                    || now
                        .checked_duration_since(s.observed_at.into())
                        .is_none_or(|age| age > SNAPSHOT_LIFETIME)
                    || (i > 0 && s.observed_at < sequence.samples[i - 1].observed_at)
            })
        {
            return Err(invalid("invalid mapping observation input"));
        }
        let observed_at = sequence.samples[0].observed_at.into();
        let wire = SnapshotWire {
            mapped: sequence.samples.iter().map(|s| s.mapped).collect(),
            local_ips: sequence.local_ips.clone(),
            assisted: Vec::new(),
        };
        wire.validate()?;
        Ok(Self {
            wire,
            validity: Validity::Sequence(sequence),
            observed_at,
        })
    }

    /// Missing classification evidence still permits the legacy basic attempt
    /// at an observed candidate. No candidate means an explicitly disabled plan.
    pub fn fallback(endpoint: &iroh::Endpoint, observation: UdpObservation) -> io::Result<Self> {
        Self::fallback_generation(endpoint.udp_network_generation(), observation)
    }
    /// Construct candidate-only evidence from a captured physical network epoch.
    pub fn fallback_generation(
        generation: ObservationValidity,
        observation: UdpObservation,
    ) -> io::Result<Self> {
        if matches!(
            observation,
            UdpObservation::Failed(UdpObservationFailure::NetworkChanged)
        ) {
            return Err(invalid("cannot use a network-invalidated fallback"));
        }
        let mapped = match &observation {
            UdpObservation::Fresh { addresses, .. } if observation.is_fresh() => addresses
                .iter()
                .copied()
                .filter(SocketAddr::is_ipv4)
                .take(1)
                .collect(),
            _ => Vec::new(),
        };
        let wire = SnapshotWire {
            mapped,
            local_ips: Vec::new(),
            assisted: Vec::new(),
        };
        wire.validate()?;
        Ok(Self {
            wire,
            validity: Validity::Fallback { generation },
            observed_at: Instant::now(),
        })
    }

    pub fn with_assisted(mut self, assisted: Vec<SocketAddr>) -> io::Result<Self> {
        self.wire.assisted = assisted;
        self.wire.validate()?;
        Ok(self)
    }

    pub fn feature(&self) -> Option<crate::nat_selector::NatFeature> {
        self.wire.view().map(|v| v.feature())
    }

    pub fn is_current(&self) -> bool {
        if self.observed_at.elapsed() > SNAPSHOT_LIFETIME {
            return false;
        }
        match &self.validity {
            Validity::Sequence(sequence) => sequence.is_current_generation(),
            Validity::Fallback { generation } => generation.is_current(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Round([u8; 16]);
impl Round {
    fn fresh() -> Self {
        Self(rand::random())
    }
    pub fn encoded(self) -> String {
        hex::encode(self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub id: BehaviorId,
    pub c: Behavior,
    pub v: Behavior,
    pub scores_before: Vec<BehaviorScore>,
    pub fallback: bool,
    pub disabled: bool,
}

fn make_plan(
    analyzer: &mut Analyzer,
    scope: LearningScope,
    c: &SnapshotWire,
    v: &SnapshotWire,
) -> (Plan, Option<Recommendation>) {
    if let (Some(c), Some(v)) = (c.view(), v.view()) {
        let rec = analyzer.recommend(scope, &c, &v, Instant::now());
        (
            Plan {
                id: rec.id,
                c: rec.c,
                v: rec.v,
                scores_before: rec.scores_before.clone(),
                fallback: false,
                disabled: false,
            },
            Some(rec),
        )
    } else {
        let c_is_receiver = v.target().is_none() && c.target().is_some();
        let c_behavior = Behavior {
            role: if c_is_receiver {
                crate::nat_selector::Role::Receiver
            } else {
                crate::nat_selector::Role::Sender
            },
            ttl: 0,
            send_delay_ms: 0,
            ports_range: 0,
            ports_random: 0,
            listen_random: 0,
        };
        let v_behavior = Behavior {
            role: if c_is_receiver {
                crate::nat_selector::Role::Sender
            } else {
                crate::nat_selector::Role::Receiver
            },
            ..c_behavior
        };
        (
            Plan {
                id: BehaviorId {
                    mode: 0,
                    index: if c_is_receiver { 7 } else { 6 },
                },
                c: c_behavior,
                v: v_behavior,
                scores_before: Vec::new(),
                fallback: true,
                disabled: c.target().is_none() && v.target().is_none(),
            },
            None,
        )
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum Frame {
    Hello {
        network: [u8; 32],
        generation: u64,
    },
    Challenge {
        network: [u8; 32],
        generation: u64,
        round: Round,
    },
    Offer {
        round: Round,
        snapshot: SnapshotWire,
    },
    Plan {
        round: Round,
        snapshot: SnapshotWire,
        plan: Plan,
    },
    Start {
        round: Round,
    },
    Ack {
        round: Round,
    },
    Established {
        round: Round,
    },
    WinnerReady {
        round: Round,
    },
    WinnerReadyAck {
        round: Round,
    },
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

async fn guard_remaining<T>(
    check: impl Fn() -> io::Result<Duration>,
    operation: impl std::future::Future<Output = io::Result<T>>,
) -> io::Result<T> {
    let remaining = check()?;
    let guard = async {
        loop {
            check()?;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        #[allow(unreachable_code)]
        Err::<T, io::Error>(io::Error::other("handoff guard stopped"))
    };
    tokio::select! {
        result = tokio::time::timeout(remaining, operation) => {
            let result = result.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "legacy winner handoff deadline elapsed"))??;
            check()?;
            Ok(result)
        },
        result = guard => result,
    }
}

fn current_scope(control: &ControlSession, gate: &AdmissionGate) -> io::Result<LearningScope> {
    control.nat_learning_scope(gate).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "current member proof required for NAT planning",
        )
    })
}

fn check_scope(
    control: &ControlSession,
    gate: &AdmissionGate,
    expected: &LearningScope,
    snapshot: &MappingSnapshot,
) -> io::Result<()> {
    if current_scope(control, gate)? != *expected || !snapshot.is_current() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "NAT scope or observation generation changed",
        ));
    }
    Ok(())
}

async fn send(exchange: &iroh::endpoint::ControlExchange, frame: Frame) -> io::Result<()> {
    let bytes = serde_json::to_vec(&frame).map_err(io::Error::other)?;
    tokio::time::timeout(
        EXCHANGE_WAIT,
        exchange.send_message(
            iroh::endpoint::ControlMessageFormat::EndTerminated {
                maximum: FRAME_LIMIT,
            },
            &bytes,
        ),
    )
    .await
    .map_err(io::Error::other)?
}
async fn receive(exchange: &iroh::endpoint::ControlExchange) -> io::Result<Frame> {
    let bytes = tokio::time::timeout(
        EXCHANGE_WAIT,
        exchange.recv_message(iroh::endpoint::ControlMessageFormat::EndTerminated {
            maximum: FRAME_LIMIT,
        }),
    )
    .await
    .map_err(io::Error::other)??;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}
async fn open(control: &ControlSession) -> io::Result<iroh::endpoint::ControlExchange> {
    tokio::time::timeout(EXCHANGE_WAIT, control.open_control_exchange())
        .await
        .map_err(io::Error::other)?
}

/// The exact original connection, round and scope remain attached to the
/// attempt. Taking a successful result consumes feedback once.
#[derive(Debug)]
pub struct Attempt {
    round: Round,
    plan: Plan,
    local: Behavior,
    remote_target: Option<SocketAddr>,
    remote_snapshot: SnapshotWire,
    scope: LearningScope,
    connection_id: usize,
    snapshot: MappingSnapshot,
    recommendation: Option<Recommendation>,
    submitted: bool,
    finished: bool,
    execution_trace: Option<crate::nat_scheduler::ExecutionTrace>,
    local_winner: Option<iroh::endpoint::LocalSocketBinding>,
    winner_lease: Option<iroh::endpoint::UdpSocketPathLease>,
    feedback_ready: bool,
    phase_deadline: Option<tokio::time::Instant>,
}

pub async fn coordinate_server(
    control: &ControlSession,
    gate: &AdmissionGate,
    snapshot: &MappingSnapshot,
    analyzer: &Arc<Mutex<Analyzer>>,
) -> io::Result<Attempt> {
    let scope = current_scope(control, gate)?;
    check_scope(control, gate, &scope, snapshot)?;
    let exchange = tokio::time::timeout(EXCHANGE_WAIT, control.accept_control_exchange())
        .await
        .map_err(io::Error::other)?
        .map_err(io::Error::other)?;
    match receive(&exchange).await? {
        Frame::Hello {
            network,
            generation,
        } if network == scope.network && generation == scope.secret_generation => {}
        _ => return Err(invalid("wrong NAT network or hello")),
    }
    let round = Round::fresh();
    check_scope(control, gate, &scope, snapshot)?;
    send(
        &exchange,
        Frame::Challenge {
            network: scope.network,
            generation: scope.secret_generation,
            round,
        },
    )
    .await?;
    let exchange = tokio::time::timeout(EXCHANGE_WAIT, control.accept_control_exchange())
        .await
        .map_err(io::Error::other)?
        .map_err(io::Error::other)?;
    let Frame::Offer {
        round: offered,
        snapshot: remote,
    } = receive(&exchange).await?
    else {
        return Err(invalid("mapping offer required"));
    };
    if offered != round {
        return Err(invalid("old NAT round"));
    }
    remote.validate()?;
    check_scope(control, gate, &scope, snapshot)?;
    #[cfg(feature = "nat-lab-test-hooks")]
    if let Ok(value) = std::env::var("MIOPUNCH_LEGACY_PREVIOUS_FAILURES") {
        let count = value
            .parse::<usize>()
            .map_err(|_| invalid("invalid test history count"))?;
        if count > 128 {
            return Err(invalid("test history count too large"));
        }
        if let (Some(c), Some(v)) = (snapshot.wire.view(), remote.view()) {
            for _ in 0..count {
                analyzer.lock().expect("NAT analyzer lock").recommend(
                    scope.clone(),
                    &c,
                    &v,
                    Instant::now(),
                );
            }
            println!("LEGACY_TEST_HISTORY previous_selections={count}");
        }
    }
    let (plan, recommendation) = make_plan(
        &mut analyzer.lock().expect("NAT analyzer lock"),
        scope.clone(),
        &snapshot.wire,
        &remote,
    );
    send(
        &exchange,
        Frame::Plan {
            round,
            snapshot: snapshot.wire.clone(),
            plan: plan.clone(),
        },
    )
    .await?;
    let exchange = tokio::time::timeout(EXCHANGE_WAIT, control.accept_control_exchange())
        .await
        .map_err(io::Error::other)?
        .map_err(io::Error::other)?;
    if !matches!(receive(&exchange).await?,Frame::Start {round:started} if started == round) {
        return Err(invalid("old or cancelled NAT start"));
    }
    check_scope(control, gate, &scope, snapshot)?;
    send(&exchange, Frame::Ack { round }).await?;
    // The visitor has checked the final scope and received the plan before an
    // unavailable executor can close the coordinator's connection.
    let exchange = tokio::time::timeout(EXCHANGE_WAIT, control.accept_control_exchange())
        .await
        .map_err(io::Error::other)?
        .map_err(io::Error::other)?;
    if !matches!(receive(&exchange).await?,Frame::Established {round:received} if received==round) {
        return Err(invalid("NAT plan receipt mismatch"));
    }
    check_scope(control, gate, &scope, snapshot)?;
    Ok(Attempt {
        round,
        local: plan.c,
        plan,
        remote_target: remote.target(),
        remote_snapshot: remote,
        scope,
        connection_id: control.stable_id(),
        snapshot: snapshot.clone(),
        recommendation,
        submitted: false,
        finished: false,
        execution_trace: None,
        local_winner: None,
        winner_lease: None,
        feedback_ready: false,
        phase_deadline: None,
    })
}

pub async fn coordinate_client(
    control: &ControlSession,
    gate: &AdmissionGate,
    snapshot: &MappingSnapshot,
) -> io::Result<Attempt> {
    let scope = current_scope(control, gate)?;
    check_scope(control, gate, &scope, snapshot)?;
    let exchange = open(control).await?;
    send(
        &exchange,
        Frame::Hello {
            network: scope.network,
            generation: scope.secret_generation,
        },
    )
    .await?;
    let Frame::Challenge {
        network,
        generation,
        round,
    } = receive(&exchange).await?
    else {
        return Err(invalid("NAT challenge required"));
    };
    if network != scope.network || generation != scope.secret_generation {
        return Err(invalid("wrong NAT challenge scope"));
    }
    check_scope(control, gate, &scope, snapshot)?;
    let exchange = open(control).await?;
    check_scope(control, gate, &scope, snapshot)?;
    send(
        &exchange,
        Frame::Offer {
            round,
            snapshot: snapshot.wire.clone(),
        },
    )
    .await?;
    let Frame::Plan {
        round: planned,
        snapshot: remote,
        plan,
    } = receive(&exchange).await?
    else {
        return Err(invalid("NAT plan required"));
    };
    if planned != round {
        return Err(invalid("old NAT plan"));
    }
    remote.validate()?;
    if let (Some(c), Some(v)) = (remote.view(), snapshot.wire.view()) {
        let expected = initial_scores(c.feature(), v.feature());
        let first_maximum = plan
            .scores_before
            .iter()
            .fold(None::<&BehaviorScore>, |best, item| match best {
                Some(best) if best.score >= item.score => Some(best),
                _ => Some(item),
            })
            .map(|s| s.id);
        if plan.fallback
            || plan.disabled
            || behaviors(plan.id, c.feature(), v.feature()) != Some((plan.c, plan.v))
            || plan
                .scores_before
                .iter()
                .map(|s| s.id)
                .ne(expected.iter().map(|s| s.id))
            || plan.scores_before.iter().any(|s| s.score > 10)
            || first_maximum != Some(plan.id)
        {
            return Err(invalid("NAT plan does not match legacy parameters"));
        }
    } else {
        let (expected, _) = make_plan(
            &mut Analyzer::new(Instant::now()),
            scope.clone(),
            &remote,
            &snapshot.wire,
        );
        if plan != expected {
            return Err(invalid("invalid NAT fallback"));
        }
    }
    check_scope(control, gate, &scope, snapshot)?;
    let exchange = open(control).await?;
    send(&exchange, Frame::Start { round }).await?;
    if !matches!(receive(&exchange).await?,Frame::Ack {round:acked} if acked == round) {
        return Err(invalid("NAT ack mismatch"));
    }
    check_scope(control, gate, &scope, snapshot)?;
    let exchange = open(control).await?;
    send(&exchange, Frame::Established { round }).await?;
    Ok(Attempt {
        round,
        local: plan.v,
        plan,
        remote_target: remote.target(),
        remote_snapshot: remote,
        scope,
        connection_id: control.stable_id(),
        snapshot: snapshot.clone(),
        recommendation: None,
        submitted: false,
        finished: false,
        execution_trace: None,
        local_winner: None,
        winner_lease: None,
        feedback_ready: false,
        phase_deadline: None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionStatus {
    PartialExecution,
    Ready,
    CompleteExecution,
    ExecutionUnavailable,
    Disabled,
}

impl Attempt {
    pub fn round(&self) -> Round {
        self.round
    }
    pub fn plan(&self) -> &Plan {
        &self.plan
    }
    pub fn local_behavior(&self) -> Behavior {
        self.local
    }
    pub fn remote_target(&self) -> Option<SocketAddr> {
        self.remote_target
    }
    /// Actual receiving physical socket from the authenticated raw winner.
    pub fn local_winner(&self) -> Option<iroh::endpoint::LocalSocketBinding> {
        self.local_winner
    }
    /// Remaining original phase budget for the protected winner/path handoff.
    pub fn path_wait_remaining(
        &self,
        control: &ControlSession,
        gate: &AdmissionGate,
    ) -> io::Result<Duration> {
        check_scope(control, gate, &self.scope, &self.snapshot)?;
        if self.finished
            || !self.feedback_ready
            || control.stable_id() != self.connection_id
            || control.is_closed()
        {
            return Err(invalid("NAT winner handoff is not current"));
        }
        self.phase_deadline
            .and_then(|deadline| deadline.checked_duration_since(tokio::time::Instant::now()))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "legacy winner handoff deadline elapsed",
                )
            })
    }

    /// Both peers retain their winner before either starts QUIC validation.
    /// This is an original authenticated control exchange within the same
    /// phase deadline; raw receipts never grant a peer's business capability.
    pub async fn synchronize_winner(
        &self,
        control: &ControlSession,
        gate: &AdmissionGate,
    ) -> io::Result<()> {
        let exchange = async {
            let outgoing = open(control).await?;
            send(&outgoing, Frame::WinnerReady { round: self.round }).await?;
            let incoming = control.accept_control_exchange().await?;
            if !matches!(receive(&incoming).await?, Frame::WinnerReady { round } if round == self.round)
            {
                return Err(invalid("wrong NAT winner handoff round"));
            }
            self.path_wait_remaining(control, gate)?;
            send(&incoming, Frame::WinnerReadyAck { round: self.round }).await?;
            if !matches!(receive(&outgoing).await?, Frame::WinnerReadyAck { round } if round == self.round)
            {
                return Err(invalid("wrong NAT winner handoff acknowledgement"));
            }
            self.path_wait_remaining(control, gate)?;
            println!(
                "LEGACY_WINNER_READY round={} stable_id={} remaining_ms={}",
                self.round.encoded(),
                self.connection_id,
                self.path_wait_remaining(control, gate)?.as_millis()
            );
            Ok(())
        };
        self.guard_handoff(control, gate, exchange).await
    }

    /// Validated open path on the actual winner, under the captured scope.
    pub fn verify_winner_path(
        &self,
        control: &ControlSession,
        gate: &AdmissionGate,
    ) -> io::Result<()> {
        self.path_wait_remaining(control, gate)?;
        let local = self
            .local_winner
            .ok_or_else(|| invalid("NAT winner has no local binding"))?;
        let remote = self
            .remote_target
            .ok_or_else(|| invalid("NAT winner has no remote source"))?;
        if control.has_data_path_from(local, remote) {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "actual NAT winner path not validated",
            ))
        }
    }

    /// Guard every handoff wait with the same scope and absolute deadline.
    pub async fn guard_handoff<T>(
        &self,
        control: &ControlSession,
        gate: &AdmissionGate,
        operation: impl std::future::Future<Output = io::Result<T>>,
    ) -> io::Result<T> {
        guard_remaining(|| self.path_wait_remaining(control, gate), operation).await
    }

    /// Retain both winners, then validate the original connection's exact
    /// physical winner. The original QUIC dialer initiates path validation;
    /// the legacy raw sender/receiver role remains independent.
    pub async fn prepare_data_path(
        &self,
        control: &ControlSession,
        gate: &AdmissionGate,
    ) -> io::Result<DataPathOpen> {
        self.synchronize_winner(control, gate).await?;
        let local = self
            .local_winner
            .ok_or_else(|| invalid("NAT winner has no local binding"))?;
        let remote = self
            .remote_target
            .ok_or_else(|| invalid("NAT winner has no remote source"))?;
        let deadline = self
            .phase_deadline
            .ok_or_else(|| invalid("NAT phase not started"))?;
        self.guard_handoff(control, gate, async {
            let report = if control.is_dialing_side() {
                control
                    .open_data_path_from_until(local, remote, deadline, &|| {
                        self.path_wait_remaining(control, gate).map(|_| ())
                    })
                    .await?
            } else {
                DataPathOpen { path_id_retries: 0 }
            };
            loop {
                match self.verify_winner_path(control, gate) {
                    Ok(()) => return Ok(report),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        tokio::time::sleep(Duration::from_millis(10)).await
                    }
                    Err(error) => return Err(error),
                }
            }
        })
        .await
    }

    /// Commit only the current captured winner; queued business remains local
    /// until this transition has been checked again after the actor responds.
    pub async fn commit_winner_path(
        &self,
        control: ControlSession,
        gate: &AdmissionGate,
    ) -> Result<DataSession, CommitError> {
        if let Err(error) = self.verify_winner_path(&control, gate) {
            return Err(CommitError::Retryable {
                error,
                session: control,
            });
        }
        let deadline = self.phase_deadline.expect("verified phase deadline");
        let mut cancellation = control.cancellation_guard();
        let data = match tokio::time::timeout_at(deadline, control.commit_data_path()).await {
            Ok(Ok(data)) => data,
            Ok(Err(error)) => {
                cancellation.disarm();
                return Err(error);
            }
            Err(_) => {
                return Err(CommitError::Terminal {
                    error: io::Error::new(
                        io::ErrorKind::TimedOut,
                        "legacy path commitment deadline elapsed",
                    ),
                });
            }
        };
        if let Err(error) = self.data_wait_remaining(&data, gate) {
            data.connection().close(
                iroh::endpoint::VarInt::from_u32(0),
                b"invalidated NAT winner handoff",
            );
            return Err(CommitError::Terminal { error });
        }
        cancellation.disarm();
        Ok(data)
    }

    /// Current scope and selected physical winner before releasing business.
    pub fn data_wait_remaining(
        &self,
        data: &DataSession,
        gate: &AdmissionGate,
    ) -> io::Result<Duration> {
        self.data_wait_remaining_on(data.connection(), gate)
    }
    fn data_wait_remaining_on(
        &self,
        connection: &(impl crate::control_protocol::AuthenticatedControl + ?Sized),
        gate: &AdmissionGate,
    ) -> io::Result<Duration> {
        let current = !self.finished && self.feedback_ready
            && gate.nat_learning_scope(connection) == Some(self.scope.clone())
            && self.snapshot.is_current()
            && connection.close_reason().is_none()
            && connection.stable_id() == self.connection_id
            && connection.paths().iter().any(|path| path.is_selected()
                && path.local_socket_id() == self.local_winner.map(|local| local.id)
                && matches!(path.remote_addr(), iroh_base::TransportAddr::Ip(addr) if Some(*addr) == self.remote_target));
        if !current {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "NAT winner scope or physical path changed during commitment",
            ));
        }
        self.phase_deadline
            .and_then(|deadline| deadline.checked_duration_since(tokio::time::Instant::now()))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "legacy winner handoff deadline elapsed",
                )
            })
    }

    pub async fn guard_committed<T>(
        &self,
        data: &DataSession,
        gate: &AdmissionGate,
        operation: impl std::future::Future<Output = io::Result<T>>,
    ) -> io::Result<T> {
        guard_remaining(|| self.data_wait_remaining(data, gate), operation).await
    }
    pub fn remote_feature(&self) -> Option<crate::nat_selector::NatFeature> {
        self.remote_snapshot.view().map(|v| v.feature())
    }
    pub fn mapped_candidates(&self) -> &[SocketAddr] {
        &self.remote_snapshot.mapped
    }
    pub fn assisted_candidates(&self) -> &[SocketAddr] {
        &self.remote_snapshot.assisted
    }

    pub fn execution_status(&self) -> ExecutionStatus {
        if self.plan.disabled {
            ExecutionStatus::Disabled
        } else {
            ExecutionStatus::Ready
        }
    }

    pub(crate) fn peer_datagram_bound(&self) -> usize {
        let peer_delay = if self.local == self.plan.c {
            self.plan.v.send_delay_ms
        } else {
            self.plan.c.send_delay_ms
        };
        crate::nat_scheduler::PhasePlan::new(
            self.local,
            peer_delay,
            &self.remote_snapshot.mapped,
            &self.remote_snapshot.assisted,
            self.remote_feature().map_or(0, |f| f.ports_difference),
        )
        .peer_datagram_bound(usize::from(self.local.listen_random) + 1)
    }

    pub(crate) fn check_execution(&self, control: &ControlSession) -> io::Result<()> {
        if self.submitted || self.finished || control.stable_id() != self.connection_id {
            return Err(invalid("old, submitted or finished NAT attempt"));
        }
        Ok(())
    }

    /// Existing same-socket primitive only: one guarded UDP submission to the
    /// observed peer. Does not implement range, TTL, delay or repeat scheduling.
    pub async fn execute_partial(
        &mut self,
        control: &ControlSession,
        gate: &AdmissionGate,
        budget: &NodeProbeBudget,
    ) -> io::Result<ExecutionStatus> {
        if self.submitted || self.finished || control.stable_id() != self.connection_id {
            return Err(invalid("old, submitted or finished NAT attempt"));
        }
        let status = self.execution_status();
        if status != ExecutionStatus::Ready {
            return Ok(status);
        }
        if self.plan.c.ports_random > 0
            || self.plan.v.ports_random > 0
            || self.plan.c.listen_random > 0
            || self.plan.v.listen_random > 0
        {
            return Ok(ExecutionStatus::ExecutionUnavailable);
        }
        check_scope(control, gate, &self.scope, &self.snapshot)?;
        let Some(target) = self.remote_target else {
            if self.local.role == crate::nat_selector::Role::Receiver {
                self.submitted = true;
                self.feedback_ready = true;
                return Ok(ExecutionStatus::PartialExecution);
            }
            return Err(invalid(
                "sender has no peer candidate for partial execution",
            ));
        };
        if !budget.spend_peer_datagram() {
            return Err(io::Error::other("NAT probe budget exhausted"));
        }
        self.submitted = true;
        check_scope(control, gate, &self.scope, &self.snapshot)?;
        control.prime_udp_path(target).await?;
        self.feedback_ready = true;
        Ok(ExecutionStatus::PartialExecution)
    }

    /// Execute the complete old physical-socket phase with continuous scope checks.
    /// A provisional raw winner never grants business or updates learning.
    pub async fn execute(
        &mut self,
        control: &ControlSession,
        gate: &AdmissionGate,
        budget: &NodeProbeBudget,
    ) -> io::Result<ExecutionStatus> {
        self.execute_checked(control, gate, budget, &|| Ok(()))
            .await
    }

    pub(crate) async fn execute_checked(
        &mut self,
        control: &ControlSession,
        gate: &AdmissionGate,
        budget: &NodeProbeBudget,
        permission: &(dyn Fn() -> io::Result<()> + Sync),
    ) -> io::Result<ExecutionStatus> {
        if self.submitted || self.finished || control.stable_id() != self.connection_id {
            return Err(invalid("old, submitted or finished NAT attempt"));
        }
        let status = self.execution_status();
        if status != ExecutionStatus::Ready {
            return Ok(status);
        }
        check_scope(control, gate, &self.scope, &self.snapshot)?;
        let peer_delay = if self.local == self.plan.c {
            self.plan.v.send_delay_ms
        } else {
            self.plan.c.send_delay_ms
        };
        let phase = crate::nat_scheduler::PhasePlan::new(
            self.local,
            peer_delay,
            &self.remote_snapshot.mapped,
            &self.remote_snapshot.assisted,
            self.remote_feature().map_or(0, |f| f.ports_difference),
        );
        let key = control.traversal_key(&self.round.0)?;
        let auth = crate::nat_scheduler::PacketAuth {
            key,
            local: self.scope.local,
            remote: self.scope.remote,
        };
        let mut lease = control.register_udp_traversal(self.round.0).await?;
        self.submitted = true;
        // Old MakeHole starts the total deadline before creating random
        // listeners. Binding work therefore consumes the same phase budget.
        let started = Instant::now();
        let deadline =
            tokio::time::Instant::now() + Duration::from_millis(phase.total_budget_ms.into());
        self.phase_deadline = Some(deadline);
        let check = || {
            permission()?;
            if tokio::time::Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "legacy NAT phase budget elapsed",
                ));
            }
            check_scope(control, gate, &self.scope, &self.snapshot)?;
            if control.is_closed() {
                return Err(io::Error::other("NAT connection closed"));
            }
            Ok(())
        };
        let trace = Arc::new(Mutex::new(crate::nat_scheduler::ExecutionTrace::default()));
        let _summary = crate::nat_scheduler::TraceSummary {
            trace: trace.clone(),
            round: self.round.encoded(),
            connection_id: self.connection_id,
        };
        let setup = tokio::time::timeout_at(
            deadline,
            lease.listen_random_ports_checked(usize::from(self.local.listen_random), &check),
        )
        .await
        .unwrap_or_else(|_| {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "legacy NAT phase budget elapsed",
            ))
        });
        let report = lease.listener_report();
        {
            let mut ledger = trace.lock().expect("NAT execution trace");
            ledger.listener_attempted = report.attempted;
            ledger.listener_bound = report.bound;
            ledger.listener_failed = report.failed;
        }
        if let Err(error) = setup {
            self.execution_trace = Some(trace.lock().expect("NAT execution trace").clone());
            println!(
                "LEGACY_PHASE_STOP round={} stable_id={} elapsed_ms={} outcome=failed",
                self.round.encoded(),
                self.connection_id,
                started.elapsed().as_millis()
            );
            return Err(error);
        }
        println!(
            "LEGACY_RECV_START round={} stable_id={} bind_elapsed_ms={} phase={phase:?}",
            self.round.encoded(),
            self.connection_id,
            started.elapsed().as_millis()
        );
        for binding in lease.local_sockets() {
            println!(
                "LEGACY_LISTENER round={} stable_id={} local_socket_id={} local_addr={} primary={}",
                self.round.encoded(),
                self.connection_id,
                binding.id.get(),
                binding.local_addr,
                binding.id.is_primary()
            );
        }
        let guard = async {
            loop {
                permission()?;
                check_scope(control, gate, &self.scope, &self.snapshot)?;
                if control.is_closed() {
                    return Err(io::Error::other("NAT connection closed"));
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            #[allow(unreachable_code)]
            Ok::<(), io::Error>(())
        };
        let result = tokio::select! {
            result=tokio::time::timeout_at(deadline,crate::nat_scheduler::run_with_trace(&phase,&mut lease,&auth,budget,&trace,&check))=>result.unwrap_or_else(|_|Err(io::Error::new(io::ErrorKind::TimedOut,"legacy NAT phase budget elapsed"))),
            result=guard=> match result { Err(error)=>Err(error),Ok(())=>unreachable!() }
        };
        println!(
            "LEGACY_PHASE_STOP round={} stable_id={} elapsed_ms={} outcome={}",
            self.round.encoded(),
            self.connection_id,
            started.elapsed().as_millis(),
            if result.is_ok() { "winner" } else { "failed" }
        );
        self.execution_trace = Some(trace.lock().expect("NAT execution trace").clone());
        let winner = result?;
        check_scope(control, gate, &self.scope, &self.snapshot)?;
        let winner_lease = lease
            .retain_winner(winner.local_socket_id, winner.local_addr)
            .await?;
        check_scope(control, gate, &self.scope, &self.snapshot)?;
        // The original Connection now owns the physical winner. Raw registration
        // and every loser are closed when the round ends.
        drop(lease);
        let trace = self.execution_trace.as_ref().expect("execution ledger");
        println!(
            "LEGACY_WINNER round={} stable_id={} remote={} response={} before_probe={} probes={} responses={} ttl_degraded={} local_socket_id={} local_addr={}",
            self.round.encoded(),
            self.connection_id,
            winner.remote,
            winner.response,
            winner.before_probe,
            trace.probes,
            trace.responses,
            trace.ttl_degraded,
            winner.local_socket_id.get(),
            winner.local_addr
        );
        self.remote_target = Some(winner.remote);
        self.local_winner = Some(iroh::endpoint::LocalSocketBinding {
            id: winner.local_socket_id,
            local_addr: winner.local_addr,
        });
        self.winner_lease = winner_lease;
        self.feedback_ready = true;
        Ok(ExecutionStatus::CompleteExecution)
    }

    pub fn execution_trace(&self) -> Option<&crate::nat_scheduler::ExecutionTrace> {
        self.execution_trace.as_ref()
    }

    /// Ordinary Endpoint handoff retains the validated physical socket without
    /// inventing an application success report. Explicit business-confirming
    /// callers keep using confirm_success and its established feedback rule.
    pub(crate) fn retain_application_path(
        &mut self,
        data: &DataSession,
        gate: &AdmissionGate,
    ) -> io::Result<()> {
        self.data_wait_remaining(data, gate)?;
        if let Some(lease) = self.winner_lease.as_mut() {
            lease.commit(data.connection())?;
        }
        self.finished = true;
        self.recommendation = None;
        Ok(())
    }

    pub(crate) fn retain_control_path(
        &mut self,
        control: &ControlSession,
        gate: &AdmissionGate,
    ) -> io::Result<()> {
        self.data_wait_remaining_on(&control.scope_connection(), gate)?;
        if let Some(lease) = self.winner_lease.as_mut() {
            control.retain_udp_winner(lease)?;
        }
        self.finished = true;
        self.recommendation = None;
        Ok(())
    }

    /// Invoke after authorized business confirmation. DataSession can only be
    /// obtained after the original connection's verified data-path commit.
    pub fn confirm_success(
        &mut self,
        data: &DataSession,
        gate: &AdmissionGate,
        analyzer: &Arc<Mutex<Analyzer>>,
    ) -> bool {
        if self.finished {
            return false;
        }
        self.finished = true;
        if data.connection().stable_id() != self.connection_id
            || !self.submitted
            || !self.feedback_ready
            || gate.nat_learning_scope(data.connection()).as_ref() != Some(&self.scope)
            || !self.snapshot.is_current()
            || data.connection().close_reason().is_some()
            || !data.connection().paths().iter().any(|path| path.is_selected()
                && matches!(path.remote_addr(),iroh_base::TransportAddr::Ip(addr) if Some(*addr) == self.remote_target)
                && self.local_winner.is_none_or(|winner| path.local_socket_id() == Some(winner.id)))
        { self.winner_lease.take(); return false; }
        if self
            .winner_lease
            .as_mut()
            .is_some_and(|lease| lease.commit(data.connection()).is_err())
        {
            self.winner_lease.take();
            return false;
        }
        self.recommendation.take().is_some_and(|rec| {
            analyzer
                .lock()
                .expect("NAT analyzer lock")
                .report_success(&rec, Instant::now())
        })
    }

    pub fn finish_failure(&mut self) {
        self.winner_lease.take();
        self.finished = true;
        self.recommendation = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nat_selector::{ProbeProtocol, Role};

    #[test]
    fn incomplete_samples_keep_legacy_fallback_sender_target_preference() {
        let with_target = SnapshotWire {
            mapped: vec!["192.0.2.1:41000".parse().unwrap()],
            local_ips: Vec::new(),
            assisted: Vec::new(),
        };
        let empty = SnapshotWire {
            mapped: Vec::new(),
            local_ips: Vec::new(),
            assisted: Vec::new(),
        };
        let scope = LearningScope {
            local: [1; 32],
            remote: [2; 32],
            network: [3; 32],
            secret_generation: 1,
            credential: [4; 32],
            authorization_epoch: 1,
            protocol: ProbeProtocol::Udp,
        };
        let mut analyzer = Analyzer::new(Instant::now());
        for (c, v, c_role, v_role, disabled) in [
            (
                &with_target,
                &with_target,
                Role::Sender,
                Role::Receiver,
                false,
            ),
            (&empty, &with_target, Role::Sender, Role::Receiver, false),
            (&with_target, &empty, Role::Receiver, Role::Sender, false),
            (&empty, &empty, Role::Sender, Role::Receiver, true),
        ] {
            let (plan, recommendation) = make_plan(&mut analyzer, scope.clone(), c, v);
            assert_eq!(
                (plan.c.role, plan.v.role, plan.disabled),
                (c_role, v_role, disabled)
            );
            assert!(plan.fallback);
            assert!(recommendation.is_none());
            assert_eq!(
                (
                    plan.c.ttl,
                    plan.v.ttl,
                    plan.c.send_delay_ms,
                    plan.v.send_delay_ms
                ),
                (0, 0, 0, 0)
            );
        }
    }
}
