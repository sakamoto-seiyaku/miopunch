//! Browser-native WebSocket MQTT 3.1.1 datagram adapter and iroh control carrier.

use std::{
    cell::RefCell,
    collections::VecDeque,
    rc::Rc,
    task::{Poll, Waker},
};

use futures_util::future::poll_fn;
use iroh_base::CustomAddr;
use serde::{Deserialize, Serialize};
use wasm_bindgen::{JsCast, closure::Closure, prelude::*};
use web_sys::{Event, MessageEvent, WebSocket};

use crate::{
    error,
    mqtt_datagram::{self as wire, MqttFacility, Route, Service},
};

const MAX_MQTT_PACKET: usize = 4096;
const MAX_BUFFERED_BYTES: u32 = 64 * 1024;

#[derive(Deserialize)]
struct Config {
    facility: MqttFacility,
    #[serde(default)]
    disabled: Vec<String>,
}

#[derive(Default, Serialize)]
struct Report {
    generation: u64,
    connack: Option<u8>,
    subscribe_id: Option<u16>,
    suback_id: Option<u16>,
    suback_codes: Vec<u8>,
    ready_ms: Option<f64>,
    withdraw_ms: Option<f64>,
    sent_packets: u64,
    sent_bytes: u64,
    received_packets: u64,
    received_bytes: u64,
    rejected_packets: u64,
    rx_queue_peak: usize,
    buffered_bytes_peak: u32,
    sent_frames: u64,
    sent_frame_bytes: u64,
    received_frames: u64,
    received_frame_bytes: u64,
    unsubscribe_sent: bool,
    disconnect_sent: bool,
    failure: Option<String>,
}

struct State {
    route: Route,
    service: Service,
    qualification: wire::Qualification,
    closed: bool,
    pending: Vec<u8>,
    incoming: VecDeque<(Route, Vec<u8>)>,
    waiters: Vec<Waker>,
    report: Report,
    addresses: n0_watcher::Watchable<Vec<CustomAddr>>,
}

impl State {
    fn wake(&mut self) {
        for waker in self.waiters.drain(..) {
            waker.wake();
        }
    }

    fn wait(&mut self, waker: &Waker) {
        if !self.waiters.iter().any(|old| old.will_wake(waker)) {
            self.waiters.push(waker.clone());
        }
    }

    fn withdraw(&mut self, reason: Option<String>) {
        if self.closed {
            return;
        }
        self.closed = true;
        let _ = self.addresses.set(Vec::new());
        self.qualification.retire();
        self.pending.clear();
        self.incoming.clear();
        self.report
            .withdraw_ms
            .get_or_insert_with(js_sys::Date::now);
        if self.report.failure.is_none() {
            self.report.failure = reason;
        }
        self.wake();
    }

    fn send(&mut self, socket: &WebSocket, packet: &[u8]) -> Result<(), JsValue> {
        if self.closed || socket.ready_state() != WebSocket::OPEN {
            return Err(error("MQTT WebSocket closed"));
        }
        if socket.buffered_amount().saturating_add(packet.len() as u32) > MAX_BUFFERED_BYTES {
            return Err(error("MQTT WebSocket send queue full"));
        }
        socket.send_with_u8_array(packet)?;
        self.report.sent_frames += 1;
        self.report.sent_frame_bytes += packet.len() as u64;
        self.report.buffered_bytes_peak = self
            .report
            .buffered_bytes_peak
            .max(socket.buffered_amount());
        Ok(())
    }

    fn frame(&mut self, socket: &WebSocket, header: u8, body: &[u8]) -> Result<(), JsValue> {
        match header {
            0x20 => {
                if body.len() != 2 || body[0] > 1 {
                    return Err(error("invalid MQTT CONNACK"));
                }
                self.report.connack = Some(body[1]);
                self.qualification
                    .connack(body[1], body[0] != 0)
                    .map_err(error)?;
                self.qualification.subscribe(1).map_err(error)?;
                self.report.subscribe_id = Some(1);
                let mut subscribe = vec![0, 1];
                field(&mut subscribe, &wire::topic(self.route));
                subscribe.push(0);
                self.send(socket, &packet(0x82, &subscribe))?;
            }
            0x90 => {
                if body.len() < 3 {
                    return Err(error("invalid MQTT SUBACK"));
                }
                let id = u16::from_be_bytes([body[0], body[1]]);
                self.report.suback_id = Some(id);
                self.report.suback_codes = body[2..].to_vec();
                self.qualification.suback(id, &body[2..]).map_err(error)?;
                self.report.ready_ms = Some(js_sys::Date::now());
                let _ = self
                    .addresses
                    .set(vec![wire::address_for(Some(self.service), self.route)]);
                self.wake();
            }
            // Restrict the carrier to unretained QoS0 publishes.
            0x30 => {
                if body.len() < 2 {
                    self.report.rejected_packets += 1;
                    return Ok(());
                }
                let topic_len = u16::from_be_bytes([body[0], body[1]]) as usize;
                let end = 2 + topic_len;
                let own_topic = wire::topic(self.route);
                if !self.qualification.ready()
                    || end > body.len()
                    || body[2..end] != *own_topic.as_bytes()
                {
                    self.report.rejected_packets += 1;
                    return Ok(());
                }
                match wire::decode_datagram(&body[end..]) {
                    Ok((source, bytes))
                        if source != self.route && self.incoming.len() < wire::QUEUE_DEPTH =>
                    {
                        self.report.received_packets += 1;
                        self.report.received_bytes += bytes.len() as u64;
                        self.incoming.push_back((source, bytes.to_vec()));
                        self.report.rx_queue_peak =
                            self.report.rx_queue_peak.max(self.incoming.len());
                        self.wake();
                    }
                    _ => self.report.rejected_packets += 1,
                }
            }
            0xd0 if body.is_empty() => {}
            value if value >> 4 == 3 => self.report.rejected_packets += 1,
            _ => return Err(error("unexpected MQTT control packet")),
        }
        Ok(())
    }

    fn message(&mut self, socket: &WebSocket, bytes: &[u8]) -> Result<(), JsValue> {
        if self.closed {
            return Ok(());
        }
        self.report.received_frames += 1;
        self.report.received_frame_bytes += bytes.len() as u64;
        if self.pending.len() + bytes.len() > 2 * MAX_MQTT_PACKET {
            return Err(error("MQTT receive buffer full"));
        }
        self.pending.extend_from_slice(bytes);
        loop {
            let Some((header_len, body_len)) = remaining_length(&self.pending)? else {
                break;
            };
            if header_len + body_len > MAX_MQTT_PACKET {
                return Err(error("MQTT packet too large"));
            }
            if self.pending.len() < header_len + body_len {
                break;
            }
            let header = self.pending[0];
            let frame: Vec<u8> = self.pending.drain(..header_len + body_len).collect();
            self.frame(socket, header, &frame[header_len..])?;
        }
        Ok(())
    }
}

/// A testable carrier boundary. `route` is unavailable until both MQTT ACKs
/// qualify this generation; close/error retires it and wakes all waiters.
#[wasm_bindgen]
pub struct BrowserMqttDatagram {
    socket: WebSocket,
    state: Rc<RefCell<State>>,
    onopen: Closure<dyn FnMut(Event)>,
    onmessage: Closure<dyn FnMut(MessageEvent)>,
    onerror: Closure<dyn FnMut(Event)>,
    onclose: Closure<dyn FnMut(Event)>,
}

#[wasm_bindgen]
impl BrowserMqttDatagram {
    pub fn connect(config: String) -> Result<Self, JsValue> {
        let config: Config = serde_json::from_str(&config).map_err(error)?;
        config
            .facility
            .check_allowed(&config.disabled)
            .map_err(error)?;
        let service = config.facility.service_tag().map_err(error)?;
        let route: Route = rand::random();
        let socket = WebSocket::new_with_str(&config.facility.wss_url, "mqtt")?;
        socket.set_binary_type(web_sys::BinaryType::Arraybuffer);
        let state = Rc::new(RefCell::new(State {
            route,
            service,
            qualification: wire::Qualification::default(),
            closed: false,
            pending: Vec::new(),
            incoming: VecDeque::new(),
            waiters: Vec::new(),
            report: Report {
                generation: rand::random::<u32>() as u64,
                ..Report::default()
            },
            addresses: n0_watcher::Watchable::new(Vec::new()),
        }));
        let opened = state.clone();
        let open_socket = socket.clone();
        let onopen = Closure::new(move |_: Event| {
            let mut state = opened.borrow_mut();
            if state.closed {
                let _ = open_socket.close();
                return;
            }
            let result = (|| {
                if open_socket.protocol() != "mqtt" {
                    return Err(error("MQTT WSS subprotocol mismatch"));
                }
                let mut body = Vec::new();
                field(&mut body, "MQTT");
                // MQTT 3.1.1, clean session, no idle keepalive for this bounded
                // carrier. Caller owns the attempt deadline and closes it.
                body.extend_from_slice(&[4, 2, 0, 0]);
                field(
                    &mut body,
                    &format!("mio-browser-{}", hex::encode(state.route)),
                );
                state.send(&open_socket, &packet(0x10, &body))
            })();
            if let Err(err) = result {
                state.withdraw(Some(format!("{err:?}")));
                let _ = open_socket.close();
            }
        });
        let received = state.clone();
        let message_socket = socket.clone();
        let onmessage = Closure::new(move |event: MessageEvent| {
            let mut state = received.borrow_mut();
            let result = if event.data().is_instance_of::<js_sys::ArrayBuffer>() {
                state.message(
                    &message_socket,
                    &js_sys::Uint8Array::new(&event.data()).to_vec(),
                )
            } else {
                Err(error("nonbinary MQTT WebSocket message"))
            };
            if let Err(err) = result {
                state.withdraw(Some(format!("{err:?}")));
                let _ = message_socket.close();
            }
        });
        let failed = state.clone();
        let onerror = Closure::new(move |_: Event| {
            failed
                .borrow_mut()
                .withdraw(Some("MQTT WebSocket error".into()))
        });
        let closed = state.clone();
        let onclose = Closure::new(move |_: Event| {
            closed
                .borrow_mut()
                .withdraw(Some("MQTT WebSocket closed".into()))
        });
        socket.set_onopen(Some(onopen.as_ref().unchecked_ref()));
        socket.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        socket.set_onerror(Some(onerror.as_ref().unchecked_ref()));
        socket.set_onclose(Some(onclose.as_ref().unchecked_ref()));
        Ok(Self {
            socket,
            state,
            onopen,
            onmessage,
            onerror,
            onclose,
        })
    }

    pub async fn wait_ready(&self) -> Result<(), JsValue> {
        poll_fn(|cx| {
            let mut state = self.state.borrow_mut();
            if state.closed {
                return Poll::Ready(Err(error(
                    state.report.failure.as_deref().unwrap_or("MQTT closed"),
                )));
            }
            if state.qualification.ready() {
                return Poll::Ready(Ok(()));
            }
            state.wait(cx.waker());
            Poll::Pending
        })
        .await
    }

    pub fn route(&self) -> Result<String, JsValue> {
        let state = self.state.borrow();
        if !state.qualification.ready() || state.closed {
            return Err(error("MQTT subscription not ready"));
        }
        Ok(hex::encode(
            wire::address_for(Some(state.service), state.route).data(),
        ))
    }

    pub fn send(&self, destination: String, bytes: Vec<u8>) -> Result<(), JsValue> {
        let mut state = self.state.borrow_mut();
        if !state.qualification.ready() || state.closed {
            return Err(error("MQTT subscription not ready"));
        }
        let destination = hex::decode(destination).map_err(error)?;
        let address = CustomAddr::from_parts(wire::TRANSPORT_ID, &destination);
        let target = wire::route_from_addr(&address, Some(state.service)).map_err(error)?;
        let frame = wire::encode_datagram(state.route, &bytes).map_err(error)?;
        let mut body = Vec::new();
        field(&mut body, &wire::topic(target));
        body.extend_from_slice(&frame);
        state.send(&self.socket, &packet(0x30, &body))?;
        state.report.sent_packets += 1;
        state.report.sent_bytes += bytes.len() as u64;
        Ok(())
    }

    /// JSON keeps the test boundary independent from a future QUIC interface.
    pub async fn receive(&self) -> Result<String, JsValue> {
        poll_fn(|cx| {
            let mut state = self.state.borrow_mut();
            if state.closed {
                return Poll::Ready(Err(error("MQTT WebSocket closed")));
            }
            if let Some((source, bytes)) = state.incoming.pop_front() {
                return Poll::Ready(Ok(serde_json::json!({
                    "source": hex::encode(wire::address_for(Some(state.service), source).data()),
                    "bytes": hex::encode(bytes),
                })
                .to_string()));
            }
            state.wait(cx.waker());
            Poll::Pending
        })
        .await
    }

    pub fn diagnostics(&self) -> String {
        let state = self.state.borrow();
        serde_json::json!({"ready": state.qualification.ready(), "closed": state.closed,
            "route": if state.qualification.ready() { Some(hex::encode(wire::address_for(Some(state.service), state.route).data())) } else { None },
            "queued_packets": state.incoming.len(), "pending_bytes": state.pending.len(),
            "buffered_bytes": self.socket.buffered_amount(), "protocol": self.socket.protocol(),
            "traffic": state.report}).to_string()
    }

    pub fn close(&self) {
        let mut state = self.state.borrow_mut();
        if !state.closed
            && state.qualification.connected()
            && self.socket.ready_state() == WebSocket::OPEN
        {
            let mut body = vec![0, 2];
            field(&mut body, &wire::topic(state.route));
            state.report.unsubscribe_sent = state.send(&self.socket, &packet(0xa2, &body)).is_ok();
            state.report.disconnect_sent = state.send(&self.socket, &[0xe0, 0]).is_ok();
        }
        state.withdraw(None);
        // Endpoint/transport clones can keep this carrier alive after close.
        // Retire its callbacks here; Drop still keeps their owners alive until
        // every binding has been removed.
        self.socket.set_onopen(None);
        self.socket.set_onmessage(None);
        self.socket.set_onerror(None);
        self.socket.set_onclose(None);
        let _ = self.socket.close();
    }
}

impl Drop for BrowserMqttDatagram {
    fn drop(&mut self) {
        self.close();
        self.socket.set_onopen(None);
        self.socket.set_onmessage(None);
        self.socket.set_onerror(None);
        self.socket.set_onclose(None);
        // Keep callback owners alive until after all bindings are removed.
        let _ = (&self.onopen, &self.onmessage, &self.onerror, &self.onclose);
    }
}

/// iroh requires Send/Sync transport objects even on its single-threaded WASM
/// runtime. SendWrapper checks access and destruction remain on the JS thread;
/// no browser socket or callback is moved to a worker.
#[derive(Clone)]
pub(crate) struct BrowserMqttTransport {
    carrier: std::sync::Arc<send_wrapper::SendWrapper<Rc<BrowserMqttDatagram>>>,
    resource: iroh::RelayUrl,
}

impl std::fmt::Debug for BrowserMqttTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserMqttTransport")
            .field("resource", &self.resource)
            .finish_non_exhaustive()
    }
}

impl BrowserMqttTransport {
    pub(crate) fn new(facility: &MqttFacility, disabled: &[String]) -> Result<Self, JsValue> {
        let carrier = BrowserMqttDatagram::connect(
            serde_json::json!({"facility":facility,"disabled":disabled}).to_string(),
        )?;
        Ok(Self {
            carrier: std::sync::Arc::new(send_wrapper::SendWrapper::new(Rc::new(carrier))),
            resource: facility.wss_url.parse().map_err(error)?,
        })
    }
    pub(crate) async fn wait_ready(&self) -> Result<(), JsValue> {
        self.carrier.wait_ready().await
    }
    pub(crate) fn route(&self) -> Result<CustomAddr, JsValue> {
        Ok(CustomAddr::from_parts(
            wire::TRANSPORT_ID,
            &hex::decode(self.carrier.route()?).map_err(error)?,
        ))
    }
    pub(crate) fn diagnostics(&self) -> String {
        self.carrier.diagnostics()
    }
    pub(crate) fn close(&self) {
        self.carrier.close();
    }
}

impl iroh::endpoint::transports::CustomTransport for BrowserMqttTransport {
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

impl iroh::endpoint::transports::CustomEndpoint for BrowserMqttTransport {
    fn shutdown_handler(
        &self,
    ) -> Option<std::sync::Arc<dyn iroh::endpoint::transports::CustomEndpointShutdown>> {
        Some(std::sync::Arc::new(self.clone()))
    }
    fn watch_local_addrs(&self) -> n0_watcher::Direct<Vec<CustomAddr>> {
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
        let mut state = self.carrier.state.borrow_mut();
        if state.closed {
            return Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "MQTT carrier closed",
            )));
        }
        if bufs.is_empty() {
            return Poll::Ready(Ok(0));
        }
        while let Some((source, bytes)) = state.incoming.pop_front() {
            if !crate::custom_datagram::copy_received(
                &bytes,
                wire::address_for(Some(state.service), source),
                wire::address_for(Some(state.service), state.route),
                &mut bufs[0],
                &mut metas[0],
                &mut infos[0],
            ) {
                continue;
            }
            return Poll::Ready(Ok(1));
        }
        state.wait(cx.waker());
        Poll::Pending
    }
}

impl iroh::endpoint::transports::CustomSender for BrowserMqttTransport {
    fn is_valid_send_addr(&self, addr: &CustomAddr) -> bool {
        wire::route_from_addr(addr, Some(self.carrier.state.borrow().service)).is_ok()
    }
    fn poll_send(
        &self,
        _cx: &mut std::task::Context,
        dst: &CustomAddr,
        _src: Option<&CustomAddr>,
        transmit: &iroh::endpoint::transports::Transmit<'_>,
    ) -> Poll<std::io::Result<()>> {
        Poll::Ready(
            self.carrier
                .send(hex::encode(dst.data()), transmit.contents.to_vec())
                .map_err(|error| {
                    std::io::Error::other(
                        error
                            .as_string()
                            .unwrap_or_else(|| "MQTT send failed".into()),
                    )
                }),
        )
    }
}

impl iroh::endpoint::transports::CustomEndpointShutdown for BrowserMqttTransport {
    fn close(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        self.close();
        Box::pin(std::future::ready(()))
    }
}

fn field(body: &mut Vec<u8>, value: &str) {
    body.extend_from_slice(&(value.len() as u16).to_be_bytes());
    body.extend_from_slice(value.as_bytes());
}

fn packet(header: u8, body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![header];
    let mut remaining = body.len();
    loop {
        let mut digit = (remaining % 128) as u8;
        remaining /= 128;
        if remaining != 0 {
            digit |= 128;
        }
        bytes.push(digit);
        if remaining == 0 {
            break;
        }
    }
    bytes.extend_from_slice(body);
    bytes
}

fn remaining_length(bytes: &[u8]) -> Result<Option<(usize, usize)>, JsValue> {
    if bytes.len() < 2 {
        return Ok(None);
    }
    let mut length = 0;
    for offset in 1..=4 {
        let Some(&digit) = bytes.get(offset) else {
            return Ok(None);
        };
        length += ((digit & 127) as usize) << (7 * (offset - 1));
        if digit & 128 == 0 {
            return Ok(Some((offset + 1, length)));
        }
    }
    Err(error("malformed MQTT remaining length"))
}
