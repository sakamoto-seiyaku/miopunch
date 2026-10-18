//! Real gateway protocol and kernel forwarding are driven by udp_portmap_lab.py.
use std::net::Ipv4Addr;
use std::time::Duration;

use iroh::{EndpointAddr, TransportAddr};
use miopunch_native_probe::admission_gate::{ADMISSION_REQUEST_ALPN, ProtocolScope};
use miopunch_native_probe::manual_admission::ManualExchange;
use miopunch_native_probe::native_network::NativeNetwork;
use miopunch_native_probe::resources::PortMapping;

const BUSINESS: &[u8] = b"miopunch/udp-portmap-business/1";

// Worker is inert unless the isolated lab invokes this exact public test entry.
#[test]
fn gateway_node_worker() {
    let Ok(role) = std::env::var("MIO39_ROLE") else {
        return;
    };
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async move {
            let directory = std::path::PathBuf::from(std::env::var("MIO39_DIRECTORY").unwrap());
            if role == "prepare" {
                let admin = NativeNetwork::create(directory.join("admin"))
                    .await
                    .unwrap();
                let invitation = admin.manual_invitation().unwrap();
                let joining = NativeNetwork::join(directory.join("joining"), invitation)
                    .await
                    .unwrap();
                let case = std::env::var("MIO39_CASE").unwrap_or_default();
                for (node, mapping) in [
                    (
                        &admin,
                        if case == "disabled" {
                            PortMapping::Disabled
                        } else {
                            PortMapping::NatPmp
                        },
                    ),
                    (&joining, PortMapping::Disabled),
                ] {
                    let mut config = node.resource_configuration().unwrap();
                    config.replacement = Some(vec![]);
                    config.port_mapping = mapping;
                    node.save_resource_configuration(&config).unwrap();
                }
                std::fs::write(
                    directory.join("admin-id"),
                    admin.status().node_id.to_string(),
                )
                .unwrap();
                joining.close().await;
                admin.close().await;
                return;
            }
            let mut node = NativeNetwork::open(directory.join(
                if matches!(
                    role.as_str(),
                    "serve" | "disabled" | "allocation-timeout" | "allocation-cancel"
                ) {
                    "admin"
                } else {
                    "joining"
                },
            ))
            .await
            .unwrap();
            node.admission_gate()
                .register_protocol(BUSINESS, ProtocolScope::Member);
            let endpoint = node
                .bind_configured_endpoint(Ipv4Addr::UNSPECIFIED)
                .await
                .unwrap();
            endpoint
                .endpoint()
                .set_alpns(vec![ADMISSION_REQUEST_ALPN.to_vec(), BUSINESS.to_vec()]);
            if matches!(role.as_str(), "allocation-timeout" | "allocation-cancel") {
                tokio::time::timeout(Duration::from_secs(3), async {
                    while !directory.join("address-seen").exists() {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .expect("gateway must acknowledge allocation before the address query is lost");
                if role == "allocation-timeout" {
                    tokio::time::sleep(Duration::from_millis(750)).await;
                }
                assert!(
                    !endpoint
                        .endpoint()
                        .addr()
                        .ip_addrs()
                        .any(|addr| addr.ip().to_string() == "100.64.39.1")
                );
                let started = std::time::Instant::now();
                endpoint.endpoint().close().await;
                assert!(
                    started.elapsed() < Duration::from_millis(1500),
                    "close must remain bounded while querying gateway"
                );
                println!(
                    "ALLOCATION_CLOSED case={role} seconds={:.3}",
                    started.elapsed().as_secs_f64()
                );
                node.close().await;
                return;
            }
            if role == "disabled" {
                tokio::time::sleep(Duration::from_secs(1)).await;
                assert!(
                    !endpoint
                        .endpoint()
                        .addr()
                        .ip_addrs()
                        .any(|addr| addr.ip().to_string() == "100.64.39.1")
                );
                println!("MAPPING_DISABLED relay=false");
                endpoint.endpoint().close().await;
                node.close().await;
                return;
            }
            if matches!(role.as_str(), "reconnect" | "fallback") {
                let id = std::fs::read_to_string(directory.join("admin-id"))
                    .unwrap()
                    .parse()
                    .unwrap();
                let address = std::fs::read_to_string(directory.join(if role == "fallback" {
                    "internal"
                } else {
                    "mapped"
                }))
                .unwrap()
                .parse()
                .unwrap();
                let connection = endpoint
                    .endpoint()
                    .connect(EndpointAddr::new(id).with_ip_addr(address), BUSINESS)
                    .await
                    .unwrap();
                assert!(
                    node.admission_gate()
                        .allows(&connection, ProtocolScope::Member)
                );
                let (mut send, mut recv) = connection.open_bi().await.unwrap();
                send.write_all(b"authorized via actual UDP mapping")
                    .await
                    .unwrap();
                send.finish().unwrap();
                assert_eq!(
                    recv.read_to_end(128).await.unwrap(),
                    b"authorized reply via actual UDP mapping"
                );
                connection.close(0u32.into(), b"done");
                println!("RECONNECT_OK role={role} real_member=true relay=false");
                endpoint.endpoint().close().await;
                node.close().await;
                return;
            }
            if role == "unreachable" {
                let id = std::fs::read_to_string(directory.join("admin-id"))
                    .unwrap()
                    .parse()
                    .unwrap();
                let address = std::fs::read_to_string(directory.join("mapped"))
                    .unwrap()
                    .parse()
                    .unwrap();
                let target = EndpointAddr::new(id).with_ip_addr(address);
                let result = tokio::time::timeout(
                    Duration::from_secs(2),
                    endpoint.endpoint().connect(target, BUSINESS),
                )
                .await;
                assert!(
                    !matches!(result, Ok(Ok(_))),
                    "deleted mapping must not admit a fresh connection"
                );
                println!("NEW_CONNECTION_UNAVAILABLE endpoint_still_running=true");
                std::fs::write(directory.join("probe-complete"), b"").unwrap();
                endpoint.endpoint().close().await;
                node.close().await;
                return;
            }
            if role == "serve" {
                let expected: std::net::SocketAddr = "100.64.39.1:41039".parse().unwrap();
                tokio::time::timeout(Duration::from_secs(8), async {
                    loop {
                        if endpoint
                            .endpoint()
                            .addr()
                            .ip_addrs()
                            .any(|addr| *addr == expected)
                        {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                })
                .await
                .expect("real NAT-PMP candidate must appear without relay");
                assert!(endpoint.endpoint().addr().relay_urls().next().is_none());
                println!(
                    "MAPPED candidate={expected} bound={:?}",
                    endpoint.endpoint().bound_sockets()
                );
                std::fs::write(directory.join("mapped"), expected.to_string()).unwrap();
                let internal_port = endpoint
                    .endpoint()
                    .bound_sockets()
                    .iter()
                    .find(|addr| addr.is_ipv4())
                    .unwrap()
                    .port();
                std::fs::write(
                    directory.join("internal"),
                    format!("10.39.0.2:{internal_port}"),
                )
                .unwrap();
                let connection = std::sync::Arc::new(
                    endpoint
                        .endpoint()
                        .accept()
                        .await
                        .unwrap()
                        .accept_control()
                        .unwrap()
                        .await
                        .unwrap(),
                );
                node.serve_manual_admission(&connection).await.unwrap();
                let request = node.manual_requests().await.unwrap().pop().unwrap();
                node.approve_manual_admission(&request).await.unwrap();
                std::fs::write(directory.join("approved"), b"").unwrap();
                node.serve_manual_admission(&connection).await.unwrap();
                connection.closed().await;
                let business = endpoint.endpoint().accept().await.unwrap().await.unwrap();
                assert!(
                    node.admission_gate()
                        .allows(&business, ProtocolScope::Member)
                );
                let (mut send, mut recv) = business.accept_bi().await.unwrap();
                assert_eq!(
                    recv.read_to_end(128).await.unwrap(),
                    b"authorized via actual UDP mapping"
                );
                send.write_all(b"authorized reply via actual UDP mapping")
                    .await
                    .unwrap();
                send.finish().unwrap();
                business.closed().await;
                println!("DATA_OK real_member=true relay=false");
                std::fs::write(directory.join("data-ready"), b"").unwrap();
                let case = std::env::var("MIO39_CASE").unwrap_or_default();
                if case == "renewal-address-failure" {
                    tokio::time::timeout(Duration::from_secs(6), async {
                        while !directory.join("mapping-cleaned").exists() {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                    })
                    .await
                    .expect("renewed allocation must be cleaned after its address query fails");
                    tokio::time::timeout(Duration::from_millis(600), async {
                        while endpoint
                            .endpoint()
                            .addr()
                            .ip_addrs()
                            .any(|addr| *addr == expected)
                        {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                    })
                    .await
                    .expect("a mapping we explicitly deleted must no longer be advertised");
                    println!("CANDIDATE_WITHDRAWN_AFTER_CLEANUP old={expected}");
                    std::fs::write(directory.join("withdrawn"), b"").unwrap();
                    tokio::time::timeout(Duration::from_secs(5), async {
                        while !directory.join("probe-complete").exists() {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                    })
                    .await
                    .unwrap();
                }
                if case == "expired" {
                    tokio::time::timeout(Duration::from_secs(9), async {
                        loop {
                            if !endpoint
                                .endpoint()
                                .addr()
                                .ip_addrs()
                                .any(|addr| *addr == expected)
                            {
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(25)).await;
                        }
                    })
                    .await
                    .expect("failed renewal must withdraw the lease candidate");
                    println!("CANDIDATE_WITHDRAWN old={expected}");
                    std::fs::write(directory.join("withdrawn"), b"").unwrap();
                    tokio::time::timeout(Duration::from_secs(5), async {
                        while !directory.join("probe-complete").exists() {
                            tokio::time::sleep(Duration::from_millis(25)).await;
                        }
                    })
                    .await
                    .unwrap();
                }
                if matches!(case.as_str(), "changed" | "restart") {
                    let changed: std::net::SocketAddr = "100.64.39.1:42039".parse().unwrap();
                    tokio::time::timeout(Duration::from_secs(6), async {
                        while !endpoint
                            .endpoint()
                            .addr()
                            .ip_addrs()
                            .any(|addr| *addr == changed)
                        {
                            tokio::time::sleep(Duration::from_millis(25)).await;
                        }
                    })
                    .await
                    .expect("renewal after change/restart must publish replacement mapping");
                    assert!(
                        !endpoint
                            .endpoint()
                            .addr()
                            .ip_addrs()
                            .any(|addr| *addr == expected)
                    );
                    std::fs::write(directory.join("mapped"), changed.to_string()).unwrap();
                    println!("CANDIDATE_REPLACED old={expected} new={changed} case={case}");
                    std::fs::write(directory.join("changed"), b"").unwrap();
                }
                if case == "renewed" {
                    tokio::time::sleep(Duration::from_secs(7)).await;
                    assert!(
                        endpoint
                            .endpoint()
                            .addr()
                            .ip_addrs()
                            .any(|addr| *addr == expected)
                    );
                    println!("LEASE_RENEWED candidate={expected}");
                    std::fs::write(directory.join("renewed"), b"").unwrap();
                }
                if matches!(case.as_str(), "expired" | "changed" | "restart" | "renewed") {
                    let business = endpoint.endpoint().accept().await.unwrap().await.unwrap();
                    assert!(
                        node.admission_gate()
                            .allows(&business, ProtocolScope::Member)
                    );
                    let (mut send, mut recv) = business.accept_bi().await.unwrap();
                    assert_eq!(
                        recv.read_to_end(128).await.unwrap(),
                        b"authorized via actual UDP mapping"
                    );
                    send.write_all(b"authorized reply via actual UDP mapping")
                        .await
                        .unwrap();
                    send.finish().unwrap();
                    business.closed().await;
                    println!("OTHER_PATH_OK case={case} real_member=true relay=false");
                }
            } else {
                let id = std::fs::read_to_string(directory.join("admin-id"))
                    .unwrap()
                    .parse()
                    .unwrap();
                let address = std::fs::read_to_string(directory.join("mapped"))
                    .unwrap()
                    .parse()
                    .unwrap();
                let target = EndpointAddr::new(id).with_addrs([TransportAddr::Ip(address)]);
                let connection = endpoint
                    .endpoint()
                    .connect_control(target.clone(), ADMISSION_REQUEST_ALPN)
                    .await
                    .unwrap();
                let connection = std::sync::Arc::new(connection);
                assert!(matches!(
                    node.request_manual_admission(&connection).await.unwrap(),
                    ManualExchange::AwaitingApproval
                ));
                while !directory.join("approved").exists() {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                let ManualExchange::Materials(materials) =
                    node.request_manual_admission(&connection).await.unwrap()
                else {
                    panic!("approved materials");
                };
                node.complete_manual_admission(materials).await.unwrap();
                connection.close(0u32.into(), b"admitted");
                let connection = endpoint.endpoint().connect(target, BUSINESS).await.unwrap();
                assert!(
                    node.admission_gate()
                        .allows(&connection, ProtocolScope::Member)
                );
                let (mut send, mut recv) = connection.open_bi().await.unwrap();
                send.write_all(b"authorized via actual UDP mapping")
                    .await
                    .unwrap();
                send.finish().unwrap();
                assert_eq!(
                    recv.read_to_end(128).await.unwrap(),
                    b"authorized reply via actual UDP mapping"
                );
                connection.close(0u32.into(), b"done");
                println!("DATA_OK real_member=true relay=false");
            }
            endpoint.endpoint().close().await;
            node.close().await;
        });
}

#[tokio::test]
async fn local_gateway_consent_survives_release_updates_and_node_restore() {
    use miopunch_native_probe::resources::{ResourceTable, SignedResourceTable};
    use p2panda_core::SigningKey;
    let directory = tempfile::tempdir().unwrap();
    let node = NativeNetwork::create(directory.path().join("node"))
        .await
        .unwrap();
    let publisher = SigningKey::generate();
    let mut config = node.resource_configuration().unwrap();
    config.port_mapping = PortMapping::NatPmp;
    config.trusted_update_key = Some(publisher.verifying_key());
    let update = SignedResourceTable::sign(
        &publisher,
        &ResourceTable {
            revision: 2,
            resources: vec![],
        },
    )
    .unwrap();
    config.apply_signed_update(&update).unwrap();
    config.import_trusted_replacement(vec![]).unwrap();
    assert_eq!(config.port_mapping, PortMapping::NatPmp);
    node.save_resource_configuration(&config).unwrap();
    let backup = directory.path().join("backup");
    node.create_backup(&backup).await.unwrap();
    let restored = NativeNetwork::restore_backup(&backup, directory.path().join("restored"))
        .await
        .unwrap();
    assert_eq!(
        restored.resource_configuration().unwrap().port_mapping,
        PortMapping::NatPmp
    );
    restored.close().await;
}

#[test]
fn old_resource_configuration_does_not_start_new_gateway_probes() {
    use miopunch_native_probe::resources::ResourceConfig;
    let mut document = serde_json::to_value(ResourceConfig::default()).unwrap();
    document.as_object_mut().unwrap().remove("port_mapping");
    let config: ResourceConfig = serde_json::from_value(document.clone()).unwrap();
    assert_eq!(config.port_mapping, PortMapping::Disabled);
    document["port_mapping"] = serde_json::json!("UnknownGatewayProtocol");
    assert!(serde_json::from_value::<ResourceConfig>(document).is_err());
}
