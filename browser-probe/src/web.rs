//! Real Browser Web consumption through the shared protocol and current gate.
//! No host listener/provider, management bridge or second ACL is introduced.
use super::*;
use crate::web::{self, WebRequest, WebResponse, WebSocketMessage};
use futures_util::{FutureExt, future::LocalBoxFuture};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::{Rc, Weak},
};
use tokio::sync::{Mutex, Notify, mpsc};
use tokio_util::sync::CancellationToken;

const MAX_OPERATIONS: usize = 8;
#[derive(Default)]
pub(super) struct BrowserWeb {
    next: Cell<u64>,
    closed: Cell<bool>,
    active: RefCell<BTreeMap<u64, (CancellationToken, Option<Connection>)>>,
    requests: RefCell<BTreeMap<u64, Rc<RequestSlot>>>,
    sockets: RefCell<BTreeMap<u64, Rc<Socket>>>,
    joined: Notify,
    connecting: Mutex<()>,
}
pub(super) struct WebOwner(pub Rc<BrowserWeb>);
impl Drop for WebOwner {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
struct OperationOwner {
    runtime: Weak<BrowserWeb>,
    id: u64,
    cancel: CancellationToken,
    connection: Option<Connection>,
    completed: Cell<bool>,
}
impl Drop for OperationOwner {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(connection) = &self.connection {
            if self.completed.get() {
                connection.close(0u32.into(), b"Web operation completed");
            } else {
                connection.close(1u32.into(), b"Web operation failed");
            }
        }
        if let Some(runtime) = self.runtime.upgrade() {
            runtime.active.borrow_mut().remove(&self.id);
            runtime.joined.notify_waiters();
        }
    }
}
struct Socket {
    outgoing: mpsc::Sender<WebSocketMessage>,
    incoming: Mutex<mpsc::Receiver<WebSocketMessage>>,
    cancel: CancellationToken,
    outcome: Rc<Cell<Option<bool>>>,
}
struct RequestSlot {
    cancel: CancellationToken,
    result: RefCell<Option<Result<WebResponse, String>>>,
    done: Cell<bool>,
    joined: Notify,
}
struct PreparedRequest {
    id: u64,
    cancel: CancellationToken,
    exchange: LocalBoxFuture<'static, Result<WebResponse, JsValue>>,
}
impl BrowserWeb {
    fn claim(self: &Rc<Self>) -> Result<OperationOwner, JsValue> {
        if self.closed.get() || self.active.borrow().len() >= MAX_OPERATIONS {
            return Err(error("Web unavailable or busy"));
        }
        let id = self
            .next
            .get()
            .checked_add(1)
            .ok_or_else(|| error("Web identifier exhausted"))?;
        self.next.set(id);
        let cancel = CancellationToken::new();
        self.active.borrow_mut().insert(id, (cancel.clone(), None));
        Ok(OperationOwner {
            runtime: Rc::downgrade(self),
            id,
            cancel,
            connection: None,
            completed: Cell::new(false),
        })
    }
    fn attach(&self, owner: &mut OperationOwner, connection: Connection) -> Result<(), JsValue> {
        if owner.cancel.is_cancelled() || self.closed.get() {
            connection.close(1u32.into(), b"Web cancelled");
            return Err(error("Web cancelled"));
        }
        self.active
            .borrow_mut()
            .get_mut(&owner.id)
            .ok_or_else(rejected)?
            .1 = Some(connection.clone());
        owner.connection = Some(connection);
        Ok(())
    }
    fn cancel(&self) {
        for (cancel, connection) in self.active.borrow().values() {
            cancel.cancel();
            if let Some(connection) = connection {
                connection.close(1u32.into(), b"Web stopped");
            }
        }
    }
    async fn stop(&self) {
        self.cancel();
        self.sockets.borrow_mut().clear();
        loop {
            let joined = self.joined.notified();
            if self.active.borrow().is_empty() {
                break;
            }
            joined.await;
        }
        self.requests.borrow_mut().clear();
    }
    pub async fn close(&self) {
        self.closed.set(true);
        self.stop().await;
    }
    fn socket(&self, id: &str) -> Result<Rc<Socket>, JsValue> {
        self.sockets
            .borrow()
            .get(&id.parse::<u64>().map_err(error)?)
            .cloned()
            .ok_or_else(|| error("unknown WebSocket handle"))
    }
    async fn socket_joined(&self, id: u64) {
        loop {
            let joined = self.joined.notified();
            if !self.active.borrow().contains_key(&id) {
                return;
            }
            joined.await;
        }
    }
    async fn duplex(
        owner: OperationOwner,
        stream: crate::data_stream::DataStream,
        mut outgoing: mpsc::Receiver<WebSocketMessage>,
        incoming: mpsc::Sender<WebSocketMessage>,
        outcome: Rc<Cell<Option<bool>>>,
    ) {
        let (mut writer, mut reader) = stream.split();
        let sending = async {
            loop {
                let message = outgoing.recv().await.ok_or_else(rejected)?;
                let closed = message.is_close();
                web::write_socket_message(&mut writer, &message)
                    .await
                    .map_err(error)?;
                if closed {
                    return Ok::<_, JsValue>(());
                }
            }
        }
        .fuse();
        let receiving = async {
            loop {
                let message = web::read_socket_message(&mut reader).await.map_err(error)?;
                let closed = message.is_close();
                incoming.send(message).await.map_err(error)?;
                if closed {
                    owner.completed.set(true);
                    return Ok::<_, JsValue>(());
                }
            }
        }
        .fuse();
        let stop = owner.cancel.cancelled().fuse();
        let peer_closed = owner
            .connection
            .as_ref()
            .expect("attached WebSocket connection")
            .closed()
            .fuse();
        futures_util::pin_mut!(sending, receiving, stop, peer_closed);
        // These are persistent directional futures, not per-message selects
        // that could discard a partially read shared wire record.
        let completed = futures_util::select_biased! {
            _ = stop => false,
            result = receiving => result.is_ok(),
            _ = peer_closed => false,
            result = sending => {
                if result.is_ok() {
                    let drain = n0_future::time::timeout(Duration::from_secs(5), receiving).fuse();
                    futures_util::pin_mut!(drain);
                    futures_util::select_biased! { _ = stop => false, result = drain => result.is_ok_and(|result| result.is_ok()), _ = peer_closed => false }
                } else { false }
            },
        };
        outcome.set(Some(completed));
    }
}
impl Drop for BrowserWeb {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[wasm_bindgen]
impl BrowserNetwork {
    pub fn web_status(&self) -> String {
        serde_json::json!({"provider":false,"native_listener":false,"available":!self.web.closed.get(),
            "active_operations":self.web.active.borrow().len(),"max_operations":MAX_OPERATIONS,
            "request_handles":self.web.requests.borrow().len(),"max_request_handles":MAX_OPERATIONS,
            "socket_handles":self.web.sockets.borrow().len(),"socket_queue_capacity":16,
            "max_response_bytes":web::MAX_BODY,"max_request_bytes":web::MAX_REQUEST_BODY,
            "max_socket_message_bytes":web::MAX_SOCKET_MESSAGE}).to_string()
    }
    pub async fn stop_web(&self) {
        self.web.stop().await;
    }
    pub async fn request_web(&self, peer: String, request: String) -> Result<String, JsValue> {
        let response = self
            .prepare_web_request(peer, request)
            .await?
            .exchange
            .await?;
        serde_json::to_string(&response).map_err(error)
    }
    /// Own one HTTP request without keeping the caller's control Promise pending.
    /// Completed responses retain a bounded handle until consumed or cancelled.
    pub async fn begin_web_request(
        &self,
        peer: String,
        request: String,
    ) -> Result<String, JsValue> {
        let prepared = self.prepare_web_request(peer, request).await?;
        if self.web.requests.borrow().len() >= MAX_OPERATIONS {
            return Err(error("Web request handle limit reached"));
        }
        let PreparedRequest {
            id,
            cancel,
            exchange,
        } = prepared;
        let slot = Rc::new(RequestSlot {
            cancel,
            result: RefCell::new(None),
            done: Cell::new(false),
            joined: Notify::new(),
        });
        self.web.requests.borrow_mut().insert(id, slot.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let result = exchange.await;
            *slot.result.borrow_mut() = Some(result.map_err(|_| "Web request failed".to_string()));
            slot.done.set(true);
            slot.joined.notify_waiters();
        });
        Ok(serde_json::json!({"id":id.to_string(),"pending":true}).to_string())
    }
    /// Consume only this completed handle. Unknown or already consumed IDs fail.
    pub fn web_request_result(&self, id: String) -> Result<String, JsValue> {
        let id = id.parse::<u64>().map_err(error)?;
        let mut requests = self.web.requests.borrow_mut();
        let slot = requests
            .get(&id)
            .ok_or_else(|| error("unknown Web request handle"))?;
        if !slot.done.get() {
            return Ok(serde_json::json!({"pending":true}).to_string());
        }
        let slot = requests.remove(&id).expect("checked Web request handle");
        drop(requests);
        let result = slot
            .result
            .borrow_mut()
            .take()
            .ok_or_else(|| error("Web request result unavailable"))?;
        match result {
            Ok(response) => {
                Ok(serde_json::json!({"pending":false,"response":response}).to_string())
            }
            Err(failure) => Err(error(failure)),
        }
    }
    /// Cancel and join this request owner; the node and other requests stay live.
    pub async fn cancel_web_request(&self, id: String) -> Result<String, JsValue> {
        let slot = self
            .web
            .requests
            .borrow_mut()
            .remove(&id.parse::<u64>().map_err(error)?)
            .ok_or_else(|| error("unknown Web request handle"))?;
        slot.cancel.cancel();
        n0_future::time::timeout(Duration::from_secs(5), async {
            loop {
                let notified = slot.joined.notified();
                if slot.done.get() {
                    break;
                }
                notified.await;
            }
        })
        .await
        .map_err(|_| error("Web request cancelled owner did not finish"))?;
        Ok(serde_json::json!({"cancelled":true,"joined":true}).to_string())
    }
    pub async fn open_web_socket(
        &self,
        peer: String,
        url: String,
        headers: String,
    ) -> Result<String, JsValue> {
        self.require_device_ready()?;
        self.check_store().await?;
        if self.web.sockets.borrow().len() >= MAX_OPERATIONS
            || url.len() > 8192
            || headers.len() > web::MAX_HEADERS
        {
            return Err(error("WebSocket unavailable or input too large"));
        }
        let peer = peer.parse().map_err(error)?;
        let headers = serde_json::from_str(&headers).map_err(error)?;
        let mut owner = self.web.claim()?;
        let cancel = owner.cancel.clone();
        let (stream, ready) = {
            let opening = async {
                let connection = {
                    let _contact = self.web.connecting.lock().await;
                    self.connect_target(peer, web::WEB_ALPN).await?
                };
                self.web.attach(&mut owner, connection.clone())?;
                web::open_socket(&connection, url, headers)
                    .await
                    .map_err(error)
            }
            .fuse();
            let stop = cancel.cancelled().fuse();
            futures_util::pin_mut!(opening, stop);
            futures_util::select_biased! {
                _ = stop => return Err(error("WebSocket opening stopped")),
                result = opening => result?,
            }
        };
        if self.web.closed.get() || self.web.sockets.borrow().len() >= MAX_OPERATIONS {
            return Err(error("WebSocket opening stopped or handle limit reached"));
        }
        let (sender, outgoing) = mpsc::channel(16);
        let (incoming, receiver) = mpsc::channel(16);
        let id = owner.id;
        let outcome = Rc::new(Cell::new(None));
        self.web.sockets.borrow_mut().insert(
            id,
            Rc::new(Socket {
                outgoing: sender,
                incoming: Mutex::new(receiver),
                cancel,
                outcome: outcome.clone(),
            }),
        );
        wasm_bindgen_futures::spawn_local(BrowserWeb::duplex(
            owner, stream, outgoing, incoming, outcome,
        ));
        Ok(serde_json::json!({"id":id.to_string(),"headers":ready.headers,"max_message_bytes":web::MAX_SOCKET_MESSAGE}).to_string())
    }
    pub async fn send_web_socket(&self, id: String, message: String) -> Result<(), JsValue> {
        if message.len() > web::MAX_SOCKET_MESSAGE * 6 + 1024 {
            return Err(error("WebSocket message too large"));
        }
        let message: WebSocketMessage = serde_json::from_str(&message).map_err(error)?;
        message.validate().map_err(error)?;
        if matches!(
            &message,
            WebSocketMessage::Close {
                code: 1004 | 1005 | 1006 | 1015,
                ..
            }
        ) {
            return Err(error("reserved WebSocket close code"));
        }
        let socket = self.web.socket(&id)?;
        let send = socket.outgoing.send(message).fuse();
        let stop = socket.cancel.cancelled().fuse();
        futures_util::pin_mut!(send, stop);
        futures_util::select_biased! { _ = stop => Err(error("WebSocket ended")), result = send => result.map_err(error) }
    }
    pub async fn read_web_socket(&self, id: String, timeout_ms: u32) -> Result<String, JsValue> {
        if timeout_ms > 5000 {
            return Err(error("WebSocket read wait too long"));
        }
        let socket = self.web.socket(&id)?;
        let mut receiver = socket.incoming.lock().await;
        let response = match n0_future::time::timeout(
            Duration::from_millis(timeout_ms as u64),
            receiver.recv(),
        )
        .await
        {
            Err(_) => serde_json::json!({"pending":true,"closed":false}),
            Ok(Some(message)) => {
                serde_json::json!({"pending":false,"closed":message.is_close(),"message":message})
            }
            Ok(None) => {
                serde_json::json!({"pending":false,"closed":true,"failed":socket.outcome.get() != Some(true)})
            }
        };
        Ok(response.to_string())
    }
    pub async fn close_web_socket(&self, id: String) -> Result<String, JsValue> {
        let socket = self.web.socket(&id)?;
        let identifier = id.parse::<u64>().map_err(error)?;
        let _close_owner = socket.cancel.clone().drop_guard();
        // Match Native: enqueue and handshake share one protocol deadline;
        // then cancellation joins this exact owner within its cleanup budget.
        let protocol = n0_future::time::timeout(Duration::from_secs(5), async {
            if !socket.cancel.is_cancelled() {
                let _ = self
                    .send_web_socket(
                        id.clone(),
                        serde_json::json!({"type":"close","code":1000,"reason":""}).to_string(),
                    )
                    .await;
            }
            self.web.socket_joined(identifier).await;
        })
        .await;
        if protocol.is_err() {
            socket.cancel.cancel();
        }
        let joined =
            n0_future::time::timeout(Duration::from_secs(1), self.web.socket_joined(identifier))
                .await;
        self.web.sockets.borrow_mut().remove(&identifier);
        if joined.is_err() {
            return Err(error("WebSocket cancelled owner did not finish"));
        }
        if protocol.is_err() {
            return Err(error("WebSocket close handshake did not finish"));
        }
        Ok(
            serde_json::json!({"closed":true,"failed":socket.outcome.get() != Some(true)})
                .to_string(),
        )
    }
}

impl BrowserNetwork {
    async fn prepare_web_request(
        &self,
        peer: String,
        request: String,
    ) -> Result<PreparedRequest, JsValue> {
        self.require_device_ready()?;
        self.check_store().await?;
        if request.len() > web::MAX_REQUEST_BODY * 2 + web::MAX_HEADERS {
            return Err(error("Web request too large"));
        }
        let request: WebRequest = serde_json::from_str(&request).map_err(error)?;
        if request.body.len() > web::MAX_REQUEST_BODY {
            return Err(error("Web request body too large"));
        }
        let peer = peer.parse().map_err(error)?;
        let address = self.target_address(peer)?;
        let coordinator = self.connection_coordinator();
        let budget = self.caller_budget;
        let token = self
            .lease
            .as_ref()
            .ok_or_else(|| error("browser node disconnected"))?
            .0;
        let committed = self.committed.clone();
        let gate = self.gate.clone();
        let runtime = self.web.clone();
        let mut owner = runtime.claim()?;
        let id = owner.id;
        let cancel = owner.cancel.clone();
        let stopping = cancel.clone();
        let exchange = async move {
            let work = async {
                let connection = {
                    let _contact = runtime.connecting.lock().await;
                    // Recheck the same current store after queued contact work,
                    // without borrowing the JS node while this owner is running.
                    let installed = gate
                        .0
                        .lock()
                        .unwrap()
                        .saved
                        .as_ref()
                        .map(serde_json::to_string)
                        .transpose()
                        .map_err(error)?;
                    let actual = read_node(token).await?.as_string();
                    if actual.as_deref() != Some(&committed) && actual != installed {
                        return Err(error(
                            "browser materials cleared or changed; restore or rejoin",
                        ));
                    }
                    coordinator
                        .connect_alpn(
                            address,
                            web::WEB_ALPN,
                            crate::connection_coordinator::AttemptContext::new(
                                n0_future::time::Instant::now() + budget,
                                stopping.clone(),
                            ),
                        )
                        .await
                        .map_err(error)?
                };
                runtime.attach(&mut owner, connection.clone())?;
                let response = web::request(&connection, request).await.map_err(error)?;
                owner.completed.set(true);
                Ok(response)
            }
            .fuse();
            let stop = stopping.cancelled().fuse();
            futures_util::pin_mut!(work, stop);
            futures_util::select_biased! {
                _ = stop => Err(error("Web request stopped")),
                result = work => result,
            }
        }
        .boxed_local();
        Ok(PreparedRequest {
            id,
            cancel,
            exchange,
        })
    }
}
