//! Public Endpoint and configured-node boundaries for actual STUN datagrams.
use miopunch_native_probe::control_protocol::AuthenticatedControl;
use std::{collections::BTreeSet, net::Ipv4Addr, time::Duration};

use iroh::{Endpoint, endpoint::presets};
use miopunch_native_probe::{
    native_network::NativeNetwork,
    resources::{Resource, ResourceUse},
};
use tokio::net::UdpSocket;

fn response(request: &[u8], port: u16) -> Vec<u8> {
    let mut packet = vec![1, 1, 0, 12];
    packet.extend_from_slice(&request[4..20]);
    packet.extend_from_slice(&[0, 0x20, 0, 8, 0, 1]);
    packet.extend_from_slice(&(port ^ 0x2112).to_be_bytes());
    packet
        .extend_from_slice(&(u32::from(Ipv4Addr::new(203, 0, 113, 7)) ^ 0x2112a442).to_be_bytes());
    packet
}

#[test]
fn bundled_stun_is_observation_only_and_preserves_all_four_official_qad_nodes() {
    use miopunch_native_probe::resources::ResourceConfig;
    let mut config = ResourceConfig::default();
    let official = iroh::defaults::prod::default_relay_map();
    let qad = config.relay_map(ResourceUse::Observation).unwrap();
    assert_eq!(qad.urls::<BTreeSet<_>>(), official.urls::<BTreeSet<_>>());
    assert_eq!(qad.len(), 4);
    assert!(
        config.tcp_listen.is_none(),
        "TCP remains explicitly enabled only"
    );
    let udp = config.udp_stun_resources();
    assert_eq!(
        udp.len(),
        2,
        "usable multi-destination default backup requires two services"
    );
    assert_eq!(config.tcp_stun_resources().len(), 1);
    for url in udp.iter().chain(config.tcp_stun_resources().iter()) {
        assert!(!config.allows(url.as_str(), ResourceUse::DataRelay));
        assert!(!config.allows(url.as_str(), ResourceUse::Signaling));
    }
    config.disabled_uses.insert(ResourceUse::Observation);
    assert!(config.udp_stun_resources().is_empty());
    assert!(config.tcp_stun_resources().is_empty());
    config.disabled_uses.clear();
    config.import_trusted_replacement(Vec::new()).unwrap();
    assert!(
        config.udp_stun_resources().is_empty(),
        "replacement does not leak bundled services"
    );
    assert!(config.tcp_stun_resources().is_empty());
}

#[tokio::test]
async fn stun_uses_the_endpoint_socket_and_rejects_wrong_source_and_transaction() {
    let server = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let address = server.local_addr().unwrap();
    let endpoint = Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .bind()
        .await
        .unwrap();
    let bound = endpoint.bound_sockets();
    let task = tokio::spawn(async move {
        let mut request = [0; 128];
        let (len, peer) = server.recv_from(&mut request).await.unwrap();
        assert_eq!(len, 20);
        assert_eq!(&request[..8], &[0, 1, 0, 0, 0x21, 0x12, 0xa4, 0x42]);
        assert!(
            bound.contains(&peer),
            "query must come from retained Endpoint binding"
        );
        let wrong_source = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        wrong_source
            .send_to(&response(&request, 49999), peer)
            .await
            .unwrap();
        let mut wrong_transaction = response(&request, 49998);
        wrong_transaction[8] ^= 1;
        server.send_to(&wrong_transaction, peer).await.unwrap();
        tokio::time::sleep(Duration::from_millis(40)).await;
        server
            .send_to(&response(&request, 41000), peer)
            .await
            .unwrap();
        peer
    });
    let observation = endpoint
        .observe_udp_stun(&format!("stun://{address}/").parse().unwrap())
        .await
        .unwrap();
    assert_eq!(
        observation.mapped,
        "203.0.113.7:41000".parse::<std::net::SocketAddr>().unwrap()
    );
    assert_eq!(observation.destination, address);
    assert_eq!(observation.local_addr, task.await.unwrap());
    assert!(observation.is_fresh());
    endpoint.close().await;
    assert!(
        !observation.is_fresh(),
        "closing the owning socket invalidates its mapping"
    );
}

#[tokio::test]
async fn ordered_stun_sequence_uses_one_data_socket_and_failed_new_query_invalidates_old_snapshot()
{
    let endpoint = Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .bind()
        .await
        .unwrap();
    let mut observers = Vec::new();
    let mut tasks = Vec::new();
    for port in [41000, 41001] {
        let server = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let destination = server.local_addr().unwrap();
        observers.push(format!("stun://{destination}/").parse().unwrap());
        let bindings = endpoint.bound_sockets();
        tasks.push(tokio::spawn(async move {
            for _ in 0..2 {
                let mut request = [0; 128];
                let (len, peer) = server.recv_from(&mut request).await.unwrap();
                assert_eq!(len, 20);
                assert!(
                    bindings.contains(&peer),
                    "each sample uses the actual data binding"
                );
                server
                    .send_to(&response(&request, port), peer)
                    .await
                    .unwrap();
            }
        }));
    }
    let first = endpoint
        .observe_udp_stun_mapping_sequence(&observers)
        .await
        .unwrap();
    assert_eq!(first.samples.len(), 2);
    assert_eq!(
        first.samples[0].mapped,
        "203.0.113.7:41000".parse::<std::net::SocketAddr>().unwrap()
    );
    assert_eq!(
        first.samples[1].mapped,
        "203.0.113.7:41001".parse::<std::net::SocketAddr>().unwrap()
    );
    assert_ne!(first.samples[0].destination, first.samples[1].destination);
    assert!(first.samples[0].observed_at <= first.samples[1].observed_at);
    let snapshot =
        miopunch_native_probe::nat_punch::MappingSnapshot::from_sequence(first.clone()).unwrap();
    assert!(first.is_fresh() && snapshot.is_current());
    let unavailable = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let url = format!("stun://{}/", unavailable.local_addr().unwrap())
        .parse()
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), endpoint.observe_udp_stun(&url))
            .await
            .is_err()
    );
    assert!(
        !first.is_fresh() && !snapshot.is_current(),
        "a failed/canceled replacement cannot revive old mapping"
    );
    let fresh = endpoint
        .observe_udp_stun_mapping_sequence(&observers)
        .await
        .unwrap();
    assert!(fresh.is_fresh() && fresh.is_current_generation());
    for task in tasks {
        task.await.unwrap();
    }
    endpoint.close().await;
    assert!(!fresh.is_fresh() && !fresh.is_current_generation());
}

#[tokio::test]
async fn dns_names_that_share_one_udp_destination_cannot_manufacture_a_mapping_sequence() {
    let endpoint = Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .bind()
        .await
        .unwrap();
    let server = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = server.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        for _ in 0..2 {
            let mut request = [0; 128];
            let (_, peer) = server.recv_from(&mut request).await.unwrap();
            server
                .send_to(&response(&request, 41000), peer)
                .await
                .unwrap();
        }
    });
    let observers = [
        format!("stun://localhost:{port}/").parse().unwrap(),
        format!("stun://127.0.0.1:{port}/").parse().unwrap(),
    ];
    let error = endpoint
        .observe_udp_stun_mapping_sequence(&observers)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("insufficient"));
    task.await.unwrap();
    endpoint.close().await;
}

#[tokio::test]
async fn configured_stun_is_not_a_qad_relay_and_disabled_observation_sends_nothing() {
    let server = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let url: iroh::RelayUrl = format!("stun://{}/", server.local_addr().unwrap())
        .parse()
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let node = NativeNetwork::create(directory.path().join("node"))
        .await
        .unwrap();
    let mut config = node.resource_configuration().unwrap();
    config
        .import_trusted_replacement(vec![Resource {
            url: url.to_string(),
            uses: BTreeSet::from([ResourceUse::Observation]),
            limitation: "test STUN only".into(),
        }])
        .unwrap();
    assert!(
        config
            .relay_map(ResourceUse::Observation)
            .unwrap()
            .is_empty(),
        "STUN must never be passed to QAD"
    );
    config.disabled_uses.insert(ResourceUse::Observation);
    node.save_resource_configuration(&config).unwrap();
    let resources = node
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    assert_eq!(
        resources
            .endpoint()
            .observe_udp_stun(&url)
            .await
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), server.recv_from(&mut [0; 2048]))
            .await
            .is_err()
    );
    resources.close().await;
    config.disabled_uses.clear();
    config.udp_direct_disabled = true;
    node.save_resource_configuration(&config).unwrap();
    let resources = node
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    assert!(resources.endpoint().bound_sockets().is_empty());
    assert_eq!(
        resources
            .endpoint()
            .observe_udp_stun(&url)
            .await
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::Unsupported,
        "a missing UDP transport is a local prerequisite failure, not public STUN availability"
    );
    resources.close().await;
    node.close().await;
}

#[tokio::test]
async fn tcp_stun_retains_the_configured_transport_port_and_expires_on_close() {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    let server = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let destination = server.local_addr().unwrap();
    let url: iroh::RelayUrl = format!("stun+tcp://{destination}/").parse().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let node = NativeNetwork::create(directory.path().join("node"))
        .await
        .unwrap();
    let mut config = node.resource_configuration().unwrap();
    config
        .import_trusted_replacement(vec![Resource {
            url: url.to_string(),
            uses: BTreeSet::from([ResourceUse::Observation]),
            limitation: "test TCP STUN".into(),
        }])
        .unwrap();
    config.tcp_listen = Some((Ipv4Addr::LOCALHOST, 0).into());
    node.save_resource_configuration(&config).unwrap();
    let resources = node
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, source) = server.accept().await.unwrap();
        let mut request = [0; 20];
        stream.read_exact(&mut request).await.unwrap();
        stream.write_all(&response(&request, 41001)).await.unwrap();
        (stream, source)
    });
    let observation = resources.observe_tcp_stun(&url).await.unwrap();
    let (stream, peer) = task.await.unwrap();
    assert_eq!(peer, observation.local_addr);
    assert_eq!(
        observation.listener.port(),
        peer.port(),
        "no temporary ephemeral port may impersonate the transport"
    );
    assert_eq!(observation.destination, destination);
    assert_eq!(
        observation.mapped,
        "203.0.113.7:41001".parse::<std::net::SocketAddr>().unwrap()
    );
    assert!(observation.is_fresh());
    resources.close().await;
    assert!(!observation.is_fresh());
    drop(stream);
    node.close().await;
}

#[tokio::test]
async fn closing_the_endpoint_retires_a_retained_tcp_observation_even_while_resource_wrapper_lives()
{
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    let server = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let url: iroh::RelayUrl = format!("stun+tcp://{}/", server.local_addr().unwrap())
        .parse()
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let node = NativeNetwork::create(directory.path().join("node"))
        .await
        .unwrap();
    let mut config = node.resource_configuration().unwrap();
    config.replacement = Some(vec![Resource {
        url: url.to_string(),
        uses: BTreeSet::from([ResourceUse::Observation]),
        limitation: "owned endpoint-lifetime observer".into(),
    }]);
    config.tcp_listen = Some((Ipv4Addr::LOCALHOST, 0).into());
    node.save_resource_configuration(&config).unwrap();
    let resources = node
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let (observation, mut stream) = tokio::join!(resources.observe_tcp_stun(&url), async {
        let (mut stream, _) = server.accept().await.unwrap();
        let mut request = [0; 20];
        stream.read_exact(&mut request).await.unwrap();
        stream.write_all(&response(&request, 41000)).await.unwrap();
        stream
    });
    let observation = observation.unwrap();
    assert!(observation.is_fresh());
    resources.endpoint().close().await;
    assert!(!observation.is_fresh());
    let eof = tokio::time::timeout(Duration::from_millis(300), stream.read(&mut [0; 1])).await;
    assert!(
        matches!(eof, Ok(Ok(0))),
        "raw Endpoint close must release the actual retained observer socket"
    );
    resources.close().await;
    node.close().await;
}

#[tokio::test]
async fn a_new_tcp_stun_attempt_invalidates_an_older_pending_query() {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    let server = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let url: iroh::RelayUrl = format!("stun+tcp://{}/", server.local_addr().unwrap())
        .parse()
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let node = NativeNetwork::create(directory.path().join("node"))
        .await
        .unwrap();
    let mut config = node.resource_configuration().unwrap();
    config.replacement = Some(vec![Resource {
        url: url.to_string(),
        uses: BTreeSet::from([ResourceUse::Observation]),
        limitation: "supersession test".into(),
    }]);
    config.tcp_listen = Some((Ipv4Addr::LOCALHOST, 0).into());
    node.save_resource_configuration(&config).unwrap();
    let resources = node
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let (older, stream) = tokio::join!(resources.observe_tcp_stun(&url), async {
        let (mut stream, _) = server.accept().await.unwrap();
        let mut request = [0; 20];
        stream.read_exact(&mut request).await.unwrap();
        // The second request must start before the first response is sent.
        // It may fail to reuse the same TCP four-tuple or be cancelled while
        // awaiting its response; either way the older query is superseded.
        let _ = tokio::time::timeout(Duration::from_millis(100), resources.observe_tcp_stun(&url))
            .await;
        let _ = stream.write_all(&response(&request, 41002)).await;
        stream
    });
    assert!(
        older.is_err(),
        "a superseded in-flight query must not publish a fresh observation"
    );
    drop(stream);
    resources.close().await;
    node.close().await;
}

#[tokio::test]
async fn tcp_stun_accepts_ipv6_mapping_of_the_actual_ipv4_transport_connection() {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    let server = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let url: iroh::RelayUrl = format!("stun+tcp://{}/", server.local_addr().unwrap())
        .parse()
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let node = NativeNetwork::create(directory.path().join("node"))
        .await
        .unwrap();
    let mut config = node.resource_configuration().unwrap();
    config
        .import_trusted_replacement(vec![Resource {
            url: url.to_string(),
            uses: BTreeSet::from([ResourceUse::Observation]),
            limitation: "IPv6 mapping from IPv4 TCP observer".into(),
        }])
        .unwrap();
    config.tcp_listen = Some((Ipv4Addr::LOCALHOST, 0).into());
    node.save_resource_configuration(&config).unwrap();
    let resources = node
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = server.accept().await.unwrap();
        let mut request = [0; 20];
        stream.read_exact(&mut request).await.unwrap();
        let mut packet = vec![1, 1, 0, 24];
        packet.extend_from_slice(&request[4..]);
        packet.extend_from_slice(&[0, 0x20, 0, 20, 0, 2]);
        packet.extend_from_slice(&(41001u16 ^ 0x2112).to_be_bytes());
        let address: std::net::Ipv6Addr = "2001:db8::7".parse().unwrap();
        packet.extend(
            address
                .octets()
                .into_iter()
                .zip(request[4..20].iter())
                .map(|(byte, mask)| byte ^ mask),
        );
        stream.write_all(&packet).await.unwrap();
        stream
    });
    let observation = resources.observe_tcp_stun(&url).await.unwrap();
    assert_eq!(
        observation.mapped,
        "[2001:db8::7]:41001"
            .parse::<std::net::SocketAddr>()
            .unwrap()
    );
    assert!(observation.local_addr.is_ipv4());
    let stream = task.await.unwrap();
    resources.close().await;
    drop(stream);
    node.close().await;
}

#[tokio::test]
async fn locally_closed_paths_are_absent_from_new_public_snapshots_before_abandonment() {
    use futures_util::StreamExt;
    let mut config = iroh_relay::server::ServerConfig::default();
    config.relay = Some(iroh_relay::server::RelayConfig::new((
        Ipv4Addr::LOCALHOST,
        0,
    )));
    let relay = iroh_relay::server::Server::spawn(config).await.unwrap();
    let relay_map = iroh::RelayMap::from_iter([relay.http_url().unwrap()]);
    let server = Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .relay_mode(iroh::RelayMode::Custom(relay_map.clone()))
        .alpns(vec![b"snapshot-test".to_vec()])
        .bind()
        .await
        .unwrap();
    let client = Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .relay_mode(iroh::RelayMode::Custom(relay_map))
        .bind()
        .await
        .unwrap();
    server.online().await;
    let (out, incoming) = tokio::join!(client.connect(server.addr(), b"snapshot-test"), async {
        server.accept().await.unwrap().await
    });
    let out = out.unwrap();
    let incoming = incoming.unwrap();
    let captured = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let paths = out.paths();
            if paths.iter().any(|path| path.is_relay()) && paths.iter().any(|path| path.is_ip()) {
                break paths;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let path = captured.iter().find(|path| path.is_relay()).unwrap();
    let closed = path.id();
    path.close().unwrap();
    // No await between close and capture: the actor cannot process Abandoned.
    assert!(
        out.paths().get(closed).is_none(),
        "a released handle must not advertise closed signaling paths"
    );
    assert!(
        out.paths_stream()
            .next()
            .await
            .unwrap()
            .get(closed)
            .is_none()
    );
    assert!(
        captured.get(closed).is_some(),
        "old snapshots retain their entries and final statistics"
    );
    captured.get(closed).unwrap().stats();
    out.close(0u32.into(), b"done");
    incoming.close(0u32.into(), b"done");
    client.close().await;
    server.close().await;
    relay.shutdown().await.unwrap();
}

#[tokio::test]
async fn mqtt_bootstrap_can_release_only_the_original_connection_over_an_allowed_relay() {
    use miopunch_native_probe::{
        admission_gate::{ADMISSION_REQUEST_ALPN, ProtocolScope},
        manual_admission::ManualExchange,
    };
    let Some(broker) = std::env::var("MIOPUNCH_MQTT_BROKER").ok() else {
        return;
    };
    let mut server_config = iroh_relay::server::ServerConfig::default();
    server_config.relay = Some(iroh_relay::server::RelayConfig::new((
        Ipv4Addr::LOCALHOST,
        0,
    )));
    let relay = iroh_relay::server::Server::spawn(server_config)
        .await
        .unwrap();
    let relay_url = relay.http_url().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(directory.path().join("admin"))
        .await
        .unwrap();
    let mut member = NativeNetwork::join(
        directory.path().join("member"),
        admin.manual_invitation().unwrap(),
    )
    .await
    .unwrap();
    for node in [&admin, &member] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(vec![Resource {
            url: relay_url.to_string(),
            uses: BTreeSet::from([ResourceUse::DataRelay]),
            limitation: "test relay".into(),
        }]);
        config.udp_direct_disabled = true;
        node.save_resource_configuration(&config).unwrap();
    }
    let a = admin
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let b = member
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    for approved in [false, true] {
        let (out, incoming) =
            tokio::join!(b.connect_control(a.addr(), ADMISSION_REQUEST_ALPN), async {
                a.endpoint()
                    .accept()
                    .await
                    .unwrap()
                    .accept_control()
                    .unwrap()
                    .await
            });
        let out = out.unwrap();
        let incoming = std::sync::Arc::new(incoming.unwrap());
        let receiving = async {
            let materials = member.request_manual_admission(&out).await.unwrap();
            if approved {
                let ManualExchange::Materials(materials) = materials else {
                    panic!("approved materials")
                };
                member.complete_manual_admission(materials).await.unwrap();
            } else {
                assert!(matches!(materials, ManualExchange::AwaitingApproval));
            }
        };
        let (_, served) = tokio::join!(receiving, admin.serve_manual_admission(&incoming));
        served.unwrap();
        if !approved {
            admin
                .approve_manual_admission(&admin.manual_requests().await.unwrap()[0])
                .await
                .unwrap();
        }
        out.close(0u32.into(), b"admission complete");
        incoming.close(0u32.into(), b"admission complete");
    }
    a.close().await;
    b.close().await;
    const BUSINESS: &[u8] = miopunch_native_probe::connection_coordinator::MEMBER_ALPN;
    for node in [&admin, &member] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement.as_mut().unwrap().push(Resource {
            url: format!("mqtt://{broker}/"),
            uses: BTreeSet::from([ResourceUse::Signaling]),
            limitation: "test signaling".into(),
        });
        node.save_resource_configuration(&config).unwrap();
        node.admission_gate()
            .register_protocol(BUSINESS, ProtocolScope::Member);
    }
    let a = admin
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let b = member
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    a.endpoint().set_alpns(vec![BUSINESS.to_vec()]);
    let mqtt = a
        .endpoint()
        .custom_addrs()
        .into_iter()
        .find(|address| address.id() == miopunch_native_probe::mqtt_transport::TRANSPORT_ID)
        .unwrap();
    let target =
        iroh::EndpointAddr::new(a.endpoint().id()).with_addrs([iroh::TransportAddr::Custom(mqtt)]);
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        let (out, incoming) = tokio::join!(b.endpoint().connect(target, BUSINESS), async { a.endpoint().accept().await.unwrap().await });
        let out = out.unwrap(); let incoming = incoming.unwrap();
        assert!(out.paths().iter().any(|p| p.is_relay()));
        assert!(out.paths().iter().all(|p| !matches!(p.remote_addr(), iroh::TransportAddr::Custom(address) if address.id() == miopunch_native_probe::mqtt_transport::TRANSPORT_ID)));
        let id = out.stable_id();
        let sender = async { let (mut send, mut recv) = out.open_bi().await.unwrap(); send.write_all(b"public path").await.unwrap(); send.finish().unwrap(); assert_eq!(recv.read_to_end(64).await.unwrap(), b"ACK"); };
        let receiver = async { let (mut send, mut recv) = incoming.accept_bi().await.unwrap(); assert_eq!(recv.read_to_end(64).await.unwrap(), b"public path"); send.write_all(b"ACK").await.unwrap(); send.finish().unwrap(); };
        tokio::join!(sender, receiver);
        assert_eq!(out.stable_id(), id);
        out.close(0u32.into(), b"done"); incoming.close(0u32.into(), b"done");
    }).await;
    a.close().await;
    b.close().await;
    admin.close().await;
    member.close().await;
    relay.shutdown().await.unwrap();
    result.expect("bounded MQTT-to-relay data release");
}
