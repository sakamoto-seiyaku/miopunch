//! Actual native observation sockets, resource selection and hostname verification.
use iroh::{
    Endpoint, RelayConfig, RelayMap,
    endpoint::{ObservationDialAddresses, ObservationFamily, presets},
};
use iroh_relay::RelayQuicConfig;
use miopunch_native_probe::{
    native_network::NativeNetwork,
    resources::{Resource, ResourceUse},
};
use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, UdpSocket},
    sync::mpsc,
    task::JoinSet,
};

fn resource(url: String) -> Resource {
    Resource {
        url,
        uses: [ResourceUse::Observation].into(),
        limitation: "owned observation station".into(),
    }
}
fn response(request: &[u8], source: SocketAddr) -> Vec<u8> {
    assert_eq!(&request[..8], &[0, 1, 0, 0, 0x21, 0x12, 0xa4, 0x42]);
    let size: u16 = if source.is_ipv4() { 8 } else { 20 };
    let mut reply = vec![1, 1];
    reply.extend_from_slice(&(size + 4).to_be_bytes());
    reply.extend_from_slice(&request[4..20]);
    reply.extend_from_slice(&[0, 0x20]);
    reply.extend_from_slice(&size.to_be_bytes());
    reply.extend_from_slice(&[0, if source.is_ipv4() { 1 } else { 2 }]);
    reply.extend_from_slice(&(source.port() ^ 0x2112).to_be_bytes());
    let bytes = match source.ip() {
        IpAddr::V4(ip) => ip.octets().to_vec(),
        IpAddr::V6(ip) => ip.octets().to_vec(),
    };
    for (index, byte) in bytes.iter().enumerate() {
        reply.push(byte ^ request[4 + index]);
    }
    reply
}
async fn udp_observer(
    bind: SocketAddr,
    tasks: &mut JoinSet<()>,
) -> (
    SocketAddr,
    mpsc::UnboundedReceiver<SocketAddr>,
    Arc<AtomicUsize>,
) {
    let socket = UdpSocket::bind(bind).await.unwrap();
    let address = socket.local_addr().unwrap();
    let (send, receive) = mpsc::unbounded_channel();
    let count = Arc::new(AtomicUsize::new(0));
    let received = count.clone();
    tasks.spawn(async move {
        loop {
            let mut request = [0; 1500];
            let (length, source) = socket.recv_from(&mut request).await.unwrap();
            assert_eq!(length, 20);
            received.fetch_add(1, Ordering::Relaxed);
            let _ = send.send(source);
            socket
                .send_to(&response(&request[..length], source), source)
                .await
                .unwrap();
        }
    });
    (address, receive, count)
}
fn builder() -> iroh::endpoint::Builder {
    Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .bind_addr((Ipv6Addr::LOCALHOST, 0))
        .unwrap()
}

#[tokio::test]
async fn native_stun_uses_each_actual_binding_and_refills_to_the_fifth_station() {
    let root = tempfile::tempdir().unwrap();
    let node = NativeNetwork::create(root.path().join("node"))
        .await
        .unwrap();
    let mut tasks = JoinSet::new();
    let (v4, mut received_v4, count_v4) =
        udp_observer((Ipv4Addr::LOCALHOST, 0).into(), &mut tasks).await;
    let (v6, mut received_v6, _) = udp_observer((Ipv6Addr::LOCALHOST, 0).into(), &mut tasks).await;
    let origin_v4: iroh::RelayUrl = format!("stun://localhost:{}/", v4.port()).parse().unwrap();
    let origin_v6: iroh::RelayUrl = format!("stun://localhost:{}/", v6.port()).parse().unwrap();
    let alias: iroh::RelayUrl = format!("stun://127.0.0.1:{}/", v4.port()).parse().unwrap();
    // Four owned stations receive the requests without replying. The fifth
    // station must get a turn within the same three-second absolute deadline.
    let mut failures = Vec::new();
    let mut silent_stations = Vec::new();
    for _ in 0..4 {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        failures.push(
            format!("stun://{}/", socket.local_addr().unwrap())
                .parse::<iroh::RelayUrl>()
                .unwrap(),
        );
        silent_stations.push(socket);
    }
    let mut config = node.resource_configuration().unwrap();
    config.replacement = Some(
        failures
            .iter()
            .chain([&origin_v4, &origin_v6, &alias])
            .map(|url| resource(url.to_string()))
            .collect(),
    );
    config.tcp_direct_disabled = true;
    config.observation_addresses.insert(
        origin_v4.to_string(),
        ObservationDialAddresses {
            ipv4: None,
            ipv6: Some(Ipv6Addr::LOCALHOST),
        },
    );
    config.observation_addresses.insert(
        origin_v6.to_string(),
        ObservationDialAddresses {
            ipv4: Some(Ipv4Addr::LOCALHOST),
            ipv6: None,
        },
    );
    node.save_resource_configuration(&config).unwrap();
    assert!(config.udp_stun_resources().len() >= 5);
    let endpoint = node.bind_configured_endpoint_with(builder()).await.unwrap();
    let raw = endpoint.endpoint();
    // Both requests coexist. An observation of the other family must not supersede either result.
    let (v4_result, v6_result) = tokio::join!(
        raw.observe_udp_stun_for_family(&origin_v4, ObservationFamily::Ipv4),
        raw.observe_udp_stun_for_family(&origin_v6, ObservationFamily::Ipv6)
    );
    let (v4_result, v6_result) = (v4_result.unwrap(), v6_result.unwrap());
    assert_eq!(v4_result.mapped, received_v4.recv().await.unwrap());
    assert_eq!(v6_result.mapped, received_v6.recv().await.unwrap());
    assert_eq!(v4_result.destination, v4);
    assert_eq!(v6_result.destination, v6);
    assert_eq!(v4_result.local_addr, v4_result.mapped);
    assert_eq!(v6_result.local_addr, v6_result.mapped);
    assert!(v4_result.is_fresh() && v6_result.is_fresh());
    failures.push(origin_v4.clone());
    let before = count_v4.load(Ordering::Relaxed);
    let started = tokio::time::Instant::now();
    let sequence = tokio::time::timeout(
        Duration::from_secs(7),
        raw.observe_udp_mapping_round_for_family_until(
            &RelayMap::empty(),
            &failures,
            ObservationFamily::Ipv4,
            tokio::time::Instant::now() + Duration::from_secs(3),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(started.elapsed() < Duration::from_secs(3));
    for socket in &silent_stations { assert!(socket.try_recv_from(&mut [0; 1500]).is_ok()); }
    assert_eq!(sequence.samples.len(), 1);
    assert_eq!(sequence.samples[0].destination, v4);
    assert_eq!(sequence.samples[0].mapped, v4_result.mapped);
    assert!(count_v4.load(Ordering::Relaxed) > before);
    let before = count_v4.load(Ordering::Relaxed);
    let dedup = raw
        .observe_udp_mapping_round_for_family(
            &RelayMap::empty(),
            &[origin_v4.clone(), alias],
            ObservationFamily::Ipv4,
        )
        .await
        .unwrap();
    assert_eq!(dedup.samples.len(), 1);
    assert_eq!(count_v4.load(Ordering::Relaxed) - before, 1);
    let denied: iroh::RelayUrl = "stun://127.0.0.1:9/".parse().unwrap();
    assert_eq!(
        raw.observe_udp_stun(&denied).await.unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    eprintln!(
        "NATIVE_OBSERVATION_STUN v4={v4_result:?} v6={v6_result:?} fifth={:?} alias_requests=1",
        sequence.samples
    );
    endpoint.close().await;
    node.close().await;
    tasks.shutdown().await;
}

#[tokio::test]
async fn native_tcp_stun_uses_the_corresponding_listener_and_per_family_dns_rule() {
    let root = tempfile::tempdir().unwrap();
    let node = NativeNetwork::create(root.path().join("node"))
        .await
        .unwrap();
    let mut tasks = JoinSet::new();
    let mut origins = Vec::new();
    let mut sources = Vec::new();
    for bind in [
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        SocketAddr::from((Ipv6Addr::LOCALHOST, 0)),
    ] {
        let listener = TcpListener::bind(bind).await.unwrap();
        let destination = listener.local_addr().unwrap();
        let (send, receive) = mpsc::unbounded_channel();
        tasks.spawn(async move {
            loop {
                let (mut stream, source) = listener.accept().await.unwrap();
                let mut request = [0; 20];
                stream.read_exact(&mut request).await.unwrap();
                stream.write_all(&response(&request, source)).await.unwrap();
                send.send((source, destination)).unwrap();
                // Preserve the actual observation socket as required by freshness.
                tokio::spawn(async move {
                    let _ = stream.read_u8().await;
                });
            }
        });
        origins.push(
            format!("stun+tcp://localhost:{}/", destination.port())
                .parse::<iroh::RelayUrl>()
                .unwrap(),
        );
        sources.push(receive);
    }
    let mut config = node.resource_configuration().unwrap();
    config.replacement = Some(
        origins
            .iter()
            .map(|url| resource(url.to_string()))
            .collect(),
    );
    config.observation_addresses.insert(
        origins[0].to_string(),
        ObservationDialAddresses {
            ipv4: None,
            ipv6: Some(Ipv6Addr::LOCALHOST),
        },
    );
    config.observation_addresses.insert(
        origins[1].to_string(),
        ObservationDialAddresses {
            ipv4: Some(Ipv4Addr::LOCALHOST),
            ipv6: None,
        },
    );
    node.save_resource_configuration(&config).unwrap();
    let endpoint = node.bind_configured_endpoint_with(builder()).await.unwrap();
    for (index, family) in [ObservationFamily::Ipv4, ObservationFamily::Ipv6]
        .into_iter()
        .enumerate()
    {
        let observation = endpoint
            .observe_tcp_stun_for_family(&origins[index], family)
            .await
            .unwrap();
        let (source, destination) = sources[index].recv().await.unwrap();
        assert_eq!(observation.destination, destination);
        assert_eq!(observation.local_addr, source);
        assert_eq!(observation.listener.port(), source.port());
        assert!(family.matches(observation.listener));
        assert_eq!(observation.mapped, source);
        assert!(observation.is_fresh());
        eprintln!(
            "NATIVE_OBSERVATION_TCP family={family:?} listener={} source={source} destination={destination}",
            observation.listener
        );
    }
    endpoint.close().await;
    node.close().await;
    tasks.shutdown().await;
}

#[tokio::test]
async fn configured_qad_keeps_hostname_verification_and_the_actual_family_binding() {
    use iroh::Watcher;
    iroh_relay::client::enable_relay_diagnostics();
    use iroh_relay::{server, tls::CaTlsConfig};
    for (bind, family) in [
        (
            SocketAddr::from((Ipv4Addr::LOCALHOST, 7842)),
            ObservationFamily::Ipv4,
        ),
        (
            SocketAddr::from((Ipv6Addr::LOCALHOST, 7842)),
            ObservationFamily::Ipv6,
        ),
    ] {
        let (certs, tls) = server::testing::self_signed_tls_certs_and_config();
        let mut quic = server::QuicConfig::new(bind);
        quic.server_config = Some(tls);
        let mut server_config = server::ServerConfig::default();
        server_config.quic = Some(quic);
        let server = server::Server::spawn(server_config).await.unwrap();
        let root = tempfile::tempdir().unwrap();
        let node = NativeNetwork::create(root.path().join("node"))
            .await
            .unwrap();
        let origin: iroh::RelayUrl = "https://localhost/".parse().unwrap();
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(vec![
            resource(origin.to_string()),
            resource("https://wrong.invalid/".into()),
        ]);
        config.tcp_direct_disabled = true;
        let dial = match family {
            ObservationFamily::Ipv4 => ObservationDialAddresses {
                ipv4: Some(Ipv4Addr::LOCALHOST),
                ipv6: None,
            },
            ObservationFamily::Ipv6 => ObservationDialAddresses {
                ipv4: None,
                ipv6: Some(Ipv6Addr::LOCALHOST),
            },
        };
        config
            .observation_addresses
            .insert(origin.to_string(), dial.clone());
        config
            .observation_addresses
            .insert("https://wrong.invalid/".into(), dial);
        node.save_resource_configuration(&config).unwrap();
        let builder = Endpoint::builder(presets::Minimal)
            .clear_ip_transports()
            .bind_addr((bind.ip(), 0))
            .unwrap()
            .ca_tls_config(CaTlsConfig::custom_roots(certs));
        let endpoint = node.bind_configured_endpoint_with(builder).await.unwrap();
        eprintln!(
            "QAD_CONFIG {:?} bind={bind} locals={:?}",
            config.relay_map(ResourceUse::Observation).unwrap(),
            endpoint.endpoint().bound_sockets()
        );
        let mut observation = endpoint.endpoint().udp_observation();
        let state = tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                let state = observation.get();
                if state.is_fresh() {
                    break state;
                }
                observation.updated().await.unwrap();
            }
        })
        .await
        .unwrap_or_else(|error| {
            panic!(
                "periodic QAD {bind}: {error:?}, state={:?}",
                observation.get()
            )
        });
        let sequence = endpoint
            .endpoint()
            .observe_udp_mapping_round_for_family(
                &config.relay_map(ResourceUse::Observation).unwrap(),
                &[],
                family,
            )
            .await
            .unwrap();
        assert_eq!(sequence.samples.len(), 1);
        assert_eq!(sequence.samples[0].observer, origin.to_string());
        assert_eq!(sequence.samples[0].destination, bind);
        let local = endpoint
            .endpoint()
            .bound_sockets()
            .into_iter()
            .find(|addr| family.matches(*addr))
            .unwrap();
        assert_eq!(sequence.samples[0].mapped, local);
        let mut invalid = RelayConfig::new(
            "https://wrong.invalid/".parse().unwrap(),
            Some(RelayQuicConfig::new(7842)),
        );
        invalid.quic.as_mut().unwrap().dial_addresses =
            config.observation_addresses["https://wrong.invalid/"].clone();
        assert!(
            endpoint
                .endpoint()
                .observe_udp_mapping_round_for_family(&RelayMap::from(invalid), &[], family)
                .await
                .is_err()
        );
        eprintln!(
            "NATIVE_OBSERVATION_QAD family={family:?} periodic={state:?} sample={:?} wrong_name_rejected=true",
            sequence.samples
        );
        endpoint.close().await;
        node.close().await;
        server.shutdown().await.unwrap();
    }
}
