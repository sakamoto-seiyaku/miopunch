//! Provider-side HTTP(S), using the existing identity and single service ACL.
use crate::{
    data_stream::DataStream,
    native_client::{ClientError, NativeClient},
    service_policy::{Service, ServiceAccess},
    web::{self, RequestHead, ResponseHead, WebRequest, WebResponse},
};
use iroh::{EndpointAddr, endpoint::Connection};
use p2panda_core::VerifyingKey;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}},
    time::Duration,
};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

const MAX_FLOWS: usize = 16;
#[path = "native_web_socket.rs"]
mod socket;

struct Flow {
    peer: VerifyingKey,
    provider: bool,
    origin: Option<String>,
    connection: Option<Connection>,
    cancel: CancellationToken,
    phase: &'static str,
    completed: bool,
}
struct RequestSlot {
    cancel: CancellationToken,
    result: Mutex<Option<Result<WebResponse, String>>>,
    done: AtomicBool,
    joined: tokio::sync::Notify,
}
#[derive(Default)]
struct State {
    access: Option<ServiceAccess>,
    origins: BTreeSet<String>,
    client: Option<reqwest::Client>,
    tls: Option<Arc<rustls::ClientConfig>>,
    sockets: BTreeMap<u64, Arc<socket::SocketSlot>>,
    requests: BTreeMap<u64, Arc<RequestSlot>>,
    flows: BTreeMap<u64, Flow>,
    next: u64,
    closed: bool,
    completed: u64,
    failed: u64,
    last_failure_phase: Option<&'static str>,
}
pub(crate) struct NativeWeb {
    state: Mutex<State>,
    gate: crate::admission_gate::AdmissionGate,
    tasks: tokio::sync::Mutex<JoinSet<()>>,
    connecting: tokio::sync::Mutex<()>,
    joined: tokio::sync::Notify,
}
pub(crate) struct WebOwner(pub Arc<NativeWeb>);
impl Drop for WebOwner {
    fn drop(&mut self) {
        self.0.cancel_all();
    }
}
struct FlowOwner {
    runtime: Arc<NativeWeb>,
    id: u64,
    cancel: CancellationToken,
}
impl Drop for FlowOwner {
    fn drop(&mut self) {
        let mut state = self.runtime.state.lock().unwrap();
        if let Some(flow) = state.flows.remove(&self.id) {
            if flow.completed {
                state.completed = state.completed.saturating_add(1);
            } else {
                state.failed = state.failed.saturating_add(1);
                state.last_failure_phase = Some(flow.phase);
            }
            flow.cancel.cancel();
            if let Some(connection) = flow.connection {
                if flow.completed {
                    connection.close(0u32.into(), b"Web operation completed");
                } else {
                    connection.close(1u32.into(), b"Web operation failed");
                }
            }
        }
        self.runtime.joined.notify_waiters();
    }
}
fn origin(value: &str) -> Result<String, ClientError> {
    let url = url::Url::parse(value)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Web requires an HTTP(S) URL without user information".into());
    }
    Ok(url.origin().ascii_serialization())
}
fn forward_header(name: &str) -> bool {
    !matches!(
        name.to_ascii_lowercase().as_str(),
        "host"
            | "connection"
            | "content-length"
            | "transfer-encoding"
            | "upgrade"
            | "proxy-authorization"
            | "proxy-authenticate"
            | "keep-alive"
            | "te"
            | "trailer"
    )
}
impl NativeWeb {
    pub fn new(gate: crate::admission_gate::AdmissionGate) -> Arc<Self> {
        Arc::new(Self {
            state: Default::default(),
            gate,
            tasks: Default::default(),
            connecting: Default::default(),
            joined: Default::default(),
        })
    }
    fn permitted(state: &State, access: &ServiceAccess, flow: &Flow) -> bool {
        web::peer_permitted(access, flow.peer)
            && (!flow.provider
                || access.member(access.local)
                    && access.permits_service(Service::Web, flow.peer)
                    && state.client.is_some()
                    && flow
                        .origin
                        .as_ref()
                        .is_none_or(|target| state.origins.contains(target)))
    }
    pub fn enforce_access(&self, access: &ServiceAccess) {
        self.gate.update_service_policy(
            web::WEB_ALPN,
            access.guest_pairs_for(Service::Web),
            Default::default(),
        );
        let mut state = self.state.lock().unwrap();
        for flow in state.flows.values() {
            if !Self::permitted(&state, access, flow) {
                flow.cancel.cancel();
                if let Some(connection) = &flow.connection {
                    connection.close(1u32.into(), b"Web authorization ended");
                }
            }
        }
        state.access = Some(access.clone());
    }
    fn claim(
        self: &Arc<Self>,
        peer: VerifyingKey,
        provider: bool,
    ) -> Result<FlowOwner, ClientError> {
        let mut state = self.state.lock().unwrap();
        let flow = Flow {
            peer,
            provider,
            origin: None,
            connection: None,
            cancel: CancellationToken::new(),
            phase: "connecting",
            completed: false,
        };
        if state.closed
            || state.flows.len() >= MAX_FLOWS
            || !state
                .access
                .as_ref()
                .is_some_and(|access| Self::permitted(&state, access, &flow))
        {
            return Err("Web service unavailable or busy".into());
        }
        state.next = state
            .next
            .checked_add(1)
            .ok_or("Web identifier exhausted")?;
        let id = state.next;
        let cancel = flow.cancel.clone();
        state.flows.insert(id, flow);
        Ok(FlowOwner {
            runtime: self.clone(),
            id,
            cancel,
        })
    }
    fn attach(&self, owner: &FlowOwner, connection: Connection) -> Result<(), ClientError> {
        let mut state = self.state.lock().unwrap();
        let flow = state
            .flows
            .get_mut(&owner.id)
            .ok_or("Web operation ended")?;
        if flow.cancel.is_cancelled() {
            connection.close(1u32.into(), b"Web cancelled");
            return Err("Web operation cancelled".into());
        }
        flow.connection = Some(connection);
        Ok(())
    }
    fn phase(&self, owner: &FlowOwner, phase: &'static str) {
        if let Some(flow) = self.state.lock().unwrap().flows.get_mut(&owner.id) {
            flow.phase = phase;
        }
    }
    fn completed(&self, owner: &FlowOwner) {
        if let Some(flow) = self.state.lock().unwrap().flows.get_mut(&owner.id) {
            flow.completed = true;
        }
    }
    fn authorized_client(
        &self,
        owner: &FlowOwner,
        url: &str,
    ) -> Result<reqwest::Client, ClientError> {
        let target = origin(url)?;
        let mut state = self.state.lock().unwrap();
        let flow = state
            .flows
            .get_mut(&owner.id)
            .ok_or("Web operation ended")?;
        flow.origin = Some(target);
        if flow.cancel.is_cancelled()
            || !state
                .access
                .as_ref()
                .is_some_and(|access| Self::permitted(&state, access, &state.flows[&owner.id]))
        {
            return Err("Web target not authorized".into());
        }
        state
            .client
            .clone()
            .ok_or_else(|| "Web service is off".into())
    }
    pub async fn receive(self: &Arc<Self>, connection: Connection) -> Result<(), ClientError> {
        let peer = VerifyingKey::from_bytes(connection.remote_id().as_bytes())?;
        let mut tasks = self.tasks.lock().await;
        while tasks.try_join_next().is_some() {}
        let owner = match self.claim(peer, true) {
            Ok(owner) => owner,
            Err(error) => {
                connection.close(1u32.into(), b"Web unavailable");
                return Err(error);
            }
        };
        self.attach(&owner, connection.clone())?;
        let runtime = self.clone();
        let peer_connection = connection.clone();
        tasks.spawn(async move {
            let result = tokio::select! {
                biased;
                _ = owner.cancel.cancelled() => Err("Web request cancelled".into()),
                result = runtime.serve(&owner, connection) => result,
                _ = peer_connection.closed() => Err("Web peer ended before operation completed".into()),
            };
            if result.is_ok() {
                runtime.completed(&owner);
            }
        });
        Ok(())
    }
    async fn serve(&self, owner: &FlowOwner, connection: Connection) -> Result<(), ClientError> {
        self.phase(owner, "accepting_stream");
        let mut stream = DataStream::accept(&connection).await?;
        self.phase(owner, "reading_request");
        let request: RequestHead = web::read_head(&mut stream).await?;
        let body = web::read_body(&mut stream, web::MAX_REQUEST_BODY).await?;
        if request.socket {
            if !body.is_empty() {
                return Err("WebSocket opening body is not supported".into());
            }
            return self.serve_socket(owner, connection, stream, request).await;
        }
        self.phase(owner, "checking_target");
        let client = self.authorized_client(owner, &request.url)?;
        let method = reqwest::Method::from_bytes(request.method.as_bytes())?;
        if method == reqwest::Method::CONNECT {
            return Err("CONNECT is not a Web request".into());
        }
        let mut outgoing = client.request(method, &request.url).body(body);
        for (name, value) in request.headers {
            if forward_header(&name) {
                outgoing = outgoing.header(name, value);
            }
        }
        self.phase(owner, "requesting_target");
        let mut response = outgoing.send().await?;
        let head = ResponseHead {
            url: response.url().to_string(),
            status: response.status().as_u16(),
            headers: response
                .headers()
                .iter()
                .filter(|(name, _)| forward_header(name.as_str()))
                .map(|(name, value)| Ok((name.to_string(), value.to_str()?.to_string())))
                .collect::<Result<Vec<_>, reqwest::header::ToStrError>>()?,
        };
        self.phase(owner, "writing_response");
        web::write_head(&mut stream, &head).await?;
        let mut received = 0;
        while let Some(chunk) = response.chunk().await? {
            received += chunk.len();
            if received > web::MAX_BODY {
                return Err("Web response too large".into());
            }
            for bytes in chunk.chunks(crate::data_stream::CHUNK_BYTES) {
                stream.write_chunk(bytes).await?;
            }
        }
        stream.write_chunk(&[]).await?;
        stream.finish()?;
        // finish() queues a FIN; closing the connection here would discard
        // queued final bytes. The consumer closes only after reading the
        // complete bounded response, and marks failed reads with another code.
        self.phase(owner, "awaiting_response_receipt");
        let reason = tokio::time::timeout(Duration::from_secs(30), connection.closed()).await?;
        if !matches!(reason, iroh::endpoint::ConnectionError::ApplicationClosed(reason)
            if reason.error_code == 0u32.into() && reason.reason.as_ref() == b"Web operation completed")
        {
            return Err("Web consumer did not complete response receipt".into());
        }
        Ok(())
    }
    fn configure(&self, targets: Vec<String>, roots: Vec<String>) -> Result<(), ClientError> {
        if targets.is_empty() || targets.len() > 32 || roots.len() > 8 {
            return Err("invalid Web configuration".into());
        }
        let origins = targets
            .iter()
            .map(|target| origin(target))
            .collect::<Result<BTreeSet<_>, _>>()?;
        // Configure this service's client explicitly. Do not install a process
        // global provider or change the crypto defaults of existing SDK users.
        let mut certificates =
            rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        for root in roots {
            if root.len() > 64 * 1024 {
                return Err("Web trust root too large".into());
            }
            let certs =
                rustls_pemfile::certs(&mut root.as_bytes()).collect::<Result<Vec<_>, _>>()?;
            if certs.is_empty() {
                return Err("Web trust root has no certificate".into());
            }
            for cert in certs {
                certificates.add(cert)?;
            }
        }
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()?
        .with_root_certificates(certificates)
        .with_no_client_auth();
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(0)
            .tls_sslkeylogfile(false)
            .tls_backend_preconfigured(tls.clone());
        let client = client.build()?;
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return Err("Web host closed".into());
        }
        state.origins = origins;
        state.client = Some(client);
        state.tls = Some(Arc::new(tls));
        Ok(())
    }
    pub fn status(&self) -> Value {
        let state = self.state.lock().unwrap();
        json!({"provider":true,"enabled":state.client.is_some() && !state.closed,"origins":state.origins,
            "active_flows":state.flows.len(),"max_flows":MAX_FLOWS,"max_response_bytes":web::MAX_BODY,
            "completed":state.completed,"failed":state.failed,"last_failure_phase":state.last_failure_phase,
            "request_handles":state.requests.len(),"max_request_handles":MAX_FLOWS,
            "socket_handles":state.sockets.len(),"max_socket_message_bytes":web::MAX_SOCKET_MESSAGE,"socket_queue_capacity":16})
    }
    fn cancel_all(&self) {
        for flow in self.state.lock().unwrap().flows.values() {
            flow.cancel.cancel();
            if let Some(connection) = &flow.connection {
                connection.close(1u32.into(), b"Web host stopped");
            }
        }
    }
    pub async fn close(&self) -> Result<(), ClientError> {
        self.state.lock().unwrap().closed = true;
        self.cancel_all();
        self.state.lock().unwrap().sockets.clear();
        let mut tasks = self.tasks.lock().await;
        while tasks.join_next().await.is_some() {}
        self.state.lock().unwrap().requests.clear();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let notified = self.joined.notified();
                if self.state.lock().unwrap().flows.is_empty() {
                    break;
                }
                notified.await;
            }
        })
        .await?;
        Ok(())
    }
}

impl NativeClient {
    pub async fn enable_web(
        &self,
        targets: Vec<String>,
        trust_roots_pem: Vec<String>,
    ) -> Result<Value, ClientError> {
        self.require_ready()?;
        let node = self.node.lock().await;
        let mut services = self.services.lock().await;
        let access = services.access(&node).await?;
        if !access.member(access.local) {
            return Err("only members provide Web".into());
        }
        self.web.configure(targets, trust_roots_pem)?;
        if let Some(change) = access.declaration_change(Service::Web, true, true) {
            services.apply(&node, &self.files, change).await?;
        }
        self.web.enforce_access(&services.access(&node).await?);
        Ok(self.web.status())
    }
    pub async fn disable_web(&self) -> Result<Value, ClientError> {
        {
            let mut state = self.web.state.lock().unwrap();
            state.client = None;
            state.tls = None;
            state.origins.clear();
        }
        self.web.cancel_all();
        let node = self.node.lock().await;
        let mut services = self.services.lock().await;
        let access = services.access(&node).await?;
        if let Some(change) = access.declaration_change(Service::Web, true, false) {
            services.apply(&node, &self.files, change).await?;
        }
        Ok(self.web.status())
    }
    pub fn web_status(&self) -> Value {
        self.web.status()
    }
    pub async fn request_web(
        &self,
        address: EndpointAddr,
        request: WebRequest,
    ) -> Result<WebResponse, ClientError> {
        let owner = self.prepare_web_request(&address, &request).await?;
        NativeWeb::exchange(self.web.clone(), self.resources.clone(), owner, address, request).await
    }
    async fn prepare_web_request(
        &self,
        address: &EndpointAddr,
        request: &WebRequest,
    ) -> Result<FlowOwner, ClientError> {
        self.require_ready()?;
        origin(&request.url)?;
        let peer = VerifyingKey::from_bytes(address.id.as_bytes())?;
        let access = {
            let node = self.node.lock().await;
            self.services.lock().await.access(&node).await?
        };
        self.web.enforce_access(&access);
        self.web.claim(peer, false)
    }
    /// Start a bounded HTTP operation without blocking the host control loop.
    /// Results occupy a slot until read or cancelled, including completed work.
    pub async fn begin_web_request(
        &self,
        address: EndpointAddr,
        request: WebRequest,
    ) -> Result<Value, ClientError> {
        let mut tasks = self.web.tasks.lock().await;
        while tasks.try_join_next().is_some() {}
        let owner = self.prepare_web_request(&address, &request).await?;
        let id = owner.id;
        let slot = Arc::new(RequestSlot {
            cancel: owner.cancel.clone(), result: Mutex::new(None),
            done: AtomicBool::new(false), joined: Default::default(),
        });
        {
            let mut state = self.web.state.lock().unwrap();
            if state.closed || state.requests.len() >= MAX_FLOWS {
                return Err("Web request handle limit reached".into());
            }
            state.requests.insert(id, slot.clone());
        }
        let runtime = self.web.clone();
        let resources = self.resources.clone();
        tasks.spawn(async move {
            let result = NativeWeb::exchange(runtime, resources, owner, address, request).await;
            *slot.result.lock().unwrap() = Some(result.map_err(|_| "Web request failed".to_string()));
            slot.done.store(true, Ordering::Release);
            slot.joined.notify_waiters();
        });
        Ok(json!({"id":id.to_string(),"pending":true}))
    }
    /// Consume a completed response, or report that its owner is still running.
    pub fn web_request_result(&self, id: &str) -> Result<Value, ClientError> {
        let id = id.parse::<u64>()?;
        let slot = {
            let mut state = self.web.state.lock().unwrap();
            let std::collections::btree_map::Entry::Occupied(entry) = state.requests.entry(id) else {
                return Err("unknown Web request handle".into());
            };
            if !entry.get().done.load(Ordering::Acquire) {
                return Ok(json!({"pending":true}));
            }
            entry.remove()
        };
        let result = slot.result.lock().unwrap().take().ok_or("Web request result unavailable")?;
        match result {
            Ok(response) => Ok(json!({"pending":false,"response":response})),
            Err(error) => Err(error.into()),
        }
    }
    pub async fn cancel_web_request(&self, id: &str) -> Result<Value, ClientError> {
        let slot = self.web.state.lock().unwrap().requests.remove(&id.parse::<u64>()?)
            .ok_or("unknown Web request handle")?;
        slot.cancel.cancel();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let notified = slot.joined.notified();
                if slot.done.load(Ordering::Acquire) { break; }
                notified.await;
            }
        }).await?;
        Ok(json!({"cancelled":true,"joined":true}))
    }
}
impl NativeWeb {
    async fn exchange(
        runtime: Arc<Self>,
        resources: Arc<crate::resources::ResourceEndpoint>,
        owner: FlowOwner,
        address: EndpointAddr,
        request: WebRequest,
    ) -> Result<WebResponse, ClientError> {
        tokio::select! {
            biased;
            _ = owner.cancel.cancelled() => Err("Web request cancelled".into()),
            result = async {
                // ResourceEndpoint permits one contact attempt at a time.
                // Queue bounded Web owners, without retrying failed contact
                // or holding this lock during application I/O.
                let connection = {
                    let _contact = runtime.connecting.lock().await;
                    resources.connect(address, web::WEB_ALPN).await?
                };
                runtime.attach(&owner, connection.clone())?;
                runtime.phase(&owner, "exchanging_response");
                let response = web::request(&connection, request).await?;
                runtime.completed(&owner);
                Ok::<_, ClientError>(response)
            } => result,
        }
    }
}
