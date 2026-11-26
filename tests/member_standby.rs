#![cfg(feature = "nat-lab-test-hooks")]

use std::{net::Ipv4Addr, time::Duration};

use miopunch_native_probe::{
    device_operation::{DeviceAction, DeviceOperation},
    manual_admission::AdmissionStage,
    native_client::NativeClient,
    native_network::{NativeNetwork, member_fusion_attempt_id},
    resources::{Resource, ResourceConfig, ResourceTable, ResourceUse},
    service_policy,
};
use tokio::time::{sleep, timeout};

async fn admitted(
    root: &std::path::Path,
    resources: &ResourceConfig,
) -> (NativeClient, NativeClient) {
    let provider = NativeClient::create_with_resources(
        root.join("provider"),
        Ipv4Addr::LOCALHOST,
        resources.clone(),
    )
    .await
    .unwrap();
    provider
        .export_recovery_card_to(root.join("card"), None)
        .await
        .unwrap();
    provider.acknowledge_recovery_saved().await.unwrap();
    let invitation = provider
        .issue_precode(service_policy::now() + 300, Some(1))
        .await
        .unwrap();
    let consumer = NativeClient::join_with_resources(
        root.join("consumer"),
        &invitation,
        Ipv4Addr::LOCALHOST,
        resources.clone(),
    )
    .await
    .unwrap();
    assert_eq!(
        consumer.request_admission(&invitation).await.unwrap(),
        AdmissionStage::Complete
    );
    (provider, consumer)
}

async fn relay() -> iroh_relay::server::Server {
    let mut config = iroh_relay::server::ServerConfig::default();
    config.relay = Some(iroh_relay::server::RelayConfig::new((
        Ipv4Addr::LOCALHOST,
        0,
    )));
    iroh_relay::server::Server::spawn(config).await.unwrap()
}

fn resources(url: &iroh::RelayUrl) -> ResourceConfig {
    let mut resources = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![Resource {
            url: url.to_string(),
            uses: [ResourceUse::DataRelay].into(),
            limitation: "owned standby startup fixture".into(),
        }],
    });
    resources.udp_direct_disabled = true;
    resources.tcp_direct_disabled = true;
    resources.maintenance.exploration_interval_ms = 250;
    resources.maintenance.contact_timeout_ms = 3000;
    resources
}

async fn standby(
    client: &NativeClient,
    peer: iroh::EndpointId,
) -> iroh::endpoint::MemberConnection {
    timeout(Duration::from_secs(12), async {
        loop {
            if let Ok(connection) = client.test_member_connection(peer).await {
                assert!(connection.close_reason().is_none());
                return connection;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("normal startup establishes an admitted session before any business")
}

async fn five_idle_peers(
    policy: miopunch_native_probe::network_maintenance::MaintenancePolicy,
    legacy: Option<usize>,
) {
    let relay = relay().await;
    let root = tempfile::tempdir().unwrap();
    let mut config = resources(&relay.http_url().unwrap());
    config.maintenance = policy;
    config.maintenance.exploration_interval_ms = 250;
    config.maintenance.contact_timeout_ms = 3000;
    let limit = config.maintenance.max_concurrent_contacts;
    let hub_path = root.path().join("hub");
    let hub = NativeClient::create_with_resources(&hub_path, Ipv4Addr::LOCALHOST, config.clone())
        .await
        .unwrap();
    hub.export_recovery_card_to(root.path().join("card"), None)
        .await
        .unwrap();
    hub.acknowledge_recovery_saved().await.unwrap();
    let invitation = hub
        .issue_precode(service_policy::now() + 600, Some(6))
        .await
        .unwrap();
    let mut peers = Vec::new();
    for index in 0..6 {
        let path = root.path().join(format!("peer-{index}"));
        let peer = NativeClient::join_with_resources(
            &path,
            &invitation,
            Ipv4Addr::LOCALHOST,
            config.clone(),
        )
        .await
        .unwrap();
        assert_eq!(
            peer.request_admission(&invitation).await.unwrap(),
            AdmissionStage::Complete
        );
        peers.push((peer.address().id, path));
        peer.close().await.unwrap();
    }
    let hub_id = hub.address().id;
    hub.close().await.unwrap();
    if let Some(legacy_limit) = legacy {
        let path = hub_path.join("resources.json");
        let mut saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let maintenance = saved["maintenance"].as_object_mut().unwrap();
        maintenance.remove("standby_member_limit");
        maintenance.insert(
            "max_standby_member_connections".into(),
            legacy_limit.into(),
        );
        std::fs::write(path, serde_json::to_vec(&saved).unwrap()).unwrap();
    }
    peers.sort_by_key(|(id, _)| *id);
    let offline = peers.remove(0);
    let hub = NativeClient::open(&hub_path, Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let mut online = Vec::new();
    for (id, path) in &peers {
        online.push((
            *id,
            NativeClient::open(path, Ipv4Addr::LOCALHOST).await.unwrap(),
        ));
        sleep(Duration::from_millis(150)).await;
    }
    let mut max_pending = 0;
    let originals = timeout(Duration::from_secs(40), async {
        loop {
            let pending = peers
                .iter()
                .chain(std::iter::once(&offline))
                .filter(|(id, _)| hub.test_member_dial_pending(*id))
                .count();
            max_pending = max_pending.max(pending);
            assert!(
                pending <= limit,
                "pending contacts {pending} exceed node budget {limit}"
            );
            let mut connections = Vec::new();
            for (id, peer) in &online {
                if let (Ok(a), Ok(b)) = (
                    hub.test_member_connection(*id).await,
                    peer.test_member_connection(hub_id).await,
                ) {
                    if a.close_reason().is_none() && b.close_reason().is_none() {
                        assert_eq!(
                            member_fusion_attempt_id(&a).unwrap(),
                            member_fusion_attempt_id(&b).unwrap()
                        );
                        assert!(a.paths().iter().any(|p| p.is_selected() && p.is_relay()));
                        connections.push((a, b));
                    }
                }
            }
            if connections.len() == 5
                && connections
                    .iter()
                    .all(|(a, b)| a.close_reason().is_none() && b.close_reason().is_none())
            {
                break connections;
            }
            sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("all five admitted peers remain simultaneously idle before the first product ping");
    // The normal host grants an initial 60-second first-stream handoff. Cross
    // that window without traffic so temporary handoffs cannot prove retention.
    let idle_until = tokio::time::Instant::now() + Duration::from_secs(61);
    while tokio::time::Instant::now() < idle_until {
        let pending = peers
            .iter()
            .chain(std::iter::once(&offline))
            .filter(|(id, _)| hub.test_member_dial_pending(*id))
            .count();
        max_pending = max_pending.max(pending);
        assert!(
            pending <= limit,
            "pending contacts {pending} exceed node budget {limit}"
        );
        assert!(
            originals
                .iter()
                .all(|(a, b)| a.close_reason().is_none() && b.close_reason().is_none()),
            "healthy idle sessions survive the fifth peer"
        );
        sleep(Duration::from_millis(100)).await;
    }
    let identities: Vec<_> = originals
        .iter()
        .map(|(a, b)| {
            serde_json::json!({
                "peer": a.remote_id(), "hubStableId": a.stable_id(), "peerStableId": b.stable_id(),
                "attempt": hex::encode(member_fusion_attempt_id(a).unwrap()),
            })
        })
        .collect();
    eprintln!(
        "{}",
        serde_json::json!({"event":"five-idle-members-before-business", "connections": identities, "legacy":legacy, "maxConcurrentContacts":limit, "maxObservedPending":max_pending})
    );
    for _ in 0..2 {
        for ((id, peer), (a, b)) in online.iter().zip(&originals) {
            let reply = hub.ping(peer.address()).await.unwrap_or_else(|error| {
                panic!(
                    "product ping {id}: {error}; original close reasons {:?}/{:?}",
                    a.close_reason(),
                    b.close_reason()
                )
            });
            assert_eq!(reply["reply"], "PONG");
            assert_eq!(reply["transport"]["connection"], a.stable_id());
            assert_eq!(
                peer.test_member_connection(hub_id)
                    .await
                    .unwrap()
                    .stable_id(),
                b.stable_id()
            );
            assert_eq!(
                hub.test_member_connection(*id).await.unwrap().stable_id(),
                a.stable_id()
            );
            assert!(
                originals
                    .iter()
                    .all(|(a, b)| a.close_reason().is_none() && b.close_reason().is_none()),
                "each completed business leaves all five originals alive"
            );
        }
    }
    assert!(
        originals
            .iter()
            .all(|(a, b)| a.close_reason().is_none() && b.close_reason().is_none()),
        "business completion returns every healthy original to standby"
    );
    eprintln!(
        "FIVE_STANDBY_READY peers=5 relay_only=5 budget={limit} max_pending={max_pending} legacy={legacy:?}"
    );
    for (_, peer) in online {
        peer.close().await.unwrap();
    }
    hub.close().await.unwrap();
    relay.shutdown().await.unwrap();
}

#[tokio::test]
async fn conservative_node_retains_five_idle_peers() {
    five_idle_peers(
        miopunch_native_probe::network_maintenance::MaintenancePolicy::conservative(),
        None,
    )
    .await;
}

#[tokio::test]
async fn active_node_retains_five_idle_peers() {
    five_idle_peers(
        miopunch_native_probe::network_maintenance::MaintenancePolicy::active(),
        None,
    )
    .await;
}

#[tokio::test]
async fn old_conservative_default_retains_five_idle_peers_after_reopen() {
    five_idle_peers(
        miopunch_native_probe::network_maintenance::MaintenancePolicy::conservative(),
        Some(1),
    )
    .await;
}

#[tokio::test]
async fn old_active_default_retains_five_idle_peers_after_reopen() {
    five_idle_peers(
        miopunch_native_probe::network_maintenance::MaintenancePolicy::active(),
        Some(4),
    )
    .await;
}

#[tokio::test]
async fn startup_and_peer_restart_prepare_the_session_before_first_ping() {
    let relay = relay().await;
    let resources = resources(&relay.http_url().unwrap());
    let root = tempfile::tempdir().unwrap();
    let provider_path = root.path().join("provider");
    let consumer_path = root.path().join("consumer");
    let (provider, consumer) = admitted(root.path(), &resources).await;
    let provider_id = provider.address().id;
    let consumer_id = consumer.address().id;
    consumer.close().await.unwrap();
    provider.close().await.unwrap();

    // No product ping or demo heartbeat has ever run. Reopen only the two hosts.
    let (provider, consumer) = tokio::join!(
        NativeClient::open(&provider_path, Ipv4Addr::LOCALHOST),
        NativeClient::open(&consumer_path, Ipv4Addr::LOCALHOST),
    );
    let provider = provider.unwrap();
    let consumer = consumer.unwrap();
    let (provider_session, consumer_session) = tokio::join!(
        standby(&provider, consumer_id),
        standby(&consumer, provider_id)
    );
    assert_eq!(
        member_fusion_attempt_id(&provider_session).unwrap(),
        member_fusion_attempt_id(&consumer_session).unwrap()
    );
    let reply = consumer.ping(provider.address()).await.unwrap();
    assert_eq!(reply["reply"], "PONG");
    assert_eq!(
        reply["transport"]["connection"],
        consumer_session.stable_id()
    );
    assert_eq!(
        standby(&provider, consumer_id).await.stable_id(),
        provider_session.stable_id()
    );

    provider.close().await.unwrap();
    sleep(Duration::from_millis(1500)).await;
    let provider = NativeClient::open(&provider_path, Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let (new_provider, new_consumer) = tokio::join!(
        standby(&provider, consumer_id),
        standby(&consumer, provider_id)
    );
    assert_eq!(
        member_fusion_attempt_id(&new_provider).unwrap(),
        member_fusion_attempt_id(&new_consumer).unwrap()
    );
    assert_ne!(new_provider.stable_id(), provider_session.stable_id());
    assert_ne!(new_consumer.stable_id(), consumer_session.stable_id());
    assert!(provider_session.close_reason().is_some());
    assert!(consumer_session.close_reason().is_some());
    let reply = consumer.ping(provider.address()).await.unwrap();
    assert_eq!(reply["reply"], "PONG");
    assert_eq!(
        reply["transport"]["connection"],
        new_consumer.stable_id(),
        "prepared session closed as {:?}",
        new_consumer.close_reason()
    );
    consumer.close().await.unwrap();
    provider.close().await.unwrap();
    relay.shutdown().await.unwrap();
}

#[tokio::test]
async fn first_product_ping_joins_crossed_background_dials() {
    let relay = relay().await;
    let root = tempfile::tempdir().unwrap();
    let (provider, consumer) = admitted(root.path(), &resources(&relay.http_url().unwrap())).await;
    let provider_id = provider.address().id;
    let consumer_id = consumer.address().id;
    consumer.close().await.unwrap();
    provider.close().await.unwrap();

    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let proxy_url: iroh::RelayUrl = format!("http://{}", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let backend = (
        Ipv4Addr::LOCALHOST,
        relay.http_url().unwrap().port().unwrap(),
    );
    let (release, ready) = tokio::sync::watch::channel(false);
    let proxy = tokio::spawn(async move {
        let mut circuits = tokio::task::JoinSet::new();
        loop {
            let (mut peer, _) = listener.accept().await.unwrap();
            let mut ready = ready.clone();
            circuits.spawn(async move {
                while !*ready.borrow_and_update() {
                    ready.changed().await.unwrap();
                }
                let mut remote = tokio::net::TcpStream::connect(backend).await.unwrap();
                let _ = tokio::io::copy_bidirectional(&mut peer, &mut remote).await;
            });
        }
    });
    for name in ["provider", "consumer"] {
        let node = NativeNetwork::open(root.path().join(name)).await.unwrap();
        let mut config = resources(&proxy_url);
        config.maintenance.contact_timeout_ms = 8000;
        node.save_resource_configuration(&config).unwrap();
        node.close().await;
    }
    let (provider, consumer) = tokio::join!(
        NativeClient::open(root.path().join("provider"), Ipv4Addr::LOCALHOST),
        NativeClient::open(root.path().join("consumer"), Ipv4Addr::LOCALHOST),
    );
    let provider = provider.unwrap();
    let consumer = consumer.unwrap();
    timeout(Duration::from_secs(3), async {
        while !provider.test_member_dial_pending(consumer_id)
            || !consumer.test_member_dial_pending(provider_id)
        {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("both normal hosts started their background dial before the product request");
    let reply = {
        let ping = consumer.ping(iroh::EndpointAddr::new(provider_id));
        tokio::pin!(ping);
        assert!(
            timeout(Duration::from_millis(50), ping.as_mut())
                .await
                .is_err()
        );
        release.send_replace(true);
        timeout(Duration::from_secs(6), ping.as_mut())
            .await
            .unwrap()
            .unwrap()
    };
    assert_eq!(reply["reply"], "PONG");
    let (a, b) = tokio::join!(
        standby(&provider, consumer_id),
        standby(&consumer, provider_id)
    );
    assert_eq!(
        member_fusion_attempt_id(&a).unwrap(),
        member_fusion_attempt_id(&b).unwrap()
    );
    assert_eq!(reply["transport"]["connection"], b.stable_id());
    consumer.close().await.unwrap();
    provider.close().await.unwrap();
    proxy.abort();
    relay.shutdown().await.unwrap();
}

#[tokio::test]
async fn standby_respects_current_membership_and_disabled_relay_on_reopen() {
    let relay = relay().await;
    let root = tempfile::tempdir().unwrap();
    let (provider, consumer) = admitted(root.path(), &resources(&relay.http_url().unwrap())).await;
    let consumer_id = consumer.address().id;
    let provider_id = provider.address().id;
    let _ = tokio::join!(
        standby(&provider, consumer_id),
        standby(&consumer, provider_id)
    );
    consumer.close().await.unwrap();
    let consumer_path = root.path().join("consumer");
    let node = NativeNetwork::open(&consumer_path).await.unwrap();
    let mut config = node.resource_configuration().unwrap();
    config.disabled_uses.insert(ResourceUse::DataRelay);
    node.save_resource_configuration(&config).unwrap();
    node.close().await;
    let consumer = NativeClient::open(&consumer_path, Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    sleep(Duration::from_millis(1500)).await;
    assert!(
        consumer.test_member_connection(provider_id).await.is_err(),
        "disabled relay cannot establish standby"
    );
    consumer.close().await.unwrap();
    let node = NativeNetwork::open(&consumer_path).await.unwrap();
    config.disabled_uses.remove(&ResourceUse::DataRelay);
    node.save_resource_configuration(&config).unwrap();
    node.close().await;
    let consumer = NativeClient::open(&consumer_path, Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        standby(&provider, consumer_id),
        standby(&consumer, provider_id)
    );
    let proposal = provider
        .propose_device_operation(
            DeviceAction::RemoveMember,
            consumer.status().await.node.node_id,
        )
        .await
        .unwrap();
    let operation: DeviceOperation = serde_json::from_value(proposal["operation"].clone()).unwrap();
    assert_eq!(
        provider
            .execute_device_operation(operation.id)
            .await
            .unwrap()["effective_locally"],
        true
    );
    timeout(Duration::from_secs(3), async {
        while a.close_reason().is_none() || b.close_reason().is_none() {
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("revocation closes the already prepared member session");
    sleep(Duration::from_millis(1200)).await;
    assert!(!provider.test_member_dial_pending(consumer_id));
    assert!(provider.test_member_connection(consumer_id).await.is_err());
    consumer.close().await.unwrap();
    provider.close().await.unwrap();
    relay.shutdown().await.unwrap();
}

#[tokio::test]
async fn offline_member_leaves_a_contact_opportunity_for_the_next_member() {
    let relay = relay().await;
    let root = tempfile::tempdir().unwrap();
    let mut config = resources(&relay.http_url().unwrap());
    config.maintenance.standby_member_limit = Some(4);
    let (provider, first) = admitted(root.path(), &config).await;
    let invitation = provider
        .issue_precode(service_policy::now() + 300, Some(1))
        .await
        .unwrap();
    let second = NativeClient::join_with_resources(
        root.path().join("second"),
        &invitation,
        Ipv4Addr::LOCALHOST,
        config,
    )
    .await
    .unwrap();
    assert_eq!(
        second.request_admission(&invitation).await.unwrap(),
        AdmissionStage::Complete
    );
    let provider_id = provider.address().id;
    let ids = [first.address().id, second.address().id];
    first.close().await.unwrap();
    second.close().await.unwrap();
    provider.close().await.unwrap();
    let (offline_id, online_id, online_name) = if ids[0] < ids[1] {
        (ids[0], ids[1], "second")
    } else {
        (ids[1], ids[0], "consumer")
    };
    // Only the hub initiates. The other host accepts and delivers the first stream.
    let node = NativeNetwork::open(root.path().join(online_name))
        .await
        .unwrap();
    let mut config = node.resource_configuration().unwrap();
    config.maintenance.standby_member_limit = Some(0);
    node.save_resource_configuration(&config).unwrap();
    node.close().await;
    let online = NativeClient::open(root.path().join(online_name), Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let provider = NativeClient::open(root.path().join("provider"), Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    timeout(Duration::from_secs(2), async {
        while !provider.test_member_dial_pending(offline_id) {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the first scheduled identity is offline");
    let connection = standby(&provider, online_id).await;
    assert!(
        connection.side().is_client(),
        "the hub itself reached the next identity"
    );
    let reply = provider.ping(online.address()).await.unwrap();
    assert_eq!(reply["reply"], "PONG");
    assert_eq!(reply["transport"]["connection"], connection.stable_id());
    assert_eq!(standby(&online, provider_id).await.remote_id(), provider_id);
    online.close().await.unwrap();
    provider.close().await.unwrap();
    relay.shutdown().await.unwrap();
}

#[tokio::test]
async fn maintained_application_startup_prepares_members_during_other_business() {
    let relay = relay().await;
    let root = tempfile::tempdir().unwrap();
    let (provider, consumer) = admitted(root.path(), &resources(&relay.http_url().unwrap())).await;
    consumer.close().await.unwrap();
    provider.close().await.unwrap();
    let consumer = NativeNetwork::open(root.path().join("consumer"))
        .await
        .unwrap()
        .maintain_application_network(Ipv4Addr::LOCALHOST, |builder| builder)
        .await
        .unwrap();
    let activity = consumer.business_activity();
    let provider = NativeNetwork::open(root.path().join("provider"))
        .await
        .unwrap()
        .maintain_application_network(Ipv4Addr::LOCALHOST, |builder| builder)
        .await
        .unwrap();
    let (a, b) = timeout(Duration::from_secs(10), async {
        tokio::join!(provider.accept_member(), consumer.accept_member())
    })
    .await
    .expect("startup prepares shared members despite a separate business lease");
    let a = a.unwrap().unwrap();
    let b = b.unwrap().unwrap();
    assert_eq!(
        member_fusion_attempt_id(&a).unwrap(),
        member_fusion_attempt_id(&b).unwrap()
    );
    assert_eq!(
        consumer
            .connect_member(provider.addr())
            .await
            .unwrap()
            .stable_id(),
        b.stable_id()
    );
    let (mut request, mut response) = tokio::join!(b.open_bi(), a.accept_bi());
    let mut request = request.unwrap();
    let mut response = response.unwrap();
    request.streams().0.write_all(b"PING").await.unwrap();
    request.streams().0.finish().unwrap();
    assert_eq!(response.streams().1.read_to_end(64).await.unwrap(), b"PING");
    response.streams().0.write_all(b"PONG").await.unwrap();
    response.streams().0.finish().unwrap();
    assert_eq!(request.streams().1.read_to_end(64).await.unwrap(), b"PONG");
    drop(activity);
    tokio::join!(consumer.close(), provider.close());
    relay.shutdown().await.unwrap();
}
