//! Terminal service ownership, using the existing admitted connection and ACL.
use crate::{
    data_stream::DataStream,
    native_client::{ClientError, NativeClient},
    service_policy::{Service, ServiceAccess},
    terminal::{self, TerminalInput, TerminalOutput, TerminalProvider, TerminalRequest},
    terminal_tmux::{self, ControlClient, Tmux},
};
use iroh::{EndpointAddr, endpoint::Connection};
use p2panda_core::VerifyingKey;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, atomic::Ordering},
    time::Duration,
};
use tokio::{
    io::AsyncWriteExt,
    sync::{Mutex as AsyncMutex, Notify, mpsc},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

const MAX_OPERATIONS: usize = 16;
struct Flow {
    peer: VerifyingKey,
    provider: bool,
    cancel: CancellationToken,
    connection: Option<Connection>,
}
struct Attachment {
    outgoing: mpsc::Sender<TerminalInput>,
    incoming: AsyncMutex<mpsc::Receiver<TerminalOutput>>,
    cancel: CancellationToken,
    outcome: Arc<Mutex<Option<bool>>>,
}
#[derive(Default)]
struct State {
    access: Option<ServiceAccess>,
    backend: Option<Arc<Tmux>>,
    enabled: bool,
    closed: bool,
    next: u64,
    active: BTreeMap<u64, Flow>,
    attachments: BTreeMap<u64, Arc<Attachment>>,
    completed: u64,
    failed: u64,
}
pub(crate) struct NativeTerminal {
    state: Mutex<State>,
    gate: crate::admission_gate::AdmissionGate,
    tasks: AsyncMutex<JoinSet<()>>,
    configuring: AsyncMutex<()>,
    connecting: AsyncMutex<()>,
    joined: Notify,
}
pub(crate) struct TerminalOwner(pub Arc<NativeTerminal>);
impl Drop for TerminalOwner {
    fn drop(&mut self) {
        self.0.cancel_all();
    }
}
struct Operation {
    runtime: Arc<NativeTerminal>,
    id: u64,
    cancel: CancellationToken,
    completed: bool,
}
impl Drop for Operation {
    fn drop(&mut self) {
        let mut state = self.runtime.state.lock().unwrap();
        if let Some(flow) = state.active.remove(&self.id) {
            flow.cancel.cancel();
            if self.completed {
                state.completed = state.completed.saturating_add(1);
            } else {
                state.failed = state.failed.saturating_add(1);
            }
            if let Some(connection) = flow.connection {
                if self.completed {
                    connection.close(0u32.into(), b"Terminal operation completed");
                } else {
                    connection.close(1u32.into(), b"Terminal operation failed");
                }
            }
        }
        self.runtime.joined.notify_waiters();
    }
}

impl NativeTerminal {
    pub fn new(gate: crate::admission_gate::AdmissionGate) -> Arc<Self> {
        Arc::new(Self {
            state: Default::default(),
            gate,
            tasks: Default::default(),
            configuring: Default::default(),
            connecting: Default::default(),
            joined: Default::default(),
        })
    }
    fn permitted(state: &State, access: &ServiceAccess, flow: &Flow) -> bool {
        terminal::peer_permitted(access, flow.peer)
            && (!flow.provider
                || state.enabled
                    && state.backend.is_some()
                    && access.member(access.local)
                    && access.permits_service(Service::Terminal, flow.peer))
    }
    pub fn enforce_access(&self, access: &ServiceAccess) {
        self.gate.update_service_policy(
            terminal::TERMINAL_ALPN,
            access.guest_pairs_for(Service::Terminal),
            Default::default(),
        );
        let mut state = self.state.lock().unwrap();
        for flow in state.active.values() {
            if !Self::permitted(&state, access, flow) {
                flow.cancel.cancel();
                if let Some(connection) = &flow.connection {
                    connection.close(1u32.into(), b"Terminal authorization ended");
                }
            }
        }
        state.access = Some(access.clone());
    }
    fn claim(
        self: &Arc<Self>,
        peer: VerifyingKey,
        provider: bool,
    ) -> Result<Operation, ClientError> {
        let mut state = self.state.lock().unwrap();
        let flow = Flow {
            peer,
            provider,
            cancel: CancellationToken::new(),
            connection: None,
        };
        if state.closed
            || state.active.len() >= MAX_OPERATIONS
            || !state
                .access
                .as_ref()
                .is_some_and(|access| Self::permitted(&state, access, &flow))
        {
            return Err("Terminal unavailable, unauthorized or busy".into());
        }
        state.next = state
            .next
            .checked_add(1)
            .ok_or("Terminal identifier exhausted")?;
        let id = state.next;
        let cancel = flow.cancel.clone();
        state.active.insert(id, flow);
        Ok(Operation {
            runtime: self.clone(),
            id,
            cancel,
            completed: false,
        })
    }
    fn authorize(&self, owner: &Operation) -> Result<Arc<Tmux>, ClientError> {
        let state = self.state.lock().unwrap();
        let flow = state
            .active
            .get(&owner.id)
            .ok_or("terminal operation ended")?;
        if owner.cancel.is_cancelled()
            || !state
                .access
                .as_ref()
                .is_some_and(|access| Self::permitted(&state, access, flow))
        {
            return Err("terminal authorization ended".into());
        }
        state
            .backend
            .clone()
            .ok_or_else(|| "terminal provider not configured".into())
    }
    fn attach_connection(
        &self,
        owner: &Operation,
        connection: Connection,
    ) -> Result<(), ClientError> {
        let mut state = self.state.lock().unwrap();
        if owner.cancel.is_cancelled() {
            connection.close(1u32.into(), b"Terminal cancelled");
            return Err("terminal operation cancelled".into());
        }
        state
            .active
            .get_mut(&owner.id)
            .ok_or("terminal operation ended")?
            .connection = Some(connection);
        Ok(())
    }
    pub fn status(&self) -> Value {
        let state = self.state.lock().unwrap();
        let processes = state
            .backend
            .as_ref()
            .map_or(0, |backend| backend.live_clients.load(Ordering::Acquire));
        let cleanup_unknown = state
            .backend
            .as_ref()
            .is_some_and(|backend| backend.cleanup_unknown.load(Ordering::Acquire));
        json!({"provider_platform":cfg!(any(target_os="linux", windows)),"configured":state.backend.is_some(),
            "capable":state.backend.is_some(),"enabled":state.enabled,"closed":state.closed,"client_available":!state.closed,
            "active_operations":state.active.len(),"attachment_handles":state.attachments.len(),
            "attachment_process_owners":processes,"process_cleanup_unknown":cleanup_unknown,
            "max_operations":MAX_OPERATIONS,"max_workspaces":terminal::MAX_WORKSPACES,
            "queue_capacity":terminal::QUEUE_CAPACITY,"max_chunk_bytes":terminal::MAX_BYTES,
            "completed_operations":state.completed,"failed_operations":state.failed,
            "workspace_lifetime":"tmux","detach_ends_workspace":false})
    }
    fn cancel_all(&self) {
        let state = self.state.lock().unwrap();
        for flow in state.active.values() {
            flow.cancel.cancel();
            if let Some(connection) = &flow.connection {
                connection.close(1u32.into(), b"Terminal attachments stopped");
            }
        }
    }
    async fn joined(&self, id: Option<u64>) {
        loop {
            let joined = self.joined.notified();
            let finished = {
                let state = self.state.lock().unwrap();
                id.map_or(state.active.is_empty(), |id| {
                    !state.active.contains_key(&id)
                })
            };
            if finished {
                return;
            }
            joined.await;
        }
    }
    pub async fn close(&self) -> Result<(), ClientError> {
        self.state.lock().unwrap().closed = true;
        self.cancel_all();
        self.state.lock().unwrap().attachments.clear();
        tokio::time::timeout(Duration::from_secs(7), self.joined(None))
            .await
            .map_err(|_| "terminal owners did not settle")?;
        let mut tasks = self.tasks.lock().await;
        while tasks.join_next().await.is_some() {}
        let state = self.state.lock().unwrap();
        if state.backend.as_ref().is_some_and(|backend| {
            backend.live_clients.load(Ordering::Acquire) != 0
                || backend.cleanup_unknown.load(Ordering::Acquire)
        }) {
            return Err("terminal attachment process settlement unproven".into());
        }
        Ok(())
    }
    pub async fn receive(self: &Arc<Self>, connection: Connection) -> Result<(), ClientError> {
        let peer = VerifyingKey::from_bytes(connection.remote_id().as_bytes())?;
        let mut tasks = self.tasks.lock().await;
        while tasks.try_join_next().is_some() {}
        let owner = match self.claim(peer, true) {
            Ok(owner) => owner,
            Err(error) => {
                connection.close(1u32.into(), b"Terminal unavailable");
                return Err(error);
            }
        };
        self.attach_connection(&owner, connection.clone())?;
        let runtime = self.clone();
        tasks.spawn(async move {
            runtime.serve(owner, connection).await;
        });
        Ok(())
    }
    async fn serve(self: Arc<Self>, mut owner: Operation, connection: Connection) {
        let result = async {
            let (mut stream, request) = tokio::select! { biased;
                _ = owner.cancel.cancelled() => return Err("terminal operation cancelled".into()),
                _ = connection.closed() => return Err("terminal peer closed".into()),
                result = async {
                    let mut stream = DataStream::accept(&connection).await?;
                    let request:TerminalRequest = stream.read_control().await?;
                    request.validate()?;
                    Ok::<_, ClientError>((stream,request))
                } => result?,
            };
            let backend = self.authorize(&owner)?;
            let reply = match request {
                TerminalRequest::Create => json!({"ok":true,"workspace":backend.create(&owner.cancel).await?,"retained":true}),
                TerminalRequest::List => json!({"ok":true,"workspaces":backend.list(&owner.cancel).await?}),
                TerminalRequest::End { workspace } => {
                    backend.end(&workspace, &owner.cancel).await?;
                    json!({"ok":true,"workspace":workspace,"ended":true})
                }
                TerminalRequest::Attach { workspace, columns, rows } => {
                    let mut client = backend.attach(&workspace, columns, rows, &owner.cancel).await?;
                    let response = json!({"ok":true,"workspace":workspace,"attached":true,
                        "max_chunk_bytes":terminal::MAX_BYTES,"queue_capacity":terminal::QUEUE_CAPACITY});
                    let ready = tokio::select! { biased;
                        _ = owner.cancel.cancelled() => Err("terminal attachment cancelled".into()),
                        result = stream.write_control(&response) => result.map_err(Into::into),
                    };
                    let result = if ready.is_ok() { self.serve_attachment(&owner, &connection, stream, &mut client).await }
                                 else { ready };
                    let cleanup = client.close().await;
                    result?;
                    cleanup?;
                    return Ok::<_, ClientError>(());
                }
            };
            tokio::select! { biased;
                _ = owner.cancel.cancelled() => return Err("terminal operation cancelled".into()),
                result = async { stream.write_control(&reply).await?; stream.finish() } => result?,
            }
            let reason = tokio::time::timeout(Duration::from_secs(5), connection.closed()).await?;
            if !matches!(reason, iroh::endpoint::ConnectionError::ApplicationClosed(reason)
                if reason.error_code == 0u32.into() && reason.reason.as_ref() == b"Terminal operation completed") {
                return Err("terminal consumer did not acknowledge operation".into());
            }
            Ok(())
        }.await;
        owner.completed = result.is_ok();
    }
    async fn serve_attachment(
        &self,
        owner: &Operation,
        connection: &Connection,
        stream: DataStream,
        client: &mut ControlClient,
    ) -> Result<(), ClientError> {
        let (mut writer, mut reader) = stream.split();
        let workspace = client.workspace().to_owned();
        let mut input = client.input.take().ok_or("terminal input unavailable")?;
        let mut output = client.output.take().ok_or("terminal output unavailable")?;
        let (events, mut incoming) = mpsc::channel(terminal::QUEUE_CAPACITY);
        let result = {
            let receiving = async {
                loop {
                    let message = terminal::read_input(&mut reader).await?;
                    self.authorize(owner)?;
                    input
                        .write_all(&ControlClient::encode(&workspace, &message)?)
                        .await?;
                    if matches!(message, TerminalInput::Detach) {
                        return Ok::<_, ClientError>(());
                    }
                }
            };
            let reading = async {
                while let Some(bytes) = terminal_tmux::read_output(&mut output).await? {
                    self.authorize(owner)?;
                    for chunk in bytes.chunks(terminal::MAX_BYTES) {
                        events
                            .try_send(TerminalOutput::Bytes {
                                data: chunk.to_vec(),
                            })
                            .map_err(|_| "terminal output consumer fell behind")?;
                    }
                }
                drop(events);
                Ok::<_, ClientError>(())
            };
            let sending = async {
                while let Some(message) = incoming.recv().await {
                    self.authorize(owner)?;
                    terminal::write_output(&mut writer, &message).await?;
                }
                Ok::<(), ClientError>(())
            };
            tokio::pin!(receiving, reading, sending);
            tokio::select! { biased;
                _ = owner.cancel.cancelled() => Err("terminal authorization or attachment ended".into()),
                _ = connection.closed() => Err("terminal peer disconnected".into()),
                result = &mut receiving => result,
                result = &mut reading => {
                    if let Err(failure) = result { Err(failure) }
                    else {
                        tokio::select! { biased;
                            _ = owner.cancel.cancelled() => Err("terminal attachment ended".into()),
                            _ = connection.closed() => Err("terminal peer disconnected".into()),
                            result = tokio::time::timeout(Duration::from_secs(5), &mut sending) => result.map_err(|_| "terminal output drain timed out".into()).and_then(|result| result),
                        }
                    }
                },
                result = &mut sending => result,
            }
        };
        client.input = Some(input);
        client.output = Some(output);
        // Close/join the exact tmux attachment before publishing Detached.
        // The retained workspace is independent of this process.
        let cleanup = client.close().await;
        result?;
        cleanup?;
        tokio::select! { biased;
            _ = owner.cancel.cancelled() => return Err("terminal attachment cancelled".into()),
            result = tokio::time::timeout(Duration::from_secs(5), async {
                terminal::write_output(&mut writer, &TerminalOutput::Detached).await?;
                writer.finish()?;
                let reason = connection.closed().await;
                if !matches!(reason, iroh::endpoint::ConnectionError::ApplicationClosed(reason)
                    if reason.error_code == 0u32.into() && reason.reason.as_ref() == b"Terminal operation completed") {
                    return Err("terminal consumer did not acknowledge detach".into());
                }
                Ok::<_, ClientError>(())
            }) => result??,
        }
        Ok(())
    }
    fn attachment(&self, id: &str) -> Result<Arc<Attachment>, ClientError> {
        self.state
            .lock()
            .unwrap()
            .attachments
            .get(&id.parse::<u64>()?)
            .cloned()
            .ok_or_else(|| "unknown terminal attachment".into())
    }
    async fn consume(
        mut owner: Operation,
        stream: DataStream,
        outgoing: mpsc::Receiver<TerminalInput>,
        incoming: mpsc::Sender<TerminalOutput>,
        outcome: Arc<Mutex<Option<bool>>>,
        connection: Connection,
    ) {
        let completed =
            terminal::consume_attachment(stream, &connection, &owner.cancel, outgoing, incoming)
                .await;
        *outcome.lock().unwrap() = Some(completed);
        owner.completed = completed;
    }
}

impl NativeClient {
    pub async fn terminal_environments(&self) -> Result<Value, ClientError> {
        Tmux::environments().await
    }
    pub async fn enable_terminal(
        &self,
        configuration: TerminalProvider,
    ) -> Result<Value, ClientError> {
        self.require_ready()?;
        let _configuring = self.terminal.configuring.lock().await;
        let owner = {
            let node = self.node.lock().await;
            let access = self.services.lock().await.access(&node).await?;
            if !access.member(access.local) {
                return Err("only members provide terminals".into());
            }
            format!("{}:{}", node.anchor.genesis, access.local)
        };
        let existing = self.terminal.state.lock().unwrap().backend.clone();
        let backend = if let Some(backend) = existing {
            if backend.configuration != configuration {
                return Err("terminal provider environment already configured".into());
            }
            backend
        } else {
            Tmux::configure(configuration, owner).await?
        };
        let node = self.node.lock().await;
        let mut services = self.services.lock().await;
        let access = services.access(&node).await?;
        if !access.member(access.local) {
            return Err("terminal provider membership ended".into());
        }
        {
            let mut state = self.terminal.state.lock().unwrap();
            if state.closed {
                return Err("terminal host closed".into());
            }
            state.backend = Some(backend);
            state.enabled = true;
        }
        if let Some(change) = access.declaration_change(Service::Terminal, true, true) {
            if let Err(failure) = services.apply(&node, &self.files, change).await {
                self.terminal.state.lock().unwrap().enabled = false;
                return Err(failure);
            }
        }
        self.terminal.enforce_access(&services.access(&node).await?);
        Ok(self.terminal.status())
    }
    pub async fn disable_terminal(&self) -> Result<Value, ClientError> {
        self.terminal.state.lock().unwrap().enabled = false;
        let node = self.node.lock().await;
        let mut services = self.services.lock().await;
        let access = services.access(&node).await?;
        self.terminal.enforce_access(&access);
        let capable = self.terminal.state.lock().unwrap().backend.is_some();
        if let Some(change) = access.declaration_change(Service::Terminal, capable, false) {
            services.apply(&node, &self.files, change).await?;
        }
        Ok(self.terminal.status())
    }
    pub fn terminal_status(&self) -> Value {
        self.terminal.status()
    }

    async fn terminal_operation(&self, address: EndpointAddr) -> Result<Operation, ClientError> {
        self.require_ready()?;
        let peer = VerifyingKey::from_bytes(address.id.as_bytes())?;
        let node = self.node.lock().await;
        let access = self.services.lock().await.access(&node).await?;
        self.terminal.enforce_access(&access);
        self.terminal.claim(peer, false)
    }
    pub async fn request_terminal(
        &self,
        address: EndpointAddr,
        request: TerminalRequest,
    ) -> Result<Value, ClientError> {
        if matches!(request, TerminalRequest::Attach { .. }) {
            return Err("use attach_terminal for attachment ownership".into());
        }
        request.validate()?;
        let mut owner = self.terminal_operation(address.clone()).await?;
        let result = tokio::select! { biased;
            _ = owner.cancel.cancelled() => Err("terminal request cancelled".into()),
            result = async {
                let connection = { let _contact = self.terminal.connecting.lock().await; self.resources.connect(address, terminal::TERMINAL_ALPN).await? };
                self.terminal.attach_connection(&owner, connection.clone())?;
                let (_, response) = terminal::open(&connection, &request).await?;
                Ok::<_, ClientError>(response)
            } => result,
        };
        owner.completed = result.is_ok();
        result
    }
    pub async fn attach_terminal(
        &self,
        address: EndpointAddr,
        workspace: String,
        columns: u16,
        rows: u16,
    ) -> Result<Value, ClientError> {
        let request = TerminalRequest::Attach {
            workspace,
            columns,
            rows,
        };
        request.validate()?;
        if self.terminal.state.lock().unwrap().attachments.len() >= MAX_OPERATIONS {
            return Err("terminal attachment limit reached".into());
        }
        let owner = self.terminal_operation(address.clone()).await?;
        let (connection, stream, ready) = tokio::select! { biased;
            _ = owner.cancel.cancelled() => return Err("terminal attach cancelled".into()),
            result = async {
                let connection = { let _contact = self.terminal.connecting.lock().await; self.resources.connect(address, terminal::TERMINAL_ALPN).await? };
                self.terminal.attach_connection(&owner, connection.clone())?;
                let (stream, ready) = terminal::open(&connection, &request).await?;
                Ok::<_, ClientError>((connection, stream, ready))
            } => result?,
        };
        let id = owner.id;
        let (sender, outgoing) = mpsc::channel(terminal::QUEUE_CAPACITY);
        let (incoming, receiver) = mpsc::channel(terminal::QUEUE_CAPACITY);
        let outcome = Arc::new(Mutex::new(None));
        let slot = Arc::new(Attachment {
            outgoing: sender,
            incoming: AsyncMutex::new(receiver),
            cancel: owner.cancel.clone(),
            outcome: outcome.clone(),
        });
        let mut tasks = self.terminal.tasks.lock().await;
        while tasks.try_join_next().is_some() {}
        {
            let mut state = self.terminal.state.lock().unwrap();
            if state.closed
                || owner.cancel.is_cancelled()
                || state.attachments.len() >= MAX_OPERATIONS
            {
                return Err("terminal attach cancelled or handle limit reached".into());
            }
            state.attachments.insert(id, slot);
        }
        tasks.spawn(NativeTerminal::consume(
            owner, stream, outgoing, incoming, outcome, connection,
        ));
        Ok(
            json!({"id":id.to_string(),"workspace":ready["workspace"],"attached":true,"max_chunk_bytes":terminal::MAX_BYTES}),
        )
    }
    pub async fn send_terminal(&self, id: &str, message: TerminalInput) -> Result<(), ClientError> {
        message.validate()?;
        let slot = self.terminal.attachment(id)?;
        tokio::select! { biased;
            _ = slot.cancel.cancelled() => Err("terminal attachment ended".into()),
            result = slot.outgoing.send(message) => result.map_err(|_| "terminal attachment ended".into()),
        }
    }
    pub async fn read_terminal(&self, id: &str, timeout: Duration) -> Result<Value, ClientError> {
        if timeout > Duration::from_secs(5) {
            return Err("terminal read wait too long".into());
        }
        let slot = self.terminal.attachment(id)?;
        if slot.cancel.is_cancelled() && *slot.outcome.lock().unwrap() != Some(true) {
            return Ok(json!({"pending":false,"attached":false,"failed":true}));
        }
        let waiting = async {
            let mut reader = slot.incoming.lock().await;
            tokio::select! { biased;
                _ = slot.cancel.cancelled() => {
                    if *slot.outcome.lock().unwrap() == Some(true) { reader.recv().await } else { None }
                }
                message = reader.recv() => message,
            }
        };
        match tokio::time::timeout(timeout, waiting).await {
            Err(_) => Ok(json!({"pending":true,"attached":true})),
            Ok(Some(message)) => {
                Ok(json!({"pending":false,"attached":!message.detached(),"message":message}))
            }
            Ok(None) => Ok(
                json!({"pending":false,"attached":false,"failed":*slot.outcome.lock().unwrap() != Some(true)}),
            ),
        }
    }
    pub async fn detach_terminal(&self, id: &str) -> Result<Value, ClientError> {
        let slot = self.terminal.attachment(id)?;
        let identifier = id.parse::<u64>()?;
        let _close = slot.cancel.clone().drop_guard();
        let protocol = tokio::time::timeout(Duration::from_secs(5), async {
            if !slot.cancel.is_cancelled() {
                let _ = self.send_terminal(id, TerminalInput::Detach).await;
            }
            self.terminal.joined(Some(identifier)).await;
        })
        .await;
        if protocol.is_err() {
            slot.cancel.cancel();
        }
        let joined = tokio::time::timeout(
            Duration::from_secs(1),
            self.terminal.joined(Some(identifier)),
        )
        .await;
        self.terminal
            .state
            .lock()
            .unwrap()
            .attachments
            .remove(&identifier);
        if joined.is_err() {
            return Err("terminal cancelled owner did not settle".into());
        }
        if protocol.is_err() {
            return Err("terminal detach exceeded deadline; workspace retained".into());
        }
        Ok(
            json!({"attached":false,"workspace_ended":false,"failed":*slot.outcome.lock().unwrap() != Some(true)}),
        )
    }
    pub async fn cancel_terminal(&self, id: &str) -> Result<Value, ClientError> {
        let slot = self.terminal.attachment(id)?;
        let identifier = id.parse::<u64>()?;
        slot.cancel.cancel();
        tokio::time::timeout(
            Duration::from_secs(1),
            self.terminal.joined(Some(identifier)),
        )
        .await
        .map_err(|_| "terminal cancelled owner did not settle")?;
        self.terminal
            .state
            .lock()
            .unwrap()
            .attachments
            .remove(&identifier);
        Ok(json!({"attached":false,"workspace_ended":false,"cancelled":true}))
    }
}
