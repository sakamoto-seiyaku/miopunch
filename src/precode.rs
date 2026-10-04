//! Designated-node precode consumption shares durable admission delivery.
use std::time::{SystemTime, UNIX_EPOCH};

use iroh::endpoint::Connection;
use sqlx::Row;

use crate::management_store::{ManagementRecord, ManagementStore, StoreError};
use crate::manual_admission::{
    AdmissionRequest, AdmissionStage, ApprovedAdmission, ManualAdmissionError, ManualExchange,
    ManualInvitation,
};
use crate::native_network::NativeNetwork;

/// Trusted invitation material; no long-lived admission secret is included.
/// The opaque token is a bearer capability and is intentionally not Debug.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct Precode {
    invitation: ManualInvitation,
    token: [u8; 32],
}
impl Precode {
    pub fn invitation(&self) -> ManualInvitation {
        self.invitation
    }

    /// Pass these bearer-capability bytes only through a trusted channel.
    pub fn encode(&self) -> [u8; 128] {
        let mut bytes = [0; 128];
        bytes[..96].copy_from_slice(&self.invitation.encode());
        bytes[96..].copy_from_slice(&self.token);
        bytes
    }

    pub fn decode(bytes: &[u8; 128]) -> Result<Self, ManualAdmissionError> {
        Ok(Self {
            invitation: ManualInvitation::decode(&bytes[..96].try_into().unwrap())?,
            token: bytes[96..].try_into().unwrap(),
        })
    }
}

fn now() -> Result<i64, ManualAdmissionError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|time| time.as_secs().try_into().ok())
        .ok_or(ManualAdmissionError::Rejected)
}

/// Secret-free local consumption evidence. A restored or inconsistent ledger
/// cannot promise a remaining allowance, even when a saved count is readable.
pub struct PrecodeUsageStatus {
    pub saved_uses: Option<u64>,
    pub count_trusted: bool,
}

impl NativeNetwork {
    pub async fn issue_precode(
        &mut self,
        expires_unix_seconds: u64,
        max_uses: Option<u64>,
    ) -> Result<Precode, ManualAdmissionError> {
        self.recover_admission_state().await?;
        let invitation = self.manual_invitation()?;
        let expires: i64 = expires_unix_seconds
            .try_into()
            .map_err(|_| ManualAdmissionError::Rejected)?;
        let max_uses: Option<i64> = max_uses
            .map(i64::try_from)
            .transpose()
            .map_err(|_| ManualAdmissionError::Rejected)?;
        if expires <= now()? || max_uses == Some(0) {
            return Err(ManualAdmissionError::Rejected);
        }
        let origin = self
            .store
            .administrator_authorization_origin(self.node_key.verifying_key())
            .ok_or(ManualAdmissionError::Rejected)?;
        let code = Precode {
            invitation,
            token: rand::random(),
        };
        sqlx::query("INSERT INTO precodes(token, origin, expires, max_uses) VALUES(?,?,?,?)")
            .bind(code.token.as_slice())
            .bind(origin.as_bytes().as_slice())
            .bind(expires)
            .bind(max_uses)
            .execute(&self.store.pool)
            .await
            .map_err(StoreError::from)?;
        Ok(code)
    }

    fn own_precode(&self, code: &Precode) -> Result<(), ManualAdmissionError> {
        if code.invitation != self.manual_invitation()? {
            return Err(ManualAdmissionError::Rejected);
        }
        Ok(())
    }

    /// Only this designated node can actively revoke its own code. Granted
    /// membership and already committed delivery results remain independent.
    pub async fn revoke_precode(&self, code: &Precode) -> Result<(), ManualAdmissionError> {
        self.own_precode(code)?;
        let changed = sqlx::query("UPDATE precodes SET revoked=1 WHERE token=?")
            .bind(code.token.as_slice())
            .execute(&self.store.pool)
            .await
            .map_err(StoreError::from)?;
        if changed.rows_affected() != 1 {
            return Err(ManualAdmissionError::Rejected);
        }
        Ok(())
    }

    /// Saved consumption count, not a promise of remaining quota. A supported
    /// restore stops the code even when its restored saved count is zero.
    pub async fn precode_usage(&self, code: &Precode) -> Result<u64, ManualAdmissionError> {
        self.own_precode(code)?;
        let used: i64 = sqlx::query_scalar("SELECT used FROM precodes WHERE token=?")
            .bind(code.token.as_slice())
            .fetch_one(&self.store.pool)
            .await
            .map_err(StoreError::from)?;
        used.try_into().map_err(|_| ManualAdmissionError::Rejected)
    }

    pub async fn precode_usage_status(
        &self,
        code: &Precode,
    ) -> Result<PrecodeUsageStatus, ManualAdmissionError> {
        self.own_precode(code)?;
        let row = sqlx::query("SELECT used, stopped, used>=0 AND used=(SELECT count(*) FROM precode_redemptions r WHERE r.token=precodes.token) AND NOT EXISTS (SELECT 1 FROM precode_redemptions r LEFT JOIN admission_deliveries d ON d.applicant=r.applicant AND d.request_id=r.request_id WHERE r.token=precodes.token AND (d.authorization IS NULL OR d.phase<1)) AS consistent FROM precodes WHERE token=?")
            .bind(code.token.as_slice()).fetch_one(&self.store.pool).await.map_err(StoreError::from)?;
        let used: i64 = row.try_get("used").map_err(StoreError::from)?;
        let stopped: bool = row.try_get("stopped").map_err(StoreError::from)?;
        let consistent: bool = row.try_get("consistent").map_err(StoreError::from)?;
        Ok(PrecodeUsageStatus {
            saved_uses: used.try_into().ok(),
            count_trusted: !stopped && consistent,
        })
    }

    pub async fn precode_redemptions(
        &mut self,
        code: &Precode,
    ) -> Result<Vec<ApprovedAdmission>, ManualAdmissionError> {
        self.own_precode(code)?;
        self.recover_admission_state().await?;
        let rows = sqlx::query(
            "SELECT applicant, request_id FROM precode_redemptions WHERE token=? ORDER BY rowid",
        )
        .bind(code.token.as_slice())
        .fetch_all(&self.store.pool)
        .await
        .map_err(StoreError::from)?;
        let mut results = Vec::new();
        for row in rows {
            let applicant: Vec<u8> = row.try_get("applicant").map_err(StoreError::from)?;
            let request_id: Vec<u8> = row.try_get("request_id").map_err(StoreError::from)?;
            let request = AdmissionRequest {
                applicant: p2panda_core::VerifyingKey::from_bytes(
                    &applicant
                        .try_into()
                        .map_err(|_| ManualAdmissionError::Rejected)?,
                )
                .map_err(|_| ManualAdmissionError::Rejected)?,
                request_id: request_id
                    .try_into()
                    .map_err(|_| ManualAdmissionError::Rejected)?,
            };
            results.push(
                self.approved_admission(&request)
                    .await?
                    .ok_or(ManualAdmissionError::Rejected)?,
            );
        }
        Ok(results)
    }

    pub(crate) async fn redeem_precode(
        &mut self,
        token: [u8; 32],
        request: &AdmissionRequest,
    ) -> Result<ApprovedAdmission, ManualAdmissionError> {
        self.store.ensure_admissions_open().await?;
        if request.applicant == self.anchor.root {
            return Err(ManualAdmissionError::Rejected);
        }
        let exists: i64 = sqlx::query_scalar("SELECT count(*) FROM precode_redemptions WHERE token=? AND applicant=? AND request_id=?")
            .bind(token.as_slice()).bind(request.applicant.as_bytes().as_slice()).bind(request.request_id.as_slice())
            .fetch_one(&self.store.pool).await.map_err(StoreError::from)?;
        if exists == 0 {
            let row = sqlx::query("SELECT origin,expires,max_uses,used,stopped,revoked, used=(SELECT count(*) FROM precode_redemptions r WHERE r.token=precodes.token) AND NOT EXISTS (SELECT 1 FROM precode_redemptions r LEFT JOIN admission_deliveries d ON d.applicant=r.applicant AND d.request_id=r.request_id WHERE r.token=precodes.token AND (d.authorization IS NULL OR d.phase<1)) AS consistent FROM precodes WHERE token=?")
                .bind(token.as_slice()).fetch_optional(&self.store.pool).await.map_err(StoreError::from)?
                .ok_or(ManualAdmissionError::Rejected)?;
            let origin: Vec<u8> = row.try_get("origin").map_err(StoreError::from)?;
            let origin = p2panda_core::Hash::from_bytes(
                origin
                    .try_into()
                    .map_err(|_| ManualAdmissionError::Rejected)?,
            );
            let unsigned =
                |value: i64| u64::try_from(value).map_err(|_| ManualAdmissionError::Rejected);
            let policy = crate::precode_policy::RedemptionPolicy {
                expires: unsigned(row.try_get("expires").map_err(StoreError::from)?)?,
                max_uses: row
                    .try_get::<Option<i64>, _>("max_uses")
                    .map_err(StoreError::from)?
                    .map(unsigned)
                    .transpose()?,
                used: unsigned(row.try_get("used").map_err(StoreError::from)?)?,
                stopped: row.try_get("stopped").map_err(StoreError::from)?,
                revoked: row.try_get("revoked").map_err(StoreError::from)?,
                ledger_consistent: row.try_get("consistent").map_err(StoreError::from)?,
                issuer_authorized: self
                    .store
                    .administrator_authorization_valid(origin, self.node_key.verifying_key()),
            };
            if !policy.permits_new(now()? as u64) {
                return Err(ManualAdmissionError::Rejected);
            }
            let record = self.admission_record(request).await?;
            self.store
                .consume_precode(token, request, &record, now()?)
                .await?;
            self.store.apply_ready().await?;
            self.refresh_authorization();
        }
        let approved = self
            .store
            .approved_admission(request)
            .await?
            .ok_or(ManualAdmissionError::Rejected)?;
        self.valid_approval(&approved)?;
        Ok(approved)
    }

    pub async fn serve_precode_admission(
        &mut self,
        connection: &Connection,
    ) -> Result<AdmissionStage, ManualAdmissionError> {
        let result = tokio::time::timeout(
            crate::manual_admission::admission_deadline(connection)
                .saturating_duration_since(n0_future::time::Instant::now()),
            self.serve_admission_inner(connection, true),
        )
        .await
        .unwrap_or(Err(ManualAdmissionError::PathUnavailable));
        if result.is_err() {
            connection.close(0u32.into(), b"precode admission ended");
        }
        result
    }

    pub async fn request_precode_admission(
        &mut self,
        connection: &Connection,
        code: &Precode,
    ) -> Result<ManualExchange, ManualAdmissionError> {
        if self.join.map(|binding| binding.invitation) != Some(code.invitation) {
            return Err(ManualAdmissionError::Rejected);
        }
        let deadline = crate::manual_admission::admission_deadline(connection);
        let result = tokio::time::timeout(
            deadline.saturating_duration_since(n0_future::time::Instant::now()),
            self.request_admission_inner(connection, Some(code.token), deadline),
        )
        .await
        .unwrap_or(Err(ManualAdmissionError::PathUnavailable));
        if result.is_err() {
            connection.close(0u32.into(), b"precode admission ended");
        }
        result
    }
}

impl ManagementStore {
    /// Code consumption, authenticated request, signed grant and pending
    /// delivery are one recoverable SQLite commit, before network delivery.
    async fn consume_precode(
        &self,
        token: [u8; 32],
        request: &AdmissionRequest,
        record: &ManagementRecord,
        now: i64,
    ) -> Result<(), StoreError> {
        let header =
            crate::management_auth::decode_complete(record.header(), record.body(), self.anchor())?;
        let mut tx = self.pool.begin().await?;
        let changed = sqlx::query("UPDATE precodes SET used=used+1 WHERE token=? AND stopped=0 AND revoked=0 AND expires>?
            AND used>=0 AND used<9223372036854775807
            AND used=(SELECT count(*) FROM precode_redemptions r WHERE r.token=precodes.token)
            AND NOT EXISTS (SELECT 1 FROM precode_redemptions r LEFT JOIN admission_deliveries d
                ON d.applicant=r.applicant AND d.request_id=r.request_id
                WHERE r.token=precodes.token AND (d.authorization IS NULL OR d.phase<1))
            AND (max_uses IS NULL OR used<max_uses)")
            .bind(token.as_slice()).bind(now).execute(&mut *tx).await?;
        if changed.rows_affected() != 1 {
            return Err(StoreError::PrecodeStopped);
        }
        sqlx::query("INSERT INTO precode_redemptions(token,applicant,request_id) VALUES(?,?,?)")
            .bind(token.as_slice())
            .bind(request.applicant.as_bytes().as_slice())
            .bind(request.request_id.as_slice())
            .execute(&mut *tx)
            .await?;
        let approved = sqlx::query("INSERT INTO admission_deliveries(applicant,request_id,authorization,phase) VALUES(?,?,?,1) ON CONFLICT(applicant,request_id) DO UPDATE SET authorization=excluded.authorization,phase=1 WHERE admission_deliveries.authorization IS NULL")
            .bind(request.applicant.as_bytes().as_slice()).bind(request.request_id.as_slice()).bind(header.hash().as_bytes().as_slice()).execute(&mut *tx).await?;
        if approved.rows_affected() != 1 {
            return Err(StoreError::Corrupt(
                "redemption request already has another result",
            ));
        }
        sqlx::query("INSERT OR IGNORE INTO management_records(id,header,body) VALUES(?,?,?)")
            .bind(header.hash().as_bytes().as_slice())
            .bind(record.header())
            .bind(record.body())
            .execute(&mut *tx)
            .await?;
        #[cfg(feature = "replica-test-hooks")]
        self.observe(crate::management_store::RecordCheckpoint::BeforePrecodeCommit);
        tx.commit().await?;
        #[cfg(feature = "replica-test-hooks")]
        self.observe(crate::management_store::RecordCheckpoint::AfterPrecodeCommit);
        Ok(())
    }
}
