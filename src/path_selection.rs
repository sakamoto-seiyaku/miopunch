//! Connection quality and explicit preference within the already qualified paths.
use iroh::endpoint::transports::{
    ConnectionPathSelector, FourTuple, PathSelection, PathSelectionContext,
};
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
    pub(crate) fn selector(&self) -> std::io::Result<Arc<dyn ConnectionPathSelector>> {
        self.selector_with_native_rtc(false)
    }

    /// Native builders pass actual RTC installation, rather than feature support.
    pub(crate) fn selector_with_native_rtc(
        &self,
        rtc_installed: bool,
    ) -> std::io::Result<Arc<dyn ConnectionPathSelector>> {
        let carriers = CarrierClasses::native(rtc_installed);
        self.selector_with_carriers(carriers)
    }

    pub(crate) fn selector_with_carriers(
        &self,
        carriers: CarrierClasses,
    ) -> std::io::Result<Arc<dyn ConnectionPathSelector>> {
        if !(250..=30_000).contains(&self.reevaluation_interval_ms) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "path reevaluation interval must be 250..=30000 milliseconds",
            ));
        }
        Ok(Arc::new(QualityPathSelector {
            policy: self.clone(),
            carriers,
        }))
    }
}

/// Kinds of installed custom carriers, never permission for an address or M.
/// The selector receives only paths already qualified by iroh for that M.
#[derive(Clone, Debug, Default)]
pub(crate) struct CarrierClasses {
    direct: Arc<[u64]>,
    relay: Arc<[u64]>,
}
impl CarrierClasses {
    pub(crate) fn new(
        direct: impl IntoIterator<Item = u64>,
        relay: impl IntoIterator<Item = u64>,
    ) -> Self {
        Self {
            direct: direct.into_iter().collect(),
            relay: relay.into_iter().collect(),
        }
    }

    pub(crate) fn native(rtc_installed: bool) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let carriers = Self::new(
            [crate::tcp_transport::DATA_TRANSPORT_ID],
            [crate::member_transport::DATA_TRANSPORT_ID],
        );
        #[cfg(target_arch = "wasm32")]
        let carriers = Self::default();
        #[cfg(all(not(target_arch = "wasm32"), feature = "rtc-probe"))]
        {
            let mut carriers = carriers;
            if rtc_installed {
                carriers.direct = carriers
                    .direct
                    .iter()
                    .copied()
                    .chain([crate::rtc_transport::TRANSPORT_ID])
                    .collect();
            }
            carriers
        }
        #[cfg(any(target_arch = "wasm32", not(feature = "rtc-probe")))]
        {
            let _ = rtc_installed;
            carriers
        }
    }

    fn tier(&self, path: &FourTuple, preference: PathPreference) -> u8 {
        match preference {
            PathPreference::Quality => 0,
            PathPreference::PreferDirect => u8::from(
                !(path.is_ip()
                    || matches!(path, FourTuple::Custom { remote, .. } if self.direct.contains(&remote.id()))),
            ),
            PathPreference::PreferRelay => u8::from(
                !(path.is_relay()
                    || matches!(path, FourTuple::Custom { remote, .. } if self.relay.contains(&remote.id()))),
            ),
        }
    }
}

/// The score uses measured RTT after carrier preference, with unchanged jitter
/// hysteresis. Keeping this value separate makes the same rule testable without
/// constructing or authorizing a synthetic connection path.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct PathScore(u8, Duration);
impl PathScore {
    pub(crate) fn new(
        path: &FourTuple,
        rtt: Duration,
        preference: PathPreference,
        carriers: &CarrierClasses,
    ) -> Self {
        Self(carriers.tier(path, preference), rtt)
    }

    pub(crate) fn should_replace(self, current: Option<Self>) -> bool {
        current.is_none_or(|old| {
            self.0 < old.0
                || (self.0 == old.0 && self.1.saturating_add(Duration::from_millis(5)) <= old.1)
        })
    }
}

#[derive(Debug)]
struct QualityPathSelector {
    policy: PathSelectionPolicy,
    carriers: CarrierClasses,
}
impl ConnectionPathSelector for QualityPathSelector {
    fn reevaluation_interval(&self) -> Option<Duration> {
        Some(Duration::from_millis(self.policy.reevaluation_interval_ms))
    }
    fn select(&self, context: &PathSelectionContext<'_>) -> PathSelection {
        let mut best = None;
        let mut current = None;
        for path in context.paths() {
            let Some(stats) = path.stats() else { continue };
            let key = PathScore::new(
                path.network_path(),
                stats.rtt,
                self.policy.preference,
                &self.carriers,
            );
            if context.is_current(&path) {
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
            if key.should_replace(current) {
                selection.set(&path);
            }
        }
        selection
    }
}
