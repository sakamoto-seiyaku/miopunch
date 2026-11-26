use p2panda_core::{Hash, VerifyingKey};
use serde::{Deserialize, Serialize};

/// Evidence durably confirmed by the applicant for its selected request.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[repr(u8)]
pub enum ApplicationProgress {
    Created = 0,
    Delivered = 1,
    Authorized = 2,
    Complete = 3,
    Rejected = 4,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AdmissionAttempt {
    pub request_id: [u8; 16],
    pub progress: ApplicationProgress,
}

impl AdmissionAttempt {
    pub fn public_json(self) -> serde_json::Value {
        serde_json::json!({"request_id": hex::encode(self.request_id), "progress": self.progress})
    }
}

/// Evidence recorded by the provider, independent of current authorization.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum AdmissionDecision {
    AwaitingApproval,
    Rejected,
    ApprovedAwaitingReceipt,
    ReceiptReceived,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmissionHistoryEntry {
    pub applicant: VerifyingKey,
    pub request_id: [u8; 16],
    pub decision: AdmissionDecision,
    pub authorization: Option<Hash>,
}

impl AdmissionHistoryEntry {
    pub(crate) fn from_evidence(
        applicant: VerifyingKey,
        request_id: [u8; 16],
        authorization: Option<Hash>,
        rejected: bool,
        receipt_received: bool,
    ) -> Self {
        let decision = if receipt_received {
            AdmissionDecision::ReceiptReceived
        } else if authorization.is_some() {
            AdmissionDecision::ApprovedAwaitingReceipt
        } else if rejected {
            AdmissionDecision::Rejected
        } else {
            AdmissionDecision::AwaitingApproval
        };
        Self {
            applicant,
            request_id,
            decision,
            authorization,
        }
    }

    pub fn public_json(self) -> serde_json::Value {
        serde_json::json!({"applicant": self.applicant.to_string(), "request_id": hex::encode(self.request_id),
            "decision": self.decision, "authorization": self.authorization.map(|id| id.to_string())})
    }
}

pub(crate) const RECENT_ADMISSION_ATTEMPTS: usize = 128;
