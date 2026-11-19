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
            resources: vec![Resource {
                url: relay_url.to_string(),
                uses: [ResourceUse::DataRelay].into(),
                limitation: "owned relay for native service streams".into(),
            }],
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
async fn only_configured_mqtt_contact_still_reaches_the_old_admitted_data_entry() {
    let broker = relay_contact_broker::Broker::new().await;
    let mut resources = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![Resource {
            url: format!("mqtt://{}/", broker.address),
            uses: [ResourceUse::Signaling].into(),
            limitation: "owned MQTT contact; payload requires a qualified UDP path".into(),
        }],
    });
    resources.tcp_direct_disabled = true;
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
    let consumer = NativeClient::open(root.path().join("consumer"), Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let mut contact = provider.invitation().await.unwrap().address().clone();
    contact.addrs.retain(|addr| {
        matches!(addr, iroh::TransportAddr::Custom(address)
        if address.id() == miopunch_native_probe::mqtt_transport::TRANSPORT_ID)
    });
    assert_eq!(
        contact.addrs.len(),
        1,
        "only approved MQTT contact is supplied"
    );
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
                let status = flow(&consumer, Protocol::Tcp);
                assert!(status["transport"]["stream"].is_null());
                assert!(
                    status["transport"]["paths"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|path| path["selected"] == true && path["remote"].get("Ip").is_some())
                );
                socket.write_all(b"qualified data reply").await.unwrap();
            },
        )
    })
    .await
    .expect("the existing MQTT to data entry remains usable");
    assert_eq!(reply.unwrap(), b"qualified data reply");
    consumer.close().await.unwrap();
    provider.close().await.unwrap();
    broker.close().await;
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
