//! Explicit, bounded private encoder observations for an owned integration fixture.
//! No observer is installed by default. The callback is observational: no I/O,
//! async waits, or QUIC re-entry. Fixture-only synchronization must protect bounded
//! metadata without retaining SDK/transport locks or awaiting another SDK task.
//! Observations cannot affect packet construction or permission.

use std::{
    cell::Cell,
    io,
    marker::PhantomData,
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

const RECORD_LIMIT: usize = 16_384;
const BYTE_LIMIT: usize = 16 * 1024 * 1024;
static OBSERVER: OnceLock<Box<dyn Fn(Record) + Send + Sync>> = OnceLock::new();
static ENABLED: AtomicBool = AtomicBool::new(false);
static RECORDS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
static LOST: AtomicUsize = AtomicUsize::new(0);
thread_local! {
    static CONNECTION: Cell<Option<usize>> = const { Cell::new(None) };
    static EMITTING: Cell<bool> = const { Cell::new(false) };
}

/// Actual scalar or bounded private bytes; no field is inferred by this adapter.
#[derive(Clone, Debug)]
pub enum Value {
    U64(u64),
    Bool(bool),
    Text(String),
    Bytes(Vec<u8>),
}
/// One observation, with the real drive-poll connection context when available.
#[derive(Clone, Debug)]
pub struct Record {
    pub sequence: usize,
    pub connection_stable_id: Option<usize>,
    pub stage: &'static str,
    pub fields: Vec<(&'static str, Value)>,
}
/// Capture loss and bounds; the fixture must also check its own writer status.
#[derive(Clone, Copy, Debug)]
pub struct Status {
    pub records: usize,
    pub bytes: usize,
    pub lost: usize,
}
/// Install only in an explicitly opted-in fixture before its endpoints start.
/// There is no dependency on an application's tracing subscriber or logger.
pub fn install(observer: impl Fn(Record) + Send + Sync + 'static) -> io::Result<()> {
    OBSERVER.set(Box::new(observer)).map_err(|_| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "private packet observer already installed",
        )
    })?;
    ENABLED.store(true, Ordering::Release);
    Ok(())
}
/// Whether the explicitly installed private observer is still accepting events.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Acquire)
}
/// Stop capture after all owned producers have ended; does not alter any transport.
pub fn finish() -> Status {
    ENABLED.store(false, Ordering::Release);
    Status {
        records: RECORDS.load(Ordering::Acquire),
        bytes: BYTES.load(Ordering::Acquire),
        lost: LOST.load(Ordering::Acquire),
    }
}
fn loss() {
    LOST.fetch_add(1, Ordering::Relaxed);
    ENABLED.store(false, Ordering::Release);
}

/// Scope one real synchronous QUIC drive poll, not an async task lifetime.
pub struct ConnectionScope {
    previous: Option<usize>,
    active: bool,
    _thread: PhantomData<Rc<()>>,
}
pub fn enter_connection(stable: usize) -> ConnectionScope {
    let active = enabled();
    let previous = if active {
        CONNECTION.with(|v| v.replace(Some(stable)))
    } else {
        None
    };
    ConnectionScope {
        previous,
        active,
        _thread: PhantomData,
    }
}
impl Drop for ConnectionScope {
    fn drop(&mut self) {
        if self.active {
            CONNECTION.with(|v| v.set(self.previous));
        }
    }
}
struct Emitting;
impl Drop for Emitting {
    fn drop(&mut self) {
        EMITTING.with(|v| v.set(false));
    }
}

/// Emit only actual source values. Byte copies are private observations, never a queue.
pub fn emit(stage: &'static str, fields: Vec<(&'static str, Value)>) {
    if !enabled() {
        return;
    }
    if EMITTING.with(|v| v.replace(true)) {
        loss();
        return;
    }
    let _emitting = Emitting;
    let bytes = fields
        .iter()
        .map(|(name, value)| {
            name.len()
                + match value {
                    Value::U64(_) | Value::Bool(_) => 8,
                    Value::Text(value) => value.len(),
                    Value::Bytes(value) => value.len(),
                }
        })
        .sum::<usize>();
    if fields.len() > 32
        || fields.iter().any(|(_, value)| match value {
            Value::Text(value) => value.len() > 256 * 1024,
            Value::Bytes(value) => value.len() > 64 * 1024,
            _ => false,
        })
        || BYTES
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                n.checked_add(bytes).filter(|n| *n <= BYTE_LIMIT)
            })
            .is_err()
    {
        loss();
        return;
    }
    let sequence = match RECORDS.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
        n.checked_add(1).filter(|n| *n <= RECORD_LIMIT)
    }) {
        Ok(previous) => previous + 1,
        Err(_) => {
            loss();
            return;
        }
    };
    let record = Record {
        sequence,
        connection_stable_id: CONNECTION.with(Cell::get),
        stage,
        fields,
    };
    if let Some(observer) = OBSERVER.get() {
        if catch_unwind(AssertUnwindSafe(|| observer(record))).is_err() {
            loss();
        }
    }
}
/// Exact identity at the real pump boundary. The caller hashes its actual packet.
pub fn emit_pump(
    stage: &'static str,
    stable: usize,
    len: usize,
    digest: &str,
    active: bool,
    monotonic_ms: u64,
) {
    if enabled() {
        emit(
            stage,
            vec![
                ("original_stable_id", Value::U64(stable as u64)),
                ("packet_len", Value::U64(len as u64)),
                ("ciphertext_sha256", Value::Text(digest.to_owned())),
                ("route_active", Value::Bool(active)),
                ("monotonic_ms", Value::U64(monotonic_ms)),
            ],
        );
    }
}
/// Declare an actual ordinary write before it occurs. This alone proves no encoded payload.
pub fn emit_ordinary_stream(stable: usize, stream: crate::StreamId, len: usize, digest: &str) {
    if enabled() {
        emit(
            "ordinary-stream-declaration",
            vec![
                ("original_stable_id", Value::U64(stable as u64)),
                ("stream_id", Value::U64(u64::from(stream))),
                ("payload_len", Value::U64(len as u64)),
                ("payload_sha256", Value::Text(digest.to_owned())),
            ],
        );
    }
}
