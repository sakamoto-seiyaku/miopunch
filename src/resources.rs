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
    /// Explicit native TCP listener. None preserves the existing startup.
    #[serde(default)]
    pub tcp_listen: Option<std::net::SocketAddr>,
    #[serde(default)]
    pub udp_direct_disabled: bool,
    /// Direct peer TCP is independent of auxiliary relay permission.
    #[serde(default)]
    pub tcp_direct_disabled: bool,
    /// Forbid ordinary-member data circuits independently of signaling.
    #[serde(default)]
    pub member_data_relay_disabled: bool,
    #[serde(default)]
    pub disabled_tcp_addresses: BTreeSet<std::net::SocketAddr>,
    #[serde(default)]
    pub path_selection: crate::path_selection::PathSelectionPolicy,
    #[serde(default)]
    pub maintenance: crate::network_maintenance::MaintenancePolicy,
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
            revision: 2,
            resources: iroh::defaults::prod::default_relay_map()
                .urls::<Vec<_>>()
                .into_iter()
                .map(|url| Resource {
                    url: url.to_string(),
                    uses: BTreeSet::from([ResourceUse::Observation, ResourceUse::DataRelay]),
                    limitation: "number0 public relay: best effort, no availability guarantee; QAD and end-to-end encrypted relay, not membership authority".into(),
                })
                .chain([
                    Resource {
                        url: "stun://stun.cloudflare.com:3478/".into(),
                        uses: BTreeSet::from([ResourceUse::Observation]),
                        limitation: "Cloudflare documented public STUN/UDP Binding; best effort observation only, no TURN allocation or data relay".into(),
                    },
                    Resource {
                        url: "stun://global.stun.twilio.com:3478/".into(),
                        uses: BTreeSet::from([ResourceUse::Observation]),
                        limitation: "Twilio documented public STUN/UDP Binding; best effort observation only, no TURN allocation or data relay".into(),
                    },
                    Resource {
                        url: "stun+tcp://turn.cloudflare.com:3478/".into(),
                        uses: BTreeSet::from([ResourceUse::Observation]),
                        limitation: "Cloudflare actual TCP Binding verified in MIOPUNCH-64; observation only when TCP explicitly enabled, not TURN or a data relay".into(),
                    },
                ])
                .collect(),
        })
    }
}

impl ResourceConfig {
    /// Permitted RFC 8489 UDP observers, separate from HTTPS QAD facilities.
    pub fn udp_stun_resources(&self) -> Vec<iroh::RelayUrl> {
        self.stun_resources("stun")
    }

    /// Permitted STUN-over-TCP service names, resolved at the actual endpoint.
    pub fn tcp_stun_resources(&self) -> Vec<iroh::RelayUrl> {
        self.stun_resources("stun+tcp")
    }

    fn stun_resources(&self, scheme: &str) -> Vec<iroh::RelayUrl> {
        let mut origins = BTreeMap::new();
        for resource in self
            .replacement
            .as_ref()
            .unwrap_or(&self.defaults.resources)
        {
            let Ok(url) = resource.url.parse::<iroh::RelayUrl>() else {
                continue;
            };
            if url.scheme() == scheme && self.allows(&resource.url, ResourceUse::Observation) {
                origins.insert(resource_origin(&resource.url).unwrap(), url);
            }
        }
        origins.into_values().take(4).collect()
    }
    /// Approved message origins retain independently qualified routes.
    pub(crate) fn mqtt_resources(&self) -> Vec<iroh::RelayUrl> {
        self.signaling_resources("mqtt")
    }
    pub(crate) fn irc_resources(&self) -> Vec<iroh::RelayUrl> {
        self.signaling_resources("irc")
    }
    fn signaling_resources(&self, scheme: &str) -> Vec<iroh::RelayUrl> {
        let mut origins = BTreeMap::new();
        for resource in self
            .replacement
            .as_ref()
            .unwrap_or(&self.defaults.resources)
        {
            let Ok(url) = resource.url.parse::<iroh::RelayUrl>() else {
                continue;
            };
            if url.scheme() == scheme && self.allows(&resource.url, ResourceUse::Signaling) {
                origins.insert(resource_origin(&resource.url).unwrap(), url);
            }
        }
        origins.into_values().collect()
    }
    pub fn with_defaults(defaults: ResourceTable) -> Self {
        Self {
            defaults,
            replacement: None,
            disabled: BTreeSet::new(),
            disabled_uses: BTreeSet::new(),
            lan_discovery: LanDiscovery::Disabled,
            port_mapping: PortMapping::Disabled,
            tcp_listen: None,
            udp_direct_disabled: false,
            tcp_direct_disabled: false,
            member_data_relay_disabled: false,
            disabled_tcp_addresses: BTreeSet::new(),
            path_selection: Default::default(),
            maintenance: Default::default(),
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
            .filter(|resource| {
                resource
                    .url
                    .parse::<RelayUrl>()
                    .is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
            })
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
        self.configure_with_tcp(builder).map(|(builder, _)| builder)
    }

    pub(crate) fn configure_with_tcp(
        &self,
        builder: iroh::endpoint::Builder,
    ) -> Result<
        (
            iroh::endpoint::Builder,
            Option<crate::tcp_transport::TcpTransport>,
        ),
        std::io::Error,
    > {
        let config = self.clone();
        let builder = builder.default_path_selector(self.path_selection.selector()?);
        let builder = if self.udp_direct_disabled {
            builder.clear_ip_transports()
        } else {
            builder
        };
        let tcp = self
            .tcp_listen
            .filter(|_| !self.tcp_direct_disabled)
            .map(|address| {
                let mut transport = crate::tcp_transport::TcpTransport::listen(
                    address,
                    std::sync::Arc::new(crate::tcp_transport::Traffic::default()),
                );
                if self.port_mapping == PortMapping::NatPmp {
                    transport = transport.with_nat_pmp_mapping();
                }
                if !self.tcp_stun_resources().is_empty() {
                    transport = transport.with_punching();
                }
                transport
            });
        let builder = if let Some(transport) = &tcp {
            builder
                .add_custom_transport(std::sync::Arc::new(transport.clone()))
                .transport_config(
                    iroh::endpoint::QuicTransportConfig::builder()
                        .keep_alive_interval(Duration::from_secs(5))
                        .max_idle_timeout(Some(Duration::from_secs(10).try_into().unwrap()))
                        .congestion_controller_factory(std::sync::Arc::new(
                            noq_proto::congestion::Bbr3Config::default(),
                        ))
                        .build(),
                )
        } else {
            builder
        };
        let builder = builder.portmapper_config(match self.port_mapping {
            PortMapping::Disabled => iroh::endpoint::PortmapperConfig::Disabled,
            PortMapping::NatPmp => iroh::endpoint::PortmapperConfig::NatPmp,
        });
        let custom_config = self.clone();
        Ok((
            builder.relay_resource_policy(
                iroh::endpoint::RelayResourcePolicy::new(move |url, usage| {
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
                })
                .with_custom_addresses(move |addr| custom_config.allows_custom(addr)),
            ),
            tcp,
        ))
    }

    /// Recognized TCP candidates are checked at actual fork send; unknown
    /// custom carriers retain their own auxiliary/use qualification.
    pub fn allows_custom(&self, address: &iroh_base::CustomAddr) -> bool {
        if address.id() == crate::member_transport::DATA_TRANSPORT_ID {
            return !self.member_data_relay_disabled
                && !self.disabled_uses.contains(&ResourceUse::DataRelay);
        }
        if address.id() != crate::tcp_transport::DATA_TRANSPORT_ID {
            return true;
        }
        !self.tcp_direct_disabled
            && crate::tcp_transport::TcpTransport::socket_addr(address)
                .is_ok_and(|addr| !self.disabled_tcp_addresses.contains(&addr))
    }
}

/// Resource identity is the service origin, not an input URL's spelling or
/// fixed protocol path. A DNS absolute-name suffix also names the same service.
pub(crate) fn resource_origin(value: &str) -> Option<String> {
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
    in_flight: bool,
}

struct ContactAttempt {
    states: std::sync::Arc<std::sync::Mutex<BTreeMap<iroh::EndpointId, ContactBackoff>>>,
    id: iroh::EndpointId,
}
impl Drop for ContactAttempt {
    fn drop(&mut self) {
        if let Some(state) = self
            .states
            .lock()
            .expect("contact budget")
            .get_mut(&self.id)
        {
            state.in_flight = false;
        }
    }
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
    backoff: std::sync::Arc<std::sync::Mutex<BTreeMap<iroh::EndpointId, ContactBackoff>>>,
    pub(crate) contacts: iroh::address_lookup::memory::MemoryLookup,
    pub(crate) responders: Vec<tokio::task::JoinHandle<()>>,
    tcp: Option<crate::tcp_transport::TcpTransport>,
    mqtt: Vec<std::sync::Arc<crate::mqtt_transport::MqttTransport>>,
}

/// One observation of a retained connection opened from the real TCP carrier's
/// listening port. Its mapping is TCP-specific; it is never a UDP candidate or
/// proof that another destination has the same mapping or is reachable.
pub struct TcpStunObservation {
    pub observer: iroh::RelayUrl,
    pub destination: std::net::SocketAddr,
    pub listener: std::net::SocketAddr,
    pub local_addr: std::net::SocketAddr,
    pub mapped: std::net::SocketAddr,
    pub observed_at: Instant,
    pub valid_until: Instant,
    stream: std::sync::Weak<tokio::net::TcpStream>,
    endpoint: iroh::Endpoint,
    generation: iroh::endpoint::ObservationValidity,
    request: tokio_util::sync::CancellationToken,
}

impl TcpStunObservation {
    pub fn is_fresh(&self) -> bool {
        if self.endpoint.is_closed()
            || !self.generation.is_current()
            || self.request.is_cancelled()
            || Instant::now() >= self.valid_until
        {
            return false;
        }
        self.stream.upgrade().is_some_and(|stream| {
            matches!(stream.try_read(&mut [0; 1]), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        })
    }
}

impl ResourceEndpoint {
    pub(crate) fn new(endpoint: iroh::Endpoint, config: ResourceConfig) -> Self {
        Self {
            endpoint,
            config,
            timeout: Duration::from_secs(10),
            backoff: Default::default(),
            contacts: iroh::address_lookup::memory::MemoryLookup::new(),
            responders: Vec::new(),
            tcp: None,
            mqtt: Vec::new(),
        }
    }

    pub fn endpoint(&self) -> &iroh::Endpoint {
        &self.endpoint
    }

    pub(crate) fn with_tcp(mut self, tcp: Option<crate::tcp_transport::TcpTransport>) -> Self {
        self.tcp = tcp;
        self
    }

    pub(crate) fn with_mqtt(
        mut self,
        mqtt: Vec<std::sync::Arc<crate::mqtt_transport::MqttTransport>>,
    ) -> Self {
        self.mqtt = mqtt;
        self
    }

    pub fn mqtt_diagnostics(&self) -> Vec<(String, bool, u64, u64)> {
        self.mqtt
            .iter()
            .map(|transport| transport.diagnostics())
            .collect()
    }

    /// Resolve and transact only a configured, permitted TCP observer. Success
    /// retains the query connection until a replacement query or endpoint close.
    pub async fn observe_tcp_stun(
        &self,
        observer: &iroh::RelayUrl,
    ) -> std::io::Result<TcpStunObservation> {
        use std::io;
        if observer.scheme() != "stun+tcp"
            || !self
                .config
                .allows(observer.as_str(), ResourceUse::Observation)
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "TCP STUN resource denied",
            ));
        }
        let origin = resource_origin(observer.as_str())
            .ok_or_else(|| io::Error::other("invalid TCP STUN origin"))?;
        let transport = self.tcp.as_ref().ok_or_else(|| {
            io::Error::new(io::ErrorKind::Unsupported, "no configured TCP transport")
        })?;
        let result = crate::tcp_observation::observe(&self.endpoint, transport, observer).await?;
        let stream = std::sync::Arc::new(result.stream);
        let observation = TcpStunObservation {
            observer: observer.clone(),
            destination: result.destination,
            listener: result.listener,
            local_addr: result.local_addr,
            mapped: result.mapped,
            observed_at: result.observed_at,
            valid_until: result.observed_at + Duration::from_secs(40),
            stream: std::sync::Arc::downgrade(&stream),
            endpoint: self.endpoint.clone(),
            generation: result.generation,
            request: result.request.clone(),
        };
        if !observation.is_fresh() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "TCP STUN query superseded",
            ));
        }
        transport.retain_observation(&origin, &result.request, stream)?;
        Ok(observation)
    }

    /// Public contact address including explicitly configured direct TCP.
    /// Signaling-only custom addresses are not advertised as data candidates.
    pub fn addr(&self) -> iroh::EndpointAddr {
        let mut address = self.endpoint.addr();
        address.addrs.extend(
            self.endpoint
                .custom_addrs()
                .into_iter()
                .filter(|addr| {
                    matches!(
                        addr.id(),
                        crate::tcp_transport::DATA_TRANSPORT_ID
                            | crate::member_transport::DATA_TRANSPORT_ID
                    ) && self.config.allows_custom(addr)
                })
                .map(iroh::TransportAddr::Custom),
        );
        address
    }

    /// Candidate eligibility for an existing member data connection. Unknown
    /// custom/signaling carriers are never promoted by the repair coordinator.
    pub(crate) fn allows_data_path(&self, path: &iroh::TransportAddr) -> bool {
        match path {
            iroh::TransportAddr::Ip(_) => !self.config.udp_direct_disabled,
            iroh::TransportAddr::Relay(url) => {
                self.config.allows(url.as_str(), ResourceUse::DataRelay)
            }
            iroh::TransportAddr::Custom(address) => {
                matches!(
                    address.id(),
                    crate::tcp_transport::DATA_TRANSPORT_ID
                        | crate::member_transport::DATA_TRANSPORT_ID
                ) && self.config.allows_custom(address)
            }
            _ => false,
        }
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
        &self,
        target: iroh::EndpointAddr,
        alpn: &[u8],
    ) -> Result<iroh::endpoint::Connection, ResourceContactError> {
        self.connect_attempt(target, alpn, true).await
    }

    /// Recovery is not complete merely because QUIC admitted the peer. Keep
    /// prior protocol failures until the coordinator verifies/saves its scope.
    pub(crate) async fn connect_for_recovery(
        &self,
        target: iroh::EndpointAddr,
        alpn: &[u8],
    ) -> Result<iroh::endpoint::Connection, ResourceContactError> {
        self.connect_attempt(target, alpn, false).await
    }

    async fn connect_attempt(
        &self,
        mut target: iroh::EndpointAddr,
        alpn: &[u8],
        complete_on_handshake: bool,
    ) -> Result<iroh::endpoint::Connection, ResourceContactError> {
        let id = target.id;
        {
            let mut states = self.backoff.lock().expect("contact budget");
            let state = states.entry(id).or_default();
            if state.in_flight {
                return Err(ResourceContactError::TemporarilyUnreachable {
                    retry_after: self.timeout,
                    cause: Some("contact attempt already in flight for this identity".into()),
                });
            }
            if let Some(retry_after) = state
                .retry_at
                .and_then(|at| at.checked_duration_since(Instant::now()))
            {
                return Err(ResourceContactError::TemporarilyUnreachable {
                    retry_after,
                    cause: state.cause.clone(),
                });
            }
            state.in_flight = true;
        }
        let _attempt = ContactAttempt {
            states: self.backoff.clone(),
            id,
        };
        target.addrs.retain(|addr| match addr {
            iroh::TransportAddr::Relay(url) => {
                self.config.allows(url.as_str(), ResourceUse::DataRelay)
            }
            iroh::TransportAddr::Custom(addr) => self.config.allows_custom(addr),
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
                    if complete_on_handshake {
                        self.reset_contact(id);
                    }
                    return Ok(connection);
                }
                Ok(Err(error)) => error.to_string(),
                Err(_) => "bounded contact attempt timed out".to_owned(),
            };
        Err(self
            .backoff
            .lock()
            .expect("contact budget")
            .entry(id)
            .or_default()
            .failed(cause))
    }

    pub(crate) fn recovery_origins(&self) -> Vec<iroh::RelayUrl> {
        self.config
            .mqtt_resources()
            .into_iter()
            .filter(|url| self.config.allows(url.as_str(), ResourceUse::Lookup))
            .take(4)
            .collect()
    }

    pub(crate) fn recovery_udp_allowed(&self) -> bool {
        !self.config.udp_direct_disabled
    }

    pub(crate) fn contact_failed(&self, id: iroh::EndpointId) {
        self.backoff
            .lock()
            .expect("contact budget")
            .entry(id)
            .or_default()
            .failed("candidate recovery did not complete".into());
    }

    pub(crate) fn remember(&self, target: iroh::EndpointAddr) {
        if self
            .contacts
            .get_endpoint_info(target.id)
            .map(|info| info.into_endpoint_addr())
            .as_ref()
            != Some(&target)
        {
            self.reset_contact(target.id);
        }
        self.contacts.set_endpoint_info(target);
    }

    pub(crate) fn reset_contact(&self, id: iroh::EndpointId) {
        if let Some(state) = self.backoff.lock().expect("contact budget").get_mut(&id) {
            state.failures = 0;
            state.retry_at = None;
            state.cause = None;
        }
    }

    pub(crate) fn contact_ready(&self, id: iroh::EndpointId) -> bool {
        self.backoff
            .lock()
            .expect("contact budget")
            .get(&id)
            .is_none_or(|state| {
                !state.in_flight && state.retry_at.is_none_or(|at| at <= Instant::now())
            })
    }

    pub async fn close(self) {
        self.endpoint.close().await;
    }
}

impl Drop for ResourceEndpoint {
    fn drop(&mut self) {
        for responder in &self.responders {
            responder.abort();
        }
    }
}
