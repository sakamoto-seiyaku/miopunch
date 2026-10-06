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
    driver_active: AtomicU64,
    driver_closed: tokio::sync::Notify,
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
            "driver_active": binding.driver_active.load(Ordering::Acquire),
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
    facility: Option<MqttFacility>,
    binding: Mutex<Option<Binding>>,
    traffic: Arc<Traffic>,
    tls_ca: Option<Vec<u8>>,
}

#[derive(Debug)]
struct Binding {
    route: Route,
    status: watch::Sender<Status>,
    report: Arc<BindingReport>,
}

impl MqttTransport {
    /// Exact canonical mapping used to construct this carrier. A native URL
    /// or service tag alone does not identify the permitted logical facility.
    pub(crate) fn facility(&self) -> Option<&MqttFacility> {
        self.facility.as_ref()
    }
    pub fn traffic(&self) -> &Arc<Traffic> {
        &self.traffic
    }
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
            facility: None,
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
            facility: None,
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
        let facility = facility.validate()?;
        facility.check_allowed(disabled)?;
        Ok(Self {
            resource: facility.native_url.parse().map_err(io::Error::other)?,
            service: Some(facility.service_tag()?),
            facility: Some(facility),
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
                report: binding.report.clone(),
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

    /// Route withdrawal is synchronous; this separately waits for the exact
    /// currently bound driver's future and its socket owners to be dropped.
    /// Kernel-level reclamation still needs an independent observation.
    pub async fn wait_driver_closed(&self) -> io::Result<()> {
        let report = self.binding()?.report;
        loop {
            let notified = report.driver_closed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if report.driver_active.load(Ordering::Acquire) == 0 {
                return Ok(());
            }
            notified.await;
        }
    }
}

struct DriverCompletion(Arc<BindingReport>);
impl Drop for DriverCompletion {
    fn drop(&mut self) {
        self.0.driver_active.store(0, Ordering::Release);
        self.0.driver_closed.notify_waiters();
    }
}

/// Dropped before the MQTT eventloop/socket on every return or cancellation.
struct RouteWithdrawal {
    addresses: Watchable<Vec<CustomAddr>>,
    status: watch::Sender<Status>,
    report: Arc<BindingReport>,
}
impl Drop for RouteWithdrawal {
    fn drop(&mut self) {
        self.addresses.set(Vec::new()).ok();
        self.report.withdraw();
        self.status.send_replace(Status::Closed);
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
        let report = {
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
            if binding
                .as_ref()
                .is_some_and(|binding| binding.report.driver_active.load(Ordering::Acquire) != 0)
            {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "previous MQTT driver is still stopping",
                ));
            }
            let report = Arc::new(BindingReport {
                generation: self.traffic.generation.fetch_add(1, Ordering::Relaxed) + 1,
                driver_active: AtomicU64::new(1),
                ..BindingReport::default()
            });
            *binding = Some(Binding {
                route,
                status: status.clone(),
                report: report.clone(),
            });
            report
        };
        let (outbound, queue) = mpsc::channel(QUEUE_DEPTH);
        let (incoming, receiver) = tokio::sync::mpsc::channel(QUEUE_DEPTH);
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
        let completion = DriverCompletion(report.clone());
        tokio::spawn(async move {
            let _completion = completion;
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
    queue: mpsc::Receiver<Outbound>,
    incoming: &tokio::sync::mpsc::Sender<Packet>,
    status: watch::Sender<Status>,
    traffic: Arc<Traffic>,
    report: Arc<BindingReport>,
    tls_ca: Option<Vec<u8>>,
    stop: watch::Receiver<bool>,
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
            None => {
                #[cfg(target_os = "android")]
                {
                    // Android does not expose the desktop CA files searched by
                    // rustls-native-certs. Use the SDK's compiled public roots,
                    // retaining rustls certificate and hostname verification.
                    rumqttc::Transport::tls_with_config(rumqttc::TlsConfiguration::Rustls(
                        android_public_tls_config(),
                    ))
                }
                #[cfg(not(target_os = "android"))]
                {
                    rumqttc::Transport::tls_with_default_config()
                }
            }
        });
    }
    options.set_keep_alive(Duration::from_secs(5));
    options.set_clean_session(true);
    options.set_max_packet_size(4096, 4096);
    let (client, eventloop) = AsyncClient::new(options, QUEUE_DEPTH);
    let _withdrawal = RouteWithdrawal {
        addresses: addresses.clone(),
        status: status.clone(),
        report: report.clone(),
    };
    client
        .subscribe(topic(route), QoS::AtMostOnce)
        .await
        .map_err(io::Error::other)?;
    drive_connected(
        client, eventloop, route, service, addresses, queue, incoming, status, traffic, report,
        stop,
    )
    .await
}

// One production queue owner and EventLoop poller. The test exercises this
// exact loop with a real pinned MQTT client and a backpressured TCP peer.
async fn drive_connected(
    client: AsyncClient,
    mut eventloop: rumqttc::EventLoop,
    route: Route,
    service: Option<Service>,
    addresses: Watchable<Vec<CustomAddr>>,
    mut queue: mpsc::Receiver<Outbound>,
    incoming: &tokio::sync::mpsc::Sender<Packet>,
    status: watch::Sender<Status>,
    traffic: Arc<Traffic>,
    report: Arc<BindingReport>,
    mut stop: watch::Receiver<bool>,
) -> io::Result<()> {
    let own_topic = topic(route);
    let mut qualification = Qualification::default();
    loop {
        // EventLoop::poll may already own a dequeued publish while feed/flush
        // is Pending. Keep that exact future across additional queue work.
        let event = {
            let polling = eventloop.poll();
            tokio::pin!(polling);
            loop {
                tokio::select! {
                    biased;
                    _ = stop.changed() => break None,
                    event = &mut polling => break Some(event),
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
        };
        let Some(event) = event else {
            addresses.set(Vec::new()).ok();
            report.withdraw();
            status.send_replace(Status::Closed);
            let _ = client.try_unsubscribe(own_topic.clone());
            let _ = client.try_disconnect();
            let _ = tokio::time::timeout(Duration::from_millis(500), async {
                loop {
                    match eventloop.poll().await {
                        Ok(Event::Outgoing(Outgoing::Unsubscribe(_))) => {
                            report.protocol.lock().unwrap().unsubscribe_sent = true
                        }
                        Ok(Event::Outgoing(Outgoing::Disconnect)) => {
                            report.protocol.lock().unwrap().disconnect_sent = true;
                            break;
                        }
                        Err(_) => break,
                        _ => {}
                    }
                }
            })
            .await;
            return Ok(());
        };
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
                    protocol.suback_codes = ack
                        .return_codes
                        .iter()
                        .map(|code| match code {
                            SubscribeReasonCode::Success(qos) => *qos as u8,
                            SubscribeReasonCode::Failure => 128,
                        })
                        .collect();
                }
                qualification.suback(ack.pkid, &report.protocol.lock().unwrap().suback_codes)?;
                report.protocol.lock().unwrap().ready_ms = Some(now_ms());
                addresses.set(vec![address_for(service, route)]).ok();
                status.send_replace(Status::Ready);
            }
            Event::Incoming(MqttPacket::Publish(publish)) => {
                let payload = publish.payload.as_ref();
                let decoded = decode_datagram(payload);
                if !qualification.ready()
                    || publish.topic != own_topic
                    || publish.retain
                    || publish.qos != QoS::AtMostOnce
                    || decoded.is_err()
                {
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
                    report.rx_queue_peak.fetch_max(
                        (QUEUE_DEPTH - incoming.capacity()).max(1) as u64,
                        Ordering::Relaxed,
                    );
                    traffic.received_packets.fetch_add(1, Ordering::Relaxed);
                    traffic
                        .received_bytes
                        .fetch_add(len as u64, Ordering::Relaxed);
                } else {
                    traffic.rejected_packets.fetch_add(1, Ordering::Relaxed);
                }
            }
            _ => {}
        }
    }
}

#[cfg(any(target_os = "android", test))]
fn android_public_tls_config() -> Arc<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    Arc::new(
        rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS versions")
        .with_root_certificates(roots)
        .with_no_client_auth(),
    )
}

#[cfg(test)]
mod tls_tests {
    use super::*;

    #[test]
    fn android_public_mqtt_roots_reject_an_untrusted_server_certificate() {
        let config = android_public_tls_config();
        assert!(config.enable_sni);
        let certificate = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let server_config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![certificate.cert.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(certificate.signing_key.serialize_der())
                .into(),
        )
        .unwrap();
        let mut client = rustls::ClientConnection::new(
            config,
            rustls::pki_types::ServerName::try_from("localhost").unwrap(),
        )
        .unwrap();
        let mut server = rustls::ServerConnection::new(Arc::new(server_config)).unwrap();
        let mut client_hello = Vec::new();
        client.write_tls(&mut client_hello).unwrap();
        server
            .read_tls(&mut std::io::Cursor::new(client_hello))
            .unwrap();
        server.process_new_packets().unwrap();
        let mut server_flight = Vec::new();
        server.write_tls(&mut server_flight).unwrap();
        client
            .read_tls(&mut std::io::Cursor::new(server_flight))
            .unwrap();
        assert!(matches!(
            client.process_new_packets(),
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::UnknownIssuer
            ))
        ));
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

#[cfg(test)]
mod poll_retention_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn mqtt_packet(socket: &mut tokio::net::TcpStream) -> io::Result<(u8, Vec<u8>)> {
        let kind = socket.read_u8().await?;
        let mut length = 0usize;
        let mut shift = 0;
        loop {
            let byte = socket.read_u8().await?;
            length += usize::from(byte & 127) << shift;
            if length > 4096 || shift > 21 {
                return Err(io::Error::other("owned MQTT packet exceeds limit"));
            }
            if byte & 128 == 0 {
                break;
            }
            shift += 7;
        }
        let mut body = vec![0; length];
        socket.read_exact(&mut body).await?;
        Ok((kind, body))
    }

    #[tokio::test]
    async fn backpressured_publish_keeps_every_datagram() {
        const COUNT: usize = 48;
        // Set the receiver window before accept, so an unread real socket
        // forces feed/flush Pending while another bounded datagram is queued.
        let listener =
            socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).unwrap();
        listener.set_recv_buffer_size(1024).unwrap();
        listener.set_nonblocking(true).unwrap();
        listener
            .bind(&"127.0.0.1:0".parse::<SocketAddr>().unwrap().into())
            .unwrap();
        listener.listen(1).unwrap();
        let listener = tokio::net::TcpListener::from_std(listener.into()).unwrap();
        let address = listener.local_addr().unwrap();
        let (release, resume) = tokio::sync::oneshot::channel();
        let (delivered, delivery) = tokio::sync::oneshot::channel();
        let own_route = [1u8; 16];
        let destination_route = [2u8; 16];
        let broker = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            assert_eq!(mqtt_packet(&mut socket).await.unwrap().0 >> 4, 1);
            socket.write_all(&[0x20, 2, 0, 0]).await.unwrap();
            let (kind, subscribe) = mqtt_packet(&mut socket).await.unwrap();
            assert_eq!(kind >> 4, 8);
            socket
                .write_all(&[0x90, 3, subscribe[0], subscribe[1], 0])
                .await
                .unwrap();
            resume.await.unwrap();
            let mut received = Vec::new();
            let mut delivered = Some(delivered);
            let deadline = tokio::time::sleep(Duration::from_secs(3));
            tokio::pin!(deadline);
            loop {
                let packet = {
                    let reading = mqtt_packet(&mut socket);
                    tokio::pin!(reading);
                    tokio::select! {
                        packet = &mut reading => packet,
                        _ = &mut deadline, if delivered.is_some() => {
                            delivered.take().unwrap().send(received.clone()).unwrap();
                            // Retain this same real read while natural shutdown arrives.
                            reading.await
                        }
                    }
                };
                let Ok((kind, body)) = packet else {
                    break;
                };
                match kind >> 4 {
                    3 => {
                        assert_eq!(kind, 0x30, "the real carrier must retain QoS0");
                        let length = usize::from(u16::from_be_bytes([body[0], body[1]]));
                        assert_eq!(&body[2..2 + length], topic(destination_route).as_bytes());
                        let (source, payload) = decode_datagram(&body[2 + length..]).unwrap();
                        assert_eq!(source, own_route);
                        assert_eq!(payload.len(), MAX_DATAGRAM);
                        let index = u32::from_be_bytes(payload[..4].try_into().unwrap()) as usize;
                        assert!(payload[4..].iter().all(|byte| *byte == index as u8));
                        received.push(index);
                        if received.len() == COUNT {
                            if let Some(delivered) = delivered.take() {
                                delivered.send(received.clone()).unwrap();
                            }
                        }
                    }
                    10 => socket
                        .write_all(&[0xb0, 2, body[0], body[1]])
                        .await
                        .unwrap(),
                    12 => socket.write_all(&[0xd0, 0]).await.unwrap(),
                    14 => break,
                    _ => panic!("unexpected owned broker packet"),
                }
            }
            received
        });
        let mut options = MqttOptions::new(
            "owned-poll-regression",
            address.ip().to_string(),
            address.port(),
        );
        options.set_keep_alive(Duration::from_secs(5));
        options.set_clean_session(true);
        options.set_max_packet_size(4096, 4096);
        let (client, mut eventloop) = AsyncClient::new(options, QUEUE_DEPTH);
        eventloop.network_options.set_tcp_send_buffer_size(1024);
        eventloop.network_options.set_tcp_nodelay(true);
        client
            .subscribe(topic(own_route), QoS::AtMostOnce)
            .await
            .unwrap();
        let traffic = Arc::new(Traffic::default());
        let report = Arc::new(BindingReport::default());
        let (status, mut ready) = watch::channel(Status::Starting);
        let (stop, stopping) = watch::channel(false);
        let (sender, queue) = mpsc::channel(QUEUE_DEPTH);
        let (incoming, _received) = tokio::sync::mpsc::channel(QUEUE_DEPTH);
        let addresses = Watchable::new(Vec::new());
        let driver_status = status.clone();
        let driver_traffic = traffic.clone();
        let driver_report = report.clone();
        let task = tokio::spawn(async move {
            drive_connected(
                client,
                eventloop,
                own_route,
                None,
                addresses,
                queue,
                &incoming,
                driver_status,
                driver_traffic,
                driver_report,
                stopping,
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(3), async {
            while *ready.borrow() != Status::Ready {
                ready.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        let shared = Shared {
            route: own_route,
            service: None,
            outbound: Mutex::new(sender),
            status,
            shutdown: stop.clone(),
            traffic: traffic.clone(),
            report,
        };
        let target = address_for(None, destination_route);
        for index in 0..COUNT {
            let mut bytes = vec![index as u8; MAX_DATAGRAM];
            bytes[..4].copy_from_slice(&(index as u32).to_be_bytes());
            futures_util::future::poll_fn(|cx| shared.poll_datagram(cx, &target, &bytes))
                .await
                .unwrap();
        }
        tokio::time::timeout(Duration::from_secs(3), async {
            while traffic.sent_packets() < COUNT as u64 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        release.send(()).unwrap();
        let delivered = tokio::time::timeout(Duration::from_secs(4), delivery)
            .await
            .unwrap()
            .unwrap();
        // Observe actual broker delivery before the original natural stop.
        stop.send_replace(true);
        task.await.unwrap().unwrap();
        let actual = broker.await.unwrap();
        assert_eq!(traffic.sent_packets(), COUNT as u64);
        assert_eq!(delivered, actual, "delivery only appeared during shutdown");
        assert_eq!(
            actual,
            (0..COUNT).collect::<Vec<_>>(),
            "accepted datagrams were lost across cancellation of a real MQTT feed/flush"
        );
    }
}
