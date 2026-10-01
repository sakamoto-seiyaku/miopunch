//! Small durable management-record pipeline for MIOPUNCH-24.
//!
//! Ingest commits only complete signed records. Applying a ready record and
//! saving the corresponding Groups state share one SQLite transaction. Recovery
//! replays the committed records through the same authority verifier.

use std::path::Path;

use p2panda_auth::Access;
use p2panda_auth::group::GroupCrdtError;
use p2panda_core::{Hash, VerifyingKey};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Row, SqlitePool};
use thiserror::Error;

use crate::admission_proof::{AdmissionScope, AdmissionSecret};
use crate::management_auth::{AuthError, ManagementAuth, NetworkAnchor, decode_complete};
use crate::manual_admission::{AdmissionRequest, AdmissionStage, ApprovedAdmission};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagementRecord {
    header: Vec<u8>,
    body: Option<Vec<u8>>,
}

impl ManagementRecord {
    pub fn new(header: Vec<u8>, body: Option<&[u8]>) -> Self {
        Self {
            header,
            body: body.map(|bytes| bytes.to_vec()),
        }
    }

    pub fn header(&self) -> &[u8] {
        &self.header
    }

    pub fn body(&self) -> Option<&[u8]> {
        self.body.as_deref()
    }
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
    #[cfg(feature = "replica-test-hooks")]
    checkpoint_observer: Option<fn(RecordCheckpoint)>,
}

/// Real durability boundaries available only to explicit fault-injection
/// builds. Observers can terminate the test process without altering storage.
#[cfg(feature = "replica-test-hooks")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordCheckpoint {
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
            .bind(empty.state_bytes()?)
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
        if !auth.matches_member_view_bytes(&state)? {
            return Err(StoreError::Corrupt(
                "saved Groups view differs from signed replay",
            ));
        }
        Ok(Self {
            pool,
            anchor,
            auth,
            applied_count,
            #[cfg(feature = "replica-test-hooks")]
            checkpoint_observer: None,
        })
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
        sqlx::query(
            "INSERT OR IGNORE INTO admission_deliveries (applicant, request_id) VALUES (?, ?)",
        )
        .bind(request.applicant.as_bytes().as_slice())
        .bind(request.request_id.as_slice())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub(crate) async fn admission_requests(&self) -> Result<Vec<AdmissionRequest>, StoreError> {
        let rows =
            sqlx::query("SELECT applicant, request_id FROM admission_deliveries ORDER BY rowid")
                .fetch_all(&self.pool)
                .await?;
        rows.into_iter()
            .map(|row| {
                let applicant: Vec<u8> = row.try_get("applicant")?;
                let request: Vec<u8> = row.try_get("request_id")?;
                Ok(AdmissionRequest {
                    applicant: VerifyingKey::from_bytes(
                        &applicant
                            .try_into()
                            .map_err(|_| StoreError::Corrupt("admission applicant"))?,
                    )
                    .map_err(|_| StoreError::Corrupt("admission applicant"))?,
                    request_id: request
                        .try_into()
                        .map_err(|_| StoreError::Corrupt("admission request id"))?,
                })
            })
            .collect()
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
        let changed = sqlx::query("UPDATE admission_deliveries SET authorization = ?, phase = 1 WHERE applicant = ? AND request_id = ? AND authorization IS NULL")
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
        let changed = sqlx::query("UPDATE admission_deliveries SET phase = max(phase, ?) WHERE applicant = ? AND request_id = ? AND authorization = ?")
            .bind(phase).bind(approved.request.applicant.as_bytes().as_slice())
            .bind(approved.request.request_id.as_slice()).bind(approved.authorization.as_bytes().as_slice())
            .execute(&self.pool).await?;
        if changed.rows_affected() != 1 {
            return Err(StoreError::Corrupt("delivery result changed"));
        }
        Ok(())
    }

    /// The returned hash means the complete record is durable, not yet applied.
    pub async fn ingest(&self, record: &ManagementRecord) -> Result<Hash, StoreError> {
        let header = decode_complete(record.header(), record.body(), self.anchor)?;
        let id = header.hash();
        let mut tx = self.pool.begin().await?;
        let inserted = sqlx::query(
            "INSERT OR IGNORE INTO management_records (id, header, body) VALUES (?, ?, ?)",
        )
        .bind(id.as_bytes().as_slice())
        .bind(record.header())
        .bind(record.body())
        .execute(&mut *tx)
        .await?;
        #[cfg(feature = "replica-test-hooks")]
        if inserted.rows_affected() != 0 {
            self.observe(RecordCheckpoint::BeforeStorageCommit);
        }
        tx.commit().await?;
        #[cfg(feature = "replica-test-hooks")]
        if inserted.rows_affected() != 0 {
            self.observe(RecordCheckpoint::AfterStorageCommit);
        }
        #[cfg(not(feature = "replica-test-hooks"))]
        let _ = inserted;
        Ok(id)
    }

    /// Apply every ready record. A dependency or same-log predecessor can arrive later.
    /// No external side effects are emitted while replaying or committing this view.
    pub async fn apply_ready(&mut self) -> Result<usize, StoreError> {
        self.apply_ready_inner(|_| false).await
    }

    async fn apply_ready_inner(
        &mut self,
        mut interrupted: impl FnMut(Checkpoint) -> bool,
    ) -> Result<usize, StoreError> {
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
                    ready = Some((id, header_bytes, body));
                    break;
                }
            }
            let Some((id, header, body)) = ready else {
                break;
            };
            #[cfg(feature = "replica-test-hooks")]
            self.observe(RecordCheckpoint::AfterOrdererSelect);
            if interrupted(Checkpoint::AfterOrdererSelect) {
                return Err(StoreError::Interrupted);
            }
            let mut next = self.auth.clone();
            if let Err(error) = next.accept_record(&header, body.as_deref()) {
                // A signed but unauthorized record cannot become authorized by
                // records outside its causal past. Quarantine it durably so it
                // cannot poison every subsequent drain or process restart.
                if matches!(error, AuthError::Group(GroupCrdtError::Inner(_)))
                    || !matches!(
                        error,
                        AuthError::Group(_) | AuthError::InvalidLog | AuthError::InvalidCausality
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
            let state = next.state_bytes()?;
            let mut tx = self.pool.begin().await?;
            let updated = sqlx::query(
                "UPDATE management_records SET applied_order = ?
                 WHERE id = ? AND applied_order IS NULL",
            )
            .bind(next_count)
            .bind(&id)
            .execute(&mut *tx)
            .await?;
            if updated.rows_affected() != 1 {
                return Err(StoreError::Corrupt("record changed while applying"));
            }
            let updated = sqlx::query(
                "UPDATE management_view SET applied_count = ?, state = ?
                 WHERE singleton = 1 AND applied_count = ?",
            )
            .bind(next_count)
            .bind(state)
            .bind(self.applied_count)
            .execute(&mut *tx)
            .await?;
            if updated.rows_affected() != 1 {
                return Err(StoreError::Corrupt("view changed while applying"));
            }
            if interrupted(Checkpoint::BeforeGroupsCommit) {
                return Err(StoreError::Interrupted);
            }
            #[cfg(feature = "replica-test-hooks")]
            self.observe(RecordCheckpoint::BeforeGroupsCommit);
            tx.commit().await?;
            #[cfg(feature = "replica-test-hooks")]
            self.observe(RecordCheckpoint::AfterGroupsCommit);
            if interrupted(Checkpoint::AfterGroupsCommit) {
                return Err(StoreError::Interrupted);
            }
            self.auth = next;
            self.applied_count = next_count;
            applied += 1;
        }
        Ok(applied)
    }

    pub fn nodes(&self) -> Vec<(VerifyingKey, Access)> {
        self.auth.nodes()
    }

    pub fn known_removed(&self, target: VerifyingKey) -> bool {
        self.auth.known_removed(target)
    }

    pub fn membership_authorization_valid(
        &self,
        authorization: Hash,
        target: VerifyingKey,
    ) -> bool {
        self.auth
            .membership_authorization_valid(authorization, target)
    }

    pub(crate) fn administrator_authorization_origin(&self, target: VerifyingKey) -> Option<Hash> {
        self.auth.administrator_authorization_origin(target)
    }

    pub(crate) fn administrator_authorization_valid(
        &self,
        origin: Hash,
        target: VerifyingKey,
    ) -> bool {
        self.auth.administrator_authorization_valid(origin, target)
    }

    pub fn removed_nodes(&self) -> Vec<VerifyingKey> {
        self.auth.removed_nodes()
    }

    pub fn applied_count(&self) -> i64 {
        self.applied_count
    }

    pub fn anchor(&self) -> NetworkAnchor {
        self.anchor
    }

    pub fn tips(&self) -> Vec<Hash> {
        self.auth.tips()
    }

    /// Preflight local authoring without persisting a rejected action. Actual
    /// application still runs the identical verifier in its durable transaction.
    pub(crate) fn validate_local_record(
        &self,
        record: &ManagementRecord,
    ) -> Result<Hash, StoreError> {
        Ok(self
            .auth
            .clone()
            .accept_record(record.header(), record.body())?)
    }

    pub fn has_author_log_id(&self, author: VerifyingKey, log_id: [u8; 16]) -> bool {
        self.auth.has_author_log_id(author, log_id)
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
        row.map(|row| {
            Ok(ManagementRecord {
                header: row.try_get("header")?,
                body: row.try_get("body")?,
            })
        })
        .transpose()
    }

    /// Complete locally applied history, in the order accepted by this store.
    pub async fn applied_records(&self) -> Result<Vec<ManagementRecord>, StoreError> {
        let rows = sqlx::query(
            "SELECT header, body FROM management_records
             WHERE applied_order IS NOT NULL ORDER BY applied_order",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ManagementRecord {
                    header: row.try_get("header")?,
                    body: row.try_get("body")?,
                })
            })
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

    pub async fn close(self) {
        self.pool.close().await;
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
