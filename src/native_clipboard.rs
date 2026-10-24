//! Native host for the shared clipboard pair and existing provider authority.
use crate::{
    clipboard::{
        self, ClipboardClaim, ClipboardConnection, ClipboardError, ClipboardHost, ClipboardLease,
        ClipboardRequest, ClipboardState,
    },
    data_stream::DataStream,
    native_client::{ClientError, NativeClient},
    native_network::NativeNetwork,
    native_services::NativeServices,
    system_clipboard::SystemClipboard,
};
use iroh::{EndpointAddr, endpoint::Connection};
use p2panda_core::VerifyingKey;
use std::{
    sync::{Arc, Mutex as SyncMutex},
    time::Duration,
};
use tokio::{sync::Mutex, task::JoinSet};

pub(crate) struct NativeClipboard {
    pub state: Arc<ClipboardState>,
    gate: crate::admission_gate::AdmissionGate,
    platform: Mutex<Option<Arc<SystemClipboard>>>,
    setup: Mutex<()>,
    effect: Mutex<()>,
    tasks: Mutex<JoinSet<()>>,
    connection: SyncMutex<Option<Connection>>,
}
pub(crate) struct ClipboardOwner(pub Arc<NativeClipboard>);
impl Drop for ClipboardOwner {
    fn drop(&mut self) {
        self.0.stop_now(true);
    }
}
impl NativeClipboard {
    pub fn enforce_access(&self, access: &crate::service_policy::ServiceAccess) {
        let peers = access.auth.nodes().into_iter().map(|(peer, _)| peer).chain(
            access
                .ledger
                .guests()
                .into_iter()
                .filter_map(|guest| guest.guest),
        );
        let denied = peers
            .filter(|peer| !clipboard::permitted(access, *peer))
            .map(|peer| *peer.as_bytes())
            .collect();
        self.gate.update_service_policy(
            clipboard::CLIPBOARD_ALPN,
            access.guest_pairs_for(crate::service_policy::Service::Clipboard),
            denied,
        );
        if self
            .state
            .status()
            .peer
            .is_some_and(|peer| !clipboard::permitted(access, peer))
        {
            self.stop_now(false);
        }
    }
    pub fn new(gate: crate::admission_gate::AdmissionGate) -> Arc<Self> {
        Arc::new(Self {
            gate,
            state: Arc::new(ClipboardState::new(crate::system_clipboard::capability())),
            platform: Mutex::new(None),
            setup: Mutex::new(()),
            effect: Mutex::new(()),
            tasks: Mutex::new(JoinSet::new()),
            connection: SyncMutex::new(None),
        })
    }
    fn stop_now(&self, disable: bool) {
        self.state.stop(disable);
        if let Some(connection) = self.connection.lock().unwrap().as_ref() {
            connection.close(0u32.into(), b"clipboard pair stopped");
        }
    }
    pub async fn stop(&self, disable: bool) {
        // Cancel first: a setup may own the mutex while awaiting transport.
        // Waiting for that mutex before signalling would miss pending starts.
        self.stop_now(disable);
        let _setup = self.setup.lock().await;
        self.stop_now(disable);
        self.join_stopped().await;
    }
    async fn join_stopped(&self) {
        let mut tasks = self.tasks.lock().await;
        while tasks.join_next().await.is_some() {}
        // Also join any already started system operation before returning.
        let _effect = self.effect.lock().await;
    }
    pub async fn close(&self) {
        self.stop(true).await;
        if let Some(platform) = self.platform.lock().await.take() {
            platform.close().await;
        }
    }
    pub async fn run(
        self: &Arc<Self>,
        connection: Connection,
        node: Arc<Mutex<NativeNetwork>>,
        services: Arc<Mutex<NativeServices>>,
        initiator: bool,
        request: ClipboardRequest,
    ) -> Result<(), ClipboardError> {
        let _setup = self.setup.lock().await;
        let connection = ClipboardConnection::new(connection);
        let peer = VerifyingKey::from_bytes(connection.get().remote_id().as_bytes())
            .map_err(|_| ClipboardError::Denied)?;
        let prepared = self.prepare(peer, node, services, request).await?;
        self.attach(connection, prepared, initiator).await
    }
    async fn prepare(
        self: &Arc<Self>,
        peer: VerifyingKey,
        node: Arc<Mutex<NativeNetwork>>,
        services: Arc<Mutex<NativeServices>>,
        request: ClipboardRequest,
    ) -> Result<PreparedClipboard, ClipboardError> {
        let claim = ClipboardClaim::new(self.state.clone(), peer, request)?;
        let host = NativeClipboardHost {
            service: self.clone(),
            node,
            services,
        };
        let baseline = match host.sample(claim.lease()).await {
            Ok(baseline) => baseline,
            Err(failure) => {
                claim.fail(failure);
                return Err(failure);
            }
        };
        Ok(PreparedClipboard {
            claim,
            host,
            baseline,
        })
    }
    async fn dial(
        self: &Arc<Self>,
        address: EndpointAddr,
        node: Arc<Mutex<NativeNetwork>>,
        services: Arc<Mutex<NativeServices>>,
        resources: &crate::resources::ResourceEndpoint,
    ) -> Result<(), ClipboardError> {
        let request = self.state.request();
        let _setup = self.setup.lock().await;
        let peer =
            VerifyingKey::from_bytes(address.id.as_bytes()).map_err(|_| ClipboardError::Denied)?;
        let prepared = self.prepare(peer, node, services, request).await?;
        let connection = tokio::select! {
            biased;
            _ = prepared.claim.lease().cancellation.cancelled() => return Err(ClipboardError::Stopped),
            result = resources.connect(address, clipboard::CLIPBOARD_ALPN) => match result {
                Ok(connection) => connection,
                Err(_) => { prepared.claim.fail(ClipboardError::Transport); return Err(ClipboardError::Transport); }
            },
        };
        self.attach(ClipboardConnection::new(connection), prepared, true)
            .await
    }
    async fn attach(
        self: &Arc<Self>,
        connection: ClipboardConnection,
        prepared: PreparedClipboard,
        initiator: bool,
    ) -> Result<(), ClipboardError> {
        let PreparedClipboard {
            claim,
            host,
            baseline,
        } = prepared;
        let lease = claim.lease();
        let setup = async {
            let mut stream = if initiator {
                DataStream::open(connection.get()).await
            } else {
                DataStream::accept(connection.get()).await
            }
            .map_err(|_| ClipboardError::Transport)?;
            clipboard::begin(&mut stream, initiator).await?;
            // Recheck current authority and platform after the transport wait.
            // The earlier baseline still detects copies made during setup.
            host.sample(lease).await?;
            Ok::<_, ClipboardError>(stream)
        };
        let stream = tokio::select! {
            biased;
            _ = lease.cancellation.cancelled() => return Err(ClipboardError::Stopped),
            result = tokio::time::timeout(Duration::from_secs(10), setup) => match result.unwrap_or(Err(ClipboardError::Transport)) {
                Ok(stream) => stream,
                Err(failure) => { claim.fail(failure); return Err(failure); }
            },
        };
        let service = self.clone();
        let mut tasks = self.tasks.lock().await;
        while tasks.try_join_next().is_some() {}
        self.state.activated(lease)?;
        let connection = connection.transfer();
        *self.connection.lock().unwrap() = Some(connection.clone());
        self.state.transport(&connection);
        let lease = claim.transfer();
        tasks.spawn(async move {
            let result =
                clipboard::synchronize(&host, &service.state, &lease, stream, baseline).await;
            connection.close(0u32.into(), b"clipboard pair ended");
            service.state.finish(&lease, result);
            let mut current = service.connection.lock().unwrap();
            if current
                .as_ref()
                .is_some_and(|saved| saved.stable_id() == connection.stable_id())
            {
                *current = None;
            }
        });
        Ok(())
    }
}

struct PreparedClipboard {
    claim: ClipboardClaim,
    host: NativeClipboardHost,
    baseline: Option<String>,
}

struct NativeClipboardHost {
    service: Arc<NativeClipboard>,
    node: Arc<Mutex<NativeNetwork>>,
    services: Arc<Mutex<NativeServices>>,
}
impl ClipboardHost for NativeClipboardHost {
    async fn sample(&self, lease: &ClipboardLease) -> Result<Option<String>, ClipboardError> {
        let _effect = self.service.effect.lock().await;
        let node = self.node.lock().await;
        let services = self.services.lock().await;
        let access = services
            .access(&node)
            .await
            .map_err(|_| ClipboardError::Denied)?;
        self.service.state.current(lease)?;
        if !clipboard::permitted(&access, lease.peer) {
            return Err(ClipboardError::Denied);
        }
        let platform = self
            .service
            .platform
            .lock()
            .await
            .clone()
            .ok_or(ClipboardError::Unavailable)?;
        let result = platform.read().await?;
        self.service.state.current(lease)?;
        Ok(result)
    }
    async fn apply(&self, lease: &ClipboardLease, text: &str) -> Result<(), ClipboardError> {
        let _effect = self.service.effect.lock().await;
        let node = self.node.lock().await;
        // Service/management mutations use the same locks. A verified policy
        // change cannot be published in the middle of this platform effect.
        let services = self.services.lock().await;
        let access = services
            .access(&node)
            .await
            .map_err(|_| ClipboardError::Denied)?;
        self.service.state.current(lease)?;
        if !clipboard::permitted(&access, lease.peer) {
            return Err(ClipboardError::Denied);
        }
        let platform = self
            .service
            .platform
            .lock()
            .await
            .clone()
            .ok_or(ClipboardError::Unavailable)?;
        platform.write(text).await?;
        self.service.state.current(lease)?;
        Ok(())
    }
}

impl NativeClient {
    /// Discover and opt into the real clipboard service. This does not select
    /// a peer or start synchronization. No capability starts business by itself.
    pub async fn enable_clipboard(&self) -> Result<serde_json::Value, ClientError> {
        self.require_ready()?;
        let request = self.clipboard.state.request();
        let _setup = self.clipboard.setup.lock().await;
        self.clipboard.state.current_request(request)?;
        if self.clipboard.state.status().enabled && self.clipboard.state.status().capability.read {
            return self.clipboard_status().await;
        }
        if self.clipboard.state.status().peer.is_some() {
            return Err(ClipboardError::Busy.into());
        }
        let discovered = match SystemClipboard::detect() {
            Ok(platform) => Arc::new(platform),
            Err(failure) => {
                self.clipboard.state.capability(
                    crate::clipboard::ClipboardCapability::unavailable(
                        "native",
                        failure.to_string(),
                    ),
                );
                return Err(failure.into());
            }
        };
        // Keep the existing foreground selection owner across disable/enable.
        // Replacing an otherwise usable X11/Wayland adapter would clear the
        // received system selection when its old child is dropped.
        let platform = self
            .clipboard
            .platform
            .lock()
            .await
            .clone()
            .filter(|previous| previous.capability().backend == discovered.capability().backend)
            .unwrap_or(discovered);
        // Exercise actual current read permission, without publishing the value.
        if let Err(failure) = platform.read().await {
            self.clipboard
                .state
                .capability(crate::clipboard::ClipboardCapability::unavailable(
                    platform.capability().backend,
                    failure.to_string(),
                ));
            return Err(failure.into());
        }
        let capability = platform.capability();
        let node = self.node.lock().await;
        self.services
            .lock()
            .await
            .declare_clipboard(&node, &self.files, true, true)
            .await?;
        *self.clipboard.platform.lock().await = Some(platform);
        self.clipboard.state.enable(capability)?;
        self.clipboard_status().await
    }
    pub async fn disable_clipboard(&self) -> Result<(), ClientError> {
        self.clipboard.stop_now(true);
        let _setup = self.clipboard.setup.lock().await;
        self.clipboard.stop_now(true);
        self.clipboard.join_stopped().await;
        let node = self.node.lock().await;
        self.services
            .lock()
            .await
            .declare_clipboard(
                &node,
                &self.files,
                crate::system_clipboard::capability().read,
                false,
            )
            .await
    }
    /// Select one peer whose enabled service accepts the effective ACL. The
    /// peer begins the same duplex pair; neither endpoint broadcasts changes.
    pub async fn start_clipboard(
        &self,
        address: EndpointAddr,
    ) -> Result<serde_json::Value, ClientError> {
        self.require_ready()?;
        self.clipboard
            .dial(
                address,
                self.node.clone(),
                self.services.clone(),
                &self.resources,
            )
            .await?;
        self.clipboard_status().await
    }
    pub async fn stop_clipboard(&self) {
        self.clipboard.stop(false).await;
    }
    pub async fn clipboard_status(&self) -> Result<serde_json::Value, ClientError> {
        self.clipboard
            .state
            .capability(crate::system_clipboard::current_capability().await);
        if let Some(connection) = self.clipboard.connection.lock().unwrap().as_ref() {
            self.clipboard.state.transport(connection);
        }
        Ok(serde_json::to_value(self.clipboard.state.status())?)
    }
}
