#![cfg(feature = "nat-lab-test-hooks")]
#[path = "support/native_connections.rs"]
mod native_connections;
use iroh::{
    Endpoint, EndpointAddr, EndpointId,
    endpoint::{BeforeConnectOutcome, EndpointHooks, presets},
};
use miopunch_native_probe::{
    admission_gate::ADMISSION_REQUEST_ALPN,
    manual_admission::ManualExchange,
    native_network::{FUSED_MEMBER_ALPN, NativeNetwork},
};
use std::{
    collections::BTreeSet,
    net::Ipv4Addr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::Notify,
    task::JoinSet,
    time::{sleep, timeout},
};
async fn join(admin: &mut NativeNetwork, path: &std::path::Path) -> NativeNetwork {
    let mut node = NativeNetwork::join(path, admin.manual_invitation().unwrap())
        .await
        .unwrap();
    let ae = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let ne = node.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = native_connections::control_pair(&ne, &ae, ADMISSION_REQUEST_ALPN).await;
    let (request, served) = tokio::join!(
        node.request_manual_admission(&client),
        admin.serve_manual_admission(&server)
    );
    assert!(matches!(request.unwrap(), ManualExchange::AwaitingApproval));
    served.unwrap();
    let request = admin.manual_requests().await.unwrap().pop().unwrap();
    admin.approve_manual_admission(&request).await.unwrap();
    let receive = async {
        let ManualExchange::Materials(material) =
            node.request_manual_admission(&client).await.unwrap()
        else {
            panic!("approval")
        };
        node.complete_manual_admission(material).await.unwrap();
    };
    let (_, served) = tokio::join!(receive, admin.serve_manual_admission(&server));
    served.unwrap();
    ne.close().await;
    ae.close().await;
    node
}

#[derive(Clone, Debug, Default)]
struct HoldDials {
    bypass: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
    started: Arc<Mutex<BTreeSet<EndpointId>>>,
    release: Arc<Notify>,
}
impl EndpointHooks for HoldDials {
    async fn before_connect(&self, target: &EndpointAddr, alpn: &[u8]) -> BeforeConnectOutcome {
        if alpn == FUSED_MEMBER_ALPN && !self.bypass.load(Ordering::Relaxed) {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.started.lock().unwrap().insert(target.id);
            self.release.notified().await;
        }
        BeforeConnectOutcome::Accept
    }
}
#[tokio::test]
async fn ninth_member_waits_for_an_actual_building_position() {
    nine_members(false).await;
}

#[tokio::test]
async fn cancellation_releases_a_position_before_the_original_deadline() {
    nine_members(true).await;
}

async fn nine_members(cancel_one: bool) {
    timeout(Duration::from_secs(45), async {
        let root = tempfile::tempdir().unwrap();
        let mut hub = NativeNetwork::create(root.path().join("hub"))
            .await
            .unwrap();
        let mut nodes = Vec::new();
        for index in 0..9 {
            nodes.push(join(&mut hub, &root.path().join(format!("peer-{index}"))).await);
        }
        for node in std::iter::once(&hub).chain(nodes.iter()) {
            let mut config = node.resource_configuration().unwrap();
            config.replacement = Some(Vec::new());
            config.tcp_direct_disabled = true;
            config.maintenance.standby_member_limit = None;
            node.save_resource_configuration(&config).unwrap();
        }
        let hooks = HoldDials::default();
        let client = hub
            .bind_member_fusion_endpoint_with(
                Endpoint::builder(presets::Minimal).hooks(hooks.clone()),
                Duration::from_secs(30),
            )
            .await
            .unwrap();
        let mut peers = Vec::new();
        let mut accepts = JoinSet::new();
        let mut accept_handles = Vec::new();
        for node in &nodes {
            let peer = node
                .bind_member_fusion_endpoint(Duration::from_secs(30))
                .await
                .unwrap();
            let acceptor = peer.clone();
            let accept_handle = accepts.spawn(async move {
                let connection = acceptor.accept().await.unwrap().unwrap();
                let mut stream = connection.accept_bi().await.unwrap();
                let (send, recv) = stream.streams();
                let mut bytes = [0; 8];
                recv.read_exact(&mut bytes).await.unwrap();
                send.write_all(&bytes).await.unwrap();
                send.finish().unwrap();
                send.stopped().await.unwrap();
            });
            accept_handles.push((peer.id(), accept_handle));
            peers.push(peer);
        }
        let mut dials = JoinSet::new();
        let mut dial_handles = Vec::new();
        for peer in &peers {
            let client = client.clone();
            let target = peer.addr();
            let peer_id = target.id;
            let handle = dials.spawn(async move {
                let connection = client.connect(target).await.unwrap();
                let mut stream = connection.open_bi().await.unwrap();
                let (send, recv) = stream.streams();
                send.write_all(b"member-8").await.unwrap();
                send.finish().unwrap();
                assert_eq!(recv.read_to_end(8).await.unwrap(), b"member-8");
                connection
            });
            dial_handles.push((peer_id, handle));
        }
        timeout(Duration::from_secs(3), async {
            while hooks.started.lock().unwrap().len() < 8 {
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        sleep(Duration::from_millis(150)).await;
        assert_eq!(
            hooks.started.lock().unwrap().len(),
            8,
            "ninth actual member dial must queue"
        );
        let repeated_peer = *hooks.started.lock().unwrap().iter().next().unwrap();
        let duplicate_client = client.clone();
        let duplicate = tokio::spawn(async move {
            duplicate_client
                .connect(EndpointAddr::new(repeated_peer))
                .await
                .unwrap()
        });
        sleep(Duration::from_millis(50)).await;
        assert_eq!(
            hooks.calls.load(Ordering::Relaxed),
            8,
            "same peer joins its in-flight work"
        );
        if cancel_one {
            let cancelled = *hooks
                .started
                .lock()
                .unwrap()
                .iter()
                .find(|id| **id != repeated_peer)
                .unwrap();
            dial_handles
                .iter()
                .find(|(id, _)| *id == cancelled)
                .unwrap()
                .1
                .abort();
            accept_handles
                .iter()
                .find(|(id, _)| *id == cancelled)
                .unwrap()
                .1
                .abort();
        } else {
            hooks.release.notify_one();
        }
        timeout(Duration::from_secs(5), async {
            while hooks.started.lock().unwrap().len() < 9 {
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        hooks.release.notify_waiters();
        let mut connections = Vec::new();
        while let Some(done) = dials.join_next().await {
            match done {
                Ok(connection) => connections.push(connection),
                Err(error) if cancel_one && error.is_cancelled() => {}
                Err(error) => panic!("member dial task failed: {error}"),
            }
        }
        while let Some(done) = accepts.join_next().await {
            if let Err(error) = done {
                assert!(cancel_one && error.is_cancelled());
            }
        }
        assert_eq!(connections.len(), if cancel_one { 8 } else { 9 });
        let repeated = duplicate.await.unwrap();
        assert_eq!(
            repeated.stable_id(),
            connections
                .iter()
                .find(|c| c.remote_id() == repeated_peer)
                .unwrap()
                .stable_id()
        );
        for connection in &connections {
            let reused = client
                .connect(EndpointAddr::new(connection.remote_id()))
                .await
                .unwrap();
            assert_eq!(reused.stable_id(), connection.stable_id());
        }
        eprintln!(
            "MEMBER_BUILDING_DATA peers={} actual_dials={} duplicate_reused=true cancelled={cancel_one}",
            connections.len(),
            hooks.calls.load(Ordering::Relaxed)
        );
        client.close().await;
        for peer in peers {
            peer.close().await;
        }
        for node in nodes {
            node.close().await;
        }
        hub.close().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn relay_improvement_shares_positions_with_first_contacts() {
    use iroh::TransportAddr;
    use miopunch_native_probe::{
        resources::{Resource, ResourceUse},
        tcp_transport::TcpTransport,
    };
    timeout(Duration::from_secs(50),async {
        let root = tempfile::tempdir().unwrap();
        let mut hub = NativeNetwork::create(root.path().join("hub"))
            .await
            .unwrap();
        let mut nodes = Vec::new();
        for index in 0..9 {
            nodes.push(join(&mut hub, &root.path().join(format!("peer-{index}"))).await);
        }
        let mut relay_config = iroh_relay::server::ServerConfig::default();
        relay_config.relay = Some(iroh_relay::server::RelayConfig::new((
            Ipv4Addr::LOCALHOST,
            0,
        )));
        let relay = iroh_relay::server::Server::spawn(relay_config)
            .await
            .unwrap();
        let relay_url = relay.http_url().unwrap();
        for node in std::iter::once(&hub).chain(nodes.iter()) {
            let mut config = node.resource_configuration().unwrap();
            config.replacement = Some(vec![Resource {
                url: relay_url.to_string(),
                uses: [ResourceUse::DataRelay].into(),
                limitation: "owned shared-position relay".into(),
            }]);
            config.udp_direct_disabled = true;
            config.tcp_direct_disabled = false;
            config.tcp_listen = Some((Ipv4Addr::LOCALHOST, 0).into());
            node.save_resource_configuration(&config).unwrap();
        }
        let hooks = HoldDials::default();
        hooks.bypass.store(true, Ordering::Relaxed);
        let client = hub
            .bind_member_fusion_endpoint_with(
                Endpoint::builder(presets::Minimal)
                    .hooks(hooks.clone())
                    .test_suppress_member_direct_announcements(),
                Duration::from_secs(30),
            )
            .await
            .unwrap();
        let target_transport = TcpTransport::new(Ipv4Addr::LOCALHOST, Arc::default());
        let mut peers = Vec::new();
        for (index, node) in nodes.iter().enumerate() {
            let builder=Endpoint::builder(presets::Minimal).test_suppress_member_direct_announcements();
            let builder=if index == 0 { builder.add_custom_transport(Arc::new(target_transport.clone())) } else { builder };
            peers.push(
                node.bind_member_fusion_endpoint_with(
                    builder,
                    Duration::from_secs(30),
                )
                .await
                .unwrap(),
            );
        }
        let (out, incoming) = tokio::join!(
            client.connect(EndpointAddr::new(peers[0].id()).with_relay_url(relay_url.clone())),
            peers[0].accept()
        );
        let original = out.unwrap();
        let remote = incoming.unwrap().unwrap();
        let original_id = original.stable_id();
        assert!(
            original
                .paths()
                .iter()
                .any(|path| path.is_selected() && path.is_relay())
        );
        // One healthy idle session leaves all eight construction positions available.
        hooks.bypass.store(false, Ordering::Relaxed);
        let mut dials = JoinSet::new();
        let mut accepts = JoinSet::new();
        for peer in peers.iter().skip(1) {
            let dialer = client.clone();
            let acceptor = peer.clone();
            let target = EndpointAddr::new(peer.id()).with_relay_url(relay_url.clone());
            dials.spawn(async move { dialer.connect(target).await.unwrap() });
            accepts.spawn(async move { acceptor.accept().await.unwrap().unwrap() });
        }
        timeout(Duration::from_secs(3), async {
            while hooks.calls.load(Ordering::Relaxed) < 8 {
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let mut outgoing = original.open_bi().await.unwrap();
        outgoing.streams().0.write_all(b"before-8").await.unwrap();
        let mut incoming = remote.accept_bi().await.unwrap();
        let mut bytes = [0; 8];
        incoming.streams().1.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"before-8");
        incoming.streams().0.write_all(b"before-8").await.unwrap();
        outgoing.streams().1.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"before-8");
        let tcp = TransportAddr::Custom(TcpTransport::address(target_transport.bound_socket_addr().unwrap()));
        client
            .learn_path_candidates(EndpointAddr::new(peers[0].id()).with_addrs([tcp.clone()]))
            .await;
        sleep(Duration::from_millis(350)).await;
        assert!(
            !original
                .paths()
                .iter()
                .any(|path| matches!(path.remote_addr(), TransportAddr::Custom(_))),
            "improvement must wait behind eight different building peers"
        );
        hooks.release.notify_one();
        timeout(Duration::from_secs(5), async {
            loop {
                if original.paths().iter().any(|path| {
                    path.is_selected()
                        && path.remote_addr() == &tcp
                        && path.stats().acknowledgment.is_some()
                }) {
                    break;
                }
                sleep(Duration::from_millis(15)).await;
            }
        })
        .await
        .unwrap();
        outgoing.streams().0.write_all(b"after--8").await.unwrap();
        incoming.streams().1.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"after--8");
        incoming.streams().0.write_all(b"after--8").await.unwrap();
        outgoing.streams().1.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"after--8");
        assert_eq!(original.stable_id(), original_id);
        hooks.release.notify_waiters();
        let mut live = Vec::new();
        while let Some(done) = dials.join_next().await {
            live.push(done.unwrap());
        }
        while let Some(done) = accepts.join_next().await {
            live.push(done.unwrap());
        }
        assert_eq!(hooks.calls.load(Ordering::Relaxed), 8);
        eprintln!(
            "MEMBER_BUILDING_MIXED held_first_contacts=8 healthy_relay_occupies=0 tcp_waited=true original={original_id}"
        );
        drop((outgoing, incoming, live));
        client.close().await;
        for peer in peers {
            peer.close().await;
        }
        for node in nodes {
            node.close().await;
        }
        hub.close().await;
        relay.shutdown().await.unwrap();
    }).await.unwrap();
}

#[tokio::test]
async fn ordinary_qnt_keeps_its_position_until_the_path_acknowledges() {
    use miopunch_native_probe::{
        path_selection::PathPreference,
        resources::{Resource, ResourceUse},
    };
    let root = tempfile::tempdir().unwrap();
    let mut hub = NativeNetwork::create(root.path().join("hub"))
        .await
        .unwrap();
    let peer = join(&mut hub, &root.path().join("peer")).await;
    let mut relay_config = iroh_relay::server::ServerConfig::default();
    relay_config.relay = Some(iroh_relay::server::RelayConfig::new((
        Ipv4Addr::LOCALHOST,
        0,
    )));
    let relay = iroh_relay::server::Server::spawn(relay_config)
        .await
        .unwrap();
    let url = relay.http_url().unwrap();
    for node in [&hub, &peer] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(vec![Resource {
            url: url.to_string(),
            uses: [ResourceUse::DataRelay].into(),
            limitation: "owned QNT lease handoff".into(),
        }]);
        config.tcp_direct_disabled = true;
        config.path_selection.preference = PathPreference::PreferDirect;
        node.save_resource_configuration(&config).unwrap();
    }
    let builder = || {
        Endpoint::builder(presets::Minimal)
            .clear_ip_transports()
            .bind_addr((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
    };
    let left = hub
        .bind_member_fusion_endpoint_with(builder(), Duration::from_secs(15))
        .await
        .unwrap();
    let right = peer
        .bind_member_fusion_endpoint_with(builder(), Duration::from_secs(15))
        .await
        .unwrap();
    let (out, incoming) = tokio::join!(
        left.connect(EndpointAddr::new(right.id()).with_relay_url(url)),
        right.accept()
    );
    let original = out.unwrap();
    let remote = incoming.unwrap().unwrap();
    let ids = [original.stable_id(), remote.stable_id()];
    let suppressed = Arc::new(AtomicBool::new(true));
    for connection in [&original, &remote] {
        connection.test_suppress_path_acks(noq_proto::PathId::from(1u8), suppressed.clone());
    }
    let mut from = original.open_bi().await.unwrap();
    from.streams().0.write_all(b"relay---").await.unwrap();
    let mut to = remote.accept_bi().await.unwrap();
    let mut bytes = [0; 8];
    to.streams().1.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"relay---");
    let left_ip = *left.addr().ip_addrs().find(|addr| addr.is_ipv4()).unwrap();
    let right_ip = *right.addr().ip_addrs().find(|addr| addr.is_ipv4()).unwrap();
    left.learn_ip_candidates(EndpointAddr::new(right.id()).with_ip_addr(right_ip))
        .await;
    right
        .learn_ip_candidates(EndpointAddr::new(left.id()).with_ip_addr(left_ip))
        .await;
    timeout(Duration::from_secs(5), async {
        while ![&original, &remote].iter().all(|connection| {
            connection.paths().iter().any(|path| {
                path.id() == noq_proto::PathId::from(1u8) && path.stats().frame_rx.path_response > 0
            })
        }) {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    sleep(Duration::from_millis(1100)).await;
    assert!(
        left.test_building_peers().contains(&right.id()),
        "client keeps its QNT position while the return-path ACK is pending"
    );
    assert!(
        right.test_building_peers().contains(&left.id()),
        "server keeps its QNT position while the return-path ACK is pending"
    );
    suppressed.store(false, Ordering::Relaxed);
    timeout(Duration::from_secs(5), async {
        loop {
            if [&original, &remote].iter().all(|connection| {
                connection.paths().iter().any(|path| {
                    path.is_ip() && path.is_selected() && path.stats().acknowledgment.is_some()
                })
            }) {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    from.streams().0.write_all(b"direct--").await.unwrap();
    to.streams().1.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"direct--");
    to.streams().0.write_all(&bytes).await.unwrap();
    from.streams().1.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"direct--");
    timeout(Duration::from_secs(3), async {
        while !left.test_building_peers().is_empty() || !right.test_building_peers().is_empty() {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!([original.stable_id(), remote.stable_id()], ids);
    eprintln!(
        "MEMBER_BUILDING_QNT pending_ack_owned=true qualified_released=true originals={ids:?}"
    );
    drop((from, to));
    left.close().await;
    right.close().await;
    peer.close().await;
    hub.close().await;
    relay.shutdown().await.unwrap();
}
