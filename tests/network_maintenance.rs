//! Public node lifecycle, real admission and durable LogSync; no mocked peers.
use iroh::{Endpoint, endpoint::Connection};
use miopunch_native_probe::{
    admission_gate::ADMISSION_REQUEST_ALPN,
    manual_admission::ManualExchange,
    native_network::NativeNetwork,
    network_recovery::RECOVERY_REPLICA_ALPN,
    resources::{Resource, ResourceUse},
};
use std::{collections::BTreeSet, net::Ipv4Addr, sync::Arc, time::Duration};

async fn observe_queries(
    broker: &str,
) -> (
    Arc<std::sync::Mutex<Vec<std::time::Instant>>>,
    tokio::task::JoinHandle<()>,
) {
    use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
    let (host, port) = broker.rsplit_once(':').unwrap();
    let options = MqttOptions::new(
        format!("mio49-observer-{}", rand::random::<u64>()),
        host,
        port.parse().unwrap(),
    );
    let (client, mut events) = AsyncClient::new(options, 32);
    client
        .subscribe("miopunch/#", QoS::AtMostOnce)
        .await
        .unwrap();
    loop {
        if matches!(
            events.poll().await.unwrap(),
            Event::Incoming(Packet::SubAck(_))
        ) {
            break;
        }
    }
    let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let records = observed.clone();
    let task = tokio::spawn(async move {
        while let Ok(event) = events.poll().await {
            if let Event::Incoming(Packet::Publish(message)) = event {
                if message.payload.starts_with(b"MDQ1") {
                    records.lock().unwrap().push(std::time::Instant::now());
                }
            }
        }
    });
    (observed, task)
}

async fn connect(a: &Endpoint, b: &Endpoint, alpn: &[u8]) -> (Connection, Connection) {
    let (client, server) = tokio::join!(a.connect(b.addr(), alpn), async {
        b.accept().await.unwrap().await
    });
    (client.unwrap(), server.unwrap())
}

async fn join(admin: &mut NativeNetwork, path: &std::path::Path) -> NativeNetwork {
    let mut node = NativeNetwork::join(path, admin.manual_invitation().unwrap())
        .await
        .unwrap();
    let ae = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let ne = node.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = connect(&ne, &ae, ADMISSION_REQUEST_ALPN).await;
    let (request, served) = tokio::join!(
        node.request_manual_admission(&client),
        admin.serve_manual_admission(&server)
    );
    assert!(matches!(request.unwrap(), ManualExchange::AwaitingApproval));
    served.unwrap();
    let request = admin.manual_requests().await.unwrap().pop().unwrap();
    admin.approve_manual_admission(&request).await.unwrap();
    let receiver = async {
        let ManualExchange::Materials(material) =
            node.request_manual_admission(&client).await.unwrap()
        else {
            panic!("approval")
        };
        node.complete_manual_admission(material).await.unwrap();
    };
    let (_, served) = tokio::join!(receiver, admin.serve_manual_admission(&server));
    served.unwrap();
    ne.close().await;
    ae.close().await;
    node
}

fn configure(node: &NativeNetwork, broker: &str) {
    let mut config = node.resource_configuration().unwrap();
    config.maintenance.exploration_interval_ms = 1_000;
    config.maintenance.discovery_interval_ms = 1_000;
    config.replacement = Some(vec![Resource {
        url: format!("mqtt://{broker}/"),
        uses: BTreeSet::from([ResourceUse::Signaling, ResourceUse::Lookup]),
        limitation: "owned maintenance fixture, never business relay".into(),
    }]);
    node.save_resource_configuration(&config).unwrap();
}

#[tokio::test]
async fn startup_automatically_recovers_and_saves_history_without_business() {
    let broker = std::env::var("MIOPUNCH_MQTT_BROKER")
        .expect("run scripts/check_network_maintenance.py for the real broker");
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let a = join(&mut admin, &dir.path().join("a")).await;
    a.close().await;
    let c = join(&mut admin, &dir.path().join("c")).await;
    admin.close().await;
    let a = NativeNetwork::open(dir.path().join("a")).await.unwrap();
    configure(&a, &broker);
    configure(&c, &broker);
    let c_id = c.status().node_id;
    assert!(!a.status().members.iter().any(|(id, _)| *id == c_id));
    let resources = Arc::new(
        c.bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap(),
    );
    resources
        .endpoint()
        .set_alpns(vec![RECOVERY_REPLICA_ALPN.to_vec()]);
    let serving_resources = resources.clone();
    let serving = tokio::spawn(async move {
        while let Some(incoming) = serving_resources.endpoint().accept().await {
            if let Ok(connection) = incoming.await {
                let _ = c
                    .serve_network_recovery(&serving_resources, &connection)
                    .await;
            }
        }
        c.close().await;
    });
    let a = a.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    let recovered = tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            if a.node_status().members.iter().any(|(id, _)| *id == c_id) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .is_ok();
    a.close().await;
    resources.endpoint().close().await;
    serving.await.unwrap();
    let reopened = NativeNetwork::open(dir.path().join("a")).await.unwrap();
    let saved = reopened.status().members.iter().any(|(id, _)| *id == c_id);
    reopened.close().await;
    assert!(
        recovered && saved,
        "startup maintenance must recover and durably save the unknown member without a business request"
    );
}

#[tokio::test]
async fn continuous_failure_backs_off_then_an_online_member_recovers_without_dial_storm() {
    let broker = std::env::var("MIOPUNCH_MQTT_BROKER").expect("owned real broker required");
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let a = join(&mut admin, &dir.path().join("a")).await;
    a.close().await;
    let c = join(&mut admin, &dir.path().join("c")).await;
    admin.close().await;
    let a = NativeNetwork::open(dir.path().join("a")).await.unwrap();
    configure(&a, &broker);
    configure(&c, &broker);
    let (observed, observer) = observe_queries(&broker).await;
    let a = a.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    tokio::time::sleep(Duration::from_secs(11)).await;
    let times = observed.lock().unwrap().clone();
    let backed_off = times.len() >= 3
        && times[2].duration_since(times[1])
            > times[1].duration_since(times[0]) + Duration::from_millis(500);
    let c_id = c.status().node_id;
    let resources = Arc::new(
        c.bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap(),
    );
    resources
        .endpoint()
        .set_alpns(vec![RECOVERY_REPLICA_ALPN.to_vec()]);
    let serving_resources = resources.clone();
    let serving = tokio::spawn(async move {
        while let Some(incoming) = serving_resources.endpoint().accept().await {
            if let Ok(connection) = incoming.await {
                let _ = c
                    .serve_network_recovery(&serving_resources, &connection)
                    .await;
            }
        }
        c.close().await;
    });
    let recovered = tokio::time::timeout(Duration::from_secs(14), async {
        loop {
            if a.node_status().members.iter().any(|(id, _)| *id == c_id) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .is_ok();
    a.close().await;
    resources.endpoint().close().await;
    serving.await.unwrap();
    observer.abort();
    eprintln!(
        "OBSERVED discovery_queries={} first_gaps_ms={:?}",
        times.len(),
        times
            .windows(2)
            .map(|w| w[1].duration_since(w[0]).as_millis())
            .collect::<Vec<_>>()
    );
    assert!(
        backed_off,
        "repeated real failed discovery must slow down instead of querying at a fixed interval"
    );
    assert!(
        recovered,
        "a later online member must recover without user business traffic"
    );
    assert!(times.len() <= 4, "no background query storm");
}

#[tokio::test]
async fn a_new_contact_wakes_waiting_maintenance_and_verified_contact_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let a = join(&mut admin, &dir.path().join("a")).await;
    a.close().await;
    let c = join(&mut admin, &dir.path().join("c")).await;
    admin.close().await;
    let a = NativeNetwork::open(dir.path().join("a")).await.unwrap();
    for node in [&a, &c] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(Vec::new());
        node.save_resource_configuration(&config).unwrap();
    }
    let c_id = c.status().node_id;
    let c = c.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    let a = a.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!a.node_status().members.iter().any(|(id, _)| *id == c_id));
    for _ in 0..32 {
        a.offer_contact(c.addr()).await.unwrap();
    }
    let recovered = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if a.node_status().members.iter().any(|(id, _)| *id == c_id)
                && a.maintenance_status().neighbors.len() == 1
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .is_ok();
    let attempts = a.maintenance_status().contact_attempts;
    eprintln!(
        "OBSERVED duplicate_hints=32 actual_contact_attempts={attempts} live_neighbors={}",
        a.maintenance_status().neighbors.len()
    );
    a.close().await;
    assert!(
        recovered,
        "a new allowed direct hint must wake maintenance without business"
    );
    assert!(
        attempts <= 2,
        "duplicate identical hints must not cause simultaneous or repeated dials"
    );
    let a = NativeNetwork::open(dir.path().join("a"))
        .await
        .unwrap()
        .maintain_network(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let restored = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if a.maintenance_status().neighbors.len() == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .is_ok();
    a.close().await;
    c.close().await;
    assert!(
        restored,
        "the verified saved contact must be tried at startup without external discovery"
    );
}

#[tokio::test]
async fn conservative_and_active_nodes_observe_distinct_frequency_and_concurrency_budgets() {
    use miopunch_native_probe::network_maintenance::MaintenancePolicy;
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let mut hints = Vec::new();
    let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut observers = Vec::new();
    for index in 0..4 {
        let peer = join(&mut admin, &dir.path().join(format!("peer{index}"))).await;
        let id = iroh::EndpointId::from_bytes(peer.status().node_id.as_bytes()).unwrap();
        peer.close().await;
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        hints.push(iroh::EndpointAddr::new(id).with_ip_addr(socket.local_addr().unwrap()));
        let observed = observed.clone();
        observers.push(tokio::spawn(async move {
            let mut bytes = [0; 2048];
            while socket.recv_from(&mut bytes).await.is_ok() {
                observed
                    .lock()
                    .unwrap()
                    .push((index, std::time::Instant::now()));
            }
        }));
    }
    let conservative = join(&mut admin, &dir.path().join("conservative")).await;
    let active = join(&mut admin, &dir.path().join("active")).await;
    admin.close().await;
    let mut results = Vec::new();
    for (node, mut policy) in [
        (conservative, MaintenancePolicy::conservative()),
        (active, MaintenancePolicy::active()),
    ] {
        policy.contact_timeout_ms = 500;
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(Vec::new());
        config.maintenance = policy;
        node.save_resource_configuration(&config).unwrap();
        observed.lock().unwrap().clear();
        let node = node.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
        for hint in &hints {
            node.offer_contact(hint.clone()).await.unwrap();
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        let packets = observed.lock().unwrap().clone();
        let peers: BTreeSet<_> = packets.iter().map(|(peer, _)| *peer).collect();
        let status = node.maintenance_status();
        results.push((status.contact_attempts, status.max_in_flight, peers.len()));
        assert!(status.neighbors.is_empty());
        node.close().await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    for observer in observers {
        observer.abort();
    }
    eprintln!(
        "OBSERVED (attempts,max_in_flight,UDP_targets) conservative={:?} active={:?}",
        results[0], results[1]
    );
    assert_eq!(results[0].0, 1);
    assert_eq!(results[0].1, 1);
    assert_eq!(results[0].2, 1);
    assert!(
        results[1].0 >= 4 && results[1].1 > 1 && results[1].1 <= 4 && results[1].2 == 4,
        "active effort must affect actual packets and parallel attempts"
    );
}

const BUSINESS: &[u8] = b"miopunch/maintenance-business/1";
#[derive(Debug)]
struct Echo;
impl iroh::protocol::ProtocolHandler for Echo {
    async fn accept(&self, connection: Connection) -> Result<(), iroh::protocol::AcceptError> {
        while let Ok((mut send, mut recv)) = connection.accept_bi().await {
            let Ok(bytes) = recv.read_to_end(64 * 1024).await else {
                break;
            };
            if send.write_all(&bytes).await.is_err() {
                break;
            }
            if send.finish().is_err() {
                break;
            }
        }
        Ok(())
    }
}
async fn wait_neighbors(
    node: &miopunch_native_probe::network_maintenance::MaintainedNetwork,
    count: usize,
) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if node.maintenance_status().neighbors.len() == count {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("real neighbor state");
}

#[tokio::test]
async fn real_business_has_priority_and_shared_identity_guard_prevents_duplicate_dials() {
    use miopunch_native_probe::{
        admission_gate::ProtocolScope, network_maintenance::MaintenanceState,
    };
    let broker = std::env::var("MIOPUNCH_MQTT_BROKER").expect("owned real broker required");
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let a = join(&mut admin, &dir.path().join("a")).await;
    a.close().await;
    let c = join(&mut admin, &dir.path().join("c")).await;
    admin.close().await;
    let a = NativeNetwork::open(dir.path().join("a")).await.unwrap();
    for node in [&a, &c] {
        configure(node, &broker);
        node.admission_gate()
            .register_protocol(BUSINESS, ProtocolScope::Member);
    }
    let c = c
        .maintain_network_with(Ipv4Addr::LOCALHOST, |router| router.accept(BUSINESS, Echo))
        .await
        .unwrap();
    let c_priority = c.business_activity();
    let (queries, observer) = observe_queries(&broker).await;
    let a = a.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    a.offer_contact(c.addr()).await.unwrap();
    wait_neighbors(&a, 1).await;
    let original_neighbor = a.maintenance_status().neighbors[0].stable_id;
    let target = c.addr();
    let (first, duplicate) = tokio::join!(
        a.connect(target.clone(), BUSINESS),
        a.connect(target, BUSINESS)
    );
    assert!(
        first.is_ok() && duplicate.is_err(),
        "the SDK and background share one in-flight guard for each identity"
    );
    let (connection, lease) = first.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while a.maintenance_status().state != MaintenanceState::BusinessPriority {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let before = queries.lock().unwrap().len();
    let before_rounds = a.maintenance_status().discovery_rounds;
    let (mut send, mut recv) = connection.open_bi().await.unwrap();
    let bytes = vec![0x59; 32 * 1024];
    send.write_all(&bytes).await.unwrap();
    send.finish().unwrap();
    assert_eq!(recv.read_to_end(64 * 1024).await.unwrap(), bytes);
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(
        queries.lock().unwrap().len(),
        before,
        "a real business lease suppresses background lookup packets"
    );
    assert_eq!(a.maintenance_status().discovery_rounds, before_rounds);
    assert_eq!(
        a.maintenance_status().neighbors[0].stable_id,
        original_neighbor
    );
    connection.close(0u32.into(), b"business completed");
    drop(lease);
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if a.maintenance_status().discovery_rounds > before_rounds {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("maintenance resumes after business");
    drop(c_priority);
    observer.abort();
    a.close().await;
    c.close().await;
    eprintln!(
        "OBSERVED business_echo_bytes=32768 background_queries_during_business=0 duplicate_in_flight_rejected=true"
    );
}

#[tokio::test]
async fn partial_links_propagate_new_signed_history_on_the_same_neighbor_session() {
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let a = join(&mut admin, &dir.path().join("a")).await;
    a.close().await;
    let c = join(&mut admin, &dir.path().join("c")).await;
    let a = NativeNetwork::open(dir.path().join("a")).await.unwrap();
    for node in [&a, &c, &admin] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(Vec::new());
        config.maintenance.exploration_interval_ms = 250;
        node.save_resource_configuration(&config).unwrap();
    }
    let c = c.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    let a = a.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    a.offer_contact(c.addr()).await.unwrap();
    wait_neighbors(&a, 1).await;
    let original = a.maintenance_status().neighbors[0].stable_id;
    let a_priority = a.business_activity();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let d = join(&mut admin, &dir.path().join("d")).await;
    let d_id = d.status().node_id;
    d.close().await;
    let admin = admin.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    c.offer_contact(admin.addr()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if c.node_status().members.iter().any(|(id, _)| *id == d_id) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("C syncs the original signed D authorization");
    admin.close().await; // Neither original author nor D remains online for A.
    assert!(!a.node_status().members.iter().any(|(id, _)| *id == d_id));
    drop(a_priority);
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if a.node_status().members.iter().any(|(id, _)| *id == d_id) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("A gets durable D records over partial A-C connectivity");
    let status = a.maintenance_status();
    assert_eq!(status.neighbors.len(), 1);
    assert_eq!(status.neighbors[0].stable_id, original);
    assert_eq!(status.last_sync.as_ref().unwrap().source, c.endpoint().id());
    assert!(status.last_sync.as_ref().unwrap().complete());
    a.close().await;
    c.close().await;
    let a = NativeNetwork::open(dir.path().join("a")).await.unwrap();
    assert!(a.status().members.iter().any(|(id, _)| *id == d_id));
    a.close().await;
    eprintln!(
        "OBSERVED partial_link=A-C-Creator original_author_offline_at_A_sync=true same_neighbor_session=true saved_new_member=true"
    );
}

#[tokio::test]
async fn node_quality_budget_changes_actual_quic_probe_frequency() {
    use miopunch_native_probe::network_maintenance::MaintenancePolicy;
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let a = join(&mut admin, &dir.path().join("a")).await;
    let c = join(&mut admin, &dir.path().join("c")).await;
    // A must know C so its first ordinary session can be sampled directly.
    let a = {
        let mut a = a;
        for record in admin.management_history().await.unwrap() {
            a.ingest_management_record(record.header(), record.body())
                .await
                .unwrap();
        }
        a
    };
    for node in [&admin, &a, &c] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(Vec::new());
        config.maintenance.exploration_interval_ms = 250;
        node.save_resource_configuration(&config).unwrap();
    }
    admin.close().await;
    let c = c.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    a.close().await;
    let mut measured = Vec::new();
    // Reopen exactly the same durable A identity in the same controlled data
    // environment, varying only the maintenance quality interval.
    for interval in [5_000, 1_000] {
        let a = NativeNetwork::open(dir.path().join("a")).await.unwrap();
        let mut config = a.resource_configuration().unwrap();
        config.maintenance = MaintenancePolicy {
            exploration_interval_ms: 250,
            quality_interval_ms: interval,
            ..MaintenancePolicy::conservative()
        };
        a.save_resource_configuration(&config).unwrap();
        let a = a.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
        a.offer_contact(c.addr()).await.unwrap();
        wait_neighbors(&a, 1).await;
        let id = a.maintenance_status().neighbors[0].stable_id;
        let before = a.maintenance_status().neighbors[0].pings_sent;
        tokio::time::sleep(Duration::from_millis(3_500)).await;
        let status = a.maintenance_status();
        assert_eq!(status.neighbors[0].stable_id, id);
        measured.push(status.neighbors[0].pings_sent.saturating_sub(before));
        a.close().await;
        // C's peer-side connection promptly closes; keep its serving endpoint.
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    c.close().await;
    eprintln!(
        "OBSERVED actual_QUIC_ping_delta conservative={} active={} window_ms=3500",
        measured[0], measured[1]
    );
    assert!(
        measured[1] >= 3 && measured[1] > measured[0],
        "quality budget must affect QUIC probes, beyond saved configuration values"
    );
}

#[tokio::test]
async fn active_budget_never_enables_disabled_lookup_or_forbidden_data_carriers() {
    use miopunch_native_probe::network_maintenance::{MaintenancePolicy, MaintenanceState};
    let broker = std::env::var("MIOPUNCH_MQTT_BROKER").expect("owned real broker required");
    let dir = tempfile::tempdir().unwrap();
    let node = NativeNetwork::create(dir.path().join("node"))
        .await
        .unwrap();
    configure(&node, &broker);
    let mut config = node.resource_configuration().unwrap();
    config.maintenance = MaintenancePolicy::active();
    config.udp_direct_disabled = true;
    config.tcp_direct_disabled = true;
    config
        .disabled_uses
        .extend([ResourceUse::Lookup, ResourceUse::DataRelay]);
    node.save_resource_configuration(&config).unwrap();
    let (queries, observer) = observe_queries(&broker).await;
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let id = iroh::SecretKey::generate().public();
    let target = iroh::EndpointAddr::new(id).with_addrs([
        iroh::TransportAddr::Ip(socket.local_addr().unwrap()),
        iroh::TransportAddr::Custom(iroh_base::CustomAddr::from_parts(
            miopunch_native_probe::member_transport::DATA_TRANSPORT_ID,
            &[0; 32],
        )),
    ]);
    let node = node.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    for _ in 0..16 {
        node.offer_contact(target.clone()).await.unwrap();
        node.network_changed().await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let mut packet = [0; 2048];
    assert!(
        tokio::time::timeout(Duration::from_millis(100), socket.recv_from(&mut packet))
            .await
            .is_err()
    );
    let status = node.maintenance_status();
    assert_eq!(status.discovery_rounds, 0);
    assert_eq!(status.contact_attempts, 0);
    assert_eq!(queries.lock().unwrap().len(), 0);
    assert_eq!(status.state, MaintenanceState::TemporarilyUnreachable);
    assert!(status.cause.as_ref().unwrap().contains("no permitted"));
    assert!(status.node.members.len() == 1 && status.neighbors.is_empty());
    node.close().await;
    observer.abort();
    eprintln!(
        "OBSERVED active_budget=true disabled_lookup_queries=0 forbidden_data_dials=0 UDP_packets=0 waiting_retains_membership=true"
    );
}

#[tokio::test]
async fn stale_saved_contact_expands_to_discovery_and_recovers_an_unknown_member() {
    let broker = std::env::var("MIOPUNCH_MQTT_BROKER").expect("owned real broker required");
    let dir = tempfile::tempdir().unwrap();
    let mut b = NativeNetwork::create(dir.path().join("b")).await.unwrap();
    let a = join(&mut b, &dir.path().join("a")).await;
    for node in [&a, &b] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(Vec::new());
        config.maintenance.exploration_interval_ms = 250;
        config.maintenance.contact_timeout_ms = 500;
        node.save_resource_configuration(&config).unwrap();
    }
    let b = b.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    let a = a.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    a.offer_contact(b.addr()).await.unwrap();
    wait_neighbors(&a, 1).await;
    a.close().await;
    b.close().await;
    let mut b = NativeNetwork::open(dir.path().join("b")).await.unwrap();
    let c = join(&mut b, &dir.path().join("c")).await;
    let c_id = c.status().node_id;
    b.close().await;
    let a = NativeNetwork::open(dir.path().join("a")).await.unwrap();
    assert!(!a.status().members.iter().any(|(id, _)| *id == c_id));
    configure(&a, &broker);
    configure(&c, &broker);
    let c = c.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    let c_priority = c.business_activity(); // It serves but initiates no competing discovery.
    let a = a.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    tokio::time::timeout(Duration::from_secs(16), async {
        loop {
            if a.node_status().members.iter().any(|(id, _)| *id == c_id) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("the unavailable saved route expands to qualified discovery");
    let status = a.maintenance_status();
    assert!(status.contact_attempts >= 2 && status.discovery_rounds >= 1);
    assert_eq!(status.last_sync.as_ref().unwrap().source, c.endpoint().id());
    assert!(status.last_sync.as_ref().unwrap().complete());
    drop(c_priority);
    a.close().await;
    c.close().await;
    eprintln!(
        "OBSERVED saved_contact_unavailable=true discovery_expanded=true unknown_source_verified_and_saved=true"
    );
}

#[tokio::test]
async fn starting_business_does_not_abort_an_already_admitted_sync_exchange() {
    use miopunch_native_probe::{admission_gate::ProtocolScope, offline_replica::REPLICA_ALPN};
    struct HeldReplica {
        node: Arc<NativeNetwork>,
        accepted: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
        completed: Arc<tokio::sync::Notify>,
    }
    impl std::fmt::Debug for HeldReplica {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("HeldReplica")
        }
    }
    impl iroh::protocol::ProtocolHandler for HeldReplica {
        async fn accept(&self, connection: Connection) -> Result<(), iroh::protocol::AcceptError> {
            self.accepted.notify_one();
            self.release.notified().await;
            let result = self.node.serve_replica(&connection).await;
            if result.is_ok() {
                self.completed.notify_one();
            }
            while connection.close_reason().is_none() {
                if self.node.serve_replica(&connection).await.is_err() {
                    break;
                }
            }
            Ok(())
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let mut a = join(&mut admin, &dir.path().join("a")).await;
    let c = join(&mut admin, &dir.path().join("c")).await;
    for record in admin.management_history().await.unwrap() {
        a.ingest_management_record(record.header(), record.body())
            .await
            .unwrap();
    }
    admin.close().await;
    for node in [&a, &c] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(Vec::new());
        config.maintenance.exploration_interval_ms = 250;
        node.save_resource_configuration(&config).unwrap();
        node.admission_gate()
            .register_protocol(REPLICA_ALPN, ProtocolScope::Member);
    }
    let resources = c
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let accepted = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let completed = Arc::new(tokio::sync::Notify::new());
    let router = iroh::protocol::Router::builder(resources.endpoint().clone())
        .accept(
            REPLICA_ALPN,
            HeldReplica {
                node: Arc::new(c),
                accepted: accepted.clone(),
                release: release.clone(),
                completed: completed.clone(),
            },
        )
        .spawn();
    let a = a.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    a.offer_contact(resources.addr()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), accepted.notified())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let activity = a.business_activity();
    tokio::time::sleep(Duration::from_millis(100)).await;
    release.notify_one();
    let finished = tokio::time::timeout(Duration::from_secs(3), completed.notified())
        .await
        .is_ok();
    drop(activity);
    a.close().await;
    router.shutdown().await.unwrap();
    resources.close().await;
    assert!(
        finished,
        "business priority must stop new exploration while allowing the already admitted bounded LogSync exchange to finish"
    );
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn a_full_neighbor_budget_still_explores_a_new_source_without_requiring_full_mesh() {
    use miopunch_native_probe::{
        admission_gate::ProtocolScope, management_store::ManagementRecord,
        offline_replica::REPLICA_ALPN,
    };
    struct FrozenSource {
        node: Arc<NativeNetwork>,
        records: Vec<ManagementRecord>,
    }
    impl std::fmt::Debug for FrozenSource {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("FrozenSource")
        }
    }
    impl iroh::protocol::ProtocolHandler for FrozenSource {
        async fn accept(&self, connection: Connection) -> Result<(), iroh::protocol::AcceptError> {
            while connection.close_reason().is_none() {
                if self
                    .node
                    .serve_replica_fixture(&connection, &self.records)
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Ok(())
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let mut b = NativeNetwork::create(dir.path().join("b")).await.unwrap();
    let mut a = join(&mut b, &dir.path().join("a")).await;
    let mut c = join(&mut b, &dir.path().join("c")).await;
    let frozen = b.management_history().await.unwrap();
    for record in &frozen {
        a.ingest_management_record(record.header(), record.body())
            .await
            .unwrap();
    }
    let d = join(&mut b, &dir.path().join("d")).await;
    let d_id = d.status().node_id;
    d.close().await;
    for record in b.management_history().await.unwrap() {
        c.ingest_management_record(record.header(), record.body())
            .await
            .unwrap();
    }
    for node in [&a, &b, &c] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(Vec::new());
        config.maintenance.exploration_interval_ms = 250;
        node.save_resource_configuration(&config).unwrap();
        node.admission_gate()
            .register_protocol(REPLICA_ALPN, ProtocolScope::Member);
    }
    let b_resources = b
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let b_router = iroh::protocol::Router::builder(b_resources.endpoint().clone())
        .accept(
            REPLICA_ALPN,
            FrozenSource {
                node: Arc::new(b),
                records: frozen,
            },
        )
        .spawn();
    let c = c.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    let a = a.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    a.offer_contact(b_resources.addr()).await.unwrap();
    wait_neighbors(&a, 1).await;
    let original = a.maintenance_status().neighbors[0].stable_id;
    assert!(!a.node_status().members.iter().any(|(id, _)| *id == d_id));
    a.offer_contact(c.addr()).await.unwrap();
    let recovered = tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if a.node_status().members.iter().any(|(id, _)| *id == d_id) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .is_ok();
    let status = a.maintenance_status();
    a.close().await;
    c.close().await;
    b_router.shutdown().await.unwrap();
    b_resources.close().await;
    assert!(
        recovered,
        "a full permanent neighbor view still samples a newly reachable source for better state coverage"
    );
    assert_eq!(status.neighbors.len(), 1);
    assert_eq!(status.neighbors[0].stable_id, original);
}
