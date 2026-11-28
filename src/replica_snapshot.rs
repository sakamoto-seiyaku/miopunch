//! Shared bounded LogSync snapshot and manifest used on native and browser.
use crate::codec::{Codec, CodecError};
use crate::management_auth::{ManagementExtension, NetworkAnchor};
use crate::management_record::ManagementRecord;
use crate::management_record::{MAX_PAGE_BYTES, MAX_PAGE_RECORDS, history_page};
use futures_util::{Sink, Stream};
use iroh::endpoint::{ControlExchange, ControlMessageFormat};
use p2panda_core::{Body, Hash, Header, Operation, SeqNum, VerifyingKey};
use p2panda_store::logs::LogStore;
use p2panda_sync::protocols::LogSyncMessage;
use p2panda_sync::protocols::Logs;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use tokio_util::{
    bytes::BytesMut,
    codec::{Decoder, Encoder},
};
#[derive(Debug, thiserror::Error)]
#[error("replica snapshot rejected")]
pub(crate) struct SnapshotError;
type ManagementOperation = Operation<ManagementExtension>;
type Entry = (ManagementOperation, Vec<u8>);
type Entries = BTreeMap<(VerifyingKey, [u8; 16]), Vec<Entry>>;
pub(crate) const MAX_RECORDS: usize = MAX_PAGE_RECORDS;
pub(crate) const MAX_BYTES: usize = MAX_PAGE_BYTES;
#[derive(Serialize, Deserialize)]
pub(crate) struct Manifest {
    pub(crate) anchor: NetworkAnchor,
    pub(crate) logs: Logs<[u8; 16]>,
    pub(crate) ids: Vec<Hash>,
    /// Absent for the original one-page wire. A continuation is another
    /// manifest and LogSync, never a successful final receipt acknowledgement.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) more: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Receipt {
    pub(crate) records_saved: bool,
    pub(crate) view_applied: bool,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReplicaProgress {
    pub(crate) completed_pages: usize,
    pub(crate) received: usize,
    pub(crate) reused: usize,
}

/// Receiving a complete transfer is distinct from saving or applying every record.
#[derive(Default)]
pub(crate) struct ReceivedHistory {
    pub(crate) scope: Vec<Hash>,
    pub(crate) saved: Vec<Hash>,
    seen: BTreeSet<Hash>,
}

impl ReceivedHistory {
    pub(crate) fn append(&mut self, page: ReceivedHistory) {
        let saved: BTreeSet<_> = page.saved.into_iter().collect();
        for id in page.scope {
            if self.seen.insert(id) {
                self.scope.push(id);
                if saved.contains(&id) {
                    self.saved.push(id);
                }
            }
        }
    }

    pub(crate) fn receipt(&self, applied: &[Hash]) -> Receipt {
        Receipt {
            records_saved: self.saved == self.scope,
            view_applied: applied == self.scope,
        }
    }
}

#[derive(Debug)]
pub(crate) enum ExchangeError {
    Rejected,
    Transport,
}

pub(crate) struct IncomingRecords<F> {
    protocol: Option<std::pin::Pin<Box<F>>>,
    events: tokio::sync::broadcast::Receiver<
        p2panda_sync::protocols::LogSyncEvent<ManagementExtension>,
    >,
    expected: BTreeSet<Hash>,
    bytes: usize,
}

impl Manifest {
    pub(crate) fn receive<'a, S, R, SE, RE>(
        &mut self,
        anchor: NetworkAnchor,
        known: Snapshot,
        sink: &'a mut S,
        stream: &'a mut R,
    ) -> Result<IncomingRecords<impl Future<Output = Result<(), ExchangeError>> + 'a>, ExchangeError>
    where
        S: Sink<LogSyncMessage<[u8; 16]>, Error = SE> + Unpin,
        R: Stream<Item = Result<LogSyncMessage<[u8; 16]>, RE>> + Unpin,
        SE: std::fmt::Debug + 'a,
        RE: std::fmt::Debug + 'a,
    {
        use p2panda_sync::{protocols::LogSync, traits::Protocol};
        let expected: BTreeSet<_> = self.ids.iter().copied().collect();
        if self.anchor != anchor
            || self.ids.is_empty()
            || self.ids.len() > MAX_RECORDS
            || expected.len() != self.ids.len()
            || self.logs.values().map(Vec::len).sum::<usize>() > MAX_RECORDS
        {
            return Err(ExchangeError::Rejected);
        }
        let (sender, events) = tokio::sync::broadcast::channel(MAX_RECORDS + 1);
        let sync = LogSync::new(known, std::mem::take(&mut self.logs), sender);
        let protocol = async move {
            sync.run(sink, stream)
                .await
                .map(|_| ())
                .map_err(|_| ExchangeError::Transport)
        };
        Ok(IncomingRecords {
            protocol: Some(Box::pin(protocol)),
            events,
            expected,
            bytes: 0,
        })
    }

    pub(crate) fn received(self, saved: BTreeSet<Hash>) -> ReceivedHistory {
        let saved = self
            .ids
            .iter()
            .filter(|id| saved.contains(id))
            .copied()
            .collect();
        ReceivedHistory {
            scope: self.ids,
            saved,
            seen: BTreeSet::new(),
        }
    }
}

impl<F: Future<Output = Result<(), ExchangeError>>> IncomingRecords<F> {
    pub(crate) async fn next(&mut self) -> Result<Option<ManagementRecord>, ExchangeError> {
        use p2panda_sync::protocols::LogSyncEvent;
        use tokio::sync::broadcast::error::RecvError;
        loop {
            let event = if let Some(protocol) = &mut self.protocol {
                tokio::select! {
                    biased;
                    event = self.events.recv() => event,
                    result = protocol => {
                        result?;
                        // Dropping the protocol closes its only sender. Drain
                        // events queued by its final poll before completing.
                        self.protocol = None;
                        continue;
                    }
                }
            } else {
                self.events.recv().await
            };
            match event {
                Ok(LogSyncEvent::OperationReceived { operation, .. }) => {
                    if !self.expected.contains(&operation.hash) {
                        return Err(ExchangeError::Rejected);
                    }
                    let record = ManagementRecord::from_verified_operation(
                        operation.header,
                        operation.body.map(|body| body.to_bytes()),
                    );
                    self.bytes += record.header().len() + record.body().map_or(0, <[u8]>::len);
                    if self.bytes > MAX_BYTES {
                        return Err(ExchangeError::Rejected);
                    }
                    return Ok(Some(record));
                }
                Ok(_) => {}
                Err(RecvError::Lagged(_)) => return Err(ExchangeError::Transport),
                Err(RecvError::Closed) => {
                    if let Some(protocol) = self.protocol.take() {
                        protocol.await?;
                    }
                    return Ok(None);
                }
            }
        }
    }
}

/// Read-only LogStore projection of one durable, authorized snapshot. Pending
/// records never advertise heights; pruning has no implementation here.
#[derive(Clone, Default)]
pub(crate) struct Snapshot(Arc<Entries>);

pub(crate) struct SnapshotPages<'a> {
    anchor: NetworkAnchor,
    remaining: &'a [ManagementRecord],
}

impl Iterator for SnapshotPages<'_> {
    type Item = Result<(Snapshot, Manifest), SnapshotError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining.is_empty() {
            return None;
        }
        const TARGET_RECORDS: usize = 128;
        const TARGET_BYTES: usize = 4 * 1024 * 1024;
        let candidates = &self.remaining[..self.remaining.len().min(TARGET_RECORDS)];
        let page = match history_page(candidates) {
            Ok(page) => page,
            Err(_) => {
                self.remaining = &[];
                return Some(Err(SnapshotError));
            }
        };
        // An individual signed record can exceed the target up to the hard
        // wire ceiling. It must travel whole.
        let mut bytes = 0;
        let mut count = 0;
        for record in page {
            let size = record.header().len() + record.body().map_or(0, <[u8]>::len);
            if count != 0 && bytes + size > TARGET_BYTES {
                break;
            }
            bytes += size;
            count += 1;
        }
        let page = &page[..count];
        let result = Snapshot::from_records(self.anchor, page).map(|(snapshot, mut manifest)| {
            manifest.more = page.len() < self.remaining.len();
            (snapshot, manifest)
        });
        self.remaining = &self.remaining[page.len()..];
        Some(result)
    }
}

/// Serve a frozen, applied snapshot. The host owns the operation deadline
/// and re-reads live admission wherever this exchange releases data.
pub(crate) async fn send_snapshot(
    exchange: &ControlExchange,
    anchor: NetworkAnchor,
    records: &[ManagementRecord],
    allowed: impl Fn() -> bool,
) -> Result<(), ExchangeError> {
    use futures_util::{SinkExt, StreamExt};
    use p2panda_sync::{protocols::{LogSync, LogSyncEvent}, traits::Protocol};
    use tokio::sync::broadcast;

    let request = exchange
        .recv_message(ControlMessageFormat::Fixed { length: 4 })
        .await
        .map_err(|_| ExchangeError::Transport)?;
    if request != b"REPL" {
        return Err(ExchangeError::Rejected);
    }
    for page in Snapshot::pages(anchor, records) {
        if !allowed() {
            return Err(ExchangeError::Rejected);
        }
        let (snapshot, manifest) = page.map_err(|_| ExchangeError::Rejected)?;
        let more = manifest.more;
        let bytes = p2panda_core::cbor::encode_cbor(&manifest)
            .map_err(|_| ExchangeError::Rejected)?;
        if bytes.len() > MAX_BYTES {
            return Err(ExchangeError::Rejected);
        }
        exchange
            .send_message(
                ControlMessageFormat::LengthPrefixed32 { maximum: MAX_BYTES },
                &bytes,
            )
            .await
            .map_err(|_| ExchangeError::Transport)?;
        let (events, mut received) =
            broadcast::channel::<LogSyncEvent<ManagementExtension>>(MAX_RECORDS + 1);
        let sync = LogSync::new(snapshot, manifest.logs, events);
        let allowed = &allowed;
        let mut sink = Box::pin(
            codec_sink::<LogSyncMessage<[u8; 16]>>(exchange, MAX_BYTES).with(
                |message| async move {
                    if !allowed() {
                        return Err(CodecError::Io(
                            std::io::ErrorKind::PermissionDenied.into(),
                        ));
                    }
                    Ok(message)
                },
            ),
        );
        let mut stream = codec_stream::<LogSyncMessage<[u8; 16]>>(exchange, MAX_BYTES);
        sync.run(&mut sink, &mut stream)
            .await
            .map_err(|_| ExchangeError::Transport)?;
        // This is a one-way replica fetch, not an unsolicited publish route.
        while let Ok(event) = received.try_recv() {
            if matches!(event, LogSyncEvent::OperationReceived { .. }) {
                return Err(ExchangeError::Rejected);
            }
        }
        drop(sink);
        drop(stream);
        let receipt = codec_stream::<Receipt>(exchange, 64)
            .next()
            .await
            .ok_or(ExchangeError::Transport)?
            .map_err(|_| ExchangeError::Transport)?;
        if !receipt.records_saved || !allowed() {
            return Err(ExchangeError::Rejected);
        }
        // Withhold the old completion ACK while another page remains.
        // An old one-page reader cannot accept the next CBOR manifest
        // as a postcard Receipt, and therefore cannot report completion.
        if !more {
            codec_sink::<Receipt>(exchange, 64)
                .send(Receipt {
                    records_saved: true,
                    view_applied: receipt.view_applied,
                })
                .await
                .map_err(|_| ExchangeError::Transport)?;
        }
    }
    exchange
        .finish_send_confirmed()
        .await
        .map_err(|_| ExchangeError::Transport)?;
    Ok(())
}

impl Snapshot {
    pub(crate) fn pages(anchor: NetworkAnchor, records: &[ManagementRecord]) -> SnapshotPages<'_> {
        SnapshotPages {
            anchor,
            remaining: records,
        }
    }

    pub(crate) fn from_records(
        anchor: NetworkAnchor,
        records: &[ManagementRecord],
    ) -> Result<(Self, Manifest), SnapshotError> {
        let mut entries = Entries::new();
        let mut ids = Vec::new();
        let mut logs = Logs::new();
        let mut bytes = 0;
        for record in records {
            bytes += record.header().len() + record.body().map_or(0, |body| body.len());
            if ids.len() >= MAX_RECORDS || bytes > MAX_BYTES {
                return Err(SnapshotError);
            }
            let header = match record.verified_header() {
                Some(header) => header.as_ref().clone(),
                None => Header::<ManagementExtension>::decode(record.header())
                    .map_err(|_| SnapshotError)?,
            };
            let author = header.verifying_key;
            let log = header.extensions.log_id;
            let author_logs = logs.entry(author).or_default();
            if !author_logs.contains(&log) {
                author_logs.push(log);
            }
            if !ids.contains(&header.hash()) {
                ids.push(header.hash());
            }
            entries.entry((author, log)).or_default().push((
                Operation {
                    hash: header.hash(),
                    header,
                    body: record.body().map(Body::from_bytes),
                },
                record.header().to_vec(),
            ));
        }
        for log in entries.values_mut() {
            log.sort_by_key(|(op, _)| op.header.seq_num);
        }
        Ok((
            Self(Arc::new(entries)),
            Manifest {
                anchor,
                logs,
                ids,
                more: false,
            },
        ))
    }

    fn range(
        &self,
        author: &VerifyingKey,
        log: &[u8; 16],
        after: Option<SeqNum>,
        until: Option<SeqNum>,
    ) -> Option<Vec<Entry>> {
        self.0.get(&(*author, *log)).map(|entries| {
            entries
                .iter()
                .filter(|(op, _)| {
                    after.is_none_or(|seq| op.header.seq_num > seq)
                        && until.is_none_or(|seq| op.header.seq_num <= seq)
                })
                .cloned()
                .collect()
        })
    }
}

impl LogStore<ManagementOperation, VerifyingKey, [u8; 16], SeqNum, Hash> for Snapshot {
    type Error = SnapshotError;
    async fn get_latest_entry(
        &self,
        author: &VerifyingKey,
        log: &[u8; 16],
    ) -> Result<Option<ManagementOperation>, Self::Error> {
        Ok(self
            .0
            .get(&(*author, *log))
            .and_then(|entries| entries.last())
            .map(|(op, _)| op.clone()))
    }
    async fn get_latest_entry_tx(
        &self,
        author: &VerifyingKey,
        log: &[u8; 16],
    ) -> Result<Option<ManagementOperation>, Self::Error> {
        self.get_latest_entry(author, log).await
    }
    async fn get_log_heights(
        &self,
        author: &VerifyingKey,
        logs: &[[u8; 16]],
    ) -> Result<Option<BTreeMap<[u8; 16], SeqNum>>, Self::Error> {
        let mut heights = BTreeMap::new();
        for log in logs {
            if let Some(op) = self.get_latest_entry(author, log).await? {
                heights.insert(*log, op.header.seq_num);
            }
        }
        Ok((!heights.is_empty()).then_some(heights))
    }
    async fn get_log_size(
        &self,
        author: &VerifyingKey,
        log: &[u8; 16],
        after: Option<SeqNum>,
        until: Option<SeqNum>,
    ) -> Result<Option<(u32, u32)>, Self::Error> {
        Ok(self.range(author, log, after, until).map(|entries| {
            (
                entries.len() as u32,
                entries
                    .iter()
                    .map(|(op, header)| {
                        header.len() + op.body.as_ref().map_or(0, |body| body.to_bytes().len())
                    })
                    .sum::<usize>() as u32,
            )
        }))
    }
    async fn get_log_entries(
        &self,
        author: &VerifyingKey,
        log: &[u8; 16],
        after: Option<SeqNum>,
        until: Option<SeqNum>,
    ) -> Result<Option<Vec<Entry>>, Self::Error> {
        Ok(self.range(author, log, after, until))
    }
    async fn prune_entries(
        &self,
        _: &VerifyingKey,
        _: &[u8; 16],
        _: &SeqNum,
    ) -> Result<u64, Self::Error> {
        Err(SnapshotError)
    }
}

// Reuse the original p2panda postcard codec and LogSync algorithm. Complete
// messages replace raw streams; no read-ahead crosses the LogSync/receipt boundary.
pub(crate) fn codec_sink<M: serde::Serialize + Send>(
    exchange: &ControlExchange,
    maximum: usize,
) -> impl futures_util::Sink<M, Error = CodecError> + Unpin + Send {
    Box::pin(futures_util::sink::unfold(
        (),
        move |(), message| async move {
            let mut encoded = BytesMut::new();
            Codec::<M>::new()
                .max_frame_len(maximum)
                .encode(message, &mut encoded)?;
            exchange
                .send_message(
                    ControlMessageFormat::LengthPrefixed32 { maximum },
                    &encoded[4..],
                )
                .await?;
            Ok(())
        },
    ))
}
pub(crate) fn codec_stream<M: serde::de::DeserializeOwned + Send>(
    exchange: &ControlExchange,
    maximum: usize,
) -> impl futures_util::Stream<Item = Result<M, CodecError>> + Unpin + Send {
    Box::pin(futures_util::stream::unfold(
        false,
        move |ended| async move {
            if ended {
                return None;
            }
            let result = async {
                let body = exchange
                    .recv_message(ControlMessageFormat::LengthPrefixed32 { maximum })
                    .await?;
                let mut frame = BytesMut::with_capacity(4 + body.len());
                frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
                frame.extend_from_slice(&body);
                Codec::<M>::new()
                    .max_frame_len(maximum)
                    .decode(&mut frame)?
                    .ok_or_else(|| {
                        CodecError::Io(std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            "incomplete control frame",
                        ))
                    })
            }
            .await;
            let ended = result.is_err();
            Some((result, ended))
        },
    ))
}
