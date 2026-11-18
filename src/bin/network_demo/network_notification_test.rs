//! Exercises the demo sampler against a real relay and UDP address discovery.
//! The Linux namespace driver changes the default interface after `noise-passed`
//! appears in its fixture directory, preserving the assigned local IPs.
use super::{tests::FixtureDirectory, *};
use iroh_relay::server;
use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicBool, Ordering},
};
use tokio::net::UdpSocket;

async fn forward_qad(
    ingress: UdpSocket,
    old_mapping: UdpSocket,
    new_mapping: UdpSocket,
    use_new_mapping: Arc<AtomicBool>,
) {
    let mut endpoint = None;
    let (mut query, mut old_reply, mut new_reply) = ([0; 2048], [0; 2048], [0; 2048]);
    loop {
        tokio::select! {
            packet = ingress.recv_from(&mut query) => {
                let (size, source) = packet.unwrap();
                endpoint = Some(source);
                let mapping = if use_new_mapping.load(Ordering::Relaxed) { &new_mapping } else { &old_mapping };
                mapping.send(&query[..size]).await.unwrap();
            }
            packet = old_mapping.recv(&mut old_reply) => {
                ingress.send_to(&old_reply[..packet.unwrap()], endpoint.unwrap()).await.unwrap();
            }
            packet = new_mapping.recv(&mut new_reply) => {
                ingress.send_to(&new_reply[..packet.unwrap()], endpoint.unwrap()).await.unwrap();
            }
        }
    }
}

async fn wait_for_event(state: &Mutex<Observations>, kind: &str, matches: impl Fn(&Value) -> bool) {
    loop {
        if state
            .lock()
            .unwrap()
            .events
            .iter()
            .any(|event| event["kind"] == kind && matches(&event["detail"]))
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn local_relay(addr: SocketAddr) -> server::Server {
    let mut config = server::ServerConfig::default();
    config.relay = Some(server::RelayConfig::new(addr));
    server::Server::spawn(config).await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "run with a dedicated Linux network namespace and a default-route switch"]
async fn sampler_wakes_once_on_a_same_ip_route_change() {
    let signal_directory =
        std::env::var_os("MIOPUNCH_DEMO_ROUTE_FIXTURE").expect("fixture directory");
    timeout(
        Duration::from_secs(45),
        exercise_sampler(PathBuf::from(signal_directory)),
    )
    .await
    .unwrap();
}

async fn exercise_sampler(signal_directory: PathBuf) {
    let directory = FixtureDirectory::new();
    let (certs, tls) = server::testing::self_signed_tls_certs_and_config();
    let local = (Ipv4Addr::LOCALHOST, 0);
    let mut observer = server::QuicConfig::new(local);
    observer.server_config = Some(tls);
    let mut qad_config = server::ServerConfig::default();
    qad_config.quic = Some(observer);
    let qad = server::Server::spawn(qad_config).await.unwrap();
    let ingress = UdpSocket::bind((Ipv4Addr::LOCALHOST, iroh::defaults::DEFAULT_RELAY_QUIC_PORT))
        .await
        .unwrap();
    let old_mapping = UdpSocket::bind(local).await.unwrap();
    let new_mapping = UdpSocket::bind(local).await.unwrap();
    let old_addr = old_mapping.local_addr().unwrap();
    let new_addr = new_mapping.local_addr().unwrap();
    let observer = qad.quic_addr().unwrap();
    old_mapping.connect(observer).await.unwrap();
    new_mapping.connect(observer).await.unwrap();
    let use_new_mapping = Arc::new(AtomicBool::new(false));
    let proxy = tokio::spawn(forward_qad(
        ingress,
        old_mapping,
        new_mapping,
        use_new_mapping.clone(),
    ));

    let mut reserved = [
        TcpListener::bind(local).await.unwrap(),
        TcpListener::bind(local).await.unwrap(),
    ];
    let url = |addr| format!("http://{addr}/").parse::<iroh::RelayUrl>().unwrap();
    reserved.sort_by_key(|listener| url(listener.local_addr().unwrap()));
    let [preferred, fallback] = reserved.map(|listener| listener.local_addr().unwrap());
    let relay = local_relay(preferred).await;
    let backup_relay = local_relay(fallback).await;
    let node = NativeNetwork::create(directory.0.join("node"))
        .await
        .unwrap();
    let mut config = node.resource_configuration().unwrap();
    config.replacement = Some(
        [preferred, fallback]
            .into_iter()
            .map(|addr| {
                let mut uses = BTreeSet::from([ResourceUse::DataRelay]);
                if addr == preferred {
                    uses.insert(ResourceUse::Observation);
                }
                miopunch_native_probe::resources::Resource {
                    url: url(addr).to_string(),
                    uses,
                    limitation: "owned network notification fixture".into(),
                }
            })
            .collect(),
    );
    node.save_resource_configuration(&config).unwrap();
    let network = Arc::new(
        node.maintain_network_with_endpoint(
            Ipv4Addr::UNSPECIFIED,
            |builder| builder.ca_tls_config(iroh_relay::tls::CaTlsConfig::custom_roots(certs)),
            |router| router,
        )
        .await
        .unwrap(),
    );
    let initial = network.endpoint().udp_network_generation();
    let mut maintenance = network.watch_maintenance();
    maintenance
        .wait_for(|status| status.state == miopunch_native_probe::network_maintenance::MaintenanceState::TemporarilyUnreachable)
        .await
        .unwrap();
    // Suspend periodic work after its initial step. Subsequent publications
    // witness commands handled by the maintainer, separately from sampler logs.
    let activity = network.business_activity();
    maintenance.changed().await.unwrap();
    let state = tests::probe_server();
    let sampler = tokio::spawn(sample(
        network.clone(),
        state.observations.clone(),
        directory.0.clone(),
        state.cancellation.clone(),
    ));
    let contains_ip_address = |address: &Value, expected: SocketAddr| {
        serde_json::from_value::<EndpointAddr>(address.clone())
            .unwrap()
            .ip_addrs()
            .any(|addr| *addr == expected)
    };
    loop {
        let sampled = state.observations.lock().unwrap().local["address"].clone();
        if !sampled.is_null() && contains_ip_address(&sampled, old_addr) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(network.addr().relay_urls().next(), Some(&url(preferred)));
    relay.shutdown().await.unwrap();
    wait_for_event(&state.observations, "local-address-changed", |detail| {
        serde_json::from_value::<EndpointAddr>(detail["after"].clone())
            .unwrap()
            .relay_urls()
            .any(|relay| *relay == url(fallback))
    })
    .await;
    use_new_mapping.store(true, Ordering::Relaxed);
    wait_for_event(&state.observations, "local-address-changed", |detail| {
        contains_ip_address(&detail["before"], old_addr)
            && contains_ip_address(&detail["after"], new_addr)
            && !contains_ip_address(&detail["after"], old_addr)
    })
    .await;
    assert!(initial.is_current());
    assert!(
        state
            .observations
            .lock()
            .unwrap()
            .events
            .iter()
            .all(|event| event["kind"] != "network-generation-changed")
    );
    assert!(
        timeout(Duration::from_secs(1), maintenance.changed())
            .await
            .is_err(),
        "advertisement-only changes woke the maintenance owner"
    );
    println!(
        "relay advertisement and periodic QAD port change {old_addr} -> {new_addr} kept the physical generation and did not wake maintenance"
    );

    let signal = signal_directory.join("noise-passed");
    std::fs::write(signal, b"switch the default route now\n").unwrap();
    wait_for_event(&state.observations, "network-generation-changed", |_| true).await;
    maintenance.changed().await.unwrap();
    assert!(!initial.is_current());
    let changed = network.endpoint().udp_network_generation();
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(changed.is_current());
    assert!(
        !maintenance.has_changed().unwrap(),
        "maintenance woke again after the route switch"
    );
    assert_eq!(
        state
            .observations
            .lock()
            .unwrap()
            .events
            .iter()
            .filter(|event| event["kind"] == "network-generation-changed")
            .count(),
        1
    );
    println!("same-IP default route switch woke the maintenance owner once");

    state.cancellation.cancel();
    sampler.await.unwrap().unwrap();
    drop(activity);
    Arc::try_unwrap(network).ok().unwrap().close().await;
    proxy.abort();
    let _ = proxy.await;
    let (qad_closed, relay_closed) = tokio::join!(qad.shutdown(), backup_relay.shutdown());
    qad_closed.unwrap();
    relay_closed.unwrap();
}
