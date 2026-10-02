//! One-device, durable prepare → validate → switch → exit lifecycle.
//! The target is deliberately not exposed as a normal NativeNetwork until the
//! switch commits. Its separate store and scoped admission never see source logs.
use crate::{
    admission_gate::{ADMISSION_REQUEST_ALPN, ProtocolScope},
    daily_management::{ManagementAction, ManagementError, ManagementSession},
    genesis_recovery::GenesisCredential,
    manual_admission::{AdmissionStage, ManualAdmissionError, ManualExchange, ManualInvitation},
    native_files::{publish_file, sync_directory},
    native_network::{NativeNetwork, NodeError, NodeStatus, write_private},
    offline_replica::{REPLICA_ALPN, ReplicaError, ReplicaOutcome},
    precode::Precode,
};
use iroh::{EndpointAddr, endpoint::Connection};
use p2panda_auth::Access;
use std::{
    collections::BTreeSet,
    net::Ipv4Addr,
    path::{Path, PathBuf},
    time::Duration,
};

const INVITATION: &str = "migration.invitation";
const TARGET: &str = "migration-target";
const VALIDATED: &str = "migration.validated";
const SWITCHED: &str = "migration.switched";
const EXITED: &str = "migration.exited";
const CANCELLING: &str = "migration.cancelling";
const CANCELLED: &str = "migration.cancelled";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("migration is not at the required stage")]
    Stage,
    #[error("target member data path failed or timed out")]
    Path,
    #[error(transparent)]
    Node(#[from] NodeError),
    #[error(transparent)]
    Admission(#[from] ManualAdmissionError),
    #[error(transparent)]
    Replica(#[from] ReplicaError),
    #[error(transparent)]
    Management(#[from] ManagementError),
}

/// Owns both contexts and exposes exactly one for normal operations. Like the
/// existing native entry, the caller must own the directory exclusively; open
/// it through this entry on restart, never as two concurrent native processes.
pub struct NetworkMigration {
    source: NativeNetwork,
    target: NativeNetwork,
    directory: PathBuf,
    switched: bool,
    exited: bool,
    validated: bool,
    cancelling: bool,
    cancelled: bool,
}

impl NetworkMigration {
    /// This explicit call is this device's confirmation. No other device or
    /// source member is changed. The invitation contains no admission secret.
    pub async fn prepare(
        source: NativeNetwork,
        invitation: ManualInvitation,
    ) -> Result<Self, MigrationError> {
        if source.anchor == invitation.anchor
            || source.directory.join(INVITATION).exists()
            || !source
                .status()
                .members
                .iter()
                .any(|(id, _)| *id == source.status().node_id)
        {
            return Err(MigrationError::Stage);
        }
        let directory = source.directory.clone();
        publish(&directory, INVITATION, &invitation.encode())?;
        Self::load(source, directory).await
    }

    pub async fn open(directory: impl AsRef<Path>) -> Result<Self, MigrationError> {
        let directory = directory.as_ref().to_path_buf();
        let source = NativeNetwork::open_migration_context(&directory).await?;
        Self::load(source, directory).await
    }

    async fn load(source: NativeNetwork, directory: PathBuf) -> Result<Self, MigrationError> {
        let bytes = std::fs::read(directory.join(INVITATION)).map_err(NodeError::from)?;
        let invitation =
            ManualInvitation::decode(&bytes.try_into().map_err(|_| NodeError::InvalidData)?)?;
        if invitation.anchor == source.anchor {
            return Err(MigrationError::Stage);
        }
        let path = directory.join(TARGET);
        let target = if path.exists() {
            NativeNetwork::open_migration_context(&path).await?
        } else {
            NativeNetwork::join_migration(&path, invitation, &source.node_key).await?
        };
        if target.anchor != invitation.anchor || target.status().node_id != source.status().node_id
        {
            return Err(NodeError::InvalidData.into());
        }
        if !path.join("resources.json").exists() {
            target.save_resource_configuration(&source.resource_configuration()?)?;
        }
        let switched = marker(&directory, SWITCHED)?;
        let cancelling = marker(&directory, CANCELLING)?;
        let cancelled = marker(&directory, CANCELLED)?;
        if (switched && cancelling)
            || (cancelled && !cancelling)
            || (cancelled
                && target
                    .status()
                    .members
                    .iter()
                    .any(|(id, _)| *id == target.status().node_id))
        {
            return Err(MigrationError::Stage);
        }
        let exited = marker(&directory, EXITED)?;
        let validated = marker(&directory, VALIDATED)?;
        // A lost/rolled-back progress marker cannot override signed departure.
        // Ambiguous state fails closed instead of reopening source membership.
        let source_member = source
            .status()
            .members
            .iter()
            .any(|(id, _)| *id == source.status().node_id);
        if (!source_member || exited) && !switched {
            return Err(MigrationError::Stage);
        }
        if switched && !validated {
            return Err(MigrationError::Stage);
        }
        if exited && source_member {
            return Err(MigrationError::Stage);
        }
        target
            .gate
            .register_protocol(REPLICA_ALPN, ProtocolScope::Member);
        let result = Self {
            source,
            target,
            directory,
            switched,
            exited,
            validated,
            cancelling,
            cancelled,
        };
        result.apply_restrictions();
        Ok(result)
    }

    fn apply_restrictions(&self) {
        if self.switched {
            self.source.gate.restrict_lifecycle(Some(BTreeSet::new()));
            self.target.gate.restrict_lifecycle(None);
        } else if self.cancelling {
            self.target.gate.restrict_lifecycle(Some(BTreeSet::new()));
        } else {
            self.target.gate.restrict_lifecycle(Some(BTreeSet::from([
                ADMISSION_REQUEST_ALPN.to_vec(),
                REPLICA_ALPN.to_vec(),
            ])));
        }
    }

    #[cfg(feature = "replica-test-hooks")]
    pub fn set_checkpoint_observer(
        &mut self,
        observer: fn(crate::management_store::RecordCheckpoint),
    ) {
        self.source.store.set_checkpoint_observer(observer);
        self.target.store.set_checkpoint_observer(observer);
    }

    pub fn phase(&self) -> MigrationPhase {
        if self.cancelled {
            MigrationPhase::Cancelled
        } else if self.cancelling {
            MigrationPhase::Cancelling
        } else if self.exited {
            MigrationPhase::SourceExited
        } else if self.switched {
            MigrationPhase::Switched
        } else if self.validated {
            MigrationPhase::Validated
        } else {
            match self.target.admission_stage() {
                AdmissionStage::Complete => MigrationPhase::MaterialsSaved,
                AdmissionStage::PendingMaterials => MigrationPhase::PendingMaterials,
                _ => MigrationPhase::Preparing,
            }
        }
    }
    pub fn active(&self) -> &NativeNetwork {
        if self.switched {
            &self.target
        } else {
            &self.source
        }
    }
    pub fn active_mut(&mut self) -> &mut NativeNetwork {
        if self.switched {
            &mut self.target
        } else {
            &mut self.source
        }
    }
    pub fn configure_target_resources(
        &self,
        config: &crate::resources::ResourceConfig,
    ) -> Result<(), MigrationError> {
        self.preparing()?;
        self.target.save_resource_configuration(config)?;
        Ok(())
    }
    pub fn target_status(&self) -> NodeStatus {
        self.target.status()
    }
    pub fn source_status(&self) -> NodeStatus {
        self.source.status()
    }

    fn preparing(&self) -> Result<(), MigrationError> {
        if self.switched || self.cancelling {
            Err(MigrationError::Stage)
        } else {
            Ok(())
        }
    }

    /// Resume the same durable request and original result. New-contact completion
    /// explicitly changes only the target binding and uses the new code's quota.
    pub async fn use_new_target_precode(&mut self, code: &Precode) -> Result<(), MigrationError> {
        self.preparing()?;
        self.target.begin_precode_completion(code).await?;
        Ok(())
    }

    pub async fn redeem_target(
        &mut self,
        address: EndpointAddr,
        code: &Precode,
    ) -> Result<AdmissionStage, MigrationError> {
        self.deliver_target(address, Some(code)).await
    }
    pub async fn request_target(
        &mut self,
        address: EndpointAddr,
    ) -> Result<AdmissionStage, MigrationError> {
        self.deliver_target(address, None).await
    }
    async fn deliver_target(
        &mut self,
        address: EndpointAddr,
        code: Option<&Precode>,
    ) -> Result<AdmissionStage, MigrationError> {
        self.preparing()?;
        let endpoint = self
            .target
            .bind_configured_endpoint(Ipv4Addr::UNSPECIFIED)
            .await?;
        let result = async {
            let connection = tokio::time::timeout(
                Duration::from_secs(15),
                endpoint.connect(address, ADMISSION_REQUEST_ALPN),
            )
            .await
            .map_err(|_| MigrationError::Path)?
            .map_err(|_| MigrationError::Path)?;
            let exchange = if let Some(code) = code {
                self.target
                    .request_precode_admission(&connection, code)
                    .await?
            } else {
                self.target.request_manual_admission(&connection).await?
            };
            if let ManualExchange::Materials(delivery) = exchange {
                self.target.complete_manual_admission(delivery).await?;
            }
            Ok(self.target.admission_stage())
        }
        .await;
        endpoint.close().await;
        result
    }

    /// Real authenticated member LogSync exchanges application data. Restricted
    /// control handshakes and invitation delivery cannot set this durable fact.
    pub async fn validate_target(
        &mut self,
        address: EndpointAddr,
    ) -> Result<ReplicaOutcome, MigrationError> {
        self.preparing()?;
        if self.target.admission_stage() != AdmissionStage::Complete {
            return Err(MigrationError::Stage);
        }
        let endpoint = self
            .target
            .bind_configured_endpoint(Ipv4Addr::UNSPECIFIED)
            .await?;
        let result = async {
            let connection: Connection = tokio::time::timeout(
                Duration::from_secs(15),
                endpoint.connect(address, REPLICA_ALPN),
            )
            .await
            .map_err(|_| MigrationError::Path)?
            .map_err(|_| MigrationError::Path)?;
            let outcome = self.target.sync_replica(&connection).await?;
            if !outcome.complete() || !self.target.gate.allows(&connection, ProtocolScope::Member) {
                return Err(MigrationError::Path);
            }
            publish(&self.directory, VALIDATED, b"1")?;
            #[cfg(feature = "replica-test-hooks")]
            self.source
                .store
                .observe(crate::management_store::RecordCheckpoint::MigrationValidated);
            self.validated = true;
            Ok(outcome)
        }
        .await;
        endpoint.close().await;
        result
    }

    pub fn begin_source_management_session(&self) -> ManagementSession {
        self.source.begin_management_session()
    }
    pub fn elevate_source(
        &self,
        session: &mut ManagementSession,
        credential: GenesisCredential,
        duration: Duration,
    ) -> Result<(), MigrationError> {
        self.source.elevate_genesis(session, credential, duration)?;
        Ok(())
    }
    pub async fn switch(&mut self, session: &mut ManagementSession) -> Result<(), MigrationError> {
        if self.switched {
            return Ok(());
        }
        if self.cancelling
            || !self.validated
            || self.target.admission_stage() != AdmissionStage::Complete
        {
            return Err(MigrationError::Stage);
        }
        self.source.check_management_session(session)?;
        if self
            .source
            .status()
            .members
            .contains(&(self.source.status().node_id, Access::manage()))
            && session.elevation_remaining().is_none()
        {
            return Err(ManagementError::ElevationRequired.into());
        }
        let published = publish(&self.directory, SWITCHED, b"1");
        // Publication may succeed before directory flushing reports an error.
        // Honor a possibly committed switch in this process too; never keep
        // serving the source while restart would select the destination.
        if published.is_err() {
            if marker(&self.directory, SWITCHED).unwrap_or(true) {
                self.switched = true;
                self.apply_restrictions();
            }
            return Err(published.unwrap_err().into());
        }
        #[cfg(feature = "replica-test-hooks")]
        self.source
            .store
            .observe(crate::management_store::RecordCheckpoint::MigrationSwitched);
        self.switched = true;
        self.apply_restrictions();
        Ok(())
    }
    /// Separately durable source departure. After restart, an administrator
    /// explicitly elevates again; target remains active while exit is pending.
    pub async fn finish_source_exit(
        &mut self,
        session: &mut ManagementSession,
    ) -> Result<(), MigrationError> {
        if !self.switched {
            return Err(MigrationError::Stage);
        }
        if self.exited {
            return Ok(());
        }
        let id = self.source.status().node_id;
        if self
            .source
            .status()
            .members
            .iter()
            .any(|(member, _)| *member == id)
        {
            self.source
                .manage(session, ManagementAction::RemoveMember(id))
                .await?;
        }
        #[cfg(feature = "replica-test-hooks")]
        self.source
            .store
            .observe(crate::management_store::RecordCheckpoint::MigrationSourceExitCommitted);
        publish(&self.directory, EXITED, b"1")?;
        self.exited = true;
        Ok(())
    }
    /// Explicitly leave the prepared destination and retain its signed departure
    /// for forwarding. Unknown approval must first resume delivery; closing the
    /// coordinator merely pauses. An independently appointed target admin needs
    /// that target's genesis credential, never the source credential.
    pub async fn cancel_preparation(
        &mut self,
        credential: Option<GenesisCredential>,
    ) -> Result<(), MigrationError> {
        if self.switched {
            return Err(MigrationError::Stage);
        }
        if self.cancelled {
            return Ok(());
        }
        let id = self.target.status().node_id;
        let member = self
            .target
            .status()
            .members
            .iter()
            .any(|(member, _)| *member == id);
        if !member && !self.cancelling {
            return Err(MigrationError::Stage);
        }
        let mut session = self.target.begin_management_session();
        if let Some(credential) = credential {
            self.target
                .elevate_genesis(&mut session, credential, Duration::from_secs(60))?;
        }
        if member
            && self
                .target
                .status()
                .members
                .contains(&(id, Access::manage()))
            && session.elevation_remaining().is_none()
        {
            return Err(ManagementError::ElevationRequired.into());
        }
        publish(&self.directory, CANCELLING, b"1")?;
        self.cancelling = true;
        self.apply_restrictions();
        if member {
            self.target
                .manage(&mut session, ManagementAction::RemoveMember(id))
                .await?;
        }
        publish(&self.directory, CANCELLED, b"1")?;
        self.cancelled = true;
        Ok(())
    }

    pub async fn cancelled_target_history(
        &self,
    ) -> Result<Vec<crate::management_store::ManagementRecord>, MigrationError> {
        if !self.cancelled {
            return Err(MigrationError::Stage);
        }
        self.target
            .management_history()
            .await
            .map_err(NodeError::from)
            .map_err(Into::into)
    }

    /// Signed departure is retained for offline forwarding, without restoring
    /// source business or requiring a global cross-network transaction.
    pub async fn source_history(
        &self,
    ) -> Result<Vec<crate::management_store::ManagementRecord>, MigrationError> {
        self.source
            .management_history()
            .await
            .map_err(NodeError::from)
            .map_err(Into::into)
    }
    pub async fn close(self) {
        self.target.close().await;
        self.source.close().await;
    }
}

fn marker(directory: &Path, name: &str) -> Result<bool, NodeError> {
    match std::fs::read(directory.join(name)) {
        Ok(bytes) if bytes == b"1" => Ok(true),
        Ok(_) => Err(NodeError::InvalidData),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}
fn publish(directory: &Path, name: &str, bytes: &[u8]) -> Result<(), NodeError> {
    let target = directory.join(name);
    if target.exists() {
        return if std::fs::read(target)? == bytes {
            Ok(())
        } else {
            Err(NodeError::InvalidData)
        };
    }
    let temporary = directory.join(format!(".migration-{:032x}", rand::random::<u128>()));
    write_private(&temporary, bytes)?;
    let result = publish_file(&temporary, &target);
    let _ = std::fs::remove_file(&temporary);
    result?;
    sync_directory(directory)?;
    Ok(())
}
