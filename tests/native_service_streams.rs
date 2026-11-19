#[path = "support/delayed_relay.rs"]
mod delayed_relay;
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

async fn first_pong(
    client: &NativeClient,
    target: iroh::EndpointAddr,
    expected_path: iroh::TransportAddr,
) -> u64 {
    let peer = target.id.to_string();
    let reply = timeout(IO, client.ping(target)).await.unwrap().unwrap();
    println!("first-pong {reply}");
    assert_eq!(reply["reply"], "PONG");
    assert_eq!(reply["transport"]["peer"], peer);
    let transport = serde_json::to_value(expected_path).unwrap();
    assert_eq!(
        reply["transport"]["initial_data_path"], transport,
        "the released connection was qualified on {transport}: {reply}"
    );
    reply["transport"]["connection"].as_u64().unwrap()
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
    } = Pair::admit_with_resources(ResourceConfig {
        replacement: Some(vec![relay_resource(&relay)]),
        tcp_direct_disabled: true,
        ..ResourceConfig::default()
    })
    .await;
    let consumer_path = root.path().join("consumer");
    let consumer_id = consumer.address().id;
    let direct = target
        .ip_addrs()
        .find(|address| address.is_ipv4())
        .copied()
        .unwrap();
    consumer.close().await.unwrap();
    let consumer = reopen_with_resources(&consumer_path, |resources| {
        resources.disabled_uses.insert(ResourceUse::DataRelay);
    })
    .await;
    first_pong(
        &consumer,
        iroh::EndpointAddr::new(target.id).with_ip_addr(direct),
        iroh::TransportAddr::Ip(direct),
    )
    .await;
    consumer.close().await.unwrap();

    let verified = read_member_routes(&consumer_path, target.id);
    assert!(
        verified.iter().any(|route| {
            route["verified"] == true && route["address"]["Ip"] == direct.to_string()
        }),
        "the first PONG saved the actual verified address"
    );
    let consumer = reopen_with_resources(&consumer_path, |resources| {
        resources.disabled_uses.remove(&ResourceUse::DataRelay);
    })
    .await;
    assert_eq!(consumer.address().id, consumer_id);
    println!("redial-fixture verified-ip-wins");
    let selected = first_pong(
        &consumer,
        iroh::EndpointAddr::new(target.id),
        iroh::TransportAddr::Ip(direct),
    )
    .await;
    assert_eq!(
        first_pong(
            &consumer,
            iroh::EndpointAddr::new(target.id).with_relay_url(relay.clone()),
            iroh::TransportAddr::Ip(direct)
        )
        .await,
        selected
    );
    consumer.close().await.unwrap();
    provider.close().await.unwrap();

    // Keep the old port open without a QUIC endpoint while the same target
    // identity starts on a different port.
    let stale = UdpSocket::bind(direct).await.unwrap();
    let provider_path = root.path().join("provider");
    let provider = NativeClient::open(&provider_path, Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    assert_eq!(provider.address().id, target.id);
    let consumer = NativeClient::open(&consumer_path, Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    assert_eq!(consumer.address().id, consumer_id);
    println!("redial-fixture stale-ip");
    let mut packet = [0; 2048];
    let (_, observed) = tokio::join!(
        first_pong(
            &consumer,
            iroh::EndpointAddr::new(target.id),
            iroh::TransportAddr::Relay(relay.clone())
        ),
        timeout(IO, stale.recv_from(&mut packet)),
    );
    assert!(
        observed.unwrap().unwrap().0 >= 1200,
        "the stale UDP port saw the QUIC Initial"
    );
    drop(stale);
    consumer.close().await.unwrap();

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
    let consumer = NativeClient::open(&consumer_path, Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    println!("redial-fixture wrong-peer-ip");
    let (_, observed) = tokio::join!(
        first_pong(
            &consumer,
            iroh::EndpointAddr::new(target.id),
            iroh::TransportAddr::Relay(relay.clone())
        ),
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
    drop(wrong_connection);
    consumer.close().await.unwrap();
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
    let consumer = NativeClient::open(&consumer_path, Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    println!("redial-fixture reset-scram");
    let (_, observed) = tokio::join!(
        first_pong(
            &consumer,
            iroh::EndpointAddr::new(target.id),
            iroh::TransportAddr::Relay(relay.clone())
        ),
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
    consumer.close().await.unwrap();
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
        let consumer = reopen_with_resources(&consumer_path, |resources| {
            resources.udp_direct_disabled = !udp;
            if data_relay {
                resources.disabled_uses.remove(&ResourceUse::DataRelay);
            } else {
                resources.disabled_uses.insert(ResourceUse::DataRelay);
            }
        })
        .await;
        println!("redial-fixture {label}");
        first_pong(&consumer, iroh::EndpointAddr::new(target.id), expected).await;
        consumer.close().await.unwrap();
    }

    // The removed peer is stopped when the administrator applies the record,
    // so the fresh dial reaches the target's refusal rather than a local check.
    let peer = p2panda_core::VerifyingKey::from_bytes(consumer_id.as_bytes()).unwrap();
    let proposal = provider
        .propose_device_operation(DeviceAction::RemoveMember, peer)
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
    write_member_routes(&consumer_path, target.id, vec![recent(current, now, true)]);
    let consumer = reopen_with_resources(&consumer_path, |resources| {
        resources.udp_direct_disabled = false;
    })
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
    consumer.close().await.unwrap();
    provider.close().await.unwrap();
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
    assert_eq!(flow(consumer, Protocol::Tcp)["transport"], client_transport);
    assert_eq!(flow(provider, Protocol::Tcp)["transport"], server_transport);

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
            consumer.forward_exchange(
                contact,
                Protocol::Tcp,
                target_address,
                b"contact-only request",
                b"qualified data reply".len()
            ),
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
    assert_eq!(reply.unwrap(), b"qualified data reply");
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
    let relay = relay().await;
    let (url, stop) =
        delayed_relay::delay_relay(relay.http_addr().unwrap(), Duration::from_millis(350)).await;
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
    let reply = {
        let ping = timed_ping(&pair);
        tokio::pin!(ping);
        loop {
            tokio::select! {
                reply = &mut ping => break reply,
                _ = sleep(Duration::from_millis(50)) => {
                    let status = received_file_status(&pair.provider, &task).unwrap();
                    if status.bytes > peak {
                        peak = status.bytes;
                        advancements += 1;
                    }
                }
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
        "MIO145 high_rtt relay_leg_ms=350 deadline_ms=5000 solo={} loaded={} progress_start={} progress_end={} samples={advancements}",
        solo["timing"], reply["timing"], before.bytes, peak
    );
    pair.close().await;
    stop.cancel();
    relay.shutdown().await.unwrap();
}
