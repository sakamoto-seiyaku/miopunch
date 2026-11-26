//! Signed rotation statements and control framing shared by both SDK runtimes.
use crate::{control_protocol::AuthenticatedControl, management_auth::NetworkAnchor};
use iroh::endpoint::{ControlExchange, ControlMessageFormat};
use n0_future::time::{Duration, Instant, timeout};
use p2panda_core::{Hash, Signature, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum WireError {
    #[error("rotation identity, generation, signature or current membership rejected")]
    Rejected,
    #[error("rotation connection failed; saved progress remains retryable")]
    PathUnavailable,
}

pub const ADMISSION_ROTATION_ALPN: &[u8] = b"miopunch/admission-rotation/1";
/// Existing approved-material delivery exception, limited to a current
/// durable pending grant and this root-authorized retained replacement.
pub const ROTATION_DELIVERY_ALPN: &[u8] = b"miopunch/admission-rotation-delivery/1";
const MAX_FRAME: usize = 16 * 1024 * 1024;
pub(crate) const MAX_MEMBERS: usize = 4096;
const TIMEOUT: Duration = Duration::from_secs(15);
pub(crate) const PROPOSAL_DOMAIN: &[u8] = b"miopunch/admission-rotation/proposal/1\0";
pub(crate) const ACK_DOMAIN: &[u8] = b"miopunch/admission-rotation/durable-ack/1\0";
pub(crate) const COMMIT_DOMAIN: &[u8] = b"miopunch/admission-rotation/commit/1\0";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RotationStage {
    Distributing,
    Complete,
}

/// Local durable progress, not a claim of global propagation or convergence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RotationProgress {
    pub id: [u8; 16],
    pub generation: u64,
    pub initiator: VerifyingKey,
    pub retained: Vec<VerifyingKey>,
    pub acknowledged: Vec<VerifyingKey>,
    pub stage: RotationStage,
}

impl RotationProgress {
    pub(crate) fn public_json(&self) -> serde_json::Value {
        serde_json::json!({"id":hex::encode(self.id), "generation":self.generation,
            "initiator":self.initiator.to_string(), "stage":format!("{:?}",self.stage),
            "retained":self.retained.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "acknowledged":self.acknowledged.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "waiting":self.retained.iter().filter(|peer| !self.acknowledged.contains(peer))
                .map(ToString::to_string).collect::<Vec<_>>()})
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Signed {
    pub(crate) statement: Vec<u8>,
    pub(crate) signature: Vec<u8>,
}

pub(crate) fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, WireError> {
    p2panda_core::cbor::encode_cbor(value).map_err(|_| WireError::Rejected)
}

pub(crate) fn decode<T: for<'de> Deserialize<'de>>(value: &[u8]) -> Result<T, WireError> {
    p2panda_core::cbor::decode_cbor(value).map_err(|_| WireError::Rejected)
}

impl Signed {
    pub(crate) fn sign<T: Serialize>(
        domain: &[u8],
        value: &T,
        key: &SigningKey,
    ) -> Result<Self, WireError> {
        let mut statement = domain.to_vec();
        statement.extend(encode(value)?);
        Ok(Self {
            signature: key.sign(&statement).to_bytes().to_vec(),
            statement,
        })
    }

    pub(crate) fn verify<T: for<'de> Deserialize<'de>>(
        &self,
        domain: &[u8],
        key: VerifyingKey,
    ) -> Result<T, WireError> {
        let signature = Signature::from_bytes(
            &self
                .signature
                .as_slice()
                .try_into()
                .map_err(|_| WireError::Rejected)?,
        );
        if !self.statement.starts_with(domain) || !key.verify(&self.statement, &signature) {
            return Err(WireError::Rejected);
        }
        decode(&self.statement[domain.len()..])
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Proposal {
    pub(crate) network: [u8; 32],
    pub(crate) genesis: [u8; 32],
    pub(crate) id: [u8; 16],
    pub(crate) initiator: [u8; 32],
    pub(crate) previous_generation: u64,
    pub(crate) generation: u64,
    pub(crate) retained: Vec<[u8; 32]>,
    pub(crate) fingerprint: [u8; 32],
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Acknowledgement {
    pub(crate) proposal: [u8; 32],
    pub(crate) member: [u8; 32],
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Completion {
    pub(crate) proposal: [u8; 32],
    pub(crate) retained: Vec<[u8; 32]>,
    pub(crate) acknowledgements: Vec<Signed>,
}

pub(crate) struct SavedRotation {
    pub(crate) signed: Signed,
    pub(crate) proposal: Proposal,
    pub(crate) completion: Option<Signed>,
    pub(crate) proposal_hash: [u8; 32],
}

impl SavedRotation {
    pub(crate) fn new(
        signed: Signed,
        proposal: Proposal,
        completion: Option<Signed>,
    ) -> Result<Self, WireError> {
        let proposal_hash = *Hash::digest(encode(&signed)?).as_bytes();
        Ok(Self {
            signed,
            proposal,
            completion,
            proposal_hash,
        })
    }

    pub(crate) fn stage(&self) -> RotationStage {
        if self.completion.is_some() {
            RotationStage::Complete
        } else {
            RotationStage::Distributing
        }
    }

    pub(crate) fn sign_ack(&self, key: &SigningKey) -> Result<Signed, WireError> {
        Signed::sign(
            ACK_DOMAIN,
            &Acknowledgement {
                proposal: self.proposal_hash,
                member: *key.verifying_key().as_bytes(),
            },
            key,
        )
    }
}

pub(crate) type HistoryFrame = (
    Signed,
    Option<Signed>,
    Vec<(Vec<u8>, Option<Vec<u8>>)>,
    Option<(Signed, Signed)>,
);

/// The dialing side names its active generation on a separate stream before
/// either side attempts SCRAM. Only the listener can select staged material.
pub(crate) async fn negotiate_generation(
    connection: &(impl AuthenticatedControl + ?Sized),
    active: u64,
) -> Result<u64, WireError> {
    let deadline =
        Instant::now() + crate::connection_budget::remaining(connection.caller_deadline());
    let exchange = if connection.side().is_client() {
        let wire = RotationExchange {
            control: wire_io(deadline, connection.open_control_exchange()).await?,
            deadline,
        };
        wire.write_fixed(&active.to_be_bytes()).await?;
        wire.finish_send().await?;
        return Ok(active);
    } else {
        wire_io(deadline, connection.accept_control_exchange()).await?
    };
    let wire = RotationExchange {
        control: exchange,
        deadline,
    };
    let mut selected = [0; 8];
    wire.read_fixed(&mut selected).await?;
    wire.finish_send().await?;
    Ok(u64::from_be_bytes(selected))
}

pub(crate) fn keys(values: &[[u8; 32]]) -> Result<Vec<VerifyingKey>, WireError> {
    values
        .iter()
        .map(|bytes| VerifyingKey::from_bytes(bytes).map_err(|_| WireError::Rejected))
        .collect()
}

fn ordered_members(values: &[[u8; 32]]) -> bool {
    !values.is_empty()
        && values.len() <= MAX_MEMBERS
        && values.windows(2).all(|pair| pair[0] < pair[1])
}

pub(crate) fn verify_proposal(
    anchor: NetworkAnchor,
    signed: &Signed,
) -> Result<Proposal, WireError> {
    let proposal: Proposal = signed.verify(PROPOSAL_DOMAIN, anchor.root)?;
    if proposal.network != *anchor.root.as_bytes()
        || proposal.genesis != *anchor.genesis.as_bytes()
        || proposal.previous_generation.checked_add(1) != Some(proposal.generation)
        || proposal.generation > i64::MAX as u64
        || !ordered_members(&proposal.retained)
        || proposal.retained.contains(anchor.root.as_bytes())
        || !proposal.retained.contains(&proposal.initiator)
    {
        return Err(WireError::Rejected);
    }
    keys(&proposal.retained)?;
    Ok(proposal)
}

pub(crate) fn verify_ack(signed: &Signed, rotation: &SavedRotation) -> Result<[u8; 32], WireError> {
    // Decode only to identify the signature key; no fact is counted until the
    // actual Ed25519 signature and exact durable proposal binding verify.
    if !signed.statement.starts_with(ACK_DOMAIN) {
        return Err(WireError::Rejected);
    }
    let claimed: Acknowledgement = decode(&signed.statement[ACK_DOMAIN.len()..])?;
    let key = VerifyingKey::from_bytes(&claimed.member).map_err(|_| WireError::Rejected)?;
    let ack: Acknowledgement = signed.verify(ACK_DOMAIN, key)?;
    if ack.proposal != rotation.proposal_hash
        || rotation
            .proposal
            .retained
            .binary_search(&ack.member)
            .is_err()
    {
        return Err(WireError::Rejected);
    }
    Ok(ack.member)
}

pub(crate) fn verify_completion(
    rotation: &SavedRotation,
    root: VerifyingKey,
    signed: &Signed,
) -> Result<Completion, WireError> {
    let completion: Completion = signed.verify(COMMIT_DOMAIN, root)?;
    if completion.proposal != rotation.proposal_hash
        || !ordered_members(&completion.retained)
        || !completion.retained.contains(&rotation.proposal.initiator)
        || completion
            .retained
            .iter()
            .any(|member| rotation.proposal.retained.binary_search(member).is_err())
        || completion.acknowledgements.len() != completion.retained.len()
    {
        return Err(WireError::Rejected);
    }
    let mut acknowledged = completion
        .acknowledgements
        .iter()
        .map(|ack| verify_ack(ack, rotation))
        .collect::<Result<Vec<_>, _>>()?;
    acknowledged.sort();
    if acknowledged != completion.retained {
        return Err(WireError::Rejected);
    }
    Ok(completion)
}

pub(crate) fn rotation_deadline(connection: &(impl AuthenticatedControl + ?Sized)) -> Instant {
    let local = Instant::now() + TIMEOUT;
    connection
        .caller_deadline()
        .map_or(local, |deadline| local.min(deadline))
}
pub(crate) async fn wire_io<T>(
    deadline: Instant,
    operation: impl Future<Output = std::io::Result<T>>,
) -> Result<T, WireError> {
    if deadline <= Instant::now() {
        return Err(WireError::PathUnavailable);
    }
    timeout(
        deadline.saturating_duration_since(Instant::now()),
        operation,
    )
    .await
    .map_err(|_| WireError::PathUnavailable)?
    .map_err(|error| match error.kind() {
        std::io::ErrorKind::InvalidData => WireError::Rejected,
        _ => WireError::PathUnavailable,
    })
}
pub(crate) struct RotationExchange {
    pub(crate) control: ControlExchange,
    pub(crate) deadline: Instant,
}
impl RotationExchange {
    pub(crate) async fn write_fixed(&self, bytes: &[u8]) -> Result<(), WireError> {
        let format = ControlMessageFormat::Fixed {
            length: bytes.len(),
        };
        wire_io(self.deadline, self.control.send_message(format, bytes)).await
    }

    pub(crate) async fn read_fixed<const N: usize>(
        &self,
        bytes: &mut [u8; N],
    ) -> Result<(), WireError> {
        let format = ControlMessageFormat::Fixed { length: N };
        let message = wire_io(self.deadline, self.control.recv_message(format)).await?;
        bytes.copy_from_slice(&message);
        Ok(())
    }

    pub(crate) async fn write_frame(&self, bytes: &[u8]) -> Result<(), WireError> {
        if bytes.len() > MAX_FRAME {
            return Err(WireError::Rejected);
        }
        let format = ControlMessageFormat::LengthPrefixed32 { maximum: MAX_FRAME };
        wire_io(self.deadline, self.control.send_message(format, bytes)).await
    }

    pub(crate) async fn read_frame(&self) -> Result<Vec<u8>, WireError> {
        let format = ControlMessageFormat::LengthPrefixed32 { maximum: MAX_FRAME };
        wire_io(self.deadline, self.control.recv_message(format)).await
    }

    pub(crate) async fn finish_send(&self) -> Result<(), WireError> {
        wire_io(self.deadline, self.control.finish_send()).await
    }
}
