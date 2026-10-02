//! A bounded, source-scoped management-history exchange over an admitted data
//! connection. The immutable serving snapshot contains only applied records;
//! all received LogSync events go through the durable management consumer.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use iroh::endpoint::Connection;
use iroh_base::EndpointId;
use p2panda_core::{Body, Hash, Header, Operation, SeqNum, VerifyingKey};
use p2panda_net::codec::Codec;
use p2panda_store::logs::LogStore;
use p2panda_sync::protocols::{LogSync, LogSyncEvent, LogSyncMessage, Logs};
use p2panda_sync::traits::Protocol;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::broadcast;
use tokio_util::codec::{FramedRead, FramedWrite};

use crate::admission_gate::ProtocolScope;
use crate::management_auth::{ManagementExtension, NetworkAnchor};
use crate::management_store::{ManagementRecord, StoreError};
use crate::native_network::NativeNetwork;

pub const REPLICA_ALPN: &[u8] = b"miopunch/control-replica/1";
const MAX_RECORDS: usize = 4096;
const MAX_BYTES: usize = 16 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(20);
type ManagementOperation = Operation<ManagementExtension>;
type Entry = (ManagementOperation, Vec<u8>);
type Entries = BTreeMap<(VerifyingKey, [u8; 16]), Vec<Entry>>;

#[derive(Debug, Error)]
pub enum ReplicaError {
    #[error("replica access or scope rejected")]
    Rejected,
    #[error("replica transport failed or timed out")]
    Transport,
    #[error("replica storage failed: {0}")]
    Store(#[from] StoreError),
}

/// Transport Done is separate from complete durable records and an applied
/// authorization view. Every ID belongs to this source's frozen exchange,
/// without claiming knowledge of other sources or globally latest state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplicaOutcome {
    pub source: EndpointId,
    pub scope: Vec<Hash>,
    pub transport_done: bool,
    pub saved: Vec<Hash>,
    pub applied: Vec<Hash>,
}

impl ReplicaOutcome {
    pub fn complete(&self) -> bool {
        self.transport_done && self.scope == self.saved && self.scope == self.applied
    }
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    anchor: NetworkAnchor,
    logs: Logs<[u8; 16]>,
    ids: Vec<Hash>,
}

#[derive(Serialize, Deserialize)]
struct Receipt {
    records_saved: bool,
    view_applied: bool,
}

/// Read-only LogStore projection of one durable, authorized snapshot. Pending
/// records never advertise heights; pruning has no implementation here.
#[derive(Clone, Default)]
struct Snapshot(Arc<Entries>);

impl Snapshot {
    fn from_records(
        anchor: NetworkAnchor,
        records: &[ManagementRecord],
    ) -> Result<(Self, Manifest), ReplicaError> {
        let mut entries = Entries::new();
        let mut ids = Vec::new();
        let mut logs = Logs::new();
        let mut bytes = 0;
        for record in records {
            bytes += record.header().len() + record.body().map_or(0, |body| body.len());
            if ids.len() >= MAX_RECORDS || bytes > MAX_BYTES {
                return Err(ReplicaError::Rejected);
            }
            let header = Header::<ManagementExtension>::decode(record.header())
                .map_err(|_| ReplicaError::Rejected)?;
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
    type Error = ReplicaError;
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
        Err(ReplicaError::Rejected)
    }
}

impl NativeNetwork {
    pub async fn management_history(&self) -> Result<Vec<ManagementRecord>, StoreError> {
        self.store.applied_records().await
    }
    pub async fn management_record(
        &self,
        id: Hash,
    ) -> Result<Option<ManagementRecord>, StoreError> {
        self.store.get_record(id).await
    }

    #[cfg(feature = "replica-test-hooks")]
    pub fn set_replica_checkpoint_observer(
        &mut self,
        observer: fn(crate::management_store::RecordCheckpoint),
    ) {
        self.store.set_checkpoint_observer(observer);
    }

    async fn consume_replica_event(
        &mut self,
        connection: &Connection,
        event: LogSyncEvent<ManagementExtension>,
        expected: &BTreeSet<Hash>,
        bytes: &mut usize,
    ) -> Result<Option<Hash>, ReplicaError> {
        let LogSyncEvent::OperationReceived { operation, .. } = event else {
            return Ok(None);
        };
        if !expected.contains(&operation.hash) || !self.replica_allowed(connection) {
            return Err(ReplicaError::Rejected);
        }
        let header = operation.header.encode();
        let body = operation.body.map(|body| body.to_bytes());
        *bytes += header.len() + body.as_ref().map_or(0, Vec::len);
        if *bytes > MAX_BYTES {
            return Err(ReplicaError::Rejected);
        }
        let id = self
            .store
            .ingest(&ManagementRecord::new(header, body.as_deref()))
            .await?;
        self.store.apply_ready().await?;
        self.refresh_authorization();
        Ok(Some(id))
    }

    fn replica_allowed(&self, connection: &Connection) -> bool {
        (connection.alpn() == REPLICA_ALPN && self.gate.allows(connection, ProtocolScope::Member))
            || (connection.alpn() == crate::network_recovery::RECOVERY_REPLICA_ALPN
                && self.gate.allows(connection, ProtocolScope::Bootstrap)
                && !connection.paths().is_empty()
                && connection
                    .paths()
                    .iter()
                    .all(|path| matches!(path.remote_addr(), iroh::TransportAddr::Ip(_))))
    }

    /// Serve the complete locally applied control history, including original
    /// author bodies and dependencies. Admission and data-path gates apply
    /// before emitting even the manifest; no private delivery tables are read.
    pub async fn serve_replica(&self, connection: &Connection) -> Result<(), ReplicaError> {
        let records = self.management_history().await?;
        self.serve_replica_records(connection, &records).await
    }

    /// Explicit fault-fixture build only: deliver incomplete, missing-dependency
    /// or unauthorized signed input through the same admitted wire protocol.
    #[cfg(feature = "replica-test-hooks")]
    pub async fn serve_replica_fixture(
        &self,
        connection: &Connection,
        records: &[ManagementRecord],
    ) -> Result<(), ReplicaError> {
        self.serve_replica_records(connection, records).await
    }

    pub(crate) async fn serve_replica_records(
        &self,
        connection: &Connection,
        records: &[ManagementRecord],
    ) -> Result<(), ReplicaError> {
        let result = tokio::time::timeout(TIMEOUT, async {
            if !self.replica_allowed(connection)
                || (connection.alpn() == crate::network_recovery::RECOVERY_REPLICA_ALPN
                    && !self.gate.allows_member_path_upgrade(connection))
            {
                return Err(ReplicaError::Rejected);
            }
            let (snapshot, manifest) = Snapshot::from_records(self.anchor, records)?;
            let (mut send, mut recv) = connection
                .accept_bi()
                .await
                .map_err(|_| ReplicaError::Transport)?;
            let mut request = [0; 4];
            recv.read_exact(&mut request)
                .await
                .map_err(|_| ReplicaError::Transport)?;
            if &request != b"REPL" {
                return Err(ReplicaError::Rejected);
            }
            if !self.replica_allowed(connection)
                || (connection.alpn() == crate::network_recovery::RECOVERY_REPLICA_ALPN
                    && !self.gate.allows_member_path_upgrade(connection))
            {
                return Err(ReplicaError::Rejected);
            }
            let bytes =
                p2panda_core::cbor::encode_cbor(&manifest).map_err(|_| ReplicaError::Rejected)?;
            if bytes.len() > MAX_BYTES {
                return Err(ReplicaError::Rejected);
            }
            send.write_all(&(bytes.len() as u32).to_be_bytes())
                .await
                .map_err(|_| ReplicaError::Transport)?;
            send.write_all(&bytes)
                .await
                .map_err(|_| ReplicaError::Transport)?;
            let (events, mut received) =
                broadcast::channel::<LogSyncEvent<ManagementExtension>>(MAX_RECORDS + 1);
            let sync = LogSync::new(snapshot, manifest.logs, events);
            let mut sink = FramedWrite::new(
                send,
                Codec::<LogSyncMessage<[u8; 16]>>::new().max_frame_len(MAX_BYTES),
            );
            let mut stream = FramedRead::new(
                recv,
                Codec::<LogSyncMessage<[u8; 16]>>::new().max_frame_len(MAX_BYTES),
            );
            sync.run(&mut sink, &mut stream)
                .await
                .map_err(|_| ReplicaError::Transport)?;
            // This is a one-way replica fetch, not an unsolicited publish route.
            while let Ok(event) = received.try_recv() {
                if matches!(event, LogSyncEvent::OperationReceived { .. }) {
                    return Err(ReplicaError::Rejected);
                }
            }
            // Wire Done only describes LogSync. Keep this actual Connection
            // alive until the receiver has drained its durable consumer.
            let buffered = stream.read_buffer().clone();
            let mut receipt_stream = FramedRead::new(
                stream.into_inner(),
                Codec::<Receipt>::new().max_frame_len(64),
            );
            receipt_stream
                .read_buffer_mut()
                .extend_from_slice(&buffered);
            let receipt = receipt_stream
                .next()
                .await
                .ok_or(ReplicaError::Transport)?
                .map_err(|_| ReplicaError::Transport)?;
            if !receipt.records_saved {
                return Err(ReplicaError::Rejected);
            }
            if connection.alpn() == crate::network_recovery::RECOVERY_REPLICA_ALPN {
                let mut acknowledgement =
                    FramedWrite::new(sink.into_inner(), Codec::<Receipt>::new().max_frame_len(64));
                acknowledgement
                    .send(Receipt {
                        records_saved: true,
                        view_applied: receipt.view_applied,
                    })
                    .await
                    .map_err(|_| ReplicaError::Transport)?;
            }
            Ok(())
        })
        .await
        .unwrap_or(Err(ReplicaError::Transport));
        if result.is_err() {
            connection.close(0u32.into(), b"replica rejected");
        }
        result
    }

    /// Consume real LogSync OperationReceived events through complete-record
    /// verification, durable ingest, causal ordering and product authorization.
    /// An incomplete body aborts this exchange without durable hash dedup;
    /// retransmission in a new exchange can complete it. No-body signed actions
    /// use this same consumer and are never silently acknowledged or skipped.
    pub async fn sync_replica(
        &mut self,
        connection: &Connection,
    ) -> Result<ReplicaOutcome, ReplicaError> {
        let result = tokio::time::timeout(TIMEOUT, async {
            if !self.replica_allowed(connection) { return Err(ReplicaError::Rejected); }
            let (send, mut recv) = connection.open_bi().await.map_err(|_| ReplicaError::Transport)?;
            // QUIC opens the stream lazily; the request makes it visible before
            // the provider sends its manifest and starts the real LogSync.
            let mut send = send;
            send.write_all(b"REPL").await.map_err(|_| ReplicaError::Transport)?;
            let mut size = [0;4]; recv.read_exact(&mut size).await.map_err(|_| ReplicaError::Transport)?;
            let size = u32::from_be_bytes(size) as usize;
            if size == 0 || size > MAX_BYTES { return Err(ReplicaError::Rejected); }
            let mut bytes = vec![0; size]; recv.read_exact(&mut bytes).await.map_err(|_| ReplicaError::Transport)?;
            let manifest: Manifest = p2panda_core::cbor::decode_cbor(bytes.as_slice()).map_err(|_| ReplicaError::Rejected)?;
            let expected: BTreeSet<_> = manifest.ids.iter().copied().collect();
            if manifest.anchor != self.anchor || manifest.ids.is_empty() || manifest.ids.len() > MAX_RECORDS || expected.len() != manifest.ids.len() || manifest.logs.values().map(Vec::len).sum::<usize>() > MAX_RECORDS {
                return Err(ReplicaError::Rejected);
            }
            // Start empty so pending/incomplete records and repeated exchanges
            // cannot suppress the source's full verified completion route.
            let (events, mut received) = broadcast::channel::<LogSyncEvent<ManagementExtension>>(MAX_RECORDS + 1);
            let sync = LogSync::new(Snapshot::default(), manifest.logs, events);
            let mut sink = FramedWrite::new(send, Codec::<LogSyncMessage<[u8; 16]>>::new().max_frame_len(MAX_BYTES));
            let mut stream = FramedRead::new(recv, Codec::<LogSyncMessage<[u8; 16]>>::new().max_frame_len(MAX_BYTES));
            let mut saved_ids = BTreeSet::new();
            let mut bytes = 0;
            let mut run = Box::pin(sync.run(&mut sink, &mut stream));
            // Persist as events arrive, including exchanges later interrupted
            // on the wire. A broadcast lag or storage error cannot mean Done.
            loop {
                tokio::select! {
                    biased;
                    event = received.recv() => {
                        match event {
                            Ok(event) => {
                                if let Some(id) = self.consume_replica_event(connection, event, &expected, &mut bytes).await? { saved_ids.insert(id); }
                            }
                            Err(broadcast::error::RecvError::Closed) => {
                                (&mut run).await.map_err(|_| ReplicaError::Transport)?;
                                break;
                            }
                            Err(_) => return Err(ReplicaError::Transport),
                        }
                    }
                    result = &mut run => { result.map_err(|_| ReplicaError::Transport)?; break; }
                }
            }
            drop(run);
            // Events enqueued during the protocol's final poll must also be
            // consumed, rather than being auto-acked by a body-only stream.
            while let Ok(event) = received.try_recv() {
                if let Some(id) = self.consume_replica_event(connection, event, &expected, &mut bytes).await? { saved_ids.insert(id); }
            }
            self.store.apply_ready().await?;
            self.refresh_authorization();
            let mut applied = Vec::new();
            for id in &manifest.ids {
                if self.store.get_record(*id).await?.is_some() { applied.push(*id); }
            }
            let outcome = ReplicaOutcome { source: connection.remote_id(), saved: manifest.ids.iter().filter(|id| saved_ids.contains(id)).copied().collect(), scope: manifest.ids, transport_done: true, applied };
            let mut receipt_sink = FramedWrite::new(sink.into_inner(), Codec::<Receipt>::new().max_frame_len(64));
            receipt_sink.send(Receipt { records_saved: outcome.saved == outcome.scope, view_applied: outcome.applied == outcome.scope }).await.map_err(|_| ReplicaError::Transport)?;
            if connection.alpn() == crate::network_recovery::RECOVERY_REPLICA_ALPN {
                let buffered = stream.read_buffer().clone();
                let mut acknowledgement = FramedRead::new(stream.into_inner(), Codec::<Receipt>::new().max_frame_len(64));
                acknowledgement.read_buffer_mut().extend_from_slice(&buffered);
                let ack = acknowledgement.next().await.ok_or(ReplicaError::Transport)?.map_err(|_| ReplicaError::Transport)?;
                if !ack.records_saved { return Err(ReplicaError::Rejected); }
            }
            Ok(outcome)
        }).await.unwrap_or(Err(ReplicaError::Transport));
        if result.is_err() {
            connection.close(0u32.into(), b"replica rejected");
        }
        result
    }
}
