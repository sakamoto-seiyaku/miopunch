//! Shared frp observation evidence, selector inputs and execution budgets.

use std::{
    collections::HashSet,
    io,
    net::{IpAddr, SocketAddr},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use super::nat_selector::{
    Analyzer, Behavior, BehaviorId, BehaviorScore, FeatureView, LearningScope, Recommendation,
    Role, behaviors, initial_scores,
};
use super::{UdpMappingSequence, UdpObservation, UdpObservationFailure};

/// Supports the longest old 38-second plan while bounding retained evidence.
pub const SNAPSHOT_LIFETIME: Duration = Duration::from_secs(60);
/// Maximum length of each observation or assistance list.
pub const MAX_OBSERVATIONS: usize = 64;

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Ordered IPv4 mappings and local candidates exchanged in a round.
pub struct SnapshotWire {
    /// Addresses reported by the independent observers, in observation order.
    pub mapped: Vec<SocketAddr>,
    /// IPs shared by a reported mapping and a local network interface.
    pub local_ips: Vec<IpAddr>,
    /// Direct local candidates to try when sending.
    pub assisted: Vec<SocketAddr>,
}

impl SnapshotWire {
    /// Validate the size and usability of the exchanged address lists.
    pub fn validate(&self) -> io::Result<()> {
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

    /// Derive classification only when enough observations are available.
    pub fn view(&self) -> Option<FeatureView> {
        FeatureView::new(self.mapped.clone(), &self.local_ips)
    }
    /// Return the first assisted address or the most recent mapped address.
    pub fn target(&self) -> Option<SocketAddr> {
        self.assisted
            .first()
            .copied()
            .or_else(|| self.mapped.last().copied())
    }
}

impl SnapshotWire {
    /// Construct evidence from current samples with distinct actual destinations.
    pub fn from_samples(
        sequence: &UdpMappingSequence,
        minimum: usize,
    ) -> io::Result<(Self, Instant)> {
        let now = Instant::now();
        let mut destinations = HashSet::new();
        if sequence.samples.len() < minimum
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
        let mapped: Vec<_> = sequence
            .samples
            .iter()
            .map(|sample| sample.mapped)
            .collect();
        // The classifier only asks whether a reported IP belongs to a local
        // interface. Unrelated interfaces cannot affect that decision.
        let mut seen_local_ips = HashSet::new();
        let local_ips = mapped
            .iter()
            .map(SocketAddr::ip)
            .filter(|ip| sequence.local_ips.contains(ip) && seen_local_ips.insert(*ip))
            .collect();
        let wire = Self {
            mapped,
            local_ips,
            assisted: Vec::new(),
        };
        wire.validate()?;
        Ok((wire, observed_at))
    }

    /// Use at most one existing mapping as a basic candidate.
    pub fn existing(observation: UdpObservation) -> io::Result<Self> {
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
        let wire = Self {
            mapped,
            local_ips: Vec::new(),
            assisted: Vec::new(),
        };
        wire.validate()?;
        Ok(wire)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// The shared traversal behavior and two roles in the legacy plan.
pub struct Plan {
    /// Mode and index of the selected behavior.
    pub id: BehaviorId,
    /// Parameters for the coordinator.
    pub c: Behavior,
    /// Parameters for the initiating peer.
    pub v: Behavior,
    /// Coordinator's score table before the one selection.
    pub scores_before: Vec<BehaviorScore>,
    /// Whether either side lacked enough evidence to select an enhanced behavior.
    pub fallback: bool,
    /// Whether neither side had a usable UDP candidate.
    pub disabled: bool,
}

/// Select one frp behavior or construct the basic candidate-only plan.
pub fn make_plan(
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
        (basic_plan(c, v), None)
    }
}

pub(crate) fn basic_plan(c: &SnapshotWire, v: &SnapshotWire) -> Plan {
    let c_is_receiver = v.target().is_none() && c.target().is_some();
    let c_behavior = Behavior {
        role: if c_is_receiver {
            Role::Receiver
        } else {
            Role::Sender
        },
        ttl: 0,
        send_delay_ms: 0,
        ports_range: 0,
        ports_random: 0,
        listen_random: 0,
    };
    let v_behavior = Behavior {
        role: if c_is_receiver {
            Role::Sender
        } else {
            Role::Receiver
        },
        ..c_behavior
    };

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
    }
}

/// Check the received legacy plan against both sides' observations and the table.
pub fn validate_plan(plan: &Plan, c_wire: &SnapshotWire, v_wire: &SnapshotWire) -> io::Result<()> {
    if let (Some(c), Some(v)) = (c_wire.view(), v_wire.view()) {
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
        let expected = basic_plan(c_wire, v_wire);
        if *plan != expected {
            return Err(invalid("invalid NAT fallback"));
        }
    }
    Ok(())
}

/// A radius is converted using the last sample; it is never a next-port guess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhasePlan {
    /// Local role and frp parameters.
    pub behavior: Behavior,
    /// Explicit addresses eligible for the regular send phase.
    pub explicit: Vec<SocketAddr>,
    /// Peer mappings used to derive range and random destinations.
    pub candidates: Vec<SocketAddr>,
    /// Inclusive port range to try on the mapped IPs.
    pub range: Option<(u16, u16)>,
    /// Original frp phase duration, including the send delay.
    pub total_budget_ms: u32,
    /// Original frp receive window following the send delay.
    pub read_budget_ms: u32,
}
impl PhasePlan {
    /// Construct one phase from its behavior and the peer's evidence.
    pub fn new(
        behavior: Behavior,
        peer_delay_ms: u32,
        mapped: &[SocketAddr],
        assisted: &[SocketAddr],
        difference: u16,
    ) -> Self {
        let range = mapped
            .last()
            .filter(|_| behavior.ports_range > 0)
            .map(|last| {
                let p = i32::from(last.port());
                let d = i32::from(difference);
                let r = i32::from(behavior.ports_range);
                (
                    (p - d - 5).max(p - r).max(1) as u16,
                    (p + d + 5).min(p + r).min(65535) as u16,
                )
            });
        let mut candidates = mapped.to_vec();
        candidates.dedup();
        let mut explicit = match behavior.role {
            Role::Sender => assisted.iter().chain(mapped).copied().collect::<Vec<_>>(),
            Role::Receiver if range.is_none() => mapped.to_vec(),
            Role::Receiver => Vec::new(),
        };
        explicit.dedup();
        let random_phase = behavior.ports_random > 0 || behavior.listen_random > 0;
        // The old selector extends both partners' read timeout by 30s for
        // random sending/listening, on top of max delay + 5s.
        let total_budget_ms =
            behavior.send_delay_ms.max(peer_delay_ms) + 5000 + if random_phase { 30000 } else { 0 };
        Self {
            behavior,
            explicit,
            candidates,
            range,
            total_budget_ms,
            read_budget_ms: total_budget_ms - behavior.send_delay_ms,
        }
    }
    /// IP order and adjacent-only compaction retain legacy parseIPs semantics.
    pub fn range_targets(&self) -> Vec<SocketAddr> {
        let Some((from, to)) = self.range else {
            return Vec::new();
        };
        let mut ips = self
            .candidates
            .iter()
            .map(SocketAddr::ip)
            .collect::<Vec<IpAddr>>();
        ips.dedup();
        ips.into_iter()
            .flat_map(|ip| (from..=to).map(move |port| SocketAddr::new(ip, port)))
            .collect()
    }

    /// A conservative allowance for the complete phase, without shrinking any
    /// candidate, listener, repeat or random iteration. Wall time may stop it first.
    pub fn peer_datagram_bound(&self, physical_sockets: usize) -> usize {
        let repeats = 1 + (self.total_budget_ms as usize).div_ceil(200);
        let mut ips = self
            .candidates
            .iter()
            .map(SocketAddr::ip)
            .collect::<Vec<_>>();
        ips.dedup();
        physical_sockets
            * (self.explicit.len() * repeats
                + self.range_targets().len()
                + usize::from(self.behavior.ports_random) * ips.len()
                + 3)
    }
}

/// Maximum wire candidates/assistance are 64 each. This deliberately covers
/// the full 38s, 257 physical bindings and all literal legacy parameters.
/// It is an allowance, never a promise to transmit this many datagrams.
pub const DEFAULT_LEGACY_PEER_ALLOWANCE: usize = 257 * (128 * 191 + 64 * 21 + 1000 * 64 + 3);
