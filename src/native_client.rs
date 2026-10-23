//! Controllable native client over the existing durable node and configured SDK.
//!
//! A host owns one client, calls operations, and awaits `close` before reopening
//! its directory. Invitations are capabilities: pass them through a trusted
//! channel and never include them in diagnostic output.
use std::{
    error::Error,
    net::Ipv4Addr,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use iroh::{EndpointAddr, endpoint::Connection};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{sync::Mutex, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::{
    admission_gate::{ADMISSION_REQUEST_ALPN, PRECODE_REQUEST_ALPN, ProtocolScope},
    control_protocol::{AuthenticatedControl, ControlOwner, OwnedControl},
    manual_admission::{AdmissionRequest, AdmissionStage, ManualExchange, ManualInvitation},
    native_network::{NativeNetwork, NodeStatus},
    precode::Precode,
    resources::ResourceEndpoint,
};

pub type ClientError = Box<dyn Error + Send + Sync>;
const MEMBER_MESSAGE: &[u8] = b"miopunch/client-message/1";

pub use crate::client_invitation::ClientInvitation;
impl ClientInvitation {
    fn material(&self) -> Result<ManualInvitation, ClientError> {
        Ok(ManualInvitation::decode(&self.material_bytes()?)?)
    }
    fn code(&self) -> Result<Option<Precode>, ClientError> {
        self.precode_bytes()?
            .as_ref()
            .map(|bytes| Ok(Precode::decode(bytes)?))
            .transpose()
    }
}

/// Secret-free local state, obtained after verifying the durable SDK records.
pub struct ClientStatus {
    pub node: NodeStatus,
    pub admission: AdmissionStage,
    pub recovery_saved: bool,
    pub manual_rejected: bool,
}

/// A bounded native host. Incoming admission and member exchanges run while the
/// caller controls the client; there is no UI, alternate transport or SSH path.
pub struct NativeClient {
    node: Arc<Mutex<NativeNetwork>>,
    resources: Arc<ResourceEndpoint>,
    files: Arc<crate::native_file_transfer::NativeFiles>,
    _file_owner: crate::native_file_transfer::FileOwner,
    cancel: CancellationToken,
    _drop_cancel: tokio_util::sync::DropGuard,
    listener: JoinHandle<()>,
    recovery_saved: Arc<AtomicBool>,
    manual_rejected: AtomicBool,
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
        let config = crate::client_contact::import_contact(
            node.resource_configuration()?,
            invitation.bootstrap(),
        )?;
        node.begin_precode_completion(&code).await?;
        node.save_resource_configuration(&config)?;
        crate::client_contact::save_invitation(&node, invitation)?;
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
        Self::start(NativeNetwork::open(path).await?, bind).await
    }
    async fn start(node: NativeNetwork, bind: Ipv4Addr) -> Result<Self, ClientError> {
        let manual_rejected = AtomicBool::new(node.manual_request_declined().await?);
        let recovery_saved = Arc::new(AtomicBool::new(
            recovery_state(&node)?.is_none_or(|state| state.saved),
        ));
        node.admission_gate()
            .register_protocol(MEMBER_MESSAGE, ProtocolScope::Member);
        node.admission_gate()
            .register_protocol(crate::file_transfer::FILE_ALPN, ProtocolScope::Member);
        // Dial-only pre-admission invitation delivery. This ALPN is not in
        // this network host's advertised incoming protocol set.
        node.admission_gate().register_protocol(
            crate::invitation_pairing::PAIRING_ALPN,
            ProtocolScope::AdmissionRequest,
        );
        let resources = Arc::new(node.bind_configured_endpoint(bind).await?);
        resources.endpoint().set_alpns(vec![
            ADMISSION_REQUEST_ALPN.to_vec(),
            PRECODE_REQUEST_ALPN.to_vec(),
            MEMBER_MESSAGE.to_vec(),
            crate::file_transfer::FILE_ALPN.to_vec(),
        ]);
        let files = Arc::new(crate::native_file_transfer::NativeFiles::default());
        let node = Arc::new(Mutex::new(node));
        let cancel = CancellationToken::new();
        let listener = {
            let endpoint = resources.endpoint().clone();
            let node = node.clone();
            let cancel = cancel.clone();
            let recovery_saved = recovery_saved.clone();
            let files = files.clone();
            tokio::spawn(async move {
                let mut handlers = tokio::task::JoinSet::new();
                let mut expiry = tokio::time::interval(Duration::from_secs(1));
                loop {
                    tokio::select! {
                        _ = cancel.cancelled() => break,
                        _ = expiry.tick() => files.expire().await,
                        incoming = endpoint.accept() => {
                            let Some(incoming) = incoming else { break };
                            if handlers.len() >= 16 { incoming.refuse(); continue; }
                            let node = node.clone();
                            let recovery_saved = recovery_saved.clone();
                            let files = files.clone();
                            handlers.spawn(async move {
                                // The handshake has a finite budget. A large file has no
                                // total-duration cap: its stream enforces an idle timeout.
                                let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
                                let accepted = tokio::time::timeout_at(deadline, async {
                                    if !recovery_saved.load(Ordering::Acquire) {
                                        incoming.refuse();
                                        return Err("recovery card save confirmation required".into());
                                    }
                                    let mut accepting = incoming.accept()?;
                                    let alpn = accepting.alpn().await?;
                                    Ok::<_, ClientError>((accepting, alpn))
                                }).await;
                                let Ok(Ok((accepting, alpn))) = accepted else { return };
                                if alpn == crate::file_transfer::FILE_ALPN {
                                    if let Ok(Ok(connection)) = tokio::time::timeout_at(deadline, accepting).await {
                                        files.receive(connection);
                                    }
                                    return;
                                }
                                let _ = tokio::time::timeout_at(deadline, async {
                                    if matches!(alpn.as_slice(), ADMISSION_REQUEST_ALPN | PRECODE_REQUEST_ALPN) {
                                        let connection = Arc::new(accepting.into_control().await?);
                                        serve_admission(connection.retain_control(), node).await
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
                while handlers.join_next().await.is_some() {}
            })
        };
        let drop_cancel = cancel.clone().drop_guard();
        Ok(Self {
            node,
            resources,
            _file_owner: crate::native_file_transfer::FileOwner(files.clone()),
            files,
            cancel,
            _drop_cancel: drop_cancel,
            listener,
            recovery_saved,
            manual_rejected,
        })
    }
    pub async fn status(&self) -> ClientStatus {
        let node = self.node.lock().await;
        ClientStatus {
            node: node.status(),
            admission: node.admission_stage(),
            recovery_saved: self.recovery_saved.load(Ordering::Acquire),
            manual_rejected: self.manual_rejected.load(Ordering::Acquire),
        }
    }
    pub fn address(&self) -> EndpointAddr {
        self.resources.addr()
    }
    /// Explicit local opt-in. Membership alone never starts the receiver.
    pub async fn enable_files(&self, directory: impl AsRef<Path>) -> Result<(), ClientError> {
        self.require_ready()?;
        if self.node.lock().await.admission_stage() != AdmissionStage::Complete {
            return Err("membership required".into());
        }
        self.files.enable(directory.as_ref()).await?;
        Ok(())
    }
    pub async fn disable_files(&self) {
        self.files.disable().await;
    }
    pub fn files_enabled(&self) -> bool {
        self.files.enabled()
    }
    pub async fn send_file(
        &self,
        address: EndpointAddr,
        path: impl AsRef<Path>,
    ) -> Result<String, ClientError> {
        self.require_ready()?;
        if self.node.lock().await.admission_stage() != AdmissionStage::Complete {
            return Err("membership required".into());
        }
        Ok(self
            .files
            .send(self.resources.clone(), address, path.as_ref().to_owned())
            .await?)
    }
    pub fn file_tasks(&self) -> Vec<crate::file_transfer::FileTaskStatus> {
        self.files.tasks.status()
    }
    pub fn cancel_file(&self, id: &str) -> Result<bool, ClientError> {
        Ok(self.files.cancel(id)?)
    }
    pub fn disconnect_file(&self, id: &str) -> Result<(), ClientError> {
        Ok(self.files.tasks.disconnect(id)?)
    }
    pub async fn invitation(&self) -> Result<ClientInvitation, ClientError> {
        self.require_ready()?;
        let (address, bootstrap) = self.resources.invitation_contact()?;
        Ok(ClientInvitation::new(
            self.node.lock().await.manual_invitation()?.encode(),
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
        self.require_ready()?;
        let code = self
            .node
            .lock()
            .await
            .issue_precode(expires, max_uses)
            .await?;
        let (address, bootstrap) = self.resources.invitation_contact()?;
        Ok(ClientInvitation::new(
            code.invitation().encode(),
            address,
            Some(code.encode()),
            bootstrap,
        )?)
    }
    pub async fn precode_uses(&self, invitation: &ClientInvitation) -> Result<u64, ClientError> {
        let code = invitation.code()?.ok_or("not a precode")?;
        let status = self.node.lock().await.precode_usage_status(&code).await?;
        if !status.count_trusted {
            return Err("untrusted precode count".into());
        }
        status
            .saved_uses
            .ok_or_else(|| "precode count unavailable".into())
    }
    pub async fn requests(&self) -> Result<Vec<AdmissionRequest>, ClientError> {
        let mut node = self.node.lock().await;
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
    pub async fn reject(&self, request: &AdmissionRequest) -> Result<(), ClientError> {
        self.require_ready()?;
        Ok(self
            .node
            .lock()
            .await
            .reject_manual_admission(request)
            .await?)
    }
    /// Only an explicit user action can start another attempt after rejection.
    pub async fn new_manual_attempt(
        &self,
        invitation: &ClientInvitation,
    ) -> Result<(), ClientError> {
        let mut node = self.node.lock().await;
        if !node.manual_request_declined().await? || invitation.code()?.is_some() {
            return Err("no declined manual request".into());
        }
        node.begin_rejoin(invitation.material()?).await?;
        self.manual_rejected.store(false, Ordering::Release);
        Ok(())
    }
    pub async fn approve(&self, request: &AdmissionRequest) -> Result<AdmissionStage, ClientError> {
        self.require_ready()?;
        Ok(self
            .node
            .lock()
            .await
            .approve_manual_admission(request)
            .await?
            .stage)
    }
    /// Resume the same durable SDK attempt. Complete is returned only after
    /// verified membership and the current admission secret have been saved.
    pub async fn request_admission(
        &self,
        invitation: &ClientInvitation,
    ) -> Result<AdmissionStage, ClientError> {
        self.receive_admission(invitation, true).await
    }
    /// Receive the verified authorization without finishing secret delivery.
    /// Hosts can stop here and resume `request_admission` after reopening. This
    /// state is deliberately never reported as completed admission.
    pub async fn receive_authorization(
        &self,
        invitation: &ClientInvitation,
    ) -> Result<AdmissionStage, ClientError> {
        self.receive_admission(invitation, false).await
    }
    async fn receive_admission(
        &self,
        invitation: &ClientInvitation,
        save: bool,
    ) -> Result<AdmissionStage, ClientError> {
        let code = invitation.code()?;
        if code.is_none() && self.manual_rejected.load(Ordering::Acquire) {
            return Err("request declined; explicit new attempt required".into());
        }
        if let Some(code) = &code {
            self.node
                .lock()
                .await
                .begin_precode_completion(code)
                .await?;
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
                match node.request_manual_admission(&connection).await {
                    Err(crate::manual_admission::ManualAdmissionError::RequestDeclined) => {
                        self.manual_rejected.store(true, Ordering::Release);
                        return Err("request declined; explicit new attempt required".into());
                    }
                    result => result?,
                }
            };
            if let ManualExchange::Materials(delivery) = exchange {
                if save {
                    node.complete_manual_admission(delivery).await?;
                }
            }
            Ok(node.admission_stage())
        }
        .await;
        connection.close(0u32.into(), b"client admission ended");
        if code.is_some() && result.is_ok() {
            self.manual_rejected.store(false, Ordering::Release);
        }
        result
    }
    /// Finite member business exchange through configured SDK admission and
    /// route selection. Diagnostics report the actual selected connection path.
    pub async fn ping(&self, address: EndpointAddr) -> Result<Value, ClientError> {
        self.require_ready()?;
        let connection = self.resources.connect(address, MEMBER_MESSAGE).await?;
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            let (mut send, mut recv) = connection.open_bi().await?;
            send.write_all(b"PING").await?;
            send.finish()?;
            if recv.read_to_end(64).await? != b"PONG" {
                return Err("unexpected member response".into());
            }
            Ok::<_, ClientError>(json!({"reply": "PONG", "transport": path_status(&connection)}))
        })
        .await;
        connection.close(0u32.into(), b"client ping ended");
        result?
    }
    /// Stop this host's listener and handlers, close endpoint resources, then
    /// close the durable store. Identity and authorization remain on disk.
    pub async fn close(self) -> Result<(), ClientError> {
        self.cancel.cancel();
        self.listener.await?;
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
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
struct RecoverySave {
    genesis: p2panda_core::Hash,
    exported: bool,
    saved: bool,
}
fn recovery_state(node: &NativeNetwork) -> Result<Option<RecoverySave>, ClientError> {
    if !node.directory.join("genesis.credential").exists() {
        return Ok(None);
    }
    let state = match std::fs::read(node.directory.join("client.recovery-save")) {
        Ok(bytes) => serde_json::from_slice::<RecoverySave>(&bytes)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => RecoverySave {
            genesis: node.anchor.genesis,
            exported: false,
            saved: false,
        },
        Err(error) => return Err(error.into()),
    };
    if state.genesis != node.anchor.genesis || state.saved && !state.exported {
        return Err("invalid recovery acknowledgment".into());
    }
    Ok(Some(state))
}
fn save_recovery_state(node: &NativeNetwork, state: &RecoverySave) -> Result<(), ClientError> {
    crate::native_files::write_private_atomic(
        &node.directory.join("client.recovery-save"),
        &serde_json::to_vec(state)?,
    )?;
    Ok(())
}
impl NativeClient {
    pub async fn deliver_invitation(
        &self,
        pairing: &crate::invitation_pairing::PairingInfo,
        invitation: &ClientInvitation,
    ) -> Result<(), ClientError> {
        self.require_ready()?;
        if invitation.material_bytes()? != self.node.lock().await.manual_invitation()?.encode() {
            return Err("invitation belongs to another network or contact".into());
        }
        let connection = self
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
                self.resources.endpoint().id(),
                pairing,
                invitation,
            ),
        )
        .await;
        connection.close(0u32.into(), b"invitation delivery ended");
        result??;
        Ok(())
    }
    fn require_ready(&self) -> Result<(), ClientError> {
        if !self.recovery_saved.load(Ordering::Acquire) {
            return Err("recovery card save confirmation required".into());
        }
        Ok(())
    }
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
        let node = self.node.lock().await;
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
        Ok(text)
    }
    /// User confirmation only. No password or read-back test is required.
    /// Retrying this confirmation does not create a new network or credential.
    pub async fn acknowledge_recovery_saved(&self) -> Result<(), ClientError> {
        let node = self.node.lock().await;
        let mut state = recovery_state(&node)?.ok_or("not a network creator")?;
        if !state.exported {
            return Err("export the recovery card first".into());
        }
        state.saved = true;
        save_recovery_state(&node, &state)?;
        self.recovery_saved.store(true, Ordering::Release);
        Ok(())
    }
}
async fn serve_admission(
    connection: ControlOwner,
    node: Arc<Mutex<NativeNetwork>>,
) -> Result<(), ClientError> {
    let result = match connection.alpn() {
        PRECODE_REQUEST_ALPN => node.lock().await.serve_precode_admission(&connection).await,
        ADMISSION_REQUEST_ALPN => node.lock().await.serve_manual_admission(&connection).await,
        _ => return Err("unsupported admission operation".into()),
    };
    if let Err(error) = result {
        connection.close(0u32.into(), b"client admission failed");
        return Err(error.into());
    }
    connection.revocation_handle().closed().await;
    Ok(())
}

fn path_status(connection: &Connection) -> Value {
    json!({"peer": connection.remote_id().to_string(), "connection": connection.stable_id(),
        "paths": connection.paths().iter().map(|path| json!({"remote": path.remote_addr(),
            "local": format!("{:?}", path.local_addr()), "selected": path.is_selected(),
            "tx_bytes": path.stats().udp_tx.bytes, "rx_bytes": path.stats().udp_rx.bytes})).collect::<Vec<_>>()})
}
