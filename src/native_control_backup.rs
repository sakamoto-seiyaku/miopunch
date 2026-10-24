//! Native storage adapter for portable, original admission and completed
//! rotation metadata. Restore never makes an old precode redeemable again.
use std::path::Path;

use p2panda_core::{Hash, VerifyingKey};
use sqlx::{Row, sqlite::SqliteRow};

use crate::{
    management_store::ManagementStore,
    native_client::ClientError,
    native_network::NativeNetwork,
    node_control_backup::{
        AdmissionLedger, JoinRequest, MaterialGeneration, NativeRotationState, RetainedRotation,
        RetainedRotationAck, RetainedRotationMaterial, SavedApproval, SavedCode,
    },
};

fn blob<const N: usize>(row: &SqliteRow, name: &str) -> Result<[u8; N], ClientError> {
    Ok(row
        .try_get::<Vec<u8>, _>(name)?
        .try_into()
        .map_err(|_| "invalid portable control field length")?)
}
fn optional_hash(row: &SqliteRow, name: &str) -> Result<Option<Hash>, ClientError> {
    row.try_get::<Option<Vec<u8>>, _>(name)?
        .map(|b| {
            Ok(Hash::from_bytes(
                b.try_into()
                    .map_err(|_| "invalid portable authorization hash")?,
            ))
        })
        .transpose()
}

pub(crate) async fn export(
    node: &NativeNetwork,
) -> Result<(AdmissionLedger, NativeRotationState), ClientError> {
    node.store.ensure_admissions_open().await?;
    let mut admission = AdmissionLedger::default();
    admission.own_request = node.join.map(|j| JoinRequest {
        contact: j.invitation.contact,
        request: j.request_id,
    });
    let rows = sqlx::query(
        "SELECT token,origin,expires,max_uses,used,stopped,revoked FROM precodes ORDER BY token",
    )
    .fetch_all(&node.store.pool)
    .await?;
    for row in rows {
        admission.codes.push(SavedCode {
            token: blob(&row, "token")?,
            origin: Hash::from_bytes(blob(&row, "origin")?),
            expires: u64::try_from(row.try_get::<i64, _>("expires")?)?,
            limit: row
                .try_get::<Option<i64>, _>("max_uses")?
                .map(u64::try_from)
                .transpose()?,
            used: u64::try_from(row.try_get::<i64, _>("used")?)?,
            stopped: row.try_get::<i64, _>("stopped")? != 0,
            revoked: row.try_get::<i64, _>("revoked")? != 0,
        });
    }
    let redemptions=sqlx::query("SELECT token,applicant,request_id FROM precode_redemptions ORDER BY token,applicant,request_id").fetch_all(&node.store.pool).await?;
    let mut codes = std::collections::BTreeMap::new();
    for row in redemptions {
        let key = (
            blob::<32>(&row, "applicant")?,
            blob::<16>(&row, "request_id")?,
        );
        if codes.insert(key, blob::<32>(&row, "token")?).is_some() {
            return Err("request consumed more than one code".into());
        }
    }
    let rejected = sqlx::query(
        "SELECT applicant,request_id FROM admission_rejections ORDER BY applicant,request_id",
    )
    .fetch_all(&node.store.pool)
    .await?;
    let rejected = rejected
        .iter()
        .map(|r| Ok((blob::<32>(r, "applicant")?, blob::<16>(r, "request_id")?)))
        .collect::<Result<std::collections::BTreeSet<_>, ClientError>>()?;
    let rows=sqlx::query("SELECT applicant,request_id,authorization,phase FROM admission_deliveries ORDER BY applicant,request_id").fetch_all(&node.store.pool).await?;
    for row in rows {
        let applicant = blob::<32>(&row, "applicant")?;
        let request = blob::<16>(&row, "request_id")?;
        let phase = u8::try_from(row.try_get::<i64, _>("phase")?)?;
        admission.approvals.push(SavedApproval {
            applicant: VerifyingKey::from_bytes(&applicant)?,
            request,
            authorization: optional_hash(&row, "authorization")?,
            code: codes.remove(&(applicant, request)),
            complete: phase == 3,
            rejected: rejected.contains(&(applicant, request)),
            delivery_phase: Some(phase),
        });
    }
    if !codes.is_empty() {
        return Err("redemption has no original delivery".into());
    }
    let rows = sqlx::query(
        "SELECT request_id,generation FROM admission_material_generations ORDER BY request_id",
    )
    .fetch_all(&node.store.pool)
    .await?;
    for row in rows {
        admission.material_generations.push(MaterialGeneration {
            request: blob(&row, "request_id")?,
            generation: u64::try_from(row.try_get::<i64, _>("generation")?)?,
        });
    }
    admission.verify_shape()?;

    let mut rotation = NativeRotationState::default();
    let rows = sqlx::query(
        "SELECT generation,proposal,completion FROM admission_rotations ORDER BY generation",
    )
    .fetch_all(&node.store.pool)
    .await?;
    for row in rows {
        rotation.rotations.push(RetainedRotation {
            generation: u64::try_from(row.try_get::<i64, _>("generation")?)?,
            proposal: row.try_get("proposal")?,
            completion: row
                .try_get::<Option<Vec<u8>>, _>("completion")?
                .ok_or("finish admission-secret rotation before portable replacement")?,
        });
    }
    let rows=sqlx::query("SELECT rotation,member,acknowledgement FROM admission_rotation_acks ORDER BY rotation,member").fetch_all(&node.store.pool).await?;
    for row in rows {
        rotation.acknowledgements.push(RetainedRotationAck {
            rotation: blob(&row, "rotation")?,
            member: blob(&row, "member")?,
            acknowledgement: row.try_get("acknowledgement")?,
        });
    }
    for entry in std::fs::read_dir(&node.directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if let Some(suffix) = name.strip_prefix("rotation.material.") {
            if suffix.len() == 16 {
                if let Ok(generation) = u64::from_str_radix(suffix, 16) {
                    rotation.materials.push(RetainedRotationMaterial {
                        generation,
                        bytes: std::fs::read(entry.path())?,
                    });
                }
            }
        }
    }
    rotation.materials.sort_by_key(|m| m.generation);
    rotation.verify_shape()?;
    Ok((admission, rotation))
}

pub(crate) async fn import(
    store: &ManagementStore,
    path: &Path,
    admission: &AdmissionLedger,
    rotation: &NativeRotationState,
) -> Result<(), ClientError> {
    admission.verify_shape()?;
    rotation.verify_shape()?;
    let mut tx = store.pool.begin().await?;
    for code in &admission.codes {
        sqlx::query("INSERT INTO precodes (token,origin,expires,max_uses,used,stopped,revoked) VALUES (?,?,?,?,?,1,?)")
            .bind(code.token.as_slice()).bind(code.origin.as_bytes().as_slice()).bind(i64::try_from(code.expires)?)
            .bind(code.limit.map(i64::try_from).transpose()?).bind(i64::try_from(code.used)?).bind(i64::from(code.revoked)).execute(&mut *tx).await?;
    }
    for approval in &admission.approvals {
        let phase = approval.delivery_phase.unwrap_or(if approval.complete {
            3
        } else if approval.authorization.is_some() {
            1
        } else {
            0
        });
        sqlx::query("INSERT INTO admission_deliveries (applicant,request_id,authorization,phase) VALUES (?,?,?,?)")
            .bind(approval.applicant.as_bytes().as_slice()).bind(approval.request.as_slice()).bind(approval.authorization.map(|id|id.as_bytes().to_vec())).bind(i64::from(phase)).execute(&mut *tx).await?;
        if let Some(token) = approval.code {
            sqlx::query("INSERT INTO precode_redemptions VALUES (?,?,?)")
                .bind(token.as_slice())
                .bind(approval.applicant.as_bytes().as_slice())
                .bind(approval.request.as_slice())
                .execute(&mut *tx)
                .await?;
        }
        if approval.rejected {
            sqlx::query("INSERT INTO admission_rejections VALUES (?,?)")
                .bind(approval.applicant.as_bytes().as_slice())
                .bind(approval.request.as_slice())
                .execute(&mut *tx)
                .await?;
        }
    }
    for row in &admission.material_generations {
        sqlx::query("INSERT INTO admission_material_generations VALUES (?,?)")
            .bind(row.request.as_slice())
            .bind(i64::try_from(row.generation)?)
            .execute(&mut *tx)
            .await?;
    }
    for row in &rotation.rotations {
        sqlx::query("INSERT INTO admission_rotations VALUES (?,?,?)")
            .bind(i64::try_from(row.generation)?)
            .bind(&row.proposal)
            .bind(&row.completion)
            .execute(&mut *tx)
            .await?;
    }
    for row in &rotation.acknowledgements {
        sqlx::query("INSERT INTO admission_rotation_acks VALUES (?,?,?)")
            .bind(row.rotation.as_slice())
            .bind(row.member.as_slice())
            .bind(&row.acknowledgement)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    for material in &rotation.materials {
        crate::native_network::write_private(
            &path.join(format!("rotation.material.{:016x}", material.generation)),
            &material.bytes,
        )?;
    }
    Ok(())
}
