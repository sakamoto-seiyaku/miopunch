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
        RelayConfig::new(
            "https://localhost/".parse().unwrap(),
            Some(RelayQuicConfig::new(second.ingress_addr.port())),
        ),
    ]);
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
    endpoint.close().await;
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
