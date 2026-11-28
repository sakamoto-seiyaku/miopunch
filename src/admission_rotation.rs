//! Durable admission-secret rotation. Signed control statements contain only
//! identities, generations and credential tags; secret bytes stay in private
//! files and admitted TLS streams. Staged credentials never grant business.
use std::collections::BTreeMap;
use std::fs;
use std::ops::{Deref, DerefMut};
use std::path::Path;

use crate::control_protocol::AuthenticatedControl;
#[cfg(feature = "replica-test-hooks")]
use crate::admission_rotation_wire::{ACK_DOMAIN, Acknowledgement};
use p2panda_core::{Hash, Signature, SigningKey, VerifyingKey};
use sqlx::Row;
use thiserror::Error;
use tokio::sync::{Mutex, MutexGuard};
use tokio::time::{Instant, timeout_at};

use crate::admission_gate::ProtocolScope;
pub use crate::admission_rotation_wire::{
    ADMISSION_ROTATION_ALPN, ROTATION_DELIVERY_ALPN, RotationProgress, RotationStage,
};
use crate::admission_rotation_wire::{
    COMMIT_DOMAIN, Completion, HistoryFrame, MAX_MEMBERS, PROPOSAL_DOMAIN,
    Proposal, RotationExchange, SavedRotation, Signed, WireError, decode, encode, retained_members,
    rotation_deadline, verify_ack, verify_completion, verify_proposal, wire_io,
};
use crate::admission_proof::{AdmissionScope, AdmissionSecret};
use crate::daily_management::{ManagementAction, ManagementError, ManagementSession};
use crate::management_store::{ManagementRecord, ManagementStore, StoreError};
use crate::native_network::{NativeNetwork, NodeError, write_private};

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

impl From<WireError> for RotationError {
    fn from(value: WireError) -> Self {
        match value {
            WireError::Rejected => Self::Rejected,
            WireError::PathUnavailable => Self::PathUnavailable,
        }
    }
}

// The low-level API already has exclusive access to the node. The SDK owns a
// shared node and borrows it only for each authorization or storage phase.
enum RotationNode<'a> {
    Exclusive(&'a mut NativeNetwork),
    Shared(&'a Mutex<NativeNetwork>),
}

enum RotationAccess<'a> {
    Exclusive(&'a mut NativeNetwork),
    Shared(MutexGuard<'a, NativeNetwork>),
}

impl RotationNode<'_> {
    async fn access(&mut self, deadline: Instant) -> Result<RotationAccess<'_>, RotationError> {
        match self {
            Self::Exclusive(node) => Ok(RotationAccess::Exclusive(node)),
            Self::Shared(node) => timeout_at(deadline, node.lock())
                .await
                .map(RotationAccess::Shared)
                .map_err(|_| RotationError::PathUnavailable),
        }
    }
}

impl Deref for RotationAccess<'_> {
    type Target = NativeNetwork;

    fn deref(&self) -> &NativeNetwork {
        match self {
            Self::Exclusive(node) => node,
            Self::Shared(node) => node,
        }
    }
}

impl DerefMut for RotationAccess<'_> {
    fn deref_mut(&mut self) -> &mut NativeNetwork {
        match self {
            Self::Exclusive(node) => node,
            Self::Shared(node) => node,
        }
    }
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
        let proposal = verify_proposal(self.anchor(), &signed)?;
        let generation: i64 = row.try_get("generation").map_err(StoreError::from)?;
        if generation < 0 || generation as u64 != proposal.generation {
            return Err(RotationError::Rejected);
        }
        let result = SavedRotation::new(signed, proposal, completion.as_deref().map(decode).transpose()?)?;
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
        let proposal = verify_proposal(self.anchor(), &signed)?;
        if proposal.generation != generation {
            return Err(RotationError::Rejected);
        }
        let completed: Signed = decode(&completion)?;
        let rotation = SavedRotation::new(signed.clone(), proposal, Some(completed.clone()))?;
        verify_completion(&rotation, self.anchor().root, &completed)?;
        Ok(Some((signed, completed)))
    }

    async fn save_rotation(&self, signed: &Signed) -> Result<(), RotationError> {
        let proposal = verify_proposal(self.anchor(), signed)?;
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
        self.changes.publish();
        Ok(())
    }

    async fn rotation_acks(&self, rotation: &SavedRotation) -> Result<Vec<([u8; 32], Signed)>, RotationError> {
        let bytes: Vec<Vec<u8>> = sqlx::query_scalar(
            "SELECT acknowledgement FROM admission_rotation_acks WHERE rotation=? ORDER BY member",
        )
        .bind(rotation.proposal.id.as_slice())
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::from)?;
        bytes.iter().map(|bytes| {
            let ack = decode(bytes)?;
            Ok((verify_ack(&ack, rotation)?, ack))
        }).collect()
    }

    async fn save_rotation_ack(
        &self,
        rotation: &SavedRotation,
        signed: &Signed,
    ) -> Result<(), RotationError> {
        let member = verify_ack(signed, rotation)?;
        self.save_verified_rotation_ack(rotation, &member, signed).await
    }

    async fn save_verified_rotation_ack(
        &self,
        rotation: &SavedRotation,
        member: &[u8; 32],
        signed: &Signed,
    ) -> Result<(), RotationError> {
        let bytes = encode(signed)?;
        // SQLite FULL commits before the caller can count this acknowledgement.
        let inserted =
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
        if inserted.rows_affected() != 0 {
            self.changes.publish();
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
        retained_members(self.store.authorization(), self.anchor.root)
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
                    if rotation.proposal.initiator == *self.node_key.verifying_key().as_bytes() {
                        let ack = self.durable_rotation_ack(&rotation)?;
                        self.store.save_rotation_ack(&rotation, &ack).await?;
                    }
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
        Ok(Some(self.progress_of(&rotation).await?))
    }

    async fn progress_of(
        &self,
        rotation: &SavedRotation,
    ) -> Result<RotationProgress, RotationError> {
        if let Some(progress) = rotation.completed_progress(self.anchor.root)? {
            return Ok(progress);
        }
        let acknowledged = self.store.rotation_acks(rotation).await?;
        Ok(rotation.progress(&self.retained_members(), acknowledged.into_iter().map(|(member, _)| member))?)
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
        Ok(rotation.sign_ack(&self.node_key)?)
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

    /// Resume an already signed switch, and select recipients for the current
    /// proposal. A non-initiator serves only its own pending material grants.
    pub(crate) async fn rotation_delivery_plan(
        &mut self,
    ) -> Result<Option<(RotationProgress, BTreeMap<iroh::EndpointId, &'static [u8]>)>, RotationError>
    {
        let Some(rotation) = self.store.saved_rotation().await? else {
            return Ok(None);
        };
        let local = self.node_key.verifying_key();
        if !self.is_member(local) || !rotation.proposal.retained.contains(local.as_bytes()) {
            return Ok(None);
        }
        if rotation.completion.is_some() {
            self.activate_rotation(&rotation).await?;
            if self.scope.secret_generation() != rotation.proposal.generation {
                return Ok(None);
            }
        }
        let progress = self.progress_of(&rotation).await?;
        let mut pending_peers = std::collections::BTreeSet::new();
        if progress.forwarding_pending(local) {
            let pending: Vec<Vec<u8>> = sqlx::query_scalar(
                "SELECT applicant FROM admission_deliveries WHERE authorization IS NOT NULL AND phase<3 \
                 UNION SELECT target FROM management_recoveries WHERE authorization IS NOT NULL AND phase<3",
            )
            .fetch_all(&self.store.pool)
            .await
            .map_err(StoreError::from)?;
            for bytes in pending {
                let peer = VerifyingKey::from_bytes(
                    &bytes.try_into().map_err(|_| RotationError::Rejected)?,
                )
                .map_err(|_| RotationError::Rejected)?;
                if peer != local && progress.retained.contains(&peer)
                    && rotation.proposal.retained.binary_search(peer.as_bytes()).is_ok()
                    && self.pending_rotation_profile(peer).await.is_ok()
                {
                    pending_peers.insert(peer);
                }
            }
        }
        let recipients = progress.recipients(&rotation, local, |peer| self.is_member(peer), |peer| pending_peers.contains(&peer))?;
        Ok(Some((progress, recipients)))
    }

    async fn activate_rotation(&mut self, rotation: &SavedRotation) -> Result<(), RotationError> {
        if rotation.completion.is_none() {
            return Err(RotationError::Rejected);
        }
        // Rejoining can intentionally suspend material in the same generation.
        // A historical completion cannot restore that credential.
        if self.scope.secret_generation() >= rotation.proposal.generation {
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
        self.store.changes.publish();
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
            acks.retain(|(member, _)| retained.contains(member));
            if acks.len() != retained.len() {
                return Err(RotationError::Waiting);
            }
            let completion = Completion {
                proposal: rotation.proposal_hash,
                retained,
                acknowledgements: acks.into_iter().map(|(_, ack)| ack).collect(),
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
        Self::distribute_rotation_inner(&mut RotationNode::Exclusive(self), connection, None).await
    }

    pub(crate) async fn deliver_current_rotation(
        node: &Mutex<Self>,
        connection: &(impl AuthenticatedControl + ?Sized),
        id: [u8; 16],
        stage: RotationStage,
    ) -> Result<RotationProgress, RotationError> {
        Self::distribute_rotation_inner(
            &mut RotationNode::Shared(node),
            connection,
            Some((id, stage)),
        )
        .await
    }

    async fn distribute_rotation_inner(
        node: &mut RotationNode<'_>,
        connection: &(impl AuthenticatedControl + ?Sized),
        expected: Option<([u8; 16], RotationStage)>,
    ) -> Result<RotationProgress, RotationError> {
        let result = Self::send_rotation(node, connection, expected).await;
        if result.is_err() {
            connection.close(0u32.into(), b"rotation delivery ended");
        }
        result
    }

    async fn send_rotation(
        node: &mut RotationNode<'_>,
        connection: &(impl AuthenticatedControl + ?Sized),
        expected: Option<([u8; 16], RotationStage)>,
    ) -> Result<RotationProgress, RotationError> {
        let deadline = rotation_deadline(connection);
        let peer = VerifyingKey::from_bytes(connection.remote_id().as_bytes())
            .map_err(|_| RotationError::Rejected)?;
        let (rotation, profile, anchor, local) = {
            let mut state = node.access(deadline).await?;
            state.rotation_connection(connection)?;
            state.store.apply_ready().await?;
            state.refresh_authorization();
            let rotation = state
                .store
                .saved_rotation()
                .await?
                .ok_or(RotationError::Rejected)?;
            let saved = (rotation.proposal.id, rotation.stage());
            if expected.is_some_and(|expected| expected != saved) {
                return Err(RotationError::Rejected);
            }
            state.check_rotation_recipient(connection, peer, &rotation)?;
            let profile = if connection.alpn() == ROTATION_DELIVERY_ALPN {
                if rotation.completion.is_some() {
                    return Err(RotationError::Rejected);
                }
                Some(state.pending_rotation_profile(peer).await?)
            } else {
                None
            };
            let local = iroh_base::SecretKey::from_bytes(state.node_key.as_bytes()).public();
            (rotation, profile, state.anchor, local)
        };
        let exchange = RotationExchange {
            control: wire_io(deadline, connection.open_control_exchange()).await?,
            deadline,
        };
        let mut _root_session = None;
        if let Some(profile) = profile {
            exchange.write_fixed(&[profile]).await?;
            if profile == 2 {
                _root_session = Some(
                    crate::genesis_recovery::SessionGuard::begin(connection)
                        .map_err(|_| RotationError::Rejected)?,
                );
                let statement = crate::genesis_recovery::proof_statement(
                    connection,
                    anchor,
                    connection.remote_id(),
                    local,
                )
                .map_err(|_| RotationError::Rejected)?;
                let mut signature = [0; 64];
                exchange.read_fixed(&mut signature).await?;
                if !anchor
                    .root
                    .verify(&statement, &Signature::from_bytes(&signature))
                {
                    return Err(RotationError::Rejected);
                }
            }
        }
        let history_frame = {
            let state = node.access(deadline).await?;
            state.check_current_rotation(&rotation).await?;
            state.check_rotation_recipient(connection, peer, &rotation)?;
            if let Some(profile) = profile
                && profile != state.pending_rotation_profile(peer).await?
            {
                return Err(RotationError::Rejected);
            }
            let records = state.store.applied_records().await?;
            if records.len() > MAX_MEMBERS {
                return Err(RotationError::Rejected);
            }
            let history: Vec<_> = records
                .into_iter()
                .map(|r| (r.header().to_vec(), r.body().map(<[u8]>::to_vec)))
                .collect();
            encode(&(
                rotation.signed.clone(),
                rotation.completion.clone(),
                history,
                state
                    .store
                    .predecessor_completion(rotation.proposal.previous_generation)
                    .await?,
            ))?
        };
        exchange.write_frame(&history_frame).await?;
        if rotation.completion.is_none() {
            // The peer may have lost its grant while reading history.
            // Serialize this last check and short private-material release.
            let state = node.access(deadline).await?;
            state.check_current_rotation(&rotation).await?;
            state.check_rotation_recipient(connection, peer, &rotation)?;
            if let Some(profile) = profile
                && profile != state.pending_rotation_profile(peer).await?
            {
                return Err(RotationError::Rejected);
            }
            let secret = staged_material(&state.directory, &rotation)?;
            exchange.write_fixed(secret.saved_bytes()).await?;
        }
        if rotation.completion.is_none() {
            let (ack, forwarded): (Signed, Vec<Signed>) = decode(&exchange.read_frame().await?)?;
            let state = node.access(deadline).await?;
            #[cfg(feature = "replica-test-hooks")]
            state.store.observe(
                crate::management_store::RecordCheckpoint::RotationAcknowledgementReceived,
            );
            state.check_current_rotation(&rotation).await?;
            state.check_rotation_recipient(connection, peer, &rotation)?;
            let receipts = crate::admission_rotation_wire::verify_delivery_acks(
                &rotation, peer, &state.retained_members(), ack, forwarded,
            )?;
            state.rotation_connection(connection)?;
            for (member, signed) in receipts {
                state.store.save_verified_rotation_ack(&rotation, member.as_bytes(), &signed).await?;
            }
            #[cfg(feature = "replica-test-hooks")]
            state
                .store
                .observe(crate::management_store::RecordCheckpoint::RotationAcknowledgementSaved);
        } else {
            let mut ack = [0; 4];
            exchange.read_fixed(&mut ack).await?;
            if &ack != b"DONE" {
                return Err(RotationError::Rejected);
            }
        }
        exchange.write_fixed(b"SAVE").await?;
        exchange.finish_send().await?;
        let mut received = [0; 4];
        exchange.read_fixed(&mut received).await?;
        if &received != b"RCVD" {
            return Err(RotationError::Rejected);
        }
        let state = node.access(deadline).await?;
        state.check_current_rotation(&rotation).await?;
        state.progress_of(&rotation).await
    }

    fn check_rotation_recipient(
        &self,
        connection: &(impl AuthenticatedControl + ?Sized),
        peer: VerifyingKey,
        rotation: &SavedRotation,
    ) -> Result<(), RotationError> {
        self.rotation_connection(connection)?;
        let local = self.node_key.verifying_key();
        if !rotation.proposal.retained.contains(local.as_bytes())
            || !self.is_member(local)
            || !rotation.proposal.retained.contains(peer.as_bytes())
            || !self.is_member(peer)
            || rotation.completion.is_some()
                && self.scope.secret_generation() != rotation.proposal.generation
        {
            return Err(RotationError::Rejected);
        }
        Ok(())
    }

    async fn check_current_rotation(&self, expected: &SavedRotation) -> Result<(), RotationError> {
        let current = self
            .store
            .saved_rotation()
            .await?
            .ok_or(RotationError::Rejected)?;
        if current.proposal.id != expected.proposal.id || current.stage() != expected.stage() {
            return Err(RotationError::Rejected);
        }
        Ok(())
    }

    pub async fn receive_admission_rotation(
        &mut self,
        connection: &(impl AuthenticatedControl + ?Sized),
    ) -> Result<RotationProgress, RotationError> {
        Self::receive_rotation_exchange(
            &mut RotationNode::Exclusive(self),
            connection,
            ReceiptMode::Normal,
        )
        .await
    }

    pub(crate) async fn receive_current_rotation(
        node: &Mutex<Self>,
        connection: &(impl AuthenticatedControl + ?Sized),
    ) -> Result<RotationProgress, RotationError> {
        Self::receive_rotation_exchange(
            &mut RotationNode::Shared(node),
            connection,
            ReceiptMode::Normal,
        )
        .await
    }

    #[cfg(feature = "replica-test-hooks")]
    pub async fn receive_admission_rotation_fixture(
        &mut self,
        connection: &(impl AuthenticatedControl + ?Sized),
        fault: RotationAckFault,
    ) -> Result<RotationProgress, RotationError> {
        Self::receive_rotation_exchange(
            &mut RotationNode::Exclusive(self),
            connection,
            ReceiptMode::Fault(fault),
        )
        .await
    }

    async fn receive_rotation_exchange(
        node: &mut RotationNode<'_>,
        connection: &(impl AuthenticatedControl + ?Sized),
        mode: ReceiptMode,
    ) -> Result<RotationProgress, RotationError> {
        let result = Self::receive_rotation_inner(node, connection, mode).await;
        if result.is_err() {
            connection.close(0u32.into(), b"rotation receipt ended");
        }
        result
    }

    async fn receive_rotation_inner(
        node: &mut RotationNode<'_>,
        connection: &(impl AuthenticatedControl + ?Sized),
        mode: ReceiptMode,
    ) -> Result<RotationProgress, RotationError> {
        let deadline = rotation_deadline(connection);
        node.access(deadline)
            .await?
            .rotation_connection(connection)?;
        let exchange = RotationExchange {
            control: wire_io(deadline, connection.accept_control_exchange()).await?,
            deadline,
        };
        let mut _root_session = None;
        let profile = if connection.alpn() == ROTATION_DELIVERY_ALPN {
            let mut profile = [0];
            exchange.read_fixed(&mut profile).await?;
            let proof = {
                let state = node.access(deadline).await?;
                state.check_original_rotation_provider(profile[0], connection.remote_id())?;
                match profile[0] {
                    2 => {
                        _root_session = Some(
                            crate::genesis_recovery::SessionGuard::begin(connection)
                                .map_err(|_| RotationError::Rejected)?,
                        );
                        let credential = state.saved_genesis_credential()?;
                        let local =
                            iroh_base::SecretKey::from_bytes(state.node_key.as_bytes()).public();
                        let statement = crate::genesis_recovery::proof_statement(
                            connection,
                            state.anchor,
                            local,
                            connection.remote_id(),
                        )
                        .map_err(|_| RotationError::Rejected)?;
                        Some(credential.root.sign(&statement).to_bytes())
                    }
                    _ => None,
                }
            };
            if let Some(proof) = proof {
                exchange.write_fixed(&proof).await?;
            }
            Some(profile[0])
        } else {
            None
        };
        let (signed, completion, history, predecessor): HistoryFrame = decode(&exchange.read_frame().await?)?;
        let peer = VerifyingKey::from_bytes(connection.remote_id().as_bytes())
            .map_err(|_| RotationError::Rejected)?;
        let pending = {
            let mut state = node.access(deadline).await?;
            state.rotation_connection(connection)?;
            if let Some(profile) = profile {
                state.check_original_rotation_provider(profile, connection.remote_id())?;
            }
            let proposal = verify_proposal(state.anchor, &signed)?;
            if !proposal
                .retained
                .contains(connection.remote_id().as_bytes())
                || !proposal
                    .retained
                    .contains(state.node_key.verifying_key().as_bytes())
                || history.len() > MAX_MEMBERS
            {
                return Err(RotationError::Rejected);
            }
            for (header, body) in history {
                state
                    .store
                    .ingest(&ManagementRecord::new(header, body.as_deref()))
                    .await?;
            }
            state.store.apply_ready().await?;
            state.refresh_authorization();
            state.check_rotation_sender(connection, peer)?;
            // An acknowledged predecessor can still be staged at this recipient
            // after the initiator has begun the next round. Apply only its exact
            // durable root decision, never infer completion from a later proposal.
            if state.scope.secret_generation() < proposal.previous_generation
                && state.store.saved_rotation().await?.is_some_and(|old| {
                    old.completion.is_none()
                        && old.proposal.generation == proposal.previous_generation
                })
            {
                let previous = state
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
                state.verify_local_completion(&previous, &previous_completion)?;
                state
                    .store
                    .save_rotation_completion(&previous, &previous_completion)
                    .await?;
                let completed = state
                    .store
                    .saved_rotation()
                    .await?
                    .ok_or(RotationError::Rejected)?;
                state.activate_rotation(&completed).await?;
                state.rotation_connection(connection)?;
            }
            let existing = state.store.saved_rotation().await?;
            if completion.is_none() {
                if connection.alpn() == ADMISSION_ROTATION_ALPN
                    && proposal.previous_generation != state.scope.secret_generation()
                    || connection.alpn() == ROTATION_DELIVERY_ALPN
                        && proposal.generation <= state.scope.secret_generation()
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
                state.store.save_rotation(&signed).await?;
                Some(
                    state
                        .store
                        .saved_rotation()
                        .await?
                        .ok_or(RotationError::Rejected)?,
                )
            } else {
                let rotation = existing.ok_or(RotationError::Rejected)?;
                if encode(&rotation.signed)? != encode(&signed)? {
                    return Err(RotationError::Rejected);
                }
                let completion = completion.unwrap();
                // Membership may have changed after the initiator signed the
                // decision. Verify the original quorum and our inclusion in it.
                state.verify_local_completion(&rotation, &completion)?;
                state
                    .store
                    .save_rotation_completion(&rotation, &completion)
                    .await?;
                #[cfg(feature = "replica-test-hooks")]
                state
                    .store
                    .observe(crate::management_store::RecordCheckpoint::RotationCommitSaved);
                let saved = state
                    .store
                    .saved_rotation()
                    .await?
                    .ok_or(RotationError::Rejected)?;
                state.activate_rotation(&saved).await?;
                None
            }
        };
        if let Some(rotation) = pending {
            let mut bytes = [0; 32];
            exchange.read_fixed(&mut bytes).await?;
            let ack_frame = {
                let state = node.access(deadline).await?;
                state.check_current_rotation(&rotation).await?;
                state.check_rotation_sender(connection, peer)?;
                if let Some(profile) = profile {
                    state.check_original_rotation_provider(profile, connection.remote_id())?;
                }
                #[cfg(feature = "replica-test-hooks")]
                state
                    .store
                    .observe(crate::management_store::RecordCheckpoint::RotationSecretReceived);
                let secret = AdmissionSecret::from_saved(bytes);
                let proposal = &rotation.proposal;
                let scope = AdmissionScope::new(proposal.network, proposal.generation);
                if secret.fingerprint(&scope) != proposal.fingerprint {
                    return Err(RotationError::Rejected);
                }
                save_staged(&state.directory, proposal, &secret)?;
                state.gate.stage_rotation_secret(scope, secret);
                #[cfg(feature = "replica-test-hooks")]
                state
                    .store
                    .observe(crate::management_store::RecordCheckpoint::RotationSecretSaved);
                let ack = state.durable_rotation_ack(&rotation)?;
                state.store.save_rotation_ack(&rotation, &ack).await?;
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
                            proposal: rotation.proposal_hash,
                            member: *key.verifying_key().as_bytes(),
                        },
                        &key,
                    )?,
                    #[cfg(feature = "replica-test-hooks")]
                    ReceiptMode::Fault(RotationAckFault::PreviousRotation) => {
                        let previous: Vec<u8> = sqlx::query_scalar("SELECT proposal FROM admission_rotations WHERE generation<? ORDER BY generation DESC LIMIT 1")
                        .bind(rotation.proposal.generation as i64).fetch_one(&state.store.pool).await.map_err(StoreError::from)?;
                        let signed: Signed = decode(&previous)?;
                        let proposal = verify_proposal(state.anchor, &signed)?;
                        let saved: Vec<u8> = sqlx::query_scalar("SELECT acknowledgement FROM admission_rotation_acks WHERE rotation=? AND member=?")
                        .bind(proposal.id.as_slice()).bind(state.node_key.verifying_key().as_bytes().as_slice())
                        .fetch_one(&state.store.pool).await.map_err(StoreError::from)?;
                        let stale: Signed = decode(&saved)?;
                        verify_ack(
                            &stale,
                            &SavedRotation::new(signed, proposal, None)?,
                        )?;
                        stale
                    }
                };
                let retained = state.retained_members();
                let forwarded = state
                    .store
                    .rotation_acks(&rotation)
                    .await?
                    .into_iter()
                    .filter(|(member, _)| retained.contains(member))
                    .map(|(_, ack)| ack)
                    .collect::<Vec<_>>();
                #[cfg(feature = "replica-test-hooks")]
                let forwarded = if tamper_forwarded {
                    let mut mutated = forwarded;
                    let other = mutated
                        .iter_mut()
                        .find(|ack| {
                            verify_ack(ack, &rotation).is_ok_and(|member| {
                                member != *state.node_key.verifying_key().as_bytes()
                            })
                        })
                        .ok_or(RotationError::Rejected)?;
                    other.signature[0] ^= 1;
                    mutated
                } else {
                    forwarded
                };
                encode(&(wire_ack, forwarded))?
            };
            exchange.write_frame(&ack_frame).await?;
            #[cfg(feature = "replica-test-hooks")]
            node.access(deadline)
                .await?
                .store
                .observe(crate::management_store::RecordCheckpoint::RotationAcknowledgementSent);
        } else {
            exchange.write_fixed(b"DONE").await?;
        }
        let mut saved = [0; 4];
        exchange.read_fixed(&mut saved).await?;
        if &saved != b"SAVE" {
            return Err(RotationError::Rejected);
        }
        exchange.write_fixed(b"RCVD").await?;
        exchange.finish_send().await?;
        if _root_session.is_some() {
            let _ = timeout_at(deadline, connection.revocation_handle().closed()).await;
        }
        node.access(deadline)
            .await?
            .rotation_progress()
            .await?
            .ok_or(RotationError::Rejected)
    }

    fn check_rotation_sender(
        &self,
        connection: &(impl AuthenticatedControl + ?Sized),
        peer: VerifyingKey,
    ) -> Result<(), RotationError> {
        self.rotation_connection(connection)?;
        if !self.is_member(self.node_key.verifying_key()) || !self.is_member(peer) {
            return Err(RotationError::Rejected);
        }
        Ok(())
    }

    fn check_original_rotation_provider(
        &self,
        profile: u8,
        provider: iroh::EndpointId,
    ) -> Result<(), RotationError> {
        let original = match profile {
            1 => self.join.map(|binding| binding.invitation.contact),
            2 => self.recovery.map(|binding| binding.contact),
            _ => None,
        };
        if original == Some(provider) {
            Ok(())
        } else {
            Err(RotationError::Rejected)
        }
    }

    fn verify_local_completion(
        &self,
        rotation: &SavedRotation,
        signed: &Signed,
    ) -> Result<(), RotationError> {
        let completed = verify_completion(rotation, self.anchor.root, signed)?;
        if !completed
            .retained
            .contains(self.node_key.verifying_key().as_bytes())
        {
            return Err(RotationError::Rejected);
        }
        Ok(())
    }
}
