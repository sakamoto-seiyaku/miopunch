//! Browser adapter of the shared native LogSync wire protocol.
use super::*;
use crate::management_auth::decode_complete;
use crate::management_record::ManagementRecord;
use crate::replica_snapshot::{
    MAX_BYTES, MAX_RECORDS, Manifest, Receipt, ReceivedHistory, ReplicaProgress, Snapshot,
    codec_sink, codec_stream,
};
use futures_util::{SinkExt, StreamExt};
use iroh::endpoint::{ControlExchange, ControlMessageFormat};
use p2panda_sync::protocols::LogSyncMessage;

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
        let mut progress = ReplicaProgress::default();
        let result = self
            .receive_replica(&connection, deadline, &mut progress)
            .await;
        self.last_replica = Some(
            serde_json::json!({"before":before,"after":self.replica_session(&connection),
            "stableUnchanged":connection.stable_id()==original,"bindingUnchanged":replica_binding(&connection).is_ok_and(|current|current==binding),
            "callerBudgetMs":self.caller_budget.as_millis(),"operationMs":started.elapsed().as_millis(),
            "remainingCallerMs":deadline.saturating_duration_since(n0_future::time::Instant::now()).as_millis(),
            "transfer":progress,
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
        n0_future::time::timeout(
            deadline.saturating_duration_since(n0_future::time::Instant::now()),
            async {
                // Register the protocol before reading a potentially large installed
                // snapshot so the dispatcher can retain a concurrent replica caller.
                let (accepting, ()) = futures_util::try_join!(
                    self.incoming.receive(REPLICA_ALPN, &attempt),
                    self.check_store(),
                )?;
                let connection = accepting
                    .with_caller_deadline(deadline)
                    .with_attempt_cancellation(attempt.cancellation.clone())
                    .await
                    .map_err(error)?;
                let _scope = RequestConnection(Some(connection.clone()));
                self.check_store().await?;
                let saved = self.gate.service_snapshot()?;
                let auth = saved.verify()?.ok_or_else(rejected)?;
                let records: Vec<_> = saved
                    .records
                    .iter()
                    .map(|(header, body)| {
                        let verified = auth
                            .header_by_hash(Hash::digest(header))
                            .ok_or_else(rejected)?;
                        ManagementRecord::new(header.clone(), body.as_deref())
                            .with_verified_header(verified)
                            .ok_or_else(rejected)
                    })
                    .collect::<Result<_, _>>()?;
                let result = self.provide_replica(&connection, &records).await;
                connection.close(0u32.into(), b"replica served");
                result
            },
        )
        .await
        .map_err(error)?
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
        let id = self
            .gate
            .update_saved_owned(move |saved| {
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
        progress: &mut ReplicaProgress,
    ) -> Result<String, JsValue> {
        let result = n0_future::time::timeout(deadline.saturating_duration_since(n0_future::time::Instant::now()), async {
            if !self.replica_allowed(connection) { return Err(rejected()); }
            let (send, recv) = connection.open_bi().await.map_err(|_| error("replica transport failed"))?;
            let stream_id = send.id().to_string();
            let exchange = ControlExchange::from_streams(send, recv);
            // Preserve the lazy-open request and the original small-history wire.
            exchange.send_message(ControlMessageFormat::Fixed { length: 4 }, b"REPL")
                .await.map_err(|_| error("replica transport failed"))?;
            let mut received = ReceivedHistory::default();
            let known_ids: std::collections::BTreeMap<_, _> = self.saved.records.iter().enumerate()
                .map(|(index, (header, _))| (Hash::digest(header), index)).collect();
            loop {
                if !self.replica_allowed(connection) { return Err(rejected()); }
                let bytes = exchange.recv_message(ControlMessageFormat::LengthPrefixed32 { maximum: MAX_BYTES })
                    .await.map_err(|_| error("replica transport failed"))?;
                if bytes.is_empty() { return Err(rejected()); }
                let mut manifest: Manifest = p2panda_core::cbor::decode_cbor(bytes.as_slice()).map_err(|_| rejected())?;
                let more = manifest.more;
                if manifest.ids.len() > MAX_RECORDS { return Err(rejected()); }
                let auth = self.saved.verify()?.ok_or_else(rejected)?;
                let mut known_records = Vec::new();
                let mut known_bytes = 0;
                let mut saved = std::collections::BTreeSet::new();
                for id in &manifest.ids {
                    if let Some(index) = known_ids.get(id) {
                        let (header, body) = &self.saved.records[*index];
                        known_bytes += header.len() + body.as_ref().map_or(0, Vec::len);
                        if known_bytes > MAX_BYTES { return Err(rejected()); }
                        let verified = auth.header_by_hash(*id).ok_or_else(rejected)?;
                        known_records.push(ManagementRecord::new(header.clone(), body.as_deref())
                            .with_verified_header(verified).ok_or_else(rejected)?);
                        saved.insert(*id);
                    }
                }
                progress.reused += saved.len();
                let (known, _) = Snapshot::from_records(self.saved.invitation()?.anchor, &known_records).map_err(|_| rejected())?;
                let mut sink = codec_sink::<LogSyncMessage<[u8; 16]>>(&exchange, MAX_BYTES);
                let mut stream = codec_stream::<LogSyncMessage<[u8; 16]>>(&exchange, MAX_BYTES);
                {
                    let mut records = manifest.receive(self.saved.invitation()?.anchor, known, &mut sink, &mut stream).map_err(replica_exchange_error)?;
                    while let Some(record) = records.next().await.map_err(replica_exchange_error)? {
                        if !self.replica_allowed(connection) { return Err(rejected()); }
                        let (header, body) = record.into_parts();
                        saved.insert(self.ingest_record(header, body).await?);
                        progress.received += 1;
                    }
                }
                let page = manifest.received(saved);
                let page_saved = page.saved == page.scope;
                progress.completed_pages += usize::from(page_saved);
                received.append(page);
                drop(sink); drop(stream);
                if !more { break; }
                if !self.replica_allowed(connection) { return Err(rejected()); }
                codec_sink::<Receipt>(&exchange, 64).send(Receipt {
                    records_saved: page_saved, view_applied: false,
                }).await.map_err(|_| error("replica transport failed"))?;
                if !page_saved { return Err(rejected()); }
            }
            let applied_ids: std::collections::BTreeSet<_> = self.saved.records.iter()
                .map(|(header, _)| Hash::digest(header)).collect();
            let applied: Vec<_> = received.scope.iter().filter(|id| applied_ids.contains(id)).copied().collect();
            let receipt = received.receipt(&applied);
            let complete = receipt.records_saved && receipt.view_applied;
            let outcome = serde_json::json!({"source":connection.remote_id().to_string(), "scope":received.scope.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "saved":received.saved.iter().map(ToString::to_string).collect::<Vec<_>>(),"applied":applied.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "streamId":stream_id,"transport_done":true,"complete":complete,
                "received":progress.received,"reused":progress.reused,"completedPages":progress.completed_pages});
            if !self.replica_allowed(connection) { return Err(rejected()); }
            codec_sink::<Receipt>(&exchange, 64).send(receipt).await.map_err(|_| error("replica transport failed"))?;
            let ack = codec_stream::<Receipt>(&exchange, 64).next().await
                .ok_or_else(rejected)?.map_err(error)?;
            if !ack.records_saved { return Err(rejected()); }
            exchange.finish_send().await.map_err(error)?;
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
        let result = async {
            if !self.replica_allowed(connection) {
                return Err(rejected());
            }
            let (send, recv) = connection.accept_bi().await.map_err(error)?;
            let exchange = ControlExchange::from_streams(send, recv);
            crate::replica_snapshot::send_snapshot(
                &exchange, self.saved.invitation()?.anchor, records,
                || self.replica_allowed(connection),
            ).await.map_err(replica_exchange_error)
        }
        .await;
        if result.is_err() {
            connection.close(0u32.into(), b"replica rejected");
        }
        result
    }
}

fn replica_exchange_error(failure: crate::replica_snapshot::ExchangeError) -> JsValue {
    match failure {
        crate::replica_snapshot::ExchangeError::Rejected => rejected(),
        crate::replica_snapshot::ExchangeError::Transport => error("replica transport failed"),
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
        let mut auth = next.records.verified(anchor).map_err(error)?.clone();
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
        if crate::management_record::history_page(&next.pending)
            .map_err(error)?
            .len()
            != next.pending.len()
        {
            return Err(error(
                "unresolved management history exceeds the pending limit",
            ));
        }
        next.records.remember_verified(anchor, auth);
        Ok(next)
    }
}
