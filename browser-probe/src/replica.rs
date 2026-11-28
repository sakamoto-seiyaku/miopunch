//! Browser adapter of the shared native LogSync wire protocol.
use super::*;
use crate::codec::Codec;
use crate::management_auth::{ManagementExtension, decode_complete};
use crate::management_record::ManagementRecord;
use crate::replica_snapshot::{Manifest, Receipt, Snapshot, MAX_BYTES, MAX_RECORDS};
use futures_util::{SinkExt, StreamExt};
use p2panda_sync::protocols::{LogSync, LogSyncEvent, LogSyncMessage};
use p2panda_sync::traits::Protocol;
use tokio::sync::broadcast;
use tokio_util::codec::{FramedRead, FramedWrite};

impl SavedNode {
    fn control_state(&self) -> Result<serde_json::Value, JsValue> {
        let auth = self.verify()?.ok_or_else(rejected)?;
        Ok(
            serde_json::json!({"records":self.records.len(),"pending":self.pending.len(),"members":auth.nodes().iter().map(|(id,access)|serde_json::json!({"id":id.to_string(),"administrator":*access==Access::manage()})).collect::<Vec<_>>() }),
        )
    }
}

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
        let saved = self.gate.service_snapshot()?;
        let record = saved
            .records
            .iter()
            .find(|(header, _)| Hash::digest(header).to_string() == id)
            .ok_or_else(rejected)?;
        Ok(serde_json::json!({"header":hex::encode(&record.0),"body":record.1.as_ref().map(hex::encode)}).to_string())
    }
    pub fn control_state(&self) -> Result<String, JsValue> {
        Ok(self.gate.service_snapshot()?.control_state()?.to_string())
    }
    pub async fn sync_replica(&mut self, target: String) -> Result<String, JsValue> {
        let started = n0_future::time::Instant::now();
        let deadline = started + self.caller_budget;
        self.last_replica = None;
        let target = n0_future::time::timeout(
            deadline.saturating_duration_since(n0_future::time::Instant::now()),
            async {
                self.refresh_service_snapshot().await?;
                let target = target.parse().map_err(error)?;
                if !BrowserGate::permits(&self.saved, target, REPLICA_ALPN) {
                    return Err(rejected());
                }
                Ok(target)
            },
        )
        .await
        .map_err(error)??;
        let connection = self
            .connect_target_until(target, REPLICA_ALPN, deadline)
            .await?;
        let _scope = RequestConnection(Some(connection.clone()));
        let original = connection.stable_id();
        let binding = replica_binding(&connection)?;
        let before = self.replica_session(&connection);
        let result = self.receive_replica(&connection, deadline).await;
        self.last_replica = Some(
            serde_json::json!({"before":before,"after":self.replica_session(&connection),
            "stableUnchanged":connection.stable_id()==original,"bindingUnchanged":replica_binding(&connection).is_ok_and(|current|current==binding),
            "callerBudgetMs":self.caller_budget.as_millis(),"operationMs":started.elapsed().as_millis(),
            "remainingCallerMs":deadline.saturating_duration_since(n0_future::time::Instant::now()).as_millis(),
            "completeReturned":result.as_ref().ok().and_then(|value|serde_json::from_str::<serde_json::Value>(value).ok()).is_some_and(|value|value["complete"]==true),
            "failure":result.as_ref().err().map(|cause|format!("{cause:?}"))}),
        );
        connection.close(0u32.into(), b"replica completed");
        result
    }
    /// The most recent actual original-M exchange, observed before close. No
    /// exporter, credentials or stored node material are exposed here.
    pub fn replica_diagnostics(&self) -> String {
        serde_json::to_string(&self.last_replica).expect("serializable replica observations")
    }
    /// Serve one authenticated member exchange. The caller drives the page's
    /// foreground accept loop; closing the page only disconnects the node.
    pub async fn serve_replica(&self) -> Result<(), JsValue> {
        let deadline = n0_future::time::Instant::now() + self.caller_budget;
        let attempt = crate::connection_coordinator::AttemptContext::new(
            deadline,
            tokio_util::sync::CancellationToken::new(),
        );
        self.check_store().await?;
        let accepting = self.incoming.receive(REPLICA_ALPN, &attempt).await?;
        let connection = n0_future::time::timeout(
            deadline.saturating_duration_since(n0_future::time::Instant::now()),
            accepting
                .with_caller_deadline(deadline)
                .with_attempt_cancellation(attempt.cancellation.clone()),
        )
        .await
        .map_err(error)?
        .map_err(error)?;
        let _scope = RequestConnection(Some(connection.clone()));
        self.check_store().await?;
        let records: Vec<_> = self
            .gate
            .service_snapshot()?
            .records
            .iter()
            .map(|(header, body)| ManagementRecord::new(header.clone(), body.as_deref()))
            .collect();
        let result = n0_future::time::timeout(
            deadline.saturating_duration_since(n0_future::time::Instant::now()),
            self.provide_replica(&connection, &records),
        )
        .await
        .map_err(error)?;
        connection.close(0u32.into(), b"replica served");
        result
    }
}
impl BrowserNetwork {
    fn replica_session(&self, connection: &Connection) -> serde_json::Value {
        let mut observation = serde_json::json!({"stableId":connection.stable_id(),"remoteId":connection.remote_id().to_string(),
            "alpn":String::from_utf8_lossy(connection.alpn()),"closed":connection.close_reason().is_some(),
            "currentPermission":self.replica_allowed(connection),"secretGeneration":self.saved.generation,
            "paths":connection.paths().iter().map(|path| {
                let stats=path.stats();
                let is_rtc={
                    #[cfg(feature="rtc-probe")]
                    {matches!(path.remote_addr(),iroh::TransportAddr::Custom(addr) if addr.id()==crate::rtc_transport::TRANSPORT_ID)}
                    #[cfg(not(feature="rtc-probe"))]
                    {false}
                };
                serde_json::json!({"pathId":path.id().to_string(),"selected":path.is_selected(),"isRelay":path.is_relay(),"isIp":path.is_ip(),
                    "isRtc":is_rtc,
                    "isSignal":matches!(path.remote_addr(),iroh::TransportAddr::Custom(addr) if matches!(addr.id(),crate::mqtt_datagram::TRANSPORT_ID | crate::irc_datagram::TRANSPORT_ID)),
                    "sentBytes":stats.udp_tx.bytes,"receivedBytes":stats.udp_rx.bytes,"streamFramesSent":stats.frame_tx.stream,
                    "streamFramesReceived":stats.frame_rx.stream,"datagramFramesSent":stats.frame_tx.datagram,"datagramFramesReceived":stats.frame_rx.datagram})
            }).collect::<Vec<_>>()});
        let gate = self.gate.0.lock().unwrap();
        observation["sourceRevision"] = serde_json::json!(
            gate.qualification_revision
                .load(std::sync::atomic::Ordering::Acquire)
        );
        #[cfg(feature = "rtc-probe")]
        {
            observation["bootstrap"] =
                serde_json::json!(gate.member_reports.get(&connection.stable_id()));
            observation["permissionRevision"] = serde_json::json!(
                gate.rtc_qualifications
                    .get(&connection.stable_id())
                    .map(|source| source.revision.load(std::sync::atomic::Ordering::Acquire))
            );
            observation["resourceRevision"] =
                serde_json::json!(self.rtc_transport.resource_revision());
            observation["activeAssociations"] =
                serde_json::json!(self.rtc_transport.active_associations());
            observation["activePumpTasks"] = serde_json::json!(self.rtc_transport.active_pumps());
        }
        observation
    }
    fn replica_allowed(&self, connection: &Connection) -> bool {
        connection.alpn() == REPLICA_ALPN
            && BrowserGate::proved_member_current(
                &self.gate.0.lock().unwrap(),
                &self.saved,
                connection,
            )
    }
    pub(super) async fn ingest_record(
        &mut self,
        header: Vec<u8>,
        body: Option<Vec<u8>>,
    ) -> Result<Hash, JsValue> {
        if header.len() + body.as_ref().map_or(0, Vec::len) > MAX_BYTES {
            return Err(rejected());
        }
        let id = self.gate.update_saved(|saved| {
            let anchor = saved.invitation()?.anchor;
            let id = decode_complete(&header, body.as_deref(), anchor)
                .map_err(error)?
                .hash();
            *saved = saved.merge_records(vec![(header, body)])?;
            Ok(id)
        })
        .await?;
        self.refresh_service_snapshot().await?;
        Ok(id)
    }
    async fn receive_replica(
        &mut self,
        connection: &Connection,
        deadline: n0_future::time::Instant,
    ) -> Result<String, JsValue> {
        let result = n0_future::time::timeout(TIMEOUT.min(deadline.saturating_duration_since(n0_future::time::Instant::now())), async {
            if !self.replica_allowed(connection) { return Err(rejected()); }
            let (send, mut recv) = connection.open_bi().await.map_err(|_| error("replica transport failed"))?;
            // QUIC opens the stream lazily; the request makes it visible before
            // the provider sends its manifest and starts the real LogSync.
            let mut send = send;
            let stream_id=send.id().to_string();
            send.write_all(b"REPL").await.map_err(|_| error("replica transport failed"))?;
            let mut size = [0;4]; recv.read_exact(&mut size).await.map_err(|_| error("replica transport failed"))?;
            let size = u32::from_be_bytes(size) as usize;
            if size == 0 || size > MAX_BYTES { return Err(rejected()); }
            let mut bytes = vec![0; size]; recv.read_exact(&mut bytes).await.map_err(|_| error("replica transport failed"))?;
            let mut manifest: Manifest = p2panda_core::cbor::decode_cbor(bytes.as_slice()).map_err(|_| rejected())?;
            let mut sink = FramedWrite::new(send, Codec::<LogSyncMessage<[u8; 16]>>::new().max_frame_len(MAX_BYTES));
            let mut stream = FramedRead::new(recv, Codec::<LogSyncMessage<[u8; 16]>>::new().max_frame_len(MAX_BYTES));
            let receive_error = |failure| match failure {
                crate::replica_snapshot::ReceiveError::Rejected => rejected(),
                crate::replica_snapshot::ReceiveError::Transport => error("replica transport failed"),
            };
            let mut saved = std::collections::BTreeSet::new();
            {
                let mut records = manifest.receive(self.saved.invitation()?.anchor, &mut sink, &mut stream).map_err(receive_error)?;
                while let Some((header, body)) = records.next().await.map_err(receive_error)? {
                    if !self.replica_allowed(connection) { return Err(rejected()); }
                    saved.insert(self.ingest_record(header, body).await?);
                }
            }
            let received = manifest.received(saved);
            let mut applied = Vec::new();
            for id in &received.scope {
                if self.saved.records.iter().any(|(header,_)| Hash::digest(header) == *id) { applied.push(*id); }
            }
            let receipt = received.receipt(&applied);
            let complete = receipt.records_saved && receipt.view_applied;
            let outcome = serde_json::json!({"source":connection.remote_id().to_string(), "scope":received.scope.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "saved":received.saved.iter().map(ToString::to_string).collect::<Vec<_>>(),"applied":applied.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "streamId":stream_id,"transport_done":true,"complete":complete});
            let mut receipt_sink = FramedWrite::new(sink.into_inner(), Codec::<Receipt>::new().max_frame_len(64));
            receipt_sink.send(receipt).await.map_err(|_| error("replica transport failed"))?;
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

fn replica_binding(connection: &Connection) -> Result<[u8; 32], JsValue> {
    let mut binding = [0; 32];
    connection
        .export_keying_material(&mut binding, b"miopunch/replica-session-observation/1", &[])
        .map_err(|_| error("original replica binding unavailable"))?;
    Ok(binding)
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
