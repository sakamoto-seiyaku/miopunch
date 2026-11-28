//! Durable admission prefixes, separate from applied membership records.
use sqlx::{Row, SqlitePool};

use crate::{
    admission_history_wire::{HistoryBinding, HistoryCheckpoint},
    management_store::{ManagementStore, StoreError},
};

pub(crate) async fn initialize(pool: &SqlitePool) -> Result<(), StoreError> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS admission_history_stage (
        singleton INTEGER PRIMARY KEY CHECK(singleton=1), binding BLOB NOT NULL,
        authorization BLOB NOT NULL, cursor INTEGER NOT NULL CHECK(cursor>=0),
        complete INTEGER NOT NULL CHECK(complete IN (0,1)))",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS admission_history_originals (
        ordinal INTEGER PRIMARY KEY CHECK(ordinal>=0), header BLOB NOT NULL, body BLOB)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

impl ManagementStore {
    pub(crate) async fn admission_history_checkpoint(
        &self,
    ) -> Result<Option<HistoryCheckpoint>, StoreError> {
        Ok(self.staged_admission.lock().await.clone())
    }

    pub(crate) async fn restore_admission_history(&self) -> Result<(), StoreError> {
        let mut cached = self.staged_admission.lock().await;
        let checkpoint = self.read_admission_history().await?;
        if let Some(checkpoint) = &checkpoint {
            checkpoint
                .prepare()
                .map_err(|_| StoreError::Corrupt("admission history verification"))?;
        }
        *cached = checkpoint;
        Ok(())
    }

    async fn read_admission_history(&self) -> Result<Option<HistoryCheckpoint>, StoreError> {
        let Some(meta) = sqlx::query("SELECT binding, authorization, cursor, complete FROM admission_history_stage WHERE singleton=1")
            .fetch_optional(&self.pool).await? else { return Ok(None); };
        let binding: Vec<u8> = meta.try_get("binding")?;
        let binding: HistoryBinding = p2panda_core::cbor::decode_cbor(binding.as_slice())?;
        let authorization: Vec<u8> = meta.try_get("authorization")?;
        let authorization = p2panda_core::Hash::from_bytes(
            authorization
                .try_into()
                .map_err(|_| StoreError::Corrupt("admission history authorization"))?,
        );
        let count: i64 = meta.try_get("cursor")?;
        let rows = sqlx::query(
            "SELECT ordinal, header, body FROM admission_history_originals ORDER BY ordinal",
        )
        .fetch_all(&self.pool)
        .await?;
        if count < 0 || rows.len() as u64 != count as u64 {
            return Err(StoreError::Corrupt("admission history cursor"));
        }
        let mut records = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            if row.try_get::<i64, _>("ordinal")? != index as i64 {
                return Err(StoreError::Corrupt("admission history order"));
            }
            records.push((row.try_get("header")?, row.try_get("body")?));
        }
        Ok(Some(HistoryCheckpoint::new(
            binding,
            authorization,
            records,
            meta.try_get("complete")?,
        )))
    }

    pub(crate) async fn save_admission_history(
        &self,
        checkpoint: &HistoryCheckpoint,
    ) -> Result<(), StoreError> {
        let checkpoint = checkpoint.clone();
        let pool = self.pool.clone();
        let mut cached = self.staged_admission.clone().lock_owned().await;
        tokio::spawn(async move {
        let binding = p2panda_core::cbor::encode_cbor(&checkpoint.binding)?;
        let mut tx = pool.begin().await?;
        let previous = sqlx::query(
            "SELECT binding, authorization, cursor FROM admission_history_stage WHERE singleton=1",
        )
        .fetch_optional(&mut *tx)
        .await?;
        let mut cursor = 0;
        if let Some(previous) = previous {
            let stored_binding: Vec<u8> = previous.try_get("binding")?;
            let authorization: Vec<u8> = previous.try_get("authorization")?;
            let count: i64 = previous.try_get("cursor")?;
            if stored_binding == binding
                && authorization.as_slice() == checkpoint.authorization.as_bytes()
                && count >= 0
                && count as u64 <= checkpoint.records.len() as u64
            {
                cursor = count as usize;
            }
        }
        if cursor == 0 {
            sqlx::query("DELETE FROM admission_history_originals")
                .execute(&mut *tx)
                .await?;
        }
        for (index, (header, body)) in checkpoint.records.iter().enumerate().skip(cursor) {
            sqlx::query(
                "INSERT INTO admission_history_originals(ordinal,header,body) VALUES(?,?,?)",
            )
            .bind(index as i64)
            .bind(header)
            .bind(body)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query("INSERT INTO admission_history_stage(singleton,binding,authorization,cursor,complete) VALUES(1,?,?,?,?)
            ON CONFLICT(singleton) DO UPDATE SET binding=excluded.binding,authorization=excluded.authorization,cursor=excluded.cursor,complete=excluded.complete")
            .bind(binding).bind(checkpoint.authorization.as_bytes().as_slice())
            .bind(checkpoint.records.len() as i64).bind(checkpoint.complete).execute(&mut *tx).await?;
        tx.commit().await?;
        *cached = Some(checkpoint);
        Ok(())
        }).await.map_err(|_| StoreError::Corrupt("admission history commit task failed"))?
    }

    pub(crate) async fn clear_admission_history(&self) -> Result<(), StoreError> {
        let pool = self.pool.clone();
        let mut cached = self.staged_admission.clone().lock_owned().await;
        tokio::spawn(async move {
            let mut tx = pool.begin().await?;
            sqlx::query("DELETE FROM admission_history_originals")
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM admission_history_stage")
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            *cached = None;
            Ok(())
        })
        .await
        .map_err(|_| StoreError::Corrupt("admission history cleanup task failed"))?
    }
}
