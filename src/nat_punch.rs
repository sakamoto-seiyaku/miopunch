//! Protected legacy-plan exchange and the explicitly partial MIOPUNCH-62 adapter.
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

use iroh::endpoint::{UdpMappingSequence, UdpObservation};
use serde::{Deserialize, Serialize};

use crate::{
    admission_gate::AdmissionGate,
    control_session::{ControlSession, DataSession},
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
        self.mapped
            .last()
            .copied()
            .or_else(|| self.assisted.first().copied())
    }
}

#[derive(Debug, Clone)]
enum Validity {
    Sequence(UdpMappingSequence),
    Fallback(UdpObservation),
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
    pub fn fallback(observation: UdpObservation) -> io::Result<Self> {
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
            validity: Validity::Fallback(observation),
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
            Validity::Fallback(observation) => {
                self.wire.mapped.is_empty() || observation.is_fresh()
            }
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
        let c_behavior = Behavior {
            role: crate::nat_selector::Role::Receiver,
            ttl: 0,
            send_delay_ms: 0,
            ports_range: 0,
            ports_random: 0,
            listen_random: 0,
        };
        let v_behavior = Behavior {
            role: crate::nat_selector::Role::Sender,
            ..c_behavior
        };
        (
            Plan {
                id: BehaviorId { mode: 0, index: 7 },
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
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
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

async fn send(send: &mut iroh::endpoint::SendStream, frame: Frame) -> io::Result<()> {
    let bytes = serde_json::to_vec(&frame).map_err(io::Error::other)?;
    if bytes.len() > FRAME_LIMIT {
        return Err(invalid("NAT plan frame too large"));
    }
    tokio::time::timeout(EXCHANGE_WAIT, send.write_all(&bytes))
        .await
        .map_err(io::Error::other)?
        .map_err(io::Error::other)?;
    send.finish().map_err(io::Error::other)
}

async fn receive(recv: &mut iroh::endpoint::RecvStream) -> io::Result<Frame> {
    let bytes = tokio::time::timeout(EXCHANGE_WAIT, recv.read_to_end(FRAME_LIMIT))
        .await
        .map_err(io::Error::other)?
        .map_err(io::Error::other)?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

/// The exact original connection, round and scope remain attached to the
/// attempt. Taking a successful result consumes feedback once.
#[derive(Debug)]
pub struct Attempt {
    round: Round,
    plan: Plan,
    local: Behavior,
    remote_target: Option<SocketAddr>,
    scope: LearningScope,
    connection_id: usize,
    snapshot: MappingSnapshot,
    recommendation: Option<Recommendation>,
    submitted: bool,
    finished: bool,
}

pub async fn coordinate_server(
    control: &ControlSession,
    gate: &AdmissionGate,
    snapshot: &MappingSnapshot,
    analyzer: &Arc<Mutex<Analyzer>>,
) -> io::Result<Attempt> {
    let scope = current_scope(control, gate)?;
    check_scope(control, gate, &scope, snapshot)?;
    let (mut reply, mut request) = tokio::time::timeout(EXCHANGE_WAIT, control.accept_control_bi())
        .await
        .map_err(io::Error::other)?
        .map_err(io::Error::other)?;
    match receive(&mut request).await? {
        Frame::Hello {
            network,
            generation,
        } if network == scope.network && generation == scope.secret_generation => {}
        _ => return Err(invalid("wrong NAT network or hello")),
    }
    let round = Round::fresh();
    check_scope(control, gate, &scope, snapshot)?;
    send(
        &mut reply,
        Frame::Challenge {
            network: scope.network,
            generation: scope.secret_generation,
            round,
        },
    )
    .await?;
    let (mut reply, mut request) = tokio::time::timeout(EXCHANGE_WAIT, control.accept_control_bi())
        .await
        .map_err(io::Error::other)?
        .map_err(io::Error::other)?;
    let Frame::Offer {
        round: offered,
        snapshot: remote,
    } = receive(&mut request).await?
    else {
        return Err(invalid("mapping offer required"));
    };
    if offered != round {
        return Err(invalid("old NAT round"));
    }
    remote.validate()?;
    check_scope(control, gate, &scope, snapshot)?;
    let (plan, recommendation) = make_plan(
        &mut analyzer.lock().expect("NAT analyzer lock"),
        scope.clone(),
        &snapshot.wire,
        &remote,
    );
    send(
        &mut reply,
        Frame::Plan {
            round,
            snapshot: snapshot.wire.clone(),
            plan: plan.clone(),
        },
    )
    .await?;
    let (mut reply, mut request) = tokio::time::timeout(EXCHANGE_WAIT, control.accept_control_bi())
        .await
        .map_err(io::Error::other)?
        .map_err(io::Error::other)?;
    if !matches!(receive(&mut request).await?,Frame::Start {round:started} if started == round) {
        return Err(invalid("old or cancelled NAT start"));
    }
    check_scope(control, gate, &scope, snapshot)?;
    send(&mut reply, Frame::Ack { round }).await?;
    Ok(Attempt {
        round,
        local: plan.c,
        plan,
        remote_target: remote.target(),
        scope,
        connection_id: control.stable_id(),
        snapshot: snapshot.clone(),
        recommendation,
        submitted: false,
        finished: false,
    })
}

pub async fn coordinate_client(
    control: &ControlSession,
    gate: &AdmissionGate,
    snapshot: &MappingSnapshot,
) -> io::Result<Attempt> {
    let scope = current_scope(control, gate)?;
    check_scope(control, gate, &scope, snapshot)?;
    let (mut send_stream, mut recv) = control.open_control_bi().await.map_err(io::Error::other)?;
    send(
        &mut send_stream,
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
    } = receive(&mut recv).await?
    else {
        return Err(invalid("NAT challenge required"));
    };
    if network != scope.network || generation != scope.secret_generation {
        return Err(invalid("wrong NAT challenge scope"));
    }
    check_scope(control, gate, &scope, snapshot)?;
    let (mut send_stream, mut recv) = control.open_control_bi().await.map_err(io::Error::other)?;
    send(
        &mut send_stream,
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
    } = receive(&mut recv).await?
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
    let (mut send_stream, mut recv) = control.open_control_bi().await.map_err(io::Error::other)?;
    send(&mut send_stream, Frame::Start { round }).await?;
    if !matches!(receive(&mut recv).await?,Frame::Ack {round:acked} if acked == round) {
        return Err(invalid("NAT ack mismatch"));
    }
    check_scope(control, gate, &scope, snapshot)?;
    Ok(Attempt {
        round,
        local: plan.v,
        plan,
        remote_target: remote.target(),
        scope,
        connection_id: control.stable_id(),
        snapshot: snapshot.clone(),
        recommendation: None,
        submitted: false,
        finished: false,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionStatus {
    PartialExecution,
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

    pub fn execution_status(&self) -> ExecutionStatus {
        if self.plan.disabled {
            ExecutionStatus::Disabled
        } else if self.plan.c.ports_random > 0
            || self.plan.v.ports_random > 0
            || self.plan.c.listen_random > 0
            || self.plan.v.listen_random > 0
        {
            ExecutionStatus::ExecutionUnavailable
        } else {
            ExecutionStatus::PartialExecution
        }
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
        check_scope(control, gate, &self.scope, &self.snapshot)?;
        let status = self.execution_status();
        if status != ExecutionStatus::PartialExecution {
            return Ok(status);
        }
        let target = self
            .remote_target
            .ok_or_else(|| invalid("no peer candidate for partial execution"))?;
        if !budget.spend_peer_datagram() {
            return Err(io::Error::other("NAT probe budget exhausted"));
        }
        self.submitted = true;
        check_scope(control, gate, &self.scope, &self.snapshot)?;
        control.prime_udp_path(target).await?;
        Ok(status)
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
            || gate.nat_learning_scope(data.connection()).as_ref() != Some(&self.scope)
            || !self.snapshot.is_current()
            || data.connection().close_reason().is_some()
            || !data.connection().paths().iter().any(|path| path.is_selected()
                && matches!(path.remote_addr(),iroh_base::TransportAddr::Ip(addr) if Some(*addr) == self.remote_target))
        { return false; }
        self.recommendation.take().is_some_and(|rec| {
            analyzer
                .lock()
                .expect("NAT analyzer lock")
                .report_success(&rec, Instant::now())
        })
    }

    pub fn finish_failure(&mut self) {
        self.finished = true;
        self.recommendation = None;
    }
}
