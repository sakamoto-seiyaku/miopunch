//! Member history exchange after a scoped SCRAM proof on the same iroh TLS
//! connection. Discovery hints never enter this authorization decision.

use std::time::Duration;

use iroh::{Endpoint, endpoint::Connection};
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
    endpoint: &Endpoint,
    connection: &Connection,
    proof: &BootstrapProof,
    scope: &AdmissionScope,
    store: &ManagementStore,
) -> Result<(), MemberBootstrapError> {
    if connection.alpn() != MEMBER_BOOTSTRAP_ALPN
        || scope.network_id() != store.anchor().root.as_bytes()
        || !proof.applies_to(connection, scope)
        || proof.peer() != connection.remote_id()
        || !is_current_member(store, endpoint.id())?
        || store.known_removed(endpoint_verifying_key(connection.remote_id())?)
        || !store
            .current_admission_matches(scope, proof.credential_fingerprint())
            .await?
    {
        return Err(MemberBootstrapError::Rejected);
    }
    Ok(())
}

async fn wait_for_ip(connection: &Connection) -> Result<(), MemberBootstrapError> {
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
pub async fn serve_history(
    endpoint: &Endpoint,
    connection: &Connection,
    proof: &BootstrapProof,
    scope: &AdmissionScope,
    store: &ManagementStore,
) -> Result<(), MemberBootstrapError> {
    let result = tokio::time::timeout(TIMEOUT, async {
        authorized(endpoint, connection, proof, scope, store).await?;
        if !is_current_member(store, connection.remote_id())? {
            return Err(MemberBootstrapError::Rejected);
        }
        wait_for_ip(connection).await?;
        endpoint
            .commit_data_path(connection)
            .await
            .map_err(|_| MemberBootstrapError::Transport)?;
        let (mut send, mut recv) = connection
            .accept_bi()
            .await
            .map_err(|_| MemberBootstrapError::Transport)?;
        let mut request = [0; 4];
        recv.read_exact(&mut request)
            .await
            .map_err(|_| MemberBootstrapError::Transport)?;
        if &request != b"HIST" {
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
        send.write_all(&(bytes.len() as u32).to_be_bytes())
            .await
            .map_err(|_| MemberBootstrapError::Transport)?;
        send.write_all(&bytes)
            .await
            .map_err(|_| MemberBootstrapError::Transport)?;
        send.finish().map_err(|_| MemberBootstrapError::Transport)?;
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
pub async fn fetch_history(
    endpoint: &Endpoint,
    connection: &Connection,
    proof: &BootstrapProof,
    scope: &AdmissionScope,
    store: &mut ManagementStore,
    data_addr: std::net::SocketAddr,
) -> Result<usize, MemberBootstrapError> {
    let result = tokio::time::timeout(TIMEOUT, async {
        authorized(endpoint, connection, proof, scope, store).await?;
        // A known revoked identity must not receive even a history request.
        // Unknown identities can be checked only after receiving signed state.
        endpoint
            .open_data_path(connection, data_addr)
            .await
            .map_err(|_| MemberBootstrapError::Transport)?;
        wait_for_ip(connection).await?;
        endpoint
            .commit_data_path(connection)
            .await
            .map_err(|_| MemberBootstrapError::Transport)?;
        let (mut send, mut recv) = connection
            .open_bi()
            .await
            .map_err(|_| MemberBootstrapError::Transport)?;
        send.write_all(b"HIST")
            .await
            .map_err(|_| MemberBootstrapError::Transport)?;
        let mut len = [0; 4];
        recv.read_exact(&mut len)
            .await
            .map_err(|_| MemberBootstrapError::Transport)?;
        let len = u32::from_be_bytes(len) as usize;
        if len == 0 || len > MAX_HISTORY {
            return Err(MemberBootstrapError::Rejected);
        }
        let mut bytes = vec![0; len];
        recv.read_exact(&mut bytes)
            .await
            .map_err(|_| MemberBootstrapError::Transport)?;
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
        let own_key = endpoint_verifying_key(endpoint.id())?;
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
            || !is_current_member(store, endpoint.id())?
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
