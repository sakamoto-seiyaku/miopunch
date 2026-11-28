//! Shared bounded LogSync snapshot and manifest used on native and browser.
use crate::management_auth::{ManagementExtension, NetworkAnchor};
use crate::management_record::ManagementRecord;
use futures_util::{Sink, Stream};
use p2panda_core::{Body, Hash, Header, Operation, SeqNum, VerifyingKey};
use p2panda_store::logs::LogStore;
use p2panda_sync::protocols::LogSyncMessage;
use p2panda_sync::protocols::Logs;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
#[derive(Debug, thiserror::Error)]
#[error("replica snapshot rejected")]
pub(crate) struct SnapshotError;
type ManagementOperation = Operation<ManagementExtension>;
type Entry = (ManagementOperation, Vec<u8>);
type Entries = BTreeMap<(VerifyingKey, [u8; 16]), Vec<Entry>>;
pub(crate) const MAX_RECORDS: usize = 4096;
pub(crate) const MAX_BYTES: usize = 16 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
pub(crate) struct Manifest {
    pub(crate) anchor: NetworkAnchor,
    pub(crate) logs: Logs<[u8; 16]>,
    pub(crate) ids: Vec<Hash>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Receipt {
    pub(crate) records_saved: bool,
    pub(crate) view_applied: bool,
}

/// Receiving a complete transfer is distinct from saving or applying every record.
pub(crate) struct ReceivedHistory {
    pub(crate) scope: Vec<Hash>,
    pub(crate) saved: Vec<Hash>,
}

impl ReceivedHistory {
    pub(crate) fn receipt(&self, applied: &[Hash]) -> Receipt {
        Receipt {
            records_saved: self.saved == self.scope,
            view_applied: applied == self.scope,
        }
    }
}

#[derive(Debug)]
pub(crate) enum ReceiveError {
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
        sink: &'a mut S,
        stream: &'a mut R,
    ) -> Result<IncomingRecords<impl Future<Output = Result<(), ReceiveError>> + 'a>, ReceiveError>
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
            return Err(ReceiveError::Rejected);
        }
        let (sender, events) = tokio::sync::broadcast::channel(MAX_RECORDS + 1);
        let sync = LogSync::new(Snapshot::default(), std::mem::take(&mut self.logs), sender);
        let protocol = async move {
            sync.run(sink, stream)
                .await
                .map(|_| ())
                .map_err(|_| ReceiveError::Transport)
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
        }
    }
}

impl<F: Future<Output = Result<(), ReceiveError>>> IncomingRecords<F> {
    pub(crate) async fn next(
        &mut self,
    ) -> Result<Option<(Vec<u8>, Option<Vec<u8>>)>, ReceiveError> {
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
                        return Err(ReceiveError::Rejected);
                    }
                    let header = operation.header.encode();
                    let body = operation.body.map(|body| body.to_bytes());
                    self.bytes += header.len() + body.as_ref().map_or(0, Vec::len);
                    if self.bytes > MAX_BYTES {
                        return Err(ReceiveError::Rejected);
                    }
                    return Ok(Some((header, body)));
                }
                Ok(_) => {}
                Err(RecvError::Lagged(_)) => return Err(ReceiveError::Transport),
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

impl Snapshot {
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
            let header = Header::<ManagementExtension>::decode(record.header())
                .map_err(|_| SnapshotError)?;
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
        Ok((Self(Arc::new(entries)), Manifest { anchor, logs, ids }))
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
