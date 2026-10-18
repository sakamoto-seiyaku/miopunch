//! Restricted genesis recovery over one mutually identified iroh TLS connection.
//!
//! The root signs a domain-separated statement containing this connection's TLS
//! exporter, the pinned network anchor, and both authenticated node identities.
//! The peer verifies it before sending any management history or admission secret.
//! This is an Ed25519 proof of the existing root credential, not a new shared
//! password or an authorization derived from merely knowing the public root.

use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use iroh::Endpoint;
use iroh::endpoint::{
    Connection, ControlExchange, ControlHandle, ControlMessageFormat, WeakControlConnection,
};
use iroh_base::EndpointId;
use p2panda_auth::group::{GroupAction, GroupMember};
use p2panda_auth::{Access, GroupsExtensionArgs};
use p2panda_core::{Hash, Header, Signature, SigningKey, VerifyingKey};
use thiserror::Error;

use crate::admission_proof::{AdmissionScope, AdmissionSecret};
use crate::control_protocol::AuthenticatedControl;
use crate::management_auth::{AuthError, ManagementAuth, ManagementExtension, NetworkAnchor};
use crate::management_store::{ManagementRecord, ManagementStore, StoreError};

pub(crate) const DOMAIN: &[u8] = b"miopunch/genesis-recovery/v1\0";
const EXPORTER_LABEL: &[u8] = b"EXPORTER-Channel-Binding";
pub const RECOVERY_ALPN: &[u8] = b"miopunch/genesis-recovery/1";
const MAX_HISTORY_BYTES: usize = 16 * 1024 * 1024;
const MAX_GRANT_BYTES: usize = 256 * 1024;
const MAX_RECORDS: usize = 4096;
const TIMEOUT: Duration = Duration::from_secs(15);
static ACTIVE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(32);
static SESSIONS: LazyLock<Mutex<HashSet<usize>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

pub(crate) struct SessionGuard {
    id: usize,
    connection: WeakControlConnection,
}

impl SessionGuard {
    pub(crate) fn begin(
        connection: &(impl AuthenticatedControl + ?Sized),
    ) -> Result<(tokio::sync::SemaphorePermit<'static>, Self), RecoveryError> {
        let attempt = (|| {
            let permit = ACTIVE.try_acquire().map_err(|_| RecoveryError::Busy)?;
            if connection.close_reason().is_some() {
                return Err(RecoveryError::Rejected);
            }
            let mut sessions = SESSIONS.lock().map_err(|_| RecoveryError::Rejected)?;
            let id = connection.stable_id();
            if !sessions.insert(id) {
                return Err(RecoveryError::Rejected);
            }
            Ok((
                permit,
                Self {
                    id,
                    connection: connection.revocation_handle(),
                },
            ))
        })();
        if attempt.is_err() {
            connection.close(0u32.into(), b"genesis recovery rejected");
        }
        attempt
    }
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        if let Ok(mut sessions) = SESSIONS.lock() {
            sessions.remove(&self.id);
        }
        self.connection
            .close(0u32.into(), b"genesis recovery ended");
    }
}

#[derive(Debug, Error)]
pub enum RecoveryError {
    #[error("genesis recovery proof or history was rejected")]
    Rejected,
    #[error("genesis recovery timed out")]
    Timeout,
    #[error("too many concurrent genesis recovery sessions")]
    Busy,
    #[error("genesis recovery transport failed")]
    Transport,
    #[error("management history failed verification: {0}")]
    Auth(#[from] AuthError),
    #[error("management storage failed: {0}")]
    Store(#[from] StoreError),
}

/// Verified materials for one recovering device. The last record is the new
/// root-signed administrator grant, accepted by the serving member before return.
#[derive(Debug)]
pub struct RecoveredNetwork {
    pub anchor: NetworkAnchor,
    pub records: Vec<ManagementRecord>,
    pub scope: AdmissionScope,
    pub secret: AdmissionSecret,
    pub grant_id: Hash,
}

/// Backup material required to recover the original network. The canonical
/// Create hash stays separate from both the node identity and admission secret.
pub struct GenesisCredential {
    pub(crate) root: SigningKey,
    genesis: Hash,
}

impl GenesisCredential {
    pub fn new(root: SigningKey, genesis: Hash) -> Self {
        Self { root, genesis }
    }

    pub fn from_saved(root_bytes: [u8; 32], genesis: Hash) -> Self {
        Self::new(SigningKey::from_bytes(&root_bytes), genesis)
    }

    pub fn anchor(&self) -> NetworkAnchor {
        NetworkAnchor {
            root: self.root.verifying_key(),
            genesis: self.genesis,
        }
    }

    pub(crate) fn into_signing_key(self) -> SigningKey {
        self.root
    }
}

pub(crate) fn proof_statement(
    connection: &(impl AuthenticatedControl + ?Sized),
    anchor: NetworkAnchor,
    recovering: EndpointId,
    contact: EndpointId,
) -> Result<Vec<u8>, RecoveryError> {
    let mut binding = [0; 32];
    connection
        .export_keying_material(&mut binding, EXPORTER_LABEL, b"")
        .map_err(|_| RecoveryError::Transport)?;
    let mut statement = Vec::with_capacity(DOMAIN.len() + 32 * 5);
    statement.extend_from_slice(DOMAIN);
    statement.extend_from_slice(anchor.root.as_bytes());
    statement.extend_from_slice(anchor.genesis.as_bytes());
    statement.extend_from_slice(recovering.as_bytes());
    statement.extend_from_slice(contact.as_bytes());
    statement.extend_from_slice(&binding);
    Ok(statement)
}

pub(crate) async fn write_control_fixed(
    exchange: &ControlExchange,
    bytes: &[u8],
) -> Result<(), RecoveryError> {
    exchange
        .send_message(
            ControlMessageFormat::Fixed {
                length: bytes.len(),
            },
            bytes,
        )
        .await
        .map_err(|_| RecoveryError::Transport)
}
pub(crate) async fn read_control_fixed<const N: usize>(
    exchange: &ControlExchange,
    bytes: &mut [u8; N],
) -> Result<(), RecoveryError> {
    let message = exchange
        .recv_message(ControlMessageFormat::Fixed { length: N })
        .await
        .map_err(|_| RecoveryError::Transport)?;
    bytes.copy_from_slice(&message);
    Ok(())
}
pub(crate) async fn write_control_frame(
    exchange: &ControlExchange,
    bytes: &[u8],
) -> Result<(), RecoveryError> {
    if bytes.is_empty() || bytes.len() > MAX_HISTORY_BYTES {
        return Err(RecoveryError::Rejected);
    }
    exchange
        .send_message(
            ControlMessageFormat::LengthPrefixed32 {
                maximum: MAX_HISTORY_BYTES,
            },
            bytes,
        )
        .await
        .map_err(|_| RecoveryError::Transport)
}
pub(crate) async fn read_control_frame(
    exchange: &ControlExchange,
    maximum: usize,
) -> Result<Vec<u8>, RecoveryError> {
    let bytes = exchange
        .recv_message(ControlMessageFormat::LengthPrefixed32 { maximum })
        .await
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::InvalidData {
                RecoveryError::Rejected
            } else {
                RecoveryError::Transport
            }
        })?;
    if bytes.is_empty() {
        return Err(RecoveryError::Rejected);
    }
    Ok(bytes)
}

fn history_wire(
    records: &[ManagementRecord],
    scope: &AdmissionScope,
    secret: &AdmissionSecret,
) -> Result<Vec<u8>, RecoveryError> {
    if records.is_empty() || records.len() > MAX_RECORDS {
        return Err(RecoveryError::Rejected);
    }
    let raw: Vec<_> = records
        .iter()
        .map(|record| (record.header(), record.body()))
        .collect();
    let mut bytes = p2panda_core::cbor::encode_cbor(&raw).map_err(|_| RecoveryError::Rejected)?;
    bytes.extend_from_slice(&scope.secret_generation().to_be_bytes());
    bytes.extend_from_slice(secret.saved_bytes());
    if bytes.len() > MAX_HISTORY_BYTES {
        return Err(RecoveryError::Rejected);
    }
    Ok(bytes)
}

fn parse_history(
    bytes: &[u8],
    anchor: NetworkAnchor,
) -> Result<(Vec<ManagementRecord>, AdmissionScope, AdmissionSecret), RecoveryError> {
    let (history, scope_and_secret) = bytes
        .split_at_checked(bytes.len().checked_sub(40).ok_or(RecoveryError::Rejected)?)
        .ok_or(RecoveryError::Rejected)?;
    let generation = u64::from_be_bytes(
        scope_and_secret[..8]
            .try_into()
            .map_err(|_| RecoveryError::Rejected)?,
    );
    let secret = &scope_and_secret[8..];
    let raw: Vec<(Vec<u8>, Option<Vec<u8>>)> =
        p2panda_core::cbor::decode_cbor(history).map_err(|_| RecoveryError::Rejected)?;
    if raw.is_empty() || raw.len() > MAX_RECORDS {
        return Err(RecoveryError::Rejected);
    }
    let records = raw
        .into_iter()
        .map(|(header, body)| ManagementRecord::new(header, body.as_deref()))
        .collect();
    Ok((
        records,
        AdmissionScope::new(*anchor.root.as_bytes(), generation),
        AdmissionSecret::from_saved(secret.try_into().map_err(|_| RecoveryError::Rejected)?),
    ))
}

pub(crate) fn endpoint_key(id: EndpointId) -> Result<VerifyingKey, RecoveryError> {
    VerifyingKey::from_bytes(id.as_bytes()).map_err(|_| RecoveryError::Rejected)
}

/// Recover with the original root key and pinned canonical Create hash, even
/// without management history, admission secret, or an old author log tail.
/// `expected_contact` must be the node identity obtained from a trusted route.
/// This function opens only the recovery stream and closes its TLS connection
/// after success or failure; the returned records are ready for local storage.
pub async fn recover_client(
    endpoint: &Endpoint,
    connection: &Connection,
    credential: &GenesisCredential,
    expected_contact: EndpointId,
) -> Result<RecoveredNetwork, RecoveryError> {
    recover_client_at(endpoint.id(), connection, credential, expected_contact).await
}

/// Recover through bounded messages on this exact restricted TLS session.
/// No ordinary endpoint, connection, or stream can be obtained from this entry.
pub async fn recover_control_client(
    connection: &ControlHandle,
    credential: &GenesisCredential,
    expected_contact: EndpointId,
) -> Result<RecoveredNetwork, RecoveryError> {
    recover_client_at(
        connection.local_id(),
        connection,
        credential,
        expected_contact,
    )
    .await
}

async fn recover_client_at(
    local_id: EndpointId,
    connection: &(impl AuthenticatedControl + ?Sized),
    credential: &GenesisCredential,
    expected_contact: EndpointId,
) -> Result<RecoveredNetwork, RecoveryError> {
    let (_permit, _session) = SessionGuard::begin(connection)?;
    let result = tokio::time::timeout(TIMEOUT, async {
        if connection.remote_id() != expected_contact || connection.alpn() != RECOVERY_ALPN {
            return Err(RecoveryError::Rejected);
        }
        let anchor = credential.anchor();
        let statement = proof_statement(connection, anchor, local_id, expected_contact)?;
        let signature = credential.root.sign(&statement);
        let exchange = connection
            .open_control_exchange()
            .await
            .map_err(|_| RecoveryError::Transport)?;
        write_control_fixed(&exchange, &signature.to_bytes()).await?;

        let wire = read_control_frame(&exchange, MAX_HISTORY_BYTES).await?;
        let (mut records, scope, secret) = parse_history(&wire, anchor)?;
        let genesis = records
            .first()
            .map(|record| Hash::digest(record.header()))
            .ok_or(RecoveryError::Rejected)?;
        if genesis != anchor.genesis {
            return Err(RecoveryError::Rejected);
        }
        let raw: Vec<_> = records
            .iter()
            .map(|record| {
                (
                    record.header().to_vec(),
                    record.body().map(|body| body.to_vec()),
                )
            })
            .collect();
        let mut auth = ManagementAuth::replay_records(anchor, &raw)?;
        let contact_key = endpoint_key(expected_contact)?;
        if !auth.nodes().iter().any(|(node, _)| *node == contact_key) {
            return Err(RecoveryError::Rejected);
        }

        // A restored root uses a fresh author log. The graph's heads express
        // causality; an unavailable former root device need not provide its tail.
        let log_id = loop {
            let candidate: [u8; 16] = rand::random();
            if !auth.has_author_log_id(anchor.root, candidate) {
                break candidate;
            }
        };
        let grant = ManagementRecord::new(
            Header::<ManagementExtension>::builder()
                .build(
                    &credential.root,
                    ManagementExtension {
                        version: 1,
                        log_id,
                        group: GroupsExtensionArgs {
                            group_id: anchor.root,
                            action: GroupAction::Add {
                                member: GroupMember::Individual(endpoint_key(local_id)?),
                                access: Access::manage(),
                            },
                            dependencies: auth.tips(),
                        },
                    },
                )
                .encode(),
            None,
        );
        let grant_id = auth.accept_record(grant.header(), grant.body())?;
        write_control_frame(&exchange, grant.header()).await?;
        let mut acknowledgment = [0; 32];
        read_control_fixed(&exchange, &mut acknowledgment).await?;
        if acknowledgment != *grant_id.as_bytes() {
            return Err(RecoveryError::Rejected);
        }
        exchange
            .finish_send()
            .await
            .map_err(|_| RecoveryError::Transport)?;
        records.push(grant);
        Ok(RecoveredNetwork {
            anchor,
            records,
            scope,
            secret,
            grant_id,
        })
    })
    .await
    .unwrap_or(Err(RecoveryError::Timeout));
    result
}

/// A currently authorized member can serve a root recovery without having
/// administrator authority. The root proof is checked before any history or
/// secret bytes are sent. The grant must commit to the local management view.
pub async fn serve_recovery(
    endpoint: &Endpoint,
    connection: &Connection,
    store: &mut ManagementStore,
    scope: &AdmissionScope,
    secret: &AdmissionSecret,
) -> Result<Hash, RecoveryError> {
    serve_recovery_at(endpoint.id(), connection, store, scope, secret).await
}

/// Verify the original root proof before releasing history on a restricted
/// control session. The returned grant has been durably applied by this member.
pub async fn serve_control_recovery(
    connection: &ControlHandle,
    store: &mut ManagementStore,
    scope: &AdmissionScope,
    secret: &AdmissionSecret,
) -> Result<Hash, RecoveryError> {
    serve_recovery_at(connection.local_id(), connection, store, scope, secret).await
}

async fn serve_recovery_at(
    local_id: EndpointId,
    connection: &(impl AuthenticatedControl + ?Sized),
    store: &mut ManagementStore,
    scope: &AdmissionScope,
    secret: &AdmissionSecret,
) -> Result<Hash, RecoveryError> {
    let (_permit, _session) = SessionGuard::begin(connection)?;
    let result = tokio::time::timeout(TIMEOUT, async {
        store.ensure_admissions_open().await?;
        let anchor = store.anchor();
        if scope.network_id() != anchor.root.as_bytes() || connection.alpn() != RECOVERY_ALPN {
            return Err(RecoveryError::Rejected);
        }
        let local = endpoint_key(local_id)?;
        if !store.nodes().iter().any(|(node, _)| *node == local) || local == anchor.root {
            return Err(RecoveryError::Rejected);
        }
        let statement = proof_statement(connection, anchor, connection.remote_id(), local_id)?;
        let exchange = connection
            .accept_control_exchange()
            .await
            .map_err(|_| RecoveryError::Transport)?;
        let mut signature_bytes = [0; 64];
        read_control_fixed(&exchange, &mut signature_bytes).await?;
        if !anchor
            .root
            .verify(&statement, &Signature::from_bytes(&signature_bytes))
        {
            return Err(RecoveryError::Rejected);
        }
        let records = store.applied_records().await?;
        if records.first().map(|record| Hash::digest(record.header())) != Some(anchor.genesis) {
            return Err(RecoveryError::Rejected);
        }
        store.ensure_admissions_open().await?;
        write_control_frame(&exchange, &history_wire(&records, scope, secret)?).await?;
        let grant =
            ManagementRecord::new(read_control_frame(&exchange, MAX_GRANT_BYTES).await?, None);
        let header = Header::<ManagementExtension>::decode(grant.header())
            .map_err(|_| RecoveryError::Rejected)?;
        let recovering_node = endpoint_key(connection.remote_id())?;
        if header.verifying_key != anchor.root
            || header.seq_num != 0
            || header.backlink.is_some()
            || store.has_author_log_id(anchor.root, header.extensions.log_id)
            || header.extensions.group.dependencies != store.tips()
            || !matches!(
                &header.extensions.group.action,
                GroupAction::Add { member: GroupMember::Individual(node), access }
                    if *node == recovering_node && *access == Access::manage()
            )
        {
            return Err(RecoveryError::Rejected);
        }
        let grant_id = store.ingest(&grant).await?;
        store.apply_ready().await?;
        if store.get_record(grant_id).await?.is_none() {
            return Err(RecoveryError::Rejected);
        }
        if !store.nodes().contains(&(recovering_node, Access::manage())) {
            return Err(RecoveryError::Rejected);
        }
        write_control_fixed(&exchange, grant_id.as_bytes()).await?;
        exchange
            .finish_send()
            .await
            .map_err(|_| RecoveryError::Transport)?;
        // RFC 9266's binding is one authentication instance per TLS session.
        // The recovering side closes after reading this acknowledgment.
        let _ = connection.revocation_handle().closed().await;
        Ok(grant_id)
    })
    .await
    .unwrap_or(Err(RecoveryError::Timeout));
    result
}
