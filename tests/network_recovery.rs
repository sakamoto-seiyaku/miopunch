//! Public startup recovery with independent durable identities and a real broker.
#[path = "support/native_connections.rs"]
mod native_connections;
use iroh::endpoint::RestrictedEndpoint;

use iroh::{EndpointAddr, endpoint::Connection};
use miopunch_native_probe::{
    admission_gate::{ADMISSION_REQUEST_ALPN, ProtocolScope},
    manual_admission::ManualExchange,
    native_network::NativeNetwork,
    network_recovery::RecoveryStatus,
    resources::{Resource, ResourceUse},
};
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use std::{
    collections::BTreeSet,
    net::{Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::Duration,
};
const BUSINESS: &[u8] = b"miopunch/recovery-business/1";

async fn observer(
    broker: SocketAddr,
) -> (
    Arc<Mutex<Vec<(String, Vec<u8>)>>>,
    tokio::task::JoinHandle<()>,
) {
    let mut options = MqttOptions::new(
        format!("mio46-capture-{}", rand::random::<u64>()),
        broker.ip().to_string(),
        broker.port(),
    );
    options.set_keep_alive(Duration::from_secs(5));
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
    let captured = Arc::new(Mutex::new(Vec::new()));
    let captured_task = captured.clone();
    let task = tokio::spawn(async move {
        while let Ok(event) = events.poll().await {
            if let Event::Incoming(Packet::Publish(publish)) = event {
                captured_task
                    .lock()
                    .unwrap()
                    .push((publish.topic, publish.payload.to_vec()));
            }
        }
    });
    (captured, task)
}

async fn connect(
    client: &RestrictedEndpoint,
    server: &RestrictedEndpoint,
    alpn: &[u8],
) -> (Connection, Connection) {
    native_connections::member_pair(client, server, alpn).await
}
async fn join(c: &mut NativeNetwork, path: &std::path::Path) -> NativeNetwork {
    let mut node = NativeNetwork::join(path, c.manual_invitation().unwrap())
        .await
        .unwrap();
    let ce = c.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let ne = node.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = native_connections::control_pair(&ne, &ce, ADMISSION_REQUEST_ALPN).await;
    let (request, served) = tokio::join!(
        node.request_manual_admission(&client),
        c.serve_manual_admission(&server)
    );
    assert!(matches!(request.unwrap(), ManualExchange::AwaitingApproval));
    served.unwrap();
    let request = c.manual_requests().await.unwrap().pop().unwrap();
    c.approve_manual_admission(&request).await.unwrap();
    let receiver = async {
        let ManualExchange::Materials(material) =
            node.request_manual_admission(&client).await.unwrap()
        else {
            panic!("approval")
        };
        node.complete_manual_admission(material).await.unwrap();
    };
    let (_, served) = tokio::join!(receiver, c.serve_manual_admission(&server));
    served.unwrap();
    client.close(0u32.into(), b"done");
    server.close(0u32.into(), b"done");
    ne.close().await;
    ce.close().await;
    node
}

fn configure(node: &NativeNetwork, broker: &str) {
    let mut config = node.resource_configuration().unwrap();
    config.replacement = Some(vec![Resource {
        url: format!("mqtt://{broker}/"),
        uses: BTreeSet::from([ResourceUse::Signaling, ResourceUse::Lookup]),
        limitation: "owned real MQTT recovery fixture; signaling only".into(),
    }]);
    node.save_resource_configuration(&config).unwrap();
    node.admission_gate()
        .register_protocol(BUSINESS, ProtocolScope::Member);
}

#[tokio::test]
async fn old_member_recovers_from_the_only_online_unknown_member_at_startup() {
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut b = NativeNetwork::create(dir.path().join("b")).await.unwrap();
    let a = join(&mut b, &dir.path().join("a")).await;
    a.close().await; // A misses C's later authorization, preserving its stale local view.
    let c = join(&mut b, &dir.path().join("c")).await;
    b.close().await; // B and the original author remain offline throughout recovery.
    let mut a = NativeNetwork::open(dir.path().join("a")).await.unwrap();
    configure(&a, &broker);
    configure(&c, &broker);
    assert!(
        !a.status()
            .members
            .iter()
            .any(|(id, _)| id.as_bytes() == c.status().node_id.as_bytes())
    );
    let (captured, capture) = observer(broker.parse().unwrap()).await;
    let c_endpoint = c
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    c_endpoint.endpoint().set_alpns(vec![
        miopunch_native_probe::network_recovery::RECOVERY_REPLICA_ALPN.to_vec(),
        BUSINESS.to_vec(),
    ]);
    let serving = async {
        let connection = c_endpoint.endpoint().accept().await.unwrap().await.unwrap();
        c.serve_network_recovery(&c_endpoint, &connection)
            .await
            .unwrap();
    };
    let (recovered, ()) = tokio::time::timeout(Duration::from_secs(30), async {
        tokio::join!(a.resume_network(Ipv4Addr::LOCALHOST), serving)
    })
    .await
    .unwrap();
    let recovered = recovered.unwrap();
    let RecoveryStatus::Ready(outcome) = &recovered.status else {
        panic!("startup must automatically discover C and verify/save its replica");
    };
    assert_eq!(outcome.source, c_endpoint.endpoint().id());
    assert!(outcome.complete());
    let source = outcome.source;
    let (business, accepted) = tokio::join!(
        recovered
            .resources
            .connect(EndpointAddr::new(source), BUSINESS),
        async { c_endpoint.endpoint().accept().await.unwrap().await }
    );
    let business = business.unwrap();
    let accepted = accepted.unwrap();
    assert!(a.admission_gate().allows(&business, ProtocolScope::Member));
    let sender = async {
        let mut stream = business.open_uni().await.unwrap();
        stream
            .write_all(b"member communication after recovery")
            .await
            .unwrap();
        stream.finish().unwrap();
    };
    let receiver = async {
        let mut stream = accepted.accept_uni().await.unwrap();
        assert_eq!(
            stream.read_to_end(64).await.unwrap(),
            b"member communication after recovery"
        );
    };
    tokio::join!(sender, receiver);
    business.close(0u32.into(), b"done");
    accepted.close(0u32.into(), b"done");
    tokio::time::sleep(Duration::from_millis(100)).await;
    let publications = captured.lock().unwrap().clone();
    capture.abort();
    let secret = std::fs::read(dir.path().join("b/admission.secret")).unwrap();
    let queries: Vec<_> = publications
        .iter()
        .filter(|(_, bytes)| bytes.starts_with(b"MDQ1"))
        .collect();
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].1.len(), 20);
    let response = publications
        .iter()
        .find(|(_, bytes)| {
            bytes.starts_with(b"MDR2") && &bytes[20..52] == c_endpoint.endpoint().id().as_bytes()
        })
        .unwrap();
    assert_eq!(response.1.len(), 90);
    assert_eq!(&response.1[4..20], &queries[0].1[4..20]);
    let route = c_endpoint
        .endpoint()
        .custom_addrs()
        .into_iter()
        .find(|address| address.id() == miopunch_native_probe::mqtt_transport::TRANSPORT_ID)
        .unwrap();
    assert_eq!(&response.1[52..84], route.data());
    let c_ip = *c_endpoint.addr().ip_addrs().next().unwrap();
    let SocketAddr::V4(c_ip) = c_ip else {
        panic!("IPv4 profile");
    };
    assert_eq!(&response.1[84..88], &c_ip.ip().octets());
    assert_eq!(&response.1[88..90], &c_ip.port().to_be_bytes());
    assert!(
        publications
            .iter()
            .all(|(topic, bytes)| !topic.contains(&hex::encode(&secret))
                && !bytes.windows(secret.len()).any(|value| value == secret))
    );
    assert!(
        publications
            .iter()
            .all(|(_, bytes)| bytes.starts_with(b"MDQ1")
                || bytes.starts_with(b"MDR2")
                || bytes.starts_with(b"MIO1"))
    );
    println!(
        "RECOVERY_PUBLIC_FIELDS query=20 response=90 route=32 private_secret_exposed=0 publishes={}",
        publications.len()
    );
    println!(
        "UNKNOWN_RECOVERY a_knows_c_initially=0 b_online=0 real_mqtt=1 durable_logsync=1 authorized_business=1"
    );
    recovered.resources.close().await;
    c_endpoint.close().await;
    let c_id = c.status().node_id;
    a.close().await;
    c.close().await;
    let reopened = NativeNetwork::open(dir.path().join("a")).await.unwrap();
    assert!(reopened.status().members.iter().any(|(id, _)| *id == c_id));
    println!("UNKNOWN_RECOVERY_REOPEN verified_c_membership_saved=1");
    reopened.close().await;
}

#[tokio::test]
async fn old_secret_with_a_new_identity_keeps_only_bootstrap_scope() {
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut b = NativeNetwork::create(dir.path().join("b")).await.unwrap();
    let a = join(&mut b, &dir.path().join("a")).await;
    let c = join(&mut b, &dir.path().join("c")).await;
    a.close().await;
    b.close().await;
    // A former holder preserves all previously valid private admission material
    // but adopts a fresh independent device identity. This grants no membership.
    let fresh = p2panda_core::SigningKey::generate();
    std::fs::write(dir.path().join("a/node.key"), fresh.as_bytes()).unwrap();
    let mut outsider = NativeNetwork::open(dir.path().join("a")).await.unwrap();
    configure(&outsider, &broker);
    configure(&c, &broker);
    let before = outsider.status();
    let c_endpoint = c
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    c_endpoint.endpoint().set_alpns(vec![
        miopunch_native_probe::network_recovery::RECOVERY_REPLICA_ALPN.to_vec(),
        BUSINESS.to_vec(),
    ]);
    let serve = async {
        let incoming = c_endpoint.endpoint().accept().await.unwrap().await.unwrap();
        assert!(
            c.admission_gate()
                .allows(&incoming, ProtocolScope::Bootstrap)
        );
        assert!(c.admission_gate().proof_for(&incoming).is_some());
        assert!(!c.admission_gate().allows(&incoming, ProtocolScope::Member));
        assert!(
            c.serve_network_recovery(&c_endpoint, &incoming)
                .await
                .is_err()
        );
    };
    let (recovered, ()) = tokio::time::timeout(Duration::from_secs(30), async {
        tokio::join!(outsider.resume_network(Ipv4Addr::LOCALHOST), serve)
    })
    .await
    .unwrap();
    let recovered = recovered.unwrap();
    assert!(matches!(recovered.status, RecoveryStatus::Waiting));
    assert_eq!(outsider.status(), before); // no C record was disclosed or saved
    let (business, incoming) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(
            recovered
                .resources
                .endpoint()
                .connect(c_endpoint.addr(), BUSINESS),
            async { c_endpoint.endpoint().accept().await.unwrap().await }
        )
    })
    .await
    .unwrap();
    assert!(business.is_err() && incoming.is_err());
    println!(
        "UNKNOWN_RECOVERY_REJECTION old_secret=1 new_identity=1 scoped_scram=1 history_disclosed=0 member_connection=0"
    );
    recovered.resources.close().await;
    c_endpoint.close().await;
    outsider.close().await;
    c.close().await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn missing_causal_dependency_waits_then_recovers_from_the_saved_source() {
    use miopunch_native_probe::offline_replica::REPLICA_ALPN;
    use p2panda_core::Hash;
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut b = NativeNetwork::create(dir.path().join("b")).await.unwrap();
    let a = join(&mut b, &dir.path().join("a")).await;
    a.close().await;
    let mut c = join(&mut b, &dir.path().join("c")).await;
    let d = join(&mut b, &dir.path().join("d")).await;
    let missing = Hash::digest(
        b.management_history()
            .await
            .unwrap()
            .last()
            .unwrap()
            .header(),
    );
    d.close().await;
    let e = join(&mut b, &dir.path().join("e")).await;
    let pending = Hash::digest(
        b.management_history()
            .await
            .unwrap()
            .last()
            .unwrap()
            .header(),
    );
    e.close().await;
    for node in [&b, &c] {
        node.admission_gate()
            .register_protocol(REPLICA_ALPN, ProtocolScope::Member);
    }
    let be = b.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    be.set_alpns(vec![REPLICA_ALPN.to_vec()]);
    let ce = c.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = connect(&ce, &be, REPLICA_ALPN).await;
    let (received, served) = tokio::join!(c.sync_replica(&client), b.serve_replica(&server));
    assert!(received.unwrap().complete());
    served.unwrap();
    client.close(0u32.into(), b"seed");
    server.close(0u32.into(), b"seed");
    ce.close().await;
    be.close().await;
    b.close().await;
    let mut a = NativeNetwork::open(dir.path().join("a")).await.unwrap();
    configure(&a, &broker);
    configure(&c, &broker);
    let endpoint = c
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let incomplete: Vec<_> = c
        .management_history()
        .await
        .unwrap()
        .into_iter()
        .filter(|record| Hash::digest(record.header()) != missing)
        .collect();
    let serving = async {
        let connection = endpoint.endpoint().accept().await.unwrap().await.unwrap();
        c.serve_network_recovery_fixture(&endpoint, &connection, &incomplete)
            .await
            .unwrap();
    };
    let (recovered, ()) = tokio::time::timeout(Duration::from_secs(30), async {
        tokio::join!(a.resume_network(Ipv4Addr::LOCALHOST), serving)
    })
    .await
    .unwrap();
    let mut recovered = recovered.unwrap();
    assert!(
        matches!(recovered.status, RecoveryStatus::WaitingForDependencies(_)),
        "transport Done with missing causal history must explicitly wait for dependencies"
    );
    assert!(a.management_record(pending).await.unwrap().is_none());
    let serving = async {
        let connection = endpoint.endpoint().accept().await.unwrap().await.unwrap();
        c.serve_network_recovery(&endpoint, &connection)
            .await
            .unwrap();
    };
    let (status, ()) = tokio::time::timeout(Duration::from_secs(30), async {
        tokio::join!(a.recover_network(&mut recovered.resources), serving)
    })
    .await
    .unwrap();
    let RecoveryStatus::Ready(outcome) = status else {
        panic!("missing record must be fetched by a fresh source exchange");
    };
    assert!(outcome.complete());
    assert!(a.management_record(missing).await.unwrap().is_some());
    assert!(a.management_record(pending).await.unwrap().is_some());
    println!(
        "UNKNOWN_RECOVERY_DEPENDENCY partial_saved=1 causal_wait=1 retried_source=1 complete=1"
    );
    recovered.resources.close().await;
    endpoint.close().await;
    a.close().await;
    c.close().await;
}

#[tokio::test]
async fn no_online_candidate_waits_and_new_contact_recovers_on_the_same_endpoint() {
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut b = NativeNetwork::create(dir.path().join("b")).await.unwrap();
    let a = join(&mut b, &dir.path().join("a")).await;
    a.close().await;
    let c = join(&mut b, &dir.path().join("c")).await;
    b.close().await;
    let mut a = NativeNetwork::open(dir.path().join("a")).await.unwrap();
    configure(&a, &broker);
    configure(&c, &broker);
    let mut recovered = a.resume_network(Ipv4Addr::LOCALHOST).await.unwrap();
    assert!(matches!(recovered.status, RecoveryStatus::Waiting));
    let id = recovered.resources.endpoint().id();
    let c_endpoint = c
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let serving = async {
        let connection = c_endpoint.endpoint().accept().await.unwrap().await.unwrap();
        c.serve_network_recovery(&c_endpoint, &connection)
            .await
            .unwrap();
    };
    let (status, ()) = tokio::time::timeout(Duration::from_secs(30), async {
        tokio::join!(a.recover_network(&mut recovered.resources), serving)
    })
    .await
    .unwrap();
    assert!(matches!(status, RecoveryStatus::Ready(_)));
    assert_eq!(id, recovered.resources.endpoint().id());
    println!("UNKNOWN_RECOVERY_RETRY no_candidate_wait=1 new_contact=1 same_endpoint=1");
    recovered.resources.close().await;
    c_endpoint.close().await;
    a.close().await;
    c.close().await;
}

async fn entry_condition(duplicate_hint: bool) {
    use miopunch_native_probe::{
        admission_proof::AdmissionScope,
        mqtt_discovery::{Candidate, respond},
        mqtt_transport::TRANSPORT_ID,
    };
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut b = NativeNetwork::create(dir.path().join("b")).await.unwrap();
    let a = join(&mut b, &dir.path().join("a")).await;
    a.close().await;
    let c = join(&mut b, &dir.path().join("c")).await;
    b.close().await;
    let mut a = NativeNetwork::open(dir.path().join("a")).await.unwrap();
    configure(&a, &broker);
    configure(&c, &broker);
    if !duplicate_hint {
        let unused = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let mut config = a.resource_configuration().unwrap();
        config.replacement.as_mut().unwrap().push(Resource {
            url: format!("mqtt://{unused}/"),
            uses: BTreeSet::from([ResourceUse::Lookup, ResourceUse::Signaling]),
            limitation: "deliberately unavailable owned entry".into(),
        });
        a.save_resource_configuration(&config).unwrap();
    }
    let c_endpoint = c
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let route = c_endpoint
        .endpoint()
        .custom_addrs()
        .into_iter()
        .find(|addr| addr.id() == TRANSPORT_ID)
        .unwrap();
    let SocketAddr::V4(udp_addr) = *c_endpoint.addr().ip_addrs().next().unwrap() else {
        panic!("IPv4 profile");
    };
    let candidate = Candidate::<32> {
        id: c_endpoint.endpoint().id(),
        route: route.data().try_into().unwrap(),
        udp_addr,
    };
    let duplicate = if duplicate_hint {
        Some(
            respond(
                broker.parse().unwrap(),
                AdmissionScope::new(*c.status().anchor.root.as_bytes(), 0),
                candidate,
            )
            .await
            .unwrap(),
        )
    } else {
        None
    };
    let serving = async {
        let connection = c_endpoint.endpoint().accept().await.unwrap().await.unwrap();
        c.serve_network_recovery(&c_endpoint, &connection)
            .await
            .unwrap();
    };
    let (recovered, ()) = tokio::time::timeout(Duration::from_secs(30), async {
        tokio::join!(a.resume_network(Ipv4Addr::LOCALHOST), serving)
    })
    .await
    .unwrap();
    let recovered = recovered.unwrap();
    let RecoveryStatus::Ready(outcome) = &recovered.status else {
        panic!("another allowed entry must remain usable");
    };
    assert!(outcome.complete());
    assert_eq!(a.status().applied_count, c.status().applied_count);
    if let Some(duplicate) = duplicate {
        duplicate.abort();
    }
    println!(
        "UNKNOWN_RECOVERY_ENTRIES partial_failure={} duplicate_hints={} authorized_source=1",
        !duplicate_hint, duplicate_hint
    );
    recovered.resources.close().await;
    c_endpoint.close().await;
    a.close().await;
    c.close().await;
}

#[tokio::test]
async fn a_failed_entry_does_not_prevent_recovery() {
    entry_condition(false).await;
}

#[tokio::test]
async fn duplicate_tagged_hints_do_not_prevent_recovery() {
    entry_condition(true).await;
}

#[tokio::test]
async fn a_false_candidate_without_the_network_secret_never_supplies_history() {
    use miopunch_native_probe::{
        admission_proof::AdmissionScope,
        mqtt_discovery::{Candidate, respond},
        mqtt_transport::TRANSPORT_ID,
    };
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut b = NativeNetwork::create(dir.path().join("b")).await.unwrap();
    let a = join(&mut b, &dir.path().join("a")).await;
    a.close().await;
    let c = join(&mut b, &dir.path().join("c")).await;
    b.close().await;
    let fake = NativeNetwork::create(dir.path().join("fake"))
        .await
        .unwrap();
    configure(&fake, &broker);
    let fake_endpoint = fake
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let route = fake_endpoint
        .endpoint()
        .custom_addrs()
        .into_iter()
        .find(|addr| addr.id() == TRANSPORT_ID)
        .unwrap();
    let SocketAddr::V4(udp_addr) = *fake_endpoint.addr().ip_addrs().next().unwrap() else {
        panic!("IPv4");
    };
    let false_hint = respond(
        broker.parse().unwrap(),
        AdmissionScope::new(*c.status().anchor.root.as_bytes(), 0),
        Candidate::<32> {
            id: fake_endpoint.endpoint().id(),
            route: route.data().try_into().unwrap(),
            udp_addr,
        },
    )
    .await
    .unwrap();
    let mut a = NativeNetwork::open(dir.path().join("a")).await.unwrap();
    configure(&a, &broker);
    configure(&c, &broker);
    let before = a.status();
    let rejecting = async {
        assert!(
            fake_endpoint
                .endpoint()
                .accept()
                .await
                .unwrap()
                .await
                .is_err()
        );
    };
    let (recovered, ()) = tokio::time::timeout(Duration::from_secs(30), async {
        tokio::join!(a.resume_network(Ipv4Addr::LOCALHOST), rejecting)
    })
    .await
    .unwrap();
    let mut recovered = recovered.unwrap();
    assert!(matches!(recovered.status, RecoveryStatus::Waiting));
    assert_eq!(a.status(), before);
    false_hint.abort();
    fake_endpoint.close().await;
    fake.close().await;
    let ce = c
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let serving = async {
        let connection = ce.endpoint().accept().await.unwrap().await.unwrap();
        c.serve_network_recovery(&ce, &connection).await.unwrap();
    };
    let (status, ()) = tokio::time::timeout(Duration::from_secs(30), async {
        tokio::join!(a.recover_network(&mut recovered.resources), serving)
    })
    .await
    .unwrap();
    assert!(matches!(status, RecoveryStatus::Ready(_)));
    println!(
        "UNKNOWN_RECOVERY_FALSE_HINT false_candidate=1 no_secret=1 saved_history=0 subsequent_legitimate_recovery=1"
    );
    recovered.resources.close().await;
    ce.close().await;
    a.close().await;
    c.close().await;
}

#[tokio::test]
async fn disabling_lookup_does_not_reenable_discovery_via_signaling() {
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut b = NativeNetwork::create(dir.path().join("b")).await.unwrap();
    let a = join(&mut b, &dir.path().join("a")).await;
    a.close().await;
    let c = join(&mut b, &dir.path().join("c")).await;
    b.close().await;
    let mut a = NativeNetwork::open(dir.path().join("a")).await.unwrap();
    configure(&a, &broker);
    configure(&c, &broker);
    let mut config = a.resource_configuration().unwrap();
    config.disabled_uses.insert(ResourceUse::Lookup);
    a.save_resource_configuration(&config).unwrap();
    let ce = c
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let (captured, capture) = observer(broker.parse().unwrap()).await;
    let recovered = a.resume_network(Ipv4Addr::LOCALHOST).await.unwrap();
    assert!(matches!(recovered.status, RecoveryStatus::Waiting));
    assert!(
        captured
            .lock()
            .unwrap()
            .iter()
            .all(|(_, bytes)| !bytes.starts_with(b"MDQ1"))
    );
    assert!(
        recovered
            .resources
            .endpoint()
            .custom_addrs()
            .iter()
            .any(|route| route.id() == miopunch_native_probe::mqtt_transport::TRANSPORT_ID)
    );
    capture.abort();
    println!("UNKNOWN_RECOVERY_CONSENT lookup_disabled=1 signaling_live=1 discovery_queries=0");
    recovered.resources.close().await;
    ce.close().await;
    a.close().await;
    c.close().await;
}

#[tokio::test]
async fn missing_admission_material_cannot_start_network_history_recovery() {
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let b = NativeNetwork::create(dir.path().join("b")).await.unwrap();
    let mut pending =
        NativeNetwork::join(dir.path().join("pending"), b.manual_invitation().unwrap())
            .await
            .unwrap();
    configure(&pending, &broker);
    configure(&b, &broker);
    let be = b
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let recovered = pending.resume_network(Ipv4Addr::LOCALHOST).await.unwrap();
    assert!(matches!(recovered.status, RecoveryStatus::Waiting));
    assert!(pending.management_history().await.unwrap().is_empty());
    println!("UNKNOWN_RECOVERY_MATERIAL no_secret=1 no_history=1 waiting=1");
    recovered.resources.close().await;
    be.close().await;
    pending.close().await;
    b.close().await;
}

#[tokio::test]
async fn another_network_on_the_same_entry_is_never_a_recovery_source() {
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut a = NativeNetwork::create(dir.path().join("a")).await.unwrap();
    let c = NativeNetwork::create(dir.path().join("c")).await.unwrap();
    configure(&a, &broker);
    configure(&c, &broker);
    let ce = c
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let before = a.status();
    let recovered = a.resume_network(Ipv4Addr::LOCALHOST).await.unwrap();
    assert!(matches!(recovered.status, RecoveryStatus::Waiting));
    assert_eq!(a.status(), before);
    println!("UNKNOWN_RECOVERY_NETWORK wrong_network=1 source_accepted=0");
    recovered.resources.close().await;
    ce.close().await;
    a.close().await;
    c.close().await;
}

#[tokio::test]
async fn a_mismatched_saved_generation_does_not_fetch_current_member_history() {
    use miopunch_native_probe::{
        admission_proof::{AdmissionScope, AdmissionSecret},
        management_store::ManagementStore,
    };
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut b = NativeNetwork::create(dir.path().join("b")).await.unwrap();
    let a = join(&mut b, &dir.path().join("a")).await;
    let anchor = a.status().anchor;
    a.close().await;
    let c = join(&mut b, &dir.path().join("c")).await;
    b.close().await;
    // Explicit persisted fault: this device labels its old secret as a different
    // generation. The online network remains generation zero.
    let path = dir.path().join("a/admission.material");
    let mut material = std::fs::read(&path).unwrap();
    material[..8].copy_from_slice(&1u64.to_be_bytes());
    std::fs::write(path, &material).unwrap();
    let mut store = ManagementStore::open(dir.path().join("a/management.sqlite"), anchor)
        .await
        .unwrap();
    store
        .set_current_admission(
            &AdmissionScope::new(*anchor.root.as_bytes(), 1),
            &AdmissionSecret::from_saved(material[8..].try_into().unwrap()),
        )
        .await
        .unwrap();
    store.close().await;
    let mut a = NativeNetwork::open(dir.path().join("a")).await.unwrap();
    configure(&a, &broker);
    configure(&c, &broker);
    let ce = c
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let before = a.status();
    let recovered = a.resume_network(Ipv4Addr::LOCALHOST).await.unwrap();
    assert!(matches!(recovered.status, RecoveryStatus::Waiting));
    assert_eq!(a.status(), before);
    println!("UNKNOWN_RECOVERY_GENERATION wrong_generation=1 source_accepted=0");
    recovered.resources.close().await;
    ce.close().await;
    a.close().await;
    c.close().await;
}

#[tokio::test]
async fn a_known_revoked_candidate_is_rejected_before_any_history_request() {
    use miopunch_native_probe::management_auth::ManagementExtension;
    use p2panda_auth::{
        GroupsExtensionArgs,
        group::{GroupAction, GroupMember},
    };
    use p2panda_core::{Hash, Header, SigningKey};
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut b = NativeNetwork::create(dir.path().join("b")).await.unwrap();
    let mut a = join(&mut b, &dir.path().join("a")).await;
    let c = join(&mut b, &dir.path().join("c")).await;
    let history = b.management_history().await.unwrap();
    for record in &history {
        a.ingest_management_record(record.header(), record.body())
            .await
            .unwrap();
    }
    let key = SigningKey::from_bytes(
        &std::fs::read(dir.path().join("b/node.key"))
            .unwrap()
            .try_into()
            .unwrap(),
    );
    let removal = Header::<ManagementExtension>::builder()
        .build(
            &key,
            ManagementExtension {
                version: 1,
                log_id: rand::random(),
                group: GroupsExtensionArgs {
                    group_id: b.status().anchor.root,
                    action: GroupAction::Remove {
                        member: GroupMember::Individual(c.status().node_id),
                    },
                    dependencies: vec![Hash::digest(history.last().unwrap().header())],
                },
            },
        )
        .encode();
    a.ingest_management_record(&removal, None).await.unwrap();
    b.close().await;
    configure(&a, &broker);
    configure(&c, &broker);
    let ce = c
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let (captured, capture) = observer(broker.parse().unwrap()).await;
    let before = a.status();
    let recovered = a.resume_network(Ipv4Addr::LOCALHOST).await.unwrap();
    assert!(matches!(recovered.status, RecoveryStatus::Waiting));
    assert_eq!(a.status(), before);
    let packets = captured.lock().unwrap().clone();
    capture.abort();
    assert!(
        packets.iter().any(|(_, bytes)| bytes.starts_with(b"MDR2")
            && &bytes[20..52] == ce.endpoint().id().as_bytes())
    );
    assert!(packets.iter().all(|(_, bytes)| !bytes.starts_with(b"MIO1")));
    println!(
        "UNKNOWN_RECOVERY_REVOCATION advertised_candidate=1 known_removed=1 handshake_packets=0 history_requested=0"
    );
    recovered.resources.close().await;
    ce.close().await;
    a.close().await;
    c.close().await;
}
