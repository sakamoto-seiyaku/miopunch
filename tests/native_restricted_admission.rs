//! Native SDK admission through the constrained endpoint/control interfaces.
#[path = "support/relay_contact_broker.rs"]
mod owned_broker;
use iroh::{
    EndpointAddr,
    endpoint::{ControlConnection, RestrictedEndpoint},
};
use miopunch_native_probe::{
    admission_gate::ADMISSION_REQUEST_ALPN,
    manual_admission::{AdmissionStage, ManualExchange},
    native_network::NativeNetwork,
};
use std::{net::Ipv4Addr, sync::Arc, time::Duration};

fn local_resources(node: &NativeNetwork) {
    let mut resources = node.resource_configuration().unwrap();
    resources.replacement = Some(vec![]);
    resources.mqtt_facilities.clear();
    resources.lan_discovery = miopunch_native_probe::resources::LanDiscovery::Disabled;
    node.save_resource_configuration(&resources).unwrap();
}
async fn control(
    client: &RestrictedEndpoint,
    server: &RestrictedEndpoint,
) -> (Arc<ControlConnection>, Arc<ControlConnection>) {
    control_alpn(client, server, ADMISSION_REQUEST_ALPN).await
}
async fn control_alpn(
    client: &RestrictedEndpoint,
    server: &RestrictedEndpoint,
    alpn: &[u8],
) -> (Arc<ControlConnection>, Arc<ControlConnection>) {
    let target = EndpointAddr::new(server.id()).with_ip_addr(
        server
            .addr()
            .ip_addrs()
            .find(|a| a.is_ipv4())
            .copied()
            .unwrap(),
    );
    let deadline = n0_future::time::Instant::now() + Duration::from_secs(5);
    let options = iroh::endpoint::ConnectOptions::new().with_caller_deadline(deadline);
    let (a, b) = tokio::time::timeout(Duration::from_secs(6), async {
        tokio::join!(
            async {
                client
                    .connect_control_with_opts(target, alpn, options)
                    .await
                    .unwrap()
                    .await
            },
            async {
                server
                    .accept()
                    .await
                    .unwrap()
                    .with_caller_deadline(deadline)
                    .accept_control()
                    .unwrap()
                    .await
            }
        )
    })
    .await
    .unwrap();
    (Arc::new(a.unwrap()), Arc::new(b.unwrap()))
}
#[tokio::test]
async fn native_restricted_request_records_waiting_approval_without_business_access() {
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
    local_resources(&admin);
    local_resources(&joining);
    use miopunch_native_probe::{
        connection_coordinator::{AttemptContext, SessionPurpose},
        control_protocol::{AuthenticatedControl, ControlOwner},
    };
    let ae = admin
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let je = joining
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let _: &RestrictedEndpoint = ae.endpoint();
    let target = EndpointAddr::new(ae.endpoint().id())
        .with_ip_addr(ae.addr().ip_addrs().find(|a| a.is_ipv4()).copied().unwrap());
    let deadline = n0_future::time::Instant::now() + Duration::from_secs(5);
    let (client, server) =
        tokio::join!(je.connect_control(target, ADMISSION_REQUEST_ALPN), async {
            ae.coordinator()
                .accept_control(
                    ae.endpoint().accept().await.unwrap(),
                    SessionPurpose::AdmissionRequest,
                    AttemptContext::new(deadline, tokio_util::sync::CancellationToken::new()),
                )
                .await
        });
    let (client, server): (ControlOwner, ControlOwner) = (client.unwrap(), server.unwrap());
    let (response, served) = tokio::join!(
        joining.request_manual_admission(&client),
        admin.serve_manual_admission(&server)
    );
    let awaiting = matches!(response.unwrap(), ManualExchange::AwaitingApproval);
    let served = served.unwrap();
    let requests = admin.manual_requests().await.unwrap();
    let exact_request = requests.len() == 1 && requests[0].applicant == joining.status().node_id;
    let stage = joining.admission_stage();
    client.close(0u32.into(), b"owned typed request complete");
    server.close(0u32.into(), b"owned typed request complete");
    drop((client, server));
    tokio::join!(je.close(), ae.close());
    tokio::join!(joining.close(), admin.close());
    assert!(awaiting && exact_request);
    assert_eq!(served, AdmissionStage::AwaitingApproval);
    assert_eq!(stage, AdmissionStage::AwaitingApproval);
}

#[derive(Clone, Copy)]
enum MemberPath {
    Direct,
    MqttUdp,
    MqttRelay,
}
async fn approved_native_admission_story(path: MemberPath) {
    let mqtt = !matches!(path, MemberPath::Direct);
    let relay_only = matches!(path, MemberPath::MqttRelay);
    let dir = tempfile::tempdir().unwrap();
    let admin_path = dir.path().join("admin");
    let joining_path = dir.path().join("joining");
    let mut admin = NativeNetwork::create(&admin_path).await.unwrap();
    let mut joining = NativeNetwork::join(&joining_path, admin.manual_invitation().unwrap())
        .await
        .unwrap();
    let admin_id = admin.status().node_id;
    let joining_id = joining.status().node_id;
    local_resources(&admin);
    local_resources(&joining);
    let ae = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let je = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = control(&je, &ae).await;
    let (response, served) = tokio::join!(
        joining.request_manual_admission(&client),
        admin.serve_manual_admission(&server)
    );
    assert!(matches!(
        response.unwrap(),
        ManualExchange::AwaitingApproval
    ));
    served.unwrap();
    let request = admin.manual_requests().await.unwrap().pop().unwrap();
    admin.approve_manual_admission(&request).await.unwrap();
    client.close(0u32.into(), b"initial request complete");
    server.close(0u32.into(), b"initial request complete");
    drop((client, server));
    let (client, server) = control(&je, &ae).await;
    let receive = async {
        let ManualExchange::Materials(delivery) =
            joining.request_manual_admission(&client).await.unwrap()
        else {
            panic!("approved authorization delivery")
        };
        joining.complete_manual_admission(delivery).await.unwrap()
    };
    let (received, served) = tokio::join!(receive, admin.serve_manual_admission(&server));
    assert_eq!(received, AdmissionStage::Complete);
    assert_eq!(served.unwrap(), AdmissionStage::Complete);
    client.close(0u32.into(), b"saved material exchange complete");
    server.close(0u32.into(), b"saved material exchange complete");
    drop((client, server));
    tokio::join!(je.close(), ae.close());
    drop((je, ae));
    tokio::join!(joining.close(), admin.close());
    let admin = NativeNetwork::open(&admin_path).await.unwrap();
    let joining = NativeNetwork::open(&joining_path).await.unwrap();
    assert_eq!(admin.status().node_id, admin_id);
    assert_eq!(joining.status().node_id, joining_id);
    assert_eq!(joining.admission_stage(), AdmissionStage::Complete);
    let relay = if relay_only {
        let mut config = iroh_relay::server::ServerConfig::default();
        config.relay = Some(iroh_relay::server::RelayConfig::new((
            Ipv4Addr::LOCALHOST,
            0,
        )));
        Some(iroh_relay::server::Server::spawn(config).await.unwrap())
    } else {
        None
    };
    let relay_url = relay.as_ref().map(|server| server.http_url().unwrap());
    let broker = if mqtt {
        Some(owned_broker::Broker::new().await)
    } else {
        None
    };
    let mut carriers = Vec::new();
    let (ae, je) = if let Some(broker) = &broker {
        for node in [&admin, &joining] {
            let mut config = node.resource_configuration().unwrap();
            config.replacement = Some(vec![miopunch_native_probe::resources::Resource {
                url: format!("mqtt://{}/", broker.address),
                uses: [miopunch_native_probe::resources::ResourceUse::Signaling].into(),
                limitation: "owned Native restricted MQTT fixture".into(),
            }]);
            if let Some(url) = &relay_url {
                config.replacement.as_mut().unwrap().push(
                    miopunch_native_probe::resources::Resource {
                        url: url.to_string(),
                        uses: [miopunch_native_probe::resources::ResourceUse::DataRelay].into(),
                        limitation: "owned Native restricted actual Iroh relay fixture".into(),
                    },
                );
                config.udp_direct_disabled = true;
                config.tcp_direct_disabled = true;
            }
            node.save_resource_configuration(&config).unwrap();
        }
        let bind = || {
            let carrier = Arc::new(miopunch_native_probe::mqtt_transport::MqttTransport::new(
                broker.address,
                Arc::default(),
            ));
            let builder = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .alpns(vec![
                    miopunch_native_probe::connection_coordinator::MEMBER_ALPN.to_vec(),
                ])
                .bind_addr("127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap())
                .unwrap()
                .add_custom_transport(carrier.clone());
            let builder = if let Some(url) = &relay_url {
                builder.relay_mode(iroh::RelayMode::Custom(iroh::RelayMap::from_iter([
                    url.clone()
                ])))
            } else {
                builder
            };
            (builder, carrier)
        };
        let (a, ac) = bind();
        let (j, jc) = bind();
        let (a, j) = tokio::join!(admin.bind_endpoint_with(a), joining.bind_endpoint_with(j));
        let (a, j) = (a.unwrap(), j.unwrap());
        let (ar, jr) = tokio::join!(ac.wait_ready(), jc.wait_ready());
        ar.unwrap();
        jr.unwrap();
        carriers.extend([ac, jc]);
        (a, j)
    } else {
        (
            admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap(),
            joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap(),
        )
    };
    let target = if mqtt {
        EndpointAddr::new(ae.id()).with_addrs([iroh::TransportAddr::Custom(
            miopunch_native_probe::mqtt_transport::MqttTransport::address(
                &carriers[0].route().unwrap(),
            )
            .unwrap(),
        )])
    } else {
        EndpointAddr::new(ae.id())
            .with_ip_addr(ae.addr().ip_addrs().find(|a| a.is_ipv4()).copied().unwrap())
    };
    let deadline = n0_future::time::Instant::now() + Duration::from_secs(5);
    let mut options = iroh::endpoint::ConnectOptions::new().with_caller_deadline(deadline);
    if mqtt {
        options = options.with_bootstrap_route(target.addrs.iter().next().unwrap().clone());
    }
    let (a, b) = tokio::time::timeout(Duration::from_secs(6), async {
        tokio::join!(
            async {
                je.connect_with_opts(
                    target,
                    miopunch_native_probe::connection_coordinator::MEMBER_ALPN,
                    options,
                )
                .await
                .unwrap()
                .await
            },
            async {
                ae.accept()
                    .await
                    .unwrap()
                    .with_caller_deadline(deadline)
                    .await
            }
        )
    })
    .await
    .unwrap();
    let (passed, failure) = match (a, b) {
        (Ok(client), Ok(server)) => {
            let (mut send, mut recv) = client.open_bi().await.unwrap();
            send.write_all(b"PING").await.unwrap();
            send.finish().unwrap();
            let (mut reply, mut request) = server.accept_bi().await.unwrap();
            assert_eq!(request.read_to_end(4).await.unwrap(), b"PING");
            reply.write_all(b"PONG").await.unwrap();
            reply.finish().unwrap();
            let passed = recv.read_to_end(4).await.unwrap() == b"PONG"
                && [&client, &server].iter().all(|c| {
                    c.paths().iter().any(|p| {
                        p.is_selected() && if relay_only { p.is_relay() } else { p.is_ip() }
                    })
                });
            client.close(0u32.into(), b"typed member stream complete");
            server.close(0u32.into(), b"typed member stream complete");
            (passed, String::new())
        }
        (a, b) => (false, format!("client={:?}; server={:?}", a.err(), b.err())),
    };
    tokio::join!(je.close(), ae.close());
    drop((je, ae));
    tokio::join!(joining.close(), admin.close());
    for carrier in carriers {
        tokio::time::timeout(Duration::from_secs(2), carrier.wait_driver_closed())
            .await
            .unwrap()
            .unwrap();
    }
    if let Some(broker) = broker {
        tokio::time::timeout(Duration::from_secs(2), async {
            while broker.active() != 0 {
                tokio::time::sleep(Duration::from_millis(5)).await
            }
        })
        .await
        .unwrap();
        broker.close().await;
    }
    if let Some(relay) = relay {
        relay.shutdown().await.unwrap();
    }
    assert!(
        passed,
        "saved and reopened admission must release the original member connection: {failure}"
    );
}
#[tokio::test]
async fn approved_native_admission_reopens_identity_and_releases_member_stream() {
    approved_native_admission_story(MemberPath::Direct).await;
}
#[tokio::test]
async fn restricted_native_mqtt_bootstrap_builds_udp_on_original_reopened_member() {
    approved_native_admission_story(MemberPath::MqttUdp).await;
}

#[tokio::test]
async fn restricted_native_mqtt_bootstrap_qualifies_actual_relay_on_original_reopened_member() {
    approved_native_admission_story(MemberPath::MqttRelay).await;
}

#[tokio::test]
async fn restricted_native_precode_saves_materials_once_and_reopens_membership() {
    let dir = tempfile::tempdir().unwrap();
    let admin_path = dir.path().join("precode-admin");
    let joining_path = dir.path().join("precode-joining");
    let mut admin = NativeNetwork::create(&admin_path).await.unwrap();
    local_resources(&admin);
    let expiry = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    let code = admin.issue_precode(expiry, Some(1)).await.unwrap();
    let mut joining = NativeNetwork::join(&joining_path, code.invitation())
        .await
        .unwrap();
    local_resources(&joining);
    let identity = joining.status().node_id;
    let ae = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let je = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = control(&je, &ae).await;
    let receive = async {
        let ManualExchange::Materials(delivery) = joining
            .request_precode_admission(&client, &code)
            .await
            .unwrap()
        else {
            panic!("valid precode grants material")
        };
        joining.complete_manual_admission(delivery).await.unwrap()
    };
    let (received, served) = tokio::join!(receive, admin.serve_precode_admission(&server));
    assert_eq!(received, AdmissionStage::Complete);
    assert_eq!(served.unwrap(), AdmissionStage::Complete);
    assert_eq!(admin.precode_usage(&code).await.unwrap(), 1);
    client.close(0u32.into(), b"owned precode saved");
    server.close(0u32.into(), b"owned precode saved");
    drop((client, server));
    tokio::join!(je.close(), ae.close());
    drop((je, ae));
    tokio::join!(joining.close(), admin.close());
    let joining = NativeNetwork::open(&joining_path).await.unwrap();
    let admin = NativeNetwork::open(&admin_path).await.unwrap();
    assert_eq!(joining.status().node_id, identity);
    assert_eq!(joining.admission_stage(), AdmissionStage::Complete);
    assert_eq!(admin.precode_usage(&code).await.unwrap(), 1);
    tokio::join!(joining.close(), admin.close());
}

#[tokio::test]
async fn restricted_native_management_recovery_uses_ordinary_member_when_admin_is_offline() {
    use miopunch_native_probe::{
        daily_management::ManagementAction,
        genesis_recovery::GenesisCredential,
        management_recovery::{MANAGEMENT_RECOVERY_ALPN, RecoveryStage},
    };
    let dir = tempfile::tempdir().unwrap();
    let creator_path = dir.path().join("creator");
    let member_path = dir.path().join("member");
    let recovered_path = dir.path().join("recovered");
    let mut creator = NativeNetwork::create(&creator_path).await.unwrap();
    local_resources(&creator);
    let original = creator.status();
    let saved = std::fs::read(creator_path.join("genesis.credential")).unwrap();
    let backup = GenesisCredential::from_saved(
        saved[..32].try_into().unwrap(),
        p2panda_core::Hash::from_bytes(saved[32..].try_into().unwrap()),
    );
    let mut member = NativeNetwork::join(&member_path, creator.manual_invitation().unwrap())
        .await
        .unwrap();
    local_resources(&member);
    let ce = creator.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let me = member.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = control(&me, &ce).await;
    let (requested, served) = tokio::join!(
        member.request_manual_admission(&client),
        creator.serve_manual_admission(&server)
    );
    assert!(matches!(
        requested.unwrap(),
        ManualExchange::AwaitingApproval
    ));
    served.unwrap();
    let request = creator.manual_requests().await.unwrap().pop().unwrap();
    creator.approve_manual_admission(&request).await.unwrap();
    let receive = async {
        let ManualExchange::Materials(delivery) =
            member.request_manual_admission(&client).await.unwrap()
        else {
            panic!("approved ordinary member")
        };
        member.complete_manual_admission(delivery).await.unwrap()
    };
    let (received, served) = tokio::join!(receive, creator.serve_manual_admission(&server));
    assert_eq!(received, AdmissionStage::Complete);
    assert_eq!(served.unwrap(), AdmissionStage::Complete);
    client.close(0u32.into(), b"ordinary member saved");
    server.close(0u32.into(), b"ordinary member saved");
    drop((client, server));
    tokio::join!(me.close(), ce.close());
    drop((me, ce));
    creator.close().await;
    let me = member.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    me.set_alpns(vec![MANAGEMENT_RECOVERY_ALPN.to_vec()]);
    let mut recovered = NativeNetwork::recover(&recovered_path, &backup, me.id())
        .await
        .unwrap();
    local_resources(&recovered);
    let new_id = recovered.status().node_id;
    assert_ne!(new_id, original.node_id);
    assert_eq!(recovered.status().applied_count, 0);
    assert_eq!(
        recovered.recovery_progress().await.unwrap().stage,
        RecoveryStage::AwaitingProof
    );
    let re = recovered.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    re.set_alpns(vec![MANAGEMENT_RECOVERY_ALPN.to_vec()]);
    let (client, server) = control_alpn(&re, &me, MANAGEMENT_RECOVERY_ALPN).await;
    let (restored, served) = tokio::join!(
        recovered.restore_management(&client),
        member.serve_management_recovery(&server)
    );
    let restored = restored.unwrap();
    let served = served.unwrap();
    assert_eq!(restored.stage, RecoveryStage::Complete);
    assert_eq!(served.authorization, restored.authorization);
    assert_eq!(recovered.admission_stage(), AdmissionStage::Complete);
    assert_eq!(recovered.status().anchor, original.anchor);
    assert!(
        recovered
            .status()
            .members
            .iter()
            .any(|(id, _)| *id == original.node_id)
    );
    assert!(member.manual_requests().await.unwrap().is_empty());
    let mut session = recovered.begin_management_session();
    recovered
        .manage(
            &mut session,
            ManagementAction::AddMember(p2panda_core::SigningKey::generate().verifying_key()),
        )
        .await
        .unwrap();
    drop((client, server));
    tokio::join!(re.close(), me.close());
    drop((re, me));
    tokio::join!(recovered.close(), member.close());
    let recovered = NativeNetwork::open(&recovered_path).await.unwrap();
    assert_eq!(recovered.status().node_id, new_id);
    assert_eq!(recovered.status().anchor, original.anchor);
    assert_eq!(
        recovered.recovery_progress().await.unwrap().stage,
        RecoveryStage::Complete
    );
    assert_eq!(recovered.admission_stage(), AdmissionStage::Complete);
    recovered.close().await;
}

async fn precode_member(admin: &mut NativeNetwork, path: &std::path::Path) -> NativeNetwork {
    let expiry = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    let code = admin.issue_precode(expiry, Some(1)).await.unwrap();
    let mut node = NativeNetwork::join(path, code.invitation()).await.unwrap();
    local_resources(&node);
    let ae = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let ne = node.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = control(&ne, &ae).await;
    let receive = async {
        let ManualExchange::Materials(delivery) = node
            .request_precode_admission(&client, &code)
            .await
            .unwrap()
        else {
            panic!("precode member materials")
        };
        node.complete_manual_admission(delivery).await.unwrap()
    };
    let (received, served) = tokio::join!(receive, admin.serve_precode_admission(&server));
    assert_eq!(received, AdmissionStage::Complete);
    assert_eq!(served.unwrap(), AdmissionStage::Complete);
    client.close(0u32.into(), b"precode member saved");
    server.close(0u32.into(), b"precode member saved");
    drop((client, server));
    tokio::join!(ae.close(), ne.close());
    node
}

#[tokio::test]
async fn restricted_native_rotation_waits_for_saved_ack_and_reopens_on_new_generation() {
    use miopunch_native_probe::{
        admission_rotation::{ADMISSION_ROTATION_ALPN, RotationStage},
        genesis_recovery::GenesisCredential,
    };
    let dir = tempfile::tempdir().unwrap();
    let admin_path = dir.path().join("rotation-admin");
    let member_path = dir.path().join("rotation-member");
    let mut admin = NativeNetwork::create(&admin_path).await.unwrap();
    local_resources(&admin);
    let mut member = precode_member(&mut admin, &member_path).await;
    let identities = [admin.status().node_id, member.status().node_id];
    let anchor = admin.status().anchor;
    let saved = std::fs::read(admin_path.join("genesis.credential")).unwrap();
    let backup = GenesisCredential::from_saved(
        saved[..32].try_into().unwrap(),
        p2panda_core::Hash::from_bytes(saved[32..].try_into().unwrap()),
    );
    let mut session = admin.begin_management_session();
    admin
        .elevate_genesis(&mut session, backup, Duration::from_secs(60))
        .unwrap();
    let started = admin.begin_admission_rotation(&mut session).await.unwrap();
    assert_eq!(started.generation, 1);
    assert_eq!(started.retained.len(), 2);
    assert_eq!(started.acknowledged.len(), 1);
    assert!(
        admin.finish_admission_rotation(&mut session).await.is_err(),
        "must wait for retained member's durable receipt"
    );
    for phase in 0..2 {
        let ae = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
        let me = member.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
        let (client, server) = control_alpn(&ae, &me, ADMISSION_ROTATION_ALPN).await;
        let (sent, received) = tokio::join!(
            admin.distribute_admission_rotation(&client),
            member.receive_admission_rotation(&server)
        );
        sent.unwrap();
        received.unwrap();
        client.close(0u32.into(), b"rotation exchange complete");
        server.close(0u32.into(), b"rotation exchange complete");
        drop((client, server));
        tokio::join!(ae.close(), me.close());
        if phase == 0 {
            assert_eq!(
                admin
                    .rotation_progress()
                    .await
                    .unwrap()
                    .unwrap()
                    .acknowledged
                    .len(),
                2
            );
            assert_eq!(
                admin
                    .finish_admission_rotation(&mut session)
                    .await
                    .unwrap()
                    .stage,
                RotationStage::Complete
            );
        }
    }
    tokio::join!(admin.close(), member.close());
    let mut admin = NativeNetwork::open(&admin_path).await.unwrap();
    let mut member = NativeNetwork::open(&member_path).await.unwrap();
    for (node, id) in [(&admin, identities[0]), (&member, identities[1])] {
        assert_eq!(node.status().node_id, id);
        assert_eq!(node.status().anchor, anchor);
        let progress = node.rotation_progress().await.unwrap().unwrap();
        assert_eq!(progress.generation, 1);
        assert_eq!(progress.stage, RotationStage::Complete);
    }
    let ae = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let me = member.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = control_alpn(&ae, &me, ADMISSION_ROTATION_ALPN).await;
    let (sent, received) = tokio::join!(
        admin.distribute_admission_rotation(&client),
        member.receive_admission_rotation(&server)
    );
    sent.unwrap();
    received.unwrap();
    client.close(0u32.into(), b"new generation authenticated");
    server.close(0u32.into(), b"new generation authenticated");
    drop((client, server));
    tokio::join!(ae.close(), me.close());
    tokio::join!(admin.close(), member.close());
}

#[tokio::test]
async fn restricted_native_history_sync_saves_and_applies_records_before_acknowledging() {
    use miopunch_native_probe::{
        daily_management::ManagementAction, network_recovery::RECOVERY_REPLICA_ALPN,
    };
    let dir = tempfile::tempdir().unwrap();
    let admin_path = dir.path().join("replica-admin");
    let member_path = dir.path().join("replica-member");
    let mut admin = NativeNetwork::create(&admin_path).await.unwrap();
    local_resources(&admin);
    let mut member = precode_member(&mut admin, &member_path).await;
    let saved_count = usize::try_from(member.status().applied_count).unwrap();
    let added = p2panda_core::SigningKey::generate().verifying_key();
    let mut session = admin.begin_management_session();
    admin
        .manage(&mut session, ManagementAction::AddMember(added))
        .await
        .unwrap();
    let ae = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let me = member.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = control_alpn(&me, &ae, RECOVERY_REPLICA_ALPN).await;
    for _ in 0..2 {
        let (received, served) =
            tokio::join!(member.sync_replica(&client), admin.serve_replica(&server));
        let outcome = received.unwrap();
        served.unwrap();
        assert!(outcome.complete());
        assert!(outcome.scope.len() > saved_count);
        assert!(member.status().members.iter().any(|(id, _)| *id == added));
        assert_eq!(
            member.management_history().await.unwrap(),
            admin.management_history().await.unwrap()
        );
    }
    client.close(0u32.into(), b"replica complete");
    server.close(0u32.into(), b"replica complete");
    drop((client, server));
    tokio::join!(ae.close(), me.close());
    tokio::join!(admin.close(), member.close());
    let member = NativeNetwork::open(&member_path).await.unwrap();
    assert!(member.status().members.iter().any(|(id, _)| *id == added));
    member.close().await;
}

#[tokio::test]
async fn restricted_native_startup_recovers_from_a_previously_unknown_online_member() {
    use miopunch_native_probe::{
        network_recovery::RecoveryStatus,
        resources::{Resource, ResourceUse},
    };
    let dir = tempfile::tempdir().unwrap();
    let creator_path = dir.path().join("startup-creator");
    let a_path = dir.path().join("startup-a");
    let c_path = dir.path().join("startup-c");
    let mut creator = NativeNetwork::create(&creator_path).await.unwrap();
    local_resources(&creator);
    let a = precode_member(&mut creator, &a_path).await;
    a.close().await;
    let c = precode_member(&mut creator, &c_path).await;
    let c_id = c.status().node_id;
    creator.close().await;
    let mut a = NativeNetwork::open(&a_path).await.unwrap();
    assert!(!a.status().members.iter().any(|(id, _)| *id == c_id));
    let broker = owned_broker::Broker::new().await;
    for node in [&a, &c] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(vec![Resource {
            url: format!("mqtt://{}/", broker.address),
            uses: [ResourceUse::Signaling, ResourceUse::Lookup].into(),
            limitation: "owned Native restricted startup recovery fixture".into(),
        }]);
        node.save_resource_configuration(&config).unwrap();
    }
    let ce = c
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let serving = async {
        tokio::time::timeout(Duration::from_secs(12), async {
            let control = Arc::new(
                ce.endpoint()
                    .accept()
                    .await
                    .ok_or("provider closed")?
                    .accept_control()
                    .map_err(|_| "accept control")?
                    .await
                    .map_err(|_| "authenticate control")?,
            );
            c.serve_network_recovery(&ce, &control)
                .await
                .map_err(|_| "serve actual recovery")
        })
        .await
        .map_err(|_| "provider timeout")
        .and_then(|result| result)
    };
    let (recovered, served) = tokio::join!(a.resume_network(Ipv4Addr::LOCALHOST), serving);
    let recovered = recovered.unwrap();
    let ready = matches!(&recovered.status,RecoveryStatus::Ready(outcome) if outcome.complete() && outcome.source==ce.endpoint().id());
    recovered.resources.close().await;
    ce.close().await;
    tokio::join!(a.close(), c.close());
    tokio::time::timeout(Duration::from_secs(2), async {
        while broker.active() != 0 {
            tokio::time::sleep(Duration::from_millis(5)).await
        }
    })
    .await
    .unwrap();
    broker.close().await;
    assert!(
        ready,
        "startup requires verified and saved history from the unknown online member"
    );
    assert!(served.is_ok(), "{served:?}");
    let a = NativeNetwork::open(&a_path).await.unwrap();
    assert!(a.status().members.iter().any(|(id, _)| *id == c_id));
    a.close().await;
}

#[tokio::test]
async fn restricted_native_maintenance_routes_bootstrap_history_as_control() {
    use miopunch_native_probe::{
        network_recovery::RecoveryStatus,
        resources::{Resource, ResourceUse},
    };
    let dir = tempfile::tempdir().unwrap();
    let a_path = dir.path().join("maintained-a");
    let mut creator = NativeNetwork::create(dir.path().join("maintained-creator"))
        .await
        .unwrap();
    local_resources(&creator);
    let a = precode_member(&mut creator, &a_path).await;
    a.close().await;
    let c = precode_member(&mut creator, &dir.path().join("maintained-c")).await;
    let c_id = c.status().node_id;
    creator.close().await;
    let mut a = NativeNetwork::open(&a_path).await.unwrap();
    assert!(!a.status().members.iter().any(|(id, _)| *id == c_id));
    let broker = owned_broker::Broker::new().await;
    for node in [&a, &c] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(vec![Resource {
            url: format!("mqtt://{}/", broker.address),
            uses: [ResourceUse::Signaling, ResourceUse::Lookup].into(),
            limitation: "owned Native maintenance control routing fixture".into(),
        }]);
        node.save_resource_configuration(&config).unwrap();
    }
    let maintained = c.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    let recovered = a.resume_network(Ipv4Addr::LOCALHOST).await.unwrap();
    let ready = matches!(&recovered.status,RecoveryStatus::Ready(outcome) if outcome.complete() && outcome.source==maintained.endpoint().id());
    recovered.resources.close().await;
    maintained.close().await;
    a.close().await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while broker.active() != 0 {
            tokio::time::sleep(Duration::from_millis(5)).await
        }
    })
    .await
    .unwrap();
    broker.close().await;
    assert!(
        ready,
        "the public maintenance router must serve bootstrap history through the restricted control handler"
    );
    let a = NativeNetwork::open(&a_path).await.unwrap();
    assert!(a.status().members.iter().any(|(id, _)| *id == c_id));
    a.close().await;
}

#[tokio::test]
async fn restricted_native_incoming_uses_saved_contact_deadline_through_legacy_hook() {
    use iroh::{
        endpoint::{AfterHandshakeOutcome, Connection, EndpointHooks},
        protocol::{AcceptError, ProtocolHandler},
    };
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    #[derive(Clone, Debug)]
    struct SlowHook {
        state: Arc<AtomicUsize>,
        remaining: Arc<Mutex<Option<Duration>>>,
    }
    struct InHook(Arc<AtomicUsize>);
    impl Drop for InHook {
        fn drop(&mut self) {
            self.0.fetch_or(2, Ordering::SeqCst);
        }
    }
    impl EndpointHooks for SlowHook {
        async fn after_handshake(&self, connection: &Connection) -> AfterHandshakeOutcome {
            self.state.fetch_or(1, Ordering::SeqCst);
            *self.remaining.lock().unwrap() = connection.caller_deadline().map(|deadline| {
                deadline.saturating_duration_since(n0_future::time::Instant::now())
            });
            let _entered = InHook(self.state.clone());
            tokio::time::sleep(Duration::from_secs(3)).await;
            AfterHandshakeOutcome::Accept
        }
    }
    #[derive(Debug)]
    struct Business(Arc<AtomicUsize>);
    impl ProtocolHandler for Business {
        async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
            self.0.fetch_or(4, Ordering::SeqCst);
            connection.close(0u32.into(), b"unexpected late business release");
            Ok(())
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("deadline-admin"))
        .await
        .unwrap();
    local_resources(&admin);
    let member = precode_member(&mut admin, &dir.path().join("deadline-member")).await;
    let mut config = admin.resource_configuration().unwrap();
    config.maintenance.contact_timeout_ms = 600;
    admin.save_resource_configuration(&config).unwrap();
    let state = Arc::new(AtomicUsize::new(0));
    let remaining = Arc::new(Mutex::new(None));
    let hook = SlowHook {
        state: state.clone(),
        remaining: remaining.clone(),
    };
    let business = Business(state.clone());
    let maintained = admin
        .maintain_network_with_endpoint(
            Ipv4Addr::LOCALHOST,
            move |builder| builder.hooks(hook),
            move |router| {
                router.accept(
                    miopunch_native_probe::connection_coordinator::MEMBER_ALPN,
                    business,
                )
            },
        )
        .await
        .unwrap();
    let endpoint = member.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let target = EndpointAddr::new(maintained.endpoint().id()).with_ip_addr(
        maintained
            .addr()
            .ip_addrs()
            .find(|address| address.is_ipv4())
            .copied()
            .unwrap(),
    );
    let started = std::time::Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        endpoint.connect(
            target,
            miopunch_native_probe::connection_coordinator::MEMBER_ALPN,
        ),
    )
    .await;
    let elapsed = started.elapsed();
    let rejected = matches!(&result, Ok(Err(_)));
    if let Ok(Ok(connection)) = result {
        connection.close(0u32.into(), b"deadline test ended");
    }
    maintained.close().await;
    endpoint.close().await;
    member.close().await;
    let observed = state.load(Ordering::SeqCst);
    let remaining = *remaining.lock().unwrap();
    assert!(
        rejected,
        "the incoming setup must expire before a slow legacy hook can release business"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "saved 600 ms contact budget was refreshed or lost: {elapsed:?}"
    );
    assert_eq!(
        observed, 3,
        "the entered hook must be dropped and the business handler must remain uncalled"
    );
    assert!(
        remaining.is_some_and(|remaining| remaining <= Duration::from_millis(600)),
        "hook must see the original incoming budget: {remaining:?}"
    );
}
