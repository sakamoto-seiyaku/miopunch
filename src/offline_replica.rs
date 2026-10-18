//! A bounded, source-scoped management-history exchange over an admitted data
//! connection. The immutable serving snapshot contains only applied records;
//! all received LogSync events go through the durable management consumer.
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use crate::control_protocol::AuthenticatedControl;
use futures_util::{SinkExt, StreamExt};
use iroh::endpoint::{Connection, ControlExchange, ControlMessageFormat};
use iroh_base::EndpointId;
use p2panda_core::Hash;
use p2panda_net::codec::{Codec, CodecError};
use p2panda_sync::protocols::{LogSync, LogSyncEvent, LogSyncMessage};
use p2panda_sync::traits::Protocol;
use thiserror::Error;
use tokio::sync::broadcast;
use tokio_util::{
    bytes::BytesMut,
    codec::{Decoder, Encoder},
};

use crate::admission_gate::ProtocolScope;
use crate::management_auth::{ManagementExtension, NetworkAnchor};
use crate::management_store::{ManagementRecord, StoreError};
use crate::native_network::NativeNetwork;

pub const REPLICA_ALPN: &[u8] = crate::connection_coordinator::REPLICA_ALPN;
const MAX_RECORDS: usize = 4096;
const MAX_BYTES: usize = 16 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(20);
use crate::replica_snapshot::{Manifest, Receipt, Snapshot};

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
        connection: &(impl AuthenticatedControl + ?Sized),
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

    fn replica_allowed(&self, connection: &(impl AuthenticatedControl + ?Sized)) -> bool {
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
    pub async fn serve_replica(
        &self,
        connection: &(impl AuthenticatedControl + ?Sized),
    ) -> Result<(), ReplicaError> {
        let records = self.management_history().await?;
        self.serve_replica_records(connection, &records).await
    }

    /// Serve a newly connected replica within its caller's already running
    /// operation clock. The usual provider ceiling can only shorten it.
    pub async fn serve_replica_with_deadline(
        &self,
        connection: &(impl AuthenticatedControl + ?Sized),
        deadline: n0_future::time::Instant,
    ) -> Result<(), ReplicaError> {
        let records = self.management_history().await?;
        ReplicaProvider::new(self, records)
            .serve_until(connection, deadline)
            .await
    }

    /// Explicit fault-fixture build only: deliver incomplete, missing-dependency
    /// or unauthorized signed input through the same admitted wire protocol.
    #[cfg(feature = "replica-test-hooks")]
    pub async fn serve_replica_fixture(
        &self,
        connection: &(impl AuthenticatedControl + ?Sized),
        records: &[ManagementRecord],
    ) -> Result<(), ReplicaError> {
        self.serve_replica_records(connection, records).await
    }

    pub(crate) async fn serve_replica_records(
        &self,
        connection: &(impl AuthenticatedControl + ?Sized),
        records: &[ManagementRecord],
    ) -> Result<(), ReplicaError> {
        ReplicaProvider::new(self, records.to_vec())
            .serve(connection)
            .await
    }

    /// Consume real LogSync OperationReceived events through complete-record
    /// verification, durable ingest, causal ordering and product authorization.
    /// An incomplete body aborts this exchange without durable hash dedup;
    /// retransmission in a new exchange can complete it. No-body signed actions
    /// use this same consumer and are never silently acknowledged or skipped.
    pub async fn sync_replica(
        &mut self,
        connection: &(impl AuthenticatedControl + ?Sized),
    ) -> Result<ReplicaOutcome, ReplicaError> {
        self.sync_replica_until(connection, None).await
    }

    /// A fresh foreground exchange keeps the caller's already running clock.
    pub async fn sync_replica_with_deadline(
        &mut self,
        connection: &(impl AuthenticatedControl + ?Sized),
        deadline: n0_future::time::Instant,
    ) -> Result<ReplicaOutcome, ReplicaError> {
        self.sync_replica_until(connection, Some(deadline)).await
    }

    async fn sync_replica_until(
        &mut self,
        connection: &(impl AuthenticatedControl + ?Sized),
        deadline: Option<n0_future::time::Instant>,
    ) -> Result<ReplicaOutcome, ReplicaError> {
        let remaining = deadline.map_or(TIMEOUT, |deadline| {
            TIMEOUT.min(deadline.saturating_duration_since(n0_future::time::Instant::now()))
        });
        let result = tokio::time::timeout(remaining, async {
            if !self.replica_allowed(connection) { return Err(ReplicaError::Rejected); }
            let exchange = connection.open_control_exchange().await.map_err(|_|ReplicaError::Transport)?;
            // Preserve the existing lazy-open request and bounded manifest wire.
            exchange.send_message(ControlMessageFormat::Fixed {length:4},b"REPL").await.map_err(|_|ReplicaError::Transport)?;
            let bytes=exchange.recv_message(ControlMessageFormat::LengthPrefixed32 {maximum:MAX_BYTES}).await.map_err(message_error)?;
            if bytes.is_empty() {return Err(ReplicaError::Rejected);}
            let manifest: Manifest = p2panda_core::cbor::decode_cbor(bytes.as_slice()).map_err(|_| ReplicaError::Rejected)?;
            let expected: BTreeSet<_> = manifest.ids.iter().copied().collect();
            if manifest.anchor != self.anchor || manifest.ids.is_empty() || manifest.ids.len() > MAX_RECORDS || expected.len() != manifest.ids.len() || manifest.logs.values().map(Vec::len).sum::<usize>() > MAX_RECORDS {
                return Err(ReplicaError::Rejected);
            }
            // Start empty so pending/incomplete records and repeated exchanges
            // cannot suppress the source's full verified completion route.
            let (events, mut received) = broadcast::channel::<LogSyncEvent<ManagementExtension>>(MAX_RECORDS + 1);
            let sync = LogSync::new(Snapshot::default(), manifest.logs, events);
            let mut sink = codec_sink::<LogSyncMessage<[u8;16]>>(&exchange,MAX_BYTES);
            let mut stream = codec_stream::<LogSyncMessage<[u8;16]>>(&exchange,MAX_BYTES);
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
            drop(sink);drop(stream);
            let mut receipt_sink = codec_sink::<Receipt>(&exchange,64);
            receipt_sink.send(Receipt { records_saved: outcome.saved == outcome.scope, view_applied: outcome.applied == outcome.scope }).await.map_err(|_| ReplicaError::Transport)?;
            let mut acknowledgement=codec_stream::<Receipt>(&exchange,64);
            let ack=acknowledgement.next().await.ok_or(ReplicaError::Transport)?.map_err(|_|ReplicaError::Transport)?;
            if !ack.records_saved {return Err(ReplicaError::Rejected);}
            exchange.finish_send().await.map_err(|_|ReplicaError::Transport)?;
            Ok(outcome)
        }).await.unwrap_or(Err(ReplicaError::Transport));
        if result.is_err() {
            connection.close(0u32.into(), b"replica rejected");
        }
        result
    }
}

/// Immutable applied history for concurrently serving while the node's durable
/// consumer runs. The live gate is shared and rechecked at every release.
#[derive(Clone)]
pub(crate) struct ReplicaProvider {
    anchor: NetworkAnchor,
    gate: crate::admission_gate::AdmissionGate,
    records: Arc<Vec<ManagementRecord>>,
}
impl ReplicaProvider {
    pub(crate) fn new(node: &NativeNetwork, records: Vec<ManagementRecord>) -> Self {
        Self {
            anchor: node.anchor,
            gate: node.gate.clone(),
            records: Arc::new(records),
        }
    }
    pub(crate) async fn serve(
        &self,
        connection: &(impl AuthenticatedControl + ?Sized),
    ) -> Result<(), ReplicaError> {
        self.serve_records(connection, &self.records, None, None)
            .await
    }
    async fn serve_until(
        &self,
        connection: &(impl AuthenticatedControl + ?Sized),
        deadline: n0_future::time::Instant,
    ) -> Result<(), ReplicaError> {
        self.serve_records(connection, &self.records, None, Some(deadline))
            .await
    }
    pub(crate) async fn serve_stream(
        &self,
        connection: &Connection,
        streams: (iroh::endpoint::SendStream, iroh::endpoint::RecvStream),
    ) -> Result<(), ReplicaError> {
        self.serve_records(
            connection,
            &self.records,
            Some(ControlExchange::from_streams(streams.0, streams.1)),
            None,
        )
        .await
    }
    fn replica_allowed(&self, connection: &(impl AuthenticatedControl + ?Sized)) -> bool {
        (connection.alpn() == REPLICA_ALPN && self.gate.allows(connection, ProtocolScope::Member))
            || (connection.alpn() == crate::network_recovery::RECOVERY_REPLICA_ALPN
                && self.gate.allows(connection, ProtocolScope::Bootstrap)
                && !connection.paths().is_empty()
                && connection
                    .paths()
                    .iter()
                    .all(|path| matches!(path.remote_addr(), iroh::TransportAddr::Ip(_))))
    }

    fn replica_provider_allowed(&self, connection: &(impl AuthenticatedControl + ?Sized)) -> bool {
        self.replica_allowed(connection)
            && (connection.alpn() != crate::network_recovery::RECOVERY_REPLICA_ALPN
                || self.gate.allows_member_path_upgrade(connection))
    }

    async fn serve_records(
        &self,
        connection: &(impl AuthenticatedControl + ?Sized),
        records: &[ManagementRecord],
        streams: Option<ControlExchange>,
        deadline: Option<n0_future::time::Instant>,
    ) -> Result<(), ReplicaError> {
        let remaining = deadline.map_or(TIMEOUT, |deadline| {
            TIMEOUT.min(deadline.saturating_duration_since(n0_future::time::Instant::now()))
        });
        let result = tokio::time::timeout(remaining, async {
            if !self.replica_provider_allowed(connection) {
                return Err(ReplicaError::Rejected);
            }
            let (snapshot, manifest) =
                Snapshot::from_records(self.anchor, records).map_err(|_| ReplicaError::Rejected)?;
            let exchange = match streams {
                Some(exchange) => exchange,
                None => connection
                    .accept_control_exchange()
                    .await
                    .map_err(|_| ReplicaError::Transport)?,
            };
            let request = exchange
                .recv_message(ControlMessageFormat::Fixed { length: 4 })
                .await
                .map_err(|_| ReplicaError::Transport)?;
            if request != b"REPL" {
                return Err(ReplicaError::Rejected);
            }
            if !self.replica_provider_allowed(connection) {
                return Err(ReplicaError::Rejected);
            }
            let bytes =
                p2panda_core::cbor::encode_cbor(&manifest).map_err(|_| ReplicaError::Rejected)?;
            if bytes.len() > MAX_BYTES {
                return Err(ReplicaError::Rejected);
            }
            exchange
                .send_message(
                    ControlMessageFormat::LengthPrefixed32 { maximum: MAX_BYTES },
                    &bytes,
                )
                .await
                .map_err(|_| ReplicaError::Transport)?;
            let (events, mut received) =
                broadcast::channel::<LogSyncEvent<ManagementExtension>>(MAX_RECORDS + 1);
            let sync = LogSync::new(snapshot, manifest.logs, events);
            let mut sink = codec_sink::<LogSyncMessage<[u8; 16]>>(&exchange, MAX_BYTES);
            let mut stream = codec_stream::<LogSyncMessage<[u8; 16]>>(&exchange, MAX_BYTES);
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
            drop(sink);
            drop(stream);
            let mut receipt_stream = codec_stream::<Receipt>(&exchange, 64);
            let receipt = receipt_stream
                .next()
                .await
                .ok_or(ReplicaError::Transport)?
                .map_err(|_| ReplicaError::Transport)?;
            if !receipt.records_saved {
                return Err(ReplicaError::Rejected);
            }
            let mut acknowledgement = codec_sink::<Receipt>(&exchange, 64);
            acknowledgement
                .send(Receipt {
                    records_saved: true,
                    view_applied: receipt.view_applied,
                })
                .await
                .map_err(|_| ReplicaError::Transport)?;
            exchange
                .finish_send_confirmed()
                .await
                .map_err(|_| ReplicaError::Transport)?;
            Ok(())
        })
        .await
        .unwrap_or(Err(ReplicaError::Transport));
        if result.is_err() {
            connection.close(0u32.into(), b"replica rejected");
        }
        result
    }
}

fn message_error(error: std::io::Error) -> ReplicaError {
    if error.kind() == std::io::ErrorKind::InvalidData {
        ReplicaError::Rejected
    } else {
        ReplicaError::Transport
    }
}

// Reuse the original p2panda postcard codec and LogSync algorithm. Complete
// messages replace raw streams; no read-ahead crosses the LogSync/receipt boundary.
fn codec_sink<M: serde::Serialize + Send>(
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
fn codec_stream<M: serde::de::DeserializeOwned + Send>(
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
