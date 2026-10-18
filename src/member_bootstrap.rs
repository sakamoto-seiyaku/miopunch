//! Member history exchange after a scoped SCRAM proof on the same iroh TLS
//! connection. Discovery hints never enter this authorization decision.

use std::time::Duration;

use iroh::{
    Endpoint,
    endpoint::{Connection, ControlHandle, ControlMessageFormat},
};
use iroh_base::EndpointId;
use p2panda_core::{Hash, VerifyingKey};
use thiserror::Error;

use crate::{
    admission_proof::{AdmissionScope, BootstrapProof},
    management_auth::ManagementAuth,
    management_store::{ManagementRecord, ManagementStore, StoreError},
};

pub const MEMBER_BOOTSTRAP_ALPN: &[u8] = b"miopunch/member-bootstrap/1";
const MAX_HISTORY: usize = 16 * 1024 * 1024;
const MAX_RECORDS: usize = 4096;
const TIMEOUT: Duration = Duration::from_secs(12);

#[derive(Debug, Error)]
pub enum MemberBootstrapError {
    #[error("member bootstrap rejected")]
    Rejected,
    #[error("member bootstrap timed out")]
    Timeout,
    #[error("member bootstrap transport failed")]
    Transport,
    #[error("member bootstrap storage failed: {0}")]
    Store(#[from] StoreError),
}

fn endpoint_verifying_key(id: EndpointId) -> Result<VerifyingKey, MemberBootstrapError> {
    VerifyingKey::from_bytes(id.as_bytes()).map_err(|_| MemberBootstrapError::Rejected)
}

fn is_current_member(
    store: &ManagementStore,
    id: EndpointId,
) -> Result<bool, MemberBootstrapError> {
    let key = endpoint_verifying_key(id)?;
    Ok(store.nodes().iter().any(|(node, _)| *node == key))
}

async fn authorized(
    connection: &ControlHandle,
    proof: &BootstrapProof,
    scope: &AdmissionScope,
    store: &ManagementStore,
) -> Result<(), MemberBootstrapError> {
    if connection.alpn() != MEMBER_BOOTSTRAP_ALPN
        || scope.network_id() != store.anchor().root.as_bytes()
        || !proof.applies_to(connection, scope)
        || proof.peer() != connection.remote_id()
        || !is_current_member(store, connection.id())?
        || store.known_removed(endpoint_verifying_key(connection.remote_id())?)
        || !store
            .current_admission_matches(scope, proof.credential_fingerprint())
            .await?
    {
        return Err(MemberBootstrapError::Rejected);
    }
    Ok(())
}

async fn wait_for_ip(connection: &ControlHandle) -> Result<(), MemberBootstrapError> {
    tokio::time::timeout(TIMEOUT, async {
        loop {
            if connection.close_reason().is_some() {
                return Err(MemberBootstrapError::Transport);
            }
            if connection
                .paths()
                .iter()
                .any(|path| matches!(path.remote_addr(), iroh_base::TransportAddr::Ip(_)))
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|_| MemberBootstrapError::Timeout)?
}

/// The serving member checks the other identity against its currently applied
/// view before releasing a single byte of management history. Records travel
/// only after this exact connection has a verified data path.
pub async fn serve_control_history(
    connection: &ControlHandle,
    proof: &BootstrapProof,
    scope: &AdmissionScope,
    store: &ManagementStore,
) -> Result<(), MemberBootstrapError> {
    let result = tokio::time::timeout(TIMEOUT, async {
        authorized(connection, proof, scope, store).await?;
        if !is_current_member(store, connection.remote_id())? {
            return Err(MemberBootstrapError::Rejected);
        }
        wait_for_ip(connection).await?;
        connection
            .commit_data_path()
            .await
            .map_err(|_| MemberBootstrapError::Transport)?;
        let exchange = connection
            .accept_control_exchange()
            .await
            .map_err(control_error)?;
        let request = exchange
            .recv_message(ControlMessageFormat::Fixed { length: 4 })
            .await
            .map_err(control_error)?;
        if request != b"HIST" {
            return Err(MemberBootstrapError::Rejected);
        }
        if !store
            .current_admission_matches(scope, proof.credential_fingerprint())
            .await?
        {
            return Err(MemberBootstrapError::Rejected);
        }
        let records = store.applied_records().await?;
        if records.is_empty()
            || records.len() > MAX_RECORDS
            || Hash::digest(records[0].header()) != store.anchor().genesis
        {
            return Err(MemberBootstrapError::Rejected);
        }
        let raw: Vec<_> = records
            .iter()
            .map(|record| (record.header(), record.body()))
            .collect();
        let bytes =
            p2panda_core::cbor::encode_cbor(&raw).map_err(|_| MemberBootstrapError::Rejected)?;
        if bytes.len() > MAX_HISTORY {
            return Err(MemberBootstrapError::Rejected);
        }
        exchange
            .send_message(
                ControlMessageFormat::LengthPrefixed32 {
                    maximum: MAX_HISTORY,
                },
                &bytes,
            )
            .await
            .map_err(control_error)?;
        exchange.finish_send().await.map_err(control_error)?;
        Ok(())
    })
    .await
    .unwrap_or(Err(MemberBootstrapError::Timeout));
    if result.is_err() {
        connection.close(0u32.into(), b"member bootstrap rejected");
    }
    result
}

/// Receive complete signed records from a discovered unknown node. Replay all
/// of them against the pinned anchor and verify both members before changing
/// the local durable view. A proof of the shared secret alone is insufficient.
pub async fn fetch_control_history(
    connection: &ControlHandle,
    proof: &BootstrapProof,
    scope: &AdmissionScope,
    store: &mut ManagementStore,
    data_addr: std::net::SocketAddr,
) -> Result<usize, MemberBootstrapError> {
    let result = tokio::time::timeout(TIMEOUT, async {
        authorized(connection, proof, scope, store).await?;
        // A known revoked identity must not receive even a history request.
        // Unknown identities can be checked only after receiving signed state.
        connection
            .open_data_path(data_addr)
            .await
            .map_err(|_| MemberBootstrapError::Transport)?;
        wait_for_ip(connection).await?;
        connection
            .commit_data_path()
            .await
            .map_err(|_| MemberBootstrapError::Transport)?;
        let exchange = connection
            .open_control_exchange()
            .await
            .map_err(control_error)?;
        exchange
            .send_message(ControlMessageFormat::Fixed { length: 4 }, b"HIST")
            .await
            .map_err(control_error)?;
        let bytes = exchange
            .recv_message(ControlMessageFormat::LengthPrefixed32 {
                maximum: MAX_HISTORY,
            })
            .await
            .map_err(control_error)?;
        if bytes.is_empty() {
            return Err(MemberBootstrapError::Rejected);
        }
        if !store
            .current_admission_matches(scope, proof.credential_fingerprint())
            .await?
        {
            return Err(MemberBootstrapError::Rejected);
        }
        let raw: Vec<(Vec<u8>, Option<Vec<u8>>)> =
            p2panda_core::cbor::decode_cbor(bytes.as_slice())
                .map_err(|_| MemberBootstrapError::Rejected)?;
        if raw.is_empty()
            || raw.len() > MAX_RECORDS
            || Hash::digest(&raw[0].0) != store.anchor().genesis
        {
            return Err(MemberBootstrapError::Rejected);
        }
        let auth = ManagementAuth::replay_records(store.anchor(), &raw)
            .map_err(|_| MemberBootstrapError::Rejected)?;
        let peer_key = endpoint_verifying_key(connection.remote_id())?;
        let own_key = endpoint_verifying_key(connection.id())?;
        let nodes = auth.nodes();
        if !nodes.iter().any(|(node, _)| *node == peer_key)
            || !nodes.iter().any(|(node, _)| *node == own_key)
        {
            return Err(MemberBootstrapError::Rejected);
        }
        let old_count = store.applied_count();
        for (header, body) in raw {
            store
                .ingest(&ManagementRecord::new(header, body.as_deref()))
                .await?;
        }
        store.apply_ready().await?;
        if !is_current_member(store, connection.remote_id())?
            || !is_current_member(store, connection.id())?
        {
            return Err(MemberBootstrapError::Rejected);
        }
        Ok((store.applied_count() - old_count) as usize)
    })
    .await
    .unwrap_or(Err(MemberBootstrapError::Timeout));
    if result.is_err() {
        connection.close(0u32.into(), b"member bootstrap rejected");
    }
    result
}

fn control_error(error: std::io::Error) -> MemberBootstrapError {
    if error.kind() == std::io::ErrorKind::InvalidData {
        MemberBootstrapError::Rejected
    } else {
        MemberBootstrapError::Transport
    }
}

/// History exchange for an already-owned ordinary connection.
pub async fn serve_history(
    endpoint: &Endpoint,
    connection: &Connection,
    proof: &BootstrapProof,
    scope: &AdmissionScope,
    store: &ManagementStore,
) -> Result<(), MemberBootstrapError> {
    serve_control_history(&endpoint.control_handle(connection), proof, scope, store).await
}

/// History exchange for an already-owned ordinary connection.
pub async fn fetch_history(
    endpoint: &Endpoint,
    connection: &Connection,
    proof: &BootstrapProof,
    scope: &AdmissionScope,
    store: &mut ManagementStore,
    data_addr: std::net::SocketAddr,
) -> Result<usize, MemberBootstrapError> {
    fetch_control_history(
        &endpoint.control_handle(connection),
        proof,
        scope,
        store,
        data_addr,
    )
    .await
}
