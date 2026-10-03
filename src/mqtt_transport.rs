//! A bounded MQTT packet carrier for restricted endpoint bootstrap. Each publish
//! carries exactly one QUIC datagram. Topics and route tokens are public.

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use futures_channel::mpsc;
use futures_util::{Sink, StreamExt};
use iroh::endpoint::transports::{
    CustomEndpoint, CustomSender, CustomTransport, RecvInfo, Transmit,
};
use iroh_base::CustomAddr;
use n0_watcher::{Direct, Watchable};
use rumqttc::mqttbytes::v4::SubscribeReasonCode;
use rumqttc::{AsyncClient, Event, MqttOptions, Outgoing, Packet as MqttPacket, QoS};
use sha2::{Digest, Sha256};
use tokio::sync::watch;

pub use crate::mqtt_datagram::TRANSPORT_ID;
use crate::mqtt_datagram::{
    MAX_DATAGRAM, MqttFacility, QUEUE_DEPTH, Qualification, Route, Service, address_for,
    decode_datagram, encode_datagram, route_from_addr, topic,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Starting,
    Ready,
    Closed,
}

pub(crate) fn service_tag(resource: &iroh::RelayUrl) -> io::Result<[u8; 16]> {
    let origin = crate::resources::resource_origin(resource.as_str())
        .filter(|_| resource.scheme() == "mqtt")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid MQTT resource"))?;
    let mut hash = Sha256::new();
    hash.update(b"miopunch/mqtt-service/1\0");
    hash.update(origin);
    Ok(hash.finalize()[..16].try_into().unwrap())
}

#[derive(Debug, Default)]
pub struct Traffic {
    sent_packets: AtomicU64,
    sent_bytes: AtomicU64,
    received_packets: AtomicU64,
    received_bytes: AtomicU64,
    rejected_packets: AtomicU64,
    generation: AtomicU64,
    current_binding: Mutex<Arc<BindingReport>>,
}

#[derive(Debug, Default)]
struct BindingReport {
    generation: u64,
    queued_packets: AtomicU64,
    queue_peak: AtomicU64,
    rx_queue_peak: AtomicU64,
    protocol: Mutex<ProtocolReport>,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
struct ProtocolReport {
    connack: Option<u8>,
    subscribe_id: Option<u16>,
    suback_id: Option<u16>,
    suback_codes: Vec<u8>,
    ready_ms: Option<u64>,
    withdraw_ms: Option<u64>,
    unsubscribe_sent: bool,
    disconnect_sent: bool,
    failure: Option<String>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

impl Traffic {
    pub fn snapshot(&self) -> serde_json::Value {
        let binding = self.current_binding.lock().unwrap().clone();
        serde_json::json!({
            "generation": binding.generation,
            "sent_packets": self.sent_packets.load(Ordering::Relaxed),
            "sent_bytes": self.sent_bytes.load(Ordering::Relaxed),
            "received_packets": self.received_packets.load(Ordering::Relaxed),
            "received_bytes": self.received_bytes.load(Ordering::Relaxed),
            "rejected_packets": self.rejected_packets.load(Ordering::Relaxed),
            "queue_peak": binding.queue_peak.load(Ordering::Relaxed),
            "rx_queue_peak": binding.rx_queue_peak.load(Ordering::Relaxed),
            "queued_packets": binding.queued_packets.load(Ordering::Relaxed),
            "protocol": binding.protocol.lock().unwrap().clone(),
            "max_datagram": MAX_DATAGRAM,
            "queue_depth": QUEUE_DEPTH,
        })
    }

    /// Actual datagrams handed to the MQTT publish queue by this endpoint.
    pub fn sent_packets(&self) -> u64 {
        self.sent_packets.load(Ordering::Relaxed)
    }

    /// Datagrams accepted by the adapter before QUIC's connection policy.
    pub fn received_packets(&self) -> u64 {
        self.received_packets.load(Ordering::Relaxed)
    }

    pub fn report(&self) -> String {
        format!(
            "MQTT_TRAFFIC sent_packets={} sent_bytes={} received_packets={} received_bytes={} rejected_packets={} max_datagram={} queue_depth={}",
            self.sent_packets.load(Ordering::Relaxed),
            self.sent_bytes.load(Ordering::Relaxed),
            self.received_packets.load(Ordering::Relaxed),
            self.received_bytes.load(Ordering::Relaxed),
            self.rejected_packets.load(Ordering::Relaxed),
            MAX_DATAGRAM,
            QUEUE_DEPTH,
        )
    }
}

impl BindingReport {
    fn withdraw(&self) {
        self.protocol
            .lock()
            .unwrap()
            .withdraw_ms
            .get_or_insert_with(now_ms);
    }
}

#[derive(Debug)]
pub struct MqttTransport {
    resource: iroh::RelayUrl,
    service: Option<Service>,
    binding: Mutex<Option<Binding>>,
    traffic: Arc<Traffic>,
    tls_ca: Option<Vec<u8>>,
}

#[derive(Debug)]
struct Binding {
    route: Route,
    status: watch::Sender<Status>,
}

impl MqttTransport {
    /// Standalone access to the same carrier used by CustomTransport, for
    /// datagram qualification without constructing a QUIC session.
    pub fn bind_datagrams(&self) -> io::Result<MqttDatagram> {
        Ok(MqttDatagram {
            endpoint: self.bind_endpoint()?,
        })
    }
    /// Read-only diagnostics for the configured carrier; no private payload.
    pub fn diagnostics(&self) -> (String, bool, u64, u64) {
        let ready = self
            .binding
            .lock()
            .expect("MQTT binding")
            .as_ref()
            .is_some_and(|binding| *binding.status.borrow() == Status::Ready);
        (
            self.resource.to_string(),
            ready,
            self.traffic.sent_packets(),
            self.traffic.received_packets(),
        )
    }
    pub fn new(broker: SocketAddr, traffic: Arc<Traffic>) -> Self {
        Self {
            resource: format!("mqtt://{broker}/")
                .parse()
                .expect("socket-address MQTT URI"),
            service: None,
            binding: Mutex::new(None),
            traffic,
            tls_ca: None,
        }
    }

    /// Configured services qualify their broker-local route with a public
    /// origin tag. This identifies a sender, never a node or authentication key.
    pub(crate) fn from_resource(
        resource: iroh::RelayUrl,
        traffic: Arc<Traffic>,
    ) -> io::Result<Self> {
        let service = service_tag(&resource)?;
        Ok(Self {
            resource,
            service: Some(service),
            binding: Mutex::new(None),
            traffic,
            tls_ca: None,
        })
    }

    /// Use an explicit trusted native/WSS mapping. Both entries share the
    /// logical service tag; URL spelling and host equality never infer aliases.
    pub fn from_facility(
        facility: &MqttFacility,
        disabled: &[String],
        traffic: Arc<Traffic>,
        tls_ca: Option<Vec<u8>>,
    ) -> io::Result<Self> {
        facility.check_allowed(disabled)?;
        Ok(Self {
            resource: facility.native_url.parse().map_err(io::Error::other)?,
            service: Some(facility.service_tag()?),
            binding: Mutex::new(None),
            traffic,
            tls_ca,
        })
    }

    pub fn route(&self) -> io::Result<String> {
        let binding = self.binding()?;
        if *binding.status.borrow() != Status::Ready {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "MQTT subscription not ready",
            ));
        }
        Ok(hex::encode(address_for(self.service, binding.route).data()))
    }

    fn binding(&self) -> io::Result<Binding> {
        let binding = self.binding.lock().unwrap();
        binding
            .as_ref()
            .map(|binding| Binding {
                route: binding.route,
                status: binding.status.clone(),
            })
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "MQTT carrier not bound"))
    }

    pub fn address(route_hex: &str) -> io::Result<CustomAddr> {
        let bytes = hex::decode(route_hex)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid MQTT route"))?;
        if !matches!(bytes.len(), 16 | 32) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid MQTT route length",
            ));
        }
        Ok(CustomAddr::from_parts(TRANSPORT_ID, &bytes))
    }

    pub async fn wait_ready(&self) -> io::Result<()> {
        let mut status = self.binding()?.status.subscribe();
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                match *status.borrow() {
                    Status::Ready => return Ok(()),
                    Status::Closed => {
                        return Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            "MQTT carrier stopped",
                        ));
                    }
                    Status::Starting => {}
                }
                status.changed().await.map_err(|_| {
                    io::Error::new(io::ErrorKind::BrokenPipe, "MQTT carrier stopped")
                })?;
            }
        })
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "MQTT subscribe timeout"))?
    }

    pub async fn wait_closed(&self) -> io::Result<()> {
        let mut status = self.binding()?.status.subscribe();
        while *status.borrow() != Status::Closed {
            if status.changed().await.is_err() {
                break;
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
struct Packet {
    source: Route,
    bytes: Vec<u8>,
}

#[derive(Debug)]
struct Outbound {
    target: Route,
    bytes: Vec<u8>,
}

#[derive(Debug)]
struct Shared {
    route: Route,
    service: Option<Service>,
    outbound: Mutex<mpsc::Sender<Outbound>>,
    status: watch::Sender<Status>,
    shutdown: watch::Sender<bool>,
    traffic: Arc<Traffic>,
    report: Arc<BindingReport>,
}

#[derive(Debug)]
struct MqttEndpoint {
    shared: Arc<Shared>,
    receiver: tokio::sync::mpsc::Receiver<Packet>,
    addresses: Watchable<Vec<CustomAddr>>,
    status: watch::Sender<Status>,
}

pub struct MqttDatagram {
    endpoint: MqttEndpoint,
}

impl MqttDatagram {
    pub fn local_addresses(&self) -> Vec<CustomAddr> {
        self.endpoint.addresses.get()
    }
    pub async fn send(&self, target: &CustomAddr, bytes: &[u8]) -> io::Result<()> {
        futures_util::future::poll_fn(|cx| self.endpoint.shared.poll_datagram(cx, target, bytes))
            .await
    }

    pub async fn receive(&mut self) -> io::Result<(CustomAddr, Vec<u8>)> {
        let packet = self
            .endpoint
            .receiver
            .recv()
            .await
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "MQTT carrier closed"))?;
        Ok((
            address_for(self.endpoint.shared.service, packet.source),
            packet.bytes,
        ))
    }
}

impl Drop for MqttEndpoint {
    fn drop(&mut self) {
        self.shared.shutdown.send_replace(true);
        self.addresses.set(Vec::new()).ok();
        self.shared.report.withdraw();
        self.status.send_replace(Status::Closed);
    }
}

impl CustomTransport for MqttTransport {
    fn resource(&self) -> Option<(iroh::RelayUrl, iroh::endpoint::RelayResourceUse)> {
        Some((
            self.resource.clone(),
            iroh::endpoint::RelayResourceUse::Signaling,
        ))
    }
    fn bind(&self) -> io::Result<Box<dyn CustomEndpoint>> {
        Ok(Box::new(self.bind_endpoint()?))
    }
}

impl MqttTransport {
    fn bind_endpoint(&self) -> io::Result<MqttEndpoint> {
        let route: Route = rand::random();
        let (status, _) = watch::channel(Status::Starting);
        let (shutdown, stop) = watch::channel(false);
        {
            let mut binding = self.binding.lock().unwrap();
            if binding
                .as_ref()
                .is_some_and(|binding| *binding.status.borrow() != Status::Closed)
            {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "MQTT carrier already bound",
                ));
            }
            *binding = Some(Binding {
                route,
                status: status.clone(),
            });
        }
        let (outbound, queue) = mpsc::channel(QUEUE_DEPTH);
        let (incoming, receiver) = tokio::sync::mpsc::channel(QUEUE_DEPTH);
        let report = Arc::new(BindingReport {
            generation: self.traffic.generation.fetch_add(1, Ordering::Relaxed) + 1,
            ..BindingReport::default()
        });
        *self.traffic.current_binding.lock().unwrap() = report.clone();
        let shared = Arc::new(Shared {
            route,
            service: self.service,
            outbound: Mutex::new(outbound),
            status: status.clone(),
            shutdown,
            traffic: self.traffic.clone(),
            report: report.clone(),
        });
        let resource = self.resource.clone();
        let traffic = self.traffic.clone();
        let task_status = status.clone();
        let addresses = Watchable::new(Vec::new());
        let task_addresses = addresses.clone();
        let service = self.service;
        let tls_ca = self.tls_ca.clone();
        tokio::spawn(async move {
            if let Err(err) = drive(
                resource,
                route,
                service,
                task_addresses.clone(),
                queue,
                &incoming,
                task_status.clone(),
                traffic.clone(),
                report.clone(),
                tls_ca,
                stop,
            )
            .await
            {
                eprintln!("MQTT_CLOSE result={err}");
                report.protocol.lock().unwrap().failure = Some(err.to_string());
            }
            task_addresses.set(Vec::new()).ok();
            report.queued_packets.store(0, Ordering::Relaxed);
            report.withdraw();
            task_status.send_replace(Status::Closed);
            // Keep the RX sender alive until route retirement is observable.
            // Otherwise poll_recv can report closure while route() is still ready.
            drop(incoming);
        });
        Ok(MqttEndpoint {
            shared,
            receiver,
            addresses,
            status,
        })
    }
}

async fn drive(
    resource: iroh::RelayUrl,
    route: Route,
    service: Option<Service>,
    addresses: Watchable<Vec<CustomAddr>>,
    mut queue: mpsc::Receiver<Outbound>,
    incoming: &tokio::sync::mpsc::Sender<Packet>,
    status: watch::Sender<Status>,
    traffic: Arc<Traffic>,
    report: Arc<BindingReport>,
    tls_ca: Option<Vec<u8>>,
    mut stop: watch::Receiver<bool>,
) -> io::Result<()> {
    let mut options = MqttOptions::new(
        format!("mio20-{}", hex::encode(route)),
        resource
            .host_str()
            .expect("validated MQTT host")
            .trim_start_matches('[')
            .trim_end_matches(']'),
        resource.port().unwrap_or(if resource.scheme() == "mqtts" {
            8883
        } else {
            1883
        }),
    );
    if resource.scheme() == "mqtts" {
        options.set_transport(match tls_ca {
            Some(ca) => rumqttc::Transport::tls(ca, None, None),
            None => rumqttc::Transport::tls_with_default_config(),
        });
    }
    options.set_keep_alive(Duration::from_secs(5));
    options.set_clean_session(true);
    options.set_max_packet_size(4096, 4096);
    let (client, mut eventloop) = AsyncClient::new(options, QUEUE_DEPTH);
    client
        .subscribe(topic(route), QoS::AtMostOnce)
        .await
        .map_err(io::Error::other)?;
    let own_topic = topic(route);
    let mut qualification = Qualification::default();
    loop {
        tokio::select! {
            biased;
            _ = stop.changed() => {
                addresses.set(Vec::new()).ok();
                report.withdraw();
                status.send_replace(Status::Closed);
                let _ = client.try_unsubscribe(own_topic.clone());
                let _ = client.try_disconnect();
                let _ = tokio::time::timeout(Duration::from_millis(500), async {
                    loop {
                        match eventloop.poll().await {
                            Ok(Event::Outgoing(Outgoing::Unsubscribe(_))) => report.protocol.lock().unwrap().unsubscribe_sent = true,
                            Ok(Event::Outgoing(Outgoing::Disconnect)) => {
                                report.protocol.lock().unwrap().disconnect_sent = true;
                                break;
                            }
                            Err(_) => break,
                            _ => {}
                        }
                    }
                }).await;
                return Ok(());
            }
            event = eventloop.poll() => {
                match event.map_err(io::Error::other)? {
                    Event::Incoming(MqttPacket::ConnAck(ack)) => {
                        report.protocol.lock().unwrap().connack = Some(ack.code as u8);
                        qualification.connack(ack.code as u8, ack.session_present)?;
                    }
                    Event::Outgoing(Outgoing::Subscribe(pkid)) => {
                        qualification.subscribe(pkid)?;
                        report.protocol.lock().unwrap().subscribe_id = Some(pkid);
                    }
                    Event::Incoming(MqttPacket::SubAck(ack)) => {
                        {
                            let mut protocol = report.protocol.lock().unwrap();
                            protocol.suback_id = Some(ack.pkid);
                            protocol.suback_codes = ack.return_codes.iter().map(|code| match code {
                                SubscribeReasonCode::Success(qos) => *qos as u8,
                                SubscribeReasonCode::Failure => 128,
                            }).collect();
                        }
                        qualification.suback(ack.pkid, &report.protocol.lock().unwrap().suback_codes)?;
                        report.protocol.lock().unwrap().ready_ms = Some(now_ms());
                        addresses.set(vec![address_for(service, route)]).ok();
                        status.send_replace(Status::Ready);
                    }
                    Event::Incoming(MqttPacket::Publish(publish)) => {
                        let payload = publish.payload.as_ref();
                        let decoded = decode_datagram(payload);
                        if !qualification.ready() || publish.topic != own_topic || publish.retain || publish.qos != QoS::AtMostOnce || decoded.is_err() {
                            traffic.rejected_packets.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                        let (source, bytes) = decoded.unwrap();
                        if source == route {
                            traffic.rejected_packets.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                        let bytes = bytes.to_vec();
                        let len = bytes.len();
                        if incoming.try_send(Packet { source, bytes }).is_ok() {
                            report.rx_queue_peak.fetch_max((QUEUE_DEPTH - incoming.capacity()).max(1) as u64, Ordering::Relaxed);
                            traffic.received_packets.fetch_add(1, Ordering::Relaxed);
                            traffic.received_bytes.fetch_add(len as u64, Ordering::Relaxed);
                        } else {
                            traffic.rejected_packets.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    _ => {}
                }
            }
            item = queue.next(), if qualification.ready() => {
                let Some(item) = item else { return Ok(()); };
                report.queued_packets.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |depth| Some(depth.saturating_sub(1))).ok();
                let payload = encode_datagram(route, &item.bytes)?;
                client.try_publish(topic(item.target), QoS::AtMostOnce, false, payload)
                    .map_err(io::Error::other)?;
                traffic.sent_packets.fetch_add(1, Ordering::Relaxed);
                traffic.sent_bytes.fetch_add(item.bytes.len() as u64, Ordering::Relaxed);
            }
        }
    }
}

impl CustomEndpoint for MqttEndpoint {
    fn watch_local_addrs(&self) -> Direct<Vec<CustomAddr>> {
        self.addresses.watch()
    }

    fn create_sender(&self) -> Arc<dyn CustomSender> {
        Arc::new(MqttSender(self.shared.clone()))
    }

    fn poll_recv(
        &mut self,
        cx: &mut Context,
        bufs: &mut [io::IoSliceMut<'_>],
        metas: &mut [noq_udp::RecvMeta],
        infos: &mut [RecvInfo],
    ) -> Poll<io::Result<usize>> {
        if bufs.is_empty() {
            return Poll::Ready(Ok(0));
        }
        loop {
            let packet = match self.receiver.poll_recv(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(packet)) => packet,
                Poll::Ready(None) => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "MQTT carrier closed",
                    )));
                }
            };
            if packet.bytes.len() > bufs[0].len() {
                continue;
            }
            bufs[0][..packet.bytes.len()].copy_from_slice(&packet.bytes);
            metas[0].len = packet.bytes.len();
            metas[0].stride = packet.bytes.len();
            infos[0] = RecvInfo::new(
                address_for(self.shared.service, packet.source),
                Some(address_for(self.shared.service, self.shared.route)),
            );
            return Poll::Ready(Ok(1));
        }
    }
}

#[derive(Debug)]
struct MqttSender(Arc<Shared>);

impl CustomSender for MqttSender {
    fn is_valid_send_addr(&self, addr: &CustomAddr) -> bool {
        route_from_addr(addr, self.0.service).is_ok()
    }

    fn poll_send(
        &self,
        cx: &mut Context,
        dst: &CustomAddr,
        _src: Option<&CustomAddr>,
        transmit: &Transmit<'_>,
    ) -> Poll<io::Result<()>> {
        self.0.poll_datagram(cx, dst, transmit.contents)
    }
}

impl Shared {
    fn poll_datagram(
        &self,
        cx: &mut Context,
        dst: &CustomAddr,
        bytes: &[u8],
    ) -> Poll<io::Result<()>> {
        let target = match route_from_addr(dst, self.service) {
            Ok(route) => route,
            Err(err) => {
                self.traffic
                    .rejected_packets
                    .fetch_add(1, Ordering::Relaxed);
                return Poll::Ready(Err(err));
            }
        };
        if bytes.is_empty() || bytes.len() > MAX_DATAGRAM {
            self.traffic
                .rejected_packets
                .fetch_add(1, Ordering::Relaxed);
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid MQTT datagram size",
            )));
        }
        if *self.status.borrow() != Status::Ready {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "MQTT subscription not ready",
            )));
        }
        let mut queue = self.outbound.lock().unwrap();
        match Pin::new(&mut *queue).poll_ready(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(_)) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "MQTT carrier closed",
            ))),
            Poll::Ready(Ok(())) => {
                let depth = self.report.queued_packets.fetch_add(1, Ordering::Relaxed) + 1;
                self.report.queue_peak.fetch_max(depth, Ordering::Relaxed);
                let sent = Pin::new(&mut *queue)
                    .start_send(Outbound {
                        target,
                        bytes: bytes.to_vec(),
                    })
                    .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "MQTT carrier closed"));
                if sent.is_err() {
                    self.report
                        .queued_packets
                        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |depth| {
                            Some(depth.saturating_sub(1))
                        })
                        .ok();
                }
                Poll::Ready(sent)
            }
        }
    }
}
