//! Startup resource configuration. Candidate provenance never grants permission.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{Duration, Instant};

use iroh::{RelayConfig, RelayMap, RelayUrl};
use p2panda_core::{Signature, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum ResourceUse {
    Observation,
    Lookup,
    Signaling,
    DataRelay,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Resource {
    pub url: String,
    pub uses: BTreeSet<ResourceUse>,
    pub limitation: String,
}

/// Disabled preserves the current native assembly's absence of LAN publication.
/// Passive listens only; Active explicitly publishes this node's identity/address.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum LanDiscovery {
    #[default]
    Disabled,
    Passive,
    Active,
}

/// Local gateway consent, independent of public facilities and data relay.
/// NAT-PMP is the protocol validated by the native mapping integration.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum PortMapping {
    /// Preserve the existing absence of gateway probes unless explicitly enabled.
    #[default]
    Disabled,
    NatPmp,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResourceTable {
    pub revision: u64,
    pub resources: Vec<Resource>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResourceConfig {
    pub defaults: ResourceTable,
    /// None uses defaults; Some (including empty) replaces them entirely.
    pub replacement: Option<Vec<Resource>>,
    pub disabled: BTreeSet<(String, ResourceUse)>,
    pub disabled_uses: BTreeSet<ResourceUse>,
    pub lan_discovery: LanDiscovery,
    #[serde(default)]
    pub port_mapping: PortMapping,
    /// Explicitly trusted configuration publisher, distinct from membership.
    #[serde(default)]
    pub trusted_update_key: Option<VerifyingKey>,
}

const UPDATE_DOMAIN: &[u8] = b"miopunch/resource-table/1\0";

/// Exact domain-separated table bytes signed with the established Ed25519 API.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedResourceTable {
    pub statement: Vec<u8>,
    pub signature: Vec<u8>,
}

impl SignedResourceTable {
    pub fn sign(key: &SigningKey, table: &ResourceTable) -> Result<Self, std::io::Error> {
        let mut statement = UPDATE_DOMAIN.to_vec();
        statement.extend(serde_json::to_vec(table).map_err(std::io::Error::other)?);
        let signature = key.sign(&statement).to_bytes().to_vec();
        Ok(Self {
            statement,
            signature,
        })
    }
}

impl Default for ResourceConfig {
    fn default() -> Self {
        Self::with_defaults(ResourceTable {
            revision: 1,
            resources: iroh::defaults::prod::default_relay_map()
                .urls::<Vec<_>>()
                .into_iter()
                .map(|url| Resource {
                    url: url.to_string(),
                    uses: BTreeSet::from([ResourceUse::Observation, ResourceUse::DataRelay]),
                    limitation: "number0 public relay: best effort, no availability guarantee; QAD and end-to-end encrypted relay, not membership authority".into(),
                })
                .collect(),
        })
    }
}

impl ResourceConfig {
    pub fn with_defaults(defaults: ResourceTable) -> Self {
        Self {
            defaults,
            replacement: None,
            disabled: BTreeSet::new(),
            disabled_uses: BTreeSet::new(),
            lan_discovery: LanDiscovery::Disabled,
            port_mapping: PortMapping::Disabled,
            trusted_update_key: None,
        }
    }

    pub fn allows(&self, url: &str, usage: ResourceUse) -> bool {
        let Some(origin) = resource_origin(url) else {
            return false;
        };
        !self.disabled_uses.contains(&usage)
            && !self.disabled.iter().any(|(disabled, purpose)| {
                *purpose == usage && resource_origin(disabled).as_ref() == Some(&origin)
            })
            && self
                .replacement
                .as_ref()
                .unwrap_or(&self.defaults.resources)
                .iter()
                .any(|resource| {
                    resource_origin(&resource.url).as_ref() == Some(&origin)
                        && resource.uses.contains(&usage)
                })
    }

    pub fn relay_map(&self, usage: ResourceUse) -> Result<RelayMap, std::io::Error> {
        self.replacement
            .as_ref()
            .unwrap_or(&self.defaults.resources)
            .iter()
            .filter(|resource| self.allows(&resource.url, usage))
            .map(|resource| {
                resource
                    .url
                    .parse::<RelayUrl>()
                    .map(RelayConfig::from)
                    .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidInput, err))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(RelayMap::from_iter)
    }

    /// A trusted client-release table update; never consumes an untrusted lookup
    /// response. Local replacement and disables remain separate from the table.
    fn update_release_table(&mut self, table: ResourceTable) -> Result<(), std::io::Error> {
        if table.revision <= self.defaults.revision {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "resource table rollback",
            ));
        }
        self.defaults = table;
        Ok(())
    }

    pub fn apply_signed_update(
        &mut self,
        update: &SignedResourceTable,
    ) -> Result<(), std::io::Error> {
        let denied = || {
            std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "untrusted resource update",
            )
        };
        let key = self.trusted_update_key.ok_or_else(denied)?;
        if update.statement.len() > 64 * 1024 {
            return Err(denied());
        }
        let signature = Signature::from_bytes(
            &update
                .signature
                .as_slice()
                .try_into()
                .map_err(|_| denied())?,
        );
        if !key.verify(&update.statement, &signature) {
            return Err(denied());
        }
        let payload = update
            .statement
            .strip_prefix(UPDATE_DOMAIN)
            .ok_or_else(denied)?;
        let table = serde_json::from_slice(payload).map_err(std::io::Error::other)?;
        self.update_release_table(table)
    }

    pub fn load(path: &Path) -> Result<Self, std::io::Error> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(std::io::Error::other),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err),
        }
    }

    /// A personally trusted import replaces resources, not local restrictions,
    /// publisher trust, or LAN disclosure choices. Effective on the next start.
    pub fn import_trusted_replacement(
        &mut self,
        resources: Vec<Resource>,
    ) -> Result<(), std::io::Error> {
        if resources
            .iter()
            .any(|resource| resource_origin(&resource.url).is_none())
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid resource origin",
            ));
        }
        self.replacement = Some(resources);
        Ok(())
    }

    pub fn configure(
        &self,
        builder: iroh::endpoint::Builder,
    ) -> Result<iroh::endpoint::Builder, std::io::Error> {
        let config = self.clone();
        let builder = builder.portmapper_config(match self.port_mapping {
            PortMapping::Disabled => iroh::endpoint::PortmapperConfig::Disabled,
            PortMapping::NatPmp => iroh::endpoint::PortmapperConfig::NatPmp,
        });
        Ok(
            builder.relay_resource_policy(iroh::endpoint::RelayResourcePolicy::new(
                move |url, usage| {
                    config.allows(
                        url.as_str(),
                        match usage {
                            iroh::endpoint::RelayResourceUse::DataRelay => ResourceUse::DataRelay,
                            iroh::endpoint::RelayResourceUse::Observation => {
                                ResourceUse::Observation
                            }
                            iroh::endpoint::RelayResourceUse::Signaling => ResourceUse::Signaling,
                            iroh::endpoint::RelayResourceUse::Lookup => ResourceUse::Lookup,
                        },
                    )
                },
            )),
        )
    }
}

/// Resource identity is the service origin, not an input URL's spelling or
/// fixed protocol path. A DNS absolute-name suffix also names the same service.
fn resource_origin(value: &str) -> Option<String> {
    let parsed: RelayUrl = value.parse().ok()?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return None;
    }
    let mut normalized = std::ops::Deref::deref(&parsed).clone();
    let host = normalized
        .host_str()?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    normalized.set_host(Some(&host)).ok()?;
    normalized.set_path("/");
    normalized.set_query(None);
    normalized.set_fragment(None);
    Some(normalized.to_string())
}

#[derive(Default)]
struct ContactBackoff {
    failures: u32,
    retry_at: Option<Instant>,
    cause: Option<String>,
}

impl ContactBackoff {
    fn failed(&mut self, cause: String) -> ResourceContactError {
        let seconds = 1u64 << self.failures.min(5);
        self.failures = self.failures.saturating_add(1);
        let retry_after = Duration::from_secs(seconds);
        self.retry_at = Some(Instant::now() + retry_after);
        self.cause = Some(cause);
        ResourceContactError::TemporarilyUnreachable {
            retry_after,
            cause: self.cause.clone(),
        }
    }
}

#[derive(Debug, Error)]
pub enum ResourceContactError {
    #[error("temporarily unreachable; retry after {retry_after:?}")]
    TemporarilyUnreachable {
        retry_after: Duration,
        cause: Option<String>,
    },
    #[error("resource configuration failed: {0}")]
    Configuration(#[from] std::io::Error),
}

/// One configured endpoint and its contact budget. Candidate selection and
/// path maintenance remain in the original iroh engine, not a parallel loop.
pub struct ResourceEndpoint {
    endpoint: iroh::Endpoint,
    config: ResourceConfig,
    timeout: Duration,
    backoff: BTreeMap<iroh::EndpointId, ContactBackoff>,
}

impl ResourceEndpoint {
    pub(crate) fn new(endpoint: iroh::Endpoint, config: ResourceConfig) -> Self {
        Self {
            endpoint,
            config,
            timeout: Duration::from_secs(10),
            backoff: BTreeMap::new(),
        }
    }

    pub fn endpoint(&self) -> &iroh::Endpoint {
        &self.endpoint
    }

    /// A bounded contact attempt, independent of the resource startup snapshot.
    pub fn with_contact_timeout(mut self, timeout: Duration) -> Result<Self, std::io::Error> {
        if timeout.is_zero() || timeout > Duration::from_secs(30) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "contact timeout must be greater than zero and at most 30 seconds",
            ));
        }
        self.timeout = timeout;
        Ok(self)
    }

    pub async fn connect(
        &mut self,
        mut target: iroh::EndpointAddr,
        alpn: &[u8],
    ) -> Result<iroh::endpoint::Connection, ResourceContactError> {
        let id = target.id;
        if let Some(state) = self.backoff.get(&id)
            && let Some(retry_at) = state.retry_at
            && let Some(retry_after) = retry_at.checked_duration_since(Instant::now())
        {
            return Err(ResourceContactError::TemporarilyUnreachable {
                retry_after,
                cause: state.cause.clone(),
            });
        }
        target.addrs.retain(|addr| match addr {
            iroh::TransportAddr::Relay(url) => {
                self.config.allows(url.as_str(), ResourceUse::DataRelay)
            }
            _ => true,
        });
        target.addrs.extend(
            self.config
                .relay_map(ResourceUse::DataRelay)?
                .urls::<Vec<_>>()
                .into_iter()
                .map(iroh::TransportAddr::Relay),
        );
        let cause =
            match tokio::time::timeout(self.timeout, self.endpoint.connect(target, alpn)).await {
                Ok(Ok(connection)) => {
                    self.backoff.remove(&id);
                    return Ok(connection);
                }
                Ok(Err(error)) => error.to_string(),
                Err(_) => "bounded contact attempt timed out".to_owned(),
            };
        Err(self.backoff.entry(id).or_default().failed(cause))
    }

    pub async fn close(self) {
        self.endpoint.close().await;
    }
}
