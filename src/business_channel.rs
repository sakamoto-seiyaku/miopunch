//! Application IO and reliable byte progress retained across member streams.
use std::{
    collections::VecDeque,
    future::{Future, poll_fn},
    io,
    sync::{Arc, Mutex},
    task::Poll,
};

use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf},
    sync::watch,
};

use crate::{
    data_stream::{CHUNK_BYTES, DataReader, DataStream, DataWriter},
    forwarding::ChannelRequest,
};

pub(crate) type ChannelFailure = Box<dyn std::error::Error + Send + Sync>;

/// Carrier lifetime and recoverable termination are supplied by each host adapter.
pub(crate) trait ChannelTransport {
    fn network_interrupted(&self) -> bool;
    fn closed(&self) -> impl Future<Output = ()> + Send;
}

#[derive(Default)]
struct Usage {
    send: usize,
    receive: usize,
    peak_send: usize,
    peak_receive: usize,
}
pub(crate) struct ChannelBudget {
    pub policy: crate::forwarding::ChannelPolicy,
    usage: Mutex<Usage>,
    changed: watch::Sender<u64>,
}
impl ChannelBudget {
    pub(crate) fn new(policy: crate::forwarding::ChannelPolicy) -> io::Result<Arc<Self>> {
        policy.validate()?;
        let (changed, _) = watch::channel(0);
        Ok(Arc::new(Self {
            policy,
            usage: Default::default(),
            changed,
        }))
    }
    pub(crate) fn notify(&self) {
        self.changed.send_modify(|v| *v = v.wrapping_add(1));
    }
    pub(crate) fn status(&self) -> serde_json::Value {
        let usage = self.usage.lock().unwrap();
        serde_json::json!({"send_bytes":usage.send,"receive_bytes":usage.receive,
            "peak_send_bytes":usage.peak_send,"peak_receive_bytes":usage.peak_receive,"policy":self.policy})
    }
}
#[derive(Debug)]
pub(crate) struct RecoveryExpired;
impl std::fmt::Display for RecoveryExpired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("business_channel_recovery_expired")
    }
}
impl std::error::Error for RecoveryExpired {}

#[cfg(feature = "nat-lab-test-hooks")]
#[derive(Default)]
pub(crate) struct TestHooks {
    pub paused: std::sync::atomic::AtomicBool,
    pub suppressed: std::sync::atomic::AtomicU64,
    pub delivery_limit: std::sync::atomic::AtomicU64,
}
const DATA: u8 = 0;
const PROGRESS: u8 = 1;
const FIN: u8 = 2;

impl ChannelRequest {
    pub(crate) fn new(origin: crate::forwarding::ChannelOrigin) -> Self {
        Self {
            origin,
            id: hex::encode(rand::random::<[u8; 16]>()),
            resume: false,
            generation: 0,
        }
    }
}

pub(crate) fn retryable(error: &ChannelFailure) -> bool {
    error.is::<n0_future::time::Elapsed>()
        || error.downcast_ref::<io::Error>().is_some_and(|e| {
            !matches!(
                e.kind(),
                io::ErrorKind::PermissionDenied
                    | io::ErrorKind::InvalidData
                    | io::ErrorKind::InvalidInput
                    | io::ErrorKind::Unsupported
            )
        })
}

pub(crate) enum ChannelError {
    NetworkInterrupted,
    RecoveryExpired,
    Business(io::Error),
}

fn carrier_error(transport: &impl ChannelTransport, error: io::Error) -> ChannelError {
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
    window: u64,
}
impl Progress {
    fn record(self) -> Vec<u8> {
        let mut record = vec![PROGRESS];
        record.extend_from_slice(&self.accepted.to_be_bytes());
        record.extend_from_slice(&self.delivered.to_be_bytes());
        record.push(u8::from(self.fin));
        record.extend_from_slice(&self.window.to_be_bytes());
        record
    }
    fn parse(record: &[u8]) -> io::Result<Self> {
        if record.len() != 26 || record[0] != PROGRESS || record[17] > 1 {
            return Err(invalid());
        }
        Ok(Self {
            accepted: u64::from_be_bytes(record[1..9].try_into().unwrap()),
            delivered: u64::from_be_bytes(record[9..17].try_into().unwrap()),
            fin: record[17] == 1,
            window: u64::from_be_bytes(record[18..26].try_into().unwrap()),
        })
    }
}
#[derive(Default)]
struct State {
    // Bytes stay here until the peer has accepted them into its channel buffer.
    pending: VecDeque<u8>,
    acknowledged: u64,
    remote_delivered: u64,
    remote_window: u64,
    local_fin: bool,
    fin_acknowledged: bool,
    received: VecDeque<u8>,
    progress: Progress,
    delivered_fin: bool,
    recovery_deadline: Option<n0_future::time::Instant>,
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
        self.remote_window = progress.window;
        self.fin_acknowledged |= progress.fin;
        Ok(())
    }
    fn new_bytes<'a>(&self, record: &'a [u8]) -> io::Result<&'a [u8]> {
        if record.len() < 9 {
            return Err(invalid());
        }
        let offset = u64::from_be_bytes(record[1..9].try_into().unwrap());
        if offset > self.progress.accepted {
            return Err(invalid());
        }
        let skipped = (self.progress.accepted - offset).min((record.len() - 9) as u64) as usize;
        Ok(&record[9 + skipped..])
    }
    fn accept(&mut self, record: &[u8], window: usize) -> io::Result<()> {
        if record.len() < 9 {
            return Err(invalid());
        }
        let offset = u64::from_be_bytes(record[1..9].try_into().unwrap());
        if offset > self.progress.accepted {
            return Err(invalid());
        }
        match record[0] {
            DATA if !self.progress.fin => {
                let bytes = self.new_bytes(record)?;
                if self.received.len() + bytes.len() > window {
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

/// Local IO stays alive while the carrier future waits for another stream.
/// Each carry owns all old stream futures, fencing them before replacement.
pub(crate) struct BusinessChannel {
    state: Mutex<State>,
    budget: Arc<ChannelBudget>,
    #[cfg(feature = "nat-lab-test-hooks")]
    hooks: Arc<TestHooks>,
}
impl Drop for BusinessChannel {
    fn drop(&mut self) {
        let state = self.state.lock().unwrap();
        let mut usage = self.budget.usage.lock().unwrap();
        usage.send -= state.pending.len();
        usage.receive -= state.received.len();
        drop(usage);
        self.budget.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Connected;

    impl ChannelTransport for Connected {
        fn network_interrupted(&self) -> bool { false }
        async fn closed(&self) { std::future::pending().await }
    }

    #[tokio::test]
    async fn resumed_carrier_advertises_window_freed_during_handshake() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let budget = ChannelBudget::new(crate::forwarding::ChannelPolicy {
            channel_bytes: 1,
            ..Default::default()
        }).unwrap();
        let channel = BusinessChannel::new(budget);
        {
            let mut state = channel.state.lock().unwrap();
            state.received.push_back(42);
            state.progress.accepted = 1;
            channel.budget.usage.lock().unwrap().receive = 1;
        }
        channel.begin_recovery();
        let (stream, mut outbound, mut inbound) = DataStream::local(128);
        let (mut application, mut writer) = tokio::io::duplex(1);
        let peer = async {
            let mut frame = [0; 26];
            assert_eq!(outbound.read_u32().await.unwrap(), 26);
            outbound.read_exact(&mut frame).await.unwrap();
            assert_eq!(Progress::parse(&frame).unwrap().delivered, 0);
            let drain = channel.deliver(&mut writer, false);
            tokio::pin!(drain);
            tokio::select! {
                biased;
                _ = &mut drain => panic!("channel closed before delivering progress"),
                byte = application.read_u8() => assert_eq!(byte.unwrap(), 42),
            }
            inbound.write_u32(26).await.unwrap();
            inbound.write_all(&Progress::default().record()).await.unwrap();
            assert_eq!(outbound.read_u32().await.unwrap(), 26);
            outbound.read_exact(&mut frame).await.unwrap();
            assert_eq!(Progress::parse(&frame).unwrap().delivered, 1);
        };
        tokio::select! {
            _ = channel.carry(stream, &Connected) => panic!("carrier ended"),
            result = tokio::time::timeout(std::time::Duration::from_secs(2), peer) => {
                result.expect("peer still sees a full receive window");
            }
        }
    }
}
impl BusinessChannel {
    pub(crate) fn new(budget: Arc<ChannelBudget>) -> Self {
        Self {
            state: Mutex::new(State {
                progress: Progress {
                    window: budget.policy.channel_bytes as u64,
                    ..Default::default()
                },
                ..Default::default()
            }),
            budget,
            #[cfg(feature = "nat-lab-test-hooks")]
            hooks: Default::default(),
        }
    }
    #[cfg(feature = "nat-lab-test-hooks")]
    pub(crate) fn set_test_hooks(&mut self, hooks: Arc<TestHooks>) {
        self.hooks = hooks;
    }

    pub(crate) async fn run<R, W, F>(
        &self,
        mut read: R,
        mut write: W,
        network: F,
    ) -> Result<(), ChannelFailure>
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
        F: Future<Output = Result<(), ChannelFailure>>,
    {
        let outcome = {
            let local = async {
                tokio::try_join!(self.read_local(&mut read), self.deliver(&mut write, false))?;
                std::future::pending::<io::Result<()>>().await
            };
            tokio::pin!(network, local);
            tokio::select! {
                result = &mut network => result,
                result = &mut local => result.map_err(Into::into),
            }
        };
        if outcome
            .as_ref()
            .is_err_and(|error| error.is::<RecoveryExpired>())
        {
            // Authorized callers keep all accepted bytes. Stop upload, drain the
            // receive queue in order, and expose EOF only after that drain.
            self.state.lock().unwrap().progress.fin = true;
            self.deliver(&mut write, true).await?;
        }
        outcome
    }

    async fn read_local<R: AsyncRead + Unpin>(&self, read: &mut R) -> io::Result<()> {
        let mut updates = self.budget.changed.subscribe();
        let mut bytes = vec![0; CHUNK_BYTES];
        loop {
            updates.borrow_and_update();
            let result = tokio::select! {
                result = poll_fn(|cx| {
                    let mut state = self.state.lock().unwrap();
                    let mut usage = self.budget.usage.lock().unwrap();
                    let available = self.budget.policy.channel_bytes.saturating_sub((state.end() - state.remote_delivered) as usize)
                        .min(self.budget.policy.node_bytes_per_direction - usage.send).min(bytes.len());
                    if available == 0 { return Poll::Pending; }
                    let mut buffer = ReadBuf::new(&mut bytes[..available]);
                    match Pin::new(&mut *read).poll_read(cx, &mut buffer) {
                        Poll::Pending => Poll::Pending,
                        Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
                        Poll::Ready(Ok(())) => {
                            let count = buffer.filled().len();
                            state.pending.extend(buffer.filled());
                            usage.send += count;
                            usage.peak_send = usage.peak_send.max(usage.send);
                            if count == 0 { state.local_fin = true; }
                            Poll::Ready(Ok(count))
                        }
                    }
                }) => result?,
                _ = updates.changed() => continue,
            };
            self.budget.notify();
            if result == 0 {
                return Ok(());
            }
        }
    }

    pub(crate) fn recovery_deadline(&self) -> Option<n0_future::time::Instant> {
        self.state.lock().unwrap().recovery_deadline
    }

    /// All retries after the same interruption share the original window.
    pub(crate) fn begin_recovery(&self) -> n0_future::time::Instant {
        *self.state.lock().unwrap().recovery_deadline.get_or_insert_with(|| {
            n0_future::time::Instant::now()
                + std::time::Duration::from_millis(self.budget.policy.recovery_ms)
        })
    }

    /// Run an established carrier to completion, or recover its original
    /// attachment within the channel's remaining window after a network cut.
    pub(crate) async fn carry_or_reopen<T, Ready, F>(
        &self,
        stream: DataStream,
        transport: &T,
        reopen: impl FnOnce(n0_future::time::Instant) -> F,
    ) -> Result<Option<(T, DataStream)>, ChannelFailure>
    where
        T: ChannelTransport,
        F: Future<Output = Result<(T, DataStream, Ready), ChannelFailure>>,
    {
        match self.carry(stream, transport).await {
            Ok(()) => return Ok(None),
            Err(ChannelError::Business(error)) => return Err(error.into()),
            Err(ChannelError::RecoveryExpired) => return Err(RecoveryExpired.into()),
            Err(ChannelError::NetworkInterrupted) => {}
        }
        let deadline = self.begin_recovery();
        let (transport, stream, _) = n0_future::time::timeout(
            deadline.saturating_duration_since(n0_future::time::Instant::now()),
            reopen(deadline),
        ).await.map_err(|_| RecoveryExpired)??;
        Ok(Some((transport, stream)))
    }

    pub(crate) async fn carry(
        &self,
        mut stream: DataStream,
        transport: &impl ChannelTransport,
    ) -> Result<(), ChannelError> {
        let advertised = self.state.lock().unwrap().progress;
        let handshake = async {
            stream
                .write_channel_record(&advertised.record())
                .await
                .map_err(|e| carrier_error(transport, e))?;
            let peer = stream
                .read_channel_record()
                .await
                .map_err(|e| carrier_error(transport, e))?;
            self.acknowledge(Progress::parse(&peer).map_err(ChannelError::Business)?)
        };
        if let Some(at) = self.recovery_deadline() {
            n0_future::time::timeout(
                at.saturating_duration_since(n0_future::time::Instant::now()),
                handshake,
            )
            .await
            .map_err(|_| ChannelError::RecoveryExpired)??;
        } else {
            handshake.await?;
        }
        self.state.lock().unwrap().recovery_deadline = None;
        let (send, recv) = stream.split();
        tokio::select! {
            result = async { tokio::try_join!(self.upload(send, transport, advertised), self.download(recv, transport))?; Ok(()) } => result,
            _ = transport.closed() => Err(carrier_error(transport, io::Error::new(io::ErrorKind::ConnectionAborted, "business carrier ended"))),
        }
    }
    fn acknowledge(&self, progress: Progress) -> Result<(), ChannelError> {
        let mut state = self.state.lock().unwrap();
        let before = state.pending.len();
        state
            .acknowledge(progress)
            .map_err(ChannelError::Business)?;
        self.budget.usage.lock().unwrap().send -= before - state.pending.len();
        drop(state);
        self.budget.notify();
        Ok(())
    }
    async fn upload(
        &self,
        mut send: DataWriter,
        transport: &impl ChannelTransport,
        mut last_progress: Progress,
    ) -> Result<(), ChannelError> {
        let mut updates = self.budget.changed.subscribe();
        let state = &self.state;
        let mut cursor = state.lock().unwrap().acknowledged;
        let mut fin_sent = false;
        loop {
            updates.borrow_and_update();
            let (record, complete) = {
                let state = state.lock().unwrap();
                let record = if state.progress != last_progress {
                    last_progress = state.progress;
                    Some(last_progress.record())
                } else if cursor
                    < state
                        .end()
                        .min(state.remote_delivered.saturating_add(state.remote_window))
                {
                    let count = (state
                        .end()
                        .min(state.remote_delivered.saturating_add(state.remote_window))
                        - cursor)
                        .min((CHUNK_BYTES - 9) as u64) as usize;
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
                } else if state.local_fin && cursor == state.end() && !fin_sent {
                    fin_sent = true;
                    let mut record = vec![FIN];
                    record.extend_from_slice(&state.end().to_be_bytes());
                    Some(record)
                } else {
                    None
                };
                (
                    record,
                    state.fin_acknowledged
                        && state.delivered_fin
                        && state.progress == last_progress,
                )
            };
            if let Some(record) = record {
                #[cfg(feature = "nat-lab-test-hooks")]
                if record[0] == PROGRESS
                    && self.hooks.paused.load(std::sync::atomic::Ordering::SeqCst)
                {
                    self.hooks
                        .suppressed
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    continue;
                }
                send.write_chunk_without_timeout(&record)
                    .await
                    .map_err(|e| carrier_error(transport, e))?;
                continue;
            }
            if complete {
                send.finish().map_err(|e| carrier_error(transport, e))?;
                return Ok(());
            }
            let _ = updates.changed().await;
        }
    }
    async fn download(
        &self,
        mut recv: DataReader,
        transport: &impl ChannelTransport,
    ) -> Result<(), ChannelError> {
        let mut updates = self.budget.changed.subscribe();
        loop {
            let record = recv
                .read_chunk_without_timeout()
                .await
                .map_err(|e| carrier_error(transport, e))?;
            let Some(record) = record else {
                return if self.state.lock().unwrap().progress.fin {
                    Ok(())
                } else {
                    Err(ChannelError::Business(invalid()))
                };
            };
            if record.first() == Some(&PROGRESS) {
                self.acknowledge(Progress::parse(&record).map_err(ChannelError::Business)?)?;
            } else {
                loop {
                    updates.borrow_and_update();
                    let (accepted, changed) = {
                        let mut state = self.state.lock().unwrap();
                        let mut usage = self.budget.usage.lock().unwrap();
                        if record.first() == Some(&DATA) {
                            let bytes = state.new_bytes(&record).map_err(ChannelError::Business)?;
                            let count = bytes
                                .len()
                                .min(self.budget.policy.node_bytes_per_direction - usage.receive)
                                .min(self.budget.policy.channel_bytes - state.received.len());
                            let complete = count == bytes.len();
                            let mut part = vec![DATA];
                            part.extend_from_slice(&state.progress.accepted.to_be_bytes());
                            part.extend_from_slice(&bytes[..count]);
                            state
                                .accept(&part, self.budget.policy.channel_bytes)
                                .map_err(ChannelError::Business)?;
                            usage.receive += count;
                            usage.peak_receive = usage.peak_receive.max(usage.receive);
                            (complete, count > 0)
                        } else {
                            state
                                .accept(&record, self.budget.policy.channel_bytes)
                                .map_err(ChannelError::Business)?;
                            (true, true)
                        }
                    };
                    if changed {
                        self.budget.notify();
                    }
                    if accepted {
                        break;
                    }
                    let _ = updates.changed().await;
                }
                self.budget.notify();
            }
        }
    }
    async fn deliver<W: AsyncWrite + Unpin>(
        &self,
        write: &mut W,
        draining: bool,
    ) -> io::Result<()> {
        let state = &self.state;
        let mut updates = self.budget.changed.subscribe();
        loop {
            updates.borrow_and_update();
            let (mut bytes, fin, finished) = {
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
            #[cfg(feature = "nat-lab-test-hooks")]
            if !draining {
                let limit = self
                    .hooks
                    .delivery_limit
                    .load(std::sync::atomic::Ordering::SeqCst);
                if limit > 0 {
                    let delivered = state.lock().unwrap().progress.delivered;
                    bytes.truncate(limit.saturating_sub(delivered) as usize);
                    if bytes.is_empty() && !fin {
                        let _ = updates.changed().await;
                        continue;
                    }
                }
            }
            #[cfg(not(feature = "nat-lab-test-hooks"))]
            let _ = draining;
            if !bytes.is_empty() {
                // write() is cancellation safe. Commit each successful partial write
                // before another await, so replacing a carrier cannot duplicate it.
                let count = write.write(&bytes).await?;
                if count == 0 {
                    return Err(io::ErrorKind::WriteZero.into());
                }
                let mut state = state.lock().unwrap();
                state.received.drain(..count);
                self.budget.usage.lock().unwrap().receive -= count;
                state.progress.delivered += count as u64;
                drop(state);
                self.budget.notify();
            } else if fin {
                write.shutdown().await?;
                state.lock().unwrap().delivered_fin = true;
                self.budget.notify();
                return Ok(());
            } else {
                let _ = updates.changed().await;
            }
        }
    }
}
