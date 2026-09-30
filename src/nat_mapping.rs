//! Frozen single-candidate experimental mapping policy for MIOPUNCH-56–59.
//!
//! A QAD observation reports a public mapping, not the peer's filter rules.
//! In particular, this module never infers APDF from a mapping sequence.
//! The formal legacy classifier and complete mode selector live in `nat_selector`.

use std::{
    collections::HashSet,
    net::SocketAddr,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

/// One result from an observer reached through the endpoint's QUIC UDP socket.
#[derive(Debug, Clone)]
pub struct MappingSample {
    pub observer: String,
    pub destination: SocketAddr,
    pub mapped: SocketAddr,
    pub observed_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappingPattern {
    Stable {
        address: SocketAddr,
    },
    Sequential {
        last: SocketAddr,
        step: i16,
    },
    /// Fresh, same-IP samples with inconsistent signed transitions. This is
    /// allocation evidence, never a claim that the next mapping is in the span.
    Irregular {
        last: SocketAddr,
        low: u16,
        high: u16,
    },
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PunchRole {
    /// Old mode 1's predictable Hard side: open the peer filter from this socket.
    MappingSideSender,
    /// Old mode 1's stable Easy side: aim at the predicted mapping.
    StableSideReceiver,
    /// Both mappings advance: open the local filter first, then wait for the
    /// other side before starting the QUIC path validation.
    DualFirst,
    /// Both mappings advance: send after DualFirst has opened its filter.
    DualSecond,
    /// The experimental irregular receiver primes before one peer draw.
    IrregularReceiver,
    /// The experimental stable sender samples one irregular peer port.
    RandomTargetSender,
    /// The experimental irregular receiver primes one regular prediction.
    IrregularRegularReceiver,
    /// The experimental regular sender samples one irregular peer port.
    RegularRandomTargetSender,
}

/// A local policy ceiling, not an assertion about real NAT allocation.
pub const MAX_ABS_STEP: i16 = 2;
pub const MAX_EVIDENCE_AGE: Duration = Duration::from_secs(2);
/// The controlled irregular allocator has sixteen available ports. Broader
/// spans have no measured local-window benefit in this slice.
pub const MAX_IRREGULAR_SAMPLE_SPAN: u16 = 15;

/// Share one instance across strategy attempts on a node. Failed operations
/// still consume their reservation, so retrying cannot refill the allowance.
#[derive(Debug)]
pub struct NodeProbeBudget {
    additional_qad: AtomicUsize,
    peer_datagrams: AtomicUsize,
}

impl NodeProbeBudget {
    pub fn new(additional_qad: usize, peer_datagrams: usize) -> Self {
        Self {
            additional_qad: AtomicUsize::new(additional_qad),
            peer_datagrams: AtomicUsize::new(peer_datagrams),
        }
    }

    pub fn reserve_observations(&self, count: usize) -> bool {
        self.additional_qad
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |left| {
                left.checked_sub(count)
            })
            .is_ok()
    }

    pub fn spend_peer_datagram(&self) -> bool {
        self.peer_datagrams
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |left| {
                left.checked_sub(1)
            })
            .is_ok()
    }
}

/// Require four ordered, independent destinations so three transitions agree.
/// A changed public IP, repeated destination, stale sample,
/// or invalid next port makes the result unknown rather than widening the scan.
pub fn classify(samples: &[MappingSample], now: Instant) -> MappingPattern {
    if samples.len() != 4 {
        return MappingPattern::Unknown;
    }
    let mut seen = HashSet::new();
    for (index, sample) in samples.iter().enumerate() {
        if sample.mapped.port() == 0
            || !sample.mapped.is_ipv4()
            || !seen.insert(sample.destination)
            || now
                .checked_duration_since(sample.observed_at)
                .is_none_or(|age| age > MAX_EVIDENCE_AGE)
            || (index > 0 && sample.observed_at <= samples[index - 1].observed_at)
        {
            return MappingPattern::Unknown;
        }
    }
    let first = samples[0].mapped;
    if samples
        .iter()
        .any(|sample| sample.mapped.ip() != first.ip())
    {
        return MappingPattern::Unknown;
    }
    let ports: Vec<i32> = samples
        .iter()
        .map(|sample| i32::from(sample.mapped.port()))
        .collect();
    let deltas: Vec<i32> = ports.windows(2).map(|pair| pair[1] - pair[0]).collect();
    if deltas.iter().all(|delta| *delta == 0) {
        return MappingPattern::Stable {
            address: samples[3].mapped,
        };
    }
    let step = deltas[0];
    if deltas.iter().any(|delta| *delta != step) {
        return MappingPattern::Irregular {
            last: samples[3].mapped,
            low: *ports.iter().min().unwrap() as u16,
            high: *ports.iter().max().unwrap() as u16,
        };
    }
    if step == 0 || step.unsigned_abs() > MAX_ABS_STEP as u32 {
        return MappingPattern::Unknown;
    }
    if !(1..=65535).contains(&(ports[3] + step)) {
        return MappingPattern::Unknown;
    }
    MappingPattern::Sequential {
        last: samples[3].mapped,
        step: step as i16,
    }
}

/// One random destination per round: fresh narrow allocation evidence allows
/// a local window hypothesis; broad evidence permits only one whole-port draw.
/// Neither Unknown nor a second socket provides permission for a scan.
pub fn irregular_window(pattern: MappingPattern) -> Option<(SocketAddr, u16, u16)> {
    irregular_window_with_padding(pattern, 4)
}

/// Keep the observed-only comparison under the same validity and span policy.
/// Only the measured zero/four-port padding arms are available.
pub fn irregular_window_with_padding(
    pattern: MappingPattern,
    padding: u16,
) -> Option<(SocketAddr, u16, u16)> {
    if !matches!(padding, 0 | 4) {
        return None;
    }
    let MappingPattern::Irregular { last, low, high } = pattern else {
        return None;
    };
    if !last.is_ipv4() || low == 0 || low >= high || !(low..=high).contains(&last.port()) {
        return None;
    }
    let (low, high) = if high - low <= MAX_IRREGULAR_SAMPLE_SPAN {
        (
            low.saturating_sub(padding).max(1),
            high.saturating_add(padding),
        )
    } else {
        (1, u16::MAX)
    };
    Some((last, low, high))
}

/// A single-candidate comparison inspired by old mode2/4 roles. The
/// irregular side submits its UDP prime before the protected signal permits
/// the peer's submission. A gateway can still delay/reorder WAN forwarding;
/// only the later QUIC validation establishes reachability. Unsupported pairs
/// stop without peer packets.
pub fn choose_irregular_pair(local: MappingPattern, remote: MappingPattern) -> Option<PunchRole> {
    use MappingPattern::{Irregular, Sequential, Stable};
    match (local, remote) {
        (Irregular { .. }, Stable { .. }) => Some(PunchRole::IrregularReceiver),
        (Stable { .. }, Irregular { .. }) => Some(PunchRole::RandomTargetSender),
        (Irregular { .. }, Sequential { .. }) => Some(PunchRole::IrregularRegularReceiver),
        (Sequential { .. }, Irregular { .. }) => Some(PunchRole::RegularRandomTargetSender),
        _ => None,
    }
}

pub fn predicted_peer_address(pattern: MappingPattern) -> Option<SocketAddr> {
    let MappingPattern::Sequential { last, step } = pattern else {
        return None;
    };
    let port = i32::from(last.port()) + i32::from(step);
    (1..=65535)
        .contains(&port)
        .then(|| SocketAddr::new(last.ip(), port as u16))
}

/// Retain the old Easy/Hard mode-1 role decision, with explicit evidence.
pub fn choose_single_sequential(
    local: MappingPattern,
    remote: MappingPattern,
) -> Option<PunchRole> {
    match (local, remote) {
        (MappingPattern::Sequential { .. }, MappingPattern::Stable { .. }) => {
            Some(PunchRole::MappingSideSender)
        }
        (MappingPattern::Stable { .. }, MappingPattern::Sequential { .. }) => {
            Some(PunchRole::StableSideReceiver)
        }
        _ => None,
    }
}

/// Keep bilateral ordering independent of which node has which port step.
/// The accepted connection's server sends first for either dialer role.
pub fn choose_regular_pair(
    local: MappingPattern,
    remote: MappingPattern,
    is_server: bool,
) -> Option<PunchRole> {
    if matches!(local, MappingPattern::Sequential { .. })
        && matches!(remote, MappingPattern::Sequential { .. })
    {
        Some(if is_server {
            PunchRole::DualFirst
        } else {
            PunchRole::DualSecond
        })
    } else {
        choose_single_sequential(local, remote)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples(ports: [u16; 4]) -> (Vec<MappingSample>, Instant) {
        let now = Instant::now();
        (
            ports
                .into_iter()
                .enumerate()
                .map(|(i, port)| MappingSample {
                    observer: format!("qad-{i}"),
                    destination: format!("100.64.27.1:{}", 46761 + i as u16).parse().unwrap(),
                    mapped: format!("100.64.27.2:{port}").parse().unwrap(),
                    observed_at: now - Duration::from_millis((4 - i) as u64),
                })
                .collect(),
            now,
        )
    }

    #[test]
    fn derives_stable_and_signed_steps_from_ordered_destinations() {
        for (ports, expected) in [
            (
                [42027; 4],
                MappingPattern::Stable {
                    address: "100.64.27.2:42027".parse().unwrap(),
                },
            ),
            (
                [42031, 42032, 42033, 42034],
                MappingPattern::Sequential {
                    last: "100.64.27.2:42034".parse().unwrap(),
                    step: 1,
                },
            ),
            (
                [42040, 42038, 42036, 42034],
                MappingPattern::Sequential {
                    last: "100.64.27.2:42034".parse().unwrap(),
                    step: -2,
                },
            ),
            (
                [42031, 42033, 42035, 42037],
                MappingPattern::Sequential {
                    last: "100.64.27.2:42037".parse().unwrap(),
                    step: 2,
                },
            ),
        ] {
            let (samples, now) = samples(ports);
            assert_eq!(classify(&samples, now), expected);
        }
    }

    #[test]
    fn fresh_irregular_evidence_is_distinct_from_missing_or_invalid_evidence() {
        let (samples, now) = samples([42031, 42039, 42035, 42037]);
        assert!(matches!(
            classify(&samples, now),
            MappingPattern::Irregular { .. }
        ));
        assert_eq!(classify(&samples[..3], now), MappingPattern::Unknown);
    }

    #[test]
    fn distinguishes_reversal_from_old_or_reused_evidence() {
        let (mut samples, now) = samples([42031, 42033, 42035, 42034]);
        assert!(matches!(
            classify(&samples, now),
            MappingPattern::Irregular { .. }
        ));
        let (too_wide, now) = self::samples([42031, 42034, 42037, 42040]);
        assert_eq!(classify(&too_wide, now), MappingPattern::Unknown);
        samples[3].mapped.set_port(42037);
        samples[3].destination = samples[2].destination;
        assert_eq!(classify(&samples, now), MappingPattern::Unknown);
        samples[3].destination = "100.64.27.1:46764".parse().unwrap();
        samples[0].observed_at = now - MAX_EVIDENCE_AGE - Duration::from_millis(1);
        assert_eq!(classify(&samples, now), MappingPattern::Unknown);
    }

    #[test]
    fn selects_old_mode_one_roles_only_for_one_regular_and_one_stable() {
        let stable = MappingPattern::Stable {
            address: "100.64.27.6:43027".parse().unwrap(),
        };
        let sequential = MappingPattern::Sequential {
            last: "100.64.27.2:42037".parse().unwrap(),
            step: 2,
        };
        assert_eq!(
            choose_single_sequential(sequential, stable),
            Some(PunchRole::MappingSideSender)
        );
        assert_eq!(
            choose_single_sequential(stable, sequential),
            Some(PunchRole::StableSideReceiver)
        );
        assert_eq!(choose_single_sequential(stable, stable), None);
        assert_eq!(
            predicted_peer_address(sequential),
            Some("100.64.27.2:42039".parse().unwrap())
        );
    }

    #[test]
    fn irregular_roles_and_targets_require_fresh_allocation_evidence() {
        let (samples, now) = samples([42031, 42039, 42035, 42037]);
        let irregular = classify(&samples, now);
        let stable = MappingPattern::Stable {
            address: "100.64.27.6:43027".parse().unwrap(),
        };
        let regular = MappingPattern::Sequential {
            last: "100.64.27.6:43037".parse().unwrap(),
            step: 2,
        };
        assert_eq!(
            choose_irregular_pair(irregular, stable),
            Some(PunchRole::IrregularReceiver)
        );
        assert_eq!(
            choose_irregular_pair(stable, irregular),
            Some(PunchRole::RandomTargetSender)
        );
        assert_eq!(
            choose_irregular_pair(regular, irregular),
            Some(PunchRole::RegularRandomTargetSender)
        );
        assert_eq!(
            choose_irregular_pair(irregular, regular),
            Some(PunchRole::IrregularRegularReceiver)
        );
        assert_eq!(choose_irregular_pair(irregular, irregular), None);
        assert_eq!(
            choose_irregular_pair(MappingPattern::Unknown, regular),
            None
        );
        assert_eq!(choose_irregular_pair(regular, regular), None);
        assert_eq!(choose_irregular_pair(stable, stable), None);
        assert_eq!(
            irregular_window(irregular).map(|(_, low, high)| (low, high)),
            Some((42027, 42043))
        );
        assert_eq!(irregular_window(MappingPattern::Unknown), None);
        assert_eq!(
            irregular_window_with_padding(irregular, 0).map(|(_, low, high)| (low, high)),
            Some((42031, 42039))
        );
        assert_eq!(irregular_window_with_padding(irregular, 5), None);
        let broader = MappingPattern::Irregular {
            last: "100.64.27.2:42039".parse().unwrap(),
            low: 42031,
            high: 42047,
        };
        assert_eq!(
            irregular_window(broader).map(|(_, low, high)| (low, high)),
            Some((1, u16::MAX))
        );
    }

    #[test]
    fn bilateral_regular_branch_is_distinct_from_stable_and_single_regular() {
        let stable = MappingPattern::Stable {
            address: "100.64.27.6:43027".parse().unwrap(),
        };
        let regular = MappingPattern::Sequential {
            last: "100.64.27.2:42037".parse().unwrap(),
            step: 2,
        };
        assert_eq!(
            choose_regular_pair(regular, regular, true),
            Some(PunchRole::DualFirst)
        );
        assert_eq!(
            choose_regular_pair(regular, regular, false),
            Some(PunchRole::DualSecond)
        );
        assert_eq!(
            choose_regular_pair(regular, stable, true),
            Some(PunchRole::MappingSideSender)
        );
        assert_eq!(
            choose_regular_pair(stable, regular, false),
            Some(PunchRole::StableSideReceiver)
        );
        assert_eq!(choose_regular_pair(stable, stable, true), None);
        assert_eq!(
            choose_regular_pair(MappingPattern::Unknown, regular, true),
            None
        );
    }

    #[test]
    fn one_node_budget_cannot_be_refilled_by_a_second_attempt() {
        let budget = NodeProbeBudget::new(4, 1);
        assert!(budget.reserve_observations(4));
        assert!(!budget.reserve_observations(4));
        assert!(budget.spend_peer_datagram());
        assert!(!budget.spend_peer_datagram());
    }

    #[test]
    fn bilateral_first_side_cannot_spend_a_third_datagram() {
        let budget = NodeProbeBudget::new(4, 2);
        assert!(budget.reserve_observations(4));
        assert!(budget.spend_peer_datagram());
        assert!(budget.spend_peer_datagram());
        assert!(!budget.spend_peer_datagram());
        assert!(!budget.reserve_observations(4));
    }
}
