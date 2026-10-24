//! One selected clipboard pair, bounded duplex changes, and secret-free status.
//!
//! Platform adapters execute each sample/write under their current service
//! authority. Clipboard bytes are transient business data, never ledger data.
use crate::{
    data_stream::{CHUNK_BYTES, DataReader, DataStream, DataWriter},
    service_policy::{Service, ServiceAccess},
};
use futures_util::FutureExt;
use p2panda_core::VerifyingKey;
use serde::{Deserialize, Serialize};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

pub const CLIPBOARD_ALPN: &[u8] = b"miopunch/clipboard/1";
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;
const POLL: Duration = Duration::from_millis(250);

/// Describes the current runtime, including permission/context limitations.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClipboardCapability {
    pub backend: String,
    pub read: bool,
    pub write: bool,
    pub reason: Option<String>,
    pub formats: Vec<String>,
    pub max_text_bytes: usize,
}
impl ClipboardCapability {
    pub fn available(backend: impl Into<String>) -> Self {
        Self {
            backend: backend.into(),
            read: true,
            write: true,
            reason: None,
            formats: vec!["text/plain".into()],
            max_text_bytes: MAX_TEXT_BYTES,
        }
    }
    pub fn unavailable(backend: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            read: false,
            write: false,
            reason: Some(reason.into()),
            ..Self::available(backend)
        }
    }
}

/// Errors deliberately contain no OS output, clipboard value or value digest.
#[derive(Clone, Copy, Debug, thiserror::Error, PartialEq, Eq)]
pub enum ClipboardError {
    #[error("clipboard_disabled")]
    Disabled,
    #[error("clipboard_pair_busy")]
    Busy,
    #[error("clipboard_access_denied")]
    Denied,
    #[error("clipboard_platform_unavailable")]
    Unavailable,
    #[error("clipboard_read_failed")]
    Read,
    #[error("clipboard_write_failed")]
    Write,
    #[error("clipboard_text_too_large")]
    TooLarge,
    #[error("clipboard_transport_failed")]
    Transport,
    #[error("clipboard_protocol_failed")]
    Protocol,
    #[error("clipboard_pair_stopped")]
    Stopped,
}

#[derive(Clone)]
pub struct ClipboardLease {
    pub peer: VerifyingKey,
    generation: u64,
    pub cancellation: CancellationToken,
}

/// Captures the local start operation before its first wait. Stop retires all
/// such earlier requests, including ones still queued for platform/transport.
#[derive(Clone, Copy)]
pub struct ClipboardRequest(u64);

/// A cancelled/dropped setup releases its reservation. Only a completed setup
/// transfers the lease to the joined automatic worker.
pub struct ClipboardClaim {
    state: Arc<ClipboardState>,
    lease: ClipboardLease,
    transferred: bool,
}
impl ClipboardClaim {
    pub fn new(
        state: Arc<ClipboardState>,
        peer: VerifyingKey,
        request: ClipboardRequest,
    ) -> Result<Self, ClipboardError> {
        let lease = state.claim(peer, request)?;
        Ok(Self {
            state,
            lease,
            transferred: false,
        })
    }
    pub fn lease(&self) -> &ClipboardLease {
        &self.lease
    }
    pub fn fail(&self, failure: ClipboardError) {
        self.lease.cancellation.cancel();
        self.state.finish(&self.lease, Err(failure));
    }
    pub fn transfer(mut self) -> ClipboardLease {
        self.transferred = true;
        self.lease.clone()
    }
}

pub(crate) struct ClipboardConnection(Option<iroh::endpoint::Connection>);
impl ClipboardConnection {
    pub fn new(connection: iroh::endpoint::Connection) -> Self {
        Self(Some(connection))
    }
    pub fn get(&self) -> &iroh::endpoint::Connection {
        self.0.as_ref().expect("owned clipboard setup connection")
    }
    pub fn transfer(mut self) -> iroh::endpoint::Connection {
        self.0.take().expect("owned clipboard setup connection")
    }
}
impl Drop for ClipboardConnection {
    fn drop(&mut self) {
        if let Some(connection) = &self.0 {
            connection.close(0u32.into(), b"clipboard setup ended");
        }
    }
}
impl Drop for ClipboardClaim {
    fn drop(&mut self) {
        if !self.transferred {
            self.lease.cancellation.cancel();
            self.state.finish(&self.lease, Err(ClipboardError::Stopped));
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ClipboardStatus {
    pub capability: ClipboardCapability,
    pub enabled: bool,
    pub phase: String,
    pub peer: Option<VerifyingKey>,
    pub sent: u64,
    pub written: u64,
    pub acknowledged: u64,
    pub failure: Option<String>,
    pub transport: Option<serde_json::Value>,
}

struct State {
    status: ClipboardStatus,
    generation: u64,
    request_epoch: u64,
    pair: Option<ClipboardLease>,
}
pub struct ClipboardState(Mutex<State>);
impl ClipboardState {
    pub fn new(capability: ClipboardCapability) -> Self {
        Self(Mutex::new(State {
            status: ClipboardStatus {
                capability,
                enabled: false,
                phase: "disabled".into(),
                peer: None,
                sent: 0,
                written: 0,
                acknowledged: 0,
                failure: None,
                transport: None,
            },
            generation: 0,
            request_epoch: 0,
            pair: None,
        }))
    }
    pub fn status(&self) -> ClipboardStatus {
        self.0.lock().unwrap().status.clone()
    }
    pub fn request(&self) -> ClipboardRequest {
        ClipboardRequest(self.0.lock().unwrap().request_epoch)
    }
    pub fn current_request(&self, request: ClipboardRequest) -> Result<(), ClipboardError> {
        if self.0.lock().unwrap().request_epoch != request.0 {
            return Err(ClipboardError::Stopped);
        }
        Ok(())
    }
    pub fn capability(&self, capability: ClipboardCapability) {
        self.0.lock().unwrap().status.capability = capability;
    }
    pub fn transport(&self, connection: &iroh::endpoint::Connection) {
        self.0.lock().unwrap().status.transport = Some(serde_json::json!({
            "peer": connection.remote_id().to_string(), "connection": connection.stable_id(),
            "paths": connection.paths().iter().map(|path| serde_json::json!({
                "remote":path.remote_addr(),"local":format!("{:?}",path.local_addr()),
                "selected":path.is_selected(),"tx_bytes":path.stats().udp_tx.bytes,
                "rx_bytes":path.stats().udp_rx.bytes})).collect::<Vec<_>>()
        }));
    }
    pub fn enable(&self, capability: ClipboardCapability) -> Result<(), ClipboardError> {
        let mut state = self.0.lock().unwrap();
        if state.pair.is_some() {
            return Err(ClipboardError::Busy);
        }
        state.status.capability = capability;
        if !state.status.capability.read || !state.status.capability.write {
            return Err(ClipboardError::Unavailable);
        }
        state.status.enabled = true;
        state.status.phase = "idle".into();
        state.status.failure = None;
        Ok(())
    }
    pub fn stop(&self, disable: bool) {
        let mut state = self.0.lock().unwrap();
        state.request_epoch = state
            .request_epoch
            .checked_add(1)
            .expect("clipboard request epoch exhausted");
        if disable {
            state.status.enabled = false;
            state.status.phase = "disabled".into();
        }
        if let Some(pair) = &state.pair {
            pair.cancellation.cancel();
        }
    }
    fn claim(
        &self,
        peer: VerifyingKey,
        request: ClipboardRequest,
    ) -> Result<ClipboardLease, ClipboardError> {
        let mut state = self.0.lock().unwrap();
        if request.0 != state.request_epoch {
            return Err(ClipboardError::Stopped);
        }
        if !state.status.enabled {
            return Err(ClipboardError::Disabled);
        }
        if state.pair.is_some() {
            return Err(ClipboardError::Busy);
        }
        state.generation = state
            .generation
            .checked_add(1)
            .ok_or(ClipboardError::Protocol)?;
        let lease = ClipboardLease {
            peer,
            generation: state.generation,
            cancellation: CancellationToken::new(),
        };
        state.status.peer = Some(peer);
        state.status.phase = "connecting".into();
        state.status.transport = None;
        state.status.failure = None;
        state.pair = Some(lease.clone());
        Ok(lease)
    }
    pub fn activated(&self, lease: &ClipboardLease) -> Result<(), ClipboardError> {
        let mut state = self.0.lock().unwrap();
        if !state.status.enabled
            || lease.cancellation.is_cancelled()
            || state
                .pair
                .as_ref()
                .is_none_or(|pair| pair.generation != lease.generation)
        {
            return Err(ClipboardError::Stopped);
        }
        state.status.phase = "synchronizing".into();
        Ok(())
    }
    pub fn current(&self, lease: &ClipboardLease) -> Result<(), ClipboardError> {
        let state = self.0.lock().unwrap();
        if !state.status.enabled || lease.cancellation.is_cancelled() {
            return Err(ClipboardError::Stopped);
        }
        if state
            .pair
            .as_ref()
            .is_none_or(|pair| pair.generation != lease.generation)
        {
            return Err(ClipboardError::Stopped);
        }
        Ok(())
    }
    pub fn finish(&self, lease: &ClipboardLease, result: Result<(), ClipboardError>) {
        let mut state = self.0.lock().unwrap();
        if state
            .pair
            .as_ref()
            .is_some_and(|pair| pair.generation == lease.generation)
        {
            if result.is_err_and(|failure| {
                matches!(
                    failure,
                    ClipboardError::Unavailable | ClipboardError::Read | ClipboardError::Write
                )
            }) {
                state.status.capability.read = false;
                state.status.capability.write = false;
                state.status.capability.reason = result.err().map(|failure| failure.to_string());
            }
            state.status.peer = None;
            state.status.phase = if state.status.enabled {
                "idle"
            } else {
                "disabled"
            }
            .into();
            state.status.failure = result
                .err()
                .filter(|e| *e != ClipboardError::Stopped)
                .map(|e| e.to_string());
            state.pair = None;
        }
    }
    fn count(&self, counter: Counter) {
        let mut state = self.0.lock().unwrap();
        let value = match counter {
            Counter::Sent => &mut state.status.sent,
            Counter::Written => &mut state.status.written,
            Counter::Acknowledged => &mut state.status.acknowledged,
        };
        *value = value.saturating_add(1);
    }
}
enum Counter {
    Sent,
    Written,
    Acknowledged,
}

/// Apply the existing effective provider ACL, including any scoped guest grant.
/// Platform capability, service enablement and active-pair ownership are separate.
pub fn permitted(access: &ServiceAccess, peer: VerifyingKey) -> bool {
    access.permits_service(Service::Clipboard, peer)
}

/// The public host seam: real OS samples/effects plus current authorization.
/// Implementations must join a started platform effect before returning, even
/// when stop/revocation occurs. Calling code joins the pair before stop returns.
#[allow(async_fn_in_trait)]
pub trait ClipboardHost {
    async fn sample(&self, lease: &ClipboardLease) -> Result<Option<String>, ClipboardError>;
    async fn apply(&self, lease: &ClipboardLease, text: &str) -> Result<(), ClipboardError>;
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Header {
    Hello { version: u32 },
    Ready,
    Change { sequence: u64, bytes: usize },
    Written { sequence: u64 },
    Heartbeat,
}
enum Message {
    Change(u64, String),
    Written(u64),
    Heartbeat,
}

pub async fn begin(stream: &mut DataStream, initiator: bool) -> Result<(), ClipboardError> {
    if initiator {
        stream
            .write_control(&Header::Hello { version: 1 })
            .await
            .map_err(|_| ClipboardError::Transport)?;
        if !matches!(
            stream
                .read_control::<Header>()
                .await
                .map_err(|_| ClipboardError::Transport)?,
            Header::Ready
        ) {
            return Err(ClipboardError::Protocol);
        }
    } else {
        if !matches!(
            stream
                .read_control::<Header>()
                .await
                .map_err(|_| ClipboardError::Transport)?,
            Header::Hello { version: 1 }
        ) {
            return Err(ClipboardError::Protocol);
        }
        stream
            .write_control(&Header::Ready)
            .await
            .map_err(|_| ClipboardError::Transport)?;
    }
    Ok(())
}

async fn write_header(writer: &mut DataWriter, header: &Header) -> Result<(), ClipboardError> {
    let bytes = serde_json::to_vec(header).map_err(|_| ClipboardError::Protocol)?;
    writer
        .write_chunk(&bytes)
        .await
        .map_err(|_| ClipboardError::Transport)
}
async fn read_message(reader: &mut DataReader) -> Result<Message, ClipboardError> {
    let bytes = reader
        .read_chunk()
        .await
        .map_err(|_| ClipboardError::Transport)?;
    if bytes.len() > 256 {
        return Err(ClipboardError::Protocol);
    }
    match serde_json::from_slice(&bytes).map_err(|_| ClipboardError::Protocol)? {
        Header::Change { sequence, bytes } => {
            if bytes > MAX_TEXT_BYTES || sequence == 0 {
                return Err(ClipboardError::TooLarge);
            }
            let mut text = Vec::with_capacity(bytes);
            while text.len() < bytes {
                let chunk = reader
                    .read_chunk()
                    .await
                    .map_err(|_| ClipboardError::Transport)?;
                if chunk.is_empty() || chunk.len() > bytes - text.len() {
                    return Err(ClipboardError::Protocol);
                }
                text.extend(chunk);
            }
            Ok(Message::Change(
                sequence,
                String::from_utf8(text).map_err(|_| ClipboardError::Protocol)?,
            ))
        }
        Header::Written { sequence } => Ok(Message::Written(sequence)),
        Header::Heartbeat => Ok(Message::Heartbeat),
        _ => Err(ClipboardError::Protocol),
    }
}

/// One in-flight value, one bounded receive and no plaintext status/history.
/// The pending framed read stays alive across polls, so a timer never discards
/// a partially consumed stream record. Clipboard writes are awaited to completion.
pub async fn synchronize(
    host: &impl ClipboardHost,
    state: &ClipboardState,
    lease: &ClipboardLease,
    stream: DataStream,
    mut baseline: Option<String>,
) -> Result<(), ClipboardError> {
    let (mut writer, mut reader) = stream.split();
    let mut sequence = 0u64;
    let mut received = 0u64;
    let mut in_flight = None;
    let mut heartbeat = 0;
    loop {
        let message = read_message(&mut reader).fuse();
        futures_util::pin_mut!(message);
        loop {
            let tick = n0_future::time::sleep(POLL).fuse();
            let stop = lease.cancellation.cancelled().fuse();
            futures_util::pin_mut!(tick, stop);
            futures_util::select_biased! {
                _ = stop => return Err(ClipboardError::Stopped),
                result = message => {
                    state.current(lease)?;
                    match result? {
                        Message::Change(id, text) => {
                            if id != received.checked_add(1).ok_or(ClipboardError::Protocol)? {
                                return Err(ClipboardError::Protocol);
                            }
                            host.apply(lease, &text).await?;
                            state.count(Counter::Written);
                            baseline = Some(text);
                            received = id;
                            write_header(&mut writer, &Header::Written { sequence: id }).await?;
                        }
                        Message::Written(id) => {
                            if in_flight != Some(id) {
                                return Err(ClipboardError::Protocol);
                            }
                            in_flight = None;
                            state.count(Counter::Acknowledged);
                        }
                        Message::Heartbeat => {}
                    }
                    break;
                },
                _ = tick => {
                    state.current(lease)?;
                    let current = host.sample(lease).await?;
                    if in_flight.is_none() && current != baseline {
                        if let Some(text) = &current {
                            if text.len() > MAX_TEXT_BYTES {
                                return Err(ClipboardError::TooLarge);
                            }
                            sequence = sequence.checked_add(1).ok_or(ClipboardError::Protocol)?;
                            write_header(&mut writer, &Header::Change { sequence, bytes: text.len() }).await?;
                            for chunk in text.as_bytes().chunks(CHUNK_BYTES) {
                                writer.write_chunk(chunk).await.map_err(|_| ClipboardError::Transport)?;
                            }
                            in_flight = Some(sequence);
                            state.count(Counter::Sent);
                        }
                        baseline = current;
                    }
                    heartbeat += 1;
                    if heartbeat == 20 {
                        write_header(&mut writer, &Header::Heartbeat).await?;
                        heartbeat = 0;
                    }
                }
            }
        }
    }
}
