//! Test-only fixed native RTC endpoint. Signaling stays inside an admitted relay M.
use crate::{
    admission_gate::{AdmissionGate, ProtocolScope},
    rtc_association::{
        CHANNEL_ID, CHANNEL_LABEL, CHANNEL_PROTOCOL, DATAGRAM_BYTES, DatagramQueue, EXPORTER_LABEL,
        QUEUE_BYTES, attempt_context, channel_parameters,
    },
};
use bytes::BytesMut;
use iroh::endpoint::{Connection, SendStream};
use serde_json::{Value, json};
use std::{
    error::Error,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use webrtc::{
    data_channel::{DataChannel, DataChannelEvent, RTCDataChannelInit},
    peer_connection::{
        PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCConfigurationBuilder,
        RTCIceConnectionState, RTCIceGatheringState, RTCPeerConnectionState, RTCSessionDescription,
        SettingEngineBuilder,
    },
};
type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

struct Handler {
    started: Instant,
    gathered: mpsc::Sender<()>,
    events: Arc<Mutex<Vec<Value>>>,
}
impl Handler {
    fn event(&self, kind: &str, state: impl std::fmt::Display) {
        let mut events = self.events.lock().unwrap();
        if events.len() < 128 {
            events.push(json!({"kind":kind,"state":state.to_string(),"elapsedMs":self.started.elapsed().as_millis()}));
        }
    }
}
#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_data_channel(&self, channel: Arc<dyn DataChannel>) {
        // This profile has one explicitly agreed out-of-band channel only.
        let _ = channel.close().await;
    }
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        self.event("gathering", state);
        if state == RTCIceGatheringState::Complete {
            let _ = self.gathered.try_send(());
        }
    }
    async fn on_ice_connection_state_change(&self, state: RTCIceConnectionState) {
        self.event("ice", state);
    }
    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        self.event("connection", state);
    }
}
fn sockets() -> usize {
    std::fs::read_dir("/proc/self/fd")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| std::fs::read_link(entry.path()).ok())
        .filter(|path| path.to_string_lossy().starts_with("socket:"))
        .count()
}
async fn frame(send: &mut SendStream, value: &Value) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > 65536 {
        return Err("RTC control frame too large".into());
    }
    send.write_all(&(bytes.len() as u32).to_be_bytes()).await?;
    send.write_all(&bytes).await?;
    Ok(())
}
/// Hash an exporter scoped to identities, network, current secret generation and attempt.
/// The exporter itself is never serialized or logged.
pub fn binding(connection: &Connection, context: &[u8]) -> Result<String> {
    let mut exporter = [0; 32];
    connection
        .export_keying_material(&mut exporter, EXPORTER_LABEL, context)
        .map_err(|_| "RTC exporter unavailable")?;
    Ok(blake3::hash(&exporter).to_hex().to_string())
}

pub async fn serve(
    connection: &Connection,
    gate: &AdmissionGate,
    local: String,
    request: Value,
    send: &mut SendStream,
) -> Result<()> {
    let proof = gate
        .proof_for(connection)
        .ok_or("RTC requires a current M proof")?;
    let context = request
        .get("context")
        .and_then(Value::as_str)
        .ok_or("missing context")?;
    let attempt = request["attempt"].as_str().ok_or("missing attempt")?;
    let expected_context = attempt_context(
        attempt,
        proof.scope().network_id(),
        proof.scope().secret_generation(),
        &connection.remote_id().to_string(),
        &local,
    );
    if !gate.allows(connection, ProtocolScope::Member)
        || context != expected_context
        || request["binding"].as_str() != Some(binding(connection, context.as_bytes())?.as_str())
        || request["attempt"]
            .as_str()
            .is_none_or(|id| !hex::decode(id).is_ok_and(|bytes| bytes.len() == 16))
    {
        return Err("RTC association binding rejected".into());
    }
    if request["channel"] != channel_parameters() {
        return Err("RTC channel profile not confirmed".into());
    }
    let offer: RTCSessionDescription = serde_json::from_value(request["description"].clone())?;
    // Reject relay candidates even if a remote endpoint ignores the fixture's empty ICE servers.
    if offer.sdp.contains(" typ relay") {
        return Err("RTC direct-only fixture rejects relay candidates".into());
    }
    let socket_before = sockets();
    let (gathered, mut gather_rx) = mpsc::channel(1);
    let events = Arc::new(Mutex::new(Vec::new()));
    let pc = PeerConnectionBuilder::new()
        .with_configuration(
            RTCConfigurationBuilder::default()
                .with_ice_servers(vec![])
                .build(),
        )
        .with_setting_engine(
            SettingEngineBuilder::default()
                .with_sctp_max_receive_buffer_size(QUEUE_BYTES as u32)
                .build(),
        )
        .with_data_channel_send_buffer_limit(QUEUE_BYTES)
        .with_handler(Arc::new(Handler {
            started: Instant::now(),
            gathered,
            events: events.clone(),
        }))
        .with_udp_addrs(vec!["0.0.0.0:0"])
        .build()
        .await?;
    let tx = DatagramQueue::new(QUEUE_BYTES, DATAGRAM_BYTES);
    let rx = DatagramQueue::new(QUEUE_BYTES, DATAGRAM_BYTES);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let started = Instant::now();
    let outcome = async {
        let dc = pc
            .create_data_channel(
                CHANNEL_LABEL,
                Some(RTCDataChannelInit {
                    ordered: false,
                    max_retransmits: Some(0),
                    negotiated: Some(CHANNEL_ID),
                    protocol: CHANNEL_PROTOCOL.into(),
                    ..Default::default()
                }),
            )
            .await?;
        pc.set_remote_description(offer).await?;
        pc.set_local_description(pc.create_answer(None).await?)
            .await?;
        tokio::time::timeout_at(deadline, gather_rx.recv())
            .await?
            .ok_or("gathering stopped")?;
        let answer = pc.local_description().await.ok_or("missing local SDP")?;
        if !gate.allows(connection, ProtocolScope::Member) {
            return Err("RTC M retired before answer".into());
        }
        frame(send,&json!({"description":answer,"binding":request["binding"],"attempt":request["attempt"],"channel":channel_parameters()})).await?;
        let mut opened = false;
        while !opened {
            match tokio::time::timeout_at(deadline, dc.poll()).await? {
                Some(DataChannelEvent::OnOpen) => opened = true,
                Some(DataChannelEvent::OnClose) | None => {
                    return Err("channel closed before open".into());
                }
                _ => {}
            }
        }
        tx.open()?;
        rx.open()?;
        let attributes = json!({"label":dc.label().await?,"ordered":dc.ordered().await?,"maxRetransmits":dc.max_retransmits().await?,"maxPacketLifeTime":dc.max_packet_life_time().await?,"negotiated":dc.negotiated().await?,"protocol":dc.protocol().await?,"id":dc.id(),"readyState":format!("{:?}",dc.ready_state().await?)});
        if attributes["ordered"] != false
            || attributes["maxRetransmits"] != 0
            || attributes["label"] != CHANNEL_LABEL
            || attributes["id"] != CHANNEL_ID
            || attributes["negotiated"] != true
        {
            return Err(format!("channel attributes mismatch: {attributes}").into());
        }
        let sctp = pc.sctp().await.ok_or("SCTP absent")?;
        let max_message = sctp.max_message_size().await?;
        if max_message < DATAGRAM_BYTES as u32 {
            return Err("negotiated maxMessageSize too small".into());
        }
        let opened_ms = started.elapsed().as_millis();
        let withhold_reply = request["withholdReply"].as_bool().unwrap_or(false);
        if !withhold_reply {
            tx.try_push(vec![0xa7; DATAGRAM_BYTES])?;
            let payload = tx.try_pop()?.ok_or("TX missing")?;
            dc.try_send(BytesMut::from(payload.as_slice())).await?;
        }
        let mut matched = false;
        loop {
            if !gate.allows(connection, ProtocolScope::Member) {
                return Err("RTC M retired".into());
            }
            match dc.poll().await {
                Some(DataChannelEvent::OnMessage(message))
                    if message.data.as_ref() == b"DONE" && matched =>
                {
                    break;
                }
                Some(DataChannelEvent::OnMessage(message)) => {
                    rx.try_push(message.data.to_vec())?;
                    let payload = rx.try_pop()?.ok_or("RX missing")?;
                    if matched || payload != vec![0x5c; DATAGRAM_BYTES] {
                        return Err("synthetic payload mismatch".into());
                    }
                    matched = true;
                    if withhold_reply {
                        println!(
                            "{}",
                            json!({"event":"rtc-waiting", "phase":"native-received-synthetic-without-reply", "memberStableId":connection.stable_id()})
                        );
                    }
                }
                Some(DataChannelEvent::OnClose) | None => {
                    return Err("channel closed before completion".into());
                }
                _ => {}
            }
        }
        let dtls = sctp.transport();
        let ice = dtls.ice_transport();
        let pair = ice
            .get_selected_candidate_pair()
            .await?
            .ok_or("selected pair absent")?;
        let report = json!({"library":"webrtc 0.21.0","features":["runtime-tokio","crypto-ring"],"iceServers":[],"turnServers":[],"openedMs":opened_ms,"channel":attributes,"maxMessageSize":max_message,"ice":format!("{:?}",ice.state().await?),"dtls":format!("{:?}",dtls.state().await?),"sctp":format!("{:?}",sctp.state().await?),"selectedPair":{"local":pair.local(),"remote":pair.remote()},"matchedBytes":DATAGRAM_BYTES,"sentBytes":DATAGRAM_BYTES,"txPeakBytes":tx.peak_bytes(),"rxPeakBytes":rx.peak_bytes(),"queueMessageLimit":tx.message_limit(),"txPeakMessages":tx.peak_messages(),"rxPeakMessages":rx.peak_messages(),"sendBufferLimit":QUEUE_BYTES,"sctpReceiveBufferLimit":QUEUE_BYTES,"adapterSyntheticPayloadCopies":{"txVecToBytesMut":1,"rxBytesToVec":1,"libraryInternalCopies":"not instrumented"},"outstandingBytes":dc.outstanding_bytes().await?,"events":events.lock().unwrap().clone(),"bindingMatched":true,"memberStableId":connection.stable_id()});
        dc.close().await?;
        Ok::<_, Box<dyn Error + Send + Sync>>(report)
    };
    let outcome = tokio::select! {
        outcome=tokio::time::timeout(Duration::from_secs(55),outcome) => outcome.map_err(|_| "RTC total deadline exceeded".into()).and_then(|result| result),
        _=connection.closed()=>Err("RTC signaling M closed".into()),
    };
    tx.close();
    rx.close();
    let close_result = pc.close().await;
    drop(pc);
    // close aborts its owned driver; allow the executor to drop aborted socket futures.
    tokio::task::yield_now().await;
    let socket_after = sockets();
    let cleanup = json!({"pcClosed":close_result.is_ok(),"txBytes":tx.bytes(),"rxBytes":rx.bytes(),"ownedPumpTasks":0,"socketBefore":socket_before,"socketAfter":socket_after});
    println!(
        "{}",
        json!({"event":"rtc-cleanup","cleanup":cleanup,"failure":outcome.as_ref().err().map(ToString::to_string)})
    );
    let mut report = outcome?;
    close_result?;
    report["cleanup"] = cleanup;
    frame(send, &report).await?;
    send.finish()?;
    println!("{}", json!({"event":"rtc","report":report}));
    Ok(())
}
