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
    cleanup: std::cell::Cell<bool>,
    cleanup_gate: BrowserGate,
    cleanup_stable: usize,
}
impl OwnedRtc {
    fn new(
        connection: &Connection,
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

/// The admission hook owns this setup, independent of a business or replica
/// handle. Its successful holder contains no Endpoint, strong M or setup clock.
pub(super) async fn bootstrap_control(
    endpoint: &Endpoint,
    connection: &Connection,
    gate: &BrowserGate,
    saved: &SavedNode,
    transport: &crate::rtc_transport::RtcTransport,
) -> Result<Value, JsValue> {
    bootstrap_control_inner(endpoint, connection, gate, saved, transport, None)
        .await
        .map(|(report, _)| report)
}

pub(super) async fn bootstrap_control_unified(
    endpoint: &Endpoint,
    connection: &Connection,
    gate: &BrowserGate,
    saved: &SavedNode,
    transport: &crate::rtc_transport::RtcTransport,
    attempt: crate::connection_coordinator::ProtectedAttempt,
) -> Result<(Value, crate::connection_coordinator::PreparedPath), JsValue> {
    if attempt.rtc_role == crate::connection_coordinator::RtcRole::None {
        return Err(error("unified/InvalidControl"));
    }
    let (report, prepared) =
        bootstrap_control_inner(endpoint, connection, gate, saved, transport, Some(attempt))
            .await?;
    Ok((
        report,
        prepared.ok_or_else(|| error("unified/DataPathUnavailable"))?,
    ))
}

pub(super) async fn bootstrap_control_inner(
    endpoint: &Endpoint,
    connection: &Connection,
    gate: &BrowserGate,
    saved: &SavedNode,
    transport: &crate::rtc_transport::RtcTransport,
    unified: Option<crate::connection_coordinator::ProtectedAttempt>,
) -> Result<(Value, Option<crate::connection_coordinator::PreparedPath>), JsValue> {
    let offerer = unified
        .is_none_or(|attempt| attempt.rtc_role == crate::connection_coordinator::RtcRole::Offerer);
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
    let attempt = unified.map_or_else(
        || hex::encode(rand::random::<[u8; 16]>()),
        |attempt| attempt.id_hex(),
    );
    let protocol = if unified.is_some() {
        crate::rtc_association::UNIFIED_RTC_PROTOCOL
    } else {
        MEMBER_RTC_PROTOCOL
    };
    let quic_dialer = unified.map_or(endpoint.id(), |attempt| {
        attempt.quic_dialer(endpoint, connection)
    });
    let mut prepared_for_coordinator = None;
    let mut initial_unified_probe = None;
    let context = attempt_context(
        &attempt,
        saved.invitation()?.anchor.root.as_bytes(),
        saved.generation,
        &if offerer {
            endpoint.id()
        } else {
            connection.remote_id()
        }
        .to_string(),
        &if offerer {
            connection.remote_id()
        } else {
            endpoint.id()
        }
        .to_string(),
    );
    let mut exporter = [0; 32];
    connection
        .export_keying_material(&mut exporter, EXPORTER_LABEL, context.as_bytes())
        .map_err(|_| error("RTC original M exporter unavailable"))?;
    let protected_binding = blake3::hash(&exporter).to_hex().to_string();
    let (mut send, mut recv) = if offerer {
        connection.open_bi().await.map_err(error)?
    } else {
        connection.accept_bi().await.map_err(error)?
    };
    let request = if offerer {
        None
    } else {
        let request = read_frame(&mut recv).await.map_err(error)?;
        let remaining = request["remainingMs"]
            .as_u64()
            .filter(|remaining| *remaining > 0 && *remaining <= 12000)
            .ok_or_else(|| error("unified/InvalidControl"))?;
        if request["protocol"] != protocol
            || request["attempt"] != attempt
            || request["context"] != context
            || request["binding"] != protected_binding
            || request["channel"] != datagram_channel_parameters()
            || request["quicDialer"] != quic_dialer.to_string()
            || request["rtcOfferer"] != connection.remote_id().to_string()
            || request["rtcAnswerer"] != endpoint.id().to_string()
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
    let stream_id = send.id().to_string();
    let (revision, observed, _) = gate.rtc_qualification(connection)?;

    let work = async {
        if offerer {
            let offer: RtcSessionDescriptionInit = JsFuture::from(owner.association.pc.create_offer()).await?.unchecked_into();
            JsFuture::from(owner.association.pc.set_local_description(&offer)).await?;
            owner.association.gathered().await?;
            let offer = owner.association.pc.local_description().ok_or_else(|| error("RTC offer absent"))?;
            let local_sdp=offer.sdp();

            let published_sdp=transport.ice_profile().local_sdp(&local_sdp).map_err(error)?;

            write_frame(&mut send, &json!({"protocol":protocol,"attempt":attempt,"context":context,"binding":protected_binding,
                "description":{"type":"offer","sdp":published_sdp},"channel":datagram_channel_parameters(),"route":hex::encode(owner.lease.local_addr().data()),
                "quicDialer":quic_dialer.to_string(),"rtcOfferer":endpoint.id().to_string(),"rtcAnswerer":connection.remote_id().to_string(),
                "remainingMs":budget.remaining().map_err(error)?.as_millis() as u64,"authorizationRevision":observed,"resourceRevision":transport.resource_revision()})).await.map_err(error)?;
            let answer = phase(&mut recv, "answer").await?;
            if answer["attempt"] != attempt || answer["binding"] != protected_binding || answer["channel"] != datagram_channel_parameters()
                || answer["description"]["type"] != "answer" {
                return Err(error(if unified.is_some() { "unified/InvalidControl" } else { "RTC answer context/profile binding mismatch" }));
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

            write_frame(&mut send, &json!({"kind":"answer","attempt":attempt,"binding":protected_binding,
                "channel":datagram_channel_parameters(),"description":{"type":"answer","sdp":published_sdp},
                "route":hex::encode(owner.lease.local_addr().data()),"authorizationRevision":observed,"resourceRevision":transport.resource_revision()})).await.map_err(error)?;
        }

        owner.association.wait_open().await?;

        // Validate and activate in the same turn as the open wakeup. Awaiting
        // getStats here lets a peer's first QUIC datagram reach an inactive
        // receive lease and retire an otherwise valid, open association.
        let mut opening = channel_metadata(&owner.association)?;
        if opening["channel"] != datagram_channel_parameters()
            || opening["maxMessageSize"].as_f64().is_none_or(|maximum| maximum < MAX_DATAGRAM as f64)
        { return Err(error("RTC negotiated profile/maximum mismatch")); }

        owner.lease.activate().map_err(error)?;
        owner.start_pump(connection.weak_handle());

        opening["stats"] = stats(&owner.association.pc).await?;

        let actual = owner.stats.clone();
        transport.control_fixture_checkpoint(crate::rtc_invalidation_fixture::ControlCheckpointStage::ChannelReady,
            connection, &owner.lease, budget,
            json!({"attempt":attempt,"stableId":stable,"localRoute":hex::encode(owner.lease.local_addr().data()),
                "remoteRoute":hex::encode(owner.lease.remote_addr().map_err(error)?.data()),"freshAck":false,
                "authorizationRevision":observed,"secretGeneration":saved.generation,"resourceRevision":transport.resource_revision()}),
            Arc::new(move || actual.report())).await.map_err(error)?;
        if unified.is_some() {
            if connection.side().is_client() {

                loop {
                    budget.remaining().map_err(error)?;
                    let result=endpoint.open_control_path(connection,owner.lease.remote_addr().map_err(error)?).await;

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

            endpoint.retain_control_data_transport_path(connection,owner.lease.remote_addr().map_err(error)?).await.map_err(error)?;

            send.finish().map_err(error)?;
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
        }

        phase(&mut recv, "channel-open").await?;

        write_frame(&mut send, &json!({"kind":"channel-open"})).await.map_err(error)?;

        let remote = iroh::TransportAddr::Custom(owner.lease.remote_addr().map_err(error)?);
        if connection.side().is_client() {  loop {
            budget.remaining().map_err(error)?;
            let result=endpoint.open_control_path(connection,owner.lease.remote_addr().map_err(error)?).await;

            match result {
                Ok(()) => break,
                Err(cause) if cause.kind() == std::io::ErrorKind::WouldBlock => n0_future::time::sleep(Duration::from_millis(10)).await,
                Err(cause) => return Err(error(cause)),
            }
        }
        }


        loop {
            budget.remaining().map_err(error)?;
            if connection.paths().iter().any(|path| path.remote_addr() == &remote) { break; }
            n0_future::time::sleep(Duration::from_millis(10)).await;
        }

        if offerer {
            write_frame(&mut send, &json!({"kind":"begin-probe","dropQuic":false})).await.map_err(error)?;
            phase(&mut recv, "probe-started").await?;
        } else { phase(&mut recv, "begin-probe").await?; }

        let probe = owner.lease.start_probe(connection).map_err(error)?;

        let probe_started = budget.elapsed_ms();
        if !offerer { write_frame(&mut send, &json!({"kind":"probe-started","pathId":probe.path_id(),"atMs":probe_started})).await.map_err(error)?; }
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
        if offerer {
            write_frame(&mut send, &json!({"kind":"retain"})).await.map_err(error)?;
            phase(&mut recv, "retained").await?;
        } else { phase(&mut recv, "retain").await?; }
        let pending_paths = connection.paths();
        let pending = pending_paths.iter().find(|path| path.id().to_string() == probe.path_id())
            .ok_or_else(|| error("RTC exact probe path disappeared"))?;
        let before_retain_frames = json!({"streamSent":pending.stats().frame_tx.stream,"streamReceived":pending.stats().frame_rx.stream,
            "datagramSent":pending.stats().frame_tx.datagram,"datagramReceived":pending.stats().frame_rx.datagram});
        owner.lease.retain(connection, &probe).map_err(error)?;
        if offerer { write_frame(&mut send, &json!({"kind":"ready"})).await.map_err(error)?; }
        else {
            write_frame(&mut send, &json!({"kind":"retained","pathId":probe.path_id()})).await.map_err(error)?;
            phase(&mut recv, "ready").await?;
        }
        endpoint.retain_control_data_transport_path(connection, owner.lease.remote_addr().map_err(error)?).await.map_err(error)?;
        if offerer { phase(&mut recv, "selected").await?; }
        else { write_frame(&mut send, &json!({"kind":"selected","pathId":probe.path_id()})).await.map_err(error)?; }
        // This very private stream began over MQTT and now confirms over RTC.
        let native = if offerer {
            write_frame(&mut send, &json!({"kind":"finish","sequence":1})).await.map_err(error)?;
            phase(&mut recv, "final").await?[if unified.is_some(){"peer"}else{"native"}].clone()
        } else {
            phase(&mut recv, "finish").await?;
            let local = json!({"freshAck":true,"probeWatermark":probe.watermark(),"rtcRole":"answerer",
                "quicRole":if connection.side().is_client(){"client"}else{"server"},
                "channel":datagram_channel_parameters(),"maxMessageSize":opening["maxMessageSize"],
                "stableUnchanged":connection.stable_id()==stable,"bindingUnchanged":binding(connection)?==original_binding,
                "route":owner.lease.report(),"dataChannel":owner.stats.report(),"authorizationRevision":observed,
                "secretGeneration":saved.generation,"resourceRevision":transport.resource_revision()});
            write_frame(&mut send, &json!({"kind":"final","peer":local})).await.map_err(error)?;
            Value::Null
        };
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
        let release_watermark = release_probe.watermark();
        if unified.is_some() {
            let io = owner.lease.io();
            let rollback_io = io.clone();
            let path_id = release_probe.path_id().to_owned();
            prepared_for_coordinator = Some(crate::connection_coordinator::PreparedPath::candidate(
                connection, iroh::TransportAddr::Custom(owner.lease.remote_addr().map_err(error)?), move || {
                    if io.is_active() { Ok(()) } else { Err(crate::connection_coordinator::control_error("DataPathUnavailable")) }
                }).with_proof(path_id, release_watermark, move || release_probe.is_acknowledged())
                .with_rollback(move || rollback_io.retire()));
        }
        send.finish().map_err(error)?;
        Ok::<_, JsValue>(json!({"switched":true,"memberStableId":stable,"stableUnchanged":connection.stable_id()==stable,
            "bindingUnchanged":binding(connection)?==original_binding,"privateStreamId":stream_id,"privateSequence":1,
            "pathId":probe.path_id(),"freshProbeStartedMs":probe_started,"freshAckMs":acknowledged,"freshAck":true,
            "probeWatermark":probe.watermark(),"beforeRetainFrames":before_retain_frames,"rtc":metadata(&owner.association).await?,
            "finalReleaseFreshAck":true,"finalReleaseProbeWatermark":release_watermark,
            "finalReleaseProbeStartedMs":release_started,"finalReleaseAckMs":budget.elapsed_ms(),
            "route":owner.lease.report(),"dataChannel":owner.stats.report(),"native":native,"authorizationRevision":observed,
            "secretGeneration":saved.generation,"resourceRevision":transport.resource_revision(),"quicRole":if connection.side().is_client(){"client"}else{"server"},"rtcRole":if offerer{"offerer"}else{"answerer"},
            "attemptMs":budget.elapsed_ms()}))
    }.fuse();
    let closed = connection.closed().fuse();
    let outcome = {
        futures_util::pin_mut!(work, closed);
        n0_future::time::timeout(budget.remaining().map_err(error)?, async {
            futures_util::select_biased! { _ = closed => Err(error("RTC original M closed")), result = work => result }
        }).await.map_err(|_| error("RTC original caller deadline exceeded")).and_then(|result|result)
    };

    let report = outcome?;

    if unified.is_some() {
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
        let original = connection.weak_handle();
        let generation = saved.generation;
        // WebSys/Rc ownership stays on the same browser executor, following the
        // adapter's existing SendWrapper convention. No lifetime M clone.
        let pending = send_wrapper::SendWrapper::new((
            owner,
            gate.clone(),
            transport.clone(),
            report.clone(),
        ));
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
            let current=original.upgrade().ok_or_else(||control_error("DataPathUnavailable"))?;

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
        return Ok((report, Some(prepared)));
    }
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
    Ok((report, prepared_for_coordinator.take()))
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
        let (_, observed, _) = self.rtc_qualification()?;
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
            write_frame(&mut send, &json!({"protocol":MEMBER_RTC_PROTOCOL,"attempt":attempt,"context":context,"binding":protected_binding,"description":{"type":"offer","sdp":self.rtc_transport.ice_profile().local_sdp(&offer.sdp()).map_err(error)?},
                "channel":datagram_channel_parameters(),"route":hex::encode(owner.lease.local_addr().data()),"quicDialer":self.endpoint.id().to_string(),
                "rtcOfferer":self.endpoint.id().to_string(),"rtcAnswerer":self.connection.remote_id().to_string(),"remainingMs":budget.remaining().map_err(error)?.as_millis() as u64,
                "authorizationRevision":observed,"resourceRevision":self.rtc_transport.resource_revision(),"faultNativePumpAfterFreshAck":fault_native_pump})).await.map_err(error)?;
            let answer = phase(&mut recv, "answer").await?;
            if answer["attempt"] != attempt || answer["binding"] != protected_binding || answer["channel"] != datagram_channel_parameters() { return Err(error("RTC answer context/profile binding mismatch")); }
            owner.lease.set_remote(iroh_base::CustomAddr::from_parts(crate::rtc_transport::TRANSPORT_ID, &hex::decode(answer["route"].as_str().ok_or_else(|| error("RTC peer route absent"))?).map_err(error)?)).map_err(error)?;
            // The original business stream responds while RTC is still being negotiated.
            continuity.push(original.exchange(&self.connection, 1).await?);
            let init = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
            let remote_sdp = answer["description"]["sdp"].as_str().ok_or_else(|| error("RTC native answer absent"))?;
            self.rtc_transport.ice_profile().check_remote_sdp(remote_sdp).map_err(error)?;
            init.set_sdp(remote_sdp);
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
                "relayBefore":relay_before,"relayAfter":relay_after,"authorizationRevision":observed,"secretGeneration":saved.generation,"resourceRevision":self.rtc_transport.resource_revision(),"quicRole":"client","rtcRole":"offerer","iceServers":self.rtc_transport.ice_profile().stun_servers,"turnServers":[],"events":owner.association.events.borrow().clone(),"ownedPumpTasks":owner.lease.report().owned_pump_tasks});
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
            "bootstrap":self.gate.0.lock().unwrap().member_reports.get(&self.connection.stable_id()),
            "paths":self.connection.paths().iter().map(|path|json!({"pathId":path.id().to_string(),"address":format!("{:?}",path.remote_addr()),"selected":path.is_selected(),"isRelay":path.is_relay(),"isRtc":matches!(path.remote_addr(),iroh::TransportAddr::Custom(address) if address.id()==crate::rtc_transport::TRANSPORT_ID),
                "isSignal":matches!(path.remote_addr(),iroh::TransportAddr::Custom(address) if matches!(address.id(),crate::mqtt_datagram::TRANSPORT_ID | crate::irc_datagram::TRANSPORT_ID)),
                "sentBytes":path.stats().udp_tx.bytes,"receivedBytes":path.stats().udp_rx.bytes,"streamFramesSent":path.stats().frame_tx.stream,"streamFramesReceived":path.stats().frame_rx.stream,
                "pingFramesSent":path.stats().frame_tx.ping,"ackFramesReceived":path.stats().frame_rx.acks,"ackFramesSent":path.stats().frame_tx.acks})).collect::<Vec<_>>()}).to_string()
    }
}
