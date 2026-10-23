//! Native filesystem adapter for the shared file operation. No file bytes pass
//! through a host control response; the node reads, streams and saves them.
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    sync::{Arc, Mutex},
    time::Duration,
};

use iroh::{EndpointAddr, endpoint::Connection};
use sha2::{Digest, Sha256};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
};

use crate::{
    data_stream::{CHUNK_BYTES, DataStream},
    file_transfer::{
        self, ACTIVE_TASKS, FileError, FileFuture, FileOffer, FileReceipt, FileSink, FileSource,
        FileState, FileTaskStatus, FileTasks, PartialFile, Reply,
    },
    resources::ResourceEndpoint,
};

struct NativeSource(File);
impl FileSource for NativeSource {
    fn read_at(&mut self, offset: u64, length: usize) -> FileFuture<'_, Vec<u8>> {
        Box::pin(async move {
            self.0
                .seek(std::io::SeekFrom::Start(offset))
                .await
                .map_err(|_| FileError::SourceFailed)?;
            let mut bytes = vec![0; length];
            self.0
                .read_exact(&mut bytes)
                .await
                .map_err(|_| FileError::SourceFailed)?;
            Ok(bytes)
        })
    }
}

struct NativeSink {
    file: Option<File>,
    temporary: PathBuf,
    destination: PathBuf,
}
impl NativeSink {
    async fn new(directory: &Path, offer: &FileOffer) -> Result<Self, FileError> {
        let temporary = directory.join(format!(".miopunch-{}.partial", offer.id));
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options
            .open(&temporary)
            .await
            .map_err(|_| FileError::SaveFailed)?;
        Ok(Self {
            file: Some(file),
            temporary,
            destination: directory.join(offer.saved_name()),
        })
    }

    async fn abort(&mut self) -> Result<(), FileError> {
        if let Some(file) = self.file.take() {
            // Join a cancelled Tokio filesystem operation before closing its
            // Windows handle and deleting the private partial.
            drop(file.into_std().await);
        }
        match std::fs::remove_file(&self.temporary) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(FileError::SaveFailed),
        }
    }
}
impl FileSink for NativeSink {
    fn write(&mut self, offset: u64, bytes: Vec<u8>) -> FileFuture<'_, ()> {
        Box::pin(async move {
            let file = self.file.as_mut().ok_or(FileError::SaveFailed)?;
            file.seek(std::io::SeekFrom::Start(offset))
                .await
                .map_err(|_| FileError::SaveFailed)?;
            file.write_all(&bytes)
                .await
                .map_err(|_| FileError::SaveFailed)
        })
    }
    fn save(&mut self) -> FileFuture<'_, ()> {
        Box::pin(async move {
            // Keep the handle owned by the sink through every cancellable I/O
            // await, so abort can join it before deleting a Windows partial.
            let file = self.file.as_mut().ok_or(FileError::SaveFailed)?;
            file.flush().await.map_err(|_| FileError::SaveFailed)?;
            file.sync_all().await.map_err(|_| FileError::SaveFailed)?;
            drop(self.file.take());
            crate::native_files::publish_file(&self.temporary, &self.destination)
                .map_err(|_| FileError::SaveFailed)?;
            // Unix publication retains the temporary link; Windows moved it.
            if self.temporary.exists() {
                std::fs::remove_file(&self.temporary).map_err(|_| FileError::SaveFailed)?;
            }
            crate::native_files::sync_directory(
                self.destination.parent().ok_or(FileError::SaveFailed)?,
            )
            .map_err(|_| FileError::SaveFailed)
        })
    }
    fn discard(&mut self) -> FileFuture<'_, ()> {
        Box::pin(self.abort())
    }
}
impl Drop for NativeSink {
    fn drop(&mut self) {
        self.file.take();
        let _ = std::fs::remove_file(&self.temporary);
    }
}

type Partial = Arc<tokio::sync::Mutex<PartialFile<NativeSink>>>;

pub(crate) struct NativeFiles {
    gate: crate::admission_gate::AdmissionGate,
    access: Mutex<Option<crate::service_policy::ServiceAccess>>,
    directory: Mutex<Option<PathBuf>>,
    partials: tokio::sync::Mutex<BTreeMap<String, Partial>>,
    pub tasks: FileTasks,
    jobs: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    generation: AtomicU64,
    dials: tokio::sync::Mutex<()>,
    receivers: Mutex<BTreeMap<usize, Connection>>,
    stopped: AtomicBool,
}

pub(crate) struct FileOwner(pub Arc<NativeFiles>);
impl Drop for FileOwner {
    fn drop(&mut self) {
        self.0.stopped.store(true, Ordering::Release);
        self.0.deactivate();
        self.0.tasks.cancel_all(false);
        self.0.close_receivers();
    }
}

struct ReceiveOwner {
    service: Arc<NativeFiles>,
    connection: usize,
}
impl Drop for ReceiveOwner {
    fn drop(&mut self) {
        self.service
            .receivers
            .lock()
            .unwrap()
            .remove(&self.connection);
    }
}

impl NativeFiles {
    pub fn new(gate: crate::admission_gate::AdmissionGate) -> Self {
        Self {
            gate,
            access: Mutex::new(None),
            directory: Mutex::new(None),
            partials: Default::default(),
            tasks: Default::default(),
            jobs: Default::default(),
            generation: Default::default(),
            dials: Default::default(),
            receivers: Default::default(),
            stopped: Default::default(),
        }
    }
    pub fn update_access(self: &Arc<Self>, access: crate::service_policy::ServiceAccess) {
        *self.access.lock().unwrap() = Some(access);
        self.enforce_access();
    }
    fn permits(&self, peer: &str, receive: bool) -> bool {
        peer.parse().ok().is_some_and(|peer| {
            self.access.lock().unwrap().as_ref().is_some_and(|access| {
                if receive {
                    access.can_receive(peer)
                } else {
                    access.can_send(peer)
                }
            })
        })
    }
    fn enforce_access(self: &Arc<Self>) {
        let access = self.access.lock().unwrap().clone();
        let Some(access) = access else { return };
        let mut peers: Vec<_> = access.auth.nodes().into_iter().map(|(id, _)| id).collect();
        peers.extend(
            access
                .ledger
                .guests()
                .into_iter()
                .filter_map(|guest| guest.guest),
        );
        let denied = peers
            .into_iter()
            .filter(|peer| !access.can_receive(*peer))
            .map(|id| *id.as_bytes())
            .collect();
        self.gate.update_file_policy(access.guest_pairs(), denied);
        for connection in self.receivers.lock().unwrap().values() {
            if !self.permits(&connection.remote_id().to_string(), true) {
                connection.close(1u32.into(), b"file access revoked");
            }
        }
        for task in self.tasks.status() {
            if !task.state.terminal() && !self.permits(&task.peer, task.direction == "receive") {
                let _ = self.cancel(&task.id);
            }
        }
    }
    pub async fn enable(&self, directory: &Path) -> Result<(), FileError> {
        let generation = self.generation.load(Ordering::Acquire);
        if self.directory.lock().unwrap().is_some() {
            return Err(FileError::Busy);
        }
        tokio::fs::create_dir_all(directory)
            .await
            .map_err(|_| FileError::SaveFailed)?;
        let directory = tokio::fs::canonicalize(directory)
            .await
            .map_err(|_| FileError::SaveFailed)?;
        if !directory.is_dir() {
            return Err(FileError::SaveFailed);
        }
        let mut setting = self.directory.lock().unwrap();
        if setting.is_some() || generation != self.generation.load(Ordering::Acquire) {
            return Err(FileError::Busy);
        }
        *setting = Some(directory);
        self.generation.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    fn deactivate(&self) {
        self.directory.lock().unwrap().take();
        self.generation.fetch_add(1, Ordering::AcqRel);
    }

    fn close_receivers(&self) {
        // This also wakes network setup before a task/partial exists. Its
        // storage future remains owned and is joined; closing the transport
        // never aborts an in-flight filesystem operation.
        for connection in self.receivers.lock().unwrap().values() {
            connection.close(1u32.into(), b"file service disabled");
        }
    }

    pub async fn disable(&self) {
        self.deactivate();
        self.tasks.cancel_all(true);
        self.close_receivers();
        let partials: Vec<_> = self
            .partials
            .lock()
            .await
            .iter()
            .map(|(id, partial)| (id.clone(), partial.clone()))
            .collect();
        for (id, partial) in partials {
            // Join any current write before returning the disabled state.
            // Dropping the last sink removes only this service's partial file.
            file_transfer::discard_partial(
                &mut *partial.lock().await,
                &self.tasks,
                FileError::Cancelled,
            )
            .await;
            self.partials.lock().await.remove(&id);
        }
    }

    pub fn enabled(&self) -> bool {
        self.directory.lock().unwrap().is_some()
            && self.access.lock().unwrap().as_ref().is_some_and(|access| {
                !access.member(access.local)
                    || access
                        .ledger
                        .declaration(access.local, crate::service_policy::Service::Files)
                        .is_some_and(|declaration| declaration.capable && declaration.enabled)
            })
    }

    fn track_job(&self, job: tokio::task::JoinHandle<()>) {
        let mut jobs = self.jobs.lock().unwrap();
        jobs.retain(|job| !job.is_finished());
        jobs.push(job);
    }

    pub fn cancel(self: &Arc<Self>, id: &str) -> Result<bool, FileError> {
        let requested = self.tasks.cancel(id)?;
        if requested
            && self
                .tasks
                .status_for(id)
                .is_some_and(|status| status.direction == "receive")
        {
            let service = self.clone();
            let id = id.to_owned();
            self.track_job(tokio::spawn(async move {
                let partial = service.partials.lock().await.get(&id).cloned();
                if let Some(partial) = partial {
                    file_transfer::discard_partial(
                        &mut *partial.lock().await,
                        &service.tasks,
                        FileError::Cancelled,
                    )
                    .await;
                    service.partials.lock().await.remove(&id);
                }
            }));
        }
        Ok(requested)
    }

    pub async fn expire(self: &Arc<Self>) {
        self.enforce_access();
        let expired: Vec<_> = self
            .partials
            .lock()
            .await
            .keys()
            .filter(|id| self.tasks.expired(id))
            .cloned()
            .collect();
        for id in expired {
            let partial = self.partials.lock().await.get(&id).cloned();
            if let Some(partial) = partial {
                let mut partial = partial.lock().await;
                if self.tasks.expired(&id) {
                    file_transfer::discard_partial(
                        &mut *partial,
                        &self.tasks,
                        FileError::Interrupted,
                    )
                    .await;
                    self.partials.lock().await.remove(&id);
                }
            }
        }
    }

    pub async fn send(
        self: &Arc<Self>,
        resources: Arc<ResourceEndpoint>,
        address: EndpointAddr,
        path: PathBuf,
    ) -> Result<String, FileError> {
        if !self.permits(&address.id.to_string(), false) {
            return Err(FileError::Rejected);
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(FileError::InvalidFile)?
            .to_string();
        let metadata = tokio::fs::metadata(&path)
            .await
            .map_err(|_| FileError::SourceFailed)?;
        if !metadata.is_file() {
            return Err(FileError::SourceFailed);
        }
        let id = hex::encode(rand::random::<[u8; 16]>());
        let cancel = self.tasks.begin(FileTaskStatus::new(
            id.clone(),
            "send",
            address.id.to_string(),
            name.clone(),
            metadata.len(),
        ))?;
        let service = self.clone();
        let task_id = id.clone();
        let job = tokio::spawn(async move {
            let work = service.send_inner(
                resources,
                address,
                path,
                task_id.clone(),
                name,
                metadata.len(),
            );
            let result = tokio::select! { biased;
                _ = cancel.cancelled() => Err(FileError::Cancelled),
                result = work => result,
            };
            service.tasks.finish(&task_id, &result);
        });
        self.track_job(job);
        Ok(id)
    }

    async fn send_inner(
        self: &Arc<Self>,
        resources: Arc<ResourceEndpoint>,
        address: EndpointAddr,
        path: PathBuf,
        id: String,
        name: String,
        size: u64,
    ) -> Result<FileReceipt, FileError> {
        let mut source = NativeSource(
            File::open(path)
                .await
                .map_err(|_| FileError::SourceFailed)?,
        );
        let mut hash = Sha256::new();
        let mut buffer = vec![0; CHUNK_BYTES];
        loop {
            let length = source
                .0
                .read(&mut buffer)
                .await
                .map_err(|_| FileError::SourceFailed)?;
            if length == 0 {
                break;
            }
            hash.update(&buffer[..length]);
        }
        let offer = FileOffer {
            id: id.clone(),
            name,
            size,
            sha256: hex::encode(hash.finalize()),
        };
        offer.validate()?;
        for attempt in 0..3 {
            // The existing resource owner admits one pending contact per peer.
            // Serialize only setup, then let bounded file streams run together.
            let connection = {
                let _dial = self.dials.lock().await;
                resources
                    .connect(address.clone(), file_transfer::FILE_ALPN)
                    .await
                    .map_err(|_| FileError::Interrupted)?
            };
            self.tasks.connection(&id, Some(&connection));
            let result = async {
                let mut stream = DataStream::open(&connection)
                    .await
                    .map_err(|_| FileError::Interrupted)?;
                file_transfer::send_file(&mut stream, &offer, &mut source, |bytes, resumed| {
                    self.tasks.update(&id, |status| {
                        status.state = FileState::Transferring;
                        status.bytes = bytes;
                        status.resumed_bytes = status.resumed_bytes.max(resumed);
                    });
                })
                .await
            }
            .await;
            let result = if file_transfer::peer_cancelled(&connection) {
                Err(FileError::Cancelled)
            } else {
                result
            };
            connection.close(0u32.into(), b"file send ended");
            self.tasks.connection(&id, None);
            if result != Err(FileError::Interrupted) || attempt == 2 {
                return result;
            }
            self.tasks
                .update(&id, |status| status.state = FileState::Interrupted);
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        Err(FileError::Interrupted)
    }

    async fn prepare(
        self: &Arc<Self>,
        offer: &FileOffer,
        peer: &str,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<Partial, FileError> {
        offer.validate()?;
        if !self.enabled() {
            return Err(FileError::ServiceDisabled);
        }
        if !self.permits(peer, true) {
            return Err(FileError::Rejected);
        }
        let generation = self.generation.load(Ordering::Acquire);
        let directory = self
            .directory
            .lock()
            .unwrap()
            .clone()
            .ok_or(FileError::ServiceDisabled)?;
        self.expire().await;
        let mut partials = self.partials.lock().await;
        if generation != self.generation.load(Ordering::Acquire) {
            return Err(FileError::ServiceDisabled);
        }
        if let Some(partial) = partials.get(&offer.id) {
            let partial = partial.clone();
            drop(partials);
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
        // Completed sinks are closed. Their cache bound is independent of
        // summary eviction/reuse by unrelated send or rejected receive tasks.
        if partials.len() >= file_transfer::TASK_HISTORY {
            let completed = partials.iter().find_map(|(id, partial)| {
                partial
                    .try_lock()
                    .ok()
                    .filter(|file| file.receipt.is_some())
                    .map(|_| id.clone())
            });
            if let Some(id) = completed {
                partials.remove(&id);
            } else {
                return Err(FileError::Busy);
            }
        }
        if partials
            .values()
            .filter(|partial| {
                partial
                    .try_lock()
                    .map_or(true, |file| file.receipt.is_none())
            })
            .count()
            >= ACTIVE_TASKS
        {
            return Err(FileError::Busy);
        }
        let mut sink = match NativeSink::new(&directory, offer).await {
            Ok(sink) => sink,
            Err(reason) => {
                return Err(reason);
            }
        };
        if cancel.is_cancelled() || generation != self.generation.load(Ordering::Acquire) {
            let reason = if cancel.is_cancelled() {
                FileError::Cancelled
            } else {
                FileError::ServiceDisabled
            };
            let reason =
                file_transfer::reject_prepared(&mut sink, &self.tasks, &offer.id, reason).await;
            return Err(reason);
        }
        let partial = Arc::new(tokio::sync::Mutex::new(PartialFile::new(
            offer.clone(),
            peer.into(),
            sink,
        )));
        partials.insert(offer.id.clone(), partial.clone());
        Ok(partial)
    }

    async fn reject_receive(&self, offer: &FileOffer, peer: &str, reason: FileError) {
        let partial = self.partials.lock().await.get(&offer.id).cloned();
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
                self.partials.lock().await.remove(&offer.id);
            }
        } else {
            self.tasks.fail_receive(&offer.id, reason);
        }
    }

    /// Once file I/O can start, the service owns the entire future. The
    /// listener may abort its finite handshakes, but never an in-flight
    /// filesystem open/write/save whose completion must be joined at close.
    pub fn receive(self: &Arc<Self>, connection: Connection) {
        {
            let mut receivers = self.receivers.lock().unwrap();
            if self.stopped.load(Ordering::Acquire) {
                connection.close(1u32.into(), b"file host closed");
                return;
            }
            if receivers.len() >= 16 {
                connection.close(0u32.into(), b"file receiver busy");
                return;
            }
            receivers.insert(connection.stable_id(), connection.clone());
        }
        let owner = ReceiveOwner {
            service: self.clone(),
            connection: connection.stable_id(),
        };
        self.track_job(tokio::spawn(async move {
            let owner = owner;
            owner.service.receive_inner(connection).await;
        }));
    }

    async fn receive_inner(self: &Arc<Self>, connection: Connection) {
        let mut stream = match DataStream::accept(&connection).await {
            Ok(stream) => stream,
            Err(_) => return,
        };
        let offer = match stream.read_control::<FileOffer>().await {
            Ok(offer) => offer,
            Err(_) => return,
        };
        let peer = connection.remote_id().to_string();
        if !self.enabled() || !self.permits(&peer, true) {
            let reason = if self.enabled() {
                FileError::Rejected
            } else {
                FileError::ServiceDisabled
            };
            let _ = stream.write_control(&Reply::Failed { reason }).await;
            let _ = stream.finish();
            wait_for_sender(&connection).await;
            return;
        }
        let needs_approval = self.enabled()
            && peer.parse().ok().is_some_and(|peer| {
                self.access
                    .lock()
                    .unwrap()
                    .as_ref()
                    .is_some_and(|access| !access.member(peer) && access.can_receive(peer))
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
                self.partials.lock().await.remove(&offer.id);
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
            self.partials.lock().await.remove(&id);
        }
        drop(partial);
        wait_for_sender(&connection).await;
    }

    pub async fn close(&self) {
        self.stopped.store(true, Ordering::Release);
        self.tasks.cancel_all(false);
        // Invalidate preparation before joining it. A pending create/open
        // must complete under its owner and clean up, rather than becoming a
        // fresh uncancelled receive after the close request.
        self.disable().await;
        let jobs = std::mem::take(&mut *self.jobs.lock().unwrap());
        for job in jobs {
            let _ = job.await;
        }
    }
}

async fn wait_for_sender(connection: &Connection) {
    let _ = tokio::time::timeout(Duration::from_secs(5), connection.closed()).await;
    connection.close(0u32.into(), b"file receive ended");
}
