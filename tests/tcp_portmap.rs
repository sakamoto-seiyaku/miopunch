//! Real gateway protocol and kernel forwarding are driven by tcp_portmap_lab.py.
use std::net::Ipv4Addr;
use std::time::Duration;

use iroh::{EndpointAddr, TransportAddr};
use miopunch_native_probe::admission_gate::{ADMISSION_REQUEST_ALPN, ProtocolScope};
use miopunch_native_probe::manual_admission::ManualExchange;
use miopunch_native_probe::native_network::NativeNetwork;
use miopunch_native_probe::resources::{PortMapping, ResourceEndpoint};
use miopunch_native_probe::tcp_transport::TcpTransport;

const BUSINESS: &[u8] = b"miopunch/tcp-portmap-business/1";

fn tcp_addrs(endpoint: &ResourceEndpoint) -> Vec<iroh_base::CustomAddr> {
    endpoint
        .addr()
        .addrs
        .into_iter()
        .filter_map(|addr| match addr {
            TransportAddr::Custom(addr) => Some(addr),
            _ => None,
        })
        .collect()
}

// Worker is inert unless the isolated lab invokes this exact public test entry.
#[test]
fn gateway_node_worker() {
    let Ok(role) = std::env::var("MIO41_ROLE") else {
        return;
    };
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async move {
            let directory = std::path::PathBuf::from(std::env::var("MIO41_DIRECTORY").unwrap());
            if role == "prepare" {
                let admin = NativeNetwork::create(directory.join("admin"))
                    .await
                    .unwrap();
                let invitation = admin.manual_invitation().unwrap();
                let joining = NativeNetwork::join(directory.join("joining"), invitation)
                    .await
                    .unwrap();
                let case = std::env::var("MIO41_CASE").unwrap_or_default();
                for (node, mapping) in [
                    (
                        &admin,
                        if matches!(case.as_str(), "disabled" | "disabled-advanced") {
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
                    config.tcp_listen =
                        if matches!(case.as_str(), "disabled-advanced" | "tcp-disabled-advanced") {
                            None
                        } else {
                            Some(
                                if node.status().node_id == admin.status().node_id {
                                    "0.0.0.0:41041"
                                } else {
                                    "0.0.0.0:0"
                                }
                                .parse()
                                .unwrap(),
                            )
                        };
                    config.tcp_direct_disabled =
                        matches!(case.as_str(), "tcp-disabled" | "tcp-disabled-advanced");
                    config.udp_direct_disabled = case != "tcp-disabled";
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
                    "serve"
                        | "disabled"
                        | "disabled-advanced"
                        | "tcp-disabled-advanced"
                        | "tcp-disabled"
                        | "allocation-timeout"
                        | "allocation-cancel"
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
            if matches!(role.as_str(), "disabled-advanced" | "tcp-disabled-advanced") {
                let transport = TcpTransport::new(
                    Ipv4Addr::UNSPECIFIED,
                    std::sync::Arc::new(Default::default()),
                )
                .with_nat_pmp_mapping();
                let raw = node
                    .bind_endpoint_with(
                        iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
                            .clear_ip_transports()
                            .add_custom_transport(std::sync::Arc::new(transport))
                            .portmapper_config(iroh::endpoint::PortmapperConfig::NatPmp),
                    )
                    .await
                    .unwrap();
                tokio::time::sleep(Duration::from_secs(1)).await;
                assert!(!raw.custom_addrs().into_iter().any(|addr| {
                    TcpTransport::socket_addr(&addr).unwrap().ip().to_string() == "100.64.41.1"
                }));
                raw.close().await;
                node.close().await;
                println!("ADVANCED_FACTORY_RESPECTS_LOCAL_MAPPING_DISABLE");
                return;
            }
            let mut endpoint = node
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
                assert!(!tcp_addrs(&endpoint).into_iter().any(|addr| {
                    TcpTransport::socket_addr(&addr).unwrap().ip().to_string() == "100.64.41.1"
                }));
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
            if matches!(role.as_str(), "disabled" | "tcp-disabled") {
                tokio::time::sleep(Duration::from_secs(1)).await;
                assert!(!tcp_addrs(&endpoint).into_iter().any(|addr| {
                    TcpTransport::socket_addr(&addr).unwrap().ip().to_string() == "100.64.41.1"
                }));
                println!("MAPPING_DISABLED role={role} relay=false");
                endpoint.endpoint().close().await;
                node.close().await;
                return;
            }
            if matches!(role.as_str(), "reconnect" | "fallback") {
                let id = std::fs::read_to_string(directory.join("admin-id"))
                    .unwrap()
                    .parse()
                    .unwrap();
                let address: std::net::SocketAddr =
                    std::fs::read_to_string(directory.join(if role == "fallback" {
                        "internal"
                    } else {
                        "mapped"
                    }))
                    .unwrap()
                    .parse()
                    .unwrap();
                let connection = endpoint
                    .connect(
                        EndpointAddr::new(id)
                            .with_addrs([TransportAddr::Custom(TcpTransport::address(address))]),
                        BUSINESS,
                    )
                    .await
                    .unwrap();
                assert!(
                    node.admission_gate()
                        .allows(&connection, ProtocolScope::Member)
                );
                let (mut send, mut recv) = connection.open_bi().await.unwrap();
                send.write_all(b"authorized via actual TCP mapping")
                    .await
                    .unwrap();
                send.finish().unwrap();
                assert_eq!(
                    recv.read_to_end(128).await.unwrap(),
                    b"authorized reply via actual TCP mapping"
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
                let address: std::net::SocketAddr = std::fs::read_to_string(directory.join(
                    if directory.join("changed").exists() {
                        "old-mapped"
                    } else {
                        "mapped"
                    },
                ))
                .unwrap()
                .parse()
                .unwrap();
                let target = EndpointAddr::new(id)
                    .with_addrs([TransportAddr::Custom(TcpTransport::address(address))]);
                let result = tokio::time::timeout(
                    Duration::from_secs(2),
                    endpoint.connect(target, BUSINESS),
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
                let case = std::env::var("MIO41_CASE").unwrap_or_default();
                let expected: std::net::SocketAddr = if case == "initial-failure" {
                    "10.41.0.2:41041"
                } else {
                    "100.64.41.1:41041"
                }
                .parse()
                .unwrap();
                if case == "initial-failure" {
                    tokio::time::sleep(Duration::from_millis(800)).await;
                    assert!(!tcp_addrs(&endpoint).into_iter().any(|addr| {
                        TcpTransport::socket_addr(&addr).unwrap().ip().to_string() == "100.64.41.1"
                    }));
                    println!("ALLOCATION_FAILED_NO_PUBLIC_CANDIDATE allowed_lan_fallback=true");
                } else {
                    tokio::time::timeout(Duration::from_secs(8), async {
                        loop {
                            if tcp_addrs(&endpoint)
                                .into_iter()
                                .any(|addr| TcpTransport::socket_addr(&addr).unwrap() == expected)
                            {
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                    })
                    .await
                    .expect("real NAT-PMP candidate must appear without relay");
                }
                assert!(endpoint.endpoint().addr().relay_urls().next().is_none());
                println!(
                    "MAPPED candidate={expected} bound={:?}",
                    endpoint.endpoint().bound_sockets()
                );
                std::fs::write(directory.join("mapped"), expected.to_string()).unwrap();
                std::fs::write(directory.join("old-mapped"), expected.to_string()).unwrap();
                let internal_port = 41041;
                std::fs::write(
                    directory.join("internal"),
                    format!("10.41.0.2:{internal_port}"),
                )
                .unwrap();
                let connection = endpoint.endpoint().accept().await.unwrap().await.unwrap();
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
                    b"authorized via actual TCP mapping"
                );
                send.write_all(b"authorized reply via actual TCP mapping")
                    .await
                    .unwrap();
                send.finish().unwrap();
                println!("DATA_OK real_member=true relay=false");
                std::fs::write(directory.join("data-ready"), b"").unwrap();
                let case = std::env::var("MIO41_CASE").unwrap_or_default();
                if case == "renewal-address-failure" {
                    tokio::time::timeout(Duration::from_secs(6), async {
                        while !directory.join("mapping-cleaned").exists() {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                    })
                    .await
                    .expect("renewed allocation must be cleaned after its address query fails");
                    tokio::time::timeout(Duration::from_millis(600), async {
                        while tcp_addrs(&endpoint)
                            .into_iter()
                            .any(|addr| TcpTransport::socket_addr(&addr).unwrap() == expected)
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
                            if !tcp_addrs(&endpoint)
                                .into_iter()
                                .any(|addr| TcpTransport::socket_addr(&addr).unwrap() == expected)
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
                    let changed: std::net::SocketAddr = "100.64.41.1:42041".parse().unwrap();
                    tokio::time::timeout(Duration::from_secs(6), async {
                        while !tcp_addrs(&endpoint)
                            .into_iter()
                            .any(|addr| TcpTransport::socket_addr(&addr).unwrap() == changed)
                        {
                            tokio::time::sleep(Duration::from_millis(25)).await;
                        }
                    })
                    .await
                    .expect("renewal after change/restart must publish replacement mapping");
                    assert!(
                        !tcp_addrs(&endpoint)
                            .into_iter()
                            .any(|addr| TcpTransport::socket_addr(&addr).unwrap() == expected)
                    );
                    std::fs::write(directory.join("mapped"), changed.to_string()).unwrap();
                    println!("CANDIDATE_REPLACED old={expected} new={changed} case={case}");
                    std::fs::write(directory.join("changed"), b"").unwrap();
                }
                if case == "renewed" {
                    tokio::time::sleep(Duration::from_secs(7)).await;
                    assert!(
                        tcp_addrs(&endpoint)
                            .into_iter()
                            .any(|addr| TcpTransport::socket_addr(&addr).unwrap() == expected)
                    );
                    println!("LEASE_RENEWED candidate={expected}");
                    std::fs::write(directory.join("renewed"), b"").unwrap();
                }
                if matches!(
                    case.as_str(),
                    "expired" | "changed" | "restart" | "renewed" | "renewal-address-failure"
                ) {
                    std::fs::write(directory.join("existing-check"), b"").unwrap();
                    let (mut send, mut recv) = business.accept_bi().await.unwrap();
                    assert_eq!(
                        recv.read_to_end(128).await.unwrap(),
                        b"existing TCP session after lease change"
                    );
                    send.write_all(b"existing session remains authenticated")
                        .await
                        .unwrap();
                    send.finish().unwrap();
                    println!(
                        "EXISTING_CONNECTION_CONTINUES id={} case={case}",
                        business.stable_id()
                    );
                }
                business.closed().await;
                if matches!(case.as_str(), "expired" | "changed" | "restart" | "renewed") {
                    let business = endpoint.endpoint().accept().await.unwrap().await.unwrap();
                    assert!(
                        node.admission_gate()
                            .allows(&business, ProtocolScope::Member)
                    );
                    let (mut send, mut recv) = business.accept_bi().await.unwrap();
                    assert_eq!(
                        recv.read_to_end(128).await.unwrap(),
                        b"authorized via actual TCP mapping"
                    );
                    send.write_all(b"authorized reply via actual TCP mapping")
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
                let address: std::net::SocketAddr =
                    std::fs::read_to_string(directory.join("mapped"))
                        .unwrap()
                        .parse()
                        .unwrap();
                let target = EndpointAddr::new(id)
                    .with_addrs([TransportAddr::Custom(TcpTransport::address(address))]);
                let connection = endpoint
                    .connect(target.clone(), ADMISSION_REQUEST_ALPN)
                    .await
                    .unwrap();
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
                let connection = endpoint.connect(target, BUSINESS).await.unwrap();
                assert!(
                    node.admission_gate()
                        .allows(&connection, ProtocolScope::Member)
                );
                let (mut send, mut recv) = connection.open_bi().await.unwrap();
                send.write_all(b"authorized via actual TCP mapping")
                    .await
                    .unwrap();
                send.finish().unwrap();
                assert_eq!(
                    recv.read_to_end(128).await.unwrap(),
                    b"authorized reply via actual TCP mapping"
                );
                println!("DATA_OK real_member=true relay=false");
                let case = std::env::var("MIO41_CASE").unwrap_or_default();
                if matches!(
                    case.as_str(),
                    "expired" | "changed" | "restart" | "renewed" | "renewal-address-failure"
                ) {
                    tokio::time::timeout(Duration::from_secs(15), async {
                        while !directory.join("existing-check").exists() {
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                    })
                    .await
                    .unwrap();
                    let (mut send, mut recv) = connection.open_bi().await.unwrap();
                    send.write_all(b"existing TCP session after lease change")
                        .await
                        .unwrap();
                    send.finish().unwrap();
                    assert_eq!(
                        recv.read_to_end(128).await.unwrap(),
                        b"existing session remains authenticated"
                    );
                    println!(
                        "EXISTING_CONNECTION_CONTINUES id={} case={case}",
                        connection.stable_id()
                    );
                }
                connection.close(0u32.into(), b"done");
                println!("DATA_OK real_member=true relay=false");
            }
            endpoint.endpoint().close().await;
            node.close().await;
        });
}
