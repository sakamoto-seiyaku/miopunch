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
        let egress = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
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
