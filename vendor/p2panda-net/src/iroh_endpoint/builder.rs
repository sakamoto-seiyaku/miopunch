// SPDX-License-Identifier: MIT OR Apache-2.0

use std::collections::HashMap;
use std::sync::Arc;

use p2panda_core::SigningKey;
use ractor::thread_local::{ThreadLocalActor, ThreadLocalActorSpawner};

use crate::address_book::AddressBook;
use crate::iroh_endpoint::actors::{IrohEndpoint, IrohEndpointArgs};
use crate::iroh_endpoint::api::{Endpoint, EndpointError, RestrictedEndpoint};
use crate::iroh_endpoint::config::IrohConfig;
use crate::{DEFAULT_NETWORK_ID, NetworkId};

pub struct Builder {
    network_id: Option<NetworkId>,
    signing_key: Option<SigningKey>,
    config: Option<IrohConfig>,
    relays: HashMap<iroh::RelayUrl, Option<String>>,
    address_book: AddressBook,
    endpoint_builder_hook: Option<EndpointBuilderHook>,
}

/// Reusable endpoint configuration applied on every actor bind, including restarts.
#[derive(Clone, Default)]
pub struct EndpointBuilderHook {
    ordinary: Option<Arc<dyn Fn(iroh::endpoint::Builder) -> iroh::endpoint::Builder + Send + Sync>>,
    pub(crate) restricted: Option<
        Arc<dyn Fn(iroh::endpoint::Builder) -> iroh::endpoint::RestrictedBuilder + Send + Sync>,
    >,
}

impl std::fmt::Debug for EndpointBuilderHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EndpointBuilderHook(..)")
    }
}

impl EndpointBuilderHook {
    pub(crate) fn apply(&self, builder: iroh::endpoint::Builder) -> iroh::endpoint::Builder {
        match &self.ordinary {
            Some(hook) => hook(builder),
            None => builder,
        }
    }
}

impl Builder {
    pub fn new(address_book: AddressBook) -> Self {
        Self {
            network_id: None,
            signing_key: None,
            config: None,
            address_book,
            relays: HashMap::new(),
            endpoint_builder_hook: None,
        }
    }

    pub fn network_id(mut self, network_id: NetworkId) -> Self {
        self.network_id = Some(network_id);
        self
    }

    pub fn signing_key(mut self, signing_key: SigningKey) -> Self {
        self.signing_key = Some(signing_key);
        self
    }

    pub fn config(mut self, config: IrohConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Configure iroh before binding while retaining p2panda's accept and close ownership.
    pub fn endpoint_builder_hook(
        mut self,
        hook: impl Fn(iroh::endpoint::Builder) -> iroh::endpoint::Builder + Send + Sync + 'static,
    ) -> Self {
        self.endpoint_builder_hook = Some(EndpointBuilderHook {
            ordinary: Some(Arc::new(hook)),
            restricted: None,
        });
        self
    }

    /// Adds iroh relay server to assist in establishing direct connections.
    ///
    /// Multiple relays can be added, iroh will automatically choose a "home relay" based on
    /// latency.
    ///
    /// Relays fullfil multiple functions:
    ///
    /// 1. The relay server helps establish connections by temporarily routing encrypted traffic
    ///    until a direct, P2P connection is feasible. This allows nodes to immediately get
    ///    started, without waiting for holepunching / STUN to complete first.
    /// 2. Handle learning a node's public addresses (via QUIC address discovery), signalling and
    ///    hole-punching to establish direct connections between two nodes. This set of methods is
    ///    also understood as STUN. After this point the relay is not required anymore.
    /// 3. Relayed and encrypted fallback using the server when establishing a direct connection
    ///    failed (TURN).
    ///
    /// If no relay is given other nodes can only connect to us if a directly reachable IP address
    /// is available and known to them.
    pub fn relay_url(mut self, url: iroh::RelayUrl) -> Self {
        self.relays.insert(url, None);
        self
    }

    pub fn relay_url_with_token(mut self, url: iroh::RelayUrl, token: impl Into<String>) -> Self {
        self.relays.insert(url, Some(token.into()));
        self
    }

    pub(crate) fn build_args(self) -> IrohEndpointArgs {
        let network_id = self.network_id.unwrap_or(DEFAULT_NETWORK_ID);
        let signing_key = self.signing_key.unwrap_or_default();
        let config = self.config.unwrap_or_default();
        let relay_map = self
            .relays
            .into_iter()
            .map(|(url, token)| match token {
                Some(token) => iroh::RelayConfig::from(url).with_auth_token(token),
                None => iroh::RelayConfig::from(url),
            })
            .collect();
        (
            network_id,
            signing_key,
            config,
            relay_map,
            self.address_book,
            self.endpoint_builder_hook,
        )
    }

    pub async fn spawn(self) -> Result<Endpoint, EndpointError> {
        Self::spawn_args(self.build_args()).await
    }

    /// Explicitly bind a restricted endpoint using the same p2panda actors.
    /// Ordinary builder configuration runs first on every bind. The returned
    /// wrapper exposes no ordinary endpoint or pre-admission connection handle.
    pub async fn spawn_restricted(
        self,
        configure: impl Fn(iroh::endpoint::Builder) -> iroh::endpoint::RestrictedBuilder
        + Send
        + Sync
        + 'static,
    ) -> Result<RestrictedEndpoint, EndpointError> {
        let mut args = self.build_args();
        args.5
            .get_or_insert_with(EndpointBuilderHook::default)
            .restricted = Some(Arc::new(configure));
        Self::spawn_args(args).await.map(RestrictedEndpoint::new)
    }

    async fn spawn_args(args: IrohEndpointArgs) -> Result<Endpoint, EndpointError> {
        let (actor_ref, _) = {
            let thread_pool = ThreadLocalActorSpawner::new();
            IrohEndpoint::spawn(None, args.clone(), thread_pool).await?
        };

        Ok(Endpoint::new(Some(actor_ref), args))
    }
}
