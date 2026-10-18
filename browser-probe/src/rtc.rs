//! Native browser RTC API adapter for the bounded synthetic channel experiment.
use super::*;
use crate::rtc_association::{
    CHANNEL_ID, CHANNEL_LABEL, CHANNEL_PROTOCOL, DATAGRAM_BYTES, DATAGRAM_CHANNEL_LABEL,
    DATAGRAM_CHANNEL_PROTOCOL, DatagramQueue, EXPORTER_LABEL, QUEUE_BYTES, attempt_context,
    channel_parameters,
};
use futures_util::{FutureExt, future::poll_fn, task::AtomicWaker};
use js_sys::{Array, Reflect, Uint8Array};
use serde_json::{Value, json};
use std::{cell::RefCell, rc::Rc, task::Poll};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    Event, MessageEvent, RtcConfiguration, RtcDataChannel, RtcDataChannelInit, RtcDataChannelState,
    RtcDataChannelType, RtcIceGatheringState, RtcPeerConnection, RtcPeerConnectionState,
    RtcSdpType, RtcSessionDescriptionInit,
};

pub(super) struct ActiveAttempt(pub(super) Arc<std::sync::atomic::AtomicBool>);
impl Drop for ActiveAttempt {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

pub(super) struct Association {
    pub(super) pc: RtcPeerConnection,
    pub(super) dc: RtcDataChannel,
    tx: Arc<DatagramQueue>,
    rx: Arc<DatagramQueue>,
    callbacks: Vec<Closure<dyn FnMut(Event)>>,
    pub(super) events: Rc<RefCell<Vec<Value>>>,
    pub(super) ice_profile: crate::rtc_ice_profile::RtcIceProfile,
    progress: Rc<AtomicWaker>,
    receiver: Rc<RefCell<Option<Box<dyn Fn(Vec<u8>) -> Result<(), JsValue>>>>>,
    retired: Rc<RefCell<Option<Box<dyn Fn()>>>>,
}
fn event(events: &RefCell<Vec<Value>>, kind: &str, state: String) {
    let mut events = events.borrow_mut();
    if events.len() < 128 {
        events.push(json!({"kind":kind,"state":state,"atMs":js_sys::Date::now()}));
    }
}
impl Association {
    fn new(connection: &(impl AuthenticatedControl + ?Sized)) -> Result<Self, JsValue> {
        Self::with_profile(
            connection,
            false,
            &crate::rtc_ice_profile::RtcIceProfile::default(),
        )
    }
    pub(super) fn datagram(
        connection: &(impl AuthenticatedControl + ?Sized),
        profile: &crate::rtc_ice_profile::RtcIceProfile,
    ) -> Result<Self, JsValue> {
        Self::with_profile(connection, true, profile)
    }
    fn with_profile(
        connection: &(impl AuthenticatedControl + ?Sized),
        datagram: bool,
        profile: &crate::rtc_ice_profile::RtcIceProfile,
    ) -> Result<Self, JsValue> {
        profile.validate().map_err(error)?;
        let maximum = if datagram {
            crate::rtc_transport::MAX_DATAGRAM
        } else {
            DATAGRAM_BYTES
        };
        let config = RtcConfiguration::new();
        let observers = Array::new();
        for url in &profile.stun_servers {
            let server = js_sys::Object::new();
            Reflect::set(
                server.as_ref(),
                &JsValue::from_str("urls"),
                &JsValue::from_str(url),
            )?;
            observers.push(server.as_ref());
        }
        config.set_ice_servers(&observers);
        let pc = RtcPeerConnection::new_with_configuration(&config)?;
        let init = RtcDataChannelInit::new();
        init.set_ordered(false);
        init.set_max_retransmits(0);
        init.set_negotiated(true);
        init.set_id(CHANNEL_ID);
        init.set_protocol(if datagram {
            DATAGRAM_CHANNEL_PROTOCOL
        } else {
            CHANNEL_PROTOCOL
        });
        let dc = pc.create_data_channel_with_data_channel_dict(
            if datagram {
                DATAGRAM_CHANNEL_LABEL
            } else {
                CHANNEL_LABEL
            },
            &init,
        );
        dc.set_binary_type(RtcDataChannelType::Arraybuffer);
        dc.set_buffered_amount_low_threshold((QUEUE_BYTES - maximum) as u32);
        let tx = Arc::new(DatagramQueue::new(QUEUE_BYTES, maximum));
        let rx = Arc::new(DatagramQueue::new(QUEUE_BYTES, maximum));
        let events = Rc::new(RefCell::new(Vec::new()));
        let progress = Rc::new(AtomicWaker::new());
        let mut callbacks = Vec::new();
        let (tx_open, rx_open, dc_open, open_events) =
            (tx.clone(), rx.clone(), dc.clone(), events.clone());
        let open_member = connection.revocation_handle();
        let open_progress = progress.clone();
        let open = Closure::<dyn FnMut(Event)>::new(move |_| {
            if !open_member.is_open() {
                tx_open.close();
                rx_open.close();
                dc_open.close();
                event(&open_events, "lateOpen", "ignored after M close".into());
                return;
            }
            if tx_open.open().is_err() || rx_open.open().is_err() {
                event(&open_events, "lateOpen", "ignored".into());
                dc_open.close();
            } else {
                event(&open_events, "channel", "open".into());
            }
            open_progress.wake();
        });
        dc.set_onopen(Some(open.as_ref().unchecked_ref()));
        callbacks.push(open);
        let receiver: Rc<RefCell<Option<Box<dyn Fn(Vec<u8>) -> Result<(), JsValue>>>>> =
            Rc::new(RefCell::new(None));
        let bridge_receiver = receiver.clone();
        let retired: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let message_retired = retired.clone();
        let (message_rx, message_tx, message_dc, message_events) =
            (rx.clone(), tx.clone(), dc.clone(), events.clone());
        let message = Closure::<dyn FnMut(Event)>::new(move |value: Event| {
            let value: MessageEvent = value.unchecked_into();
            let data = Uint8Array::new(&value.data());
            let accepted = if data.length() as usize > maximum {
                false
            } else if let Some(receiver) = bridge_receiver.borrow().as_ref() {
                receiver(data.to_vec()).is_ok()
            } else {
                message_rx.try_push(data.to_vec()).is_ok()
            };
            if !accepted {
                if let Some(retire) = message_retired.borrow().as_ref() {
                    retire();
                }
                event(&message_events, "receive", "rejected".into());
                message_rx.close();
                message_tx.close();
                message_dc.close();
            }
        });
        dc.set_onmessage(Some(message.as_ref().unchecked_ref()));
        callbacks.push(message);
        let (close_tx, close_rx, close_progress, close_events) =
            (tx.clone(), rx.clone(), progress.clone(), events.clone());
        let close_retired = retired.clone();
        let close = Closure::<dyn FnMut(Event)>::new(move |_| {
            close_tx.close();
            close_rx.close();
            close_progress.wake();
            if let Some(retire) = close_retired.borrow().as_ref() {
                retire();
            }
            event(&close_events, "channel", "closed".into());
        });
        dc.set_onclose(Some(close.as_ref().unchecked_ref()));
        callbacks.push(close);
        let (error_tx, error_rx, error_progress, error_dc, error_events) = (
            tx.clone(),
            rx.clone(),
            progress.clone(),
            dc.clone(),
            events.clone(),
        );
        let failure = Closure::<dyn FnMut(Event)>::new(move |_| {
            error_tx.close();
            error_rx.close();
            error_progress.wake();
            error_dc.close();
            event(&error_events, "channel", "error".into());
        });
        dc.set_onerror(Some(failure.as_ref().unchecked_ref()));
        callbacks.push(failure);
        let low_progress = progress.clone();
        let low = Closure::<dyn FnMut(Event)>::new(move |_| low_progress.wake());
        dc.set_onbufferedamountlow(Some(low.as_ref().unchecked_ref()));
        callbacks.push(low);
        let (state_pc, state_progress, state_events, state_tx, state_rx, state_dc) = (
            pc.clone(),
            progress.clone(),
            events.clone(),
            tx.clone(),
            rx.clone(),
            dc.clone(),
        );
        let states = Closure::<dyn FnMut(Event)>::new(move |_| {
            event(
                &state_events,
                "ice",
                format!("{:?}", state_pc.ice_connection_state()),
            );
            event(
                &state_events,
                "connection",
                format!("{:?}", state_pc.connection_state()),
            );
            if matches!(
                state_pc.connection_state(),
                RtcPeerConnectionState::Failed | RtcPeerConnectionState::Closed
            ) {
                state_tx.close();
                state_rx.close();
                state_dc.close();
            }
            state_progress.wake();
        });
        pc.set_onicegatheringstatechange(Some(states.as_ref().unchecked_ref()));
        pc.set_oniceconnectionstatechange(Some(states.as_ref().unchecked_ref()));
        pc.set_onconnectionstatechange(Some(states.as_ref().unchecked_ref()));
        callbacks.push(states);
        Ok(Self {
            pc,
            dc,
            tx,
            rx,
            callbacks,
            events,
            ice_profile: profile.clone(),
            progress,
            receiver,
            retired,
        })
    }
    pub(super) fn retire(&self) {
        self.tx.close();
        self.rx.close();
        self.progress.wake();
        self.dc.close();
        self.pc.close();
    }
    pub(super) fn set_receiver(
        &self,
        receive: impl Fn(Vec<u8>) -> Result<(), JsValue> + 'static,
        retire: impl Fn() + 'static,
    ) {
        *self.receiver.borrow_mut() = Some(Box::new(receive));
        *self.retired.borrow_mut() = Some(Box::new(retire));
    }
    pub(super) async fn wait_open(&self) -> Result<(), JsValue> {
        poll_fn(|cx| {
            self.progress.register(cx.waker());
            match self.dc.ready_state() {
                RtcDataChannelState::Open => Poll::Ready(Ok(())),
                RtcDataChannelState::Closed | RtcDataChannelState::Closing => {
                    Poll::Ready(Err(error("RTC channel closed before open")))
                }
                _ => Poll::Pending,
            }
        })
        .await
    }
    pub(super) fn poll_capacity(
        &self,
        cx: &mut std::task::Context<'_>,
        size: usize,
    ) -> Poll<Result<(), JsValue>> {
        self.progress.register(cx.waker());
        if self.dc.ready_state() != RtcDataChannelState::Open {
            return Poll::Ready(Err(error("RTC channel closed")));
        }
        if self.dc.buffered_amount() as usize + size <= QUEUE_BYTES {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }
    pub(super) async fn gathered(&self) -> Result<(), JsValue> {
        poll_fn(|cx| {
            self.progress.register(cx.waker());
            if self.pc.ice_gathering_state() == RtcIceGatheringState::Complete {
                Poll::Ready(Ok(()))
            } else if self.dc.ready_state() == RtcDataChannelState::Closed {
                Poll::Ready(Err(error("RTC cancelled")))
            } else {
                Poll::Pending
            }
        })
        .await
    }
    async fn send(&self, payload: Vec<u8>) -> Result<(), JsValue> {
        let size = payload.len();
        poll_fn(|cx| self.tx.poll_capacity(cx, size))
            .await
            .map_err(error)?;
        // The negotiated SCTP maximum is available only after the open event.
        let sctp = Reflect::get(self.pc.as_ref(), &JsValue::from_str("sctp"))?;
        let maximum = Reflect::get(&sctp, &JsValue::from_str("maxMessageSize"))?
            .as_f64()
            .ok_or_else(|| error("RTC maxMessageSize absent"))?;
        if size as f64 > maximum {
            return Err(error("RTC payload exceeds negotiated maxMessageSize"));
        }
        event(&self.events, "sendLimit", maximum.to_string());
        self.tx.try_push(payload).map_err(error)?;
        // One pump owns dequeuing and JS send, including the full-message high water check.
        poll_fn(|cx| {
            self.progress.register(cx.waker());
            if self.dc.ready_state() != RtcDataChannelState::Open {
                return Poll::Ready(Err(error("RTC channel closed")));
            }
            if self.dc.buffered_amount() as usize + size <= QUEUE_BYTES {
                Poll::Ready(Ok(()))
            } else {
                Poll::Pending
            }
        })
        .await?;
        let payload = self
            .tx
            .try_pop()
            .map_err(error)?
            .ok_or_else(|| error("RTC TX retired"))?;
        self.dc.send_with_u8_array(&payload)
    }
    async fn receive(&self) -> Result<Vec<u8>, JsValue> {
        poll_fn(|cx| self.rx.poll_pop(cx)).await.map_err(error)
    }
    pub(super) fn detach(&self) {
        self.dc.set_onopen(None);
        self.dc.set_onmessage(None);
        self.dc.set_onclose(None);
        self.dc.set_onerror(None);
        self.dc.set_onbufferedamountlow(None);
        self.pc.set_onicegatheringstatechange(None);
        self.pc.set_oniceconnectionstatechange(None);
        self.pc.set_onconnectionstatechange(None);
    }
}
impl Drop for Association {
    fn drop(&mut self) {
        self.retire();
        self.dc.set_onopen(None);
        self.dc.set_onmessage(None);
        self.dc.set_onclose(None);
        self.dc.set_onerror(None);
        self.dc.set_onbufferedamountlow(None);
        self.pc.set_onicegatheringstatechange(None);
        self.pc.set_oniceconnectionstatechange(None);
        self.pc.set_onconnectionstatechange(None);
        self.callbacks.clear();
    }
}
async fn read_frame(recv: &mut iroh::endpoint::RecvStream) -> Result<Value, JsValue> {
    crate::rtc_wire::read_frame(recv).await.map_err(error)
}
pub(super) async fn stats(pc: &RtcPeerConnection) -> Result<Value, JsValue> {
    let report = JsFuture::from(pc.get_stats()).await?;
    let values = Rc::new(RefCell::new(Vec::new()));
    let collect_values = values.clone();
    let collect = Closure::<dyn FnMut(JsValue)>::new(move |value| {
        if let Ok(string) = js_sys::JSON::stringify(&value) {
            if let Some(string) = string.as_string() {
                if let Ok(value) = serde_json::from_str::<Value>(&string) {
                    collect_values.borrow_mut().push(value);
                }
            }
        }
    });
    let for_each: js_sys::Function =
        Reflect::get(&report, &JsValue::from_str("forEach"))?.dyn_into()?;
    for_each.call1(&report, collect.as_ref())?;
    let values = values.borrow();
    let transport = values
        .iter()
        .find(|value| value["type"] == "transport")
        .ok_or_else(|| error("RTC transport stats absent"))?;
    let selected_id = &transport["selectedCandidatePairId"];
    let pair = values
        .iter()
        .find(|value| &value["id"] == selected_id)
        .ok_or_else(|| error("RTC selected pair absent"))?;
    let local = values
        .iter()
        .find(|value| value["id"] == pair["localCandidateId"])
        .ok_or_else(|| error("RTC local candidate absent"))?;
    let remote = values
        .iter()
        .find(|value| value["id"] == pair["remoteCandidateId"])
        .ok_or_else(|| error("RTC remote candidate absent"))?;
    if local["candidateType"] == "relay" || remote["candidateType"] == "relay" {
        return Err(error("RTC direct-only fixture selected relay"));
    }
    Ok(
        json!({"transport":transport,"selectedPair":pair,"localCandidate":local,"remoteCandidate":remote,"dataChannels":values.iter().filter(|value|value["type"]=="data-channel").collect::<Vec<_>>()}),
    )
}

#[wasm_bindgen]
impl BrowserMemberConnection {
    /// A synthetic experiment inside this already qualified relay M; not a MIO path upgrade.
    pub async fn rtc_channel_probe(&self, withhold_reply: bool) -> Result<String, JsValue> {
        let saved = self.qualified_snapshot().await?;
        if self.rtc_live.borrow().is_some() {
            return Err(error("RTC association already owned by M"));
        }
        self.rtc_active
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .map_err(|_| error("RTC attempt already active on this M"))?;
        let _active = ActiveAttempt(self.rtc_active.clone());
        if !self.rtc_transport.resource_allowed() {
            return Err(error("RTC adapter is disabled"));
        }
        let association = Association::new(&self.connection)?;
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
            .map_err(|_| error("RTC exporter unavailable"))?;
        let binding = blake3::hash(&exporter).to_hex().to_string();
        let work = async {
            let offer: RtcSessionDescriptionInit = JsFuture::from(association.pc.create_offer())
                .await?
                .unchecked_into();
            JsFuture::from(association.pc.set_local_description(&offer)).await?;
            association.gathered().await?;
            let offer = association
                .pc
                .local_description()
                .ok_or_else(|| error("RTC offer absent"))?;
            let request = json!({"protocol":"miopunch/rtc-probe/1","attempt":attempt,"context":context,"binding":binding,"channel":channel_parameters(),"withholdReply":withhold_reply,"description":{"type":"offer","sdp":offer.sdp()}});
            let (mut send, mut recv) = self.connection.open_bi().await.map_err(error)?;
            send.write_all(&serde_json::to_vec(&request).map_err(error)?)
                .await
                .map_err(error)?;
            send.finish().map_err(error)?;
            let answer = read_frame(&mut recv).await?;
            if answer["binding"] != binding
                || answer["attempt"] != attempt
                || answer["channel"] != channel_parameters()
            {
                return Err(error("RTC answer binding mismatch"));
            }
            let init = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
            init.set_sdp(
                answer["description"]["sdp"]
                    .as_str()
                    .ok_or_else(|| error("RTC answer absent"))?,
            );
            JsFuture::from(association.pc.set_remote_description(&init)).await?;
            association.send(vec![0x5c; DATAGRAM_BYTES]).await?;
            let reply = association.receive().await?;
            if reply != vec![0xa7; DATAGRAM_BYTES] {
                return Err(error("RTC native payload mismatch"));
            }
            // web-sys does not yet expose the W3C SCTP/DTLS/ICE transport objects.
            // Read only these spec properties through Reflect; RTC construction remains web-sys.
            let property =
                |object: &JsValue, name: &str| Reflect::get(object, &JsValue::from_str(name));
            let sctp = property(association.pc.as_ref(), "sctp")?;
            let max_message = property(&sctp, "maxMessageSize")?
                .as_f64()
                .ok_or_else(|| error("RTC maxMessageSize absent"))?;
            if max_message < DATAGRAM_BYTES as f64 {
                return Err(error("RTC maxMessageSize too small"));
            }
            let dtls = property(&sctp, "transport")?;
            let ice = property(&dtls, "iceTransport")?;
            let browser = json!({"iceServers":[],"turnServers":[],"channel":{"ordered":property(association.dc.as_ref(),"ordered")?.as_bool(),"maxRetransmits":association.dc.max_retransmits(),"maxPacketLifeTime":association.dc.max_packet_life_time(),"negotiated":property(association.dc.as_ref(),"negotiated")?.as_bool(),"protocol":property(association.dc.as_ref(),"protocol")?.as_string(),"label":association.dc.label(),"id":association.dc.id(),"readyState":format!("{:?}",association.dc.ready_state())},"ice":property(&ice,"state")?.as_string(),"dtls":property(&dtls,"state")?.as_string(),"sctp":property(&sctp,"state")?.as_string(),"maxMessageSize":max_message,"stats":stats(&association.pc).await?,"txPeakBytes":association.tx.peak_bytes(),"rxPeakBytes":association.rx.peak_bytes(),"queueMessageLimit":association.tx.message_limit(),"txPeakMessages":association.tx.peak_messages(),"rxPeakMessages":association.rx.peak_messages(),"sendBufferLimit":QUEUE_BYTES,"bufferedAmount":association.dc.buffered_amount(),"adapterSyntheticPayloadCopies":{"txWasmMemoryViews":1,"rxJsToWasmCopies":1,"browserInternalCopies":"not instrumented"},"sentBytes":DATAGRAM_BYTES,"matchedBytes":DATAGRAM_BYTES,"events":association.events.borrow().clone(),"bindingMatched":true,"memberStableId":self.connection.stable_id()});
            association.send(b"DONE".to_vec()).await?;
            let native = read_frame(&mut recv).await?;
            Ok::<_, JsValue>(json!({"browser":browser,"native":native}))
        };
        let closed = self.connection.closed().fuse();
        let work = work.fuse();
        futures_util::pin_mut!(closed, work);
        let result = n0_future::time::timeout(std::time::Duration::from_secs(20), async {
            futures_util::select_biased! {
                _ = closed => Err(error("RTC signaling M closed; association cancelled")),
                result = work => result,
            }
        })
        .await
        .map_err(|_| error("RTC 20 second setup/test deadline"))?;
        association.retire();
        let mut result = result?;
        result["cleanup"] = json!({"pcClosed":true,"channelClosed":true,"txBytes":association.tx.bytes(),"rxBytes":association.rx.bytes(),"ownedPumpTasks":0,"callbacksDetachedOnDrop":true});
        serde_json::to_string(&result).map_err(error)
    }
    /// Local failure injection on owned RTC objects; no public traffic or remote peer.
    pub fn rtc_failure_probe(&self) -> Result<String, JsValue> {
        if !self.rtc_transport.resource_allowed() {
            return Err(error("RTC adapter is disabled"));
        }
        let association = Association::new(&self.connection)?;
        association.tx.open().map_err(error)?;
        for _ in 0..54 {
            association.tx.try_push(vec![1; 1200]).map_err(error)?;
        }
        let refused = association.tx.try_push(vec![2; 1200]).is_err();
        association.retire();
        association.dc.dispatch_event(&Event::new("open")?)?;
        let late = association.tx.open().is_err();
        let report = json!({"queueFullRefused":refused,"peakBytes":association.tx.peak_bytes(),"txBytes":association.tx.bytes(),"rxBytes":association.rx.bytes(),"lateOpenIgnored":late,"events":association.events.borrow().clone(),"pcState":format!("{:?}",association.pc.connection_state())});
        serde_json::to_string(&report).map_err(error)
    }
}
