//! MIOPUNCH-21: a real MQTT handshake followed by a data path on the same QUIC connection.
#[path = "support/pending_path_endpoint.rs"]
mod pending_path;
use iroh::endpoint::ControlMessageFormat;
use pending_path::PathEndpoint;
use std::{
    io,
    net::{Ipv4Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use iroh::{Endpoint, EndpointAddr, endpoint::presets};
use iroh_base::{SecretKey, TransportAddr};
use miopunch_native_probe::control_session::{CommitError, ControlSession};
use miopunch_native_probe::mqtt_transport::{MqttTransport, Traffic};
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use tokio::net::UdpSocket;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

const ALPN: &[u8] = b"miopunch/path-probe/1";

async fn endpoint(broker: SocketAddr) -> (PathEndpoint, Arc<MqttTransport>, Arc<Traffic>) {
    configured_endpoint(broker, false).await
}
async fn configured_endpoint(
    broker: SocketAddr,
    only: bool,
) -> (PathEndpoint, Arc<MqttTransport>, Arc<Traffic>) {
    let traffic = Arc::new(Traffic::default());
    let carrier = Arc::new(MqttTransport::new(broker, traffic.clone()));
    let builder = Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .secret_key(SecretKey::generate())
        .alpns(vec![ALPN.to_vec()])
        .transport_config(
            iroh::endpoint::QuicTransportConfig::builder()
                .max_idle_timeout(Some(Duration::from_secs(4).try_into().unwrap()))
                .build(),
        )
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .clear_relay_transports()
        .add_custom_transport(carrier.clone());
    let endpoint = PathEndpoint::bind(builder, only).await;
    carrier.wait_ready().await.unwrap();
    (endpoint, carrier, traffic)
}

struct UdpGate {
    addr: SocketAddr,
    hold: Arc<AtomicBool>,
    drop_packets: Arc<AtomicBool>,
    held: Arc<AtomicU64>,
    lost_after_cut: Arc<AtomicU64>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for UdpGate {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl UdpGate {
    async fn new(server: SocketAddr) -> Self {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = socket.local_addr().unwrap();
        let hold = Arc::new(AtomicBool::new(false));
        let drop_packets = Arc::new(AtomicBool::new(false));
        let held = Arc::new(AtomicU64::new(0));
        let lost_after_cut = Arc::new(AtomicU64::new(0));
        let hold_task = hold.clone();
        let drop_task = drop_packets.clone();
        let held_task = held.clone();
        let lost_task = lost_after_cut.clone();
        let task = tokio::spawn(async move {
            let mut client = None;
            let mut buf = [0; 4096];
            let mut pending = Vec::<(Vec<u8>, SocketAddr)>::new();
            loop {
                let (len, source) = socket.recv_from(&mut buf).await.unwrap();
                let target = if source == server {
                    let Some(client) = client else {
                        continue;
                    };
                    client
                } else {
                    client = Some(source);
                    server
                };
                if hold_task.load(Ordering::SeqCst) {
                    pending.push((buf[..len].to_vec(), target));
                    held_task.fetch_add(1, Ordering::SeqCst);
                    continue;
                }
                if drop_task.load(Ordering::SeqCst) {
                    pending.clear();
                    lost_task.fetch_add(1, Ordering::SeqCst);
                    continue;
                }
                for (packet, target) in pending.drain(..) {
                    socket.send_to(&packet, target).await.unwrap();
                }
                socket.send_to(&buf[..len], target).await.unwrap();
            }
        });
        Self {
            addr,
            hold,
            drop_packets,
            held,
            lost_after_cut,
            task,
        }
    }
}

async fn mqtt_frame(reader: &mut (impl AsyncRead + Unpin)) -> io::Result<Option<Vec<u8>>> {
    let mut byte = [0; 1];
    if reader.read_exact(&mut byte).await.is_err() {
        return Ok(None);
    }
    let mut frame = vec![byte[0]];
    let mut len = 0usize;
    let mut multiplier = 1usize;
    for _ in 0..4 {
        reader.read_exact(&mut byte).await?;
        frame.push(byte[0]);
        len += ((byte[0] & 0x7f) as usize) * multiplier;
        if byte[0] & 0x80 == 0 {
            if len > 4096 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "MQTT frame too large",
                ));
            }
            let start = frame.len();
            frame.resize(start + len, 0);
            reader.read_exact(&mut frame[start..]).await?;
            return Ok(Some(frame));
        }
        multiplier *= 128;
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid MQTT frame length",
    ))
}

struct MqttReorderProxy {
    addr: SocketAddr,
    active: Arc<AtomicBool>,
    reversed_pairs: Arc<AtomicU64>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for MqttReorderProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl MqttReorderProxy {
    async fn new(broker: SocketAddr) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let active = Arc::new(AtomicBool::new(false));
        let reversed_pairs = Arc::new(AtomicU64::new(0));
        let active_task = active.clone();
        let reversed_task = reversed_pairs.clone();
        let task = tokio::spawn(async move {
            let (client, _) = listener.accept().await.unwrap();
            let upstream = TcpStream::connect(broker).await.unwrap();
            let (mut client_read, mut client_write) = client.into_split();
            let (mut upstream_read, mut upstream_write) = upstream.into_split();
            let downstream = tokio::spawn(async move {
                let _ = tokio::io::copy(&mut upstream_read, &mut client_write).await;
            });
            let mut held: Option<Vec<u8>> = None;
            while let Some(frame) = mqtt_frame(&mut client_read).await.unwrap() {
                if frame[0] >> 4 == 3 && active_task.load(Ordering::SeqCst) {
                    if let Some(first) = held.take() {
                        upstream_write.write_all(&frame).await.unwrap();
                        upstream_write.write_all(&first).await.unwrap();
                        reversed_task.fetch_add(1, Ordering::SeqCst);
                        active_task.store(false, Ordering::SeqCst);
                    } else {
                        held = Some(frame);
                    }
                } else {
                    upstream_write.write_all(&frame).await.unwrap();
                }
            }
            downstream.abort();
        });
        Self {
            addr,
            active,
            reversed_pairs,
            task,
        }
    }
}

async fn observe_topic(
    broker: SocketAddr,
    route: &str,
) -> (
    AsyncClient,
    tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    tokio::task::JoinHandle<()>,
) {
    let mut options = MqttOptions::new(
        format!("mio21-observer-{}", rand::random::<u64>()),
        broker.ip().to_string(),
        broker.port(),
    );
    options.set_keep_alive(Duration::from_secs(5));
    let (client, mut eventloop) = AsyncClient::new(options, 64);
    let topic = format!("miopunch/test/{route}/packet");
    client.subscribe(topic, QoS::AtMostOnce).await.unwrap();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        let mut ready = Some(ready_tx);
        while let Ok(event) = eventloop.poll().await {
            match event {
                Event::Incoming(Packet::SubAck(_)) => {
                    if let Some(ready) = ready.take() {
                        let _ = ready.send(());
                    }
                }
                Event::Incoming(Packet::Publish(publish)) => {
                    let _ = tx.send(publish.payload.to_vec());
                }
                _ => {}
            }
        }
    });
    tokio::time::timeout(Duration::from_secs(8), ready_rx)
        .await
        .unwrap()
        .unwrap();
    (client, rx, task)
}

async fn wait_ip(connection: &ControlSession) {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if connection.has_data_path() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

async fn wait_control_closed(mut events: iroh::endpoint::PathEventStream) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(event) = events.next().await {
            if matches!(
                event,
                iroh::endpoint::PathEvent::Closed {
                    remote_addr: TransportAddr::Custom(_),
                    ..
                }
            ) {
                return;
            }
        }
        panic!("path events ended before control path closed");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn restricted_mqtt_control_purpose_cannot_be_delivered_as_business() {
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let broker = broker.parse().unwrap();
    let (server, carrier, _) = configured_endpoint(broker, true).await;
    let (client, _, _) = configured_endpoint(broker, true).await;
    let target = EndpointAddr::new(server.id()).with_addrs([TransportAddr::Custom(
        MqttTransport::address(&carrier.route().unwrap()).unwrap(),
    )]);
    let (c, s) = tokio::join!(client.connect(target.clone(), ALPN), async {
        server.accept().await.unwrap().await
    });
    assert!(c.is_err() && s.is_err());
    let (c, s) = tokio::join!(
        client.control(Some(target), ALPN),
        server.control(None, ALPN)
    );
    let (c, s) = (c.unwrap(), s.unwrap());
    assert_eq!(c.remote_id(), server.id());
    assert_eq!(s.remote_id(), client.id());
    client.close().await;
    server.close().await;
}

#[tokio::test]
async fn closed_control_session_never_releases_queued_business() {
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let broker = broker.parse().unwrap();
    let (server, carrier, _) = endpoint(broker).await;
    let (client, _, _) = endpoint(broker).await;
    let accept = tokio::spawn({
        let server = server.clone();
        async move { server.control(None, ALPN).await.unwrap() }
    });
    let target = EndpointAddr::new(server.id()).with_addrs([TransportAddr::Custom(
        MqttTransport::address(&carrier.route().unwrap()).unwrap(),
    )]);
    let mut control = client.control(Some(target), ALPN).await.unwrap();
    let _server_control = accept.await.unwrap();
    control.queue_business(b"never-delivered".to_vec()).unwrap();
    client.close().await;
    assert!(matches!(
        control.commit_data_path().await,
        Err(CommitError::Terminal { .. })
    ));
    server.close().await;
}

#[tokio::test]
async fn reordered_real_mqtt_datagrams_preserve_control_stream_bytes() {
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let broker = broker.parse().unwrap();
    let proxy = MqttReorderProxy::new(broker).await;
    let (server, carrier, _) = endpoint(broker).await;
    let (client, _, _) = endpoint(proxy.addr).await;
    let accept = tokio::spawn({
        let server = server.clone();
        async move { server.control(None, ALPN).await.unwrap() }
    });
    let target = EndpointAddr::new(server.id()).with_addrs([TransportAddr::Custom(
        MqttTransport::address(&carrier.route().unwrap()).unwrap(),
    )]);
    let client_control = client.control(Some(target), ALPN).await.unwrap();
    let server_control = accept.await.unwrap();
    let send = client_control.open_control_exchange().await.unwrap();
    send.send_message(ControlMessageFormat::Fixed { length: 4 }, b"PING")
        .await
        .unwrap();
    let reply = server_control.accept_control_exchange().await.unwrap();
    assert_eq!(
        reply
            .recv_message(ControlMessageFormat::Fixed { length: 4 })
            .await
            .unwrap(),
        b"PING"
    );
    reply
        .send_message(ControlMessageFormat::Fixed { length: 4 }, b"PONG")
        .await
        .unwrap();
    assert_eq!(
        send.recv_message(ControlMessageFormat::Fixed { length: 4 })
            .await
            .unwrap(),
        b"PONG"
    );

    proxy.active.store(true, Ordering::SeqCst);
    let payload = vec![0x5a; 16 * 1024];
    send.send_message(
        ControlMessageFormat::EndTerminated { maximum: 32 * 1024 },
        &payload,
    )
    .await
    .unwrap();
    let delivered = tokio::time::timeout(
        Duration::from_secs(8),
        reply.recv_message(ControlMessageFormat::EndTerminated { maximum: 32 * 1024 }),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(delivered, payload);
    assert_eq!(proxy.reversed_pairs.load(Ordering::SeqCst), 1);
    println!(
        "MQTT_REORDER_EVIDENCE reversed_publish_pairs=1 control_bytes={}",
        delivered.len()
    );
    client.close().await;
    server.close().await;
}

#[tokio::test]
async fn queued_business_and_retransmission_never_fall_back_to_mqtt() {
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let broker = broker.parse().unwrap();
    let (server, server_carrier, server_traffic) = endpoint(broker).await;
    let (client, _, client_traffic) = endpoint(broker).await;
    let server_udp = server
        .bound_sockets()
        .iter()
        .copied()
        .find(|addr| addr.is_ipv4())
        .unwrap();
    let gate = UdpGate::new(server_udp).await;
    let route = server_carrier.route().unwrap();
    let accept = tokio::spawn({
        let server = server.clone();
        async move { server.control(None, ALPN).await.unwrap() }
    });
    let target = EndpointAddr::new(server.id()).with_addrs([TransportAddr::Custom(
        MqttTransport::address(&route).unwrap(),
    )]);
    let mut client_control = client.control(Some(target), ALPN).await.unwrap();
    let server_control = accept.await.unwrap();
    let id = client_control.stable_id();
    let send = client_control.open_control_exchange().await.unwrap();
    send.send_message(ControlMessageFormat::Fixed { length: 7 }, b"CONTROL")
        .await
        .unwrap();
    let recv = server_control.accept_control_exchange().await.unwrap();
    assert_eq!(
        recv.recv_message(ControlMessageFormat::Fixed { length: 7 })
            .await
            .unwrap(),
        b"CONTROL"
    );

    // Ordinary business arrives before any UDP path exists. ControlSession
    // retains business until Core admission and path commitment both return.
    tokio::time::sleep(Duration::from_millis(150)).await;
    let mqtt_before_queue = client_traffic.sent_packets();
    client_control
        .queue_business(vec![0x42; 16 * 1024])
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(client_traffic.sent_packets(), mqtt_before_queue);
    client_control = match client_control.commit_data_path().await {
        Err(CommitError::Retryable { session, .. }) => session,
        other => panic!("control-only path must retain the session: {other:?}"),
    };
    assert_eq!(client_traffic.sent_packets(), mqtt_before_queue);

    client_control.open_data_path(gate.addr).await.unwrap();
    tokio::time::timeout(Duration::from_secs(8), async {
        while !client_control.has_data_path() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(8), async {
        while !server_control.has_data_path() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let client_events = client_control.path_events();
    let server_events = server_control.path_events();
    let (server_data, client_data) = tokio::join!(
        server_control.commit_data_path(),
        client_control.commit_data_path()
    );
    let server_data = server_data.unwrap();
    let mut client_data = client_data.unwrap();
    let server_conn = server_data.connection();
    let queued_business = client_data.take_pending_business().pop_front().unwrap();
    let client_conn = client_data.connection();
    wait_control_closed(client_events).await;
    wait_control_closed(server_events).await;
    assert!(
        client
            .control_handle(&client_conn)
            .open_control_path(MqttTransport::address(&route).unwrap())
            .await
            .is_err()
    );
    tokio::time::sleep(Duration::from_millis(150)).await;
    let client_mqtt = client_traffic.sent_packets();
    let server_mqtt = server_traffic.sent_packets();

    // Hold the first data packet in the network, proving it was queued on
    // this still-live connection, then lose the path before forwarding it.
    gate.hold.store(true, Ordering::SeqCst);
    let (mut send, _) = client_conn.open_bi().await.unwrap();
    send.write_all(&queued_business).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while gate.held.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    gate.drop_packets.store(true, Ordering::SeqCst);
    gate.hold.store(false, Ordering::SeqCst);
    let cut_at = Instant::now();
    tokio::time::timeout(Duration::from_secs(2), async {
        while gate.lost_after_cut.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("no UDP retry packet reached the cut path");
    client_conn
        .send_datagram(bytes::Bytes::from_static(b"LOST-DATAGRAM"))
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), server_conn.accept_bi())
            .await
            .is_err()
    );
    assert_eq!(client_traffic.sent_packets(), client_mqtt);
    assert_eq!(server_traffic.sent_packets(), server_mqtt);
    tokio::time::timeout(Duration::from_secs(10), client_conn.closed())
        .await
        .unwrap();
    assert!(client_conn.close_reason().is_some());
    assert_eq!(client_conn.stable_id(), id);
    assert_eq!(client_traffic.sent_packets(), client_mqtt);
    assert_eq!(server_traffic.sent_packets(), server_mqtt);
    println!(
        "FAILURE_EVIDENCE queued_udp_packets={} retry_or_probe_packets_after_cut={} client_stable_id={id} mqtt_after_failure=({client_mqtt},{server_mqtt}) close={:?} cut_to_close_ms={}",
        gate.held.load(Ordering::SeqCst),
        gate.lost_after_cut.load(Ordering::SeqCst),
        client_conn.close_reason(),
        cut_at.elapsed().as_millis()
    );
    client.close().await;
    server.close().await;
}

#[tokio::test]
async fn real_mqtt_control_connection_upgrades_to_udp_without_redial() {
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let broker = broker.parse().unwrap();
    let (server, server_carrier, server_traffic) = endpoint(broker).await;
    let (client, _client_carrier, client_traffic) = endpoint(broker).await;
    let route = server_carrier.route().unwrap();
    let (observer, mut observed, observer_task) = observe_topic(broker, &route).await;
    let server_udp = server
        .bound_sockets()
        .iter()
        .copied()
        .find(|addr| addr.is_ipv4())
        .unwrap();

    let accept = tokio::spawn({
        let server = server.clone();
        async move { server.control(None, ALPN).await.unwrap() }
    });
    let addr = EndpointAddr::new(server.id()).with_addrs([TransportAddr::Custom(
        MqttTransport::address(&route).unwrap(),
    )]);
    let client_conn =
        tokio::time::timeout(Duration::from_secs(8), client.control(Some(addr), ALPN))
            .await
            .unwrap()
            .unwrap();
    let server_conn = accept.await.unwrap();
    let client_id = client_conn.stable_id();
    let server_id = server_conn.stable_id();
    assert_eq!(client_conn.remote_id(), server.id());
    assert_eq!(server_conn.remote_id(), client.id());
    assert!(
        client_conn
            .control_handle()
            .paths()
            .iter()
            .all(|path| matches!(path.remote_addr(), TransportAddr::Custom(_)))
    );
    assert!(
        server_conn
            .control_handle()
            .paths()
            .iter()
            .all(|path| matches!(path.remote_addr(), TransportAddr::Custom(_)))
    );

    // A second restricted connection to the same peer remains active while
    // the first connection changes purpose. It must not reattach its MQTT
    // path to the data connection via the peer-wide selected-path actor.
    let accept_control = tokio::spawn({
        let server = server.clone();
        async move { server.control(None, ALPN).await.unwrap() }
    });
    let control_addr = EndpointAddr::new(server.id()).with_addrs([TransportAddr::Custom(
        MqttTransport::address(&route).unwrap(),
    )]);
    let parallel_client = client.control(Some(control_addr), ALPN).await.unwrap();
    let parallel_server = accept_control.await.unwrap();
    assert_ne!(parallel_client.stable_id(), client_id);
    assert!(
        parallel_client
            .control_handle()
            .paths()
            .iter()
            .any(|path| matches!(path.remote_addr(), TransportAddr::Custom(_)))
    );

    // Keep this authenticated bounded exchange alive until Core retires control.
    let control_send = client_conn.open_control_exchange().await.unwrap();
    control_send
        .send_message(ControlMessageFormat::Fixed { length: 4 }, b"PING")
        .await
        .unwrap();
    let reply_send = server_conn.accept_control_exchange().await.unwrap();
    assert_eq!(
        reply_send
            .recv_message(ControlMessageFormat::Fixed { length: 4 })
            .await
            .unwrap(),
        b"PING"
    );
    reply_send
        .send_message(ControlMessageFormat::Fixed { length: 4 }, b"PONG")
        .await
        .unwrap();
    assert_eq!(
        control_send
            .recv_message(ControlMessageFormat::Fixed { length: 4 })
            .await
            .unwrap(),
        b"PONG"
    );

    // Retain an authenticated 1-RTT packet for a replay onto the old route
    // after the connection becomes data-only. Drain the parallel attempt's
    // handshake packets, then emit another packet on the original stream.
    while observed.try_recv().is_ok() {}
    control_send
        .send_message(ControlMessageFormat::Fixed { length: 4 }, b"PING")
        .await
        .unwrap();
    assert_eq!(
        reply_send
            .recv_message(ControlMessageFormat::Fixed { length: 4 })
            .await
            .unwrap(),
        b"PING"
    );
    reply_send
        .send_message(ControlMessageFormat::Fixed { length: 4 }, b"PONG")
        .await
        .unwrap();
    assert_eq!(
        control_send
            .recv_message(ControlMessageFormat::Fixed { length: 4 })
            .await
            .unwrap(),
        b"PONG"
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut replay = None;
    while let Ok(payload) = observed.try_recv() {
        if payload.len() > 21 && payload.starts_with(b"MIO1") && payload[20] & 0x80 == 0 {
            replay = Some(payload);
        }
    }
    let replay = replay.expect("observer did not capture an encrypted 1-RTT MQTT packet");

    client_conn.open_data_path(server_udp).await.unwrap();
    wait_ip(&client_conn).await;
    wait_ip(&server_conn).await;
    let client_events = client_conn.path_events();
    let server_events = server_conn.path_events();
    server_conn.commit_control_path().await.unwrap();
    let inbound_before_reopen = server_conn
        .control_handle()
        .path_filter_stats()
        .denied_inbound;
    let adapter_before_reopen = server_traffic.received_packets();
    client_conn
        .control_handle()
        .test_reopen_control_path(MqttTransport::address(&route).unwrap())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while server_conn
            .control_handle()
            .path_filter_stats()
            .denied_inbound
            == inbound_before_reopen
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("remote custom path packet did not reach the pre-QUIC ingress gate");
    assert!(server_traffic.received_packets() > adapter_before_reopen);
    client_conn.commit_control_path().await.unwrap();
    assert!(client_conn.prime_udp_path(server_udp).await.is_err());
    wait_control_closed(client_events).await;
    wait_control_closed(server_events).await;
    let (client_data, server_data) = tokio::join!(
        client_conn.commit_data_path(),
        server_conn.commit_data_path()
    );
    let client_data = client_data.unwrap();
    let server_data = server_data.unwrap();
    let client_conn = client_data.connection();
    let server_conn = server_data.connection();
    assert_eq!(client_conn.stable_id(), client_id);
    assert!(
        client
            .control_handle(&client_conn)
            .open_control_path(MqttTransport::address(&route).unwrap())
            .await
            .is_err()
    );
    assert_eq!(server_conn.stable_id(), server_id);
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        client_conn
            .paths()
            .iter()
            .all(|path| !matches!(path.remote_addr(), TransportAddr::Custom(_)))
    );
    assert!(
        server_conn
            .paths()
            .iter()
            .all(|path| !matches!(path.remote_addr(), TransportAddr::Custom(_)))
    );
    assert!(
        parallel_client
            .control_handle()
            .paths()
            .iter()
            .any(|path| matches!(path.remote_addr(), TransportAddr::Custom(_)))
    );
    assert!(
        parallel_server
            .control_handle()
            .paths()
            .iter()
            .any(|path| matches!(path.remote_addr(), TransportAddr::Custom(_)))
    );
    let mqtt_client_after_commit = client_traffic.sent_packets();
    let mqtt_server_after_commit = server_traffic.sent_packets();

    assert!(
        control_send
            .send_message(ControlMessageFormat::Fixed { length: 4 }, b"PING")
            .await
            .is_err()
    );
    assert!(
        reply_send
            .recv_message(ControlMessageFormat::Fixed { length: 4 })
            .await
            .is_err()
    );

    let (mut business_send, mut business_recv) = client_conn.open_bi().await.unwrap();
    business_send.write_all(b"DATA").await.unwrap();
    business_send.finish().unwrap();
    let (mut response_send, mut data_recv) = server_conn.accept_bi().await.unwrap();
    assert_eq!(data_recv.read_to_end(64).await.unwrap(), b"DATA");
    response_send.write_all(b"ACK!").await.unwrap();
    response_send.finish().unwrap();
    assert_eq!(business_recv.read_to_end(64).await.unwrap(), b"ACK!");
    client_conn
        .send_datagram(bytes::Bytes::from_static(b"DGRAM"))
        .unwrap();
    let datagram = tokio::time::timeout(Duration::from_secs(3), server_conn.read_datagram())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&datagram[..], b"DGRAM");

    assert_eq!(client_traffic.sent_packets(), mqtt_client_after_commit);
    assert_eq!(server_traffic.sent_packets(), mqtt_server_after_commit);

    let other_attempt_before = parallel_server
        .control_handle()
        .path_filter_stats()
        .denied_inbound;
    let carrier_before = server_traffic.received_packets();
    observer
        .publish(
            format!("miopunch/test/{route}/packet"),
            QoS::AtMostOnce,
            false,
            replay,
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while server_traffic.received_packets() == carrier_before {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(server_traffic.received_packets() > carrier_before);
    assert!(
        tokio::time::timeout(Duration::from_millis(200), server_conn.accept_bi())
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(
            Duration::from_millis(200),
            parallel_server.accept_control_exchange()
        )
        .await
        .is_err()
    );
    assert_eq!(
        parallel_server
            .control_handle()
            .path_filter_stats()
            .denied_inbound,
        other_attempt_before
    );
    observer_task.abort();

    // A later recovery-style control attempt must honor its own MQTT route
    // even while this peer's selected path is the earlier connection's IP.
    let before_late_control = client_traffic.sent_packets();
    let accept_late = tokio::spawn({
        let server = server.clone();
        async move { server.control(None, ALPN).await.unwrap() }
    });
    let late_addr = EndpointAddr::new(server.id()).with_addrs([TransportAddr::Custom(
        MqttTransport::address(&route).unwrap(),
    )]);
    let late_client = tokio::time::timeout(
        Duration::from_secs(8),
        client.control(Some(late_addr), ALPN),
    )
    .await
    .unwrap()
    .unwrap();
    let late_server = accept_late.await.unwrap();
    assert_ne!(late_client.stable_id(), client_id);
    assert!(
        late_client
            .control_handle()
            .paths()
            .iter()
            .all(|path| matches!(path.remote_addr(), TransportAddr::Custom(_)))
    );
    assert!(
        late_server
            .control_handle()
            .paths()
            .iter()
            .all(|path| matches!(path.remote_addr(), TransportAddr::Custom(_)))
    );
    let late_send = late_client.open_control_exchange().await.unwrap();
    late_send
        .send_message(ControlMessageFormat::Fixed { length: 7 }, b"CONTROL")
        .await
        .unwrap();
    let late_recv = late_server.accept_control_exchange().await.unwrap();
    assert_eq!(
        late_recv
            .recv_message(ControlMessageFormat::Fixed { length: 7 })
            .await
            .unwrap(),
        b"CONTROL"
    );
    assert!(client_traffic.sent_packets() > before_late_control);
    assert!(
        client_conn
            .paths()
            .iter()
            .all(|path| !matches!(path.remote_addr(), TransportAddr::Custom(_)))
    );
    println!(
        "UPGRADE_EVIDENCE client_stable_id={client_id} server_stable_id={server_id} mqtt_after_commit=({mqtt_client_after_commit},{mqtt_server_after_commit}) client_paths={:?} server_paths={:?}",
        client_conn.paths(),
        server_conn.paths()
    );
    client_conn.close(0u32.into(), b"done");
    client.close().await;
    server.close().await;
}
