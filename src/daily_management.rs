//! Device/session-local genesis elevation authors real root-signed records.
//! Remote receipt and durable replay continue to use the causal verifier;
//! neither a session handle nor its local lifetime is a network authority.
use std::time::{Duration, Instant};

use p2panda_auth::{
    Access, GroupsExtensionArgs,
    group::{GroupAction, GroupMember},
};
use p2panda_core::{Hash, Header, SigningKey, VerifyingKey};
use thiserror::Error;

use crate::genesis_recovery::GenesisCredential;
use crate::management_auth::ManagementExtension;
use crate::management_store::{ManagementRecord, StoreError};
use crate::native_network::{NativeNetwork, NodeError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagementAction {
    AddMember(VerifyingKey),
    RemoveMember(VerifyingKey),
    AppointAdministrator(VerifyingKey),
    DemoteAdministrator(VerifyingKey),
}

impl ManagementAction {
    fn target(self) -> VerifyingKey {
        match self {
            Self::AddMember(target)
            | Self::RemoveMember(target)
            | Self::AppointAdministrator(target)
            | Self::DemoteAdministrator(target) => target,
        }
    }
}

/// A committed record and the local effective view at return time. Later
/// causal correction may change that view; this is not global finality.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagementResult {
    pub record: Hash,
    pub action: ManagementAction,
    pub effective_access: Option<Access>,
}

#[derive(Debug, Error)]
pub enum ManagementError {
    #[error("management session belongs to another node instance or has ended")]
    InvalidSession,
    #[error("a current genesis elevation is required")]
    ElevationRequired,
    #[error("genesis credential does not match this network and canonical Create")]
    InvalidCredential,
    #[error("elevation duration must be nonzero and representable")]
    InvalidWindow,
    #[error("management validation or storage failed: {0}")]
    Store(#[from] StoreError),
    #[error("management application failed: {0}")]
    Node(#[from] NodeError),
}

struct Elevation {
    root: SigningKey,
    deadline: Instant,
}

/// An opaque, non-cloneable local management session. Each session owns its
/// own credential window. It cannot be serialized or transferred to another
/// open node, including a reopened instance of the same device.
pub struct ManagementSession {
    node_instance: [u8; 16],
    ended: bool,
    elevation: Option<Elevation>,
}

impl ManagementSession {
    pub fn exit_genesis_elevation(&mut self) {
        self.elevation = None;
    }

    pub fn end(&mut self) {
        self.exit_genesis_elevation();
        self.ended = true;
    }

    /// Secret-free feedback only; signing always checks the window again.
    pub fn elevation_remaining(&mut self) -> Option<Duration> {
        let remaining = self
            .elevation
            .as_ref()?
            .deadline
            .checked_duration_since(Instant::now());
        match remaining {
            Some(remaining) if !remaining.is_zero() => Some(remaining),
            _ => {
                self.exit_genesis_elevation();
                None
            }
        }
    }
}

impl NativeNetwork {
    pub fn begin_management_session(&self) -> ManagementSession {
        ManagementSession {
            node_instance: self.management_instance,
            ended: false,
            elevation: None,
        }
    }

    fn check_management_session(&self, session: &ManagementSession) -> Result<(), ManagementError> {
        if session.ended || session.node_instance != self.management_instance {
            return Err(ManagementError::InvalidSession);
        }
        Ok(())
    }

    /// Supply the independent root private key AND trusted canonical Create
    /// hash. Opening a creator's stored data never automatically elevates it.
    /// The caller chooses a finite window; re-verification replaces that window.
    pub fn elevate_genesis(
        &self,
        session: &mut ManagementSession,
        credential: GenesisCredential,
        window: Duration,
    ) -> Result<(), ManagementError> {
        self.check_management_session(session)?;
        if credential.anchor() != self.anchor {
            return Err(ManagementError::InvalidCredential);
        }
        let deadline = Instant::now()
            .checked_add(window)
            .filter(|_| !window.is_zero())
            .ok_or(ManagementError::InvalidWindow)?;
        session.elevation = Some(Elevation {
            root: credential.into_signing_key(),
            deadline,
        });
        Ok(())
    }

    /// Author against verified current causal heads, preflight with the same
    /// policy as receipt/replay, then durably ingest and apply. A local session
    /// never substitutes for the signed root author on a privileged record.
    pub async fn manage(
        &mut self,
        session: &mut ManagementSession,
        action: ManagementAction,
    ) -> Result<ManagementResult, ManagementError> {
        self.check_management_session(session)?;
        self.store.apply_ready().await?;
        self.refresh_authorization();
        let target = action.target();
        let current = self
            .store
            .nodes()
            .into_iter()
            .find(|(node, _)| *node == target)
            .map(|(_, access)| access);
        let privileged = matches!(
            action,
            ManagementAction::AppointAdministrator(_) | ManagementAction::DemoteAdministrator(_)
        ) || matches!(action, ManagementAction::RemoveMember(_))
            && current == Some(Access::manage());
        let elevated = session.elevation_remaining().is_some();
        if privileged && !elevated {
            return Err(ManagementError::ElevationRequired);
        }
        let signed_action = match action {
            ManagementAction::AddMember(_) => GroupAction::Add {
                member: GroupMember::Individual(target),
                access: Access::read(),
            },
            ManagementAction::RemoveMember(_) => GroupAction::Remove {
                member: GroupMember::Individual(target),
            },
            ManagementAction::AppointAdministrator(_) if current.is_some() => {
                GroupAction::Promote {
                    member: GroupMember::Individual(target),
                    access: Access::manage(),
                }
            }
            ManagementAction::AppointAdministrator(_) => GroupAction::Add {
                member: GroupMember::Individual(target),
                access: Access::manage(),
            },
            ManagementAction::DemoteAdministrator(_) => GroupAction::Demote {
                member: GroupMember::Individual(target),
                access: Access::read(),
            },
        };
        let author_id = if elevated {
            session.elevation.as_ref().unwrap().root.verifying_key()
        } else {
            self.node_key.verifying_key()
        };
        let log_id = loop {
            let candidate = rand::random();
            if !self.store.has_author_log_id(author_id, candidate) {
                break candidate;
            }
        };
        let extension = ManagementExtension {
            version: 1,
            log_id,
            group: GroupsExtensionArgs {
                group_id: self.anchor.root,
                action: signed_action,
                dependencies: self.store.tips(),
            },
        };
        // Check immediately before signing, after asynchronous recovery and
        // potentially costly graph/log preparation. No root key is borrowed
        // for a signature after this session's window has expired.
        let author = if elevated {
            session
                .elevation_remaining()
                .ok_or(ManagementError::ElevationRequired)?;
            &session.elevation.as_ref().unwrap().root
        } else {
            &self.node_key
        };
        let header = Header::<ManagementExtension>::builder()
            .build(author, extension)
            .encode();
        self.store
            .validate_local_record(&ManagementRecord::new(header.clone(), None))?;
        let record = self.ingest_management_record(&header, None).await?;
        let effective_access = self
            .store
            .nodes()
            .into_iter()
            .find(|(node, _)| *node == target)
            .map(|(_, access)| access);
        Ok(ManagementResult {
            record,
            action,
            effective_access,
        })
    }
}
