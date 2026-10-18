//! Owned loopback lifecycle supplement; public SDK/device stories are separate.
use miopunch_native_probe::control_protocol::AuthenticatedControl;
use miopunch_native_probe::{
    connection_coordinator::{AttemptContext, MEMBER_ALPN, SessionPurpose},
    contact_listener::ContactListenerPhase,
    manual_admission::ManualExchange,
    mqtt_transport::{MqttTransport, Traffic},
    native_network::NativeNetwork,
    resources::{Resource, ResourceUse},
};
use std::{
    collections::BTreeSet,
    net::Ipv4Addr,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
#[path = "support/contact_broker.rs"]
mod contact_broker;

#[tokio::test]
async fn dropped_generation_cannot_rebind_until_its_driver_really_exits() {
    let broker = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let transport = MqttTransport::new(broker.local_addr().unwrap(), Arc::new(Traffic::default()));
    let first = transport.bind_datagrams().unwrap();
    drop(first);
    // The driver's spawned future has not run. Closed status is only route
    // withdrawal, not permission to overlap its socket/task with a new generation.
    let second = transport.bind_datagrams();
    let prematurely_rebound = second.is_ok();
    drop(second);
    tokio::time::timeout(Duration::from_secs(1), transport.wait_driver_closed())
        .await
        .unwrap()
        .unwrap();
    assert!(
        !prematurely_rebound,
        "previous MQTT driver was still owned when rebound"
    );
    assert_eq!(transport.traffic().snapshot()["driver_active"], 0);
    let third = transport.bind_datagrams().unwrap();
    drop(third);
    transport.wait_driver_closed().await.unwrap();
}

#[derive(Debug)]
struct Echo;
impl iroh::protocol::ProtocolHandler for Echo {
    async fn accept(
        &self,
        connection: iroh::endpoint::Connection,
    ) -> Result<(), iroh::protocol::AcceptError> {
        while let Ok((send, recv)) = connection.accept_bi().await {
            if miopunch_native_probe::member_message::serve(send, recv)
                .await
                .is_err()
            {
                break;
            }
        }
        Ok(())
    }
}
async fn admit(admin: &mut NativeNetwork, directory: &std::path::Path) -> NativeNetwork {
    let mut member = NativeNetwork::join(directory, admin.manual_invitation().unwrap())
        .await
        .unwrap();
    let server = admin
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let client = member
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let server_coordinator = server.coordinator();
    let client_coordinator = client.coordinator();
    let deadline = n0_future::time::Instant::now() + Duration::from_secs(12);
    let cancellation = tokio_util::sync::CancellationToken::new();
    let (outgoing, incoming) = tokio::join!(
        client_coordinator.connect_control(
            server.addr(),
            SessionPurpose::AdmissionRequest,
            AttemptContext::new(deadline, cancellation.clone()),
        ),
        async {
            let incoming = server.endpoint().accept().await.unwrap();
            server_coordinator
                .accept_control(
                    incoming,
                    SessionPurpose::AdmissionRequest,
                    AttemptContext::new(deadline, cancellation),
                )
                .await
        }
    );
    let outgoing = outgoing.unwrap();
    let incoming = incoming.unwrap();
    let (request, served) = tokio::join!(
        member.request_manual_admission(&outgoing),
        admin.serve_manual_admission(&incoming)
    );
    assert!(matches!(request.unwrap(), ManualExchange::AwaitingApproval));
    served.unwrap();
    let request = admin.manual_requests().await.unwrap().pop().unwrap();
    admin.approve_manual_admission(&request).await.unwrap();
    let receive = async {
        let ManualExchange::Materials(material) =
            member.request_manual_admission(&outgoing).await.unwrap()
        else {
            panic!("approved materials required");
        };
        member.complete_manual_admission(material).await.unwrap();
    };
    let (_, served) = tokio::join!(receive, admin.serve_manual_admission(&incoming));
    served.unwrap();
    outgoing.close(0u32.into(), b"owned admission complete");
    incoming.close(0u32.into(), b"owned admission complete");
    client.close().await;
    server.close().await;
    member
}
async fn ping(connection: &iroh::endpoint::Connection) {
    let (mut send, mut recv) = connection.open_bi().await.unwrap();
    send.write_all(b"PING").await.unwrap();
    send.finish().unwrap();
    assert_eq!(recv.read_to_end(64).await.unwrap(), b"PONG");
}

#[tokio::test]
async fn node_keeps_contact_ready_and_restarts_during_healthy_relay_and_business() {
    let directory = tempfile::tempdir().unwrap();
    let broker = contact_broker::Broker::new().await;
    let mut relay_config = iroh_relay::server::ServerConfig::default();
    relay_config.relay = Some(iroh_relay::server::RelayConfig::new((
        Ipv4Addr::LOCALHOST,
        0,
    )));
    let relay = iroh_relay::server::Server::spawn(relay_config)
        .await
        .unwrap();
    let relay_url = relay.http_url().unwrap();
    let mut native = NativeNetwork::create(directory.path().join("native"))
        .await
        .unwrap();
    let mut configuration = native.resource_configuration().unwrap();
    configuration.replacement = Some(vec![]);
    native.save_resource_configuration(&configuration).unwrap();
    let peer = admit(&mut native, &directory.path().join("peer")).await;
    peer.save_resource_configuration(&configuration).unwrap();
    configuration.replacement = Some(vec![
        Resource {
            url: relay_url.to_string(),
            uses: BTreeSet::from([ResourceUse::DataRelay]),
            limitation: "owned loopback healthy relay supplement".into(),
        },
        Resource {
            url: format!("mqtt://{}/", broker.address),
            uses: BTreeSet::from([ResourceUse::Signaling]),
            limitation: "owned loopback exact disconnect fixture; no public qualification".into(),
        },
    ]);
    configuration.maintenance.exploration_interval_ms = 250;
    configuration.maintenance.discovery_interval_ms = 1000;
    native.save_resource_configuration(&configuration).unwrap();
    let native_id = native.status().node_id;
    let peer_id = peer.status().node_id;
    let node = native
        .maintain_network_with(Ipv4Addr::LOCALHOST, |router| {
            router.accept(MEMBER_ALPN, Echo)
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), node.endpoint().online())
        .await
        .unwrap();
    assert!(node.addr().relay_urls().any(|url| url == &relay_url));
    let initial = node.maintenance_status().contact_listener.unwrap();
    assert_eq!(initial.phase, ContactListenerPhase::Ready);
    let activity = node.business_activity();
    let resources = peer
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let caller_started = tokio::time::Instant::now();
    let connection = resources
        .coordinator()
        .connect(
            node.addr(),
            SessionPurpose::MemberBusiness,
            AttemptContext::new(
                n0_future::time::Instant::now() + Duration::from_secs(2),
                tokio_util::sync::CancellationToken::new(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(connection.remote_id().as_bytes(), native_id.as_bytes());
    assert_eq!(resources.endpoint().id().as_bytes(), peer_id.as_bytes());
    assert!(
        connection
            .paths()
            .iter()
            .any(|path| matches!(path.remote_addr(), iroh::TransportAddr::Ip(_)))
    );
    ping(&connection).await;
    let caller_ms = caller_started.elapsed().as_millis();
    broker.disconnect();
    let mut updates = node.watch_maintenance();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let status = updates.borrow_and_update().clone();
            if status
                .contact_listener
                .as_ref()
                .is_some_and(|listener| listener.phase == ContactListenerPhase::Unavailable)
            {
                break;
            }
            updates.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert!(
        node.endpoint()
            .custom_addrs()
            .iter()
            .all(|address| address.id() != miopunch_native_probe::mqtt_transport::TRANSPORT_ID)
    );
    assert!(connection.close_reason().is_none());
    ping(&connection).await;
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let status = node.maintenance_status().contact_listener.unwrap();
            if status.phase == ContactListenerPhase::Ready && status.generation > initial.generation
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(node.addr().relay_urls().any(|url| url == &relay_url));
    assert!(connection.close_reason().is_none());
    ping(&connection).await;
    assert_eq!(broker.connections.load(Ordering::SeqCst), 2);
    let status = node.maintenance_status();
    assert_eq!(
        status.state,
        miopunch_native_probe::network_maintenance::MaintenanceState::BusinessPriority
    );
    let local_address = *node.addr().ip_addrs().next().unwrap();
    connection.close(0u32.into(), b"supplement ended");
    drop(activity);
    resources.close().await;
    peer.close().await;
    let cleanup_started = tokio::time::Instant::now();
    node.close().await;
    assert_eq!(
        updates.borrow().contact_listener.as_ref().unwrap().phase,
        ContactListenerPhase::Stopped
    );
    let rebound = tokio::net::UdpSocket::bind(local_address).await.unwrap();
    drop(rebound);
    let sockets = broker.no_sockets().await;
    let late = tokio::net::UdpSocket::bind(local_address).await.unwrap();
    drop(late);
    println!(
        "CONTACT_NODE_PASS formal_sdk=true strict_member_alpn=true identity_matches=true actual_data_route=udp_loopback relay_stayed_online=true healthy_same_m=true single_selector_restart=true caller_ms={} cleanup_ms={} udp_immediate_late_empty=true mqtt_immediate_empty={} mqtt_settled_ms={} mqtt_late_empty={}",
        caller_ms,
        cleanup_started.elapsed().as_millis(),
        sockets.immediate_empty,
        sockets.settled_ms,
        sockets.late_empty
    );
    broker.close().await;
    relay.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_maintained_node_with_healthy_relay_accepts_member_only_restricted_contact() {
    use miopunch_native_probe::{
        admission_gate::ProtocolScope,
        member_transport::{MemberForwarder, MemberSignalingTransport},
    };
    const LINK_ALPN: &[u8] = b"miopunch/contact-listener-forwarder/1";
    let directory = tempfile::tempdir().unwrap();
    let mut forwarder_node = NativeNetwork::create(directory.path().join("forwarder"))
        .await
        .unwrap();
    let mut config = forwarder_node.resource_configuration().unwrap();
    config.replacement = Some(vec![]);
    config.member_data_relay_disabled = true;
    forwarder_node.save_resource_configuration(&config).unwrap();
    let mut caller_node = admit(&mut forwarder_node, &directory.path().join("caller")).await;
    let receiver_node = admit(&mut forwarder_node, &directory.path().join("receiver")).await;
    for record in forwarder_node.management_history().await.unwrap() {
        caller_node
            .ingest_management_record(record.header(), record.body())
            .await
            .unwrap();
    }
    caller_node.save_resource_configuration(&config).unwrap();
    let mut relay_config = iroh_relay::server::ServerConfig::default();
    relay_config.relay = Some(iroh_relay::server::RelayConfig::new((
        Ipv4Addr::LOCALHOST,
        0,
    )));
    let relay = iroh_relay::server::Server::spawn(relay_config)
        .await
        .unwrap();
    let relay_url = relay.http_url().unwrap();
    let mut receiver_config = config.clone();
    receiver_config.replacement = Some(vec![Resource {
        url: relay_url.to_string(),
        uses: BTreeSet::from([ResourceUse::DataRelay]),
        limitation: "owned healthy relay; member forwards restricted contact only".into(),
    }]);
    receiver_node
        .save_resource_configuration(&receiver_config)
        .unwrap();
    let caller_identity = caller_node.status().node_id;
    let receiver_identity = receiver_node.status().node_id;
    let forwarder_identity = forwarder_node.status().node_id;
    assert!(caller_identity != receiver_identity && caller_identity != forwarder_identity);
    assert!(receiver_identity != forwarder_identity);
    caller_node.close().await;
    receiver_node.close().await;
    forwarder_node.close().await;
    let caller_node = NativeNetwork::open(directory.path().join("caller"))
        .await
        .unwrap();
    let receiver_node = NativeNetwork::open(directory.path().join("receiver"))
        .await
        .unwrap();
    let forwarder_node = NativeNetwork::open(directory.path().join("forwarder"))
        .await
        .unwrap();
    assert_eq!(caller_node.status().node_id, caller_identity);
    assert_eq!(receiver_node.status().node_id, receiver_identity);
    assert_eq!(forwarder_node.status().node_id, forwarder_identity);
    for node in [&forwarder_node, &caller_node, &receiver_node] {
        node.admission_gate()
            .register_protocol(LINK_ALPN, ProtocolScope::Member);
    }
    let forwarder_resources = forwarder_node
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    forwarder_resources
        .endpoint()
        .set_alpns(vec![LINK_ALPN.to_vec()]);
    let caller_transport = Arc::new(MemberSignalingTransport::new(
        forwarder_resources.endpoint().id(),
        caller_node.admission_gate(),
    ));
    let receiver_transport = Arc::new(MemberSignalingTransport::new(
        forwarder_resources.endpoint().id(),
        receiver_node.admission_gate(),
    ));
    let caller_resources = caller_node
        .bind_configured_endpoint_with(
            iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .bind_addr((Ipv4Addr::LOCALHOST, 0))
                .unwrap()
                .add_custom_transport(caller_transport.clone()),
        )
        .await
        .unwrap();
    let receiver = receiver_node
        .maintain_network_with_endpoint(
            Ipv4Addr::LOCALHOST,
            |builder| builder.add_custom_transport(receiver_transport.clone()),
            |router| router.accept(MEMBER_ALPN, Echo),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), receiver.endpoint().online())
        .await
        .unwrap();
    assert!(receiver.addr().relay_urls().any(|url| url == &relay_url));
    assert!(caller_resources.addr().relay_urls().next().is_none());
    let forwarder = MemberForwarder::new(forwarder_node.admission_gate());
    let mut leases = Vec::new();
    let mut links = Vec::new();
    for (endpoint, transport) in [
        (caller_resources.endpoint(), &caller_transport),
        (receiver.endpoint(), &receiver_transport),
    ] {
        let (outgoing, incoming) = tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(
                endpoint.connect(forwarder_resources.addr(), LINK_ALPN),
                async { forwarder_resources.endpoint().accept().await.unwrap().await }
            )
        })
        .await
        .unwrap();
        let outgoing = outgoing.unwrap();
        let incoming = incoming.unwrap();
        let (client_lease, server_lease) = tokio::join!(
            transport.attach(&outgoing, Duration::from_secs(30)),
            forwarder.accept_link(&incoming, Duration::from_secs(30)),
        );
        leases.extend([client_lease.unwrap(), server_lease.unwrap()]);
        links.extend([outgoing, incoming]);
    }
    assert_eq!(forwarder.active_links(), 2);
    let target = iroh::EndpointAddr::new(receiver.endpoint().id()).with_addrs([
        iroh::TransportAddr::Custom(receiver_transport.address().unwrap()),
    ]);
    assert!(
        target
            .addrs
            .iter()
            .all(|address| matches!(address, iroh::TransportAddr::Custom(_)))
    );
    let activity = receiver.business_activity();
    let started = tokio::time::Instant::now();
    let connection = caller_resources
        .coordinator()
        .connect(
            target,
            SessionPurpose::MemberBusiness,
            AttemptContext::new(
                n0_future::time::Instant::now() + Duration::from_secs(12),
                tokio_util::sync::CancellationToken::new(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        connection.remote_id().as_bytes(),
        receiver_identity.as_bytes()
    );
    assert!(
        connection
            .paths()
            .iter()
            .any(|path| path.is_selected() && path.is_ip())
    );
    ping(&connection).await;
    let caller_ms = started.elapsed().as_millis();
    assert!(receiver.addr().relay_urls().any(|url| url == &relay_url));
    assert!(
        links
            .iter()
            .all(|connection| connection.close_reason().is_none())
    );
    let sockets = [
        caller_resources.endpoint(),
        receiver.endpoint(),
        forwarder_resources.endpoint(),
    ]
    .map(|endpoint| *endpoint.addr().ip_addrs().next().unwrap());
    let cleanup = tokio::time::Instant::now();
    connection.close(0u32.into(), b"member contact supplement ended");
    drop(connection);
    for link in &links {
        link.close(0u32.into(), b"forwarding ended");
    }
    drop(links);
    drop(activity);
    receiver.close().await;
    caller_resources.close().await;
    forwarder_resources.close().await;
    caller_node.close().await;
    forwarder_node.close().await;
    tokio::time::timeout(Duration::from_secs(1), async {
        while leases.iter().any(|lease| !lease.is_finished()) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("owned forwarding tasks must end after endpoint close");
    assert_eq!(forwarder.active_links(), 0);
    drop(leases);
    let mut immediate_empty = Vec::new();
    for address in sockets {
        immediate_empty.push(tokio::net::UdpSocket::bind(address).await.is_ok());
    }
    let settlement = tokio::time::Instant::now();
    tokio::time::timeout(Duration::from_secs(1), async {
        for address in sockets {
            loop {
                match tokio::net::UdpSocket::bind(address).await {
                    Ok(socket) => {
                        drop(socket);
                        break;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                        tokio::time::sleep(Duration::from_millis(5)).await
                    }
                    Err(error) => panic!("socket retirement observation failed: {error}"),
                }
            }
        }
    })
    .await
    .expect("closed Node and forwarding sockets must retire promptly");
    let settled_ms = settlement.elapsed().as_millis();
    tokio::time::sleep(Duration::from_millis(100)).await;
    for address in sockets {
        drop(tokio::net::UdpSocket::bind(address).await.unwrap());
    }
    assert_eq!(forwarder.active_links(), 0);
    println!(
        "CONTACT_MEMBER_NODE_PASS formal_sdk=true strict_member_alpn=true identity_matches=true independent_members=3 stores_reopened=true receiver_relay_stayed_online=true caller_relay_disabled=true configured_public_mqtt=false restricted_member_contact_only=true actual_data_route=udp_loopback caller_ms={} cleanup_ms={} socket_immediate_empty={:?} socket_settled_ms={} socket_late_empty=true forwarding_tasks_completed=true",
        caller_ms,
        cleanup.elapsed().as_millis(),
        immediate_empty,
        settled_ms,
    );
    relay.shutdown().await.unwrap();
}
