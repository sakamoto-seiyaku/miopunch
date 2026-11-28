use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use iroh::{
    Endpoint, RelayConfig, RelayMap, Watcher as _,
    endpoint::{BindError, RelayMode, UdpObservation, UdpObservationFailure, presets},
};
use iroh_relay::{RelayQuicConfig, server, tls::CaTlsConfig};
use tokio::{net::UdpSocket, task::JoinHandle};

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "requires an isolated network namespace with two disposable interfaces"]
async fn restoring_the_gateway_revalidates_the_same_ip_on_a_replacement_interface() {
    fn ip(args: &[&str]) {
        assert!(std::process::Command::new("ip").args(args).status().unwrap().success());
    }
    let address = Ipv4Addr::new(192, 0, 2, 205);
    for name in ["old205", "new205"] {
        ip(&["link", "add", name, "type", "dummy"]);
        ip(&["link", "set", name, "up"]);
    }
    ip(&["addr", "add", "192.0.2.205/32", "dev", "old205"]);
    ip(&["route", "add", "default", "dev", "old205"]);
    let bind = || Endpoint::builder(presets::Minimal)
        .alpns(vec![b"gateway-change".to_vec()])
        .clear_ip_transports().bind_addr((address, 0)).unwrap().bind();
    let (local, remote) = tokio::try_join!(bind(), bind()).unwrap();
    let (client, server) = tokio::join!(
        local.connect(remote.addr(), b"gateway-change"),
        async { remote.accept().await.unwrap().await.unwrap() }
    );
    let client = client.unwrap();
    let bound = local.bound_sockets()[0];
    let previous = local.binding_network_generation(bound).unwrap();
    ip(&["link", "del", "old205"]);
    ip(&["addr", "add", "192.0.2.205/32", "dev", "new205"]);
    tokio::time::timeout(Duration::from_secs(2), async {
        while previous.is_current() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("the socket actor has processed the replacement before the gateway returns");
    let retired = || {
        let frames = client.stats().frame_tx;
        frames.retire_connection_id + frames.path_retire_connection_id
    };
    let before = retired();
    ip(&["route", "add", "default", "dev", "new205"]);
    tokio::time::timeout(Duration::from_secs(2), async {
        while retired() == before {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("QUIC refreshed the original path CID despite the same IP on gateway restoration");
    assert!(server.close_reason().is_none());
    tokio::join!(local.close(), remote.close());
}

struct NatProxy {
    ingress_addr: SocketAddr,
    mapped_addr: SocketAddr,
    endpoint_source: Arc<Mutex<Option<SocketAddr>>>,
    packets: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl NatProxy {
    async fn start(observer_addr: SocketAddr) -> Self {
        Self::start_on(observer_addr, (Ipv4Addr::LOCALHOST, 0).into()).await
    }

    async fn start_on(observer_addr: SocketAddr, bind: SocketAddr) -> Self {
        Self::start_on_with_delay(observer_addr, bind, None).await
    }

    async fn start_on_with_delay(
        observer_addr: SocketAddr,
        bind: SocketAddr,
        mut first_packet_delay: Option<Duration>,
    ) -> Self {
        let ingress = UdpSocket::bind(bind).await.unwrap();
        let egress = UdpSocket::bind(if observer_addr.is_ipv6() {
            SocketAddr::from((std::net::Ipv6Addr::LOCALHOST, 0))
        } else { SocketAddr::from((Ipv4Addr::LOCALHOST, 0)) }).await.unwrap();
        let ingress_addr = ingress.local_addr().unwrap();
        let mapped_addr = egress.local_addr().unwrap();
        let endpoint_source = Arc::new(Mutex::new(None));
        let packets = Arc::new(AtomicUsize::new(0));
        let source = endpoint_source.clone();
        let count = packets.clone();
        let task = tokio::spawn(async move {
            let mut inbound = [0u8; 2048];
            let mut outbound = [0u8; 2048];
            loop {
                tokio::select! {
                    Ok((len, from)) = ingress.recv_from(&mut inbound) => {
                        *source.lock().unwrap() = Some(from);
                        count.fetch_add(1, Ordering::Relaxed);
                        if let Some(delay) = first_packet_delay.take() {
                            tokio::time::sleep(delay).await;
                        }
                        egress.send_to(&inbound[..len], observer_addr).await.unwrap();
                    }
                    Ok((len, from)) = egress.recv_from(&mut outbound) => {
                        assert_eq!(from, observer_addr);
                        let destination = *source.lock().unwrap();
                        if let Some(destination) = destination {
                            ingress.send_to(&outbound[..len], destination).await.unwrap();
                        }
                    }
                }
            }
        });
        Self {
            ingress_addr,
            mapped_addr,
            endpoint_source,
            packets,
            task,
        }
    }
}

async fn observer() -> (server::Server, CaTlsConfig) {
    let (certs, tls) = server::testing::self_signed_tls_certs_and_config();
    let mut quic = server::QuicConfig::new((Ipv4Addr::LOCALHOST, 0));
    quic.server_config = Some(tls);
    let mut config = server::ServerConfig::default();
    config.quic = Some(quic);
    let server = server::Server::spawn(config).await.unwrap();
    (server, CaTlsConfig::custom_roots(certs))
}

fn observation_map(port: u16) -> RelayMap {
    RelayMap::from(RelayConfig::new(
        "https://127.0.0.1/".parse().unwrap(),
        Some(RelayQuicConfig::new(port)),
    ))
}

fn localhost_observer(port: u16) -> RelayConfig {
    RelayConfig::new(
        "https://localhost/".parse().unwrap(),
        Some(RelayQuicConfig::new(port)),
    )
}

async fn fresh(endpoint: &Endpoint) -> UdpObservation {
    let mut watcher = endpoint.udp_observation();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let state = watcher.get();
            if !matches!(state, UdpObservation::Pending) {
                return state;
            }
            watcher.updated().await.unwrap();
        }
    })
    .await
    .unwrap()
}

async fn wait_for(
    endpoint: &Endpoint,
    desired: impl Fn(&UdpObservation) -> bool,
) -> UdpObservation {
    wait_for_with_timeout(endpoint, Duration::from_secs(12), desired).await
}

async fn wait_for_with_timeout(
    endpoint: &Endpoint,
    deadline: Duration,
    desired: impl Fn(&UdpObservation) -> bool,
) -> UdpObservation {
    let mut watcher = endpoint.udp_observation();
    tokio::time::timeout(deadline, async {
        loop {
            let state = watcher.get();
            if desired(&state) {
                return state;
            }
            watcher.updated().await.unwrap();
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn relay_disabled_observes_the_real_quic_socket_through_a_controlled_nat() {
    let (observer, roots) = observer().await;
    let nat = NatProxy::start(observer.quic_addr().unwrap()).await;
    let endpoint = Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .observation_servers(observation_map(nat.ingress_addr.port()))
        .ca_tls_config(roots)
        .bind()
        .await
        .unwrap();
    let state = fresh(&endpoint).await;
    assert!(state.is_fresh(), "{state:?}");
    let UdpObservation::Fresh { addresses, .. } = state else {
        panic!("{state:?}")
    };
    assert_eq!(addresses, vec![nat.mapped_addr]);
    assert_eq!(
        *nat.endpoint_source.lock().unwrap(),
        endpoint.bound_sockets().first().copied()
    );
    assert!(nat.packets.load(Ordering::Relaxed) > 0);
    assert!(
        endpoint
            .addr()
            .ip_addrs()
            .any(|address| *address == nat.mapped_addr)
    );
    assert!(endpoint.home_relay_status().get().is_empty());
    endpoint.close().await;
}

#[tokio::test]
async fn default_qad_completes_hostname_verified_ipv4_and_ipv6_observation() {
    use n0_future::StreamExt;
    use std::net::Ipv6Addr;

    for bind in [
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        SocketAddr::from((Ipv6Addr::LOCALHOST, 0)),
    ] {
        let (certs, server_tls) = server::testing::self_signed_tls_certs_and_config();
        let mut quic = server::QuicConfig::new(bind);
        quic.server_config = Some(server_tls);
        let mut config = server::ServerConfig::default();
        config.quic = Some(quic);
        let server = server::Server::spawn(config).await.unwrap();
        let endpoint = noq::Endpoint::client(bind).unwrap();
        let local = endpoint.local_addr().unwrap();
        let tls = CaTlsConfig::custom_roots(certs)
            .client_config(Arc::new(rustls::crypto::ring::default_provider()))
            .unwrap();
        let client = iroh_relay::quic::QuicClient::new(endpoint.clone(), tls);
        let connection = tokio::time::timeout(
            Duration::from_secs(5),
            client.create_conn(server.quic_addr().unwrap(), "localhost"),
        )
        .await
        .unwrap()
        .unwrap();
        let observed = tokio::time::timeout(
            Duration::from_secs(5),
            connection.observed_external_addr().next(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            connection.stats().frame_tx.crypto > 2,
            "default QAD must split the ClientHello CRYPTO stream on {bind}"
        );
        assert_eq!(observed.ip().to_canonical(), local.ip());
        assert_eq!(observed.port(), local.port());
        let handshake = connection
            .handshake_data()
            .unwrap()
            .downcast::<noq::crypto::rustls::HandshakeData>()
            .unwrap();
        assert_eq!(
            handshake.protocol.as_deref(),
            Some(iroh_relay::quic::ALPN_QUIC_ADDR_DISC)
        );
        connection.close(0u32.into(), b"QAD test complete");
        endpoint.close(0u32.into(), b"QAD test complete");
        endpoint.wait_idle().await;
        server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn default_qad_uses_the_endpoint_congestion_factory() {
    use iroh::endpoint::QuicTransportConfig;
    use noq_proto::congestion::{Controller, ControllerFactory, CubicConfig};

    #[derive(Default)]
    struct CountingCubic(AtomicUsize);

    impl ControllerFactory for CountingCubic {
        fn build(self: Arc<Self>, now: std::time::Instant, mtu: u16) -> Box<dyn Controller> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Arc::new(CubicConfig::default()).build(now, mtu)
        }
    }

    let (observer, roots) = observer().await;
    let address = observer.quic_addr().unwrap();
    for observers in [
        observation_map(address.port()),
        RelayMap::from(localhost_observer(address.port())),
    ] {
        let factory = Arc::new(CountingCubic::default());
        let endpoint = Endpoint::builder(presets::Minimal)
            .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
            .clear_ip_transports()
            .bind_addr((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .observation_servers(observers)
            .ca_tls_config(roots.clone())
            .transport_config(
                QuicTransportConfig::builder()
                    .congestion_controller_factory(factory.clone())
                    .build(),
            )
            .bind()
            .await
            .unwrap();
        let state = fresh(&endpoint).await;
        assert!(state.is_fresh(), "{state:?}");
        let UdpObservation::Fresh { addresses, .. } = state else {
            panic!("{state:?}")
        };
        assert_eq!(addresses, endpoint.bound_sockets());
        assert!(factory.0.load(Ordering::Relaxed) > 0);
        endpoint.close().await;
    }

    observer.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_periodic_observation_withdraws_the_old_udp_candidate() {
    let (observer, roots) = observer().await;
    let nat = NatProxy::start(observer.quic_addr().unwrap()).await;
    let old_mapped = nat.mapped_addr;
    let endpoint = Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .observation_servers(observation_map(nat.ingress_addr.port()))
        .ca_tls_config(roots)
        .bind()
        .await
        .unwrap();
    let old_state = fresh(&endpoint).await;
    assert!(old_state.is_fresh());
    nat.task.abort();
    let _ = nat.task.await;
    let state = wait_for_with_timeout(&endpoint, Duration::from_secs(35), |state| {
        matches!(
            state,
            UdpObservation::Failed(UdpObservationFailure::NoObservedAddress)
        )
    })
    .await;
    assert!(!state.is_fresh());
    assert!(!old_state.is_fresh(), "superseded snapshot stayed valid");
    assert!(!endpoint.addr().ip_addrs().any(|addr| *addr == old_mapped));
    assert!(endpoint.home_relay_status().get().is_empty());
    endpoint.close().await;
}

#[tokio::test]
async fn destination_dependent_nat_reports_mapping_variance() {
    let (observer, roots) = observer().await;
    let observer_addr = observer.quic_addr().unwrap();
    let first = NatProxy::start(observer_addr).await;
    let second = NatProxy::start(observer_addr).await;
    let servers = RelayMap::from_iter([
        RelayConfig::new(
            "https://127.0.0.1/".parse().unwrap(),
            Some(RelayQuicConfig::new(first.ingress_addr.port())),
        ),
        localhost_observer(second.ingress_addr.port()),
    ]);
    let endpoint = Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .observation_servers(servers.clone())
        .ca_tls_config(roots)
        .bind()
        .await
        .unwrap();
    let state = fresh(&endpoint).await;
    assert!(
        matches!(
            state,
            UdpObservation::Fresh {
                mapping_varies_by_dest: Some(true),
                ..
            }
        ),
        "{state:?}"
    );
    assert!(first.packets.load(Ordering::Relaxed) > 0);
    assert!(second.packets.load(Ordering::Relaxed) > 0);
    let sequence = endpoint
        .observe_udp_mapping_sequence(&servers)
        .await
        .unwrap();
    for proxy in [&first, &second] {
        assert!(sequence.samples.iter().any(|sample| {
            sample.destination == proxy.ingress_addr && sample.mapped == proxy.mapped_addr
        }));
        assert_eq!(
            *proxy.endpoint_source.lock().unwrap(),
            endpoint.bound_sockets().first().copied()
        );
    }
    endpoint.close().await;
}

#[tokio::test]
async fn default_qad_rejects_a_trusted_certificate_for_another_name() {
    let certificate = rcgen::generate_simple_self_signed(vec!["other.test".into()]).unwrap();
    let cert = certificate.cert.der().clone();
    let key = rustls::pki_types::PrivatePkcs8KeyDer::from(certificate.signing_key.serialize_der());
    let tls = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(vec![cert.clone()], key.into())
    .unwrap();
    let mut quic = server::QuicConfig::new((Ipv4Addr::LOCALHOST, 0));
    quic.server_config = Some(tls);
    let mut config = server::ServerConfig::default();
    config.quic = Some(quic);
    let server = server::Server::spawn(config).await.unwrap();
    let proxy = NatProxy::start(server.quic_addr().unwrap()).await;
    let endpoint = Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .observation_servers(RelayMap::from(localhost_observer(
            proxy.ingress_addr.port(),
        )))
        .ca_tls_config(CaTlsConfig::custom_roots([cert]))
        .bind()
        .await
        .unwrap();
    assert!(matches!(fresh(&endpoint).await, UdpObservation::Failed(_)));
    assert!(proxy.packets.load(Ordering::Relaxed) > 0);
    endpoint.close().await;
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn parallel_qad_includes_a_slow_observer_after_three_fast_reports() {
    let (observer, roots) = observer().await;
    let observer_addr = observer.quic_addr().unwrap();
    let fast = NatProxy::start(observer_addr).await;
    let slow = NatProxy::start_on_with_delay(
        observer_addr,
        (Ipv4Addr::LOCALHOST, 0).into(),
        Some(Duration::from_secs(4)),
    )
    .await;
    // Three observer URLs share the fast mapping. A different mapping in
    // the report therefore proves the delayed observer was included.
    let regular = (20001..=20003).map(|port| {
        RelayConfig::new(
            format!("https://127.0.0.1:{port}/").parse().unwrap(),
            Some(RelayQuicConfig::new(fast.ingress_addr.port())),
        )
    });
    let servers =
        RelayMap::from_iter(regular.chain([localhost_observer(slow.ingress_addr.port())]));
    let endpoint = Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .observation_servers(servers)
        .ca_tls_config(roots)
        .bind()
        .await
        .unwrap();
    let state = fresh(&endpoint).await;
    let UdpObservation::Fresh {
        addresses,
        mapping_varies_by_dest: Some(true),
        ..
    } = state
    else {
        panic!("late mapping missing from {state:?}");
    };
    assert_eq!(addresses, [fast.mapped_addr]);
    endpoint.close().await;
    observer.shutdown().await.unwrap();
}

#[tokio::test]
async fn disabled_observation_endpoint_never_contacts_the_observer() {
    let (observer, roots) = observer().await;
    let nat = NatProxy::start(observer.quic_addr().unwrap()).await;
    let endpoint = Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .observation_servers(RelayMap::empty())
        .ca_tls_config(roots)
        .bind()
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(endpoint.udp_observation().get(), UdpObservation::Disabled);
    assert_eq!(nat.packets.load(Ordering::Relaxed), 0);
    endpoint.close().await;
}

#[tokio::test]
async fn mixed_observation_and_data_relay_configuration_fails_at_bind() {
    let observer = observation_map(12345);
    let relay = observation_map(12346);
    let result = Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .relay_mode(RelayMode::Custom(relay))
        .observation_servers(observer)
        .bind()
        .await;
    assert!(matches!(
        result,
        Err(BindError::InvalidTransportConfig { .. })
    ));
}

#[tokio::test]
async fn changed_nat_mapping_replaces_the_old_candidate_after_network_change() {
    let (observer, roots) = observer().await;
    let observer_addr = observer.quic_addr().unwrap();
    let nat = NatProxy::start(observer_addr).await;
    let ingress = nat.ingress_addr;
    let old_mapped = nat.mapped_addr;
    let endpoint = Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .observation_servers(observation_map(ingress.port()))
        .ca_tls_config(roots)
        .bind()
        .await
        .unwrap();
    let old_state = fresh(&endpoint).await;
    assert!(old_state.is_fresh());
    nat.task.abort();
    let _ = nat.task.await;
    let new_nat = NatProxy::start_on(observer_addr, ingress).await;
    assert_ne!(old_mapped, new_nat.mapped_addr);
    endpoint.network_change().await;
    let state = wait_for(&endpoint, |state| {
        matches!(state, UdpObservation::Fresh { addresses, .. } if addresses.contains(&new_nat.mapped_addr))
    }).await;
    assert!(state.is_fresh());
    assert!(!old_state.is_fresh(), "old NAT mapping stayed valid");
    assert!(!endpoint.addr().ip_addrs().any(|addr| *addr == old_mapped));
    endpoint.close().await;
}

#[tokio::test]
async fn ordered_qad_skips_first_timeout_and_keeps_four_independent_samples() {
    let (observer, roots) = observer().await;
    let mut proxies = Vec::new();
    for _ in 0..4 {
        proxies.push(NatProxy::start(observer.quic_addr().unwrap()).await);
    }
    // Keep the blackhole first in RelayMap's URL order, regardless of the
    // kernel-assigned ports of the successful independent destinations.
    let blackhole = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let mut configs = vec![RelayConfig::new(
        "https://127.0.0.1:1/".parse().unwrap(),
        Some(RelayQuicConfig::new(blackhole.local_addr().unwrap().port())),
    )];
    for (index, proxy) in proxies.iter().enumerate() {
        configs.push(RelayConfig::new(
            format!("https://127.0.0.1:{}/", 20000 + index)
                .parse()
                .unwrap(),
            Some(RelayQuicConfig::new(proxy.ingress_addr.port())),
        ));
    }
    let endpoint = Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .ca_tls_config(roots)
        .bind()
        .await
        .unwrap();
    let map = RelayMap::from_iter(configs);
    let sequence = endpoint
        .observe_udp_mapping_sequence(&map)
        .await
        .expect("one unavailable QAD observer must not discard later samples");
    assert_eq!(sequence.samples.len(), 4);
    assert!(sequence.is_current_generation());
    for (sample, proxy) in sequence.samples.iter().zip(&proxies) {
        assert_eq!(sample.destination, proxy.ingress_addr);
        assert_eq!(sample.mapped, proxy.mapped_addr);
        assert_eq!(
            *proxy.endpoint_source.lock().unwrap(),
            endpoint.bound_sockets().first().copied()
        );
    }
    assert!(
        sequence
            .samples
            .windows(2)
            .all(|pair| pair[0].observed_at <= pair[1].observed_at)
    );
    let recovered_addr = blackhole.local_addr().unwrap();
    drop(blackhole);
    let recovered = NatProxy::start_on(observer.quic_addr().unwrap(), recovered_addr).await;
    let retried = endpoint.observe_udp_mapping_sequence(&map).await.unwrap();
    assert_eq!(
        retried.samples.len(),
        5,
        "a failed observer is retried on the next round"
    );
    assert_eq!(retried.samples[0].mapped, recovered.mapped_addr);
    endpoint.close().await;
    recovered.task.abort();
    for proxy in proxies {
        proxy.task.abort();
    }
}

#[tokio::test]
async fn observation_stops_when_permission_is_revoked() {
    use iroh::endpoint::{RelayResourcePolicy, RelayResourceUse};
    use std::sync::atomic::AtomicBool;
    for mixed in [false, true] {
        let allowed = Arc::new(AtomicBool::new(true));
        let policy_allowed = allowed.clone();
        let blackhole = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = blackhole.local_addr().unwrap().port();
        let map = RelayMap::from_iter([1, 2].map(|tag| {
            RelayConfig::new(
                format!("https://127.0.0.1:{tag}/").parse().unwrap(),
                Some(RelayQuicConfig::new(port)),
            )
        }));
        let endpoint = Endpoint::builder(presets::Minimal)
            .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
            .clear_ip_transports()
            .bind_addr((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .relay_resource_policy(RelayResourcePolicy::new(move |_, usage| {
                !matches!(usage, RelayResourceUse::Observation)
                    || policy_allowed.load(Ordering::SeqCst)
            }))
            .bind()
            .await
            .unwrap();
        let revoke = async {
            blackhole.recv_from(&mut [0; 2048]).await.unwrap();
            allowed.store(false, Ordering::SeqCst);
        };
        let observe = async {
            if mixed {
                endpoint.observe_udp_mapping_round(&map, &[]).await
            } else {
                endpoint.observe_udp_mapping_sequence(&map).await
            }
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(observe, revoke)
        })
        .await
        .expect("revocation must cancel pending observation, not wait for per-observer timeout");
        assert_eq!(
            result.unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        endpoint.close().await;
    }
}

#[tokio::test]
async fn mixed_round_counts_each_actual_destination_once_and_expires_on_network_change() {
    let (observer, roots) = observer().await;
    let addr = observer.quic_addr().unwrap();
    let relays = RelayMap::from_iter([1, 2].map(|tag| {
        RelayConfig::new(
            format!("https://127.0.0.1:{tag}/").parse().unwrap(),
            Some(RelayQuicConfig::new(addr.port())),
        )
    }));
    let endpoint = Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .ca_tls_config(roots)
        .bind()
        .await
        .unwrap();
    let first = endpoint
        .observe_udp_mapping_round(&relays, &[])
        .await
        .unwrap();
    assert_eq!(first.samples.len(), 1);
    assert_eq!(first.samples[0].destination, addr);
    assert_eq!(first.samples[0].mapped, endpoint.bound_sockets()[0]);
    assert!(first.is_current_generation());

    let second = endpoint
        .observe_udp_mapping_round(&relays, &[])
        .await
        .unwrap();
    assert!(!first.is_current_generation());
    assert!(second.is_current_generation());
    endpoint.network_change().await;
    tokio::time::timeout(Duration::from_secs(1), async {
        while second.is_current_generation() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("network actor must invalidate the observation round");
    assert!(!second.is_current_generation());
    endpoint.close().await;
    observer.shutdown().await.unwrap();
}

#[tokio::test]
async fn ordered_qad_discards_partial_samples_on_network_change_or_close() {
    for change_network in [true, false] {
        let (observer, roots) = observer().await;
        let blackhole = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let map = RelayMap::from_iter([
            RelayConfig::new(
                "https://127.0.0.1:1/".parse().unwrap(),
                Some(RelayQuicConfig::new(observer.quic_addr().unwrap().port())),
            ),
            RelayConfig::new(
                "https://127.0.0.1:2/".parse().unwrap(),
                Some(RelayQuicConfig::new(blackhole.local_addr().unwrap().port())),
            ),
        ]);
        let endpoint = Endpoint::builder(presets::Minimal)
            .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
            .clear_ip_transports()
            .bind_addr((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .ca_tls_config(roots)
            .bind()
            .await
            .unwrap();
        let changing = endpoint.clone();
        let invalidate = tokio::spawn(async move {
            // Arrival at the second observer proves the first sample was acquired.
            blackhole.recv_from(&mut [0; 2048]).await.unwrap();
            if change_network {
                changing.network_change().await;
            } else {
                changing.close().await;
            }
        });
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            endpoint.observe_udp_mapping_sequence(&map),
        )
        .await
        .expect("invalidation must promptly end the old observation");
        // QUIC draining after close is independent of cancelling this operation.
        invalidate.await.unwrap();
        assert!(
            result.is_err(),
            "partial samples from old work cannot escape"
        );
        endpoint.close().await;
    }
}

#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[tokio::test]
#[ignore = "requires a disposable network namespace for actual IPv6 prefix changes"]
async fn ipv6_change_retains_the_periodic_ipv4_qad_candidate_without_renewing_it() {
    use iroh::endpoint::ObservationDialAddresses;
    use std::{net::Ipv6Addr, process::Command};
    fn ip(arguments: &[&str]) {
        assert!(
            Command::new("ip")
                .args(arguments)
                .status()
                .unwrap()
                .success()
        );
    }
    let old6: Ipv6Addr = "2001:db8:205::1".parse().unwrap();
    let observer6: Ipv6Addr = "2001:db8:205::fe".parse().unwrap();
    ip(&["link", "add", "mio205-qad", "type", "dummy"]);
    ip(&["link", "set", "mio205-qad", "up"]);
    ip(&["addr", "add", "10.205.0.1/24", "dev", "mio205-qad"]);
    for address in ["2001:db8:205::1/64", "2001:db8:205::fe/64"] {
        ip(&["-6", "addr", "add", address, "dev", "mio205-qad", "nodad"]);
    }
    let (certs, tls) = server::testing::self_signed_tls_certs_and_config();
    let mut servers = Vec::new();
    for bind in [
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        SocketAddr::from((Ipv6Addr::LOCALHOST, 0)),
    ] {
        let mut quic = server::QuicConfig::new(bind);
        quic.server_config = Some(tls.clone());
        let mut config = server::ServerConfig::default();
        config.quic = Some(quic);
        servers.push(server::Server::spawn(config).await.unwrap());
    }
    let v4 = NatProxy::start(servers[0].quic_addr().unwrap()).await;
    let v6 = NatProxy::start_on(servers[1].quic_addr().unwrap(), (observer6, 0).into()).await;
    let endpoint = Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .bind_addr((old6, 0))
        .unwrap()
        .observation_servers(RelayMap::from_iter([
            RelayConfig::new(
                "https://127.0.0.1/".parse().unwrap(),
                Some(RelayQuicConfig::new(v4.ingress_addr.port())),
            ),
            {
                let mut quic = RelayQuicConfig::new(v6.ingress_addr.port());
                quic.dial_addresses = ObservationDialAddresses {
                    ipv4: None,
                    ipv6: Some(observer6),
                };
                RelayConfig::new("https://localhost/".parse().unwrap(), Some(quic))
            },
        ]))
        .ca_tls_config(CaTlsConfig::custom_roots(certs))
        .bind()
        .await
        .unwrap();
    let mut observed = endpoint.udp_observation();
    let before = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let state = observed.get();
            if matches!(&state, UdpObservation::Fresh { addresses, .. } if addresses.contains(&v4.mapped_addr) && addresses.contains(&v6.mapped_addr)) { break state; }
            observed.updated().await.unwrap();
        }
    }).await.unwrap_or_else(|error| panic!("initial dual QAD: {error}; state={:?} v4Packets={} v6Packets={} v4Source={:?} v6Source={:?}", observed.get(), v4.packets.load(Ordering::Relaxed), v6.packets.load(Ordering::Relaxed), *v4.endpoint_source.lock().unwrap(), *v6.endpoint_source.lock().unwrap()));
    assert!(before.is_fresh());
    let UdpObservation::Fresh {
        observed_at,
        valid_until,
        ..
    } = before.clone()
    else {
        unreachable!()
    };
    let old_source = *v6.endpoint_source.lock().unwrap();
    assert_eq!(old_source.unwrap().ip(), std::net::IpAddr::V6(old6));
    for address in [v4.mapped_addr, v6.mapped_addr] {
        assert!(
            endpoint
                .addr()
                .ip_addrs()
                .any(|current| *current == address)
        );
    }
    println!(
        "NETWORK_CHANGE_QAD_INITIAL v4={} v6={} source={old_source:?}",
        v4.mapped_addr, v6.mapped_addr
    );
    for server in servers {
        server.shutdown().await.unwrap();
    }
    let changed = endpoint
        .binding_network_generation((old6, 0).into())
        .unwrap();
    let stun = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let origin = format!("stun://{}/", stun.local_addr().unwrap()).parse().unwrap();
    let (sample, source) = tokio::join!(
        endpoint.observe_udp_stun_for_family(&origin, iroh::endpoint::ObservationFamily::Ipv4),
        route_source_stun::reply_one_stun(&stun)
    );
    let sample = sample.unwrap();
    assert_eq!(sample.mapped, source);
    ip(&[
        "-6",
        "addr",
        "del",
        "2001:db8:205::1/64",
        "dev",
        "mio205-qad",
    ]);
    ip(&[
        "-6",
        "addr",
        "add",
        "2001:db8:205:1::1/64",
        "dev",
        "mio205-qad",
        "nodad",
    ]);
    tokio::time::timeout(Duration::from_secs(3), async {
        while changed.is_current() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let after = wait_for_with_timeout(&endpoint, Duration::from_secs(1), |state| {
        matches!(state, UdpObservation::Fresh { addresses, .. } if addresses == &vec![v4.mapped_addr])
    }).await;
    assert!(after.is_fresh());
    assert!(sample.is_fresh(), "an IPv6 prefix change must preserve the single-station IPv4 STUN sample");
    assert!(
        !before.is_fresh(),
        "the old mixed snapshot cannot still advertise its IPv6 candidate"
    );
    let UdpObservation::Fresh {
        observed_at: retained_at,
        valid_until: retained_until,
        ..
    } = after.clone()
    else {
        unreachable!()
    };
    assert_eq!((retained_at, retained_until), (observed_at, valid_until));
    assert!(
        endpoint
            .addr()
            .ip_addrs()
            .any(|address| *address == v4.mapped_addr)
    );
    assert!(!endpoint.addr().ip_addrs().any(|address| *address == v6.mapped_addr || address.ip() == std::net::IpAddr::V6(old6)));
    println!(
        "NETWORK_CHANGE_QAD retained={} removed={} original_expiry=true",
        v4.mapped_addr, v6.mapped_addr
    );
    let forced = endpoint.udp_network_generation();
    endpoint.network_change().await;
    tokio::time::timeout(Duration::from_secs(1), async {
        while forced.is_current() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        !after.is_fresh(),
        "an explicit NAT-only notification still invalidates the retained sample"
    );
    assert!(!sample.is_fresh());
    assert!(
        !endpoint
            .addr()
            .ip_addrs()
            .any(|address| *address == v4.mapped_addr)
    );
    endpoint.close().await;
}

#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[path = "support/stun_observer.rs"]
mod route_source_stun;

#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[tokio::test]
#[ignore = "requires a disposable network namespace for destination-specific source routes"]
async fn mixed_stun_and_qad_use_the_source_of_the_first_permitted_station() {
    use iroh::endpoint::{RelayResourcePolicy, RelayResourceUse};
    use std::process::Command;
    // The QAD proxy uses the loopback default. Only the permitted STUN station
    // has a preferred source of .2, and both probes must use that source.
    let source = Ipv4Addr::new(127, 0, 0, 2);
    for args in [vec!["addr", "add", "127.0.0.2/32", "dev", "lo"],
                 vec!["route", "add", "local", "127.0.0.11/32", "dev", "lo", "src", "127.0.0.2", "table", "local"]] {
        assert!(Command::new("ip").args(args).status().unwrap().success());
    }
    let (server, roots) = observer().await;
    let proxy = NatProxy::start(server.quic_addr().unwrap()).await;
    let stun = UdpSocket::bind(("127.0.0.11", 0)).await.unwrap();
    let denied: iroh::RelayUrl = "stun://127.0.0.1:9/".parse().unwrap();
    let forbidden = denied.clone();
    let endpoint = Endpoint::builder(presets::Minimal)
        .clear_ip_transports().bind_addr((Ipv4Addr::UNSPECIFIED, 0)).unwrap()
        .ca_tls_config(roots)
        .relay_resource_policy(RelayResourcePolicy::new(move |origin, usage|
            !matches!(usage, RelayResourceUse::Observation) || origin != &forbidden))
        .bind().await.unwrap();
    let relays = observation_map(proxy.ingress_addr.port());
    let allowed: iroh::RelayUrl = format!("stun://{}/", stun.local_addr().unwrap()).parse().unwrap();
    for stations in [vec![allowed.clone()], vec![denied, allowed]] {
        let (samples, stun_source) = tokio::join!(
            endpoint.observe_udp_mapping_round(&relays, &stations),
            route_source_stun::reply_one_stun(&stun)
        );
        assert_eq!(samples.unwrap().samples.len(), 2);
        let binding = SocketAddr::from((source, endpoint.bound_sockets()[0].port()));
        assert_eq!(stun_source, binding);
        assert_eq!(*proxy.endpoint_source.lock().unwrap(), Some(binding));
    }
    endpoint.close().await;
    proxy.task.abort();
    server.shutdown().await.unwrap();
}

#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[tokio::test]
#[ignore = "requires unshare -Urnm with a private /run/netns"]
async fn route_source_change_refreshes_only_the_known_destination_scope() {
    use iroh::endpoint::ObservationFamily;
    use std::{
        path::Path,
        process::{Command, Stdio},
    };
    async fn signal(root: &Path, name: &str) {
        tokio::time::timeout(Duration::from_secs(8), async {
            while !root.join(name).exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap_or_else(|error| panic!("missing {name}: {error}"));
    }
    if let Some(root) = std::env::var_os("MIO205_ROUTE_CHILD") {
        let root = std::path::PathBuf::from(root);
        let destinations: Vec<SocketAddr> = std::fs::read_to_string(root.join("observers"))
            .unwrap()
            .lines()
            .map(|line| line.parse().unwrap())
            .collect();
        let observers: Vec<iroh::RelayUrl> = destinations
            .iter()
            .map(|destination| format!("stun://{destination}/").parse().unwrap())
            .collect();
        let endpoint = Endpoint::builder(presets::Minimal)
            .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
            .clear_ip_transports()
            .bind_addr((Ipv4Addr::UNSPECIFIED, 0))
            .unwrap()
            .bind()
            .await
            .unwrap();
        let port = endpoint.bound_sockets()[0].port();
        let source: SocketAddr = format!("10.206.0.1:{port}").parse().unwrap();
        let binding = endpoint.binding_network_generation(source).unwrap();
        let route = endpoint
            .binding_route_generation(source, destinations[0])
            .unwrap();
        let unaffected = endpoint
            .binding_network_generation("[::1]:0".parse().unwrap())
            .unwrap();
        let network = endpoint.udp_network_generation();
        let old = tokio::spawn({
            let endpoint = endpoint.clone();
            let observers = observers.clone();
            async move {
                endpoint
                    .observe_udp_mapping_round_for_family_until(
                        &RelayMap::empty(),
                        &observers,
                        ObservationFamily::Ipv4,
                        tokio::time::Instant::now() + Duration::from_secs(5),
                    )
                    .await
            }
        });
        signal(&root, "observed-a-0").await;
        signal(&root, "observed-a-1").await;
        std::fs::write(root.join("ready"), []).unwrap();
        signal(&root, "control").await;
        // The negative control spans the existing 250 ms netmon debounce.
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert!(
            route.is_current()
                && binding.is_current()
                && unaffected.is_current()
                && network.is_current()
        );
        assert!(
            !old.is_finished(),
            "a same-source route update preserves the actual in-flight query"
        );
        std::fs::write(root.join("control-checked"), []).unwrap();
        signal(&root, "changed").await;
        tokio::time::timeout(Duration::from_secs(3), async {
            while route.is_current() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("actual known-destination preferred-source change invalidates its token");
        assert!(
            binding.is_current() && unaffected.is_current(),
            "the assigned source and unrelated family still exist"
        );
        let result = tokio::time::timeout(Duration::from_secs(1), old)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::Interrupted);
        let current = endpoint
            .observe_udp_mapping_round_for_family_until(
                &RelayMap::empty(),
                &observers,
                ObservationFamily::Ipv4,
                tokio::time::Instant::now() + Duration::from_secs(5),
            )
            .await
            .unwrap();
        assert_eq!(current.samples.len(), 2);
        assert!(current.is_current_generation());
        assert!(
            current
                .samples
                .iter()
                .all(|sample| sample.mapped
                    == SocketAddr::from((Ipv4Addr::new(10, 206, 0, 2), port)))
        );
        println!(
            "NETWORK_ROUTE_SOURCE old={source} new=10.206.0.2:{port} control=current old_query=Interrupted new_samples={}",
            current.samples.len()
        );
        endpoint.close().await;
        return;
    }
    fn ip(arguments: &[&str]) -> String {
        let output = Command::new("ip").args(arguments).output().unwrap();
        assert!(
            output.status.success(),
            "ip {arguments:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    struct Lab;
    impl Drop for Lab {
        fn drop(&mut self) {
            let _ = Command::new("ip")
                .args(["netns", "del", "mio205-route"])
                .output();
            let _ = Command::new("ip")
                .args(["link", "del", "m205-host"])
                .output();
        }
    }
    let root = tempfile::tempdir().unwrap();
    ip(&["netns", "add", "mio205-route"]);
    let _lab = Lab;
    ip(&[
        "link",
        "add",
        "m205-host",
        "type",
        "veth",
        "peer",
        "name",
        "lan0",
        "netns",
        "mio205-route",
    ]);
    ip(&["addr", "add", "10.206.0.254/24", "dev", "m205-host"]);
    ip(&["link", "set", "m205-host", "up"]);
    ip(&["-n", "mio205-route", "link", "set", "lo", "up"]);
    ip(&[
        "-n",
        "mio205-route",
        "link",
        "set",
        "lan0",
        "addrgenmode",
        "none",
    ]);
    for address in ["10.206.0.1/24", "10.206.0.2/24"] {
        ip(&["-n", "mio205-route", "addr", "add", address, "dev", "lan0"]);
    }
    ip(&["-n", "mio205-route", "link", "set", "lan0", "up"]);
    ip(&[
        "-n",
        "mio205-route",
        "route",
        "replace",
        "10.206.0.254/32",
        "dev",
        "lan0",
        "src",
        "10.206.0.1",
    ]);
    let addresses = ip(&["-j", "-n", "mio205-route", "addr", "show", "dev", "lan0"]);
    let mut tasks = tokio::task::JoinSet::new();
    let mut observers = Vec::new();
    for number in 0..2 {
        let socket = UdpSocket::bind((Ipv4Addr::new(10, 206, 0, 254), 0))
            .await
            .unwrap();
        observers.push(socket.local_addr().unwrap().to_string());
        let root = root.path().to_path_buf();
        tasks.spawn(async move {
            loop {
                let mut request = [0; 1500];
                let (_, source) = socket.peek_from(&mut request).await.unwrap();
                if source.ip() == Ipv4Addr::new(10, 206, 0, 1) {
                    socket.recv_from(&mut request).await.unwrap();
                    std::fs::write(root.join(format!("observed-a-{number}")), []).unwrap();
                } else {
                    assert_eq!(
                        route_source_stun::reply_one_stun(&socket).await.ip(),
                        Ipv4Addr::new(10, 206, 0, 2)
                    );
                }
            }
        });
    }
    std::fs::write(root.path().join("observers"), observers.join("\n")).unwrap();
    let child = tokio::process::Command::new("ip")
        .args(["netns", "exec", "mio205-route"])
        .arg(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "route_source_change_refreshes_only_the_known_destination_scope",
            "--ignored",
            "--nocapture",
        ])
        .env("MIO205_ROUTE_CHILD", root.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    signal(root.path(), "ready").await;
    ip(&[
        "-n",
        "mio205-route",
        "route",
        "replace",
        "10.206.0.254/32",
        "dev",
        "lan0",
        "src",
        "10.206.0.1",
        "mtu",
        "1400",
    ]);
    std::fs::write(root.path().join("control"), []).unwrap();
    signal(root.path(), "control-checked").await;
    let control = ip(&["-n", "mio205-route", "route", "get", "10.206.0.254"]);
    assert!(control.contains("src 10.206.0.1"));
    ip(&[
        "-n",
        "mio205-route",
        "route",
        "replace",
        "10.206.0.254/32",
        "dev",
        "lan0",
        "src",
        "10.206.0.2",
        "mtu",
        "1400",
    ]);
    let changed = ip(&["-n", "mio205-route", "route", "get", "10.206.0.254"]);
    assert!(changed.contains("src 10.206.0.2"));
    assert_eq!(
        ip(&["-j", "-n", "mio205-route", "addr", "show", "dev", "lan0"]),
        addresses
    );
    println!(
        "NETWORK_ROUTE_KERNEL control={control:?} changed={changed:?} interface_addresses=unchanged"
    );
    std::fs::write(root.path().join("changed"), []).unwrap();
    let output = child.wait_with_output().await.unwrap();
    println!("{}", String::from_utf8_lossy(&output.stdout));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
