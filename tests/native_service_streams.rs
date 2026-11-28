#[path = "support/delayed_relay.rs"]
mod delayed_relay;
#[path = "support/native_service_nat.rs"]
mod native_service_nat;
#[path = "support/native_service_parallel.rs"]
mod native_service_parallel;
#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[path = "support/native_service_backup.rs"]
mod native_service_backup;
#[path = "support/native_service_recovery.rs"]
mod native_service_recovery;
#[path = "support/native_service_tcp_nat.rs"]
mod native_service_tcp_nat;
#[path = "support/relay_contact_broker.rs"]
#[allow(dead_code)]
mod relay_contact_broker;

use std::{
    net::{Ipv4Addr, SocketAddr},
    time::Duration,
};

use miopunch_native_probe::{
    admission_gate::ProtocolScope,
    data_stream::DataStream,
    device_operation::{DeviceAction, DeviceOperation},
    file_transfer::{FILE_ALPN, FileOffer, FileReceipt, FileTaskStatus},
    forwarding::Protocol,
    manual_admission::AdmissionStage,
    native_client::NativeClient,
    native_network::NativeNetwork,
    resources::{Resource, ResourceConfig, ResourceTable, ResourceUse},
    service_policy::{self, Service, ServiceRule},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
    time::{Instant, sleep, timeout},
};

const IO: Duration = Duration::from_secs(5);

async fn relay() -> iroh_relay::server::Server {
    let mut config = iroh_relay::server::ServerConfig::default();
    config.relay = Some(iroh_relay::server::RelayConfig::new((
        Ipv4Addr::LOCALHOST,
        0,
    )));
    iroh_relay::server::Server::spawn(config).await.unwrap()
}

fn relay_resource(url: &iroh::RelayUrl) -> Resource {
    Resource {
        url: url.to_string(),
        uses: [ResourceUse::DataRelay].into(),
        limitation: "owned relay for native service streams".into(),
    }
}

struct Pair {
    root: tempfile::TempDir,
    provider: NativeClient,
    consumer: NativeClient,
    target: iroh::EndpointAddr,
}

impl Pair {
    async fn admit(relay_url: iroh::RelayUrl) -> Self {
        let mut resources = ResourceConfig::with_defaults(ResourceTable {
            revision: 1,
            resources: vec![relay_resource(&relay_url)],
        });
        resources.udp_direct_disabled = true;
        resources.tcp_direct_disabled = true;
        let mut pair = Self::admit_with_resources(resources).await;
        pair.target = iroh::EndpointAddr::new(pair.provider.address().id).with_relay_url(relay_url);
        pair
    }

    async fn admit_with_resources(resources: ResourceConfig) -> Self {
        let root = tempfile::tempdir().unwrap();
        let provider = NativeClient::create_with_resources(
            root.path().join("provider"),
            Ipv4Addr::LOCALHOST,
            resources.clone(),
        )
        .await
        .unwrap();
        provider
            .export_recovery_card_to(root.path().join("card"), None)
            .await
            .unwrap();
        provider.acknowledge_recovery_saved().await.unwrap();
        let invitation = provider
            .issue_precode(service_policy::now() + 300, Some(1))
            .await
            .unwrap();
        let consumer = NativeClient::join_with_resources(
            root.path().join("consumer"),
            &invitation,
            Ipv4Addr::LOCALHOST,
            resources,
        )
        .await
        .unwrap();
        assert_eq!(
            consumer.request_admission(&invitation).await.unwrap(),
            AdmissionStage::Complete
        );
        let target = provider.address();
        Self {
            root,
            provider,
            consumer,
            target,
        }
    }

    async fn close(self) {
        self.consumer.close().await.unwrap();
        self.provider.close().await.unwrap();
    }
}

async fn reopen_with_resources(
    path: &std::path::Path,
    update: impl FnOnce(&mut ResourceConfig),
) -> NativeClient {
    let node = NativeNetwork::open(path).await.unwrap();
    let mut resources = node.resource_configuration().unwrap();
    update(&mut resources);
    node.save_resource_configuration(&resources).unwrap();
    node.close().await;
    NativeClient::open(path, Ipv4Addr::LOCALHOST).await.unwrap()
}

async fn member_pong(client: &NativeClient, target: iroh::EndpointAddr) -> serde_json::Value {
    let peer = target.id.to_string();
    // The SDK bounds the cold dial, then starts the five-second PING budget.
    let reply = client.ping(target).await.unwrap();
    println!("first-pong {reply}");
    assert_eq!(reply["reply"], "PONG");
    assert_eq!(reply["transport"]["peer"], peer);
    assert!(reply["timing"]["response_ms"].as_f64().unwrap() <= IO.as_secs_f64() * 1000.0);
    reply
}

async fn first_pong(
    client: &NativeClient,
    target: iroh::EndpointAddr,
    expected_path: iroh::TransportAddr,
) -> u64 {
    timeout(IO, cold_pong_on_path(client, target, expected_path))
        .await
        .expect("member PONG within the fixture deadline")
}

async fn cold_pong_on_path(
    client: &NativeClient,
    target: iroh::EndpointAddr,
    expected_path: iroh::TransportAddr,
) -> u64 {
    let reply = member_pong(client, target).await;
    let transport = serde_json::to_value(expected_path).unwrap();
    assert_eq!(
        reply["transport"]["initial_data_path"], transport,
        "the released connection was qualified on {transport}: {reply}"
    );
    reply["transport"]["connection"].as_u64().unwrap()
}

#[cfg(feature = "nat-lab-test-hooks")]
#[tokio::test]
async fn native_member_congestion_control_applies_on_startup() {
    use iroh::{TransportAddr, endpoint::CongestionControl};
    use noq_proto::congestion::{Bbr3, Cubic, NewReno};

    let server = relay().await;
    let url = server.http_url().unwrap();
    let Pair {
        root,
        provider,
        mut consumer,
        target,
    } = Pair::admit(url.clone()).await;
    let consumer_path = root.path().join("consumer");
    let config_path = consumer_path.join("resources.json");
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
    assert!(config.get("congestion_control").is_none());
    let mut invalid = config;
    invalid["congestion_control"] = serde_json::json!("unknown");
    assert!(serde_json::from_value::<ResourceConfig>(invalid).is_err());

    for control in [
        None,
        Some(CongestionControl::Cubic),
        Some(CongestionControl::NewReno),
    ] {
        if let Some(control) = control {
            consumer.close().await.unwrap();
            consumer = reopen_with_resources(&consumer_path, |resources| {
                resources.congestion_control = Some(control);
            })
            .await;
        }

        let original_id = first_pong(&consumer, target.clone(), TransportAddr::Relay(url.clone()))
            .await;
        let connection = consumer.test_member_connection(target.id).await.unwrap();
        assert_eq!(connection.stable_id() as u64, original_id);
        let paths = connection.paths();
        let path = paths.iter().find(|path| path.is_selected()).unwrap();
        assert!(path.is_relay());
        let controller = connection
            .test_congestion_state(path.id())
            .unwrap()
            .into_any();
        let algorithm = control.unwrap_or_default();
        assert!(
            match algorithm {
                CongestionControl::Bbrv3 => controller.is::<Bbr3>(),
                CongestionControl::Cubic => controller.is::<Cubic>(),
                CongestionControl::NewReno => controller.is::<NewReno>(),
            },
            "relay path must use {algorithm:?}"
        );
    }

    consumer.close().await.unwrap();
    provider.close().await.unwrap();
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn native_ping_reuses_a_verified_member_address_after_reopen() {
    let server = relay().await;
    let backend = (
        Ipv4Addr::LOCALHOST,
        server.http_url().unwrap().port().unwrap(),
    )
        .into();
    let (relay, stop) = delayed_relay::delay_relay(backend, Duration::from_millis(100)).await;
    let Pair {
        root,
        provider,
        consumer,
        target,
    } = Box::pin(Pair::admit_with_resources(ResourceConfig {
        replacement: Some(vec![relay_resource(&relay)]),
        tcp_direct_disabled: true,
        ..ResourceConfig::default()
    }))
    .await;
    let consumer_path = root.path().join("consumer");
    let consumer_id = consumer.address().id;
    let direct = target
        .ip_addrs()
        .find(|address| address.is_ipv4())
        .copied()
        .unwrap();
    Box::pin(consumer.close()).await.unwrap();
    let consumer = Box::pin(reopen_with_resources(&consumer_path, |resources| {
        resources.disabled_uses.insert(ResourceUse::DataRelay);
    }))
    .await;
    Box::pin(cold_pong_on_path(
        &consumer,
        iroh::EndpointAddr::new(target.id).with_ip_addr(direct),
        iroh::TransportAddr::Ip(direct),
    ))
    .await;
    Box::pin(consumer.close()).await.unwrap();

    let verified = read_member_routes(&consumer_path, target.id);
    let verified_direct = verified
        .into_iter()
        .find(|route| route["verified"] == true && route["address"]["Ip"] == direct.to_string())
        .expect("the first PONG saved the actual verified address");
    let consumer = Box::pin(reopen_with_resources(&consumer_path, |resources| {
        resources.disabled_uses.remove(&ResourceUse::DataRelay);
    }))
    .await;
    assert_eq!(consumer.address().id, consumer_id);
    println!("redial-fixture verified-ip-wins");
    let selected = Box::pin(cold_pong_on_path(
        &consumer,
        iroh::EndpointAddr::new(target.id),
        iroh::TransportAddr::Ip(direct),
    ))
    .await;
    assert_eq!(
        Box::pin(first_pong(
            &consumer,
            iroh::EndpointAddr::new(target.id).with_relay_url(relay.clone()),
            iroh::TransportAddr::Ip(direct)
        ))
        .await,
        selected
    );
    Box::pin(consumer.close()).await.unwrap();
    Box::pin(provider.close()).await.unwrap();

    // Keep the old port open without a QUIC endpoint while the same target
    // identity starts on a different port.
    let stale = UdpSocket::bind(direct).await.unwrap();
    let provider_path = root.path().join("provider");
    // Keep each cached-route restart initiated by the consumer under test.
    let provider = Box::pin(reopen_with_resources(&provider_path, |resources| {
        resources.maintenance.standby_member_limit = Some(0);
    }))
    .await;
    assert_eq!(provider.address().id, target.id);
    let consumer = Box::pin(NativeClient::open(&consumer_path, Ipv4Addr::LOCALHOST))
        .await
        .unwrap();
    assert_eq!(consumer.address().id, consumer_id);
    println!("redial-fixture stale-ip");
    let mut packet = [0; 2048];
    let (_, observed) = tokio::join!(
        Box::pin(cold_pong_on_path(
            &consumer,
            iroh::EndpointAddr::new(target.id),
            iroh::TransportAddr::Relay(relay.clone())
        )),
        timeout(IO, stale.recv_from(&mut packet)),
    );
    assert!(
        observed.unwrap().unwrap().0 >= 1200,
        "the stale UDP port saw the QUIC Initial"
    );
    drop(stale);
    Box::pin(consumer.close()).await.unwrap();

    // A reachable device with an unrelated TLS key takes the same cached port.
    let wrong = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
        .relay_mode(iroh::RelayMode::Disabled)
        .clear_ip_transports()
        .bind_addr(direct)
        .unwrap()
        .alpns(vec![
            miopunch_native_probe::native_network::FUSED_MEMBER_ALPN.to_vec(),
        ])
        .bind()
        .await
        .unwrap();
    assert_ne!(wrong.id(), target.id);
    // A previous relay session can learn the live provider's new UDP port.
    // Each failure case starts with the original verified contact.
    write_member_routes(&consumer_path, target.id, vec![verified_direct.clone()]);
    let consumer = Box::pin(NativeClient::open(&consumer_path, Ipv4Addr::LOCALHOST))
        .await
        .unwrap();
    println!("redial-fixture wrong-peer-ip");
    let (reply, observed) = tokio::join!(
        Box::pin(member_pong(&consumer, iroh::EndpointAddr::new(target.id))),
        timeout(IO, async {
            let mut connecting = wrong.accept().await.unwrap().accept().unwrap();
            let alpn = connecting.alpn().await.unwrap();
            (alpn, connecting)
        }),
    );
    let (alpn, wrong_connection) = observed.unwrap();
    assert_eq!(
        alpn,
        miopunch_native_probe::native_network::FUSED_MEMBER_ALPN
    );
    assert_ne!(
        reply["transport"]["initial_data_path"],
        serde_json::to_value(iroh::TransportAddr::Ip(direct)).unwrap(),
        "the cached address now authenticates the wrong peer"
    );
    drop(wrong_connection);
    Box::pin(consumer.close()).await.unwrap();
    wrong.close().await;
    drop(wrong);

    // The cached route authenticates the right TLS identity, then loses its
    // SCRAM stream. The admitted provider is still reachable through relay.
    let key: [u8; 32] = std::fs::read(provider_path.join("node.key"))
        .unwrap()
        .try_into()
        .unwrap();
    let interrupted = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
        .secret_key(iroh::SecretKey::from_bytes(&key))
        .relay_mode(iroh::RelayMode::Disabled)
        .clear_ip_transports()
        .bind_addr(direct)
        .unwrap()
        .alpns(vec![
            miopunch_native_probe::native_network::FUSED_MEMBER_ALPN.to_vec(),
        ])
        .bind()
        .await
        .unwrap();
    assert_eq!(interrupted.id(), target.id);
    write_member_routes(&consumer_path, target.id, vec![verified_direct]);
    let consumer = Box::pin(NativeClient::open(&consumer_path, Ipv4Addr::LOCALHOST))
        .await
        .unwrap();
    println!("redial-fixture reset-scram");
    let (_, observed) = tokio::join!(
        Box::pin(cold_pong_on_path(
            &consumer,
            iroh::EndpointAddr::new(target.id),
            iroh::TransportAddr::Relay(relay.clone())
        )),
        timeout(IO, async {
            let connection = interrupted.accept().await.unwrap().await.unwrap();
            assert_eq!(connection.remote_id(), consumer_id);
            let (mut send, mut recv) = connection.accept_bi().await.unwrap();
            let mut length = [0; 2];
            recv.read_exact(&mut length).await.unwrap();
            let mut message = vec![0; usize::from(u16::from_be_bytes(length))];
            recv.read_exact(&mut message).await.unwrap();
            assert!(message.starts_with(b"p=tls-exporter,,n="));
            send.reset(1u32.into()).unwrap();
            recv.stop(1u32.into()).unwrap();
            println!(
                "scram-reset tls-peer={} stream={}",
                connection.remote_id(),
                send.id()
            );
            connection
        }),
    );
    drop(observed.unwrap());
    Box::pin(consumer.close()).await.unwrap();
    interrupted.close().await;

    let current = provider
        .address()
        .ip_addrs()
        .find(|address| address.is_ipv4())
        .copied()
        .unwrap();
    assert_ne!(current, direct);
    let now = service_policy::now();
    let recent = |address: SocketAddr, observed_at, verified| {
        serde_json::json!({
            "address": {"Ip": address}, "observed_at": observed_at, "verified": verified,
        })
    };
    for (label, contacts, udp, data_relay, expected) in [
        (
            "newer-ip",
            vec![recent(direct, now - 100, true), recent(current, now, true)],
            true,
            true,
            iroh::TransportAddr::Ip(current),
        ),
        (
            "announced-ip",
            vec![recent(current, now, false)],
            true,
            true,
            iroh::TransportAddr::Relay(relay.clone()),
        ),
        (
            "expired-ip",
            vec![recent(current, now - 301, true)],
            true,
            true,
            iroh::TransportAddr::Relay(relay.clone()),
        ),
        (
            "expired-ip-without-relay",
            vec![recent(current, now - 301, true)],
            true,
            false,
            iroh::TransportAddr::Ip(current),
        ),
        (
            "disabled-ip",
            vec![recent(current, now, true)],
            false,
            true,
            iroh::TransportAddr::Relay(relay.clone()),
        ),
    ] {
        write_member_routes(&consumer_path, target.id, contacts);
        let consumer = Box::pin(reopen_with_resources(&consumer_path, |resources| {
            resources.udp_direct_disabled = !udp;
            if data_relay {
                resources.disabled_uses.remove(&ResourceUse::DataRelay);
            } else {
                resources.disabled_uses.insert(ResourceUse::DataRelay);
            }
        }))
        .await;
        println!("redial-fixture {label}");
        Box::pin(cold_pong_on_path(&consumer, iroh::EndpointAddr::new(target.id), expected)).await;
        Box::pin(consumer.close()).await.unwrap();
    }

    // The removed peer is stopped when the administrator applies the record,
    // so the fresh dial reaches the target's refusal rather than a local check.
    let peer = p2panda_core::VerifyingKey::from_bytes(consumer_id.as_bytes()).unwrap();
    let proposal = Box::pin(provider.propose_device_operation(DeviceAction::RemoveMember, peer))
        .await
        .unwrap();
    let operation: DeviceOperation = serde_json::from_value(proposal["operation"].clone()).unwrap();
    assert_eq!(
        Box::pin(provider.execute_device_operation(operation.id))
            .await
            .unwrap()["effective_locally"],
        true
    );
    write_member_routes(&consumer_path, target.id, vec![recent(current, now, true)]);
    let consumer = Box::pin(reopen_with_resources(&consumer_path, |resources| {
        resources.udp_direct_disabled = false;
    }))
    .await;
    assert!(!consumer.status().await.membership_ended);
    println!("redial-fixture target-denied");
    let rejected = timeout(IO, consumer.ping(iroh::EndpointAddr::new(target.id)))
        .await
        .expect("the authenticated target rejected within the original attempt")
        .unwrap_err();
    assert_eq!(
        rejected.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    println!("target-admission-denied {rejected}");
    Box::pin(consumer.close()).await.unwrap();
    Box::pin(provider.close()).await.unwrap();
    stop.cancel();
    server.shutdown().await.unwrap();
}

fn read_member_routes(path: &std::path::Path, peer: iroh::EndpointId) -> Vec<serde_json::Value> {
    let bytes = std::fs::read(path.join("member.contacts.json")).unwrap();
    let peers: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
    let id = serde_json::to_value(peer).unwrap();
    peers.iter().find(|entry| entry["id"] == id).unwrap()["routes"]
        .as_array()
        .unwrap()
        .clone()
}

fn write_member_routes(
    path: &std::path::Path,
    peer: iroh::EndpointId,
    routes: Vec<serde_json::Value>,
) {
    let saved = serde_json::json!([{"id":peer,"routes":routes}]);
    std::fs::write(
        path.join("member.contacts.json"),
        serde_json::to_vec(&saved).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn native_ping_reaches_a_configured_relay_when_the_advertised_relay_is_silent() {
    let server = relay().await;
    let healthy = server.http_url().unwrap();
    let Pair {
        root,
        provider,
        consumer,
        target,
    } = Pair::admit(healthy.clone()).await;
    let backend = (Ipv4Addr::LOCALHOST, healthy.port().unwrap()).into();
    let (silent, stop) = delayed_relay::delay_relay(backend, Duration::from_secs(30)).await;
    consumer.close().await.unwrap();
    let consumer = reopen_with_resources(&root.path().join("consumer"), |resources| {
        resources.replacement = Some(vec![relay_resource(&healthy), relay_resource(&silent)]);
    })
    .await;
    let target = iroh::EndpointAddr::new(target.id).with_relay_url(silent);
    println!("redial-fixture silent-relay");
    let start = Instant::now();
    first_pong(&consumer, target, iroh::TransportAddr::Relay(healthy)).await;
    let elapsed = start.elapsed();
    consumer.close().await.unwrap();
    provider.close().await.unwrap();
    stop.cancel();
    server.shutdown().await.unwrap();
    assert!(
        elapsed >= Duration::from_millis(250),
        "the second relay started after the stagger"
    );
}

#[tokio::test]
#[ignore = "requires an actual host LAN address in MIOPUNCH_TEST_LAN"]
async fn native_ping_keeps_lan_candidates() {
    let lan: Ipv4Addr = std::env::var("MIOPUNCH_TEST_LAN").unwrap().parse().unwrap();
    let Pair {
        root,
        provider,
        consumer,
        ..
    } = Pair::admit_with_resources(ResourceConfig {
        replacement: Some(Vec::new()),
        tcp_direct_disabled: true,
        ..ResourceConfig::default()
    })
    .await;
    let provider_id = provider.address().id;
    let consumer_id = consumer.address().id;
    consumer.close().await.unwrap();
    provider.close().await.unwrap();
    let provider = NativeClient::open(root.path().join("provider"), lan)
        .await
        .unwrap();
    let consumer_path = root.path().join("consumer");
    let consumer = NativeClient::open(&consumer_path, lan).await.unwrap();
    let addresses = provider.address();
    assert_eq!(addresses.id, provider_id);
    assert_eq!(consumer.address().id, consumer_id);
    let ipv4 = addresses
        .ip_addrs()
        .copied()
        .find(|address| address.ip() == std::net::IpAddr::V4(lan))
        .unwrap();
    println!("redial-fixture lan-ipv4");
    first_pong(
        &consumer,
        iroh::EndpointAddr::new(provider_id).with_ip_addr(ipv4),
        iroh::TransportAddr::Ip(ipv4),
    )
    .await;
    consumer.close().await.unwrap();
    provider.close().await.unwrap();
}

#[tokio::test]
async fn public_native_file_and_ping_share_the_admitted_member_connection() {
    let relay = relay().await;
    let pair = Pair::admit(relay.http_url().unwrap()).await;
    let Pair {
        root,
        provider,
        consumer,
        target,
    } = pair;
    let inbox = root.path().join("inbox");
    provider.enable_files(&inbox).await.unwrap();
    consumer.sync_services(target.clone()).await.unwrap();
    let source = root.path().join("member-file.bin");
    let bytes = b"independent member file\0\xff";
    std::fs::write(&source, bytes).unwrap();
    let (replies, file) = tokio::time::timeout(Duration::from_secs(20), async {
        let (first, task) = tokio::join!(
            consumer.ping(target.clone()),
            consumer.send_file(target.clone(), &source)
        );
        let task = task.unwrap();
        let status = loop {
            let found = consumer
                .file_tasks()
                .into_iter()
                .find(|status| status.id == task)
                .unwrap();
            if found.state.terminal() {
                break found;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        let second = consumer.ping(target).await.unwrap();
        ([first.unwrap(), second], status)
    })
    .await
    .unwrap();
    consumer.close().await.unwrap();
    provider.close().await.unwrap();
    relay.shutdown().await.unwrap();
    for reply in &replies {
        assert_eq!(reply["reply"], "PONG");
        assert!(reply["transport"]["connection"].is_u64());
    }
    assert_eq!(
        replies[0]["transport"]["connection"],
        replies[1]["transport"]["connection"]
    );
    let receipt = file.receipt.expect("real file transfer completes");
    assert_eq!(
        std::fs::read(inbox.join(receipt.saved_name)).unwrap(),
        bytes
    );
    assert_eq!(
        file.transport.unwrap()["connection"],
        replies[0]["transport"]["connection"]
    );
}

#[tokio::test]
#[cfg(feature = "nat-lab-test-hooks")]
async fn slow_file_contact_does_not_block_another_peers_healthy_member_session() {
    let relay = relay().await;
    let url = relay.http_url().unwrap();
    let mut resources = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![relay_resource(&url)],
    });
    resources.udp_direct_disabled = true;
    resources.tcp_direct_disabled = true;
    // A's pending dial below must be the new file operation, rather than a
    // background standby contact. B remains active through a real TCP stream.
    resources.maintenance.standby_member_limit = Some(0);
    let Pair {
        root,
        provider,
        consumer,
        ..
    } = Pair::admit_with_resources(resources.clone()).await;
    let healthy = iroh::EndpointAddr::new(provider.address().id).with_relay_url(url.clone());
    let invitation = provider
        .issue_precode(service_policy::now() + 300, Some(1))
        .await
        .unwrap();
    let offline = NativeClient::join_with_resources(
        root.path().join("offline"),
        &invitation,
        Ipv4Addr::LOCALHOST,
        resources,
    )
    .await
    .unwrap();
    assert_eq!(
        offline.request_admission(&invitation).await.unwrap(),
        AdmissionStage::Complete
    );
    let offline_peer = offline.address().id;
    offline
        .enable_files(root.path().join("offline-inbox"))
        .await
        .unwrap();
    offline.sync_services(healthy.clone()).await.unwrap();
    let inbox = root.path().join("healthy-inbox");
    provider.enable_files(&inbox).await.unwrap();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let local_target = listener.local_addr().unwrap();
    provider
        .enable_forwarding(Protocol::Tcp, vec![local_target])
        .await
        .unwrap();
    consumer.sync_services(healthy.clone()).await.unwrap();
    offline.close().await.unwrap();

    let mut held = Box::pin(consumer.forward_exchange(
        healthy.clone(),
        Protocol::Tcp,
        local_target,
        b"hold",
        4,
    ));
    let (mut socket, _) = timeout(Duration::from_secs(20), async {
        tokio::select! {
            accepted = listener.accept() => accepted.unwrap(),
            result = &mut held => panic!("the held target has not replied: {result:?}"),
        }
    })
    .await
    .unwrap();
    let mut marker = [0; 4];
    timeout(IO, async {
        tokio::select! {
            read = socket.read_exact(&mut marker) => { read.unwrap(); },
            result = &mut held => panic!("the held target has not replied: {result:?}"),
        }
    })
    .await
    .unwrap();
    assert_eq!(&marker, b"hold");
    let original = consumer
        .test_member_connection(healthy.id)
        .await
        .unwrap()
        .stable_id() as u64;
    assert!(!consumer.test_member_dial_pending(offline_peer));

    let path = root.path().join("independent.bin");
    let bytes = vec![0x93; 256 * 1024];
    std::fs::write(&path, &bytes).unwrap();
    let blocked = consumer
        .send_file(
            iroh::EndpointAddr::new(offline_peer).with_relay_url(url),
            &path,
        )
        .await
        .unwrap();
    timeout(IO, async {
        while !consumer.test_member_dial_pending(offline_peer) {
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("A's actual file dial has started");
    let task = consumer.send_file(healthy.clone(), &path).await.unwrap();
    let completed = timeout(IO, async {
        loop {
            let status = consumer
                .file_tasks()
                .into_iter()
                .find(|status| status.id == task)
                .unwrap();
            if status.state.terminal() {
                break status;
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("B sends bytes while A's file dial is still pending");
    assert!(
        consumer.test_member_dial_pending(offline_peer),
        "B did not wait for A's setup budget"
    );
    let receipt = completed.receipt.as_ref().expect("B's real saved receipt");
    assert_eq!(completed.bytes, bytes.len() as u64);
    assert_eq!(
        completed.transport.as_ref().unwrap()["connection"],
        original
    );
    assert_eq!(
        std::fs::read(inbox.join(&receipt.saved_name)).unwrap(),
        bytes
    );
    let received = provider
        .file_tasks()
        .into_iter()
        .find(|status| status.id == task)
        .unwrap();
    assert_eq!(received.receipt.as_ref(), Some(receipt));
    assert_eq!(received.bytes, bytes.len() as u64);

    assert!(consumer.cancel_file(&blocked).unwrap());
    timeout(IO, async {
        loop {
            let status = consumer
                .file_tasks()
                .into_iter()
                .find(|status| status.id == blocked)
                .unwrap();
            if status.state.terminal() && !consumer.test_member_dial_pending(offline_peer) {
                assert_eq!(
                    status.state,
                    miopunch_native_probe::file_transfer::FileState::Cancelled
                );
                assert_eq!(status.bytes, 0);
                break;
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("cancelling A releases its shared pending dial");
    socket.write_all(b"done").await.unwrap();
    assert_eq!(timeout(IO, &mut held).await.unwrap().unwrap(), b"done");
    drop(held);
    eprintln!(
        "offline file dial isolated; B saved {} bytes on original member={original}; A cancellation released setup",
        completed.bytes
    );
    consumer.close().await.unwrap();
    provider.close().await.unwrap();
    relay.shutdown().await.unwrap();
}

#[tokio::test]
async fn file_resume_still_uses_saved_offsets_and_file_receipts() {
    let relay = relay().await;
    let pair = Pair::admit(relay.http_url().unwrap()).await;
    let inbox = pair.root.path().join("resume-inbox");
    pair.provider.enable_files(&inbox).await.unwrap();
    pair.consumer
        .sync_services(pair.target.clone())
        .await
        .unwrap();
    let bytes: Vec<u8> = (0..32 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
    let source = pair.root.path().join("resume-source.bin");
    std::fs::write(&source, &bytes).unwrap();
    let id = pair
        .consumer
        .send_file(pair.target.clone(), source)
        .await
        .unwrap();
    let partial = timeout(Duration::from_secs(20), async {
        loop {
            if let Some(status) = pair
                .provider
                .file_tasks()
                .into_iter()
                .find(|status| status.id == id && status.bytes > 0)
            {
                break status;
            }
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    assert!(partial.bytes < partial.size);
    pair.consumer.disconnect_file(&id).unwrap();
    let sent = timeout(Duration::from_secs(60), async {
        loop {
            let status = pair
                .consumer
                .file_tasks()
                .into_iter()
                .find(|status| status.id == id)
                .unwrap();
            if status.state.terminal() {
                break status;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(sent.attempts >= 2 && sent.resumed_bytes > 0, "{sent:?}");
    let received = pair
        .provider
        .file_tasks()
        .into_iter()
        .find(|status| status.id == id)
        .unwrap();
    let receipt = sent
        .receipt
        .as_ref()
        .expect("file service returns its saved receipt");
    assert_eq!(Some(receipt), received.receipt.as_ref());
    assert_eq!(
        std::fs::read(inbox.join(&receipt.saved_name)).unwrap(),
        bytes
    );
    eprintln!(
        "MIO171 file partial={} resumed={} attempts={} matching_saved_receipt=true",
        partial.bytes, sent.resumed_bytes, sent.attempts
    );
    pair.close().await;
    relay.shutdown().await.unwrap();
}

#[tokio::test]
async fn udp_keeps_datagram_boundaries_and_discards_only_an_oversized_datagram() {
    let relay = relay().await;
    let pair = Pair::admit(relay.http_url().unwrap()).await;
    let target = UdpSocket::bind("[::1]:0").await.unwrap();
    let client = UdpSocket::bind("[::1]:0").await.unwrap();
    let destination = target.local_addr().unwrap();
    pair.provider.enable_forwarding(Protocol::Udp, vec![destination]).await.unwrap();
    pair.consumer.sync_services(pair.target.clone()).await.unwrap();
    let listener = pair.consumer.start_forwarding(pair.target.clone(), Protocol::Udp, destination, "[::1]:0".parse().unwrap()).await.unwrap();
    let listen: SocketAddr = serde_json::from_value(listener["listen"].clone()).unwrap();
    timeout(IO, async {
        let oversized = vec![77; 65508];
        // IPv6 legally carries this host datagram, unlike IPv4. Establish the
        // OS premise independently before checking the forwarding wire bound.
        client.send_to(&oversized, destination).await.unwrap();
        let mut buffer = vec![0; 65536];
        let (count, _) = target.recv_from(&mut buffer).await.unwrap();
        assert_eq!(&buffer[..count], oversized);
        client.send_to(&oversized, listen).await.unwrap();
        wait_until(|| pair.consumer.forwarding_status()["listeners"][0]["counters"]["dropped_datagrams"] == 1).await;
        assert!(timeout(Duration::from_millis(100), target.recv_from(&mut buffer)).await.is_err());
        for length in [0, 17, 65000, 65507] {
            let payload: Vec<u8> = (0..length).map(|i| (i % 251) as u8).collect();
            client.send_to(&payload, listen).await.unwrap();
            let (count, source) = target.recv_from(&mut buffer).await.unwrap();
            assert_eq!(&buffer[..count], payload);
            if length == 17 {
                target.send_to(&oversized, source).await.unwrap();
                wait_until(|| flow(&pair.provider, Protocol::Udp)["dropped_datagrams"] == 1).await;
                assert!(timeout(Duration::from_millis(100), client.recv_from(&mut buffer)).await.is_err());
            }
            target.send_to(&payload, source).await.unwrap();
            let (count, _) = client.recv_from(&mut buffer).await.unwrap();
            assert_eq!(&buffer[..count], payload);
        }
    }).await.expect("UDP drops one host datagram and continues with whole datagrams");
    eprintln!("MIO171 IPv6 UDP lengths=0,17,65000,65507 exact; oversized=65508 discarded once in both directions");
    pair.close().await;
    relay.shutdown().await.unwrap();
}

fn flow(client: &NativeClient, protocol: Protocol) -> serde_json::Value {
    let status = client.forwarding_status();
    status["flows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|flow| flow["protocol"] == serde_json::to_value(protocol).unwrap())
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

async fn tcp_round_trip(client: &mut TcpStream, target: &mut TcpStream, bytes: &[u8]) {
    client.write_all(bytes).await.unwrap();
    let mut received = vec![0; bytes.len()];
    timeout(IO, target.read_exact(&mut received))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received, bytes);
    target.write_all(b"reply over TCP").await.unwrap();
    let mut reply = [0; 14];
    timeout(IO, client.read_exact(&mut reply))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&reply, b"reply over TCP");
}

async fn udp_round_trip(socket: &UdpSocket, target: &UdpSocket, destination: SocketAddr) {
    let bytes = b"original UDP datagram";
    socket.send_to(bytes, destination).await.unwrap();
    let mut data = [0; 64];
    let (count, source) = timeout(IO, target.recv_from(&mut data))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&data[..count], bytes);
    target.send_to(b"reply over UDP", source).await.unwrap();
    let (count, _) = timeout(IO, socket.recv_from(&mut data))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&data[..count], b"reply over UDP");
}

async fn wait_until(mut check: impl FnMut() -> bool) {
    timeout(IO, async {
        while !check() {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ordinary_service_close_and_revocation_are_scoped_until_membership_ends() {
    let relay = relay().await;
    let pair = Pair::admit(relay.http_url().unwrap()).await;
    let Pair {
        provider,
        consumer,
        target,
        ..
    } = &pair;
    let tcp_target = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let udp_target = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let tcp_address = tcp_target.local_addr().unwrap();
    let udp_address = udp_target.local_addr().unwrap();
    provider
        .enable_forwarding(Protocol::Tcp, vec![tcp_address])
        .await
        .unwrap();
    provider
        .enable_forwarding(Protocol::Udp, vec![udp_address])
        .await
        .unwrap();
    consumer.sync_services(target.clone()).await.unwrap();

    let (tcp, udp) = tokio::join!(
        consumer.start_forwarding(
            target.clone(),
            Protocol::Tcp,
            tcp_address,
            (Ipv4Addr::LOCALHOST, 0).into()
        ),
        consumer.start_forwarding(
            target.clone(),
            Protocol::Udp,
            udp_address,
            (Ipv4Addr::LOCALHOST, 0).into()
        ),
    );
    let tcp: SocketAddr = serde_json::from_value(tcp.unwrap()["listen"].clone()).unwrap();
    let udp: SocketAddr = serde_json::from_value(udp.unwrap()["listen"].clone()).unwrap();
    let mut client_socket = TcpStream::connect(tcp).await.unwrap();
    let udp_socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let (accepted, ()) = tokio::join!(
        timeout(IO, tcp_target.accept()),
        udp_round_trip(&udp_socket, &udp_target, udp),
    );
    let (mut target_socket, _) = accepted.unwrap().unwrap();
    tcp_round_trip(
        &mut client_socket,
        &mut target_socket,
        b"two services are active",
    )
    .await;
    let client_transport = flow(consumer, Protocol::Tcp)["transport"].clone();
    let server_transport = flow(provider, Protocol::Tcp)["transport"].clone();
    for (endpoint, tcp) in [(consumer, &client_transport), (provider, &server_transport)] {
        let udp = &flow(endpoint, Protocol::Udp)["transport"];
        assert!(tcp["connection"].is_u64());
        assert_eq!(tcp["connection"], udp["connection"]);
        assert_ne!(tcp["stream"], udp["stream"]);
    }

    let id = flow(consumer, Protocol::Udp)["id"]
        .as_str()
        .unwrap()
        .to_owned();
    consumer.cancel_forwarding_flow(&id).await.unwrap();
    wait_until(|| flow(provider, Protocol::Udp).is_null()).await;
    tcp_round_trip(
        &mut client_socket,
        &mut target_socket,
        b"TCP stays after UDP close",
    )
    .await;
    let next_udp = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    udp_round_trip(&next_udp, &udp_target, udp).await;

    let provider_id = provider.status().await.node.node_id;
    provider
        .set_service_rule(
            provider_id,
            Service::UdpForward,
            None,
            ServiceRule {
                members: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    wait_until(|| {
        flow(provider, Protocol::Udp).is_null() && flow(consumer, Protocol::Udp).is_null()
    })
    .await;
    assert!(
        timeout(
            IO,
            consumer.forward_exchange(target.clone(), Protocol::Udp, udp_address, b"denied", 1)
        )
        .await
        .expect("the provider promptly rejects a new unauthorized UDP stream")
        .is_err()
    );
    let mut denied = [0; 64];
    assert!(
        timeout(
            Duration::from_millis(150),
            udp_target.recv_from(&mut denied)
        )
        .await
        .is_err()
    );
    tcp_round_trip(
        &mut client_socket,
        &mut target_socket,
        b"TCP stays after UDP revoke",
    )
    .await;
    for (node, before) in [(consumer, &client_transport), (provider, &server_transport)] {
        let current = &flow(node, Protocol::Tcp)["transport"];
        assert_eq!(current["connection"], before["connection"]);
        assert_eq!(current["stream"], before["stream"]);
    }

    let peer = consumer.status().await.node.node_id;
    let proposal = provider
        .propose_device_operation(DeviceAction::RemoveMember, peer)
        .await
        .unwrap();
    let operation: DeviceOperation = serde_json::from_value(proposal["operation"].clone()).unwrap();
    let executed = provider
        .execute_device_operation(operation.id)
        .await
        .unwrap();
    assert_eq!(executed["effective_locally"], true);
    let mut byte = [0];
    assert_eq!(
        timeout(IO, client_socket.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert_eq!(
        timeout(IO, target_socket.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert!(
        timeout(IO, consumer.ping(target.clone()))
            .await
            .expect("revoked member redial is rejected")
            .is_err()
    );
    eprintln!(
        "MIO145 lifecycle client_connection={} provider_connection={} udp_revoked=true tcp_survived=true member_redial_denied=true",
        client_transport["connection"], server_transport["connection"]
    );
    pair.close().await;
    relay.shutdown().await.unwrap();
}

#[tokio::test]
async fn only_configured_mqtt_contact_qualifies_the_first_native_member_path() {
    let broker = relay_contact_broker::Broker::new().await;
    let (_root, provider, consumer, contact) = restricted_service_pair(
        format!("mqtt://{}/", broker.address),
        miopunch_native_probe::mqtt_transport::TRANSPORT_ID,
        true,
    )
    .await;
    verify_first_forward_reply(&provider, &consumer, contact).await;
    consumer.close().await.unwrap();
    provider.close().await.unwrap();
    broker.close().await;
}

#[tokio::test]
#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[ignore = "requires a disposable namespace for the UDP filter"]
async fn native_tcp_channel_keeps_quic_and_stream_when_only_udp_path_is_cut() {
    let relay = relay().await;
    let mut config = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![relay_resource(&relay.http_url().unwrap())],
    });
    config.maintenance.standby_member_limit = Some(0);
    let pair = Pair::admit_with_resources(config).await;
    let target = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let destination = target.local_addr().unwrap();
    pair.provider
        .enable_forwarding(Protocol::Tcp, vec![destination])
        .await
        .unwrap();
    pair.consumer
        .sync_services(pair.provider.address())
        .await
        .unwrap();
    let mut direct = pair.provider.address();
    direct
        .addrs
        .retain(|route| matches!(route, iroh::TransportAddr::Ip(_)));
    assert!(!direct.addrs.is_empty(), "provider advertises UDP address");
    let listener = pair
        .consumer
        .start_forwarding(
            direct,
            Protocol::Tcp,
            destination,
            (Ipv4Addr::LOCALHOST, 0).into(),
        )
        .await
        .unwrap();
    let address: SocketAddr = serde_json::from_value(listener["listen"].clone()).unwrap();
    let mut sender = TcpStream::connect(address).await.unwrap();
    let (mut receiver, _) = timeout(IO, target.accept()).await.unwrap().unwrap();
    sequence_reply(&mut sender, &mut receiver, 1).await;
    let original = pair
        .consumer
        .test_member_connection(pair.provider.address().id)
        .await
        .unwrap();
    assert!(original.initial_data_path().is_ip());
    pair.consumer
        .ping(
            iroh::EndpointAddr::new(pair.provider.address().id)
                .with_relay_url(relay.http_url().unwrap()),
        )
        .await
        .unwrap();
    wait_until(|| {
        original
            .paths()
            .iter()
            .any(|path| path.is_relay() && path.stats().acknowledgment.is_some())
    })
    .await;
    let udp_frames = original
        .paths()
        .iter()
        .find(|path| path.is_ip())
        .unwrap()
        .stats()
        .frame_tx
        .stream;
    sequence_reply(&mut sender, &mut receiver, 11).await;
    assert!(
        original
            .paths()
            .iter()
            .find(|path| path.is_ip())
            .unwrap()
            .stats()
            .frame_tx
            .stream
            > udp_frames,
        "the pre-fault business uses UDP"
    );
    let before = flow(&pair.consumer, Protocol::Tcp);
    let iroh::TransportAddr::Ip(address) = original.initial_data_path() else {
        unreachable!()
    };
    let fault_filter = PortBlackhole(address.port().to_string(), "udp");
    let fault = Instant::now();
    fault_filter.apply("-I");
    wait_until(|| {
        original.paths().iter().any(|path| {
            path.is_relay()
                && path.is_selected()
                && path
                    .stats()
                    .acknowledgment
                    .is_some_and(|ack| ack.received_at >= fault.into_std())
        })
    })
    .await;
    sequence_reply(&mut sender, &mut receiver, 2).await;
    sequence_reply(&mut receiver, &mut sender, 3).await;
    let after = flow(&pair.consumer, Protocol::Tcp);
    assert_eq!(before["channel"], after["channel"]);
    assert_eq!(
        before["transport"]["connection"],
        after["transport"]["connection"]
    );
    assert_eq!(before["transport"]["stream"], after["transport"]["stream"]);
    assert!(fault_filter.dropped() > 0);
    eprintln!("MIO170 same carrier before={before} after={after}");
    pair.close().await;
    relay.shutdown().await.unwrap();
}

struct NativeStreamCut {
    pair: Pair,
    broker: relay_contact_broker::Broker,
    target: TcpListener,
    listener_id: String,
    sender: TcpStream,
    receiver: TcpStream,
    proxy: tokio_util::sync::CancellationToken,
    before: [serde_json::Value; 2],
    contact: [iroh::EndpointAddr; 2],
}

impl NativeStreamCut {
    #[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
    fn block_direct_tcp(&self) -> [PortBlackhole; 2] {
        use miopunch_native_probe::tcp_transport::{DATA_TRANSPORT_ID, TcpTransport};

        [&self.pair.consumer, &self.pair.provider].map(|node| {
            let port = node
                .address()
                .addrs
                .iter()
                .find_map(|route| match route {
                    iroh::TransportAddr::Custom(address) if address.id() == DATA_TRANSPORT_ID => {
                        Some(TcpTransport::socket_addr(address).unwrap().port())
                    }
                    _ => None,
                })
                .unwrap();
            PortBlackhole::new(port)
        })
    }

    async fn new() -> Self {
        Self::new_open(false).await
    }

    async fn new_open(pause_ready: bool) -> Self {
        Self::new_policy(pause_ready, Default::default()).await
    }

    async fn new_policy(pause_ready: bool, policy: miopunch_native_probe::forwarding::ChannelPolicy) -> Self {
        use miopunch_native_probe::tcp_transport::{DATA_TRANSPORT_ID, TcpTransport};
        let broker = relay_contact_broker::Broker::new().await;
        let relay = relay().await;
        let Pair {
            root,
            provider,
            consumer,
            ..
        } = Pair::admit(relay.http_url().unwrap()).await;
        provider.close().await.unwrap();
        consumer.close().await.unwrap();
        relay.shutdown().await.unwrap();
        let mqtt = Resource {
            url: format!("mqtt://{}/", broker.address),
            uses: [ResourceUse::Signaling].into(),
            limitation: "owned restricted contact for path recovery".into(),
        };
        let configure = |resources: &mut ResourceConfig| {
            resources.defaults.resources = vec![mqtt.clone()];
            resources.business_channel = policy.clone();
            resources.tcp_direct_disabled = false;
            resources.tcp_listen = Some((Ipv4Addr::LOCALHOST, 0).into());
        };
        let provider = reopen_with_resources(&root.path().join("provider"), configure).await;
        let [provider_route]: [String; 1] = broker.datagram_routes().try_into().unwrap();
        let consumer = reopen_with_resources(&root.path().join("consumer"), configure).await;
        let [consumer_route]: [String; 1] = broker
            .datagram_routes()
            .into_iter()
            .filter(|route| route != &provider_route)
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        let mut provider_contact = provider.invitation().await.unwrap().address().clone();
        use miopunch_native_probe::mqtt_datagram::{TRANSPORT_ID, address_for};
        provider_contact.addrs.retain(|route| {
            matches!(route,
            iroh::TransportAddr::Custom(address) if address.id() == TRANSPORT_ID)
        });
        let Some(iroh::TransportAddr::Custom(address)) = provider_contact.addrs.first() else {
            panic!("provider advertises the configured signaling carrier");
        };
        assert_eq!(provider_contact.addrs.len(), 1);
        assert_eq!(hex::encode(&address.data()[16..]), provider_route);
        // The admitted ordinary member cannot issue invitations. Read its
        // actual broker subscription and use the shared configured origin tag.
        let service = address.data()[..16].try_into().unwrap();
        let route = hex::decode(consumer_route).unwrap().try_into().unwrap();
        let consumer_contact = iroh::EndpointAddr::new(consumer.address().id).with_addrs([
            iroh::TransportAddr::Custom(address_for(Some(service), route)),
        ]);
        let contact = [provider_contact, consumer_contact];
        let (mut sender, mut receiver, before, target, listener_id, proxy) = {
            let server_data = provider.address();
            let actual = server_data
                .addrs
                .iter()
                .find_map(|address| match address {
                    iroh::TransportAddr::Custom(address) if address.id() == DATA_TRANSPORT_ID => {
                        Some(TcpTransport::socket_addr(address).unwrap())
                    }
                    _ => None,
                })
                .unwrap();
            let (proxy_address, proxy) = delayed_relay::delay_relay(actual, Duration::ZERO).await;
            let route = iroh::TransportAddr::Custom(TcpTransport::address((
                Ipv4Addr::LOCALHOST,
                proxy_address.port_or_known_default().unwrap(),
            )));
            let target = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let target_address = target.local_addr().unwrap();
            provider
                .enable_forwarding(Protocol::Tcp, vec![target_address])
                .await
                .unwrap();
            let dial = iroh::EndpointAddr::new(server_data.id).with_addrs([route.clone()]);
            #[cfg(feature = "nat-lab-test-hooks")]
            provider.test_pause_forward_ready(pause_ready);
            let listener = consumer
                .start_forwarding(
                    dial,
                    Protocol::Tcp,
                    target_address,
                    (Ipv4Addr::LOCALHOST, 0).into(),
                )
                .await
                .unwrap();
            let address: SocketAddr = serde_json::from_value(listener["listen"].clone()).unwrap();
            let sender = TcpStream::connect(address).await.unwrap();
            let (receiver, _) = timeout(IO, target.accept()).await.unwrap().unwrap();
            let before =
                [&consumer, &provider].map(|node| flow(node, Protocol::Tcp)["transport"].clone());
            assert_eq!(
                before[0]["initial_data_path"],
                serde_json::to_value(route).unwrap()
            );
            assert!(
                before
                    .iter()
                    .all(|side| side["paths"].as_array().unwrap().len() == 1)
            );
            (
                sender,
                receiver,
                before,
                target,
                listener["id"].as_str().unwrap().to_owned(),
                proxy,
            )
        };
        if !pause_ready {
            sequence_reply(&mut sender, &mut receiver, 1).await;
        }
        let target_address = provider.address();
        Self {
            pair: Pair {
                root,
                provider,
                consumer,
                target: target_address,
            },
            broker,
            target,
            listener_id,
            sender,
            receiver,
            proxy,
            before,
            contact,
        }
    }

    #[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
    async fn interrupt_member(&self) {
        self.interrupt_with_inflight(None).await;
    }

    #[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
    async fn interrupt_with_inflight(&self, data: Option<(&[u8], &[u8], bool)>) {
        let original = self
            .pair
            .consumer
            .test_member_connection(self.pair.target.id)
            .await
            .unwrap();
        let provider_original = self
            .pair
            .provider
            .test_member_connection(self.pair.consumer.address().id)
            .await
            .unwrap();
        use miopunch_native_probe::tcp_transport::{DATA_TRANSPORT_ID, TcpTransport};
        let port = self
            .pair
            .consumer
            .address()
            .addrs
            .iter()
            .find_map(|route| match route {
                iroh::TransportAddr::Custom(address) if address.id() == DATA_TRANSPORT_ID => {
                    Some(TcpTransport::socket_addr(address).unwrap().port())
                }
                _ => None,
            })
            .unwrap();
        let provider_port = self
            .pair
            .provider
            .address()
            .addrs
            .iter()
            .find_map(|route| match route {
                iroh::TransportAddr::Custom(address) if address.id() == DATA_TRANSPORT_ID => {
                    Some(TcpTransport::socket_addr(address).unwrap().port())
                }
                _ => None,
            })
            .unwrap();
        let no_return = PortBlackhole::new(port);
        let no_provider = PortBlackhole::new(provider_port);
        self.broker.mute_datagrams();
        self.proxy.cancel();
        if let Some((outbound, inbound, half_close)) = data {
            async fn queue(socket: &TcpStream, mut bytes: &[u8]) {
                while !bytes.is_empty() {
                    socket.writable().await.unwrap();
                    match socket.try_write(bytes) {
                        Ok(count) => bytes = &bytes[count..],
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(error) => panic!("queue original socket: {error}"),
                    }
                }
            }
            timeout(IO, async {
                tokio::join!(
                    queue(&self.sender, outbound),
                    queue(&self.receiver, inbound)
                );
            })
            .await
            .expect("local sockets accept bounded in-flight bytes");
            if half_close {
                socket2::SockRef::from(&self.sender)
                    .shutdown(std::net::Shutdown::Write)
                    .unwrap();
            }
        }
        // A separate probe puts transport bytes in flight while the TCP business
        // itself is quiescent, so QUIC's loss/idle termination is observable.
        let mut probe = original.open_bi().await.unwrap();
        probe.streams().0.write_all(&[0]).await.unwrap();
        let reason = timeout(Duration::from_secs(36), original.closed())
            .await
            .unwrap();
        assert!(matches!(reason, iroh::endpoint::ConnectionError::TimedOut));
        let remote_reason = timeout(Duration::from_secs(36), provider_original.closed())
            .await
            .unwrap();
        assert!(matches!(
            remote_reason,
            iroh::endpoint::ConnectionError::TimedOut
        ));
        let dropped = no_return.dropped() + no_provider.dropped();
        assert!(dropped > 0);
        eprintln!(
            "MIO170 terminal consumer={reason:?} provider={remote_reason:?} dropped={dropped}"
        );
        drop(no_return);
        drop(no_provider);
        self.broker.unmute_datagrams();
    }
    #[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
    async fn hold_interruption(&self) -> [PortBlackhole; 2] {
        let original = self.pair.consumer.test_member_connection(self.pair.target.id).await.unwrap();
        let faults = self.block_direct_tcp();
        self.broker.mute_datagrams();
        self.proxy.cancel();
        let mut probe = original.open_bi().await.unwrap();
        probe.streams().0.write_all(&[0]).await.unwrap();
        assert!(matches!(timeout(Duration::from_secs(36), original.closed()).await.unwrap(), iroh::endpoint::ConnectionError::TimedOut));
        faults
    }

    async fn learn_contact(&self, side: usize) {
        let peers = [&self.pair.consumer, &self.pair.provider];
        let reply = peers[side].ping(self.contact[side].clone()).await.unwrap();
        assert_eq!(
            reply["transport"]["connection"],
            self.before[side]["connection"]
        );
    }

    #[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
    async fn register_repair_contacts(&self, side: usize, first: Option<iroh_base::CustomAddr>) {
        // Teach the original only after the fault is armed so the fixture can
        // observe the first repair dial from its actual start.
        let peers = [&self.pair.consumer, &self.pair.provider];
        let connection = peers[side]
            .test_member_connection(peers[1 - side].address().id)
            .await
            .unwrap();
        assert_eq!(
            connection.stable_id() as u64,
            self.before[side]["connection"].as_u64().unwrap()
        );
        let iroh::TransportAddr::Custom(live) = self.contact[side].addrs.first().unwrap() else {
            unreachable!("fixture uses an MQTT contact");
        };
        connection.learn_repair_contacts(first.into_iter().chain([live.clone()]).collect());
    }

    fn independent_dials(&self, side: usize) -> usize {
        let Some(iroh::TransportAddr::Custom(route)) = self.contact[side].addrs.first() else {
            unreachable!("fixture uses a restricted MQTT address");
        };
        self.broker.initial_dials_to(route)
    }

    fn status(&self) -> [serde_json::Value; 2] {
        [&self.pair.consumer, &self.pair.provider]
            .map(|node| flow(node, Protocol::Tcp)["transport"].clone())
    }

    async fn wait_for_replacement_paths(&self) -> [serde_json::Value; 2] {
        wait_until(|| {
            self.status().iter().zip(&self.before).all(|(now, before)| {
                selected_path(now)
                    .is_some_and(|new| new["id"] != before["paths"][0]["id"])
            })
        })
        .await;
        self.status()
    }

    fn listener_stats(&self) -> serde_json::Value {
        self.pair.consumer.forwarding_status()["listeners"]
            .as_array()
            .unwrap()
            .iter()
            .find(|listener| listener["id"] == self.listener_id)
            .unwrap()["counters"]
            .clone()
    }

    async fn close(self) {
        self.pair.close().await;
        self.broker.close().await;
    }
}

async fn sequence_reply(sender: &mut TcpStream, receiver: &mut TcpStream, sequence: u64) {
    timeout(IO, async {
        let mut bytes = [0; 8];
        sender.write_all(&sequence.to_be_bytes()).await.unwrap();
        receiver.read_exact(&mut bytes).await.unwrap();
        assert_eq!(bytes, sequence.to_be_bytes());
        receiver.write_all(&bytes).await.unwrap();
        sender.read_exact(&mut bytes).await.unwrap();
        assert_eq!(bytes, sequence.to_be_bytes());
    })
    .await
    .expect("a sequence reply within the original five-second business deadline");
}

fn selected_path(transport: &serde_json::Value) -> Option<&serde_json::Value> {
    transport["paths"]
        .as_array()?
        .iter()
        .find(|path| path["selected"] == true)
}

#[tokio::test]
async fn restricted_contact_repairs_the_live_native_forwarding_stream() {
    #[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
    if native_cut_namespace("restricted_contact_repairs_the_live_native_forwarding_stream") {
        return;
    }
    native_forwarding_repair(0).await;
}

#[tokio::test]
#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[ignore = "requires a disposable network namespace to hold the direct TCP backup"]
async fn accepting_peer_alone_can_contact_and_repair_the_original_native_stream() {
    native_forwarding_repair(1).await;
}

#[tokio::test]
#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[ignore = "requires a disposable network namespace to hold the direct TCP backup"]
async fn a_silent_repair_contact_leaves_time_for_the_live_alternative() {
    use miopunch_native_probe::mqtt_datagram::address_for;

    let mut cut = NativeStreamCut::new().await;
    let Some(iroh::TransportAddr::Custom(live)) = cut.contact[0].addrs.first() else {
        unreachable!("fixture uses a configured MQTT address");
    };
    let stale = address_for(Some(live.data()[..16].try_into().unwrap()), [0; 16]);
    assert!(stale < *live);
    assert!(!cut.broker.datagram_routes().contains(&hex::encode([0; 16])));
    let first_dials = cut.broker.initial_dials_to(&stale);
    let backup_dials = cut.independent_dials(0);
    let direct = cut.block_direct_tcp();
    let started = Instant::now();
    cut.proxy.cancel();
    cut.register_repair_contacts(0, Some(stale.clone())).await;
    cut.sender.write_all(&2u64.to_be_bytes()).await.unwrap();
    wait_until(|| cut.broker.initial_dials_to(&stale) == first_dials + 1).await;
    assert_eq!(
        cut.independent_dials(0),
        backup_dials,
        "the first route is given a chance"
    );
    tokio::time::timeout_at(started + IO, async {
        wait_until(|| cut.independent_dials(0) > backup_dials).await;
        drop(direct);
        let mut bytes = [0; 8];
        cut.receiver.read_exact(&mut bytes).await.unwrap();
        assert_eq!(bytes, 2u64.to_be_bytes());
        cut.receiver.write_all(&bytes).await.unwrap();
        cut.sender.read_exact(&mut bytes).await.unwrap();
        assert_eq!(bytes, 2u64.to_be_bytes());
    })
    .await
    .expect("the live alternative repairs the original flow within its business deadline");
    let first_reply = started.elapsed();
    assert_eq!(cut.broker.initial_dials_to(&stale), first_dials + 1);
    assert_eq!(cut.independent_dials(0), backup_dials + 1);
    let repaired = cut.wait_for_replacement_paths().await;
    sequence_reply(&mut cut.sender, &mut cut.receiver, 3).await;
    for ((side, current), repaired) in cut.status().iter().enumerate().zip(&repaired) {
        assert_eq!(current["connection"], cut.before[side]["connection"]);
        assert_eq!(current["stream"], cut.before[side]["stream"]);
        let path = selected_path(current).unwrap();
        assert_ne!(path["id"], cut.before[side]["paths"][0]["id"]);
        assert!(path["last_ack_age_ms"].is_u64());
        assert!(
            path["rx_stream_frames"].as_u64().unwrap()
                > selected_path(repaired).unwrap()["rx_stream_frames"]
                    .as_u64()
                    .unwrap()
        );
    }
    eprintln!("MIO154 silent-first backup_dials=1 original_reply={first_reply:?}");
    cut.close().await;
}

#[tokio::test]
#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[ignore = "requires a disposable network namespace to hold the direct TCP backup"]
async fn crossed_repair_contacts_install_one_path_on_the_original_connection() {
    let mut cut = NativeStreamCut::new().await;
    cut.broker.mute_datagrams();
    let before = [cut.independent_dials(0), cut.independent_dials(1)];
    let direct = cut.block_direct_tcp();
    let started = Instant::now();
    cut.proxy.cancel();
    cut.register_repair_contacts(0, None).await;
    cut.register_repair_contacts(1, None).await;
    cut.sender.write_all(&2u64.to_be_bytes()).await.unwrap();
    cut.receiver.write_all(&3u64.to_be_bytes()).await.unwrap();
    wait_until(|| (0..2).all(|side| cut.independent_dials(side) > before[side])).await;
    drop(direct);
    cut.broker.unmute_datagrams();
    tokio::time::timeout_at(started + IO, async {
        let mut received = [0; 8];
        cut.receiver.read_exact(&mut received).await.unwrap();
        assert_eq!(received, 2u64.to_be_bytes());
        cut.sender.read_exact(&mut received).await.unwrap();
        assert_eq!(received, 3u64.to_be_bytes());
    })
    .await
    .expect("both queued sequences cross the same original stream");
    cut.wait_for_replacement_paths().await;
    sequence_reply(&mut cut.sender, &mut cut.receiver, 4).await;
    for (side, current) in cut.status().iter().enumerate() {
        let original = &cut.before[side];
        assert_eq!(current["connection"], original["connection"]);
        assert_eq!(current["stream"], original["stream"]);
        assert_eq!(cut.independent_dials(side), before[side] + 1);
        let original_id = &original["paths"][0]["id"];
        let candidates: Vec<_> = current["paths"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|path| path["id"] != *original_id)
            .collect();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0]["selected"], true);
        assert!(candidates[0]["last_ack_age_ms"].is_u64());
    }
    eprintln!(
        "MIO154 crossed_contact one_candidate_both_ends seq4_after={:?}",
        started.elapsed()
    );
    cut.close().await;
}

async fn native_forwarding_repair(contact_side: usize) {
    let mut cut = NativeStreamCut::new().await;
    let mqtt_before = cut.broker.datagram_publishes();
    let dials_before = cut.independent_dials(contact_side);
    let reverse_dials_before = cut.independent_dials(1 - contact_side);
    if contact_side == 0 {
        cut.learn_contact(0).await;
    }
    #[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
    let mut direct_fault = Some(cut.block_direct_tcp());
    let started = Instant::now();
    cut.proxy.cancel();
    if contact_side == 0 {
        #[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
        let iroh::TransportAddr::Custom(route) = cut.contact[contact_side].addrs.first().unwrap()
        else {
            unreachable!("fixture uses an MQTT contact");
        };
        #[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
        let release = async {
            let direct = direct_fault.take().expect("controlled TCP fault");
            wait_until(|| cut.broker.initial_dials_to(route) > dials_before).await;
            assert!(
                direct.iter().any(|filter| filter.dropped() > 0),
                "the original and direct TCP paths must actually be blocked"
            );
            drop(direct);
        };
        #[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
        tokio::join!(
            sequence_reply(&mut cut.sender, &mut cut.receiver, 2),
            release
        );
        #[cfg(not(all(target_os = "linux", feature = "nat-lab-test-hooks")))]
        sequence_reply(&mut cut.sender, &mut cut.receiver, 2).await;
    }
    #[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
    if contact_side == 1 {
        let direct = direct_fault.take().expect("controlled TCP fault");
        cut.register_repair_contacts(contact_side, None).await;
        let iroh::TransportAddr::Custom(route) = cut.contact[contact_side].addrs.first().unwrap()
        else {
            unreachable!("fixture uses an MQTT contact");
        };
        tokio::join!(
            sequence_reply(&mut cut.receiver, &mut cut.sender, 2),
            async {
                wait_until(|| cut.broker.initial_dials_to(route) > dials_before).await;
                assert!(direct.iter().any(|filter| filter.dropped() > 0));
                drop(direct);
            },
        );
    }
    let first_reply = started.elapsed();
    assert!(cut.broker.datagram_publishes() > mqtt_before);
    assert_eq!(cut.independent_dials(contact_side), dials_before + 1);
    assert_eq!(cut.independent_dials(1 - contact_side), reverse_dials_before);
    let repaired = timeout(IO, async {
        loop {
            let status = cut.status();
            if status.iter().zip(&cut.before).all(|(now, before)| {
                replacement_on_selected_binding(now, before).is_some()
            }) {
                break status;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("both ends validate a replacement on their selected TCP binding");
    let repaired_at = started.elapsed();
    assert_ne!(
        selected_path(&repaired[0]).unwrap()["remote"],
        cut.before[0]["paths"][0]["remote"],
        "the consumer must leave the failed TCP proxy"
    );
    for sequence in 3..=8 {
        sequence_reply(&mut cut.sender, &mut cut.receiver, sequence).await;
    }
    for ((now, repaired), before) in cut.status().iter().zip(&repaired).zip(&cut.before) {
        assert_eq!(now["peer"], before["peer"]);
        assert_eq!(now["connection"], before["connection"]);
        assert_eq!(now["stream"], before["stream"]);
        let path = selected_path(now).unwrap();
        let selected = selected_path(repaired).unwrap();
        assert!(same_tcp_binding(path, selected));
        let proof = replacement_on_selected_binding(repaired, before).unwrap();
        assert!(u128::from(proof["last_ack_age_ms"].as_u64().unwrap()) < repaired_at.as_millis());
        let previous = repaired["paths"].as_array().unwrap();
        let sent_on_repaired_binding = now["paths"].as_array().unwrap().iter().any(|candidate| {
            let previous_frames = previous
                .iter()
                .find(|path| path["id"] == candidate["id"])
                .map_or(0, |path| path["tx_stream_frames"].as_u64().unwrap());
            same_tcp_binding(candidate, selected)
                && candidate["tx_stream_frames"].as_u64().unwrap() > previous_frames
        });
        assert!(sent_on_repaired_binding, "the original sends on the repaired binding");
        eprintln!(
            "MIO154 contact_side={contact_side} first_original_reply={first_reply:?} repair_selected={} repair_confirmed={} before={before} after={now}",
            selected["id"], proof["id"]
        );
    }
    cut.close().await;
}

fn same_tcp_binding(left: &serde_json::Value, right: &serde_json::Value) -> bool {
    left["local"] == right["local"] && left["remote"] == right["remote"]
}

fn replacement_on_selected_binding<'a>(
    current: &'a serde_json::Value,
    before: &serde_json::Value,
) -> Option<&'a serde_json::Value> {
    let selected = selected_path(current)?;
    // Two QUIC path IDs can share the server's TCP binding. The selector may
    // keep the existing ID while the replacement confirms that exact binding.
    current["paths"].as_array()?.iter().find(|candidate| {
        candidate["id"] != before["paths"][0]["id"]
            && same_tcp_binding(candidate, selected)
            && candidate["last_ack_age_ms"].is_u64()
    })
}

#[tokio::test]
#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[ignore = "requires a disposable namespace for the old TCP return-path filter"]
async fn native_tcp_channel_keeps_both_sockets_after_member_replacement() {
    let mut cut = NativeStreamCut::new().await;
    cut.learn_contact(0).await;
    let channel = flow(&cut.pair.consumer, Protocol::Tcp)["channel"].clone();
    assert!(channel.is_string());
    let listen: SocketAddr = serde_json::from_value(
        cut.pair.consumer.forwarding_status()["listeners"][0]["listen"].clone(),
    )
    .unwrap();
    let mut others = Vec::new();
    for index in 1..16 {
        let mut sender = TcpStream::connect(listen).await.unwrap();
        let (mut receiver, _) = timeout(IO, cut.target.accept()).await.unwrap().unwrap();
        sequence_reply(&mut sender, &mut receiver, 100 + index).await;
        others.push((sender, receiver));
    }
    assert_eq!(cut.pair.provider.forwarding_status()["active_flows"], 16);
    cut.interrupt_member().await;
    sequence_reply(&mut cut.sender, &mut cut.receiver, 2).await;
    sequence_reply(&mut cut.receiver, &mut cut.sender, 3).await;
    for (index, (sender, receiver)) in others.iter_mut().enumerate() {
        sequence_reply(sender, receiver, 200 + index as u64).await;
        sequence_reply(receiver, sender, 300 + index as u64).await;
    }
    let after = cut.status();
    for (before, after) in cut.before.iter().zip(&after) {
        assert_eq!(before["peer"], after["peer"]);
        assert_ne!(before["connection"], after["connection"]);
    }
    assert!(
        timeout(Duration::from_millis(100), cut.target.accept())
            .await
            .is_err(),
        "original target is accepted exactly once"
    );
    assert_eq!(cut.listener_stats()["accepted"], 16);
    assert_eq!(cut.listener_stats()["failed"], 0);
    for node in [&cut.pair.consumer, &cut.pair.provider] {
        assert_eq!(flow(node, Protocol::Tcp)["channel"], channel);
    }
    eprintln!(
        "MIO170 channel={channel} old={:?} replacement={after:?}",
        cut.before
    );
    cut.close().await;
}

#[tokio::test]
#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[ignore = "requires a disposable namespace for the old TCP return-path filter"]
async fn original_tcp_sockets_replay_bidirectional_inflight_bytes_once() {
    let mut cut = NativeStreamCut::new().await;
    cut.learn_contact(0).await;
    let outbound: Vec<u8> = (0..256 * 1024).map(|i| (i % 251) as u8).collect();
    let inbound: Vec<u8> = (0..256 * 1024).map(|i| (250 - i % 251) as u8).collect();
    cut.interrupt_with_inflight(Some((&outbound, &inbound, false)))
        .await;
    timeout(Duration::from_secs(10), async {
        let mut received = vec![0; outbound.len()];
        cut.receiver.read_exact(&mut received).await.unwrap();
        assert_eq!(received, outbound);
        cut.sender.read_exact(&mut received).await.unwrap();
        assert_eq!(received, inbound);
    })
    .await
    .expect("all original bytes arrive once after replacement");
    sequence_reply(&mut cut.sender, &mut cut.receiver, 909).await;
    assert!(
        timeout(Duration::from_millis(100), cut.target.accept())
            .await
            .is_err()
    );
    let after = cut.status();
    for (old, new) in cut.before.iter().zip(&after) {
        assert_ne!(old["connection"], new["connection"]);
    }
    eprintln!("MIO171 duplex 262144 bytes each direction, exact sequence and tail, one target");
    cut.close().await;
}

#[tokio::test]
#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[ignore = "requires a disposable namespace for the old TCP return-path filter"]
async fn lost_channel_ack_and_interrupted_half_close_preserve_exact_tail() {
    let mut cut = NativeStreamCut::new().await;
    cut.learn_contact(0).await;
    let old = cut
        .pair
        .consumer
        .test_member_connection(cut.pair.target.id)
        .await
        .unwrap();
    let old_provider = cut
        .pair
        .provider
        .test_member_connection(cut.pair.consumer.address().id)
        .await
        .unwrap();
    let suppressed = cut.pair.provider.test_pause_channel_progress(true);
    let prefix = vec![71; 4096];
    cut.sender.write_all(&prefix).await.unwrap();
    let mut received = vec![0; prefix.len()];
    timeout(IO, cut.receiver.read_exact(&mut received))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        received, prefix,
        "target independently received bytes whose channel ACK will be lost"
    );
    wait_until(|| cut.pair.provider.test_pause_channel_progress(true) > suppressed).await;
    let tail: Vec<u8> = (0..128 * 1024).map(|i| (i % 239) as u8).collect();
    let response = vec![29; 128 * 1024];
    cut.interrupt_with_inflight(Some((&tail, &response, true)))
        .await;
    cut.pair.provider.test_pause_channel_progress(false);
    received.clear();
    timeout(
        Duration::from_secs(10),
        cut.receiver.read_to_end(&mut received),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        received, tail,
        "accepted prefix is not repeated and FIN follows the complete tail"
    );
    let after = cut.status();
    for (before, after) in cut.before.iter().zip(&after) {
        assert_ne!(before["connection"], after["connection"]);
    }
    // Delayed close/stream operations from the retired carriers cannot reach
    // the new carry, which is the only owner of the channel's IO and progress.
    old.test_close(0, b"late old carrier close");
    old_provider.test_close(0, b"late old carrier close");
    assert!(old.open_bi().await.is_err());
    cut.receiver
        .write_all(b"response after original half-close")
        .await
        .unwrap();
    cut.receiver.shutdown().await.unwrap();
    received.clear();
    timeout(
        Duration::from_secs(10),
        cut.sender.read_to_end(&mut received),
    )
    .await
    .unwrap()
    .unwrap();
    let mut expected = response;
    expected.extend_from_slice(b"response after original half-close");
    assert_eq!(received, expected);
    assert!(
        timeout(Duration::from_millis(100), cut.target.accept())
            .await
            .is_err()
    );
    assert_eq!(cut.listener_stats()["failed"], 0);
    eprintln!(
        "MIO171 ACK lost after 4096 independently received bytes; 131072-byte tails, interrupted FIN, late retired carrier close, exact final response"
    );
    cut.close().await;
}

#[tokio::test]
#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[ignore = "requires a disposable namespace for the old TCP return-path filter"]
async fn ended_tcp_target_is_never_opened_again_after_member_replacement() {
    let mut cut = NativeStreamCut::new().await;
    cut.learn_contact(0).await;
    cut.interrupt_member().await;
    socket2::SockRef::from(&cut.receiver)
        .set_linger(Some(Duration::ZERO))
        .unwrap();
    drop(cut.receiver);
    let mut byte = [0];
    assert_eq!(
        timeout(Duration::from_secs(65), cut.sender.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    cut.sender.shutdown().await.unwrap();
    wait_until(|| cut.pair.consumer.forwarding_status()["listeners"][0]["counters"]["failed"] == 1)
        .await;
    assert_eq!(
        cut.pair.consumer.forwarding_status()["listeners"][0]["counters"]["completed"],
        0
    );
    assert!(
        timeout(Duration::from_millis(200), cut.target.accept())
            .await
            .is_err()
    );
    wait_until(|| cut.pair.provider.forwarding_status()["active_flows"] == 0).await;
    cut.pair.close().await;
    cut.broker.close().await;
}

#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
fn native_cut_namespace(case: &str) -> bool {
    const CHILD: &str = "MIOPUNCH_NATIVE_CUT_NETNS";
    if std::env::var_os(CHILD).is_some() {
        return false;
    }
    let output = std::process::Command::new("unshare")
        .args([
            "-Urn",
            "--fork",
            "sh",
            "-c",
            "ip link set lo up && exec \"$@\"",
            "native-cut-fixture",
        ])
        .arg(std::env::current_exe().unwrap())
        .args([case, "--exact", "--nocapture"])
        .env(CHILD, "1")
        .output()
        .expect("disposable native path fault namespace");
    assert!(
        output.status.success(),
        "isolated {case} failed\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    true
}

#[cfg(target_os = "linux")]
struct PortBlackhole(String, &'static str);

#[cfg(target_os = "linux")]
impl PortBlackhole {
    fn new(port: u16) -> Self {
        let filter = Self(port.to_string(), "tcp");
        filter.apply("-I");
        filter
    }

    fn apply(&self, action: &str) {
        for port in ["--sport", "--dport"] {
            assert!(
                std::process::Command::new("iptables")
                    .args([
                        "-w", action, "OUTPUT", "-o", "lo", "-p", self.1, port, &self.0, "-j",
                        "DROP",
                    ])
                    .status()
                    .unwrap()
                    .success()
            );
        }
    }

    fn dropped(&self) -> u64 {
        let stats = std::process::Command::new("iptables")
            .args(["-w", "-nvx", "-L", "OUTPUT"])
            .output()
            .unwrap();
        assert!(stats.status.success());
        let source = format!("spt:{}", self.0);
        let destination = format!("dpt:{}", self.0);
        String::from_utf8(stats.stdout)
            .unwrap()
            .lines()
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                let packets = fields.next()?.parse::<u64>().ok()?;
                fields
                    .any(|field| field == source || field == destination)
                    .then_some(packets)
            })
            .sum()
    }
}

#[cfg(target_os = "linux")]
impl Drop for PortBlackhole {
    fn drop(&mut self) {
        self.apply("-D");
    }
}

#[tokio::test]
#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[ignore = "requires a disposable network namespace to hold the direct TCP backup"]
async fn cancelling_the_last_native_flow_stops_independent_repair() {
    let mut cut = NativeStreamCut::new().await;
    cut.broker.mute_datagrams();
    let initial_dials = cut.independent_dials(0);
    let _direct = cut.block_direct_tcp();
    cut.proxy.cancel();
    cut.register_repair_contacts(0, None).await;
    cut.sender.write_all(&2u64.to_be_bytes()).await.unwrap();
    wait_until(|| cut.independent_dials(0) > initial_dials).await;
    assert_eq!(cut.independent_dials(0), initial_dials + 1);
    let id = flow(&cut.pair.consumer, Protocol::Tcp)["id"]
        .as_str()
        .unwrap()
        .to_owned();
    cut.pair.consumer.cancel_forwarding_flow(&id).await.unwrap();
    assert!(cut.status()[0].is_null());
    assert!(!cut.status()[1].is_null());
    // MQTT's already queued packet and QUIC's cancellation close can reach
    // the broker after the flow disappears. Keep them behind the fault.
    sleep(Duration::from_millis(150)).await;
    cut.broker.unmute_datagrams();
    let stopped = cut.broker.datagram_publishes();
    let mut bytes = [0; 8];
    assert!(
        timeout(
            Duration::from_millis(1350),
            cut.receiver.read_exact(&mut bytes)
        )
        .await
        .is_err()
    );
    assert_eq!(cut.broker.datagram_publishes(), stopped);
    assert_eq!(cut.independent_dials(0), initial_dials + 1);
    assert_ne!(
        cut.listener_stats()["last_error"],
        "member_connection_terminated"
    );
    cut.close().await;
}

#[tokio::test]
async fn membership_revocation_closes_the_original_even_when_path_repair_has_started() {
    #[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
    if native_cut_namespace(
        "membership_revocation_closes_the_original_even_when_path_repair_has_started",
    ) {
        return;
    }
    let mut cut = NativeStreamCut::new().await;
    cut.learn_contact(0).await;
    cut.broker.mute_datagrams();
    let initial = cut.broker.datagram_publishes();
    #[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
    let direct = cut.block_direct_tcp();
    cut.proxy.cancel();
    cut.sender.write_all(&2u64.to_be_bytes()).await.unwrap();
    wait_until(|| cut.broker.datagram_publishes() > initial).await;
    #[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
    assert!(
        direct.iter().any(|filter| filter.dropped() > 0),
        "revocation must happen during a genuine blocked-path repair"
    );
    let proposal = cut
        .pair
        .provider
        .propose_device_operation(
            DeviceAction::RemoveMember,
            cut.pair.consumer.status().await.node.node_id,
        )
        .await
        .unwrap();
    let operation: DeviceOperation = serde_json::from_value(proposal["operation"].clone()).unwrap();
    assert_eq!(
        cut.pair
            .provider
            .execute_device_operation(operation.id)
            .await
            .unwrap()["effective_locally"],
        true
    );
    cut.broker.unmute_datagrams();
    let mut bytes = [0; 8];
    assert_eq!(
        timeout(IO, cut.receiver.read(&mut bytes))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    wait_until(|| cut.status()[1].is_null()).await;
    assert!(
        timeout(IO, cut.pair.consumer.ping(cut.pair.target.clone()))
            .await
            .unwrap()
            .is_err()
    );
    assert!(cut.status()[1].is_null());
    cut.close().await;
}

#[tokio::test]
async fn silent_relay_race_starts_mqtt_before_the_relays_finish() {
    let broker = relay_contact_broker::Broker::new().await;
    let (root, provider, consumer, mut contact) = restricted_service_pair(
        format!("mqtt://{}/", broker.address),
        miopunch_native_probe::mqtt_transport::TRANSPORT_ID,
        true,
    )
    .await;
    consumer.close().await.unwrap();
    let backend = relay().await;
    let mut silent = Vec::new();
    for _ in 0..4 {
        silent.push(
            delayed_relay::delay_relay(backend.http_addr().unwrap(), Duration::from_secs(10)).await,
        );
    }
    let consumer = reopen_with_resources(&root.path().join("consumer"), |resources| {
        resources.maintenance.standby_member_limit = Some(0);
        resources
            .defaults
            .resources
            .extend(silent.iter().map(|(url, _)| relay_resource(url)));
    })
    .await;
    contact.addrs.extend(
        silent
            .iter()
            .map(|(url, _)| iroh::TransportAddr::Relay(url.clone())),
    );
    let started = Instant::now();
    timeout(
        Duration::from_secs(4),
        verify_first_forward_reply(&provider, &consumer, contact),
    )
    .await
    .expect("four silent relays leave time for the first MQTT business reply");
    println!("first reply after {:?}", started.elapsed());
    consumer.close().await.unwrap();
    provider.close().await.unwrap();
    for (_, stop) in silent {
        stop.cancel();
    }
    backend.shutdown().await.unwrap();
    broker.close().await;
}

#[tokio::test]
async fn verified_cache_and_mqtt_both_start_when_data_relays_are_disabled() {
    let broker = relay_contact_broker::Broker::new().await;
    let (root, provider, consumer, _) = restricted_service_pair(
        format!("mqtt://{}/", broker.address),
        miopunch_native_probe::mqtt_transport::TRANSPORT_ID,
        true,
    )
    .await;
    let consumer_path = root.path().join("consumer");
    let provider_path = root.path().join("provider");
    let peer = provider.address().id;
    let cached = *provider
        .address()
        .ip_addrs()
        .find(|addr| addr.is_ipv4())
        .unwrap();
    let direct = iroh::EndpointAddr::new(peer).with_ip_addr(cached);
    first_pong(&consumer, direct, iroh::TransportAddr::Ip(cached)).await;
    consumer.close().await.unwrap();
    provider.close().await.unwrap();
    assert!(
        read_member_routes(&consumer_path, peer)
            .iter()
            .any(|route| {
                route["verified"] == true && route["address"]["Ip"] == cached.to_string()
            })
    );
    let previous = UdpSocket::bind(cached).await.unwrap();
    let provider = NativeClient::open(provider_path, Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let consumer = NativeClient::open(consumer_path, Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let invitation = provider.invitation().await.unwrap();
    let mut contact = invitation.address().clone();
    contact.addrs.retain(|route| {
        matches!(route, iroh::TransportAddr::Custom(address)
        if address.id() == miopunch_native_probe::mqtt_transport::TRANSPORT_ID)
    });
    assert_eq!(contact.addrs.len(), 1);
    let mqtt_before = broker.datagram_publishes();
    let started = Instant::now();
    let (reply, cached_initial) = timeout(IO, async {
        tokio::join!(consumer.ping(contact), async {
            let mut packet = [0; 2048];
            let (len, caller) = previous.recv_from(&mut packet).await.unwrap();
            (Instant::now(), len, caller)
        })
    })
    .await
    .unwrap();
    let reply = reply.unwrap();
    assert_eq!(reply["reply"], "PONG");
    assert_eq!(reply["transport"]["peer"], peer.to_string());
    assert_ne!(
        reply["transport"]["initial_data_path"]["Ip"],
        cached.to_string()
    );
    assert!(reply["transport"]["initial_data_path"]["Ip"].is_string());
    assert!(cached_initial.1 >= 1200);
    assert!(cached_initial.0 - started < Duration::from_millis(250));
    let mqtt_packets = broker.datagram_publishes() - mqtt_before;
    assert!(mqtt_packets > 0);
    println!(
        "MIO149 zero-data-relays cached_initial={cached_initial:?} mqtt_packets={mqtt_packets} reply_after={:?} {reply}",
        started.elapsed()
    );
    consumer.close().await.unwrap();
    provider.close().await.unwrap();
    broker.close().await;
}

#[tokio::test]
async fn mqtt_member_rejection_ends_the_restricted_first_dial() {
    let broker = relay_contact_broker::Broker::new().await;
    let (_root, provider, consumer, contact) = restricted_service_pair(
        format!("mqtt://{}/", broker.address),
        miopunch_native_probe::mqtt_transport::TRANSPORT_ID,
        true,
    )
    .await;
    let proposal = provider
        .propose_device_operation(
            DeviceAction::RemoveMember,
            consumer.status().await.node.node_id,
        )
        .await
        .unwrap();
    let operation: DeviceOperation = serde_json::from_value(proposal["operation"].clone()).unwrap();
    assert_eq!(
        provider
            .execute_device_operation(operation.id)
            .await
            .unwrap()["effective_locally"],
        true
    );
    assert!(!consumer.status().await.membership_ended);
    verify_no_forward_release(
        &provider,
        &consumer,
        contact,
        std::io::ErrorKind::PermissionDenied,
    )
    .await;
    consumer.close().await.unwrap();
    provider.close().await.unwrap();
    broker.close().await;
}

#[tokio::test]
#[ignore = "requires the owned real IRCd supplied by MIOPUNCH_IRC_SERVER"]
async fn only_configured_irc_contact_qualifies_the_first_native_member_path() {
    let (_root, provider, consumer, contact) = restricted_service_pair(
        owned_irc_url(),
        miopunch_native_probe::irc_transport::TRANSPORT_ID,
        true,
    )
    .await;
    verify_first_forward_reply(&provider, &consumer, contact).await;
    consumer.close().await.unwrap();
    provider.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the owned real IRCd supplied by MIOPUNCH_IRC_SERVER"]
async fn public_contact_selection_preserves_the_confirmed_irc_reply_deadline() {
    use miopunch_native_probe::{irc_transport, mqtt_transport};

    const REPLY_DEADLINE: Duration = Duration::from_secs(35);
    let other_broker = relay_contact_broker::Broker::new().await;
    let irc = owned_irc_url();
    let wrong_origin = format!("mqtt://{}/", other_broker.address);
    let backend = relay().await;
    let (silent_relay, stop) =
        delayed_relay::delay_relay(backend.http_addr().unwrap(), Duration::from_secs(45)).await;

    for label in [
        "irc-only",
        "both-ready",
        "other-mqtt-origin",
        "withdrawn-mqtt",
        "silent-mqtt-first",
    ] {
        let broker = relay_contact_broker::Broker::new().await;
        let mqtt = format!("mqtt://{}/", broker.address);
        let resources = ResourceConfig {
            tcp_direct_disabled: true,
            member_data_relay_disabled: true,
            replacement: Some(
                [&irc, &mqtt]
                    .into_iter()
                    .filter(|url| label != "irc-only" || *url == &irc)
                    .map(|url| Resource {
                        url: url.clone(),
                        uses: [ResourceUse::Signaling].into(),
                        limitation: "owned public contact facility".into(),
                    })
                    .collect(),
            ),
            ..ResourceConfig::default()
        };
        let Pair {
            root,
            provider,
            consumer,
            ..
        } = Pair::admit_with_resources(resources).await;
        consumer.close().await.unwrap();
        provider.close().await.unwrap();
        let provider = NativeClient::open(root.path().join("provider"), Ipv4Addr::LOCALHOST)
            .await
            .unwrap();
        let consumer = reopen_with_resources(&root.path().join("consumer"), |resources| {
            let records = resources.replacement.as_mut().unwrap();
            if label == "other-mqtt-origin" {
                records
                    .iter_mut()
                    .find(|resource| resource.url == mqtt)
                    .unwrap()
                    .url = wrong_origin.clone();
            } else if label == "silent-mqtt-first" {
                records.push(relay_resource(&silent_relay));
            }
        })
        .await;
        let target = provider.invitation().await.unwrap().address().clone();
        let mut contact = iroh::EndpointAddr::new(target.id);
        contact.addrs.extend(target.addrs.into_iter().filter(|addr| {
            matches!(addr, iroh::TransportAddr::Custom(address)
                if [irc_transport::TRANSPORT_ID, mqtt_transport::TRANSPORT_ID].contains(&address.id()))
        }));
        assert_eq!(contact.addrs.len(), if label == "irc-only" { 1 } else { 2 });
        assert!(provider.address().relay_urls().next().is_none());
        if label == "silent-mqtt-first" {
            contact
                .addrs
                .insert(iroh::TransportAddr::Relay(silent_relay.clone()));
        }
        if matches!(label, "withdrawn-mqtt" | "silent-mqtt-first") {
            broker.mute_datagrams();
        }
        let before = broker.datagram_publishes();
        let started = Instant::now();
        println!("MIO149 public {label} start deadline={REPLY_DEADLINE:?}");
        let (reply, ()) = timeout(REPLY_DEADLINE, async {
            tokio::join!(consumer.ping(contact), async {
                if label == "withdrawn-mqtt" {
                    wait_until(|| broker.datagram_publishes() > before).await;
                    broker.disconnect();
                }
            })
        })
        .await
        .expect("the preselected standalone IRC deadline also bounds signal fallback");
        let reply = reply.unwrap();
        let elapsed = started.elapsed();
        assert_eq!(reply["reply"], "PONG");
        assert_eq!(reply["transport"]["peer"], target.id.to_string());
        assert!(
            reply["transport"]["initial_data_path"]["Ip"].is_string(),
            "{reply}"
        );
        let mqtt_packets = broker.datagram_publishes() - before;
        if matches!(label, "both-ready" | "withdrawn-mqtt" | "silent-mqtt-first") {
            assert!(
                mqtt_packets > 0,
                "the first MQTT entry actually sent packets"
            );
        } else {
            assert_eq!(mqtt_packets, 0, "this route should only use IRC");
        }
        if label == "withdrawn-mqtt" {
            assert!(
                elapsed < IO,
                "known unavailability leaves the IRC opportunity immediately"
            );
        }
        assert_eq!(
            other_broker.datagram_publishes(),
            0,
            "the other MQTT origin cannot contact this peer"
        );
        println!(
            "MIO149 public {label} elapsed={elapsed:?} mqtt_packets={mqtt_packets} reply={reply}"
        );
        consumer.close().await.unwrap();
        provider.close().await.unwrap();
        broker.close().await;
    }
    stop.cancel();
    backend.shutdown().await.unwrap();
    other_broker.close().await;
}

#[tokio::test]
#[ignore = "requires the owned real IRCd supplied by MIOPUNCH_IRC_SERVER"]
async fn only_configured_irc_contact_cannot_release_business_without_udp() {
    let (_root, provider, consumer, contact) = restricted_service_pair(
        owned_irc_url(),
        miopunch_native_probe::irc_transport::TRANSPORT_ID,
        false,
    )
    .await;
    verify_no_forward_release(
        &provider,
        &consumer,
        contact,
        std::io::ErrorKind::NotConnected,
    )
    .await;
    consumer.close().await.unwrap();
    provider.close().await.unwrap();
}

fn owned_irc_url() -> String {
    format!(
        "irc://{}/",
        std::env::var("MIOPUNCH_IRC_SERVER").expect("owned real IRCd required")
    )
}

async fn restricted_service_pair(
    url: String,
    carrier: u64,
    receiver_udp: bool,
) -> (
    tempfile::TempDir,
    NativeClient,
    NativeClient,
    iroh::EndpointAddr,
) {
    let mut resources = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![Resource {
            url,
            uses: [ResourceUse::Signaling].into(),
            limitation: "owned restricted contact; payload requires a qualified UDP path".into(),
        }],
    });
    resources.tcp_direct_disabled = true;
    resources.member_data_relay_disabled = true;
    let Pair {
        root,
        provider,
        consumer,
        ..
    } = Pair::admit_with_resources(resources).await;
    consumer.close().await.unwrap();
    provider.close().await.unwrap();
    let provider = reopen_with_resources(&root.path().join("provider"), |resources| {
        resources.udp_direct_disabled = !receiver_udp;
    })
    .await;
    let consumer = NativeClient::open(root.path().join("consumer"), Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let mut contact = provider.invitation().await.unwrap().address().clone();
    contact.addrs.retain(|addr| {
        matches!(addr, iroh::TransportAddr::Custom(address)
        if address.id() == carrier)
    });
    assert_eq!(
        contact.addrs.len(),
        1,
        "only the approved restricted contact is supplied"
    );
    assert!(consumer.address().relay_urls().next().is_none());
    assert!(provider.address().relay_urls().next().is_none());
    (root, provider, consumer, contact)
}

async fn verify_first_forward_reply(
    provider: &NativeClient,
    consumer: &NativeClient,
    contact: iroh::EndpointAddr,
) {
    let target = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let target_address = target.local_addr().unwrap();
    provider
        .enable_forwarding(Protocol::Tcp, vec![target_address])
        .await
        .unwrap();
    let (reply, ()) = timeout(Duration::from_secs(20), async {
        tokio::join!(
            async {
                consumer
                    .forward_exchange(
                        contact,
                        Protocol::Tcp,
                        target_address,
                        b"contact-only request",
                        b"qualified data reply".len(),
                    )
                    .await
                    .expect("restricted call must deliver the qualified native reply")
            },
            async {
                let (mut socket, _) = target.accept().await.unwrap();
                let mut request = [0; 20];
                socket.read_exact(&mut request).await.unwrap();
                assert_eq!(&request, b"contact-only request");
                for (side, endpoint, peer) in [
                    ("consumer", consumer, provider),
                    ("provider", provider, consumer),
                ] {
                    let status = flow(endpoint, Protocol::Tcp);
                    let transport = &status["transport"];
                    assert_eq!(
                        transport["peer"],
                        peer.address().id.to_string(),
                        "{side}: {status}"
                    );
                    assert!(transport["stream"].is_string(), "{side}: {status}");
                    assert!(
                        transport["initial_data_path"]["Ip"].is_string(),
                        "{side}: {status}"
                    );
                    assert!(
                        transport["paths"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|path| path["selected"] == true
                                && path["remote"] == transport["initial_data_path"]),
                        "{side}: {status}"
                    );
                    eprintln!("MIO148 {side} {transport}");
                }
                socket.write_all(b"qualified data reply").await.unwrap();
            },
        )
    })
    .await
    .expect("restricted contact qualifies and releases the first native member data path");
    assert_eq!(reply, b"qualified data reply");
}

async fn verify_no_forward_release(
    provider: &NativeClient,
    consumer: &NativeClient,
    contact: iroh::EndpointAddr,
    kind: std::io::ErrorKind,
) {
    let target = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let address = target.local_addr().unwrap();
    provider
        .enable_forwarding(Protocol::Tcp, vec![address])
        .await
        .unwrap();
    let started = Instant::now();
    let error = timeout(
        Duration::from_secs(20),
        consumer.forward_exchange(
            contact,
            Protocol::Tcp,
            address,
            b"must not reach the target",
            1,
        ),
    )
    .await
    .expect("the restricted first dial settles within its original budget")
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        kind,
        "{error}"
    );
    if kind == std::io::ErrorKind::NotConnected {
        assert!(
            error.to_string().contains("no permitted member data path"),
            "{error}"
        );
    }
    assert!(
        timeout(Duration::from_millis(150), target.accept())
            .await
            .is_err()
    );
    eprintln!("MIO148 data stayed closed {:?}: {error}", started.elapsed());
}

#[tokio::test]
async fn file_provider_with_only_the_dedicated_service_protocol_still_interoperates() {
    let relay = relay().await;
    let Pair {
        root,
        provider,
        consumer,
        target,
    } = Pair::admit(relay.http_url().unwrap()).await;
    provider
        .enable_files(root.path().join("receive"))
        .await
        .unwrap();
    consumer.sync_services(target.clone()).await.unwrap();
    provider.close().await.unwrap();
    let node = NativeNetwork::open(root.path().join("provider"))
        .await
        .unwrap();
    node.admission_gate()
        .register_protocol(FILE_ALPN, ProtocolScope::Member);
    let endpoint = node
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    endpoint.endpoint().set_alpns(vec![FILE_ALPN.to_vec()]);
    let path = root.path().join("compatible.bin");
    let bytes = b"regular file bytes on a dedicated ALPN";
    std::fs::write(&path, bytes).unwrap();
    let task = consumer.send_file(target, path).await.unwrap();
    timeout(Duration::from_secs(15), async {
        let connection = loop {
            if let Ok(connection) = endpoint.endpoint().accept().await.unwrap().await {
                break connection;
            }
        };
        assert_eq!(connection.alpn(), FILE_ALPN);
        assert_eq!(connection.remote_id(), consumer.address().id);
        let mut stream = DataStream::accept(&connection).await.unwrap();
        let offer: FileOffer = serde_json::from_slice(&stream.read_chunk().await.unwrap()).unwrap();
        assert_eq!(offer.size, bytes.len() as u64);
        stream
            .write_chunk(br#"{"type":"ready","offset":0}"#)
            .await
            .unwrap();
        assert_eq!(stream.read_chunk().await.unwrap(), bytes);
        assert!(stream.read_chunk().await.unwrap().is_empty());
        let receipt = FileReceipt {
            saved_name: offer.saved_name(),
            id: offer.id,
            size: offer.size,
            sha256: offer.sha256,
        };
        let saved =
            serde_json::to_vec(&serde_json::json!({"type": "saved", "receipt": receipt})).unwrap();
        stream.write_chunk(&saved).await.unwrap();
        stream.finish().unwrap();
        connection.closed().await;
        wait_until(|| {
            consumer
                .file_tasks()
                .iter()
                .any(|status| status.id == task && status.receipt.as_ref() == Some(&receipt))
        })
        .await;
    })
    .await
    .unwrap();
    consumer.close().await.unwrap();
    endpoint.close().await;
    node.close().await;
    relay.shutdown().await.unwrap();
}

fn received_file_status(client: &NativeClient, task: &str) -> Option<FileTaskStatus> {
    client
        .file_tasks()
        .into_iter()
        .find(|status| status.id == task && status.direction == "receive")
}

async fn timed_ping(pair: &Pair) -> serde_json::Value {
    let started = Instant::now();
    let reply = timeout(IO, pair.consumer.ping(pair.target.clone()))
        .await
        .expect("same five-second small-request deadline for both runs")
        .unwrap();
    assert_eq!(reply["reply"], "PONG");
    assert!(
        started.elapsed() >= Duration::from_secs(1),
        "configured relay delay reached the real business bytes"
    );
    assert!(
        reply["transport"]["paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path["selected"] == true && path["remote"].get("Relay").is_some()),
        "selected path is the delayed relay: {reply}"
    );
    reply
}

#[tokio::test]
async fn small_reply_meets_its_original_deadline_during_large_relay_stream_progress() {
    const RELAY_BYTES_PER_SECOND: usize = 1024 * 1024;
    let relay = relay().await;
    // A 64 KiB file chunk takes over 50 ms on this downstream leg, so the
    // receiver can expose separate progress samples even if QUIC sends a burst.
    let (url, stop) = delayed_relay::delay_relay_with_rate(
        relay.http_addr().unwrap(),
        Duration::from_millis(350),
        std::num::NonZeroUsize::new(RELAY_BYTES_PER_SECOND),
    )
    .await;
    let pair = Pair::admit(url).await;
    pair.provider
        .enable_files(pair.root.path().join("inbox"))
        .await
        .unwrap();
    pair.consumer
        .sync_services(pair.target.clone())
        .await
        .unwrap();
    // Warm the shared connection. Both timed requests reuse this same admitted
    // connection and observe the native ping's original five-second request budget.
    assert_eq!(
        pair.consumer.ping(pair.target.clone()).await.unwrap()["reply"],
        "PONG"
    );
    let solo = timed_ping(&pair).await;

    const LARGE: u64 = 64 * 1024 * 1024;
    let path = pair.root.path().join("concurrent.bin");
    std::fs::File::create(&path)
        .unwrap()
        .set_len(LARGE)
        .unwrap();
    let task = pair
        .consumer
        .send_file(pair.target.clone(), path)
        .await
        .unwrap();
    let before = timeout(Duration::from_secs(30), async {
        loop {
            if let Some(status) = received_file_status(&pair.provider, &task)
                && status.bytes >= 512 * 1024
            {
                break status;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("actual receiver progress before loading the small request");
    assert!(before.bytes < LARGE && !before.state.terminal());
    let mut peak = before.bytes;
    let mut advancements = 0;
    let mut sample = || {
        let status = received_file_status(&pair.provider, &task).unwrap();
        if status.bytes > peak {
            peak = status.bytes;
            advancements += 1;
        }
    };
    let reply = {
        let ping = timed_ping(&pair);
        tokio::pin!(ping);
        loop {
            tokio::select! {
                reply = &mut ping => {
                    // Include progress since the last poll without waiting past PONG.
                    sample();
                    break reply;
                }
                _ = sleep(Duration::from_millis(50)) => sample(),
            }
        }
    };
    assert!(
        advancements >= 2,
        "the receiving endpoint observed ongoing large-stream progress during the small reply, bytes {} to {peak}",
        before.bytes
    );
    assert_eq!(
        solo["transport"]["connection"],
        reply["transport"]["connection"]
    );
    let sender = pair
        .consumer
        .file_tasks()
        .into_iter()
        .find(|s| s.id == task)
        .unwrap();
    assert_eq!(
        sender.transport.unwrap()["connection"],
        reply["transport"]["connection"]
    );
    eprintln!(
        "MIO145 high_rtt relay_leg_ms=350 relay_bytes_per_second={RELAY_BYTES_PER_SECOND} deadline_ms=5000 solo={} loaded={} progress_start={} progress_end={} samples={advancements}",
        solo["timing"], reply["timing"], before.bytes, peak
    );
    pair.close().await;
    stop.cancel();
    relay.shutdown().await.unwrap();
}

#[tokio::test]
#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[ignore = "requires a disposable network namespace"]
async fn lost_initial_ready_rejoins_the_once_accepted_tcp_target() {
    let mut cut = NativeStreamCut::new_open(true).await;
    let channel = flow(&cut.pair.consumer, Protocol::Tcp)["channel"].clone();
    cut.interrupt_member().await;
    cut.pair.provider.test_pause_forward_ready(false);
    sequence_reply(&mut cut.sender, &mut cut.receiver, 172).await;
    sequence_reply(&mut cut.receiver, &mut cut.sender, 173).await;
    assert!(timeout(Duration::from_millis(150), cut.target.accept()).await.is_err(), "first open has one target accept");
    assert_eq!(cut.listener_stats()["accepted"], 1);
    assert_eq!(cut.listener_stats()["failed"], 0);
    assert_eq!(flow(&cut.pair.consumer, Protocol::Tcp)["channel"], channel);
    for (old, new) in cut.before.iter().zip(cut.status()) {
        assert_ne!(old["connection"], new["connection"]);
    }
    eprintln!("MIO172 first Ready lost after independent target accept, original sockets and channel={channel} on new QUIC");
    cut.close().await;
}

#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
async fn original_open_request(cut: &NativeStreamCut) -> miopunch_native_probe::forwarding::ChannelRequest {
    use miopunch_native_probe::forwarding::{ChannelOrigin, ChannelRequest};
    let connection = cut.pair.consumer.test_member_connection(cut.pair.target.id).await.unwrap();
    let mut origin = [0; 32];
    connection.export_keying_material(&mut origin, b"miopunch-forward-open", &[]).unwrap();
    let displayed = cut.before[0]["stream"].as_str().unwrap();
    let initiator = if displayed.starts_with("client ") { noq_proto::Side::Client } else { noq_proto::Side::Server };
    let index = displayed.rsplit(' ').next().unwrap().parse().unwrap();
    let stream = noq_proto::StreamId::new(initiator, noq_proto::Dir::Bi, index).into();
    ChannelRequest {
        id: flow(&cut.pair.consumer, Protocol::Tcp)["channel"].as_str().unwrap().to_owned(),
        origin: ChannelOrigin { connection: origin, stream },
        resume: false,
        generation: 0,
    }
}

#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
async fn duplicate_open_result(cut: &NativeStreamCut, request: miopunch_native_probe::forwarding::ChannelRequest) -> miopunch_native_probe::forwarding::ChannelReady {
    use miopunch_native_probe::forwarding::{ChannelReady, Open};
    let connection = cut.pair.consumer.test_member_connection(cut.pair.target.id).await.unwrap();
    let mut duplicate = connection.open_bi().await.unwrap();
    let (send, recv) = duplicate.streams();
    send.write_all(&[4]).await.unwrap();
    let request = serde_json::to_vec(&Open {
        target: cut.target.local_addr().unwrap(), channel: Some(request),
    }).unwrap();
    send.write_all(&(request.len() as u32).to_be_bytes()).await.unwrap();
    send.write_all(&request).await.unwrap();
    let mut length = [0; 4];
    timeout(IO, recv.read_exact(&mut length)).await.unwrap().unwrap();
    let mut record = vec![0; u32::from_be_bytes(length) as usize];
    recv.read_exact(&mut record).await.unwrap();
    let result = serde_json::from_slice::<ChannelReady>(&record).unwrap();
    let mut byte = [0];
    assert!(timeout(IO, recv.read(&mut byte)).await.unwrap().unwrap().is_none(), "a duplicate never becomes a second IO carrier");
    send.finish().unwrap();
    result
}

#[tokio::test]
#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[ignore = "requires a disposable network namespace"]
async fn duplicate_and_delayed_open_leave_one_effective_channel() {
    let mut cut = NativeStreamCut::new_open(true).await;
    // Repeat the first request while its Ready is still paused. The result
    // refers to the same accepted object and cannot grant another IO carrier.
    let request = original_open_request(&cut).await;
    assert!(matches!(duplicate_open_result(&cut, request.clone()).await, miopunch_native_probe::forwarding::ChannelReady::Connected { attached: false }));
    assert!(timeout(Duration::from_millis(100), cut.target.accept()).await.is_err());
    cut.interrupt_member().await;
    cut.pair.provider.test_pause_forward_ready(false);
    sequence_reply(&mut cut.sender, &mut cut.receiver, 1721).await;
    // A delayed copy of generation zero arrives after the replacement took over.
    assert!(matches!(duplicate_open_result(&cut, request.clone()).await, miopunch_native_probe::forwarding::ChannelReady::Connected { attached: false }));
    let mut resumed = request;
    resumed.generation = 1;
    resumed.resume = true;
    assert!(matches!(duplicate_open_result(&cut, resumed).await, miopunch_native_probe::forwarding::ChannelReady::Connected { attached: false }));
    sequence_reply(&mut cut.receiver, &mut cut.sender, 1722).await;
    assert!(timeout(Duration::from_millis(150), cut.target.accept()).await.is_err());
    assert_eq!(cut.pair.provider.forwarding_status()["active_flows"], 1);
    assert_eq!(cut.listener_stats()["accepted"], 1);
    assert_eq!(cut.listener_stats()["failed"], 0);
    eprintln!("MIO172 duplicate first Open before Ready and delayed generations after replacement return same connected result; one IO owner and one target");
    cut.close().await;
}

#[tokio::test]
#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[ignore = "requires a disposable network namespace"]
async fn lost_open_record_reports_unknown_without_opening_another_target() {
    let mut cut = NativeStreamCut::new_open(true).await;
    let id = flow(&cut.pair.consumer, Protocol::Tcp)["channel"].as_str().unwrap().to_owned();
    let original = original_open_request(&cut).await;
    cut.pair.provider.test_forget_forward_open(&id);
    assert!(matches!(duplicate_open_result(&cut, original).await, miopunch_native_probe::forwarding::ChannelReady::OpenOutcomeUnknown), "a late original Open cannot recreate a lost target record even on the original QUIC");
    assert!(timeout(Duration::from_millis(150), cut.target.accept()).await.is_err());
    cut.interrupt_member().await;
    cut.pair.provider.test_pause_forward_ready(false);
    timeout(Duration::from_secs(60), async {
        while cut.listener_stats()["failed"] != 1 {
            sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("opening result settles within the node's existing reconnect budget");
    assert_eq!(cut.listener_stats()["last_error"], "open_outcome_unknown");
    let mut byte = [0];
    assert_eq!(timeout(IO, cut.sender.read(&mut byte)).await.unwrap().unwrap(), 0);
    assert!(timeout(Duration::from_millis(150), cut.target.accept()).await.is_err());
    assert_eq!(cut.listener_stats()["accepted"], 1);
    eprintln!("MIO172 lost opening record => public listener.last_error=open_outcome_unknown, original socket EOF, target accept=1");
    cut.close().await;
}

#[tokio::test]
#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[ignore = "requires a disposable network namespace"]
async fn disconnected_channel_accepts_bounded_bytes_while_application_wait_can_cancel() {
    let mut cut = NativeStreamCut::new().await;
    cut.learn_contact(0).await;
    let faults = cut.hold_interruption().await;
    cut.sender.write_all(&[173; 4096]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let status = cut.pair.consumer.forwarding_status();
    assert_eq!(
        status["channel_buffers"]["send_bytes"], 4096,
        "the channel accepts application bytes during carrier recovery"
    );
    let mut byte = [0];
    assert!(
        timeout(Duration::from_millis(100), cut.sender.read(&mut byte))
            .await
            .is_err()
    );
    drop(faults);
    cut.broker.unmute_datagrams();
    let mut bytes = [0; 4096];
    timeout(Duration::from_secs(15), cut.receiver.read_exact(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bytes, [173; 4096]);
    sequence_reply(&mut cut.sender, &mut cut.receiver, 174).await;
    cut.close().await;
}

#[tokio::test]
#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[ignore = "requires a disposable network namespace"]
async fn recovery_expiry_delivers_accepted_unread_tail_before_interruption() {
    let policy = miopunch_native_probe::forwarding::ChannelPolicy {
        recovery_ms: 1200,
        ..Default::default()
    };
    let mut cut = NativeStreamCut::new_policy(false, policy).await;
    cut.learn_contact(0).await;
    cut.pair.consumer.test_limit_channel_delivery(68);
    cut.receiver.write_all(&[173; 100]).await.unwrap();
    let mut prefix = [0; 60];
    timeout(IO, cut.sender.read_exact(&mut prefix))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(prefix, [173; 60]);
    wait_until(|| cut.pair.consumer.forwarding_status()["channel_buffers"]["receive_bytes"] == 40)
        .await;
    let faults = cut.hold_interruption().await;
    let started = Instant::now();
    let mut tail = [0; 40];
    timeout(IO, cut.sender.read_exact(&mut tail))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(tail, [173; 40]);
    let mut byte = [0];
    assert_eq!(
        timeout(IO, cut.sender.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    wait_until(|| cut.listener_stats()["failed"] == 1).await;
    assert_eq!(
        cut.listener_stats()["last_error"],
        "business_channel_recovery_expired"
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(
        timeout(Duration::from_millis(100), cut.target.accept())
            .await
            .is_err()
    );
    eprintln!(
        "MIO173 accepted=100 application_before_expiry=60 tail_after_expiry=40 EOF=true elapsed={:?} buffers={}",
        started.elapsed(),
        cut.pair.consumer.forwarding_status()["channel_buffers"]
    );
    drop(faults);
    cut.close().await;
}

#[tokio::test]
#[cfg(feature = "nat-lab-test-hooks")]
async fn healthy_quiet_and_one_way_channels_do_not_spend_recovery_time() {
    let policy = miopunch_native_probe::forwarding::ChannelPolicy {
        recovery_ms: 100,
        ..Default::default()
    };
    let mut cut = NativeStreamCut::new_policy(false, policy).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    sequence_reply(&mut cut.sender, &mut cut.receiver, 173).await;
    for number in 0..5u8 {
        cut.receiver.write_all(&[number; 100]).await.unwrap();
        let mut bytes = [0; 100];
        timeout(IO, cut.sender.read_exact(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(bytes, [number; 100]);
        tokio::time::sleep(Duration::from_millis(80)).await;
    }
    assert_eq!(cut.status()[0]["connection"], cut.before[0]["connection"]);
    cut.close().await;
}

#[tokio::test]
#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[ignore = "requires a disposable network namespace"]
async fn node_and_channel_limits_backpressure_both_directions_and_release_after_resume() {
    let policy = miopunch_native_probe::forwarding::ChannelPolicy {
        channel_bytes: 64 * 1024,
        node_bytes_per_direction: 96 * 1024,
        ..Default::default()
    };
    let mut cut = NativeStreamCut::new_policy(false, policy).await;
    cut.learn_contact(0).await;
    let listener = cut.pair.consumer.forwarding_status()["listeners"][0]["listen"].clone();
    let address: SocketAddr = serde_json::from_value(listener).unwrap();
    let mut second_sender = TcpStream::connect(address).await.unwrap();
    let (mut second_receiver, _) = timeout(IO, cut.target.accept()).await.unwrap().unwrap();
    sequence_reply(&mut second_sender, &mut second_receiver, 1).await;
    wait_until(|| {
        [&cut.pair.consumer, &cut.pair.provider]
            .iter()
            .all(|node| node.forwarding_status()["channel_buffers"]["send_bytes"] == 0)
    })
    .await;
    let faults = cut.hold_interruption().await;
    cut.sender.write_all(&vec![1; 128 * 1024]).await.unwrap();
    cut.receiver.write_all(&vec![2; 128 * 1024]).await.unwrap();
    sleep(Duration::from_millis(200)).await;
    eprintln!(
        "MIO173 first quota consumer={} provider={}",
        cut.pair.consumer.forwarding_status()["channel_buffers"],
        cut.pair.provider.forwarding_status()["channel_buffers"]
    );
    wait_until(|| {
        [&cut.pair.consumer, &cut.pair.provider].iter().all(|node| {
            node.forwarding_status()["channel_buffers"]["send_bytes"]
                .as_u64()
                .is_some_and(|bytes| (65520..=65552).contains(&bytes))
        })
    })
    .await;
    second_sender.write_all(&[3; 16 * 1024]).await.unwrap();
    second_receiver.write_all(&[4; 16 * 1024]).await.unwrap();
    sleep(Duration::from_millis(200)).await;
    eprintln!(
        "MIO173 spare quota consumer={} provider={}",
        cut.pair.consumer.forwarding_status()["channel_buffers"],
        cut.pair.provider.forwarding_status()["channel_buffers"]
    );
    wait_until(|| {
        [&cut.pair.consumer, &cut.pair.provider].iter().all(|node| {
            node.forwarding_status()["channel_buffers"]["send_bytes"]
                .as_u64()
                .is_some_and(|bytes| (81904..=81936).contains(&bytes))
        })
    })
    .await;
    second_sender.write_all(&[5; 32 * 1024]).await.unwrap();
    second_receiver.write_all(&[6; 32 * 1024]).await.unwrap();
    wait_until(|| {
        [&cut.pair.consumer, &cut.pair.provider]
            .iter()
            .all(|node| node.forwarding_status()["channel_buffers"]["send_bytes"] == 98304)
    })
    .await;
    socket2::SockRef::from(&cut.sender)
        .set_send_buffer_size(4096)
        .unwrap();
    let fill = vec![7; 4 * 1024 * 1024];
    assert!(
        timeout(Duration::from_millis(150), cut.sender.write_all(&fill))
            .await
            .is_err(),
        "full channel eventually backpressures the application socket"
    );
    // The cancelled write may have enqueued a prefix in the OS. Verify the
    // independent second channel exactly, and consume the first's known prefix.
    drop(faults);
    cut.broker.unmute_datagrams();
    let started = Instant::now();
    let mut first_out = vec![0; 128 * 1024];
    let mut first_in = vec![0; 128 * 1024];
    let mut second_out = vec![0; 48 * 1024];
    let mut second_in = vec![0; 48 * 1024];
    timeout(Duration::from_secs(20), async {
        tokio::try_join!(
            cut.receiver.read_exact(&mut first_out),
            cut.sender.read_exact(&mut first_in),
            second_receiver.read_exact(&mut second_out),
            second_sender.read_exact(&mut second_in)
        )
        .unwrap();
    })
    .await
    .unwrap();
    assert_eq!(first_out, vec![1; 128 * 1024]);
    assert_eq!(first_in, vec![2; 128 * 1024]);
    assert_eq!(&second_out[..16 * 1024], &[3; 16 * 1024]);
    assert_eq!(&second_out[16 * 1024..], &[5; 32 * 1024]);
    assert_eq!(&second_in[..16 * 1024], &[4; 16 * 1024]);
    assert_eq!(&second_in[16 * 1024..], &[6; 32 * 1024]);
    sequence_reply(&mut second_sender, &mut second_receiver, 173).await;
    for node in [&cut.pair.consumer, &cut.pair.provider] {
        let stats = node.forwarding_status();
        assert!(
            stats["channel_buffers"]["peak_send_bytes"]
                .as_u64()
                .unwrap()
                <= 98304
        );
        assert!(
            stats["channel_buffers"]["peak_receive_bytes"]
                .as_u64()
                .unwrap()
                <= 98304
        );
        assert_eq!(
            flow(node, Protocol::Tcp)["transport"]["connection"],
            stats["flows"][1]["transport"]["connection"],
            "both channels reuse the one admitted member connection"
        );
    }
    eprintln!(
        "MIO173 two-way channel=65536 node=98304 spare=16384 full-write-cancel=true resumed_bytes={} elapsed={:?} budgets={} {}",
        352 * 1024,
        started.elapsed(),
        cut.pair.consumer.forwarding_status()["channel_buffers"],
        cut.pair.provider.forwarding_status()["channel_buffers"]
    );
    drop(second_sender);
    drop(second_receiver);
    cut.close().await;
}

#[tokio::test]
#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[ignore = "requires a disposable network namespace"]
async fn recovery_cancellation_and_target_removal_do_not_revive_original_objects() {
    let mut cut = NativeStreamCut::new().await;
    cut.learn_contact(0).await;
    let other_target = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let other_address = other_target.local_addr().unwrap();
    cut.pair
        .provider
        .enable_forwarding(
            Protocol::Tcp,
            vec![cut.target.local_addr().unwrap(), other_address],
        )
        .await
        .unwrap();
    let listener = cut
        .pair
        .consumer
        .start_forwarding(
            cut.pair.target.clone(),
            Protocol::Tcp,
            other_address,
            (Ipv4Addr::LOCALHOST, 0).into(),
        )
        .await
        .unwrap();
    let mut other_sender = TcpStream::connect(
        serde_json::from_value::<SocketAddr>(listener["listen"].clone()).unwrap(),
    )
    .await
    .unwrap();
    let (mut other_receiver, _) = timeout(IO, other_target.accept()).await.unwrap().unwrap();
    sequence_reply(&mut other_sender, &mut other_receiver, 1).await;
    let faults = cut.hold_interruption().await;
    cut.pair
        .provider
        .enable_forwarding(Protocol::Tcp, vec![other_address])
        .await
        .unwrap();
    wait_until(|| cut.pair.provider.forwarding_status()["active_flows"] == 1).await;
    drop(faults);
    cut.broker.unmute_datagrams();
    timeout(Duration::from_secs(20), async {
        let mut byte = [0];
        assert_eq!(cut.sender.read(&mut byte).await.unwrap(), 0);
        assert_eq!(cut.receiver.read(&mut byte).await.unwrap(), 0);
        sequence_reply(&mut other_sender, &mut other_receiver, 173).await;
    })
    .await
    .unwrap();
    assert!(
        timeout(Duration::from_millis(150), cut.target.accept())
            .await
            .is_err()
    );
    let id = cut.pair.consumer.forwarding_status()["flows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["target"] == serde_json::to_value(other_address).unwrap())
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    cut.pair.consumer.cancel_forwarding_flow(&id).await.unwrap();
    let mut byte = [0];
    assert_eq!(
        timeout(IO, other_sender.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert_eq!(
        timeout(IO, other_receiver.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert!(
        timeout(Duration::from_millis(150), other_target.accept())
            .await
            .is_err()
    );
    eprintln!(
        "MIO173 removed target ends while unaffected target resumes; explicit cancel closes both original sockets with no late accept"
    );
    cut.close().await;
}

#[tokio::test]
#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[ignore = "requires a disposable network namespace"]
async fn replacement_ready_handshake_cannot_restart_the_recovery_deadline() {
    let policy = miopunch_native_probe::forwarding::ChannelPolicy {
        recovery_ms: 12000,
        ..Default::default()
    };
    let mut cut = NativeStreamCut::new_policy(false, policy).await;
    cut.learn_contact(0).await;
    cut.pair.provider.test_pause_forward_ready(true);
    let faults = cut.hold_interruption().await;
    let started = Instant::now();
    drop(faults);
    cut.broker.unmute_datagrams();
    wait_until(|| cut.status()[0]["connection"] != cut.before[0]["connection"]).await;
    let replaced = started.elapsed();
    let mut byte = [0];
    assert_eq!(
        timeout(Duration::from_secs(14), cut.sender.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert!(started.elapsed() >= Duration::from_secs(10));
    assert!(
        started.elapsed() < Duration::from_secs(14),
        "replacement before Ready keeps the same recovery cutoff"
    );
    wait_until(|| cut.listener_stats()["failed"] == 1).await;
    assert_eq!(
        cut.listener_stats()["last_error"],
        "business_channel_recovery_expired"
    );
    assert!(
        timeout(Duration::from_millis(100), cut.target.accept())
            .await
            .is_err()
    );
    eprintln!(
        "MIO173 new QUIC after={replaced:?}, paused Ready expired once at={:?}, original target accepts=1",
        started.elapsed()
    );
    cut.close().await;
}

#[tokio::test]
#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[ignore = "requires a disposable network namespace"]
async fn service_revocation_during_recovery_cancels_only_the_denied_service() {
    let mut cut = NativeStreamCut::new().await;
    cut.learn_contact(0).await;
    let faults = cut.hold_interruption().await;
    let provider_id = cut.pair.provider.status().await.node.node_id;
    cut.pair
        .provider
        .set_service_rule(
            provider_id,
            Service::TcpForward,
            None,
            ServiceRule {
                members: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    wait_until(|| cut.pair.provider.forwarding_status()["active_flows"] == 0).await;
    let mut byte = [0];
    assert_eq!(
        timeout(IO, cut.receiver.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    drop(faults);
    cut.broker.unmute_datagrams();
    assert_eq!(
        timeout(Duration::from_secs(20), cut.sender.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert!(
        timeout(Duration::from_millis(100), cut.target.accept())
            .await
            .is_err()
    );
    let pong = timeout(
        Duration::from_secs(20),
        cut.pair.consumer.ping(cut.pair.provider.address()),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(pong["transport"]["connection"].is_u64());
    eprintln!(
        "MIO173 revoked TCP during recovery ends both sockets, no late target accept, authorized ping still works"
    );
    cut.close().await;
}

#[tokio::test]
#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[ignore = "requires a disposable network namespace and real guest expiry"]
async fn scoped_guest_expiry_during_outage_retires_its_original_target() {
    let relay = relay().await;
    let url = relay.http_url().unwrap();
    let pair = Pair::admit(url.clone()).await;
    let target = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let destination = target.local_addr().unwrap();
    pair.provider
        .enable_forwarding(Protocol::Tcp, vec![destination])
        .await
        .unwrap();
    let expires = service_policy::now() + 20;
    let invitation = pair
        .provider
        .issue_service_guest(pair.target.clone(), Service::TcpForward, expires)
        .await
        .unwrap();
    let mut resources = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![relay_resource(&url)],
    });
    resources.udp_direct_disabled = true;
    resources.tcp_direct_disabled = true;
    let guest = NativeClient::join_guest_with_resources(
        pair.root.path().join("guest"),
        &invitation,
        Ipv4Addr::LOCALHOST,
        resources,
    )
    .await
    .unwrap();
    assert_eq!(guest.redeem_guest().await.unwrap()["stage"], "GuestActive");
    let listener = guest
        .start_forwarding(
            pair.target.clone(),
            Protocol::Tcp,
            destination,
            (Ipv4Addr::LOCALHOST, 0).into(),
        )
        .await
        .unwrap();
    let mut sender = TcpStream::connect(
        serde_json::from_value::<SocketAddr>(listener["listen"].clone()).unwrap(),
    )
    .await
    .unwrap();
    let (mut receiver, _) = timeout(IO, target.accept()).await.unwrap().unwrap();
    sequence_reply(&mut sender, &mut receiver, 173).await;
    assert!(service_policy::now() < expires);
    let fault = PortBlackhole::new(url.port_or_known_default().unwrap());
    while service_policy::now() <= expires {
        sleep(Duration::from_millis(100)).await;
    }
    wait_until(|| pair.provider.forwarding_status()["active_flows"] == 0).await;
    let mut byte = [0];
    assert_eq!(
        timeout(IO, receiver.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    drop(fault);
    assert!(
        guest
            .start_forwarding(
                pair.target.clone(),
                Protocol::Tcp,
                destination,
                (Ipv4Addr::LOCALHOST, 0).into()
            )
            .await
            .is_err()
    );
    assert!(
        timeout(Duration::from_millis(150), target.accept())
            .await
            .is_err()
    );
    assert!(
        timeout(
            Duration::from_secs(20),
            pair.consumer.ping(pair.target.clone())
        )
        .await
        .unwrap()
        .is_ok()
    );
    eprintln!(
        "MIO173 guest actual expiry={expires}, target closed during relay blackout, no late accept, member ping allowed"
    );
    guest.close().await.unwrap();
    pair.close().await;
    relay.shutdown().await.unwrap();
}

#[tokio::test]
#[cfg(feature = "nat-lab-test-hooks")]
async fn reliable_channel_defaults_have_measured_bidirectional_progress_and_bounded_memory() {
    let mut cut = NativeStreamCut::new().await;
    let before = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let before_rss = before
        .lines()
        .find(|line| line.starts_with("VmRSS:"))
        .unwrap_or("RSS unavailable");
    let count = 4 * 1024 * 1024;
    let outgoing: Vec<_> = (0..count).map(|n| (n % 251) as u8).collect();
    let incoming: Vec<_> = (0..count).map(|n| (n % 239) as u8).collect();
    let mut out_read = vec![0; count];
    let mut in_read = vec![0; count];
    let started = Instant::now();
    {
        let (mut consumer_read, mut consumer_write) = cut.sender.split();
        let (mut provider_read, mut provider_write) = cut.receiver.split();
        timeout(Duration::from_secs(30), async {
            tokio::try_join!(
                consumer_write.write_all(&outgoing),
                provider_write.write_all(&incoming),
                consumer_read.read_exact(&mut in_read),
                provider_read.read_exact(&mut out_read)
            )
            .unwrap();
        })
        .await
        .unwrap();
    }
    assert_eq!(out_read, outgoing);
    assert_eq!(in_read, incoming);
    let elapsed = started.elapsed();
    let after = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let after_rss = after
        .lines()
        .filter(|line| line.starts_with("VmRSS:") || line.starts_with("VmHWM:"))
        .collect::<Vec<_>>();
    eprintln!(
        "MIO173 measured duplex bytes={} elapsed={elapsed:?} bytes_per_second={} RSS_before={before_rss} RSS_after={after_rss:?} budget_consumer={} budget_provider={}",
        2 * count,
        (2 * count) as f64 / elapsed.as_secs_f64(),
        cut.pair.consumer.forwarding_status()["channel_buffers"],
        cut.pair.provider.forwarding_status()["channel_buffers"]
    );
    cut.close().await;
}

#[tokio::test]
#[cfg(feature = "nat-lab-test-hooks")]
async fn receive_node_quota_is_shared_and_released_by_application_delivery() {
    let policy = miopunch_native_probe::forwarding::ChannelPolicy {
        channel_bytes: 64 * 1024,
        node_bytes_per_direction: 96 * 1024,
        ..Default::default()
    };
    let mut cut = NativeStreamCut::new_policy(false, policy).await;
    let address: SocketAddr = serde_json::from_value(
        cut.pair.consumer.forwarding_status()["listeners"][0]["listen"].clone(),
    )
    .unwrap();
    let mut sender2 = TcpStream::connect(address).await.unwrap();
    let (mut receiver2, _) = timeout(IO, cut.target.accept()).await.unwrap().unwrap();
    sequence_reply(&mut sender2, &mut receiver2, 1).await;
    wait_until(|| {
        [&cut.pair.consumer, &cut.pair.provider]
            .iter()
            .all(|node| node.forwarding_status()["channel_buffers"]["send_bytes"] == 0)
    })
    .await;
    cut.pair.consumer.test_limit_channel_delivery(8);
    cut.pair.provider.test_limit_channel_delivery(8);
    cut.sender.write_all(&vec![1; 64 * 1024]).await.unwrap();
    cut.receiver.write_all(&vec![2; 64 * 1024]).await.unwrap();
    wait_until(|| {
        [&cut.pair.consumer, &cut.pair.provider]
            .iter()
            .all(|node| node.forwarding_status()["channel_buffers"]["receive_bytes"] == 65536)
    })
    .await;
    sender2.write_all(&vec![3; 48 * 1024]).await.unwrap();
    receiver2.write_all(&vec![4; 48 * 1024]).await.unwrap();
    wait_until(|| {
        [&cut.pair.consumer, &cut.pair.provider]
            .iter()
            .all(|node| node.forwarding_status()["channel_buffers"]["receive_bytes"] == 98304)
    })
    .await;
    cut.pair.consumer.test_limit_channel_delivery(0);
    cut.pair.provider.test_limit_channel_delivery(0);
    let mut a = vec![0; 64 * 1024];
    let mut b = vec![0; 64 * 1024];
    let mut c = vec![0; 48 * 1024];
    let mut d = vec![0; 48 * 1024];
    timeout(IO, async {
        tokio::try_join!(
            cut.receiver.read_exact(&mut a),
            cut.sender.read_exact(&mut b),
            receiver2.read_exact(&mut c),
            sender2.read_exact(&mut d)
        )
        .unwrap();
    })
    .await
    .unwrap();
    assert_eq!(a, vec![1; 64 * 1024]);
    assert_eq!(b, vec![2; 64 * 1024]);
    assert_eq!(c, vec![3; 48 * 1024]);
    assert_eq!(d, vec![4; 48 * 1024]);
    wait_until(|| {
        [&cut.pair.consumer, &cut.pair.provider]
            .iter()
            .all(|node| node.forwarding_status()["channel_buffers"]["receive_bytes"] == 0)
    })
    .await;
    sequence_reply(&mut sender2, &mut receiver2, 173).await;
    eprintln!(
        "MIO173 receive pool at capacity=98304 both directions; application release drained exact bytes and next sequence"
    );
    drop(sender2);
    drop(receiver2);
    cut.close().await;
}

#[tokio::test]
#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[ignore = "requires a disposable network namespace"]
async fn cancelling_while_disconnected_ends_local_io_and_retained_peer_expires() {
    let policy = miopunch_native_probe::forwarding::ChannelPolicy {
        recovery_ms: 1200,
        ..Default::default()
    };
    let mut cut = NativeStreamCut::new_policy(false, policy).await;
    cut.learn_contact(0).await;
    let id = flow(&cut.pair.consumer, Protocol::Tcp)["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let faults = cut.hold_interruption().await;
    let started = Instant::now();
    cut.pair.consumer.cancel_forwarding_flow(&id).await.unwrap();
    let mut byte = [0];
    assert_eq!(
        timeout(Duration::from_millis(300), cut.sender.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert_eq!(cut.pair.consumer.forwarding_status()["active_flows"], 0);
    assert_eq!(
        timeout(Duration::from_secs(3), cut.receiver.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    drop(faults);
    cut.broker.unmute_datagrams();
    sleep(Duration::from_millis(300)).await;
    assert!(
        timeout(Duration::from_millis(150), cut.target.accept())
            .await
            .is_err()
    );
    eprintln!(
        "MIO173 explicit cancellation during outage closes local IO immediately; unreachable retained peer bounded by recovery window; no target reopens; elapsed={:?}",
        started.elapsed()
    );
    cut.close().await;
}

#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[tokio::test]
#[ignore = "requires a disposable network namespace and real tmux"]
async fn terminal_keeps_original_attachment_and_shell_after_member_replacement() {
    use miopunch_native_probe::terminal::{TerminalInput, TerminalProvider, TerminalRequest};
    let cut = NativeStreamCut::new().await;
    cut.learn_contact(0).await;
    let directory = cut.pair.root.path().join("terminal");
    cut.pair
        .provider
        .enable_terminal(TerminalProvider {
            directory: directory.to_str().unwrap().into(),
            distribution: None,
        })
        .await
        .unwrap();
    cut.pair
        .provider
        .sync_services(cut.pair.consumer.address())
        .await
        .unwrap();
    let workspace = cut
        .pair
        .consumer
        .request_terminal(cut.pair.provider.address(), TerminalRequest::Create)
        .await
        .unwrap()["workspace"]
        .as_str()
        .unwrap()
        .to_owned();
    let attached = cut
        .pair
        .consumer
        .attach_terminal(cut.pair.provider.address(), workspace.clone(), 80, 24)
        .await
        .unwrap();
    let id = attached["id"].as_str().unwrap();
    let command = "stty -echo; MIO174_STATE=original; printf '%s\\n' $$ > shell; printf 'BEFORE\\n' >> observed; printf 'MIO174_'; printf 'READY\\n'\n";
    cut.pair
        .consumer
        .send_terminal(
            id,
            TerminalInput::Bytes {
                data: command.as_bytes().to_vec(),
            },
        )
        .await
        .unwrap();
    terminal_literal(&cut.pair.consumer, id, b"MIO174_READY").await;
    let shell = std::fs::read(directory.join(&workspace).join("shell")).unwrap();
    cut.interrupt_member().await;
    cut.pair.consumer.send_terminal(id, TerminalInput::Bytes {data:b"printf '%s\\n' $$ > shell-after; printf '%s\\n' \"$MIO174_STATE\" >> observed; printf 'MIO174_'; printf 'RESUMED\\n'\n".to_vec()}).await.unwrap();
    terminal_literal(&cut.pair.consumer, id, b"MIO174_RESUMED").await;
    assert_eq!(
        std::fs::read(directory.join(&workspace).join("shell-after")).unwrap(),
        shell
    );
    assert_eq!(
        std::fs::read(directory.join(&workspace).join("observed")).unwrap(),
        b"BEFORE\noriginal\n"
    );
    assert_eq!(
        cut.pair.consumer.detach_terminal(id).await.unwrap()["failed"],
        false
    );
    cut.pair
        .consumer
        .request_terminal(
            cut.pair.provider.address(),
            TerminalRequest::End { workspace },
        )
        .await
        .unwrap();
    eprintln!(
        "MIO174 original attachment={} shell={} survived actual QUIC TimedOut",
        id,
        String::from_utf8_lossy(&shell)
    );
    cut.close().await;
}

#[tokio::test]
#[ignore = "observes a healthy retained channel beyond the former sixty-second IO deadline"]
async fn healthy_retained_channel_survives_more_than_sixty_seconds_of_application_silence() {
    let mut cut = NativeStreamCut::new().await;
    sleep(Duration::from_secs(62)).await;
    sequence_reply(&mut cut.sender, &mut cut.receiver, 174).await;
    for (old, new) in cut.before.iter().zip(cut.status()) {
        assert_eq!(old["connection"], new["connection"]);
        assert_eq!(old["stream"], new["stream"]);
    }
    eprintln!(
        "MIO174 healthy original channel accepted new IO after 62 seconds of application silence"
    );
    cut.close().await;
}

#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
async fn terminal_literal(node: &NativeClient, id: &str, literal: &[u8]) {
    let mut bytes = Vec::new();
    timeout(Duration::from_secs(15), async {
        loop {
            let reply = node
                .read_terminal(id, Duration::from_millis(100))
                .await
                .unwrap();
            assert_eq!(reply["attached"], true, "{reply}");
            if reply["message"]["type"] == "bytes" {
                bytes.extend(
                    serde_json::from_value::<Vec<u8>>(reply["message"]["data"].clone()).unwrap(),
                );
                if bytes.windows(literal.len()).any(|s| s == literal) {
                    break;
                }
            }
        }
    })
    .await
    .unwrap();
}

#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
async fn terminal_workspace(cut: &NativeStreamCut) -> (std::path::PathBuf, String) {
    use miopunch_native_probe::terminal::{TerminalProvider, TerminalRequest};
    let directory = cut.pair.root.path().join("terminal");
    cut.pair
        .provider
        .enable_terminal(TerminalProvider {
            directory: directory.to_str().unwrap().into(),
            distribution: None,
        })
        .await
        .unwrap();
    cut.pair
        .provider
        .sync_services(cut.pair.consumer.address())
        .await
        .unwrap();
    let workspace = cut
        .pair
        .consumer
        .request_terminal(cut.pair.provider.address(), TerminalRequest::Create)
        .await
        .unwrap()["workspace"]
        .as_str()
        .unwrap()
        .to_owned();
    (directory, workspace)
}

#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[tokio::test]
#[ignore = "requires disposable network namespace and real tmux"]
async fn terminal_lost_initial_ready_recovers_the_original_control_process() {
    use miopunch_native_probe::terminal::{TerminalInput, TerminalRequest};
    let cut = NativeStreamCut::new().await;
    cut.learn_contact(0).await;
    let (_, workspace) = terminal_workspace(&cut).await;
    cut.pair.provider.test_pause_terminal_ready(true);
    let opening =
        cut.pair
            .consumer
            .attach_terminal(cut.pair.provider.address(), workspace.clone(), 80, 24);
    let fault = async {
        wait_until(|| cut.pair.provider.terminal_status()["attachment_process_owners"] == 1).await;
        let original = cut.pair.provider.terminal_status()["operations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| !o["channel"].is_null())
            .unwrap()["transport"]["connection"]
            .clone();
        cut.interrupt_member().await;
        timeout(Duration::from_secs(15), async {
            loop {
                let changed = cut.pair.provider.terminal_status()["operations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|o| !o["channel"].is_null() && o["transport"]["connection"] != original);
                if changed {
                    break;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        // The first replacement also loses Ready. A fresh generation must
        // resume within the same original recovery deadline.
        cut.interrupt_member().await;
        cut.pair.provider.test_pause_terminal_ready(false);
    };
    let (attached, ()) = tokio::join!(opening, fault);
    let attached = attached.unwrap();
    let id = attached["id"].as_str().unwrap();
    assert_eq!(
        cut.pair.provider.terminal_status()["attachment_process_owners"],
        1
    );
    cut.pair
        .consumer
        .send_terminal(
            id,
            TerminalInput::Bytes {
                data: b"stty -echo; printf 'MIO174_'; printf 'INITIAL\\n'\n".to_vec(),
            },
        )
        .await
        .unwrap();
    terminal_literal(&cut.pair.consumer, id, b"MIO174_INITIAL").await;
    assert_eq!(
        cut.pair.consumer.detach_terminal(id).await.unwrap()["failed"],
        false
    );
    wait_until(|| cut.pair.provider.terminal_status()["attachment_process_owners"] == 0).await;
    cut.pair
        .consumer
        .request_terminal(
            cut.pair.provider.address(),
            TerminalRequest::End { workspace },
        )
        .await
        .unwrap();
    eprintln!(
        "MIO174 lost first and replacement Ready, original single tmux control process and channel recovered"
    );
    cut.close().await;
}

#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[tokio::test]
#[ignore = "requires disposable network namespace and real tmux"]
async fn terminal_expiry_delivers_queued_confirmed_output_before_failure() {
    use miopunch_native_probe::terminal::{TerminalInput, TerminalRequest};
    let policy = miopunch_native_probe::forwarding::ChannelPolicy {
        recovery_ms: 1200,
        ..Default::default()
    };
    let cut = NativeStreamCut::new_policy(false, policy).await;
    cut.learn_contact(0).await;
    let (_, workspace) = terminal_workspace(&cut).await;
    let attached = cut
        .pair
        .consumer
        .attach_terminal(cut.pair.provider.address(), workspace.clone(), 80, 24)
        .await
        .unwrap();
    let id = attached["id"].as_str().unwrap();
    cut.pair
        .consumer
        .send_terminal(
            id,
            TerminalInput::Bytes {
                data: b"stty -echo; printf 'MIO174_'; printf 'UNREAD\\n'\n".to_vec(),
            },
        )
        .await
        .unwrap();
    wait_until(|| {
        cut.pair.consumer.terminal_status()["queued_output_records"]
            .as_u64()
            .unwrap()
            > 0
    })
    .await;
    let faults = cut.hold_interruption().await;
    sleep(Duration::from_millis(1600)).await;
    let mut bytes = Vec::new();
    timeout(Duration::from_secs(6), async {
        loop {
            let reply = cut
                .pair
                .consumer
                .read_terminal(id, Duration::from_millis(100))
                .await
                .unwrap();
            if reply["message"]["type"] == "bytes" {
                bytes.extend(
                    serde_json::from_value::<Vec<u8>>(reply["message"]["data"].clone()).unwrap(),
                );
            }
            if reply["attached"] == false {
                assert_eq!(reply["failed"], true);
                break;
            }
        }
    })
    .await
    .unwrap();
    assert!(
        bytes
            .windows(b"MIO174_UNREAD".len())
            .any(|x| x == b"MIO174_UNREAD"),
        "confirmed queued output {bytes:?}"
    );
    drop(faults);
    cut.broker.unmute_datagrams();
    cut.pair.consumer.cancel_terminal(id).await.unwrap();
    cut.pair
        .consumer
        .request_terminal(
            cut.pair.provider.address(),
            TerminalRequest::End { workspace },
        )
        .await
        .unwrap();
    eprintln!("MIO174 confirmed unread terminal output drained before expiry failure");
    cut.close().await;
}

#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[tokio::test]
#[ignore = "requires disposable network namespace and sixteen real tmux control clients"]
async fn terminal_full_attachment_capacity_recovers_on_one_member_connection() {
    use miopunch_native_probe::terminal::{TerminalInput, TerminalRequest};
    let cut = NativeStreamCut::new().await;
    cut.learn_contact(0).await;
    let (_, workspace) = terminal_workspace(&cut).await;
    let mut attachments = Vec::new();
    for _ in 0..16 {
        let a = cut
            .pair
            .consumer
            .attach_terminal(cut.pair.provider.address(), workspace.clone(), 80, 24)
            .await
            .unwrap();
        attachments.push(a["id"].as_str().unwrap().to_owned());
    }
    assert_eq!(
        cut.pair.provider.terminal_status()["attachment_process_owners"],
        16
    );
    cut.interrupt_member().await;
    for (n, id) in attachments.iter().enumerate() {
        let marker = format!("MIO174_CAP_{n}");
        let command = format!("printf 'MIO174_'; printf 'CAP_{n}\\n'\n");
        cut.pair
            .consumer
            .send_terminal(
                id,
                TerminalInput::Bytes {
                    data: command.into_bytes(),
                },
            )
            .await
            .unwrap();
        terminal_literal(&cut.pair.consumer, id, marker.as_bytes()).await;
    }
    assert_eq!(
        cut.pair.provider.terminal_status()["attachment_process_owners"],
        16
    );
    let operations = cut.pair.consumer.terminal_status()["operations"]
        .as_array()
        .unwrap()
        .clone();
    let connections: std::collections::BTreeSet<_> = operations
        .iter()
        .filter_map(|o| o["transport"]["connection"].as_u64())
        .collect();
    assert_eq!(connections.len(), 1);
    for id in attachments {
        loop {
            let reply = cut
                .pair
                .consumer
                .read_terminal(&id, Duration::from_millis(20))
                .await
                .unwrap();
            if reply["pending"] == true {
                break;
            }
            assert_eq!(reply["attached"], true, "{reply}");
        }
        assert_eq!(
            cut.pair.consumer.detach_terminal(&id).await.unwrap()["failed"],
            false
        );
    }
    cut.pair
        .consumer
        .request_terminal(
            cut.pair.provider.address(),
            TerminalRequest::End { workspace },
        )
        .await
        .unwrap();
    eprintln!("MIO174 sixteen original attachments recovered on one shared new QUIC");
    cut.close().await;
}

#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[tokio::test]
#[ignore = "requires disposable network namespace and real tmux"]
async fn terminal_provider_expiry_delivers_confirmed_input_before_ending_control_client() {
    use miopunch_native_probe::terminal::{TerminalInput, TerminalRequest};
    let policy = miopunch_native_probe::forwarding::ChannelPolicy {
        recovery_ms: 1200,
        ..Default::default()
    };
    let cut = NativeStreamCut::new_policy(false, policy).await;
    cut.learn_contact(0).await;
    let (directory, workspace) = terminal_workspace(&cut).await;
    let attached = cut
        .pair
        .consumer
        .attach_terminal(cut.pair.provider.address(), workspace.clone(), 80, 24)
        .await
        .unwrap();
    let id = attached["id"].as_str().unwrap();
    cut.pair
        .consumer
        .send_terminal(
            id,
            TerminalInput::Bytes {
                data: b"stty -echo; printf 'MIO174_'; printf 'INPUT_READY\\n'\n".to_vec(),
            },
        )
        .await
        .unwrap();
    terminal_literal(&cut.pair.consumer, id, b"MIO174_INPUT_READY").await;
    cut.pair.provider.test_pause_terminal_input(true);
    cut.pair
        .consumer
        .send_terminal(
            id,
            TerminalInput::Bytes {
                data: b"printf 'confirmed-input' > expiry-input\n".to_vec(),
            },
        )
        .await
        .unwrap();
    wait_until(|| cut.pair.provider.test_terminal_input_waiting()).await;
    let faults = cut.hold_interruption().await;
    sleep(Duration::from_millis(1600)).await;
    assert!(!directory.join(&workspace).join("expiry-input").exists());
    cut.pair.provider.test_pause_terminal_input(false);
    wait_until(|| {
        std::fs::read(directory.join(&workspace).join("expiry-input"))
            .ok()
            .as_deref()
            == Some(b"confirmed-input")
    })
    .await;
    wait_until(|| cut.pair.provider.terminal_status()["attachment_process_owners"] == 0).await;
    drop(faults);
    cut.broker.unmute_datagrams();
    cut.pair.consumer.cancel_terminal(id).await.unwrap();
    cut.pair
        .consumer
        .request_terminal(
            cut.pair.provider.address(),
            TerminalRequest::End { workspace },
        )
        .await
        .unwrap();
    eprintln!(
        "MIO174 provider drains confirmed original input before expiry closes the tmux control client"
    );
    cut.close().await;
}
