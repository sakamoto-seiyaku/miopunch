//! Browser adapter of the shared native LogSync wire protocol.
use super::*;
use crate::codec::Codec;
use crate::management_auth::{ManagementExtension, decode_complete};
use crate::management_record::ManagementRecord;
use crate::replica_snapshot::{Manifest, Receipt, Snapshot};
use futures_util::{SinkExt, StreamExt};
use p2panda_sync::protocols::{LogSync, LogSyncEvent, LogSyncMessage};
use p2panda_sync::traits::Protocol;
use std::collections::BTreeSet;
use tokio::sync::broadcast;
use tokio_util::codec::{FramedRead, FramedWrite};
const MAX_RECORDS: usize = 4096;
const MAX_BYTES: usize = 16 * 1024 * 1024;

#[wasm_bindgen]
impl BrowserNetwork {
    pub async fn ingest_management_record(
        &mut self,
        header: String,
        body: Option<String>,
    ) -> Result<String, JsValue> {
        let header = hex::decode(header).map_err(error)?;
        let body = body.map(hex::decode).transpose().map_err(error)?;
        Ok(self.ingest_record(header, body).await?.to_string())
    }
    pub fn control_record(&self, id: String) -> Result<String, JsValue> {
        let record = self
            .saved
            .records
            .iter()
            .find(|(header, _)| Hash::digest(header).to_string() == id)
            .ok_or_else(rejected)?;
        Ok(serde_json::json!({"header":hex::encode(&record.0),"body":record.1.as_ref().map(hex::encode)}).to_string())
    }
    pub fn control_state(&self) -> Result<String, JsValue> {
        let auth = self.saved.verify()?.ok_or_else(rejected)?;
        Ok(serde_json::json!({"records":self.saved.records.len(),"pending":self.saved.pending.len(),"members":auth.nodes().iter().map(|(id,access)|serde_json::json!({"id":id.to_string(),"administrator":*access==Access::manage()})).collect::<Vec<_>>()}).to_string())
    }
    pub async fn sync_replica(&mut self, target: String) -> Result<String, JsValue> {
        let connection = self
            .connect_target(target.parse().map_err(error)?, REPLICA_ALPN)
            .await?;
        let result = self.receive_replica(&connection).await;
        connection.close(0u32.into(), b"replica completed");
        result
    }
    /// Serve one authenticated member exchange. The caller drives the page's
    /// foreground accept loop; closing the page only disconnects the node.
    pub async fn serve_replica(&self) -> Result<(), JsValue> {
        self.check_store().await?;
        let incoming = self.endpoint.accept().await.ok_or_else(rejected)?;
        let connection = incoming.await.map_err(error)?;
        self.check_store().await?;
        let records: Vec<_> = self
            .saved
            .records
            .iter()
            .map(|(header, body)| ManagementRecord::new(header.clone(), body.as_deref()))
            .collect();
        let result = self.provide_replica(&connection, &records).await;
        connection.close(0u32.into(), b"replica served");
        result
    }
}
impl BrowserNetwork {
    fn replica_allowed(&self, connection: &Connection) -> bool {
        crate::member_path_protocol::MemberProtocol::from_wire(connection.alpn()).is_some_and(
            |(protocol, _)| protocol == crate::member_path_protocol::MemberProtocol::Replica,
        ) && BrowserGate::permits(&self.saved, connection.remote_id(), REPLICA_ALPN)
    }
    async fn consume_replica_event(
        &mut self,
        connection: &Connection,
        event: LogSyncEvent<ManagementExtension>,
        expected: &BTreeSet<Hash>,
        bytes: &mut usize,
    ) -> Result<Option<Hash>, JsValue> {
        let LogSyncEvent::OperationReceived { operation, .. } = event else {
            return Ok(None);
        };
        if !expected.contains(&operation.hash) || !self.replica_allowed(connection) {
            return Err(rejected());
        }
        let header = operation.header.encode();
        let body = operation.body.map(|body| body.to_bytes());
        *bytes += header.len() + body.as_ref().map_or(0, Vec::len);
        if *bytes > MAX_BYTES {
            return Err(rejected());
        }
        self.ingest_record(header, body).await.map(Some)
    }
    pub(super) async fn ingest_record(
        &mut self,
        header: Vec<u8>,
        body: Option<Vec<u8>>,
    ) -> Result<Hash, JsValue> {
        if header.len() + body.as_ref().map_or(0, Vec::len) > MAX_BYTES {
            return Err(rejected());
        }
        let anchor = self.saved.invitation()?.anchor;
        let id = decode_complete(&header, body.as_deref(), anchor)
            .map_err(error)?
            .hash();
        let next = self.saved.merge_records(vec![(header, body)])?;
        self.commit(next).await?;
        Ok(id)
    }
    async fn receive_replica(&mut self, connection: &Connection) -> Result<String, JsValue> {
        let result = n0_future::time::timeout(TIMEOUT, async {
            if !self.replica_allowed(connection) { return Err(rejected()); }
            let (send, mut recv) = connection.open_bi().await.map_err(|_| error("replica transport failed"))?;
            // QUIC opens the stream lazily; the request makes it visible before
            // the provider sends its manifest and starts the real LogSync.
            let mut send = send;
            send.write_all(b"REPL").await.map_err(|_| error("replica transport failed"))?;
            let mut size = [0;4]; recv.read_exact(&mut size).await.map_err(|_| error("replica transport failed"))?;
            let size = u32::from_be_bytes(size) as usize;
            if size == 0 || size > MAX_BYTES { return Err(rejected()); }
            let mut bytes = vec![0; size]; recv.read_exact(&mut bytes).await.map_err(|_| error("replica transport failed"))?;
            let manifest: Manifest = p2panda_core::cbor::decode_cbor(bytes.as_slice()).map_err(|_| rejected())?;
            let expected: BTreeSet<_> = manifest.ids.iter().copied().collect();
            if manifest.anchor != self.saved.invitation()?.anchor || manifest.ids.is_empty() || manifest.ids.len() > MAX_RECORDS || expected.len() != manifest.ids.len() || manifest.logs.values().map(Vec::len).sum::<usize>() > MAX_RECORDS {
                return Err(rejected());
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
                                (&mut run).await.map_err(|_| error("replica transport failed"))?;
                                break;
                            }
                            Err(_) => return Err(error("replica transport failed")),
                        }
                    }
                    result = &mut run => { result.map_err(|_| error("replica transport failed"))?; break; }
                }
            }
            drop(run);
            // Events enqueued during the protocol's final poll must also be
            // consumed, rather than being auto-acked by a body-only stream.
            while let Ok(event) = received.try_recv() {
                if let Some(id) = self.consume_replica_event(connection, event, &expected, &mut bytes).await? { saved_ids.insert(id); }
            }

            let mut applied = Vec::new();
            for id in &manifest.ids {
                if self.saved.records.iter().any(|(header,_)| Hash::digest(header) == *id) { applied.push(*id); }
            }
            let saved: Vec<_> = manifest.ids.iter().filter(|id| saved_ids.contains(id)).copied().collect();
            let complete = saved == manifest.ids && applied == manifest.ids;
            let outcome = serde_json::json!({"source":connection.remote_id().to_string(), "scope":manifest.ids.iter().map(ToString::to_string).collect::<Vec<_>>(), "transport_done":true,"complete":complete});
            let mut receipt_sink = FramedWrite::new(sink.into_inner(), Codec::<Receipt>::new().max_frame_len(64));
            receipt_sink.send(Receipt { records_saved: saved == manifest.ids, view_applied: applied == manifest.ids }).await.map_err(|_| error("replica transport failed"))?;
            let buffered = stream.read_buffer().clone();
            let mut acknowledgement = FramedRead::new(stream.into_inner(), Codec::<Receipt>::new().max_frame_len(64));
            acknowledgement.read_buffer_mut().extend_from_slice(&buffered);
            let ack = acknowledgement.next().await.ok_or_else(rejected)?.map_err(error)?;
            if !ack.records_saved { return Err(rejected()); }
            Ok(outcome.to_string())
        }).await.unwrap_or(Err(error("replica transport failed")));
        if result.is_err() {
            connection.close(0u32.into(), b"replica rejected");
        }
        result
    }
    async fn provide_replica(
        &self,
        connection: &Connection,
        records: &[ManagementRecord],
    ) -> Result<(), JsValue> {
        let result = n0_future::time::timeout(TIMEOUT, async {
            if !self.replica_allowed(connection) {
                return Err(rejected());
            }
            let (snapshot, manifest) =
                Snapshot::from_records(self.saved.invitation()?.anchor, records)
                    .map_err(|_| rejected())?;
            let (mut send, mut recv) = connection.accept_bi().await.map_err(error)?;
            let mut request = [0; 4];
            recv.read_exact(&mut request)
                .await
                .map_err(|_| error("replica transport failed"))?;
            if &request != b"REPL" {
                return Err(rejected());
            }
            if !self.replica_allowed(connection) {
                return Err(rejected());
            }
            let bytes = p2panda_core::cbor::encode_cbor(&manifest).map_err(|_| rejected())?;
            if bytes.len() > MAX_BYTES {
                return Err(rejected());
            }
            send.write_all(&(bytes.len() as u32).to_be_bytes())
                .await
                .map_err(|_| error("replica transport failed"))?;
            send.write_all(&bytes)
                .await
                .map_err(|_| error("replica transport failed"))?;
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
                .map_err(|_| error("replica transport failed"))?;
            // This is a one-way replica fetch, not an unsolicited publish route.
            while let Ok(event) = received.try_recv() {
                if matches!(event, LogSyncEvent::OperationReceived { .. }) {
                    return Err(rejected());
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
                .ok_or(error("replica transport failed"))?
                .map_err(|_| error("replica transport failed"))?;
            if !receipt.records_saved {
                return Err(rejected());
            }
            let mut acknowledgement =
                FramedWrite::new(sink.into_inner(), Codec::<Receipt>::new().max_frame_len(64));
            acknowledgement
                .send(Receipt {
                    records_saved: true,
                    view_applied: receipt.view_applied,
                })
                .await
                .map_err(error)?;
            let mut send = acknowledgement.into_inner();
            send.finish().map_err(error)?;
            connection.closed().await;
            Ok(())
        })
        .await
        .unwrap_or(Err(error("replica transport failed")));
        if result.is_err() {
            connection.close(0u32.into(), b"replica rejected");
        }
        result
    }
}

impl SavedNode {
    pub(super) fn merge_records(&self, records: Records) -> Result<Self, JsValue> {
        let anchor = self.invitation()?.anchor;
        let mut next = self.clone();
        for (header, body) in records {
            let id = decode_complete(&header, body.as_deref(), anchor)
                .map_err(error)?
                .hash();
            if !next
                .records
                .iter()
                .chain(next.pending.iter())
                .any(|(h, _)| Hash::digest(h) == id)
            {
                next.pending.push((header, body));
            }
        }
        if next.records.len() + next.pending.len() > MAX_RECORDS
            || next
                .records
                .iter()
                .chain(next.pending.iter())
                .map(|(h, b)| h.len() + b.as_ref().map_or(0, Vec::len))
                .sum::<usize>()
                > MAX_BYTES
        {
            return Err(rejected());
        }
        let mut auth = ManagementAuth::replay_records(anchor, &next.records).map_err(error)?;
        loop {
            let mut progress = false;
            let mut waiting = Vec::new();
            for record in std::mem::take(&mut next.pending) {
                match auth.accept_record(&record.0, record.1.as_deref()) {
                    Ok(_) => {
                        next.records.push(record);
                        progress = true;
                    }
                    Err(
                        crate::management_auth::AuthError::MissingHistory
                        | crate::management_auth::AuthError::InvalidLog,
                    ) => waiting.push(record),
                    Err(err) => return Err(error(err)),
                }
            }
            next.pending = waiting;
            if !progress {
                break;
            }
        }
        Ok(next)
    }
}
