//! Application IO and reliable byte progress retained across member streams.
use std::{collections::VecDeque, io, sync::Mutex, time::Duration};

use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::watch,
};

use crate::{
    data_stream::{CHUNK_BYTES, DataReader, DataStream, DataWriter},
    forwarding::ChannelRequest,
    native_member_services::ServiceTransport,
};

// Resource defaults and node-wide accounting are selected by MIOPUNCH-173.
pub(crate) const RECOVERY: Duration = Duration::from_secs(60);
const WINDOW: usize = CHUNK_BYTES;
#[cfg(feature = "nat-lab-test-hooks")]
#[derive(Default)]
pub(crate) struct TestHooks {
    pub paused: std::sync::atomic::AtomicBool,
    pub suppressed: std::sync::atomic::AtomicU64,
}
const DATA: u8 = 0;
const PROGRESS: u8 = 1;
const FIN: u8 = 2;

impl ChannelRequest {
    pub(crate) fn new() -> Self {
        Self {
            id: hex::encode(rand::random::<[u8; 16]>()),
            resume: false,
        }
    }
}

pub(crate) enum ChannelError {
    NetworkInterrupted,
    Business(io::Error),
}

fn carrier_error(transport: &ServiceTransport, error: io::Error) -> ChannelError {
    if transport.network_interrupted() {
        ChannelError::NetworkInterrupted
    } else {
        ChannelError::Business(error)
    }
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid business channel progress",
    )
}

#[derive(Clone, Copy, Default, PartialEq)]
struct Progress {
    accepted: u64,
    delivered: u64,
    fin: bool,
}
impl Progress {
    fn record(self) -> Vec<u8> {
        let mut record = vec![PROGRESS];
        record.extend_from_slice(&self.accepted.to_be_bytes());
        record.extend_from_slice(&self.delivered.to_be_bytes());
        record.push(u8::from(self.fin));
        record
    }
    fn parse(record: &[u8]) -> io::Result<Self> {
        if record.len() != 18 || record[0] != PROGRESS || record[17] > 1 {
            return Err(invalid());
        }
        Ok(Self {
            accepted: u64::from_be_bytes(record[1..9].try_into().unwrap()),
            delivered: u64::from_be_bytes(record[9..17].try_into().unwrap()),
            fin: record[17] == 1,
        })
    }
}
#[derive(Default)]
struct State {
    // Bytes stay here until the peer has accepted them into its channel buffer.
    pending: VecDeque<u8>,
    acknowledged: u64,
    remote_delivered: u64,
    local_fin: bool,
    fin_acknowledged: bool,
    received: VecDeque<u8>,
    progress: Progress,
    delivered_fin: bool,
}
impl State {
    fn end(&self) -> u64 {
        self.acknowledged + self.pending.len() as u64
    }
    fn acknowledge(&mut self, progress: Progress) -> io::Result<()> {
        if progress.accepted < self.acknowledged
            || progress.accepted > self.end()
            || progress.delivered < self.remote_delivered
            || progress.delivered > progress.accepted
            || progress.fin && (!self.local_fin || progress.accepted != self.end())
        {
            return Err(invalid());
        }
        self.pending
            .drain(..(progress.accepted - self.acknowledged) as usize);
        self.acknowledged = progress.accepted;
        self.remote_delivered = progress.delivered;
        self.fin_acknowledged |= progress.fin;
        Ok(())
    }
    fn accept(&mut self, record: &[u8]) -> io::Result<()> {
        if record.len() < 9 {
            return Err(invalid());
        }
        let offset = u64::from_be_bytes(record[1..9].try_into().unwrap());
        if offset > self.progress.accepted {
            return Err(invalid());
        }
        match record[0] {
            DATA if !self.progress.fin => {
                let skipped =
                    (self.progress.accepted - offset).min((record.len() - 9) as u64) as usize;
                let bytes = &record[9 + skipped..];
                if self.received.len() + bytes.len() > WINDOW {
                    return Err(invalid());
                }
                self.received.extend(bytes);
                self.progress.accepted += bytes.len() as u64;
            }
            FIN if record.len() == 9 && offset == self.progress.accepted => {
                self.progress.fin = true
            }
            _ => return Err(invalid()),
        }
        Ok(())
    }
}

/// A single carry owns the stream futures. Dropping it fences every old stream
/// event before a replacement can touch the retained channel state.
pub(crate) struct BusinessChannel<R, W> {
    read: R,
    write: W,
    state: Mutex<State>,
    #[cfg(feature = "nat-lab-test-hooks")]
    hooks: std::sync::Arc<TestHooks>,
}
impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> BusinessChannel<R, W> {
    pub(crate) fn new(read: R, write: W) -> Self {
        Self {
            read,
            write,
            state: Mutex::new(State::default()),
            #[cfg(feature = "nat-lab-test-hooks")]
            hooks: Default::default(),
        }
    }
    #[cfg(feature = "nat-lab-test-hooks")]
    pub(crate) fn set_test_hooks(&mut self, hooks: std::sync::Arc<TestHooks>) {
        self.hooks = hooks;
    }
    pub(crate) async fn carry(
        &mut self,
        mut stream: DataStream,
        transport: &ServiceTransport,
    ) -> Result<(), ChannelError> {
        // Exchange accepted positions before replay, including an ACK lost with
        // the previous carrier. Neither side waits on application IO here.
        let progress = self.state.lock().unwrap().progress;
        stream
            .write_chunk(&progress.record())
            .await
            .map_err(|e| carrier_error(transport, e))?;
        let peer = stream
            .read_chunk()
            .await
            .map_err(|e| carrier_error(transport, e))?;
        self.state
            .lock()
            .unwrap()
            .acknowledge(Progress::parse(&peer).map_err(ChannelError::Business)?)
            .map_err(ChannelError::Business)?;
        let (send, recv) = stream.split();
        let (changed, _) = watch::channel(0u64);
        let upload = upload(
            &mut self.read,
            send,
            &self.state,
            &changed,
            transport,
            #[cfg(feature = "nat-lab-test-hooks")]
            &self.hooks,
        );
        let download = download(recv, &self.state, &changed, transport);
        let deliver = deliver(&mut self.write, &self.state, &changed);
        tokio::select! {
            result = async { tokio::try_join!(upload, download, deliver)?; Ok(()) } => result,
            _ = transport.closed() => Err(carrier_error(transport, io::Error::new(io::ErrorKind::ConnectionAborted, "business carrier ended"))),
        }
    }
}

async fn upload<R: AsyncRead + Unpin>(
    read: &mut R,
    mut send: DataWriter,
    state: &Mutex<State>,
    changed: &watch::Sender<u64>,
    transport: &ServiceTransport,
    #[cfg(feature = "nat-lab-test-hooks")] hooks: &TestHooks,
) -> Result<(), ChannelError> {
    let mut updates = changed.subscribe();
    let mut cursor = state.lock().unwrap().acknowledged;
    let mut fin_sent = false;
    let mut last_progress = state.lock().unwrap().progress;
    let mut buffer = vec![0; WINDOW];
    loop {
        updates.borrow_and_update();
        let (record, read_limit, complete) = {
            let state = state.lock().unwrap();
            let record = if state.progress != last_progress {
                last_progress = state.progress;
                Some(last_progress.record())
            } else if cursor < state.end() {
                let count = (state.end() - cursor).min((CHUNK_BYTES - 9) as u64) as usize;
                let mut record = vec![DATA];
                record.extend_from_slice(&cursor.to_be_bytes());
                record.extend(
                    state
                        .pending
                        .iter()
                        .skip((cursor - state.acknowledged) as usize)
                        .take(count),
                );
                cursor += count as u64;
                Some(record)
            } else if state.local_fin && !fin_sent {
                fin_sent = true;
                let mut record = vec![FIN];
                record.extend_from_slice(&state.end().to_be_bytes());
                Some(record)
            } else {
                None
            };
            let read_limit = if state.local_fin {
                0
            } else {
                WINDOW.saturating_sub((state.end() - state.remote_delivered) as usize)
            };
            (
                record,
                read_limit,
                state.fin_acknowledged && state.delivered_fin && state.progress == last_progress,
            )
        };
        if let Some(record) = record {
            #[cfg(feature = "nat-lab-test-hooks")]
            if record[0] == PROGRESS && hooks.paused.load(std::sync::atomic::Ordering::SeqCst) {
                hooks
                    .suppressed
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                continue;
            }
            send.write_chunk(&record)
                .await
                .map_err(|e| carrier_error(transport, e))?;
            continue;
        }
        if complete {
            send.finish().map_err(|e| carrier_error(transport, e))?;
            return Ok(());
        }
        tokio::select! {
            result = read.read(&mut buffer[..read_limit]), if read_limit > 0 => {
                let count = result.map_err(ChannelError::Business)?;
                let mut state = state.lock().unwrap();
                if count == 0 { state.local_fin = true; } else { state.pending.extend(&buffer[..count]); }
            }
            _ = updates.changed() => {},
        }
    }
}
async fn download(
    mut recv: DataReader,
    state: &Mutex<State>,
    changed: &watch::Sender<u64>,
    transport: &ServiceTransport,
) -> Result<(), ChannelError> {
    loop {
        let record = recv
            .read_chunk_or_end()
            .await
            .map_err(|e| carrier_error(transport, e))?;
        let mut state = state.lock().unwrap();
        let Some(record) = record else {
            return if state.progress.fin {
                Ok(())
            } else {
                Err(ChannelError::Business(invalid()))
            };
        };
        if record.first() == Some(&PROGRESS) {
            state
                .acknowledge(Progress::parse(&record).map_err(ChannelError::Business)?)
                .map_err(ChannelError::Business)?;
        } else {
            state.accept(&record).map_err(ChannelError::Business)?;
        }
        drop(state);
        changed.send_modify(|version| *version = version.wrapping_add(1));
    }
}
async fn deliver<W: AsyncWrite + Unpin>(
    write: &mut W,
    state: &Mutex<State>,
    changed: &watch::Sender<u64>,
) -> Result<(), ChannelError> {
    let mut updates = changed.subscribe();
    loop {
        updates.borrow_and_update();
        let (bytes, fin, finished) = {
            let state = state.lock().unwrap();
            (
                state.received.iter().copied().collect::<Vec<_>>(),
                state.progress.fin,
                state.delivered_fin,
            )
        };
        if finished {
            return Ok(());
        }
        if !bytes.is_empty() {
            // write() is cancellation safe. Commit each successful partial write
            // before another await, so replacing a carrier cannot duplicate it.
            let count = write.write(&bytes).await.map_err(ChannelError::Business)?;
            if count == 0 {
                return Err(ChannelError::Business(io::ErrorKind::WriteZero.into()));
            }
            let mut state = state.lock().unwrap();
            state.received.drain(..count);
            state.progress.delivered += count as u64;
            drop(state);
            changed.send_modify(|version| *version = version.wrapping_add(1));
        } else if fin {
            write.shutdown().await.map_err(ChannelError::Business)?;
            state.lock().unwrap().delivered_fin = true;
            changed.send_modify(|version| *version = version.wrapping_add(1));
            return Ok(());
        } else {
            let _ = updates.changed().await;
        }
    }
}
