//! Same-M Ordinary RTC experiment, using the existing member and endpoint.
use super::rtc::{ActiveAttempt, Association, stats};
use super::*;
use crate::{
    rtc_association::{
        CONTINUITY_BINDING_LABEL, CONTINUITY_MARKER, CONTROL_MARKER, EXPORTER_LABEL,
        MEMBER_RTC_PROTOCOL, RtcDeadline, attempt_context, datagram_channel_parameters,
    },
    rtc_transport::{MAX_DATAGRAM, RtcRouteLease},
    rtc_wire::{read_frame, write_frame},
};
use futures_util::{FutureExt, future::poll_fn};
use js_sys::Reflect;
use serde_json::{Value, json};
use std::{
    rc::Rc,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{RtcSdpType, RtcSessionDescriptionInit};

fn binding(connection: &Connection) -> Result<[u8; 32], JsValue> {
    let mut bytes = [0; 32];
    connection
        .export_keying_material(&mut bytes, CONTINUITY_BINDING_LABEL, &[])
        .map_err(|_| error("original M exporter unavailable"))?;
    Ok(bytes)
}

pub(super) struct Continuity {
    send: SendStream,
    recv: RecvStream,
    stream_id: String,
    stable_id: usize,
    binding: [u8; 32],
}
impl Continuity {
    async fn exchange(&mut self, connection: &Connection, sequence: u64) -> Result<Value, JsValue> {
        let payload = format!("original stream PING {sequence}");
        write_frame(
            &mut self.send,
            &json!({"sequence":sequence,"payload":payload}),
        )
        .await
        .map_err(error)?;
        let reply = read_frame(&mut self.recv).await.map_err(error)?;
        if reply["sequence"] != sequence
            || reply["payload"] != payload
            || reply["streamId"] != self.stream_id
            || reply["bindingUnchanged"] != true
        {
            return Err(error("original stream continuity mismatch"));
        }
        Ok(
            json!({"sequence":sequence,"streamId":self.stream_id,"localStableId":connection.stable_id(),"stableUnchanged":connection.stable_id()==self.stable_id,
            "bindingUnchanged":binding(connection)? == self.binding,"native":reply}),
        )
    }
}

#[derive(Default)]
struct PumpStats {
    sent_packets: AtomicU64,
    sent_bytes: AtomicU64,
    received_packets: AtomicU64,
    received_bytes: AtomicU64,
    dropped_packets: AtomicU64,
    dropped_bytes: AtomicU64,
    buffer_peak: AtomicU64,
}
impl PumpStats {
    fn report(&self) -> Value {
        json!({"actualSentPackets":self.sent_packets.load(Ordering::Relaxed),"actualSentBytes":self.sent_bytes.load(Ordering::Relaxed),
            "actualReceivedPackets":self.received_packets.load(Ordering::Relaxed),"actualReceivedBytes":self.received_bytes.load(Ordering::Relaxed),
            "droppedQuicPackets":self.dropped_packets.load(Ordering::Relaxed),"droppedQuicBytes":self.dropped_bytes.load(Ordering::Relaxed),"bufferPeak":self.buffer_peak.load(Ordering::Relaxed)})
    }
}
pub(super) struct OwnedRtc {
    association: Rc<Association>,
    lease: RtcRouteLease,
    attempt_live: Arc<AtomicBool>,
    drop_quic: Arc<AtomicBool>,
    stats: Arc<PumpStats>,
}
impl OwnedRtc {
    fn new(
        connection: &Connection,
        transport: &crate::rtc_transport::RtcTransport,
        gate: &BrowserGate,
    ) -> Result<Self, JsValue> {
        let (revision, observed, updates) = gate.rtc_qualification();
        let attempt_live = Arc::new(AtomicBool::new(true));
        let live = attempt_live.clone();
        let lease = transport
            .reserve_with_updates(
                connection,
                move || {
                    revision.load(Ordering::Acquire) == observed && live.load(Ordering::Acquire)
                },
                updates,
            )
            .map_err(error)?;
        let owner = Self {
            association: Rc::new(Association::datagram(connection)?),
            lease,
            attempt_live,
            drop_quic: Arc::new(AtomicBool::new(false)),
            stats: Arc::new(PumpStats::default()),
        };
        let receive_io = owner.lease.io();
        let receive_drop = owner.drop_quic.clone();
        let receive_stats = owner.stats.clone();
        let retire_io = owner.lease.io();
        owner.association.set_receiver(
            move |packet| {
                if receive_drop.load(Ordering::Acquire) {
                    receive_stats
                        .dropped_packets
                        .fetch_add(1, Ordering::Relaxed);
                    receive_stats
                        .dropped_bytes
                        .fetch_add(packet.len() as u64, Ordering::Relaxed);
                    Ok(())
                } else {
                    receive_stats
                        .received_packets
                        .fetch_add(1, Ordering::Relaxed);
                    receive_stats
                        .received_bytes
                        .fetch_add(packet.len() as u64, Ordering::Relaxed);
                    receive_io.receive(packet).map_err(error)
                }
            },
            move || retire_io.retire(),
        );
        Ok(owner)
    }
    fn start_pump(&self, original: iroh::endpoint::WeakConnectionHandle) {
        let association = self.association.clone();
        let io = self.lease.io();
        let drop_quic = self.drop_quic.clone();
        let stats = self.stats.clone();
        spawn_local(async move {
            let _driver = io.pump_guard();
            let work = async {
                loop {
                    let packet = poll_fn(|cx| io.poll_outbound(cx)).await.map_err(error)?;
                    if drop_quic.load(Ordering::Acquire) {
                        stats.dropped_packets.fetch_add(1, Ordering::Relaxed);
                        stats
                            .dropped_bytes
                            .fetch_add(packet.len() as u64, Ordering::Relaxed);
                        continue;
                    }
                    poll_fn(|cx| association.poll_capacity(cx, packet.len())).await?;
                    stats
                        .buffer_peak
                        .fetch_max(association.dc.buffered_amount() as u64, Ordering::Relaxed);
                    if !io.is_active() {
                        return Err(error("RTC route retired before channel send"));
                    }
                    association.dc.send_with_u8_array(&packet)?;
                    stats.sent_packets.fetch_add(1, Ordering::Relaxed);
                    stats
                        .sent_bytes
                        .fetch_add(packet.len() as u64, Ordering::Relaxed);
                }
                #[allow(unreachable_code)]
                Ok::<_, JsValue>(())
            }
            .fuse();
            let running = async { io.while_active(work).await.map_err(error)? }.fuse();
            let closed = original.closed().fuse();
            futures_util::pin_mut!(closed, running);
            let _ = futures_util::select_biased! { _ = closed => Ok::<_, JsValue>(()), result = running => result };
            io.retire();
            association.retire();
            association.detach();
        });
    }
    fn retire(&self) {
        self.attempt_live.store(false, Ordering::Release);
        self.lease.close();
        self.association.retire();
        self.association.detach();
    }
}
impl Drop for OwnedRtc {
    fn drop(&mut self) {
        self.retire();
    }
}

async fn phase(recv: &mut RecvStream, expected: &str) -> Result<Value, JsValue> {
    let message = read_frame(recv).await.map_err(error)?;
    if message["kind"] != expected {
        return Err(error(format!(
            "expected RTC {expected}, got {}",
            message["kind"]
        )));
    }
    Ok(message)
}

async fn metadata(association: &Association) -> Result<Value, JsValue> {
    let property = |object: &JsValue, name: &str| Reflect::get(object, &JsValue::from_str(name));
    let sctp = property(association.pc.as_ref(), "sctp")?;
    let dtls = property(&sctp, "transport")?;
    let ice = property(&dtls, "iceTransport")?;
    Ok(
        json!({"channel":{"id":association.dc.id(),"label":association.dc.label(),"protocol":property(association.dc.as_ref(),"protocol")?.as_string(),"ordered":property(association.dc.as_ref(),"ordered")?.as_bool(),
        "maxRetransmits":association.dc.max_retransmits(),"maxPacketLifeTime":association.dc.max_packet_life_time(),"negotiated":property(association.dc.as_ref(),"negotiated")?.as_bool()},
        "maxMessageSize":property(&sctp,"maxMessageSize")?.as_f64(),"iceState":property(&ice,"state")?.as_string(),"iceRole":property(&ice,"role")?.as_string(),
        "dtlsState":property(&dtls,"state")?.as_string(),"sctpState":property(&sctp,"state")?.as_string(),"stats":stats(&association.pc).await?}),
    )
}

/// The admission hook owns this setup, independent of a business or replica
/// handle. Its successful holder contains no Endpoint, strong M or setup clock.
pub(super) async fn bootstrap_control(
    endpoint: &Endpoint,
    connection: &Connection,
    gate: &BrowserGate,
    saved: &SavedNode,
    transport: &crate::rtc_transport::RtcTransport,
) -> Result<Value, JsValue> {
    let budget = RtcDeadline::from_caller(
        connection
            .caller_deadline()
            .ok_or_else(|| error("RTC caller deadline absent"))?,
    );
    let owner = OwnedRtc::new(connection, transport, gate)?;
    let stable = connection.stable_id();
    let original_binding = binding(connection)?;
    let attempt = hex::encode(rand::random::<[u8; 16]>());
    let context = attempt_context(
        &attempt,
        saved.invitation()?.anchor.root.as_bytes(),
        saved.generation,
        &endpoint.id().to_string(),
        &connection.remote_id().to_string(),
    );
    let mut exporter = [0; 32];
    connection
        .export_keying_material(&mut exporter, EXPORTER_LABEL, context.as_bytes())
        .map_err(|_| error("RTC original M exporter unavailable"))?;
    let protected_binding = blake3::hash(&exporter).to_hex().to_string();
    let (mut send, mut recv) = connection.open_bi().await.map_err(error)?;
    let stream_id = send.id().to_string();
    let (revision, observed, _) = gate.rtc_qualification();
    let work = async {
        let offer: RtcSessionDescriptionInit = JsFuture::from(owner.association.pc.create_offer()).await?.unchecked_into();
        JsFuture::from(owner.association.pc.set_local_description(&offer)).await?;
        owner.association.gathered().await?;
        let offer = owner.association.pc.local_description().ok_or_else(|| error("RTC offer absent"))?;
        write_frame(&mut send, &json!({"protocol":MEMBER_RTC_PROTOCOL,"attempt":attempt,"context":context,"binding":protected_binding,
            "description":{"type":"offer","sdp":offer.sdp()},"channel":datagram_channel_parameters(),"route":hex::encode(owner.lease.local_addr().data()),
            "quicDialer":endpoint.id().to_string(),"rtcOfferer":endpoint.id().to_string(),"rtcAnswerer":connection.remote_id().to_string(),
            "remainingMs":budget.remaining().map_err(error)?.as_millis() as u64,"authorizationRevision":observed,"resourceRevision":transport.resource_revision()})).await.map_err(error)?;
        let answer = phase(&mut recv, "answer").await?;
        if answer["attempt"] != attempt || answer["binding"] != protected_binding || answer["channel"] != datagram_channel_parameters() {
            return Err(error("RTC answer context/profile binding mismatch"));
        }
        owner.lease.set_remote(iroh_base::CustomAddr::from_parts(crate::rtc_transport::TRANSPORT_ID,
            &hex::decode(answer["route"].as_str().ok_or_else(|| error("RTC peer route absent"))?).map_err(error)?)).map_err(error)?;
        let init = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
        init.set_sdp(answer["description"]["sdp"].as_str().ok_or_else(|| error("RTC native answer absent"))?);
        JsFuture::from(owner.association.pc.set_remote_description(&init)).await?;
        owner.association.wait_open().await?;
        let opening = metadata(&owner.association).await?;
        if opening["channel"] != datagram_channel_parameters()
            || opening["maxMessageSize"].as_f64().is_none_or(|maximum| maximum < MAX_DATAGRAM as f64)
        { return Err(error("RTC negotiated profile/maximum mismatch")); }
        owner.lease.activate().map_err(error)?;
        owner.start_pump(connection.weak_handle());
        let actual = owner.stats.clone();
        transport.control_fixture_checkpoint(crate::rtc_invalidation_fixture::ControlCheckpointStage::ChannelReady,
            connection, &owner.lease, budget,
            json!({"attempt":attempt,"stableId":stable,"localRoute":hex::encode(owner.lease.local_addr().data()),
                "remoteRoute":hex::encode(owner.lease.remote_addr().map_err(error)?.data()),"freshAck":false,
                "authorizationRevision":observed,"secretGeneration":saved.generation,"resourceRevision":transport.resource_revision()}),
            Arc::new(move || actual.report())).await.map_err(error)?;
        phase(&mut recv, "channel-open").await?;
        write_frame(&mut send, &json!({"kind":"channel-open"})).await.map_err(error)?;
        let remote = iroh::TransportAddr::Custom(owner.lease.remote_addr().map_err(error)?);
        loop {
            budget.remaining().map_err(error)?;
            match endpoint.open_control_path(connection, owner.lease.remote_addr().map_err(error)?).await {
                Ok(()) => break,
                Err(cause) if cause.kind() == std::io::ErrorKind::WouldBlock => n0_future::time::sleep(Duration::from_millis(10)).await,
                Err(cause) => return Err(error(cause)),
            }
        }
        loop {
            budget.remaining().map_err(error)?;
            if connection.paths().iter().any(|path| path.remote_addr() == &remote) { break; }
            n0_future::time::sleep(Duration::from_millis(10)).await;
        }
        write_frame(&mut send, &json!({"kind":"begin-probe","dropQuic":false})).await.map_err(error)?;
        phase(&mut recv, "probe-started").await?;
        let probe = owner.lease.start_probe(connection).map_err(error)?;
        let probe_started = budget.elapsed_ms();
        while !probe.is_acknowledged().map_err(error)? {
            budget.remaining().map_err(error)?;
            n0_future::time::sleep(Duration::from_millis(10)).await;
        }
        let acknowledged = budget.elapsed_ms();
        write_frame(&mut send, &json!({"kind":"probe-ack"})).await.map_err(error)?;
        phase(&mut recv, "probe-ack").await?;
        gate.qualified_snapshot(connection).await?;
        if revision.load(Ordering::Acquire) != observed || !owner.lease.io().is_active() {
            return Err(error("RTC qualification changed before bilateral retain"));
        }
        write_frame(&mut send, &json!({"kind":"retain"})).await.map_err(error)?;
        phase(&mut recv, "retained").await?;
        let pending_paths = connection.paths();
        let pending = pending_paths.iter().find(|path| path.id().to_string() == probe.path_id())
            .ok_or_else(|| error("RTC exact probe path disappeared"))?;
        let before_retain_frames = json!({"streamSent":pending.stats().frame_tx.stream,"streamReceived":pending.stats().frame_rx.stream,
            "datagramSent":pending.stats().frame_tx.datagram,"datagramReceived":pending.stats().frame_rx.datagram});
        owner.lease.retain(connection, &probe).map_err(error)?;
        write_frame(&mut send, &json!({"kind":"ready"})).await.map_err(error)?;
        endpoint.retain_control_data_transport_path(connection, owner.lease.remote_addr().map_err(error)?).await.map_err(error)?;
        phase(&mut recv, "selected").await?;
        // This very private stream began over MQTT and now confirms over RTC.
        write_frame(&mut send, &json!({"kind":"finish","sequence":1})).await.map_err(error)?;
        let native = phase(&mut recv, "final").await?["native"].clone();
        let actual = owner.stats.clone();
        transport.control_fixture_checkpoint(crate::rtc_invalidation_fixture::ControlCheckpointStage::BeforeRelease,
            connection, &owner.lease, budget,
            json!({"attempt":attempt,"stableId":stable,"localRoute":hex::encode(owner.lease.local_addr().data()),
                "remoteRoute":hex::encode(owner.lease.remote_addr().map_err(error)?.data()),"freshAck":true,"bothConfirmed":true,
                "pathId":probe.path_id(),"probeWatermark":probe.watermark(),"freshProbeStartedMs":probe_started,"freshAckMs":acknowledged,
                "authorizationRevision":observed,"secretGeneration":saved.generation,"resourceRevision":transport.resource_revision()}),
            Arc::new(move || actual.report())).await.map_err(error)?;
        gate.qualified_snapshot(connection).await?;
        let release_started = budget.elapsed_ms();
        let release_probe = owner.lease.validate_release(connection, budget).await.map_err(error)?;
        gate.qualified_snapshot(connection).await?;
        if revision.load(Ordering::Acquire) != observed || !owner.lease.io().is_active() {
            return Err(error("RTC qualification changed during final release validation"));
        }
        send.finish().map_err(error)?;
        Ok::<_, JsValue>(json!({"switched":true,"memberStableId":stable,"stableUnchanged":connection.stable_id()==stable,
            "bindingUnchanged":binding(connection)?==original_binding,"privateStreamId":stream_id,"privateSequence":1,
            "pathId":probe.path_id(),"freshProbeStartedMs":probe_started,"freshAckMs":acknowledged,"freshAck":true,
            "probeWatermark":probe.watermark(),"beforeRetainFrames":before_retain_frames,"rtc":metadata(&owner.association).await?,
            "finalReleaseFreshAck":true,"finalReleaseProbeWatermark":release_probe.watermark(),
            "finalReleaseProbeStartedMs":release_started,"finalReleaseAckMs":budget.elapsed_ms(),
            "route":owner.lease.report(),"dataChannel":owner.stats.report(),"native":native,"authorizationRevision":observed,
            "secretGeneration":saved.generation,"resourceRevision":transport.resource_revision(),"quicRole":"client","rtcRole":"offerer",
            "attemptMs":budget.elapsed_ms()}))
    }.fuse();
    let closed = connection.closed().fuse();
    let report = {
        futures_util::pin_mut!(work, closed);
        n0_future::time::timeout(budget.remaining().map_err(error)?, async {
            futures_util::select_biased! { _ = closed => Err(error("RTC original M closed")), result = work => result }
        }).await.map_err(|_| error("RTC original caller deadline exceeded"))??
    };
    let original = connection.weak_handle();
    let io = owner.lease.io();
    spawn_local(async move {
        let retired = io
            .while_active(futures_util::future::pending::<()>())
            .fuse();
        let closed = original.closed().fuse();
        futures_util::pin_mut!(retired, closed);
        futures_util::select_biased! { _ = closed => {}, _ = retired => {} }
        owner.retire();
    });
    Ok(report)
}

/// Owned diagnostics operate on this installed carrier and pending hook. They
/// neither expose an unadmitted M nor change saved membership/materials.
#[wasm_bindgen]
impl BrowserNetwork {
    pub fn rtc_invalidation_arm(&self, stage: String) -> Result<(), JsValue> {
        use crate::rtc_invalidation_fixture::ControlCheckpointStage;
        let stage = match stage.as_str() {
            "channel-ready" => ControlCheckpointStage::ChannelReady,
            "before-release" => ControlCheckpointStage::BeforeRelease,
            _ => return Err(error("unknown owned Control checkpoint")),
        };
        self.rtc_transport.arm_control_fixture(stage).map_err(error)
    }

    pub fn rtc_invalidation_diagnostics(&self) -> String {
        let gate = self.gate.0.lock().unwrap();
        json!({"checkpoint":self.rtc_transport.control_fixture_snapshot(),
            "publishedRoutes":self.rtc_transport.published_routes_fixture(),
            "activeAssociations":self.rtc_transport.active_associations(),"activePumpTasks":self.rtc_transport.active_pumps(),
            "resourceRevision":self.rtc_transport.resource_revision(),"resourceAllowed":self.rtc_transport.resource_allowed(),
            "sourceRevision":gate.qualification_revision.load(Ordering::Acquire),
            "secretGeneration":gate.saved.as_ref().map(|saved|saved.generation)}).to_string()
    }

    pub fn rtc_invalidation_release(&self, attempt: String) -> Result<(), JsValue> {
        self.rtc_transport
            .release_control_fixture(&attempt)
            .map_err(error)
    }

    pub fn rtc_invalidation_clear(&self) {
        self.rtc_transport.clear_control_fixture();
    }
}

#[wasm_bindgen]
impl BrowserMemberConnection {
    /// Start a bounded real stream on the already qualified WSS-started M.
    pub async fn rtc_continuity_start(&self) -> Result<String, JsValue> {
        self.qualified_snapshot().await?;
        if self.rtc_continuity.borrow().is_some() {
            return Err(error("continuity stream already started"));
        }
        n0_future::time::timeout(TIMEOUT, async {
            let (mut send, recv) = self.connection.open_bi().await.map_err(error)?;
            send.write_all(CONTINUITY_MARKER).await.map_err(error)?;
            let mut stream = Continuity {
                stream_id: send.id().to_string(),
                send,
                recv,
                stable_id: self.connection.stable_id(),
                binding: binding(&self.connection)?,
            };
            let first = stream.exchange(&self.connection, 0).await?;
            *self.rtc_continuity.borrow_mut() = Some(stream);
            Ok(first.to_string())
        })
        .await
        .map_err(|_| error("continuity start caller deadline"))?
    }
    /// Preserve the existing relay M and original stream. `caller_ms` is the
    /// existing caller remainder, bounded by the browser's ordinary 12 seconds.
    pub async fn rtc_relay_member_probe(
        &self,
        drop_probe_quic: bool,
        caller_ms: u32,
    ) -> Result<String, JsValue> {
        self.rtc_member_attempt(drop_probe_quic, false, caller_ms)
            .await
    }
    /// Owned diagnostic fault: terminate the native bridge driver immediately
    /// after its fresh authenticated QUIC acknowledgment.
    pub async fn rtc_native_pump_failure_probe(&self, caller_ms: u32) -> Result<String, JsValue> {
        self.rtc_member_attempt(false, true, caller_ms).await
    }
}
impl BrowserMemberConnection {
    async fn rtc_member_attempt(
        &self,
        drop_probe_quic: bool,
        fault_native_pump: bool,
        caller_ms: u32,
    ) -> Result<String, JsValue> {
        if caller_ms == 0
            || caller_ms > 12000
            || !self.connection.side().is_client()
            || self.rtc_live.borrow().is_some()
        {
            return Err(error(
                "RTC requires the existing browser-dialed relay M and caller remainder",
            ));
        }
        let budget = RtcDeadline::from_caller(
            n0_future::time::Instant::now() + Duration::from_millis(caller_ms as u64),
        );
        let saved = n0_future::time::timeout(
            budget.remaining().map_err(error)?,
            self.qualified_snapshot(),
        )
        .await
        .map_err(|_| error("RTC qualification caller deadline"))??;
        if !self.connection.paths().iter().any(|path| path.is_relay()) {
            return Err(error(
                "RTC ordinary experiment requires a current allowed relay path",
            ));
        }
        self.rtc_active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| error("RTC attempt already active on this M"))?;
        let _active = ActiveAttempt(self.rtc_active.clone());
        let (_, observed, _) = self.rtc_qualification();
        let owner = OwnedRtc::new(&self.connection, &self.rtc_transport, &self.gate)?;
        let attempt = hex::encode(rand::random::<[u8; 16]>());
        let context = attempt_context(
            &attempt,
            saved.invitation()?.anchor.root.as_bytes(),
            saved.generation,
            &saved.identity().to_string(),
            &self.connection.remote_id().to_string(),
        );
        let mut exporter = [0; 32];
        self.connection
            .export_keying_material(&mut exporter, EXPORTER_LABEL, context.as_bytes())
            .map_err(|_| error("RTC M exporter unavailable"))?;
        let protected_binding = blake3::hash(&exporter).to_hex().to_string();
        let initial_paths = self.connection.paths();
        let relay_paths: Vec<_> = initial_paths
            .iter()
            .filter(|path| path.is_relay())
            .collect();
        let relay_before: Vec<_> = relay_paths
            .iter()
            .map(|path| {
                (
                    path.id().to_string(),
                    path.stats().udp_tx.bytes,
                    path.stats().udp_rx.bytes,
                )
            })
            .collect();
        let (mut send, mut recv, mut original) =
            n0_future::time::timeout(budget.remaining().map_err(error)?, async {
                let (mut send, recv) = self.connection.open_bi().await.map_err(error)?;
                send.write_all(CONTROL_MARKER).await.map_err(error)?;
                let (mut original_send, original_recv) =
                    self.connection.open_bi().await.map_err(error)?;
                original_send
                    .write_all(CONTINUITY_MARKER)
                    .await
                    .map_err(error)?;
                let original = Continuity {
                    stream_id: original_send.id().to_string(),
                    send: original_send,
                    recv: original_recv,
                    stable_id: self.connection.stable_id(),
                    binding: binding(&self.connection)?,
                };
                Ok::<_, JsValue>((send, recv, original))
            })
            .await
            .map_err(|_| error("RTC stream preparation caller deadline"))??;
        let mut continuity = Vec::new();
        let mut probe_started_ms = None;
        let mut probe_path_id = None;
        let outcome = {
            let work = async {
            continuity.push(original.exchange(&self.connection, 0).await?);
            let offer: RtcSessionDescriptionInit = JsFuture::from(owner.association.pc.create_offer()).await?.unchecked_into();
            JsFuture::from(owner.association.pc.set_local_description(&offer)).await?;
            owner.association.gathered().await?;
            let offer = owner.association.pc.local_description().ok_or_else(|| error("RTC offer absent"))?;
            write_frame(&mut send, &json!({"protocol":MEMBER_RTC_PROTOCOL,"attempt":attempt,"context":context,"binding":protected_binding,"description":{"type":"offer","sdp":offer.sdp()},
                "channel":datagram_channel_parameters(),"route":hex::encode(owner.lease.local_addr().data()),"quicDialer":self.endpoint.id().to_string(),
                "rtcOfferer":self.endpoint.id().to_string(),"rtcAnswerer":self.connection.remote_id().to_string(),"remainingMs":budget.remaining().map_err(error)?.as_millis() as u64,
                "authorizationRevision":observed,"resourceRevision":self.rtc_transport.resource_revision(),"faultNativePumpAfterFreshAck":fault_native_pump})).await.map_err(error)?;
            let answer = phase(&mut recv, "answer").await?;
            if answer["attempt"] != attempt || answer["binding"] != protected_binding || answer["channel"] != datagram_channel_parameters() { return Err(error("RTC answer context/profile binding mismatch")); }
            owner.lease.set_remote(iroh_base::CustomAddr::from_parts(crate::rtc_transport::TRANSPORT_ID, &hex::decode(answer["route"].as_str().ok_or_else(|| error("RTC peer route absent"))?).map_err(error)?)).map_err(error)?;
            // The original business stream responds while RTC is still being negotiated.
            continuity.push(original.exchange(&self.connection, 1).await?);
            let init = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
            init.set_sdp(answer["description"]["sdp"].as_str().ok_or_else(|| error("RTC native answer absent"))?);
            JsFuture::from(owner.association.pc.set_remote_description(&init)).await?;
            owner.association.wait_open().await?;
            let opening = metadata(&owner.association).await?;
            if opening["channel"] != datagram_channel_parameters() || opening["maxMessageSize"].as_f64().is_none_or(|maximum| maximum < MAX_DATAGRAM as f64) { return Err(error("RTC negotiated profile/maximum mismatch")); }
            owner.lease.activate().map_err(error)?;
            owner.start_pump(self.connection.weak_handle());
            phase(&mut recv, "channel-open").await?;
            write_frame(&mut send, &json!({"kind":"channel-open"})).await.map_err(error)?;
            let rtc_remote = iroh::TransportAddr::Custom(owner.lease.remote_addr().map_err(error)?);
            loop {
                budget.remaining().map_err(error)?;
                match self.endpoint.open_data_transport_path(&self.connection, iroh::TransportAddr::Custom(owner.lease.remote_addr().map_err(error)?)).await {
                    Ok(()) => break,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => n0_future::time::sleep(Duration::from_millis(10)).await,
                    Err(cause) => return Err(error(cause)),
                }
            }
            // Wait for real QUIC establishment before injecting probe loss.
            loop {
                budget.remaining().map_err(error)?;
                if self.connection.paths().iter().any(|path| path.remote_addr() == &rtc_remote) { break; }
                n0_future::time::sleep(Duration::from_millis(10)).await;
            }
            write_frame(&mut send, &json!({"kind":"begin-probe","dropQuic":drop_probe_quic})).await.map_err(error)?;
            phase(&mut recv, "probe-started").await?;
            owner.drop_quic.store(drop_probe_quic, Ordering::Release);
            let probe = owner.lease.start_probe(&self.connection).map_err(error)?;
            probe_started_ms = Some(budget.elapsed_ms());
            probe_path_id = Some(probe.path_id().to_owned());
            while !probe.is_acknowledged().map_err(error)? { budget.remaining().map_err(error)?; n0_future::time::sleep(Duration::from_millis(10)).await; }
            let ack_ms = budget.elapsed_ms();
            write_frame(&mut send, &json!({"kind":"probe-ack"})).await.map_err(error)?;
            phase(&mut recv, "probe-ack").await?;
            self.qualified_snapshot().await?;
            if !owner.lease.io().is_active() { return Err(error("RTC qualification changed before bilateral retain")); }
            write_frame(&mut send, &json!({"kind":"retain"})).await.map_err(error)?;
            phase(&mut recv, "retained").await?;
            let pending_paths = self.connection.paths();
            let pending = pending_paths.iter().find(|path| path.id().to_string() == probe.path_id()).ok_or_else(|| error("RTC exact probe path disappeared"))?;
            let before_retain_frames = json!({"streamSent":pending.stats().frame_tx.stream,"streamReceived":pending.stats().frame_rx.stream,"datagramSent":pending.stats().frame_tx.datagram,"datagramReceived":pending.stats().frame_rx.datagram});
            owner.lease.retain(&self.connection, &probe).map_err(error)?;
            write_frame(&mut send, &json!({"kind":"ready"})).await.map_err(error)?;
            self.endpoint.retain_data_transport_path(&self.connection, owner.lease.remote_addr().map_err(error)?).await.map_err(error)?;
            phase(&mut recv, "selected").await?;
            write_frame(&mut send, &json!({"kind":"disable-relay"})).await.map_err(error)?;
            for path in self.connection.paths().iter().filter(|path| path.is_relay()) { path.close().map_err(error)?; }
            phase(&mut recv, "relay-disabled").await?;
            continuity.push(original.exchange(&self.connection, 2).await?);
            write_frame(&mut send, &json!({"kind":"finish"})).await.map_err(error)?;
            let native = phase(&mut recv, "final").await?["native"].clone();
            send.finish().map_err(error)?;
            let relay_after: Vec<_> = relay_paths.iter().map(|path| (path.id().to_string(), path.stats().udp_tx.bytes, path.stats().udp_rx.bytes)).collect();
            let browser = json!({"memberStableId":self.connection.stable_id(),"stableUnchanged":self.connection.stable_id()==original.stable_id,"bindingUnchanged":binding(&self.connection)? == original.binding,
                "pathId":probe.path_id(),"freshProbeStartedMs":probe_started_ms,"freshAckMs":ack_ms,"freshAck":true,"probeWatermark":probe.watermark(),"beforeRetainFrames":before_retain_frames,"rtc":metadata(&owner.association).await?,"route":owner.lease.report(),"dataChannel":owner.stats.report(),
                "relayBefore":relay_before,"relayAfter":relay_after,"authorizationRevision":observed,"secretGeneration":saved.generation,"resourceRevision":self.rtc_transport.resource_revision(),"quicRole":"client","rtcRole":"offerer","iceServers":[],"turnServers":[],"events":owner.association.events.borrow().clone(),"ownedPumpTasks":owner.lease.report().owned_pump_tasks});
            Ok::<_, JsValue>(json!({"switched":true,"browser":browser,"native":native,"continuity":continuity}))
        }.fuse();
            let closed = self.connection.closed().fuse();
            futures_util::pin_mut!(work, closed);
            n0_future::time::timeout(budget.remaining().unwrap_or(Duration::ZERO), async {
            futures_util::select_biased! { _ = closed => Err(error("RTC original M closed")), result = work => result }
        }).await.map_err(|_| error("RTC caller deadline exceeded")).and_then(|result| result)
        };
        let mut report = match outcome {
            Ok(report) => {
                *self.rtc_live.borrow_mut() = Some(owner);
                report
            }
            Err(cause) => {
                let _ = send.reset(0u32.into());
                let before_cleanup = owner.lease.report();
                owner.retire();
                // Allow the woken local pump to observe its retired queue and
                // drop its task guard before collecting the cleanup receipt.
                n0_future::time::sleep(Duration::ZERO).await;
                json!({"switched":false,"failure":cause.as_string(),"memberStableId":self.connection.stable_id(),"stableUnchanged":self.connection.stable_id()==original.stable_id,"bindingUnchanged":binding(&self.connection)? == original.binding,
                    "pathId":probe_path_id,"freshProbeStartedMs":probe_started_ms,"freshAck":false,"beforeCleanup":before_cleanup,"route":owner.lease.report(),"dataChannel":owner.stats.report(),"pcClosed":owner.association.pc.connection_state()==web_sys::RtcPeerConnectionState::Closed,"callbacksDetached":owner.association.pc.onicegatheringstatechange().is_none() && owner.association.pc.oniceconnectionstatechange().is_none() && owner.association.pc.onconnectionstatechange().is_none(),"ownedPumpTasks":owner.lease.report().owned_pump_tasks,"continuity":continuity,"relayStillPresent":self.connection.paths().iter().any(|path| path.is_relay())})
            }
        };
        report["callerBudgetMs"] = json!(caller_ms);
        report["diagnosticCapMs"] = json!(45000);
        report["attemptMs"] = json!(budget.elapsed_ms());
        *self.rtc_continuity.borrow_mut() = Some(original);
        serde_json::to_string(&report).map_err(error)
    }
}

#[wasm_bindgen]
impl BrowserMemberConnection {
    /// A separate ordinary business operation on the retained original stream.
    pub async fn rtc_continuity_exchange(&self, sequence: u64) -> Result<String, JsValue> {
        self.qualified_snapshot().await?;
        let mut stream = self
            .rtc_continuity
            .borrow_mut()
            .take()
            .ok_or_else(|| error("original continuity stream unavailable or busy"))?;
        let result = n0_future::time::timeout(TIMEOUT, stream.exchange(&self.connection, sequence))
            .await
            .map_err(|_| error("ordinary continuity caller deadline"))?;
        *self.rtc_continuity.borrow_mut() = Some(stream);
        serde_json::to_string(&result?).map_err(error)
    }

    pub fn rtc_path_diagnostics(&self) -> String {
        json!({"memberStableId":self.connection.stable_id(),"closed":self.is_closed(),"activeAssociations":self.rtc_transport.active_associations(),"activePumpTasks":self.rtc_transport.active_pumps(),"resourceRevision":self.rtc_transport.resource_revision(),
            "bootstrap":self.gate.0.lock().unwrap().rtc_reports.get(&self.connection.stable_id()),
            "paths":self.connection.paths().iter().map(|path|json!({"pathId":path.id().to_string(),"address":format!("{:?}",path.remote_addr()),"selected":path.is_selected(),"isRelay":path.is_relay(),"isRtc":matches!(path.remote_addr(),iroh::TransportAddr::Custom(address) if address.id()==crate::rtc_transport::TRANSPORT_ID),
                "isSignal":matches!(path.remote_addr(),iroh::TransportAddr::Custom(address) if matches!(address.id(),crate::mqtt_datagram::TRANSPORT_ID | crate::irc_datagram::TRANSPORT_ID)),
                "sentBytes":path.stats().udp_tx.bytes,"receivedBytes":path.stats().udp_rx.bytes,"streamFramesSent":path.stats().frame_tx.stream,"streamFramesReceived":path.stats().frame_rx.stream,
                "pingFramesSent":path.stats().frame_tx.ping,"ackFramesReceived":path.stats().frame_rx.acks,"ackFramesSent":path.stats().frame_tx.acks})).collect::<Vec<_>>()}).to_string()
    }
}
