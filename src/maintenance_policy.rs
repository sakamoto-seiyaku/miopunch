use serde::{Deserialize, Serialize};

/// This node's local effort, independent of membership or resource permission.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(from = "SavedPolicy")]
pub struct MaintenancePolicy {
    pub exploration_interval_ms: u64,
    pub discovery_interval_ms: u64,
    pub max_concurrent_contacts: usize,
    pub max_neighbors: usize,
    pub quality_interval_ms: u64,
    pub contact_timeout_ms: u64,
    /// Keep all healthy idle peers by default. Zero disables background standby.
    pub standby_member_limit: Option<usize>,
    pub enhanced_udp_cooldown_ms: u64,
    pub path_retry_base_ms: u64,
}
impl Default for MaintenancePolicy {
    fn default() -> Self {
        Self::conservative()
    }
}
impl MaintenancePolicy {
    pub fn conservative() -> Self {
        Self {
            exploration_interval_ms: 5_000,
            standby_member_limit: None,
            enhanced_udp_cooldown_ms: 160_000,
            path_retry_base_ms: 5_000,
            discovery_interval_ms: 15_000,
            max_concurrent_contacts: 1,
            max_neighbors: 1,
            quality_interval_ms: 5_000,
            contact_timeout_ms: crate::connection_budget::DEFAULT_MS,
        }
    }
    pub fn active() -> Self {
        Self {
            exploration_interval_ms: 1_000,
            standby_member_limit: None,
            enhanced_udp_cooldown_ms: 32_000,
            path_retry_base_ms: 1_000,
            discovery_interval_ms: 5_000,
            max_concurrent_contacts: 4,
            max_neighbors: 4,
            quality_interval_ms: 1_000,
            contact_timeout_ms: crate::connection_budget::DEFAULT_MS,
        }
    }
    pub(crate) fn validate(&self) -> std::io::Result<()> {
        if !(250..=60_000).contains(&self.exploration_interval_ms)
            || !(1_000..=120_000).contains(&self.discovery_interval_ms)
            || !(1..=8).contains(&self.max_concurrent_contacts)
            || !(1..=8).contains(&self.max_neighbors)
            || self.enhanced_udp_cooldown_ms == 0
            || self.enhanced_udp_cooldown_ms > 1_920_000
            || !(1..=60_000).contains(&self.path_retry_base_ms)
            || !(250..=30_000).contains(&self.quality_interval_ms)
            || !(100..=crate::connection_budget::MAX_MS).contains(&self.contact_timeout_ms)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "node maintenance budget is outside bounded limits",
            ));
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(default)]
struct SavedPolicy {
    exploration_interval_ms: u64,
    discovery_interval_ms: u64,
    max_concurrent_contacts: usize,
    max_neighbors: usize,
    quality_interval_ms: u64,
    contact_timeout_ms: u64,
    max_standby_member_connections: Option<usize>,
    #[serde(default, deserialize_with = "saved_standby_limit")]
    standby_member_limit: Option<Option<usize>>,
    enhanced_udp_cooldown_ms: Option<u64>,
    path_retry_base_ms: Option<u64>,
}

impl Default for SavedPolicy {
    fn default() -> Self {
        let old = MaintenancePolicy::conservative();
        Self {
            exploration_interval_ms: old.exploration_interval_ms,
            discovery_interval_ms: old.discovery_interval_ms,
            max_concurrent_contacts: old.max_concurrent_contacts,
            max_neighbors: old.max_neighbors,
            quality_interval_ms: old.quality_interval_ms,
            contact_timeout_ms: old.contact_timeout_ms,
            max_standby_member_connections: None,
            standby_member_limit: None,
            enhanced_udp_cooldown_ms: None,
            path_retry_base_ms: None,
        }
    }
}

impl From<SavedPolicy> for MaintenancePolicy {
    fn from(saved: SavedPolicy) -> Self {
        let active = Self::active();
        let profile = if saved.discovery_interval_ms == active.discovery_interval_ms
            && saved.max_concurrent_contacts == active.max_concurrent_contacts
            && saved.max_neighbors == active.max_neighbors
            && saved.quality_interval_ms == active.quality_interval_ms
            && saved.contact_timeout_ms == active.contact_timeout_ms
        {
            active
        } else {
            Self::conservative()
        };
        Self {
            exploration_interval_ms: saved.exploration_interval_ms,
            discovery_interval_ms: saved.discovery_interval_ms,
            max_concurrent_contacts: saved.max_concurrent_contacts,
            max_neighbors: saved.max_neighbors,
            quality_interval_ms: saved.quality_interval_ms,
            contact_timeout_ms: saved.contact_timeout_ms,
            // The old 1/4 values were persisted profile defaults, including on
            // nodes with independently tuned timing/budget fields. The new key
            // preserves explicit limits and lets null override a legacy value.
            standby_member_limit: saved.standby_member_limit.unwrap_or_else(|| {
                saved
                    .max_standby_member_connections
                    .filter(|limit| !matches!(limit, 1 | 4))
            }),
            enhanced_udp_cooldown_ms: saved
                .enhanced_udp_cooldown_ms
                .unwrap_or(profile.enhanced_udp_cooldown_ms),
            path_retry_base_ms: saved
                .path_retry_base_ms
                .unwrap_or(profile.path_retry_base_ms),
        }
    }
}

fn saved_standby_limit<'de, D>(value: D) -> Result<Option<Option<usize>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<usize>::deserialize(value).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_profiles_round_trip_and_migrate_independently_of_exploration() {
        for default in [
            MaintenancePolicy::active(),
            MaintenancePolicy::conservative(),
        ] {
            for interval in [1000, 30_000] {
                let policy = MaintenancePolicy {
                    exploration_interval_ms: interval,
                    ..default.clone()
                };
                let mut saved = serde_json::to_value(&policy).unwrap();
                for key in [
                    "standby_member_limit",
                    "enhanced_udp_cooldown_ms",
                    "path_retry_base_ms",
                ] {
                    saved.as_object_mut().unwrap().remove(key);
                }
                assert_eq!(
                    serde_json::from_value::<MaintenancePolicy>(saved.clone()).unwrap(),
                    policy
                );
                saved["path_retry_base_ms"] = 725.into();
                assert_eq!(
                    serde_json::from_value::<MaintenancePolicy>(saved)
                        .unwrap()
                        .path_retry_base_ms,
                    725
                );
            }
        }
        let config: MaintenancePolicy =
            serde_json::from_str(r#"{"max_neighbors":7,"exploration_interval_ms":30000}"#).unwrap();
        assert_eq!(config.standby_member_limit, None);
        assert_eq!(config.enhanced_udp_cooldown_ms, 160_000);
    }

    #[test]
    fn old_standby_defaults_migrate_without_changing_effort_or_explicit_new_limits() {
        for old_limit in [1, 4] {
            let mut saved = serde_json::to_value(MaintenancePolicy::active()).unwrap();
            saved
                .as_object_mut()
                .unwrap()
                .remove("standby_member_limit");
            saved["max_standby_member_connections"] = old_limit.into();
            saved["exploration_interval_ms"] = 30_000.into();
            saved["max_concurrent_contacts"] = 2.into();
            saved["enhanced_udp_cooldown_ms"] = 725.into();
            let migrated: MaintenancePolicy = serde_json::from_value(saved).unwrap();
            assert_eq!(migrated.standby_member_limit, None);
            assert_eq!(migrated.exploration_interval_ms, 30_000);
            assert_eq!(migrated.max_concurrent_contacts, 2);
            assert_eq!(migrated.enhanced_udp_cooldown_ms, 725);
        }
        for limit in [0, 2, 8] {
            let saved = serde_json::json!({"max_standby_member_connections": limit});
            let policy: MaintenancePolicy = serde_json::from_value(saved).unwrap();
            assert_eq!(policy.standby_member_limit, Some(limit));
        }
        for limit in [None, Some(0), Some(1), Some(4), Some(8)] {
            let saved = serde_json::json!({"max_standby_member_connections": 2, "standby_member_limit": limit});
            let policy: MaintenancePolicy = serde_json::from_value(saved).unwrap();
            assert_eq!(policy.standby_member_limit, limit);
            let canonical = serde_json::to_value(&policy).unwrap();
            assert!(canonical.get("max_standby_member_connections").is_none());
            assert_eq!(
                serde_json::from_value::<MaintenancePolicy>(canonical).unwrap(),
                policy
            );
        }
    }
}
