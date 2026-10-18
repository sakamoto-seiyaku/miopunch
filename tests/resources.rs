//! Resource policy through public nodes, real admission, relay actors and TLS.
use std::{collections::BTreeSet, net::Ipv4Addr, time::Duration};

use iroh::{Endpoint, EndpointAddr, endpoint::presets};
use iroh_relay::server::{RelayConfig as ServerRelayConfig, Server, ServerConfig};
use miopunch_native_probe::control_protocol::AuthenticatedControl;
use miopunch_native_probe::{
    admission_gate::{ADMISSION_REQUEST_ALPN, ProtocolScope},
    manual_admission::{AdmissionStage, ManualExchange},
    native_network::NativeNetwork,
    resources::{Resource, ResourceConfig, ResourceTable, ResourceUse},
};

const BUSINESS: &[u8] = b"miopunch/resources-business/1";

async fn relay() -> Server {
    let mut config = ServerConfig::default();
    config.relay = Some(ServerRelayConfig::new((Ipv4Addr::LOCALHOST, 0)));
    Server::spawn(config).await.unwrap()
}

fn resource(url: &iroh::RelayUrl) -> Resource {
    Resource {
        url: url.to_string(),
        uses: BTreeSet::from([ResourceUse::DataRelay]),
        limitation:
            "controlled loopback HTTP relay; actual endpoint TLS/QUIC, no public availability claim"
                .into(),
    }
}

async fn bind(node: &NativeNetwork) -> iroh::endpoint::RestrictedEndpoint {
    let config = node.resource_configuration().unwrap();
    node.bind_endpoint_with(
        Endpoint::builder(presets::Minimal)
            .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
            .clear_ip_transports()
            .relay_mode(iroh::RelayMode::Custom(
                config.relay_map(ResourceUse::DataRelay).unwrap(),
            ))
            .alpns(vec![ADMISSION_REQUEST_ALPN.to_vec(), BUSINESS.to_vec()]),
    )
    .await
    .unwrap()
}

async fn connect(
    client: &iroh::endpoint::RestrictedEndpoint,
    server: &iroh::endpoint::RestrictedEndpoint,
    url: &iroh::RelayUrl,
    alpn: &[u8],
) -> (iroh::endpoint::Connection, iroh::endpoint::Connection) {
    tokio::time::timeout(Duration::from_secs(15), async {
        let (out, incoming) = tokio::join!(
            client.connect(
                EndpointAddr::new(server.id()).with_relay_url(url.clone()),
                alpn
            ),
            async { server.accept().await.unwrap().await },
        );
        (out.unwrap(), incoming.unwrap())
    })
    .await
    .expect("bounded real relay-only TLS connection")
}

async fn connect_control(
    client: &iroh::endpoint::RestrictedEndpoint,
    server: &iroh::endpoint::RestrictedEndpoint,
    url: &iroh::RelayUrl,
    alpn: &[u8],
) -> (
    std::sync::Arc<iroh::endpoint::ControlConnection>,
    std::sync::Arc<iroh::endpoint::ControlConnection>,
) {
    tokio::time::timeout(Duration::from_secs(15), async {
        let (out, incoming) = tokio::join!(
            client.connect_control(
                EndpointAddr::new(server.id()).with_relay_url(url.clone()),
                alpn
            ),
            async {
                server
                    .accept()
                    .await
                    .unwrap()
                    .accept_control()
                    .unwrap()
                    .await
            },
        );
        (
            std::sync::Arc::new(out.unwrap()),
            std::sync::Arc::new(incoming.unwrap()),
        )
    })
    .await
    .expect("bounded real relay-only TLS connection")
}

async fn admission_and_business(self_hosted: bool) {
    let service = relay().await;
    let url = service.http_url().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let mut joining = NativeNetwork::join(
        dir.path().join("joining"),
        admin.manual_invitation().unwrap(),
    )
    .await
    .unwrap();
    let mut config = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![resource(&url)],
    });
    if self_hosted {
        config = ResourceConfig::default();
        config.replacement = Some(vec![resource(&url)]);
    }
    admin.save_resource_configuration(&config).unwrap();
    joining.save_resource_configuration(&config).unwrap();
    let admin_endpoint = bind(&admin).await;
    let join_endpoint = bind(&joining).await;
    let (client, server) = connect_control(
        &join_endpoint,
        &admin_endpoint,
        &url,
        ADMISSION_REQUEST_ALPN,
    )
    .await;
    let (result, served) = tokio::join!(
        joining.request_manual_admission(&client),
        admin.serve_manual_admission(&server)
    );
    assert!(matches!(result.unwrap(), ManualExchange::AwaitingApproval));
    assert_eq!(served.unwrap(), AdmissionStage::AwaitingApproval);
    let request = admin.manual_requests().await.unwrap().pop().unwrap();
    admin.approve_manual_admission(&request).await.unwrap();
    let receive = async {
        let ManualExchange::Materials(materials) =
            joining.request_manual_admission(&client).await.unwrap()
        else {
            panic!("actual materials")
        };
        joining.complete_manual_admission(materials).await.unwrap()
    };
    let (saved, served) = tokio::join!(receive, admin.serve_manual_admission(&server));
    assert_eq!(saved, AdmissionStage::Complete);
    assert_eq!(served.unwrap(), AdmissionStage::Complete);
    client.close(0u32.into(), b"admitted");
    server.close(0u32.into(), b"admitted");
    join_endpoint.close().await;
    joining.close().await;
    let joining = NativeNetwork::open(dir.path().join("joining"))
        .await
        .unwrap();
    assert_eq!(joining.admission_stage(), AdmissionStage::Complete);
    admin
        .admission_gate()
        .register_protocol(BUSINESS, ProtocolScope::Member);
    joining
        .admission_gate()
        .register_protocol(BUSINESS, ProtocolScope::Member);
    let join_endpoint = bind(&joining).await;
    let (client, server) = connect(&join_endpoint, &admin_endpoint, &url, BUSINESS).await;
    assert!(
        joining
            .admission_gate()
            .allows(&client, ProtocolScope::Member)
    );
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"resource-configured member communication")
        .await
        .unwrap();
    send.finish().unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    assert_eq!(
        receive.read_to_end(128).await.unwrap(),
        b"resource-configured member communication"
    );
    eprintln!(
        "MIO38_REAL configuration={} admission=Complete reopened=true data=received relay={} server_metrics={:?}",
        if self_hosted {
            "self-hosted"
        } else {
            "default-test"
        },
        url,
        service.metrics()
    );
    client.close(0u32.into(), b"done");
    server.close(0u32.into(), b"done");
    join_endpoint.close().await;
    admin_endpoint.close().await;
    joining.close().await;
    admin.close().await;
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn first_real_default_test_configuration_admits_and_communicates() {
    admission_and_business(false).await;
}

#[tokio::test]
async fn first_real_self_hosted_replacement_admits_and_communicates() {
    admission_and_business(true).await;
}

#[tokio::test]
async fn priority_disabled_relay_from_address_book_is_never_accessed() {
    use p2panda_core::SigningKey;
    use p2panda_net::addrs::{NodeInfo, TrustedTransportInfo};
    let service = relay().await;
    let url = service.http_url().unwrap();
    let target = SigningKey::generate().verifying_key();
    let mut config = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![resource(&url)],
    });
    config
        .disabled
        .insert((url.to_string(), ResourceUse::DataRelay));
    let book = p2panda_net::AddressBook::builder().spawn().await.unwrap();
    let mut info = NodeInfo::new(target).bootstrap();
    info.update_transports(
        TrustedTransportInfo::from(
            EndpointAddr::new(iroh::EndpointId::from_bytes(target.as_bytes()).unwrap())
                .with_relay_url(url.clone()),
        )
        .into(),
    )
    .unwrap();
    book.insert_node_info(info).await.unwrap();
    let panda = p2panda_net::Endpoint::builder(book)
        .endpoint_builder_hook(move |builder| config.configure(builder).unwrap())
        .spawn()
        .await
        .unwrap();
    let raw = panda.endpoint().await.unwrap();
    let attempted =
        tokio::time::timeout(Duration::from_secs(3), panda.connect(target, BUSINESS)).await;
    assert!(!matches!(attempted, Ok(Ok(_))));
    let connections = service.metrics().server.http_connections.get();
    let relay_bytes = service.metrics().server.bytes_recv.get();
    eprintln!(
        "MIO38_DISABLED source=address-book http_connections={connections} relay_bytes={relay_bytes}"
    );
    raw.close().await;
    drop(panda);
    service.shutdown().await.unwrap();
    assert_eq!(connections, 0, "the disabled relay must not even be dialed");
    assert_eq!(relay_bytes, 0, "no datagram may reach the disabled relay");
}

#[tokio::test]
async fn disabled_relay_in_default_startup_table_is_never_accessed() {
    let service = relay().await;
    let url = service.http_url().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let node = NativeNetwork::create(dir.path().join("node"))
        .await
        .unwrap();
    let mut config = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![resource(&url)],
    });
    config
        .disabled
        .insert((url.to_string(), ResourceUse::DataRelay));
    node.save_resource_configuration(&config).unwrap();
    // An independently supplied builder still contains the disabled URL. It
    // cannot turn that table into permission to probe or dial the resource.
    let endpoint = node
        .bind_endpoint_with(
            Endpoint::builder(presets::Minimal)
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .relay_mode(iroh::RelayMode::Custom(
                    iroh::RelayConfig::from(url.clone()).into(),
                )),
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let connections = service.metrics().server.http_connections.get();
    eprintln!("MIO38_DISABLED source=default-table http_connections={connections}");
    endpoint.close().await;
    node.close().await;
    service.shutdown().await.unwrap();
    assert_eq!(
        connections, 0,
        "startup observation must obey its separate permission too"
    );
}

#[tokio::test]
async fn disabled_relay_from_invitation_hint_is_never_accessed() {
    let service = relay().await;
    let url = service.http_url().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let invitation = admin.manual_invitation().unwrap();
    let joining = NativeNetwork::join(dir.path().join("joining"), invitation)
        .await
        .unwrap();
    let mut config = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![resource(&url)],
    });
    config
        .disabled
        .insert((url.to_string(), ResourceUse::DataRelay));
    joining.save_resource_configuration(&config).unwrap();
    let endpoint = joining
        .bind_endpoint_with(
            Endpoint::builder(presets::Minimal)
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .relay_mode(iroh::RelayMode::Custom(iroh::RelayMap::empty())),
        )
        .await
        .unwrap();
    let target = EndpointAddr::new(invitation.contact).with_relay_url(url);
    let attempted = tokio::time::timeout(
        Duration::from_secs(2),
        endpoint.connect_control(target, ADMISSION_REQUEST_ALPN),
    )
    .await;
    assert!(!matches!(attempted, Ok(Ok(_))));
    let connections = service.metrics().server.http_connections.get();
    let bytes = service.metrics().server.bytes_recv.get();
    eprintln!(
        "MIO38_DISABLED source=invitation-hint http_connections={connections} relay_bytes={bytes}"
    );
    endpoint.close().await;
    joining.close().await;
    admin.close().await;
    service.shutdown().await.unwrap();
    assert_eq!((connections, bytes), (0, 0));
}

#[tokio::test]
async fn disabled_relay_from_lookup_result_is_never_accessed() {
    let service = relay().await;
    let url = service.http_url().unwrap();
    let target = iroh::SecretKey::generate().public();
    let mut config = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![resource(&url)],
    });
    config
        .disabled
        .insert((url.to_string(), ResourceUse::DataRelay));
    let lookup = iroh::address_lookup::memory::MemoryLookup::from_endpoint_info([
        EndpointAddr::new(target).with_relay_url(url),
    ]);
    let endpoint = config
        .configure(
            Endpoint::builder(presets::Minimal)
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .relay_mode(iroh::RelayMode::Custom(iroh::RelayMap::empty()))
                .address_lookup(lookup),
        )
        .unwrap()
        .bind()
        .await
        .unwrap();
    let attempted =
        tokio::time::timeout(Duration::from_secs(2), endpoint.connect(target, BUSINESS)).await;
    assert!(!matches!(attempted, Ok(Ok(_))));
    let connections = service.metrics().server.http_connections.get();
    let bytes = service.metrics().server.bytes_recv.get();
    eprintln!(
        "MIO38_DISABLED source=lookup-result http_connections={connections} relay_bytes={bytes}"
    );
    endpoint.close().await;
    service.shutdown().await.unwrap();
    assert_eq!((connections, bytes), (0, 0));
}

#[tokio::test]
async fn disabled_relay_retained_in_cache_is_never_accessed() {
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    let service = relay().await;
    let url = service.http_url().unwrap();
    let target = iroh::SecretKey::generate().public();
    let mut config = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![resource(&url)],
    });
    config
        .disabled
        .insert((url.to_string(), ResourceUse::DataRelay));
    let denied = Arc::new(AtomicU64::new(0));
    let observed = denied.clone();
    let policy = iroh::endpoint::RelayResourcePolicy::new(move |url, usage| {
        let usage = match usage {
            iroh::endpoint::RelayResourceUse::DataRelay => ResourceUse::DataRelay,
            iroh::endpoint::RelayResourceUse::Observation => ResourceUse::Observation,
            iroh::endpoint::RelayResourceUse::Signaling => ResourceUse::Signaling,
            iroh::endpoint::RelayResourceUse::Lookup => ResourceUse::Lookup,
        };
        let allowed = config.allows(url.as_str(), usage);
        if !allowed && usage == ResourceUse::DataRelay {
            observed.fetch_add(1, Ordering::Relaxed);
        }
        allowed
    });
    let endpoint = Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .relay_mode(iroh::RelayMode::Custom(iroh::RelayMap::empty()))
        .relay_resource_policy(policy)
        .bind()
        .await
        .unwrap();
    let hinted = EndpointAddr::new(target).with_relay_url(url.clone());
    let _ = tokio::time::timeout(Duration::from_secs(1), endpoint.connect(hinted, BUSINESS)).await;
    let cached = endpoint.remote_info(target).await.unwrap();
    assert!(
        cached
            .addrs()
            .any(|info| info.addr() == &iroh::TransportAddr::Relay(url.clone()))
    );
    endpoint.address_lookup().unwrap().clear();
    assert!(endpoint.address_lookup().unwrap().is_empty());
    let before = denied.load(Ordering::Relaxed);
    assert!(before > 0);
    let attempted =
        tokio::time::timeout(Duration::from_secs(2), endpoint.connect(target, BUSINESS)).await;
    assert!(!matches!(attempted, Ok(Ok(_))));
    assert!(
        denied.load(Ordering::Relaxed) > before,
        "the identity-only retry must reach actual dispatch using the cached candidate"
    );
    let connections = service.metrics().server.http_connections.get();
    let bytes = service.metrics().server.bytes_recv.get();
    eprintln!(
        "MIO38_DISABLED source=cache cached=true lookup_services=0 dispatch_denials={} http_connections={connections} relay_bytes={bytes}",
        denied.load(Ordering::Relaxed)
    );
    endpoint.close().await;
    service.shutdown().await.unwrap();
    assert_eq!((connections, bytes), (0, 0));
}

#[tokio::test]
async fn a_known_actor_route_cannot_replace_an_allowed_candidate_with_a_forbidden_relay() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    };
    let old_service = relay().await;
    let alternate_service = relay().await;
    let old = old_service.http_url().unwrap();
    let alternate = alternate_service.http_url().unwrap();
    let denied = Arc::new(AtomicBool::new(false));
    let denied_checks = Arc::new(AtomicU64::new(0));
    let alternate_checks = Arc::new(AtomicU64::new(0));
    let client_policy = {
        let denied = denied.clone();
        let denied_checks = denied_checks.clone();
        let alternate_checks = alternate_checks.clone();
        let old = old.clone();
        let alternate = alternate.clone();
        iroh::endpoint::RelayResourcePolicy::new(move |url, usage| {
            if !matches!(usage, iroh::endpoint::RelayResourceUse::DataRelay) {
                return false;
            }
            if url == &old {
                if denied.load(Ordering::Relaxed) {
                    denied_checks.fetch_add(1, Ordering::Relaxed);
                    return false;
                }
                return true;
            }
            if url == &alternate {
                alternate_checks.fetch_add(1, Ordering::Relaxed);
                return true;
            }
            false
        })
    };
    let map: iroh::RelayMap = iroh::RelayConfig::from(old.clone()).into();
    let client = Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .clear_ip_transports()
        .relay_mode(iroh::RelayMode::Custom(map.clone()))
        .relay_resource_policy(client_policy)
        .alpns(vec![BUSINESS.to_vec()])
        .bind()
        .await
        .unwrap();
    let server_old = old.clone();
    let server = Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .clear_ip_transports()
        .relay_mode(iroh::RelayMode::Custom(map))
        .relay_resource_policy(iroh::endpoint::RelayResourcePolicy::new(
            move |url, usage| {
                url == &server_old && matches!(usage, iroh::endpoint::RelayResourceUse::DataRelay)
            },
        ))
        .alpns(vec![BUSINESS.to_vec()])
        .bind()
        .await
        .unwrap();
    let (out, incoming) = tokio::join!(
        client.connect(
            EndpointAddr::new(server.id()).with_relay_url(old.clone()),
            BUSINESS
        ),
        async { server.accept().await.unwrap().await }
    );
    let (out, incoming) = (out.unwrap(), incoming.unwrap());
    let (mut send, _) = out.open_bi().await.unwrap();
    send.write_all(b"warm the actual authenticated endpoint route")
        .await
        .unwrap();
    send.finish().unwrap();
    let (_, mut recv) = incoming.accept_bi().await.unwrap();
    assert_eq!(
        recv.read_to_end(128).await.unwrap(),
        b"warm the actual authenticated endpoint route"
    );
    out.close(0u32.into(), b"fixture route warm");
    incoming.close(0u32.into(), b"fixture route warm");
    let _ = tokio::join!(out.closed(), incoming.closed());
    tokio::time::sleep(Duration::from_millis(500)).await;
    // This mutable low-level predicate is only a route-injection fixture. The
    // product's ResourceConfig snapshots do not implement a runtime hot update.
    denied.store(true, Ordering::Relaxed);
    let before = old_service.metrics().server.bytes_recv.get();
    let attempted = tokio::time::timeout(
        Duration::from_secs(2),
        client.connect(
            EndpointAddr::new(server.id()).with_relay_url(alternate.clone()),
            BUSINESS,
        ),
    )
    .await;
    assert!(!matches!(attempted, Ok(Ok(_))));
    let after = old_service.metrics().server.bytes_recv.get();
    let new_access = alternate_service.metrics().server.http_connections.get();
    eprintln!(
        "MIO38_ACTOR_ROUTE warm_real_tls=true forbidden_before={before} forbidden_after={after} denied_checks={} candidate_checks={} candidate_http={new_access}",
        denied_checks.load(Ordering::Relaxed),
        alternate_checks.load(Ordering::Relaxed)
    );
    client.close().await;
    server.close().await;
    old_service.shutdown().await.unwrap();
    alternate_service.shutdown().await.unwrap();
    assert!(before > 0);
    assert_eq!(
        after, before,
        "a warm known actor route must not carry another datagram after denial"
    );
    assert!(denied_checks.load(Ordering::Relaxed) > 0);
    assert!(
        alternate_checks.load(Ordering::Relaxed) > 0,
        "the different allowed candidate must reach the actor"
    );
    assert!(
        new_access > 0,
        "the allowed candidate is actually dialed instead of the forbidden known route"
    );
}

#[tokio::test]
async fn data_relay_disable_preserves_independently_allowed_address_observation() {
    use iroh::Watcher;
    let (certs, tls) = iroh_relay::server::testing::self_signed_tls_certs_and_config();
    let mut quic = iroh_relay::server::QuicConfig::new((Ipv4Addr::LOCALHOST, 0));
    quic.server_config = Some(tls);
    let mut server_config = ServerConfig::default();
    server_config.quic = Some(quic);
    let service = Server::spawn(server_config).await.unwrap();
    let url: iroh::RelayUrl = "https://127.0.0.1/".parse().unwrap();
    let mut entry = resource(&url);
    entry.uses.insert(ResourceUse::Observation);
    let mut config = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![entry],
    });
    config
        .disabled
        .insert((url.to_string(), ResourceUse::DataRelay));
    let observers: iroh::RelayMap = iroh::RelayConfig::new(
        url,
        Some(iroh_relay::RelayQuicConfig::new(
            service.quic_addr().unwrap().port(),
        )),
    )
    .into();
    let endpoint = config
        .configure(
            Endpoint::builder(presets::Minimal)
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .clear_ip_transports()
                .bind_addr((Ipv4Addr::LOCALHOST, 0))
                .unwrap()
                .relay_mode(iroh::RelayMode::Custom(iroh::RelayMap::empty()))
                .observation_servers(observers)
                .ca_tls_config(iroh_relay::tls::CaTlsConfig::custom_roots(certs)),
        )
        .unwrap()
        .bind()
        .await
        .unwrap();
    let mut watch = endpoint.udp_observation();
    let observation = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let state = watch.get();
            if state.is_fresh() {
                break state;
            }
            watch.updated().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert!(endpoint.home_relay_status().get().is_empty());
    let metrics = service.metrics().clone();
    endpoint.close().await;
    service.shutdown().await.unwrap();
    let incoming = metrics.server.qad_incoming.get();
    let connections = metrics.server.qad_connections.get();
    let http = metrics.server.http_connections.get();
    let data = metrics.server.bytes_recv.get();
    eprintln!(
        "MIO38_INDEPENDENT_USE data_relay=disabled observation={observation:?} actual_qad_incoming={incoming} established_qad={connections} relay_http={http} relay_bytes={data}"
    );
    assert!(incoming > 0);
    assert_eq!((http, data), (0, 0));
}

#[tokio::test]
async fn external_mapping_observer_map_cannot_bypass_permission() {
    let (certs, tls) = iroh_relay::server::testing::self_signed_tls_certs_and_config();
    let mut services = Vec::new();
    let mut entries = Vec::new();
    let mut observers = Vec::new();
    for index in 0..3 {
        let mut quic = iroh_relay::server::QuicConfig::new((Ipv4Addr::LOCALHOST, 0));
        quic.server_config = Some(tls.clone());
        let mut server_config = ServerConfig::default();
        server_config.quic = Some(quic);
        let service = Server::spawn(server_config).await.unwrap();
        let url: iroh::RelayUrl = format!("https://127.0.0.1:{}/", 44300 + index)
            .parse()
            .unwrap();
        let mut entry = resource(&url);
        entry.uses = BTreeSet::from([ResourceUse::Observation]);
        entries.push(entry);
        observers.push(iroh::RelayConfig::new(
            url,
            Some(iroh_relay::RelayQuicConfig::new(
                service.quic_addr().unwrap().port(),
            )),
        ));
        services.push(service);
    }
    let forbidden_url = observers[0].url.clone();
    let forbidden_destination = services[0].quic_addr().unwrap();
    let mut config = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: entries,
    });
    config
        .disabled
        .insert((forbidden_url.to_string(), ResourceUse::Observation));
    let endpoint = config
        .configure(
            Endpoint::builder(presets::Minimal)
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .clear_ip_transports()
                .bind_addr((Ipv4Addr::LOCALHOST, 0))
                .unwrap()
                .observation_servers(iroh::RelayMap::empty())
                .ca_tls_config(iroh_relay::tls::CaTlsConfig::custom_roots(certs)),
        )
        .unwrap()
        .bind()
        .await
        .unwrap();
    let bound = endpoint.bound_sockets();
    let mapping = endpoint
        .observe_udp_mapping_sequence(&iroh::RelayMap::from_iter(observers))
        .await
        .unwrap();
    assert_eq!(mapping.samples.len(), 2);
    assert!(
        mapping
            .samples
            .iter()
            .all(|sample| sample.observer != forbidden_url.to_string()
                && sample.destination != forbidden_destination)
    );
    assert!(
        mapping
            .samples
            .iter()
            .all(|sample| bound.contains(&sample.mapped))
    );
    assert_eq!(
        endpoint.bound_sockets(),
        bound,
        "observation retains the original real QUIC socket"
    );
    let metrics: Vec<_> = services
        .iter()
        .map(|service| service.metrics().clone())
        .collect();
    endpoint.close().await;
    for service in services {
        service.shutdown().await.unwrap();
    }
    let counts: Vec<_> = metrics
        .iter()
        .map(|metrics| metrics.server.qad_incoming.get())
        .collect();
    eprintln!(
        "MIO38_MAPPING external_map=3 allowed_samples=2 actual_qad_incoming={counts:?} original_socket={bound:?}"
    );
    assert_eq!(counts[0], 0);
    assert!(counts[1] > 0 && counts[2] > 0);
}

#[tokio::test]
async fn trusted_table_update_preserves_local_replacement_and_disable_on_restart() {
    use miopunch_native_probe::resources::SignedResourceTable;
    use p2panda_core::SigningKey;
    let disabled = relay().await;
    let local = relay().await;
    let new_default = relay().await;
    let disabled_url = disabled.http_url().unwrap();
    let local_url = local.http_url().unwrap();
    let new_url = new_default.http_url().unwrap();
    let publisher = SigningKey::generate();
    let mut config = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![resource(&disabled_url)],
    });
    config.trusted_update_key = Some(publisher.verifying_key());
    config.replacement = Some(vec![resource(&local_url), resource(&disabled_url)]);
    config
        .disabled
        .insert((disabled_url.to_string(), ResourceUse::DataRelay));
    let table = ResourceTable {
        revision: 2,
        resources: vec![resource(&disabled_url), resource(&new_url)],
    };
    let signed = SignedResourceTable::sign(&publisher, &table).unwrap();
    config.apply_signed_update(&signed).unwrap();
    assert_eq!(config.defaults.revision, 2);
    assert!(config.allows(local_url.as_str(), ResourceUse::DataRelay));
    assert!(!config.allows(disabled_url.as_str(), ResourceUse::DataRelay));
    assert!(!config.allows(new_url.as_str(), ResourceUse::DataRelay));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("node");
    let node = NativeNetwork::create(&path).await.unwrap();
    node.save_resource_configuration(&config).unwrap();
    node.close().await;
    let node = NativeNetwork::open(&path).await.unwrap();
    assert_eq!(node.resource_configuration().unwrap(), config);
    let configured = node
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let endpoint = configured.endpoint();
    tokio::time::timeout(Duration::from_secs(5), endpoint.online())
        .await
        .unwrap();
    assert_eq!(
        endpoint.addr().relay_urls().cloned().collect::<Vec<_>>(),
        vec![local_url]
    );
    let actual = [
        disabled.metrics().clone(),
        local.metrics().clone(),
        new_default.metrics().clone(),
    ];
    configured.close().await;
    node.close().await;
    disabled.shutdown().await.unwrap();
    local.shutdown().await.unwrap();
    new_default.shutdown().await.unwrap();
    let accesses: Vec<_> = actual
        .iter()
        .map(|metrics| metrics.server.http_connections.get())
        .collect();
    eprintln!(
        "MIO38_UPDATE signature=verified revision=2 restarted=true http_connections_disabled_local_newdefault={accesses:?}"
    );
    assert_eq!(accesses[0], 0);
    assert!(accesses[1] > 0);
    assert_eq!(accesses[2], 0);
}

#[tokio::test]
async fn equivalent_url_spelling_cannot_bypass_a_local_disable() {
    let service = relay().await;
    let url = service.http_url().unwrap();
    let mut config = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![resource(&url)],
    });
    config.disabled.insert((
        url.as_str().trim_end_matches('/').to_owned(),
        ResourceUse::DataRelay,
    ));
    let dir = tempfile::tempdir().unwrap();
    let node = NativeNetwork::create(dir.path().join("node"))
        .await
        .unwrap();
    node.save_resource_configuration(&config).unwrap();
    let configured = node
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    let metrics = service.metrics().clone();
    configured.close().await;
    node.close().await;
    service.shutdown().await.unwrap();
    let connections = metrics.server.http_connections.get();
    eprintln!("MIO38_URL_ALIAS disabled_without_slash=true actual_http_connections={connections}");
    assert_eq!(connections, 0);
}

#[test]
fn an_update_signed_by_an_untrusted_publisher_cannot_change_configuration() {
    use miopunch_native_probe::resources::SignedResourceTable;
    use p2panda_core::SigningKey;
    let publisher = SigningKey::generate();
    let stranger = SigningKey::generate();
    let mut config = ResourceConfig::default();
    config.trusted_update_key = Some(publisher.verifying_key());
    config.disabled_uses.insert(ResourceUse::DataRelay);
    let before = config.clone();
    let proposed = ResourceTable {
        revision: 2,
        resources: vec![],
    };
    let forged = SignedResourceTable::sign(&stranger, &proposed).unwrap();
    assert_eq!(
        config.apply_signed_update(&forged).unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_eq!(config, before);
}

#[test]
fn tampering_and_signed_rollback_cannot_change_configuration() {
    use miopunch_native_probe::resources::SignedResourceTable;
    use p2panda_core::SigningKey;
    let publisher = SigningKey::generate();
    let mut config = ResourceConfig::default();
    config.trusted_update_key = Some(publisher.verifying_key());
    config.disabled_uses.insert(ResourceUse::Signaling);
    let before = config.clone();
    let mut tampered = SignedResourceTable::sign(
        &publisher,
        &ResourceTable {
            revision: 2,
            resources: vec![],
        },
    )
    .unwrap();
    *tampered.statement.last_mut().unwrap() ^= 1;
    assert!(config.apply_signed_update(&tampered).is_err());
    assert_eq!(config, before);
    let rollback = SignedResourceTable::sign(
        &publisher,
        &ResourceTable {
            revision: 1,
            resources: vec![],
        },
    )
    .unwrap();
    assert!(config.apply_signed_update(&rollback).is_err());
    assert_eq!(config, before);
}

struct FailedFacility {
    url: iroh::RelayUrl,
    connections: std::sync::Arc<std::sync::atomic::AtomicU64>,
    worker: tokio::task::JoinHandle<()>,
}

impl FailedFacility {
    async fn new() -> Self {
        use std::sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        };
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let connections = Arc::new(AtomicU64::new(0));
        let count = connections.clone();
        let worker = tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                count.fetch_add(1, Ordering::Relaxed);
                drop(socket);
            }
        });
        Self {
            url,
            connections,
            worker,
        }
    }

    async fn close(self) {
        self.worker.abort();
        let _ = self.worker.await;
    }
}

#[tokio::test]
async fn all_allowed_entries_fail_with_bounded_backoff_and_import_restart_recovery() {
    use miopunch_native_probe::resources::ResourceContactError;
    use std::sync::atomic::Ordering;
    let first = FailedFacility::new().await;
    let second = FailedFacility::new().await;
    let forbidden = relay().await;
    let allowed = relay().await;
    let forbidden_url = forbidden.http_url().unwrap();
    let allowed_url = allowed.http_url().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let invitation = admin.manual_invitation().unwrap();
    let joining_path = dir.path().join("joining");
    let joining = NativeNetwork::join(&joining_path, invitation)
        .await
        .unwrap();
    let mut config = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![
            resource(&first.url),
            resource(&second.url),
            resource(&forbidden_url),
        ],
    });
    config
        .disabled
        .insert((forbidden_url.to_string(), ResourceUse::DataRelay));
    joining.save_resource_configuration(&config).unwrap();
    let contact = joining
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap()
        .with_contact_timeout(Duration::from_millis(250))
        .unwrap();
    let started = std::time::Instant::now();
    let error = contact
        .connect_control(
            EndpointAddr::new(invitation.contact),
            ADMISSION_REQUEST_ALPN,
        )
        .await
        .unwrap_err();
    let ResourceContactError::TemporarilyUnreachable { retry_after, .. } = error else {
        panic!("temporary reachability, never a global offline assertion")
    };
    assert!(retry_after > Duration::ZERO && retry_after <= Duration::from_secs(32));
    assert!(started.elapsed() < Duration::from_secs(2));
    let before = [
        first.connections.load(Ordering::Relaxed),
        second.connections.load(Ordering::Relaxed),
    ];
    assert!(
        before.iter().all(|count| *count > 0),
        "all allowed facilities must actually have failed: {before:?}"
    );
    let retry = contact
        .connect_control(
            EndpointAddr::new(invitation.contact),
            ADMISSION_REQUEST_ALPN,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        retry,
        ResourceContactError::TemporarilyUnreachable { .. }
    ));
    assert_eq!(
        [
            first.connections.load(Ordering::Relaxed),
            second.connections.load(Ordering::Relaxed)
        ],
        before,
        "a bounded-backoff call must not retry the failed entries"
    );
    contact.close().await;
    // A personally trusted import changes only the selected resource list.
    config
        .import_trusted_replacement(vec![resource(&allowed_url), resource(&forbidden_url)])
        .unwrap();
    joining.save_resource_configuration(&config).unwrap();
    joining.close().await;
    let mut joining = NativeNetwork::open(&joining_path).await.unwrap();
    let mut admin_config = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![resource(&allowed_url)],
    });
    admin_config.disabled_uses.insert(ResourceUse::Observation);
    admin.save_resource_configuration(&admin_config).unwrap();
    let admin_contact = admin
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let contact = joining
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let server_endpoint = admin_contact.endpoint();
    let (client, server) = tokio::join!(
        contact.connect_control(
            EndpointAddr::new(invitation.contact),
            ADMISSION_REQUEST_ALPN
        ),
        async {
            std::sync::Arc::new(
                server_endpoint
                    .accept()
                    .await
                    .unwrap()
                    .accept_control()
                    .unwrap()
                    .await
                    .unwrap(),
            )
        }
    );
    let client = client.unwrap();
    let (requested, served) = tokio::join!(
        joining.request_manual_admission(&client),
        admin.serve_manual_admission(&server)
    );
    assert!(matches!(
        requested.unwrap(),
        ManualExchange::AwaitingApproval
    ));
    assert_eq!(served.unwrap(), AdmissionStage::AwaitingApproval);
    assert_eq!(joining.admission_stage(), AdmissionStage::AwaitingApproval);
    let forbidden_access = forbidden.metrics().server.http_connections.get();
    eprintln!(
        "MIO38_UNREACHABLE actual_failed_entries={before:?} retry_after={retry_after:?} repeat_backed_off=true trusted_import_restarted=true real_authenticated_contact_recovered=true forbidden_http={forbidden_access}"
    );
    client.close(0u32.into(), b"done");
    server.close(0u32.into(), b"done");
    contact.close().await;
    admin_contact.close().await;
    joining.close().await;
    admin.close().await;
    first.close().await;
    second.close().await;
    forbidden.shutdown().await.unwrap();
    allowed.shutdown().await.unwrap();
    assert_eq!(forbidden_access, 0);
}

#[tokio::test]
async fn failed_initial_data_only_relay_falls_back_to_another_allowed_entry() {
    use std::sync::atomic::Ordering;
    let failed = FailedFacility::new().await;
    let service = relay().await;
    // Force the failed numeric host to sort before the live localhost entry.
    let allowed: iroh::RelayUrl =
        format!("http://localhost:{}/", service.http_addr().unwrap().port())
            .parse()
            .unwrap();
    let config = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![resource(&failed.url), resource(&allowed)],
    });
    assert_eq!(
        config
            .relay_map(ResourceUse::DataRelay)
            .unwrap()
            .urls::<Vec<_>>()[0],
        failed.url
    );
    let dir = tempfile::tempdir().unwrap();
    let node = NativeNetwork::create(dir.path().join("node"))
        .await
        .unwrap();
    node.save_resource_configuration(&config).unwrap();
    let contact = node
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let online = tokio::time::timeout(Duration::from_secs(3), contact.endpoint().online()).await;
    let advertised = contact
        .endpoint()
        .addr()
        .relay_urls()
        .cloned()
        .collect::<Vec<_>>();
    let actual_failed = failed.connections.load(Ordering::Relaxed);
    let actual_live = service.metrics().server.accepts.get();
    eprintln!(
        "MIO38_STARTUP_FALLBACK online={} actual_failed_entry_connections={actual_failed} actual_allowed_relay_accepts={actual_live} advertised={advertised:?}",
        online.is_ok()
    );
    contact.close().await;
    node.close().await;
    failed.close().await;
    service.shutdown().await.unwrap();
    assert!(
        online.is_ok(),
        "observation permission is not required to reach another allowed data relay"
    );
    assert!(actual_failed > 0 && actual_live > 0);
    assert_eq!(advertised, vec![allowed]);
}

#[tokio::test]
async fn resource_choices_survive_the_public_node_backup_and_restore() {
    use miopunch_native_probe::resources::LanDiscovery;
    use p2panda_core::SigningKey;
    let dir = tempfile::tempdir().unwrap();
    let node = NativeNetwork::create(dir.path().join("node"))
        .await
        .unwrap();
    let mut config = ResourceConfig::default();
    config.replacement = Some(vec![]);
    config.disabled_uses.insert(ResourceUse::DataRelay);
    config.lan_discovery = LanDiscovery::Passive;
    config.trusted_update_key = Some(SigningKey::generate().verifying_key());
    node.save_resource_configuration(&config).unwrap();
    let backup = dir.path().join("backup");
    node.create_backup(&backup).await.unwrap();
    let restored = NativeNetwork::restore_backup(&backup, dir.path().join("restored"))
        .await
        .unwrap();
    let actual = restored.resource_configuration().unwrap();
    restored.close().await;
    assert_eq!(
        actual, config,
        "restore must not re-enable bundled facilities or widen LAN publication"
    );
}

#[tokio::test]
async fn a_disabled_signaling_broker_cannot_be_opened_by_a_custom_carrier() {
    use miopunch_native_probe::mqtt_transport::{MqttTransport, Traffic};
    use std::sync::{Arc, atomic::Ordering};
    let facility = FailedFacility::new().await;
    let broker: std::net::SocketAddr = format!("127.0.0.1:{}", facility.url.port().unwrap())
        .parse()
        .unwrap();
    let uri = format!("mqtt://{broker}/");
    let mut config = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![Resource {
            url: uri.clone(),
            uses: BTreeSet::from([ResourceUse::Signaling]),
            limitation: "controlled signaling fixture".into(),
        }],
    });
    config.disabled.insert((uri, ResourceUse::Signaling));
    let dir = tempfile::tempdir().unwrap();
    let node = NativeNetwork::create(dir.path().join("node"))
        .await
        .unwrap();
    node.save_resource_configuration(&config).unwrap();
    let carrier = Arc::new(MqttTransport::new(broker, Arc::new(Traffic::default())));
    let bound = node
        .bind_endpoint_with(
            Endpoint::builder(presets::Minimal)
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .clear_ip_transports()
                .clear_relay_transports()
                .add_custom_transport(carrier),
        )
        .await;
    let rejected = bound.is_err();
    tokio::time::sleep(Duration::from_millis(100)).await;
    if let Ok(endpoint) = bound {
        endpoint.close().await;
    }
    let actual = facility.connections.load(Ordering::Relaxed);
    eprintln!(
        "MIO38_SIGNALING disabled=true endpoint_bind_rejected={rejected} actual_broker_tcp_connections={actual}"
    );
    node.close().await;
    facility.close().await;
    assert!(rejected);
    assert_eq!(actual, 0);
}

#[tokio::test]
async fn explicit_lan_disclosure_publishes_only_active_nodes_and_passive_discovers() {
    use futures_util::StreamExt;
    use iroh_mdns_address_lookup::{DiscoveryEvent, MdnsAddressLookup};
    use miopunch_native_probe::resources::LanDiscovery;
    let spy = MdnsAddressLookup::builder()
        .advertise(false)
        .service_name("miopunchv1")
        .build(iroh::SecretKey::generate().public())
        .unwrap();
    let mut events = spy.subscribe().await;
    let dir = tempfile::tempdir().unwrap();
    let mut active = NativeNetwork::create(dir.path().join("active"))
        .await
        .unwrap();
    let invitation = active.manual_invitation().unwrap();
    let mut passive = NativeNetwork::join(dir.path().join("passive"), invitation)
        .await
        .unwrap();
    let disabled = NativeNetwork::create(dir.path().join("disabled"))
        .await
        .unwrap();
    let mut config = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![],
    });
    assert_eq!(config.lan_discovery, LanDiscovery::Disabled);
    disabled.save_resource_configuration(&config).unwrap();
    config.lan_discovery = LanDiscovery::Active;
    active.save_resource_configuration(&config).unwrap();
    config.lan_discovery = LanDiscovery::Passive;
    passive.save_resource_configuration(&config).unwrap();
    let active_contact = active
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let passive_contact = passive
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let disabled_contact = disabled
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let active_id = active_contact.endpoint().id();
    let passive_id = passive_contact.endpoint().id();
    let disabled_id = disabled_contact.endpoint().id();
    assert!(
        disabled_contact
            .endpoint()
            .address_lookup()
            .unwrap()
            .is_empty()
    );
    let published = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(DiscoveryEvent::Discovered { endpoint_info, .. }) = events.next().await {
                assert_ne!(endpoint_info.endpoint_id, passive_id);
                assert_ne!(endpoint_info.endpoint_id, disabled_id);
                if endpoint_info.endpoint_id == active_id {
                    break;
                }
            }
        }
    })
    .await;
    if published.is_ok() {
        // The common coordinator consumes explicit contact candidates. Resolve
        // them through this Passive node's actual configured mDNS service.
        let mut discovered = Box::pin(
            passive_contact
                .endpoint()
                .address_lookup()
                .unwrap()
                .resolve(active_id),
        );
        let target = tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(item) = discovered.next().await {
                if let Ok(Ok(item)) = item {
                    let candidate = item.into_endpoint_addr();
                    if candidate.id == active_id && !candidate.addrs.is_empty() {
                        return candidate;
                    }
                }
            }
            panic!("Passive mDNS lookup ended without its advertised peer");
        })
        .await
        .expect("Passive node discovers the actual active publication");
        let (client, server) = tokio::time::timeout(Duration::from_secs(15), async {
            tokio::join!(
                passive_contact.connect_control(target, ADMISSION_REQUEST_ALPN),
                async {
                    active_contact
                        .endpoint()
                        .accept()
                        .await
                        .unwrap()
                        .accept_control()
                        .unwrap()
                        .await
                        .map(std::sync::Arc::new)
                        .unwrap()
                }
            )
        })
        .await
        .expect("bounded mDNS candidate control connection");
        let client = client.unwrap();
        let (requested, served) = tokio::join!(
            passive.request_manual_admission(&client),
            active.serve_manual_admission(&server)
        );
        assert!(matches!(
            requested.unwrap(),
            ManualExchange::AwaitingApproval
        ));
        assert_eq!(served.unwrap(), AdmissionStage::AwaitingApproval);
        client.close(0u32.into(), b"done");
        server.close(0u32.into(), b"done");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while let Ok(Some(event)) = tokio::time::timeout_at(deadline, events.next()).await {
            if let DiscoveryEvent::Discovered { endpoint_info, .. } = event {
                assert_ne!(endpoint_info.endpoint_id, passive_id);
                assert_ne!(endpoint_info.endpoint_id, disabled_id);
            }
        }
    }
    eprintln!(
        "MIO38_LAN active_wire_publication={} passive_published=false default_disabled_lookup_count={}",
        published.is_ok(),
        disabled_contact.endpoint().address_lookup().unwrap().len()
    );
    active_contact.close().await;
    passive_contact.close().await;
    disabled_contact.close().await;
    active.close().await;
    passive.close().await;
    disabled.close().await;
    drop(spy);
    assert!(
        published.is_ok(),
        "explicit Active must actually publish; Passive and Disabled must not publish"
    );
}

#[tokio::test]
async fn p2panda_high_level_active_publishes_while_its_passive_branch_is_inactive() {
    use futures_util::StreamExt;
    use iroh_mdns_address_lookup::{DiscoveryEvent, MdnsAddressLookup};
    use p2panda_net::{MdnsDiscovery, iroh_mdns::MdnsDiscoveryMode};
    let spy = MdnsAddressLookup::builder()
        .advertise(false)
        .service_name("p2pandav1")
        .build(iroh::SecretKey::generate().public())
        .unwrap();
    let mut events = spy.subscribe().await;
    let book_a = p2panda_net::AddressBook::builder().spawn().await.unwrap();
    let book_p = p2panda_net::AddressBook::builder().spawn().await.unwrap();
    let endpoint_a = p2panda_net::Endpoint::builder(book_a.clone())
        .endpoint_builder_hook(|builder| {
            builder
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .clear_relay_transports()
        })
        .spawn()
        .await
        .unwrap();
    let endpoint_p = p2panda_net::Endpoint::builder(book_p.clone())
        .endpoint_builder_hook(|builder| {
            builder
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .clear_relay_transports()
        })
        .spawn()
        .await
        .unwrap();
    let raw_a = endpoint_a.endpoint().await.unwrap();
    let raw_p = endpoint_p.endpoint().await.unwrap();
    assert_eq!(MdnsDiscoveryMode::default(), MdnsDiscoveryMode::Passive);
    // The upper p2panda Network selects Active; the reusable actor defaults to
    // Passive and currently returns before starting its low-level service.
    let mdns_p = MdnsDiscovery::builder(book_p.clone(), endpoint_p.clone())
        .spawn()
        .await
        .unwrap();
    let mdns_a = MdnsDiscovery::builder(book_a.clone(), endpoint_a.clone())
        .mode(MdnsDiscoveryMode::Active)
        .spawn()
        .await
        .unwrap();
    let active_publication = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(DiscoveryEvent::Discovered { endpoint_info, .. }) = events.next().await {
                assert_ne!(endpoint_info.endpoint_id, raw_p.id());
                if endpoint_info.endpoint_id == raw_a.id() {
                    break;
                }
            }
        }
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let passive_received = book_p
        .node_info(endpoint_a.node_id())
        .await
        .unwrap()
        .is_some();
    eprintln!(
        "MIO38_HIGH_LAN explicit_active_wire_publication={} passive_default_discovered={passive_received}",
        active_publication.is_ok()
    );
    drop(mdns_a);
    drop(mdns_p);
    raw_a.close().await;
    raw_p.close().await;
    drop(endpoint_a);
    drop(endpoint_p);
    drop(spy);
    assert!(active_publication.is_ok());
    assert!(
        !passive_received,
        "the high-level Passive branch is currently inactive, not receive-only discovery"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn cli_import_update_and_startup_preserve_local_resource_choices() {
    use miopunch_native_probe::resources::{LanDiscovery, SignedResourceTable};
    use tokio::io::{AsyncBufReadExt, BufReader};
    let binary = env!("CARGO_BIN_EXE_miopunch-native-probe");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("node");
    let node = NativeNetwork::create(&path).await.unwrap();
    let publisher = p2panda_core::SigningKey::generate();
    let mut config = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![],
    });
    config.disabled_uses.insert(ResourceUse::DataRelay);
    config.trusted_update_key = Some(publisher.verifying_key());
    node.save_resource_configuration(&config).unwrap();
    node.close().await;
    let imported = vec![resource(&"http://127.0.0.1:1".parse().unwrap())];
    let import_path = dir.path().join("import.json");
    std::fs::write(&import_path, serde_json::to_vec(&imported).unwrap()).unwrap();
    let output = tokio::process::Command::new(binary)
        .args(["network", "resources-import"])
        .arg(&path)
        .arg(&import_path)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let update_path = dir.path().join("update.json");
    let table = ResourceTable {
        revision: 2,
        resources: vec![resource(&"http://127.0.0.1:2".parse().unwrap())],
    };
    std::fs::write(
        &update_path,
        serde_json::to_vec(&SignedResourceTable::sign(&publisher, &table).unwrap()).unwrap(),
    )
    .unwrap();
    let output = tokio::process::Command::new(binary)
        .args(["network", "resources-update"])
        .arg(&path)
        .arg(&update_path)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = tokio::process::Command::new(binary)
        .args(["network", "resources"])
        .arg(&path)
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    let saved: ResourceConfig = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(saved.defaults, table);
    assert_eq!(saved.replacement, Some(imported));
    assert_eq!(saved.disabled_uses, config.disabled_uses);
    assert_eq!(saved.trusted_update_key, config.trusted_update_key);
    assert_eq!(saved.lan_discovery, LanDiscovery::Disabled);
    let mut child = tokio::process::Command::new(binary)
        .args(["network", "run"])
        .arg(&path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let ready = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(ready.starts_with("READY "));
    rustix::process::kill_process(
        rustix::process::Pid::from_raw(child.id().unwrap() as i32).unwrap(),
        rustix::process::Signal::INT,
    )
    .unwrap();
    let exit = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(exit.success());
    eprintln!(
        "MIO38_CLI import_saved=true signed_update_saved=true local_choices_preserved=true next_start_ready=true actual_child_exit={exit}"
    );
}

struct PkarrFixture {
    url: iroh::RelayUrl,
    requests: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    packets: std::sync::Arc<std::sync::Mutex<std::collections::BTreeMap<String, Vec<u8>>>>,
    task: tokio::task::JoinHandle<()>,
}

impl PkarrFixture {
    async fn start() -> Self {
        Self::start_with_redirect(None).await
    }

    async fn start_with_redirect(redirect: Option<String>) -> Self {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let url = format!("http://{}/pkarr", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let packets = std::sync::Arc::new(std::sync::Mutex::new(std::collections::BTreeMap::<
            String,
            Vec<u8>,
        >::new()));
        let recorded = requests.clone();
        let stored = packets.clone();
        let task = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut bytes = Vec::new();
                let end = loop {
                    let mut part = [0; 4096];
                    let count = stream.read(&mut part).await.unwrap();
                    if count == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&part[..count]);
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        break end + 4;
                    }
                    assert!(bytes.len() < 8192);
                };
                let header = String::from_utf8(bytes[..end].to_vec()).unwrap();
                let request = header.lines().next().unwrap().to_owned();
                recorded.lock().unwrap().push(request.clone());
                let length = header
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|value| value.parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                assert!(length < 65536);
                while bytes.len() < end + length {
                    let mut part = [0; 4096];
                    let count = stream.read(&mut part).await.unwrap();
                    if count == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&part[..count]);
                }
                let fields: Vec<_> = request.split_whitespace().collect();
                let (status, body) = if redirect.is_some() {
                    ("302 Found", vec![])
                } else {
                    match fields[0] {
                        "PUT" => {
                            stored
                                .lock()
                                .unwrap()
                                .insert(fields[1].to_owned(), bytes[end..end + length].to_vec());
                            ("200 OK", vec![])
                        }
                        "GET" => match stored.lock().unwrap().get(fields[1]).cloned() {
                            Some(body) => ("200 OK", body),
                            None => ("404 Not Found", vec![]),
                        },
                        _ => panic!("unexpected fixture method"),
                    }
                };
                let location = redirect
                    .as_ref()
                    .map(|url| format!("Location: {url}\r\n"))
                    .unwrap_or_default();
                let response = format!(
                    "HTTP/1.1 {status}\r\n{location}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
                stream.write_all(&body).await.unwrap();
            }
        });
        Self {
            url,
            requests,
            packets,
            task,
        }
    }

    async fn close(self) -> Vec<String> {
        self.task.abort();
        let _ = self.task.await;
        self.requests.lock().unwrap().clone()
    }
}

#[tokio::test]
async fn disabled_pkarr_lookup_facility_is_never_accessed() {
    use iroh::address_lookup::PkarrResolver;
    let facility = PkarrFixture::start().await;
    let dir = tempfile::tempdir().unwrap();
    let node = NativeNetwork::create(dir.path().join("node"))
        .await
        .unwrap();
    let mut config = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![Resource {
            url: facility.url.to_string(),
            uses: BTreeSet::from([ResourceUse::Lookup]),
            limitation: "controlled HTTP lookup".into(),
        }],
    });
    config
        .disabled
        .insert((facility.url.to_string(), ResourceUse::Lookup));
    node.save_resource_configuration(&config).unwrap();
    let endpoint = node
        .bind_endpoint_with(
            Endpoint::builder(presets::Minimal)
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .address_lookup(PkarrResolver::builder(
                    std::ops::Deref::deref(&facility.url).clone(),
                )),
        )
        .await
        .unwrap();
    let attempted = tokio::time::timeout(
        Duration::from_secs(1),
        endpoint.connect_control(iroh::SecretKey::generate().public(), ADMISSION_REQUEST_ALPN),
    )
    .await;
    assert!(!matches!(attempted, Ok(Ok(_))));
    endpoint.close().await;
    node.close().await;
    let accesses = facility.close().await;
    eprintln!("MIO38_LOOKUP disabled=true actual_http_requests={accesses:?}");
    assert!(
        accesses.is_empty(),
        "a disabled lookup facility must not be queried"
    );
}

#[tokio::test]
async fn disabled_pkarr_publication_facility_is_never_accessed() {
    use iroh::address_lookup::{AddrFilter, PkarrPublisher};
    let facility = PkarrFixture::start().await;
    let dir = tempfile::tempdir().unwrap();
    let node = NativeNetwork::create(dir.path().join("node"))
        .await
        .unwrap();
    let mut config = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![],
    });
    config.disabled_uses.insert(ResourceUse::Lookup);
    node.save_resource_configuration(&config).unwrap();
    let endpoint = node
        .bind_endpoint_with(
            Endpoint::builder(presets::Minimal)
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .address_lookup(
                    PkarrPublisher::builder(std::ops::Deref::deref(&facility.url).clone())
                        .addr_filter(AddrFilter::unfiltered())
                        .republish_interval(Duration::from_millis(50)),
                ),
        )
        .await
        .unwrap();
    endpoint.set_user_data_for_address_lookup(Some("changed".parse().unwrap()));
    tokio::time::sleep(Duration::from_millis(300)).await;
    endpoint.close().await;
    node.close().await;
    let requests = facility.close().await;
    eprintln!("MIO38_LOOKUP publication_disabled=true actual_http_requests={requests:?}");
    assert!(
        requests.is_empty(),
        "a disabled lookup facility must not receive identity publication or refreshes"
    );
}

#[tokio::test]
async fn disabled_lookup_added_after_bind_through_wrappers_is_never_accessed() {
    use iroh::address_lookup::{AddrFilter, FilteredAddressLookup, PkarrPublisher, PkarrResolver};
    let facility = PkarrFixture::start().await;
    let dir = tempfile::tempdir().unwrap();
    let node = NativeNetwork::create(dir.path().join("node"))
        .await
        .unwrap();
    let mut config = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![],
    });
    config.disabled_uses.insert(ResourceUse::Lookup);
    node.save_resource_configuration(&config).unwrap();
    let endpoint = node
        .bind_endpoint_with(
            Endpoint::builder(presets::Minimal)
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled),
        )
        .await
        .unwrap();
    let lookup = endpoint
        .pkarr_resolver(PkarrResolver::builder(
            std::ops::Deref::deref(&facility.url).clone(),
        ))
        .unwrap();
    let lookup = std::sync::Arc::new(FilteredAddressLookup::new(lookup, AddrFilter::unfiltered()));
    endpoint.address_lookup().unwrap().add(lookup);
    let publisher = endpoint
        .pkarr_publisher(
            PkarrPublisher::builder(std::ops::Deref::deref(&facility.url).clone())
                .addr_filter(AddrFilter::unfiltered()),
        )
        .unwrap();
    endpoint
        .address_lookup()
        .unwrap()
        .add(std::sync::Arc::new(FilteredAddressLookup::new(
            publisher,
            AddrFilter::unfiltered(),
        )));
    let attempted = tokio::time::timeout(
        Duration::from_secs(1),
        endpoint.connect_control(iroh::SecretKey::generate().public(), ADMISSION_REQUEST_ALPN),
    )
    .await;
    assert!(!matches!(attempted, Ok(Ok(_))));
    endpoint.close().await;
    node.close().await;
    let requests = facility.close().await;
    eprintln!(
        "MIO38_LOOKUP added_after_bind=true filtered_arc_wrappers=true actual_http_requests={requests:?}"
    );
    assert!(requests.is_empty());
}

#[tokio::test]
async fn allowed_signed_pkarr_lookup_reaches_real_admission_with_data_relay_disabled() {
    use iroh::address_lookup::{AddrFilter, PkarrPublisher, PkarrResolver};
    let facility = PkarrFixture::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let mut joining = NativeNetwork::join(
        dir.path().join("joining"),
        admin.manual_invitation().unwrap(),
    )
    .await
    .unwrap();
    let mut config = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![Resource {
            url: facility.url.to_string(),
            uses: BTreeSet::from([ResourceUse::Lookup]),
            limitation: "controlled pkarr store forwards actual signed packets".into(),
        }],
    });
    config.disabled_uses.insert(ResourceUse::DataRelay);
    admin.save_resource_configuration(&config).unwrap();
    joining.save_resource_configuration(&config).unwrap();
    let url = std::ops::Deref::deref(&facility.url).clone();
    let server = admin
        .bind_endpoint_with(
            Endpoint::builder(presets::Minimal)
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .alpns(vec![ADMISSION_REQUEST_ALPN.to_vec()])
                .bind_addr((Ipv4Addr::LOCALHOST, 0))
                .unwrap()
                .address_lookup(
                    PkarrPublisher::builder(url.clone()).addr_filter(AddrFilter::unfiltered()),
                ),
        )
        .await
        .unwrap();
    let client = joining
        .bind_endpoint_with(
            Endpoint::builder(presets::Minimal)
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .alpns(vec![ADMISSION_REQUEST_ALPN.to_vec()])
                .bind_addr((Ipv4Addr::LOCALHOST, 0))
                .unwrap()
                .address_lookup(PkarrResolver::builder(url)),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while facility.packets.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("actual signed identity/address publication");
    let (outgoing, incoming) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(
            client.connect_control(server.id(), ADMISSION_REQUEST_ALPN),
            async {
                server
                    .accept()
                    .await
                    .unwrap()
                    .accept_control()
                    .unwrap()
                    .await
            }
        )
    })
    .await
    .unwrap();
    let outgoing = std::sync::Arc::new(outgoing.unwrap());
    let incoming = std::sync::Arc::new(incoming.unwrap());
    let (requested, served) = tokio::join!(
        joining.request_manual_admission(&outgoing),
        admin.serve_manual_admission(&incoming)
    );
    assert!(matches!(
        requested.unwrap(),
        ManualExchange::AwaitingApproval
    ));
    assert_eq!(served.unwrap(), AdmissionStage::AwaitingApproval);
    outgoing.close(0u32.into(), b"done");
    incoming.close(0u32.into(), b"done");
    client.close().await;
    server.close().await;
    joining.close().await;
    admin.close().await;
    let requests = facility.close().await;
    assert!(requests.iter().any(|request| request.starts_with("PUT ")));
    assert!(requests.iter().any(|request| request.starts_with("GET ")));
    eprintln!(
        "MIO38_LOOKUP allowed_signed_pkarr=true data_relay_disabled=true id_only_tls=true actual_admission=AwaitingApproval actual_http_requests={requests:?}"
    );
}

#[tokio::test]
async fn allowed_lookup_cannot_redirect_access_to_a_disabled_facility() {
    use iroh::address_lookup::PkarrResolver;
    let forbidden = PkarrFixture::start().await;
    let redirecting = PkarrFixture::start_with_redirect(Some(forbidden.url.to_string())).await;
    let dir = tempfile::tempdir().unwrap();
    let node = NativeNetwork::create(dir.path().join("node"))
        .await
        .unwrap();
    let entries = [&redirecting.url, &forbidden.url]
        .into_iter()
        .map(|url| Resource {
            url: url.to_string(),
            uses: BTreeSet::from([ResourceUse::Lookup]),
            limitation: "controlled HTTP facility".into(),
        })
        .collect();
    let mut config = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: entries,
    });
    config
        .disabled
        .insert((forbidden.url.to_string(), ResourceUse::Lookup));
    node.save_resource_configuration(&config).unwrap();
    let endpoint = node
        .bind_endpoint_with(
            Endpoint::builder(presets::Minimal)
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .address_lookup(PkarrResolver::builder(
                    std::ops::Deref::deref(&redirecting.url).clone(),
                )),
        )
        .await
        .unwrap();
    let attempted = tokio::time::timeout(
        Duration::from_secs(1),
        endpoint.connect_control(iroh::SecretKey::generate().public(), ADMISSION_REQUEST_ALPN),
    )
    .await;
    assert!(!matches!(attempted, Ok(Ok(_))));
    endpoint.close().await;
    node.close().await;
    let allowed_requests = redirecting.close().await;
    let forbidden_requests = forbidden.close().await;
    eprintln!(
        "MIO38_LOOKUP redirect_allowed_requests={allowed_requests:?} redirect_forbidden_requests={forbidden_requests:?}"
    );
    assert!(!allowed_requests.is_empty());
    assert!(
        forbidden_requests.is_empty(),
        "an allowed facility must not substitute a forbidden redirect destination"
    );
}

#[tokio::test]
async fn explicit_pkarr_client_rechecks_permission_before_each_request() {
    use iroh::address_lookup::{EndpointData, EndpointInfo, PkarrRelayClient};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let facility = PkarrFixture::start().await;
    let endpoint = Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .bind()
        .await
        .unwrap();
    let allowed = Arc::new(AtomicBool::new(true));
    let check = allowed.clone();
    let url = facility.url.clone();
    let client = PkarrRelayClient::builder(
        std::ops::Deref::deref(&url).clone(),
        endpoint.tls_config().clone(),
        endpoint.dns_resolver().unwrap().clone(),
    )
    .resource_policy(iroh::endpoint::RelayResourcePolicy::new(
        move |resource, usage| {
            *resource == url
                && matches!(usage, iroh::endpoint::RelayResourceUse::Lookup)
                && check.load(Ordering::SeqCst)
        },
    ))
    .build();
    let key = iroh::SecretKey::generate();
    let packet = EndpointInfo::from_parts(key.public(), EndpointData::default())
        .to_pkarr_signed_packet(&key, 30)
        .unwrap();
    client.publish(&packet).await.unwrap();
    assert_eq!(
        client
            .resolve(key.public())
            .await
            .unwrap()
            .to_relay_payload(),
        packet.to_relay_payload()
    );
    allowed.store(false, Ordering::SeqCst);
    assert!(client.publish(&packet).await.is_err());
    assert!(client.resolve(key.public()).await.is_err());
    endpoint.close().await;
    let requests = facility.close().await;
    assert_eq!(requests.len(), 2, "revoked requests must stop before HTTP");
}

#[tokio::test]
async fn allowed_pkarr_publication_cannot_redirect_on_retry_or_reconnect() {
    use iroh::address_lookup::PkarrPublisher;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let forbidden = PkarrFixture::start().await;
    let redirecting = PkarrFixture::start_with_redirect(Some(forbidden.url.to_string())).await;
    let dir = tempfile::tempdir().unwrap();
    let node = NativeNetwork::create(dir.path().join("node"))
        .await
        .unwrap();
    let config = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![Resource {
            url: redirecting.url.to_string(),
            uses: BTreeSet::from([ResourceUse::Lookup]),
            limitation: "owned redirect service; forbidden origin omitted".into(),
        }],
    });
    node.save_resource_configuration(&config).unwrap();
    let allowed = Arc::new(AtomicBool::new(true));
    let check = allowed.clone();
    let endpoint = node
        .bind_endpoint_with(
            Endpoint::builder(presets::Minimal)
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .address_lookup(
                    PkarrPublisher::builder(std::ops::Deref::deref(&redirecting.url).clone())
                        .resource_policy(iroh::endpoint::RelayResourcePolicy::new(move |_, _| {
                            check.load(Ordering::SeqCst)
                        })),
                ),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
        while redirecting.requests.lock().unwrap().len() < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("real failed publication retried on a new HTTP connection");
    allowed.store(false, Ordering::SeqCst);
    let before = redirecting.requests.lock().unwrap().len();
    endpoint.set_user_data_for_address_lookup(Some("permission-changed".parse().unwrap()));
    tokio::time::sleep(Duration::from_millis(2200)).await;
    assert_eq!(
        redirecting.requests.lock().unwrap().len(),
        before,
        "revocation must apply to new publication and background retries"
    );
    endpoint.close().await;
    node.close().await;
    let permitted_requests = redirecting.close().await;
    let forbidden_requests = forbidden.close().await;
    assert!(permitted_requests.len() >= 2);
    assert!(
        permitted_requests
            .iter()
            .all(|request| request.starts_with("PUT "))
    );
    assert!(
        forbidden_requests.is_empty(),
        "every retry/reconnect must keep redirects disabled"
    );
}

#[tokio::test]
async fn pkarr_builder_preserves_both_endpoint_and_caller_restrictions() {
    use iroh::address_lookup::{PkarrPublisher, PkarrResolver};
    for endpoint_allows in [false, true] {
        let facility = PkarrFixture::start().await;
        let dir = tempfile::tempdir().unwrap();
        let node = NativeNetwork::create(dir.path().join("node"))
            .await
            .unwrap();
        let config = ResourceConfig::with_defaults(ResourceTable {
            revision: 1,
            resources: if endpoint_allows {
                vec![Resource {
                    url: facility.url.to_string(),
                    uses: BTreeSet::from([ResourceUse::Lookup]),
                    limitation: "owned lookup fixture".into(),
                }]
            } else {
                vec![]
            },
        });
        node.save_resource_configuration(&config).unwrap();
        let policy = iroh::endpoint::RelayResourcePolicy::new(move |_, _| !endpoint_allows);
        let url = std::ops::Deref::deref(&facility.url).clone();
        let endpoint = node
            .bind_endpoint_with(
                Endpoint::builder(presets::Minimal)
                    .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                    .address_lookup(
                        PkarrPublisher::builder(url.clone()).resource_policy(policy.clone()),
                    )
                    .address_lookup(PkarrResolver::builder(url).resource_policy(policy)),
            )
            .await
            .unwrap();
        let attempted = tokio::time::timeout(
            Duration::from_millis(300),
            endpoint.connect_control(iroh::SecretKey::generate().public(), ADMISSION_REQUEST_ALPN),
        )
        .await;
        assert!(!matches!(attempted, Ok(Ok(_))));
        endpoint.close().await;
        node.close().await;
        assert!(
            facility.close().await.is_empty(),
            "neither policy may broaden the other"
        );
    }
}

#[tokio::test]
async fn prebuilt_unconstrained_pkarr_cannot_bypass_product_policy() {
    use iroh::address_lookup::{AddrFilter, FilteredAddressLookup, PkarrPublisher, PkarrResolver};
    use std::sync::Arc;
    let ordinary = Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .bind()
        .await
        .unwrap();
    for before_bind in [true, false] {
        let forbidden = PkarrFixture::start().await;
        let redirecting = PkarrFixture::start_with_redirect(Some(forbidden.url.to_string())).await;
        let dir = tempfile::tempdir().unwrap();
        let node = NativeNetwork::create(dir.path().join("node"))
            .await
            .unwrap();
        node.save_resource_configuration(&ResourceConfig::with_defaults(ResourceTable {
            revision: 1,
            resources: vec![Resource {
                url: redirecting.url.to_string(),
                uses: BTreeSet::from([ResourceUse::Lookup]),
                limitation: "owned redirect origin".into(),
            }],
        }))
        .unwrap();
        let url = std::ops::Deref::deref(&redirecting.url).clone();
        let publisher = Arc::new(FilteredAddressLookup::new(
            PkarrPublisher::builder(url.clone())
                .build(ordinary.secret_key().clone(), ordinary.tls_config().clone()),
            AddrFilter::unfiltered(),
        ));
        let resolver = Arc::new(FilteredAddressLookup::new(
            PkarrResolver::builder(url).build(ordinary.tls_config().clone()),
            AddrFilter::unfiltered(),
        ));
        let mut builder = Endpoint::builder(presets::Minimal)
            .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled);
        if before_bind {
            builder = builder
                .address_lookup(publisher.clone())
                .address_lookup(resolver.clone());
        }
        let endpoint = node.bind_endpoint_with(builder).await.unwrap();
        if !before_bind {
            endpoint.address_lookup().unwrap().add(publisher.clone());
            endpoint.address_lookup().unwrap().add(resolver.clone());
        }
        let attempted = tokio::time::timeout(
            Duration::from_millis(300),
            endpoint.connect_control(iroh::SecretKey::generate().public(), ADMISSION_REQUEST_ALPN),
        )
        .await;
        assert!(!matches!(attempted, Ok(Ok(_))));
        endpoint.close().await;
        drop(publisher);
        drop(resolver);
        node.close().await;
        assert!(
            redirecting.close().await.is_empty(),
            "ordinary prebuilt services require explicit constraints before product registration"
        );
        assert!(forbidden.close().await.is_empty());
    }
    ordinary.close().await;
}
