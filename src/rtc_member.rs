//! Experimental Ordinary-M RTC coordinator and owned native DataChannel pump.
//! Signaling and echo streams stay on the original admitted Connection.
use crate::{
    admission_gate::{AdmissionGate, ProtocolScope},
    rtc_association::{
        CHANNEL_ID, CONTINUITY_BINDING_LABEL, CONTINUITY_MARKER, CONTROL_MARKER,
        DATAGRAM_CHANNEL_LABEL, DATAGRAM_CHANNEL_PROTOCOL, MEMBER_RTC_PROTOCOL, QUEUE_BYTES,
        RtcDeadline, attempt_context, datagram_channel_parameters,
    },
    rtc_probe::{self, Result},
    rtc_transport::{MAX_DATAGRAM, RtcDatagramIo, RtcRouteLease, RtcTransport, TRANSPORT_ID},
    rtc_wire::{read_frame, read_optional_frame, write_frame},
};
use bytes::BytesMut;
use futures_util::future::poll_fn;
use iroh::{
    Endpoint,
    endpoint::{Connection, RecvStream, SendStream},
};
use iroh_base::CustomAddr;
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use webrtc::{
    data_channel::{DataChannel, DataChannelEvent, RTCDataChannelInit},
    peer_connection::{PeerConnection, RTCSessionDescription, RTCStatsReportEntry, StatsSelector},
};

fn session_binding(connection: &Connection) -> Result<[u8; 32]> {
    let mut bytes = [0; 32];
    connection
        .export_keying_material(&mut bytes, CONTINUITY_BINDING_LABEL, &[])
        .map_err(|_| "original M exporter unavailable")?;
    Ok(bytes)
}

/// Preserve ordinary echo requests and the step-4 synthetic protocol while
/// exposing one bounded, long-lived continuity stream for the step-5 fixture.
pub async fn serve_stream(
    endpoint: Endpoint,
    connection: Connection,
    gate: AdmissionGate,
    transport: RtcTransport,
    mut send: SendStream,
    mut recv: RecvStream,
) -> Result<()> {
    let mut prefix = [0; 4];
    let mut length = 0;
    while length < prefix.len() {
        match recv.read(&mut prefix[length..]).await? {
            Some(n) => length += n,
            None => break,
        }
    }
    if length == 4 && &prefix == CONTROL_MARKER {
        let request = read_frame(&mut recv).await?;
        // An RTC attempt failure does not terminate an otherwise allowed relay M.
        if let Err(error) = upgrade(
            &endpoint,
            &connection,
            &gate,
            &transport,
            request,
            &mut send,
            &mut recv,
            false,
            None,
            false,
            None,
        )
        .await
        {
            println!(
                "{}",
                json!({"event":"rtc-path-rejected","stableId":connection.stable_id(),"failure":error.to_string()})
            );
        }
        return Ok(());
    }
    if length == 4 && &prefix == CONTINUITY_MARKER {
        let binding = session_binding(&connection)?;
        let stream_id = send.id().to_string();
        while let Some(message) = read_optional_frame(&mut recv, 4096).await? {
            if !gate.allows(&connection, ProtocolScope::Member) {
                return Err("continuity M is no longer qualified".into());
            }
            if message["sequence"].as_u64().is_none() || message["payload"].as_str().is_none() {
                return Err("invalid continuity request".into());
            }
            write_frame(&mut send, &json!({"sequence":message["sequence"],"payload":message["payload"],"streamId":stream_id,"stableId":connection.stable_id(),"bindingUnchanged":session_binding(&connection)? == binding})).await?;
        }
        send.finish()?;
        return Ok(());
    }
    let mut request = prefix[..length].to_vec();
    request.extend(recv.read_to_end(65536 - length).await?);
    if let Ok(value) = serde_json::from_slice::<Value>(&request)
        && value["protocol"] == "miopunch/rtc-probe/1"
    {
        rtc_probe::serve(
            &connection,
            &gate,
            endpoint.id().to_string(),
            value,
            &mut send,
        )
        .await?;
        return Ok(());
    }
    if request == b"PING" {
        if !gate.allows(&connection, ProtocolScope::Member) {
            return Err("member message M is no longer qualified".into());
        }
        crate::member_message::respond(&request, &mut send).await?;
    } else {
        send.write_all(&request).await?;
        send.finish()?;
    }
    println!(
        "{}",
        json!({"event":"business","bytes":request.len(),"stableId":connection.stable_id(),
            "alpn":String::from_utf8_lossy(connection.alpn()),"streamId":send.id().to_string(),
            "memberMessage":request==b"PING","reply":if request==b"PING"{Some("PONG")}else{None}})
    );
    Ok(())
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

async fn pump(
    original: iroh::endpoint::WeakConnectionHandle,
    channel: Arc<dyn DataChannel>,
    io: RtcDatagramIo,
    drop_quic: Arc<AtomicBool>,
    stats: Arc<PumpStats>,
    mut fault: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let _driver = io.pump_guard();
    let work = async {
        let closed = original.closed();
        tokio::pin!(closed);
        let mut pending: Option<Vec<u8>> = None;
        let mut blocked = false;
        loop {
            let outstanding = channel.outstanding_bytes().await?;
            stats
                .buffer_peak
                .fetch_max(outstanding as u64, Ordering::Relaxed);
            let writable = !blocked
                && pending
                    .as_ref()
                    .is_some_and(|packet| outstanding + packet.len() <= QUEUE_BYTES);
            tokio::select! { biased;
                _ = &mut closed => return Ok(()),
                _ = fault.changed() => return Err("injected native RTC pump failure".into()),
                event = channel.poll() => match event {
                    Some(DataChannelEvent::OnMessage(message)) => {
                        if message.is_string || message.data.len() > MAX_DATAGRAM { return Err("RTC bridge requires bounded binary datagrams".into()); }
                        if drop_quic.load(Ordering::Acquire) {
                            stats.dropped_packets.fetch_add(1, Ordering::Relaxed);
                            stats.dropped_bytes.fetch_add(message.data.len() as u64, Ordering::Relaxed);
                        } else {
                            stats.received_packets.fetch_add(1, Ordering::Relaxed);
                            stats.received_bytes.fetch_add(message.data.len() as u64, Ordering::Relaxed);
                            io.receive(message.data.to_vec())?;
                        }
                    }
                    Some(DataChannelEvent::OnBufferedAmountLow) => blocked = false,
                    Some(DataChannelEvent::OnClose | DataChannelEvent::OnClosing | DataChannelEvent::OnError) | None => return Err("RTC DataChannel ended".into()),
                    _ => {}
                },
                packet = poll_fn(|cx| io.poll_outbound(cx)), if pending.is_none() => {
                    let packet = packet?;
                    if drop_quic.load(Ordering::Acquire) {
                        stats.dropped_packets.fetch_add(1, Ordering::Relaxed);
                        stats.dropped_bytes.fetch_add(packet.len() as u64, Ordering::Relaxed);
                    } else { pending = Some(packet); }
                },
                result = async {
                    if !io.is_active() { return Err(webrtc::error::Error::ErrClosedPipe); }
                    channel.try_send(BytesMut::from(pending.as_ref().unwrap().as_slice())).await
                }, if writable => match result {
                    Ok(()) => {
                        let packet = pending.take().unwrap();
                        stats.sent_packets.fetch_add(1, Ordering::Relaxed);
                        stats.sent_bytes.fetch_add(packet.len() as u64, Ordering::Relaxed);
                    }
                    Err(webrtc::error::Error::ErrSendBufferFull) => blocked = true,
                    Err(error) => return Err(error.into()),
                },
            }
        }
    };
    io.while_active(work).await?
}

async fn phase(recv: &mut RecvStream, expected: &str, budget: RtcDeadline) -> Result<Value> {
    let message = tokio::time::timeout(budget.remaining()?, read_frame(recv)).await??;
    if message["kind"] != expected {
        return Err(format!("expected RTC {expected}, got {}", message["kind"]).into());
    }
    Ok(message)
}

async fn path_probe(
    lease: &RtcRouteLease,
    connection: &Connection,
    budget: RtcDeadline,
) -> Result<crate::rtc_transport::RtcPathProbe> {
    loop {
        budget.remaining()?;
        match lease.start_probe(connection) {
            Ok(probe) => return Ok(probe),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                tokio::time::sleep(Duration::from_millis(10)).await
            }
            Err(error) => return Err(error.into()),
        }
    }
}

async fn stats(pc: &Arc<dyn PeerConnection>) -> Value {
    let stats = pc.get_stats(Instant::now(), StatsSelector::None).await;
    let mut entries = Vec::new();
    for entry in stats.iter() {
        let value = match entry {
            RTCStatsReportEntry::Transport(value) => serde_json::to_value(value),
            RTCStatsReportEntry::IceCandidatePair(value) => serde_json::to_value(value),
            RTCStatsReportEntry::LocalCandidate(value)
            | RTCStatsReportEntry::RemoteCandidate(value) => serde_json::to_value(value),
            RTCStatsReportEntry::DataChannel(value) => serde_json::to_value(value),
            _ => continue,
        };
        if let Ok(value) = value {
            entries.push(value);
        }
        if entries.len() == 128 {
            break;
        }
    }
    json!(entries)
}

async fn upgrade(
    endpoint: &Endpoint,
    connection: &Connection,
    gate: &AdmissionGate,
    transport: &RtcTransport,
    mut request: Value,
    send: &mut SendStream,
    recv: &mut RecvStream,
    control: bool,
    unified: Option<crate::connection_coordinator::ProtectedAttempt>,
    active: bool,
    active_deadline: Option<n0_future::time::Instant>,
) -> Result<Option<crate::connection_coordinator::PreparedPath>> {
    let offerer = unified
        .is_some_and(|attempt| attempt.rtc_role == crate::connection_coordinator::RtcRole::Offerer);
    let remaining_ms = request["remainingMs"]
        .as_u64()
        .filter(|value| *value > 0 && *value <= crate::connection_budget::MAX_MS)
        .ok_or("RTC caller remainder invalid")?;
    let remote_deadline = n0_future::time::Instant::now() + Duration::from_millis(remaining_ms);
    let caller_deadline = active_deadline.or_else(|| connection.caller_deadline());
    let peer_budget_receiver = control
        && !active
        && unified.is_some_and(|attempt| {
            attempt.rtc_role == crate::connection_coordinator::RtcRole::Answerer
        });
    if caller_deadline.is_none() && !peer_budget_receiver {
        return Err("RTC caller deadline absent".into());
    }
    let budget =
        caller_deadline.map(|deadline| RtcDeadline::from_caller(deadline.min(remote_deadline)));
    let qualification = gate
        .path_qualification(connection)
        .ok_or("RTC requires current Ordinary M")?;
    let proof = gate.proof_for(connection).ok_or("RTC M proof missing")?;
    let attempt = request["attempt"]
        .as_str()
        .filter(|attempt| hex::decode(attempt).is_ok_and(|bytes| bytes.len() == 16))
        .ok_or("RTC attempt invalid")?
        .to_owned();
    let context = attempt_context(
        &attempt,
        proof.scope().network_id(),
        proof.scope().secret_generation(),
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
    if unified.is_some_and(|expected| {
        !matches!(
            expected.rtc_role,
            crate::connection_coordinator::RtcRole::Offerer
                | crate::connection_coordinator::RtcRole::Answerer
        ) || !expected.matches(&attempt)
    }) {
        return Err(crate::connection_coordinator::control_error("InvalidControl").into());
    }
    let expected_protocol = if unified.is_some() {
        crate::rtc_association::UNIFIED_RTC_PROTOCOL
    } else {
        MEMBER_RTC_PROTOCOL
    };
    let quic_dialer = if let Some(expected) = unified {
        expected.quic_dialer(endpoint, connection)
    } else {
        connection.remote_id()
    };
    if (unified.is_none() && !connection.side().is_server())
        || request["protocol"] != expected_protocol
        || request["context"] != context
        || request["binding"].as_str()
            != Some(rtc_probe::binding(connection, context.as_bytes())?.as_str())
        || request["channel"] != datagram_channel_parameters()
        || request["quicDialer"] != quic_dialer.to_string()
        || request["rtcOfferer"]
            != if offerer {
                endpoint.id()
            } else {
                connection.remote_id()
            }
            .to_string()
        || request["rtcAnswerer"]
            != if offerer {
                connection.remote_id()
            } else {
                endpoint.id()
            }
            .to_string()
    {
        return Err(if unified.is_some() {
            crate::connection_coordinator::control_error("InvalidControl").into()
        } else {
            "RTC original-M context/profile/role binding rejected".into()
        });
    }
    let offer = if offerer {
        None
    } else {
        if request["description"]["type"] != "offer" {
            return Err(crate::connection_coordinator::control_error("InvalidControl").into());
        }
        let offer: RTCSessionDescription = serde_json::from_value(request["description"].clone())?;
        transport.ice_profile().check_remote_sdp(&offer.sdp)?;
        Some(offer)
    };
    // Initial unified incoming M may have no local caller metadata. Only after
    // validating its protected request can the answerer use the peer's existing
    // remaining budget. Keep the receive-time deadline; never restart a stage.
    let budget = budget.unwrap_or_else(|| RtcDeadline::from_caller(remote_deadline));
    let current = qualification.clone();
    let attempt_live = Arc::new(AtomicBool::new(true));
    let live = attempt_live.clone();
    let lease = transport.reserve_with_updates(
        connection,
        move || current.is_current() && live.load(Ordering::Acquire),
        qualification.updates(),
    )?;
    if !offerer {
        lease.set_remote(CustomAddr::from_parts(
            TRANSPORT_ID,
            &hex::decode(request["route"].as_str().ok_or("RTC peer route missing")?)?,
        ))?;
    }
    let socket_before = rtc_probe::sockets();
    let (gathered, mut gather_rx) = tokio::sync::mpsc::channel(1);
    let events = Arc::new(Mutex::new(Vec::new()));
    let preparation_io = lease.io();
    let pc = preparation_io
        .while_current(tokio::time::timeout(
            budget.remaining()?,
            rtc_probe::peer_with_profile(gathered, events.clone(), transport.ice_profile()),
        ))
        .await???;
    // Dropping a hook future must retire its exact route and close the PC,
    // even when it never reaches the explicit result/cleanup tail.
    let mut cancellation_cleanup = CancelRtc {
        io: lease.io(),
        live: attempt_live.clone(),
        pc: Some(pc.clone()),
        pump: None,
    };
    let drop_quic = Arc::new(AtomicBool::new(false));
    let pump_stats = Arc::new(PumpStats::default());
    let mut owned_pump = None;
    let mut prepared_for_coordinator = None;
    let mut initial_unified_probe = None;
    // A diagnostic may retain the actual channel until the finite PC-cleanup
    // hold ends. No IO/pump/M is retained or reactivated by this extra handle.
    let mut fixture_channel: Option<Arc<dyn DataChannel>> = None;
    let (pump_fault, pump_fault_rx) = tokio::sync::watch::channel(false);
    let (pump_done_tx, mut pump_done_rx) = tokio::sync::oneshot::channel::<String>();
    let mut pump_done_tx = Some(pump_done_tx);
    let stable_before = connection.stable_id();
    let binding_before = session_binding(connection)?;
    let initial_paths = connection.paths();
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

    let work = async {
        let (dc, attributes, sctp, max_message_size) = preparation_io.while_current(async {
            let dc = pc
                .create_data_channel(
                    DATAGRAM_CHANNEL_LABEL,
                    Some(RTCDataChannelInit {
                        ordered: false,
                        max_retransmits: Some(0),
                        negotiated: Some(CHANNEL_ID),
                        protocol: DATAGRAM_CHANNEL_PROTOCOL.into(),
                        ..Default::default()
                    }),
                )
                .await?;
            dc.set_buffered_amount_low_threshold((QUEUE_BYTES - MAX_DATAGRAM) as u32)
                .await?;
            if control && transport.holds_cleanup_fixture() {
                fixture_channel = Some(dc.clone());
            }
            if offerer {
                let offer = pc.create_offer(None).await?;

                pc.set_local_description(offer).await?;

                tokio::time::timeout(budget.remaining()?, gather_rx.recv())
                    .await?
                    .ok_or("RTC gathering ended")?;

                let mut offer = pc
                    .local_description()
                    .await
                    .ok_or("RTC native offer absent")?;

                offer.sdp = transport.ice_profile().local_sdp(&offer.sdp)?;

                request["description"] = serde_json::to_value(offer)?;
                request["route"] = json!(hex::encode(lease.local_addr().data()));
                request["remainingMs"] = json!(budget.remaining()?.as_millis() as u64);

                write_frame(send, &request).await?;

                let answer = phase(recv, "answer", budget).await?;

                if answer["attempt"] != request["attempt"]
                    || answer["binding"] != request["binding"]
                    || answer["channel"] != datagram_channel_parameters()
                    || answer["description"]["type"] != "answer"
                {
                    return Err(crate::connection_coordinator::control_error("InvalidControl").into());
                }
                let remote: RTCSessionDescription =
                    serde_json::from_value(answer["description"].clone())?;
                transport.ice_profile().check_remote_sdp(&remote.sdp)?;

                lease.set_remote(CustomAddr::from_parts(
                    TRANSPORT_ID,
                    &hex::decode(answer["route"].as_str().ok_or("RTC peer route missing")?)?,
                ))?;

                pc.set_remote_description(remote).await?;
            } else {
                pc.set_remote_description(offer.ok_or("RTC peer offer absent")?)
                    .await?;
                pc.set_local_description(pc.create_answer(None).await?)
                    .await?;
                tokio::time::timeout(budget.remaining()?, gather_rx.recv())
                    .await?
                    .ok_or("RTC gathering ended")?;
                let mut answer = pc
                    .local_description()
                    .await
                    .ok_or("RTC native answer absent")?;

                answer.sdp = transport.ice_profile().local_sdp(&answer.sdp)?;

                write_frame(send, &json!({"kind":"answer","attempt":request["attempt"],"binding":request["binding"],"channel":datagram_channel_parameters(),"description":answer,"route":hex::encode(lease.local_addr().data()),"authorizationRevision":qualification.revision(),"resourceRevision":transport.resource_revision()})).await?;
            }

            loop {
                let event = tokio::time::timeout(budget.remaining()?, dc.poll()).await?;

                match event {
                    Some(DataChannelEvent::OnOpen) => break,
                    Some(DataChannelEvent::OnClose | DataChannelEvent::OnError) | None => {
                        return Err("RTC native channel did not open".into());
                    }
                    _ => {}
                }
            }

            let attributes = rtc_probe::channel_attributes(&dc).await?;
            let mut parameters = datagram_channel_parameters();
            parameters["readyState"] = "Open".into();
            if attributes != parameters {
                return Err(format!("RTC data channel profile mismatch: {attributes}").into());
            }
            let sctp = pc.sctp().await.ok_or("SCTP unavailable")?;
            let max_message_size = sctp.max_message_size().await?;
            if max_message_size < MAX_DATAGRAM as u32 {
                return Err("RTC negotiated maximum below bridge bound".into());
            }

            lease.activate()?;
            // The route stores only a weak observer handle after the cancellation
            // guard owns the strong PC and the route is active. This preserves the
            // exact association without changing its cleanup lifetime.
            lease.attach_peer_connection(&pc)?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>((dc, attributes, sctp, max_message_size))
        }).await??;
        // Preparation relinquishes its watcher before the pump acquires it.
        let original = connection.weak_handle();
        let io = lease.io();
        let loss = drop_quic.clone();
        let traffic = pump_stats.clone();
        let stable = connection.stable_id();
        let done = pump_done_tx.take().ok_or("RTC pump already started")?;
        owned_pump = Some(tokio::spawn(async move {
            let result = pump(original, dc, io.clone(), loss, traffic, pump_fault_rx).await;
            println!(
                "{}",
                json!({"event":"rtc-pump-ended","stableId":stable,"active":io.is_active(),"qualified":io.report().qualified,"failure":result.as_ref().err().map(ToString::to_string),
                    "retirementDiagnostic":io.retirement_diagnostic(),
                    "monotonicMs":crate::rtc_transport::diagnostic_monotonic_ms()})
            );
            let _ = done.send(
                result
                    .as_ref()
                    .err()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "RTC pump ended".into()),
            );
            result
        }));
        cancellation_cleanup.pump = owned_pump
            .as_ref()
            .map(tokio::task::JoinHandle::abort_handle);
        if unified.is_some() {
            // Both endpoints validate their exact route independently. Active M
            // keeps the lease validation-only until the common Ready barrier;
            // initial Control M preserves its restricted control seal.
            if connection.side().is_client() {
                loop {
                    budget.remaining()?;
                    let result = if active {
                        endpoint
                            .open_data_transport_path(
                                connection,
                                iroh::TransportAddr::Custom(lease.remote_addr()?),
                            )
                            .await
                    } else {
                        endpoint
                            .open_control_path(connection, lease.remote_addr()?)
                            .await
                    };

                    match result {
                        Ok(()) => break,
                        Err(cause) if cause.kind() == std::io::ErrorKind::WouldBlock => {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                        Err(cause) => return Err(cause.into()),
                    }
                }
            }

            let started = budget.elapsed_ms();
            let probe = path_probe(&lease, connection, budget).await?;
            if active {
                let observed = connection.paths();
                let frames = observed
                    .iter()
                    .find(|path| path.id().to_string() == probe.path_id())
                    .map(|path| {
                        json!({"streamSent":path.stats().frame_tx.stream,
                        "streamReceived":path.stats().frame_rx.stream,
                        "datagramSent":path.stats().frame_tx.datagram,
                        "datagramReceived":path.stats().frame_rx.datagram})
                    });
                println!(
                    "{}",
                    json!({"event":"rtc-active-progress","stage":"probe-created",
                    "stableId":connection.stable_id(),"pathId":probe.path_id(),
                    "probeWatermark":probe.watermark(),"acknowledged":probe.is_acknowledged().ok(),
                    "frames":frames,"elapsedMs":budget.elapsed_ms(),
                    "rtcRole":if offerer {"offerer"} else {"answerer"}})
                );
            }
            while !probe.is_acknowledged()? {
                budget.remaining()?;
                if !qualification.is_current() || !lease.io().is_active() {
                    return Err("RTC initial qualification retired during probe".into());
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            if active {
                println!(
                    "{}",
                    json!({"event":"rtc-active-progress","stage":"probe-acknowledged",
                    "stableId":connection.stable_id(),"pathId":probe.path_id(),
                    "probeWatermark":probe.watermark(),"elapsedMs":budget.elapsed_ms()})
                );
            }

            if !qualification.is_current() || !gate.allows(connection, ProtocolScope::Member) {
                return Err("RTC initial qualification changed before control seal".into());
            }
            let paths = connection.paths();
            let path = paths
                .iter()
                .find(|path| path.id().to_string() == probe.path_id())
                .ok_or("RTC initial exact path disappeared")?;
            let frames = json!({"streamSent":path.stats().frame_tx.stream,"streamReceived":path.stats().frame_rx.stream,
                "datagramSent":path.stats().frame_tx.datagram,"datagramReceived":path.stats().frame_rx.datagram});
            if !active {
                lease.retain(connection, &probe)?;
                endpoint
                    .retain_control_data_transport_path(connection, lease.remote_addr()?)
                    .await?;
            }

            send.finish()?;
            let report = json!({"prepared":true,"finalized":false,"switched":false,
                "memberStableId":stable_before,"stableUnchanged":connection.stable_id()==stable_before,
                "bindingUnchanged":session_binding(connection)?==binding_before,"freshAck":true,
                "initialFreshAck":true,"initialProbeWatermark":probe.watermark(),"probeWatermark":probe.watermark(),
                "pathId":probe.path_id(),"freshProbeStartedMs":started,"freshAckMs":budget.elapsed_ms(),
                "beforeRetainFrames":frames,"finalReleaseFreshAck":false,"channel":attributes,
                "maxMessageSize":max_message_size,"authorizationRevision":qualification.revision(),
                "secretGeneration":proof.scope().secret_generation(),"resourceRevision":transport.resource_revision(),
                "quicRole":if connection.side().is_client(){"client"}else{"server"},
                "rtcRole":if offerer{"offerer"}else{"answerer"},"route":lease.report(),"dataChannel":pump_stats.report()});
            initial_unified_probe = Some(probe);
            return Ok(report);
        }

        write_frame(
            send,
            &json!({"kind":"channel-open","atMs":budget.elapsed_ms()}),
        )
        .await?;

        phase(recv, "channel-open", budget).await?;

        if connection.side().is_client() {
            loop {
                budget.remaining()?;
                let result = endpoint
                    .open_control_path(connection, lease.remote_addr()?)
                    .await;

                match result {
                    Ok(()) => break,
                    Err(cause) if cause.kind() == std::io::ErrorKind::WouldBlock => {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    Err(cause) => return Err(cause.into()),
                }
            }
        }
        if connection.side().is_client() {}

        let begin = if offerer {
            write_frame(send, &json!({"kind":"begin-probe","dropQuic":false})).await?;
            phase(recv, "probe-started", budget).await?;
            json!({"dropQuic":false})
        } else {
            phase(recv, "begin-probe", budget).await?
        };
        if begin["dropQuic"].as_bool().unwrap_or(false) {
            drop_quic.store(true, Ordering::Release);
        }

        let probe = path_probe(&lease, connection, budget).await?;

        let probe_started_ms = budget.elapsed_ms();
        println!(
            "{}",
            json!({"event":"rtc-probe-started","stableId":connection.stable_id(),"pathId":probe.path_id(),"dropQuic":drop_quic.load(Ordering::Acquire),"atMs":probe_started_ms})
        );
        if !offerer {
            write_frame(
                send,
                &json!({"kind":"probe-started","pathId":probe.path_id(),"atMs":probe_started_ms}),
            )
            .await?;
        }
        let mut client_ack = false;
        loop {
            budget.remaining()?;
            if !qualification.is_current() || !lease.io().is_active() {
                return Err("RTC qualification retired during probe".into());
            }
            if probe.is_acknowledged()? {
                break;
            }
            tokio::select! { biased;
                message = read_frame(recv), if !client_ack => {
                    let message = message?;
                    if message["kind"] != "probe-ack" { return Err("RTC peer probe phase invalid".into()); }
                    client_ack = true;
                },
                _ = tokio::time::sleep(Duration::from_millis(10)) => {},
            }
        }
        let ack_ms = budget.elapsed_ms();
        if request["faultNativePumpAfterFreshAck"]
            .as_bool()
            .unwrap_or(false)
        {
            println!(
                "{}",
                json!({"event":"rtc-pump-fault-after-fresh-ack","stableId":connection.stable_id(),"pathId":probe.path_id(),"freshAck":true,"atMs":ack_ms})
            );
            let _ = pump_fault.send(true);
            tokio::task::yield_now().await;
        }
        write_frame(
            send,
            &json!({"kind":"probe-ack","pathId":probe.path_id(),"atMs":ack_ms}),
        )
        .await?;
        if !client_ack {
            phase(recv, "probe-ack", budget).await?;
        }
        if offerer {
            write_frame(send, &json!({"kind":"retain"})).await?;
        } else {
            phase(recv, "retain", budget).await?;
        }
        if !qualification.is_current() || !gate.allows(connection, ProtocolScope::Member) {
            return Err("RTC member qualification changed before retain".into());
        }
        let pending_paths = connection.paths();
        let pending = pending_paths
            .iter()
            .find(|path| path.id().to_string() == probe.path_id())
            .ok_or("RTC exact probe path disappeared")?;
        let before_retain_frames = json!({"streamSent":pending.stats().frame_tx.stream,"streamReceived":pending.stats().frame_rx.stream,"datagramSent":pending.stats().frame_tx.datagram,"datagramReceived":pending.stats().frame_rx.datagram});
        lease.retain(connection, &probe)?;
        // Keep the already Available relay for this bilateral confirmation.
        if offerer {
            phase(recv, "retained", budget).await?;
            write_frame(send, &json!({"kind":"ready"})).await?;
        } else {
            write_frame(send, &json!({"kind":"retained","pathId":probe.path_id()})).await?;
            phase(recv, "ready", budget).await?;
        }
        if control {
            endpoint
                .retain_control_data_transport_path(connection, lease.remote_addr()?)
                .await?;
        } else if !active {
            endpoint
                .retain_data_transport_path(connection, lease.remote_addr()?)
                .await?;
        }
        if offerer {
            phase(recv, "selected", budget).await?;
        } else {
            write_frame(send, &json!({"kind":"selected","pathId":probe.path_id()})).await?;
        }
        if !control && !active {
            phase(recv, "disable-relay", budget).await?;
            for path in connection.paths().iter().filter(|path| path.is_relay()) {
                path.close()?;
            }
            write_frame(send, &json!({"kind":"relay-disabled"})).await?;
        }
        let peer_report = if offerer {
            write_frame(send, &json!({"kind":"finish","sequence":1})).await?;
            Some(phase(recv, "final", budget).await?["peer"].clone())
        } else {
            phase(recv, "finish", budget).await?;
            None
        };
        let pair = sctp
            .transport()
            .ice_transport()
            .get_selected_candidate_pair()
            .await?
            .ok_or("RTC selected candidate pair unavailable")?;
        let relay_after: Vec<_> = relay_paths
            .iter()
            .map(|path| {
                (
                    path.id().to_string(),
                    path.stats().udp_tx.bytes,
                    path.stats().udp_rx.bytes,
                )
            })
            .collect();
        let mut report = json!({"switched":true,"memberStableId":connection.stable_id(),"stableUnchanged":connection.stable_id()==stable_before,"bindingUnchanged":session_binding(connection)?==binding_before,"pathId":probe.path_id(),"freshProbeStartedMs":probe_started_ms,"freshAckMs":ack_ms,"freshAck":true,"probeWatermark":probe.watermark(),"beforeRetainFrames":before_retain_frames,"channel":attributes,"maxMessageSize":max_message_size,"selectedPair":{"local":pair.local(),"remote":pair.remote()},"stats":stats(&pc).await,"relayBefore":relay_before,"relayAfter":relay_after,"route":lease.report(),"dataChannel":pump_stats.report(),"authorizationRevision":qualification.revision(),"secretGeneration":proof.scope().secret_generation(),"resourceRevision":transport.resource_revision(),"quicRole":if connection.side().is_client(){"client"}else{"server"},"rtcRole":if offerer{"offerer"}else{"answerer"},"iceServers":transport.ice_profile().stun_servers,"turnServers":[],"events":events.lock().unwrap().clone(),"callerRemainderMs":remaining_ms,"attemptMs":budget.elapsed_ms()});
        if let Some(peer) = peer_report {
            report["peer"] = peer;
        }
        if !offerer {
            write_frame(
                send,
                &if unified.is_some() {
                    json!({"kind":"final","peer":report})
                } else {
                    json!({"kind":"final","native":report})
                },
            )
            .await?;
        }
        if !active {
            send.finish()?;
        }
        if control || active {
            let actual = pump_stats.clone();
            if control {
                transport
                    .control_fixture_checkpoint(
                    crate::rtc_invalidation_fixture::ControlCheckpointStage::BeforeRelease,
                    connection,
                    &lease,
                    budget,
                    json!({"attempt":attempt,"stableId":connection.stable_id(),"peer":connection.remote_id().to_string(),
                        "localRoute":hex::encode(lease.local_addr().data()),"remoteRoute":hex::encode(lease.remote_addr()?.data()),
                        "pathId":probe.path_id(),"freshAck":true,"bothConfirmed":true,"probeWatermark":probe.watermark(),
                        "freshProbeStartedMs":probe_started_ms,"freshAckMs":ack_ms,
                        "source":gate.qualification_fixture(),"resourceRevision":transport.resource_revision(),"resourceAllowed":transport.resource_allowed()}),
                    Arc::new(move || actual.report()),
                )
                    .await?;
            }
            // A resumed diagnostic is not an authorization snapshot. The real
            // final actor also rechecks the exact route seal after this hook.
            if !qualification.is_current()
                || !gate.allows(connection, ProtocolScope::Member)
                || !lease.io().is_active()
            {
                return Err("RTC current qualification changed before ordinary release".into());
            }
            let release_started = budget.elapsed_ms();
            let release_probe = lease.validate_release(connection, budget).await?;
            if !qualification.is_current() || !gate.allows(connection, ProtocolScope::Member) {
                return Err("RTC current qualification changed during release validation".into());
            }
            let release_watermark = release_probe.watermark();
            if unified.is_some() && control {
                let io = lease.io();
                let rollback_io = io.clone();
                let path_id = release_probe.path_id().to_owned();
                let watermark = release_probe.watermark();
                prepared_for_coordinator = Some(
                    crate::connection_coordinator::PreparedPath::candidate(
                        connection,
                        iroh::TransportAddr::Custom(lease.remote_addr()?),
                        move || {
                            if io.is_active() {
                                Ok(())
                            } else {
                                Err(crate::connection_coordinator::control_error(
                                    "DataPathUnavailable",
                                ))
                            }
                        },
                    )
                    .with_proof(path_id, watermark, move || release_probe.is_acknowledged())
                    .with_rollback(move || {
                        rollback_io.retire_with_reason("prepared-path-rollback")
                    }),
                );
            }
            report["finalReleaseFreshAck"] = json!(true);
            report["finalReleaseProbeWatermark"] = json!(release_watermark);
            report["finalReleaseProbeStartedMs"] = json!(release_started);
            report["finalReleaseAckMs"] = json!(budget.elapsed_ms());
        }
        if active {
            initial_unified_probe = Some(probe);
        }
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(report)
    };
    let outcome = tokio::select! { biased;
        _ = connection.closed() => Err("RTC original M closed".into()),
        failure = &mut pump_done_rx => Err(failure.unwrap_or_else(|_| "RTC pump completion lost".into()).into()),
        result = tokio::time::timeout(budget.remaining().unwrap_or(Duration::ZERO), work) => result.map_err(|_| Box::<dyn std::error::Error + Send + Sync>::from("RTC caller deadline exceeded")).and_then(|result| result),
    };

    if unified.is_some() && outcome.is_ok() {
        use crate::connection_coordinator::{PreparedPath, control_error};
        let mut report = outcome.unwrap();
        let probe = Arc::new(
            initial_unified_probe
                .take()
                .ok_or("RTC initial proof absent")?,
        );
        let initial_path = probe.path_id().to_owned();
        let initial_watermark = probe.watermark();
        let final_proof: Arc<Mutex<Option<Arc<crate::rtc_transport::RtcPathProbe>>>> =
            Arc::new(Mutex::new(None));
        let checked_final = final_proof.clone();
        let io = lease.io();
        let rollback_io = io.clone();
        let remote = lease.remote_addr()?;
        let original = connection.weak_handle();
        let gate = gate.clone();
        let transport = transport.clone();
        let endpoint_for_retain = endpoint.clone();
        let pending = PreparedPath::candidate(
            connection,
            iroh::TransportAddr::Custom(remote),
            move || {
                if !io.is_active() {
                    return Err(control_error("DataPathUnavailable"));
                }
                if let Some(proof) = checked_final.lock().unwrap().as_ref()
                    && !proof.is_acknowledged()?
                {
                    return Err(control_error("DataPathUnavailable"));
                }
                Ok(())
            },
        )
        .with_proof(initial_path, initial_watermark, move || probe.is_acknowledged())
        .with_rollback(move || rollback_io.retire_with_reason("prepared-path-rollback"))
        .with_retain(move || {
            Box::pin(async move {
                // A pending finalizer owns the RAII lease/PC/pump. It upgrades
                // original M only for this finite callback, not its lifetime holder.
                let current = original
                    .upgrade()
                    .ok_or_else(|| control_error("DataPathUnavailable"))?;

                let finalize = async {

                    let actual = pump_stats.clone();
                    if control {
                        transport
                            .control_fixture_checkpoint(
                            crate::rtc_invalidation_fixture::ControlCheckpointStage::BeforeRelease,
                            &current,
                            &lease,
                            budget,
                            json!({"attempt":attempt,"stableId":stable_before,
                                "localRoute":hex::encode(lease.local_addr().data()),
                                "remoteRoute":hex::encode(lease.remote_addr()?.data()),
                                "pathId":report["pathId"],"freshAck":true,"bothConfirmed":true,
                                "probeWatermark":initial_watermark,"freshProbeStartedMs":report["freshProbeStartedMs"],
                                "freshAckMs":report["freshAckMs"],"source":gate.qualification_fixture(),
                                "resourceRevision":transport.resource_revision(),"resourceAllowed":transport.resource_allowed()}),
                            Arc::new(move || actual.report()),
                        )
                            .await?;
                    }

                    if !qualification.is_current()
                        || !gate.allows(&current, ProtocolScope::Member)
                        || !lease.io().is_active()
                    {
                        return Err(control_error("IdentityOrAdmissionRejected"));
                    }

                    let started = budget.elapsed_ms();
                    let final_ack = Arc::new(lease.validate_release(&current, budget).await?);
                    if !qualification.is_current()
                        || !gate.allows(&current, ProtocolScope::Member)
                        || session_binding(&current).map_err(std::io::Error::other)? != binding_before
                    {
                        return Err(control_error("IdentityOrAdmissionRejected"));
                    }
                    // The restricted control seal was installed once during
                    // preparation. Renew the exact lease, never reseal M.
                    lease.retain(&current, &final_ack)?;
                    if !control {
                        endpoint_for_retain
                            .retain_data_transport_path(&current, lease.remote_addr()?)
                            .await?;
                    }
                    report["prepared"] = json!(false);
                    report["finalized"] = json!(true);
                    if !active {
                        report["bothConfirmed"] = json!(true);
                    }
                    report["finalReleaseFreshAck"] = json!(true);
                    report["finalReleasePathId"] = json!(final_ack.path_id());
                    report["finalReleaseProbeWatermark"] = json!(final_ack.watermark());
                    report["finalReleaseProbeStartedMs"] = json!(started);
                    report["finalReleaseAckMs"] = json!(budget.elapsed_ms());
                    report["route"] = json!(lease.report());
                    report["dataChannel"] = pump_stats.report();
                    *final_proof.lock().unwrap() = Some(final_ack);

                    Ok::<_, std::io::Error>(())
                }
                .await;

                finalize?;
                budget.remaining()?;
                let pair = match pc.sctp().await {
                    Some(sctp) => sctp
                        .transport()
                        .ice_transport()
                        .get_selected_candidate_pair()
                        .await
                        .ok()
                        .flatten(),
                    None => None,
                };
                report["selectedPair"] = pair
                    .map(|pair| json!({"local":pair.local(),"remote":pair.remote()}))
                    .unwrap_or(Value::Null);
                report["stats"] = stats(&pc).await;
                budget.remaining()?;
                if !qualification.is_current()
                    || !gate.allows(&current, ProtocolScope::Member)
                    || !lease.io().is_active()
                {
                    return Err(control_error("IdentityOrAdmissionRejected"));
                }
                println!("{}", json!({"event":"rtc-path","report":report}));
                drop(current);
                let mut task = owned_pump.take();
                cancellation_cleanup.pc = None;
                cancellation_cleanup.pump = None;
                drop(cancellation_cleanup);
                tokio::spawn(async move {
                    let _fault_sender = pump_fault;
                    if let Some(pump) = task.as_mut() {
                        tokio::select! { _ = original.closed() => {}, _ = pump => {} }
                    }
                    attempt_live.store(false, Ordering::Release);
                    lease.close();
                    if let Some(pump) = task {
                        if !pump.is_finished() {
                            pump.abort();
                            let _ = pump.await;
                        }
                    }
                    let closed = pc.close().await.is_ok();
                    drop(pc);
                    tokio::task::yield_now().await;
                    println!("{}", json!({"event":"rtc-path-cleanup","stableId":stable_before,
                        "switched":true,"cleanup":{"pcClosed":closed,"ownedPumpTasks":lease.report().owned_pump_tasks,
                        "txBytes":lease.report().tx_bytes,"rxBytes":lease.report().rx_bytes,
                        "activePumpTasks":transport.active_pumps(),"activeAssociations":transport.active_associations(),
                        "socketBefore":socket_before,"socketAfter":rtc_probe::sockets(),
                        "route":lease.report(),"dataChannel":pump_stats.report()}}));
                });
                Ok(())
            })
        });
        return Ok(Some(pending));
    }
    let mut pump_joined = false;
    if let Ok(report) = &outcome {
        println!("{}", json!({"event":"rtc-path","report":report}));
        if control {
            // This holder owns only the association, exact lease and pump.
            // It cannot retain Endpoint, a strong M, or private hook streams.
            let original = connection.weak_handle();
            let stable = stable_before;
            let transport = RtcTransport::clone(transport);
            let mut task = owned_pump.take();
            cancellation_cleanup.pc = None;
            cancellation_cleanup.pump = None;
            drop(cancellation_cleanup);
            tokio::spawn(async move {
                let _fault_sender = pump_fault;
                if let Some(pump) = task.as_mut() {
                    tokio::select! { _ = original.closed() => {}, _ = pump => {} }
                }
                attempt_live.store(false, Ordering::Release);
                lease.close();
                if let Some(pump) = task {
                    if !pump.is_finished() {
                        pump.abort();
                        let _ = pump.await;
                    }
                }
                let final_stats = tokio::time::timeout(Duration::from_millis(250), stats(&pc))
                    .await
                    .ok();
                let closed = pc.close().await.is_ok();
                drop(pc);
                tokio::task::yield_now().await;
                println!(
                    "{}",
                    json!({"event":"rtc-path-cleanup","stableId":stable,"switched":true,
                    "cleanup":{"pcClosed":closed,"ownedPumpTasks":lease.report().owned_pump_tasks,"txBytes":lease.report().tx_bytes,"rxBytes":lease.report().rx_bytes,
                        "activePumpTasks":transport.active_pumps(),"activeAssociations":transport.active_associations(),
                        "socketBefore":socket_before,"socketAfter":rtc_probe::sockets(),"route":lease.report(),"dataChannel":pump_stats.report(),"rtcStatsBeforeClose":final_stats}})
                );
            });
            return Ok(prepared_for_coordinator.take());
        }
        // The association persists beyond the upgrade result, owned by original M.
        if let Some(task) = owned_pump.as_mut() {
            tokio::select! { _ = connection.closed() => {}, _ = task => { pump_joined = true; } }
        }
    }
    attempt_live.store(false, Ordering::Release);
    lease.close();
    if let Some(task) = owned_pump {
        if !pump_joined {
            task.abort();
            let _ = task.await;
        }
    }
    if control && outcome.is_err() {
        if let Some(channel) = fixture_channel.as_ref() {
            let sctp_state = match pc.sctp().await {
                Some(sctp) => sctp.state().await.ok().map(|state| format!("{state:?}")),
                None => None,
            };
            let details = json!({"channel":rtc_probe::channel_attributes(channel).await.ok(),
                "sctpState":sctp_state,"originalOpen":connection.weak_handle().is_open(),
                "route":lease.report(),"socketCount":rtc_probe::sockets()});
            transport.record_cleanup_details_fixture(stable_before, details.clone());
            println!(
                "{}",
                json!({"event":"rtc-native-cleanup-retired","stableId":stable_before,"details":details})
            );
        }
        transport
            .before_pc_close_fixture(stable_before, budget)
            .await;
    }
    let closed = pc.close().await.is_ok();
    drop(fixture_channel);
    cancellation_cleanup.pc = None;
    cancellation_cleanup.pump = None;
    drop(pc);
    tokio::task::yield_now().await;
    let cleanup = json!({"pcClosed":closed,"txBytes":lease.report().tx_bytes,"rxBytes":lease.report().rx_bytes,"ownedPumpTasks":lease.report().owned_pump_tasks,"activePumpTasks":transport.active_pumps(),"socketBefore":socket_before,"socketAfter":rtc_probe::sockets(),"activeAssociations":transport.active_associations(),"dataChannel":pump_stats.report()});
    println!(
        "{}",
        json!({"event":"rtc-path-cleanup","stableId":stable_before,"cleanup":cleanup,"failure":outcome.as_ref().err().map(ToString::to_string),"switched":outcome.is_ok()})
    );
    if let Err(error) = outcome {
        let _ = write_frame(send, &json!({"kind":"failed","switched":false,"freshAck":false,"failure":error.to_string(),"cleanup":cleanup})).await;
        let _ = send.finish();
        if control {
            return Err(error);
        }
    }
    Ok(None)
}

struct CancelRtc {
    io: RtcDatagramIo,
    live: Arc<AtomicBool>,
    pc: Option<Arc<dyn PeerConnection>>,
    pump: Option<tokio::task::AbortHandle>,
}
impl Drop for CancelRtc {
    fn drop(&mut self) {
        // A successful handoff empties pc; the weak lifetime holder now owns
        // retirement. Normal cleanup already retired the route before emptying it.
        if let Some(pc) = self.pc.take() {
            self.live.store(false, Ordering::Release);
            self.io.retire_with_reason("candidate-cancellation-cleanup");
            if let Some(pump) = self.pump.take() {
                pump.abort();
            }
            tokio::spawn(async move {
                let _ = pc.close().await;
            });
        }
    }
}

/// Answer on the original restricted M, then return before ordinary release.
/// Same shared profile, SDP, pump, fresh probe and bilateral selection as S5.
pub(crate) async fn bootstrap_control(
    endpoint: &Endpoint,
    connection: &Connection,
    gate: &AdmissionGate,
    transport: &RtcTransport,
) -> std::io::Result<()> {
    let (mut send, mut recv) = connection.accept_bi().await?;
    let request = read_frame(&mut recv).await.map_err(std::io::Error::other)?;
    upgrade(
        endpoint, connection, gate, transport, request, &mut send, &mut recv, true, None, false,
        None,
    )
    .await
    .map(|_| ())
    .map_err(std::io::Error::other)
}

pub(crate) async fn bootstrap_control_unified(
    endpoint: &Endpoint,
    connection: &Connection,
    gate: &AdmissionGate,
    transport: &RtcTransport,
    attempt: crate::connection_coordinator::ProtectedAttempt,
) -> std::io::Result<crate::connection_coordinator::PreparedPath> {
    use crate::connection_coordinator::{RtcRole, control_error};
    let (mut send, mut recv, request) = match attempt.rtc_role {
        RtcRole::Answerer => {
            let (send, mut recv) = connection.accept_bi().await?;
            let request = read_frame(&mut recv).await.map_err(std::io::Error::other)?;
            (send, recv, request)
        }
        RtcRole::Offerer => {
            let proof = gate
                .proof_for(connection)
                .ok_or_else(|| control_error("IdentityOrAdmissionRejected"))?;
            let budget = RtcDeadline::from_caller(
                connection
                    .caller_deadline()
                    .ok_or_else(|| control_error("InvalidControl"))?,
            );
            let id = attempt.id_hex();
            let context = attempt_context(
                &id,
                proof.scope().network_id(),
                proof.scope().secret_generation(),
                &endpoint.id().to_string(),
                &connection.remote_id().to_string(),
            );
            let request = json!({"protocol":crate::rtc_association::UNIFIED_RTC_PROTOCOL,"attempt":id,
                "context":context,"binding":rtc_probe::binding(connection,context.as_bytes()).map_err(std::io::Error::other)?,
                "channel":datagram_channel_parameters(),"quicDialer":attempt.quic_dialer(endpoint,connection).to_string(),
                "rtcOfferer":endpoint.id().to_string(),"rtcAnswerer":connection.remote_id().to_string(),
                "remainingMs":budget.remaining()?.as_millis() as u64});
            let (send, recv) = connection.open_bi().await?;
            (send, recv, request)
        }
        RtcRole::None => return Err(control_error("InvalidControl")),
    };
    upgrade(
        endpoint,
        connection,
        gate,
        transport,
        request,
        &mut send,
        &mut recv,
        true,
        Some(attempt),
        false,
        None,
    )
    .await
    .map_err(|error| {
        if error.to_string().starts_with("unified/") {
            std::io::Error::other(error.to_string())
        } else {
            crate::connection_coordinator::control_error("DataPathUnavailable")
        }
    })?
    .ok_or_else(|| crate::connection_coordinator::control_error("DataPathUnavailable"))
}

/// Active M improvement adapter. SDP and candidate control run on the
/// restricted C; QUIC path preparation and the eventual retain stay on the
/// exact original M.
pub(crate) async fn bootstrap_control_active(
    endpoint: &Endpoint,
    original: &Connection,
    control: &Connection,
    gate: &AdmissionGate,
    transport: &RtcTransport,
    attempt: crate::connection_coordinator::ProtectedAttempt,
) -> std::io::Result<crate::connection_coordinator::PreparedPath> {
    use crate::connection_coordinator::{RtcRole, control_error};
    let proof = gate
        .proof_for(original)
        .ok_or_else(|| control_error("IdentityOrAdmissionRejected"))?;
    let caller_deadline = control
        .caller_deadline()
        .unwrap_or_else(|| n0_future::time::Instant::now() + crate::connection_budget::DEFAULT);
    let budget = RtcDeadline::from_caller(caller_deadline);
    let id = attempt.id_hex();
    let context = attempt_context(
        &id,
        proof.scope().network_id(),
        proof.scope().secret_generation(),
        &endpoint.id().to_string(),
        &original.remote_id().to_string(),
    );
    let request = json!({"protocol":crate::rtc_association::UNIFIED_RTC_PROTOCOL,"attempt":id,
        "context":context,"binding":rtc_probe::binding(original,context.as_bytes()).map_err(std::io::Error::other)?,
        "channel":datagram_channel_parameters(),"quicDialer":attempt.quic_dialer(endpoint,original).to_string(),
        "rtcOfferer":endpoint.id().to_string(),"rtcAnswerer":original.remote_id().to_string(),
        "remainingMs":budget.remaining()?.as_millis() as u64});
    let (mut send, mut recv, request) = match attempt.rtc_role {
        RtcRole::Offerer => {
            let (send, recv) = control.open_bi().await?;
            (send, recv, request)
        }
        RtcRole::Answerer => {
            let (send, mut recv) = control.accept_bi().await?;
            let request = read_frame(&mut recv).await.map_err(std::io::Error::other)?;
            (send, recv, request)
        }
        RtcRole::None => return Err(control_error("InvalidControl")),
    };
    upgrade(
        endpoint,
        original,
        gate,
        transport,
        request,
        &mut send,
        &mut recv,
        false,
        Some(attempt),
        true,
        Some(caller_deadline),
    )
    .await
    .map_err(|error| {
        if error.to_string().starts_with("unified/") {
            std::io::Error::other(error.to_string())
        } else {
            std::io::Error::other(error.to_string())
        }
    })?
    .ok_or_else(|| control_error("DataPathUnavailable"))
}
