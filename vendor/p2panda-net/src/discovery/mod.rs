// SPDX-License-Identifier: MIT OR Apache-2.0

//! Confidential topic discovery and random-walk strategy to resolve transport infos.
mod actors;
mod api;
mod backoff;
mod builder;
mod config;
mod events;
#[cfg(feature = "supervisor")]
mod supervisor;
#[cfg(test)]
mod tests;

pub use actors::DiscoveryMetrics;
pub use api::{Discovery, DiscoveryError};
/// Reuse discovery retry pacing when the application supplies its own admitted
/// session protocol rather than running a second set of discovery actors.
pub use backoff::{Backoff as DiscoveryBackoff, Config as DiscoveryBackoffConfig};
pub use builder::Builder;
pub use config::DiscoveryConfig;
pub use events::{DiscoveryEvent, SessionRole};
