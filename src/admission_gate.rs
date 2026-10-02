//! Shared endpoint admission policy. Every ALPN is classified before an
//! authenticated iroh connection is handed to a protocol handler. A successful
//! TLS handshake alone never grants network data or management history.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::endpoint::{
    AfterHandshakeOutcome, BeforeConnectOutcome, Builder, Connection, EndpointHooks,
    WeakConnectionHandle,
};
use iroh_base::EndpointId;

use crate::admission_proof::{
    AdmissionScope, AdmissionSecret, BootstrapProof, prove_client, prove_server,
};
use crate::genesis_recovery::RECOVERY_ALPN;
use crate::member_bootstrap::MEMBER_BOOTSTRAP_ALPN;
use crate::nat_selector::{LearningScope, ProbeProtocol};
use crate::tcp_transport::DATA_TRANSPORT_ID;

pub const ADMISSION_REQUEST_ALPN: &[u8] = b"miopunch/admission-request/1";
pub const CONTROL_ALPN: &[u8] = b"miopunch/control/1";

const MAX_SESSIONS: usize = 4096;
const STAGE_TIMEOUT: Duration = Duration::from_secs(5);

/// The maximum scope an ALPN can receive. Its handler must still validate
/// invitation or root proofs before releasing any admission material.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolScope {
    AdmissionRequest,
    GenesisRecovery,
    Bootstrap,
    Member,
    /// Restricted replacement distribution. A staged credential can prove
    /// this scope only; ordinary bootstrap/business still require the active one.
    Rotation,
}

#[derive(Debug)]
struct Session {
    connection: WeakConnectionHandle,
    marker: Arc<()>,
    peer: [u8; 32],
    scope: ProtocolScope,
    proof: Option<Arc<BootstrapProof>>,
}

impl Drop for Session {
    fn drop(&mut self) {
        // Exceptions have no BootstrapProof to close them. A newly verified
        // removal must terminate their existing restricted streams as well.
        if let Some(connection) = self.connection.upgrade() {
            connection.close(0u32.into(), b"admission session ended");
        }
    }
}

impl Session {
    fn valid(&self, policy: &Policy) -> bool {
        if !self
            .connection
            .upgrade()
            .is_some_and(|connection| connection.close_reason().is_none())
        {
            return false;
        }
        match self.scope {
            ProtocolScope::AdmissionRequest => true,
            ProtocolScope::GenesisRecovery => !policy.removed.contains(&self.peer),
            ProtocolScope::Bootstrap | ProtocolScope::Member | ProtocolScope::Rotation => {
                if policy.removed.contains(&self.peer) || policy.removed.contains(&policy.local_id)
                {
                    return false;
                }
                let Some(proof) = &self.proof else {
                    return false;
                };
                let current = policy.secret.as_ref().is_some_and(|secret| {
                    proof.scope() == &policy.scope
                        && proof.credential_fingerprint() == &secret.fingerprint(&policy.scope)
                });
                let staged = self.scope == ProtocolScope::Rotation
                    && policy.staged.as_ref().is_some_and(|(scope, secret)| {
                        proof.scope() == scope
                            && proof.credential_fingerprint() == &secret.fingerprint(scope)
                    });
                (current || staged)
                    && (!matches!(self.scope, ProtocolScope::Member | ProtocolScope::Rotation)
                        || (policy.members.contains(&self.peer)
                            && policy.members.contains(&policy.local_id)))
            }
        }
    }
}

#[derive(Clone, Debug)]
struct Policy {
    local_id: [u8; 32],
    scope: AdmissionScope,
    secret: Option<AdmissionSecret>,
    staged: Option<(AdmissionScope, AdmissionSecret)>,
    members: HashSet<[u8; 32]>,
    removed: HashSet<[u8; 32]>,
    authorization_epoch: u64,
}

#[derive(Debug)]
struct State {
    policy: Policy,
    protocols: HashMap<Vec<u8>, ProtocolScope>,
    allowed_data_custom_ids: BTreeSet<u64>,
    sessions: HashMap<usize, Session>,
}

/// Cloneable policy handle for the one shared native or p2panda iroh Endpoint.
/// Sessions keep only weak QUIC handles; dropping a protocol's last connection
/// handle still closes it normally. Management view changes close affected
/// sessions immediately, including ones already handed to protocol handlers.
#[derive(Clone, Debug)]
pub struct AdmissionGate {
    state: Arc<Mutex<State>>,
    message_bootstrap: bool,
}

impl AdmissionGate {
    pub fn new(
        local_id: EndpointId,
        scope: AdmissionScope,
        secret: Option<AdmissionSecret>,
    ) -> Self {
        let mut protocols = HashMap::new();
        protocols.insert(
            ADMISSION_REQUEST_ALPN.to_vec(),
            ProtocolScope::AdmissionRequest,
        );
        protocols.insert(RECOVERY_ALPN.to_vec(), ProtocolScope::GenesisRecovery);
        protocols.insert(
            crate::management_recovery::MANAGEMENT_RECOVERY_ALPN.to_vec(),
            ProtocolScope::GenesisRecovery,
        );
        protocols.insert(MEMBER_BOOTSTRAP_ALPN.to_vec(), ProtocolScope::Bootstrap);
        protocols.insert(
            crate::connection_recovery::CONNECTION_RECOVERY_ALPN.to_vec(),
            ProtocolScope::Bootstrap,
        );
        protocols.insert(CONTROL_ALPN.to_vec(), ProtocolScope::Bootstrap);
        protocols.insert(
            crate::network_recovery::RECOVERY_REPLICA_ALPN.to_vec(),
            ProtocolScope::Bootstrap,
        );
        protocols.insert(
            crate::admission_rotation::ADMISSION_ROTATION_ALPN.to_vec(),
            ProtocolScope::Rotation,
        );
        protocols.insert(
            crate::admission_rotation::ROTATION_DELIVERY_ALPN.to_vec(),
            ProtocolScope::AdmissionRequest,
        );
        Self {
            message_bootstrap: false,
            state: Arc::new(Mutex::new(State {
                policy: Policy {
                    local_id: *local_id.as_bytes(),
                    scope,
                    secret,
                    staged: None,
                    members: HashSet::new(),
                    removed: HashSet::new(),
                    authorization_epoch: 0,
                },
                protocols,
                allowed_data_custom_ids: BTreeSet::from([DATA_TRANSPORT_ID]),
                sessions: HashMap::new(),
            })),
        }
    }

    /// Classify a protocol before registering it with Router or p2panda.
    /// Unknown ALPNs fail closed even if the endpoint advertises them.
    pub fn register_protocol(&self, alpn: &[u8], scope: ProtocolScope) {
        let mut state = self.state.lock().expect("admission policy lock");
        match state.protocols.get(alpn) {
            Some(existing) => assert_eq!(*existing, scope, "an ALPN cannot change admission scope"),
            None => {
                state.protocols.insert(alpn.to_vec(), scope);
            }
        }
    }

    /// p2panda mixes each protocol identifier with its 32-byte NetworkId.
    /// Register the wire ALPN rather than the unmixed identifier: Gossip holds
    /// the underlying endpoint directly, so a connect wrapper is insufficient.
    pub fn register_p2panda_protocol(
        &self,
        protocol_id: &[u8],
        network_id: [u8; 32],
        scope: ProtocolScope,
    ) {
        let mut input = Vec::with_capacity(protocol_id.len() + network_id.len());
        input.extend_from_slice(protocol_id);
        input.extend_from_slice(&network_id);
        self.register_protocol(p2panda_core::Hash::digest(&input).as_bytes(), scope);
    }

    pub fn register_p2panda_defaults(&self, network_id: [u8; 32]) {
        self.register_p2panda_protocol(
            b"p2panda/confidential_discovery/v1",
            network_id,
            ProtocolScope::Member,
        );
        self.register_p2panda_protocol(b"/iroh-gossip/1", network_id, ProtocolScope::Member);
        self.register_p2panda_protocol(b"p2panda/log_sync/v1", network_id, ProtocolScope::Member);
    }

    /// Explicitly allow a data-capable custom carrier on member connections.
    /// Unknown custom carriers remain restricted to bootstrap/control. Calling
    /// this after sessions exist closes admitted sessions so the expanded path
    /// policy is applied on reconnect.
    pub fn register_data_carrier(&self, transport_id: u64) {
        let mut state = self.state.lock().expect("admission policy lock");
        if state.allowed_data_custom_ids.insert(transport_id) {
            state
                .sessions
                .retain(|_, session| session.scope != ProtocolScope::Member);
        }
    }

    /// Installs the gate at the shared iroh Endpoint. This disables 0-RTT and
    /// withholds automatic QNT candidates before the hook grants normal scope.
    pub fn configure(&self, builder: Builder) -> Builder {
        builder.admission_gate(self.clone())
    }

    /// Endpoint-local permission for supported signaling adapters' bootstrap.
    /// Install only together with the before-data-release path coordinator.
    pub(crate) fn with_message_bootstrap(&self) -> Self {
        Self {
            state: self.state.clone(),
            message_bootstrap: true,
        }
    }

    fn member_carrier_allowed(&self, id: u64, data_ids: &BTreeSet<u64>) -> bool {
        data_ids.contains(&id)
            || (self.message_bootstrap
                && matches!(
                    id,
                    crate::mqtt_transport::TRANSPORT_ID
                        | crate::member_transport::TRANSPORT_ID
                        | crate::irc_transport::TRANSPORT_ID
                ))
    }

    /// Supply only the locally verified, currently applied management view.
    /// A valid correction can re-add an identity; it must use a new connection.
    pub fn update_authorization(
        &self,
        members: impl IntoIterator<Item = [u8; 32]>,
        removed: impl IntoIterator<Item = [u8; 32]>,
    ) {
        let mut state = self.state.lock().expect("admission policy lock");
        let members = members.into_iter().collect();
        let removed: HashSet<_> = removed.into_iter().collect();
        if state.policy.members != members || state.policy.removed != removed {
            state.policy.authorization_epoch = state
                .policy
                .authorization_epoch
                .checked_add(1)
                .expect("authorization epoch exhausted");
        }
        state.policy.members = members;
        // Close every existing session affected by a newly effective removal,
        // including restricted requests. Fresh requests remain possible, but
        // can only obtain a separately approved, currently valid grant.
        let newly_removed: HashSet<_> =
            removed.difference(&state.policy.removed).copied().collect();
        let local_removed = newly_removed.contains(&state.policy.local_id);
        state
            .sessions
            .retain(|_, session| !local_removed && !newly_removed.contains(&session.peer));
        state.policy.removed = removed;
        let policy = state.policy.clone();
        state.sessions.retain(|_, session| session.valid(&policy));
    }

    /// Rotate the locally accepted credential generation and terminate every
    /// session proved under the old credential. The durable store must commit
    /// the rotation before the caller invokes this method.
    pub fn rotate_secret(&self, scope: AdmissionScope, secret: AdmissionSecret) {
        let mut state = self.state.lock().expect("admission policy lock");
        state.policy.scope = scope;
        state.policy.secret = Some(secret);
        state.policy.staged = None;
        let policy = state.policy.clone();
        state.sessions.retain(|_, session| session.valid(&policy));
    }

    pub(crate) fn stage_rotation_secret(&self, scope: AdmissionScope, secret: AdmissionSecret) {
        self.state
            .lock()
            .expect("admission policy lock")
            .policy
            .staged = Some((scope, secret));
    }

    pub(crate) fn suspend_secret(&self) {
        let mut state = self.state.lock().expect("admission policy lock");
        state.policy.secret = None;
        let policy = state.policy.clone();
        state.sessions.retain(|_, session| session.valid(&policy));
    }

    /// Recheck a live connection before a handler releases protected data.
    /// In particular, a Bootstrap connection cannot be used as Member scope.
    pub fn allows(&self, connection: &Connection, required: ProtocolScope) -> bool {
        let state = self.state.lock().expect("admission policy lock");
        state
            .sessions
            .get(&connection.stable_id())
            .is_some_and(|session| {
                session.peer == *connection.remote_id().as_bytes()
                    && session.scope == required
                    && session.valid(&state.policy)
            })
    }

    pub(crate) fn member_connection_for_binding(
        &self,
        peer: iroh::EndpointId,
        binding: &[u8; 32],
    ) -> Option<Connection> {
        let state = self.state.lock().expect("admission policy lock");
        state
            .sessions
            .values()
            .filter(|session| {
                session.peer == *peer.as_bytes()
                    && session.scope == ProtocolScope::Member
                    && session.valid(&state.policy)
            })
            .filter_map(|session| session.connection.upgrade())
            .find(|connection| {
                connection.close_reason().is_none()
                    && crate::connection_recovery::binding(connection)
                        .is_ok_and(|actual| &actual == binding)
            })
    }

    /// An admitted control connection may coordinate a data-path upgrade only
    /// while both identities are current members. This does not grant the
    /// control ALPN general member-protocol access: callers must still verify
    /// the IP path and close the control carrier before releasing business.
    pub fn allows_member_path_upgrade(&self, connection: &Connection) -> bool {
        let state = self.state.lock().expect("admission policy lock");
        state
            .sessions
            .get(&connection.stable_id())
            .is_some_and(|session| {
                session.peer == *connection.remote_id().as_bytes()
                    && matches!(
                        session.scope,
                        ProtocolScope::Bootstrap | ProtocolScope::Member
                    )
                    && session.valid(&state.policy)
                    && state.policy.members.contains(&session.peer)
                    && state.policy.members.contains(&state.policy.local_id)
            })
    }

    /// An opaque learning scope derives from this exact proved connection and
    /// current member view. Candidate claims never supply identity or epochs.
    pub(crate) fn nat_learning_scope(&self, connection: &Connection) -> Option<LearningScope> {
        let state = self.state.lock().expect("admission policy lock");
        let session = state.sessions.get(&connection.stable_id())?;
        if session.peer != *connection.remote_id().as_bytes()
            || !matches!(
                session.scope,
                ProtocolScope::Bootstrap | ProtocolScope::Member
            )
            || !session.valid(&state.policy)
            || !state.policy.members.contains(&session.peer)
            || !state.policy.members.contains(&state.policy.local_id)
        {
            return None;
        }
        let proof = session.proof.as_ref()?;
        Some(LearningScope {
            local: state.policy.local_id,
            remote: session.peer,
            network: *proof.scope().network_id(),
            secret_generation: proof.scope().secret_generation(),
            credential: *proof.credential_fingerprint(),
            authorization_epoch: state.policy.authorization_epoch,
            protocol: ProbeProtocol::Udp,
        })
    }

    /// Borrow the exact connection's completed SCRAM fact for a restricted
    /// history handler. It cannot be transferred to another connection, and a
    /// policy change removes the fact and closes its session.
    pub fn proof_for(&self, connection: &Connection) -> Option<Arc<BootstrapProof>> {
        let state = self.state.lock().expect("admission policy lock");
        let session = state.sessions.get(&connection.stable_id())?;
        (session.peer == *connection.remote_id().as_bytes() && session.valid(&state.policy))
            .then(|| session.proof.as_ref().cloned())
            .flatten()
    }

    pub(crate) fn discovery_current(&self, scope: &AdmissionScope) -> bool {
        let state = self.state.lock().expect("admission policy lock");
        state.policy.scope == *scope
            && state.policy.secret.is_some()
            && state.policy.members.contains(&state.policy.local_id)
            && !state.policy.removed.contains(&state.policy.local_id)
    }

    fn reject() -> AfterHandshakeOutcome {
        AfterHandshakeOutcome::Reject {
            error_code: 0u32.into(),
            reason: b"network admission rejected".to_vec(),
        }
    }
}

/// Both authenticated sides must agree that the exact connection reached the
/// requested stage. Without this second stream, a dialing side with stale
/// membership could receive a normal handle just before the server rejects it.
async fn agree_on_stage(connection: &Connection) -> bool {
    tokio::time::timeout(STAGE_TIMEOUT, async {
        let (mut send, mut recv) = if connection.side().is_client() {
            connection.open_bi().await.ok()?
        } else {
            connection.accept_bi().await.ok()?
        };
        send.write_all(b"\x01").await.ok()?;
        send.finish().ok()?;
        let mut result = [0];
        recv.read_exact(&mut result).await.ok()?;
        (result == [1]).then_some(())
    })
    .await
    .ok()
    .flatten()
    .is_some()
}

impl EndpointHooks for AdmissionGate {
    async fn before_connect(
        &self,
        remote_addr: &iroh::EndpointAddr,
        alpn: &[u8],
    ) -> BeforeConnectOutcome {
        let state = self.state.lock().expect("admission policy lock");
        if !state.protocols.contains_key(alpn)
            || (state.protocols.get(alpn) != Some(&ProtocolScope::AdmissionRequest)
                && state.policy.removed.contains(remote_addr.id.as_bytes()))
        {
            BeforeConnectOutcome::Reject
        } else {
            BeforeConnectOutcome::Accept
        }
    }

    async fn after_handshake(&self, connection: &Connection) -> AfterHandshakeOutcome {
        let peer = *connection.remote_id().as_bytes();
        let (protocol, policy, allowed_data_custom_ids) = {
            let state = self.state.lock().expect("admission policy lock");
            let Some(protocol) = state.protocols.get(connection.alpn()).copied() else {
                return Self::reject();
            };
            (
                protocol,
                state.policy.clone(),
                state.allowed_data_custom_ids.clone(),
            )
        };
        if protocol != ProtocolScope::AdmissionRequest && policy.removed.contains(&peer) {
            return Self::reject();
        }
        // An unregistered custom carrier cannot start a member protocol.
        if protocol == ProtocolScope::Member
            && connection.paths().iter().any(|path| {
                matches!(path.remote_addr(), iroh_base::TransportAddr::Custom(custom)
                    if !self.member_carrier_allowed(custom.id(), &allowed_data_custom_ids))
            })
        {
            return Self::reject();
        }
        let proof = match protocol {
            ProtocolScope::AdmissionRequest | ProtocolScope::GenesisRecovery => None,
            ProtocolScope::Bootstrap | ProtocolScope::Member | ProtocolScope::Rotation => {
                // A fresh rotation TLS connection chooses one generation before
                // the one permitted SCRAM attempt. This never repeats SASL on
                // an existing session or promotes a staged proof to business.
                let selected = if protocol == ProtocolScope::Rotation {
                    let result = tokio::time::timeout(STAGE_TIMEOUT, async {
                        let (mut send, mut recv) = if connection.side().is_client() {
                            connection.open_bi().await.ok()?
                        } else {
                            connection.accept_bi().await.ok()?
                        };
                        let generation = if connection.side().is_client() {
                            send.write_all(&policy.scope.secret_generation().to_be_bytes())
                                .await
                                .ok()?;
                            policy.scope.secret_generation()
                        } else {
                            let mut bytes = [0; 8];
                            recv.read_exact(&mut bytes).await.ok()?;
                            u64::from_be_bytes(bytes)
                        };
                        send.finish().ok()?;
                        if generation == policy.scope.secret_generation() {
                            Some((policy.scope.clone(), policy.secret.clone()?))
                        } else {
                            policy
                                .staged
                                .clone()
                                .filter(|(scope, _)| scope.secret_generation() == generation)
                        }
                    })
                    .await
                    .ok()
                    .flatten();
                    result
                } else {
                    policy
                        .secret
                        .clone()
                        .map(|secret| (policy.scope.clone(), secret))
                };
                let Some((scope, secret)) = selected else {
                    return Self::reject();
                };
                let result = if connection.side().is_client() {
                    prove_client(connection, &scope, &secret).await
                } else {
                    prove_server(connection, &scope, &secret).await
                };
                let Ok(proof) = result else {
                    return Self::reject();
                };
                Some(Arc::new(proof))
            }
        };
        {
            let state = self.state.lock().expect("admission policy lock");
            if (protocol != ProtocolScope::AdmissionRequest && state.policy.removed.contains(&peer))
                || (matches!(protocol, ProtocolScope::Member | ProtocolScope::Rotation)
                    && (!state.policy.members.contains(&peer)
                        || !state.policy.members.contains(&state.policy.local_id)))
            {
                return Self::reject();
            }
        }
        if !agree_on_stage(connection).await {
            return Self::reject();
        }
        let weak = connection.weak_handle();
        let closed = weak.closed();
        let marker = Arc::new(());
        {
            let mut state = self.state.lock().expect("admission policy lock");
            let current = state.policy.clone();
            state.sessions.retain(|_, session| session.valid(&current));
            if state.protocols.get(connection.alpn()) != Some(&protocol)
                || state.sessions.len() >= MAX_SESSIONS
                || state.sessions.contains_key(&connection.stable_id())
                || (protocol == ProtocolScope::Member
                    && connection.paths().iter().any(|path| {
                        matches!(
                            path.remote_addr(),
                            iroh_base::TransportAddr::Custom(custom)
                                if !self.member_carrier_allowed(custom.id(), &state.allowed_data_custom_ids)
                        )
                    }))
            {
                return Self::reject();
            }
            let session = Session {
                connection: weak,
                marker: marker.clone(),
                peer,
                scope: protocol,
                proof,
            };
            if !session.valid(&state.policy) {
                return Self::reject();
            }
            state.sessions.insert(connection.stable_id(), session);
        }
        let state = Arc::downgrade(&self.state);
        let id = connection.stable_id();
        tokio::spawn(async move {
            let _ = closed.await;
            if let Some(state) = state.upgrade()
                && let Ok(mut state) = state.lock()
                && state
                    .sessions
                    .get(&id)
                    .is_some_and(|session| Arc::ptr_eq(&session.marker, &marker))
            {
                state.sessions.remove(&id);
            }
        });
        if protocol == ProtocolScope::Member {
            AfterHandshakeOutcome::AcceptData {
                allowed_data_custom_ids: self
                    .state
                    .lock()
                    .expect("admission policy lock")
                    .allowed_data_custom_ids
                    .iter()
                    .copied()
                    .collect(),
            }
        } else {
            AfterHandshakeOutcome::AcceptRestricted
        }
    }
}
