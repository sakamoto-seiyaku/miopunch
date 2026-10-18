//! Durable root recovery, separate from approval and precode redemption.
//!
//! Each retry proves the original root on a fresh restricted TLS session. An
//! ordinary member forwards signed history and the current pinned secret; the
//! new device itself signs its administrator grant with the independent root.

use std::time::Duration;

use crate::control_protocol::AuthenticatedControl;
use iroh_base::{EndpointId, SecretKey};
use p2panda_auth::group::{GroupAction, GroupMember};
use p2panda_auth::{Access, GroupsExtensionArgs};
use p2panda_core::{Hash, Header, Signature, VerifyingKey};
use sqlx::Row;
use thiserror::Error;

use crate::admission_gate::ProtocolScope;
use crate::admission_proof::{AdmissionScope, AdmissionSecret};
use crate::genesis_recovery::{
    DOMAIN, RecoveryError as ProofError, SessionGuard, endpoint_key, proof_statement,
    read_control_fixed, read_control_frame, write_control_fixed, write_control_frame,
};
use crate::management_auth::{ManagementAuth, ManagementExtension};
use crate::management_store::{ManagementRecord, ManagementStore, StoreError};
use crate::native_network::{NativeNetwork, NodeError};

/// A distinct wire entry leaves the earlier one-shot proof validation intact.
pub const MANAGEMENT_RECOVERY_ALPN: &[u8] = b"miopunch/management-recovery/1";
const MAX_HISTORY_BYTES: usize = 16 * 1024 * 1024;
const MAX_GRANT_BYTES: usize = 256 * 1024;
const MAX_RECORDS: usize = 4096;
const TIMEOUT: Duration = Duration::from_secs(15);
type WireRecords = Vec<(Vec<u8>, Option<Vec<u8>>)>;

#[derive(Clone, Copy)]
pub(crate) struct RecoveryBinding {
    pub(crate) contact: EndpointId,
    pub(crate) attempt: [u8; 16],
}

impl RecoveryBinding {
    pub(crate) fn encode(self) -> [u8; 48] {
        let mut bytes = [0; 48];
        bytes[..32].copy_from_slice(self.contact.as_bytes());
        bytes[32..].copy_from_slice(&self.attempt);
        bytes
    }

    pub(crate) fn decode(bytes: &[u8; 48]) -> Result<Self, NodeError> {
        Ok(Self {
            contact: EndpointId::from_bytes(bytes[..32].try_into().unwrap())
                .map_err(|_| NodeError::InvalidData)?,
            attempt: bytes[32..].try_into().unwrap(),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryRequest {
    pub target: VerifyingKey,
    pub attempt: [u8; 16],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryStage {
    AwaitingProof,
    ProofVerified,
    AuthorizationSaved,
    Complete,
}

/// Sender Complete is an authenticated acknowledgement. Local Complete is
/// independently derived from this device's valid saved grant and pinned secret.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryProgress {
    pub request: RecoveryRequest,
    pub authorization: Option<Hash>,
    pub stage: RecoveryStage,
}

#[derive(Debug, Error)]
pub enum ManagementRecoveryError {
    #[error("management recovery identity, proof or current authorization was rejected")]
    Rejected,
    #[error("management recovery connection failed or timed out")]
    PathUnavailable,
    #[error("root proof failed: {0}")]
    Proof(#[from] ProofError),
    #[error("durable node failed: {0}")]
    Node(#[from] NodeError),
    #[error("recovery storage failed: {0}")]
    Store(#[from] StoreError),
}

impl ManagementStore {
    async fn recovery_result(
        &self,
        request: RecoveryRequest,
    ) -> Result<RecoveryProgress, StoreError> {
        let row = sqlx::query("SELECT authorization, phase, proof_statement, proof_signature FROM management_recoveries WHERE target = ? AND attempt = ?")
            .bind(request.target.as_bytes().as_slice()).bind(request.attempt.as_slice())
            .fetch_optional(&self.pool).await?;
        let Some(row) = row else {
            return Ok(RecoveryProgress {
                request,
                authorization: None,
                stage: RecoveryStage::AwaitingProof,
            });
        };
        let statement: Vec<u8> = row.try_get("proof_statement")?;
        let signature: Vec<u8> = row.try_get("proof_signature")?;
        let signature: [u8; 64] = signature
            .try_into()
            .map_err(|_| StoreError::Corrupt("invalid recovery signature"))?;
        self.validate_saved_proof(request, &statement, &signature)?;
        let raw: Option<Vec<u8>> = row.try_get("authorization")?;
        let authorization = raw
            .map(|bytes| {
                bytes
                    .try_into()
                    .map(Hash::from_bytes)
                    .map_err(|_| StoreError::Corrupt("invalid recovery authorization hash"))
            })
            .transpose()?;
        let phase: i64 = row.try_get("phase")?;
        let stage = match (phase, authorization.is_some()) {
            (1, false) => RecoveryStage::ProofVerified,
            (2, true) => RecoveryStage::AuthorizationSaved,
            (3, true) => RecoveryStage::Complete,
            _ => return Err(StoreError::Corrupt("invalid recovery phase")),
        };
        Ok(RecoveryProgress {
            request,
            authorization,
            stage,
        })
    }

    fn validate_saved_proof(
        &self,
        request: RecoveryRequest,
        statement: &[u8],
        signature: &[u8; 64],
    ) -> Result<(), StoreError> {
        let anchor = self.anchor();
        let start = DOMAIN.len();
        if statement.len() != start + 32 * 5
            || !statement.starts_with(DOMAIN)
            || &statement[start..start + 32] != anchor.root.as_bytes()
            || &statement[start + 32..start + 64] != anchor.genesis.as_bytes()
            || &statement[start + 64..start + 96] != request.target.as_bytes()
            || !anchor
                .root
                .verify(statement, &Signature::from_bytes(signature))
        {
            return Err(StoreError::Corrupt("saved recovery proof is invalid"));
        }
        Ok(())
    }

    /// Retain genuine signed evidence for the stage, never a trusted flag.
    /// This evidence cannot authorize a retry: the live handler first verifies
    /// a NEW exporter-bound proof for that retry's actual TLS session.
    async fn save_root_proof(
        &self,
        request: RecoveryRequest,
        statement: &[u8],
        signature: &[u8; 64],
    ) -> Result<(), StoreError> {
        self.validate_saved_proof(request, statement, signature)?;
        sqlx::query("INSERT INTO management_recoveries (target, attempt, phase, proof_statement, proof_signature) VALUES (?, ?, 1, ?, ?) ON CONFLICT(target, attempt) DO UPDATE SET proof_statement=excluded.proof_statement, proof_signature=excluded.proof_signature")
            .bind(request.target.as_bytes().as_slice()).bind(request.attempt.as_slice())
            .bind(statement).bind(signature.as_slice())
            .execute(&self.pool).await?;
        Ok(())
    }

    /// Persist the exact signed grant and result atomically, before application
    /// or sending. A restarted device resumes these bytes, never another Add.
    async fn save_recovery_grant(
        &self,
        request: RecoveryRequest,
        record: &ManagementRecord,
    ) -> Result<Hash, StoreError> {
        let id = Hash::digest(record.header());
        let existing = self.recovery_result(request).await?;
        if let Some(old) = existing.authorization {
            if old != id {
                return Err(StoreError::Corrupt("recovery result changed"));
            }
            if self.get_record(id).await?.as_ref() != Some(record)
                || !self.administrator_authorization_valid(id, request.target)
            {
                return Err(StoreError::Corrupt("saved recovery grant is invalid"));
            }
            return Ok(id);
        }
        // Retry or a verified relay history can already contain these exact
        // signed bytes. Rechecking their current authority is required; feeding
        // an applied sequence-zero operation through NEW authoring preflight
        // would correctly fail append-only log continuity.
        if let Some(applied) = self.get_record(id).await? {
            if applied != *record || !self.administrator_authorization_valid(id, request.target) {
                return Err(StoreError::Corrupt("relayed recovery grant is invalid"));
            }
        } else {
            self.validate_local_record(record)?;
        }
        let mut tx = self.pool.begin().await?;
        let changed = sqlx::query("UPDATE management_recoveries SET authorization = ?, phase = 2 WHERE target = ? AND attempt = ? AND authorization IS NULL")
            .bind(id.as_bytes().as_slice()).bind(request.target.as_bytes().as_slice())
            .bind(request.attempt.as_slice()).execute(&mut *tx).await?;
        if changed.rows_affected() != 1 {
            return Err(StoreError::Corrupt("root proof is absent"));
        }
        sqlx::query("INSERT OR IGNORE INTO management_records (id, header, body) VALUES (?, ?, ?)")
            .bind(id.as_bytes().as_slice())
            .bind(record.header())
            .bind(record.body())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        #[cfg(feature = "replica-test-hooks")]
        self.observe(crate::management_store::RecordCheckpoint::RecoveryGrantCommitted);
        Ok(id)
    }

    async fn complete_recovery(&self, progress: RecoveryProgress) -> Result<(), StoreError> {
        let authorization = progress
            .authorization
            .ok_or(StoreError::Corrupt("recovery has no grant"))?;
        let changed = sqlx::query("UPDATE management_recoveries SET phase = 3 WHERE target = ? AND attempt = ? AND authorization = ?")
            .bind(progress.request.target.as_bytes().as_slice()).bind(progress.request.attempt.as_slice())
            .bind(authorization.as_bytes().as_slice()).execute(&self.pool).await?;
        if changed.rows_affected() != 1 {
            return Err(StoreError::Corrupt("recovery result changed"));
        }
        Ok(())
    }
}

impl NativeNetwork {
    /// Historical completion permits switching to a fresh ordinary request;
    /// it grants no current membership. Reopening that request may have lost
    /// its secret, so the completed durable stage remains usable only alongside
    /// the actual original root proof and applied, signed grant for this node.
    pub(crate) async fn has_completed_root_recovery(
        &self,
        binding: RecoveryBinding,
    ) -> Result<bool, StoreError> {
        let request = RecoveryRequest {
            target: self.node_key.verifying_key(),
            attempt: binding.attempt,
        };
        let progress = self.store.recovery_result(request).await?;
        let Some(id) = progress.authorization else {
            return Ok(false);
        };
        let Some(record) = self.store.get_record(id).await? else {
            return Ok(false);
        };
        let header = Header::<ManagementExtension>::decode(record.header())
            .map_err(|_| StoreError::Corrupt("invalid historical recovery grant"))?;
        if header.verifying_key != self.anchor.root
            || header.seq_num != 0
            || header.backlink.is_some()
            || !matches!(&header.extensions.group.action,
                GroupAction::Add { member: GroupMember::Individual(node), access }
                    if *node == request.target && *access == Access::manage())
        {
            return Ok(false);
        }
        if progress.stage == RecoveryStage::Complete {
            return Ok(true);
        }
        let Some(secret) = &self.secret else {
            return Ok(false);
        };
        self.store
            .current_admission_matches(&self.scope, &secret.fingerprint(&self.scope))
            .await
    }

    pub(crate) async fn seal_completed_root_recovery(
        &self,
        binding: RecoveryBinding,
    ) -> Result<(), StoreError> {
        let request = RecoveryRequest {
            target: self.node_key.verifying_key(),
            attempt: binding.attempt,
        };
        self.store
            .complete_recovery(self.store.recovery_result(request).await?)
            .await
    }

    fn recovery_connection(
        &self,
        connection: &(impl AuthenticatedControl + ?Sized),
    ) -> Result<(), ManagementRecoveryError> {
        if connection.alpn() != MANAGEMENT_RECOVERY_ALPN
            || !self.gate.allows(connection, ProtocolScope::GenesisRecovery)
        {
            return Err(ManagementRecoveryError::Rejected);
        }
        Ok(())
    }

    fn valid_recovery(&self, progress: RecoveryProgress) -> Result<Hash, ManagementRecoveryError> {
        let grant = progress
            .authorization
            .ok_or(ManagementRecoveryError::Rejected)?;
        if !self
            .store
            .administrator_authorization_valid(grant, progress.request.target)
        {
            return Err(ManagementRecoveryError::Rejected);
        }
        Ok(grant)
    }

    pub async fn recovery_progress(&self) -> Result<RecoveryProgress, ManagementRecoveryError> {
        let binding = self.recovery.ok_or(ManagementRecoveryError::Rejected)?;
        let request = RecoveryRequest {
            target: self.node_key.verifying_key(),
            attempt: binding.attempt,
        };
        let mut progress = self.store.recovery_result(request).await?;
        if progress.authorization.is_some() {
            self.valid_recovery(progress)?;
            progress.stage = if self.secret.is_some() {
                RecoveryStage::Complete
            } else {
                RecoveryStage::AuthorizationSaved
            };
        }
        Ok(progress)
    }

    /// Current durable relay result, without giving a query the authority to
    /// approve anything. A removed/demoted result is no longer a valid delivery.
    pub async fn served_management_recovery(
        &mut self,
        request: RecoveryRequest,
    ) -> Result<RecoveryProgress, ManagementRecoveryError> {
        self.store.apply_ready().await?;
        self.refresh_authorization();
        let progress = self.store.recovery_result(request).await?;
        if progress.authorization.is_some() {
            self.valid_recovery(progress)?;
        }
        Ok(progress)
    }

    pub async fn restore_management(
        &mut self,
        connection: &(impl AuthenticatedControl + ?Sized),
    ) -> Result<RecoveryProgress, ManagementRecoveryError> {
        let (_permit, _session) = SessionGuard::begin(connection)?;
        tokio::time::timeout(TIMEOUT, self.restore_management_inner(connection))
            .await
            .unwrap_or(Err(ManagementRecoveryError::PathUnavailable))
    }

    async fn restore_management_inner(
        &mut self,
        connection: &(impl AuthenticatedControl + ?Sized),
    ) -> Result<RecoveryProgress, ManagementRecoveryError> {
        self.store.ensure_admissions_open().await?;
        self.store.apply_ready().await?;
        self.refresh_authorization();
        self.recovery_connection(connection)?;
        let binding = self.recovery.ok_or(ManagementRecoveryError::Rejected)?;
        if connection.remote_id() != binding.contact {
            return Err(ManagementRecoveryError::Rejected);
        }
        let credential = self.saved_genesis_credential()?;
        let local = SecretKey::from_bytes(self.node_key.as_bytes()).public();
        let request = RecoveryRequest {
            target: self.node_key.verifying_key(),
            attempt: binding.attempt,
        };
        let statement = proof_statement(connection, self.anchor, local, binding.contact)?;
        let exchange = connection
            .open_control_exchange()
            .await
            .map_err(|_| ManagementRecoveryError::PathUnavailable)?;
        let signature = credential.root.sign(&statement).to_bytes();
        write_control_fixed(&exchange, &signature)
            .await
            .map_err(|_| ManagementRecoveryError::PathUnavailable)?;
        write_control_fixed(&exchange, &request.attempt)
            .await
            .map_err(|_| ManagementRecoveryError::PathUnavailable)?;

        let wire = read_control_frame(&exchange, MAX_HISTORY_BYTES).await?;
        let (server_grant, raw): (Option<Hash>, WireRecords) =
            p2panda_core::cbor::decode_cbor(wire.as_slice())
                .map_err(|_| ManagementRecoveryError::Rejected)?;
        if raw.is_empty()
            || raw.len() > MAX_RECORDS
            || Hash::digest(&raw[0].0) != self.anchor.genesis
        {
            return Err(ManagementRecoveryError::Rejected);
        }
        let auth = ManagementAuth::replay_records(self.anchor, &raw)
            .map_err(|_| ManagementRecoveryError::Rejected)?;
        let contact = endpoint_key(binding.contact)?;
        if !auth.nodes().iter().any(|(id, _)| *id == contact) {
            return Err(ManagementRecoveryError::Rejected);
        }
        if let Some(grant) = server_grant {
            if !auth.administrator_authorization_valid(grant, request.target) {
                return Err(ManagementRecoveryError::Rejected);
            }
        }
        #[cfg(feature = "replica-test-hooks")]
        self.store
            .observe(crate::management_store::RecordCheckpoint::RecoveryHistoryVerified);
        self.store
            .save_root_proof(request, &statement, &signature)
            .await?;
        for (header, body) in raw {
            self.store
                .ingest(&ManagementRecord::new(header, body.as_deref()))
                .await?;
        }
        self.store.apply_ready().await?;
        self.refresh_authorization();
        if !self.is_member(contact) {
            return Err(ManagementRecoveryError::Rejected);
        }
        #[cfg(feature = "replica-test-hooks")]
        self.store
            .observe(crate::management_store::RecordCheckpoint::RecoveryHistorySaved);
        let previous = self.store.recovery_result(request).await?;
        let grant = match previous.authorization.or(server_grant) {
            Some(id) => {
                if server_grant.is_some_and(|server| server != id) {
                    return Err(ManagementRecoveryError::Rejected);
                }
                self.store
                    .get_record(id)
                    .await?
                    .ok_or(ManagementRecoveryError::Rejected)?
            }
            None => {
                let log_id = loop {
                    let log = rand::random();
                    if !self.store.has_author_log_id(self.anchor.root, log) {
                        break log;
                    }
                };
                ManagementRecord::new(
                    Header::<ManagementExtension>::builder()
                        .build(
                            &credential.root,
                            ManagementExtension {
                                version: 1,
                                log_id,
                                group: GroupsExtensionArgs {
                                    group_id: self.anchor.root,
                                    action: GroupAction::Add {
                                        member: GroupMember::Individual(request.target),
                                        access: Access::manage(),
                                    },
                                    dependencies: self.store.tips(),
                                },
                            },
                        )
                        .encode(),
                    None,
                )
            }
        };
        let grant_id = self.store.save_recovery_grant(request, &grant).await?;
        self.store.apply_ready().await?;
        self.refresh_authorization();
        self.valid_recovery(self.store.recovery_result(request).await?)?;
        #[cfg(feature = "replica-test-hooks")]
        self.store
            .observe(crate::management_store::RecordCheckpoint::AuthorizationSaved);
        write_control_frame(&exchange, grant.header()).await?;
        #[cfg(feature = "replica-test-hooks")]
        self.store
            .observe(crate::management_store::RecordCheckpoint::AuthorizationSent);
        let mut acknowledged = [0; 32];
        read_control_fixed(&exchange, &mut acknowledged)
            .await
            .map_err(|_| ManagementRecoveryError::PathUnavailable)?;
        if acknowledged != *grant_id.as_bytes() {
            return Err(ManagementRecoveryError::Rejected);
        }
        write_control_fixed(&exchange, b"AUTH")
            .await
            .map_err(|_| ManagementRecoveryError::PathUnavailable)?;
        #[cfg(feature = "replica-test-hooks")]
        self.store
            .observe(crate::management_store::RecordCheckpoint::AuthorizationAcknowledged);
        let mut material = [0; 40];
        read_control_fixed(&exchange, &mut material)
            .await
            .map_err(|_| ManagementRecoveryError::PathUnavailable)?;
        #[cfg(feature = "replica-test-hooks")]
        self.store
            .observe(crate::management_store::RecordCheckpoint::SecretReceived);
        self.valid_recovery(self.store.recovery_result(request).await?)?;
        self.save_admission_material(
            AdmissionScope::new(
                *self.anchor.root.as_bytes(),
                u64::from_be_bytes(material[..8].try_into().unwrap()),
            ),
            AdmissionSecret::from_saved(material[8..].try_into().unwrap()),
        )
        .await?;
        #[cfg(feature = "replica-test-hooks")]
        self.store
            .observe(crate::management_store::RecordCheckpoint::SecretSaved);
        self.store
            .complete_recovery(self.store.recovery_result(request).await?)
            .await?;
        write_control_fixed(&exchange, b"SAVE")
            .await
            .map_err(|_| ManagementRecoveryError::PathUnavailable)?;
        #[cfg(feature = "replica-test-hooks")]
        self.store
            .observe(crate::management_store::RecordCheckpoint::CompletionSent);
        exchange
            .finish_send()
            .await
            .map_err(|_| ManagementRecoveryError::PathUnavailable)?;
        let mut done = [0; 4];
        read_control_fixed(&exchange, &mut done)
            .await
            .map_err(|_| ManagementRecoveryError::PathUnavailable)?;
        if &done != b"DONE" {
            return Err(ManagementRecoveryError::Rejected);
        }
        self.recovery_progress().await
    }

    pub async fn serve_management_recovery(
        &mut self,
        connection: &(impl AuthenticatedControl + ?Sized),
    ) -> Result<RecoveryProgress, ManagementRecoveryError> {
        let (_permit, _session) = SessionGuard::begin(connection)?;
        tokio::time::timeout(TIMEOUT, self.serve_management_recovery_inner(connection))
            .await
            .unwrap_or(Err(ManagementRecoveryError::PathUnavailable))
    }

    async fn serve_management_recovery_inner(
        &mut self,
        connection: &(impl AuthenticatedControl + ?Sized),
    ) -> Result<RecoveryProgress, ManagementRecoveryError> {
        self.store.ensure_admissions_open().await?;
        self.store.apply_ready().await?;
        self.refresh_authorization();
        self.recovery_connection(connection)?;
        let local = SecretKey::from_bytes(self.node_key.as_bytes()).public();
        if !self.is_member(self.node_key.verifying_key())
            || self.node_key.verifying_key() == self.anchor.root
        {
            return Err(ManagementRecoveryError::Rejected);
        }
        let secret = self
            .secret
            .as_ref()
            .ok_or(ManagementRecoveryError::Rejected)?;
        if !self
            .store
            .current_admission_matches(&self.scope, &secret.fingerprint(&self.scope))
            .await?
        {
            return Err(ManagementRecoveryError::Rejected);
        }
        let statement = proof_statement(connection, self.anchor, connection.remote_id(), local)?;
        let exchange = connection
            .accept_control_exchange()
            .await
            .map_err(|_| ManagementRecoveryError::PathUnavailable)?;
        let mut signature = [0; 64];
        read_control_fixed(&exchange, &mut signature)
            .await
            .map_err(|_| ManagementRecoveryError::PathUnavailable)?;
        if !self
            .anchor
            .root
            .verify(&statement, &Signature::from_bytes(&signature))
        {
            return Err(ManagementRecoveryError::Rejected);
        }
        let mut attempt = [0; 16];
        read_control_fixed(&exchange, &mut attempt)
            .await
            .map_err(|_| ManagementRecoveryError::PathUnavailable)?;
        let request = RecoveryRequest {
            target: endpoint_key(connection.remote_id())?,
            attempt,
        };
        self.store
            .save_root_proof(request, &statement, &signature)
            .await?;
        #[cfg(feature = "replica-test-hooks")]
        self.store
            .observe(crate::management_store::RecordCheckpoint::RecoveryProofSaved);
        let progress = self.store.recovery_result(request).await?;
        if progress.authorization.is_some() {
            self.valid_recovery(progress)?;
        }
        let records = self.store.applied_records().await?;
        if records.is_empty()
            || records.len() > MAX_RECORDS
            || Hash::digest(records[0].header()) != self.anchor.genesis
        {
            return Err(ManagementRecoveryError::Rejected);
        }
        let raw: WireRecords = records
            .iter()
            .map(|r| (r.header().to_vec(), r.body().map(<[u8]>::to_vec)))
            .collect();
        self.store.ensure_admissions_open().await?;
        let wire = p2panda_core::cbor::encode_cbor(&(progress.authorization, raw))
            .map_err(|_| ManagementRecoveryError::Rejected)?;
        write_control_frame(&exchange, &wire).await?;
        let grant =
            ManagementRecord::new(read_control_frame(&exchange, MAX_GRANT_BYTES).await?, None);
        let header = Header::<ManagementExtension>::decode(grant.header())
            .map_err(|_| ManagementRecoveryError::Rejected)?;
        if header.verifying_key != self.anchor.root
            || header.seq_num != 0
            || header.backlink.is_some()
            || !matches!(&header.extensions.group.action,
                GroupAction::Add { member: GroupMember::Individual(node), access }
                    if *node == request.target && *access == Access::manage())
        {
            return Err(ManagementRecoveryError::Rejected);
        }
        if let Some(existing) = progress.authorization {
            if Hash::digest(grant.header()) != existing {
                return Err(ManagementRecoveryError::Rejected);
            }
            self.valid_recovery(progress)?;
        } else if self
            .store
            .has_author_log_id(self.anchor.root, header.extensions.log_id)
            || header.extensions.group.dependencies != self.store.tips()
        {
            return Err(ManagementRecoveryError::Rejected);
        }
        let grant_id = self.store.save_recovery_grant(request, &grant).await?;
        self.store.apply_ready().await?;
        self.refresh_authorization();
        let progress = self.store.recovery_result(request).await?;
        self.valid_recovery(progress)?;
        write_control_fixed(&exchange, grant_id.as_bytes())
            .await
            .map_err(|_| ManagementRecoveryError::PathUnavailable)?;
        #[cfg(feature = "replica-test-hooks")]
        self.store
            .observe(crate::management_store::RecordCheckpoint::AuthorizationSent);
        let mut ack = [0; 4];
        read_control_fixed(&exchange, &mut ack)
            .await
            .map_err(|_| ManagementRecoveryError::PathUnavailable)?;
        if &ack != b"AUTH" {
            return Err(ManagementRecoveryError::Rejected);
        }
        self.valid_recovery(progress)?;
        self.store.ensure_admissions_open().await?;
        let secret = self
            .secret
            .as_ref()
            .ok_or(ManagementRecoveryError::Rejected)?;
        if !self
            .store
            .current_admission_matches(&self.scope, &secret.fingerprint(&self.scope))
            .await?
        {
            return Err(ManagementRecoveryError::Rejected);
        }
        let mut material = [0; 40];
        material[..8].copy_from_slice(&self.scope.secret_generation().to_be_bytes());
        material[8..].copy_from_slice(secret.saved_bytes());
        #[cfg(feature = "replica-test-hooks")]
        self.store
            .observe(crate::management_store::RecordCheckpoint::SecretPrepared);
        write_control_fixed(&exchange, &material)
            .await
            .map_err(|_| ManagementRecoveryError::PathUnavailable)?;
        #[cfg(feature = "replica-test-hooks")]
        self.store
            .observe(crate::management_store::RecordCheckpoint::SecretSent);
        read_control_fixed(&exchange, &mut ack)
            .await
            .map_err(|_| ManagementRecoveryError::PathUnavailable)?;
        if &ack != b"SAVE" {
            return Err(ManagementRecoveryError::Rejected);
        }
        #[cfg(feature = "replica-test-hooks")]
        self.store
            .observe(crate::management_store::RecordCheckpoint::CompletionAcknowledged);
        self.store.complete_recovery(progress).await?;
        #[cfg(feature = "replica-test-hooks")]
        self.store
            .observe(crate::management_store::RecordCheckpoint::DeliveryComplete);
        write_control_fixed(&exchange, b"DONE")
            .await
            .map_err(|_| ManagementRecoveryError::PathUnavailable)?;
        exchange
            .finish_send()
            .await
            .map_err(|_| ManagementRecoveryError::PathUnavailable)?;
        // Let the recipient read DONE before closing this one-use TLS session.
        let _ = connection.revocation_handle().closed().await;
        self.store
            .recovery_result(request)
            .await
            .map_err(Into::into)
    }
}
