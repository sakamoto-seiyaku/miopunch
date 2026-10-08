//! Bounded datagram ownership and terminal retirement shared by RTC probes.
//! Limits cover application queues, not the browser's or SCTP's total memory.
use futures_util::task::AtomicWaker;
use std::{
    collections::VecDeque,
    sync::Mutex,
    task::{Context, Poll},
};

pub const QUEUE_BYTES: usize = 64 * 1024;
pub const DATAGRAM_BYTES: usize = 1200;

pub const CHANNEL_ID: u16 = 0;
pub const CHANNEL_LABEL: &str = "mio-rtc-synthetic/1";
pub const CHANNEL_PROTOCOL: &str = "miopunch/synthetic/1";

pub const EXPORTER_LABEL: &[u8] = b"miopunch/rtc-probe/1";
pub const DATAGRAM_CHANNEL_LABEL: &str = "mio-quic-datagram/1";
pub const DATAGRAM_CHANNEL_PROTOCOL: &str = "miopunch/quic-datagram/1";
pub const UNIFIED_RTC_PROTOCOL: &str = "miopunch/session/rtc-candidate/1";
pub const CONTINUITY_MARKER: &[u8; 4] = b"MRS1";
pub const CONTINUITY_BINDING_LABEL: &[u8] = b"miopunch/rtc-original-stream/1";

/// The same qualified reliability semantics, with an explicit data-purpose profile.
pub fn datagram_channel_parameters() -> serde_json::Value {
    let mut profile = channel_parameters();
    profile["label"] = DATAGRAM_CHANNEL_LABEL.into();
    profile["protocol"] = DATAGRAM_CHANNEL_PROTOCOL.into();
    profile
}

/// One monotonic deadline for the entire attempt. The shared connection cap
/// never extends an explicit shorter caller deadline.
#[derive(Clone, Copy, Debug)]
pub struct RtcDeadline {
    started: n0_future::time::Instant,
    deadline: n0_future::time::Instant,
}
impl RtcDeadline {
    pub fn from_caller(deadline: n0_future::time::Instant) -> Self {
        let started = n0_future::time::Instant::now();
        Self {
            started,
            deadline: deadline.min(started + crate::connection_budget::MAX),
        }
    }
    pub fn remaining(&self) -> std::io::Result<std::time::Duration> {
        self.deadline
            .checked_duration_since(n0_future::time::Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::TimedOut, "RTC caller deadline exceeded")
            })
    }
    pub fn elapsed_ms(&self) -> u128 {
        self.started.elapsed().as_millis()
    }
}

/// Attempt role order shared by both native and browser adapters.
pub fn attempt_context(
    attempt: &str,
    network: &[u8; 32],
    generation: u64,
    offerer: &str,
    answerer: &str,
) -> String {
    serde_json::json!([attempt, hex::encode(network), generation, offerer, answerer]).to_string()
}

/// Test profile explicitly confirmed by both endpoints over their protected M.
/// Incoming DCEP metadata in rtc 0.21.0 does not preserve the reliability fields.
pub fn channel_parameters() -> serde_json::Value {
    serde_json::json!({
        "id": CHANNEL_ID,
        "label": CHANNEL_LABEL,
        "protocol": CHANNEL_PROTOCOL,
        "ordered": false,
        "maxRetransmits": 0,
        "maxPacketLifeTime": null,
        "negotiated": true,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueError {
    Full,
    TooLarge,
    Closed,
    NotOpen,
}
impl std::fmt::Display for QueueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RTC queue {self:?}")
    }
}
impl std::error::Error for QueueError {}
#[derive(Default)]
struct State {
    open: bool,
    retired: bool,
    messages: VecDeque<Vec<u8>>,
    bytes: usize,
    peak: usize,
    peak_messages: usize,
}

/// One producer and one consumer; registration before checking avoids lost wakes.
pub struct DatagramQueue {
    limit: usize,
    maximum: usize,
    message_limit: usize,
    state: Mutex<State>,
    reader: AtomicWaker,
    writer: AtomicWaker,
}
impl DatagramQueue {
    pub fn new(limit: usize, maximum: usize) -> Self {
        assert!(maximum > 0 && maximum <= limit);
        Self {
            limit,
            maximum,
            message_limit: limit.div_ceil(maximum),
            state: Mutex::new(State::default()),
            reader: AtomicWaker::new(),
            writer: AtomicWaker::new(),
        }
    }
    pub fn open(&self) -> Result<(), QueueError> {
        let mut state = self.state.lock().unwrap();
        if state.retired {
            return Err(QueueError::Closed);
        }
        state.open = true;
        drop(state);
        self.writer.wake();
        Ok(())
    }
    pub fn try_push(&self, message: Vec<u8>) -> Result<(), QueueError> {
        let mut state = self.state.lock().unwrap();
        if state.retired {
            return Err(QueueError::Closed);
        }
        if !state.open {
            return Err(QueueError::NotOpen);
        }
        if message.is_empty() || message.len() > self.maximum {
            return Err(QueueError::TooLarge);
        }
        if state.bytes + message.len() > self.limit || state.messages.len() >= self.message_limit {
            return Err(QueueError::Full);
        }
        state.bytes += message.len();
        state.peak = state.peak.max(state.bytes);
        state.messages.push_back(message);
        state.peak_messages = state.peak_messages.max(state.messages.len());
        drop(state);
        self.reader.wake();
        Ok(())
    }
    pub fn try_pop(&self) -> Result<Option<Vec<u8>>, QueueError> {
        let mut state = self.state.lock().unwrap();
        if state.retired {
            return Err(QueueError::Closed);
        }
        let message = state.messages.pop_front();
        if let Some(message) = &message {
            state.bytes -= message.len();
        }
        drop(state);
        if message.is_some() {
            self.writer.wake();
        }
        Ok(message)
    }
    pub fn poll_pop(&self, cx: &mut Context<'_>) -> Poll<Result<Vec<u8>, QueueError>> {
        self.reader.register(cx.waker());
        match self.try_pop() {
            Ok(Some(message)) => Poll::Ready(Ok(message)),
            Ok(None) => Poll::Pending,
            Err(error) => Poll::Ready(Err(error)),
        }
    }
    pub fn poll_capacity(
        &self,
        cx: &mut Context<'_>,
        bytes: usize,
    ) -> Poll<Result<(), QueueError>> {
        self.writer.register(cx.waker());
        let state = self.state.lock().unwrap();
        if state.retired {
            return Poll::Ready(Err(QueueError::Closed));
        }
        if bytes == 0 || bytes > self.maximum {
            return Poll::Ready(Err(QueueError::TooLarge));
        }
        if state.open
            && state.bytes + bytes <= self.limit
            && state.messages.len() < self.message_limit
        {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }
    /// Cancel and close share a terminal transition; pending payloads are retired.
    pub fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.retired = true;
        state.open = false;
        state.messages.clear();
        state.bytes = 0;
        drop(state);
        self.reader.wake();
        self.writer.wake();
    }
    pub fn bytes(&self) -> usize {
        self.state.lock().unwrap().bytes
    }
    pub fn peak_messages(&self) -> usize {
        self.state.lock().unwrap().peak_messages
    }
    pub fn message_limit(&self) -> usize {
        self.message_limit
    }
    pub fn peak_bytes(&self) -> usize {
        self.state.lock().unwrap().peak
    }
}
