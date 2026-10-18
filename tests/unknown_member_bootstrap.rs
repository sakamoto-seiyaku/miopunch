//! MIOPUNCH-26: A has only offline B, discovers previously unknown online C.
use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::Duration,
};

use iroh::{
    Endpoint, EndpointAddr,
    endpoint::{
        AdmissionOutcome, ControlConfirmation, ControlConnection, EndpointAdmission,
        RestrictedEndpoint, presets,
    },
};
use iroh_base::{SecretKey, TransportAddr};
use miopunch_native_probe::{
    admission_proof::{AdmissionScope, AdmissionSecret, prove_client, prove_server},
    management_auth::{ManagementExtension, NetworkAnchor},
    management_store::{ManagementRecord, ManagementStore},
    member_bootstrap::{MEMBER_BOOTSTRAP_ALPN, fetch_control_history, serve_control_history},
    mqtt_discovery::{Candidate, discover, respond},
    mqtt_transport::{MqttTransport, Traffic},
};
use p2panda_auth::{
    Access, GroupsExtensionArgs,
    group::{GroupAction, GroupMember},
};
use p2panda_core::{Hash, Header, SigningKey, VerifyingKey};
use rumqttc::{AsyncClient, Event, MqttOptions, Outgoing, Packet, QoS};
use sha2::{Digest, Sha256};

/// The component protocol proves SCRAM and signed membership after the control
/// handshake; TLS alone never releases an ordinary business connection.
#[derive(Debug)]
struct HistoryControlOnly;
impl EndpointAdmission for HistoryControlOnly {
    async fn admit(&self, _: &ControlConnection) -> std::io::Result<AdmissionOutcome> {
        Ok(AdmissionOutcome::ControlOnlyWithConfirmation(
            ControlConfirmation::new(async { Ok(()) }),
        ))
    }
    fn is_current(&self, control: &ControlConnection) -> bool {
        control.close_reason().is_none()
    }
}

async fn endpoint(
    broker: SocketAddr,
    key: &SigningKey,
) -> (RestrictedEndpoint, Arc<MqttTransport>) {
    let carrier = Arc::new(MqttTransport::new(broker, Arc::new(Traffic::default())));
    let endpoint = Endpoint::builder(presets::Minimal)
        .secret_key(SecretKey::from_bytes(key.as_bytes()))
        .alpns(vec![MEMBER_BOOTSTRAP_ALPN.to_vec()])
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .clear_relay_transports()
        .add_custom_transport(carrier.clone())
        .restricted_application(HistoryControlOnly)
        .bind()
        .await
        .unwrap();
    carrier.wait_ready().await.unwrap();
    (endpoint, carrier)
}

fn create(root: &SigningKey, admin: &SigningKey) -> ManagementRecord {
    ManagementRecord::new(
        Header::<ManagementExtension>::builder()
            .build(
                root,
                ManagementExtension {
                    version: 1,
                    log_id: [1; 16],
                    group: GroupsExtensionArgs {
                        group_id: root.verifying_key(),
                        action: GroupAction::Create {
                            initial_members: vec![
                                (
                                    GroupMember::Individual(root.verifying_key()),
                                    Access::manage(),
                                ),
                                (
                                    GroupMember::Individual(admin.verifying_key()),
                                    Access::manage(),
                                ),
                            ],
                        },
                        dependencies: vec![],
                    },
                },
            )
            .encode(),
        None,
    )
}

fn add(
    admin: &SigningKey,
    root: VerifyingKey,
    member: &SigningKey,
    dependency: Hash,
    log_id: u8,
) -> ManagementRecord {
    ManagementRecord::new(
        Header::<ManagementExtension>::builder()
            .build(
                admin,
                ManagementExtension {
                    version: 1,
                    log_id: [log_id; 16],
                    group: GroupsExtensionArgs {
                        group_id: root,
                        action: GroupAction::Add {
                            member: GroupMember::Individual(member.verifying_key()),
                            access: Access::read(),
                        },
                        dependencies: vec![dependency],
                    },
                },
            )
            .encode(),
        None,
    )
}

fn remove(
    admin: &SigningKey,
    root: VerifyingKey,
    member: &SigningKey,
    dependency: Hash,
    log_id: u8,
) -> ManagementRecord {
    ManagementRecord::new(
        Header::<ManagementExtension>::builder()
            .build(
                admin,
                ManagementExtension {
                    version: 1,
                    log_id: [log_id; 16],
                    group: GroupsExtensionArgs {
                        group_id: root,
                        action: GroupAction::Remove {
                            member: GroupMember::Individual(member.verifying_key()),
                        },
                        dependencies: vec![dependency],
                    },
                },
            )
            .encode(),
        None,
    )
}

async fn observer(
    broker: SocketAddr,
) -> (
    Arc<Mutex<Vec<(String, Vec<u8>)>>>,
    tokio::task::JoinHandle<()>,
) {
    let mut options = MqttOptions::new(
        format!("mio26-capture-{}", rand::random::<u64>()),
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

async fn seed(store: &mut ManagementStore, records: &[ManagementRecord]) {
    for record in records {
        store.ingest(record).await.unwrap();
    }
    store.apply_ready().await.unwrap();
}

fn candidate(endpoint: &RestrictedEndpoint, carrier: &MqttTransport) -> Candidate {
    Candidate {
        id: endpoint.id(),
        route: hex::decode(carrier.route().unwrap())
            .unwrap()
            .try_into()
            .unwrap(),
        udp_addr: endpoint
            .bound_sockets()
            .iter()
            .copied()
            .find_map(|addr| match addr {
                SocketAddr::V4(addr) => Some(addr),
                _ => None,
            })
            .unwrap(),
    }
}

fn target(candidate: Candidate) -> EndpointAddr {
    EndpointAddr::new(candidate.id).with_addrs([TransportAddr::Custom(
        MqttTransport::address(&hex::encode(candidate.route)).unwrap(),
    )])
}

#[tokio::test]
async fn offline_known_contact_does_not_block_unknown_member_bootstrap() {
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let broker: SocketAddr = broker.parse().unwrap();
    let root = SigningKey::generate();
    let a_key = SigningKey::generate();
    let b_key = SigningKey::generate(); // B is the offline original approver.
    let c_key = SigningKey::generate();
    let create = create(&root, &b_key);
    let anchor = NetworkAnchor {
        root: root.verifying_key(),
        genesis: Hash::digest(create.header()),
    };
    let add_a = add(&b_key, anchor.root, &a_key, anchor.genesis, 2);
    let c = add(&b_key, anchor.root, &c_key, Hash::digest(add_a.header()), 3);
    assert_eq!(
        Header::<ManagementExtension>::decode(c.header())
            .unwrap()
            .verifying_key,
        b_key.verifying_key()
    );
    let a_dir = tempfile::tempdir().unwrap();
    let c_dir = tempfile::tempdir().unwrap();
    let mut a_store = ManagementStore::open(a_dir.path().join("state.sqlite"), anchor)
        .await
        .unwrap();
    let mut c_store = ManagementStore::open(c_dir.path().join("state.sqlite"), anchor)
        .await
        .unwrap();
    seed(&mut a_store, &[create.clone(), add_a.clone()]).await;
    seed(&mut c_store, &[create, add_a, c.clone()]).await;
    assert_eq!(a_store.nodes().len(), 2);
    assert!(
        !a_store
            .nodes()
            .iter()
            .any(|(id, _)| *id == c_key.verifying_key())
    );

    let scope = AdmissionScope::new(*anchor.root.as_bytes(), 7);
    let secret = AdmissionSecret::generate();
    a_store
        .set_current_admission(&scope, &secret)
        .await
        .unwrap();
    c_store
        .set_current_admission(&scope, &secret)
        .await
        .unwrap();
    let (a, a_carrier) = endpoint(broker, &a_key).await;
    let (c_endpoint, c_carrier) = endpoint(broker, &c_key).await;
    let (captured, capture_task) = observer(broker).await;
    let responder = respond(broker, scope.clone(), candidate(&c_endpoint, &c_carrier))
        .await
        .unwrap();
    let found = discover(broker, &scope).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, c_endpoint.id());
    let accepted = tokio::spawn({
        let c_endpoint = c_endpoint.clone();
        async move {
            c_endpoint
                .accept()
                .await
                .unwrap()
                .accept_control()
                .unwrap()
                .await
                .unwrap()
        }
    });
    let a_conn = a
        .connect_control(target(found[0]), MEMBER_BOOTSTRAP_ALPN)
        .await
        .unwrap();
    let c_conn = accepted.await.unwrap();
    assert_eq!(a_conn.remote_id(), c_endpoint.id());
    assert_eq!(c_conn.remote_id(), a.id());
    let (a_proof, c_proof) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(
            prove_client(&a_conn, &scope, &secret),
            prove_server(&c_conn, &scope, &secret),
        )
    })
    .await
    .unwrap();
    let a_proof = a_proof.unwrap();
    let c_proof = c_proof.unwrap();
    let a_history = a_conn.control_handle();
    let c_history = c_conn.control_handle();
    let (received, served) = tokio::time::timeout(Duration::from_secs(15), async {
        tokio::join!(
            fetch_control_history(
                &a_history,
                &a_proof,
                &scope,
                &mut a_store,
                found[0].udp_addr.into()
            ),
            serve_control_history(&c_history, &c_proof, &scope, &c_store),
        )
    })
    .await
    .unwrap();
    assert_eq!(received.unwrap(), 1);
    served.unwrap();
    assert!(
        a_store
            .nodes()
            .iter()
            .any(|(id, _)| *id == c_key.verifying_key())
    );
    assert_eq!(
        a_store.get_record(Hash::digest(c.header())).await.unwrap(),
        Some(c)
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    let observed = captured.lock().unwrap();
    let mut hash = Sha256::new();
    hash.update(b"miopunch/mqtt-discovery/1\0");
    hash.update(scope.network_id());
    hash.update(scope.secret_generation().to_be_bytes());
    let prefix = format!("miopunch/bootstrap/1/{}", hex::encode(hash.finalize()));
    let query_topic = format!("{prefix}/query");
    let queries: Vec<_> = observed
        .iter()
        .filter(|(topic, _)| *topic == query_topic)
        .collect();
    assert_eq!(queries.len(), 1);
    let query = &queries[0].1;
    assert_eq!(query.len(), 20);
    assert_eq!(&query[..4], b"MDQ1");
    let reply_topic = format!("{prefix}/reply/{}", hex::encode(&query[4..20]));
    let replies: Vec<_> = observed
        .iter()
        .filter(|(topic, _)| *topic == reply_topic)
        .collect();
    assert_eq!(replies.len(), 1);
    let response = &replies[0].1;
    assert_eq!(response.len(), 74);
    assert_eq!(&response[..4], b"MDR1");
    assert_eq!(&response[4..20], &query[4..20]);
    assert_eq!(&response[20..52], c_endpoint.id().as_bytes());
    assert_eq!(&response[52..68], &found[0].route);
    assert_eq!(&response[68..72], &found[0].udp_addr.ip().octets());
    assert_eq!(&response[72..74], &found[0].udp_addr.port().to_be_bytes());
    let a_route = a_carrier.route().unwrap();
    let c_route = c_carrier.route().unwrap();
    let mut packet_count = 0;
    for (topic, bytes) in observed.iter() {
        if topic == &query_topic || topic == &reply_topic {
            continue;
        }
        assert!(
            topic == &format!("miopunch/test/{a_route}/packet")
                || topic == &format!("miopunch/test/{c_route}/packet"),
            "unexpected MQTT topic: {topic}"
        );
        assert!(bytes.len() >= 21 && bytes.len() <= 2068 && bytes.starts_with(b"MIO1"));
        assert!(
            bytes[4..20] == hex::decode(&a_route).unwrap()
                || bytes[4..20] == hex::decode(&c_route).unwrap()
        );
        packet_count += 1;
    }
    assert!(packet_count > 0);
    assert!(observed.iter().all(|(topic, bytes)| {
        !topic.contains(&hex::encode(secret.saved_bytes()))
            && !bytes
                .windows(32)
                .any(|window| window == secret.saved_bytes())
    }));
    println!(
        "MQTT_CAPTURE publishes={} query=1 reply=1 quic_packets={} public_fields_match=1 secret_exposed=0",
        observed.len(),
        packet_count
    );
    drop(observed);
    println!(
        "UNKNOWN_MEMBER_BOOTSTRAP a_known_offline_b=1 discovered_c=1 tls=1 scram=1 signed_history=1 new_records=1"
    );
    responder.abort();
    capture_task.abort();
    drop((a_proof, c_proof));
    a.close().await;
    c_endpoint.close().await;
}

#[tokio::test]
async fn false_candidates_and_former_secret_holders_do_not_receive_history() {
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let broker: SocketAddr = broker.parse().unwrap();
    let root = SigningKey::generate();
    let a_key = SigningKey::generate();
    let b_key = SigningKey::generate();
    let c_key = SigningKey::generate();
    let fake_key = SigningKey::generate();
    let create = create(&root, &a_key);
    let anchor = NetworkAnchor {
        root: root.verifying_key(),
        genesis: Hash::digest(create.header()),
    };
    let add_b = add(&a_key, anchor.root, &b_key, anchor.genesis, 2);
    let add_c = add(&a_key, anchor.root, &c_key, Hash::digest(add_b.header()), 4);
    let remove_b = remove(&a_key, anchor.root, &b_key, Hash::digest(add_c.header()), 3);
    let directory = tempfile::tempdir().unwrap();
    let mut a_store = ManagementStore::open(directory.path().join("state.sqlite"), anchor)
        .await
        .unwrap();
    seed(&mut a_store, &[create, add_b, add_c, remove_b]).await;
    assert!(a_store.known_removed(b_key.verifying_key()));
    let scope = AdmissionScope::new(*anchor.root.as_bytes(), 7);
    let secret = AdmissionSecret::generate();
    a_store
        .set_current_admission(&scope, &secret)
        .await
        .unwrap();
    let wrong_secret = AdmissionSecret::generate();
    let (a, a_carrier) = endpoint(broker, &a_key).await;
    let (fake, fake_carrier) = endpoint(broker, &fake_key).await;
    let (captured, capture_task) = observer(broker).await;

    // The false candidate may publish a plausible ID and route. TLS confirms
    // only that it owns that ID; without the secret, SCRAM rejects it.
    let fake_responder = respond(broker, scope.clone(), candidate(&fake, &fake_carrier))
        .await
        .unwrap();
    let found = discover(broker, &scope).await.unwrap();
    assert_eq!(found.len(), 1);
    let accepted = tokio::spawn({
        let fake = fake.clone();
        async move {
            fake.accept()
                .await
                .unwrap()
                .accept_control()
                .unwrap()
                .await
                .unwrap()
        }
    });
    let a_conn = a
        .connect_control(target(found[0]), MEMBER_BOOTSTRAP_ALPN)
        .await
        .unwrap();
    let fake_conn = accepted.await.unwrap();
    let (a_result, fake_result) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(
            prove_client(&a_conn, &scope, &secret),
            prove_server(&fake_conn, &scope, &wrong_secret),
        )
    })
    .await
    .unwrap();
    assert!(a_result.is_err() && fake_result.is_err());
    fake_responder.abort();

    // A former holder can prove the old shared secret from a new identity,
    // but A's signed member view denies its request for management history.
    let a_responder = respond(broker, scope.clone(), candidate(&a, &a_carrier))
        .await
        .unwrap();
    let found = discover(broker, &scope).await.unwrap();
    assert_eq!(found.len(), 1);
    // With a legitimate responder still online, other network/credential
    // generations do not receive its candidate reply.
    assert!(
        discover(broker, &AdmissionScope::new(*anchor.root.as_bytes(), 8))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        discover(broker, &AdmissionScope::new([99; 32], 7))
            .await
            .unwrap()
            .is_empty()
    );
    let accepted = tokio::spawn({
        let a = a.clone();
        async move {
            a.accept()
                .await
                .unwrap()
                .accept_control()
                .unwrap()
                .await
                .unwrap()
        }
    });
    let fake_conn = fake
        .connect_control(target(found[0]), MEMBER_BOOTSTRAP_ALPN)
        .await
        .unwrap();
    let a_conn = accepted.await.unwrap();
    let (fake_proof, a_proof) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(
            prove_client(&fake_conn, &scope, &secret),
            prove_server(&a_conn, &scope, &secret),
        )
    })
    .await
    .unwrap();
    let fake_proof = fake_proof.unwrap();
    let a_proof = a_proof.unwrap();
    let rejected =
        serve_control_history(&a_conn.control_handle(), &a_proof, &scope, &a_store).await;
    assert!(rejected.is_err());
    drop((fake_proof, a_proof));

    // A known removed B has the same restriction even with its original key.
    let (b, b_carrier) = endpoint(broker, &b_key).await;
    let accepted = tokio::spawn({
        let a = a.clone();
        async move {
            a.accept()
                .await
                .unwrap()
                .accept_control()
                .unwrap()
                .await
                .unwrap()
        }
    });
    let b_conn = b
        .connect_control(target(found[0]), MEMBER_BOOTSTRAP_ALPN)
        .await
        .unwrap();
    let a_conn = accepted.await.unwrap();
    let (b_proof, a_proof) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(
            prove_client(&b_conn, &scope, &secret),
            prove_server(&a_conn, &scope, &secret),
        )
    })
    .await
    .unwrap();
    let b_proof = b_proof.unwrap();
    let a_proof = a_proof.unwrap();
    assert!(
        serve_control_history(&a_conn.control_handle(), &a_proof, &scope, &a_store)
            .await
            .is_err()
    );
    drop((b_proof, a_proof));

    // A also refuses to request history from a known removed B, even though
    // B can still complete the shared-secret proof before rotation.
    let accepted = tokio::spawn({
        let b = b.clone();
        async move {
            b.accept()
                .await
                .unwrap()
                .accept_control()
                .unwrap()
                .await
                .unwrap()
        }
    });
    let a_conn = a
        .connect_control(target(candidate(&b, &b_carrier)), MEMBER_BOOTSTRAP_ALPN)
        .await
        .unwrap();
    let b_conn = accepted.await.unwrap();
    let (a_proof, b_proof) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(
            prove_client(&a_conn, &scope, &secret),
            prove_server(&b_conn, &scope, &secret),
        )
    })
    .await
    .unwrap();
    let a_proof = a_proof.unwrap();
    let b_proof = b_proof.unwrap();
    assert!(
        fetch_control_history(
            &a_conn.control_handle(),
            &a_proof,
            &scope,
            &mut a_store,
            candidate(&b, &b_carrier).udp_addr.into()
        )
        .await
        .is_err()
    );
    drop((a_proof, b_proof));

    // Even a still-authorized C cannot fetch history with a previously saved
    // generation or with the wrong secret tagged as the new generation. The
    // current credential tag survives reopening the actual SQLite store.
    let new_scope = AdmissionScope::new(*anchor.root.as_bytes(), 8);
    let new_secret = AdmissionSecret::generate();
    a_store
        .set_current_admission(&new_scope, &new_secret)
        .await
        .unwrap();
    a_store.close().await;
    let mut a_store = ManagementStore::open(directory.path().join("state.sqlite"), anchor)
        .await
        .unwrap();
    assert!(
        a_store
            .set_current_admission(&scope, &secret)
            .await
            .is_err()
    );
    let (c, _) = endpoint(broker, &c_key).await;
    for (attempt_scope, attempt_secret) in [(&scope, &secret), (&new_scope, &secret)] {
        let accepted = tokio::spawn({
            let a = a.clone();
            async move {
                a.accept()
                    .await
                    .unwrap()
                    .accept_control()
                    .unwrap()
                    .await
                    .unwrap()
            }
        });
        let c_conn = c
            .connect_control(target(found[0]), MEMBER_BOOTSTRAP_ALPN)
            .await
            .unwrap();
        let a_conn = accepted.await.unwrap();
        let (c_proof, a_proof) = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(
                prove_client(&c_conn, attempt_scope, attempt_secret),
                prove_server(&a_conn, attempt_scope, attempt_secret),
            )
        })
        .await
        .unwrap();
        let c_proof = c_proof.unwrap();
        let a_proof = a_proof.unwrap();
        assert!(
            serve_control_history(&a_conn.control_handle(), &a_proof, attempt_scope, &a_store)
                .await
                .is_err()
        );
        drop((c_proof, a_proof));
    }
    c.close().await;

    // Replay the previous response onto the next reply topic. Its embedded
    // nonce is stale, so an otherwise plausible ID and route are discarded.
    let (old_topic, old_payload) = captured
        .lock()
        .unwrap()
        .iter()
        .find(|(_, bytes)| {
            bytes.len() == 74 && bytes.starts_with(b"MDR1") && &bytes[20..52] == a.id().as_bytes()
        })
        .cloned()
        .unwrap();
    let prefix = old_topic.split_once("/reply/").unwrap().0.to_owned();
    a_responder.abort();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut options = MqttOptions::new(
        format!("mio26-replay-{}", rand::random::<u64>()),
        broker.ip().to_string(),
        broker.port(),
    );
    options.set_keep_alive(Duration::from_secs(5));
    let (mqtt, mut events) = AsyncClient::new(options, 16);
    mqtt.subscribe(format!("{prefix}/query"), QoS::AtMostOnce)
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
    let replay = async {
        let publish = loop {
            if let Event::Incoming(Packet::Publish(publish)) = events.poll().await.unwrap() {
                break publish;
            }
        };
        mqtt.publish(
            format!("{prefix}/reply/{}", hex::encode(&publish.payload[4..20])),
            QoS::AtMostOnce,
            false,
            old_payload,
        )
        .await
        .unwrap();
        loop {
            if matches!(
                events.poll().await.unwrap(),
                Event::Outgoing(Outgoing::Publish(_))
            ) {
                break;
            }
        }
    };
    let (replayed, ()) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(discover(broker, &scope), replay)
    })
    .await
    .unwrap();
    assert!(replayed.unwrap().is_empty());
    assert!(
        discover(broker, &AdmissionScope::new(*anchor.root.as_bytes(), 8))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        discover(broker, &AdmissionScope::new([99; 32], 7))
            .await
            .unwrap()
            .is_empty()
    );
    let observed = captured.lock().unwrap();
    assert!(observed.iter().any(|(_, bytes)| bytes.len() == 74
        && bytes.starts_with(b"MDR1")
        && &bytes[20..52] == fake.id().as_bytes()));
    assert!(observed.iter().all(|(topic, bytes)| {
        [
            secret.saved_bytes(),
            wrong_secret.saved_bytes(),
            new_secret.saved_bytes(),
        ]
        .iter()
        .all(|material| {
            !topic.contains(&hex::encode(material))
                && !bytes.windows(32).any(|window| window == *material)
        })
    }));
    drop(observed);
    println!(
        "UNKNOWN_MEMBER_REJECTION no_secret=1 old_secret_new_identity=1 known_removed=1 expired_reply=1 wrong_generation=1 wrong_network=1"
    );
    b.close().await;
    capture_task.abort();
    fake.close().await;
    a.close().await;
}
