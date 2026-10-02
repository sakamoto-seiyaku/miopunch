//! Connection quality and explicit preference within the already qualified paths.
use iroh::endpoint::transports::{FourTuple, PathSelection, PathSelectionContext, PathSelector};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};

/// Preferences rank permitted paths; resource disables always remain decisive.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum PathPreference {
    #[default]
    Quality,
    PreferDirect,
    PreferRelay,
}

/// Bounded reevaluation of existing paths, separate from discovery/probe budgets.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PathSelectionPolicy {
    pub preference: PathPreference,
    /// One reevaluation per connection at this interval, without catch-up bursts.
    pub reevaluation_interval_ms: u64,
}
impl Default for PathSelectionPolicy {
    fn default() -> Self {
        Self {
            preference: PathPreference::Quality,
            reevaluation_interval_ms: 1000,
        }
    }
}
impl PathSelectionPolicy {
    pub(crate) fn selector(&self) -> std::io::Result<Arc<dyn PathSelector>> {
        if !(250..=30_000).contains(&self.reevaluation_interval_ms) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "path reevaluation interval must be 250..=30000 milliseconds",
            ));
        }
        Ok(Arc::new(QualityPathSelector(self.clone())))
    }
}

#[derive(Debug)]
struct QualityPathSelector(PathSelectionPolicy);
impl QualityPathSelector {
    fn tier(&self, path: &FourTuple) -> u8 {
        // A custom carrier is only known to be direct when it is the qualified
        // native TCP data carrier. Other custom carriers receive no such bias.
        let direct = path.is_ip()
            || matches!(path, FourTuple::Custom { remote, .. } if remote.id() == crate::tcp_transport::DATA_TRANSPORT_ID);
        match self.0.preference {
            PathPreference::Quality => 0,
            PathPreference::PreferDirect => u8::from(!direct),
            PathPreference::PreferRelay => u8::from(!path.is_relay()),
        }
    }
}
impl PathSelector for QualityPathSelector {
    fn reevaluation_interval(&self) -> Option<Duration> {
        Some(Duration::from_millis(self.0.reevaluation_interval_ms))
    }
    fn select(&self, context: &PathSelectionContext<'_>) -> PathSelection {
        let mut best = None;
        let mut current = None;
        for path in context.paths() {
            let Some(stats) = path.stats() else { continue };
            let key = (self.tier(path.network_path()), stats.rtt);
            if Some(path.network_path()) == context.current() {
                current = Some(key);
            }
            if best.as_ref().is_none_or(|(_, previous)| key < *previous) {
                best = Some((path, key));
            }
        }
        let mut selection = PathSelection::none();
        if let Some((path, key)) = best {
            // Keep iroh's five-millisecond hysteresis against jitter. Relay is
            // equal to direct by default; an explicit preference changes tiers.
            if current.is_none_or(|old| {
                key.0 < old.0
                    || (key.0 == old.0 && key.1.saturating_add(Duration::from_millis(5)) <= old.1)
            }) {
                selection.set(&path);
            }
        }
        selection
    }
}
