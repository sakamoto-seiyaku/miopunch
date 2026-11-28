//! Shared description of a migration and the old network's departure evidence.
use p2panda_core::{Hash, VerifyingKey};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum MigrationPhase {
    Cancelling,
    Cancelled,
    Preparing,
    PendingMaterials,
    MaterialsSaved,
    Validated,
    Switched,
    SourceExited,
}

/// The result of the original network's independent signed departure process.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "stage")]
pub enum SourceExit {
    NotStarted,
    GenesisConfirmationRequired,
    MembershipAlreadyEnded,
    PendingDelivery {
        record: Hash,
    },
    Delivered {
        record: Hash,
        recipient: VerifyingKey,
    },
}

impl SourceExit {
    pub(crate) fn phase_after_switch(self) -> MigrationPhase {
        match self {
            Self::Delivered { .. } | Self::MembershipAlreadyEnded => MigrationPhase::SourceExited,
            Self::NotStarted | Self::GenesisConfirmationRequired | Self::PendingDelivery { .. } => {
                MigrationPhase::Switched
            }
        }
    }
}
