//! Real Blob/OPFS file operations for the independently identified WASM node.
use super::*;
use crate::{
    data_stream::{CHUNK_BYTES, DataStream},
    file_transfer::{
        self, ACTIVE_TASKS, FileError, FileFuture, FileOffer, FileReceipt, FileSink, FileSource,
        FileState, FileTaskStatus, FileTasks, PartialFile, Reply,
    },
};
use futures_util::FutureExt;
use sha2::{Digest, Sha256};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
};
use tokio::sync::Notify;

#[wasm_bindgen(module = "/src/file_storage.js")]
extern "C" {
    fn file_capable() -> bool;
    #[wasm_bindgen(catch)]
    fn file_metadata(file: &JsValue) -> Result<String, JsValue>;
    #[wasm_bindgen(catch)]
    async fn file_slice(file: &JsValue, offset: f64, length: u32) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch)]
    async fn files_enable(namespace: &str) -> Result<(), JsValue>;
    #[wasm_bindgen(catch)]
    async fn sink_open(namespace: &str, saved: &str, id: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch)]
    async fn sink_write(
        sink: &JsValue,
        offset: f64,
        bytes: &js_sys::Uint8Array,
    ) -> Result<(), JsValue>;
    #[wasm_bindgen(catch)]
    async fn sink_save(sink: &JsValue) -> Result<(), JsValue>;
    #[wasm_bindgen(catch)]
    async fn sink_abort(sink: &JsValue) -> Result<(), JsValue>;
    #[wasm_bindgen(catch)]
    async fn received_file(namespace: &str, name: &str) -> Result<JsValue, JsValue>;
}
pub(super) fn capable() -> bool {
    file_capable()
}

struct BrowserSource(JsValue);
impl FileSource for BrowserSource {
    fn read_at(&mut self, offset: u64, length: usize) -> FileFuture<'_, Vec<u8>> {
        Box::pin(async move {
            let bytes = file_slice(&self.0, offset as f64, length as u32)
                .await
                .map_err(|_| FileError::SourceFailed)?;
            Ok(js_sys::Uint8Array::new(&bytes).to_vec())
        })
    }
}

struct BrowserSink(Option<JsValue>);
impl BrowserSink {
    async fn abort(&mut self) -> Result<(), FileError> {
        if let Some(sink) = self.0.take() {
            sink_abort(&sink).await.map_err(|_| FileError::SaveFailed)?;
        }
        Ok(())
    }
}
impl Drop for BrowserSink {
    fn drop(&mut self) {
        if let Some(sink) = self.0.take() {
            wasm_bindgen_futures::spawn_local(async move {
                let _ = sink_abort(&sink).await;
            });
        }
    }
}
impl FileSink for BrowserSink {
    fn write(&mut self, offset: u64, bytes: Vec<u8>) -> FileFuture<'_, ()> {
        Box::pin(async move {
            sink_write(
                self.0.as_ref().ok_or(FileError::SaveFailed)?,
                offset as f64,
                &js_sys::Uint8Array::from(bytes.as_slice()),
            )
            .await
            .map_err(|_| FileError::SaveFailed)
        })
    }
    fn save(&mut self) -> FileFuture<'_, ()> {
        Box::pin(async move {
            sink_save(self.0.as_ref().ok_or(FileError::SaveFailed)?)
                .await
                .map_err(|_| FileError::SaveFailed)?;
            self.0 = None;
            Ok(())
        })
    }
    fn discard(&mut self) -> FileFuture<'_, ()> {
        Box::pin(self.abort())
    }
}

type Partial = Rc<tokio::sync::Mutex<PartialFile<BrowserSink>>>;

pub(super) struct BrowserFiles {
    pub(super) enabled: Cell<bool>,
    gate: BrowserGate,
    namespace: String,
    partials: RefCell<BTreeMap<String, Partial>>,
    pub(super) tasks: FileTasks,
    cancel: tokio_util::sync::CancellationToken,
    running: Cell<usize>,
    joined: Notify,
    connections: RefCell<Vec<Connection>>,
    generation: Cell<u64>,
}

pub(super) struct FileOwner(pub Rc<BrowserFiles>);
impl Drop for FileOwner {
    fn drop(&mut self) {
        self.0.cancel.cancel();
        self.0.tasks.cancel_all(false);
        for connection in self.0.connections.borrow().iter() {
            connection.close(1u32.into(), b"browser file host dropped");
        }
    }
}

struct Running(Rc<BrowserFiles>);
impl Running {
    fn new(service: &Rc<BrowserFiles>) -> Self {
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

impl BrowserFiles {
    pub fn start(
        endpoint: RestrictedEndpoint,
        gate: BrowserGate,
        incoming: Rc<incoming::IncomingDispatcher>,
        budget: Duration,
    ) -> Rc<Self> {
        let service = Rc::new(Self {
            enabled: Cell::new(false),
            gate: gate.clone(),
            namespace: endpoint.id().to_string(),
            partials: RefCell::new(BTreeMap::new()),
            tasks: FileTasks::default(),
            cancel: tokio_util::sync::CancellationToken::new(),
            running: Cell::new(0),
            joined: Notify::new(),
            connections: RefCell::new(Vec::new()),
            generation: Cell::new(0),
        });
        for _ in 0..ACTIVE_TASKS {
            let owner = Running::new(&service);
            let endpoint = endpoint.clone();
            let gate = gate.clone();
            let incoming = incoming.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let service = owner.0.clone();
                let _owner = owner;
                loop {
                    let work = async {
                        service.expire().await;
                        let (accepting, deadline) = n0_future::time::timeout(
                            Duration::from_secs(60),
                            incoming.receive_service(
                                file_transfer::FILE_ALPN,
                                budget,
                                &service.cancel,
                            ),
                        )
                        .await
                        .map_err(|_| error("file service maintenance tick"))??;
                        let attempt = crate::connection_coordinator::AttemptContext::new(
                            deadline,
                            service.cancel.clone(),
                        );
                        use crate::connection_coordinator::{
                            AttemptRole, MemberPathAdapter, SessionPurpose,
                        };
                        let caps = gate.capabilities(AttemptRole::Responder);
                        let coordinator =
                            crate::connection_coordinator::ConnectionCoordinator::new_restricted(
                                endpoint.clone(),
                                caps.permitted,
                                vec![],
                            );
                        let connection = coordinator
                            .accept_accepting(accepting, SessionPurpose::MemberBusiness, attempt)
                            .await
                            .map_err(error)?;
                        gate.qualified_snapshot(&connection).await?;
                        Ok::<_, JsValue>(connection)
                    }
                    .fuse();
                    let stopped = service.cancel.cancelled().fuse();
                    futures_util::pin_mut!(work, stopped);
                    let result = futures_util::select_biased! { _ = stopped => break, result = work => result };
                    if let Ok(connection) = result {
                        let _connection = RequestConnection(Some(connection.clone()));
                        service.connections.borrow_mut().push(connection.clone());
                        service.receive(connection.clone()).await;
                        service
                            .connections
                            .borrow_mut()
                            .retain(|item| item.stable_id() != connection.stable_id());
                    }
                    if service.cancel.is_cancelled() {
                        break;
                    }
                }
            });
        }
        service
    }

    async fn expire(&self) {
        let expired: Vec<_> = self
            .partials
            .borrow()
            .keys()
            .filter(|id| self.tasks.expired(id))
            .cloned()
            .collect();
        for id in expired {
            let partial = self.partials.borrow().get(&id).cloned();
            if let Some(partial) = partial {
                let mut partial = partial.lock().await;
                if self.tasks.expired(&id) {
                    file_transfer::discard_partial(
                        &mut *partial,
                        &self.tasks,
                        FileError::Interrupted,
                    )
                    .await;
                    self.partials.borrow_mut().remove(&id);
                }
            }
        }
    }

    pub(super) fn request_cancel(self: &Rc<Self>, id: &str) -> Result<bool, FileError> {
        let requested = self.tasks.cancel(id)?;
        if requested
            && self
                .tasks
                .status_for(id)
                .is_some_and(|status| status.direction == "receive")
        {
            let owner = Running::new(self);
            let id = id.to_owned();
            wasm_bindgen_futures::spawn_local(async move {
                let service = owner.0.clone();
                let _owner = owner;
                let partial = service.partials.borrow().get(&id).cloned();
                if let Some(partial) = partial {
                    file_transfer::discard_partial(
                        &mut *partial.lock().await,
                        &service.tasks,
                        FileError::Cancelled,
                    )
                    .await;
                    service.partials.borrow_mut().remove(&id);
                }
            });
        }
        Ok(requested)
    }

    async fn prepare(
        &self,
        offer: &FileOffer,
        peer: &str,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<Partial, FileError> {
        offer.validate()?;
        let generation = self.generation.get();
        if !self.enabled.get() {
            return Err(FileError::ServiceDisabled);
        }
        let access = self
            .gate
            .service_snapshot()
            .and_then(|saved| saved.service_access())
            .map_err(|_| FileError::Rejected)?;
        if !peer
            .parse()
            .ok()
            .is_some_and(|peer| access.can_receive(peer))
        {
            return Err(FileError::Rejected);
        }
        self.expire().await;
        if !self.enabled.get() || generation != self.generation.get() {
            return Err(FileError::ServiceDisabled);
        }
        let existing = self.partials.borrow().get(&offer.id).cloned();
        if let Some(partial) = existing {
            let matches = {
                let file = partial.lock().await;
                file.offer == *offer && file.peer == peer
            };
            return if matches {
                Ok(partial)
            } else {
                Err(FileError::Rejected)
            };
        }
        if self.partials.borrow().len() >= file_transfer::TASK_HISTORY {
            let completed = self.partials.borrow().iter().find_map(|(id, partial)| {
                partial
                    .try_lock()
                    .ok()
                    .filter(|file| file.receipt.is_some())
                    .map(|_| id.clone())
            });
            if let Some(id) = completed {
                self.partials.borrow_mut().remove(&id);
            } else {
                return Err(FileError::Busy);
            }
        }
        let sink = match sink_open(&self.namespace, &offer.saved_name(), &offer.id).await {
            Ok(sink)
                if !cancel.is_cancelled()
                    && self.enabled.get()
                    && generation == self.generation.get() =>
            {
                BrowserSink(Some(sink))
            }
            Ok(sink) => {
                let mut sink = BrowserSink(Some(sink));
                let reason = file_transfer::reject_prepared(
                    &mut sink,
                    &self.tasks,
                    &offer.id,
                    FileError::Cancelled,
                )
                .await;
                return Err(reason);
            }
            Err(_) => {
                return Err(FileError::SaveFailed);
            }
        };
        let partial = Rc::new(tokio::sync::Mutex::new(PartialFile::new(
            offer.clone(),
            peer.into(),
            sink,
        )));
        self.partials
            .borrow_mut()
            .insert(offer.id.clone(), partial.clone());
        Ok(partial)
    }

    async fn reject_receive(&self, offer: &FileOffer, peer: &str, reason: FileError) {
        let partial = self.partials.borrow().get(&offer.id).cloned();
        if let Some(partial) = partial {
            let remove = file_transfer::reject_owned_partial(
                &mut *partial.lock().await,
                &self.tasks,
                offer,
                peer,
                reason,
            )
            .await;
            if remove {
                self.partials.borrow_mut().remove(&offer.id);
            }
        } else {
            self.tasks.fail_receive(&offer.id, reason);
        }
    }

    async fn receive(&self, connection: Connection) {
        let mut stream = match DataStream::accept(&connection).await {
            Ok(stream) => stream,
            Err(_) => return,
        };
        let offer = match stream.read_control::<FileOffer>().await {
            Ok(offer) => offer,
            Err(_) => return,
        };
        let peer = connection.remote_id().to_string();
        let allowed = self
            .gate
            .service_snapshot()
            .and_then(|saved| saved.service_access())
            .ok()
            .is_some_and(|access| {
                peer.parse()
                    .ok()
                    .is_some_and(|peer| access.can_receive(peer))
            });
        if !self.enabled.get() || !allowed {
            let reason = if self.enabled.get() {
                FileError::Rejected
            } else {
                FileError::ServiceDisabled
            };
            let _ = stream.write_control(&Reply::Failed { reason }).await;
            let _ = stream.finish();
            wait_for_sender(&connection).await;
            return;
        }
        let needs_approval = self.enabled.get()
            && self
                .gate
                .service_snapshot()
                .and_then(|saved| saved.service_access())
                .ok()
                .is_some_and(|access| {
                    peer.parse()
                        .ok()
                        .is_some_and(|peer| !access.member(peer) && access.can_receive(peer))
                });
        let reservation = if needs_approval {
            self.tasks.begin_guest_receive(&offer, &peer)
        } else {
            self.tasks.begin_receive(&offer, &peer)
        };
        let mut reservation = match reservation {
            Ok(reservation) => reservation,
            Err(reason) => {
                let _ = stream.write_control(&Reply::Failed { reason }).await;
                let _ = stream.finish();
                wait_for_sender(&connection).await;
                return;
            }
        };
        if let Err(reason) = reservation.wait_for_approval(&connection).await {
            self.reject_receive(&offer, &peer, reason).await;
            let _ = stream.write_control(&Reply::Failed { reason }).await;
            let _ = stream.finish();
            wait_for_sender(&connection).await;
            return;
        }
        if let Some(receipt) = reservation.completed_receipt() {
            let _ = stream.write_control(&Reply::Saved { receipt }).await;
            let _ = stream.finish();
            wait_for_sender(&connection).await;
            return;
        }
        reservation.preparing();
        let partial = match self.prepare(&offer, &peer, &reservation.token()).await {
            Ok(partial) => partial,
            Err(reason) => {
                self.reject_receive(&offer, &peer, reason).await;
                let _ = stream.write_control(&Reply::Failed { reason }).await;
                let _ = stream.finish();
                wait_for_sender(&connection).await;
                return;
            }
        };
        let mut partial = partial.lock().await;
        if partial.offer != offer || partial.peer != connection.remote_id().to_string() {
            let remove = file_transfer::reject_owned_partial(
                &mut partial,
                &self.tasks,
                &offer,
                &peer,
                FileError::Rejected,
            )
            .await;
            drop(partial);
            if remove {
                self.partials.borrow_mut().remove(&offer.id);
            }
            let _ = stream
                .write_control(&Reply::Failed {
                    reason: FileError::Rejected,
                })
                .await;
            let _ = stream.finish();
            wait_for_sender(&connection).await;
            return;
        }
        let id = offer.id.clone();
        let result =
            file_transfer::run_receive(&mut stream, &mut partial, &self.tasks, &connection).await;
        if result.is_err() && result != Err(FileError::Interrupted) {
            self.partials.borrow_mut().remove(&id);
        }
        drop(partial);
        wait_for_sender(&connection).await;
    }

    async fn disable(&self) {
        self.enabled.set(false);
        self.generation.set(self.generation.get().wrapping_add(1));
        self.tasks.cancel_all(true);
        for connection in self.connections.borrow().iter() {
            connection.close(1u32.into(), b"browser file service disabled");
        }
        let partials: Vec<_> = self
            .partials
            .borrow()
            .iter()
            .map(|(id, partial)| (id.clone(), partial.clone()))
            .collect();
        for (id, partial) in partials {
            file_transfer::discard_partial(
                &mut *partial.lock().await,
                &self.tasks,
                FileError::Cancelled,
            )
            .await;
            self.partials.borrow_mut().remove(&id);
        }
    }

    pub async fn close(&self) {
        self.cancel.cancel();
        self.tasks.cancel_all(false);
        for connection in self.connections.borrow().iter() {
            connection.close(1u32.into(), b"browser file host closed");
        }
        loop {
            let joined = self.joined.notified();
            if self.running.get() == 0 {
                break;
            }
            joined.await;
        }
        self.disable().await;
    }
    pub(super) fn enforce_access(self: &Rc<Self>) {
        let Ok(access) = self
            .gate
            .service_snapshot()
            .and_then(|saved| saved.service_access())
        else {
            return;
        };
        for connection in self.connections.borrow().iter() {
            if !VerifyingKey::from_bytes(connection.remote_id().as_bytes())
                .ok()
                .is_some_and(|peer| access.can_receive(peer))
            {
                connection.close(1u32.into(), b"file access revoked");
            }
        }
        for task in self.tasks.status() {
            if !task.state.terminal()
                && !task.peer.parse().ok().is_some_and(|peer| {
                    if task.direction == "receive" {
                        access.can_receive(peer)
                    } else {
                        access.can_send(peer)
                    }
                })
            {
                let _ = self.request_cancel(&task.id);
            }
        }
    }
}

async fn wait_for_sender(connection: &Connection) {
    let _ = n0_future::time::timeout(Duration::from_secs(5), connection.closed()).await;
    connection.close(0u32.into(), b"browser file receive ended");
}

#[wasm_bindgen]
impl BrowserNetwork {
    /// Explicit local opt-in to this node's origin-private file directory.
    pub async fn enable_files(&mut self) -> Result<(), JsValue> {
        self.refresh_service_snapshot().await?;
        if !matches!(self.saved.stage(), "Complete" | "GuestActive") || self.files.enabled.get() {
            return Err(rejected());
        }
        let generation = self.files.generation.get();
        self.check_store().await?;
        files_enable(&self.files.namespace).await?;
        if self.files.cancel.is_cancelled()
            || self.files.enabled.get()
            || generation != self.files.generation.get()
        {
            return Err(rejected());
        }
        let next = self.saved.declare_files(true)?;
        self.commit(next).await?;
        self.files
            .generation
            .set(self.files.generation.get().wrapping_add(1));
        self.files.enabled.set(true);
        Ok(())
    }
    pub async fn disable_files(&mut self) -> Result<(), JsValue> {
        self.files.disable().await;
        self.refresh_service_snapshot().await?;
        let next = self.saved.declare_files(false)?;
        self.commit(next).await
    }
    pub fn file_status(&self) -> String {
        serde_json::json!({"enabled":self.files.enabled.get(), "tasks":self.files.tasks.status(),
            "chunk_bytes":CHUNK_BYTES,"active_limit":ACTIVE_TASKS,"history_limit":file_transfer::TASK_HISTORY}).to_string()
    }
    pub fn cancel_file(&self, task: String) -> Result<bool, JsValue> {
        self.files.request_cancel(&task).map_err(error)
    }
    pub fn disconnect_file(&self, task: String) -> Result<(), JsValue> {
        self.files.tasks.disconnect(&task).map_err(error)
    }
    pub fn approve_file(&self, task: String) -> Result<bool, JsValue> {
        self.files.tasks.decide_file(&task, true).map_err(error)
    }
    pub fn reject_file(&self, task: String) -> Result<bool, JsValue> {
        self.files.tasks.decide_file(&task, false).map_err(error)
    }
    /// Returns a real saved File handle; the host may export it through platform
    /// save UI. This does not allocate or serialize its complete contents.
    pub async fn received_file(&self, name: String) -> Result<JsValue, JsValue> {
        self.check_store().await?;
        received_file(&self.files.namespace, &name).await
    }

    pub async fn send_file(&self, target: String, file: JsValue) -> Result<String, JsValue> {
        #[derive(Deserialize)]
        struct Metadata {
            name: String,
            size: u64,
        }
        self.check_store().await?;
        let target: iroh_base::EndpointId = target.parse().map_err(error)?;
        let saved = self.gate.service_snapshot()?;
        if !matches!(saved.stage(), "Complete" | "GuestActive") {
            return Err(rejected());
        }
        if !saved
            .service_access()?
            .can_send(VerifyingKey::from_bytes(target.as_bytes()).map_err(error)?)
        {
            return Err(rejected());
        }
        let address = self.target_address(target)?;
        let coordinator = self.connection_coordinator();
        let metadata: Metadata = serde_json::from_str(&file_metadata(&file)?).map_err(error)?;
        let id = hex::encode(rand::random::<[u8; 16]>());
        let cancel = self
            .files
            .tasks
            .begin(FileTaskStatus::new(
                id.clone(),
                "send",
                target.to_string(),
                metadata.name.clone(),
                metadata.size,
            ))
            .map_err(error)?;
        let service = self.files.clone();
        let owner = Running::new(&service);
        let gate = self.gate.clone();
        let budget = self.caller_budget;
        let task_id = id.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let _owner = owner;
            let work = async {
                let mut source = BrowserSource(file);
                let mut hash = Sha256::new();
                let mut offset = 0;
                while offset < metadata.size {
                    let length = (metadata.size - offset).min(CHUNK_BYTES as u64) as usize;
                    let bytes = source.read_at(offset, length).await?;
                    if bytes.len() != length {
                        return Err(FileError::SourceFailed);
                    }
                    hash.update(&bytes);
                    offset += length as u64;
                }
                let offer = FileOffer {
                    id: task_id.clone(),
                    name: metadata.name,
                    size: metadata.size,
                    sha256: hex::encode(hash.finalize()),
                };
                offer.validate()?;
                for attempt in 0..3 {
                    let connection = coordinator
                        .connect_alpn(
                            address.clone(),
                            file_transfer::FILE_ALPN,
                            crate::connection_coordinator::AttemptContext::new(
                                n0_future::time::Instant::now() + budget,
                                cancel.clone(),
                            ),
                        )
                        .await
                        .map_err(|_| FileError::Interrupted)?;
                    let _connection = RequestConnection(Some(connection.clone()));
                    gate.qualified_snapshot(&connection)
                        .await
                        .map_err(|_| FileError::Rejected)?;
                    service.tasks.connection(&task_id, Some(&connection));
                    let result = async {
                        let mut stream = DataStream::open(&connection)
                            .await
                            .map_err(|_| FileError::Interrupted)?;
                        file_transfer::send_file(
                            &mut stream,
                            &offer,
                            &mut source,
                            |bytes, resumed| {
                                service.tasks.update(&task_id, |status| {
                                    status.state = FileState::Transferring;
                                    status.bytes = bytes;
                                    status.resumed_bytes = status.resumed_bytes.max(resumed);
                                });
                            },
                        )
                        .await
                    }
                    .await;
                    let result = if file_transfer::peer_cancelled(&connection) {
                        Err(FileError::Cancelled)
                    } else {
                        result
                    };
                    connection.close(0u32.into(), b"browser file send ended");
                    service.tasks.connection(&task_id, None);
                    if result != Err(FileError::Interrupted) || attempt == 2 {
                        return result;
                    }
                    service
                        .tasks
                        .update(&task_id, |status| status.state = FileState::Interrupted);
                    n0_future::time::sleep(Duration::from_millis(250)).await;
                }
                Err(FileError::Interrupted)
            }
            .fuse();
            let cancelled = cancel.cancelled().fuse();
            futures_util::pin_mut!(work, cancelled);
            let result = futures_util::select_biased! { _ = cancelled => Err(FileError::Cancelled), result = work => result };
            service.tasks.finish(&task_id, &result);
        });
        Ok(id)
    }
}
