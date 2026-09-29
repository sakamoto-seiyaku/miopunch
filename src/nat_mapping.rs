//! Mapping evidence for choosing a bounded UDP punch strategy.
//!
//! A QAD observation reports a public mapping, not the peer's filter rules.
//! In particular, this module never infers APDF from a mapping sequence.

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
    Stable { address: SocketAddr },
    Sequential { last: SocketAddr, step: i16 },
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PunchRole {
    /// Old mode 1's predictable Hard side: open the peer filter from this socket.
    MappingSideSender,
    /// Old mode 1's stable Easy side: aim at the predicted mapping.
    StableSideReceiver,
}

/// A local policy ceiling, not an assertion about real NAT allocation.
pub const MAX_ABS_STEP: i16 = 2;
pub const MAX_EVIDENCE_AGE: Duration = Duration::from_secs(2);

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
/// A direction reversal, changed public IP, repeated destination, stale sample,
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
    if step == 0
        || step.unsigned_abs() > MAX_ABS_STEP as u32
        || deltas.iter().any(|delta| *delta != step)
    {
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
    fn rejects_reversal_old_or_reused_evidence() {
        let (mut samples, now) = samples([42031, 42033, 42035, 42034]);
        assert_eq!(classify(&samples, now), MappingPattern::Unknown);
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
    fn one_node_budget_cannot_be_refilled_by_a_second_attempt() {
        let budget = NodeProbeBudget::new(4, 1);
        assert!(budget.reserve_observations(4));
        assert!(!budget.reserve_observations(4));
        assert!(budget.spend_peer_datagram());
        assert!(!budget.spend_peer_datagram());
    }
}
