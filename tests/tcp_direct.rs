//! Formal node startup, admission and streams over real direct TCP sockets.
use std::{net::Ipv4Addr, time::Duration};

use iroh::{EndpointAddr, TransportAddr, endpoint::Connection};
use miopunch_native_probe::{
    admission_gate::{ADMISSION_REQUEST_ALPN, ProtocolScope},
    manual_admission::{AdmissionStage, ManualExchange},
    native_network::NativeNetwork,
    offline_replica::REPLICA_ALPN,
    resources::{ResourceConfig, ResourceEndpoint},
};

const BUSINESS: &[u8] = b"miopunch/tcp-direct-business/1";

fn configure(node: &NativeNetwork, address: &str) {
    let mut config = serde_json::to_value(ResourceConfig::default()).unwrap();
    config["replacement"] = serde_json::json!([]);
    config["tcp_listen"] = serde_json::json!(address);
    // The fixture deliberately disables every UDP data socket, independent of
    // whether the host would otherwise allow loopback UDP.
    config["udp_direct_disabled"] = serde_json::json!(true);
    node.save_resource_configuration(&serde_json::from_value(config).unwrap())
        .unwrap();
}

async fn target(endpoint: &ResourceEndpoint) -> EndpointAddr {
    let addr = endpoint.addr();
    let addrs = addr
        .addrs
        .into_iter()
        .filter(|addr| matches!(addr, TransportAddr::Custom(_)))
        .collect::<Vec<_>>();
    assert!(!addrs.is_empty(), "configured startup must advertise TCP");
    assert!(
        endpoint.endpoint().bound_sockets().is_empty(),
        "UDP is disabled"
    );
    EndpointAddr::new(endpoint.endpoint().id()).with_addrs(addrs)
}

async fn connect(
    client: &mut ResourceEndpoint,
    server: &ResourceEndpoint,
    alpn: &[u8],
) -> (Connection, Connection) {
    tokio::time::timeout(Duration::from_secs(12), async {
        let address = target(server).await;
        let deadline = n0_future::time::Instant::now() + Duration::from_secs(12);
        let (out, incoming) = tokio::join!(client.connect(address, alpn), async {
            server
                .endpoint()
                .accept()
                .await
                .unwrap()
                .with_caller_deadline(deadline)
                .await
        });
        (out.unwrap(), incoming.unwrap())
    })
    .await
    .expect("bounded formal TCP connection")
}

async fn admit(
    admin: &mut NativeNetwork,
    joining: &mut NativeNetwork,
    ce: &mut ResourceEndpoint,
    se: &ResourceEndpoint,
) {
    let (client, server) = connect(ce, se, ADMISSION_REQUEST_ALPN).await;
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
            panic!("approved material")
        };
        joining.complete_manual_admission(materials).await.unwrap()
    };
    let (saved, served) = tokio::join!(receive, admin.serve_manual_admission(&server));
    assert_eq!(saved, AdmissionStage::Complete);
    assert_eq!(served.unwrap(), AdmissionStage::Complete);
    client.close(0u32.into(), b"admitted");
    server.close(0u32.into(), b"admitted");
}

async fn formal_path(address: &str) {
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
    configure(&admin, address);
    configure(&joining, address);
    for node in [&admin, &joining] {
        node.admission_gate()
            .register_protocol(BUSINESS, ProtocolScope::Member);
        node.admission_gate()
            .register_protocol(REPLICA_ALPN, ProtocolScope::Member);
    }
    let se = admin
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let mut ce = joining
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    se.endpoint().set_alpns(vec![
        ADMISSION_REQUEST_ALPN.to_vec(),
        REPLICA_ALPN.to_vec(),
        BUSINESS.to_vec(),
    ]);
    admit(&mut admin, &mut joining, &mut ce, &se).await;
    ce.close().await;
    joining.close().await;
    let mut joining = NativeNetwork::open(dir.path().join("joining"))
        .await
        .unwrap();
    assert_eq!(joining.admission_stage(), AdmissionStage::Complete);
    joining
        .admission_gate()
        .register_protocol(BUSINESS, ProtocolScope::Member);
    joining
        .admission_gate()
        .register_protocol(REPLICA_ALPN, ProtocolScope::Member);
    let mut ce = joining
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    // Publish a new signed control record after admission material delivery;
    // the returning member must actually acquire it over this TCP connection.
    let additional_member = p2panda_core::SigningKey::generate().verifying_key();
    let mut management = admin.begin_management_session();
    let update = admin
        .manage(
            &mut management,
            miopunch_native_probe::daily_management::ManagementAction::AddMember(additional_member),
        )
        .await
        .unwrap();
    assert!(
        joining
            .management_record(update.record)
            .await
            .unwrap()
            .is_none()
    );
    let (client, server) = connect(&mut ce, &se, REPLICA_ALPN).await;
    let (synced, served) =
        tokio::join!(joining.sync_replica(&client), admin.serve_replica(&server));
    assert!(synced.unwrap().complete());
    served.unwrap();
    assert!(
        joining
            .management_record(update.record)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        joining
            .status()
            .members
            .iter()
            .any(|(member, _)| *member == additional_member)
    );
    client.close(0u32.into(), b"synced");
    server.close(0u32.into(), b"synced");
    let (client, server) = connect(&mut ce, &se, BUSINESS).await;
    assert!(
        client.paths().iter().any(
            |path| path.is_selected() && matches!(path.remote_addr(), TransportAddr::Custom(_))
        )
    );
    let (mut send, mut receive) = client.open_bi().await.unwrap();
    send.write_all(b"formal TCP business").await.unwrap();
    send.finish().unwrap();
    let (mut reply, mut request) = server.accept_bi().await.unwrap();
    assert_eq!(
        request.read_to_end(128).await.unwrap(),
        b"formal TCP business"
    );
    reply.write_all(b"formal TCP reply").await.unwrap();
    reply.finish().unwrap();
    assert_eq!(receive.read_to_end(128).await.unwrap(), b"formal TCP reply");
    eprintln!(
        "MIO40_REAL listen={address} udp_sockets=0 admission=Complete reopen=true replica=complete business=roundtrip path=Custom"
    );
    client.close(0u32.into(), b"done");
    server.close(0u32.into(), b"done");
    ce.close().await;
    se.close().await;
    joining.close().await;
    admin.close().await;
}

#[tokio::test]
async fn first_real_udp_disabled_tcp_ipv4_admits_syncs_and_transfers() {
    formal_path("127.0.0.1:0").await;
}

#[tokio::test]
async fn priority_wrong_member_identity_cannot_bypass_tcp_admission() {
    let dir = tempfile::tempdir().unwrap();
    let admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let joining = NativeNetwork::join(
        dir.path().join("joining"),
        admin.manual_invitation().unwrap(),
    )
    .await
    .unwrap();
    configure(&admin, "127.0.0.1:0");
    configure(&joining, "127.0.0.1:0");
    let se = admin
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let mut ce = joining
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap()
        .with_contact_timeout(Duration::from_secs(2))
        .unwrap();
    let mut address = target(&se).await;
    // The IP and port are real; the expected peer identity is independently
    // generated and does not belong to the listening node.
    address.id = iroh::SecretKey::generate().public();
    let (out, incoming) = tokio::join!(
        ce.connect(address, ADMISSION_REQUEST_ALPN),
        tokio::time::timeout(Duration::from_secs(3), async {
            se.endpoint().accept().await.unwrap().await
        }),
    );
    assert!(
        out.is_err(),
        "a reachable TCP port must not replace the expected identity"
    );
    assert!(
        !matches!(incoming, Ok(Ok(_))),
        "wrong identity must not yield a usable connection"
    );
    assert!(admin.manual_requests().await.unwrap().is_empty());
    assert_ne!(joining.admission_stage(), AdmissionStage::Complete);
    eprintln!(
        "MIO40_WRONG tcp=reachable wrong_expected_identity=rejected requests=0 admission=not-complete"
    );
    ce.close().await;
    se.close().await;
    joining.close().await;
    admin.close().await;
}

#[tokio::test]
async fn disabled_tcp_candidate_is_not_dialed_even_through_raw_endpoint() {
    use miopunch_native_probe::tcp_transport::TcpTransport;
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = match listener.local_addr().unwrap() {
        std::net::SocketAddr::V4(addr) => addr,
        _ => unreachable!(),
    };
    let dir = tempfile::tempdir().unwrap();
    let node = NativeNetwork::create(dir.path().join("node"))
        .await
        .unwrap();
    configure(&node, "127.0.0.1:0");
    let mut config = serde_json::to_value(node.resource_configuration().unwrap()).unwrap();
    config["disabled_tcp_addresses"] = serde_json::json!([address.to_string()]);
    node.save_resource_configuration(&serde_json::from_value(config).unwrap())
        .unwrap();
    let endpoint = node
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let target = EndpointAddr::new(iroh::SecretKey::generate().public())
        .with_addrs([TransportAddr::Custom(TcpTransport::address(address))]);
    // Bypass the configured contact wrapper: qualification must still occur
    // inside the fork before the TCP sender opens a socket.
    let (attempt, accepted) = tokio::join!(
        tokio::time::timeout(
            Duration::from_millis(700),
            endpoint.endpoint().connect(target, ADMISSION_REQUEST_ALPN)
        ),
        tokio::time::timeout(Duration::from_secs(1), listener.accept()),
    );
    assert!(!matches!(attempt, Ok(Ok(_))));
    assert!(
        accepted.is_err(),
        "forbidden TCP candidate was actually dialed"
    );
    eprintln!("MIO40_DISABLED source=raw-endpoint actual_tcp_accepts=0");
    endpoint.close().await;
    node.close().await;
}

#[tokio::test]
async fn available_ipv6_tcp_admits_syncs_and_transfers_independently() {
    // Real environment preflight, not a simulated IPv6 success or ignored test.
    let listener = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
    eprintln!(
        "MIO40_IPV6 environment_listener={}",
        listener.local_addr().unwrap()
    );
    drop(listener);
    formal_path("[::1]:0").await;
}

#[tokio::test]
async fn valid_but_unapproved_identity_cannot_read_member_history_over_tcp() {
    let dir = tempfile::tempdir().unwrap();
    let admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let joining = NativeNetwork::join(
        dir.path().join("joining"),
        admin.manual_invitation().unwrap(),
    )
    .await
    .unwrap();
    for node in [&admin, &joining] {
        configure(node, "127.0.0.1:0");
        node.admission_gate()
            .register_protocol(REPLICA_ALPN, ProtocolScope::Member);
    }
    let se = admin
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    se.endpoint().set_alpns(vec![REPLICA_ALPN.to_vec()]);
    let mut ce = joining
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap()
        .with_contact_timeout(Duration::from_secs(6))
        .unwrap();
    let address = target(&se).await;
    let (out, incoming) = tokio::join!(
        ce.connect(address, REPLICA_ALPN),
        tokio::time::timeout(Duration::from_secs(7), async {
            se.endpoint().accept().await.unwrap().await
        }),
    );
    assert!(out.is_err());
    assert!(!matches!(incoming, Ok(Ok(_))));
    assert_ne!(joining.admission_stage(), AdmissionStage::Complete);
    eprintln!("MIO40_UNAPPROVED actual_identity=authenticated member_history=rejected");
    ce.close().await;
    se.close().await;
    joining.close().await;
    admin.close().await;
}

#[tokio::test]
async fn received_revocation_ends_tcp_flow_and_rejects_stale_member() {
    use miopunch_native_probe::daily_management::ManagementAction;
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
    for node in [&admin, &joining] {
        configure(node, "127.0.0.1:0");
        node.admission_gate()
            .register_protocol(BUSINESS, ProtocolScope::Member);
    }
    let se = admin
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    se.endpoint()
        .set_alpns(vec![ADMISSION_REQUEST_ALPN.to_vec(), BUSINESS.to_vec()]);
    let mut ce = joining
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    admit(&mut admin, &mut joining, &mut ce, &se).await;
    let (client, server) = connect(&mut ce, &se, BUSINESS).await;
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"live").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut bytes = [0; 4];
    receive.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"live");
    let mut session = admin.begin_management_session();
    admin
        .manage(
            &mut session,
            ManagementAction::RemoveMember(joining.status().node_id),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), client.closed())
        .await
        .unwrap();
    assert!(server.close_reason().is_some());
    assert!(receive.read_exact(&mut bytes).await.is_err());
    let address = target(&se).await;
    let (out, incoming) = tokio::join!(
        ce.connect(address, BUSINESS),
        tokio::time::timeout(Duration::from_secs(7), async {
            se.endpoint().accept().await.unwrap().await
        }),
    );
    assert!(out.is_err());
    assert!(!matches!(incoming, Ok(Ok(_))));
    eprintln!("MIO40_REVOKED actual_live_stream=ended stale_member_new_connection=rejected");
    ce.close().await;
    se.close().await;
    joining.close().await;
    admin.close().await;
}

#[tokio::test]
async fn cancelling_pending_tcp_contact_closes_actual_carrier() {
    use miopunch_native_probe::tcp_transport::TcpTransport;
    use tokio::io::AsyncReadExt;
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let node = NativeNetwork::create(dir.path().join("node"))
        .await
        .unwrap();
    configure(&node, "127.0.0.1:0");
    let endpoint = node
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let raw = endpoint.endpoint().clone();
    let target = EndpointAddr::new(iroh::SecretKey::generate().public()).with_addrs([
        TransportAddr::Custom(TcpTransport::address(listener.local_addr().unwrap())),
    ]);
    let pending = tokio::spawn(async move { raw.connect(target, ADMISSION_REQUEST_ALPN).await });
    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
        .await
        .unwrap()
        .unwrap();
    let mut preface = [0; 6];
    socket.read_exact(&mut preface).await.unwrap();
    endpoint.close().await;
    assert!(
        tokio::time::timeout(Duration::from_secs(2), pending)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    let mut byte = [0];
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), socket.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    eprintln!("MIO40_CANCEL pending_connect=failed actual_socket=eof bounded=true");
    node.close().await;
}

#[tokio::test]
async fn lookup_supplied_disabled_tcp_candidate_is_not_dialed() {
    use iroh::{Endpoint, endpoint::presets};
    use miopunch_native_probe::tcp_transport::TcpTransport;
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let socket_addr = listener.local_addr().unwrap();
    let id = iroh::SecretKey::generate().public();
    let lookup =
        iroh::address_lookup::memory::MemoryLookup::from_endpoint_info([EndpointAddr::new(id)
            .with_addrs([TransportAddr::Custom(TcpTransport::address(socket_addr))])]);
    let dir = tempfile::tempdir().unwrap();
    let node = NativeNetwork::create(dir.path().join("node"))
        .await
        .unwrap();
    configure(&node, "127.0.0.1:0");
    let mut config = node.resource_configuration().unwrap();
    config.disabled_tcp_addresses.insert(socket_addr);
    node.save_resource_configuration(&config).unwrap();
    let endpoint = node
        .bind_endpoint_with(
            Endpoint::builder(presets::Minimal)
                .clear_relay_transports()
                .address_lookup(lookup),
        )
        .await
        .unwrap();
    let (attempt, accepted) = tokio::join!(
        tokio::time::timeout(
            Duration::from_millis(700),
            endpoint.connect(id, ADMISSION_REQUEST_ALPN)
        ),
        tokio::time::timeout(Duration::from_secs(1), listener.accept()),
    );
    assert!(!matches!(attempt, Ok(Ok(_))));
    assert!(
        accepted.is_err(),
        "lookup must not grant permission to dial a candidate"
    );
    eprintln!("MIO40_DISABLED source=lookup-result actual_tcp_accepts=0");
    endpoint.close().await;
    node.close().await;
}

#[tokio::test]
async fn globally_disabled_tcp_cannot_be_reenabled_by_an_advanced_builder() {
    use iroh::{Endpoint, endpoint::presets};
    use miopunch_native_probe::tcp_transport::{TcpTransport, Traffic};
    use std::sync::Arc;
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let node = NativeNetwork::create(dir.path().join("node"))
        .await
        .unwrap();
    configure(&node, "127.0.0.1:0");
    let mut config = node.resource_configuration().unwrap();
    config.tcp_direct_disabled = true;
    node.save_resource_configuration(&config).unwrap();
    let normal = node
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    assert!(
        normal.addr().addrs.is_empty(),
        "disabled TCP must not be advertised"
    );
    normal.close().await;
    let endpoint = node
        .bind_endpoint_with(
            Endpoint::builder(presets::Minimal)
                .clear_relay_transports()
                .add_custom_transport(Arc::new(TcpTransport::new(
                    Ipv4Addr::LOCALHOST,
                    Arc::new(Traffic::default()),
                ))),
        )
        .await
        .unwrap();
    let target = EndpointAddr::new(iroh::SecretKey::generate().public()).with_addrs([
        TransportAddr::Custom(TcpTransport::address(listener.local_addr().unwrap())),
    ]);
    let (attempt, accepted) = tokio::join!(
        tokio::time::timeout(
            Duration::from_millis(700),
            endpoint.connect(target, ADMISSION_REQUEST_ALPN)
        ),
        tokio::time::timeout(Duration::from_secs(1), listener.accept()),
    );
    assert!(!matches!(attempt, Ok(Ok(_))));
    assert!(
        accepted.is_err(),
        "explicit builder must not override persisted TCP disable"
    );
    eprintln!("MIO40_DISABLED scope=global source=advanced-builder actual_tcp_accepts=0");
    endpoint.close().await;
    node.close().await;
}

#[tokio::test]
async fn physical_tcp_loss_ends_old_stream_and_later_contact_is_a_new_connection() {
    use miopunch_native_probe::tcp_transport::TcpTransport;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };
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
    for node in [&admin, &joining] {
        configure(node, "127.0.0.1:0");
        node.admission_gate()
            .register_protocol(BUSINESS, ProtocolScope::Member);
    }
    let se = admin
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    se.endpoint()
        .set_alpns(vec![ADMISSION_REQUEST_ALPN.to_vec(), BUSINESS.to_vec()]);
    let mut ce = joining
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    admit(&mut admin, &mut joining, &mut ce, &se).await;

    ce.close().await;
    se.close().await;
    let se = admin
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    se.endpoint().set_alpns(vec![BUSINESS.to_vec()]);
    let mut ce = joining
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();

    // A controlled byte-forwarding TCP fixture changes only the untrusted
    // route preface; all QUIC/TLS packets and peer identities remain real.
    let direct = target(&se).await;
    let TransportAddr::Custom(custom) = direct.addrs.iter().next().unwrap() else {
        unreachable!()
    };
    let server_address = TcpTransport::socket_addr(custom).unwrap();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let proxy_address = listener.local_addr().unwrap();
    let (cut, stop) = tokio::sync::oneshot::channel::<()>();
    let proxy = tokio::spawn(async move {
        let (mut downstream, _) = listener.accept().await.unwrap();
        let mut upstream = TcpStream::connect(server_address).await.unwrap();
        let mut preface = [0; 6];
        downstream.read_exact(&mut preface).await.unwrap();
        upstream
            .write_all(TcpTransport::address(proxy_address).data())
            .await
            .unwrap();
        upstream.read_exact(&mut preface).await.unwrap();
        downstream
            .write_all(TcpTransport::address(proxy_address).data())
            .await
            .unwrap();
        tokio::select! {
            _ = tokio::io::copy_bidirectional(&mut downstream, &mut upstream) => {},
            _ = stop => {},
        }
        // Dropping both sockets and the listener removes this physical path.
    });
    let target = EndpointAddr::new(se.endpoint().id())
        .with_addrs([TransportAddr::Custom(TcpTransport::address(proxy_address))]);
    let (out, incoming) = tokio::time::timeout(Duration::from_secs(12), async {
        tokio::join!(ce.connect(target, BUSINESS), async {
            se.endpoint().accept().await.unwrap().await
        })
    })
    .await
    .unwrap();
    let (client, server) = (out.unwrap(), incoming.unwrap());
    let old_id = client.stable_id();
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"before cut").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut bytes = [0; 10];
    receive.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"before cut");
    let cut_at = std::time::Instant::now();
    cut.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), proxy)
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(13), client.closed())
        .await
        .unwrap();
    eprintln!(
        "MIO40_CUT_TIMEOUT elapsed_ms={}",
        cut_at.elapsed().as_millis()
    );
    assert!(client.close_reason().is_some());
    assert!(
        tokio::time::timeout(Duration::from_secs(2), receive.read_exact(&mut bytes))
            .await
            .unwrap()
            .is_err()
    );
    assert!(send.write_all(b"after cut").await.is_err());
    let (replacement, peer) = connect(&mut ce, &se, BUSINESS).await;
    assert_ne!(replacement.stable_id(), old_id);
    assert!(
        client.close_reason().is_some(),
        "the old connection remains closed"
    );
    eprintln!(
        "MIO40_CUT physical_tcp=lost old_stream=failed original_connection=closed later_connection=new"
    );
    replacement.close(0u32.into(), b"done");
    peer.close(0u32.into(), b"done");
    ce.close().await;
    se.close().await;
    joining.close().await;
    admin.close().await;
}

#[tokio::test]
async fn normal_cli_starts_with_tcp_only_and_exits_on_interrupt() {
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
        sync::mpsc,
    };
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("node");
    let node = NativeNetwork::create(&path).await.unwrap();
    configure(&node, "127.0.0.1:0");
    let id = node.status().node_id.to_string();
    node.close().await;
    let mut child = Child(
        Command::new(env!("CARGO_BIN_EXE_miopunch-native-probe"))
            .args(["network", "run"])
            .arg(&path)
            .arg("127.0.0.1")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut ready = String::new();
        BufReader::new(stdout).read_line(&mut ready).unwrap();
        let _ = sender.send(ready);
    });
    let ready = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    reader.join().unwrap();
    let fields = ready.split_whitespace().collect::<Vec<_>>();
    assert_eq!(&fields[..2], &["READY", &id]);
    let address: std::net::SocketAddr = fields[2].parse().unwrap();
    assert!(address.ip().is_loopback() && address.port() != 0);
    rustix::process::kill_process(
        rustix::process::Pid::from_raw(child.0.id() as i32).unwrap(),
        rustix::process::Signal::INT,
    )
    .unwrap();
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(started.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(status.success());
    eprintln!("MIO40_CLI tcp_only_ready={address} interrupt_exit=0");
}
