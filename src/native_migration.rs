//! The native host keeps one endpoint registered for this device while a
//! separately persisted destination proves admission and a member data path.
use std::{
    collections::BTreeSet,
    net::Ipv4Addr,
    path::Path,
    sync::{Arc, OnceLock, RwLock},
    time::Duration,
};

use iroh::{EndpointAddr, EndpointId};
use p2panda_core::{Hash, SigningKey, VerifyingKey};
use serde::Serialize;
use tokio::{
    sync::{Mutex, Notify},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

use crate::{
    admission_gate::{ADMISSION_REQUEST_ALPN, PRECODE_REQUEST_ALPN, ProtocolScope},
    admission_progress::{AdmissionAttempt, ApplicationProgress},
    connection_coordinator::MEMBER_ALPN,
    control_protocol::AuthenticatedControl,
    device_protocol::DEPARTURE_ALPN,
    management_auth::NetworkAnchor,
    manual_admission::{AdmissionStage, ManualExchange},
    native_client::{ClientError, ClientInvitation, NativeClient, NativeClientRuntime},
    native_device_management::{
        commit_migration_departure, deliver_migration_departure, migration_departure,
    },
    native_network::NativeNetwork,
    network_migration::{
        MigrationPhase, committed_switch, confirm_client_invitation, mark_switched,
        mark_target_validated, target_directory, target_validated,
    },
    recovery_card::RecoveryCard,
    resources::ResourceEndpoint,
    state_observation::StateChanges,
};

const RETRY: Duration = Duration::from_millis(500);

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

/// Verified progress for the migration explicitly confirmed on this device.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeMigrationProgress {
    pub source: NetworkAnchor,
    pub destination: NetworkAnchor,
    pub phase: MigrationPhase,
    pub admission: Option<AdmissionAttempt>,
    pub switched: bool,
    pub source_exit: SourceExit,
}

impl NativeMigrationProgress {
    pub(crate) fn public_json(&self) -> serde_json::Value {
        serde_json::json!({
            "source": self.source,
            "destination": self.destination,
            "phase": format!("{:?}", self.phase),
            "admission": self.admission.map(|admission| admission.public_json()),
            "switched": self.switched,
            "source_exit": self.source_exit,
        })
    }
}

pub(crate) struct MigrationDriver {
    pub(crate) changes: StateChanges,
    pub(crate) cancel: CancellationToken,
    pub(crate) transition: Mutex<()>,
    bind: Ipv4Addr,
    active: RwLock<ActiveNetwork>,
    source: OnceLock<Arc<Mutex<NativeNetwork>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    exit_wake: Notify,
    #[cfg(feature = "replica-test-hooks")]
    target_checkpoint: std::sync::Mutex<Option<fn(crate::management_store::RecordCheckpoint)>>,
}

struct ActiveNetwork {
    runtime: Arc<NativeClientRuntime>,
    progress: Option<NativeMigrationProgress>,
}

impl MigrationDriver {
    pub(crate) fn new(changes: StateChanges, bind: Ipv4Addr, runtime: NativeClientRuntime) -> Self {
        Self {
            changes,
            bind,
            cancel: CancellationToken::new(),
            transition: Mutex::new(()),
            active: RwLock::new(ActiveNetwork {
                runtime: Arc::new(runtime),
                progress: None,
            }),
            source: OnceLock::new(),
            worker: Mutex::new(None),
            exit_wake: Notify::new(),
            #[cfg(feature = "replica-test-hooks")]
            target_checkpoint: std::sync::Mutex::new(None),
        }
    }

    pub(crate) fn completed_runtime(&self) -> Arc<NativeClientRuntime> {
        self.active
            .read()
            .expect("active migration runtime")
            .runtime
            .clone()
    }

    pub(crate) fn into_runtime(self) -> Arc<NativeClientRuntime> {
        self.active
            .into_inner()
            .expect("active migration runtime")
            .runtime
    }

    fn set_active(&self, runtime: NativeClientRuntime) {
        self.active
            .write()
            .expect("active migration runtime")
            .runtime = Arc::new(runtime);
    }

    pub(crate) fn progress(&self) -> Option<NativeMigrationProgress> {
        self.active
            .read()
            .expect("active migration runtime")
            .progress
            .clone()
    }

    fn publish(&self, progress: NativeMigrationProgress) {
        let mut active = self.active.write().expect("active migration runtime");
        if active.progress.as_ref() != Some(&progress) {
            active.progress = Some(progress);
            self.changes.publish();
        }
    }

    fn publish_switch(&self, runtime: NativeClientRuntime, progress: NativeMigrationProgress) {
        let mut active = self.active.write().expect("active migration runtime");
        active.runtime = Arc::new(runtime);
        active.progress = Some(progress);
        self.changes.publish();
    }

    pub(crate) async fn stop(&self) -> Result<(), ClientError> {
        self.cancel.cancel();
        if let Some(worker) = self.worker.lock().await.take() {
            worker.await?;
        }
        Ok(())
    }

    pub(crate) async fn reopen(
        path: &Path,
        bind: Ipv4Addr,
        invitation: ClientInvitation,
    ) -> Result<NativeClient, ClientError> {
        let switched = committed_switch(path)?;
        if switched {
            NativeNetwork::check_replacement_open(&target_directory(path))?;
        } else {
            NativeNetwork::check_replacement_open(path)?;
        }
        let mut source = NativeNetwork::open_migration_context(path).await?;
        let mut progress = progress_for(&source, &invitation)?;
        if switched {
            let target = open_target(&source, &invitation, true).await?;
            crate::native_device_management::recover_saved_departure(&mut source).await?;
            restrict_source(&source);
            update_target_progress(&target, &mut progress).await?;
            update_exit_progress(&source, &mut progress)?;
            let client = NativeClient::start(target, bind).await?;
            client
                .migration
                .install_source(Arc::new(Mutex::new(source)));
            client.migration.publish(progress.clone());
            let driver = client.migration.clone();
            let runtime = client.runtime();
            *client.migration.worker.lock().await = Some(tokio::spawn(async move {
                driver
                    .finish_source_exit(&runtime.resources, progress)
                    .await;
            }));
            return Ok(client);
        }

        // Reopen old business even if creating or contacting the target will
        // have to be retried. Read any existing durable progress before ready.
        if target_directory(path).exists() {
            let recovered = async {
                let target = open_target(&source, &invitation, false).await?;
                let result = update_target_progress(&target, &mut progress).await;
                target.close().await;
                result
            }
            .await;
            if let Err(error) = recovered {
                eprintln!("native migration could not recover destination progress: {error}");
            }
        }
        let client = NativeClient::start(source, bind).await?;
        client.start_migration_worker(invitation, progress).await;
        Ok(client)
    }

    fn install_source(&self, source: Arc<Mutex<NativeNetwork>>) {
        assert!(
            self.source.set(source).is_ok(),
            "one confirmed migration per source"
        );
    }

    async fn prepare(
        &self,
        invitation: &ClientInvitation,
        progress: &mut NativeMigrationProgress,
    ) -> Result<(), ClientError> {
        let source = self.source.get().expect("migration source");
        if self.completed_runtime().transport_stopped() {
            self.restore_source(source).await?;
        }
        let runtime = self.completed_runtime();
        let mut target = {
            let source = source.lock().await;
            // The worker first uses the same durable target store on every retry.
            open_target(&source, invitation, false).await?
        };
        let extra_configuration = runtime
            .resources
            .additional_migration_facilities(target.resource_configuration()?);
        let mut extra = None;
        let mut last_member = None;
        loop {
            update_target_progress(&target, progress).await?;
            self.publish(progress.clone());
            if self.cancel.is_cancelled()
                || progress
                    .admission
                    .is_some_and(|attempt| attempt.progress == ApplicationProgress::Rejected)
            {
                if let Some(resources) = extra.take() {
                    ResourceEndpoint::close(resources).await;
                }
                target.close().await;
                return Ok(());
            }

            if progress.phase == MigrationPhase::Validated {
                if let Some(resources) = extra.take() {
                    ResourceEndpoint::close(resources).await;
                }
                return self.switch(source, &runtime, target, progress).await;
            }

            if extra.is_none()
                && let Some(configuration) = &extra_configuration
            {
                extra = target
                    .bind_migration_facilities(self.bind, configuration.clone())
                    .await
                    .ok();
            }
            let transports = std::iter::once(runtime.resources.as_ref()).chain(extra.as_ref());
            if target.admission_stage() == AdmissionStage::Complete {
                let mut validated = false;
                if let Some(address) = validation_peer(&target, invitation, &mut last_member) {
                    for transport in transports {
                        if validate_member(transport, &target, address.clone(), &self.cancel)
                            .await
                            .is_ok()
                        {
                            validated = true;
                            break;
                        }
                    }
                }
                if validated {
                    let source = source.lock().await;
                    mark_target_validated(&source.directory)?;
                    continue;
                }
            } else {
                for transport in transports {
                    if self
                        .receive_target(transport, &mut target, invitation)
                        .await
                        .is_ok()
                    {
                        break;
                    }
                }
            }

            if self.wait_to_retry().await {
                if let Some(resources) = extra.take() {
                    ResourceEndpoint::close(resources).await;
                }
                target.close().await;
                return Ok(());
            }
        }
    }

    async fn restore_source(&self, source: &Arc<Mutex<NativeNetwork>>) -> Result<(), ClientError> {
        source.lock().await.gate.restrict_lifecycle(None);
        let handles = self.completed_runtime().service_handles.clone();
        let runtime = NativeClientRuntime::start(source.clone(), self.bind, handles).await?;
        self.set_active(runtime);
        Ok(())
    }

    async fn switch(
        &self,
        source: &Arc<Mutex<NativeNetwork>>,
        runtime: &NativeClientRuntime,
        mut target: NativeNetwork,
        progress: &mut NativeMigrationProgress,
    ) -> Result<(), ClientError> {
        let _transition = self.transition.lock().await;
        if self.cancel.is_cancelled() {
            target.close().await;
            return Ok(());
        }
        let mut selected_progress = progress.clone();
        let directory = {
            let source = source.lock().await;
            NativeNetwork::check_replacement_open(&source.directory)?;
            update_exit_progress(&source, &mut selected_progress)?;
            source.directory.clone()
        };
        let prepared = async {
            target.gate.restrict_lifecycle(Some(BTreeSet::new()));
            target.store.changes = self.changes.clone();
            let target = NativeClientRuntime::start_with_bind_handoff(
                Arc::new(Mutex::new(target)),
                self.bind,
                runtime.service_handles.clone(),
                async {
                    if self.cancel.is_cancelled() {
                        return Err("migration closed".into());
                    }
                    source
                        .lock()
                        .await
                        .gate
                        .restrict_lifecycle(Some(BTreeSet::new()));
                    runtime.stop_transport().await
                },
            )
            .await?;
            let active_gate = target.node.lock().await.gate.clone();
            if let Err(error) = runtime.retire_for_migration().await {
                eprintln!("native migration could not retire a source service: {error}");
            }
            let publication = mark_switched(&directory);
            if let Err(error) = publication
                && matches!(committed_switch(&directory), Ok(false))
            {
                target.retire_for_migration().await?;
                return Err(error.into());
            }
            // The destination listener already exists. Activate its gate only
            // after publication, including a publication whose final flush failed.
            Ok::<_, ClientError>((target, active_gate))
        }
        .await;
        let (target, active_gate) = match prepared {
            Ok(target) => target,
            Err(error) => {
                if runtime.transport_stopped() {
                    self.restore_source(source).await?;
                }
                return Err(error);
            }
        };
        active_gate.restrict_lifecycle(None);
        *progress = selected_progress;
        self.publish_switch(target, progress.clone());
        let source = source.lock().await;
        restrict_source(&source);
        Ok(())
    }

    async fn run(
        self: Arc<Self>,
        invitation: ClientInvitation,
        mut progress: NativeMigrationProgress,
    ) {
        while !self.cancel.is_cancelled() {
            match self.prepare(&invitation, &mut progress).await {
                Ok(()) => break,
                Err(error) => eprintln!("native migration: {error}"),
            }
            if self.wait_to_retry().await {
                return;
            }
        }
        if progress.switched {
            let active = self.completed_runtime();
            self.finish_source_exit(&active.resources, progress).await;
        }
    }

    async fn finish_source_exit(
        &self,
        resources: &ResourceEndpoint,
        mut progress: NativeMigrationProgress,
    ) {
        let source = self.source.get().expect("migration source");
        let mut last = None;
        while !self.cancel.is_cancelled() {
            let result = async {
                let mut node = source.lock().await;
                commit_migration_departure(&mut node, None).await?;
                update_exit_progress(&node, &mut progress)
            }
            .await;
            if let Err(error) = result {
                eprintln!("native migration source departure: {error}");
                if self.wait_to_retry().await {
                    return;
                }
                continue;
            }
            self.publish(progress.clone());
            match progress.source_exit {
                SourceExit::GenesisConfirmationRequired => {
                    tokio::select! {
                        _ = self.cancel.cancelled() => return,
                        _ = self.exit_wake.notified() => {},
                    }
                }
                SourceExit::PendingDelivery { .. } => {
                    let _ = deliver_migration_departure(source, resources, &mut last).await;
                    if self.wait_to_retry().await {
                        return;
                    }
                }
                _ => return,
            }
        }
    }

    async fn wait_to_retry(&self) -> bool {
        tokio::select! {
            _ = self.cancel.cancelled() => true,
            _ = tokio::time::sleep(RETRY) => false,
        }
    }
}

impl NativeClient {
    #[cfg(feature = "replica-test-hooks")]
    pub(crate) fn migration_checkpoint(
        &self,
        observer: fn(crate::management_store::RecordCheckpoint),
    ) {
        *self
            .migration
            .target_checkpoint
            .lock()
            .expect("migration checkpoint") = Some(observer);
    }

    /// Confirm this device's migration to a trusted new-network invitation.
    /// The SDK continues the same admission, data validation and local switch.
    pub async fn begin_network_migration(
        &self,
        invitation: &ClientInvitation,
    ) -> Result<NativeMigrationProgress, ClientError> {
        let _transition = self.migration.transition.lock().await;
        let runtime = self.runtime();
        runtime.require_ready()?;
        let source = runtime.node.lock().await;
        let progress = progress_for(&source, invitation)?;
        if self.migration.progress().is_some()
            || source.admission_stage() != AdmissionStage::Complete
            || !source
                .status()
                .members
                .iter()
                .any(|(id, _)| *id == source.status().node_id)
        {
            return Err("this device cannot start a new migration".into());
        }
        crate::client_contact::import_contact(
            source.resource_configuration()?,
            invitation.bootstrap(),
        )?;
        confirm_client_invitation(&source.directory, invitation)?;
        drop(source);
        self.start_migration_worker(invitation.clone(), progress.clone())
            .await;
        Ok(progress)
    }

    pub fn migration_progress(&self) -> Option<NativeMigrationProgress> {
        self.migration.progress()
    }

    /// Confirm the exact old-network self-departure using its retained genesis
    /// credential or an explicitly supplied card. This never changes the active network.
    pub async fn confirm_migration_source_exit(
        &self,
        card: Option<&RecoveryCard>,
    ) -> Result<NativeMigrationProgress, ClientError> {
        let _transition = self.migration.transition.lock().await;
        let mut progress = self
            .migration
            .progress()
            .filter(|progress| progress.switched)
            .ok_or("no switched migration")?;
        let mut source = self
            .migration
            .source
            .get()
            .expect("migration source")
            .lock()
            .await;
        if !matches!(
            migration_departure(&source)?,
            SourceExit::GenesisConfirmationRequired
        ) {
            return Err("old network does not require genesis confirmation".into());
        }
        let key = if let Some(card) = card {
            if card.anchor() != source.anchor {
                return Err("card belongs to another network".into());
            }
            SigningKey::from_bytes(&card.credential().0)
        } else {
            source.saved_genesis_credential()?.root
        };
        commit_migration_departure(&mut source, Some(&key)).await?;
        update_exit_progress(&source, &mut progress)?;
        self.migration.publish(progress.clone());
        self.migration.exit_wake.notify_one();
        Ok(progress)
    }

    async fn start_migration_worker(
        &self,
        invitation: ClientInvitation,
        progress: NativeMigrationProgress,
    ) {
        let runtime = self.runtime();
        self.migration.install_source(runtime.node.clone());
        self.migration.publish(progress.clone());
        let driver = self.migration.clone();
        *self.migration.worker.lock().await = Some(tokio::spawn(driver.run(invitation, progress)));
    }
}

fn progress_for(
    source: &NativeNetwork,
    invitation: &ClientInvitation,
) -> Result<NativeMigrationProgress, ClientError> {
    let destination = invitation.material()?.anchor;
    if source.anchor == destination
        || source.status().node_id.as_bytes() == invitation.address.id.as_bytes()
    {
        return Err("migration requires another network and an independent contact".into());
    }
    Ok(NativeMigrationProgress {
        source: source.anchor,
        destination,
        phase: MigrationPhase::Preparing,
        admission: None,
        switched: false,
        source_exit: SourceExit::NotStarted,
    })
}

async fn open_target(
    source: &NativeNetwork,
    invitation: &ClientInvitation,
    switched: bool,
) -> Result<NativeNetwork, ClientError> {
    let path = target_directory(&source.directory);
    let material = invitation.material()?;
    let mut node = if path.exists() || switched {
        NativeNetwork::open_migration_context(&path).await?
    } else {
        NativeNetwork::join_migration(&path, material, &source.node_key).await?
    };
    if node.anchor != material.anchor || node.status().node_id != source.status().node_id {
        return Err("saved migration destination does not match the confirmed operation".into());
    }
    if !switched {
        if !path.join("resources.json").exists() {
            let configuration = crate::client_contact::import_contact(
                source.resource_configuration()?,
                invitation.bootstrap(),
            )?;
            node.save_resource_configuration(&configuration)?;
        }
        if crate::client_contact::load_invitation(&node)?.is_none() {
            crate::client_contact::save_invitation(&node, invitation)?;
        }
        if let Some(name) = source.device_names.name(source.status().node_id) {
            let key = node.node_key.clone();
            node.update_device_names(|names| {
                names.seed_local(&key, name);
                Ok(())
            })?;
        }
        node.gate.restrict_lifecycle(Some(BTreeSet::from([
            ADMISSION_REQUEST_ALPN.to_vec(),
            PRECODE_REQUEST_ALPN.to_vec(),
            MEMBER_ALPN.to_vec(),
        ])));
    }
    Ok(node)
}

async fn update_target_progress(
    target: &NativeNetwork,
    progress: &mut NativeMigrationProgress,
) -> Result<(), ClientError> {
    progress.admission = NativeClientRuntime::read_admission_attempt(target).await?;
    let source = target.directory.parent().expect("managed migration target");
    progress.phase = match target.admission_stage() {
        AdmissionStage::Complete if target_validated(source)? => MigrationPhase::Validated,
        AdmissionStage::Complete => MigrationPhase::MaterialsSaved,
        AdmissionStage::PendingMaterials => MigrationPhase::PendingMaterials,
        _ => MigrationPhase::Preparing,
    };
    Ok(())
}

fn update_exit_progress(
    source: &NativeNetwork,
    progress: &mut NativeMigrationProgress,
) -> Result<(), ClientError> {
    progress.switched = true;
    progress.source_exit = migration_departure(source)?;
    progress.phase = if matches!(progress.source_exit, SourceExit::Delivered { .. }) {
        MigrationPhase::SourceExited
    } else {
        MigrationPhase::Switched
    };
    Ok(())
}

fn restrict_source(source: &NativeNetwork) {
    source
        .gate
        .register_protocol(DEPARTURE_ALPN, ProtocolScope::AdmissionRequest);
    source
        .gate
        .restrict_lifecycle(Some(BTreeSet::from([DEPARTURE_ALPN.to_vec()])));
}

impl MigrationDriver {
    async fn receive_target(
        &self,
        resources: &ResourceEndpoint,
        target: &mut NativeNetwork,
        invitation: &ClientInvitation,
    ) -> Result<(), ClientError> {
        let code = invitation.code()?;
        let alpn = if let Some(code) = &code {
            NativeClientRuntime::select_precode(target, code, invitation, None).await?;
            PRECODE_REQUEST_ALPN
        } else {
            ADMISSION_REQUEST_ALPN
        };
        let (delivery, attempt) = tokio::select! {
            _ = self.cancel.cancelled() => return Ok(()),
            result = resources.connect_migration(invitation.address.clone(), alpn, &target.gate, true) => result?,
        };
        let connection = delivery
            .control()
            .map_err(|error| attempt.failed(error.to_string()))?;
        #[cfg(feature = "replica-test-hooks")]
        if let Some(observer) = *self.target_checkpoint.lock().expect("migration checkpoint") {
            target.store.set_checkpoint_observer(observer);
        }
        let result = async {
            let exchange = if let Some(code) = &code {
                target.request_precode_admission(&connection, code).await?
            } else {
                target.request_manual_admission(&connection).await?
            };
            if let ManualExchange::Materials(delivery) = exchange {
                target.complete_manual_admission(delivery).await?;
            }
            Ok::<(), ClientError>(())
        }
        .await;
        connection.close(0u32.into(), b"migration admission ended");
        match result {
            Ok(()) => {
                attempt.succeeded();
                Ok(())
            }
            Err(error) => Err(attempt.failed(error.to_string()).into()),
        }
    }
}

fn validation_peer(
    target: &NativeNetwork,
    invitation: &ClientInvitation,
    last: &mut Option<EndpointId>,
) -> Option<EndpointAddr> {
    let status = target.status();
    let peers = status.members.iter().filter_map(|(id, _)| {
        (*id != status.node_id)
            .then(|| EndpointId::from_bytes(id.as_bytes()).ok())
            .flatten()
    });
    let peer = crate::service_protocol::next_member_sync_peer(peers, *last)?;
    *last = Some(peer);
    if peer == invitation.address.id {
        Some(invitation.address.clone())
    } else {
        Some(EndpointAddr::new(peer))
    }
}

async fn validate_member(
    resources: &ResourceEndpoint,
    target: &NativeNetwork,
    address: EndpointAddr,
    cancel: &CancellationToken,
) -> Result<(), ClientError> {
    let deadline = resources.contact_deadline();
    let (delivery, attempt) = tokio::select! {
        _ = cancel.cancelled() => return Err("migration closed".into()),
        result = resources.connect_migration(address, MEMBER_ALPN, &target.gate, false) => result?,
    };
    let connection = delivery
        .business()
        .map_err(|error| attempt.failed(error.to_string()))?;
    let result = tokio::time::timeout_at(deadline.into(), async {
        let (mut send, mut recv) = connection.open_bi().await?;
        send.write_all(b"PING").await?;
        send.finish()?;
        if recv.read_to_end(64).await? != b"PONG"
            || !target.gate.allows(&connection, ProtocolScope::Member)
        {
            return Err("target member did not complete the validated data exchange".into());
        }
        Ok::<(), ClientError>(())
    })
    .await;
    connection.close(0u32.into(), b"migration validation ended");
    match result {
        Ok(Ok(())) => {
            attempt.succeeded();
            Ok(())
        }
        Ok(Err(error)) => Err(attempt.failed(error.to_string()).into()),
        Err(error) => Err(attempt.failed(error.to_string()).into()),
    }
}
