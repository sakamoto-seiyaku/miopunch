//! Legacy NAT features and ordered learning selector.
//!
//! A mapping observation says nothing about the peer's filtering rules.
//! Observation lifetime and membership belong to the adapters, not this classifier.

use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NatType {
    Easy,
    Hard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MappingBehavior {
    NoChange,
    IpChanged,
    PortChanged,
    BothChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NatFeature {
    pub nat_type: NatType,
    pub behavior: MappingBehavior,
    pub ports_difference: u16,
    pub regular_ports_change: bool,
    pub public_network: bool,
}

/// Classifies valid observations in their original request order. Two identical
/// mapped addresses from different observers are sufficient for Easy NAT.
pub fn classify(addresses: &[SocketAddr], local_ips: &[IpAddr]) -> Option<NatFeature> {
    let first = *addresses.first()?;
    if addresses.len() < 2
        || addresses
            .iter()
            .any(|a| a.port() == 0 || a.ip().is_unspecified())
    {
        return None;
    }
    let ip_changed = addresses.iter().any(|a| a.ip() != first.ip());
    let port_changed = addresses.iter().any(|a| a.port() != first.port());
    let behavior = match (ip_changed, port_changed) {
        (false, false) => MappingBehavior::NoChange,
        (true, false) => MappingBehavior::IpChanged,
        (false, true) => MappingBehavior::PortChanged,
        (true, true) => MappingBehavior::BothChanged,
    };
    let ports_difference = if behavior == MappingBehavior::PortChanged {
        addresses.iter().map(SocketAddr::port).max()?
            - addresses.iter().map(SocketAddr::port).min()?
    } else {
        0
    };
    Some(NatFeature {
        nat_type: if behavior == MappingBehavior::NoChange {
            NatType::Easy
        } else {
            NatType::Hard
        },
        behavior,
        ports_difference,
        regular_ports_change: (1..=5).contains(&ports_difference),
        public_network: addresses.iter().any(|a| local_ips.contains(&a.ip())),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct BehaviorId {
    pub mode: u8,
    pub index: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Role {
    Sender,
    Receiver,
}

/// TTL zero means the socket default. Range is the legacy radius parameter,
/// not a port count; random/listen retain 1000 iterations/256 bind attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Behavior {
    pub role: Role,
    pub ttl: u8,
    pub send_delay_ms: u32,
    pub ports_range: u16,
    pub ports_random: u16,
    pub listen_random: u16,
}

const fn behavior(
    role: Role,
    ttl: u8,
    delay: u32,
    range: u16,
    random: u16,
    listen: u16,
) -> Behavior {
    Behavior {
        role,
        ttl,
        send_delay_ms: delay,
        ports_range: range,
        ports_random: random,
        listen_random: listen,
    }
}

use Role::{Receiver as R, Sender as S};
const MODE0: [(Behavior, Behavior); 12] = [
    (behavior(S, 0, 200, 0, 0, 0), behavior(R, 7, 0, 0, 0, 0)),
    (behavior(S, 0, 0, 0, 0, 0), behavior(R, 7, 0, 0, 0, 0)),
    (behavior(R, 7, 0, 0, 0, 0), behavior(S, 0, 200, 0, 0, 0)),
    (behavior(R, 7, 0, 0, 0, 0), behavior(S, 0, 0, 0, 0, 0)),
    (behavior(S, 0, 0, 0, 0, 0), behavior(R, 4, 0, 0, 0, 0)),
    (behavior(R, 4, 0, 0, 0, 0), behavior(S, 0, 0, 0, 0, 0)),
    (behavior(S, 0, 0, 0, 0, 0), behavior(R, 0, 0, 0, 0, 0)),
    (behavior(R, 0, 0, 0, 0, 0), behavior(S, 0, 0, 0, 0, 0)),
    (behavior(S, 0, 5000, 0, 0, 0), behavior(R, 0, 0, 0, 0, 0)),
    (behavior(S, 0, 10000, 0, 0, 0), behavior(R, 0, 0, 0, 0, 0)),
    (behavior(R, 0, 0, 0, 0, 0), behavior(S, 0, 5000, 0, 0, 0)),
    (behavior(R, 0, 0, 0, 0, 0), behavior(S, 0, 10000, 0, 0, 0)),
];
const MODE1: [(Behavior, Behavior); 6] = [
    (behavior(S, 0, 0, 0, 0, 0), behavior(R, 7, 0, 10, 0, 0)),
    (behavior(S, 0, 2000, 0, 0, 0), behavior(R, 7, 0, 10, 0, 0)),
    (behavior(S, 0, 0, 0, 0, 0), behavior(R, 4, 0, 10, 0, 0)),
    (behavior(S, 0, 2000, 0, 0, 0), behavior(R, 4, 0, 10, 0, 0)),
    (behavior(S, 0, 0, 0, 0, 0), behavior(R, 0, 0, 10, 0, 0)),
    (behavior(S, 0, 2000, 0, 0, 0), behavior(R, 0, 0, 10, 0, 0)),
];
const MODE2: [(Behavior, Behavior); 3] = [
    (
        behavior(S, 0, 3000, 0, 1000, 0),
        behavior(R, 7, 0, 0, 0, 256),
    ),
    (
        behavior(S, 0, 3000, 0, 1000, 0),
        behavior(R, 4, 0, 0, 0, 256),
    ),
    (
        behavior(S, 0, 3000, 0, 1000, 0),
        behavior(R, 0, 0, 0, 0, 256),
    ),
];
const MODE3: [(Behavior, Behavior); 6] = [
    (behavior(S, 0, 0, 10, 0, 0), behavior(R, 7, 0, 10, 0, 0)),
    (behavior(S, 0, 0, 10, 0, 0), behavior(R, 4, 0, 10, 0, 0)),
    (behavior(S, 0, 0, 10, 0, 0), behavior(R, 0, 0, 10, 0, 0)),
    (behavior(R, 7, 0, 10, 0, 0), behavior(S, 0, 0, 10, 0, 0)),
    (behavior(R, 4, 0, 10, 0, 0), behavior(S, 0, 0, 10, 0, 0)),
    (behavior(R, 0, 0, 10, 0, 0), behavior(S, 0, 0, 10, 0, 0)),
];
const MODE4: [(Behavior, Behavior); 3] = [
    (
        behavior(S, 0, 3000, 0, 1000, 0),
        behavior(R, 7, 0, 2, 0, 256),
    ),
    (
        behavior(S, 0, 3000, 0, 1000, 0),
        behavior(R, 4, 0, 2, 0, 256),
    ),
    (
        behavior(S, 0, 3000, 0, 1000, 0),
        behavior(R, 0, 0, 2, 0, 256),
    ),
];

fn behavior_table(mode: u8) -> Option<&'static [(Behavior, Behavior)]> {
    match mode {
        0 => Some(&MODE0),
        1 => Some(&MODE1),
        2 => Some(&MODE2),
        3 => Some(&MODE3),
        4 => Some(&MODE4),
        _ => None,
    }
}

pub fn behaviors(id: BehaviorId, c: NatFeature, _v: NatFeature) -> Option<(Behavior, Behavior)> {
    let table = behavior_table(id.mode)?;
    let (c_behavior, v_behavior) = *table.get(usize::from(id.index))?;
    let swap = match id.mode {
        1 => c.nat_type == NatType::Easy,
        2 => c.nat_type == NatType::Hard,
        4 => !c.regular_ports_change,
        _ => false,
    };
    // v is deliberately not consulted: the old role adjustment tests c only,
    // including fallback modes in a cached scene.
    Some(if swap {
        (v_behavior, c_behavior)
    } else {
        (c_behavior, v_behavior)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BehaviorScore {
    pub id: BehaviorId,
    pub score: i64,
}

/// Original scene insertion order and initial bias, before any learning.
pub fn initial_scores(c: NatFeature, v: NatFeature) -> Vec<BehaviorScore> {
    Records::new(c, v, Instant::now()).scores
}

#[derive(Debug)]
struct Records {
    scores: Vec<BehaviorScore>,
    last_update: Instant,
}

impl Records {
    fn new(c: NatFeature, v: NatFeature, now: Instant) -> Self {
        let hard = u8::from(c.nat_type == NatType::Hard) + u8::from(v.nat_type == NatType::Hard);
        let regular = u8::from(c.regular_ports_change) + u8::from(v.regular_ports_change);
        let modes: &[u8] = match (hard, regular) {
            (0, _) => &[0],
            (1, 1) => &[1, 2, 0],
            (1, 0) => &[2, 1, 0],
            (2, 2) => &[3, 4],
            (2, 1) => &[4],
            _ => &[0, 1, 3],
        };
        let mut scores = Vec::new();
        for &mode in modes {
            let table = behavior_table(mode).expect("scene contains only legacy modes");
            for (index, (c_behavior, _)) in table.iter().enumerate() {
                let score = if hard == 2 && regular == 0 {
                    1
                } else if mode == 0 && c.public_network {
                    i64::from(c_behavior.role == Role::Receiver)
                } else if mode == 0 && v.public_network {
                    i64::from(c_behavior.role == Role::Sender)
                } else {
                    0
                };
                scores.push(BehaviorScore {
                    id: BehaviorId {
                        mode,
                        index: index as u8,
                    },
                    score,
                });
            }
        }
        Self {
            scores,
            last_update: now,
        }
    }

    fn recommend(&mut self, now: Instant) -> BehaviorId {
        let mut best = 0;
        for i in 1..self.scores.len() {
            if self.scores[i].score > self.scores[best].score {
                best = i;
            }
        }
        self.scores[best].score -= 1;
        self.last_update = now;
        self.scores[best].id
    }

    fn report_success(&mut self, id: BehaviorId, now: Instant) {
        self.last_update = now;
        if let Some(item) = self.scores.iter_mut().find(|s| s.id == id) {
            item.score = (item.score + 2).min(10);
        }
    }
}

/// Original mapped order is retained even if an execution adapter compacts
/// candidates. The first IP belongs to the directed analysis key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureView {
    mapped: Vec<SocketAddr>,
    feature: NatFeature,
}

impl FeatureView {
    pub fn new(mapped: Vec<SocketAddr>, local_ips: &[IpAddr]) -> Option<Self> {
        let feature = classify(&mapped, local_ips)?;
        Some(Self { mapped, feature })
    }

    pub fn mapped(&self) -> &[SocketAddr] {
        &self.mapped
    }
    pub fn feature(&self) -> NatFeature {
        self.feature
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProbeProtocol {
    Udp,
    Tcp,
}

/// Obtained from the current authenticated admission gate by the runtime.
/// This key carries no authority to open a path or disclose observations.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LearningScope {
    pub local: [u8; 32],
    pub remote: [u8; 32],
    pub network: [u8; 32],
    pub secret_generation: u64,
    pub credential: [u8; 32],
    pub authorization_epoch: u64,
    pub protocol: ProbeProtocol,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FeatureKey {
    first_ip: IpAddr,
    nat_type: NatType,
    behavior: MappingBehavior,
    regular: bool,
}

impl From<&FeatureView> for FeatureKey {
    fn from(view: &FeatureView) -> Self {
        Self {
            first_ip: view.mapped[0].ip(),
            nat_type: view.feature.nat_type,
            behavior: view.feature.behavior,
            regular: view.feature.regular_ports_change,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AnalysisKey {
    scope: LearningScope,
    c: FeatureKey,
    v: FeatureKey,
}

#[derive(Debug, Clone)]
pub struct Recommendation {
    pub id: BehaviorId,
    pub c: Behavior,
    pub v: Behavior,
    /// Candidate scores immediately before recommendation's one decrement.
    pub scores_before: Vec<BehaviorScore>,
    key: AnalysisKey,
}

impl Recommendation {
    pub fn scope(&self) -> &LearningScope {
        &self.key.scope
    }
}

pub const NATIVE_RETENTION: Duration = Duration::from_secs(2 * 60 * 60);
pub const CLEAN_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// Share one analyzer for the node's lifetime. Every call recommends exactly
/// one behavior; failure needs no second decrement or inner retry loop.
#[derive(Debug)]
pub struct Analyzer {
    records: HashMap<AnalysisKey, Records>,
    last_clean: Instant,
}

impl Analyzer {
    pub fn new(now: Instant) -> Self {
        Self {
            records: HashMap::new(),
            last_clean: now,
        }
    }

    pub fn recommend(
        &mut self,
        scope: LearningScope,
        c: &FeatureView,
        v: &FeatureView,
        now: Instant,
    ) -> Recommendation {
        if now.saturating_duration_since(self.last_clean) >= CLEAN_INTERVAL {
            self.clean(now);
        }
        let key = AnalysisKey {
            scope,
            c: c.into(),
            v: v.into(),
        };
        let records = self
            .records
            .entry(key.clone())
            .or_insert_with(|| Records::new(c.feature, v.feature, now));
        let scores_before = records.scores.clone();
        let id = records.recommend(now);
        let (c, v) = behaviors(id, c.feature, v.feature).expect("stored behavior exists");
        Recommendation {
            id,
            c,
            v,
            scores_before,
            key,
        }
    }

    /// Runtime callers must first verify the same connection's committed data
    /// path, business receipt and current scope. Raw probe receipts cannot call
    /// this crate-private feedback entry point.
    pub(crate) fn report_success(&mut self, recommendation: &Recommendation, now: Instant) -> bool {
        if let Some(records) = self.records.get_mut(&recommendation.key) {
            records.report_success(recommendation.id, now);
            true
        } else {
            false
        }
    }

    pub fn clean(&mut self, now: Instant) -> (usize, usize) {
        let total = self.records.len();
        self.records
            .retain(|_, r| now.saturating_duration_since(r.last_update) <= NATIVE_RETENTION);
        self.last_clean = now;
        (total, total - self.records.len())
    }

    /// Purge on a known revocation or network switch, independent of expiry.
    pub fn forget_scope(&mut self, scope: &LearningScope) {
        self.records.retain(|key, _| &key.scope != scope);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_regular_feature_uses_spread_instead_of_signed_steps() {
        let addresses =
            [41000, 41002, 41001].map(|p| SocketAddr::new("192.0.2.1".parse().unwrap(), p));
        let feature = classify(&addresses, &[]).unwrap();
        assert_eq!(feature.nat_type, NatType::Hard);
        assert_eq!(feature.behavior, MappingBehavior::PortChanged);
        assert_eq!(feature.ports_difference, 2);
        assert!(feature.regular_ports_change);
    }

    #[test]
    fn easy_pair_recommends_every_behavior_in_insertion_order_before_repeating() {
        let easy = classify(&["192.0.2.1:41000".parse().unwrap(); 2], &[]).unwrap();
        let now = Instant::now();
        let mut records = Records::new(easy, easy, now);
        let actual: Vec<_> = (0..24).map(|_| records.recommend(now)).collect();
        let expected: Vec<_> = (0..2)
            .flat_map(|_| (0..12).map(|index| BehaviorId { mode: 0, index }))
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn cache_reuses_port_changes_and_expires_only_after_two_idle_hours() {
        let now = Instant::now();
        let c = FeatureView::new(vec!["192.0.2.1:41000".parse().unwrap(); 2], &[]).unwrap();
        let moved = FeatureView::new(vec!["192.0.2.1:42000".parse().unwrap(); 2], &[]).unwrap();
        let v = FeatureView::new(vec!["192.0.2.2:43000".parse().unwrap(); 2], &[]).unwrap();
        let scope = LearningScope {
            local: [1; 32],
            remote: [2; 32],
            network: [3; 32],
            secret_generation: 1,
            credential: [4; 32],
            authorization_epoch: 1,
            protocol: ProbeProtocol::Udp,
        };
        let mut analyzer = Analyzer::new(now);
        let first = analyzer.recommend(scope.clone(), &c, &v, now);
        let second = analyzer.recommend(scope, &moved, &v, now);
        assert_eq!(first.id, BehaviorId { mode: 0, index: 0 });
        assert_eq!(second.id, BehaviorId { mode: 0, index: 1 });
        assert_eq!(analyzer.clean(now + Duration::from_secs(7200)), (1, 0));
        assert_eq!(analyzer.clean(now + Duration::from_secs(7201)), (1, 1));
    }

    fn view(ip: &str, ports: &[u16], public: bool) -> FeatureView {
        let ip: IpAddr = ip.parse().unwrap();
        FeatureView::new(
            ports.iter().map(|&p| SocketAddr::new(ip, p)).collect(),
            if public {
                std::slice::from_ref(&ip)
            } else {
                &[]
            },
        )
        .unwrap()
    }

    fn scope() -> LearningScope {
        LearningScope {
            local: [1; 32],
            remote: [2; 32],
            network: [3; 32],
            secret_generation: 1,
            credential: [4; 32],
            authorization_epoch: 1,
            protocol: ProbeProtocol::Udp,
        }
    }

    #[test]
    fn classification_matches_legacy_boundaries_ip_changes_and_public_detection() {
        for (ports, behavior, span, regular) in [
            (vec![41000, 41000], MappingBehavior::NoChange, 0, false),
            (vec![41000, 41001], MappingBehavior::PortChanged, 1, true),
            (
                vec![41005, 41000, 41002],
                MappingBehavior::PortChanged,
                5,
                true,
            ),
            (
                vec![41000, 41002, 41004, 41006],
                MappingBehavior::PortChanged,
                6,
                false,
            ),
            (
                vec![41000, 41003, 41001],
                MappingBehavior::PortChanged,
                3,
                true,
            ),
        ] {
            let feature = view("192.0.2.1", &ports, true).feature;
            assert_eq!(
                (
                    feature.behavior,
                    feature.ports_difference,
                    feature.regular_ports_change,
                    feature.public_network
                ),
                (behavior, span, regular, true)
            );
        }
        for (other, behavior) in [
            ("198.51.100.2:41000", MappingBehavior::IpChanged),
            ("198.51.100.2:41001", MappingBehavior::BothChanged),
        ] {
            let feature = classify(
                &["192.0.2.1:41000".parse().unwrap(), other.parse().unwrap()],
                &["198.51.100.2".parse().unwrap()],
            )
            .unwrap();
            assert_eq!(
                (
                    feature.nat_type,
                    feature.behavior,
                    feature.ports_difference,
                    feature.regular_ports_change,
                    feature.public_network
                ),
                (NatType::Hard, behavior, 0, false, true)
            );
        }
        assert_eq!(classify(&[], &[]), None);
        assert_eq!(classify(&["192.0.2.1:41000".parse().unwrap()], &[]), None);
    }

    #[test]
    fn all_six_scenes_preserve_complete_order_for_two_cycles() {
        let easy = view("192.0.2.1", &[41000, 41000], false);
        let regular = view("192.0.2.2", &[42000, 42002, 42001], false);
        let irregular = view("192.0.2.3", &[43000, 43006], false);
        for (c, v, modes, initial) in [
            (&easy, &easy, vec![(0, 12)], 0),
            (&easy, &regular, vec![(1, 6), (2, 3), (0, 12)], 0),
            (&irregular, &easy, vec![(2, 3), (1, 6), (0, 12)], 0),
            (&regular, &regular, vec![(3, 6), (4, 3)], 0),
            (&regular, &irregular, vec![(4, 3)], 0),
            (&irregular, &irregular, vec![(0, 12), (1, 6), (3, 6)], 1),
        ] {
            let expected: Vec<_> = modes
                .iter()
                .flat_map(|&(mode, count)| (0..count).map(move |index| BehaviorId { mode, index }))
                .collect();
            let mut records = Records::new(c.feature, v.feature, Instant::now());
            assert!(records.scores.iter().all(|s| s.score == initial));
            assert_eq!(
                records.scores.iter().map(|s| s.id).collect::<Vec<_>>(),
                expected
            );
            let actual: Vec<_> = (0..expected.len() * 2)
                .map(|_| records.recommend(Instant::now()))
                .collect();
            assert_eq!(actual, [expected.clone(), expected].concat());
        }
    }

    #[test]
    fn public_preference_retains_c_priority_and_does_not_change_irregular_fallback() {
        let easy = view("192.0.2.1", &[41000, 41000], false).feature;
        let public = NatFeature {
            public_network: true,
            ..easy
        };
        for (c, v, first, ones) in [
            (public, easy, 2, vec![2, 3, 5, 7, 10, 11]),
            (easy, public, 0, vec![0, 1, 4, 6, 8, 9]),
            (public, public, 2, vec![2, 3, 5, 7, 10, 11]),
        ] {
            let mut records = Records::new(c, v, Instant::now());
            assert_eq!(
                records
                    .scores
                    .iter()
                    .filter(|s| s.score == 1)
                    .map(|s| s.id.index)
                    .collect::<Vec<_>>(),
                ones
            );
            assert_eq!(
                records.recommend(Instant::now()),
                BehaviorId {
                    mode: 0,
                    index: first
                }
            );
        }
        let irregular = view("192.0.2.1", &[41000, 41006], true).feature;
        let records = Records::new(irregular, irregular, Instant::now());
        assert!(records.scores.iter().all(|s| s.score == 1));
    }

    #[test]
    fn scoring_is_unbounded_downward_success_adds_two_caps_ten_and_refreshes_unknown_index() {
        let now = Instant::now();
        let easy = view("192.0.2.1", &[41000, 41000], false).feature;
        let mut records = Records::new(easy, easy, now);
        for _ in 0..12 * 15 {
            records.recommend(now);
        }
        assert!(records.scores.iter().all(|s| s.score == -15));
        let id = BehaviorId { mode: 0, index: 0 };
        records.report_success(id, now + Duration::from_secs(1));
        assert_eq!(records.scores[0].score, -13);
        for _ in 0..20 {
            records.report_success(id, now);
        }
        assert_eq!(records.scores[0].score, 10);
        let scores = records.scores.clone();
        records.report_success(
            BehaviorId { mode: 9, index: 99 },
            now + Duration::from_secs(20),
        );
        assert_eq!(records.scores, scores);
        assert_eq!(records.last_update, now + Duration::from_secs(20));
        assert_eq!(records.recommend(now), id);
        assert_eq!(records.scores[0].score, 9);
    }

    #[test]
    fn every_security_dimension_and_directed_feature_key_isolates_learning() {
        let now = Instant::now();
        let c = view("192.0.2.1", &[41000, 41000], false);
        let v = view("192.0.2.2", &[42000, 42000], false);
        let mut analyzer = Analyzer::new(now);
        let learned = analyzer.recommend(scope(), &c, &v, now);
        assert!(analyzer.report_success(&learned, now));
        for modified in [
            LearningScope {
                local: [9; 32],
                ..scope()
            },
            LearningScope {
                remote: [9; 32],
                ..scope()
            },
            LearningScope {
                network: [9; 32],
                ..scope()
            },
            LearningScope {
                secret_generation: 2,
                ..scope()
            },
            LearningScope {
                credential: [9; 32],
                ..scope()
            },
            LearningScope {
                authorization_epoch: 2,
                ..scope()
            },
            LearningScope {
                protocol: ProbeProtocol::Tcp,
                ..scope()
            },
        ] {
            assert_eq!(
                analyzer.recommend(modified, &c, &v, now).scores_before[0].score,
                0
            );
        }
        assert_eq!(
            analyzer.recommend(scope(), &v, &c, now).scores_before[0].score,
            0
        );
        let changed = view("192.0.2.1", &[41000, 41001], false);
        assert_eq!(analyzer.recommend(scope(), &changed, &v, now).id.mode, 1);
        analyzer.forget_scope(&scope());
        assert!(!analyzer.report_success(&learned, now));
        assert_eq!(
            analyzer.recommend(scope(), &c, &v, now).scores_before[0].score,
            0
        );
    }

    #[test]
    fn public_and_span_changes_reuse_existing_scene_and_opportunistic_clean_refreshes_activity() {
        let now = Instant::now();
        let c = view("192.0.2.1", &[41000, 41001], false);
        let changed = view("192.0.2.1", &[43000, 43005], true);
        let v = view("192.0.2.2", &[42000, 42000], false);
        let mut analyzer = Analyzer::new(now);
        let first = analyzer.recommend(scope(), &c, &v, now);
        let second = analyzer.recommend(scope(), &changed, &v, now + Duration::from_secs(100));
        assert_eq!(second.id, BehaviorId { mode: 1, index: 1 });
        assert!(
            second
                .scores_before
                .iter()
                .filter(|s| s.id.mode == 0)
                .all(|s| s.score == 0)
        );
        assert!(analyzer.report_success(&first, now + Duration::from_secs(7000)));
        assert_eq!(analyzer.clean(now + Duration::from_secs(7300)), (1, 0));
        analyzer.recommend(
            LearningScope {
                network: [9; 32],
                ..scope()
            },
            &c,
            &v,
            now + Duration::from_secs(15000),
        );
        assert!(!analyzer.report_success(&first, now + Duration::from_secs(15000)));
    }

    #[test]
    fn role_adjustment_uses_current_c_feature_without_changing_mode_index() {
        let easy = view("192.0.2.1", &[41000, 41000], false).feature;
        let regular = view("192.0.2.2", &[42000, 42001], false).feature;
        let irregular = view("192.0.2.3", &[43000, 43006], false).feature;
        for index in 0..6 {
            assert_eq!(
                behaviors(BehaviorId { mode: 1, index }, easy, regular)
                    .unwrap()
                    .0
                    .role,
                Role::Receiver
            );
            assert_eq!(
                behaviors(BehaviorId { mode: 1, index }, regular, easy)
                    .unwrap()
                    .0
                    .role,
                Role::Sender
            );
        }
        for index in 0..3 {
            assert_eq!(
                behaviors(BehaviorId { mode: 2, index }, regular, easy)
                    .unwrap()
                    .0
                    .role,
                Role::Receiver
            );
            assert_eq!(
                behaviors(BehaviorId { mode: 2, index }, easy, irregular)
                    .unwrap()
                    .0
                    .role,
                Role::Sender
            );
            assert_eq!(
                behaviors(BehaviorId { mode: 4, index }, irregular, regular)
                    .unwrap()
                    .0
                    .role,
                Role::Receiver
            );
            assert_eq!(
                behaviors(BehaviorId { mode: 4, index }, regular, regular)
                    .unwrap()
                    .0
                    .role,
                Role::Sender
            );
        }
    }

    #[test]
    fn all_thirty_parameter_pairs_match_the_executed_archived_go_oracle() {
        let reference: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/legacy_nat_behaviors.json"))
                .unwrap();
        let regular = view("192.0.2.1", &[41000, 41001], false).feature;
        let easy = view("192.0.2.2", &[42000, 42000], false).feature;
        let expected = reference.as_array().unwrap();
        assert_eq!(expected.len(), 30);
        for entry in expected {
            let id = BehaviorId {
                mode: entry["Mode"].as_u64().unwrap() as u8,
                index: entry["Index"].as_u64().unwrap() as u8,
            };
            // Table orientation before role swaps: mode2 needs Easy c;
            // mode1/4 need regular c. The pair itself is the old source oracle.
            let (c, v) = if id.mode == 2 {
                (easy, regular)
            } else {
                (regular, easy)
            };
            let (c, v) = behaviors(id, c, v).unwrap();
            for (actual, expected) in [(c, &entry["C"]), (v, &entry["V"])] {
                assert_eq!(
                    actual.role,
                    if expected["Role"] == "sender" {
                        Role::Sender
                    } else {
                        Role::Receiver
                    }
                );
                assert_eq!(u64::from(actual.ttl), expected["TTL"].as_u64().unwrap());
                assert_eq!(
                    u64::from(actual.send_delay_ms),
                    expected["SendDelayMs"].as_u64().unwrap()
                );
                assert_eq!(
                    u64::from(actual.ports_range),
                    expected["PortsRangeNumber"].as_u64().unwrap()
                );
                assert_eq!(
                    u64::from(actual.ports_random),
                    expected["PortsRandomNumber"].as_u64().unwrap()
                );
                assert_eq!(
                    u64::from(actual.listen_random),
                    expected["ListenRandomPorts"].as_u64().unwrap()
                );
            }
        }
    }
}
