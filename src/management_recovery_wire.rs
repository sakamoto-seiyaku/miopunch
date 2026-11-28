//! Existing exporter-bound recovery proof and its portable durable evidence.
//! Retained evidence verifies history; every live retry requires a new proof
//! bound to that actual TLS session before history or secrets are released.
use std::io;

use iroh::endpoint::{ControlExchange, ControlMessageFormat};
use iroh_base::EndpointId;
use p2panda_auth::{
    Access,
    group::{GroupAction, GroupMember},
};
use p2panda_core::{Hash, Header, Signature, VerifyingKey};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    admission_history_wire::{self as history, History, HistoryBinding, HistoryCheckpoint},
    control_protocol::AuthenticatedControl,
    management_auth::{ManagementAuth, ManagementExtension, NetworkAnchor},
    management_record::{MAX_PAGE_BYTES, MAX_PAGE_RECORDS},
    service_protocol::ManagementRecords,
};

pub const RECOVERY_ALPN: &[u8] = b"miopunch/management-recovery/1";
pub const DOMAIN: &[u8] = b"miopunch/genesis-recovery/v1\0";
// Single-page recovery is a CBOR pair. This distinct first frame announces
// the shared cursor exchange; its recovery-specific metadata follows the last ACK.
const PAGED: &[u8] = b"miopunch/recovery-history/pages/1";

pub(crate) fn history_binding(
    anchor: NetworkAnchor,
    provider: EndpointId,
    attempt: [u8; 16],
) -> HistoryBinding {
    // The attempt identifies the recovery; the current admission generation
    // is supplied separately after the applicable recovery proof.
    HistoryBinding {
        anchor,
        provider: *provider.as_bytes(),
        request: attempt,
        generation: 0,
    }
}

pub(crate) async fn send_history<M: Serialize>(
    exchange: &ControlExchange,
    anchor: NetworkAnchor,
    records: &ManagementRecords,
    material: &M,
) -> io::Result<()> {
    if records.first().map(|r| Hash::digest(&r.0)) != Some(anchor.genesis) {
        return Err(invalid());
    }
    if history::fits_single_page(records) {
        let frame =
            p2panda_core::cbor::encode_cbor(&(material, records)).map_err(io::Error::other)?;
        if frame.len() <= history::TARGET_BYTES {
            return send_frame(exchange, &frame).await;
        }
    }
    let pages = History::paged(anchor.genesis, records)?;
    send_frame(exchange, PAGED).await?;
    pages.send(exchange).await?;
    let metadata = p2panda_core::cbor::encode_cbor(material).map_err(io::Error::other)?;
    send_frame(exchange, &metadata).await
}

pub(crate) async fn receive_history<M: DeserializeOwned, Fut: Future<Output = io::Result<()>>>(
    exchange: &ControlExchange,
    binding: HistoryBinding,
    previous: Option<HistoryCheckpoint>,
    persist: impl FnMut(HistoryCheckpoint) -> Fut,
) -> io::Result<(M, ManagementRecords, ManagementAuth)> {
    let frame = recv_frame(exchange, MAX_PAGE_BYTES).await?;
    if frame == PAGED {
        let (genesis, records, verified) =
            history::receive(exchange, history::PAGED_HISTORY, binding, previous, persist).await?;
        if genesis != binding.anchor.genesis {
            return Err(invalid());
        }
        let metadata = recv_frame(exchange, 256).await?;
        let material =
            p2panda_core::cbor::decode_cbor(metadata.as_slice()).map_err(|_| invalid())?;
        return Ok((material, records, verified.ok_or_else(invalid)?));
    }
    let (material, records): (M, ManagementRecords) =
        p2panda_core::cbor::decode_cbor(frame.as_slice()).map_err(|_| invalid())?;
    if records.len() > MAX_PAGE_RECORDS
        || records.first().map(|r| Hash::digest(&r.0)) != Some(binding.anchor.genesis)
    {
        return Err(invalid());
    }
    let verified = ManagementAuth::replay_records(binding.anchor, &records)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok((material, records, verified))
}

pub fn proof_statement(
    connection: &(impl AuthenticatedControl + ?Sized),
    network: NetworkAnchor,
    recovering: EndpointId,
    contact: EndpointId,
) -> io::Result<Vec<u8>> {
    let mut binding = [0; 32];
    connection
        .export_keying_material(&mut binding, b"EXPORTER-Channel-Binding", b"")
        .map_err(|_| io::Error::other("channel binding unavailable"))?;
    let mut statement = DOMAIN.to_vec();
    statement.extend_from_slice(network.root.as_bytes());
    statement.extend_from_slice(network.genesis.as_bytes());
    statement.extend_from_slice(recovering.as_bytes());
    statement.extend_from_slice(contact.as_bytes());
    statement.extend_from_slice(&binding);
    Ok(statement)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecoveryIntent {
    pub contact: EndpointId,
    pub attempt: [u8; 16],
    pub authorization: Option<Hash>,
    pub complete: bool,
}
impl RecoveryIntent {
    pub fn verify(
        &self,
        owner: VerifyingKey,
        proofs: &[RecoveryProof],
        root_saved: bool,
    ) -> io::Result<()> {
        let proof = proofs
            .iter()
            .find(|p| p.target == owner && p.attempt == self.attempt);
        if self.complete && self.authorization.is_none() || !self.complete && !root_saved {
            return Err(invalid());
        }
        match proof {
            Some(proof) => {
                let contact_start = DOMAIN.len() + 96;
                if proof.authorization != self.authorization
                    || self.complete != (proof.phase == 3)
                    || proof.statement.get(contact_start..contact_start + 32)
                        != Some(self.contact.as_bytes())
                {
                    return Err(invalid());
                }
            }
            None if self.authorization.is_some() || self.complete => return Err(invalid()),
            None => {}
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecoveryProof {
    pub target: VerifyingKey,
    pub attempt: [u8; 16],
    pub authorization: Option<Hash>,
    pub statement: Vec<u8>,
    pub signature: Vec<u8>,
    pub phase: u8,
}
impl RecoveryProof {
    pub fn verify(&self, network: NetworkAnchor, records: &ManagementRecords) -> io::Result<()> {
        let start = DOMAIN.len();
        let signature: [u8; 64] = self
            .signature
            .as_slice()
            .try_into()
            .map_err(|_| invalid())?;
        if self.statement.len() != start + 32 * 5
            || !self.statement.starts_with(DOMAIN)
            || &self.statement[start..start + 32] != network.root.as_bytes()
            || &self.statement[start + 32..start + 64] != network.genesis.as_bytes()
            || &self.statement[start + 64..start + 96] != self.target.as_bytes()
            || !network
                .root
                .verify(&self.statement, &Signature::from_bytes(&signature))
            || !matches!(
                (self.phase, self.authorization.is_some()),
                (1, false) | (2 | 3, true)
            )
        {
            return Err(invalid());
        }
        if let Some(id) = self.authorization {
            let bytes = &records
                .iter()
                .find(|(h, _)| Hash::digest(h) == id)
                .ok_or_else(invalid)?
                .0;
            let header = Header::<ManagementExtension>::decode(bytes).map_err(io::Error::other)?;
            if header.verifying_key != network.root
                || header.seq_num != 0
                || header.backlink.is_some()
                || !matches!(&header.extensions.group.action,GroupAction::Add {member:GroupMember::Individual(target),access} if *target==self.target && *access==Access::manage())
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

pub async fn send_fixed(exchange: &ControlExchange, bytes: &[u8]) -> io::Result<()> {
    exchange
        .send_message(
            ControlMessageFormat::Fixed {
                length: bytes.len(),
            },
            bytes,
        )
        .await
}
pub async fn recv_fixed<const N: usize>(exchange: &ControlExchange) -> io::Result<[u8; N]> {
    exchange
        .recv_message(ControlMessageFormat::Fixed { length: N })
        .await?
        .try_into()
        .map_err(|_| invalid())
}
pub async fn send_frame(exchange: &ControlExchange, bytes: &[u8]) -> io::Result<()> {
    if bytes.is_empty() || bytes.len() > MAX_PAGE_BYTES {
        return Err(invalid());
    }
    exchange
        .send_message(
            ControlMessageFormat::LengthPrefixed32 {
                maximum: MAX_PAGE_BYTES,
            },
            bytes,
        )
        .await
}
pub async fn recv_frame(exchange: &ControlExchange, maximum: usize) -> io::Result<Vec<u8>> {
    let bytes = exchange
        .recv_message(ControlMessageFormat::LengthPrefixed32 { maximum })
        .await?;
    if bytes.is_empty() {
        return Err(invalid());
    }
    Ok(bytes)
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid retained management recovery evidence",
    )
}
