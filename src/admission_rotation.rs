//! Durable admission-secret rotation. Signed control statements contain only
//! identities, generations and credential tags; secret bytes stay in private
//! files and admitted TLS streams. Staged credentials never grant business.
use std::fs;
use std::path::Path;
use std::time::Duration;

use crate::control_protocol::AuthenticatedControl;
use iroh::endpoint::{ControlExchange, ControlMessageFormat};
use p2panda_core::{Hash, Signature, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use thiserror::Error;

use crate::admission_gate::ProtocolScope;
use crate::admission_proof::{AdmissionScope, AdmissionSecret};
use crate::daily_management::{ManagementAction, ManagementError, ManagementSession};
use crate::management_store::{ManagementRecord, ManagementStore, StoreError};
use crate::native_network::{NativeNetwork, NodeError, write_private};

pub const ADMISSION_ROTATION_ALPN: &[u8] = b"miopunch/admission-rotation/1";
/// Existing approved-material delivery exception, limited to a current
/// durable pending grant and this root-authorized retained replacement.
pub const ROTATION_DELIVERY_ALPN: &[u8] = b"miopunch/admission-rotation-delivery/1";
const MAX_FRAME: usize = 16 * 1024 * 1024;
const MAX_MEMBERS: usize = 4096;
const TIMEOUT: Duration = Duration::from_secs(15);
const PROPOSAL_DOMAIN: &[u8] = b"miopunch/admission-rotation/proposal/1\0";
const ACK_DOMAIN: &[u8] = b"miopunch/admission-rotation/durable-ack/1\0";
const COMMIT_DOMAIN: &[u8] = b"miopunch/admission-rotation/commit/1\0";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RotationStage {
    Distributing,
    Complete,
}

/// Wire-only mutations after genuine durable receipt. Available exclusively
/// to explicit fault builds; signing, transport and storage remain real.
#[cfg(feature = "replica-test-hooks")]
pub enum RotationAckFault {
    ForgedSignature,
    ForgedForwardedSignature,
    WrongIdentity(SigningKey),
    PreviousRotation,
}

enum ReceiptMode {
    Normal,
    #[cfg(feature = "replica-test-hooks")]
    Fault(RotationAckFault),
}

/// Local durable progress, not a claim of global propagation or convergence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RotationProgress {
    pub id: [u8; 16],
    pub generation: u64,
    pub retained: Vec<VerifyingKey>,
    pub acknowledged: Vec<VerifyingKey>,
    pub stage: RotationStage,
}

#[derive(Debug, Error)]
pub enum RotationError {
    #[error("rotation identity, generation, signature or current membership rejected")]
    Rejected,
    #[error("retained members still require durable acknowledgement")]
    Waiting,
    #[error("current retained membership changed; no switch was made")]
    RetainedChanged,
    #[error("rotation connection failed; saved progress remains retryable")]
    PathUnavailable,
    #[error("rotation needs a current genesis session: {0}")]
    Management(#[from] ManagementError),
    #[error("rotation storage failed: {0}")]
    Store(#[from] StoreError),
    #[error("rotation node persistence failed: {0}")]
    Node(#[from] NodeError),
}

#[derive(Clone, Serialize, Deserialize)]
struct Signed {
    statement: Vec<u8>,
    signature: Vec<u8>,
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, RotationError> {
    p2panda_core::cbor::encode_cbor(value).map_err(|_| RotationError::Rejected)
}

fn decode<T: for<'de> Deserialize<'de>>(value: &[u8]) -> Result<T, RotationError> {
    p2panda_core::cbor::decode_cbor(value).map_err(|_| RotationError::Rejected)
}

impl Signed {
    fn sign<T: Serialize>(
        domain: &[u8],
        value: &T,
        key: &SigningKey,
    ) -> Result<Self, RotationError> {
        let mut statement = domain.to_vec();
        statement.extend(encode(value)?);
        Ok(Self {
            signature: key.sign(&statement).to_bytes().to_vec(),
            statement,
        })
    }

    fn verify<T: for<'de> Deserialize<'de>>(
        &self,
        domain: &[u8],
        key: VerifyingKey,
    ) -> Result<T, RotationError> {
        let signature = Signature::from_bytes(
            &self
                .signature
                .as_slice()
                .try_into()
                .map_err(|_| RotationError::Rejected)?,
        );
        if !self.statement.starts_with(domain) || !key.verify(&self.statement, &signature) {
            return Err(RotationError::Rejected);
        }
        decode(&self.statement[domain.len()..])
    }

    fn hash(&self) -> Result<Hash, RotationError> {
        Ok(Hash::digest(encode(self)?))
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Proposal {
    network: [u8; 32],
    genesis: [u8; 32],
    id: [u8; 16],
    initiator: [u8; 32],
    previous_generation: u64,
    generation: u64,
    retained: Vec<[u8; 32]>,
    fingerprint: [u8; 32],
}

#[derive(Clone, Serialize, Deserialize)]
struct Acknowledgement {
    proposal: [u8; 32],
    member: [u8; 32],
}

#[derive(Clone, Serialize, Deserialize)]
struct Completion {
    proposal: [u8; 32],
    retained: Vec<[u8; 32]>,
    acknowledgements: Vec<Signed>,
}

struct SavedRotation {
    signed: Signed,
    proposal: Proposal,
    completion: Option<Signed>,
}

fn keys(values: &[[u8; 32]]) -> Result<Vec<VerifyingKey>, RotationError> {
    values
        .iter()
        .map(|bytes| VerifyingKey::from_bytes(bytes).map_err(|_| RotationError::Rejected))
        .collect()
}

fn ordered_members(values: &[[u8; 32]]) -> bool {
    !values.is_empty()
        && values.len() <= MAX_MEMBERS
        && values.windows(2).all(|pair| pair[0] < pair[1])
}

fn verify_proposal(store: &ManagementStore, signed: &Signed) -> Result<Proposal, RotationError> {
    let proposal: Proposal = signed.verify(PROPOSAL_DOMAIN, store.anchor().root)?;
    if proposal.network != *store.anchor().root.as_bytes()
        || proposal.genesis != *store.anchor().genesis.as_bytes()
        || proposal.previous_generation.checked_add(1) != Some(proposal.generation)
        || proposal.generation > i64::MAX as u64
        || !ordered_members(&proposal.retained)
        || proposal.retained.contains(store.anchor().root.as_bytes())
        || !proposal.retained.contains(&proposal.initiator)
    {
        return Err(RotationError::Rejected);
    }
    keys(&proposal.retained)?;
    Ok(proposal)
}

fn verify_ack(signed: &Signed, rotation: &SavedRotation) -> Result<[u8; 32], RotationError> {
    // Decode only to identify the signature key; no fact is counted until the
    // actual Ed25519 signature and exact durable proposal binding verify.
    if !signed.statement.starts_with(ACK_DOMAIN) {
        return Err(RotationError::Rejected);
    }
    let claimed: Acknowledgement = decode(&signed.statement[ACK_DOMAIN.len()..])?;
    let key = VerifyingKey::from_bytes(&claimed.member).map_err(|_| RotationError::Rejected)?;
    let ack: Acknowledgement = signed.verify(ACK_DOMAIN, key)?;
    if ack.proposal != *rotation.signed.hash()?.as_bytes()
        || !rotation.proposal.retained.contains(&ack.member)
    {
        return Err(RotationError::Rejected);
    }
    Ok(ack.member)
}

fn verify_completion(
    rotation: &SavedRotation,
    root: VerifyingKey,
    signed: &Signed,
) -> Result<Completion, RotationError> {
    let completion: Completion = signed.verify(COMMIT_DOMAIN, root)?;
    if completion.proposal != *rotation.signed.hash()?.as_bytes()
        || !ordered_members(&completion.retained)
        || !completion.retained.contains(&rotation.proposal.initiator)
        || completion
            .retained
            .iter()
            .any(|member| !rotation.proposal.retained.contains(member))
        || completion.acknowledgements.len() != completion.retained.len()
    {
        return Err(RotationError::Rejected);
    }
    let mut acknowledged = completion
        .acknowledgements
        .iter()
        .map(|ack| verify_ack(ack, rotation))
        .collect::<Result<Vec<_>, _>>()?;
    acknowledged.sort();
    if acknowledged != completion.retained {
        return Err(RotationError::Rejected);
    }
    Ok(completion)
}

impl ManagementStore {
    pub(crate) async fn ensure_admissions_open(&self) -> Result<(), StoreError> {
        let pending: i64 =
            sqlx::query_scalar("SELECT count(*) FROM admission_rotations WHERE completion IS NULL")
                .fetch_one(&self.pool)
                .await?;
        if pending != 0 {
            return Err(StoreError::AdmissionPaused);
        }
        Ok(())
    }

    async fn saved_rotation(&self) -> Result<Option<SavedRotation>, RotationError> {
        let row = sqlx::query("SELECT generation, proposal, completion FROM admission_rotations ORDER BY generation DESC LIMIT 1")
            .fetch_optional(&self.pool).await.map_err(StoreError::from)?;
        let Some(row) = row else { return Ok(None) };
        let bytes: Vec<u8> = row.try_get("proposal").map_err(StoreError::from)?;
        let completion: Option<Vec<u8>> = row.try_get("completion").map_err(StoreError::from)?;
        let signed = decode(&bytes)?;
        let proposal = verify_proposal(self, &signed)?;
        let generation: i64 = row.try_get("generation").map_err(StoreError::from)?;
        if generation < 0 || generation as u64 != proposal.generation {
            return Err(RotationError::Rejected);
        }
        let result = SavedRotation {
            signed,
            proposal,
            completion: completion.as_deref().map(decode).transpose()?,
        };
        if let Some(signed) = &result.completion {
            verify_completion(&result, self.anchor().root, signed)?;
        }
        Ok(Some(result))
    }

    async fn predecessor_completion(
        &self,
        generation: u64,
    ) -> Result<Option<(Signed, Signed)>, RotationError> {
        let row = sqlx::query("SELECT proposal, completion FROM admission_rotations WHERE generation=? AND completion IS NOT NULL")
            .bind(generation as i64).fetch_optional(&self.pool).await.map_err(StoreError::from)?;
        let Some(row) = row else { return Ok(None) };
        let proposal: Vec<u8> = row.try_get("proposal").map_err(StoreError::from)?;
        let completion: Vec<u8> = row.try_get("completion").map_err(StoreError::from)?;
        let signed = decode(&proposal)?;
        let proposal = verify_proposal(self, &signed)?;
        if proposal.generation != generation {
            return Err(RotationError::Rejected);
        }
        let completed: Signed = decode(&completion)?;
        let rotation = SavedRotation {
            signed: signed.clone(),
            proposal,
            completion: Some(completed.clone()),
        };
        verify_completion(&rotation, self.anchor().root, &completed)?;
        Ok(Some((signed, completed)))
    }

    async fn save_rotation(&self, signed: &Signed) -> Result<(), RotationError> {
        let proposal = verify_proposal(self, signed)?;
        if let Some(current) = self.saved_rotation().await? {
            if current.proposal.generation == proposal.generation {
                return if encode(&current.signed)? == encode(signed)? {
                    Ok(())
                } else {
                    Err(RotationError::Rejected)
                };
            }
            if current.completion.is_none() || current.proposal.generation >= proposal.generation {
                return Err(RotationError::Rejected);
            }
        }
        sqlx::query("INSERT INTO admission_rotations (generation, proposal) VALUES (?, ?)")
            .bind(proposal.generation as i64)
            .bind(encode(signed)?)
            .execute(&self.pool)
            .await
            .map_err(StoreError::from)?;
        Ok(())
    }

    async fn rotation_acks(&self, rotation: &SavedRotation) -> Result<Vec<Signed>, RotationError> {
        let bytes: Vec<Vec<u8>> = sqlx::query_scalar(
            "SELECT acknowledgement FROM admission_rotation_acks WHERE rotation=? ORDER BY member",
        )
        .bind(rotation.proposal.id.as_slice())
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::from)?;
        let acks = bytes
            .iter()
            .map(|bytes| decode(bytes))
            .collect::<Result<Vec<Signed>, _>>()?;
        for ack in &acks {
            verify_ack(ack, rotation)?;
        }
        Ok(acks)
    }

    async fn save_rotation_ack(
        &self,
        rotation: &SavedRotation,
        signed: &Signed,
    ) -> Result<(), RotationError> {
        let member = verify_ack(signed, rotation)?;
        let bytes = encode(signed)?;
        // SQLite FULL commits before the caller can count this acknowledgement.
        sqlx::query("INSERT OR IGNORE INTO admission_rotation_acks VALUES (?, ?, ?)")
            .bind(rotation.proposal.id.as_slice())
            .bind(member.as_slice())
            .bind(&bytes)
            .execute(&self.pool)
            .await
            .map_err(StoreError::from)?;
        let saved: Vec<u8> = sqlx::query_scalar(
            "SELECT acknowledgement FROM admission_rotation_acks WHERE rotation=? AND member=?",
        )
        .bind(rotation.proposal.id.as_slice())
        .bind(member.as_slice())
        .fetch_one(&self.pool)
        .await
        .map_err(StoreError::from)?;
        if saved != bytes {
            return Err(RotationError::Rejected);
        }
        Ok(())
    }

    async fn save_rotation_completion(
        &self,
        rotation: &SavedRotation,
        signed: &Signed,
    ) -> Result<(), RotationError> {
        verify_completion(rotation, self.anchor().root, signed)?;
        let bytes = encode(signed)?;
        if let Some(existing) = &rotation.completion {
            return if encode(existing)? == bytes {
                Ok(())
            } else {
                Err(RotationError::Rejected)
            };
        }
        let updated = sqlx::query("UPDATE admission_rotations SET completion=? WHERE generation=? AND proposal=? AND completion IS NULL")
            .bind(bytes).bind(rotation.proposal.generation as i64).bind(encode(&rotation.signed)?)
            .execute(&self.pool).await.map_err(StoreError::from)?;
        if updated.rows_affected() != 1 {
            return Err(RotationError::Rejected);
        }
        Ok(())
    }
}

fn material_name(generation: u64) -> String {
    format!("rotation.material.{generation:016x}")
}

fn staged_material(
    path: &Path,
    rotation: &SavedRotation,
) -> Result<AdmissionSecret, RotationError> {
    let bytes: [u8; 32] = fs::read(path.join(material_name(rotation.proposal.generation)))
        .map_err(NodeError::from)?
        .try_into()
        .map_err(|_| RotationError::Rejected)?;
    let secret = AdmissionSecret::from_saved(bytes);
    let scope = AdmissionScope::new(rotation.proposal.network, rotation.proposal.generation);
    if secret.fingerprint(&scope) != rotation.proposal.fingerprint {
        return Err(RotationError::Rejected);
    }
    Ok(secret)
}

fn save_staged(
    path: &Path,
    proposal: &Proposal,
    secret: &AdmissionSecret,
) -> Result<(), RotationError> {
    let target = path.join(material_name(proposal.generation));
    if target.exists() {
        if fs::read(&target).map_err(NodeError::from)? != secret.saved_bytes() {
            return Err(RotationError::Rejected);
        }
    } else {
        atomic_private(path, &target, secret.saved_bytes())?;
    }
    crate::native_files::sync_directory(path).map_err(NodeError::from)?;
    Ok(())
}

fn atomic_private(directory: &Path, target: &Path, bytes: &[u8]) -> Result<(), NodeError> {
    let temporary = directory.join(format!(
        ".rotation-material-{:032x}",
        rand::random::<u128>()
    ));
    let result = (|| {
        write_private(&temporary, bytes)?;
        crate::native_files::replace_file(&temporary, target)?;
        crate::native_files::sync_directory(directory)?;
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    result
}

/// A signed, committed completion makes either publication side recoverable.
/// Reopening publishes/pins the staged secret before exposing an endpoint.
pub(crate) async fn recover_rotation_material(
    store: &mut ManagementStore,
    path: &Path,
) -> Result<(), NodeError> {
    async {
        let Some(rotation) = store.saved_rotation().await? else {
            return Ok(());
        };
        if rotation.completion.is_some() {
            let active: Option<i64> =
                sqlx::query_scalar("SELECT generation FROM management_admission WHERE singleton=1")
                    .fetch_optional(&store.pool)
                    .await
                    .map_err(StoreError::from)?;
            if active.is_some_and(|active| active > rotation.proposal.generation as i64) {
                return Ok(());
            }
            let current_material = path.join("admission.material");
            if current_material.exists() {
                let bytes = fs::read(current_material).map_err(NodeError::from)?;
                if bytes.len() != 40 {
                    return Err(RotationError::Rejected);
                }
                let published = u64::from_be_bytes(bytes[..8].try_into().unwrap());
                if published > rotation.proposal.generation {
                    return Ok(());
                }
            }
            let secret = staged_material(path, &rotation)?;
            publish_current_material(path, &rotation, &secret)?;
            let scope =
                AdmissionScope::new(rotation.proposal.network, rotation.proposal.generation);
            store.set_current_admission(&scope, &secret).await?;
        } else if path
            .join(material_name(rotation.proposal.generation))
            .exists()
        {
            staged_material(path, &rotation)?;
        }
        Ok::<_, RotationError>(())
    }
    .await
    .map_err(|error| match error {
        RotationError::Store(error) => NodeError::Store(error),
        RotationError::Node(error) => error,
        _ => NodeError::InvalidData,
    })
}

fn publish_current_material(
    path: &Path,
    rotation: &SavedRotation,
    secret: &AdmissionSecret,
) -> Result<(), NodeError> {
    let mut bytes = [0; 40];
    bytes[..8].copy_from_slice(&rotation.proposal.generation.to_be_bytes());
    bytes[8..].copy_from_slice(secret.saved_bytes());
    atomic_private(path, &path.join("admission.material"), &bytes)
}

impl NativeNetwork {
    fn retained_members(&self) -> Vec<[u8; 32]> {
        let mut retained: Vec<_> = self
            .store
            .nodes()
            .into_iter()
            .filter(|(id, _)| *id != self.anchor.root)
            .map(|(id, _)| *id.as_bytes())
            .collect();
        retained.sort();
        retained
    }

    pub(crate) async fn restore_rotation_gate(&self) -> Result<(), NodeError> {
        let result = async {
            if let Some(rotation) = self.store.saved_rotation().await? {
                if rotation.completion.is_none()
                    && self
                        .directory
                        .join(material_name(rotation.proposal.generation))
                        .exists()
                {
                    let secret = staged_material(&self.directory, &rotation)?;
                    self.gate.stage_rotation_secret(
                        AdmissionScope::new(
                            rotation.proposal.network,
                            rotation.proposal.generation,
                        ),
                        secret,
                    );
                }
            }
            Ok::<_, RotationError>(())
        }
        .await;
        result.map_err(|_| NodeError::InvalidData)
    }

    pub async fn rotation_progress(&self) -> Result<Option<RotationProgress>, RotationError> {
        let Some(rotation) = self.store.saved_rotation().await? else {
            return Ok(None);
        };
        let (retained, acknowledged, stage) = if let Some(signed) = &rotation.completion {
            let complete = verify_completion(&rotation, self.anchor.root, signed)?;
            (
                complete.retained.clone(),
                complete.retained,
                RotationStage::Complete,
            )
        } else {
            let acks = self.store.rotation_acks(&rotation).await?;
            let retained = self.retained_members();
            let mut members = acks
                .iter()
                .map(|ack| verify_ack(ack, &rotation))
                .collect::<Result<Vec<_>, _>>()?;
            members.retain(|member| retained.contains(member));
            (retained, members, RotationStage::Distributing)
        };
        Ok(Some(RotationProgress {
            id: rotation.proposal.id,
            generation: rotation.proposal.generation,
            retained: keys(&retained)?,
            acknowledged: keys(&acknowledged)?,
            stage,
        }))
    }

    pub async fn begin_admission_rotation(
        &mut self,
        session: &mut ManagementSession,
    ) -> Result<RotationProgress, RotationError> {
        self.genesis_author(session)?;
        self.store.apply_ready().await?;
        self.refresh_authorization();
        if !self.is_member(self.node_key.verifying_key()) || self.secret.is_none() {
            return Err(RotationError::Rejected);
        }
        if let Some(existing) = self.store.saved_rotation().await? {
            if existing.completion.is_none() {
                if existing.proposal.initiator != *self.node_key.verifying_key().as_bytes() {
                    return Err(RotationError::Rejected);
                }
                let secret = staged_material(&self.directory, &existing)?;
                self.gate.stage_rotation_secret(
                    AdmissionScope::new(existing.proposal.network, existing.proposal.generation),
                    secret,
                );
                let ack = self.durable_rotation_ack(&existing)?;
                self.store.save_rotation_ack(&existing, &ack).await?;
                return self
                    .rotation_progress()
                    .await?
                    .ok_or(RotationError::Rejected);
            }
        }
        let generation = self
            .scope
            .secret_generation()
            .checked_add(1)
            .filter(|generation| *generation <= i64::MAX as u64)
            .ok_or(RotationError::Rejected)?;
        let scope = AdmissionScope::new(*self.anchor.root.as_bytes(), generation);
        // A crash may leave private staged material before publishing its
        // signed proposal. Reuse that exact fsynced secret for the still-unused
        // next generation; never overwrite a different pending proposal.
        let orphan = self.directory.join(material_name(generation));
        let secret = if orphan.exists() {
            let bytes = fs::read(orphan).map_err(NodeError::from)?;
            AdmissionSecret::from_saved(bytes.try_into().map_err(|_| RotationError::Rejected)?)
        } else {
            AdmissionSecret::generate()
        };
        let proposal = Proposal {
            network: *self.anchor.root.as_bytes(),
            genesis: *self.anchor.genesis.as_bytes(),
            id: rand::random(),
            initiator: *self.node_key.verifying_key().as_bytes(),
            previous_generation: self.scope.secret_generation(),
            generation,
            retained: self.retained_members(),
            fingerprint: secret.fingerprint(&scope),
        };
        let signed = Signed::sign(PROPOSAL_DOMAIN, &proposal, self.genesis_author(session)?)?;
        save_staged(&self.directory, &proposal, &secret)?;
        #[cfg(feature = "replica-test-hooks")]
        self.store
            .observe(crate::management_store::RecordCheckpoint::RotationInitialMaterialSaved);
        self.store.save_rotation(&signed).await?;
        #[cfg(feature = "replica-test-hooks")]
        self.store
            .observe(crate::management_store::RecordCheckpoint::RotationProposalSaved);
        self.gate.stage_rotation_secret(scope, secret);
        let rotation = self
            .store
            .saved_rotation()
            .await?
            .ok_or(RotationError::Rejected)?;
        let ack = self.durable_rotation_ack(&rotation)?;
        self.store.save_rotation_ack(&rotation, &ack).await?;
        self.rotation_progress()
            .await?
            .ok_or(RotationError::Rejected)
    }

    /// Removal commits and affects connections independently of the ensuing
    /// rotation. A missing acknowledgement never implicitly invokes this action.
    pub async fn remove_member_and_rotate(
        &mut self,
        session: &mut ManagementSession,
        target: VerifyingKey,
    ) -> Result<RotationProgress, RotationError> {
        self.genesis_author(session)?;
        self.manage(session, ManagementAction::RemoveMember(target))
            .await?;
        self.begin_admission_rotation(session).await
    }

    fn durable_rotation_ack(&self, rotation: &SavedRotation) -> Result<Signed, RotationError> {
        staged_material(&self.directory, rotation)?;
        Signed::sign(
            ACK_DOMAIN,
            &Acknowledgement {
                proposal: *rotation.signed.hash()?.as_bytes(),
                member: *self.node_key.verifying_key().as_bytes(),
            },
            &self.node_key,
        )
    }

    fn rotation_connection(
        &self,
        connection: &(impl AuthenticatedControl + ?Sized),
    ) -> Result<(), RotationError> {
        let allowed = if connection.alpn() == ADMISSION_ROTATION_ALPN {
            self.gate.allows(connection, ProtocolScope::Rotation)
        } else if connection.alpn() == ROTATION_DELIVERY_ALPN {
            self.gate
                .allows(connection, ProtocolScope::AdmissionRequest)
        } else {
            false
        };
        if !allowed {
            return Err(RotationError::Rejected);
        }
        Ok(())
    }

    /// Ordinary approval and root recovery remain distinct evidence. Only the
    /// original node's durable, still-valid pending result enables this narrow
    /// exception; a TLS identity or a caller's claimed membership cannot.
    async fn pending_rotation_profile(&self, peer: VerifyingKey) -> Result<u8, RotationError> {
        let grants: Vec<Vec<u8>> = sqlx::query_scalar("SELECT authorization FROM admission_deliveries WHERE applicant=? AND authorization IS NOT NULL AND phase<3")
            .bind(peer.as_bytes().as_slice()).fetch_all(&self.store.pool).await.map_err(StoreError::from)?;
        for grant in grants {
            let id = Hash::from_bytes(grant.try_into().map_err(|_| RotationError::Rejected)?);
            if self.store.membership_authorization_valid(id, peer) {
                return Ok(1);
            }
        }
        let grants: Vec<Vec<u8>> = sqlx::query_scalar("SELECT authorization FROM management_recoveries WHERE target=? AND authorization IS NOT NULL AND phase<3")
            .bind(peer.as_bytes().as_slice()).fetch_all(&self.store.pool).await.map_err(StoreError::from)?;
        for grant in grants {
            let id = Hash::from_bytes(grant.try_into().map_err(|_| RotationError::Rejected)?);
            if self.store.administrator_authorization_valid(id, peer) {
                return Ok(2);
            }
        }
        Err(RotationError::Rejected)
    }

    async fn activate_rotation(&mut self, rotation: &SavedRotation) -> Result<(), RotationError> {
        if rotation.completion.is_none() {
            return Err(RotationError::Rejected);
        }
        if self.scope.secret_generation() > rotation.proposal.generation {
            return Ok(());
        }
        let secret = staged_material(&self.directory, rotation)?;
        publish_current_material(&self.directory, rotation, &secret)?;
        #[cfg(feature = "replica-test-hooks")]
        self.store
            .observe(crate::management_store::RecordCheckpoint::RotationMaterialPublished);
        let scope = AdmissionScope::new(rotation.proposal.network, rotation.proposal.generation);
        self.store.set_current_admission(&scope, &secret).await?;
        #[cfg(feature = "replica-test-hooks")]
        self.store
            .observe(crate::management_store::RecordCheckpoint::RotationCredentialPinned);
        self.gate.rotate_secret(scope.clone(), secret.clone());
        self.scope = scope;
        self.secret = Some(secret);
        self.refresh_authorization();
        Ok(())
    }

    pub async fn finish_admission_rotation(
        &mut self,
        session: &mut ManagementSession,
    ) -> Result<RotationProgress, RotationError> {
        self.genesis_author(session)?;
        self.store.apply_ready().await?;
        self.refresh_authorization();
        let rotation = self
            .store
            .saved_rotation()
            .await?
            .ok_or(RotationError::Rejected)?;
        if rotation.proposal.initiator != *self.node_key.verifying_key().as_bytes()
            || !self.is_member(self.node_key.verifying_key())
        {
            return Err(RotationError::Rejected);
        }
        if rotation.completion.is_none() {
            let mut acks = self.store.rotation_acks(&rotation).await?;
            // Recover any newly committed records after loading durable ACKs,
            // then take the actual retained view directly before signing.
            self.store.apply_ready().await?;
            self.refresh_authorization();
            if !self.is_member(self.node_key.verifying_key()) {
                return Err(RotationError::Rejected);
            }
            let retained = self.retained_members();
            if retained
                .iter()
                .any(|member| !rotation.proposal.retained.contains(member))
            {
                return Err(RotationError::RetainedChanged);
            }
            acks.retain(|ack| {
                verify_ack(ack, &rotation).is_ok_and(|member| retained.contains(&member))
            });
            if acks.len() != retained.len() {
                return Err(RotationError::Waiting);
            }
            let completion = Completion {
                proposal: *rotation.signed.hash()?.as_bytes(),
                retained,
                acknowledgements: acks,
            };
            // No async gap between the final actual member snapshot/session
            // check and root signature; durable commit precedes active use.
            let signed = Signed::sign(COMMIT_DOMAIN, &completion, self.genesis_author(session)?)?;
            self.store
                .save_rotation_completion(&rotation, &signed)
                .await?;
            #[cfg(feature = "replica-test-hooks")]
            self.store
                .observe(crate::management_store::RecordCheckpoint::RotationCommitSaved);
        }
        let saved = self
            .store
            .saved_rotation()
            .await?
            .ok_or(RotationError::Rejected)?;
        self.activate_rotation(&saved).await?;
        self.rotation_progress()
            .await?
            .ok_or(RotationError::Rejected)
    }

    /// Send replacement material (old active proof) or an already durable
    /// signed completion (fresh new-generation proof). Retrying never counts
    /// an acknowledgement before its own SQLite commit.
    pub async fn distribute_admission_rotation(
        &mut self,
        connection: &(impl AuthenticatedControl + ?Sized),
    ) -> Result<RotationProgress, RotationError> {
        let result = tokio::time::timeout(TIMEOUT, self.distribute_rotation_inner(connection))
            .await
            .unwrap_or(Err(RotationError::PathUnavailable));
        if result.is_err() {
            connection.close(0u32.into(), b"rotation delivery ended");
        }
        result
    }

    async fn distribute_rotation_inner(
        &mut self,
        connection: &(impl AuthenticatedControl + ?Sized),
    ) -> Result<RotationProgress, RotationError> {
        self.rotation_connection(connection)?;
        self.store.apply_ready().await?;
        self.refresh_authorization();
        self.rotation_connection(connection)?;
        let rotation = self
            .store
            .saved_rotation()
            .await?
            .ok_or(RotationError::Rejected)?;
        if !rotation
            .proposal
            .retained
            .contains(self.node_key.verifying_key().as_bytes())
            || !self.is_member(self.node_key.verifying_key())
            || !self
                .retained_members()
                .contains(connection.remote_id().as_bytes())
        {
            return Err(RotationError::Rejected);
        }
        if !rotation
            .proposal
            .retained
            .contains(connection.remote_id().as_bytes())
        {
            return Err(RotationError::Rejected);
        }
        let exchange = connection
            .open_control_exchange()
            .await
            .map_err(|_| RotationError::PathUnavailable)?;
        let mut _root_session = None;
        if connection.alpn() == ROTATION_DELIVERY_ALPN {
            if rotation.completion.is_some() {
                return Err(RotationError::Rejected);
            }
            let peer = VerifyingKey::from_bytes(connection.remote_id().as_bytes())
                .map_err(|_| RotationError::Rejected)?;
            let profile = self.pending_rotation_profile(peer).await?;
            write_fixed(&exchange, &[profile]).await?;
            if profile == 2 {
                _root_session = Some(
                    crate::genesis_recovery::SessionGuard::begin(connection)
                        .map_err(|_| RotationError::Rejected)?,
                );
                let local = iroh_base::SecretKey::from_bytes(self.node_key.as_bytes()).public();
                let statement = crate::genesis_recovery::proof_statement(
                    connection,
                    self.anchor,
                    connection.remote_id(),
                    local,
                )
                .map_err(|_| RotationError::Rejected)?;
                let mut signature = [0; 64];
                read_fixed(&exchange, &mut signature).await?;
                let root_valid = self
                    .anchor
                    .root
                    .verify(&statement, &Signature::from_bytes(&signature));
                if !root_valid {
                    return Err(RotationError::Rejected);
                }
            }
        }
        let records = self.store.applied_records().await?;
        if records.len() > MAX_MEMBERS {
            return Err(RotationError::Rejected);
        }
        let history: Vec<_> = records
            .into_iter()
            .map(|r| (r.header().to_vec(), r.body().map(<[u8]>::to_vec)))
            .collect();
        write_frame(
            &exchange,
            &encode(&(
                rotation.signed.clone(),
                rotation.completion.clone(),
                history,
                self.store
                    .predecessor_completion(rotation.proposal.previous_generation)
                    .await?,
            ))?,
        )
        .await?;
        if rotation.completion.is_none() {
            self.rotation_connection(connection)?;
            let secret = staged_material(&self.directory, &rotation)?;
            write_fixed(&exchange, secret.saved_bytes()).await?;
            let (ack, forwarded): (Signed, Vec<Signed>) = decode(&read_frame(&exchange).await?)?;
            #[cfg(feature = "replica-test-hooks")]
            self.store.observe(
                crate::management_store::RecordCheckpoint::RotationAcknowledgementReceived,
            );
            let verified_ack = verify_ack(&ack, &rotation)?;
            if verified_ack != *connection.remote_id().as_bytes() {
                return Err(RotationError::Rejected);
            }
            if forwarded.len() > MAX_MEMBERS {
                return Err(RotationError::Rejected);
            }
            let retained = self.retained_members();
            for relayed in &forwarded {
                if !retained.contains(&verify_ack(relayed, &rotation)?) {
                    return Err(RotationError::Rejected);
                }
            }
            self.rotation_connection(connection)?;
            self.store.save_rotation_ack(&rotation, &ack).await?;
            // A retained original provider may have delivered to a pending
            // recipient. Its genuine exact-proposal node signature can travel
            // back on the provider's proved session, but is never counted from
            // a claimed identity or before this node commits the ACK itself.
            for relayed in &forwarded {
                self.store.save_rotation_ack(&rotation, relayed).await?;
            }
            #[cfg(feature = "replica-test-hooks")]
            self.store
                .observe(crate::management_store::RecordCheckpoint::RotationAcknowledgementSaved);
        } else {
            let mut ack = [0; 4];
            read_fixed(&exchange, &mut ack).await?;
            if &ack != b"DONE" {
                return Err(RotationError::Rejected);
            }
        }
        write_fixed(&exchange, b"SAVE").await?;
        exchange
            .finish_send()
            .await
            .map_err(|_| RotationError::PathUnavailable)?;
        let mut received = [0; 4];
        read_fixed(&exchange, &mut received).await?;
        if &received != b"RCVD" {
            return Err(RotationError::Rejected);
        }
        self.rotation_progress()
            .await?
            .ok_or(RotationError::Rejected)
    }

    pub async fn receive_admission_rotation(
        &mut self,
        connection: &(impl AuthenticatedControl + ?Sized),
    ) -> Result<RotationProgress, RotationError> {
        self.receive_rotation_exchange(connection, ReceiptMode::Normal)
            .await
    }

    #[cfg(feature = "replica-test-hooks")]
    pub async fn receive_admission_rotation_fixture(
        &mut self,
        connection: &(impl AuthenticatedControl + ?Sized),
        fault: RotationAckFault,
    ) -> Result<RotationProgress, RotationError> {
        self.receive_rotation_exchange(connection, ReceiptMode::Fault(fault))
            .await
    }

    async fn receive_rotation_exchange(
        &mut self,
        connection: &(impl AuthenticatedControl + ?Sized),
        mode: ReceiptMode,
    ) -> Result<RotationProgress, RotationError> {
        let result = tokio::time::timeout(TIMEOUT, self.receive_rotation_inner(connection, mode))
            .await
            .unwrap_or(Err(RotationError::PathUnavailable));
        if result.is_err() {
            connection.close(0u32.into(), b"rotation receipt ended");
        }
        result
    }

    async fn receive_rotation_inner(
        &mut self,
        connection: &(impl AuthenticatedControl + ?Sized),
        mode: ReceiptMode,
    ) -> Result<RotationProgress, RotationError> {
        self.rotation_connection(connection)?;
        let exchange = connection
            .accept_control_exchange()
            .await
            .map_err(|_| RotationError::PathUnavailable)?;
        let mut _root_session = None;
        if connection.alpn() == ROTATION_DELIVERY_ALPN {
            let mut profile = [0];
            read_fixed(&exchange, &mut profile).await?;
            match profile[0] {
                1 if self.join.is_some_and(|binding| {
                    binding.invitation.contact == connection.remote_id()
                }) => {}
                2 if self
                    .recovery
                    .is_some_and(|binding| binding.contact == connection.remote_id()) =>
                {
                    _root_session = Some(
                        crate::genesis_recovery::SessionGuard::begin(connection)
                            .map_err(|_| RotationError::Rejected)?,
                    );
                    let credential = self.saved_genesis_credential()?;
                    let local = iroh_base::SecretKey::from_bytes(self.node_key.as_bytes()).public();
                    let statement = crate::genesis_recovery::proof_statement(
                        connection,
                        self.anchor,
                        local,
                        connection.remote_id(),
                    )
                    .map_err(|_| RotationError::Rejected)?;
                    write_fixed(&exchange, &credential.root.sign(&statement).to_bytes()).await?;
                }
                _ => return Err(RotationError::Rejected),
            }
        }
        type WireHistory = Vec<(Vec<u8>, Option<Vec<u8>>)>;
        let (signed, completion, history, predecessor): (
            Signed,
            Option<Signed>,
            WireHistory,
            Option<(Signed, Signed)>,
        ) = decode(&read_frame(&exchange).await?)?;
        let proposal = verify_proposal(&self.store, &signed)?;
        if !proposal
            .retained
            .contains(connection.remote_id().as_bytes())
            || !proposal
                .retained
                .contains(self.node_key.verifying_key().as_bytes())
            || history.len() > MAX_MEMBERS
        {
            return Err(RotationError::Rejected);
        }
        for (header, body) in history {
            self.store
                .ingest(&ManagementRecord::new(header, body.as_deref()))
                .await?;
        }
        self.store.apply_ready().await?;
        self.refresh_authorization();
        self.rotation_connection(connection)?;
        if !self.is_member(self.node_key.verifying_key())
            || !self.is_member(
                VerifyingKey::from_bytes(connection.remote_id().as_bytes())
                    .map_err(|_| RotationError::Rejected)?,
            )
        {
            return Err(RotationError::Rejected);
        }
        // An acknowledged predecessor can still be staged at this recipient
        // after the initiator has begun the next round. Apply only its exact
        // durable root decision, never infer completion from a later proposal.
        if self.scope.secret_generation() < proposal.previous_generation
            && self.store.saved_rotation().await?.is_some_and(|old| {
                old.completion.is_none() && old.proposal.generation == proposal.previous_generation
            })
        {
            let previous = self
                .store
                .saved_rotation()
                .await?
                .ok_or(RotationError::Rejected)?;
            let (previous_signed, previous_completion) =
                predecessor.ok_or(RotationError::Rejected)?;
            if previous.proposal.generation != proposal.previous_generation
                || encode(&previous.signed)? != encode(&previous_signed)?
            {
                return Err(RotationError::Rejected);
            }
            verify_completion(&previous, self.anchor.root, &previous_completion)?;
            self.store
                .save_rotation_completion(&previous, &previous_completion)
                .await?;
            let completed = self
                .store
                .saved_rotation()
                .await?
                .ok_or(RotationError::Rejected)?;
            self.activate_rotation(&completed).await?;
            self.rotation_connection(connection)?;
        }
        let existing = self.store.saved_rotation().await?;
        if completion.is_none() {
            if connection.alpn() == ADMISSION_ROTATION_ALPN
                && proposal.previous_generation != self.scope.secret_generation()
                || connection.alpn() == ROTATION_DELIVERY_ALPN
                    && proposal.generation <= self.scope.secret_generation()
            {
                return Err(RotationError::Rejected);
            }
            if existing.as_ref().is_some_and(|old| {
                old.proposal.generation >= proposal.generation
                    && encode(&old.signed).ok() != encode(&signed).ok()
            }) {
                return Err(RotationError::Rejected);
            }
            // Persist the non-secret, root-authorized pause before waiting for
            // replacement bytes. A crash before secret commit reopens paused
            // with no acknowledgement, rather than resuming admission early.
            self.store.save_rotation(&signed).await?;
            let mut bytes = [0; 32];
            read_fixed(&exchange, &mut bytes).await?;
            #[cfg(feature = "replica-test-hooks")]
            self.store
                .observe(crate::management_store::RecordCheckpoint::RotationSecretReceived);
            let secret = AdmissionSecret::from_saved(bytes);
            let scope = AdmissionScope::new(proposal.network, proposal.generation);
            if secret.fingerprint(&scope) != proposal.fingerprint {
                return Err(RotationError::Rejected);
            }
            save_staged(&self.directory, &proposal, &secret)?;
            self.store.save_rotation(&signed).await?;
            self.gate.stage_rotation_secret(scope, secret);
            #[cfg(feature = "replica-test-hooks")]
            self.store
                .observe(crate::management_store::RecordCheckpoint::RotationSecretSaved);
            let rotation = self
                .store
                .saved_rotation()
                .await?
                .ok_or(RotationError::Rejected)?;
            let ack = self.durable_rotation_ack(&rotation)?;
            self.store.save_rotation_ack(&rotation, &ack).await?;
            #[cfg(feature = "replica-test-hooks")]
            let mut tamper_forwarded = false;
            let wire_ack = match mode {
                ReceiptMode::Normal => ack,
                #[cfg(feature = "replica-test-hooks")]
                ReceiptMode::Fault(RotationAckFault::ForgedForwardedSignature) => {
                    tamper_forwarded = true;
                    ack
                }
                #[cfg(feature = "replica-test-hooks")]
                ReceiptMode::Fault(RotationAckFault::ForgedSignature) => {
                    let mut forged = ack;
                    forged.signature[0] ^= 1;
                    forged
                }
                #[cfg(feature = "replica-test-hooks")]
                ReceiptMode::Fault(RotationAckFault::WrongIdentity(key)) => Signed::sign(
                    ACK_DOMAIN,
                    &Acknowledgement {
                        proposal: *rotation.signed.hash()?.as_bytes(),
                        member: *key.verifying_key().as_bytes(),
                    },
                    &key,
                )?,
                #[cfg(feature = "replica-test-hooks")]
                ReceiptMode::Fault(RotationAckFault::PreviousRotation) => {
                    let previous: Vec<u8> = sqlx::query_scalar("SELECT proposal FROM admission_rotations WHERE generation<? ORDER BY generation DESC LIMIT 1")
                        .bind(rotation.proposal.generation as i64).fetch_one(&self.store.pool).await.map_err(StoreError::from)?;
                    let signed: Signed = decode(&previous)?;
                    let proposal = verify_proposal(&self.store, &signed)?;
                    let saved: Vec<u8> = sqlx::query_scalar("SELECT acknowledgement FROM admission_rotation_acks WHERE rotation=? AND member=?")
                        .bind(proposal.id.as_slice()).bind(self.node_key.verifying_key().as_bytes().as_slice())
                        .fetch_one(&self.store.pool).await.map_err(StoreError::from)?;
                    let stale: Signed = decode(&saved)?;
                    verify_ack(
                        &stale,
                        &SavedRotation {
                            signed,
                            proposal,
                            completion: None,
                        },
                    )?;
                    stale
                }
            };
            let retained = self.retained_members();
            let forwarded = self
                .store
                .rotation_acks(&rotation)
                .await?
                .into_iter()
                .filter(|ack| {
                    verify_ack(ack, &rotation).is_ok_and(|member| retained.contains(&member))
                })
                .collect::<Vec<_>>();
            #[cfg(feature = "replica-test-hooks")]
            let forwarded = if tamper_forwarded {
                let mut mutated = forwarded;
                let other = mutated
                    .iter_mut()
                    .find(|ack| {
                        verify_ack(ack, &rotation)
                            .is_ok_and(|member| member != *self.node_key.verifying_key().as_bytes())
                    })
                    .ok_or(RotationError::Rejected)?;
                other.signature[0] ^= 1;
                mutated
            } else {
                forwarded
            };
            write_frame(&exchange, &encode(&(wire_ack, forwarded))?).await?;
            #[cfg(feature = "replica-test-hooks")]
            self.store
                .observe(crate::management_store::RecordCheckpoint::RotationAcknowledgementSent);
        } else {
            let rotation = existing.ok_or(RotationError::Rejected)?;
            if encode(&rotation.signed)? != encode(&signed)? {
                return Err(RotationError::Rejected);
            }
            let completion = completion.unwrap();
            let final_state = verify_completion(&rotation, self.anchor.root, &completion)?;
            if rotation.completion.is_none() && final_state.retained != self.retained_members() {
                return Err(RotationError::RetainedChanged);
            }
            self.store
                .save_rotation_completion(&rotation, &completion)
                .await?;
            #[cfg(feature = "replica-test-hooks")]
            self.store
                .observe(crate::management_store::RecordCheckpoint::RotationCommitSaved);
            let saved = self
                .store
                .saved_rotation()
                .await?
                .ok_or(RotationError::Rejected)?;
            self.activate_rotation(&saved).await?;
            write_fixed(&exchange, b"DONE").await?;
        }
        let mut saved = [0; 4];
        read_fixed(&exchange, &mut saved).await?;
        if &saved != b"SAVE" {
            return Err(RotationError::Rejected);
        }
        write_fixed(&exchange, b"RCVD").await?;
        exchange
            .finish_send()
            .await
            .map_err(|_| RotationError::PathUnavailable)?;
        if _root_session.is_some() {
            let _ = connection.revocation_handle().closed().await;
        }
        self.rotation_progress()
            .await?
            .ok_or(RotationError::Rejected)
    }
}

async fn write_fixed(exchange: &ControlExchange, bytes: &[u8]) -> Result<(), RotationError> {
    exchange
        .send_message(
            ControlMessageFormat::Fixed {
                length: bytes.len(),
            },
            bytes,
        )
        .await
        .map_err(|_| RotationError::PathUnavailable)
}
async fn read_fixed<const N: usize>(
    exchange: &ControlExchange,
    bytes: &mut [u8; N],
) -> Result<(), RotationError> {
    let message = exchange
        .recv_message(ControlMessageFormat::Fixed { length: N })
        .await
        .map_err(|_| RotationError::PathUnavailable)?;
    bytes.copy_from_slice(&message);
    Ok(())
}
async fn write_frame(exchange: &ControlExchange, bytes: &[u8]) -> Result<(), RotationError> {
    if bytes.len() > MAX_FRAME {
        return Err(RotationError::Rejected);
    }
    exchange
        .send_message(
            ControlMessageFormat::LengthPrefixed32 { maximum: MAX_FRAME },
            bytes,
        )
        .await
        .map_err(|_| RotationError::PathUnavailable)
}
async fn read_frame(exchange: &ControlExchange) -> Result<Vec<u8>, RotationError> {
    exchange
        .recv_message(ControlMessageFormat::LengthPrefixed32 { maximum: MAX_FRAME })
        .await
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::InvalidData {
                RotationError::Rejected
            } else {
                RotationError::PathUnavailable
            }
        })
}
