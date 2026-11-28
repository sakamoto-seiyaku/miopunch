//! Real QAD/TLS/control/UDP seams for the legacy selector, not NAT-rate evidence.
#[path = "support/control_handoff.rs"]
mod handoff;
use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use iroh::{
    Endpoint, EndpointAddr, RelayConfig, RelayMap, Watcher as _,
    endpoint::{UdpObservation, UdpObservationFailure, presets},
};
use iroh_base::{SecretKey, TransportAddr};
use iroh_relay::{RelayQuicConfig, server, tls::CaTlsConfig};
use miopunch_native_probe::{
    admission_gate::{AdmissionGate, CONTROL_ALPN},
    admission_proof::{AdmissionScope, AdmissionSecret},
    control_session::ControlSession,
    nat_mapping::NodeProbeBudget,
    nat_punch::{self, ExecutionStatus, MappingSnapshot},
    nat_selector::{Analyzer, BehaviorId, NatType, Role},
    native_network::NativeNetwork,
};

#[tokio::test]
async fn direct_upgrade_rejects_invalid_final_result_without_punching() {
    use miopunch_native_probe::control_session::CommitError;
    let (_directories, visitor, pair) = native_upgrade_pair(&[], false).await;
    let wrong_nonce = [1u8; 16];
    let nonce = [0u8; 16];
    for reply in [
        serde_json::json!({"Result":{"nonce":wrong_nonce,"available":false}}),
        serde_json::json!({"Ready":{"nonce":nonce}}),
    ] {
        let (peer, mut client) = pair.connect().await;
        let stable_id = client.stable_id();
        client.queue_business(b"pending business".to_vec()).unwrap();
        let (result, ()) = tokio::join!(
            visitor.upgrade_udp(client),
            scripted_direct_reply(&peer, reply)
        );
        let Err(CommitError::Retryable { error, session }) = result else {
            panic!("expected a rejected final Result on the original session");
        };
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData, "{error}");
        assert!(error.to_string().starts_with("unified/InvalidControl"));
        assert_eq!(session.stable_id(), stable_id);
    }
    pair.close().await;
}

async fn native_upgrade_pair(
    advertised: &[SocketAddr],
    relay: bool,
) -> (tempfile::TempDir, NativeNetwork, Pair) {
    let directories = tempfile::tempdir().unwrap();
    let path = directories.path().join("visitor");
    let visitor = NativeNetwork::create(&path).await.unwrap();
    let key = std::fs::read(path.join("node.key")).unwrap();
    let key = SecretKey::from_bytes(&key.try_into().unwrap());
    let pair = Pair::build(
        Ipv4Addr::LOCALHOST.into(),
        false,
        Some((key, visitor.admission_gate())),
        advertised,
        relay,
        &[],
    )
    .await;
    (directories, visitor, pair)
}

async fn scripted_direct_reply(peer: &ControlSession, reply: serde_json::Value) {
    let nonce = [0u8; 16];
    let exchanges = [
        (
            "Hello",
            serde_json::json!({"Offer":{"nonce":nonce,"addresses":[]}}),
        ),
        ("Offer", serde_json::json!({"Ready":{"nonce":nonce}})),
        ("Result", reply),
    ];
    for (expected, reply) in exchanges {
        let (exchange, _) = direct_frame(peer, expected).await;
        send_direct_frame(&exchange, reply).await;
    }
}

async fn direct_frame(
    peer: &ControlSession,
    kind: &str,
) -> (iroh::endpoint::ControlExchange, serde_json::Value) {
    let exchange = peer.accept_control_exchange().await.unwrap();
    let input = exchange.recv_message(DIRECT_FORMAT).await.unwrap();
    let frame: serde_json::Value = serde_json::from_slice(&input).unwrap();
    assert!(frame.get(kind).is_some(), "{frame}");
    (exchange, frame[kind].clone())
}

const DIRECT_FORMAT: iroh::endpoint::ControlMessageFormat =
    iroh::endpoint::ControlMessageFormat::EndTerminated { maximum: 16 * 1024 };

async fn send_direct_frame(exchange: &iroh::endpoint::ControlExchange, frame: serde_json::Value) {
    exchange
        .send_message(DIRECT_FORMAT, &serde_json::to_vec(&frame).unwrap())
        .await
        .unwrap();
}

async fn receive_direct_candidate_result(
    peer: &ControlSession,
    nonce: [u8; 16],
    addresses: &[SocketAddr],
) -> iroh::endpoint::ControlExchange {
    let (hello, _) = direct_frame(peer, "Hello").await;
    send_direct_frame(
        &hello,
        serde_json::json!({"Offer":{"nonce":nonce,"addresses":addresses}}),
    )
    .await;
    let (offer, _) = direct_frame(peer, "Offer").await;
    send_direct_frame(&offer, serde_json::json!({"Ready":{"nonce":nonce}})).await;
    let (result, received) = direct_frame(peer, "Result").await;
    assert_eq!(received["available"], false);
    result
}

#[tokio::test]
async fn public_udp_upgrade_deadline_bounds_stalled_hello_and_result() {
    use miopunch_native_probe::control_session::CommitError;
    let (_directories, visitor, pair) = native_upgrade_pair(&[], false).await;
    let nonce = [0u8; 16];
    for stalled in ["Hello", "Result"] {
        let deadline = n0_future::time::Instant::now() + Duration::from_millis(700);
        let (peer, mut client) = pair.connect_until(Some(deadline)).await;
        let original = client.stable_id();
        client.queue_business(b"pending business".to_vec()).unwrap();
        let peer_work = async {
            let (hello, _) = direct_frame(&peer, "Hello").await;
            if stalled == "Result" {
                send_direct_frame(
                    &hello,
                    serde_json::json!({"Offer":{"nonce":nonce,"addresses":[]}}),
                )
                .await;
                let (offer, _) = direct_frame(&peer, "Offer").await;
                send_direct_frame(&offer, serde_json::json!({"Ready":{"nonce":nonce}})).await;
                let (_result, _) = direct_frame(&peer, "Result").await;
                return tokio::time::sleep_until((deadline + Duration::from_millis(200)).into())
                    .await;
            }
            tokio::time::sleep_until((deadline + Duration::from_millis(200)).into()).await;
        };
        let bound = deadline + Duration::from_millis(100);
        let (result, ()) = tokio::join!(
            tokio::time::timeout_at(bound.into(), visitor.upgrade_udp(client)),
            peer_work
        );
        let error = match result.expect("upgrade_udp exceeded its absolute deadline") {
            Err(CommitError::Retryable { error, session }) => {
                assert_eq!(session.stable_id(), original);
                error
            }
            Err(CommitError::Terminal { error }) => error,
            Ok(_) => panic!("a stalled peer did not finish the direct exchange"),
        };
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::TimedOut,
            "{stalled}: {error}"
        );
    }
    pair.close().await;
}

#[tokio::test]
async fn public_udp_upgrade_advertises_all_address_classes_within_the_wire_limit() {
    let mut advertised = (1..=90)
        .map(|host| SocketAddr::from(([10, 2, 0, host], 2200)))
        .collect::<Vec<_>>();
    let v6: SocketAddr = "[2001:db8::2]:2200".parse().unwrap();
    let public: SocketAddr = "198.51.100.4:2200".parse().unwrap();
    let invalid: SocketAddr = "224.0.0.50:2200".parse().unwrap();
    advertised.extend([v6, public, public, invalid]);
    let (_directories, visitor, pair) = native_upgrade_pair(&advertised, false).await;
    let deadline = n0_future::time::Instant::now() + Duration::from_secs(3);
    let (peer, mut client) = pair.connect_until(Some(deadline)).await;
    let nonce = [0u8; 16];
    let mut advertised_across_rounds = std::collections::BTreeSet::new();
    for _ in 0..2 {
        let inspect_offer = async {
            let (hello, _) = direct_frame(&peer, "Hello").await;
            send_direct_frame(
                &hello,
                serde_json::json!({"Offer":{"nonce":nonce,"addresses":[]}}),
            )
            .await;
            let (offer, frame) = direct_frame(&peer, "Offer").await;
            let addresses: Vec<SocketAddr> =
                serde_json::from_value(frame["addresses"].clone()).unwrap();
            send_direct_frame(
                &offer,
                serde_json::json!({"Result":{"nonce":nonce,"available":false}}),
            )
            .await;
            addresses
        };
        let (result, addresses) = tokio::join!(visitor.upgrade_udp(client), inspect_offer);
        assert_eq!(addresses.len(), 64);
        assert!(
            addresses.contains(&v6) && addresses.contains(&public),
            "{addresses:?}"
        );
        assert!(!addresses.contains(&invalid));
        assert_eq!(
            addresses
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            64
        );
        advertised_across_rounds.extend(addresses);
        let Err(miopunch_native_probe::control_session::CommitError::Retryable { session, .. }) =
            result
        else {
            panic!("scripted peer rejection must retain the original");
        };
        client = session;
    }
    assert!(
        advertised
            .iter()
            .filter(|address| **address != invalid)
            .all(|address| advertised_across_rounds.contains(address))
    );
    pair.close().await;
}

#[tokio::test]
async fn ordinary_candidate_probes_leave_a_real_slot_for_the_udp_winner() {
    use miopunch_native_probe::nat_connection::{ControlUdpTraversal, UdpPolicy};
    let (_directories, visitor, pair) = native_upgrade_pair(&[], true).await;
    let (peer, mut client) = pair.connect().await;
    assert!(
        client
            .control_handle()
            .paths()
            .iter()
            .any(|path| path.is_relay())
    );
    assert!(!client.has_data_path());
    let original = client.control_handle();
    let real_path = pair.c.bound_sockets()[0];
    let nonce = [0u8; 16];
    let mut dead_sockets = Vec::new();
    for subnet in [2, 3] {
        for host in 2..8 {
            let address = SocketAddr::from(([127, subnet, 0, host], 0));
            dead_sockets.push(tokio::net::UdpSocket::bind(address).await.unwrap());
        }
    }
    let addresses = dead_sockets
        .iter()
        .map(|socket| socket.local_addr().unwrap())
        .collect::<Vec<_>>();
    for round in 1..=2 {
        let scripted = async {
            let result = receive_direct_candidate_result(&peer, nonce, &addresses).await;
            let winner_opened = if round == 2 {
                original.open_data_path(real_path).await
            } else {
                Ok(())
            };
            send_direct_frame(&result, serde_json::json!({"Ready":{"nonce":nonce}})).await;
            winner_opened
        };
        let (result, winner_opened) = tokio::join!(visitor.upgrade_udp(client), scripted);
        assert!(
            winner_opened.is_ok(),
            "opening the actual winner was blocked: {winner_opened:?}"
        );
        let Err(miopunch_native_probe::control_session::CommitError::Retryable { session, .. }) =
            result
        else {
            panic!("scripted peer rejection must retain the original");
        };
        assert_eq!(session.stable_id(), original.stable_id());
        client = session;
    }
    let probed = dead_sockets
        .iter()
        .filter(|socket| socket.try_recv(&mut [0; 1500]).is_ok())
        .count();
    assert!(
        probed > 8,
        "only reached {probed} candidates across two rounds"
    );
    assert!(original.paths().iter().any(|path| path.is_relay()));
    tokio::time::timeout(Duration::from_secs(2), async {
        while !client.has_data_path() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the winner must establish on the original");
    // Retire the slot witness. A legitimate negative Result on the next
    // pressured round must use the real protected punching and commit APIs.
    original
        .paths()
        .iter()
        .find(|path| path.is_ip())
        .unwrap()
        .close()
        .unwrap();
    let ids = (original.stable_id(), peer.stable_id());
    let peer_connector = ControlUdpTraversal::new(None, UdpPolicy::default());
    let mut client = client;
    let mut peer = peer;
    client.queue_business(b"queued client".to_vec()).unwrap();
    peer.queue_business(b"queued peer".to_vec()).unwrap();
    let peer_upgrade = async {
        let result = receive_direct_candidate_result(&peer, nonce, &addresses).await;
        send_direct_frame(
            &result,
            serde_json::json!({"Result":{"nonce":nonce,"available":false}}),
        )
        .await;
        let mut attempt = peer_connector.prepare(&peer, &pair.c_gate).await.unwrap();
        peer_connector
            .execute(&mut attempt, &peer, &pair.c_gate)
            .await
            .unwrap();
        let data = attempt
            .commit_winner_path(peer, &pair.c_gate)
            .await
            .unwrap();
        (attempt, data)
    };
    let (visitor_upgrade, (mut peer_attempt, mut peer_data)) =
        tokio::join!(visitor.upgrade_udp(client), peer_upgrade);
    let mut visitor_upgrade = visitor_upgrade.unwrap();
    assert!(visitor_upgrade.attempt().is_some());
    assert_eq!(
        visitor_upgrade
            .data
            .take_pending_business()
            .pop_front()
            .unwrap(),
        b"queued client"
    );
    assert_eq!(
        peer_data.take_pending_business().pop_front().unwrap(),
        b"queued peer"
    );
    assert_eq!(
        (
            visitor_upgrade.data.connection().stable_id(),
            peer_data.connection().stable_id()
        ),
        ids
    );
    for connection in [visitor_upgrade.data.connection(), peer_data.connection()] {
        assert!(connection.paths().iter().any(|path| path.is_relay()));
        assert!(
            connection
                .paths()
                .iter()
                .any(|path| path.is_ip() && path.is_selected())
        );
    }
    let (client_bytes, peer_bytes) = tokio::join!(
        visitor_upgrade.exchange_business(
            b"visitor payload"
        ),
        peer_connector.exchange_business(
            &mut peer_attempt,
            &peer_data,
            &pair.c_gate,
            b"peer payload"
        )
    );
    assert_eq!(client_bytes.unwrap(), b"peer payload");
    assert_eq!(peer_bytes.unwrap(), b"visitor payload");
    pair.close().await;
}

#[tokio::test]
async fn formal_entry_observes_selects_executes_and_confirms_original_business() {
    use miopunch_native_probe::{
        nat_connection::{ControlUdpTraversal, UdpPolicy},
        resources::{Resource, ResourceUse},
    };
    let pair = Pair::new().await;
    let (c, v) = pair.connect().await;
    let ids = (c.stable_id(), v.stable_id());
    let blackhole = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let blocked = RelayConfig::new(
        "https://127.0.0.1:1/".parse().unwrap(),
        Some(RelayQuicConfig::new(blackhole.local_addr().unwrap().port())),
    );
    let visitor_qad = RelayMap::from_iter(
        pair.observers
            .relays::<Vec<_>>()
            .into_iter()
            .take(1)
            .chain([Arc::new(blocked)]),
    );
    let stun = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("visitor");
    let node = NativeNetwork::create(&path).await.unwrap();
    let mut resources = node.resource_configuration().unwrap();
    resources.replacement = Some(vec![Resource {
        url: format!("stun://{}/", stun.local_addr().unwrap()),
        uses: [ResourceUse::Observation].into(),
        limitation: "owned mapping observer".into(),
    }]);
    node.save_resource_configuration(&resources).unwrap();
    node.close().await;
    let visitor =
        NativeNetwork::open_with_udp_resources(&path, Some(visitor_qad), UdpPolicy::default())
            .await
            .unwrap();
    let stun = tokio::spawn(async move {
        for _ in 0..2 {
            reply_to_stun(&stun, None).await;
        }
    });
    let cn = ControlUdpTraversal::new(Some(pair.observers.clone()), UdpPolicy::default());
    let vn = visitor.control_udp();
    let (ca, va) = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(cn.prepare(&c, &pair.c_gate), vn.prepare(&v, &pair.v_gate))
    })
    .await
    .expect("QAD and STUN must complete the protected plan before the blocked QAD expires");
    assert!(
        blackhole.try_recv(&mut [0; 1500]).is_ok(),
        "the blocked observer was attempted"
    );
    let (mut ca, mut va) = (ca.unwrap(), va.unwrap());
    assert_eq!(ca.mapped_candidates(), &[pair.v.bound_sockets()[0]; 2]);
    assert_eq!(va.mapped_candidates(), &[pair.c.bound_sockets()[0]; 2]);
    assert_eq!(ca.plan().id, BehaviorId { mode: 0, index: 2 });
    assert_eq!(ca.assisted_candidates(), pair.v.bound_sockets());
    assert_eq!(va.assisted_candidates(), pair.c.bound_sockets());
    assert_eq!(ca.plan(), va.plan());
    let (cp, vp) = tokio::join!(
        cn.execute(&mut ca, &c, &pair.c_gate),
        vn.execute(&mut va, &v, &pair.v_gate)
    );
    cp.unwrap();
    vp.unwrap();
    let (cd, vd) = tokio::join!(
        ca.commit_winner_path(c, &pair.c_gate),
        va.commit_winner_path(v, &pair.v_gate)
    );
    let (cd, vd) = (cd.unwrap(), vd.unwrap());
    assert_eq!(
        (cd.connection().stable_id(), vd.connection().stable_id()),
        ids
    );
    let (cr, vr) = tokio::join!(
        cn.exchange_business(&mut ca, &cd, &pair.c_gate, b"accepting-member"),
        vn.exchange_business(&mut va, &vd, &pair.v_gate, b"visiting-member")
    );
    assert_eq!(cr.unwrap(), b"visiting-member");
    assert_eq!(vr.unwrap(), b"accepting-member");
    pair.change_network_generation().await;
    let (c2, v2) = pair.connect().await;
    let (c_next, v_next) =
        tokio::join!(cn.prepare(&c2, &pair.c_gate), vn.prepare(&v2, &pair.v_gate));
    let c_next = c_next.unwrap();
    assert_eq!(c_next.plan(), v_next.unwrap().plan());
    assert_eq!(c_next.plan().id, BehaviorId { mode: 0, index: 2 });
    assert_eq!(
        c_next.plan().scores_before[2].score,
        2,
        "one recommendation-1 and actual business+2 retained across connections"
    );
    stun.await.unwrap();
    pair.close().await;
    visitor.close().await;
}

#[tokio::test]
async fn single_observation_builds_a_basic_plan_without_nat_classification() {
    use miopunch_native_probe::nat_connection::{ControlUdpTraversal, UdpPolicy};

    let pair = Pair::new().await;
    let (c, v) = pair.connect().await;
    let observer = pair.observers.relays::<Vec<_>>().pop().unwrap();
    let cn = ControlUdpTraversal::new(Some(RelayMap::from_iter([observer])), UdpPolicy::default());
    let vn = ControlUdpTraversal::new(Some(pair.observers.clone()), UdpPolicy::default());
    let (ca, va) = tokio::join!(cn.prepare(&c, &pair.c_gate), vn.prepare(&v, &pair.v_gate));
    let (ca, va) = (ca.unwrap(), va.unwrap());
    assert_eq!(va.mapped_candidates(), pair.c.bound_sockets());
    assert!(va.remote_feature().is_none());
    assert!(va.plan().fallback);
    assert_eq!(ca.plan(), va.plan());
    pair.close().await;
}

#[tokio::test]
async fn formal_entry_replenishes_a_complete_round_in_a_new_network_generation() {
    use miopunch_native_probe::nat_connection::{ControlUdpTraversal, UdpPolicy};

    let pair = Pair::new().await;
    let policy = UdpPolicy {
        additional_observations: 2,
        ..Default::default()
    };
    let cn = ControlUdpTraversal::new(Some(pair.observers.clone()), policy.clone());
    let vn = ControlUdpTraversal::new(Some(pair.observers.clone()), policy);
    for round in 0..2 {
        let (c, v) = pair.connect().await;
        let (ca, va) = tokio::join!(cn.prepare(&c, &pair.c_gate), vn.prepare(&v, &pair.v_gate));
        let (mut ca, mut va) = (ca.unwrap(), va.unwrap());
        let (cr, vr) = tokio::join!(
            cn.execute(&mut ca, &c, &pair.c_gate),
            vn.execute(&mut va, &v, &pair.v_gate)
        );
        cr.unwrap();
        vr.unwrap();
        assert!(ca.execution_trace().unwrap().probes + va.execution_trace().unwrap().probes > 0);
        if round == 0 {
            pair.change_network_generation().await;
        }
    }
    pair.close().await;
}

#[tokio::test]
async fn native_upgrade_reserves_qad_and_both_stun_observers_on_each_eligible_round() {
    use miopunch_native_probe::{
        control_session::CommitError,
        nat_connection::{ControlUdpTraversal, UdpPolicy},
        network_maintenance::MaintenancePolicy,
        resources::{Resource, ResourceUse},
    };
    use std::io::ErrorKind;
    use tokio::net::UdpSocket;

    let directories = tempfile::tempdir().unwrap();
    let path = directories.path().join("visitor");
    let initial = NativeNetwork::create(&path).await.unwrap();
    let mut config = initial.resource_configuration().unwrap();
    config.maintenance.enhanced_udp_cooldown_ms = 8_000;
    let mut stun_urls = Vec::new();
    let mut stun_sockets = Vec::new();
    let mut qad_origins = Vec::new();
    let mut blackholes = Vec::new();
    let stun_requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    for _ in 0..2 {
        let qad = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let endpoint = qad.local_addr().unwrap();
        qad_origins.push(RelayConfig::new(
            format!("https://{endpoint}/").parse().unwrap(),
            Some(RelayQuicConfig::new(endpoint.port())),
        ));
        blackholes.push(qad);
        let stun = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        stun_urls.push(format!("stun://{}/", stun.local_addr().unwrap()));
        stun_sockets.push(stun);
    }
    config.replacement = Some(
        stun_urls
            .iter()
            .map(|url| Resource {
                url: url.clone(),
                uses: [ResourceUse::Observation].into(),
                limitation: "owned test observer".into(),
            })
            .collect(),
    );
    initial.save_resource_configuration(&config).unwrap();
    initial.close().await;
    let allowed = qad_origins
        .iter()
        .map(|observer| observer.url.clone())
        .chain(config.udp_stun_resources())
        .collect::<Vec<_>>();
    assert_eq!(allowed.len(), 4);
    let denied_qad = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let address = denied_qad.local_addr().unwrap();
    qad_origins.push(RelayConfig::new(
        format!("https://{address}/").parse().unwrap(),
        Some(RelayQuicConfig::new(address.port())),
    ));
    let visitor = NativeNetwork::open_with_udp_resources(
        &path,
        Some(RelayMap::from_iter(qad_origins)),
        UdpPolicy {
            additional_observations: 4,
            max_rounds: 2,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let key = SecretKey::from_bytes(
        &std::fs::read(path.join("node.key"))
            .unwrap()
            .try_into()
            .unwrap(),
    );
    let pair = Pair::build(
        Ipv4Addr::LOCALHOST.into(),
        false,
        Some((key, visitor.admission_gate())),
        &[],
        true,
        &allowed,
    )
    .await;
    let peer_connector = ControlUdpTraversal::new(None, UdpPolicy::default())
        .with_maintenance_policy(&MaintenancePolicy {
            enhanced_udp_cooldown_ms: 8_000,
            ..Default::default()
        })
        .unwrap();
    let (revoke_peer, revoke_client) = pair.connect().await;
    let revoke = async {
        tokio::time::timeout(Duration::from_secs(2), async {
            let (mut q1, mut q2, mut s1, mut s2) = ([0; 1500], [0; 1500], [0; 1500], [0; 1500]);
            let (qad_a, qad_b, stun_a, stun_b) = tokio::join!(
                blackholes[0].recv_from(&mut q1),
                blackholes[1].recv_from(&mut q2),
                stun_sockets[0].recv_from(&mut s1),
                stun_sockets[1].recv_from(&mut s2),
            );
            qad_a.unwrap();
            qad_b.unwrap();
            assert_eq!(stun_a.unwrap().0, 20);
            assert_eq!(stun_b.unwrap().0, 20);
        })
        .await
        .unwrap();
        visitor.udp_connector().set_udp_enabled(false);
    };
    let traversal = visitor.control_udp();
    let (observed, ()) = tokio::join!(
        traversal
            .observe(&revoke_client, &pair.v_gate),
        revoke
    );
    assert_eq!(observed.unwrap_err().kind(), ErrorKind::PermissionDenied);
    visitor.udp_connector().set_udp_enabled(true);
    drop((revoke_peer, revoke_client));
    pair.change_network_generation().await;
    for socket in &blackholes {
        while socket.try_recv(&mut [0; 1500]).is_ok() {}
    }
    let stuns_in_flight = Arc::new(tokio::sync::Barrier::new(2));
    let stun_servers: Vec<_> = stun_sockets
        .into_iter()
        .map(|stun| {
            while stun.try_recv(&mut [0; 1500]).is_ok() {}
            let received = stun_requests.clone();
            let stuns_in_flight = stuns_in_flight.clone();
            tokio::spawn(async move {
                for _ in 0..2 {
                    reply_to_stun(&stun, Some(&stuns_in_flight)).await;
                    received.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            })
        })
        .collect();
    let (held_peer, _held_client) = pair.connect().await;
    let held = peer_connector
        .observe(&held_peer, &pair.c_gate)
        .await
        .unwrap();
    let (mut peer, mut client) = pair.connect().await;
    let ids = (client.stable_id(), peer.stable_id());
    let nonce = [0u8; 16];
    client
        .queue_business(b"client waiting across deferral".to_vec())
        .unwrap();
    peer.queue_business(b"peer waiting across deferral".to_vec())
        .unwrap();
    let (result, peer_result) = tokio::join!(visitor.upgrade_udp(client), async {
        scripted_direct_reply(
            &peer,
            serde_json::json!({"Result":{"nonce":nonce,"available":false}}),
        )
        .await;
        peer_connector.prepare(&peer, &pair.c_gate).await
    });
    assert_eq!(peer_result.unwrap_err().kind(), ErrorKind::WouldBlock);
    let Err(CommitError::Retryable { error, session }) = result else {
        panic!("peer budget must defer the original restricted upgrade");
    };
    assert_eq!(error.kind(), ErrorKind::WouldBlock);
    assert!(error.to_string().contains("peer deferred"));
    assert_eq!(session.stable_id(), ids.0);
    assert!(!session.has_data_path());
    drop(held);
    pair.change_network_generation().await;

    let mut next = (peer, session);
    for round in 0..2 {
        let (peer, client) = next;
        let started = tokio::time::Instant::now();
        let q = async {
            let (mut first, mut second) = ([0; 1500], [0; 1500]);
            tokio::select! {
                got = blackholes[0].recv_from(&mut first) => { got.unwrap(); 1 }
                got = blackholes[1].recv_from(&mut second) => { got.unwrap(); 0 }
            }
        };
        let work = async {
            tokio::join!(visitor.upgrade_udp(client), async {
                scripted_direct_reply(
                    &peer,
                    serde_json::json!({"Result":{"nonce":nonce,"available":false}}),
                )
                .await;
                let mut attempt = peer_connector.prepare(&peer, &pair.c_gate).await.unwrap();
                assert_eq!(attempt.mapped_candidates(), &[pair.v.bound_sockets()[0]; 2]);
                peer_connector
                    .execute(&mut attempt, &peer, &pair.c_gate)
                    .await
                    .unwrap();
                let data = attempt
                    .commit_winner_path(peer, &pair.c_gate)
                    .await
                    .unwrap();
                (attempt, data)
            })
        };
        let (other, (upgrade, (mut peer_attempt, mut peer_data))) =
            tokio::time::timeout(Duration::from_secs(6), async { tokio::join!(q, work) })
                .await
                .expect("both STUN results must progress while both QAD observers are blocked");
        let mut upgrade = upgrade.unwrap();
        assert!(
            upgrade
                .attempt()
                .unwrap()
                .execution_trace()
                .is_some()
        );
        let mut drain = [0; 1500];
        assert!(
            blackholes[other].try_recv(&mut drain).is_ok(),
            "both permitted QAD observers were contacted"
        );
        for blackhole in &blackholes {
            while blackhole.try_recv(&mut drain).is_ok() {}
        }
        if round == 0 {
            assert_eq!(upgrade.data.connection().stable_id(), ids.0);
            assert_eq!(peer_data.connection().stable_id(), ids.1);
            assert_eq!(
                upgrade.data.take_pending_business().pop_front().unwrap(),
                b"client waiting across deferral"
            );
            assert_eq!(
                peer_data.take_pending_business().pop_front().unwrap(),
                b"peer waiting across deferral"
            );
        }
        let (a, b) = tokio::join!(
            upgrade.exchange_business(
                b"visitor byte"
            ),
            peer_connector.exchange_business(
                &mut peer_attempt,
                &peer_data,
                &pair.c_gate,
                b"acceptor byte"
            ),
        );
        assert_eq!(a.unwrap(), b"acceptor byte");
        assert_eq!(b.unwrap(), b"visitor byte");
        if round == 1 {
            break;
        }
        next = pair.connect().await;
        assert_eq!(
            visitor
                .control_udp()
                .observe(&next.1, &pair.v_gate)
                .await
                .unwrap_err()
                .kind(),
            ErrorKind::WouldBlock
        );
        tokio::time::sleep_until(started + Duration::from_secs(9)).await;
    }
    for server in stun_servers {
        tokio::time::timeout(Duration::from_secs(1), server)
            .await
            .unwrap()
            .unwrap();
    }
    assert_eq!(stun_requests.load(std::sync::atomic::Ordering::Relaxed), 4);
    assert_eq!(
        denied_qad.try_recv(&mut [0; 1500]).unwrap_err().kind(),
        ErrorKind::WouldBlock
    );
    pair.close().await;
    visitor.close().await;
}

async fn reply_to_stun(socket: &tokio::net::UdpSocket, rendezvous: Option<&tokio::sync::Barrier>) {
    let mut request = [0; 1500];
    let (len, source) = socket.recv_from(&mut request).await.unwrap();
    assert_eq!(len, 20);
    assert_eq!(&request[..8], &[0, 1, 0, 0, 0x21, 0x12, 0xa4, 0x42]);
    if let Some(rendezvous) = rendezvous {
        rendezvous.wait().await;
    }
    let SocketAddr::V4(source) = source else {
        panic!("IPv4 observation required")
    };
    let mut reply = [0u8; 32];
    reply[..8].copy_from_slice(&[1, 1, 0, 12, 0x21, 0x12, 0xa4, 0x42]);
    reply[8..20].copy_from_slice(&request[8..20]);
    reply[20..26].copy_from_slice(&[0, 0x20, 0, 8, 0, 1]);
    reply[26..28].copy_from_slice(&(source.port() ^ 0x2112).to_be_bytes());
    reply[28..32].copy_from_slice(&(u32::from(*source.ip()) ^ 0x2112a442).to_be_bytes());
    socket.send_to(&reply, source).await.unwrap();
}

#[tokio::test]
async fn timed_out_enhanced_plan_does_not_reuse_its_allowance_for_round_two() {
    use miopunch_native_probe::{control_session::CommitError, nat_connection::UdpPolicy};
    let directories = tempfile::tempdir().unwrap();
    let path = directories.path().join("visitor");
    let initial = NativeNetwork::create(&path).await.unwrap();
    let mut config = initial.resource_configuration().unwrap();
    config.replacement = Some(Vec::new());
    initial.save_resource_configuration(&config).unwrap();
    initial.close().await;
    let visitor = NativeNetwork::open_with_udp_resources(
        &path,
        None,
        UdpPolicy {
            max_rounds: 2,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let key = SecretKey::from_bytes(
        &std::fs::read(path.join("node.key"))
            .unwrap()
            .try_into()
            .unwrap(),
    );
    let pair = Pair::build(
        Ipv4Addr::LOCALHOST.into(),
        false,
        Some((key, visitor.admission_gate())),
        &[],
        true,
        &[],
    )
    .await;
    let dead = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let generation = pair.c.udp_network_generation();
    let now = Instant::now();
    let fallback = MappingSnapshot::fallback_generation(
        generation.clone(),
        UdpObservation::Fresh {
            addresses: vec![dead.local_addr().unwrap()],
            observed_at: now,
            valid_until: now + Duration::from_secs(20),
            mapping_varies_by_dest: None,
            validity: generation,
        },
    )
    .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
    let (peer, mut client) = pair.connect_until(Some(deadline)).await;
    let id = client.stable_id();
    let nonce = [0u8; 16];
    client
        .queue_business(b"held while waiting for another round".to_vec())
        .unwrap();
    let analyzer = Arc::new(Mutex::new(Analyzer::new(Instant::now())));
    let script = async {
        scripted_direct_reply(
            &peer,
            serde_json::json!({"Result":{"nonce":nonce,"available":false}}),
        )
        .await;
        let first = nat_punch::coordinate_server(&peer, &pair.c_gate, &fallback, &analyzer)
            .await
            .unwrap();
        assert!(!first.plan().disabled);
        // Keep the original session alive while the first phase has no winner.
        let exchange = peer.accept_control_exchange().await.unwrap();
        let frame: serde_json::Value = serde_json::from_slice(
            &exchange
                .recv_message(iroh::endpoint::ControlMessageFormat::EndTerminated {
                    maximum: 16 * 1024,
                })
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(frame.get("Unavailable").is_some(), "{frame}");
        send_json(&exchange, frame).await;
    };
    let (result, ()) = tokio::time::timeout_at(deadline, async {
        tokio::join!(visitor.upgrade_udp(client), script)
    })
    .await
    .unwrap();
    let Err(CommitError::Retryable { error, session }) = result else {
        panic!("a second enhanced round must observe the original cooldown");
    };
    assert!(
        error
            .to_string()
            .contains("local enhanced-round budget deferred")
    );
    assert_eq!(session.stable_id(), id);
    assert!(!session.has_data_path());
    assert!(
        dead.try_recv(&mut [0; 1500]).is_ok(),
        "the first plan sent a real peer probe"
    );
    pair.close().await;
    visitor.close().await;
}

#[tokio::test]
async fn formal_entry_without_s_uses_local_assisted_basic_attempt_and_original_business() {
    use miopunch_native_probe::nat_connection::{ControlUdpTraversal, UdpPolicy};
    let pair = Pair::without_observers().await;
    assert!(!pair.c.udp_observation().get().is_fresh());
    assert!(!pair.v.udp_observation().get().is_fresh());
    let (c, v) = pair.connect().await;
    let ids = (c.stable_id(), v.stable_id());
    let cn = ControlUdpTraversal::new(None, UdpPolicy::default());
    let vn = ControlUdpTraversal::new(None, UdpPolicy::default());
    let (ca, va) = tokio::join!(cn.prepare(&c, &pair.c_gate), vn.prepare(&v, &pair.v_gate));
    let (mut ca, mut va) = (ca.unwrap(), va.unwrap());
    assert!(ca.plan().fallback && !ca.plan().disabled);
    assert!(ca.mapped_candidates().is_empty() && va.mapped_candidates().is_empty());
    assert_eq!(ca.assisted_candidates(), pair.v.bound_sockets());
    assert_eq!(va.assisted_candidates(), pair.c.bound_sockets());
    assert_eq!(ca.plan(), va.plan());
    assert_eq!(ca.plan().id, BehaviorId { mode: 0, index: 6 });
    for behavior in [&ca.plan().c, &ca.plan().v] {
        assert_eq!(behavior.ttl, 0);
        assert_eq!(behavior.send_delay_ms, 0);
    }
    let (cp, vp) = tokio::join!(
        cn.execute(&mut ca, &c, &pair.c_gate),
        vn.execute(&mut va, &v, &pair.v_gate)
    );
    cp.unwrap();
    vp.unwrap();
    let (cd, vd) = tokio::join!(
        ca.commit_winner_path(c, &pair.c_gate),
        va.commit_winner_path(v, &pair.v_gate)
    );
    let (cd, vd) = (cd.unwrap(), vd.unwrap());
    assert_eq!(
        (cd.connection().stable_id(), vd.connection().stable_id()),
        ids
    );
    let (cr, vr) = tokio::join!(
        cn.exchange_business(&mut ca, &cd, &pair.c_gate, b"assisted-acceptor"),
        vn.exchange_business(&mut va, &vd, &pair.v_gate, b"assisted-visitor")
    );
    assert_eq!(cr.unwrap(), b"assisted-visitor");
    assert_eq!(vr.unwrap(), b"assisted-acceptor");
    pair.close().await;
}

#[tokio::test]
async fn formal_entry_permission_and_complete_budget_stop_before_execution() {
    use miopunch_native_probe::nat_connection::{ControlUdpTraversal, UdpPolicy};
    let pair = Pair::new().await;
    let (c, v) = pair.connect().await;
    let cn = ControlUdpTraversal::new(
        Some(pair.observers.clone()),
        UdpPolicy {
            enabled: false,
            ..Default::default()
        },
    );
    assert_eq!(
        cn.observe(&c, &pair.c_gate).await.unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    cn.config().set_udp_enabled(true);
    let vn = ControlUdpTraversal::new(Some(pair.observers.clone()), UdpPolicy::default());
    let (ca, va) = tokio::join!(cn.prepare(&c, &pair.c_gate), vn.prepare(&v, &pair.v_gate));
    let (mut ca, mut va) = (ca.unwrap(), va.unwrap());
    let low = ControlUdpTraversal::new(
        None,
        UdpPolicy {
            peer_datagrams: 1,
            ..Default::default()
        },
    );
    let error = low.execute(&mut ca, &c, &pair.c_gate).await.unwrap_err();
    assert_eq!(error.to_string(), "foreign NAT round allowance");
    assert!(ca.execution_trace().is_none());
    assert!(ca.local_winner().is_none());
    let run = vn.execute(&mut va, &v, &pair.v_gate);
    let disable = async {
        tokio::time::sleep(Duration::from_millis(30)).await;
        vn.config().set_udp_enabled(false);
    };
    let (result, _) = tokio::join!(run, disable);
    assert_eq!(
        result.unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        va.execution_trace().unwrap().probes,
        0,
        "disabled during the200ms sender delay"
    );
    assert!(va.local_winner().is_none());

    let (c, v) = pair.connect().await;
    let spare = ControlUdpTraversal::new(None, UdpPolicy::default());
    let (cr, vr) = tokio::join!(
        low.prepare(&c, &pair.c_gate),
        spare.prepare(&v, &pair.v_gate)
    );
    assert_eq!(
        cr.unwrap_err().to_string(),
        "NAT complete-plan budget insufficient"
    );
    assert_eq!(
        vr.unwrap_err().to_string(),
        "peer deferred the active path attempt"
    );
    pair.close().await;
}

#[tokio::test]
async fn split_round_snapshot_and_attempt_have_one_owner_and_one_execution() {
    use miopunch_native_probe::nat_connection::{ControlUdpTraversal, UdpPolicy};
    use std::io::ErrorKind;

    let pair = Pair::without_observers().await;
    let (c, v) = pair.connect().await;
    let (c2, _) = pair.connect().await;
    let cn = ControlUdpTraversal::new(None, UdpPolicy::default());
    let vn = ControlUdpTraversal::new(None, UdpPolicy::default());
    let cs = cn.observe(&c, &pair.c_gate).await.unwrap();
    let duplicate = cs.clone();
    // Exercise caller-supplied evidence on the peer through the same seam.
    let vs = MappingSnapshot::fallback_generation(
        pair.v.udp_network_generation(),
        UdpObservation::Disabled,
    )
    .unwrap()
    .with_assisted(pair.v.bound_sockets())
    .unwrap();
    assert_eq!(
        cn.clone()
            .observe(&c2, &pair.c_gate)
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::WouldBlock,
    );
    assert_eq!(
        vn.coordinate(&c, &pair.c_gate, &duplicate)
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidInput,
    );
    let (contending, waiting) = pair.connect().await;
    let external = MappingSnapshot::fallback_generation(
        pair.c.udp_network_generation(),
        UdpObservation::Disabled,
    )
    .unwrap();
    let idle_peer = ControlUdpTraversal::new(None, UdpPolicy::default());
    let (local, remote) = tokio::join!(
        cn.coordinate(&contending, &pair.c_gate, &external),
        idle_peer.prepare(&waiting, &pair.v_gate),
    );
    assert!(
        local
            .unwrap_err()
            .to_string()
            .contains("local enhanced-round budget deferred")
    );
    assert!(remote.unwrap_err().to_string().contains("peer deferred"));
    let (ca, va) = tokio::join!(
        cn.coordinate(&c, &pair.c_gate, &cs),
        vn.coordinate(&v, &pair.v_gate, &vs),
    );
    let (mut ca, mut va) = (ca.unwrap(), va.unwrap());
    assert_eq!(ca.plan(), va.plan());
    for (connector, control, gate, copy) in [
        (&cn, &c, &pair.c_gate, duplicate),
        (&vn, &v, &pair.v_gate, vs.clone()),
    ] {
        assert_eq!(
            connector
                .coordinate(control, gate, &copy)
                .await
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidData
        );
    }
    assert_eq!(
        ca.execute(&c, &pair.c_gate, &NodeProbeBudget::new(0, usize::MAX))
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidData,
    );
    let cloned = cn.clone();
    let (cr, vr) = tokio::join!(
        cloned.execute(&mut ca, &c, &pair.c_gate),
        vn.execute(&mut va, &v, &pair.v_gate),
    );
    cr.unwrap();
    vr.unwrap();
    assert_eq!(
        cn.execute(&mut ca, &c, &pair.c_gate)
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidData
    );
    pair.change_network_generation().await;
    let (c3, _) = pair.connect().await;
    let snapshot = cn.observe(&c3, &pair.c_gate).await.unwrap();
    // The completed Attempt still owns its winner, with the round already released.
    assert!(ca.local_winner().is_some() && snapshot.is_current());
    pair.close().await;
}

#[tokio::test]
async fn discarded_and_expired_split_results_release_only_their_own_occupancy() {
    use miopunch_native_probe::nat_connection::{ControlUdpTraversal, UdpPolicy};
    use std::io::ErrorKind;

    let pair = Pair::without_observers().await;
    let cn = ControlUdpTraversal::new(None, UdpPolicy::default());
    let vn = ControlUdpTraversal::new(None, UdpPolicy::default());
    let (c, _) = pair.connect().await;
    let snapshot = cn.observe(&c, &pair.c_gate).await.unwrap();
    drop(snapshot);
    assert_eq!(
        cn.observe(&c, &pair.c_gate).await.unwrap_err().kind(),
        ErrorKind::WouldBlock
    );
    pair.change_network_generation().await;
    let (new_c, _new_v) = pair.connect().await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let (c, v) = pair.connect_until(Some(deadline)).await;
    let (ca, va) = tokio::join!(cn.prepare(&c, &pair.c_gate), vn.prepare(&v, &pair.v_gate));
    let mut ca = ca.unwrap();
    drop(va.unwrap());
    pair.change_network_generation().await;
    assert_eq!(
        cn.observe(&new_c, &pair.c_gate).await.unwrap_err().kind(),
        ErrorKind::WouldBlock,
        "a network change does not take an outstanding permit away before expiry",
    );
    let (_fresh_c, fresh_v) = pair.connect().await;
    let old_snapshot = vn.observe(&fresh_v, &pair.v_gate).await.unwrap();
    drop(old_snapshot);
    tokio::time::sleep_until(deadline).await;
    let next = cn.observe(&new_c, &pair.c_gate).await.unwrap();
    let error = cn.execute(&mut ca, &c, &pair.c_gate).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::TimedOut);
    drop(ca);
    let (third_c, _third_v) = pair.connect().await;
    assert_eq!(
        cn.observe(&third_c, &pair.c_gate).await.unwrap_err().kind(),
        ErrorKind::WouldBlock
    );
    assert!(next.is_current());
    pair.close().await;
}

#[tokio::test]
async fn ordinary_udp_opens_a_new_path_on_the_relay_control_connection() {
    use miopunch_native_probe::nat_connection::{ControlUdpTraversal, UdpPolicy};
    let (_directories, visitor, pair) = native_upgrade_pair(&[], true).await;
    let (peer, client) = pair.connect().await;
    let stable = (peer.stable_id(), client.stable_id());
    assert!(!client.has_data_path() && !peer.has_data_path());
    let acceptor = ControlUdpTraversal::new(None, UdpPolicy::default());
    let (opened_peer, opened_client) = tokio::join!(
        acceptor.upgrade(peer, &pair.c_gate),
        visitor.upgrade_udp(client),
    );
    let (opened_peer, opened_client) = (opened_peer.unwrap(), opened_client.unwrap());
    assert!(opened_peer.direct().is_some() && opened_client.direct().is_some());
    assert!(opened_peer.attempt().is_none() && opened_client.attempt().is_none());
    assert_eq!(
        (
            opened_peer.data.connection().stable_id(),
            opened_client.data.connection().stable_id()
        ),
        stable
    );
    pair.close().await;
}

#[tokio::test]
async fn formal_direct_candidates_precede_selector_and_keep_original_identity() {
    use miopunch_native_probe::nat_connection::{ControlUdpTraversal, UdpPolicy};
    let pair = Pair::new().await;
    let (c, v) = pair.connect().await;
    let ids = (c.stable_id(), v.stable_id());
    let cn = ControlUdpTraversal::new(Some(pair.observers.clone()), UdpPolicy::default());
    let vn = ControlUdpTraversal::new(Some(pair.observers.clone()), UdpPolicy::default());
    let (cd, vd) = tokio::join!(cn.upgrade(c, &pair.c_gate), vn.upgrade(v, &pair.v_gate));
    let (mut cd, mut vd) = (cd.unwrap(), vd.unwrap());
    assert!(cd.attempt().is_none() && vd.attempt().is_none());
    assert!(cd.direct().is_some() && vd.direct().is_some());
    assert_eq!(
        (
            cd.data.connection().stable_id(),
            vd.data.connection().stable_id()
        ),
        ids
    );
    let (cr, vr) = tokio::join!(
        cd.exchange_business(b"accepting-direct-business"),
        vd.exchange_business(b"visiting-direct-business")
    );
    assert_eq!(cr.unwrap(), b"visiting-direct-business");
    assert_eq!(vr.unwrap(), b"accepting-direct-business");
    cn.config().set_udp_enabled(false);
    assert_eq!(
        cd.exchange_business(b"revoked").await.unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied,
    );
    assert!(tokio::time::timeout(Duration::from_millis(100), vd.data.connection().accept_bi()).await.is_err());
    pair.close().await;
}

#[tokio::test]
async fn formal_disabled_udp_blocks_business_before_stream_creation() {
    use miopunch_native_probe::nat_connection::{ControlUdpTraversal, UdpPolicy};
    let pair = Pair::new().await;
    let (c, v) = pair.connect().await;
    let cn = ControlUdpTraversal::new(Some(pair.observers.clone()), UdpPolicy::default());
    let vn = ControlUdpTraversal::new(Some(pair.observers.clone()), UdpPolicy::default());
    let (ca, va) = tokio::join!(cn.prepare(&c, &pair.c_gate), vn.prepare(&v, &pair.v_gate));
    let (mut ca, mut va) = (ca.unwrap(), va.unwrap());
    let (cp, vp) = tokio::join!(
        cn.execute(&mut ca, &c, &pair.c_gate),
        vn.execute(&mut va, &v, &pair.v_gate)
    );
    cp.unwrap();
    vp.unwrap();
    let (cd, vd) = tokio::join!(
        ca.commit_winner_path(c, &pair.c_gate),
        va.commit_winner_path(v, &pair.v_gate)
    );
    let (cd, vd) = (cd.unwrap(), vd.unwrap());
    cn.config().set_udp_enabled(false);
    assert_eq!(
        cn.exchange_business(&mut ca, &cd, &pair.c_gate, b"must-stay-local")
            .await
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), vd.connection().accept_bi())
            .await
            .is_err(),
        "disabled resource must not even create an application stream"
    );
    pair.close().await;
}

#[tokio::test]
async fn formal_ipv6_direct_requires_no_ipv4_observation_or_punching() {
    use miopunch_native_probe::nat_connection::{ControlUdpTraversal, UdpPolicy};
    let pair = Pair::with_observers(std::net::Ipv6Addr::LOCALHOST.into(), false).await;
    let (c, v) = pair.connect().await;
    let ids = (c.stable_id(), v.stable_id());
    let cn = ControlUdpTraversal::new(None, UdpPolicy::default());
    let vn = ControlUdpTraversal::new(None, UdpPolicy::default());
    let (cu, vu) = tokio::join!(cn.upgrade(c, &pair.c_gate), vn.upgrade(v, &pair.v_gate));
    let (mut cu, mut vu) = (cu.unwrap(), vu.unwrap());
    assert!(cu.direct().unwrap().is_ipv6() && vu.direct().unwrap().is_ipv6());
    assert!(cu.attempt().is_none() && vu.attempt().is_none());
    assert_eq!(
        (
            cu.data.connection().stable_id(),
            vu.data.connection().stable_id()
        ),
        ids
    );
    let (c, v) = tokio::join!(
        cu.exchange_business(b"ipv6-acceptor"),
        vu.exchange_business(b"ipv6-visitor")
    );
    assert_eq!(c.unwrap(), b"ipv6-visitor");
    assert_eq!(v.unwrap(), b"ipv6-acceptor");
    pair.close().await;
}

struct Pair {
    c: iroh::endpoint::RestrictedEndpoint,
    v: iroh::endpoint::RestrictedEndpoint,
    pending: Mutex<Vec<tokio::sync::mpsc::UnboundedReceiver<handoff::PendingControl>>>,
    c_gate: AdmissionGate,
    v_gate: AdmissionGate,
    observers: RelayMap,
    relay: Option<iroh::RelayUrl>,
    _servers: Vec<server::Server>,
}

impl Pair {
    async fn new() -> Self {
        Self::with_observers(Ipv4Addr::LOCALHOST.into(), true).await
    }

    async fn without_observers() -> Self {
        Self::with_observers(Ipv4Addr::LOCALHOST.into(), false).await
    }

    async fn with_observers(bind: std::net::IpAddr, observe: bool) -> Self {
        Self::build(bind, observe, None, &[], false, &[]).await
    }

    async fn build(
        bind: std::net::IpAddr,
        observe: bool,
        visitor: Option<(SecretKey, AdmissionGate)>,
        advertised: &[SocketAddr],
        use_relay: bool,
        allowed_observers: &[iroh::RelayUrl],
    ) -> Self {
        let (certs, tls) = server::testing::self_signed_tls_certs_and_config();
        let mut servers = Vec::new();
        let mut configs = Vec::new();
        for _ in 0..if observe { 2 } else { 0 } {
            let mut quic = server::QuicConfig::new((Ipv4Addr::LOCALHOST, 0));
            quic.server_config = Some(tls.clone());
            let mut config = server::ServerConfig::default();
            config.quic = Some(quic);
            let server = server::Server::spawn(config).await.unwrap();
            let port = server.quic_addr().unwrap().port();
            configs.push(RelayConfig::new(
                format!("https://127.0.0.1:{port}/").parse().unwrap(),
                Some(RelayQuicConfig::new(port)),
            ));
            servers.push(server);
        }
        let observers = RelayMap::from_iter(configs);
        let c_key = SecretKey::generate();
        let scope = AdmissionScope::new([62; 32], 1);
        let secret = AdmissionSecret::from_saved([42; 32]);
        let c_gate = AdmissionGate::new(c_key.public(), scope.clone(), Some(secret.clone()));
        let (v_key, v_gate) = visitor.unwrap_or_else(|| {
            let key = SecretKey::generate();
            let gate = AdmissionGate::new(key.public(), scope.clone(), Some(secret.clone()));
            (key, gate)
        });
        v_gate.rotate_secret(scope, secret);
        let members = [*c_key.public().as_bytes(), *v_key.public().as_bytes()];
        for gate in [&c_gate, &v_gate] {
            gate.update_authorization(members, []);
        }
        let mut endpoints = Vec::new();
        let mut receivers = Vec::new();
        let relay = if use_relay {
            let mut config = server::ServerConfig::default();
            config.relay = Some(server::RelayConfig::new((Ipv4Addr::LOCALHOST, 0)));
            let relay = server::Server::spawn(config).await.unwrap();
            let url = relay.http_url().unwrap();
            servers.push(relay);
            Some(url)
        } else {
            None
        };
        for (key, gate, extra) in [(c_key, &c_gate, &[][..]), (v_key, &v_gate, advertised)] {
            let (pending, receive) = tokio::sync::mpsc::unbounded_channel();
            receivers.push(receive);
            let mut builder = Endpoint::builder(presets::Minimal)
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .secret_key(key)
                .alpns(vec![CONTROL_ALPN.to_vec()])
                .clear_ip_transports()
                .bind_addr((bind, 0))
                .unwrap()
                .clear_relay_transports()
                .observation_servers(observers.clone())
                .ca_tls_config(CaTlsConfig::custom_roots(certs.clone()));
            if let Some(url) = &relay {
                use iroh::endpoint::{RelayResourcePolicy, RelayResourceUse};
                let permitted = url.clone();
                let allowed_observers = allowed_observers.to_vec();
                builder = builder
                    .relay_mode(iroh::RelayMode::Custom(RelayMap::from_iter([
                        RelayConfig::new(url.clone(), None),
                    ])))
                    .relay_resource_policy(RelayResourcePolicy::new(move |url, usage| {
                        (*url == permitted && matches!(usage, RelayResourceUse::DataRelay))
                            || (matches!(usage, RelayResourceUse::Observation)
                                && allowed_observers.contains(url))
                    }));
            }
            for addr in extra {
                builder = builder.external_addr(*addr);
            }
            let endpoint = builder
                .restricted_application(handoff::PendingDataPolicy {
                    gate: gate.clone(),
                    pending,
                })
                .bind()
                .await
                .unwrap();
            let mut observation = endpoint.udp_observation();
            tokio::time::timeout(Duration::from_secs(5), async {
                while matches!(observation.get(), UdpObservation::Pending) {
                    observation.updated().await.unwrap();
                }
            })
            .await
            .unwrap();
            if relay.is_some() {
                tokio::time::timeout(Duration::from_secs(5), endpoint.online())
                    .await
                    .unwrap();
            }
            endpoints.push(endpoint);
        }
        let v = endpoints.pop().unwrap();
        let c = endpoints.pop().unwrap();
        Self {
            c,
            v,
            c_gate,
            v_gate,
            observers,
            relay,
            _servers: servers,
            pending: Mutex::new(receivers),
        }
    }

    async fn connect(&self) -> (ControlSession, ControlSession) {
        self.connect_until(None).await
    }

    async fn connect_until(
        &self,
        deadline: Option<n0_future::time::Instant>,
    ) -> (ControlSession, ControlSession) {
        let target = EndpointAddr::new(self.c.id()).with_addrs(match &self.relay {
            Some(url) => vec![TransportAddr::Relay(url.clone())],
            None => self
                .c
                .bound_sockets()
                .into_iter()
                .map(TransportAddr::Ip)
                .collect(),
        });
        let c = self.c.clone();
        let v = self.v.clone();
        let accept = tokio::spawn(async move {
            let incoming = c.accept().await.unwrap();
            let incoming = if let Some(deadline) = deadline {
                incoming.with_caller_deadline(deadline)
            } else {
                incoming
            };
            incoming.await.map_err(std::io::Error::other)
        });
        let dial = tokio::spawn(async move {
            let mut options = iroh::endpoint::ConnectOptions::new();
            if let Some(deadline) = deadline {
                options = options.with_caller_deadline(deadline);
            }
            v.connect_with_opts(target, CONTROL_ALPN, options)
                .await
                .map_err(std::io::Error::other)?
                .await
                .map_err(std::io::Error::other)
        });
        let mut receivers = self.pending.lock().unwrap();
        let c = receivers[0].recv().await.unwrap();
        let v = receivers[1].recv().await.unwrap();
        (c.session(accept), v.session(dial))
    }

    async fn snapshots(&self) -> (MappingSnapshot, MappingSnapshot) {
        let c = self
            .c
            .observe_udp_mapping_sequence(&self.observers)
            .await
            .unwrap();
        let v = self
            .v
            .observe_udp_mapping_sequence(&self.observers)
            .await
            .unwrap();
        assert_eq!(c.samples.len(), 2);
        assert_eq!(v.samples.len(), 2);
        assert!(
            c.samples
                .iter()
                .all(|s| s.mapped == self.c.bound_sockets()[0])
        );
        assert!(
            v.samples
                .iter()
                .all(|s| s.mapped == self.v.bound_sockets()[0])
        );
        (
            MappingSnapshot::from_sequence(c).unwrap(),
            MappingSnapshot::from_sequence(v).unwrap(),
        )
    }

    async fn close(&self) {
        self.v.close().await;
        self.c.close().await;
    }

    async fn change_network_generation(&self) {
        let (cg, vg) = (
            self.c.udp_network_generation(),
            self.v.udp_network_generation(),
        );
        tokio::join!(self.c.network_change(), self.v.network_change());
        tokio::time::timeout(Duration::from_secs(2), async {
            while cg.is_current() || vg.is_current() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            for endpoint in [&self.c, &self.v] {
                let mut observations = endpoint.udp_observation();
                while matches!(
                    observations.get(),
                    UdpObservation::Pending
                        | UdpObservation::Failed(UdpObservationFailure::NetworkChanged)
                ) {
                    observations.updated().await.unwrap();
                }
            }
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn two_real_qad_observers_public_preference_original_business_and_learning() {
    let pair = Pair::new().await;
    let (c_snapshot, v_snapshot) = pair.snapshots().await;
    for snapshot in [&c_snapshot, &v_snapshot] {
        let feature = snapshot.feature().unwrap();
        assert_eq!(feature.nat_type, NatType::Easy);
        assert!(feature.public_network);
    }
    let (c, v) = pair.connect().await;
    let stable = c.stable_id();
    let analyzer = Arc::new(Mutex::new(Analyzer::new(Instant::now())));
    let (c_attempt, v_attempt) = tokio::join!(
        nat_punch::coordinate_server(&c, &pair.c_gate, &c_snapshot, &analyzer),
        nat_punch::coordinate_client(&v, &pair.v_gate, &v_snapshot),
    );
    let mut c_attempt = c_attempt.unwrap();
    let mut v_attempt = v_attempt.unwrap();
    assert_eq!(c_attempt.plan(), v_attempt.plan());
    assert_eq!(c_attempt.plan().id, BehaviorId { mode: 0, index: 2 });
    assert_eq!(c_attempt.local_behavior().role, Role::Receiver);
    let c_budget = NodeProbeBudget::new(0, 2);
    let v_budget = NodeProbeBudget::new(0, 1);
    assert_eq!(
        c_attempt
            .execute_partial(&c, &pair.c_gate, &c_budget)
            .await
            .unwrap(),
        ExecutionStatus::PartialExecution
    );
    assert_eq!(
        v_attempt
            .execute_partial(&v, &pair.v_gate, &v_budget)
            .await
            .unwrap(),
        ExecutionStatus::PartialExecution
    );
    assert!(
        c_attempt
            .execute_partial(&c, &pair.c_gate, &c_budget)
            .await
            .is_err(),
        "a repeated attempt must not spend the remaining budget"
    );
    assert!(c_budget.spend_peer_datagram());
    let (c_data, v_data) = tokio::join!(c.commit_data_path(), v.commit_data_path());
    let (c_data, v_data) = (c_data.unwrap(), v_data.unwrap());
    assert_eq!(c_data.connection().stable_id(), stable);
    let (mut send, mut recv) = v_data.connection().open_bi().await.unwrap();
    send.write_all(b"legacy-business").await.unwrap();
    send.finish().unwrap();
    let (mut reply, mut request) = c_data.connection().accept_bi().await.unwrap();
    assert_eq!(request.read_to_end(64).await.unwrap(), b"legacy-business");
    reply.write_all(b"received").await.unwrap();
    reply.finish().unwrap();
    assert_eq!(recv.read_to_end(64).await.unwrap(), b"received");
    assert!(c_attempt.confirm_success(&c_data, &pair.c_gate, &analyzer));
    assert!(
        !c_attempt.confirm_success(&c_data, &pair.c_gate, &analyzer),
        "business feedback consumed once"
    );
    pair.close().await;
}

#[tokio::test]
async fn old_prediction_age_does_not_expire_a_legacy_plan_but_network_change_does() {
    let pair = Pair::new().await;
    let (c_snapshot, _) = pair.snapshots().await;
    tokio::time::sleep(Duration::from_millis(2100)).await;
    assert!(
        c_snapshot.is_current(),
        "old 3/5/10 second delays need more than the predictor's two seconds"
    );
    let assisted = MappingSnapshot::fallback_generation(
        pair.c.udp_network_generation(),
        UdpObservation::Disabled,
    )
    .unwrap()
    .with_assisted(pair.c.bound_sockets())
    .unwrap();
    assert!(assisted.is_current());
    pair.c.network_change().await;
    tokio::time::timeout(Duration::from_secs(3), async {
        while c_snapshot.is_current() || assisted.is_current() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        MappingSnapshot::fallback_generation(
            pair.c.udp_network_generation(),
            UdpObservation::Failed(UdpObservationFailure::NetworkChanged)
        )
        .is_err()
    );
    pair.close().await;
}

#[tokio::test]
async fn cancelled_or_revoked_round_has_no_peer_budget_or_learning_feedback() {
    let pair = Pair::new().await;
    let (c_snapshot, v_snapshot) = pair.snapshots().await;
    let (c, v) = pair.connect().await;
    let analyzer = Arc::new(Mutex::new(Analyzer::new(Instant::now())));
    let (c_attempt, v_attempt) = tokio::join!(
        nat_punch::coordinate_server(&c, &pair.c_gate, &c_snapshot, &analyzer),
        nat_punch::coordinate_client(&v, &pair.v_gate, &v_snapshot),
    );
    let mut c_attempt = c_attempt.unwrap();
    let mut v_attempt = v_attempt.unwrap();
    let budget = NodeProbeBudget::new(0, 1);
    c_attempt.finish_failure();
    assert!(
        c_attempt
            .execute_partial(&c, &pair.c_gate, &budget)
            .await
            .is_err()
    );
    assert!(budget.spend_peer_datagram());
    pair.v_gate
        .update_authorization([*pair.v.id().as_bytes()], [*pair.c.id().as_bytes()]);
    let budget = NodeProbeBudget::new(0, 1);
    assert!(
        v_attempt
            .execute_partial(&v, &pair.v_gate, &budget)
            .await
            .is_err()
    );
    assert!(budget.spend_peer_datagram());
    pair.close().await;
}

async fn send_json(exchange: &iroh::endpoint::ControlExchange, value: serde_json::Value) {
    exchange
        .send_message(
            iroh::endpoint::ControlMessageFormat::EndTerminated { maximum: 16384 },
            &serde_json::to_vec(&value).unwrap(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn wrong_network_and_old_round_are_rejected_before_snapshot_disclosure_or_selection() {
    let pair = Pair::new().await;
    let (snapshot, _) = pair.snapshots().await;
    for wrong_network in [true, false] {
        let (c, v) = pair.connect().await;
        let analyzer = Arc::new(Mutex::new(Analyzer::new(Instant::now())));
        let malicious_visitor = async {
            let exchange = v.open_control_exchange().await.unwrap();
            send_json(&exchange,serde_json::json!({"Hello":{"network":vec![if wrong_network {99} else {62};32],"generation":1}})).await;
            if wrong_network {
                return;
            }
            let challenge: serde_json::Value = serde_json::from_slice(
                &exchange
                    .recv_message(iroh::endpoint::ControlMessageFormat::EndTerminated {
                        maximum: 16384,
                    })
                    .await
                    .unwrap(),
            )
            .unwrap();
            let mut round = challenge["Challenge"]["round"].as_array().unwrap().clone();
            round[0] = serde_json::json!(round[0].as_u64().unwrap() ^ 1);
            let exchange = v.open_control_exchange().await.unwrap();
            send_json(&exchange,serde_json::json!({"Offer":{"round":round,"snapshot":{"mapped":vec![pair.v.bound_sockets()[0].to_string();2],"local_ips":[],"assisted":[]}}})).await;
        };
        let (result, _) = tokio::join!(
            nat_punch::coordinate_server(&c, &pair.c_gate, &snapshot, &analyzer),
            malicious_visitor
        );
        assert!(result.is_err());
        assert_eq!(
            analyzer.lock().unwrap().clean(Instant::now()),
            (0, 0),
            "invalid scope/round must not create or decrement a scene"
        );
        let budget = NodeProbeBudget::new(0, 1);
        assert!(budget.spend_peer_datagram());
        drop(c);
        drop(v);
    }
    pair.close().await;
}

#[tokio::test]
async fn wrong_gate_and_empty_fallback_cannot_disclose_candidates_or_spend_peer_budget() {
    let pair = Pair::new().await;
    let (c, v) = pair.connect().await;
    let snapshot = MappingSnapshot::fallback_generation(
        pair.c.udp_network_generation(),
        UdpObservation::Disabled,
    )
    .unwrap();
    let analyzer = Arc::new(Mutex::new(Analyzer::new(Instant::now())));
    let wrong = AdmissionGate::new(
        SecretKey::generate().public(),
        AdmissionScope::new([62; 32], 1),
        Some(AdmissionSecret::from_saved([42; 32])),
    );
    assert!(
        nat_punch::coordinate_server(&c, &wrong, &snapshot, &analyzer)
            .await
            .is_err()
    );
    assert_eq!(analyzer.lock().unwrap().clean(Instant::now()), (0, 0));
    let c_empty = MappingSnapshot::fallback_generation(
        pair.c.udp_network_generation(),
        UdpObservation::Disabled,
    )
    .unwrap();
    let v_empty = MappingSnapshot::fallback_generation(
        pair.v.udp_network_generation(),
        UdpObservation::Disabled,
    )
    .unwrap();
    let (c_attempt, v_attempt) = tokio::join!(
        nat_punch::coordinate_server(&c, &pair.c_gate, &c_empty, &analyzer),
        nat_punch::coordinate_client(&v, &pair.v_gate, &v_empty),
    );
    let mut c_attempt = c_attempt.unwrap();
    let mut v_attempt = v_attempt.unwrap();
    assert!(c_attempt.plan().fallback && c_attempt.plan().disabled);
    let budget = NodeProbeBudget::new(0, 1);
    assert_eq!(
        c_attempt
            .execute_partial(&c, &pair.c_gate, &budget)
            .await
            .unwrap(),
        ExecutionStatus::Disabled
    );
    assert_eq!(
        v_attempt
            .execute_partial(&v, &pair.v_gate, &budget)
            .await
            .unwrap(),
        ExecutionStatus::Disabled
    );
    assert!(budget.spend_peer_datagram());
    assert_eq!(analyzer.lock().unwrap().clean(Instant::now()), (0, 0));
    pair.close().await;
}

#[tokio::test]
async fn assisted_only_fallback_receives_without_an_outbound_target_and_sender_keeps_lan_first() {
    let pair = Pair::new().await;
    let (c, v) = pair.connect().await;
    let c_snapshot = MappingSnapshot::fallback_generation(
        pair.c.udp_network_generation(),
        UdpObservation::Disabled,
    )
    .unwrap()
    .with_assisted(pair.c.bound_sockets())
    .unwrap();
    let v_snapshot = MappingSnapshot::fallback_generation(
        pair.v.udp_network_generation(),
        UdpObservation::Disabled,
    )
    .unwrap();
    let analyzer = Arc::new(Mutex::new(Analyzer::new(Instant::now())));
    let (c_attempt, v_attempt) = tokio::join!(
        nat_punch::coordinate_server(&c, &pair.c_gate, &c_snapshot, &analyzer),
        nat_punch::coordinate_client(&v, &pair.v_gate, &v_snapshot),
    );
    let mut c_attempt = c_attempt.unwrap();
    let mut v_attempt = v_attempt.unwrap();
    assert!(c_attempt.plan().fallback);
    assert_eq!(c_attempt.local_behavior().role, Role::Receiver);
    assert_eq!(v_attempt.local_behavior().role, Role::Sender);
    assert_eq!(c_attempt.remote_target(), None);
    assert_eq!(v_attempt.remote_target(), Some(pair.c.bound_sockets()[0]));
    let receive_budget = NodeProbeBudget::new(0, 1);
    let send_budget = NodeProbeBudget::new(0, 1);
    assert_eq!(
        c_attempt
            .execute_partial(&c, &pair.c_gate, &receive_budget)
            .await
            .unwrap(),
        ExecutionStatus::PartialExecution
    );
    assert!(
        receive_budget.spend_peer_datagram(),
        "receiver without a target must not submit a packet"
    );
    assert_eq!(
        v_attempt
            .execute_partial(&v, &pair.v_gate, &send_budget)
            .await
            .unwrap(),
        ExecutionStatus::PartialExecution
    );
    assert!(!send_budget.spend_peer_datagram());
    let (c_data, v_data) = tokio::join!(c.commit_data_path(), v.commit_data_path());
    let (c_data, v_data) = (c_data.unwrap(), v_data.unwrap());
    let (mut send, _) = v_data.connection().open_bi().await.unwrap();
    send.write_all(b"assisted").await.unwrap();
    send.finish().unwrap();
    let (_, mut recv) = c_data.connection().accept_bi().await.unwrap();
    assert_eq!(recv.read_to_end(64).await.unwrap(), b"assisted");
    assert!(
        !c_attempt.confirm_success(&c_data, &pair.c_gate, &analyzer),
        "fallback is not an analyzed learning record"
    );
    assert_eq!(analyzer.lock().unwrap().clean(Instant::now()), (0, 0));
    pair.close().await;
}

#[tokio::test]
async fn existing_verified_path_cannot_learn_when_budget_was_denied_or_credential_changed() {
    for budget_denied in [true, false] {
        let pair = Pair::new().await;
        let (c_snapshot, v_snapshot) = pair.snapshots().await;
        let (c, v) = pair.connect().await;
        let analyzer = Arc::new(Mutex::new(Analyzer::new(Instant::now())));
        let (c_attempt, v_attempt) = tokio::join!(
            nat_punch::coordinate_server(&c, &pair.c_gate, &c_snapshot, &analyzer),
            nat_punch::coordinate_client(&v, &pair.v_gate, &v_snapshot),
        );
        let mut c_attempt = c_attempt.unwrap();
        let mut v_attempt = v_attempt.unwrap();
        let budget = NodeProbeBudget::new(0, usize::from(!budget_denied));
        assert_eq!(
            c_attempt
                .execute_partial(&c, &pair.c_gate, &budget)
                .await
                .is_err(),
            budget_denied
        );
        v_attempt
            .execute_partial(&v, &pair.v_gate, &NodeProbeBudget::new(0, 1))
            .await
            .unwrap();
        let (c_data, v_data) = tokio::join!(c.commit_data_path(), v.commit_data_path());
        let (c_data, v_data) = (c_data.unwrap(), v_data.unwrap());
        let (mut send, _) = v_data.connection().open_bi().await.unwrap();
        send.write_all(b"authorized-before-change").await.unwrap();
        send.finish().unwrap();
        let (_, mut recv) = c_data.connection().accept_bi().await.unwrap();
        assert_eq!(
            recv.read_to_end(64).await.unwrap(),
            b"authorized-before-change"
        );
        if !budget_denied {
            pair.c_gate.rotate_secret(
                AdmissionScope::new([62; 32], 2),
                AdmissionSecret::from_saved([43; 32]),
            );
        }
        assert!(!c_attempt.confirm_success(&c_data, &pair.c_gate, &analyzer));
        assert!(!c_attempt.confirm_success(&c_data, &pair.c_gate, &analyzer));
        pair.close().await;
    }
}

#[tokio::test]
async fn complete_scheduler_preserves_original_tls_connection_business_and_one_feedback() {
    let pair = Pair::new().await;
    let (cs, vs) = pair.snapshots().await;
    let (c, v) = pair.connect().await;
    let stable = c.stable_id();
    let analyzer = Arc::new(Mutex::new(Analyzer::new(Instant::now())));
    let (ca, va) = tokio::join!(
        nat_punch::coordinate_server(&c, &pair.c_gate, &cs, &analyzer),
        nat_punch::coordinate_client(&v, &pair.v_gate, &vs)
    );
    let mut ca = ca.unwrap();
    let mut va = va.unwrap();
    let cb = NodeProbeBudget::new(0, 200);
    let vb = NodeProbeBudget::new(0, 200);
    let (cr, vr) = tokio::join!(
        ca.execute(&c, &pair.c_gate, &cb),
        va.execute(&v, &pair.v_gate, &vb)
    );
    assert_eq!(cr.unwrap(), ExecutionStatus::CompleteExecution);
    assert_eq!(vr.unwrap(), ExecutionStatus::CompleteExecution);
    assert_eq!(ca.remote_target(), Some(pair.v.bound_sockets()[0]));
    assert_eq!(va.remote_target(), Some(pair.c.bound_sockets()[0]));
    assert_eq!(ca.execution_trace().unwrap().responses, 3);
    assert!(va.execution_trace().unwrap().probes > 0);
    let (cd, vd) = tokio::join!(c.commit_data_path(), v.commit_data_path());
    let (cd, vd) = (cd.unwrap(), vd.unwrap());
    assert_eq!(cd.connection().stable_id(), stable);
    let (mut send, mut recv) = vd.connection().open_bi().await.unwrap();
    send.write_all(b"complete-scheduler-business")
        .await
        .unwrap();
    send.finish().unwrap();
    let (mut reply, mut request) = cd.connection().accept_bi().await.unwrap();
    assert_eq!(
        request.read_to_end(64).await.unwrap(),
        b"complete-scheduler-business"
    );
    reply.write_all(b"confirmed").await.unwrap();
    reply.finish().unwrap();
    assert_eq!(recv.read_to_end(64).await.unwrap(), b"confirmed");
    assert!(ca.confirm_success(&cd, &pair.c_gate, &analyzer));
    assert!(!ca.confirm_success(&cd, &pair.c_gate, &analyzer));
    pair.close().await;
}

fn raw_auth(
    local: &iroh::endpoint::RestrictedEndpoint,
    remote: &iroh::endpoint::RestrictedEndpoint,
) -> miopunch_native_probe::nat_scheduler::PacketAuth {
    miopunch_native_probe::nat_scheduler::PacketAuth {
        key: [63; 32],
        local: *local.id().as_bytes(),
        remote: *remote.id().as_bytes(),
    }
}
fn raw_phase(
    role: Role,
    delay: u32,
    targets: &[std::net::SocketAddr],
) -> miopunch_native_probe::nat_scheduler::PhasePlan {
    let b = miopunch_native_probe::nat_selector::Behavior {
        role,
        ttl: 7,
        send_delay_ms: delay,
        ports_range: 0,
        ports_random: 0,
        listen_random: 0,
    };
    miopunch_native_probe::nat_scheduler::PhasePlan::new(b, delay, targets, &[], 0)
}

#[tokio::test]
async fn receiver_can_win_before_delay_without_sending_a_probe_and_lease_closes() {
    use miopunch_native_probe::nat_scheduler;
    let pair = Pair::new().await;
    let (c, v) = pair.connect().await;
    let mut cl = c.register_udp_traversal([1; 16]).await.unwrap();
    let old_sender = cl.sender();
    let mut vl = v.register_udp_traversal([1; 16]).await.unwrap();
    let cp = raw_phase(Role::Receiver, 500, &pair.v.bound_sockets());
    let vp = raw_phase(Role::Sender, 0, &pair.c.bound_sockets());
    let cb = NodeProbeBudget::new(0, 3);
    let vb = NodeProbeBudget::new(0, 2);
    let ca = raw_auth(&pair.c, &pair.v);
    let va = raw_auth(&pair.v, &pair.c);
    let started = Instant::now();
    let (cr, vr) = tokio::join!(
        nat_scheduler::run(&cp, &mut cl, &ca, &cb, &|| Ok(())),
        nat_scheduler::run(&vp, &mut vl, &va, &vb, &|| Ok(()))
    );
    let (cw, trace) = cr.unwrap();
    let (vw, _) = vr.unwrap();
    assert!(cw.before_probe);
    assert_eq!(trace.probes, 0);
    assert_eq!(trace.responses, 3);
    assert!(vw.response);
    assert!(started.elapsed() < Duration::from_millis(500));
    drop(cl);
    // Registering the same opaque round cannot revive an old sender clone.
    let _new = c.register_udp_traversal([1; 16]).await.unwrap();
    assert!(
        old_sender
            .send(pair.v.bound_sockets()[0], &[0; 49], 0)
            .await
            .is_err()
    );
    pair.close().await;
}

#[tokio::test]
async fn dropping_attempt_during_sender_delay_spends_nothing_and_cannot_learn() {
    let pair = Pair::new().await;
    let (cs, vs) = pair.snapshots().await;
    let (c, v) = pair.connect().await;
    let analyzer = Arc::new(Mutex::new(Analyzer::new(Instant::now())));
    let (ca, va) = tokio::join!(
        nat_punch::coordinate_server(&c, &pair.c_gate, &cs, &analyzer),
        nat_punch::coordinate_client(&v, &pair.v_gate, &vs)
    );
    let mut ca = ca.unwrap();
    let mut va = va.unwrap();
    assert_eq!(va.local_behavior().role, Role::Sender);
    let budget = NodeProbeBudget::new(0, 1);
    assert!(
        tokio::time::timeout(
            Duration::from_millis(50),
            va.execute(&v, &pair.v_gate, &budget)
        )
        .await
        .is_err()
    );
    assert!(budget.spend_peer_datagram());
    let lease = v
        .register_udp_traversal(
            hex::decode(va.round().encoded())
                .unwrap()
                .try_into()
                .unwrap(),
        )
        .await
        .unwrap();
    drop(lease);
    let (vd, cd) = tokio::join!(v.commit_data_path(), c.commit_data_path());
    let (vd, cd) = (vd.unwrap(), cd.unwrap());
    assert!(!va.confirm_success(&vd, &pair.v_gate, &analyzer));
    assert!(!ca.confirm_success(&cd, &pair.c_gate, &analyzer));
    pair.close().await;
}

#[tokio::test]
async fn scope_and_network_changes_during_delay_stop_before_submission() {
    for network_change in [false, true] {
        let pair = Pair::new().await;
        let (cs, vs) = pair.snapshots().await;
        let (c, v) = pair.connect().await;
        let analyzer = Arc::new(Mutex::new(Analyzer::new(Instant::now())));
        let (ca, va) = tokio::join!(
            nat_punch::coordinate_server(&c, &pair.c_gate, &cs, &analyzer),
            nat_punch::coordinate_client(&v, &pair.v_gate, &vs)
        );
        let mut ca = ca.unwrap();
        let mut va = va.unwrap();
        let budget = NodeProbeBudget::new(0, 1);
        let mutation = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if network_change {
                pair.v.network_change().await;
            } else {
                pair.v_gate
                    .update_authorization([*pair.v.id().as_bytes()], []);
            }
        };
        let (result, _) = tokio::join!(va.execute(&v, &pair.v_gate, &budget), mutation);
        assert!(result.is_err());
        assert!(budget.spend_peer_datagram());
        ca.finish_failure();
        va.finish_failure();
        pair.close().await;
    }
}

#[tokio::test]
async fn exhaustion_stops_full_phase_and_existing_authorized_control_remains_usable() {
    let pair = Pair::new().await;
    let (cs, vs) = pair.snapshots().await;
    let (c, v) = pair.connect().await;
    let analyzer = Arc::new(Mutex::new(Analyzer::new(Instant::now())));
    let (ca, va) = tokio::join!(
        nat_punch::coordinate_server(&c, &pair.c_gate, &cs, &analyzer),
        nat_punch::coordinate_client(&v, &pair.v_gate, &vs)
    );
    let mut ca = ca.unwrap();
    let mut va = va.unwrap();
    let result = va
        .execute(&v, &pair.v_gate, &NodeProbeBudget::new(0, 0))
        .await
        .unwrap_err();
    assert!(result.to_string().contains("budget exhausted"));
    let send = v.open_control_exchange().await.unwrap();
    send.send_message(
        iroh::endpoint::ControlMessageFormat::EndTerminated { maximum: 64 },
        b"still-control",
    )
    .await
    .unwrap();
    let reply = c.accept_control_exchange().await.unwrap();
    assert_eq!(
        reply
            .recv_message(iroh::endpoint::ControlMessageFormat::EndTerminated { maximum: 64 })
            .await
            .unwrap(),
        b"still-control"
    );
    reply
        .send_message(
            iroh::endpoint::ControlMessageFormat::EndTerminated { maximum: 64 },
            b"alive",
        )
        .await
        .unwrap();
    assert_eq!(
        send.recv_message(iroh::endpoint::ControlMessageFormat::EndTerminated { maximum: 64 })
            .await
            .unwrap(),
        b"alive"
    );
    let (vd, cd) = tokio::join!(v.commit_data_path(), c.commit_data_path());
    let (vd, cd) = (vd.unwrap(), cd.unwrap());
    assert!(!va.confirm_success(&vd, &pair.v_gate, &analyzer));
    assert!(!ca.confirm_success(&cd, &pair.c_gate, &analyzer));
    pair.close().await;
}

#[tokio::test]
async fn winner_handoff_waits_for_peer_inside_the_original_deadline_and_rechecks_scope() {
    for change_scope in [false, true] {
        let pair = Pair::new().await;
        let (cs, vs) = pair.snapshots().await;
        let (c, v) = pair.connect().await;
        let stable = (c.stable_id(), v.stable_id());
        let analyzer = Arc::new(Mutex::new(Analyzer::new(Instant::now())));
        let (ca, va) = tokio::join!(
            nat_punch::coordinate_server(&c, &pair.c_gate, &cs, &analyzer),
            nat_punch::coordinate_client(&v, &pair.v_gate, &vs)
        );
        let (mut ca, mut va) = (ca.unwrap(), va.unwrap());
        let cb = NodeProbeBudget::new(0, 1000);
        let vb = NodeProbeBudget::new(0, 1000);
        let (cr, vr) = tokio::join!(
            ca.execute(&c, &pair.c_gate, &cb),
            va.execute(&v, &pair.v_gate, &vb)
        );
        assert_eq!(cr.unwrap(), ExecutionStatus::CompleteExecution);
        assert_eq!(vr.unwrap(), ExecutionStatus::CompleteExecution);
        let started = Instant::now();
        if change_scope {
            let mutation = async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                pair.c_gate
                    .update_authorization([*pair.c.id().as_bytes()], [*pair.v.id().as_bytes()]);
            };
            let (result, _) = tokio::join!(ca.synchronize_winner(&c, &pair.c_gate), mutation);
            assert!(result.is_err());
            assert!(started.elapsed() < Duration::from_secs(1));
        } else {
            let delayed_peer = async {
                tokio::time::sleep(Duration::from_millis(3200)).await;
                va.synchronize_winner(&v, &pair.v_gate).await
            };
            let (cr, vr) = tokio::join!(ca.synchronize_winner(&c, &pair.c_gate), delayed_peer);
            cr.unwrap();
            vr.unwrap();
            assert!(started.elapsed() >= Duration::from_millis(3200));
            assert!(ca.path_wait_remaining(&c, &pair.c_gate).unwrap() < Duration::from_secs(2));
            assert_eq!((c.stable_id(), v.stable_id()), stable);
            ca.verify_winner_path(&c, &pair.c_gate).unwrap();
            va.verify_winner_path(&v, &pair.v_gate).unwrap();
            let (cd, vd) = tokio::join!(
                ca.commit_winner_path(c, &pair.c_gate),
                va.commit_winner_path(v, &pair.v_gate)
            );
            let (cd, vd) = (cd.unwrap(), vd.unwrap());
            assert_eq!(
                (cd.connection().stable_id(), vd.connection().stable_id()),
                stable
            );
            let started = Instant::now();
            let mutation = async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                pair.c.network_change().await;
            };
            let (result, _) = tokio::join!(
                ca.guard_committed(
                    &cd,
                    &pair.c_gate,
                    std::future::pending::<std::io::Result<()>>()
                ),
                mutation
            );
            assert!(result.is_err());
            assert!(started.elapsed() < Duration::from_secs(1));
        }
        ca.finish_failure();
        va.finish_failure();
        pair.close().await;
    }
}

#[tokio::test]
async fn ordinary_qad_refresh_keeps_the_physical_generation_but_real_network_change_invalidates_it()
{
    let pair = Pair::new().await;
    let sequence = pair
        .c
        .observe_udp_mapping_sequence(&pair.observers)
        .await
        .unwrap();
    let generation = pair.c.udp_observation_generation();
    let physical = pair.c.bound_sockets();
    let mut reports = pair.c.udp_observation();
    let fallback =
        MappingSnapshot::fallback_generation(pair.c.udp_network_generation(), reports.get())
            .unwrap();
    tokio::time::timeout(Duration::from_secs(40), async {
        while generation.is_current() {
            reports.updated().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert_eq!(pair.c.bound_sockets(), physical);
    assert!(
        sequence.is_current_generation(),
        "ordinary report publication is not a socket/network change"
    );
    assert!(
        fallback.is_current(),
        "ordinary report publication must not cancel a candidate-only lifecycle"
    );
    assert!(
        !sequence.is_fresh(),
        "the old predictor's two-second lifetime remains bounded"
    );
    pair.c.network_change().await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while sequence.is_current_generation() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(!fallback.is_current());
    pair.close().await;
}

#[tokio::test]
async fn low_ttl_and_unsupported_ttl_do_not_change_parallel_connection_business() {
    for degraded in [false, true] {
        let pair = Pair::new().await;
        let (cs, vs) = pair.snapshots().await;
        let (c, v) = pair.connect().await;
        let (c2, v2) = pair.connect().await;
        let (cd2, vd2) = tokio::join!(c2.commit_data_path(), v2.commit_data_path());
        let (cd2, vd2) = (cd2.unwrap(), vd2.unwrap());
        if degraded {
            pair.c.test_force_traversal_ttl_unavailable().await;
            pair.v.test_force_traversal_ttl_unavailable().await;
        }
        let analyzer = Arc::new(Mutex::new(Analyzer::new(Instant::now())));
        let (ca, va) = tokio::join!(
            nat_punch::coordinate_server(&c, &pair.c_gate, &cs, &analyzer),
            nat_punch::coordinate_client(&v, &pair.v_gate, &vs)
        );
        let mut ca = ca.unwrap();
        let mut va = va.unwrap();
        let cb = NodeProbeBudget::new(0, 200);
        let vb = NodeProbeBudget::new(0, 200);
        let business = async {
            let (send, mut recv) = vd2.connection().open_bi().await.unwrap();
            let receive = async {
                let (mut reply, mut request) = cd2.connection().accept_bi().await.unwrap();
                let data = request.read_to_end(262144).await.unwrap();
                assert_eq!(data, vec![0x63; 262144]);
                reply.write_all(b"parallel-authorized").await.unwrap();
                reply.finish().unwrap();
            };
            let transmit = async {
                let mut send = send;
                send.write_all(&vec![0x63; 262144]).await.unwrap();
                send.finish().unwrap();
                assert_eq!(recv.read_to_end(64).await.unwrap(), b"parallel-authorized");
            };
            tokio::join!(receive, transmit);
        };
        let (cr, vr, _) = tokio::join!(
            ca.execute(&c, &pair.c_gate, &cb),
            va.execute(&v, &pair.v_gate, &vb),
            business
        );
        assert_eq!(cr.unwrap(), ExecutionStatus::CompleteExecution);
        assert_eq!(vr.unwrap(), ExecutionStatus::CompleteExecution);
        assert_eq!(ca.execution_trace().unwrap().ttl_degraded > 0, degraded);
        assert_eq!(cd2.connection().remote_id(), pair.v.id());
        assert_eq!(vd2.connection().remote_id(), pair.c.id());
        pair.close().await;
    }
}

#[tokio::test]
async fn stale_round_and_foreign_connection_authentication_cannot_select_a_winner() {
    use miopunch_native_probe::nat_scheduler;
    let pair = Pair::new().await;
    let (c, v) = pair.connect().await;
    let mut cl = c.register_udp_traversal([3; 16]).await.unwrap();
    let mut old = v.register_udp_traversal([2; 16]).await.unwrap();
    let cp = raw_phase(Role::Receiver, 500, &[]);
    let vp = raw_phase(Role::Sender, 0, &pair.c.bound_sockets());
    let ca = raw_auth(&pair.c, &pair.v);
    let va = raw_auth(&pair.v, &pair.c);
    let cb = NodeProbeBudget::new(0, 3);
    let vb = NodeProbeBudget::new(0, 2);
    let receiver = tokio::time::timeout(
        Duration::from_millis(100),
        nat_scheduler::run(&cp, &mut cl, &ca, &cb, &|| Ok(())),
    );
    let stale = tokio::time::timeout(
        Duration::from_millis(100),
        nat_scheduler::run(&vp, &mut old, &va, &vb, &|| Ok(())),
    );
    let (cr, vr) = tokio::join!(receiver, stale);
    assert!(cr.is_err());
    assert!(vr.is_err());
    drop(old);
    let (_c2, v2) = pair.connect().await;
    let mut foreign = v2.register_udp_traversal([3; 16]).await.unwrap();
    let mut foreign_auth = va.clone();
    foreign_auth.key = [64; 32];
    let vb = NodeProbeBudget::new(0, 2);
    let (cr, vr) = tokio::join!(
        tokio::time::timeout(
            Duration::from_millis(100),
            nat_scheduler::run(&cp, &mut cl, &ca, &cb, &|| Ok(()))
        ),
        tokio::time::timeout(
            Duration::from_millis(100),
            nat_scheduler::run(&vp, &mut foreign, &foreign_auth, &vb, &|| Ok(()))
        )
    );
    assert!(cr.is_err());
    assert!(vr.is_err());
    drop(foreign);
    // Correct connection/round authentication still wins before the delay.
    let mut valid = v.register_udp_traversal([3; 16]).await.unwrap();
    let vb = NodeProbeBudget::new(0, 2);
    let (cr, vr) = tokio::join!(
        nat_scheduler::run(&cp, &mut cl, &ca, &cb, &|| Ok(())),
        nat_scheduler::run(&vp, &mut valid, &va, &vb, &|| Ok(()))
    );
    assert!(cr.unwrap().0.before_probe);
    assert!(vr.unwrap().0.response);
    assert!(!cb.spend_peer_datagram());
    pair.close().await;
}

#[tokio::test]
async fn fallback_retains_a_freshly_acquired_candidate_for_its_phase() {
    let pair = Pair::new().await;
    let original = UdpObservation::Fresh {
        addresses: pair.c.bound_sockets(),
        observed_at: Instant::now() - Duration::from_secs(39),
        valid_until: Instant::now() + Duration::from_millis(100),
        mapping_varies_by_dest: None,
        validity: pair.c.udp_observation_generation(),
    };
    let snapshot =
        MappingSnapshot::fallback_generation(pair.c.udp_network_generation(), original.clone())
            .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(!original.is_fresh());
    assert!(snapshot.is_current());
    pair.c.network_change().await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while snapshot.is_current() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    pair.close().await;
}

/// Explicit resource measurement, separate from controlled NAT probability trials.
/// The runner enforces raw-only INPUT drops in a disposable loopback namespace.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "run only through legacy_nat_multi_socket_cpu.py in its isolated raw-drop fixture"]
async fn isolated_functional_cpu_cost_supplement() {
    assert_eq!(
        std::env::var("MIOPUNCH_CPU_RAW_DROP_CONFIRMED").as_deref(),
        Ok("1")
    );
    let mode: u8 = std::env::var("MIOPUNCH_CPU_MODE").unwrap().parse().unwrap();
    assert!([0, 2, 4].contains(&mode));
    let pair = Pair::new().await;
    let mut cs = pair
        .c
        .observe_udp_mapping_sequence(&pair.observers)
        .await
        .unwrap();
    let mut vs = pair
        .v
        .observe_udp_mapping_sequence(&pair.observers)
        .await
        .unwrap();
    // These explicitly synthetic functional observations select the normal
    // old strategy. They are never presented as measured NAT mappings.
    for (index, sample) in cs.samples.iter_mut().enumerate() {
        sample.mapped = (
            Ipv4Addr::new(127, 0, 0, 3),
            43000 + u16::from(mode == 4) * index as u16,
        )
            .into();
    }
    for (index, sample) in vs.samples.iter_mut().enumerate() {
        sample.mapped = (
            Ipv4Addr::new(127, 0, 0, 4),
            50000 + u16::from(mode != 0) * index as u16 * 12000,
        )
            .into();
    }
    let cs = MappingSnapshot::from_sequence(cs).unwrap();
    let vs = MappingSnapshot::from_sequence(vs).unwrap();
    let (c, v) = pair.connect().await;
    let stable = (c.stable_id(), v.stable_id());
    let analyzer = Arc::new(Mutex::new(Analyzer::new(Instant::now())));
    let (ca, va) = tokio::join!(
        nat_punch::coordinate_server(&c, &pair.c_gate, &cs, &analyzer),
        nat_punch::coordinate_client(&v, &pair.v_gate, &vs)
    );
    let mut ca = ca.unwrap();
    let mut va = va.unwrap();
    assert_eq!(ca.plan().id, BehaviorId { mode, index: 0 });
    assert_eq!(ca.plan(), va.plan());
    let before = std::fs::read_dir("/proc/self/fd").unwrap().count();
    let budget = miopunch_native_probe::nat_scheduler::DEFAULT_LEGACY_PEER_ALLOWANCE;
    let cb = NodeProbeBudget::new(0, budget);
    let vb = NodeProbeBudget::new(0, budget);
    let started = Instant::now();
    let (cr, vr) = tokio::join!(
        ca.execute(&c, &pair.c_gate, &cb),
        va.execute(&v, &pair.v_gate, &vb)
    );
    for result in [cr, vr] {
        let error = result.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert_eq!(error.to_string(), "legacy NAT phase budget elapsed");
    }
    let expected_ms = if mode == 0 { 5200 } else { 38000 };
    assert!(started.elapsed() >= Duration::from_millis(expected_ms - 50));
    assert!(started.elapsed() < Duration::from_millis(expected_ms + 1000));
    let mut probes = 0;
    for (side, attempt) in [("c", &ca), ("v", &va)] {
        let trace = attempt.execution_trace().unwrap();
        let receiver = attempt.local_behavior().role == Role::Receiver;
        assert_eq!(
            trace.listener_attempted,
            if mode != 0 && receiver { 256 } else { 0 }
        );
        assert_eq!(trace.listener_bound, trace.listener_attempted);
        assert_eq!(trace.listener_failed, 0);
        assert_eq!(
            trace.random_iterations,
            if mode != 0 && !receiver { 1000 } else { 0 }
        );
        assert_eq!(trace.responses, 0);
        assert_eq!(trace.send_errors, 0);
        assert_eq!(trace.rejected_receipts, 0);
        probes += trace.probes;
        println!(
            "CPU_FUNCTIONAL_TRACE mode={mode} side={side} stable_id={} probes={} responses={} random_iterations={} random_draws={} random_skipped={} listener_attempted={} listener_bound={} listener_failed={} send_errors={} send_aborted={}",
            if side == "c" { stable.0 } else { stable.1 },
            trace.probes,
            trace.responses,
            trace.random_iterations,
            trace.random_draws,
            trace.random_skipped,
            trace.listener_attempted,
            trace.listener_bound,
            trace.listener_failed,
            trace.send_errors,
            trace.send_aborted
        );
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let after = std::fs::read_dir("/proc/self/fd").unwrap().count();
    assert_eq!(
        after, before,
        "temporary physical listeners were not released"
    );
    let send = v.open_control_exchange().await.unwrap();
    send.send_message(
        iroh::endpoint::ControlMessageFormat::EndTerminated { maximum: 64 },
        b"CPU-CONTROL-PING",
    )
    .await
    .unwrap();
    let reply = c.accept_control_exchange().await.unwrap();
    assert_eq!(
        reply
            .recv_message(iroh::endpoint::ControlMessageFormat::EndTerminated { maximum: 64 })
            .await
            .unwrap(),
        b"CPU-CONTROL-PING"
    );
    reply
        .send_message(
            iroh::endpoint::ControlMessageFormat::EndTerminated { maximum: 64 },
            b"CPU-CONTROL-PONG",
        )
        .await
        .unwrap();
    assert_eq!(
        send.recv_message(iroh::endpoint::ControlMessageFormat::EndTerminated { maximum: 64 })
            .await
            .unwrap(),
        b"CPU-CONTROL-PONG"
    );
    assert_eq!((c.stable_id(), v.stable_id()), stable);
    assert!(c.may_upgrade_member_path(&pair.c_gate));
    assert!(v.may_upgrade_member_path(&pair.v_gate));
    println!(
        "CPU_FUNCTIONAL_COMPLETE mode={mode} probes={probes} elapsed_ms={} fd_before={before} fd_after={after} control_alive=true c_stable_id={} v_stable_id={}",
        started.elapsed().as_millis(),
        stable.0,
        stable.1
    );
    pair.close().await;
}
