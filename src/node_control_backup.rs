//! Logical admission ledgers and retained Native rotation metadata for complete
//! cross-runtime backups. Private code tokens/materials have no Debug output.
use std::io;

use p2panda_core::{Hash, VerifyingKey};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct SavedCode {
    pub token: [u8; 32],
    pub origin: Hash,
    pub expires: u64,
    pub limit: Option<u64>,
    pub used: u64,
    pub revoked: bool,
    #[serde(default)]
    pub stopped: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct SavedApproval {
    pub applicant: VerifyingKey,
    pub request: [u8; 16],
    pub authorization: Option<Hash>,
    pub code: Option<[u8; 32]>,
    pub complete: bool,
    #[serde(default)]
    pub rejected: bool,
    #[serde(default)]
    pub delivery_phase: Option<u8>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct MaterialGeneration {
    pub request: [u8; 16],
    pub generation: u64,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct JoinRequest {
    pub contact: iroh_base::EndpointId,
    pub request: [u8; 16],
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct AdmissionLedger {
    pub codes: Vec<SavedCode>,
    pub approvals: Vec<SavedApproval>,
    pub material_generations: Vec<MaterialGeneration>,
    pub own_request: Option<JoinRequest>,
}
impl AdmissionLedger {
    pub fn verify_shape(&self) -> io::Result<()> {
        if self.codes.len() > 4096
            || self.approvals.len() > 4096
            || self.material_generations.len() > 4096
        {
            return Err(invalid());
        }
        let mut codes = std::collections::BTreeSet::new();
        for code in &self.codes {
            let actual = self
                .approvals
                .iter()
                .filter(|a| a.code == Some(code.token) && a.authorization.is_some())
                .count() as u64;
            if !codes.insert(code.token)
                || code.used != actual
                || code.limit == Some(0)
                || code.limit.is_some_and(|limit| code.used > limit)
            {
                return Err(invalid());
            }
        }
        let mut requests = std::collections::BTreeSet::new();
        for approval in &self.approvals {
            if !requests.insert((approval.applicant, approval.request))
                || approval.code.is_some_and(|t| !codes.contains(&t))
                || approval.complete && approval.authorization.is_none()
                || approval.rejected
                    && (approval.authorization.is_some() || approval.code.is_some())
                || approval.delivery_phase.is_some_and(|phase| {
                    phase > 3
                        || (phase == 0) != approval.authorization.is_none()
                        || (phase == 3) != approval.complete
                })
            {
                return Err(invalid());
            }
        }
        let mut generations = std::collections::BTreeSet::new();
        for row in &self.material_generations {
            if row.generation > i64::MAX as u64 || !generations.insert(row.request) {
                return Err(invalid());
            }
        }
        Ok(())
    }
    pub fn stop_restored_codes(&mut self) {
        for code in &mut self.codes {
            code.stopped = true;
        }
    }
    pub fn verify_history(
        &self,
        owner: VerifyingKey,
        auth: &crate::management_auth::ManagementAuth,
    ) -> io::Result<()> {
        self.verify_shape()?;
        for code in &self.codes {
            if auth.authorization_origin_at(owner, &[code.origin], true) != Some(code.origin) {
                return Err(invalid());
            }
        }
        for approval in &self.approvals {
            if let Some(id) = approval.authorization {
                if auth.authorization_origin_at(approval.applicant, &[id], false) != Some(id) {
                    return Err(invalid());
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RetainedRotation {
    pub generation: u64,
    pub proposal: Vec<u8>,
    pub completion: Vec<u8>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct RetainedRotationAck {
    pub rotation: [u8; 16],
    pub member: [u8; 32],
    pub acknowledgement: Vec<u8>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct RetainedRotationMaterial {
    pub generation: u64,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct NativeRotationState {
    pub rotations: Vec<RetainedRotation>,
    pub acknowledgements: Vec<RetainedRotationAck>,
    pub materials: Vec<RetainedRotationMaterial>,
}
impl NativeRotationState {
    pub fn verify_shape(&self) -> io::Result<()> {
        if self.rotations.len() > 4096
            || self.acknowledgements.len() > 4096
            || self.materials.len() > 4096
        {
            return Err(invalid());
        }
        let mut generations = std::collections::BTreeSet::new();
        for rotation in &self.rotations {
            if rotation.generation > i64::MAX as u64
                || !generations.insert(rotation.generation)
                || rotation.proposal.is_empty()
                || rotation.proposal.len() > 1024 * 1024
                || rotation.completion.is_empty()
                || rotation.completion.len() > 1024 * 1024
            {
                return Err(invalid());
            }
        }
        let mut acknowledgements = std::collections::BTreeSet::new();
        for ack in &self.acknowledgements {
            if !acknowledgements.insert((ack.rotation, ack.member))
                || ack.acknowledgement.is_empty()
                || ack.acknowledgement.len() > 1024 * 1024
            {
                return Err(invalid());
            }
        }
        let mut materials = std::collections::BTreeSet::new();
        for material in &self.materials {
            if material.generation > i64::MAX as u64
                || !materials.insert(material.generation)
                || material.bytes.is_empty()
                || material.bytes.len() > 1024 * 1024
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "inconsistent portable node control ledger",
    )
}
