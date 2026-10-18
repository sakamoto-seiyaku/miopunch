//! Same-M Ordinary RTC experiment, using the existing member and endpoint.
use super::rtc::{Association, stats};
use super::*;
use crate::{
    rtc_association::{
        CONTINUITY_BINDING_LABEL, CONTINUITY_MARKER, EXPORTER_LABEL, RtcDeadline, attempt_context,
        datagram_channel_parameters,
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

fn binding(connection: &(impl AuthenticatedControl + ?Sized)) -> Result<[u8; 32], JsValue> {
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
    /// Observe a real write error on the retained old stream, only after its
    /// original QUIC connection is already Terminal. Healthy business is never
    /// bypassed through this fixture.
    pub(super) async fn terminal_write_failure(
        &mut self,
        connection: &Connection,
    ) -> Result<(String, bool), JsValue> {
        if connection.close_reason().is_none() || connection.stable_id() != self.stable_id {
            return Err(error("actual Terminal original stream required"));
        }
        let failed = self
            .send
            .write_all(b"old-terminal-stream-must-fail")
            .await
            .is_err();
        Ok((self.stream_id.clone(), failed))
    }

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
    cleanup: std::cell::Cell<bool>,
    cleanup_gate: BrowserGate,
    cleanup_stable: usize,
}
impl OwnedRtc {
    fn new(
        connection: &ControlHandle,
        transport: &crate::rtc_transport::RtcTransport,
        gate: &BrowserGate,
    ) -> Result<Self, JsValue> {
        let (revision, observed, updates) = gate.rtc_qualification(connection)?;
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
            association: Rc::new(Association::datagram(connection, transport.ice_profile())?),
            lease,
            attempt_live,
            drop_quic: Arc::new(AtomicBool::new(false)),
            stats: Arc::new(PumpStats::default()),
            cleanup: std::cell::Cell::new(false),
            cleanup_gate: gate.clone(),
            cleanup_stable: connection.stable_id(),
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
    fn start_pump(&self, original: iroh::endpoint::WeakControlConnection) {
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
        if !self.cleanup.replace(true) {
            let started = n0_future::time::Instant::now();
            let io = self.lease.io();
            let association = self.association.clone();
            let gate = self.cleanup_gate.clone();
            let stable = self.cleanup_stable;
            gate.0.lock().unwrap().rtc_cleanup_observers += 1;
            record_retired_rtc(&gate, stable, "immediate", started, &io, &association);
            spawn_local(async move {
                struct Observer(BrowserGate);
                impl Drop for Observer {
                    fn drop(&mut self) {
                        self.0.0.lock().unwrap().rtc_cleanup_observers -= 1;
                    }
                }
                let _observer = Observer(gate.clone());
                // Independent late observation, never a setup or path budget.
                n0_future::time::sleep(Duration::from_millis(100)).await;
                record_retired_rtc(&gate, stable, "late", started, &io, &association);
            });
        }
    }
}

fn record_retired_rtc(
    gate: &BrowserGate,
    stable: usize,
    phase: &str,
    started: n0_future::time::Instant,
    io: &crate::rtc_transport::RtcDatagramIo,
    association: &Association,
) {
    let property = |object: &JsValue, name: &str| {
        Reflect::get(object, &JsValue::from_str(name))
            .ok()
            .and_then(|value| value.as_string())
    };
    let pc_state = property(association.pc.as_ref(), "signalingState");
    let dc_state = property(association.dc.as_ref(), "readyState");
    gate.remember_rtc_cleanup(
        stable,
        json!({"phase":phase,
        "observedUtcMs":js_sys::Date::now(),"retirementElapsedMs":started.elapsed().as_millis(),
        "lease":io.report(),"pcClosed":pc_state.as_ref().map(|state|state=="closed"),
        "pcState":pc_state,"dataChannelState":dc_state}),
    );
}

impl Drop for OwnedRtc {
    fn drop(&mut self) {
        self.retire();
    }
}

async fn phase(exchange: &ControlExchange, expected: &str) -> Result<Value, JsValue> {
    let message = crate::rtc_wire::read_control(exchange)
        .await
        .map_err(error)?;
    if message["kind"] != expected {
        return Err(error(format!(
            "expected RTC {expected}, got {}",
            message["kind"]
        )));
    }
    Ok(message)
}

fn channel_metadata(association: &Association) -> Result<Value, JsValue> {
    let property = |object: &JsValue, name: &str| Reflect::get(object, &JsValue::from_str(name));
    let sctp = property(association.pc.as_ref(), "sctp")?;
    let dtls = property(&sctp, "transport")?;
    let ice = property(&dtls, "iceTransport")?;
    Ok(
        json!({"channel":{"id":association.dc.id(),"label":association.dc.label(),"protocol":property(association.dc.as_ref(),"protocol")?.as_string(),"ordered":property(association.dc.as_ref(),"ordered")?.as_bool(),
        "maxRetransmits":association.dc.max_retransmits(),"maxPacketLifeTime":association.dc.max_packet_life_time(),"negotiated":property(association.dc.as_ref(),"negotiated")?.as_bool()},
        "maxMessageSize":property(&sctp,"maxMessageSize")?.as_f64(),"iceState":property(&ice,"state")?.as_string(),"iceRole":property(&ice,"role")?.as_string(),
        "dtlsState":property(&dtls,"state")?.as_string(),"sctpState":property(&sctp,"state")?.as_string(),
        "iceServers":association.ice_profile.stun_servers,"turnServers":[]}),
    )
}

async fn metadata(association: &Association) -> Result<Value, JsValue> {
    let mut report = channel_metadata(association)?;
    report["stats"] = stats(&association.pc).await?;
    Ok(report)
}

pub(super) async fn bootstrap_control_unified(
    connection: &ControlHandle,
    gate: &BrowserGate,
    saved: &SavedNode,
    transport: &crate::rtc_transport::RtcTransport,
    attempt: crate::connection_coordinator::ProtectedAttempt,
) -> Result<(Value, crate::connection_coordinator::PreparedPath), JsValue> {
    if attempt.rtc_role == crate::connection_coordinator::RtcRole::None {
        return Err(error("unified/InvalidControl"));
    }
    let (report, prepared) =
        bootstrap_control_inner(connection, gate, saved, transport, attempt).await?;
    Ok((report, prepared))
}

/// Prepare one RTC candidate on the exact original M while using the
/// restricted active C for SDP and readiness.  No ordinary handle or control
/// path is released here; the common coordinator commits after bilateral
/// Ready and a final fresh ACK.
pub(super) async fn bootstrap_control_active(
    original: &ControlHandle,
    control: &ControlHandle,
    gate: &BrowserGate,
    saved: &SavedNode,
    transport: &crate::rtc_transport::RtcTransport,
    attempt: crate::connection_coordinator::ProtectedAttempt,
) -> Result<crate::connection_coordinator::PreparedPath, JsValue> {
    use crate::connection_coordinator::{PreparedPath, RtcRole, control_error};
    let offerer = attempt.rtc_role == RtcRole::Offerer;
    if !matches!(attempt.rtc_role, RtcRole::Offerer | RtcRole::Answerer) {
        return Err(error("unified/InvalidControl"));
    }
    let budget = RtcDeadline::from_caller(attempt_deadline(&attempt, control)?);
    let stable = original.stable_id();
    let original_binding = binding(original)?;
    let attempt_id = attempt.id_hex();
    let context = attempt_context(
        &attempt_id,
        saved.invitation()?.anchor.root.as_bytes(),
        saved.generation,
        &if offerer {
            original.local_id()
        } else {
            original.remote_id()
        }
        .to_string(),
        &if offerer {
            original.remote_id()
        } else {
            original.local_id()
        }
        .to_string(),
    );
    let mut exporter = [0; 32];
    let record_progress = |stage: &'static str, completed: bool| {
        let mut state = gate.0.lock().unwrap();
        if state.member_reports.len() >= 32 && !state.member_reports.contains_key(&stable) {
            return;
        }
        let report = state
            .member_reports
            .entry(stable)
            .or_insert_with(|| json!({}));
        report["activeRtcPrepareProgress"] = json!({
            "version": 1,
            "stage": stage,
            "completed": completed,
            "role": if offerer { "offerer" } else { "answerer" },
            "originalOpen": original.close_reason().is_none(),
            "controlOpen": control.close_reason().is_none(),
            "controlPeerMatchesOriginal": control.remote_id() == original.remote_id(),
        });
    };
    original
        .export_keying_material(&mut exporter, EXPORTER_LABEL, context.as_bytes())
        .map_err(|_| error("original M exporter unavailable"))?;
    let protected_binding = blake3::hash(&exporter).to_hex().to_string();
    record_progress("control-stream-open", false);
    let exchange = if control.side().is_client() {
        control.open_control_exchange().await.map_err(error)?
    } else {
        control.accept_control_exchange().await.map_err(error)?
    };
    record_progress("control-stream-open", true);
    let mut request = if offerer {
        json!({"protocol":crate::rtc_association::UNIFIED_RTC_PROTOCOL,"attempt":attempt_id,
            "context":context,"binding":protected_binding,"channel":datagram_channel_parameters(),
            "quicDialer":attempt.quic_dialer_for(original.local_id(),original).to_string(),
            "rtcOfferer":original.local_id().to_string(),"rtcAnswerer":original.remote_id().to_string()})
    } else {
        record_progress("answerer-read-offer", false);
        let request = crate::rtc_wire::read_control(&exchange)
            .await
            .map_err(error)?;
        record_progress("answerer-read-offer", true);
        record_progress("answerer-validate-offer", false);
        if request["protocol"] != crate::rtc_association::UNIFIED_RTC_PROTOCOL
            || request["attempt"] != attempt_id
            || request["context"] != context
            || request["binding"] != protected_binding
            || request["channel"] != datagram_channel_parameters()
            || request["rtcOfferer"] != original.remote_id().to_string()
            || request["rtcAnswerer"] != original.local_id().to_string()
            || request["description"]["type"] != "offer"
        {
            return Err(error("unified/InvalidControl"));
        }
        transport
            .ice_profile()
            .check_remote_sdp(
                request["description"]["sdp"]
                    .as_str()
                    .ok_or_else(|| error("unified/InvalidControl"))?,
            )
            .map_err(error)?;
        record_progress("answerer-validate-offer", true);
        request
    };
    record_progress("rtc-owner-create", false);
    let (revision, observed, _) = gate.rtc_qualification(original)?;
    let owner = OwnedRtc::new(original, transport, gate)?;
    record_progress("rtc-owner-create", true);
    let preparation_io = owner.lease.io();
    let metadata = preparation_io.while_current(async {
        if !offerer {
            record_progress("answerer-set-remote-route", false);
            owner.lease.set_remote(iroh_base::CustomAddr::from_parts(
                crate::rtc_transport::TRANSPORT_ID,
                &hex::decode(request["route"].as_str().ok_or_else(|| error("unified/InvalidControl"))?).map_err(error)?,
            )).map_err(error)?;
            record_progress("answerer-set-remote-route", true);
        }
        if offerer {
            record_progress("offerer-signaling", false);
            let offer: RtcSessionDescriptionInit = JsFuture::from(owner.association.pc.create_offer())
                .await?
                .unchecked_into();
            JsFuture::from(owner.association.pc.set_local_description(&offer)).await?;
            owner.association.gathered().await?;
            let local = owner.association.pc.local_description().ok_or_else(|| error("RTC offer absent"))?;
            local_sdp(&mut request, &local.sdp(), transport)?;
            request["route"] = json!(hex::encode(owner.lease.local_addr().data()));
            request["remainingMs"] = json!(budget.remaining().map_err(error)?.as_millis() as u64);
            crate::rtc_wire::write_control(&exchange, &request).await.map_err(error)?;
            let answer = phase(&exchange, "answer").await?;
            if answer["attempt"] != attempt_id
                || answer["binding"] != protected_binding
                || answer["channel"] != datagram_channel_parameters()
                || answer["description"]["type"] != "answer"
            {
                return Err(error("unified/InvalidControl"));
            }
            record_progress("offerer-validate-answer", false);
            owner.lease.set_remote(iroh_base::CustomAddr::from_parts(
                crate::rtc_transport::TRANSPORT_ID,
                &hex::decode(answer["route"].as_str().ok_or_else(|| error("unified/InvalidControl"))?).map_err(error)?,
            )).map_err(error)?;
            let remote_sdp = answer["description"]["sdp"].as_str().ok_or_else(|| error("unified/InvalidControl"))?;
            transport.ice_profile().check_remote_sdp(remote_sdp).map_err(error)?;
            let init = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
            init.set_sdp(remote_sdp);
            JsFuture::from(owner.association.pc.set_remote_description(&init)).await?;
            record_progress("offerer-validate-answer", true);
            record_progress("offerer-signaling", true);
        } else {
            record_progress("answerer-signaling", false);
            let offer = RtcSessionDescriptionInit::new(RtcSdpType::Offer);
            offer.set_sdp(request["description"]["sdp"].as_str().ok_or_else(|| error("unified/InvalidControl"))?);
            JsFuture::from(owner.association.pc.set_remote_description(&offer)).await?;
            let answer: RtcSessionDescriptionInit = JsFuture::from(owner.association.pc.create_answer()).await?.unchecked_into();
            JsFuture::from(owner.association.pc.set_local_description(&answer)).await?;
            owner.association.gathered().await?;
            let local = owner.association.pc.local_description().ok_or_else(|| error("RTC answer absent"))?;
            crate::rtc_wire::write_control(&exchange, &json!({"kind":"answer","attempt":attempt_id,"binding":protected_binding,
                "channel":datagram_channel_parameters(),"description":{"type":"answer","sdp":transport.ice_profile().local_sdp(&local.sdp()).map_err(error)?},
                "route":hex::encode(owner.lease.local_addr().data())})).await.map_err(error)?;
            record_progress("answerer-signaling", true);
        }
        record_progress("data-channel-open", false);
        owner.association.wait_open().await?;
        record_progress("data-channel-open", true);
        record_progress("data-channel-metadata", false);
        let metadata = channel_metadata(&owner.association)?;
        if metadata["channel"] != datagram_channel_parameters()
            || metadata["maxMessageSize"].as_f64().is_none_or(|size| size < MAX_DATAGRAM as f64)
        {
            return Err(error("unified/InvalidControl"));
        }
        record_progress("data-channel-metadata", true);
        record_progress("lease-activate", false);
        owner.lease.activate().map_err(error)?;
        record_progress("lease-activate", true);
        Ok::<_, JsValue>(metadata)
    }).await.map_err(error)??;
    // Only the pump owns retirement observation after activation.
    owner.start_pump(original.weak_handle());
    record_progress("pump-start", true);
    record_progress("remote-route", false);
    let remote = iroh::TransportAddr::Custom(owner.lease.remote_addr().map_err(error)?);
    record_progress("remote-route", true);
    if original.side().is_client() {
        record_progress("data-path-open", false);
        loop {
            budget.remaining().map_err(error)?;
            match original.open_data_transport_path(remote.clone()).await {
                Ok(()) => break,
                Err(cause) if cause.kind() == std::io::ErrorKind::WouldBlock => {
                    n0_future::time::sleep(Duration::from_millis(10)).await
                }
                Err(cause) => return Err(error(cause)),
            }
        }
        record_progress("data-path-open", true);
    }
    record_progress("data-path-present", false);
    loop {
        budget.remaining().map_err(error)?;
        if original
            .paths()
            .iter()
            .any(|path| path.remote_addr() == &remote)
        {
            break;
        }
        n0_future::time::sleep(Duration::from_millis(10)).await;
    }
    record_progress("data-path-present", true);
    record_progress("probe-start", false);
    let probe_started_ms = budget.elapsed_ms();
    let probe = loop {
        budget.remaining().map_err(error)?;
        match owner.lease.start_probe(original) {
            Ok(probe) => break probe,
            Err(cause) if cause.kind() == std::io::ErrorKind::WouldBlock => {
                n0_future::time::sleep(Duration::from_millis(10)).await
            }
            Err(cause) => return Err(error(cause)),
        }
    };
    record_progress("probe-start", true);
    record_progress("probe-ack", false);
    while !probe.is_acknowledged().map_err(error)? {
        budget.remaining().map_err(error)?;
        if revision.load(Ordering::Acquire) != observed || !owner.lease.io().is_active() {
            return Err(error("unified/IdentityOrAdmissionRejected"));
        }
        n0_future::time::sleep(Duration::from_millis(10)).await;
    }
    let ack_ms = budget.elapsed_ms();
    record_progress("probe-ack", true);
    record_progress("qualification", false);
    gate.qualified_snapshot(original).await?;
    record_progress("qualification", true);
    if revision.load(Ordering::Acquire) != observed
        || !owner.lease.io().is_active()
        || binding(original)? != original_binding
    {
        return Err(error("unified/IdentityOrAdmissionRejected"));
    }
    let paths = original.paths();
    let path = paths
        .iter()
        .find(|path| path.id().to_string() == probe.path_id())
        .ok_or_else(|| error("unified/DataPathUnavailable"))?;
    let before_retain_frames = json!({"streamSent":path.stats().frame_tx.stream,
        "streamReceived":path.stats().frame_rx.stream,"datagramSent":path.stats().frame_tx.datagram,
        "datagramReceived":path.stats().frame_rx.datagram});
    record_progress("stream-finish", false);
    exchange.finish_send().await.map_err(error)?;
    record_progress("stream-finish", true);
    let initial_path = probe.path_id().to_owned();
    let initial_watermark = probe.watermark();
    let io = owner.lease.io();
    let rollback_io = io.clone();
    let current_io = io.clone();
    let pending_original = original.clone();
    let gate_for_retain = gate.clone();
    let report = json!({"prepared":true,"finalized":false,"switched":false,
        "memberStableId":stable,"stableUnchanged":original.stable_id()==stable,
        "bindingUnchanged":binding(original)?==original_binding,"freshAck":true,
        "initialFreshAck":true,"initialProbeWatermark":initial_watermark,
        "probeWatermark":initial_watermark,"pathId":initial_path,
        "freshProbeStartedMs":probe_started_ms,"freshAckMs":ack_ms,
        "beforeRetainFrames":before_retain_frames,"finalReleaseFreshAck":false,
        "channel":metadata,"authorizationRevision":observed,"secretGeneration":saved.generation,
        "resourceRevision":transport.resource_revision(),"route":owner.lease.report(),
        "dataChannel":owner.stats.report()});
    let pending = send_wrapper::SendWrapper::new((owner, report));
    record_progress("prepared-path-return", false);
    let prepared = PreparedPath::candidate(original, remote, move || {
        if current_io.is_active() {
            Ok(())
        } else {
            Err(control_error("DataPathUnavailable"))
        }
    })
    .with_proof(initial_path, initial_watermark, move || {
        probe.is_acknowledged()
    })
    .with_rollback(move || rollback_io.retire())
    .with_retain(move || {
        Box::pin(send_wrapper::SendWrapper::new(async move {
            let (owner, mut report) = pending.take();
            let current = pending_original;
            if current.close_reason().is_some() {
                return Err(control_error("DataPathUnavailable"));
            }
            gate_for_retain
                .qualified_snapshot(&current)
                .await
                .map_err(|_| control_error("IdentityOrAdmissionRejected"))?;
            if revision.load(Ordering::Acquire) != observed || !owner.lease.io().is_active() {
                return Err(control_error("IdentityOrAdmissionRejected"));
            }
            let started = budget.elapsed_ms();
            let final_probe = owner.lease.validate_release(&current, budget).await?;
            gate_for_retain
                .qualified_snapshot(&current)
                .await
                .map_err(|_| control_error("IdentityOrAdmissionRejected"))?;
            if revision.load(Ordering::Acquire) != observed
                || !owner.lease.io().is_active()
                || current.stable_id() != stable
                || binding(&current).map_err(|_| control_error("IdentityOrAdmissionRejected"))?
                    != original_binding
            {
                return Err(control_error("IdentityOrAdmissionRejected"));
            }
            owner.lease.retain(&current, &final_probe)?;
            current
                .retain_data_transport_path(owner.lease.remote_addr()?)
                .await?;
            report["prepared"] = json!(false);
            report["finalized"] = json!(true);
            report["stableUnchanged"] = json!(current.stable_id() == stable);
            report["bindingUnchanged"] = json!(
                binding(&current).map_err(|_| control_error("IdentityOrAdmissionRejected"))?
                    == original_binding
            );
            report["finalReleaseFreshAck"] = json!(final_probe.is_acknowledged()?);
            report["finalReleasePathId"] = json!(final_probe.path_id());
            report["finalReleaseProbeWatermark"] = json!(final_probe.watermark());
            report["finalReleaseProbeStartedMs"] = json!(started);
            report["finalReleaseAckMs"] = json!(budget.elapsed_ms());
            report["route"] = json!(owner.lease.report());
            report["dataChannel"] = owner.stats.report();
            gate_for_retain.remember_bootstrap(&current, report);
            let io = owner.lease.io();
            let closed = current.weak_handle();
            spawn_local(async move {
                let retired = io
                    .while_active(futures_util::future::pending::<()>())
                    .fuse();
                let closed_future = closed.closed().fuse();
                futures_util::pin_mut!(retired, closed_future);
                futures_util::select_biased! { _ = closed_future => {}, _ = retired => {} }
                owner.retire();
            });
            Ok::<_, std::io::Error>(())
        }))
    });
    record_progress("prepared-path-return", true);
    Ok(prepared)
}

fn attempt_deadline(
    attempt: &crate::connection_coordinator::ProtectedAttempt,
    control: &ControlHandle,
) -> Result<n0_future::time::Instant, JsValue> {
    let _ = attempt;
    Ok(control
        .caller_deadline()
        .unwrap_or_else(|| n0_future::time::Instant::now() + crate::connection_budget::DEFAULT))
}

fn local_sdp(
    request: &mut Value,
    sdp: &str,
    transport: &crate::rtc_transport::RtcTransport,
) -> Result<(), JsValue> {
    request["description"] =
        json!({"type":"offer","sdp":transport.ice_profile().local_sdp(sdp).map_err(error)?});
    Ok(())
}

fn report_stub(stable: usize, metadata: &Value) -> Value {
    json!({"memberStableId":stable,"channel":metadata,"active":true})
}

pub(super) async fn bootstrap_control_inner(
    connection: &ControlHandle,
    gate: &BrowserGate,
    saved: &SavedNode,
    transport: &crate::rtc_transport::RtcTransport,
    unified: crate::connection_coordinator::ProtectedAttempt,
) -> Result<(Value, crate::connection_coordinator::PreparedPath), JsValue> {
    let offerer = unified.rtc_role == crate::connection_coordinator::RtcRole::Offerer;
    let caller_deadline = connection
        .caller_deadline()
        .ok_or_else(|| error("RTC caller deadline absent"))?;
    let mut budget = RtcDeadline::from_caller(
        connection
            .caller_deadline()
            .ok_or_else(|| error("RTC caller deadline absent"))?,
    );
    let stable = connection.stable_id();
    let original_binding = binding(connection)?;
    let attempt = unified.id_hex();
    let protocol = crate::rtc_association::UNIFIED_RTC_PROTOCOL;
    let quic_dialer = unified.quic_dialer_for(connection.local_id(), connection);
    let mut initial_unified_probe = None;
    let context = attempt_context(
        &attempt,
        saved.invitation()?.anchor.root.as_bytes(),
        saved.generation,
        &if offerer {
            connection.local_id()
        } else {
            connection.remote_id()
        }
        .to_string(),
        &if offerer {
            connection.remote_id()
        } else {
            connection.local_id()
        }
        .to_string(),
    );
    let mut exporter = [0; 32];
    connection
        .export_keying_material(&mut exporter, EXPORTER_LABEL, context.as_bytes())
        .map_err(|_| error("RTC original M exporter unavailable"))?;
    let protected_binding = blake3::hash(&exporter).to_hex().to_string();
    let exchange = if offerer {
        connection.open_control_exchange().await.map_err(error)?
    } else {
        connection.accept_control_exchange().await.map_err(error)?
    };
    let request = if offerer {
        None
    } else {
        let request = crate::rtc_wire::read_control(&exchange)
            .await
            .map_err(error)?;
        let remaining = request["remainingMs"]
            .as_u64()
            .filter(|remaining| *remaining > 0 && *remaining <= crate::connection_budget::MAX_MS)
            .ok_or_else(|| error("unified/InvalidControl"))?;
        if request["protocol"] != protocol
            || request["attempt"] != attempt
            || request["context"] != context
            || request["binding"] != protected_binding
            || request["channel"] != datagram_channel_parameters()
            || request["quicDialer"] != quic_dialer.to_string()
            || request["rtcOfferer"] != connection.remote_id().to_string()
            || request["rtcAnswerer"] != connection.local_id().to_string()
            || request["description"]["type"] != "offer"
        {
            return Err(error("unified/InvalidControl"));
        }
        let sdp = request["description"]["sdp"]
            .as_str()
            .ok_or_else(|| error("unified/InvalidControl"))?;
        transport
            .ice_profile()
            .check_remote_sdp(sdp)
            .map_err(error)?;
        budget = RtcDeadline::from_caller(
            caller_deadline.min(n0_future::time::Instant::now() + Duration::from_millis(remaining)),
        );
        Some(request)
    };
    let owner = OwnedRtc::new(connection, transport, gate)?;
    let stream_id = exchange.id().to_string();
    let (revision, observed, _) = gate.rtc_qualification(connection)?;

    let preparation_io = owner.lease.io();
    let work = async {
        let mut opening = preparation_io.while_current(async {
            if offerer {
                let offer: RtcSessionDescriptionInit = JsFuture::from(owner.association.pc.create_offer()).await?.unchecked_into();
                JsFuture::from(owner.association.pc.set_local_description(&offer)).await?;
                owner.association.gathered().await?;
                let offer = owner.association.pc.local_description().ok_or_else(|| error("RTC offer absent"))?;
                let local_sdp=offer.sdp();

                let published_sdp=transport.ice_profile().local_sdp(&local_sdp).map_err(error)?;

                crate::rtc_wire::write_control(&exchange, &json!({"protocol":protocol,"attempt":attempt,"context":context,"binding":protected_binding,
                    "description":{"type":"offer","sdp":published_sdp},"channel":datagram_channel_parameters(),"route":hex::encode(owner.lease.local_addr().data()),
                    "quicDialer":quic_dialer.to_string(),"rtcOfferer":connection.local_id().to_string(),"rtcAnswerer":connection.remote_id().to_string(),
                    "remainingMs":budget.remaining().map_err(error)?.as_millis() as u64,"authorizationRevision":observed,"resourceRevision":transport.resource_revision()})).await.map_err(error)?;
                let answer = phase(&exchange, "answer").await?;
                if answer["attempt"] != attempt || answer["binding"] != protected_binding || answer["channel"] != datagram_channel_parameters()
                    || answer["description"]["type"] != "answer" {
                    return Err(error("unified/InvalidControl"));
                }
                owner.lease.set_remote(iroh_base::CustomAddr::from_parts(crate::rtc_transport::TRANSPORT_ID,
                    &hex::decode(answer["route"].as_str().ok_or_else(|| error("RTC peer route absent"))?).map_err(error)?)).map_err(error)?;
                let init = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
                let remote_sdp = answer["description"]["sdp"].as_str().ok_or_else(|| error("RTC peer answer absent"))?;
                transport.ice_profile().check_remote_sdp(remote_sdp).map_err(error)?;

                init.set_sdp(remote_sdp);
                JsFuture::from(owner.association.pc.set_remote_description(&init)).await?;
            } else {
                let request = request.as_ref().ok_or_else(|| error("unified/InvalidControl"))?;
                owner.lease.set_remote(iroh_base::CustomAddr::from_parts(crate::rtc_transport::TRANSPORT_ID,
                    &hex::decode(request["route"].as_str().ok_or_else(|| error("RTC peer route absent"))?).map_err(error)?)).map_err(error)?;
                let offer = RtcSessionDescriptionInit::new(RtcSdpType::Offer);
                offer.set_sdp(request["description"]["sdp"].as_str().ok_or_else(|| error("RTC peer offer absent"))?);

                JsFuture::from(owner.association.pc.set_remote_description(&offer)).await?;

                let answer: RtcSessionDescriptionInit = JsFuture::from(owner.association.pc.create_answer()).await?.unchecked_into();

                JsFuture::from(owner.association.pc.set_local_description(&answer)).await?;

                owner.association.gathered().await?;

                let answer = owner.association.pc.local_description().ok_or_else(|| error("RTC browser answer absent"))?;
                let local_sdp=answer.sdp();

                let published_sdp=transport.ice_profile().local_sdp(&local_sdp).map_err(error)?;

                crate::rtc_wire::write_control(&exchange, &json!({"kind":"answer","attempt":attempt,"binding":protected_binding,
                    "channel":datagram_channel_parameters(),"description":{"type":"answer","sdp":published_sdp},
                    "route":hex::encode(owner.lease.local_addr().data()),"authorizationRevision":observed,"resourceRevision":transport.resource_revision()})).await.map_err(error)?;
            }

            owner.association.wait_open().await?;

            // Validate and activate in the same turn as the open wakeup. Awaiting
            // getStats here lets a peer's first QUIC datagram reach an inactive
            // receive lease and retire an otherwise valid, open association.
            let opening = channel_metadata(&owner.association)?;
            if opening["channel"] != datagram_channel_parameters()
                || opening["maxMessageSize"].as_f64().is_none_or(|maximum| maximum < MAX_DATAGRAM as f64)
            { return Err(error("RTC negotiated profile/maximum mismatch")); }

            owner.lease.activate().map_err(error)?;
            Ok::<_, JsValue>(opening)
        }).await.map_err(error)??;
        owner.start_pump(connection.weak_handle());

        opening["stats"] = stats(&owner.association.pc).await?;

        let actual = owner.stats.clone();
        transport.control_fixture_checkpoint(crate::rtc_invalidation_fixture::ControlCheckpointStage::ChannelReady,
            connection, &owner.lease, budget,
            json!({"attempt":attempt,"stableId":stable,"localRoute":hex::encode(owner.lease.local_addr().data()),
                "remoteRoute":hex::encode(owner.lease.remote_addr().map_err(error)?.data()),"freshAck":false,
                "authorizationRevision":observed,"secretGeneration":saved.generation,"resourceRevision":transport.resource_revision()}),
            Arc::new(move || actual.report())).await.map_err(error)?;

            if connection.side().is_client() {

                loop {
                    budget.remaining().map_err(error)?;
                    let result=connection.open_control_path(owner.lease.remote_addr().map_err(error)?).await;

                    match result {
                        Ok(())=>break,
                        Err(cause) if cause.kind()==std::io::ErrorKind::WouldBlock=>n0_future::time::sleep(Duration::from_millis(10)).await,
                        Err(cause)=>return Err(error(cause)),
                    }
                }

            }

            let probe=loop {
                budget.remaining().map_err(error)?;
                match owner.lease.start_probe(connection) {
                    Ok(probe)=>break probe,
                    Err(cause) if cause.kind()==std::io::ErrorKind::WouldBlock=>n0_future::time::sleep(Duration::from_millis(10)).await,
                    Err(cause)=>return Err(error(cause)),
                }
            };
            let started=budget.elapsed_ms();
            while !probe.is_acknowledged().map_err(error)? {
                budget.remaining().map_err(error)?;
                n0_future::time::sleep(Duration::from_millis(10)).await;
            }

            gate.qualified_snapshot(connection).await?;
            if revision.load(Ordering::Acquire)!=observed || !owner.lease.io().is_active() {
                return Err(error("RTC initial qualification changed before control seal"));
            }
            let paths=connection.paths();
            let path=paths.iter().find(|path|path.id().to_string()==probe.path_id()).ok_or_else(||error("RTC initial exact path disappeared"))?;
            let frames=json!({"streamSent":path.stats().frame_tx.stream,"streamReceived":path.stats().frame_rx.stream,
                "datagramSent":path.stats().frame_tx.datagram,"datagramReceived":path.stats().frame_rx.datagram});
            owner.lease.retain(connection,&probe).map_err(error)?;

            connection.retain_control_data_transport_path(owner.lease.remote_addr().map_err(error)?).await.map_err(error)?;

            exchange.finish_send().await.map_err(error)?;
            let report=json!({"prepared":true,"finalized":false,"switched":false,
                "memberStableId":stable,"stableUnchanged":connection.stable_id()==stable,
                "bindingUnchanged":binding(connection)?==original_binding,"freshAck":true,
                "initialFreshAck":true,"initialProbeWatermark":probe.watermark(),"probeWatermark":probe.watermark(),
                "pathId":probe.path_id(),"freshProbeStartedMs":started,"freshAckMs":budget.elapsed_ms(),
                "beforeRetainFrames":frames,"finalReleaseFreshAck":false,"privateStreamId":stream_id,
                "rtc":opening,"authorizationRevision":observed,"secretGeneration":saved.generation,
                "resourceRevision":transport.resource_revision(),"quicRole":if connection.side().is_client(){"client"}else{"server"},
                "rtcRole":if offerer{"offerer"}else{"answerer"},"route":owner.lease.report(),"dataChannel":owner.stats.report()});
            initial_unified_probe=Some(probe);
            return Ok(report);
            }.fuse();
    let closed = connection.closed().fuse();
    let outcome = {
        futures_util::pin_mut!(work, closed);
        n0_future::time::timeout(budget.remaining().map_err(error)?, async {
            futures_util::select_biased! { _ = closed => Err(error("RTC original M closed")), result = work => result }
        }).await.map_err(|_| error("RTC original caller deadline exceeded")).and_then(|result|result)
    };

    let report = outcome?;

    use crate::connection_coordinator::{PreparedPath, control_error};
    let probe = Arc::new(
        initial_unified_probe
            .take()
            .ok_or_else(|| error("RTC initial proof absent"))?,
    );
    let initial_path = probe.path_id().to_owned();
    let initial_watermark = probe.watermark();
    let final_proof: Arc<std::sync::Mutex<Option<Arc<crate::rtc_transport::RtcPathProbe>>>> =
        Arc::default();
    let checked_final = final_proof.clone();
    let io = owner.lease.io();
    let rollback_io = io.clone();
    let remote = owner.lease.remote_addr().map_err(error)?;
    let pending_original = connection.clone();
    let original = connection.weak_handle();
    let generation = saved.generation;
    // WebSys/Rc ownership stays on the same browser executor, following the
    // adapter's existing SendWrapper convention. No lifetime M clone.
    let pending =
        send_wrapper::SendWrapper::new((owner, gate.clone(), transport.clone(), report.clone()));
    let prepared = PreparedPath::candidate(connection,iroh::TransportAddr::Custom(remote),move || {
            if !io.is_active() { return Err(control_error("DataPathUnavailable")); }
            if let Some(proof)=checked_final.lock().unwrap().as_ref() && !proof.is_acknowledged()? {
                return Err(control_error("DataPathUnavailable"));
            }
            Ok(())
        })
        .with_proof(initial_path,initial_watermark,move || probe.is_acknowledged())
        .with_rollback(move || rollback_io.retire())
        .with_retain(move || Box::pin(send_wrapper::SendWrapper::new(async move {
            let (owner,gate,transport,mut report)=pending.take();
            let current=pending_original;
            if current.close_reason().is_some() { return Err(control_error("DataPathUnavailable")); }

            let finalize=async {

                let actual=owner.stats.clone();
                transport.control_fixture_checkpoint(crate::rtc_invalidation_fixture::ControlCheckpointStage::BeforeRelease,
                    &current,&owner.lease,budget,
                    json!({"attempt":attempt,"stableId":stable,"localRoute":hex::encode(owner.lease.local_addr().data()),
                        "remoteRoute":hex::encode(owner.lease.remote_addr()?.data()),"freshAck":true,"bothConfirmed":true,
                        "pathId":report["pathId"],"probeWatermark":initial_watermark,
                        "freshProbeStartedMs":report["freshProbeStartedMs"],"freshAckMs":report["freshAckMs"],
                        "authorizationRevision":observed,"secretGeneration":generation,"resourceRevision":transport.resource_revision()}),
                    Arc::new(move || actual.report())).await?;

                gate.qualified_snapshot(&current).await.map_err(|_|control_error("IdentityOrAdmissionRejected"))?;
                if revision.load(Ordering::Acquire)!=observed || !owner.lease.io().is_active() {
                    return Err(control_error("IdentityOrAdmissionRejected"));
                }

                let started=budget.elapsed_ms();
                let final_ack=Arc::new(owner.lease.validate_release(&current,budget).await?);
                gate.qualified_snapshot(&current).await.map_err(|_|control_error("IdentityOrAdmissionRejected"))?;
                if revision.load(Ordering::Acquire)!=observed || binding(&current).map_err(|_|control_error("IdentityOrAdmissionRejected"))?!=original_binding {
                    return Err(control_error("IdentityOrAdmissionRejected"));
                }
                owner.lease.retain(&current,&final_ack)?;
                report["prepared"]=json!(false);
                report["finalized"]=json!(true);
                report["bothConfirmed"]=json!(true);
                report["finalReleaseFreshAck"]=json!(true);
                report["finalReleaseProbeWatermark"]=json!(final_ack.watermark());
                report["finalReleaseProbeStartedMs"]=json!(started);
                report["finalReleaseAckMs"]=json!(budget.elapsed_ms());
                report["route"]=json!(owner.lease.report());
                report["dataChannel"]=owner.stats.report();
                *final_proof.lock().unwrap()=Some(final_ack);

                Ok::<_,std::io::Error>(())
            }.await;

            finalize?;
            gate.remember_bootstrap(&current,report);
            drop(current);
            let io=owner.lease.io();
            spawn_local(async move {
                let retired=io.while_active(futures_util::future::pending::<()>()).fuse();
                let closed=original.closed().fuse();
                futures_util::pin_mut!(retired,closed);
                futures_util::select_biased!{ _=closed=>{}, _=retired=>{} }
                owner.retire();
            });
            Ok(())
        })));
    Ok((report, prepared))
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
            "cleanupObservers":gate.rtc_cleanup_observers,"retiredMemberReports":gate.member_reports,
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
            "bootstrap":self.gate.0.lock().unwrap().member_reports.get(&self.connection.stable_id()),
            "paths":self.connection.paths().iter().map(|path|json!({"pathId":path.id().to_string(),"address":format!("{:?}",path.remote_addr()),"selected":path.is_selected(),"isRelay":path.is_relay(),"isRtc":matches!(path.remote_addr(),iroh::TransportAddr::Custom(address) if address.id()==crate::rtc_transport::TRANSPORT_ID),
                "isSignal":matches!(path.remote_addr(),iroh::TransportAddr::Custom(address) if matches!(address.id(),crate::mqtt_datagram::TRANSPORT_ID | crate::irc_datagram::TRANSPORT_ID)),
                "sentBytes":path.stats().udp_tx.bytes,"receivedBytes":path.stats().udp_rx.bytes,"streamFramesSent":path.stats().frame_tx.stream,"streamFramesReceived":path.stats().frame_rx.stream,
                "pingFramesSent":path.stats().frame_tx.ping,"ackFramesReceived":path.stats().frame_rx.acks,"ackFramesSent":path.stats().frame_tx.acks})).collect::<Vec<_>>()}).to_string()
    }
}
