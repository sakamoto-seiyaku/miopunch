//! A bounded MQTT packet carrier for the test control protocol. Each publish
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
use rumqttc::{AsyncClient, Event, MqttOptions, Packet as MqttPacket, QoS};
use tokio::sync::watch;

const TRANSPORT_ID: u64 = 0x6d696f6d717474; // "miomqtt"
const MAGIC: &[u8; 4] = b"MIO1";
const ROUTE_LEN: usize = 16;
const MAX_DATAGRAM: usize = 2048;
const QUEUE_DEPTH: usize = 64;

type Route = [u8; ROUTE_LEN];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Starting,
    Ready,
    Closed,
}

fn topic(route: Route) -> String {
    format!("miopunch/test/{}/packet", hex::encode(route))
}

fn route_from_addr(addr: &CustomAddr) -> io::Result<Route> {
    if addr.id() != TRANSPORT_ID {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "wrong MQTT carrier",
        ));
    }
    addr.data()
        .try_into()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid MQTT route length"))
}

#[derive(Debug, Default)]
pub struct Traffic {
    sent_packets: AtomicU64,
    sent_bytes: AtomicU64,
    received_packets: AtomicU64,
    received_bytes: AtomicU64,
    rejected_packets: AtomicU64,
}

impl Traffic {
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

#[derive(Debug)]
pub struct MqttTransport {
    broker: SocketAddr,
    binding: Mutex<Option<Binding>>,
    traffic: Arc<Traffic>,
}

#[derive(Debug)]
struct Binding {
    route: Route,
    status: watch::Sender<Status>,
}

impl MqttTransport {
    pub fn new(broker: SocketAddr, traffic: Arc<Traffic>) -> Self {
        Self {
            broker,
            binding: Mutex::new(None),
            traffic,
        }
    }

    pub fn route(&self) -> io::Result<String> {
        Ok(hex::encode(self.binding()?.route))
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
        let route: Route = bytes.try_into().map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "invalid MQTT route length")
        })?;
        Ok(CustomAddr::from_parts(TRANSPORT_ID, &route))
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
    outbound: Mutex<mpsc::Sender<Outbound>>,
}

#[derive(Debug)]
struct MqttEndpoint {
    shared: Arc<Shared>,
    receiver: tokio::sync::mpsc::Receiver<Packet>,
    addresses: Watchable<Vec<CustomAddr>>,
    status: watch::Sender<Status>,
    driver: tokio::task::JoinHandle<()>,
}

impl Drop for MqttEndpoint {
    fn drop(&mut self) {
        self.driver.abort();
        self.status.send_replace(Status::Closed);
    }
}

impl CustomTransport for MqttTransport {
    fn bind(&self) -> io::Result<Box<dyn CustomEndpoint>> {
        let route: Route = rand::random();
        let (status, _) = watch::channel(Status::Starting);
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
        let shared = Arc::new(Shared {
            route,
            outbound: Mutex::new(outbound),
        });
        let broker = self.broker;
        let traffic = self.traffic.clone();
        let task_status = status.clone();
        let driver = tokio::spawn(async move {
            if let Err(err) =
                drive(broker, route, queue, incoming, task_status.clone(), traffic).await
            {
                eprintln!("MQTT_CLOSE result={err}");
            }
            task_status.send_replace(Status::Closed);
        });
        Ok(Box::new(MqttEndpoint {
            shared,
            receiver,
            addresses: Watchable::new(vec![CustomAddr::from_parts(TRANSPORT_ID, &route)]),
            status,
            driver,
        }))
    }
}

async fn drive(
    broker: SocketAddr,
    route: Route,
    mut queue: mpsc::Receiver<Outbound>,
    incoming: tokio::sync::mpsc::Sender<Packet>,
    status: watch::Sender<Status>,
    traffic: Arc<Traffic>,
) -> io::Result<()> {
    let mut options = MqttOptions::new(
        format!("mio20-{}", hex::encode(route)),
        broker.ip().to_string(),
        broker.port(),
    );
    options.set_keep_alive(Duration::from_secs(5));
    options.set_clean_session(true);
    options.set_max_packet_size(4096, 4096);
    let (client, mut eventloop) = AsyncClient::new(options, QUEUE_DEPTH);
    client
        .subscribe(topic(route), QoS::AtMostOnce)
        .await
        .map_err(io::Error::other)?;
    let own_topic = topic(route);
    let mut subscribed = false;
    loop {
        tokio::select! {
            event = eventloop.poll() => {
                match event.map_err(io::Error::other)? {
                    Event::Incoming(MqttPacket::SubAck(ack)) => {
                        if ack.return_codes != [SubscribeReasonCode::Success(QoS::AtMostOnce)] {
                            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "MQTT subscription rejected"));
                        }
                        subscribed = true;
                        status.send_replace(Status::Ready);
                    }
                    Event::Incoming(MqttPacket::Publish(publish)) => {
                        let payload = publish.payload.as_ref();
                        if !subscribed || publish.topic != own_topic || publish.retain
                            || payload.len() < MAGIC.len() + ROUTE_LEN + 1
                            || payload.len() > MAGIC.len() + ROUTE_LEN + MAX_DATAGRAM
                            || !payload.starts_with(MAGIC)
                        {
                            traffic.rejected_packets.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                        let source: Route = payload[4..20].try_into().unwrap();
                        if source == route {
                            traffic.rejected_packets.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                        let bytes = payload[20..].to_vec();
                        let len = bytes.len();
                        if incoming.try_send(Packet { source, bytes }).is_ok() {
                            traffic.received_packets.fetch_add(1, Ordering::Relaxed);
                            traffic.received_bytes.fetch_add(len as u64, Ordering::Relaxed);
                        } else {
                            traffic.rejected_packets.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    _ => {}
                }
            }
            item = queue.next(), if subscribed => {
                let Some(item) = item else { return Ok(()); };
                let mut payload = Vec::with_capacity(20 + item.bytes.len());
                payload.extend_from_slice(MAGIC);
                payload.extend_from_slice(&route);
                payload.extend_from_slice(&item.bytes);
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
                CustomAddr::from_parts(TRANSPORT_ID, &packet.source),
                Some(CustomAddr::from_parts(TRANSPORT_ID, &self.shared.route)),
            );
            return Poll::Ready(Ok(1));
        }
    }
}

#[derive(Debug)]
struct MqttSender(Arc<Shared>);

impl CustomSender for MqttSender {
    fn is_valid_send_addr(&self, addr: &CustomAddr) -> bool {
        route_from_addr(addr).is_ok()
    }

    fn poll_send(
        &self,
        cx: &mut Context,
        dst: &CustomAddr,
        _src: Option<&CustomAddr>,
        transmit: &Transmit<'_>,
    ) -> Poll<io::Result<()>> {
        let target = match route_from_addr(dst) {
            Ok(route) => route,
            Err(err) => return Poll::Ready(Err(err)),
        };
        if transmit.contents.is_empty() || transmit.contents.len() > MAX_DATAGRAM {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid MQTT datagram size",
            )));
        }
        let mut queue = self.0.outbound.lock().unwrap();
        match Pin::new(&mut *queue).poll_ready(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(_)) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "MQTT carrier closed",
            ))),
            Poll::Ready(Ok(())) => Poll::Ready(
                Pin::new(&mut *queue)
                    .start_send(Outbound {
                        target,
                        bytes: transmit.contents.to_vec(),
                    })
                    .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "MQTT carrier closed")),
            ),
        }
    }
}
