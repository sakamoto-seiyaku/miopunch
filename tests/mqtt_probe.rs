use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use miopunch_native_probe::mqtt_transport::{MqttTransport, Traffic};
use rumqttc::{Client, Event, MqttOptions, Packet, QoS};

const BIN: &str = env!("CARGO_BIN_EXE_miopunch-native-probe");

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn identity(dir: &Path) -> String {
    let result = Command::new(BIN).arg("identity").arg(dir).output().unwrap();
    assert!(result.status.success(), "{result:?}");
    String::from_utf8(result.stdout).unwrap().trim().to_owned()
}

fn start_server(dir: &Path, broker: &str, expected: &str) -> (Server, String, String) {
    let mut child = Command::new(BIN)
        .args(["serve", "mqtt"])
        .arg(dir)
        .args([broker, expected])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout).read_line(&mut line);
        let _ = tx.send((result, line));
    });
    let (result, line) = rx.recv_timeout(Duration::from_secs(12)).unwrap();
    result.unwrap();
    let fields: Vec<_> = line.split_whitespace().collect();
    assert_eq!(fields.len(), 3, "{line:?}");
    assert_eq!(fields[0], "READY");
    (Server(child), fields[1].into(), fields[2].into())
}

fn send(dir: &Path, broker: &str, target: &str, route: &str) -> Output {
    Command::new(BIN)
        .args(["send", "mqtt"])
        .arg(dir)
        .args([target, route, broker])
        .output()
        .unwrap()
}

fn wait_server(server: &mut Server) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if server.0.try_wait().unwrap().is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("server did not finish after control exchange");
}

// Set MIOPUNCH_MQTT_BROKER to a dedicated, anonymous test broker's host:port.
#[test]
fn known_nodes_exchange_control_over_a_real_mqtt_broker() {
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let client_dir = root.path().join("client");
    let client_id = identity(&client_dir);
    let (mut server, server_id, route) =
        start_server(&root.path().join("server"), &broker, &client_id);
    let started = Instant::now();
    let result = send(&client_dir, &broker, &server_id, &route);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"PONG\n");
    assert!(String::from_utf8_lossy(&result.stderr).contains("MQTT_TRAFFIC sent_packets="));
    wait_server(&mut server);
    let mut server_traffic = String::new();
    server
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut server_traffic)
        .unwrap();
    assert!(server_traffic.contains("MQTT_TRAFFIC sent_packets="));
    println!(
        "MQTT_EVIDENCE control_ms={} client={} server={}",
        started.elapsed().as_millis(),
        String::from_utf8_lossy(&result.stderr).trim(),
        server_traffic.trim()
    );
}

#[test]
fn cancelling_a_subscribed_mqtt_endpoint_completes() {
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let result = Command::new(BIN)
        .args(["cancel", "mqtt"])
        .arg(root.path().join("node"))
        .arg(broker)
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"CANCELLED\n");
}

#[tokio::test]
async fn rebinding_the_adapter_gets_a_fresh_delivery_route() {
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let transport = Arc::new(MqttTransport::new(
        broker.parse().unwrap(),
        Arc::new(Traffic::default()),
    ));
    let bind = || {
        iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
            .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
            .secret_key(iroh_base::SecretKey::from_bytes(&[9; 32]))
            .clear_ip_transports()
            .clear_relay_transports()
            .add_custom_transport(transport.clone())
            .bind()
    };
    let first = bind().await.unwrap();
    transport.wait_ready().await.unwrap();
    let first_route = transport.route().unwrap();
    first.close().await;
    drop(first);
    transport.wait_closed().await.unwrap();
    let second = bind().await.unwrap();
    transport.wait_ready().await.unwrap();
    let second_route = transport.route().unwrap();
    assert_ne!(first_route, second_route);
    second.close().await;
    drop(second);
    transport.wait_closed().await.unwrap();
}

struct Observer {
    client: Client,
    packets: Arc<Mutex<Vec<Vec<u8>>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Observer {
    fn new(broker: &str) -> Self {
        let broker: std::net::SocketAddr = broker.parse().unwrap();
        let mut options = MqttOptions::new(
            format!("mio20-observer-{}", rand::random::<u64>()),
            broker.ip().to_string(),
            broker.port(),
        );
        options.set_keep_alive(Duration::from_secs(5));
        let (client, mut connection) = Client::new(options, 64);
        client
            .subscribe("miopunch/test/#", QoS::AtMostOnce)
            .unwrap();
        let (ready_tx, ready_rx) = mpsc::channel();
        let packets = Arc::new(Mutex::new(Vec::new()));
        let captured = packets.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let thread = std::thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                match connection.recv_timeout(Duration::from_millis(100)) {
                    Ok(Ok(Event::Incoming(Packet::SubAck(_)))) => {
                        let _ = ready_tx.send(());
                    }
                    Ok(Ok(Event::Incoming(Packet::Publish(p)))) => {
                        captured.lock().unwrap().push(p.payload.to_vec())
                    }
                    Ok(Err(err)) => panic!("observer broker error: {err}"),
                    _ => {}
                }
            }
        });
        ready_rx.recv_timeout(Duration::from_secs(8)).unwrap();
        Self {
            client,
            packets,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for Observer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

#[test]
fn wrong_identity_and_other_subscriber_cannot_get_control_or_plaintext() {
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let client_dir = root.path().join("client");
    let client_id = identity(&client_dir);
    let observer = Observer::new(&broker);
    let (mut server, server_id, route) =
        start_server(&root.path().join("server"), &broker, &client_id);

    let topic = format!("miopunch/test/{route}/packet");
    observer
        .client
        .publish(&topic, QoS::AtMostOnce, false, b"MIO1".to_vec())
        .unwrap();
    let mut injected = b"MIO1".to_vec();
    injected.extend_from_slice(&[7; 16]);
    injected.extend_from_slice(&[1, 2, 3]);
    observer
        .client
        .publish(&topic, QoS::AtMostOnce, false, injected.clone())
        .unwrap();
    observer
        .client
        .publish(&topic, QoS::AtMostOnce, false, injected)
        .unwrap();

    let wrong_target = identity(&root.path().join("wrong-target"));
    let wrong = send(&client_dir, &broker, &wrong_target, &route);
    assert!(!wrong.status.success(), "{wrong:?}");
    assert!(!wrong.stdout.windows(4).any(|part| part == b"PONG"));
    let intruder = send(&root.path().join("intruder"), &broker, &server_id, &route);
    assert!(!intruder.status.success(), "{intruder:?}");
    assert!(!intruder.stdout.windows(4).any(|part| part == b"PONG"));
    assert!(server.0.try_wait().unwrap().is_none());

    let valid = send(&client_dir, &broker, &server_id, &route);
    assert!(valid.status.success(), "{valid:?}");
    assert_eq!(valid.stdout, b"PONG\n");
    wait_server(&mut server);
    let packets = observer.packets.lock().unwrap();
    assert!(!packets.is_empty());
    assert!(packets.iter().all(|packet| {
        !packet
            .windows(4)
            .any(|part| part == b"PING" || part == b"PONG")
    }));
    println!(
        "MQTT_EVIDENCE observed_publishes={} max_payload_bytes={}",
        packets.len(),
        packets.iter().map(Vec::len).max().unwrap()
    );
}
