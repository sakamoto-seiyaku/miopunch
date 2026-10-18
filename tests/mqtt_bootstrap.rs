//! Formal native connection entry in the real two-gateway MQTT lab.
use std::{collections::BTreeSet, net::Ipv4Addr, path::Path, time::Duration};

use iroh::{Endpoint, EndpointAddr, TransportAddr, endpoint::presets};
use miopunch_native_probe::{
    admission_gate::{ADMISSION_REQUEST_ALPN, ProtocolScope},
    manual_admission::ManualExchange,
    mqtt_transport::MqttTransport,
    native_network::NativeNetwork,
    offline_replica::REPLICA_ALPN,
    resources::{Resource, ResourceUse},
};

const BUSINESS: &[u8] = miopunch_native_probe::connection_coordinator::MEMBER_ALPN;

async fn saved_pair(broker: &str) -> (tempfile::TempDir, NativeNetwork, NativeNetwork) {
    let directory = tempfile::tempdir().unwrap();
    prepare(directory.path()).await;
    let admin = NativeNetwork::open(directory.path().join("admin"))
        .await
        .unwrap();
    let joining = NativeNetwork::open(directory.path().join("joining"))
        .await
        .unwrap();
    for node in [&admin, &joining] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(vec![Resource {
            url: format!("mqtt://{broker}/"),
            uses: BTreeSet::from([ResourceUse::Signaling]),
            limitation: "owned real loopback broker; signaling only".into(),
        }]);
        node.save_resource_configuration(&config).unwrap();
        node.admission_gate()
            .register_protocol(BUSINESS, ProtocolScope::Member);
    }
    (directory, admin, joining)
}

async fn bind_member(node: &NativeNetwork) -> miopunch_native_probe::resources::ResourceEndpoint {
    let builder = Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .transport_config(
            iroh::endpoint::QuicTransportConfig::builder()
                .keep_alive_interval(Duration::from_secs(1))
                .max_idle_timeout(Some(Duration::from_secs(4).try_into().unwrap()))
                .congestion_controller_factory(std::sync::Arc::new(
                    noq_proto::congestion::Bbr3Config::default(),
                ))
                .build(),
        );
    let resources = node.bind_configured_endpoint_with(builder).await.unwrap();
    resources.endpoint().set_alpns(vec![BUSINESS.to_vec()]);
    resources
}

fn mqtt_target(endpoint: &iroh::endpoint::RestrictedEndpoint) -> EndpointAddr {
    EndpointAddr::new(endpoint.id()).with_addrs([TransportAddr::Custom(
        endpoint
            .custom_addrs()
            .into_iter()
            .find(|addr| addr.id() == miopunch_native_probe::mqtt_transport::TRANSPORT_ID)
            .expect("subscribed MQTT route"),
    )])
}

async fn send_business(connection: &iroh::endpoint::Connection, bytes: &[u8]) {
    let (mut send, mut recv) = connection.open_bi().await.unwrap();
    send.write_all(bytes).await.unwrap();
    send.finish().unwrap();
    assert_eq!(recv.read_to_end(64).await.unwrap(), b"authorized ACK");
}

async fn receive_business(connection: &iroh::endpoint::Connection, bytes: &[u8]) {
    let (mut send, mut recv) = connection.accept_bi().await.unwrap();
    assert_eq!(recv.read_to_end(256).await.unwrap(), bytes);
    send.write_all(b"authorized ACK").await.unwrap();
    send.finish().unwrap();
}

async fn mqtt_observer(directory: &Path, duplicate: bool) {
    use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
    let mut options = MqttOptions::new(
        format!("mio42-observer-{}", rand::random::<u64>()),
        "100.64.27.1",
        1883,
    );
    options.set_keep_alive(Duration::from_secs(5));
    options.set_max_packet_size(4096, 4096);
    let (client, mut events) = AsyncClient::new(options, 64);
    client
        .subscribe("miopunch/test/+/packet", QoS::AtMostOnce)
        .await
        .unwrap();
    let mut seen = BTreeSet::new();
    let mut replayed = 0;
    let mut packets = 0;
    loop {
        match events.poll().await.unwrap() {
            Event::Incoming(Packet::SubAck(_)) => {
                std::fs::write(directory.join("observer-count"), b"0").unwrap();
                std::fs::write(directory.join("observer-ready"), b"").unwrap();
                println!("OBSERVER_READY wildcard=true independent_subscriber=true");
            }
            Event::Incoming(Packet::Publish(publish)) => {
                assert!(publish.payload.len() <= 2068);
                packets += 1;
                std::fs::write(directory.join("observer-count.tmp"), packets.to_string()).unwrap();
                std::fs::rename(
                    directory.join("observer-count.tmp"),
                    directory.join("observer-count"),
                )
                .unwrap();
                println!(
                    "MQTT_CAPTURE packets={packets} datagram_bytes={}",
                    publish.payload.len()
                );
                let hash = p2panda_core::Hash::digest(&publish.payload);
                if duplicate && !publish.retain && seen.len() < 1024 && seen.insert(hash) {
                    client
                        .try_publish(
                            &publish.topic,
                            QoS::AtMostOnce,
                            false,
                            publish.payload.clone(),
                        )
                        .unwrap();
                    replayed += 1;
                    println!(
                        "MQTT_DUPLICATE replayed={replayed} datagram_bytes={} hash={hash}",
                        publish.payload.len()
                    );
                }
            }
            _ => {}
        }
    }
}

async fn observer_quiet_count(directory: &Path) -> u64 {
    let mut previous = 0;
    let mut quiet = 0;
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let packets = std::fs::read_to_string(directory.join("observer-count"))
                .unwrap()
                .parse::<u64>()
                .unwrap();
            assert!(
                packets > 0,
                "actual observer must have received bootstrap datagrams"
            );
            quiet = if packets == previous { quiet + 1 } else { 0 };
            previous = packets;
            if quiet == 3 {
                return packets;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("committed data stage must stop MQTT datagrams")
}

async fn wait_file(directory: &Path, name: &str) {
    tokio::time::timeout(Duration::from_secs(8), async {
        while !directory.join(name).exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("owned fixture must deliver the requested network event");
}

async fn prepare(directory: &Path) {
    let mut admin = NativeNetwork::create(directory.join("admin"))
        .await
        .unwrap();
    let mut joining = NativeNetwork::join(
        directory.join("joining"),
        admin.manual_invitation().unwrap(),
    )
    .await
    .unwrap();
    for node in [&admin, &joining] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(vec![]);
        node.save_resource_configuration(&config).unwrap();
    }
    let a = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let b = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (outgoing, incoming) =
        tokio::join!(b.connect_control(a.addr(), ADMISSION_REQUEST_ALPN), async {
            a.accept().await.unwrap().accept_control().unwrap().await
        },);
    let outgoing = std::sync::Arc::new(outgoing.unwrap());
    let incoming = std::sync::Arc::new(incoming.unwrap());
    let (request, served) = tokio::join!(
        joining.request_manual_admission(&outgoing),
        admin.serve_manual_admission(&incoming),
    );
    assert!(matches!(request.unwrap(), ManualExchange::AwaitingApproval));
    served.unwrap();
    let request = admin.manual_requests().await.unwrap().pop().unwrap();
    admin.approve_manual_admission(&request).await.unwrap();
    let receive = async {
        let ManualExchange::Materials(materials) =
            joining.request_manual_admission(&outgoing).await.unwrap()
        else {
            panic!("saved approval");
        };
        joining.complete_manual_admission(materials).await.unwrap();
    };
    let (_, served) = tokio::join!(receive, admin.serve_manual_admission(&incoming));
    served.unwrap();
    outgoing.close(0u32.into(), b"prepared");
    incoming.close(0u32.into(), b"prepared");
    b.close().await;
    a.close().await;
    let resources = vec![
        Resource {
            url: "mqtt://100.64.27.1:1883/".into(),
            uses: BTreeSet::from([ResourceUse::Signaling]),
            limitation: "isolated Mosquitto; public routing metadata, control only".into(),
        },
        Resource {
            url: "https://qad.miopunch.test/".into(),
            uses: BTreeSet::from([ResourceUse::Observation]),
            limitation: "isolated TLS QAD on the actual bound UDP socket; no data relay".into(),
        },
    ];
    for node in [&admin, &joining] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(resources.clone());
        node.save_resource_configuration(&config).unwrap();
    }
    std::fs::write(
        directory.join("admin-id"),
        admin.status().node_id.to_string(),
    )
    .unwrap();
    joining.close().await;
    admin.close().await;
    println!("PREPARED actual_manual_admission=true durable_members=true");
}

#[tokio::test]
async fn explicitly_selected_mqtt_contact_starts_with_another_saved_service_down() {
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let node = NativeNetwork::create(directory.path().join("node"))
        .await
        .unwrap();
    let unavailable = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let closed = unavailable.local_addr().unwrap();
    drop(unavailable);
    let mut config = node.resource_configuration().unwrap();
    config.replacement = Some(
        [format!("mqtt://{closed}/"), format!("mqtt://{broker}/")]
            .into_iter()
            .map(|url| Resource {
                url,
                uses: BTreeSet::from([ResourceUse::Signaling]),
                limitation: "owned loopback MQTT fixture; one broker deliberately closed".into(),
            })
            .collect(),
    );
    node.save_resource_configuration(&config).unwrap();
    let contact = std::sync::Arc::new(MqttTransport::new(
        broker.parse().unwrap(),
        std::sync::Arc::new(miopunch_native_probe::mqtt_transport::Traffic::default()),
    ));
    let resources = node
        .bind_configured_endpoint_with_contact(
            Endpoint::builder(presets::Minimal)
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .bind_addr((Ipv4Addr::LOCALHOST, 0))
                .unwrap(),
            contact,
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while resources.endpoint().custom_addrs().len() != 1 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("only the subscribed, available service may publish a route");
    println!(
        "MQTT_OTHER_RESOURCE_USABLE unavailable_saved_services=1 ready_routes=1 explicit_contact=true"
    );
    resources.close().await;
    node.close().await;
}

#[tokio::test]
async fn saved_udp_disable_applies_to_the_next_mqtt_endpoint_without_reopening_the_node() {
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let (_directory, admin, joining) = saved_pair(&broker).await;
    let mut config = joining.resource_configuration().unwrap();
    config.udp_direct_disabled = true;
    joining.save_resource_configuration(&config).unwrap();
    let server = bind_member(&admin).await;
    let client = bind_member(&joining).await;
    let started = std::time::Instant::now();
    let outgoing = client
        .connect(mqtt_target(server.endpoint()), BUSINESS)
        .await;
    assert!(outgoing.is_err());
    assert_eq!(client.endpoint().metrics().socket.num_conns_opened.get(), 0);
    assert_eq!(server.endpoint().metrics().socket.num_conns_opened.get(), 0);
    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "disabled UDP must reject before a traversal round or candidate wait"
    );
    println!(
        "SAVED_UDP_DISABLE rejected_before_traversal=true elapsed_ms={}",
        started.elapsed().as_millis()
    );
    client.close().await;
    server.close().await;
    joining.close().await;
    admin.close().await;
}

#[tokio::test]
async fn simultaneous_formal_mqtt_dials_return_bounded_independent_data_connections() {
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let (_directory, admin, joining) = saved_pair(&broker).await;
    let a = bind_member(&admin).await;
    let b = bind_member(&joining).await;
    let a_endpoint = a.endpoint().clone();
    let b_endpoint = b.endpoint().clone();
    let (a_out, b_out, a_in, b_in) = tokio::time::timeout(Duration::from_secs(15), async {
        tokio::join!(
            a.connect(mqtt_target(&b_endpoint), BUSINESS),
            b.connect(mqtt_target(&a_endpoint), BUSINESS),
            async { a_endpoint.accept().await.unwrap().await },
            async { b_endpoint.accept().await.unwrap().await },
        )
    })
    .await
    .expect("simultaneous attempts must finish within their contact budgets");
    let (a_out, b_out, a_in, b_in) = (a_out.unwrap(), b_out.unwrap(), a_in.unwrap(), b_in.unwrap());
    for (connection, id) in [
        (&a_out, b_endpoint.id()),
        (&a_in, b_endpoint.id()),
        (&b_out, a_endpoint.id()),
        (&b_in, a_endpoint.id()),
    ] {
        assert_eq!(connection.remote_id(), id);
        assert!(
            connection
                .paths()
                .iter()
                .any(|path| matches!(path.remote_addr(), TransportAddr::Ip(_)))
        );
        assert!(
            connection
                .paths()
                .iter()
                .all(|path| !matches!(path.remote_addr(), TransportAddr::Custom(_)))
        );
    }
    assert_ne!(a_out.stable_id(), a_in.stable_id());
    assert_ne!(b_out.stable_id(), b_in.stable_id());
    tokio::join!(
        send_business(&a_out, b"from A"),
        receive_business(&b_in, b"from A"),
        send_business(&b_out, b"from B"),
        receive_business(&a_in, b"from B"),
    );
    assert_eq!(a_endpoint.metrics().socket.num_conns_opened.get(), 2);
    assert_eq!(b_endpoint.metrics().socket.num_conns_opened.get(), 2);
    println!(
        "SIMULTANEOUS_BOUNDED node_a_tls=2 node_b_tls=2 returned=4 distinct_connections=true actual_business=true"
    );
    a_out.close(0u32.into(), b"done");
    b_out.close(0u32.into(), b"done");
    a.close().await;
    b.close().await;
    joining.close().await;
    admin.close().await;
}

#[derive(Debug)]
struct DataStageNotice {
    entered: std::sync::Arc<tokio::sync::Notify>,
    id: tokio::sync::watch::Sender<Option<usize>>,
    pause_once: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl iroh::endpoint::EndpointHooks for DataStageNotice {
    async fn after_handshake(
        &self,
        connection: &iroh::endpoint::Connection,
    ) -> iroh::endpoint::AfterHandshakeOutcome {
        if self
            .pause_once
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            self.id.send_replace(Some(connection.stable_id()));
            self.entered.notify_one();
            // The ordinary hook runs after qualified path preparation. Hold
            // this first attempt here so cancellation has a definite boundary.
            std::future::pending::<()>().await;
        }
        iroh::endpoint::AfterHandshakeOutcome::Accept
    }
}

#[tokio::test]
async fn canceling_the_formal_mqtt_data_stage_closes_the_original_attempt_and_allows_a_new_contact()
{
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let (_directory, admin, joining) = saved_pair(&broker).await;
    let server = bind_member(&admin).await;
    let entered = std::sync::Arc::new(tokio::sync::Notify::new());
    let (id, original) = tokio::sync::watch::channel(None);
    let client = joining
        .bind_configured_endpoint_with(
            Endpoint::builder(presets::Minimal)
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .bind_addr((Ipv4Addr::LOCALHOST, 0))
                .unwrap()
                .hooks(DataStageNotice {
                    entered: entered.clone(),
                    id,
                    pause_once: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
                }),
        )
        .await
        .unwrap();
    client.endpoint().set_alpns(vec![BUSINESS.to_vec()]);
    let server_endpoint = server.endpoint().clone();
    let server_accept = tokio::spawn(async move { server_endpoint.accept().await.unwrap().await });
    let mut pending = Box::pin(client.connect(mqtt_target(server.endpoint()), BUSINESS));
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::select! {
            result = &mut pending => panic!("connection returned before cancellation stage: {result:?}"),
            _ = entered.notified() => {},
        }
    }).await.expect("real authenticated member must enter the fork's data stage");
    let original_id = original.borrow().unwrap();
    drop(pending);
    assert!(
        tokio::time::timeout(Duration::from_secs(3), server_accept)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while client.endpoint().metrics().socket.num_conns_closed.get() != 1
            || server.endpoint().metrics().socket.num_conns_closed.get() != 1
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("cancellation must close both original handshaked connection resources");
    assert_eq!(client.endpoint().metrics().socket.num_conns_opened.get(), 1);
    assert_eq!(server.endpoint().metrics().socket.num_conns_opened.get(), 1);
    assert_eq!(
        client.endpoint().custom_addrs().len(),
        1,
        "node subscription outlives one canceled contact"
    );
    let (outgoing, incoming) = tokio::join!(
        client.connect(mqtt_target(server.endpoint()), BUSINESS),
        async { server.endpoint().accept().await.unwrap().await },
    );
    let outgoing = outgoing.unwrap();
    let incoming = incoming.unwrap();
    assert_ne!(
        outgoing.stable_id(),
        original_id,
        "later contact cannot revive the canceled connection"
    );
    tokio::join!(
        send_business(&outgoing, b"fresh contact"),
        receive_business(&incoming, b"fresh contact")
    );
    println!(
        "CANCELLATION_CLEAN original_id={original_id} new_id={} original_tls_closed=1 subscription_retained=true actual_new_business=true",
        outgoing.stable_id()
    );
    outgoing.close(0u32.into(), b"done");
    client.close().await;
    server.close().await;
    joining.close().await;
    admin.close().await;
}

#[tokio::test]
async fn a_wrong_target_identity_at_the_real_mqtt_route_never_releases_a_member_connection() {
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let (_directory, admin, joining) = saved_pair(&broker).await;
    let server = bind_member(&admin).await;
    let client = bind_member(&joining).await;
    let mut target = mqtt_target(server.endpoint());
    target.id = iroh::SecretKey::generate().public();
    let (outgoing, incoming) = tokio::time::timeout(Duration::from_secs(6), async {
        tokio::join!(client.connect(target, BUSINESS), async {
            server.endpoint().accept().await.unwrap().await
        })
    })
    .await
    .expect("real wrong-identity handshake must reject without a data stage");
    assert!(outgoing.is_err() && incoming.is_err());
    println!("WRONG_TARGET_REJECTED actual_mqtt_route=true data_released=false");
    client.close().await;
    server.close().await;
    joining.close().await;
    admin.close().await;
}

#[tokio::test]
async fn another_invitation_holder_cannot_impersonate_a_member_at_the_formal_mqtt_entry() {
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let (directory, admin, joining) = saved_pair(&broker).await;
    let holder = NativeNetwork::join(
        directory.path().join("other-holder"),
        admin.manual_invitation().unwrap(),
    )
    .await
    .unwrap();
    holder
        .save_resource_configuration(&admin.resource_configuration().unwrap())
        .unwrap();
    holder
        .admission_gate()
        .register_protocol(BUSINESS, ProtocolScope::Member);
    assert_ne!(holder.status().node_id, joining.status().node_id);
    let server = bind_member(&admin).await;
    let attacker = bind_member(&holder).await;
    let (attempt, incoming) = tokio::time::timeout(Duration::from_secs(6), async {
        tokio::join!(
            attacker.connect(mqtt_target(server.endpoint()), BUSINESS),
            async { server.endpoint().accept().await.unwrap().await }
        )
    })
    .await
    .unwrap();
    assert!(attempt.is_err() && incoming.is_err());
    assert!(
        !admin
            .status()
            .members
            .iter()
            .any(|(id, _)| *id == holder.status().node_id)
    );
    let member = bind_member(&joining).await;
    let (outgoing, incoming) = tokio::join!(
        member.connect(mqtt_target(server.endpoint()), BUSINESS),
        async { server.endpoint().accept().await.unwrap().await },
    );
    let outgoing = outgoing.unwrap();
    let incoming = incoming.unwrap();
    assert_eq!(
        incoming.remote_id().as_bytes(),
        joining.status().node_id.as_bytes()
    );
    tokio::join!(
        send_business(&outgoing, b"real independently identified member"),
        receive_business(&incoming, b"real independently identified member")
    );
    println!(
        "INVITATION_HOLDER_REJECTED distinct_identity=true membership_unchanged=true legitimate_member_still_works=true"
    );
    outgoing.close(0u32.into(), b"done");
    member.close().await;
    attacker.close().await;
    server.close().await;
    holder.close().await;
    joining.close().await;
    admin.close().await;
}

#[tokio::test]
async fn each_saved_broker_resource_can_deliver_a_formal_original_data_connection() {
    let (Ok(first), Ok(second)) = (
        std::env::var("MIOPUNCH_MQTT_BROKER"),
        std::env::var("MIOPUNCH_MQTT_SECOND_BROKER"),
    ) else {
        return;
    };
    let (_directory, admin, joining) = saved_pair(&first).await;
    for node in [&admin, &joining] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement.as_mut().unwrap().push(Resource {
            url: format!("mqtt://{second}/"),
            uses: BTreeSet::from([ResourceUse::Signaling]),
            limitation: "second independent owned real broker".into(),
        });
        node.save_resource_configuration(&config).unwrap();
    }
    let mut first_origin = None;
    for (round, broker) in [&first, &second].into_iter().enumerate() {
        // The node owns one standing contact. Exercise each saved origin by
        // disabling the other before fresh endpoints take their policy snapshot.
        for node in [&admin, &joining] {
            let mut config = node.resource_configuration().unwrap();
            for candidate in [&first, &second] {
                let permission = (format!("mqtt://{candidate}/"), ResourceUse::Signaling);
                if candidate == broker {
                    config.disabled.remove(&permission);
                } else {
                    config.disabled.insert(permission);
                }
            }
            assert!(config.allows(&format!("mqtt://{broker}/"), ResourceUse::Signaling));
            node.save_resource_configuration(&config).unwrap();
        }
        let server = bind_member(&admin).await;
        let client = bind_member(&joining).await;
        let addresses = server.endpoint().custom_addrs();
        assert_eq!(addresses.len(), 1);
        let address = addresses
            .into_iter()
            .find(|addr| {
                first_origin
                    .as_ref()
                    .is_none_or(|origin: &Vec<u8>| addr.data()[..16] != origin[..])
            })
            .unwrap();
        first_origin = Some(address.data()[..16].to_vec());
        let (outgoing, incoming) = tokio::join!(
            client.connect(
                EndpointAddr::new(server.endpoint().id())
                    .with_addrs([TransportAddr::Custom(address)]),
                BUSINESS
            ),
            async { server.endpoint().accept().await.unwrap().await },
        );
        let outgoing = outgoing.unwrap();
        let incoming = incoming.unwrap();
        assert!(
            outgoing
                .paths()
                .iter()
                .all(|path| !matches!(path.remote_addr(), TransportAddr::Custom(_)))
        );
        tokio::join!(
            send_business(&outgoing, b"correct broker-qualified delivery"),
            receive_business(&incoming, b"correct broker-qualified delivery")
        );
        assert_eq!(client.endpoint().metrics().socket.num_conns_opened.get(), 1);
        assert_eq!(server.endpoint().metrics().socket.num_conns_opened.get(), 1);
        println!(
            "BROKER_ROUTE_OK round={round} actual_member=true fresh_endpoint=true same_connection_id={}",
            outgoing.stable_id()
        );
        outgoing.close(0u32.into(), b"done");
        client.close().await;
        server.close().await;
    }
    joining.close().await;
    admin.close().await;
}

#[test]
fn mqtt_native_node_worker() {
    let Ok(role) = std::env::var("MIO42_ROLE") else {
        return;
    };
    tokio::runtime::Runtime::new().unwrap().block_on(async move {
        let directory = std::path::PathBuf::from(std::env::var("MIO42_DIRECTORY").unwrap());
        let case = std::env::var("MIO42_CASE").unwrap_or_else(|_| "positive".into());
        if role == "prepare" { prepare(&directory).await; return; }
        if role == "observe" { mqtt_observer(&directory, case == "duplicate").await; return; }
        let server = role == "serve";
        let mut node = NativeNetwork::open(directory.join(if server { "admin" } else { "joining" })).await.unwrap();
        node.admission_gate().register_protocol(REPLICA_ALPN, ProtocolScope::Member);
        let config = node.resource_configuration().unwrap();
        let cert = std::fs::read(directory.join("cert.der")).unwrap();
        let resources = node.bind_configured_endpoint_with(
            Endpoint::builder(presets::Minimal)
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .alpns(vec![REPLICA_ALPN.to_vec()])
                .observation_servers(config.relay_map(ResourceUse::Observation).unwrap())
                .ca_tls_config(iroh_relay::tls::CaTlsConfig::custom_roots(vec![cert.into()]))
                .transport_config(iroh::endpoint::QuicTransportConfig::builder()
                    .keep_alive_interval(Duration::from_secs(1))
                    .max_idle_timeout(Some(Duration::from_secs(4).try_into().unwrap()))
                    .congestion_controller_factory(std::sync::Arc::new(noq_proto::congestion::Bbr3Config::default()))
                    .build())
                .bind_addr((Ipv4Addr::UNSPECIFIED, if server { 43027 } else { 42027 })).unwrap(),
        ).await.unwrap();
        let endpoint = resources.endpoint().clone();
        endpoint.set_alpns(vec![REPLICA_ALPN.to_vec()]);
        let route = endpoint.custom_addrs().into_iter().find_map(|address| {
            let bytes = address.data();
            (MqttTransport::address(&hex::encode(bytes)).ok().as_ref() == Some(&address))
                .then(|| hex::encode(bytes))
        }).expect("formal saved MQTT resource must bind a temporary route");
        std::fs::write(directory.join(if server { "server-route" } else { "client-route" }), &route).unwrap();
        println!("READY id={} route={route} udp={:?} relay=false", endpoint.id(), endpoint.bound_sockets());
        let target = (!server).then(|| {
            let id = std::fs::read_to_string(directory.join("admin-id")).unwrap().parse().unwrap();
            let route = std::fs::read_to_string(directory.join("server-route")).unwrap();
            EndpointAddr::new(id).with_addrs([TransportAddr::Custom(MqttTransport::address(&route).unwrap())])
        });
        let started = std::time::Instant::now();
        let connection = tokio::time::timeout(Duration::from_secs(25), async {
            if server {
                endpoint.accept().await.unwrap().await.map_err(|error| error.to_string())
            } else {
                resources.connect(target.clone().unwrap(), REPLICA_ALPN).await.map_err(|error| error.to_string())
            }
        }).await.expect("formal connection return must be bounded");
        if matches!(case.as_str(), "control_cut" | "blocked") {
            assert!(connection.is_err(), "a lost control subscription cannot release business");
            if case == "control_cut" {
                tokio::time::timeout(Duration::from_secs(2), async {
                    while !endpoint.custom_addrs().is_empty() { tokio::time::sleep(Duration::from_millis(10)).await; }
                }).await.expect("lost temporary MQTT subscription must withdraw its route");
            } else {
                assert_eq!(endpoint.custom_addrs().len(), 1, "control reachability is separate from business availability");
            }
            let opened = endpoint.metrics().socket.num_conns_opened.get();
            if let Some(target) = target {
                for _ in 0..100 {
                    assert!(resources.connect(target.clone(), REPLICA_ALPN).await.is_err());
                }
                assert_eq!(endpoint.metrics().socket.num_conns_opened.get(), opened,
                    "backoff must not create more TLS connections for repeated control attempts");
            }
            tokio::time::timeout(Duration::from_secs(2), async {
                while endpoint.metrics().socket.num_conns_closed.get() != opened {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }).await.expect("failed bootstrap must release its original connection");
            println!("CONTROL_UNAVAILABLE case={case} bounded_ms={} attempts_opened={opened} attempts_closed={} repeated_calls=100 data_released=false route_withdrawn={}",
                started.elapsed().as_millis(), endpoint.metrics().socket.num_conns_closed.get(), case == "control_cut");
            endpoint.close().await;
            node.close().await;
            return;
        }
        let connection = connection.unwrap();
        assert!(node.admission_gate().allows(&connection, ProtocolScope::Member));
        assert!(connection.paths().iter().any(|path| matches!(path.remote_addr(), TransportAddr::Ip(_))));
        assert!(connection.paths().iter().all(|path| !matches!(path.remote_addr(), TransportAddr::Custom(_))));
        println!("DATA_RETURN stable_id={} peer={} paths={:?}", connection.stable_id(), connection.remote_id(), connection.paths());
        if server {
            node.serve_replica(&connection).await.unwrap();
            let (mut send, mut recv) = connection.accept_bi().await.unwrap();
            assert_eq!(recv.read_to_end(256).await.unwrap(), b"controlled member business after automatic MQTT bootstrap");
            send.write_all(b"authorized response over original data connection").await.unwrap();
            send.finish().unwrap();
            if case == "data_cut" {
                let before = observer_quiet_count(&directory).await;
                std::fs::write(directory.join("server-cut-ready"), b"").unwrap();
                wait_file(&directory, "data-cut").await;
                assert!(!matches!(tokio::time::timeout(Duration::from_secs(8), connection.accept_bi()).await, Ok(Ok(_))),
                    "business written after the UDP cut must never arrive over MQTT");
                tokio::time::timeout(Duration::from_secs(8), connection.closed()).await.unwrap();
                let after = std::fs::read_to_string(directory.join("observer-count")).unwrap().parse::<u64>().unwrap();
                assert_eq!(after, before, "no MQTT publication may carry business, retries or fallback after data commitment");
                assert_eq!(endpoint.metrics().socket.num_conns_opened.get(), 1);
                println!("DATA_FAILED stable_id={} mqtt_before={before} mqtt_after={after} close={:?} transparent_redial=false", connection.stable_id(), connection.close_reason());
            } else { connection.closed().await; }
        } else {
            let outcome = node.sync_replica(&connection).await.unwrap();
            assert!(outcome.complete());
            let (mut send, mut recv) = connection.open_bi().await.unwrap();
            send.write_all(b"controlled member business after automatic MQTT bootstrap").await.unwrap();
            send.finish().unwrap();
            assert_eq!(recv.read_to_end(256).await.unwrap(), b"authorized response over original data connection");
            println!("SYNC_OK source={} saved={} complete=true", outcome.source, outcome.saved.len());
            if case == "data_cut" {
                let before = observer_quiet_count(&directory).await;
                std::fs::write(directory.join("client-cut-ready"), b"").unwrap();
                wait_file(&directory, "data-cut").await;
                let stable = connection.stable_id();
                let cut_at = std::time::Instant::now();
                let (mut send, mut recv) = connection.open_bi().await.unwrap();
                send.write_all(&vec![0x42; 64 * 1024]).await.unwrap();
                send.finish().unwrap();
                assert!(tokio::time::timeout(Duration::from_secs(8), recv.read_to_end(256)).await.unwrap().is_err());
                tokio::time::timeout(Duration::from_secs(8), connection.closed()).await.unwrap();
                assert_eq!(connection.stable_id(), stable);
                let after = std::fs::read_to_string(directory.join("observer-count")).unwrap().parse::<u64>().unwrap();
                assert_eq!(after, before);
                assert_eq!(endpoint.metrics().socket.num_conns_opened.get(), 1);
                println!("DATA_FAILED stable_id={stable} mqtt_before={before} mqtt_after={after} cut_to_close_ms={} queued_business_bytes=65536 close={:?} transparent_redial=false", cut_at.elapsed().as_millis(), connection.close_reason());
            } else { connection.close(0u32.into(), b"done"); }
        }
        println!("DATA_OK same_connection=true real_member=true relay=false");
        if case == "duplicate" {
            assert_eq!(endpoint.metrics().socket.num_conns_opened.get(), 1);
            println!("DUPLICATE_BOUNDED tls_connections=1 data_connections=1");
        }
        endpoint.close().await;
        node.close().await;
    });
}
