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

use crate::management_auth::{AuthError, ManagementAuth, NetworkAnchor, decode_complete};

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
    #[error("durable management state is inconsistent: {0}")]
    Corrupt(&'static str),
    #[error("simulated process interruption")]
    Interrupted,
}

/// One network's complete signed history and locally committed authority view.
/// The caller must supply an anchor obtained from a trusted source.
pub struct ManagementStore {
    pool: SqlitePool,
    anchor: NetworkAnchor,
    auth: ManagementAuth,
    applied_count: i64,
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
        })
    }

    /// The returned hash means the complete record is durable, not yet applied.
    pub async fn ingest(&self, record: &ManagementRecord) -> Result<Hash, StoreError> {
        let header = decode_complete(record.header(), record.body(), self.anchor)?;
        let id = header.hash();
        sqlx::query("INSERT OR IGNORE INTO management_records (id, header, body) VALUES (?, ?, ?)")
            .bind(id.as_bytes().as_slice())
            .bind(record.header())
            .bind(record.body())
            .execute(&self.pool)
            .await?;
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
            tx.commit().await?;
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

    pub fn applied_count(&self) -> i64 {
        self.applied_count
    }

    pub fn anchor(&self) -> NetworkAnchor {
        self.anchor
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
