//! Whole connection and active path-building budgets shared by both platforms.
//! Stages consume an existing absolute caller deadline; persisted shorter policy
//! values remain explicit choices and are never migrated by these defaults.
use std::time::Duration;

pub(crate) const DEFAULT_MS: u64 = 60_000;
pub(crate) const MAX_MS: u64 = 300_000;
pub(crate) const DEFAULT: Duration = Duration::from_millis(DEFAULT_MS);
pub(crate) const MAX: Duration = Duration::from_millis(MAX_MS);

/// Bound a handshake stage by the caller's remaining clock. Incoming exchanges
/// without local caller metadata retain a bounded fallback, not a new caller clock.
pub(crate) fn remaining(deadline: Option<n0_future::time::Instant>) -> Duration {
    deadline.map_or(DEFAULT, |deadline| {
        MAX.min(deadline.saturating_duration_since(n0_future::time::Instant::now()))
    })
}
