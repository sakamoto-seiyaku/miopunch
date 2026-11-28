//! Node UDP configuration and live permission shared with member fusion.
mod control;
pub use control::{ControlUdpTraversal, UdpUpgrade};
pub(crate) use control::{PreparedUdpPath, RoundState};

use crate::network_maintenance::MaintenancePolicy;
use iroh::RelayMap;
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

fn round_interval(policy: &MaintenancePolicy) -> Duration {
    Duration::from_millis(policy.enhanced_udp_cooldown_ms)
}

/// Local permission and maintenance ceilings. They never shrink a legacy plan.
#[derive(Clone, Debug)]
pub struct UdpPolicy {
    pub enabled: bool,
    pub additional_observations: usize,
    pub peer_datagrams: usize,
    pub max_rounds: u8,
}

impl Default for UdpPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            additional_observations: 64,
            peer_datagrams: iroh::endpoint::nat_plan::DEFAULT_LEGACY_PEER_ALLOWANCE,
            max_rounds: 1,
        }
    }
}

/// Endpoint snapshots share the node's live UDP permission.
#[derive(Clone)]
pub struct NatConnector {
    observers: Option<RelayMap>,
    stun_observers: Vec<iroh::RelayUrl>,
    observation_limit: usize,
    peer_datagrams: usize,
    enabled: Arc<AtomicBool>,
    resource_enabled: bool,
    max_rounds: u8,
    round_interval: Duration,
}

impl NatConnector {
    pub fn new(observers: Option<RelayMap>, policy: UdpPolicy) -> Self {
        Self {
            observers,
            stun_observers: Vec::new(),
            observation_limit: policy
                .additional_observations
                .min(iroh::endpoint::nat_plan::MAX_OBSERVATIONS),
            peer_datagrams: policy.peer_datagrams,
            enabled: Arc::new(AtomicBool::new(policy.enabled)),
            resource_enabled: true,
            max_rounds: policy.max_rounds.min(32),
            round_interval: round_interval(&MaintenancePolicy::default()),
        }
    }

    /// Use the node's validated enhanced round cooldown for this connector.
    pub fn with_maintenance_policy(mut self, policy: &MaintenancePolicy) -> io::Result<Self> {
        policy.validate()?;
        self.round_interval = round_interval(policy);
        Ok(self)
    }

    pub fn set_udp_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Release);
    }

    /// Bind a resource snapshot to the same live node permission.
    pub(crate) fn for_endpoint(
        &self,
        observers: Option<RelayMap>,
        stun_observers: Vec<iroh::RelayUrl>,
        enabled: bool,
        maintenance: &MaintenancePolicy,
    ) -> io::Result<Self> {
        Self {
            observers,
            stun_observers,
            resource_enabled: enabled,
            ..self.clone()
        }
        .with_maintenance_policy(maintenance)
    }

    pub(crate) fn udp_enabled(&self) -> bool {
        self.resource_enabled && self.enabled.load(Ordering::Acquire)
    }

    pub(crate) fn fusion_permission(&self) -> Arc<AtomicBool> {
        self.enabled.clone()
    }

    pub(crate) fn fusion_config(
        &self,
        resources: &crate::resources::ResourceConfig,
    ) -> io::Result<iroh::endpoint::MemberUdpConfig> {
        use crate::resources::ResourceUse;

        let qad = match &self.observers {
            Some(observers) => observers.clone(),
            None => resources.relay_map(ResourceUse::Observation)?,
        };
        let stun = resources.udp_stun_resources();
        let sources_disabled = qad.is_empty()
            && stun.is_empty()
            && resources
                .replacement
                .as_ref()
                .unwrap_or(&resources.defaults.resources)
                .iter()
                .any(|resource| {
                    resource.uses.contains(&ResourceUse::Observation)
                        && resource
                            .url
                            .parse::<iroh::RelayUrl>()
                            .is_ok_and(|url| matches!(url.scheme(), "http" | "https" | "stun"))
                });
        Ok(iroh::endpoint::MemberUdpConfig {
            enabled: self.max_rounds != 0
                && !resources.udp_direct_disabled
                && !resources.disabled_uses.contains(&ResourceUse::Observation)
                && !sources_disabled,
            qad,
            stun,
            observations: self.observation_limit,
            peer_datagrams: self.peer_datagrams,
            round_timeout: Duration::from_millis(resources.maintenance.contact_timeout_ms)
                .min(crate::connection_budget::MAX),
            round_interval: round_interval(&resources.maintenance),
            quality_interval: Duration::from_millis(resources.maintenance.quality_interval_ms),
        })
    }
}
