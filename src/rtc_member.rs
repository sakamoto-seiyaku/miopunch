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
    request: Value,
    send: &mut SendStream,
    recv: &mut RecvStream,
    control: bool,
) -> Result<()> {
    let remaining_ms = request["remainingMs"]
        .as_u64()
        .filter(|value| *value > 0 && *value <= 12000)
        .ok_or("RTC caller remainder invalid")?;
    let remote_deadline = n0_future::time::Instant::now() + Duration::from_millis(remaining_ms);
    let budget = RtcDeadline::from_caller(
        connection
            .caller_deadline()
            .map_or(remote_deadline, |local| local.min(remote_deadline)),
    );
    let qualification = gate
        .path_qualification(connection)
        .ok_or("RTC requires current Ordinary M")?;
    let proof = gate.proof_for(connection).ok_or("RTC M proof missing")?;
    let attempt = request["attempt"]
        .as_str()
        .filter(|attempt| hex::decode(attempt).is_ok_and(|bytes| bytes.len() == 16))
        .ok_or("RTC attempt invalid")?;
    let context = attempt_context(
        attempt,
        proof.scope().network_id(),
        proof.scope().secret_generation(),
        &connection.remote_id().to_string(),
        &endpoint.id().to_string(),
    );
    if !connection.side().is_server()
        || request["protocol"] != MEMBER_RTC_PROTOCOL
        || request["context"] != context
        || request["binding"].as_str()
            != Some(rtc_probe::binding(connection, context.as_bytes())?.as_str())
        || request["channel"] != datagram_channel_parameters()
        || request["quicDialer"] != connection.remote_id().to_string()
        || request["rtcOfferer"] != connection.remote_id().to_string()
        || request["rtcAnswerer"] != endpoint.id().to_string()
    {
        return Err("RTC original-M context/profile/role binding rejected".into());
    }
    let offer: RTCSessionDescription = serde_json::from_value(request["description"].clone())?;
    if offer.sdp.contains(" typ relay") {
        return Err("RTC owned direct profile rejects relay candidates".into());
    }
    let current = qualification.clone();
    let attempt_live = Arc::new(AtomicBool::new(true));
    let live = attempt_live.clone();
    let lease = transport.reserve_with_updates(
        connection,
        move || current.is_current() && live.load(Ordering::Acquire),
        qualification.updates(),
    )?;
    lease.set_remote(CustomAddr::from_parts(
        TRANSPORT_ID,
        &hex::decode(request["route"].as_str().ok_or("RTC peer route missing")?)?,
    ))?;
    let socket_before = rtc_probe::sockets();
    let (gathered, mut gather_rx) = tokio::sync::mpsc::channel(1);
    let events = Arc::new(Mutex::new(Vec::new()));
    let pc = tokio::select! { biased;
        _ = connection.closed() => return Err("RTC original M closed during construction".into()),
        result = tokio::time::timeout(budget.remaining()?, rtc_probe::peer(gathered, events.clone())) => result??,
    };
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
        pc.set_remote_description(offer).await?;
        pc.set_local_description(pc.create_answer(None).await?)
            .await?;
        tokio::time::timeout(budget.remaining()?, gather_rx.recv())
            .await?
            .ok_or("RTC gathering ended")?;
        let answer = pc
            .local_description()
            .await
            .ok_or("RTC native answer absent")?;
        write_frame(send, &json!({"kind":"answer","attempt":request["attempt"],"binding":request["binding"],"channel":datagram_channel_parameters(),"description":answer,"route":hex::encode(lease.local_addr().data()),"authorizationRevision":qualification.revision(),"resourceRevision":transport.resource_revision()})).await?;
        loop {
            match tokio::time::timeout(budget.remaining()?, dc.poll()).await? {
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
                json!({"event":"rtc-pump-ended","stableId":stable,"active":io.is_active(),"qualified":io.report().qualified,"failure":result.as_ref().err().map(ToString::to_string)})
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
        write_frame(
            send,
            &json!({"kind":"channel-open","atMs":budget.elapsed_ms()}),
        )
        .await?;
        phase(recv, "channel-open", budget).await?;
        let begin = phase(recv, "begin-probe", budget).await?;
        if begin["dropQuic"].as_bool().unwrap_or(false) {
            drop_quic.store(true, Ordering::Release);
        }
        let probe = path_probe(&lease, connection, budget).await?;
        let probe_started_ms = budget.elapsed_ms();
        println!(
            "{}",
            json!({"event":"rtc-probe-started","stableId":connection.stable_id(),"pathId":probe.path_id(),"dropQuic":drop_quic.load(Ordering::Acquire),"atMs":probe_started_ms})
        );
        write_frame(
            send,
            &json!({"kind":"probe-started","pathId":probe.path_id(),"atMs":probe_started_ms}),
        )
        .await?;
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
        phase(recv, "retain", budget).await?;
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
        write_frame(send, &json!({"kind":"retained","pathId":probe.path_id()})).await?;
        phase(recv, "ready", budget).await?;
        if control {
            endpoint
                .retain_control_data_transport_path(connection, lease.remote_addr()?)
                .await?;
        } else {
            endpoint
                .retain_data_transport_path(connection, lease.remote_addr()?)
                .await?;
        }
        write_frame(send, &json!({"kind":"selected","pathId":probe.path_id()})).await?;
        if !control {
            phase(recv, "disable-relay", budget).await?;
            for path in connection.paths().iter().filter(|path| path.is_relay()) {
                path.close()?;
            }
            write_frame(send, &json!({"kind":"relay-disabled"})).await?;
        }
        phase(recv, "finish", budget).await?;
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
        let mut report = json!({"switched":true,"memberStableId":connection.stable_id(),"stableUnchanged":connection.stable_id()==stable_before,"bindingUnchanged":session_binding(connection)?==binding_before,"pathId":probe.path_id(),"freshProbeStartedMs":probe_started_ms,"freshAckMs":ack_ms,"freshAck":true,"probeWatermark":probe.watermark(),"beforeRetainFrames":before_retain_frames,"channel":attributes,"maxMessageSize":max_message_size,"selectedPair":{"local":pair.local(),"remote":pair.remote()},"stats":stats(&pc).await,"relayBefore":relay_before,"relayAfter":relay_after,"route":lease.report(),"dataChannel":pump_stats.report(),"authorizationRevision":qualification.revision(),"secretGeneration":proof.scope().secret_generation(),"resourceRevision":transport.resource_revision(),"quicRole":"server","rtcRole":"answerer","iceServers":[],"turnServers":[],"events":events.lock().unwrap().clone(),"callerRemainderMs":remaining_ms,"attemptMs":budget.elapsed_ms()});
        write_frame(send, &json!({"kind":"final","native":report})).await?;
        send.finish()?;
        if control {
            let actual = pump_stats.clone();
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
            report["finalReleaseFreshAck"] = json!(true);
            report["finalReleaseProbeWatermark"] = json!(release_probe.watermark());
            report["finalReleaseProbeStartedMs"] = json!(release_started);
            report["finalReleaseAckMs"] = json!(budget.elapsed_ms());
        }
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(report)
    };
    let outcome = tokio::select! { biased;
        _ = connection.closed() => Err("RTC original M closed".into()),
        failure = &mut pump_done_rx => Err(failure.unwrap_or_else(|_| "RTC pump completion lost".into()).into()),
        result = tokio::time::timeout(budget.remaining().unwrap_or(Duration::ZERO), work) => result.map_err(|_| Box::<dyn std::error::Error + Send + Sync>::from("RTC caller deadline exceeded")).and_then(|result| result),
    };
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
                let closed = pc.close().await.is_ok();
                drop(pc);
                tokio::task::yield_now().await;
                println!(
                    "{}",
                    json!({"event":"rtc-path-cleanup","stableId":stable,"switched":true,
                    "cleanup":{"pcClosed":closed,"ownedPumpTasks":lease.report().owned_pump_tasks,"txBytes":lease.report().tx_bytes,"rxBytes":lease.report().rx_bytes,
                        "activePumpTasks":transport.active_pumps(),"activeAssociations":transport.active_associations(),
                        "socketBefore":socket_before,"socketAfter":rtc_probe::sockets(),"dataChannel":pump_stats.report()}})
                );
            });
            return Ok(());
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
    Ok(())
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
            self.io.retire();
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
        endpoint, connection, gate, transport, request, &mut send, &mut recv, true,
    )
    .await
    .map_err(std::io::Error::other)
}
