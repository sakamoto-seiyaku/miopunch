//! Browser adapter: real focused/permissioned system clipboard and one pair.
use super::*;
use crate::clipboard::{
    self, ClipboardCapability, ClipboardClaim, ClipboardConnection, ClipboardError, ClipboardHost,
    ClipboardLease, ClipboardRequest, ClipboardState,
};
use crate::data_stream::DataStream;
use futures_util::FutureExt;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use tokio::sync::Notify;

#[wasm_bindgen(module = "/src/clipboard.js")]
extern "C" {
    fn clipboard_capable() -> bool;
    #[wasm_bindgen(catch)]
    async fn clipboard_capability() -> Result<String, JsValue>;
    #[wasm_bindgen(catch)]
    async fn clipboard_read() -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch)]
    async fn clipboard_write(text: &str) -> Result<(), JsValue>;
}
pub(super) fn capable() -> bool {
    clipboard_capable()
}
pub(super) async fn capability() -> ClipboardCapability {
    clipboard_capability()
        .await
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_else(|| {
            ClipboardCapability::unavailable(
                "browser/async-clipboard",
                "platform_capability_unavailable",
            )
        })
}
fn read_failure(value: JsValue) -> ClipboardError {
    let message = js_sys::Reflect::get(&value, &JsValue::from_str("message"))
        .ok()
        .and_then(|value| value.as_string());
    if message.as_deref() == Some("clipboard_text_too_large") {
        ClipboardError::TooLarge
    } else {
        ClipboardError::Read
    }
}

pub(super) struct BrowserClipboard {
    pub state: Arc<ClipboardState>,
    gate: BrowserGate,
    cancel: tokio_util::sync::CancellationToken,
    running: Cell<usize>,
    joined: Notify,
    setup: tokio::sync::Mutex<()>,
    connection: RefCell<Option<Connection>>,
}
pub(super) struct ClipboardOwner(pub Rc<BrowserClipboard>);
impl Drop for ClipboardOwner {
    fn drop(&mut self) {
        self.0.cancel.cancel();
        self.0.stop_now(true);
    }
}
struct Running(Rc<BrowserClipboard>);
impl Running {
    fn new(service: &Rc<BrowserClipboard>) -> Self {
        service.running.set(service.running.get() + 1);
        Self(service.clone())
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        self.0.running.set(self.0.running.get() - 1);
        self.0.joined.notify_waiters();
    }
}
impl BrowserClipboard {
    pub fn start(
        endpoint: RestrictedEndpoint,
        gate: BrowserGate,
        incoming: Rc<incoming::IncomingDispatcher>,
        budget: Duration,
    ) -> Rc<Self> {
        let initial = if capable() {
            ClipboardCapability::available("browser/async-clipboard")
        } else {
            ClipboardCapability::unavailable(
                "browser/async-clipboard",
                "secure_clipboard_api_unavailable",
            )
        };
        let service = Rc::new(Self {
            state: Arc::new(ClipboardState::new(initial)),
            gate: gate.clone(),
            cancel: Default::default(),
            running: Cell::new(0),
            joined: Notify::new(),
            setup: tokio::sync::Mutex::new(()),
            connection: RefCell::new(None),
        });
        gate.0.lock().unwrap().clipboard = Some(service.state.clone());
        let owner = Running::new(&service);
        wasm_bindgen_futures::spawn_local(async move {
            let service = owner.0.clone();
            let _owner = owner;
            loop {
                let Ok((accepting, deadline)) = incoming
                    .receive_service(clipboard::CLIPBOARD_ALPN, budget, &service.cancel)
                    .await
                else {
                    if service.cancel.is_cancelled() {
                        break;
                    }
                    continue;
                };
                let request = service.state.request();
                use crate::connection_coordinator::{
                    AttemptContext, AttemptRole, ConnectionCoordinator, MemberPathAdapter,
                    SessionPurpose,
                };
                let attempt = AttemptContext::new(deadline, service.cancel.clone());
                let coordinator = ConnectionCoordinator::new_restricted(
                    endpoint.clone(),
                    MemberPathAdapter::capabilities(&gate, AttemptRole::Responder).permitted,
                    vec![],
                );
                if let Ok(connection) = coordinator
                    .accept_accepting(accepting, SessionPurpose::MemberBusiness, attempt)
                    .await
                {
                    let _ = service.run(connection, request).await;
                }
            }
        });
        service
    }
    fn stop_now(&self, disable: bool) {
        self.state.stop(disable);
        if let Some(connection) = self.connection.borrow().as_ref() {
            connection.close(0u32.into(), b"browser clipboard pair stopped");
        }
    }
    pub async fn stop(&self, disable: bool) {
        self.stop_now(disable);
        let _setup = self.setup.lock().await;
        self.stop_now(disable);
        self.join_pair().await;
    }
    async fn join_pair(&self) {
        while self.state.status().peer.is_some() {
            let joined = self.joined.notified();
            if self.state.status().peer.is_none() {
                break;
            }
            joined.await;
        }
    }
    pub async fn enforce_access(&self) {
        let status = self.state.status();
        if let Some(peer) = status.peer {
            let allowed = self
                .gate
                .service_snapshot()
                .ok()
                .and_then(|saved| saved.service_access().ok())
                .is_some_and(|access| clipboard::permitted(&access, peer));
            if !allowed {
                self.stop(false).await;
            }
        }
    }
    pub async fn close(&self) {
        self.cancel.cancel();
        self.stop(true).await;
        while self.running.get() != 0 {
            let joined = self.joined.notified();
            if self.running.get() == 0 {
                break;
            }
            joined.await;
        }
    }
    async fn run(
        self: &Rc<Self>,
        connection: Connection,
        request: ClipboardRequest,
    ) -> Result<(), JsValue> {
        let _setup = self.setup.lock().await;
        let connection = ClipboardConnection::new(connection);
        let peer =
            VerifyingKey::from_bytes(connection.get().remote_id().as_bytes()).map_err(error)?;
        let prepared = self.prepare(peer, request).await.map_err(error)?;
        self.attach(connection, prepared, false)
            .await
            .map_err(error)
    }
    async fn prepare(
        self: &Rc<Self>,
        peer: VerifyingKey,
        request: ClipboardRequest,
    ) -> Result<PreparedClipboard, ClipboardError> {
        let claim = ClipboardClaim::new(self.state.clone(), peer, request)?;
        let baseline = match self.sample(claim.lease()).await {
            Ok(baseline) => baseline,
            Err(failure) => {
                claim.fail(failure);
                return Err(failure);
            }
        };
        Ok(PreparedClipboard { claim, baseline })
    }
    async fn dial(
        self: &Rc<Self>,
        peer: VerifyingKey,
        connecting: impl std::future::Future<Output = Result<Connection, JsValue>>,
    ) -> Result<(), JsValue> {
        let request = self.state.request();
        let _setup = self.setup.lock().await;
        let prepared = self.prepare(peer, request).await.map_err(error)?;
        let connection = {
            let connecting = connecting.fuse();
            let stop = prepared.claim.lease().cancellation.cancelled().fuse();
            futures_util::pin_mut!(connecting, stop);
            futures_util::select_biased! {
                _ = stop => return Err(error(ClipboardError::Stopped)),
                result = connecting => match result {
                    Ok(connection) => connection,
                    Err(_) => { prepared.claim.fail(ClipboardError::Transport); return Err(error(ClipboardError::Transport)); }
                },
            }
        };
        self.attach(ClipboardConnection::new(connection), prepared, true)
            .await
            .map_err(error)
    }
    async fn attach(
        self: &Rc<Self>,
        connection: ClipboardConnection,
        prepared: PreparedClipboard,
        initiator: bool,
    ) -> Result<(), ClipboardError> {
        let PreparedClipboard { claim, baseline } = prepared;
        let lease = claim.lease();
        let setup = async {
            let mut stream = if initiator {
                DataStream::open(connection.get()).await
            } else {
                DataStream::accept(connection.get()).await
            }
            .map_err(|_| ClipboardError::Transport)?;
            clipboard::begin(&mut stream, initiator).await?;
            self.sample(lease).await?;
            Ok::<_, ClipboardError>(stream)
        };
        let stream = {
            let setup = n0_future::time::timeout(Duration::from_secs(10), setup).fuse();
            let stop = lease.cancellation.cancelled().fuse();
            futures_util::pin_mut!(setup, stop);
            futures_util::select_biased! {
                _ = stop => return Err(ClipboardError::Stopped),
                result = setup => match result.unwrap_or(Err(ClipboardError::Transport)) {
                    Ok(stream) => stream,
                    Err(failure) => { claim.fail(failure); return Err(failure); }
                },
            }
        };
        self.state.activated(lease)?;
        let connection = connection.transfer();
        *self.connection.borrow_mut() = Some(connection.clone());
        self.state.transport(&connection);
        let lease = claim.transfer();
        let owner = Running::new(self);
        wasm_bindgen_futures::spawn_local(async move {
            let service = owner.0.clone();
            let _owner = owner;
            let result =
                clipboard::synchronize(service.as_ref(), &service.state, &lease, stream, baseline)
                    .await;
            connection.close(0u32.into(), b"browser clipboard pair ended");
            service.state.finish(&lease, result);
            *service.connection.borrow_mut() = None;
        });
        Ok(())
    }
    fn check(&self, lease: &ClipboardLease) -> Result<(), ClipboardError> {
        self.state.current(lease)?;
        let saved = self
            .gate
            .service_snapshot()
            .map_err(|_| ClipboardError::Denied)?;
        if !matches!(saved.stage(), "Complete" | "GuestActive")
            || !clipboard::permitted(
                &saved.service_access().map_err(|_| ClipboardError::Denied)?,
                lease.peer,
            )
        {
            return Err(ClipboardError::Denied);
        }
        Ok(())
    }
}

struct PreparedClipboard {
    claim: ClipboardClaim,
    baseline: Option<String>,
}
impl ClipboardHost for BrowserClipboard {
    async fn sample(&self, lease: &ClipboardLease) -> Result<Option<String>, ClipboardError> {
        services::installed(&self.gate)
            .await
            .map_err(|_| ClipboardError::Denied)?;
        let current = capability().await;
        if !current.read || !current.write {
            return Err(ClipboardError::Unavailable);
        }
        self.check(lease)?;
        let text = clipboard_read().await.map_err(read_failure)?;
        self.check(lease)?;
        if text.is_null() {
            Ok(None)
        } else {
            text.as_string().map(Some).ok_or(ClipboardError::Read)
        }
    }
    async fn apply(&self, lease: &ClipboardLease, text: &str) -> Result<(), ClipboardError> {
        services::installed(&self.gate)
            .await
            .map_err(|_| ClipboardError::Denied)?;
        let current = capability().await;
        if !current.read || !current.write {
            return Err(ClipboardError::Unavailable);
        }
        self.check(lease)?;
        clipboard_write(text)
            .await
            .map_err(|_| ClipboardError::Write)?;
        self.check(lease)?;
        Ok(())
    }
}

#[wasm_bindgen]
impl BrowserNetwork {
    pub async fn clipboard_status(&self) -> Result<String, JsValue> {
        if let Some(connection) = self.clipboard.connection.borrow().as_ref() {
            self.clipboard.state.transport(connection);
        }
        let mut status = self.clipboard.state.status();
        status.capability = capability().await;
        serde_json::to_string(&status).map_err(error)
    }
    pub async fn enable_clipboard(&self) -> Result<String, JsValue> {
        self.require_device_ready()?;
        let request = self.clipboard.state.request();
        let _setup = self.clipboard.setup.lock().await;
        self.clipboard
            .state
            .current_request(request)
            .map_err(error)?;
        if self.clipboard.state.status().enabled {
            return self.clipboard_status().await;
        }
        let current = capability().await;
        if !current.read || !current.write {
            return Err(error(ClipboardError::Unavailable));
        }
        clipboard_read()
            .await
            .map_err(|failure| error(read_failure(failure)))?;
        let (saved, raw, token) = services::installed(&self.gate).await?;
        let next = saved.declare_clipboard(current.read && current.write, true)?;
        services::commit_installed(&self.gate, &raw, token, &next).await?;
        self.clipboard.state.enable(current).map_err(error)?;
        self.clipboard_status().await
    }
    pub async fn disable_clipboard(&self) -> Result<String, JsValue> {
        self.clipboard.stop_now(true);
        let _setup = self.clipboard.setup.lock().await;
        self.clipboard.stop_now(true);
        self.clipboard.join_pair().await;
        let (saved, raw, token) = services::installed(&self.gate).await?;
        let current = capability().await;
        let next = saved.declare_clipboard(current.read && current.write, false)?;
        services::commit_installed(&self.gate, &raw, token, &next).await?;
        self.clipboard_status().await
    }
    pub async fn start_clipboard(&self, target: String) -> Result<String, JsValue> {
        self.require_device_ready()?;
        let target: iroh_base::EndpointId = target.parse().map_err(error)?;
        let peer = VerifyingKey::from_bytes(target.as_bytes()).map_err(error)?;
        self.clipboard
            .dial(peer, self.connect_target(target, clipboard::CLIPBOARD_ALPN))
            .await?;
        self.clipboard_status().await
    }
    pub async fn stop_clipboard(&self) -> Result<String, JsValue> {
        self.clipboard.stop(false).await;
        self.clipboard_status().await
    }
}
