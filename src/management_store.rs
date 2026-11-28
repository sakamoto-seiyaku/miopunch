//! Small durable management-record pipeline for MIOPUNCH-24.
//!
//! Ingest commits only complete signed records. Applying a ready record and
//! saving the corresponding Groups state share one SQLite transaction. Recovery
//! replays the committed records through the same authority verifier.

use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use p2panda_auth::Access;
use p2panda_auth::group::GroupCrdtError;
use p2panda_core::{Hash, VerifyingKey};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Row, SqlitePool};
use thiserror::Error;

use crate::admission_progress::{
    AdmissionAttempt, AdmissionHistoryEntry, ApplicationProgress, RECENT_ADMISSION_ATTEMPTS,
};
use crate::admission_proof::{AdmissionScope, AdmissionSecret};
use crate::management_auth::{AuthError, ManagementAuth, NetworkAnchor, decode_complete};
use crate::manual_admission::{AdmissionRequest, AdmissionStage, ApprovedAdmission};

pub use crate::management_record::ManagementRecord;

fn decode_admission_request(row: &sqlx::sqlite::SqliteRow) -> Result<AdmissionRequest, StoreError> {
    let applicant = row
        .try_get::<Vec<u8>, _>("applicant")?
        .try_into()
        .map_err(|_| StoreError::Corrupt("admission applicant"))?;
    let request_id = row
        .try_get::<Vec<u8>, _>("request_id")?
        .try_into()
        .map_err(|_| StoreError::Corrupt("admission request id"))?;
    Ok(AdmissionRequest {
        applicant: VerifyingKey::from_bytes(&applicant)
            .map_err(|_| StoreError::Corrupt("admission applicant"))?,
        request_id,
    })
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("SQLite storage failed: {0}")]
    Sql(#[from] sqlx::Error),
    #[error("management record failed verification: {0}")]
    Auth(#[from] AuthError),
    #[error("Groups state could not be encoded: {0}")]
    Encode(#[from] p2panda_core::cbor::EncodeError),
    #[error("Groups state could not be decoded: {0}")]
    Decode(#[from] p2panda_core::cbor::DecodeError),
    #[error("database is pinned to a different network anchor")]
    WrongAnchor,
    #[error("admission generation is older, reused with a different secret, or invalid")]
    InvalidAdmission,
    #[error("admission and approved-material delivery are paused for secret rotation")]
    AdmissionPaused,
    #[error(
        "precode is stopped, expired, exhausted, revoked, or has inconsistent consumption evidence"
    )]
    PrecodeStopped,
    #[error("durable management state is inconsistent: {0}")]
    Corrupt(&'static str),
    #[error("simulated process interruption")]
    Interrupted,
}

/// One network's complete signed history and locally committed authority view.
/// The caller must supply an anchor obtained from a trusted source.
pub struct ManagementStore {
    pub(crate) pool: SqlitePool,
    anchor: NetworkAnchor,
    auth: ManagementAuth,
    applied_count: i64,
    pending_commit: Option<PendingCommit>,
    authorization_gate: Option<crate::admission_gate::AdmissionGate>,
    publication: Arc<Mutex<()>>,
    pub(crate) changes: crate::state_observation::StateChanges,
    pub(crate) staged_admission:
        Arc<tokio::sync::Mutex<Option<crate::admission_history_wire::HistoryCheckpoint>>>,
    #[cfg(feature = "replica-test-hooks")]
    checkpoint_observer: Option<fn(RecordCheckpoint)>,
}

struct CommittedView {
    auth: ManagementAuth,
    applied_count: i64,
}

struct PendingCommit {
    committed: Arc<OnceLock<CommittedView>>,
    task: tokio::task::JoinHandle<Result<(), StoreError>>,
}

pub(crate) struct LocalAdmissionDelivery<'a> {
    pub(crate) receipt: crate::admission_progress::AdmissionReceipt,
    pub(crate) contact: &'a iroh::EndpointAddr,
    pub(crate) base_request_id: [u8; 16],
}

/// Real durability boundaries available only to explicit fault-injection
/// builds. Observers can terminate the test process without altering storage.
#[cfg(feature = "replica-test-hooks")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordCheckpoint {
    MigrationValidated,
    MigrationSwitched,
    MigrationSourceExitCommitted,
    RotationInitialMaterialSaved,
    RotationProposalSaved,
    RotationSecretReceived,
    RotationSecretSaved,
    RotationAcknowledgementSent,
    RotationAcknowledgementReceived,
    RotationAcknowledgementSaved,
    RotationCommitSaved,
    RotationMaterialPublished,
    RotationCredentialPinned,
    RecoveryProofSaved,
    RecoveryHistoryVerified,
    RecoveryHistorySaved,
    RecoveryGrantCommitted,
    RecoveryRejoinPrepared,
    RecoveryRejoinPublished,
    AuthorizationPrepared,
    AuthorizationSent,
    AuthorizationAcknowledged,
    SecretPrepared,
    SecretSent,
    CompletionAcknowledged,
    DeliveryComplete,
    AuthorizationVerified,
    AuthorizationSaved,
    SecretReceived,
    SecretFilePublished,
    SecretSaved,
    CompletionSent,
    AdmissionReceiptSettled,
    AdmissionReceiptTimedOut,
    BeforePrecodeCommit,
    AfterPrecodeCommit,
    BeforeStorageCommit,
    AfterStorageCommit,
    AfterOrdererSelect,
    BeforeGroupsCommit,
    AfterGroupsCommit,
}

impl ManagementStore {
    pub async fn open(path: impl AsRef<Path>, anchor: NetworkAnchor) -> Result<Self, StoreError> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        crate::admission_history_store::initialize(&pool).await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS management_anchor (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                root BLOB NOT NULL,
                genesis BLOB NOT NULL
            )",
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS admission_deliveries (
                applicant BLOB NOT NULL,
                request_id BLOB NOT NULL,
                authorization BLOB,
                phase INTEGER NOT NULL DEFAULT 0 CHECK (phase BETWEEN 0 AND 3),
                PRIMARY KEY (applicant, request_id)
            )",
        )
        .execute(&pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS local_admission_attempt (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                request_id BLOB NOT NULL,
                progress INTEGER NOT NULL CHECK (progress BETWEEN 0 AND 4),
                authorization BLOB
            )",
        )
        .execute(&pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS pending_admission_receipts (
                receipt BLOB PRIMARY KEY,
                contact TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS management_recoveries (
                target BLOB NOT NULL,
                attempt BLOB NOT NULL,
                authorization BLOB,
                proof_statement BLOB NOT NULL,
                proof_signature BLOB NOT NULL,
                phase INTEGER NOT NULL CHECK (phase BETWEEN 1 AND 3),
                PRIMARY KEY (target, attempt)
            )",
        )
        .execute(&pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS admission_rejections (
                applicant BLOB NOT NULL, request_id BLOB NOT NULL,
                PRIMARY KEY (applicant, request_id)
            )",
        )
        .execute(&pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS precodes (
            token BLOB PRIMARY KEY, origin BLOB NOT NULL, expires INTEGER NOT NULL,
            max_uses INTEGER, used INTEGER NOT NULL DEFAULT 0,
            stopped INTEGER NOT NULL DEFAULT 0,
            revoked INTEGER NOT NULL DEFAULT 0
        )",
        )
        .execute(&pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS precode_redemptions (
            token BLOB NOT NULL, applicant BLOB NOT NULL, request_id BLOB NOT NULL,
            PRIMARY KEY(token, applicant, request_id)
        )",
        )
        .execute(&pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS management_records (
                arrival INTEGER PRIMARY KEY AUTOINCREMENT,
                id BLOB NOT NULL UNIQUE,
                header BLOB NOT NULL,
                body BLOB,
                applied_order INTEGER UNIQUE,
                rejected_reason TEXT
            )",
        )
        .execute(&pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS management_view (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                applied_count INTEGER NOT NULL,
                state BLOB NOT NULL
            )",
        )
        .execute(&pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS management_admission (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                generation INTEGER NOT NULL,
                credential_fingerprint BLOB NOT NULL
            )",
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS admission_rotations (
            generation INTEGER PRIMARY KEY, proposal BLOB NOT NULL,
            completion BLOB
        )",
        )
        .execute(&pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS admission_rotation_acks (
            rotation BLOB NOT NULL, member BLOB NOT NULL, acknowledgement BLOB NOT NULL,
            PRIMARY KEY (rotation, member)
        )",
        )
        .execute(&pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS admission_material_generations (
            request_id BLOB PRIMARY KEY, generation INTEGER NOT NULL CHECK (generation >= 0)
        )",
        )
        .execute(&pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS pending_rejoin_material (request_id BLOB PRIMARY KEY)",
        )
        .execute(&pool)
        .await?;

        sqlx::query("INSERT OR IGNORE INTO management_anchor VALUES (1, ?, ?)")
            .bind(anchor.root.as_bytes().as_slice())
            .bind(anchor.genesis.as_bytes().as_slice())
            .execute(&pool)
            .await?;
        let pinned = sqlx::query("SELECT root, genesis FROM management_anchor WHERE singleton = 1")
            .fetch_one(&pool)
            .await?;
        let root: Vec<u8> = pinned.try_get("root")?;
        let genesis: Vec<u8> = pinned.try_get("genesis")?;
        if root != anchor.root.as_bytes() || genesis != anchor.genesis.as_bytes() {
            return Err(StoreError::WrongAnchor);
        }

        let empty = ManagementAuth::new(anchor);
        sqlx::query("INSERT OR IGNORE INTO management_view VALUES (1, 0, ?)")
            .bind(empty.member_view_bytes()?)
            .execute(&pool)
            .await?;
        let snapshot =
            sqlx::query("SELECT applied_count, state FROM management_view WHERE singleton = 1")
                .fetch_one(&pool)
                .await?;
        let applied_count: i64 = snapshot.try_get("applied_count")?;
        let state: Vec<u8> = snapshot.try_get("state")?;
        let rows = sqlx::query(
            "SELECT id, header, body FROM management_records
             WHERE applied_order IS NOT NULL ORDER BY applied_order",
        )
        .fetch_all(&pool)
        .await?;
        if rows.len() as i64 != applied_count {
            return Err(StoreError::Corrupt(
                "applied count differs from committed records",
            ));
        }
        let mut records = Vec::with_capacity(rows.len());
        for row in rows {
            let id: Vec<u8> = row.try_get("id")?;
            let header: Vec<u8> = row.try_get("header")?;
            let body: Option<Vec<u8>> = row.try_get("body")?;
            if id != Hash::digest(&header).as_bytes() {
                return Err(StoreError::Corrupt("record id differs from signed header"));
            }
            records.push((header, body));
        }
        let auth = ManagementAuth::replay_records(anchor, &records)?;
        if auth.member_view_bytes()? != state {
            return Err(StoreError::Corrupt(
                "saved Groups view differs from signed replay",
            ));
        }
        let store = Self {
            pool,
            anchor,
            auth,
            applied_count,
            pending_commit: None,
            authorization_gate: None,
            publication: Arc::default(),
            changes: Default::default(),
            staged_admission: Arc::default(),
            #[cfg(feature = "replica-test-hooks")]
            checkpoint_observer: None,
        };
        store.restore_admission_history().await?;
        Ok(store)
    }

    #[cfg(feature = "replica-test-hooks")]
    pub fn set_checkpoint_observer(&mut self, observer: fn(RecordCheckpoint)) {
        self.checkpoint_observer = Some(observer);
    }

    #[cfg(feature = "replica-test-hooks")]
    pub(crate) fn observe(&self, checkpoint: RecordCheckpoint) {
        if let Some(observer) = self.checkpoint_observer {
            observer(checkpoint);
        }
    }

    /// Pin the active local admission credential before accepting bootstrap
    /// sessions. Reopening the database cannot silently restore an older
    /// generation or substitute another secret for the same generation.
    pub async fn set_current_admission(
        &mut self,
        scope: &AdmissionScope,
        secret: &AdmissionSecret,
    ) -> Result<(), StoreError> {
        self.commit_admission_material(scope, secret, None).await
    }

    pub(crate) async fn commit_admission_material(
        &self,
        scope: &AdmissionScope,
        secret: &AdmissionSecret,
        delivery: Option<LocalAdmissionDelivery<'_>>,
    ) -> Result<(), StoreError> {
        if scope.network_id() != self.anchor.root.as_bytes() {
            return Err(StoreError::WrongAnchor);
        }
        let generation: i64 = scope
            .secret_generation()
            .try_into()
            .map_err(|_| StoreError::InvalidAdmission)?;
        let fingerprint = secret.fingerprint(scope);
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query("SELECT generation, credential_fingerprint FROM management_admission WHERE singleton = 1")
            .fetch_optional(&mut *tx).await?;
        match row {
            Some(row) => {
                let old_generation: i64 = row.try_get("generation")?;
                let old_fingerprint: Vec<u8> = row.try_get("credential_fingerprint")?;
                if generation < old_generation
                    || (generation == old_generation && old_fingerprint != fingerprint)
                {
                    return Err(StoreError::InvalidAdmission);
                }
                if generation > old_generation {
                    let updated = sqlx::query("UPDATE management_admission SET generation = ?, credential_fingerprint = ? WHERE singleton = 1 AND generation < ?")
                        .bind(generation).bind(fingerprint.as_slice()).bind(generation).execute(&mut *tx).await?;
                    if updated.rows_affected() != 1 {
                        return Err(StoreError::InvalidAdmission);
                    }
                }
            }
            None => {
                sqlx::query("INSERT INTO management_admission VALUES (1, ?, ?)")
                    .bind(generation)
                    .bind(fingerprint.as_slice())
                    .execute(&mut *tx)
                    .await?;
            }
        }
        if let Some(LocalAdmissionDelivery {
            receipt,
            contact,
            base_request_id,
        }) = delivery
        {
            let changed = sqlx::query(
                "UPDATE local_admission_attempt SET progress = 3
                WHERE request_id = ? AND authorization = ? AND progress IN (2, 3)",
            )
            .bind(receipt.request_id.as_slice())
            .bind(receipt.authorization.as_bytes().as_slice())
            .execute(&mut *tx)
            .await?;
            sqlx::query("DELETE FROM pending_rejoin_material WHERE request_id = ?")
                .bind(base_request_id.as_slice())
                .execute(&mut *tx)
                .await?;
            if changed.rows_affected() != 1 {
                return Err(StoreError::Corrupt(
                    "saved admission does not match the active delivery",
                ));
            }
            let contact = serde_json::to_string(contact)
                .map_err(|_| StoreError::Corrupt("admission receipt contact"))?;
            sqlx::query(
                "INSERT OR IGNORE INTO pending_admission_receipts (receipt, contact) VALUES (?, ?)",
            )
            .bind(receipt.encode().as_slice())
            .bind(contact)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn current_admission_matches(
        &self,
        scope: &AdmissionScope,
        fingerprint: &[u8; 32],
    ) -> Result<bool, StoreError> {
        let row = sqlx::query("SELECT generation, credential_fingerprint FROM management_admission WHERE singleton = 1")
            .fetch_optional(&self.pool).await?;
        let Some(row) = row else { return Ok(false) };
        let generation: i64 = row.try_get("generation")?;
        let tag: Vec<u8> = row.try_get("credential_fingerprint")?;
        Ok(scope.network_id() == self.anchor.root.as_bytes()
            && generation >= 0
            && scope.secret_generation() == generation as u64
            && tag == fingerprint)
    }

    pub(crate) async fn expected_admission_generation(
        &self,
        request_id: [u8; 16],
    ) -> Result<Option<u64>, StoreError> {
        let generation: Option<i64> = sqlx::query_scalar(
            "SELECT generation FROM admission_material_generations WHERE request_id=?",
        )
        .bind(request_id.as_slice())
        .fetch_optional(&self.pool)
        .await?;
        generation
            .map(|generation| {
                generation
                    .try_into()
                    .map_err(|_| StoreError::InvalidAdmission)
            })
            .transpose()
    }

    pub(crate) async fn pending_rejoin_material(
        &self,
        request_id: [u8; 16],
    ) -> Result<bool, StoreError> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pending_rejoin_material WHERE request_id = ?)",
        )
        .bind(request_id.as_slice())
        .fetch_one(&self.pool)
        .await?)
    }

    pub(crate) async fn mark_pending_rejoin(
        &self,
        request_id: [u8; 16],
        previous: Option<[u8; 16]>,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM pending_rejoin_material WHERE request_id IS NOT ?")
            .bind(previous.as_ref().map(|id| id.as_slice()))
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO pending_rejoin_material VALUES (?)")
            .bind(request_id.as_slice())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn save_expected_admission_generation(
        &self,
        request_id: [u8; 16],
        generation: u64,
    ) -> Result<(), StoreError> {
        let generation: i64 = generation
            .try_into()
            .map_err(|_| StoreError::InvalidAdmission)?;
        if self
            .expected_admission_generation(request_id)
            .await?
            .is_some_and(|old| old > generation as u64)
        {
            return Err(StoreError::InvalidAdmission);
        }
        sqlx::query("INSERT INTO admission_material_generations VALUES (?, ?) ON CONFLICT(request_id) DO UPDATE SET generation=excluded.generation WHERE generation<=excluded.generation")
            .bind(request_id.as_slice()).bind(generation).execute(&self.pool).await?;
        Ok(())
    }

    /// Only the restricted handler, after matching the claimed applicant to
    /// the actual TLS peer, records requests. No grant is implied by receipt.
    pub(crate) async fn record_admission_request(
        &self,
        request: &AdmissionRequest,
    ) -> Result<(), StoreError> {
        let result = sqlx::query(
            "INSERT OR IGNORE INTO admission_deliveries (applicant, request_id) VALUES (?, ?)",
        )
        .bind(request.applicant.as_bytes().as_slice())
        .bind(request.request_id.as_slice())
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 0 {
            self.changes.publish();
        }
        Ok(())
    }

    pub(crate) async fn admission_history(&self) -> Result<Vec<AdmissionHistoryEntry>, StoreError> {
        let rows = sqlx::query("SELECT d.applicant, d.request_id, d.authorization, d.phase,
                EXISTS (SELECT 1 FROM admission_rejections r WHERE r.applicant = d.applicant AND r.request_id = d.request_id) AS rejected
            FROM admission_deliveries d ORDER BY d.rowid DESC LIMIT ?")
            .bind(RECENT_ADMISSION_ATTEMPTS as i64)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|row| {
                let request = decode_admission_request(&row)?;
                let authorization = row
                    .try_get::<Option<Vec<u8>>, _>("authorization")?
                    .map(|value| {
                        value
                            .try_into()
                            .map(Hash::from_bytes)
                            .map_err(|_| StoreError::Corrupt("admission authorization"))
                    })
                    .transpose()?;
                Ok(AdmissionHistoryEntry::from_evidence(
                    request.applicant,
                    request.request_id,
                    authorization,
                    row.try_get("rejected")?,
                    row.try_get::<i64, _>("phase")? == 3,
                ))
            })
            .collect()
    }

    pub(crate) async fn local_admission_progress(
        &self,
    ) -> Result<Option<(AdmissionAttempt, Option<Hash>)>, StoreError> {
        let Some(row) = sqlx::query("SELECT request_id, progress, authorization FROM local_admission_attempt WHERE singleton = 1")
            .fetch_optional(&self.pool).await? else { return Ok(None) };
        let request = row
            .try_get::<Vec<u8>, _>("request_id")?
            .try_into()
            .map_err(|_| StoreError::Corrupt("local admission request id"))?;
        let progress = match row.try_get::<i64, _>("progress")? {
            0 => ApplicationProgress::Created,
            1 => ApplicationProgress::Delivered,
            2 => ApplicationProgress::Authorized,
            3 => ApplicationProgress::Complete,
            4 => ApplicationProgress::Rejected,
            _ => return Err(StoreError::Corrupt("local admission progress")),
        };
        let authorization = row
            .try_get::<Option<Vec<u8>>, _>("authorization")?
            .map(|bytes| {
                bytes
                    .try_into()
                    .map(Hash::from_bytes)
                    .map_err(|_| StoreError::Corrupt("local admission authorization"))
            })
            .transpose()?;
        Ok(Some((
            AdmissionAttempt {
                request_id: request,
                progress,
            },
            authorization,
        )))
    }

    pub(crate) async fn save_local_admission_progress(
        &self,
        request_id: [u8; 16],
        progress: ApplicationProgress,
    ) -> Result<(), StoreError> {
        let result = sqlx::query("INSERT INTO local_admission_attempt (singleton, request_id, progress) VALUES (1, ?, ?)
            ON CONFLICT(singleton) DO UPDATE SET request_id = excluded.request_id, progress = excluded.progress,
                authorization = CASE WHEN request_id = excluded.request_id THEN authorization ELSE NULL END
            WHERE request_id != excluded.request_id OR progress < excluded.progress")
            .bind(request_id.as_slice())
            .bind(progress as i64)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() != 0 {
            self.changes.publish();
        }
        Ok(())
    }

    pub(crate) async fn save_local_admission_authorization(
        &self,
        request_id: [u8; 16],
        authorization: Hash,
    ) -> Result<(), StoreError> {
        sqlx::query("UPDATE local_admission_attempt SET authorization = ? WHERE request_id = ?")
            .bind(authorization.as_bytes().as_slice())
            .bind(request_id.as_slice())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub(crate) async fn pending_admission_receipts(
        &self,
    ) -> Result<
        Vec<(
            crate::admission_progress::AdmissionReceipt,
            iroh::EndpointAddr,
        )>,
        StoreError,
    > {
        let rows: Vec<(Vec<u8>, String)> = sqlx::query_as(
            "SELECT receipt, contact FROM pending_admission_receipts ORDER BY rowid",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|(bytes, contact)| {
                let receipt = crate::admission_progress::AdmissionReceipt::decode(
                    bytes
                        .try_into()
                        .map_err(|_| StoreError::Corrupt("admission receipt"))?,
                );
                let contact = serde_json::from_str(&contact)
                    .map_err(|_| StoreError::Corrupt("admission receipt contact"))?;
                Ok((receipt, contact))
            })
            .collect()
    }

    pub(crate) async fn settle_admission_receipt(
        &self,
        receipt: crate::admission_progress::AdmissionReceipt,
    ) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM pending_admission_receipts WHERE receipt = ?")
            .bind(receipt.encode().as_slice())
            .execute(&self.pool)
            .await?;
        if self
            .admission_history_checkpoint()
            .await?
            .is_some_and(|saved| {
                saved.binding.request == receipt.request_id
                    && saved.authorization == receipt.authorization
                    && saved.binding.anchor.genesis == receipt.genesis
            })
        {
            self.clear_admission_history().await?;
        }
        Ok(())
    }

    pub(crate) async fn admission_requests(&self) -> Result<Vec<AdmissionRequest>, StoreError> {
        let rows =
            sqlx::query("SELECT applicant, request_id FROM admission_deliveries ORDER BY rowid")
                .fetch_all(&self.pool)
                .await?;
        rows.iter().map(decode_admission_request).collect()
    }

    pub(crate) async fn admission_rejected(
        &self,
        request: &AdmissionRequest,
    ) -> Result<bool, StoreError> {
        Ok(
            sqlx::query(
                "SELECT 1 FROM admission_rejections WHERE applicant = ? AND request_id = ?",
            )
            .bind(request.applicant.as_bytes().as_slice())
            .bind(request.request_id.as_slice())
            .fetch_optional(&self.pool)
            .await?
            .is_some(),
        )
    }

    pub(crate) async fn save_admission_rejection(
        &self,
        request: &AdmissionRequest,
    ) -> Result<(), StoreError> {
        let result = sqlx::query(
            "INSERT OR IGNORE INTO admission_rejections (applicant, request_id) VALUES (?, ?)",
        )
        .bind(request.applicant.as_bytes().as_slice())
        .bind(request.request_id.as_slice())
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 0 {
            self.changes.publish();
        }
        Ok(())
    }

    pub(crate) async fn approved_admission(
        &self,
        request: &AdmissionRequest,
    ) -> Result<Option<ApprovedAdmission>, StoreError> {
        let row = sqlx::query("SELECT authorization, phase FROM admission_deliveries WHERE applicant = ? AND request_id = ?")
            .bind(request.applicant.as_bytes().as_slice()).bind(request.request_id.as_slice())
            .fetch_optional(&self.pool).await?;
        let Some(row) = row else { return Ok(None) };
        let authorization: Option<Vec<u8>> = row.try_get("authorization")?;
        let Some(authorization) = authorization else {
            return Ok(None);
        };
        let phase: i64 = row.try_get("phase")?;
        let stage = match phase {
            1 => AdmissionStage::Approved,
            2 => AdmissionStage::PendingMaterials,
            3 => AdmissionStage::Complete,
            _ => return Err(StoreError::Corrupt("admission phase")),
        };
        Ok(Some(ApprovedAdmission {
            request: request.clone(),
            authorization: Hash::from_bytes(
                authorization
                    .try_into()
                    .map_err(|_| StoreError::Corrupt("admission authorization"))?,
            ),
            stage,
        }))
    }

    /// The signed Add and its reusable delivery result share one durable
    /// transaction. After a crash, retry applies this exact record instead of
    /// producing another Add. This is the shared local persistence boundary;
    /// a request must already have been authenticated and recorded. Callers
    /// must recover/apply and verify the current grant before reporting an
    /// approved result or sending material; this commit alone is not admission.
    pub async fn save_admission_approval(
        &self,
        request: &AdmissionRequest,
        record: &ManagementRecord,
    ) -> Result<(), StoreError> {
        let header = decode_complete(record.header(), record.body(), self.anchor)?;
        let mut tx = self.pool.begin().await?;
        let changed = sqlx::query("UPDATE admission_deliveries SET authorization = ?, phase = 1 WHERE applicant = ? AND request_id = ? AND authorization IS NULL AND NOT EXISTS (SELECT 1 FROM admission_rejections r WHERE r.applicant = admission_deliveries.applicant AND r.request_id = admission_deliveries.request_id)")
            .bind(header.hash().as_bytes().as_slice())
            .bind(request.applicant.as_bytes().as_slice()).bind(request.request_id.as_slice())
            .execute(&mut *tx).await?;
        if changed.rows_affected() != 1 {
            return Err(StoreError::Corrupt("request is absent or already approved"));
        }
        sqlx::query("INSERT OR IGNORE INTO management_records (id, header, body) VALUES (?, ?, ?)")
            .bind(header.hash().as_bytes().as_slice())
            .bind(record.header())
            .bind(record.body())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.changes.publish();
        Ok(())
    }

    pub(crate) async fn advance_admission_delivery(
        &self,
        approved: &ApprovedAdmission,
        stage: AdmissionStage,
    ) -> Result<(), StoreError> {
        let phase = match stage {
            AdmissionStage::Approved => 1,
            AdmissionStage::PendingMaterials => 2,
            AdmissionStage::Complete => 3,
            AdmissionStage::AwaitingApproval => {
                return Err(StoreError::Corrupt("cannot undo admission approval"));
            }
        };
        let changed = sqlx::query("UPDATE admission_deliveries SET phase = ? WHERE applicant = ? AND request_id = ? AND authorization = ? AND phase < ?")
            .bind(phase).bind(approved.request.applicant.as_bytes().as_slice())
            .bind(approved.request.request_id.as_slice()).bind(approved.authorization.as_bytes().as_slice())
            .bind(phase)
            .execute(&self.pool).await?;
        if changed.rows_affected() == 0
            && self
                .approved_admission(&approved.request)
                .await?
                .is_none_or(|current| current.authorization != approved.authorization)
        {
            return Err(StoreError::Corrupt("delivery result changed"));
        }
        if changed.rows_affected() != 0 && stage == AdmissionStage::Complete {
            self.changes.publish();
        }
        Ok(())
    }

    /// The returned hash means the complete record is durable, not yet applied.
    pub async fn ingest(&self, record: &ManagementRecord) -> Result<Hash, StoreError> {
        let header = record.verify_complete(self.anchor)?;
        let id = header.hash();
        let mut tx = self.pool.begin().await?;
        let inserted = Self::insert_record(&mut tx, id, record.header(), record.body()).await?;
        #[cfg(feature = "replica-test-hooks")]
        if inserted {
            self.observe(RecordCheckpoint::BeforeStorageCommit);
        }
        tx.commit().await?;
        #[cfg(feature = "replica-test-hooks")]
        if inserted {
            self.observe(RecordCheckpoint::AfterStorageCommit);
        }
        #[cfg(not(feature = "replica-test-hooks"))]
        let _ = inserted;
        Ok(id)
    }

    /// Apply complete ordered history as one durable view update. A
    /// departure's prerequisites cannot make a grant visible on their own.
    pub(crate) async fn apply_batch(
        &mut self,
        records: &crate::service_protocol::ManagementRecords,
    ) -> Result<(), StoreError> {
        self.finish_pending_commit().await?;
        let mut next = self.auth.clone();
        let mut additions = Vec::new();
        for (header, body) in records {
            if !next.contains_operation(Hash::digest(header)) {
                let id = next.accept_record(header, body.as_deref())?;
                additions.push((id, header, body));
            }
        }
        if additions.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;
        let mut next_count = self.applied_count;
        for (id, header, body) in additions {
            Self::insert_record(&mut tx, id, header, body.as_deref()).await?;
            next_count += 1;
            Self::mark_record_applied(&mut tx, id.as_bytes(), next_count).await?;
        }
        self.save_view(&mut tx, next_count, &next).await?;
        self.commit_view(tx, next, next_count).await
    }

    /// The history receiver already verified these exact originals in order.
    /// Reuse that view only when it extends this store's current complete prefix.
    pub(crate) async fn apply_verified_history(
        &mut self,
        records: &crate::service_protocol::ManagementRecords,
        verified: ManagementAuth,
    ) -> Result<(), StoreError> {
        self.finish_pending_commit().await?;
        if verified.headers().len() != records.len()
            || verified
                .headers()
                .iter()
                .zip(records)
                .any(|(header, record)| header != &record.0)
        {
            return Err(StoreError::Corrupt("verified management records differ"));
        }
        if !verified.headers().starts_with(self.auth.headers()) {
            return self.apply_batch(records).await;
        }
        let existing = self.auth.headers().len();
        if existing == records.len() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;
        let mut count = self.applied_count;
        for (header, body) in records.iter().skip(existing) {
            let id = Hash::digest(header);
            Self::insert_record(&mut tx, id, header, body.as_deref()).await?;
            count += 1;
            Self::mark_record_applied(&mut tx, id.as_bytes(), count).await?;
        }
        self.save_view(&mut tx, count, &verified).await?;
        self.commit_view(tx, verified, count).await
    }

    /// Once COMMIT is handed to SQLite, cancelling its waiter cannot roll it
    /// back reliably. Keep commitment and live authority publication under one
    /// owner, and retain the result for this instance even when nobody awaits it.
    async fn commit_view(
        &mut self,
        tx: sqlx::Transaction<'static, sqlx::Sqlite>,
        auth: ManagementAuth,
        applied_count: i64,
    ) -> Result<(), StoreError> {
        let committed = Arc::new(OnceLock::new());
        let result = committed.clone();
        let gate = self.authorization_gate.clone();
        let publication = self.publication.clone();
        let changes = self.changes.clone();
        #[cfg(feature = "replica-test-hooks")]
        let observer = self.checkpoint_observer;
        let task = tokio::spawn(async move {
            tx.commit().await?;
            #[cfg(feature = "replica-test-hooks")]
            if let Some(observer) = observer {
                observer(RecordCheckpoint::AfterGroupsCommit);
            }
            let _publication = publication.lock().expect("management publication lock");
            if let Some(gate) = gate {
                Self::update_gate(&auth, &gate);
            }
            let inserted = result.set(CommittedView {
                auth,
                applied_count,
            });
            assert!(inserted.is_ok(), "management view published once");
            changes.publish();
            Ok(())
        });
        self.pending_commit = Some(PendingCommit { committed, task });
        self.finish_pending_commit().await
    }

    async fn finish_pending_commit(&mut self) -> Result<(), StoreError> {
        let Some(pending) = &mut self.pending_commit else {
            return Ok(());
        };
        // Borrow the handle until it completes: cancelling this wait must leave
        // the handle and its independently readable committed view in the store.
        let result = (&mut pending.task).await;
        let pending = self
            .pending_commit
            .take()
            .expect("pending management commit");
        if let Ok(committed) = Arc::try_unwrap(pending.committed) {
            if let Some(view) = committed.into_inner() {
                self.auth = view.auth;
                self.applied_count = view.applied_count;
            }
        } else {
            unreachable!("completed commit task has released its view");
        }
        result.map_err(|_| StoreError::Corrupt("management commit task failed"))?
    }

    pub(crate) fn attach_authorization_gate(&mut self, gate: crate::admission_gate::AdmissionGate) {
        self.authorization_gate = Some(gate);
        self.publish_authorization();
    }

    fn update_gate(auth: &ManagementAuth, gate: &crate::admission_gate::AdmissionGate) {
        gate.update_authorization(
            auth.nodes().into_iter().map(|(id, _)| *id.as_bytes()),
            auth.removed_nodes().into_iter().map(|id| *id.as_bytes()),
        );
    }

    pub(crate) fn publish_authorization(&self) {
        // Serialize explicit refresh with the commit task so an older snapshot
        // cannot overwrite the authorization published by a cancelled commit.
        let _publication = self
            .publication
            .lock()
            .expect("management publication lock");
        if let Some(gate) = &self.authorization_gate {
            Self::update_gate(self.authorization(), gate);
        }
    }

    async fn insert_record(
        connection: &mut sqlx::SqliteConnection,
        id: Hash,
        header: &[u8],
        body: Option<&[u8]>,
    ) -> Result<bool, StoreError> {
        let inserted = sqlx::query(
            "INSERT OR IGNORE INTO management_records (id, header, body) VALUES (?, ?, ?)",
        )
        .bind(id.as_bytes().as_slice())
        .bind(header)
        .bind(body)
        .execute(connection)
        .await?;
        Ok(inserted.rows_affected() != 0)
    }

    async fn mark_record_applied(
        connection: &mut sqlx::SqliteConnection,
        id: &[u8],
        order: i64,
    ) -> Result<(), StoreError> {
        let updated = sqlx::query(
            "UPDATE management_records SET applied_order = ?
             WHERE id = ? AND applied_order IS NULL AND rejected_reason IS NULL",
        )
        .bind(order)
        .bind(id)
        .execute(connection)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(StoreError::Corrupt("record changed while applying"));
        }
        Ok(())
    }

    async fn save_view(
        &self,
        connection: &mut sqlx::SqliteConnection,
        count: i64,
        auth: &ManagementAuth,
    ) -> Result<(), StoreError> {
        let updated = sqlx::query(
            "UPDATE management_view SET applied_count = ?, state = ?
             WHERE singleton = 1 AND applied_count = ?",
        )
        .bind(count)
        .bind(auth.member_view_bytes()?)
        .bind(self.applied_count)
        .execute(connection)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(StoreError::Corrupt("view changed while applying"));
        }
        Ok(())
    }

    /// Apply every ready record. A dependency or same-log predecessor can arrive later.
    /// Only committed views reach the live gate; replay remains side-effect free.
    pub async fn apply_ready(&mut self) -> Result<usize, StoreError> {
        self.apply_ready_inner(|_| false).await
    }

    async fn apply_ready_inner(
        &mut self,
        mut interrupted: impl FnMut(Checkpoint) -> bool,
    ) -> Result<usize, StoreError> {
        self.finish_pending_commit().await?;
        let mut applied = 0;
        loop {
            let rows = sqlx::query(
                "SELECT id, header, body FROM management_records
                 WHERE applied_order IS NULL AND rejected_reason IS NULL ORDER BY arrival",
            )
            .fetch_all(&self.pool)
            .await?;
            let mut ready = None;
            for row in rows {
                let header_bytes: Vec<u8> = row.try_get("header")?;
                let body: Option<Vec<u8>> = row.try_get("body")?;
                let header = decode_complete(&header_bytes, body.as_deref(), self.anchor)?;
                let requirements = header
                    .extensions
                    .group
                    .dependencies
                    .iter()
                    .chain(header.backlink.iter());
                if requirements
                    .into_iter()
                    .all(|id| self.auth.contains_operation(*id))
                {
                    let id: Vec<u8> = row.try_get("id")?;
                    ready = Some((id, header_bytes, header));
                    break;
                }
            }
            let Some((id, bytes, header)) = ready else {
                break;
            };
            #[cfg(feature = "replica-test-hooks")]
            self.observe(RecordCheckpoint::AfterOrdererSelect);
            if interrupted(Checkpoint::AfterOrdererSelect) {
                return Err(StoreError::Interrupted);
            }
            let mut next = self.auth.clone();
            if let Err(error) = next.accept_decoded_record(&bytes, header) {
                // A signed but unauthorized record cannot become authorized by
                // records outside its causal past. Quarantine it durably so it
                // cannot poison every subsequent drain or process restart.
                if matches!(error, AuthError::Group(GroupCrdtError::Inner(_)))
                    || !matches!(
                        error,
                        AuthError::Group(_)
                            | AuthError::PolicyRejected(_)
                            | AuthError::InvalidLog
                            | AuthError::InvalidCausality
                    )
                {
                    return Err(StoreError::Auth(error));
                }
                sqlx::query(
                    "UPDATE management_records SET rejected_reason = ?
                     WHERE id = ? AND applied_order IS NULL AND rejected_reason IS NULL",
                )
                .bind(error.to_string())
                .bind(&id)
                .execute(&self.pool)
                .await?;
                continue;
            }
            let next_count = self.applied_count + 1;
            let mut tx = self.pool.begin().await?;
            Self::mark_record_applied(&mut tx, &id, next_count).await?;
            self.save_view(&mut tx, next_count, &next).await?;
            if interrupted(Checkpoint::BeforeGroupsCommit) {
                return Err(StoreError::Interrupted);
            }
            #[cfg(feature = "replica-test-hooks")]
            self.observe(RecordCheckpoint::BeforeGroupsCommit);
            self.commit_view(tx, next, next_count).await?;
            if interrupted(Checkpoint::AfterGroupsCommit) {
                return Err(StoreError::Interrupted);
            }
            applied += 1;
        }
        Ok(applied)
    }

    pub fn nodes(&self) -> Vec<(VerifyingKey, Access)> {
        self.authorization().nodes()
    }

    pub(crate) fn authorization(&self) -> &ManagementAuth {
        self.authorization_snapshot().0
    }

    pub(crate) fn authorization_snapshot(&self) -> (&ManagementAuth, i64) {
        self.pending_commit
            .as_ref()
            .and_then(|pending| pending.committed.get())
            .map_or((&self.auth, self.applied_count), |view| {
                (&view.auth, view.applied_count)
            })
    }

    pub fn known_removed(&self, target: VerifyingKey) -> bool {
        self.authorization().known_removed(target)
    }

    pub fn membership_authorization_valid(
        &self,
        authorization: Hash,
        target: VerifyingKey,
    ) -> bool {
        self.authorization()
            .membership_authorization_valid(authorization, target)
    }

    pub(crate) fn administrator_authorization_origin(&self, target: VerifyingKey) -> Option<Hash> {
        self.authorization()
            .administrator_authorization_origin(target)
    }

    pub(crate) fn administrator_authorization_valid(
        &self,
        origin: Hash,
        target: VerifyingKey,
    ) -> bool {
        self.authorization()
            .administrator_authorization_valid(origin, target)
    }

    pub fn removed_nodes(&self) -> Vec<VerifyingKey> {
        self.authorization().removed_nodes()
    }

    pub fn applied_count(&self) -> i64 {
        self.authorization_snapshot().1
    }

    pub fn anchor(&self) -> NetworkAnchor {
        self.anchor
    }

    pub fn tips(&self) -> Vec<Hash> {
        self.authorization().tips()
    }

    /// Preflight local authoring without persisting a rejected action. Actual
    /// application still runs the identical verifier in its durable transaction.
    pub(crate) fn validate_local_record(
        &self,
        record: &ManagementRecord,
    ) -> Result<Hash, StoreError> {
        Ok(self
            .authorization()
            .clone()
            .accept_record(record.header(), record.body())?)
    }

    pub fn has_author_log_id(&self, author: VerifyingKey, log_id: [u8; 16]) -> bool {
        self.authorization().has_author_log_id(author, log_id)
    }

    /// Only committed, authorized records are supplied as control history.
    pub async fn get_record(&self, id: Hash) -> Result<Option<ManagementRecord>, StoreError> {
        let row = sqlx::query(
            "SELECT header, body FROM management_records
             WHERE id = ? AND applied_order IS NOT NULL",
        )
        .bind(id.as_bytes().as_slice())
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| self.applied_record(id, row.try_get("header")?, row.try_get("body")?))
            .transpose()
    }

    fn applied_record(
        &self,
        id: Hash,
        header: Vec<u8>,
        body: Option<Vec<u8>>,
    ) -> Result<ManagementRecord, StoreError> {
        let record = ManagementRecord::from_parts(header, body);
        // SQLite can expose a just-committed row before its task publishes the
        // runtime view. Keep the ordinary decoding path for that short window.
        match self.authorization().header_by_hash(id) {
            Some(verified) => record
                .with_verified_header(verified)
                .ok_or(StoreError::Corrupt(
                    "applied header differs from verified view",
                )),
            None => Ok(record),
        }
    }

    /// Complete locally applied history, in the order accepted by this store.
    pub async fn applied_records(&self) -> Result<Vec<ManagementRecord>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, header, body FROM management_records
             WHERE applied_order IS NOT NULL ORDER BY applied_order",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let id: Vec<u8> = row.try_get("id")?;
                let id = Hash::from_bytes(
                    id.try_into()
                        .map_err(|_| StoreError::Corrupt("applied record id"))?,
                );
                self.applied_record(id, row.try_get("header")?, row.try_get("body")?)
            })
            .collect()
    }

    pub(crate) async fn unapplied_records(
        &self,
    ) -> Result<crate::service_protocol::ManagementRecords, StoreError> {
        let rows=sqlx::query("SELECT header,body FROM management_records WHERE applied_order IS NULL ORDER BY arrival").fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|row| Ok((row.try_get("header")?, row.try_get("body")?)))
            .collect()
    }

    pub(crate) async fn checkpoint_backup(&self) -> Result<(), StoreError> {
        let checkpoint = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .fetch_one(&self.pool)
            .await?;
        let busy: i64 = checkpoint.try_get(0)?;
        if busy != 0 {
            return Err(StoreError::Corrupt("backup checkpoint is busy"));
        }
        Ok(())
    }

    pub async fn close(mut self) {
        let _ = self.finish_pending_commit().await;
        let _staging = self.staged_admission.lock().await;
        crate::sqlite_shutdown::close(&self.pool).await;
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Checkpoint {
    AfterOrdererSelect,
    BeforeGroupsCommit,
    AfterGroupsCommit,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::management_auth::ManagementExtension;
    use p2panda_auth::group::{GroupAction, GroupMember};
    use p2panda_auth::{Access, GroupsExtensionArgs};
    use p2panda_core::{Header, SigningKey};

    #[tokio::test]
    async fn applied_projection_reuses_only_verified_headers_and_preserves_original_bytes() {
        let root = SigningKey::generate();
        let admin = SigningKey::generate();
        let (anchor, original) = genesis(&root, &admin);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("management.sqlite");
        let mut store = ManagementStore::open(&path, anchor).await.unwrap();
        let id = store.ingest(&original).await.unwrap();
        assert!(store.get_record(id).await.unwrap().is_none());
        assert!(store.applied_records().await.unwrap().is_empty());
        assert!(store.authorization().header_by_hash(id).is_none());
        assert_eq!(store.apply_ready().await.unwrap(), 1);

        let projected = store.get_record(id).await.unwrap().unwrap();
        assert_eq!(projected, original);
        let verified = projected.verified_header().unwrap();
        assert_eq!(verified.hash(), id);
        assert_eq!(verified.encode(), original.header());
        let all = store.applied_records().await.unwrap();
        assert_eq!(all, vec![original.clone()]);
        assert!(Arc::ptr_eq(verified, all[0].verified_header().unwrap()));
        let cloned = store.authorization().clone();
        assert!(Arc::ptr_eq(verified, &cloned.header_by_hash(id).unwrap()));
        let (_, unrelated) = genesis(&root, &SigningKey::generate());
        assert!(unrelated.with_verified_header(verified.clone()).is_none());
        // Even a cached local projection must cross the ordinary verifier on
        // ingest; its cache cannot authenticate changed network input.
        let mut tampered = projected.clone();
        tampered.header[10] ^= 1;
        assert!(store.ingest(&tampered).await.is_err());
        assert_eq!(store.applied_count(), 1);

        let outsider = SigningKey::generate();
        let unauthorized = Header::<ManagementExtension>::builder().build(
            &outsider,
            ManagementExtension {
                version: 1,
                log_id: rand::random(),
                group: GroupsExtensionArgs {
                    group_id: anchor.root,
                    action: GroupAction::Add {
                        member: GroupMember::Individual(outsider.verifying_key()),
                        access: Access::read(),
                    },
                    dependencies: vec![id],
                },
            },
        );
        let rejected = store
            .ingest(&ManagementRecord::new(unauthorized.encode(), None))
            .await
            .unwrap();
        assert_eq!(store.apply_ready().await.unwrap(), 0);
        assert!(store.authorization().header_by_hash(rejected).is_none());
        assert!(store.get_record(rejected).await.unwrap().is_none());
        store.close().await;

        let recovered = ManagementStore::open(&path, anchor).await.unwrap();
        let projected = recovered.get_record(id).await.unwrap().unwrap();
        assert_eq!(projected, original);
        assert_eq!(projected.verified_header().unwrap().hash(), id);
        assert!(recovered.authorization().header_by_hash(rejected).is_none());
        recovered.close().await;
    }

    /// Pause the real SQLx worker inside SQLite COMMIT. The async caller is
    /// cancelled while that worker owns the transaction, then the same node
    /// must observe its committed authority and continue writing.
    #[tokio::test]
    async fn cancelled_commit_publishes_authority_on_the_same_node() {
        use crate::native_network::NativeNetwork;
        use std::sync::Arc;
        use tokio::sync::Notify;

        for batch in [false, true] {
            for remove in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let mut node = NativeNetwork::create(directory.path().join("node"))
                    .await
                    .unwrap();
                let member = SigningKey::generate();
                let peer = iroh_base::SecretKey::from_bytes(member.as_bytes()).public();
                let record = |node: &NativeNetwork, action| {
                    let header = Header::<ManagementExtension>::builder()
                        .build(
                            &node.node_key,
                            ManagementExtension {
                                version: 1,
                                log_id: rand::random(),
                                group: GroupsExtensionArgs {
                                    group_id: node.anchor.root,
                                    action,
                                    dependencies: node.store.tips(),
                                },
                            },
                        )
                        .encode();
                    ManagementRecord::new(header, None)
                };
                let add = || GroupAction::Add {
                    member: GroupMember::Individual(member.verifying_key()),
                    access: Access::read(),
                };
                if remove {
                    let first = record(&node, add());
                    node.ingest_management_record(first.header(), first.body())
                        .await
                        .unwrap();
                }
                let action = if remove {
                    GroupAction::Remove {
                        member: GroupMember::Individual(member.verifying_key()),
                    }
                } else {
                    add()
                };
                let next = record(&node, action);
                let before = node.status().applied_count;
                if !batch {
                    node.store.ingest(&next).await.unwrap();
                }

                let entered = Arc::new(Notify::new());
                let (release, wait) = std::sync::mpsc::channel();
                {
                    let mut connection = node.store.pool.acquire().await.unwrap();
                    let entered = entered.clone();
                    connection
                        .lock_handle()
                        .await
                        .unwrap()
                        .set_commit_hook(move || {
                            entered.notify_one();
                            let _ = wait.recv();
                            // SQLx uses true to allow COMMIT (the inverse of
                            // SQLite's raw callback return value).
                            true
                        });
                }
                let mut changes = node.store.changes.subscribe();
                let records = vec![(next.header().to_vec(), next.body().map(<[u8]>::to_vec))];
                let mut applying = Box::pin(async {
                    if batch {
                        node.store.apply_batch(&records).await
                    } else {
                        node.store.apply_ready().await.map(|_| ())
                    }
                });
                tokio::select! {
                    _ = entered.notified() => {},
                    result = &mut applying => panic!("COMMIT was not paused: {result:?}"),
                }
                drop(applying);
                assert_eq!(node.status().applied_count, before);
                assert_eq!(node.gate.member_contact_allowed(peer), remove);
                release.send(()).unwrap();
                {
                    let mut connection = node.store.pool.acquire().await.unwrap();
                    let durable_count: i64 = sqlx::query_scalar(
                        "SELECT applied_count FROM management_view WHERE singleton = 1",
                    )
                    .fetch_one(&mut *connection)
                    .await
                    .unwrap();
                    connection.lock_handle().await.unwrap().remove_commit_hook();
                    assert_eq!(
                        durable_count,
                        before + 1,
                        "SQLite COMMIT must have succeeded"
                    );
                }
                tokio::time::timeout(std::time::Duration::from_secs(2), changes.changed())
                    .await
                    .expect("cancelled COMMIT must publish its live view")
                    .unwrap();
                assert_eq!(node.status().applied_count, before + 1);
                assert_eq!(node.gate.member_contact_allowed(peer), !remove);
                assert_eq!(node.store.known_removed(member.verifying_key()), remove);
                assert_eq!(node.store.apply_ready().await.unwrap(), 0);
                let following = record(
                    &node,
                    if remove {
                        add()
                    } else {
                        GroupAction::Remove {
                            member: GroupMember::Individual(member.verifying_key()),
                        }
                    },
                );
                node.ingest_management_record(following.header(), following.body())
                    .await
                    .unwrap();
                assert_eq!(node.status().applied_count, before + 2);
                assert_eq!(node.gate.member_contact_allowed(peer), remove);
                node.store.close().await;
            }
        }
    }

    fn genesis(root: &SigningKey, admin: &SigningKey) -> (NetworkAnchor, ManagementRecord) {
        let header = Header::<ManagementExtension>::builder()
            .build(
                root,
                ManagementExtension {
                    version: 1,
                    log_id: [1; 16],
                    group: GroupsExtensionArgs {
                        group_id: root.verifying_key(),
                        action: GroupAction::Create {
                            initial_members: vec![
                                (
                                    GroupMember::Individual(root.verifying_key()),
                                    Access::manage(),
                                ),
                                (
                                    GroupMember::Individual(admin.verifying_key()),
                                    Access::manage(),
                                ),
                            ],
                        },
                        dependencies: vec![],
                    },
                },
            )
            .encode();
        let anchor = NetworkAnchor {
            root: root.verifying_key(),
            genesis: Hash::digest(&header),
        };
        (anchor, ManagementRecord::new(header, None))
    }

    #[tokio::test]
    async fn each_commit_boundary_recovers_from_disk_without_double_apply() {
        for checkpoint in [
            Checkpoint::AfterOrdererSelect,
            Checkpoint::BeforeGroupsCommit,
            Checkpoint::AfterGroupsCommit,
        ] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("management.sqlite");
            let root = SigningKey::generate();
            let admin = SigningKey::generate();
            let (anchor, record) = genesis(&root, &admin);

            let mut store = ManagementStore::open(&path, anchor).await.unwrap();
            let id = store.ingest(&record).await.unwrap();
            assert!(matches!(
                store.apply_ready_inner(|stage| stage == checkpoint).await,
                Err(StoreError::Interrupted)
            ));
            store.close().await;

            let mut recovered = ManagementStore::open(&path, anchor).await.unwrap();
            let already_applied = checkpoint == Checkpoint::AfterGroupsCommit;
            assert_eq!(recovered.applied_count(), i64::from(already_applied));
            assert_eq!(
                recovered.apply_ready().await.unwrap(),
                usize::from(!already_applied)
            );
            assert_eq!(recovered.apply_ready().await.unwrap(), 0);
            assert_eq!(recovered.applied_count(), 1);
            assert_eq!(recovered.get_record(id).await.unwrap(), Some(record));
            assert!(recovered.nodes().iter().any(|(id, access)| {
                *id == admin.verifying_key() && *access == Access::manage()
            }));
            recovered.close().await;
        }
    }
}
