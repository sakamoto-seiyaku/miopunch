//! Browser terminal clients use the same wire and admission rules as Native.
use super::*;
use crate::terminal::{self, TerminalInput, TerminalOutput, TerminalRequest};
use futures_util::FutureExt;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::{Rc, Weak},
};
use tokio::sync::{Mutex, Notify, mpsc};
use tokio_util::sync::CancellationToken;

const MAX_OPERATIONS: usize = 8;
#[derive(Default)]
pub(super) struct BrowserTerminal {
    next: Cell<u64>,
    closed: Cell<bool>,
    active: RefCell<BTreeMap<u64, (CancellationToken, Option<Connection>)>>,
    attachments: RefCell<BTreeMap<u64, Rc<Attachment>>>,
    joined: Notify,
    connecting: Mutex<()>,
}
pub(super) struct TerminalOwner(pub Rc<BrowserTerminal>);
impl Drop for TerminalOwner {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
struct Operation {
    runtime: Weak<BrowserTerminal>,
    id: u64,
    cancel: CancellationToken,
    connection: Option<Connection>,
    completed: bool,
}
impl Drop for Operation {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(connection) = &self.connection {
            if self.completed {
                connection.close(0u32.into(), b"Terminal operation completed");
            } else {
                connection.close(1u32.into(), b"Terminal operation failed");
            }
        }
        if let Some(runtime) = self.runtime.upgrade() {
            runtime.active.borrow_mut().remove(&self.id);
            runtime.joined.notify_waiters();
        }
    }
}
struct Attachment {
    peer: iroh_base::EndpointId,
    outgoing: mpsc::Sender<TerminalInput>,
    sending: Mutex<()>,
    incoming: Mutex<mpsc::Receiver<TerminalOutput>>,
    cancel: CancellationToken,
    outcome: Rc<Cell<Option<bool>>>,
}
impl BrowserTerminal {
    fn claim(self: &Rc<Self>) -> Result<Operation, JsValue> {
        if self.closed.get() || self.active.borrow().len() >= MAX_OPERATIONS {
            return Err(error("terminal unavailable or busy"));
        }
        let id = self
            .next
            .get()
            .checked_add(1)
            .ok_or_else(|| error("terminal identifier exhausted"))?;
        self.next.set(id);
        let cancel = CancellationToken::new();
        self.active.borrow_mut().insert(id, (cancel.clone(), None));
        Ok(Operation {
            runtime: Rc::downgrade(self),
            id,
            cancel,
            connection: None,
            completed: false,
        })
    }
    fn attach(&self, owner: &mut Operation, connection: Connection) -> Result<(), JsValue> {
        if owner.cancel.is_cancelled() || self.closed.get() {
            connection.close(1u32.into(), b"Terminal cancelled");
            return Err(error("terminal cancelled"));
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
                connection.close(1u32.into(), b"Terminal attachments stopped");
            }
        }
    }
    async fn joined(&self, id: Option<u64>) {
        loop {
            let joined = self.joined.notified();
            if id.map_or(self.active.borrow().is_empty(), |id| {
                !self.active.borrow().contains_key(&id)
            }) {
                return;
            }
            joined.await;
        }
    }
    async fn stop(&self) {
        self.cancel();
        self.attachments.borrow_mut().clear();
        self.joined(None).await;
    }
    pub async fn close(&self) {
        self.closed.set(true);
        self.stop().await;
    }
    fn attachment(&self, id: &str) -> Result<Rc<Attachment>, JsValue> {
        self.attachments
            .borrow()
            .get(&id.parse::<u64>().map_err(error)?)
            .cloned()
            .ok_or_else(|| error("unknown terminal attachment"))
    }
}
impl Drop for BrowserTerminal {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl BrowserNetwork {
    async fn terminal_target(&self, peer: &str) -> Result<iroh_base::EndpointId, JsValue> {
        self.require_device_ready()?;
        self.check_store().await?;
        let target: iroh_base::EndpointId = peer.parse().map_err(error)?;
        if !BrowserGate::permits(
            &self.gate.service_snapshot()?,
            target,
            terminal::TERMINAL_ALPN,
        ) {
            return Err(error("terminal access unavailable"));
        }
        Ok(target)
    }
    async fn terminal_attachment(&self, id: &str) -> Result<Rc<Attachment>, JsValue> {
        self.check_store().await?;
        let attachment = self.terminal.attachment(id)?;
        if !BrowserGate::permits(
            &self.gate.service_snapshot()?,
            attachment.peer,
            terminal::TERMINAL_ALPN,
        ) {
            attachment.cancel.cancel();
            return Err(error("terminal authorization ended"));
        }
        Ok(attachment)
    }
}

#[wasm_bindgen]
impl BrowserNetwork {
    pub fn terminal_status(&self) -> String {
        serde_json::json!({"provider_platform":false,"capable":false,"enabled":false,"closed":self.terminal.closed.get(),"client_available":!self.terminal.closed.get(),
            "active_operations":self.terminal.active.borrow().len(),"attachment_handles":self.terminal.attachments.borrow().len(),
            "max_operations":MAX_OPERATIONS,"queue_capacity":terminal::QUEUE_CAPACITY,"max_chunk_bytes":terminal::MAX_BYTES,
            "detach_ends_workspace":false}).to_string()
    }
    pub async fn stop_terminals(&self) {
        self.terminal.stop().await;
    }
    pub async fn request_terminal(&self, peer: String, request: String) -> Result<String, JsValue> {
        if request.len() > 4096 {
            return Err(error("terminal request too large"));
        }
        let request: TerminalRequest = serde_json::from_str(&request).map_err(error)?;
        request.validate().map_err(error)?;
        if matches!(request, TerminalRequest::Attach { .. }) {
            return Err(error("use attach_terminal for attachment ownership"));
        }
        let target = self.terminal_target(&peer).await?;
        let mut owner = self.terminal.claim()?;
        let cancel = owner.cancel.clone();
        let work = async {
            let connection = {
                let _contact = self.terminal.connecting.lock().await;
                self.connect_target(target, terminal::TERMINAL_ALPN).await?
            };
            self.terminal.attach(&mut owner, connection.clone())?;
            let (_, response) = terminal::open(&connection, &request).await.map_err(error)?;
            owner.completed = true;
            Ok(response.to_string())
        }
        .fuse();
        let stop = cancel.cancelled().fuse();
        futures_util::pin_mut!(work, stop);
        futures_util::select_biased! { _ = stop => Err(error("terminal request stopped")), result = work => result }
    }
    pub async fn attach_terminal(
        &self,
        peer: String,
        workspace: String,
        columns: u16,
        rows: u16,
    ) -> Result<String, JsValue> {
        let request = TerminalRequest::Attach {
            workspace,
            columns,
            rows,
        };
        request.validate().map_err(error)?;
        if self.terminal.attachments.borrow().len() >= MAX_OPERATIONS {
            return Err(error("terminal attachment limit reached"));
        }
        let target = self.terminal_target(&peer).await?;
        let peer = target;
        let mut owner = self.terminal.claim()?;
        let cancel = owner.cancel.clone();
        let (stream, ready) = {
            let work = async {
                let connection = {
                    let _contact = self.terminal.connecting.lock().await;
                    self.connect_target(target, terminal::TERMINAL_ALPN).await?
                };
                self.terminal.attach(&mut owner, connection.clone())?;
                terminal::open(&connection, &request).await.map_err(error)
            }
            .fuse();
            let stop = cancel.cancelled().fuse();
            futures_util::pin_mut!(work, stop);
            futures_util::select_biased! { _ = stop => return Err(error("terminal attach stopped")), result = work => result? }
        };
        if self.terminal.closed.get()
            || cancel.is_cancelled()
            || self.terminal.attachments.borrow().len() >= MAX_OPERATIONS
        {
            return Err(error("terminal attach stopped or handle limit reached"));
        }
        let id = owner.id;
        let (sender, outgoing) = mpsc::channel(terminal::QUEUE_CAPACITY);
        let (incoming, receiver) = mpsc::channel(terminal::QUEUE_CAPACITY);
        let outcome = Rc::new(Cell::new(None));
        self.terminal.attachments.borrow_mut().insert(
            id,
            Rc::new(Attachment {
                peer,
                outgoing: sender,
                sending: Mutex::new(()),
                incoming: Mutex::new(receiver),
                cancel,
                outcome: outcome.clone(),
            }),
        );
        wasm_bindgen_futures::spawn_local(async move {
            let completed = terminal::consume_attachment(
                stream,
                owner
                    .connection
                    .as_ref()
                    .expect("attached terminal connection"),
                &owner.cancel,
                outgoing,
                incoming,
            )
            .await;
            outcome.set(Some(completed));
            owner.completed = completed;
        });
        Ok(serde_json::json!({"id":id.to_string(),"workspace":ready["workspace"],"attached":true,"max_chunk_bytes":terminal::MAX_BYTES}).to_string())
    }
    pub async fn send_terminal(&self, id: String, input: String) -> Result<(), JsValue> {
        if input.len() > terminal::MAX_BYTES * 6 + 1024 {
            return Err(error("terminal input too large"));
        }
        let slot = self.terminal.attachment(&id)?;
        let send = async {
            // Bound preparation as well as the wire queue. A burst of callers
            // must not open one IndexedDB transaction per waiting input.
            let _sending = slot.sending.lock().await;
            let message: TerminalInput = serde_json::from_str(&input).map_err(error)?;
            message.validate().map_err(error)?;
            self.terminal_attachment(&id).await?;
            slot.outgoing.send(message).await.map_err(error)?;
            // WASM tasks wake through microtasks. Give browser timers and UI
            // cancellation a task turn before admitting the next input.
            n0_future::time::sleep(Duration::ZERO).await;
            Ok(())
        }
        .fuse();
        let stop = slot.cancel.cancelled().fuse();
        futures_util::pin_mut!(send, stop);
        futures_util::select_biased! { _ = stop => Err(error("terminal attachment ended")), result = send => result }
    }
    pub async fn read_terminal(&self, id: String, timeout_ms: u32) -> Result<String, JsValue> {
        if timeout_ms > 5000 {
            return Err(error("terminal read wait too long"));
        }
        let slot = self.terminal_attachment(&id).await?;
        if slot.cancel.is_cancelled() && slot.outcome.get() != Some(true) {
            return Ok(
                serde_json::json!({"pending":false,"attached":false,"failed":true}).to_string(),
            );
        }
        let waiting = async { slot.incoming.lock().await.recv().await };
        let response = match n0_future::time::timeout(
            Duration::from_millis(timeout_ms as u64),
            waiting,
        )
        .await
        {
            Err(_) => serde_json::json!({"pending":true,"attached":true}),
            Ok(Some(message)) => {
                serde_json::json!({"pending":false,"attached":!message.detached(),"message":message})
            }
            Ok(None) => {
                serde_json::json!({"pending":false,"attached":false,"failed":slot.outcome.get() != Some(true)})
            }
        };
        Ok(response.to_string())
    }
    pub async fn detach_terminal(&self, id: String) -> Result<String, JsValue> {
        let slot = self.terminal.attachment(&id)?;
        let identifier = id.parse::<u64>().map_err(error)?;
        let _close = slot.cancel.clone().drop_guard();
        let protocol = n0_future::time::timeout(Duration::from_secs(5), async {
            if !slot.cancel.is_cancelled() {
                let _ = self
                    .send_terminal(id.clone(), serde_json::json!({"type":"detach"}).to_string())
                    .await;
            }
            self.terminal.joined(Some(identifier)).await;
        })
        .await;
        if protocol.is_err() {
            slot.cancel.cancel();
        }
        let joined = n0_future::time::timeout(
            Duration::from_secs(1),
            self.terminal.joined(Some(identifier)),
        )
        .await;
        self.terminal.attachments.borrow_mut().remove(&identifier);
        if joined.is_err() {
            return Err(error("terminal cancelled owner did not settle"));
        }
        if protocol.is_err() {
            return Err(error(
                "terminal detach exceeded deadline; workspace retained",
            ));
        }
        Ok(serde_json::json!({"attached":false,"workspace_ended":false,"failed":slot.outcome.get() != Some(true)}).to_string())
    }
    pub async fn cancel_terminal(&self, id: String) -> Result<String, JsValue> {
        let slot = self.terminal.attachment(&id)?;
        let identifier = id.parse::<u64>().map_err(error)?;
        slot.cancel.cancel();
        n0_future::time::timeout(
            Duration::from_secs(1),
            self.terminal.joined(Some(identifier)),
        )
        .await
        .map_err(error)?;
        self.terminal.attachments.borrow_mut().remove(&identifier);
        Ok(
            serde_json::json!({"attached":false,"workspace_ended":false,"cancelled":true})
                .to_string(),
        )
    }
}
