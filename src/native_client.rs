//! Controllable native client over the existing durable node and configured SDK.
//!
//! A host owns one client, calls operations, and awaits `close` before reopening
//! its directory. Invitations are capabilities: pass them through a trusted
//! channel and never include them in diagnostic output.
use std::{
    collections::BTreeMap,
    error::Error,
    net::Ipv4Addr,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use iroh::{EndpointAddr, endpoint::Connection};
use p2panda_core::VerifyingKey;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{sync::Mutex, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::{
    admission_gate::{ADMISSION_REQUEST_ALPN, PRECODE_REQUEST_ALPN, ProtocolScope},
    admission_progress::{
        AdmissionAttempt, AdmissionHistoryEntry, AdmissionReceipt, ApplicationProgress,
    },
    admission_rotation::{ADMISSION_ROTATION_ALPN, ROTATION_DELIVERY_ALPN, RotationProgress},
    connection_coordinator::ADMISSION_RECEIPT_ALPN,
    control_protocol::{AuthenticatedControl, ControlOwner, OwnedControl},
    manual_admission::{AdmissionRequest, AdmissionStage, ManualExchange, ManualInvitation},
    native_admission_rotation::{self, RotationConfirmation},
    native_member_services::{self, MemberService},
    native_migration::MigrationDriver,
    native_network::{NativeNetwork, NodeStatus},
    precode::Precode,
    resources::{ContactClass, ResourceEndpoint},
};

pub type ClientError = Box<dyn Error + Send + Sync>;
pub(crate) const MEMBER_MESSAGE: &[u8] = b"miopunch/client-message/1";

pub(crate) fn allocate_service_handle(ids: &AtomicU64) -> Result<u64, ClientError> {
    ids.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .map_err(|_| "native service identifier exhausted".into())
}

fn host_device_name() -> Option<String> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    return std::fs::read_to_string("/proc/sys/kernel/hostname").ok();
    #[cfg(windows)]
    return std::env::var("COMPUTERNAME").ok();
    #[cfg(all(unix, not(any(target_os = "linux", target_os = "android"))))]
    {
        let output = std::process::Command::new("hostname").output().ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8(output.stdout).ok()
    }
}

pub use crate::client_invitation::ClientInvitation;
pub use crate::native_migration::{NativeMigrationProgress, SourceExit};
impl ClientInvitation {
    pub(crate) fn material(&self) -> Result<ManualInvitation, ClientError> {
        Ok(ManualInvitation::decode(&self.material_bytes()?)?)
    }
    pub(crate) fn code(&self) -> Result<Option<Precode>, ClientError> {
        self.precode_bytes()?
            .as_ref()
            .map(|bytes| Ok(Precode::decode(bytes)?))
            .transpose()
    }
}

/// Secret-free local state, obtained after verifying the durable SDK records.
pub struct ClientStatus {
    pub node: NodeStatus,
    pub display_name: Option<String>,
    pub member_display_names: BTreeMap<VerifyingKey, String>,
    pub admission: AdmissionStage,
    pub admission_attempt: Option<AdmissionAttempt>,
    /// Recorded recovery-card export progress. `None` means the record could not be read or verified.
    pub recovery_exported: Option<bool>,
    pub recovery_saved: bool,
    pub manual_rejected: bool,
    pub membership_ended: bool,
    pub replacement_ready: bool,
}

/// The locally applied membership, this device's admission, and the provider
/// ledger read together. Use `cursor` to await another snapshot from this client.
pub struct ClientStateSnapshot {
    pub cursor: u64,
    pub status: ClientStatus,
    pub pending_admissions: Vec<AdmissionRequest>,
    pub admission_display_names: BTreeMap<VerifyingKey, String>,
    pub admission_history: Vec<AdmissionHistoryEntry>,
    pub admission_rotation: Option<RotationProgress>,
    pub network_migration: Option<NativeMigrationProgress>,
    pub device_operations: Value,
}

/// A bounded native host. Incoming admission and member exchanges run while the
/// caller controls the client; there is no UI, alternate transport or SSH path.
pub struct NativeClient {
    pub(crate) migration: Arc<MigrationDriver>,
    _migration_cancel: tokio_util::sync::DropGuard,
}

/// The services bound to one native network.
pub(crate) struct NativeClientRuntime {
    pub(crate) service_handles: Arc<AtomicU64>,
    pub(crate) node: Arc<Mutex<NativeNetwork>>,
    pub(crate) resources: Arc<ResourceEndpoint>,
    pub(crate) files: Arc<crate::native_file_transfer::NativeFiles>,
    pub(crate) services: Arc<Mutex<crate::native_services::NativeServices>>,
    pub(crate) clipboard: Arc<crate::native_clipboard::NativeClipboard>,
    pub(crate) forwarding: Arc<crate::native_forwarding::NativeForwarding>,
    pub(crate) web: Arc<crate::native_web::NativeWeb>,
    pub(crate) terminal: Arc<crate::native_terminal::NativeTerminal>,
    _terminal_owner: crate::native_terminal::TerminalOwner,
    _web_owner: crate::native_web::WebOwner,
    _forwarding_owner: crate::native_forwarding::ForwardingOwner,
    _clipboard_owner: crate::native_clipboard::ClipboardOwner,
    _file_owner: crate::native_file_transfer::FileOwner,
    cancel: CancellationToken,
    _drop_cancel: tokio_util::sync::DropGuard,
    listener: Mutex<Option<JoinHandle<()>>>,
    pub(crate) removal_commits: tokio_util::task::TaskTracker,
    recovery_saved: Arc<AtomicBool>,
    pub(crate) replacement_ready: Arc<AtomicBool>,
    admission_operation: Mutex<()>,
    pub(crate) rotation_confirmation: Arc<RotationConfirmation>,
}
impl NativeClient {
    pub async fn create(path: impl AsRef<Path>, bind: Ipv4Addr) -> Result<Self, ClientError> {
        Self::create_with_resources(path, bind, Default::default()).await
    }
    pub async fn create_with_resources(
        path: impl AsRef<Path>,
        bind: Ipv4Addr,
        config: crate::resources::ResourceConfig,
    ) -> Result<Self, ClientError> {
        config.validate_contact_facilities()?;
        let node = NativeNetwork::create(path).await?;
        node.save_resource_configuration(&config)?;
        Self::start(node, bind).await
    }
    /// Reopen incomplete admission with a new complete precode invitation.
    /// Contact resources are imported before binding, preserving local policy.
    pub async fn resume(
        path: impl AsRef<Path>,
        invitation: &ClientInvitation,
        bind: Ipv4Addr,
    ) -> Result<Self, ClientError> {
        let mut node = NativeNetwork::open(path).await?;
        let code = invitation
            .code()?
            .ok_or("continuation requires a precode")?;
        NativeClientRuntime::remember_existing_admission(&node).await?;
        let config = crate::client_contact::import_contact(
            node.resource_configuration()?,
            invitation.bootstrap(),
        )?;
        NativeClientRuntime::select_precode(&mut node, &code, invitation, Some(&config)).await?;
        Self::start(node, bind).await
    }
    pub async fn join(
        path: impl AsRef<Path>,
        invitation: &ClientInvitation,
        bind: Ipv4Addr,
    ) -> Result<Self, ClientError> {
        Self::join_with_resources(path, invitation, bind, Default::default()).await
    }
    pub async fn join_with_resources(
        path: impl AsRef<Path>,
        invitation: &ClientInvitation,
        bind: Ipv4Addr,
        local: crate::resources::ResourceConfig,
    ) -> Result<Self, ClientError> {
        let config = crate::client_contact::import_contact(local, invitation.bootstrap())?;
        let node = NativeNetwork::join(path, invitation.material()?).await?;
        node.save_resource_configuration(&config)?;
        crate::client_contact::save_invitation(&node, invitation)?;
        Self::start(node, bind).await
    }
    pub async fn open(path: impl AsRef<Path>, bind: Ipv4Addr) -> Result<Self, ClientError> {
        let path = path.as_ref();
        if let Some(invitation) = crate::network_migration::client_invitation(path)? {
            return MigrationDriver::reopen(path, bind, invitation).await;
        }
        Self::start(NativeNetwork::open(path).await?, bind).await
    }

    pub(crate) async fn start(node: NativeNetwork, bind: Ipv4Addr) -> Result<Self, ClientError> {
        let changes = node.store.changes.clone();
        let runtime = NativeClientRuntime::start(Arc::new(Mutex::new(node)), bind, Arc::new(AtomicU64::new(1))).await?;
        let migration = Arc::new(MigrationDriver::new(changes, bind, runtime));
        let migration_cancel = migration.cancel.clone().drop_guard();
        Ok(Self { migration, _migration_cancel: migration_cancel })
    }

    pub(crate) fn runtime(&self) -> Arc<NativeClientRuntime> {
        self.migration.completed_runtime()
    }

    pub async fn state_snapshot(&self) -> Result<ClientStateSnapshot, ClientError> {
        let _transition = self.migration.transition.lock().await;
        let mut snapshot = self.runtime().runtime_snapshot().await?;
        snapshot.network_migration = self.migration.progress();
        Ok(snapshot)
    }

    /// Wait without polling across changes to either active network or migration.
    pub async fn observe_state(&self, after: u64) -> Result<ClientStateSnapshot, ClientError> {
        let mut updates = self.migration.changes.subscribe();
        tokio::select! {
            biased;
            _ = self.migration.cancel.cancelled() => return Err("client closed".into()),
            changed = updates.wait_for(|cursor| *cursor != after) => { changed?; }
        }
        self.state_snapshot().await
    }

    pub async fn close(self) -> Result<(), ClientError> {
        self.migration.stop().await?;
        let migration = Arc::try_unwrap(self.migration).map_err(|_| "client migration still owned")?;
        Arc::try_unwrap(migration.into_runtime())
            .map_err(|_| "client runtime still owned")?
            .close().await
    }
}

impl NativeClientRuntime {
    pub(crate) async fn start(shared_node: Arc<Mutex<NativeNetwork>>, bind: Ipv4Addr, service_handles: Arc<AtomicU64>) -> Result<Self, ClientError> {
        Self::start_with_bind_handoff(shared_node, bind, service_handles, async { Ok(()) }).await
    }

    pub(crate) async fn start_with_bind_handoff(
        shared_node: Arc<Mutex<NativeNetwork>>,
        bind: Ipv4Addr,
        service_handles: Arc<AtomicU64>,
        before_bind: impl Future<Output = Result<(), ClientError>>,
    ) -> Result<Self, ClientError> {
        let mut node = shared_node.lock().await;
        crate::native_device_management::recover_saved_departure(&mut node).await?;
        if node.device_names.name(node.node_key.verifying_key()).is_none()
            && let Some(name) = host_device_name()
        {
            let key = node.node_key.clone();
            node.update_device_names(|names| {
                names.seed_local(&key, &name);
                Ok(())
            })?;
        }
        Self::remember_existing_admission(&node).await?;
        let recovery_saved = Arc::new(AtomicBool::new(
            recovery_state(&node)?.is_none_or(|state| state.saved),
        ));
        let replacement_ready = Arc::new(AtomicBool::new(
            crate::native_node_replacement::prepare_for_open(&node).await?,
        ));
        node.admission_gate()
            .register_protocol(MEMBER_MESSAGE, ProtocolScope::Member);
        node.admission_gate()
            .register_protocol(ADMISSION_RECEIPT_ALPN, ProtocolScope::AdmissionRequest);
        node.admission_gate()
            .register_protocol(crate::web::WEB_ALPN, ProtocolScope::Member);
        node.admission_gate()
            .register_protocol(crate::terminal::TERMINAL_ALPN, ProtocolScope::Member);
        for protocol in [crate::forwarding::Protocol::Tcp, crate::forwarding::Protocol::Udp] {
            node.admission_gate().register_protocol(protocol.alpn(), ProtocolScope::Member);
        }
        node.admission_gate()
            .register_protocol(crate::clipboard::CLIPBOARD_ALPN, ProtocolScope::Member);
        node.admission_gate()
            .register_protocol(crate::file_transfer::FILE_ALPN, ProtocolScope::Member);
        node.admission_gate()
            .register_protocol(crate::service_policy::POLICY_ALPN, ProtocolScope::Member);
        node.admission_gate()
            .register_protocol(crate::device_operation::DEVICE_ALPN, ProtocolScope::Member);
        node.admission_gate().register_protocol(
            crate::credential_custody::CUSTODY_ALPN,
            ProtocolScope::Member,
        );
        node.admission_gate().register_protocol(
            crate::device_operation::DEVICE_RESULT_ALPN,
            ProtocolScope::AdmissionRequest,
        );
        node.admission_gate().register_protocol(
            crate::device_protocol::DEPARTURE_ALPN,
            ProtocolScope::AdmissionRequest,
        );
        node.admission_gate().register_protocol(
            crate::device_protocol::REMOVAL_ALPN,
            ProtocolScope::AdmissionRequest,
        );
        node.admission_gate().register_protocol(
            crate::service_policy::GUEST_ALPN,
            ProtocolScope::AdmissionRequest,
        );
        // Dial-only pre-admission invitation delivery. This ALPN is not in
        // this network host's advertised incoming protocol set.
        node.admission_gate().register_protocol(
            crate::invitation_pairing::PAIRING_ALPN,
            ProtocolScope::AdmissionRequest,
        );
        let files = Arc::new(crate::native_file_transfer::NativeFiles::new(
            node.admission_gate().clone(),
        ));
        crate::native_custody::validate(&node)?;
        let mut services = crate::native_services::NativeServices::load(&node).await?;
        services.declare_files(&node, &files, false).await?;
        services
            .declare_clipboard(
                &node,
                &files,
                crate::system_clipboard::current_capability().await.read,
                false,
            )
            .await?;
        for protocol in [crate::forwarding::Protocol::Tcp, crate::forwarding::Protocol::Udp] {
            let access = services.access(&node).await?;
            if let Some(change) = access.declaration_change(protocol.service(), true, false) {
                services.apply(&node, &files, change).await?;
            }
        }
        let access = services.access(&node).await?;
        if let Some(change) = access.declaration_change(crate::service_policy::Service::Web, true, false) {
            services.apply(&node, &files, change).await?;
        }
        let access = services.access(&node).await?;
        if let Some(change) = access.declaration_change(crate::service_policy::Service::Terminal, false, false) {
            services.apply(&node, &files, change).await?;
        }
        files.update_access(services.access(&node).await?);
        let services = Arc::new(Mutex::new(services));
        let clipboard =
            crate::native_clipboard::NativeClipboard::new(node.admission_gate().clone());
        clipboard.enforce_access(&services.lock().await.access(&node).await?);
        services.lock().await.clipboard = Some(Arc::downgrade(&clipboard));
        let resources_config = node.resource_configuration()?;
        resources_config.validate_contact_facilities()?;
        ResourceEndpoint::validate_contact_timeout(Duration::from_millis(resources_config.maintenance.contact_timeout_ms))?;
        let forwarding = crate::native_forwarding::NativeForwarding::new(node.admission_gate().clone(), resources_config.business_channel, service_handles.clone())?;
        forwarding.enforce_access(&services.lock().await.access(&node).await?);
        services.lock().await.forwarding = Some(Arc::downgrade(&forwarding));
        let web = crate::native_web::NativeWeb::new(node.admission_gate().clone(), service_handles.clone());
        web.enforce_access(&services.lock().await.access(&node).await?);
        services.lock().await.web = Some(Arc::downgrade(&web));
        let terminal = crate::native_terminal::NativeTerminal::new(
            node.admission_gate().clone(),
            forwarding.channel_budget.clone(),
            service_handles.clone(),
        );
        terminal.enforce_access(&services.lock().await.access(&node).await?);
        services.lock().await.terminal = Some(Arc::downgrade(&terminal));
        before_bind.await?;
        let resources = Arc::new(node.bind_application_endpoint(bind).await?);
        let (fusion, gate) = resources
            .member_fusion
            .as_ref()
            .expect("native client member owner")
            .clone();
        resources.install_member_repair();
        resources.endpoint().set_alpns(vec![
            ADMISSION_REQUEST_ALPN.to_vec(),
            PRECODE_REQUEST_ALPN.to_vec(),
            ADMISSION_RECEIPT_ALPN.to_vec(),
            ADMISSION_ROTATION_ALPN.to_vec(),
            ROTATION_DELIVERY_ALPN.to_vec(),
            MEMBER_MESSAGE.to_vec(),
            crate::connection_coordinator::MEMBER_ALPN.to_vec(),
            crate::native_network::FUSED_MEMBER_ALPN.to_vec(),
            crate::connection_recovery::CONNECTION_RECOVERY_ALPN.to_vec(),
            crate::web::WEB_ALPN.to_vec(),
            crate::terminal::TERMINAL_ALPN.to_vec(),
            crate::forwarding::TCP_ALPN.to_vec(),
            crate::forwarding::UDP_ALPN.to_vec(),
            crate::clipboard::CLIPBOARD_ALPN.to_vec(),
            crate::file_transfer::FILE_ALPN.to_vec(),
            crate::service_policy::POLICY_ALPN.to_vec(),
            crate::device_operation::DEVICE_ALPN.to_vec(),
            crate::credential_custody::CUSTODY_ALPN.to_vec(),
            crate::device_operation::DEVICE_RESULT_ALPN.to_vec(),
            crate::device_protocol::DEPARTURE_ALPN.to_vec(),
            crate::device_protocol::REMOVAL_ALPN.to_vec(),
            crate::management_recovery::MANAGEMENT_RECOVERY_ALPN.to_vec(),
            crate::service_policy::GUEST_ALPN.to_vec(),
        ]);
        drop(node);
        let node = shared_node;
        let cancel = CancellationToken::new();
        let rotation_confirmation = Arc::new(RotationConfirmation::default());
        let removal_commits = tokio_util::task::TaskTracker::new();
        let listener = {
            let endpoint = resources.endpoint().clone();
            let resources = resources.clone();
            let node = node.clone();
            let cancel = cancel.clone();
            let removal_commits = removal_commits.clone();
            let recovery_saved = recovery_saved.clone();
            let replacement_ready = replacement_ready.clone();
            let rotation_confirmation = rotation_confirmation.clone();
            let files = files.clone();
            let services = services.clone();
            let clipboard = clipboard.clone();
            let forwarding = forwarding.clone();
            let web = web.clone();
            let terminal = terminal.clone();
            let member_contacts = resources.member_contacts.clone();
            let members = native_member_services::MemberServices {
                terminal: terminal.clone(), files: files.clone(), forwarding: forwarding.clone(),
                web: web.clone(), clipboard: clipboard.clone(), node: node.clone(), policies: services.clone(),
                member_contacts: member_contacts.clone(),
            };
            tokio::spawn(async move {
                let mut handlers = tokio::task::JoinSet::new();
                let mut member_handlers = tokio::task::JoinSet::new();
                let mut background = tokio::task::JoinSet::new();
                background.spawn(admission_receipts(
                    node.clone(),
                    resources.clone(),
                    recovery_saved.clone(),
                    replacement_ready.clone(),
                ));
                background.spawn(native_admission_rotation::maintain(
                    node.clone(), resources.clone(),
                    recovery_saved.clone(), replacement_ready.clone(),
                ));
                background.spawn(native_admission_rotation::commit_when_ready(
                    node.clone(), resources.clone(), rotation_confirmation,
                ));
                background.spawn(sync_member_management(
                    members.clone(),
                    resources.clone(),
                    gate.clone(),
                    recovery_saved.clone(),
                    replacement_ready.clone(),
                    removal_commits,
                ));
                let mut standby = crate::member_standby::MemberStandby::new(&resources);
                let mut expiry = tokio::time::interval(Duration::from_secs(1));
                loop {
                    tokio::select! {
                        _ = cancel.cancelled() => break,
                        _ = standby.completed(), if standby.len() > 0 => {},
                        Some(Ok(connection)) = fusion.accept() => {
                            let path_events = connection.path_events();
                            let members = members.clone();
                            let member_contacts = member_contacts.clone();
                            member_handlers.spawn(async move {
                                if connection.side().is_server()
                                    && let Some(contacts) = member_contacts
                                    && let Err(error) = contacts.observe_member(&connection).await
                                {
                                    eprintln!("could not save inbound member data path: {error}");
                                }
                                native_member_services::receive(connection, members, path_events).await;
                            });
                        }
                        _ = member_handlers.join_next(), if !member_handlers.is_empty() => {},
                        _ = expiry.tick() => {
                            if recovery_saved.load(Ordering::Acquire) && replacement_ready.load(Ordering::Acquire) {
                                standby.schedule(&resources, &gate);
                            }
                            if let Some(contacts) = &member_contacts
                                && let Err(error) = contacts.refresh_async().await
                            {
                                eprintln!("could not save verified member paths: {error}");
                            }
                            files.expire().await;
                            let node = node.lock().await;
                            if let Ok(access) = services.lock().await.access(&node).await {
                                forwarding.enforce_access(&access);
                                web.enforce_access(&access);
                                terminal.enforce_access(&access);
                            }
                        },
                        incoming = endpoint.accept() => {
                            let Some(incoming) = incoming else { break };
                            if handlers.len() >= 16 { incoming.refuse(); continue; }
                            let node = node.clone();
                            let resources = resources.clone();
                            let gate = gate.clone();
                            let recovery_saved = recovery_saved.clone();
                            let replacement_ready = replacement_ready.clone();
                            let files = files.clone();
                            let services = services.clone();
                            let clipboard = clipboard.clone();
                            let forwarding = forwarding.clone();
                            let web = web.clone();
                            let terminal = terminal.clone();
                            let clipboard_request = clipboard.state.request();
                            handlers.spawn(async move {
                                // The handshake has a finite budget. A large file has no
                                // total-duration cap: its stream enforces an idle timeout.
                                let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
                                let accepted = tokio::time::timeout_at(deadline, async {
                                    if !recovery_saved.load(Ordering::Acquire) || !replacement_ready.load(Ordering::Acquire) {
                                        incoming.refuse();
                                        return Err("recovery card save confirmation required".into());
                                    }
                                    let mut accepting = incoming.accept()?;
                                    let alpn = accepting.alpn().await?;
                                    Ok::<_, ClientError>((accepting, alpn))
                                }).await;
                                let Ok(Ok((accepting, alpn))) = accepted else { return };
                                if alpn == crate::native_network::FUSED_MEMBER_ALPN {
                                    let _ = tokio::time::timeout_at(deadline, accepting).await;
                                    return;
                                }
                                if alpn == crate::connection_recovery::CONNECTION_RECOVERY_ALPN {
                                    let _ = tokio::time::timeout_at(deadline, async {
                                        let control = accepting.into_control().await?;
                                        resources.serve_member_repair(&control).await.map_err(ClientError::from)
                                    }).await;
                                    return;
                                }
                                if alpn == crate::web::WEB_ALPN {
                                    if let Ok(Ok(connection)) = tokio::time::timeout_at(deadline, accepting).await {
                                        let _ = web.receive(connection.into()).await;
                                    }
                                    return;
                                }
                                if alpn == crate::terminal::TERMINAL_ALPN {
                                    if let Ok(Ok(connection)) = tokio::time::timeout_at(deadline, accepting).await {
                                        let _ = terminal.receive(connection.into()).await;
                                    }
                                    return;
                                }
                                if let Some(protocol) = crate::forwarding::Protocol::from_alpn(&alpn) {
                                    if let Ok(Ok(connection)) = tokio::time::timeout_at(deadline, accepting).await {
                                        let _ = forwarding.receive(connection.into(), protocol).await;
                                    }
                                    return;
                                }
                                if alpn == crate::clipboard::CLIPBOARD_ALPN {
                                    if let Ok(Ok(connection)) = tokio::time::timeout_at(deadline, accepting).await {
                                        let _ = clipboard.run(connection.into(), node, services, false, clipboard_request).await;
                                    }
                                    return;
                                }
                                if alpn == crate::file_transfer::FILE_ALPN {
                                    if let Ok(Ok(connection)) = tokio::time::timeout_at(deadline, accepting).await {
                                        let access = {
                                            let node = node.lock().await;
                                            services.lock().await.access(&node).await
                                        };
                                        let Ok(access) = access else {
                                            connection.close(1u32.into(), b"file authorization unavailable");
                                            return;
                                        };
                                        files.update_access(access);
                                        files.receive(connection.into());
                                    }
                                    return;
                                }
                                if alpn == crate::service_policy::POLICY_ALPN {
                                    if let Ok(Ok(connection)) = tokio::time::timeout_at(deadline, accepting).await {
                                        let _ = crate::native_services::serve_policy(connection, node, services, files, deadline).await;
                                    }
                                    return;
                                }
                                if alpn == crate::device_operation::DEVICE_ALPN {
                                    if let Ok(Ok(connection)) = tokio::time::timeout_at(deadline, accepting).await {
                                        let _ = tokio::time::timeout_at(deadline, crate::native_device_management::serve_device(connection, node, services, files)).await;
                                    }
                                    return;
                                }
                                if alpn == crate::credential_custody::CUSTODY_ALPN {
                                    if let Ok(Ok(connection)) = tokio::time::timeout_at(deadline, accepting).await {
                                        let _ = tokio::time::timeout_at(deadline, crate::native_custody::serve_custody(connection, node, services, files)).await;
                                    }
                                    return;
                                }
                                if alpn == crate::device_operation::DEVICE_RESULT_ALPN {
                                    let _ = tokio::time::timeout_at(deadline, async {
                                        let connection = Arc::new(accepting.into_control().await?);
                                        crate::native_device_management::serve_result(connection.retain_control(), node).await
                                    }).await;
                                    return;
                                }
                                if alpn == crate::device_protocol::DEPARTURE_ALPN {
                                    if let Ok(Ok(connection)) = tokio::time::timeout_at(deadline, accepting.into_control()).await {
                                        let control = Arc::new(connection).retain_control();
                                        let _ = crate::native_device_management::serve_departure(
                                            control, &node, &services, &files, deadline,
                                        ).await;
                                    }
                                    return;
                                }
                                if alpn == crate::device_protocol::REMOVAL_ALPN {
                                    if let Ok(Ok(connection)) = tokio::time::timeout_at(deadline, accepting.into_control()).await {
                                        let _ = crate::native_device_management::serve_removal_notice(
                                            Arc::new(connection).retain_control(), &node, deadline,
                                        ).await;
                                    }
                                    return;
                                }
                                if alpn == crate::management_recovery::MANAGEMENT_RECOVERY_ALPN {
                                    let _ = tokio::time::timeout_at(deadline, async {
                                        let connection = Arc::new(accepting.into_control().await?);
                                        let connection = connection.retain_control();
                                        let mut node = node.lock().await;
                                        let result = node.serve_management_recovery(&connection).await;
                                        files.update_access(services.lock().await.access(&node).await?);
                                        result.map(|_| ()).map_err(ClientError::from)
                                    }).await;
                                    return;
                                }
                                if alpn == crate::service_policy::GUEST_ALPN {
                                    let _ = tokio::time::timeout_at(deadline, async {
                                        let connection = Arc::new(accepting.into_control().await?);
                                        crate::native_services::serve_guest(connection.retain_control(), node, services, files).await
                                    }).await;
                                    return;
                                }
                                if alpn == ADMISSION_RECEIPT_ALPN {
                                    if let Ok(Ok(connection)) = tokio::time::timeout_at(deadline, accepting.into_control()).await {
                                        let connection = Arc::new(connection).retain_control();
                                        let _ = crate::manual_admission::serve_admission_receipt(&node, &connection).await;
                                        connection.close(0u32.into(), b"admission receipt ended");
                                    }
                                    return;
                                }
                                if matches!(alpn.as_slice(), ADMISSION_ROTATION_ALPN | ROTATION_DELIVERY_ALPN) {
                                    if let Ok(Ok(connection)) = tokio::time::timeout_at(deadline, accepting.into_control()).await {
                                        let connection = Arc::new(connection).retain_control();
                                        let result = NativeNetwork::receive_current_rotation(&node, &connection).await;
                                        if result.is_ok() {
                                            let _ = tokio::time::timeout_at(deadline, connection.revocation_handle().closed()).await;
                                        }
                                    }
                                    return;
                                }
                                let _ = tokio::time::timeout_at(deadline, async {
                                    if matches!(alpn.as_slice(), ADMISSION_REQUEST_ALPN | PRECODE_REQUEST_ALPN) {
                                        let connection = Arc::new(accepting.into_control().await?);
                                        serve_admission(connection.retain_control(), node, services, files).await
                                    } else {
                                        let connection = accepting.await?;
                                        let result = async {
                                            let (send, recv) = connection.accept_bi().await?;
                                            crate::member_message::serve(send, recv).await?;
                                            // Await the caller's close, so the final reply is delivered.
                                            connection.closed().await;
                                            Ok::<_, ClientError>(())
                                        }.await;
                                        connection.close(0u32.into(), b"client exchange ended");
                                        result
                                    }
                                }).await;
                            });
                        },
                        _ = handlers.join_next(), if !handlers.is_empty() => {},
                    }
                }
                handlers.abort_all();
                background.shutdown().await;
                standby.stop().await;
                while handlers.join_next().await.is_some() {}
                // Drop also reaches this owned listener through its cancel
                // guard. Keep the endpoint alive while QUIC close frames are
                // drained, so peer forwarding sockets need not wait for idle
                // timeout after the local owners have already ended.
                endpoint.close().await;
                while member_handlers.join_next().await.is_some() {}
            })
        };
        let drop_cancel = cancel.clone().drop_guard();
        Ok(Self {
            service_handles,
            node,
            resources,
            _file_owner: crate::native_file_transfer::FileOwner(files.clone()),
            files,
            services,
            _clipboard_owner: crate::native_clipboard::ClipboardOwner(clipboard.clone()),
            clipboard,
            _forwarding_owner: crate::native_forwarding::ForwardingOwner(forwarding.clone()),
            forwarding,
            _web_owner: crate::native_web::WebOwner(web.clone()),
            web,
            _terminal_owner: crate::native_terminal::TerminalOwner(terminal.clone()),
            terminal,
            cancel,
            _drop_cancel: drop_cancel,
            listener: Mutex::new(Some(listener)),
            removal_commits,
            recovery_saved,
            replacement_ready,
            admission_operation: Mutex::new(()),
            rotation_confirmation,
        })
    }
}

impl NativeClient {
    pub async fn status(&self) -> ClientStatus {
        let client_runtime = self.runtime();
        let node = client_runtime.node.lock().await;
        client_runtime.status_of(&node).await
    }

}

impl NativeClientRuntime {
    async fn status_of(&self, node: &NativeNetwork) -> ClientStatus {
        let admission_attempt = Self::read_admission_attempt(node).await.ok().flatten();
        let manual_rejected = if let Some(attempt) = admission_attempt {
            attempt.progress == ApplicationProgress::Rejected
        } else {
            node.manual_request_declined().await.unwrap_or(false)
        };
        let status = node.status();
        let membership_ended = node.membership_ended();
        let member_display_names = node.device_names.display_names(status.members.iter().map(|(id, _)| *id));
        ClientStatus {
            display_name: node.device_names.name(status.node_id).map(str::to_owned),
            member_display_names,
            node: status,
            admission: node.admission_stage(),
            admission_attempt,
            recovery_exported: read_recovery_state(node)
                .map(|state| state.is_some_and(|state| state.exported))
                .ok(),
            recovery_saved: self.recovery_saved.load(Ordering::Acquire),
            manual_rejected,
            membership_ended,
            replacement_ready: self.replacement_ready.load(Ordering::Acquire),
        }
    }

}

impl NativeClient {
    /// Persist this device's display name, including while its peers are offline.
    pub async fn set_device_display_name(&self, display_name: &str) -> Result<(), ClientError> {
        let client_runtime = self.runtime();
        let mut node = client_runtime.node.lock().await;
        let key = node.node_key.clone();
        node.update_device_names(|names| names.set_local(&key, display_name))?;
        Ok(())
    }

}

impl NativeClientRuntime {
    async fn runtime_snapshot(&self) -> Result<ClientStateSnapshot, ClientError> {
        let mut node = self.node.lock().await;
        node.recover_admission_state().await?;
        let pending_admissions = Self::pending_admissions(&node).await?;
        let admission_history = node.store.admission_history().await?;
        let admission_display_names = node.device_names.display_names(
            pending_admissions
                .iter()
                .map(|request| request.applicant)
                .chain(admission_history.iter().map(|entry| entry.applicant)),
        );
        Ok(ClientStateSnapshot {
            status: self.status_of(&node).await,
            pending_admissions,
            admission_display_names,
            admission_history,
            admission_rotation: node.rotation_progress().await?,
            network_migration: None,
            device_operations: crate::native_device_management::operation_state(&node)?,
            cursor: node.store.changes.cursor(),
        })
    }

}

impl NativeClient {
    /// Durable progress of the selected local admission. An unavailable path
    /// leaves this state intact and is reported by the admission operation.
    pub async fn admission_attempt(&self) -> Result<Option<AdmissionAttempt>, ClientError> {
        let client_runtime = self.runtime();
        NativeClientRuntime::read_admission_attempt(&*client_runtime.node.lock().await).await
    }

}

impl NativeClientRuntime {
    async fn remember_existing_admission(node: &NativeNetwork) -> Result<(), ClientError> {
        if node.join.is_some()
            && node.store.local_admission_progress().await?.is_none()
            && let Ok(Some(attempt)) = Self::read_admission_attempt(node).await
        {
            node.store.save_local_admission_progress(attempt.request_id, attempt.progress).await?;
        }
        Ok(())
    }

    pub(crate) async fn read_admission_attempt(
        node: &NativeNetwork,
    ) -> Result<Option<AdmissionAttempt>, ClientError> {
        let Some(binding) = node.join else { return Ok(None) };
        let Some(invitation) = crate::client_contact::load_invitation(node)? else {
            return Ok(None);
        };
        if invitation.material()? != binding.invitation {
            return Err("saved invitation does not match the local request".into());
        }
        let code = invitation.code()?;
        let request_id = code.map_or(binding.request_id, |code| code.request_id(binding.request_id));
        let stage = node.admission_stage();
        let progress = match node.store.local_admission_progress().await? {
            Some((attempt, authorization)) if attempt.request_id == request_id => {
                if attempt.progress == ApplicationProgress::Delivered
                    && authorization.is_some_and(|id| {
                        node.store.membership_authorization_valid(id, node.node_key.verifying_key())
                    })
                    && p2panda_core::VerifyingKey::from_bytes(binding.invitation.contact.as_bytes())
                        .is_ok_and(|provider| node.is_admin(provider))
                {
                    ApplicationProgress::Authorized
                } else {
                    attempt.progress
                }
            }
            Some(_) => ApplicationProgress::Created,
            None => match stage {
                // Upgraded stores already have verified material from the
                // original request, even if the progress marker is absent.
                AdmissionStage::Complete => ApplicationProgress::Complete,
                AdmissionStage::PendingMaterials => ApplicationProgress::Authorized,
                _ => ApplicationProgress::Created,
            },
        };
        let progress = match (progress, stage) {
            _ if code.is_none() && node.manual_request_declined().await? => ApplicationProgress::Rejected,
            (ApplicationProgress::Authorized, AdmissionStage::Complete) => ApplicationProgress::Complete,
            (ApplicationProgress::Complete, AdmissionStage::PendingMaterials) => ApplicationProgress::Authorized,
            _ => progress,
        };
        Ok(Some(AdmissionAttempt { request_id, progress }))
    }
}

impl NativeClient {
    pub fn address(&self) -> EndpointAddr {
        let client_runtime = self.runtime();
        client_runtime.resources.addr()
    }
    /// Explicit local opt-in. Membership alone never starts the receiver.
    pub async fn enable_files(&self, directory: impl AsRef<Path>) -> Result<(), ClientError> {
        let client_runtime = self.runtime();
        client_runtime.require_ready()?;
        let node = client_runtime.node.lock().await;
        let access = client_runtime.services.lock().await.access(&node).await?;
        if node.admission_stage() != AdmissionStage::Complete && access.guest_pairs().is_empty() {
            return Err("membership required".into());
        }
        client_runtime.files.update_access(access);
        client_runtime.files.enable(directory.as_ref()).await?;
        if let Err(error) = client_runtime
            .services
            .lock()
            .await
            .declare_files(&node, &client_runtime.files, true)
            .await
        {
            client_runtime.files.disable().await;
            return Err(error);
        }
        Ok(())
    }
}

impl NativeClientRuntime {
    async fn disable_files(&self) -> Result<(), ClientError> {
        self.files.disable().await;
        let node = self.node.lock().await;
        self.services
            .lock()
            .await
            .declare_files(&node, &self.files, false)
            .await
    }
}

impl NativeClient {
    pub async fn disable_files(&self) -> Result<(), ClientError> {
        self.runtime().disable_files().await
    }
    pub fn files_enabled(&self) -> bool {
        let client_runtime = self.runtime();
        client_runtime.files.enabled()
    }
    pub async fn send_file(
        &self,
        address: EndpointAddr,
        path: impl AsRef<Path>,
    ) -> Result<String, ClientError> {
        let client_runtime = self.runtime();
        client_runtime.require_ready()?;
        let node = client_runtime.node.lock().await;
        let access = client_runtime.services.lock().await.access(&node).await?;
        if node.admission_stage() != AdmissionStage::Complete && access.guest_pairs().is_empty() {
            return Err("membership required".into());
        }
        client_runtime.files.update_access(access);
        drop(node);
        Ok(client_runtime
            .files
            .send(client_runtime.resources.clone(), address, path.as_ref().to_owned())
            .await?)
    }
    pub fn file_tasks(&self) -> Vec<crate::file_transfer::FileTaskStatus> {
        let client_runtime = self.runtime();
        client_runtime.files.tasks.status()
    }
    pub fn cancel_file(&self, id: &str) -> Result<bool, ClientError> {
        let client_runtime = self.runtime();
        Ok(client_runtime.files.cancel(id)?)
    }
    pub fn disconnect_file(&self, id: &str) -> Result<(), ClientError> {
        let client_runtime = self.runtime();
        Ok(client_runtime.files.tasks.disconnect(id)?)
    }
    pub fn approve_file(&self, id: &str) -> Result<bool, ClientError> {
        let client_runtime = self.runtime();
        Ok(client_runtime.files.tasks.decide_file(id, true)?)
    }
    pub fn reject_file(&self, id: &str) -> Result<bool, ClientError> {
        let client_runtime = self.runtime();
        Ok(client_runtime.files.tasks.decide_file(id, false)?)
    }
    pub async fn invitation(&self) -> Result<ClientInvitation, ClientError> {
        let client_runtime = self.runtime();
        client_runtime.require_ready()?;
        let (address, bootstrap) = client_runtime.resources.invitation_contact()?;
        Ok(ClientInvitation::new(
            client_runtime.node.lock().await.manual_invitation()?.encode(),
            address,
            None,
            bootstrap,
        )?)
    }
    pub async fn issue_precode(
        &self,
        expires: u64,
        max_uses: Option<u64>,
    ) -> Result<ClientInvitation, ClientError> {
        let client_runtime = self.runtime();
        client_runtime.require_ready()?;
        let code = client_runtime
            .node
            .lock()
            .await
            .issue_precode(expires, max_uses)
            .await?;
        let (address, bootstrap) = client_runtime.resources.invitation_contact()?;
        Ok(ClientInvitation::new(
            code.invitation().encode(),
            address,
            Some(code.encode()),
            bootstrap,
        )?)
    }
    /// Revoke future redemptions of this node's precode using the issued invitation.
    /// Existing membership and committed admissions remain independent of the code.
    pub async fn revoke_precode(&self, invitation: &ClientInvitation) -> Result<(), ClientError> {
        let client_runtime = self.runtime();
        client_runtime.require_ready()?;
        let code = invitation.code()?.ok_or("not a precode")?;
        Ok(client_runtime.node.lock().await.revoke_precode(&code).await?)
    }
    pub async fn precode_uses(&self, invitation: &ClientInvitation) -> Result<u64, ClientError> {
        let client_runtime = self.runtime();
        let code = invitation.code()?.ok_or("not a precode")?;
        let status = client_runtime.node.lock().await.precode_usage_status(&code).await?;
        if !status.count_trusted {
            return Err("untrusted precode count".into());
        }
        status
            .saved_uses
            .ok_or_else(|| "precode count unavailable".into())
    }
    pub async fn requests(&self) -> Result<Vec<AdmissionRequest>, ClientError> {
        let client_runtime = self.runtime();
        let mut node = client_runtime.node.lock().await;
        node.recover_admission_state().await?;
        NativeClientRuntime::pending_admissions(&node).await
    }

    /// Pending requests paired with the latest authenticated display name.
    pub async fn requests_with_display_names(
        &self,
    ) -> Result<Vec<(AdmissionRequest, Option<String>)>, ClientError> {
        let client_runtime = self.runtime();
        let mut node = client_runtime.node.lock().await;
        node.recover_admission_state().await?;
        Ok(NativeClientRuntime::pending_admissions(&node)
            .await?
            .into_iter()
            .map(|request| {
                let name = node.device_names.name(request.applicant).map(str::to_owned);
                (request, name)
            })
            .collect())
    }

}

impl NativeClientRuntime {
    async fn pending_admissions(
        node: &NativeNetwork,
    ) -> Result<Vec<AdmissionRequest>, ClientError> {
        let mut pending = Vec::new();
        for request in node.manual_requests().await? {
            if !node.store.admission_rejected(&request).await?
                && node.store.approved_admission(&request).await?.is_none()
            {
                pending.push(request);
            }
        }
        Ok(pending)
    }

}

impl NativeClient {
    /// The latest locally handled attempts, including historical delivery
    /// outcomes whose original membership authorization no longer applies.
    pub async fn admission_history(&self) -> Result<Vec<AdmissionHistoryEntry>, ClientError> {
        let client_runtime = self.runtime();
        client_runtime.require_ready()?;
        let mut node = client_runtime.node.lock().await;
        node.recover_admission_state().await?;
        Ok(node.store.admission_history().await?)
    }

    pub(crate) async fn admission_history_with_display_names(
        &self,
    ) -> Result<Vec<(AdmissionHistoryEntry, Option<String>)>, ClientError> {
        let client_runtime = self.runtime();
        client_runtime.require_ready()?;
        let mut node = client_runtime.node.lock().await;
        node.recover_admission_state().await?;
        Ok(node.store.admission_history().await?
            .into_iter()
            .map(|entry| {
                let name = node.device_names.name(entry.applicant).map(str::to_owned);
                (entry, name)
            })
            .collect())
    }
    pub async fn reject(&self, request: &AdmissionRequest) -> Result<(), ClientError> {
        let client_runtime = self.runtime();
        client_runtime.require_ready()?;
        Ok(client_runtime
            .node
            .lock()
            .await
            .reject_manual_admission(request)
            .await?)
    }
    /// Explicitly start another ordinary request after rejection or removal.
    pub async fn new_manual_attempt(
        &self,
        invitation: &ClientInvitation,
    ) -> Result<(), ClientError> {
        let client_runtime = self.runtime();
        let _operation = client_runtime.admission_operation.lock().await;
        client_runtime.begin_new_manual_attempt(invitation).await
    }

    /// Explicitly start a fresh request under this device identity. The full
    /// invitation may be an ordinary invitation or a new precode. Newly
    /// imported contact facilities take effect the next time the SDK opens.
    pub async fn new_rejoin_attempt(&self, invitation: &ClientInvitation) -> Result<(), ClientError> {
        let client_runtime = self.runtime();
        let _operation = client_runtime.admission_operation.lock().await;
        let mut node = client_runtime.node.lock().await;
        if !node.store.known_removed(node.node_key.verifying_key()) {
            return Err("no ended membership to rejoin".into());
        }
        let code = invitation.code()?;
        if let Some(saved) = crate::client_contact::load_invitation(&node)? {
            let same_code = code == saved.code()?;
            if node.rejoin_pending && same_code && invitation.material()? == saved.material()? {
                return Ok(());
            }
            if code.is_some() && same_code {
                return Err("rejoining with a precode requires a new code".into());
            }
        }
        client_runtime.start_admission_attempt(&mut node, invitation).await
    }

}

impl NativeClientRuntime {
    async fn begin_new_manual_attempt(
        &self,
        invitation: &ClientInvitation,
    ) -> Result<(), ClientError> {
        let mut node = self.node.lock().await;
        let removed = node.membership_ended();
        if invitation.code()?.is_some() || (!removed && !node.manual_request_declined().await?) {
            return Err("no declined manual request or ended membership".into());
        }
        self.start_admission_attempt(&mut node, invitation).await
    }

    async fn start_admission_attempt(
        &self,
        node: &mut NativeNetwork,
        invitation: &ClientInvitation,
    ) -> Result<(), ClientError> {
        let material = invitation.material()?;
        if material.anchor != node.anchor {
            return Err("invitation belongs to another network".into());
        }
        let config = crate::client_contact::import_contact(
            node.resource_configuration()?, invitation.bootstrap(),
        )?;
        node.save_resource_configuration(&config)?;
        node.begin_rejoin_with_contact(material, Some(invitation)).await?;
        self.files
            .update_access(self.services.lock().await.access(node).await?);
        Ok(())
    }

}

impl NativeClient {
    pub async fn new_saved_manual_attempt(&self) -> Result<(), ClientError> {
        let client_runtime = self.runtime();
        let _operation = client_runtime.admission_operation.lock().await;
        client_runtime.begin_new_manual_attempt(&client_runtime.saved_admission_invitation().await?).await
    }
    pub async fn approve(&self, request: &AdmissionRequest) -> Result<AdmissionStage, ClientError> {
        let client_runtime = self.runtime();
        client_runtime.require_ready()?;
        let mut node = client_runtime.node.lock().await;
        let stage = node.approve_manual_admission(request).await?.stage;
        client_runtime.files
            .update_access(client_runtime.services.lock().await.access(&node).await?);
        Ok(stage)
    }
    /// Resume the same durable SDK attempt. Complete is returned only after
    /// verified membership and the current admission secret have been saved.
    pub async fn request_admission(
        &self,
        invitation: &ClientInvitation,
    ) -> Result<AdmissionStage, ClientError> {
        let client_runtime = self.runtime();
        let _operation = client_runtime.admission_operation.lock().await;
        client_runtime.receive_admission(invitation, true).await
    }
    /// Receive the verified authorization without finishing secret delivery.
    /// Hosts can stop here and resume `request_admission` after reopening. This
    /// state is deliberately never reported as completed admission.
    pub async fn receive_authorization(
        &self,
        invitation: &ClientInvitation,
    ) -> Result<AdmissionStage, ClientError> {
        let client_runtime = self.runtime();
        let _operation = client_runtime.admission_operation.lock().await;
        client_runtime.receive_admission(invitation, false).await
    }

    /// Continue the selected request using the full invitation already saved
    /// by this client, including after closing and reopening the node.
    pub async fn continue_admission(
        &self,
        authorization_only: bool,
    ) -> Result<AdmissionStage, ClientError> {
        let client_runtime = self.runtime();
        let _operation = client_runtime.admission_operation.lock().await;
        client_runtime.receive_admission(&client_runtime.saved_admission_invitation().await?, !authorization_only)
            .await
    }

}

impl NativeClientRuntime {
    async fn saved_admission_invitation(&self) -> Result<ClientInvitation, ClientError> {
        let node = self.node.lock().await;
        crate::client_contact::load_invitation(&node)?.ok_or_else(|| "no saved admission invitation".into())
    }

    pub(crate) async fn select_precode(
        node: &mut NativeNetwork,
        code: &Precode,
        invitation: &ClientInvitation,
        configuration: Option<&crate::resources::ResourceConfig>,
    ) -> Result<(), ClientError> {
        let binding = node.prepare_precode_completion(code).await?;
        if let Some(configuration) = configuration {
            node.save_resource_configuration(configuration)?;
        }
        if let Some(binding) = binding {
            crate::client_contact::stage_invitation(node, invitation)?;
            node.persist_join_binding(binding)?;
        } else {
            Self::save_selected_invitation(node, invitation)?;
        }
        Ok(())
    }

    fn save_selected_invitation(node: &NativeNetwork, invitation: &ClientInvitation) -> Result<(), ClientError> {
        let saved = crate::client_contact::load_invitation(node)?;
        if saved.as_ref().map(ClientInvitation::encode).transpose()?.as_deref()
            != Some(invitation.encode()?.as_slice())
        {
            let attempt_changed = saved
                .as_ref()
                .map(ClientInvitation::code)
                .transpose()?
                .flatten()
                != invitation.code()?;
            crate::client_contact::save_invitation(node, invitation)?;
            if attempt_changed {
                node.store.changes.publish();
            }
        }
        Ok(())
    }
    async fn receive_admission(
        &self,
        invitation: &ClientInvitation,
        save: bool,
    ) -> Result<AdmissionStage, ClientError> {
        let code = invitation.code()?;
        if let Some(code) = &code {
            let mut node = self.node.lock().await;
            Self::select_precode(&mut node, code, invitation, None).await?;
            self.files
                .update_access(self.services.lock().await.access(&node).await?);
        } else {
            let node = self.node.lock().await;
            if node.membership_ended() {
                return Err("membership ended; start a new rejoin attempt".into());
            }
            if node.manual_request_declined().await? {
                return Err(crate::manual_admission::ManualAdmissionError::RequestDeclined.into());
            }
            let binding = node.join.ok_or("no saved admission attempt")?;
            if invitation.material()? != binding.invitation {
                return Err("invitation does not match the local request".into());
            }
            Self::save_selected_invitation(&node, invitation)?;
        }
        let connection = self
            .resources
            .connect_control(
                invitation.address.clone(),
                if code.is_some() {
                    PRECODE_REQUEST_ALPN
                } else {
                    ADMISSION_REQUEST_ALPN
                },
            )
            .await?;
        let result = async {
            let mut node = self.node.lock().await;
            let exchange = if let Some(code) = &code {
                node.request_precode_admission(&connection, code).await?
            } else {
                node.request_manual_admission(&connection).await?
            };
            if let ManualExchange::Materials(delivery) = exchange {
                if save {
                    let result = node.complete_manual_admission(delivery).await;
                    self.files
                        .update_access(self.services.lock().await.access(&node).await?);
                    return Ok(result?);
                }
            }
            self.files
                .update_access(self.services.lock().await.access(&node).await?);
            Ok(node.admission_stage())
        }
        .await;
        connection.close(0u32.into(), b"client admission ended");
        result
    }

}

impl NativeClient {
    /// Finite member business exchange through configured SDK admission and
    /// route selection. Diagnostics report the actual selected connection path.
    pub async fn ping(&self, address: EndpointAddr) -> Result<Value, ClientError> {
        let client_runtime = self.runtime();
        client_runtime.require_ready()?;
        let queued = tokio::time::Instant::now();
        let contact = client_runtime.resources.dial_service(address, MemberService::Ping).await?;
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            let transport = contact.open(MemberService::Ping).await?;
            let (mut send, mut recv) = transport.stream_halves(true).await?;
            send.write_all(b"PING").await?;
            send.finish()?;
            let sent = tokio::time::Instant::now();
            if recv.read_to_end(64).await? != b"PONG" {
                return Err("unexpected member response".into());
            }
            Ok::<_, ClientError>(json!({"reply": "PONG", "transport": contact.status(),
                "timing": {"queue_ms": (sent - queued).as_secs_f64() * 1000.0,
                    "response_ms": sent.elapsed().as_secs_f64() * 1000.0}}))
        })
        .await;
        contact.end_legacy(b"client ping ended");
        result?
    }
    /// Inspect the real selected connection in an owned transport fixture.
    #[cfg(feature = "nat-lab-test-hooks")]
    #[doc(hidden)]
    pub async fn test_member_connection(&self, peer: iroh::EndpointId) -> Result<iroh::endpoint::MemberConnection, ClientError> {
        let client_runtime = self.runtime();
        let (fusion, _) = client_runtime.resources.member_fusion.as_ref().ok_or("member owner absent")?;
        fusion.reuse_until(peer, tokio::time::Instant::now() + Duration::from_secs(1)).await?
            .ok_or_else(|| "member connection absent".into())
    }

    #[cfg(feature = "nat-lab-test-hooks")]
    #[doc(hidden)]
    pub fn test_member_dial_pending(&self, peer: iroh::EndpointId) -> bool {
        let client_runtime = self.runtime();
        client_runtime.resources.member_fusion.as_ref().is_some_and(|(owner, _)| owner.test_member_dial_pending(peer))
    }

}

impl NativeClientRuntime {
    /// Stop this host's listener and handlers, close endpoint resources, then
    /// close the durable store. Identity and authorization remain on disk.
    pub(crate) async fn stop_transport(&self) -> Result<(), ClientError> {
        self.cancel.cancel();
        if let Some(listener) = self.listener.lock().await.take() {
            listener.await?;
        }
        self.resources.stop_contact().await;
        self.removal_commits.close();
        self.removal_commits.wait().await;
        Ok(())
    }

    pub(crate) async fn retire_for_migration(&self) -> Result<(), ClientError> {
        let terminal = self.terminal.close().await;
        let web = self.web.close().await;
        let forwarding = self.forwarding.close().await;
        self.clipboard.close().await;
        self.files.close().await;
        let transport = self.stop_transport().await;
        terminal.and(web).and(forwarding).and(transport)
    }

    pub(crate) fn transport_stopped(&self) -> bool {
        self.cancel.is_cancelled()
    }

    async fn close(self) -> Result<(), ClientError> {
        let terminal_stop = self.terminal.close().await;
        let web_stop = self.web.close().await;
        let forwarding_stop = self.forwarding.close().await;
        let clipboard_stop = self.disable_clipboard().await;
        self.clipboard.close().await;
        let service_stop = self.disable_files().await;
        let contacts_stop = if let Some(contacts) = &self.resources.member_contacts {
            contacts.refresh_async().await
        } else {
            Ok(())
        };
        self.stop_transport().await?;
        self.files.close().await;
        Arc::try_unwrap(self.resources)
            .map_err(|_| "client resources still owned")?
            .close()
            .await;
        Arc::try_unwrap(self.node)
            .map_err(|_| "client node still owned")?
            .into_inner()
            .close()
            .await;
        service_stop?;
        clipboard_stop?;
        web_stop?;
        forwarding_stop?;
        terminal_stop?;
        contacts_stop?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
struct RecoverySave {
    genesis: p2panda_core::Hash,
    exported: bool,
    saved: bool,
}

fn read_recovery_state(node: &NativeNetwork) -> Result<Option<RecoverySave>, ClientError> {
    let state = match std::fs::read(node.directory.join("client.recovery-save")) {
        Ok(bytes) => serde_json::from_slice::<RecoverySave>(&bytes)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if state.genesis != node.anchor.genesis || state.saved && !state.exported {
        return Err("invalid recovery acknowledgment".into());
    }
    Ok(Some(state))
}
fn recovery_state(node: &NativeNetwork) -> Result<Option<RecoverySave>, ClientError> {
    if !node.directory.join("genesis.credential").exists() {
        return Ok(None);
    }
    Ok(Some(read_recovery_state(node)?.unwrap_or(RecoverySave {
        genesis: node.anchor.genesis,
        exported: false,
        saved: false,
    })))
}
fn save_recovery_state(node: &NativeNetwork, state: &RecoverySave) -> Result<(), ClientError> {
    crate::native_files::write_private_atomic(
        &node.directory.join("client.recovery-save"),
        &serde_json::to_vec(state)?,
    )?;
    Ok(())
}
pub(crate) fn record_received_credential(node: &NativeNetwork) -> Result<(), ClientError> {
    save_recovery_state(
        node,
        &RecoverySave {
            genesis: node.anchor.genesis,
            exported: true,
            saved: true,
        },
    )?;
    node.store.changes.publish();
    Ok(())
}
impl NativeClient {
    pub async fn deliver_invitation(
        &self,
        pairing: &crate::invitation_pairing::PairingInfo,
        invitation: &ClientInvitation,
    ) -> Result<(), ClientError> {
        let client_runtime = self.runtime();
        client_runtime.require_ready()?;
        if invitation.material_bytes()? != client_runtime.node.lock().await.manual_invitation()?.encode() {
            return Err("invitation belongs to another network or contact".into());
        }
        let connection = client_runtime
            .resources
            .connect_control(
                pairing.address().clone(),
                crate::invitation_pairing::PAIRING_ALPN,
            )
            .await?;
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            crate::invitation_pairing::deliver(
                &connection,
                client_runtime.resources.endpoint().id(),
                pairing,
                invitation,
            ),
        )
        .await;
        connection.close(0u32.into(), b"invitation delivery ended");
        result??;
        Ok(())
    }
}

impl NativeClientRuntime {
    pub(crate) fn require_ready(&self) -> Result<(), ClientError> {
        if self.cancel.is_cancelled() {
            return Err("client network is no longer active".into());
        }
        if !self.recovery_saved.load(Ordering::Acquire) {
            return Err("recovery card save confirmation required".into());
        }
        if !self.replacement_ready.load(Ordering::Acquire) {
            return Err(
                "restored node must verify current authorization before ordinary use".into(),
            );
        }
        Ok(())
    }
}

impl NativeClient {
    /// Explicit secret-bearing return for a private save/print operation.
    /// Export does not acknowledge that the user saved the card.
    pub async fn export_recovery_card(
        &self,
        password: Option<&str>,
    ) -> Result<String, ClientError> {
        self.export_recovery_card_inner(password, None).await
    }
    /// A file host records export only after the private file is actually
    /// flushed. A failed file save cannot enable the later acknowledgment.
    pub async fn export_recovery_card_to(
        &self,
        path: impl AsRef<Path>,
        password: Option<&str>,
    ) -> Result<(), ClientError> {
        self.export_recovery_card_inner(password, Some(path.as_ref()))
            .await?;
        Ok(())
    }
    async fn export_recovery_card_inner(
        &self,
        password: Option<&str>,
        destination: Option<&Path>,
    ) -> Result<String, ClientError> {
        let client_runtime = self.runtime();
        let node = client_runtime.node.lock().await;
        let credential = node.saved_genesis_credential()?;
        let mut state = recovery_state(&node)?.ok_or("no genesis credential")?;
        let card = crate::recovery_card::RecoveryCard::new(
            *credential.root.as_bytes(),
            credential.anchor().genesis,
        );
        let text = card.export(password)?;
        if let Some(path) = destination {
            crate::native_network::write_private(path, text.as_bytes())?;
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            crate::native_files::sync_directory(parent)?;
        }
        state.exported = true;
        save_recovery_state(&node, &state)?;
        node.store.changes.publish();
        Ok(text)
    }
    /// User confirmation only. No password or read-back test is required.
    /// Retrying this confirmation does not create a new network or credential.
    pub async fn acknowledge_recovery_saved(&self) -> Result<(), ClientError> {
        let client_runtime = self.runtime();
        let node = client_runtime.node.lock().await;
        let mut state = recovery_state(&node)?.ok_or("not a network creator")?;
        if !state.exported {
            return Err("export the recovery card first".into());
        }
        state.saved = true;
        save_recovery_state(&node, &state)?;
        client_runtime.recovery_saved.store(true, Ordering::Release);
        node.store.changes.publish();
        Ok(())
    }
}
async fn sync_member_management(
    members: native_member_services::MemberServices,
    resources: Arc<crate::resources::ResourceEndpoint>,
    gate: crate::admission_gate::AdmissionGate,
    recovery_saved: Arc<AtomicBool>,
    replacement_ready: Arc<AtomicBool>,
    removal_commits: tokio_util::task::TaskTracker,
) {
    use crate::{
        resources::ContactClass,
        service_protocol::{MemberSyncMode, next_member_sync_peer},
    };
    let interval = Duration::from_millis(resources.maintenance_policy().exploration_interval_ms);
    let mut ticks = tokio::time::interval(interval);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last = None;
    let mut check_removal = std::collections::BTreeSet::new();
    loop {
        ticks.tick().await;
        if !recovery_saved.load(Ordering::Acquire) || !replacement_ready.load(Ordering::Acquire) {
            continue;
        }
        let peers = gate.member_contact_peers();
        if peers.is_empty() {
            let _ = crate::native_device_management::deliver_departure(
                &members.node, &resources, &mut last,
            ).await;
            continue;
        }
        check_removal.retain(|peer| peers.contains(peer));
        let peers = peers.into_iter()
            .filter(|peer| resources.contact_ready(*peer, ContactClass::Background));
        let Some(peer) = next_member_sync_peer(peers, last) else {
            continue;
        };
        last = Some(peer);
        if check_removal.remove(&peer) {
            let _ = crate::native_device_management::receive_removal_notice(
                members.node.clone(), members.policies.clone(), members.files.clone(), &resources,
                iroh::EndpointAddr::new(peer), ContactClass::Background, &removal_commits,
            ).await;
            continue;
        }
        let deadline = resources.contact_deadline();
        let Ok((connection, attempt)) = resources
            .connect_data_attempt(
                iroh::EndpointAddr::new(peer),
                crate::service_policy::POLICY_ALPN,
                ContactClass::Background,
                deadline,
            )
            .await
        else {
            check_removal.insert(peer);
            continue;
        };
        // Keep transport expiry outside the durable management transaction.
        let result = crate::native_services::sync_member(
            &connection,
            &members.node,
            &members.policies,
            &members.files,
            MemberSyncMode::Management,
            deadline.into(),
        )
        .await;
        connection.close(0u32.into(), b"member management exchange ended");
        match result {
            Ok(()) => attempt.succeeded(),
            Err(error) => {
                attempt.failed(error.to_string());
                check_removal.insert(peer);
            }
        }
    }
}

async fn admission_receipts(
    node: Arc<Mutex<NativeNetwork>>,
    resources: Arc<ResourceEndpoint>,
    recovery_saved: Arc<AtomicBool>,
    replacement_ready: Arc<AtomicBool>,
) {
    let mut changes = node.lock().await.store.changes.subscribe();
    loop {
        let mut retry = false;
        if recovery_saved.load(Ordering::Acquire) && replacement_ready.load(Ordering::Acquire) {
            let pending = node.lock().await.store.pending_admission_receipts().await;
            match pending {
                Ok(pending) => {
                    for (receipt, contact) in pending {
                        retry |= resume_admission_receipt(&node, &resources, receipt, contact)
                            .await
                            .is_err();
                    }
                }
                Err(_) => retry = true,
            }
        }
        if retry {
            tokio::time::sleep(Duration::from_secs(2)).await;
        } else if changes.changed().await.is_err() {
            break;
        }
    }
}

async fn receipt_material(
    node: &Mutex<NativeNetwork>,
    receipt: AdmissionReceipt,
    contact: iroh::EndpointId,
) -> Result<
    Option<(
        crate::admission_proof::AdmissionScope,
        crate::admission_proof::AdmissionSecret,
    )>,
    ClientError,
> {
    let node = node.lock().await;
    let provider = p2panda_core::VerifyingKey::from_bytes(contact.as_bytes())?;
    if !node.is_admin(provider)
        || !node
            .store
            .membership_authorization_valid(receipt.authorization, node.node_key.verifying_key())
    {
        node.store.settle_admission_receipt(receipt).await?;
        return Ok(None);
    }
    let secret = node.secret.clone().ok_or("awaiting admission credential")?;
    Ok(Some((node.scope.clone(), secret)))
}

async fn resume_admission_receipt(
    node: &Mutex<NativeNetwork>,
    resources: &ResourceEndpoint,
    receipt: AdmissionReceipt,
    address: EndpointAddr,
) -> Result<(), ClientError> {
    if receipt_material(node, receipt, address.id).await?.is_none() {
        return Ok(());
    }
    let provider = address.id;
    let (connection, attempt) = match resources
        .connect_control_attempt(address, ADMISSION_RECEIPT_ALPN, ContactClass::Background)
        .await
    {
        Ok(contact) => contact,
        Err(error) => {
            #[cfg(feature = "replica-test-hooks")]
            if matches!(
                error,
                crate::resources::ResourceContactError::PathBudget(
                    crate::connection_coordinator::ConnectError::DeadlineExceeded
                )
            ) {
                node.lock()
                    .await
                    .store
                    .observe(crate::management_store::RecordCheckpoint::AdmissionReceiptTimedOut);
            }
            return Err(error.into());
        }
    };
    let material = receipt_material(node, receipt, provider).await;
    let result = match material {
        Ok(Some((scope, secret))) => {
            crate::manual_admission::submit_admission_receipt(&connection, receipt, &scope, &secret)
                .await
                .map_err(ClientError::from)
        }
        Ok(None) => {
            connection.close(0u32.into(), b"admission authorization changed");
            return Ok(());
        }
        Err(error) => Err(error),
    };
    connection.close(0u32.into(), b"admission receipt ended");
    if let Err(error) = result {
        attempt.failed(error.to_string());
        return Err(error);
    }
    attempt.succeeded();
    let node = node.lock().await;
    node.store.settle_admission_receipt(receipt).await?;
    #[cfg(feature = "replica-test-hooks")]
    node.store
        .observe(crate::management_store::RecordCheckpoint::AdmissionReceiptSettled);
    Ok(())
}

async fn serve_admission(
    connection: ControlOwner,
    node: Arc<Mutex<NativeNetwork>>,
    services: Arc<Mutex<crate::native_services::NativeServices>>,
    files: Arc<crate::native_file_transfer::NativeFiles>,
) -> Result<(), ClientError> {
    let precode = connection.alpn() == PRECODE_REQUEST_ALPN;
    let result = crate::manual_admission::serve_shared_admission(&node, &connection, precode).await;
    // Admission can save membership before secret delivery ends. Publish that
    // same verified view to an already enabled service on either outcome.
    let node = node.lock().await;
    files.update_access(services.lock().await.access(&node).await?);
    drop(node);
    if let Err(error) = result {
        connection.close(0u32.into(), b"client admission failed");
        return Err(error.into());
    }
    connection.revocation_handle().closed().await;
    Ok(())
}

pub(crate) fn path_status(connection: &Connection) -> Value {
    json!({"peer": connection.remote_id().to_string(), "connection": connection.stable_id(),
        "paths": connection.paths().iter().map(|path| json!({"remote": path.remote_addr(),
            "local": format!("{:?}", path.local_addr()), "selected": path.is_selected(),
            "tx_bytes": path.stats().udp_tx.bytes, "rx_bytes": path.stats().udp_rx.bytes})).collect::<Vec<_>>()})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::ResourceConfig;
    use std::net::Ipv6Addr;

    #[tokio::test]
    async fn recent_admission_history_leaves_older_pending_requests_actionable() {
        let work = tempfile::tempdir().unwrap();
        let path = work.path().join("provider");
        let resources = ResourceConfig {
            replacement: Some(Vec::new()),
            ..Default::default()
        };
        let provider = NativeClient::create_with_resources(&path, Ipv4Addr::LOCALHOST, resources)
            .await
            .unwrap();
        provider.export_recovery_card(None).await.unwrap();
        provider.acknowledge_recovery_saved().await.unwrap();
        let applicant = p2panda_core::SigningKey::generate().verifying_key();
        let attempts: Vec<_> = (0..130)
            .map(|number| AdmissionRequest { applicant, request_id: u128::to_be_bytes(number) })
            .collect();
        // The fixture populates the provider's authenticated-receipt ledger.
        // The browser suite also exercises actual provider/applicant exchanges.
        {
            let runtime = provider.runtime();
            let node = runtime.node.lock().await;
            for request in &attempts {
                node.store.record_admission_request(request).await.unwrap();
            }
            node.store.record_admission_request(&attempts[0]).await.unwrap();
        }
        provider.close().await.unwrap();
        let provider = NativeClient::open(&path, Ipv4Addr::LOCALHOST).await.unwrap();
        let history = provider.admission_history().await.unwrap();
        assert_eq!(history.len(), 128);
        assert_eq!(history.first().unwrap().request_id, attempts[129].request_id);
        assert_eq!(history.last().unwrap().request_id, attempts[2].request_id);
        assert_eq!(provider.requests().await.unwrap(), attempts);
        provider.reject(&attempts[0]).await.unwrap();
        assert!(provider.approve(&attempts[0]).await.is_err());
        assert_eq!(provider.approve(&attempts[1]).await.unwrap(), AdmissionStage::Approved);
        assert_eq!(provider.requests().await.unwrap(), attempts[2..]);
        provider.close().await.unwrap();
    }

    #[cfg(feature = "nat-lab-test-hooks")]
    #[tokio::test]
    #[ignore = "requires a disposable network namespace with IPv4 UDP blocked"]
    async fn idle_relay_session_selects_ipv6_before_the_first_product_ping() {
        use crate::{
            native_network::member_fusion_attempt_id,
            resources::{Resource, ResourceTable, ResourceUse},
        };
        use iroh::TransportAddr;
        use tokio::time::{sleep, timeout};

        let mut relay_config = iroh_relay::server::ServerConfig::default();
        relay_config.relay = Some(iroh_relay::server::RelayConfig::new((
            Ipv4Addr::LOCALHOST,
            0,
        )));
        let relay = iroh_relay::server::Server::spawn(relay_config)
            .await
            .unwrap();
        let resources = ResourceConfig {
            udp_direct_disabled: true,
            tcp_direct_disabled: true,
            maintenance: crate::maintenance_policy::MaintenancePolicy {
                exploration_interval_ms: 250,
                quality_interval_ms: 250,
                contact_timeout_ms: 8000,
                ..Default::default()
            },
            ..ResourceConfig::with_defaults(ResourceTable {
                revision: 1,
                resources: vec![Resource {
                    url: relay.http_url().unwrap().to_string(),
                    uses: [ResourceUse::DataRelay].into(),
                    limitation: "owned idle-session relay".into(),
                }],
            })
        };
        let root = tempfile::tempdir().unwrap();
        let provider = NativeClient::create_with_resources(
            root.path().join("provider"),
            Ipv4Addr::LOCALHOST,
            resources.clone(),
        )
        .await
        .unwrap();
        provider
            .export_recovery_card_to(root.path().join("card"), None)
            .await
            .unwrap();
        provider.acknowledge_recovery_saved().await.unwrap();
        let invitation = provider
            .issue_precode(crate::service_policy::now() + 300, Some(1))
            .await
            .unwrap();
        let consumer = NativeClient::join_with_resources(
            root.path().join("consumer"),
            &invitation,
            Ipv4Addr::LOCALHOST,
            resources,
        )
        .await
        .unwrap();
        assert_eq!(
            consumer.request_admission(&invitation).await.unwrap(),
            AdmissionStage::Complete
        );
        let peer_ids = [provider.address().id, consumer.address().id];
        consumer.close().await.unwrap();
        provider.close().await.unwrap();
        for name in ["provider", "consumer"] {
            let node = NativeNetwork::open(root.path().join(name)).await.unwrap();
            let mut resources = node.resource_configuration().unwrap();
            resources.udp_direct_disabled = false;
            node.save_resource_configuration(&resources).unwrap();
            node.close().await;
        }
        let (provider, consumer) = tokio::join!(
            NativeClient::open(root.path().join("provider"), Ipv4Addr::LOCALHOST),
            NativeClient::open(root.path().join("consumer"), Ipv4Addr::LOCALHOST),
        );
        let clients = [provider.unwrap(), consumer.unwrap()];
        let mut originals = Vec::new();
        for (side, client) in clients.iter().enumerate() {
            let connection = timeout(Duration::from_secs(12), async {
                loop {
                    if let Ok(connection) = client.test_member_connection(peer_ids[1 - side]).await {
                        break connection;
                    }
                    sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("normal startup prepares the member without application traffic");
            assert!(connection.initial_data_path().is_relay());
            originals.push(connection);
        }
        let stable_ids: Vec<_> = originals
            .iter()
            .map(|connection| connection.stable_id())
            .collect();
        assert_eq!(
            member_fusion_attempt_id(&originals[0]).unwrap(),
            member_fusion_attempt_id(&originals[1]).unwrap()
        );
        let addresses: Vec<_> = clients
            .iter()
            .map(|client| {
                let bound = client.runtime()
                    .resources
                    .endpoint()
                    .bound_sockets()
                    .into_iter()
                    .find(|address| address.is_ipv6())
                    .unwrap();
                std::net::SocketAddr::from((Ipv6Addr::LOCALHOST, bound.port()))
            })
            .collect();
        for (side, client) in clients.iter().enumerate() {
            client.runtime()
                .resources
                .member_fusion
                .as_ref()
                .unwrap()
                .0
                .learn_ip_candidates(
                    EndpointAddr::new(peer_ids[1 - side]).with_ip_addr(addresses[1 - side]),
                )
                .await;
        }
        timeout(Duration::from_secs(8), async {
            while !originals.iter().enumerate().all(|(side, connection)| {
                connection.paths().iter().any(|path| path.is_selected()
                    && path.remote_addr() == &TransportAddr::Ip(addresses[1 - side])
                    && path.stats().acknowledgment.is_some())
            }) {
                sleep(Duration::from_millis(20)).await;
            }
        }).await.expect("the original idle relay session actively validates and selects IPv6 before its first business stream");
        eprintln!("IDLE_IPV6_BEFORE_BUSINESS stable_ids={stable_ids:?} bound={addresses:?}");
        let reply = clients[1]
            .ping(EndpointAddr::new(peer_ids[0]))
            .await
            .unwrap();
        assert_eq!(reply["reply"], "PONG");
        assert_eq!(reply["transport"]["connection"], stable_ids[1]);
        assert_eq!(
            clients[0]
                .test_member_connection(peer_ids[1])
                .await
                .unwrap()
                .stable_id(),
            stable_ids[0]
        );
        let [provider, consumer] = clients;
        consumer.close().await.unwrap();
        provider.close().await.unwrap();
        relay.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn native_client_pings_a_member_over_the_bound_ipv6_socket() {
        let dir = tempfile::tempdir().unwrap();
        let resources = ResourceConfig {
            replacement: Some(Vec::new()),
            tcp_direct_disabled: true,
            ..ResourceConfig::default()
        };
        let provider = NativeClient::create_with_resources(
            dir.path().join("provider"),
            Ipv4Addr::LOCALHOST,
            resources.clone(),
        )
        .await
        .unwrap();
        provider
            .export_recovery_card_to(dir.path().join("card"), None)
            .await
            .unwrap();
        provider.acknowledge_recovery_saved().await.unwrap();
        let invitation = provider
            .issue_precode(crate::service_policy::now() + 300, Some(1))
            .await
            .unwrap();
        let path = dir.path().join("consumer");
        let consumer =
            NativeClient::join_with_resources(&path, &invitation, Ipv4Addr::LOCALHOST, resources)
                .await
                .unwrap();
        assert_eq!(
            consumer.request_admission(&invitation).await.unwrap(),
            AdmissionStage::Complete
        );
        consumer.close().await.unwrap();
        let consumer = NativeClient::open(&path, Ipv4Addr::LOCALHOST)
            .await
            .unwrap();
        let bound = provider.runtime()
            .resources
            .endpoint()
            .bound_sockets()
            .into_iter()
            .find(|socket| socket.is_ipv6())
            .expect("the native endpoint bound IPv6");
        let actual = std::net::SocketAddr::from((Ipv6Addr::LOCALHOST, bound.port()));
        let peer = provider.address().id;
        let target = EndpointAddr::new(peer).with_ip_addr(actual);
        let reply = tokio::time::timeout(Duration::from_secs(5), consumer.ping(target))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reply["reply"], "PONG");
        assert_eq!(reply["transport"]["peer"], peer.to_string());
        assert_eq!(
            reply["transport"]["initial_data_path"],
            json!({"Ip":actual})
        );
        println!("native-ipv6 {reply}");
        consumer.close().await.unwrap();
        provider.close().await.unwrap();
    }
}

#[cfg(all(test, feature = "nat-lab-test-hooks"))]
#[path = "native_client_dual_stack_tests.rs"]
mod dual_stack_tests;
