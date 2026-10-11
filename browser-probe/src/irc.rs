//! Bounded browser IRC-WSS datagram qualification; no member-data permission.
use crate::{
    error,
    irc_datagram::{self as wire, IrcFacility, Profile, Qualification, Reassembly, Route, Service},
};
use futures_util::future::poll_fn;
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    collections::VecDeque,
    rc::Rc,
    task::{Poll, Waker},
};
use wasm_bindgen::{JsCast, closure::Closure, prelude::*};
use web_sys::{Event, MessageEvent, WebSocket};

#[derive(Deserialize)]
struct Config {
    facility: IrcFacility,
    profile: Profile,
    #[serde(default)]
    disabled: Vec<String>,
}
#[derive(Default, Serialize)]
struct Report {
    ready_ms: Option<f64>,
    withdraw_ms: Option<f64>,
    failure: Option<String>,
    max_line_bytes: usize,
    server_prefix: String,
    received_fragments: Vec<serde_json::Value>,
    sent_fragments: Vec<serde_json::Value>,
    sent_packets: u64,
    received_packets: u64,
    rejected: u64,
    queue_peak: usize,
    queue_age_ms: f64,
    buffered_bytes_peak: u32,
    pong_sent: u64,
    part_sent: bool,
    quit_sent: bool,
    accepted_datagrams: Vec<serde_json::Value>,
    accepted_bytes: u64,
    received_bytes: u64,
}
struct Outbound {
    parts: VecDeque<String>,
    queued: f64,
    started: bool,
}
struct State {
    route: Route,
    service: Service,
    nick: String,
    channel: String,
    profile: Profile,
    bounded_attempt: bool,
    qualification: Qualification,
    assembly: Reassembly,
    incoming: VecDeque<(Route, Vec<u8>)>,
    outgoing: VecDeque<Outbound>,
    closed: bool,
    timer: Option<i32>,
    started: f64,
    last_fragment: Option<f64>,
    waiters: Vec<Waker>,
    report: Report,
    addresses: n0_watcher::Watchable<Vec<iroh_base::CustomAddr>>,
}
fn now() -> f64 {
    web_sys::window()
        .expect("browser Window")
        .performance()
        .expect("browser monotonic clock")
        .now()
}
impl State {
    fn elapsed(&self) -> f64 {
        now() - self.started
    }
    fn wait(&mut self, waker: &Waker) {
        if !self.waiters.iter().any(|old| old.will_wake(waker)) {
            self.waiters.push(waker.clone());
        }
    }
    fn wake(&mut self) {
        for waker in self.waiters.drain(..) {
            waker.wake();
        }
    }
    fn withdraw(&mut self, reason: Option<String>) {
        if self.closed {
            return;
        }
        if let Some(timer) = self.timer.take() {
            if let Some(window) = web_sys::window() {
                window.clear_interval_with_handle(timer);
            }
        }
        self.closed = true;
        let _ = self.addresses.set(Vec::new());
        self.qualification.retire();
        self.assembly.cancel();
        self.incoming.clear();
        self.outgoing.clear();
        self.report.withdraw_ms = Some(self.elapsed());
        self.report.failure = reason;
        self.wake();
    }
    fn line(&mut self, socket: &WebSocket, line: &str) -> Result<(), JsValue> {
        if self.closed || socket.ready_state() != WebSocket::OPEN {
            return Err(error("IRC WSS closed"));
        }
        if line.len() > 512 || socket.buffered_amount().saturating_add(line.len() as u32) > 65536 {
            return Err(error("IRC WSS line or queue limit"));
        }
        socket.send_with_str(line)?;
        self.report.buffered_bytes_peak = self
            .report
            .buffered_bytes_peak
            .max(socket.buffered_amount());
        Ok(())
    }
    fn message(&mut self, socket: &WebSocket, text: &str) -> Result<(), JsValue> {
        if self.closed {
            return Ok(());
        }
        if text.len() > 8192 {
            return Err(error("IRC WSS message limit"));
        }
        for raw in text.split('\n') {
            let raw = raw.strip_suffix('\r').unwrap_or(raw);
            if raw.is_empty() {
                continue;
            }
            let parsed = wire::parse_line(raw).map_err(error)?;
            self.report.max_line_bytes = self.report.max_line_bytes.max(raw.len() + 2);
            if self.report.server_prefix.is_empty() && !parsed.prefix.is_empty() {
                self.report.server_prefix = parsed.prefix.into();
            }
            let was_ready = self.qualification.ready;
            for command in self
                .qualification
                .receive(&parsed, &self.nick, &self.channel)
                .map_err(error)?
            {
                // IRC control writes have priority over the paced datagram queue.
                self.line(socket, &command)?;
                if command.starts_with("PONG ") {
                    self.report.pong_sent += 1;
                }
            }
            if self.qualification.ready && !was_ready {
                self.report.ready_ms = Some(self.elapsed());
                let _ = self
                    .addresses
                    .set(vec![wire::address(self.service, self.route)]);
                self.wake();
            }
            if self.qualification.ready
                && parsed.command == "PRIVMSG"
                && parsed.args.first() == Some(&self.channel.as_str())
            {
                let age = self.elapsed();
                if parsed.trailing.starts_with("MIR2 ")
                    && self.report.received_fragments.len() < 256
                {
                    self.report.received_fragments.push(serde_json::json!({"at_ms":age,"line_bytes":raw.len()+2,"prefix":parsed.prefix,"fragment":parsed.trailing.split(' ').nth(4)}));
                }
                if let Some(packet) = self
                    .assembly
                    .receive(parsed.trailing, self.route, age as u64)
                {
                    if self.incoming.len() < wire::QUEUE_DEPTH {
                        self.report.received_bytes += packet.1.len() as u64;
                        self.incoming.push_back(packet);
                        self.report.received_packets += 1;
                        self.wake();
                    } else {
                        self.report.rejected += 1;
                    }
                }
            }
        }
        Ok(())
    }
    fn tick(&mut self, socket: &WebSocket) -> Result<(), JsValue> {
        if self.closed {
            return Ok(());
        }
        let age = self.elapsed();
        self.assembly.expire(age as u64);
        if self.bounded_attempt && age >= self.profile.attempt_ms as f64 {
            return Err(error("IRC whole attempt deadline"));
        }
        if !self.qualification.ready {
            if age > 7000.0 {
                return Err(error("IRC registration or JOIN deadline"));
            }
            return Ok(());
        }
        let interval = self.qualification.fragment_interval_ms(&self.profile);
        if self
            .last_fragment
            .is_some_and(|last| age - last < interval as f64)
        {
            return Ok(());
        }
        // At most one maximum-sized datagram's fragments per turn. Return to
        // the browser event loop so incoming control and cancellation can run.
        let mut budget = wire::MAX_DATAGRAM.div_ceil(wire::CHUNK);
        while budget > 0 {
            let Some(item) = self.outgoing.front_mut() else {
                break;
            };
            if !item.started && age - item.queued > self.profile.queue_ttl_ms as f64 {
                self.outgoing.pop_front();
                self.report.rejected += 1;
                continue;
            }
            if !item.started {
                item.started = true;
                self.report.queue_age_ms = age - item.queued;
            }
            let fragment = item.parts.pop_front().expect("nonempty fragment queue");
            let done = item.parts.is_empty();
            let line = self
                .qualification
                .privmsg_line(&self.channel, &fragment)
                .map_err(error)?;
            self.line(socket, &line)?;
            if self.report.sent_fragments.len() < 256 {
                self.report
                    .sent_fragments
                    .push(serde_json::json!({"at_ms":age,"fragment":fragment.split(' ').nth(4)}));
            }
            self.last_fragment = Some(age);
            budget -= 1;
            if done {
                self.outgoing.pop_front();
                self.report.sent_packets += 1;
                self.wake();
            }
            if interval > 0 {
                break;
            }
        }
        Ok(())
    }
}

/// Standalone public datagram/lifecycle seam used before IRC QUIC qualification.
#[wasm_bindgen]
pub struct BrowserIrcDatagram {
    socket: WebSocket,
    state: Rc<RefCell<State>>,
    _onopen: Closure<dyn FnMut(Event)>,
    _onmessage: Closure<dyn FnMut(MessageEvent)>,
    _onerror: Closure<dyn FnMut(Event)>,
    _onclose: Closure<dyn FnMut(Event)>,
    _tick: Closure<dyn FnMut()>,
}
#[wasm_bindgen]
impl BrowserIrcDatagram {
    pub fn connect(config: String) -> Result<Self, JsValue> {
        Self::connect_inner(config, true)
    }
    pub async fn wait_ready(&self) -> Result<(), JsValue> {
        poll_fn(|cx| {
            let mut s = self.state.borrow_mut();
            if s.closed {
                return Poll::Ready(Err(error(
                    s.report.failure.as_deref().unwrap_or("IRC closed"),
                )));
            }
            if s.qualification.ready {
                return Poll::Ready(Ok(()));
            }
            s.wait(cx.waker());
            Poll::Pending
        })
        .await
    }
    pub fn route(&self) -> Result<String, JsValue> {
        let s = self.state.borrow();
        if s.closed || !s.qualification.ready {
            return Err(error("IRC not ready"));
        }
        Ok(hex::encode(wire::address(s.service, s.route).data()))
    }
    pub fn send(&self, destination: String, bytes: Vec<u8>) -> Result<(), JsValue> {
        let mut s = self.state.borrow_mut();
        if s.closed || !s.qualification.ready {
            return Err(error("IRC not ready"));
        }
        if s.outgoing.len() >= wire::QUEUE_DEPTH {
            return Err(error("IRC queue full"));
        }
        let address = iroh_base::CustomAddr::from_parts(
            wire::TRANSPORT_ID,
            &hex::decode(destination).map_err(error)?,
        );
        let target = wire::route_from_addr(&address, s.service).map_err(error)?;
        let capacity = s
            .qualification
            .fragment_capacity(&s.channel)
            .map_err(error)?;
        let parts =
            wire::fragments_with_capacity(target, s.route, rand::random(), &bytes, capacity)
                .map_err(error)?;
        let queued = s.elapsed();
        s.report.accepted_bytes += bytes.len() as u64;
        if s.report.accepted_datagrams.len() < 256 {
            use sha2::{Digest, Sha256};
            s.report.accepted_datagrams.push(serde_json::json!({"at_ms":queued,"bytes":bytes.len(),"first_five_bytes":hex::encode(&bytes[..bytes.len().min(5)]),"ciphertext_sha256":hex::encode(Sha256::digest(&bytes))}));
        }
        s.outgoing.push_back(Outbound {
            parts: parts.into(),
            queued,
            started: false,
        });
        s.report.queue_peak = s.report.queue_peak.max(s.outgoing.len());
        Ok(())
    }
    pub async fn receive(&self) -> Result<String, JsValue> {
        poll_fn(|cx|{let mut s=self.state.borrow_mut();if s.closed{return Poll::Ready(Err(error("IRC closed")));}if let Some((source,bytes))=s.incoming.pop_front(){return Poll::Ready(Ok(serde_json::json!({"source":hex::encode(wire::address(s.service,source).data()),"bytes":hex::encode(bytes)}).to_string()));}s.wait(cx.waker());Poll::Pending}).await
    }
    pub fn report(&self) -> String {
        let s = self.state.borrow();
        serde_json::json!({"protocol":s.report,"qualification":s.qualification,"assembly":s.assembly.snapshot(),"queue_pending":s.outgoing.len(),"rx_pending":s.incoming.len(),"waiters":s.waiters.len(),"closed":s.closed,"timer_active":s.timer.is_some(),"socket_state":self.socket.ready_state(),"buffered_bytes":self.socket.buffered_amount()}).to_string()
    }
    pub fn close(&self) {
        let mut s = self.state.borrow_mut();
        if !s.closed && self.socket.ready_state() == WebSocket::OPEN {
            let channel = s.channel.clone();
            s.report.part_sent = s
                .line(
                    &self.socket,
                    &format!("PART {channel} :experiment complete\r\n"),
                )
                .is_ok();
            s.report.quit_sent = s
                .line(&self.socket, "QUIT :experiment complete\r\n")
                .is_ok();
        }
        s.withdraw(None);
        let _ = self.socket.close();
    }
}
impl BrowserIrcDatagram {
    fn connect_inner(config: String, bounded_attempt: bool) -> Result<Self, JsValue> {
        let config: Config = serde_json::from_str(&config).map_err(error)?;
        config
            .facility
            .check_allowed(&config.disabled)
            .map_err(error)?;
        config.profile.validate().map_err(error)?;
        let service = config.facility.service_tag().map_err(error)?;
        let route = rand::random();
        let nick = format!("mio{}", hex::encode(rand::random::<[u8; 8]>()));
        let socket = WebSocket::new_with_str(&config.facility.wss_url, "text.ircv3.net")?;
        let state = Rc::new(RefCell::new(State {
            route,
            service,
            nick,
            channel: config.facility.channel,
            profile: config.profile.clone(),
            bounded_attempt,
            qualification: Qualification::default(),
            assembly: Reassembly::new(config.profile).map_err(error)?,
            incoming: VecDeque::new(),
            outgoing: VecDeque::new(),
            closed: false,
            timer: None,
            started: now(),
            last_fragment: None,
            waiters: vec![],
            report: Report::default(),
            addresses: n0_watcher::Watchable::new(Vec::new()),
        }));
        let opened = state.clone();
        let open_socket = socket.clone();
        let onopen = Closure::new(move |_: Event| {
            let mut s = opened.borrow_mut();
            let result = (|| {
                if s.closed {
                    return Err(error("IRC attempt retired"));
                }
                if open_socket.protocol() != "text.ircv3.net" {
                    return Err(error("IRC WSS subprotocol mismatch"));
                }
                let nick = s.nick.clone();
                s.line(&open_socket, &format!("NICK {nick}\r\n"))?;
                s.line(&open_socket, "USER mio 0 * :miopunch\r\n")
            })();
            if let Err(e) = result {
                s.withdraw(Some(format!("{e:?}")));
                let _ = open_socket.close();
            }
        });
        let received = state.clone();
        let message_socket = socket.clone();
        let onmessage = Closure::new(move |event: MessageEvent| {
            let mut s = received.borrow_mut();
            let result = event
                .data()
                .as_string()
                .ok_or_else(|| error("IRC requires text WSS frames"))
                .and_then(|text| s.message(&message_socket, &text));
            if let Err(e) = result {
                s.withdraw(Some(format!("{e:?}")));
                let _ = message_socket.close();
            }
        });
        let failed = state.clone();
        let onerror = Closure::new(move |_: Event| {
            failed
                .borrow_mut()
                .withdraw(Some("IRC WebSocket error".into()))
        });
        let closed = state.clone();
        let onclose = Closure::new(move |_: Event| {
            closed
                .borrow_mut()
                .withdraw(Some("IRC WebSocket closed".into()))
        });
        let tick_state = state.clone();
        let tick_socket = socket.clone();
        let tick = Closure::new(move || {
            let mut s = tick_state.borrow_mut();
            if let Err(e) = s.tick(&tick_socket) {
                s.withdraw(Some(format!("{e:?}")));
                let _ = tick_socket.close();
            }
        });
        socket.set_onopen(Some(onopen.as_ref().unchecked_ref()));
        socket.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        socket.set_onerror(Some(onerror.as_ref().unchecked_ref()));
        socket.set_onclose(Some(onclose.as_ref().unchecked_ref()));
        let timer = web_sys::window()
            .ok_or_else(|| error("browser Window missing"))?
            .set_interval_with_callback_and_timeout_and_arguments_0(
                tick.as_ref().unchecked_ref(),
                10,
            )?;
        state.borrow_mut().timer = Some(timer);
        Ok(Self {
            socket,
            state,
            _onopen: onopen,
            _onmessage: onmessage,
            _onerror: onerror,
            _onclose: onclose,
            _tick: tick,
        })
    }
}
impl Drop for BrowserIrcDatagram {
    fn drop(&mut self) {
        self.close();
        self.socket.set_onopen(None);
        self.socket.set_onmessage(None);
        self.socket.set_onerror(None);
        self.socket.set_onclose(None);
    }
}

/// The same thread-affine iroh custom transport architecture as MQTT. IRC
/// resources carry restricted control only; the admission gate grants no data.
#[derive(Clone)]
pub(crate) struct BrowserIrcTransport {
    carrier: std::sync::Arc<send_wrapper::SendWrapper<Rc<BrowserIrcDatagram>>>,
    resource: iroh::RelayUrl,
}
impl std::fmt::Debug for BrowserIrcTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserIrcTransport")
            .field("resource", &self.resource)
            .finish_non_exhaustive()
    }
}
impl BrowserIrcTransport {
    pub(crate) fn new(
        facility: &IrcFacility,
        profile: &Profile,
        disabled: &[String],
        bounded_attempt: bool,
    ) -> Result<Self, JsValue> {
        let carrier = BrowserIrcDatagram::connect_inner(
            serde_json::json!({"facility":facility,"profile":profile,"disabled":disabled})
                .to_string(),
            bounded_attempt,
        )?;
        Ok(Self {
            carrier: std::sync::Arc::new(send_wrapper::SendWrapper::new(Rc::new(carrier))),
            resource: facility.wss_url.parse().map_err(error)?,
        })
    }
    pub(crate) async fn wait_ready(&self) -> Result<(), JsValue> {
        self.carrier.wait_ready().await
    }
    pub(crate) fn route(&self) -> Result<iroh_base::CustomAddr, JsValue> {
        Ok(iroh_base::CustomAddr::from_parts(
            wire::TRANSPORT_ID,
            &hex::decode(self.carrier.route()?).map_err(error)?,
        ))
    }
    pub(crate) fn diagnostics(&self) -> String {
        self.carrier.report()
    }
    pub(crate) fn close(&self) {
        self.carrier.close();
    }
}
impl iroh::endpoint::transports::CustomTransport for BrowserIrcTransport {
    fn resource(&self) -> Option<(iroh::RelayUrl, iroh::endpoint::RelayResourceUse)> {
        Some((
            self.resource.clone(),
            iroh::endpoint::RelayResourceUse::Signaling,
        ))
    }
    fn bind(&self) -> std::io::Result<Box<dyn iroh::endpoint::transports::CustomEndpoint>> {
        Ok(Box::new(self.clone()))
    }
}
impl iroh::endpoint::transports::CustomEndpoint for BrowserIrcTransport {
    fn shutdown_handler(
        &self,
    ) -> Option<std::sync::Arc<dyn iroh::endpoint::transports::CustomEndpointShutdown>> {
        Some(std::sync::Arc::new(self.clone()))
    }
    fn watch_local_addrs(&self) -> n0_watcher::Direct<Vec<iroh_base::CustomAddr>> {
        self.carrier.state.borrow().addresses.watch()
    }
    fn create_sender(&self) -> std::sync::Arc<dyn iroh::endpoint::transports::CustomSender> {
        std::sync::Arc::new(self.clone())
    }
    fn poll_recv(
        &mut self,
        cx: &mut std::task::Context,
        bufs: &mut [std::io::IoSliceMut<'_>],
        metas: &mut [noq_udp::RecvMeta],
        infos: &mut [iroh::endpoint::transports::RecvInfo],
    ) -> Poll<std::io::Result<usize>> {
        let mut s = self.carrier.state.borrow_mut();
        if s.closed {
            return Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "IRC carrier closed",
            )));
        }
        if bufs.is_empty() {
            return Poll::Ready(Ok(0));
        }
        while let Some((source, bytes)) = s.incoming.pop_front() {
            if !crate::custom_datagram::copy_received(
                &bytes,
                wire::address(s.service, source),
                wire::address(s.service, s.route),
                &mut bufs[0],
                &mut metas[0],
                &mut infos[0],
            ) {
                continue;
            }
            return Poll::Ready(Ok(1));
        }
        s.wait(cx.waker());
        Poll::Pending
    }
}
impl iroh::endpoint::transports::CustomSender for BrowserIrcTransport {
    fn is_valid_send_addr(&self, addr: &iroh_base::CustomAddr) -> bool {
        wire::route_from_addr(addr, self.carrier.state.borrow().service).is_ok()
    }
    fn poll_send(
        &self,
        _cx: &mut std::task::Context,
        dst: &iroh_base::CustomAddr,
        _src: Option<&iroh_base::CustomAddr>,
        transmit: &iroh::endpoint::transports::Transmit<'_>,
    ) -> Poll<std::io::Result<()>> {
        Poll::Ready(
            self.carrier
                .send(hex::encode(dst.data()), transmit.contents.to_vec())
                .map_err(|e| {
                    std::io::Error::other(e.as_string().unwrap_or_else(|| "IRC send failed".into()))
                }),
        )
    }
}
impl iroh::endpoint::transports::CustomEndpointShutdown for BrowserIrcTransport {
    fn close(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        self.close();
        Box::pin(std::future::ready(()))
    }
}

/// One local fault group; no additional server messages or parallel identity protocol.
#[wasm_bindgen(js_name = ircFaultProbe)]
pub fn fault_probe() -> Result<String, JsValue> {
    let own = [1; 16];
    let source = [2; 16];
    let payload = vec![0x5a; 1200];
    let parts = wire::fragments(own, source, 1, &payload).map_err(error)?;
    let mut missing = Reassembly::new(Profile::slow()).map_err(error)?;
    for part in parts.iter().skip(1) {
        if missing.receive(part, own, 0).is_some() {
            return Err(error("missing fragment delivered"));
        }
    }
    missing.expire(8000);
    let mut conflict = Reassembly::new(Profile::slow()).map_err(error)?;
    conflict.receive(&parts[0], own, 0);
    let changed = wire::fragments(own, source, 1, &[0x5b; 1200])
        .map_err(error)?
        .remove(0);
    conflict.receive(&changed, own, 1);
    for part in &parts {
        if conflict.receive(part, own, 2).is_some() {
            return Err(error("conflicted assembly delivered"));
        }
    }
    let mut cancelled = Reassembly::new(Profile::slow()).map_err(error)?;
    cancelled.receive(&parts[0], own, 0);
    cancelled.cancel();
    Ok(serde_json::json!({"missing":missing.snapshot(),"conflict":conflict.snapshot(),"cancelled":cancelled.snapshot()}).to_string())
}
