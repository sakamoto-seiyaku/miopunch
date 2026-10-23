//! Shared bounded file protocol and task state. Platform adapters own actual
//! sources and saves; a successful receipt is emitted only after `save` returns.
use std::{collections::VecDeque, future::Future, pin::Pin, sync::Mutex};

use futures_util::FutureExt;
use iroh::endpoint::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::data_stream::{CHUNK_BYTES, DataStream};

pub const FILE_ALPN: &[u8] = b"miopunch/file/1";
pub const ACTIVE_TASKS: usize = 4;
pub const TASK_HISTORY: usize = 64;

#[cfg(not(target_arch = "wasm32"))]
pub type FileFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, FileError>> + Send + 'a>>;
#[cfg(target_arch = "wasm32")]
pub type FileFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, FileError>> + 'a>>;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum FileError {
    #[error("cancelled")]
    Cancelled,
    #[error("service_disabled")]
    ServiceDisabled,
    #[error("busy")]
    Busy,
    #[error("source_failed")]
    SourceFailed,
    #[error("save_failed")]
    SaveFailed,
    #[error("invalid_file")]
    InvalidFile,
    #[error("integrity_mismatch")]
    IntegrityMismatch,
    #[error("interrupted")]
    Interrupted,
    #[error("rejected")]
    Rejected,
    #[error("unknown_task")]
    UnknownTask,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileOffer {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

impl FileOffer {
    pub fn validate(&self) -> Result<(), FileError> {
        if self.id.len() != 32
            || hex::decode(&self.id).is_err()
            || self.sha256.len() != 64
            || hex::decode(&self.sha256).is_err()
            || self.name.is_empty()
            || self.name.len() > 255
            || self
                .name
                .chars()
                .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
            || matches!(self.name.as_str(), "." | "..")
        {
            return Err(FileError::InvalidFile);
        }
        Ok(())
    }

    /// Unique no-overwrite destination. A remote name is never interpreted as
    /// a path; the transfer identifier also avoids replacing an existing file.
    pub fn saved_name(&self) -> String {
        let name = self.name.trim_end_matches(['.', ' ']);
        let mut end = name.len().min(222);
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}-{}", self.id, &name[..end])
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileReceipt {
    pub id: String,
    pub size: u64,
    pub sha256: String,
    pub saved_name: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum Reply {
    Ready { offset: u64 },
    Saved { receipt: FileReceipt },
    Failed { reason: FileError },
}

pub trait FileSource {
    fn read_at(&mut self, offset: u64, length: usize) -> FileFuture<'_, Vec<u8>>;
}

pub trait FileSink {
    fn write(&mut self, offset: u64, bytes: Vec<u8>) -> FileFuture<'_, ()>;
    fn save(&mut self) -> FileFuture<'_, ()>;
    fn discard(&mut self) -> FileFuture<'_, ()>;
}

pub struct PartialFile<S> {
    pub offer: FileOffer,
    pub peer: String,
    pub offset: u64,
    pub sink: S,
    pub hash: Sha256,
    pub receipt: Option<FileReceipt>,
}

impl<S> PartialFile<S> {
    pub fn new(offer: FileOffer, peer: String, sink: S) -> Self {
        Self {
            offer,
            peer,
            offset: 0,
            sink,
            hash: Sha256::new(),
            receipt: None,
        }
    }
}

fn wire(_: std::io::Error) -> FileError {
    FileError::Interrupted
}

pub async fn send_file(
    stream: &mut DataStream,
    offer: &FileOffer,
    source: &mut impl FileSource,
    mut progress: impl FnMut(u64, u64),
) -> Result<FileReceipt, FileError> {
    offer.validate()?;
    stream.write_control(offer).await.map_err(wire)?;
    let mut offset = match stream.read_control::<Reply>().await.map_err(wire)? {
        Reply::Ready { offset } if offset <= offer.size => offset,
        Reply::Failed { reason } => return Err(reason),
        Reply::Saved { receipt } => return checked_receipt(offer, receipt),
        _ => return Err(FileError::InvalidFile),
    };
    let resumed = offset;
    progress(offset, resumed);
    while offset < offer.size {
        let wanted = (offer.size - offset).min(CHUNK_BYTES as u64) as usize;
        let bytes = source.read_at(offset, wanted).await?;
        if bytes.len() != wanted {
            return Err(FileError::SourceFailed);
        }
        stream.write_chunk(&bytes).await.map_err(wire)?;
        offset += bytes.len() as u64;
        progress(offset, resumed);
    }
    stream.write_chunk(&[]).await.map_err(wire)?;
    stream.finish().map_err(wire)?;
    match stream.read_control::<Reply>().await.map_err(wire)? {
        Reply::Saved { receipt } => checked_receipt(offer, receipt),
        Reply::Failed { reason } => Err(reason),
        _ => Err(FileError::InvalidFile),
    }
}

fn checked_receipt(offer: &FileOffer, receipt: FileReceipt) -> Result<FileReceipt, FileError> {
    if receipt.id != offer.id
        || receipt.size != offer.size
        || receipt.sha256 != offer.sha256
        || receipt.saved_name != offer.saved_name()
    {
        return Err(FileError::IntegrityMismatch);
    }
    Ok(receipt)
}

/// Common receive owner: both platform adapters use these exact cancellation,
/// interruption, committed-receipt and cleanup-before-terminal transitions.
pub(crate) async fn run_receive<S: FileSink>(
    stream: &mut DataStream,
    partial: &mut PartialFile<S>,
    tasks: &FileTasks,
    connection: &Connection,
) -> Result<FileReceipt, FileError> {
    let id = partial.offer.id.clone();
    let cancel = tasks.token(&id).ok_or(FileError::UnknownTask)?;
    tasks.connection(&id, Some(connection));
    tasks.update(&id, |status| status.state = FileState::Transferring);
    let result = {
        let work = receive_file(stream, partial, |bytes, saving| {
            tasks.update(&id, |status| {
                status.bytes = bytes;
                status.state = if saving {
                    FileState::Saving
                } else {
                    FileState::Transferring
                };
            });
        })
        .fuse();
        let cancelled = cancel.cancelled().fuse();
        let peer_cancelled = cancelled_by_peer(connection).fuse();
        futures_util::pin_mut!(work, cancelled, peer_cancelled);
        futures_util::select_biased! { _ = cancelled => Err(FileError::Cancelled),
        _ = peer_cancelled => Err(FileError::Cancelled), result = work => result }
    };
    let mut result = if let Some(receipt) = &partial.receipt {
        Ok(receipt.clone())
    } else if cancel.is_cancelled() || peer_cancelled(connection) {
        Err(FileError::Cancelled)
    } else {
        result
    };
    tasks.connection(&id, None);
    if result == Err(FileError::Interrupted) && !cancel.is_cancelled() {
        tasks.update(&id, |status| status.state = FileState::Interrupted);
        return result;
    }
    if let Err(reason) = result {
        if reason == FileError::Cancelled {
            tasks.update(&id, |status| status.state = FileState::Cancelling);
        }
        if partial.sink.discard().await.is_err() {
            result = Err(FileError::SaveFailed);
        }
        let _ = stream
            .write_control(&Reply::Failed {
                reason: result.as_ref().err().copied().unwrap_or(reason),
            })
            .await;
        let _ = stream.finish();
    }
    tasks.finish(&id, &result);
    result
}

/// Preparation owns a sink before it can publish a partial. Rejection must
/// await that sink's cleanup and preserve a storage error on both platforms.
pub(crate) async fn reject_prepared<S: FileSink>(
    sink: &mut S,
    tasks: &FileTasks,
    id: &str,
    reason: FileError,
) -> FileError {
    let reason = if sink.discard().await.is_err() {
        FileError::SaveFailed
    } else {
        reason
    };
    tasks.finish(id, &Err(reason));
    reason
}

/// A retained, inactive partial has no network future to observe cancellation
/// or expiry. Its service owner joins real cleanup before publishing a result.
pub(crate) async fn discard_partial<S: FileSink>(
    partial: &mut PartialFile<S>,
    tasks: &FileTasks,
    reason: FileError,
) {
    let id = partial.offer.id.clone();
    let result = if let Some(receipt) = &partial.receipt {
        Ok(receipt.clone())
    } else if partial.sink.discard().await.is_err() {
        Err(FileError::SaveFailed)
    } else {
        Err(tasks
            .status_for(&id)
            .filter(|status| status.state == FileState::Failed)
            .and_then(|status| status.error)
            .unwrap_or(reason))
    };
    tasks.finish(&id, &result);
}

pub async fn receive_file<S: FileSink>(
    stream: &mut DataStream,
    partial: &mut PartialFile<S>,
    mut progress: impl FnMut(u64, bool),
) -> Result<FileReceipt, FileError> {
    if let Some(receipt) = &partial.receipt {
        stream
            .write_control(&Reply::Saved {
                receipt: receipt.clone(),
            })
            .await
            .map_err(wire)?;
        stream.finish().map_err(wire)?;
        return Ok(receipt.clone());
    }
    stream
        .write_control(&Reply::Ready {
            offset: partial.offset,
        })
        .await
        .map_err(wire)?;
    loop {
        let bytes = stream.read_chunk().await.map_err(wire)?;
        if bytes.is_empty() {
            break;
        }
        let end = partial
            .offset
            .checked_add(bytes.len() as u64)
            .ok_or(FileError::InvalidFile)?;
        if end > partial.offer.size {
            return Err(FileError::InvalidFile);
        }
        // Hash only a successfully saved chunk. Failed writes are terminal;
        // a transport interruption retains this exact sink and prefix hash.
        let mut hash = partial.hash.clone();
        hash.update(&bytes);
        partial.sink.write(partial.offset, bytes).await?;
        partial.hash = hash;
        partial.offset = end;
        progress(end, false);
    }
    if partial.offset != partial.offer.size
        || hex::encode(partial.hash.clone().finalize()) != partial.offer.sha256
    {
        return Err(FileError::IntegrityMismatch);
    }
    progress(partial.offset, true);
    partial.sink.save().await?;
    let receipt = FileReceipt {
        id: partial.offer.id.clone(),
        size: partial.offset,
        sha256: partial.offer.sha256.clone(),
        saved_name: partial.offer.saved_name(),
    };
    // Retain the completed receipt before ACK: a lost final reply can be
    // retried without re-saving or replacing the delivered file.
    partial.receipt = Some(receipt.clone());
    stream
        .write_control(&Reply::Saved {
            receipt: receipt.clone(),
        })
        .await
        .map_err(wire)?;
    stream.finish().map_err(wire)?;
    Ok(receipt)
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileState {
    Preparing,
    Transferring,
    Interrupted,
    Saving,
    Cancelling,
    Complete,
    Failed,
    Cancelled,
}
impl FileState {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Complete | Self::Failed | Self::Cancelled)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FileTaskStatus {
    pub id: String,
    pub direction: String,
    pub peer: String,
    pub name: String,
    pub size: u64,
    pub bytes: u64,
    pub state: FileState,
    pub attempts: u32,
    pub resumed_bytes: u64,
    pub receipt: Option<FileReceipt>,
    pub error: Option<FileError>,
    pub transport: Option<serde_json::Value>,
}
impl FileTaskStatus {
    pub fn new(id: String, direction: &str, peer: String, name: String, size: u64) -> Self {
        Self {
            id,
            direction: direction.into(),
            peer,
            name,
            size,
            bytes: 0,
            state: FileState::Preparing,
            attempts: 0,
            resumed_bytes: 0,
            receipt: None,
            error: None,
            transport: None,
        }
    }
}

struct Entry {
    status: FileTaskStatus,
    cancel: CancellationToken,
    connection: Option<Connection>,
    touched: n0_future::time::Instant,
}
#[derive(Default)]
pub struct FileTasks(Mutex<VecDeque<Entry>>);

impl FileTasks {
    pub fn begin(&self, status: FileTaskStatus) -> Result<CancellationToken, FileError> {
        let mut entries = self.0.lock().unwrap();
        if entries.iter().any(|entry| entry.status.id == status.id)
            || entries
                .iter()
                .filter(|entry| !entry.status.state.terminal())
                .count()
                >= ACTIVE_TASKS
        {
            return Err(FileError::Busy);
        }
        if entries.len() == TASK_HISTORY {
            let index = entries
                .iter()
                .position(|entry| entry.status.state.terminal())
                .ok_or(FileError::Busy)?;
            entries.remove(index);
        }
        let cancel = CancellationToken::new();
        entries.push_back(Entry {
            status,
            cancel: cancel.clone(),
            connection: None,
            touched: n0_future::time::Instant::now(),
        });
        Ok(cancel)
    }

    pub fn update(&self, id: &str, update: impl FnOnce(&mut FileTaskStatus)) {
        if let Some(entry) = self
            .0
            .lock()
            .unwrap()
            .iter_mut()
            .find(|entry| entry.status.id == id)
        {
            update(&mut entry.status);
            entry.touched = n0_future::time::Instant::now();
        }
    }

    pub fn connection(&self, id: &str, connection: Option<&Connection>) {
        if let Some(entry) = self
            .0
            .lock()
            .unwrap()
            .iter_mut()
            .find(|entry| entry.status.id == id)
        {
            entry.connection = connection.cloned();
            if let Some(connection) = connection {
                entry.status.attempts += 1;
                entry.status.transport = Some(path_status(connection));
            }
        }
    }

    pub fn status(&self) -> Vec<FileTaskStatus> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|entry| entry.status.clone())
            .collect()
    }

    pub(crate) fn status_for(&self, id: &str) -> Option<FileTaskStatus> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .find(|entry| entry.status.id == id)
            .map(|entry| entry.status.clone())
    }

    pub fn token(&self, id: &str) -> Option<CancellationToken> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .find(|entry| entry.status.id == id)
            .map(|entry| entry.cancel.clone())
    }

    pub fn finish(&self, id: &str, result: &Result<FileReceipt, FileError>) {
        self.update(id, |status| match result {
            Ok(receipt) => {
                status.state = FileState::Complete;
                status.bytes = receipt.size;
                status.receipt = Some(receipt.clone());
                status.error = None;
            }
            Err(error) => {
                status.state = if *error == FileError::Cancelled {
                    FileState::Cancelled
                } else {
                    FileState::Failed
                };
                status.error = Some(*error);
            }
        });
        self.connection(id, None);
    }

    pub fn cancel(&self, id: &str) -> Result<bool, FileError> {
        let mut entries = self.0.lock().unwrap();
        let entry = entries
            .iter_mut()
            .find(|entry| entry.status.id == id)
            .ok_or(FileError::UnknownTask)?;
        if !entry.status.state.terminal() && !entry.cancel.is_cancelled() {
            entry.cancel.cancel();
            if let Some(connection) = &entry.connection {
                connection.close(1u32.into(), b"file cancelled");
            }
            // Cancellation is a request. The platform owner publishes the
            // terminal state only after its I/O and partial cleanup finish.
            entry.status.state = FileState::Cancelling;
            return Ok(true);
        }
        Ok(false)
    }

    /// Technical host control: drop only this task's present connection. The
    /// sender may resume on a new admitted connection; this is not stream revival.
    pub fn disconnect(&self, id: &str) -> Result<(), FileError> {
        let entries = self.0.lock().unwrap();
        let entry = entries
            .iter()
            .find(|entry| entry.status.id == id)
            .ok_or(FileError::UnknownTask)?;
        let connection = entry.connection.as_ref().ok_or(FileError::Interrupted)?;
        connection.close(2u32.into(), b"file connection interrupted");
        Ok(())
    }

    pub fn cancel_all(&self, receives_only: bool) {
        for status in self.status() {
            if !receives_only || status.direction == "receive" {
                let _ = self.cancel(&status.id);
            }
        }
    }

    pub fn expired(&self, id: &str) -> bool {
        self.0
            .lock()
            .unwrap()
            .iter()
            .find(|entry| entry.status.id == id)
            .is_none_or(|entry| {
                entry.status.state == FileState::Interrupted
                    && entry.touched.elapsed().as_secs() >= 60
            })
    }
}

pub fn peer_cancelled(connection: &Connection) -> bool {
    matches!(connection.close_reason(), Some(iroh::endpoint::ConnectionError::ApplicationClosed(reason))
        if reason.error_code == 1u32.into())
}

/// A lost connection may still leave an already received file saving locally.
/// Only the explicit per-file cancellation code interrupts that save owner.
pub async fn cancelled_by_peer(connection: &Connection) {
    connection.closed().await;
    if !peer_cancelled(connection) {
        futures_util::future::pending::<()>().await;
    }
}

pub fn path_status(connection: &Connection) -> serde_json::Value {
    serde_json::json!({"peer":connection.remote_id().to_string(), "connection":connection.stable_id(),
        "paths":connection.paths().iter().map(|path| serde_json::json!({"remote":path.remote_addr(),
            "selected":path.is_selected(),"tx_bytes":path.stats().udp_tx.bytes,
            "rx_bytes":path.stats().udp_rx.bytes})).collect::<Vec<_>>()})
}

#[cfg(test)]
mod storage_tests {
    use super::*;
    use std::{io::Write, path::PathBuf};

    // An owned storage adapter at the FileSink seam. The exceptional case is
    // an ordinary remove_file error on our own directory, not a host hook.
    struct Storage {
        path: PathBuf,
        release: Option<tokio::sync::oneshot::Receiver<()>>,
    }
    impl FileSink for Storage {
        fn write(&mut self, offset: u64, bytes: Vec<u8>) -> FileFuture<'_, ()> {
            Box::pin(async move {
                use std::io::{Seek, SeekFrom};
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .open(&self.path)
                    .map_err(|_| FileError::SaveFailed)?;
                file.seek(SeekFrom::Start(offset))
                    .and_then(|_| file.write_all(&bytes))
                    .map_err(|_| FileError::SaveFailed)
            })
        }
        fn save(&mut self) -> FileFuture<'_, ()> {
            Box::pin(async move {
                std::fs::File::open(&self.path)
                    .and_then(|file| file.sync_all())
                    .map_err(|_| FileError::SaveFailed)
            })
        }
        fn discard(&mut self) -> FileFuture<'_, ()> {
            Box::pin(async move {
                if let Some(release) = self.release.take() {
                    release.await.map_err(|_| FileError::SaveFailed)?;
                }
                std::fs::remove_file(&self.path).map_err(|_| FileError::SaveFailed)
            })
        }
    }

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "miopunch-storage-test-{}",
                hex::encode(rand::random::<[u8; 16]>())
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn offer() -> FileOffer {
        FileOffer {
            id: "0123456789abcdef0123456789abcdef".into(),
            name: "received.txt".into(),
            size: 0,
            sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into(),
        }
    }

    fn begin(tasks: &FileTasks, offer: &FileOffer) {
        tasks
            .begin(FileTaskStatus::new(
                offer.id.clone(),
                "receive",
                "peer".into(),
                offer.name.clone(),
                offer.size,
            ))
            .unwrap();
    }

    #[tokio::test]
    async fn preparation_cancellation_waits_for_storage_cleanup() {
        let directory = Directory::new();
        let path = directory.0.join(".partial");
        std::fs::write(&path, b"partial").unwrap();
        let (release, waiting) = tokio::sync::oneshot::channel();
        let mut sink = Storage {
            path: path.clone(),
            release: Some(waiting),
        };
        let tasks = FileTasks::default();
        let offer = offer();
        begin(&tasks, &offer);
        tasks.cancel(&offer.id).unwrap();
        let mut cleanup = Box::pin(reject_prepared(
            &mut sink,
            &tasks,
            &offer.id,
            FileError::Cancelled,
        ));
        assert!(futures_util::poll!(cleanup.as_mut()).is_pending());
        assert_eq!(tasks.status()[0].state, FileState::Cancelling);
        assert!(path.is_file());
        release.send(()).unwrap();
        assert_eq!(cleanup.await, FileError::Cancelled);
        assert_eq!(tasks.status()[0].state, FileState::Cancelled);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn preparation_cleanup_failure_reports_save_failed() {
        let directory = Directory::new();
        let mut sink = Storage {
            path: directory.0.clone(),
            release: None,
        };
        let tasks = FileTasks::default();
        let offer = offer();
        begin(&tasks, &offer);
        tasks.cancel(&offer.id).unwrap();
        assert_eq!(
            reject_prepared(&mut sink, &tasks, &offer.id, FileError::Cancelled).await,
            FileError::SaveFailed
        );
        let status = &tasks.status()[0];
        assert_eq!(status.state, FileState::Failed);
        assert_eq!(status.error, Some(FileError::SaveFailed));
        assert!(status.receipt.is_none());
        assert!(directory.0.is_dir());
    }

    #[tokio::test]
    async fn retained_expiry_cleanup_failure_reports_save_failed() {
        let directory = Directory::new();
        let tasks = FileTasks::default();
        let offer = offer();
        begin(&tasks, &offer);
        tasks.update(&offer.id, |status| status.state = FileState::Interrupted);
        let mut partial = PartialFile::new(
            offer,
            "peer".into(),
            Storage {
                path: directory.0.clone(),
                release: None,
            },
        );
        discard_partial(&mut partial, &tasks, FileError::Interrupted).await;
        let status = &tasks.status()[0];
        assert_eq!(status.state, FileState::Failed);
        assert_eq!(status.error, Some(FileError::SaveFailed));
        assert!(status.receipt.is_none());
    }
}
