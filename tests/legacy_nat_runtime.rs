//! Real QAD/TLS/control/UDP seams for the legacy selector, not NAT-rate evidence.
use std::{
    net::Ipv4Addr,
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
};

#[tokio::test]
async fn formal_entry_observes_selects_executes_and_confirms_original_business() {
    use miopunch_native_probe::nat_connection::{NatConnector, UdpPolicy};
    let pair = Pair::new().await;
    let (c, v) = pair.connect().await;
    let ids = (c.stable_id(), v.stable_id());
    let policy = UdpPolicy::default();
    let cn = NatConnector::new(Some(pair.observers.clone()), policy.clone());
    let vn = NatConnector::new(Some(pair.observers.clone()), policy);
    let (ca, va) = tokio::join!(cn.prepare(&c, &pair.c_gate), vn.prepare(&v, &pair.v_gate));
    let (mut ca, mut va) = (ca.unwrap(), va.unwrap());
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
    pair.close().await;
}

#[tokio::test]
async fn formal_entry_without_s_uses_local_assisted_basic_attempt_and_original_business() {
    use miopunch_native_probe::nat_connection::{NatConnector, UdpPolicy};
    let pair = Pair::without_observers().await;
    assert!(!pair.c.udp_observation().get().is_fresh());
    assert!(!pair.v.udp_observation().get().is_fresh());
    let (c, v) = pair.connect().await;
    let ids = (c.stable_id(), v.stable_id());
    let cn = NatConnector::new(None, UdpPolicy::default());
    let vn = NatConnector::new(None, UdpPolicy::default());
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
    use miopunch_native_probe::nat_connection::{NatConnector, UdpPolicy};
    let pair = Pair::new().await;
    let (c, v) = pair.connect().await;
    let cn = NatConnector::new(
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
    cn.set_udp_enabled(true);
    let vn = NatConnector::new(Some(pair.observers.clone()), UdpPolicy::default());
    let (ca, va) = tokio::join!(cn.prepare(&c, &pair.c_gate), vn.prepare(&v, &pair.v_gate));
    let (mut ca, mut va) = (ca.unwrap(), va.unwrap());
    let low = NatConnector::new(
        None,
        UdpPolicy {
            peer_datagrams: 1,
            ..Default::default()
        },
    );
    let error = low.execute(&mut ca, &c, &pair.c_gate).await.unwrap_err();
    assert_eq!(error.to_string(), "NAT complete-plan budget insufficient");
    assert!(ca.execution_trace().is_none());
    assert!(ca.local_winner().is_none());
    let run = cn.execute(&mut va, &v, &pair.v_gate);
    let disable = async {
        tokio::time::sleep(Duration::from_millis(30)).await;
        cn.set_udp_enabled(false);
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
    pair.close().await;
}

#[tokio::test]
async fn formal_direct_candidates_precede_selector_and_keep_original_identity() {
    use miopunch_native_probe::nat_connection::{NatConnector, UdpPolicy};
    let pair = Pair::new().await;
    let (c, v) = pair.connect().await;
    let ids = (c.stable_id(), v.stable_id());
    let cn = NatConnector::new(Some(pair.observers.clone()), UdpPolicy::default());
    let vn = NatConnector::new(Some(pair.observers.clone()), UdpPolicy::default());
    let (cd, vd) = tokio::join!(cn.upgrade(c, &pair.c_gate), vn.upgrade(v, &pair.v_gate));
    let (mut cd, mut vd) = (cd.unwrap(), vd.unwrap());
    assert!(cd.attempt.is_none() && vd.attempt.is_none());
    assert!(cd.direct.is_some() && vd.direct.is_some());
    assert_eq!(
        (
            cd.data.connection().stable_id(),
            vd.data.connection().stable_id()
        ),
        ids
    );
    let (cr, vr) = tokio::join!(
        cn.exchange_upgrade_business(&mut cd, &pair.c_gate, b"accepting-direct-business"),
        vn.exchange_upgrade_business(&mut vd, &pair.v_gate, b"visiting-direct-business")
    );
    assert_eq!(cr.unwrap(), b"visiting-direct-business");
    assert_eq!(vr.unwrap(), b"accepting-direct-business");
    pair.close().await;
}

#[tokio::test]
async fn formal_disabled_udp_blocks_business_before_stream_creation() {
    use miopunch_native_probe::nat_connection::{NatConnector, UdpPolicy};
    let pair = Pair::new().await;
    let (c, v) = pair.connect().await;
    let cn = NatConnector::new(Some(pair.observers.clone()), UdpPolicy::default());
    let vn = NatConnector::new(Some(pair.observers.clone()), UdpPolicy::default());
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
    cn.set_udp_enabled(false);
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
    use miopunch_native_probe::nat_connection::{NatConnector, UdpPolicy};
    let pair = Pair::with_bind(std::net::Ipv6Addr::LOCALHOST.into()).await;
    let (c, v) = pair.connect().await;
    let ids = (c.stable_id(), v.stable_id());
    let cn = NatConnector::new(None, UdpPolicy::default());
    let vn = NatConnector::new(None, UdpPolicy::default());
    let (cu, vu) = tokio::join!(cn.upgrade(c, &pair.c_gate), vn.upgrade(v, &pair.v_gate));
    let (mut cu, mut vu) = (cu.unwrap(), vu.unwrap());
    assert!(cu.direct.unwrap().is_ipv6() && vu.direct.unwrap().is_ipv6());
    assert!(cu.attempt.is_none() && vu.attempt.is_none());
    assert_eq!(
        (
            cu.data.connection().stable_id(),
            vu.data.connection().stable_id()
        ),
        ids
    );
    let (c, v) = tokio::join!(
        cn.exchange_upgrade_business(&mut cu, &pair.c_gate, b"ipv6-acceptor"),
        vn.exchange_upgrade_business(&mut vu, &pair.v_gate, b"ipv6-visitor")
    );
    assert_eq!(c.unwrap(), b"ipv6-visitor");
    assert_eq!(v.unwrap(), b"ipv6-acceptor");
    pair.close().await;
}

struct Pair {
    c: Endpoint,
    v: Endpoint,
    c_gate: AdmissionGate,
    v_gate: AdmissionGate,
    observers: RelayMap,
    _servers: Vec<server::Server>,
}

impl Pair {
    async fn new() -> Self {
        Self::with_bind(Ipv4Addr::LOCALHOST.into()).await
    }

    async fn with_bind(bind: std::net::IpAddr) -> Self {
        Self::with_observers(bind, true).await
    }

    async fn without_observers() -> Self {
        Self::with_observers(Ipv4Addr::LOCALHOST.into(), false).await
    }

    async fn with_observers(bind: std::net::IpAddr, observe: bool) -> Self {
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
        let v_key = SecretKey::generate();
        let scope = AdmissionScope::new([62; 32], 1);
        let c_gate = AdmissionGate::new(
            c_key.public(),
            scope.clone(),
            Some(AdmissionSecret::from_saved([42; 32])),
        );
        let v_gate = AdmissionGate::new(
            v_key.public(),
            scope,
            Some(AdmissionSecret::from_saved([42; 32])),
        );
        let members = [*c_key.public().as_bytes(), *v_key.public().as_bytes()];
        for gate in [&c_gate, &v_gate] {
            gate.update_authorization(members, []);
        }
        let mut endpoints = Vec::new();
        for (key, gate) in [(c_key, &c_gate), (v_key, &v_gate)] {
            let endpoint = gate
                .configure(Endpoint::builder(presets::Minimal))
                .secret_key(key)
                .alpns(vec![CONTROL_ALPN.to_vec()])
                .clear_ip_transports()
                .bind_addr((bind, 0))
                .unwrap()
                .clear_relay_transports()
                .observation_servers(observers.clone())
                .ca_tls_config(CaTlsConfig::custom_roots(certs.clone()))
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
            _servers: servers,
        }
    }

    async fn connect(&self) -> (ControlSession, ControlSession) {
        let target = EndpointAddr::new(self.c.id())
            .with_addrs(self.c.bound_sockets().into_iter().map(TransportAddr::Ip));
        let (c, v) = tokio::join!(
            ControlSession::accept(&self.c),
            ControlSession::connect(&self.v, target, CONTROL_ALPN)
        );
        (c.unwrap(), v.unwrap())
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
    let c_data = c.commit_data_path().await.unwrap();
    let v_data = v.commit_data_path().await.unwrap();
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
    let assisted = MappingSnapshot::fallback(&pair.c, UdpObservation::Disabled)
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
        MappingSnapshot::fallback(
            &pair.c,
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

async fn send_json(send: &mut iroh::endpoint::SendStream, value: serde_json::Value) {
    send.write_all(&serde_json::to_vec(&value).unwrap())
        .await
        .unwrap();
    send.finish().unwrap();
}

#[tokio::test]
async fn wrong_network_and_old_round_are_rejected_before_snapshot_disclosure_or_selection() {
    let pair = Pair::new().await;
    let (snapshot, _) = pair.snapshots().await;
    for wrong_network in [true, false] {
        let (c, v) = pair.connect().await;
        let analyzer = Arc::new(Mutex::new(Analyzer::new(Instant::now())));
        let malicious_visitor = async {
            let (mut send, mut recv) = v.open_control_bi().await.unwrap();
            send_json(&mut send,serde_json::json!({"Hello":{"network":vec![if wrong_network {99} else {62};32],"generation":1}})).await;
            if wrong_network {
                return;
            }
            let challenge: serde_json::Value =
                serde_json::from_slice(&recv.read_to_end(16384).await.unwrap()).unwrap();
            let mut round = challenge["Challenge"]["round"].as_array().unwrap().clone();
            round[0] = serde_json::json!(round[0].as_u64().unwrap() ^ 1);
            let (mut send, _) = v.open_control_bi().await.unwrap();
            send_json(&mut send,serde_json::json!({"Offer":{"round":round,"snapshot":{"mapped":vec![pair.v.bound_sockets()[0].to_string();2],"local_ips":[],"assisted":[]}}})).await;
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
    let snapshot = MappingSnapshot::fallback(&pair.c, UdpObservation::Disabled).unwrap();
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
    let c_empty = MappingSnapshot::fallback(&pair.c, UdpObservation::Disabled).unwrap();
    let v_empty = MappingSnapshot::fallback(&pair.v, UdpObservation::Disabled).unwrap();
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
    let c_snapshot = MappingSnapshot::fallback(&pair.c, UdpObservation::Disabled)
        .unwrap()
        .with_assisted(pair.c.bound_sockets())
        .unwrap();
    let v_snapshot = MappingSnapshot::fallback(&pair.v, UdpObservation::Disabled).unwrap();
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
    let c_data = c.commit_data_path().await.unwrap();
    let v_data = v.commit_data_path().await.unwrap();
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
        let c_data = c.commit_data_path().await.unwrap();
        let v_data = v.commit_data_path().await.unwrap();
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
    let cd = c.commit_data_path().await.unwrap();
    let vd = v.commit_data_path().await.unwrap();
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
    local: &Endpoint,
    remote: &Endpoint,
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
    let vd = v.commit_data_path().await.unwrap();
    let cd = c.commit_data_path().await.unwrap();
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
    let (mut send, mut recv) = v.open_control_bi().await.unwrap();
    send.write_all(b"still-control").await.unwrap();
    send.finish().unwrap();
    let (mut reply, mut request) = c.accept_control_bi().await.unwrap();
    assert_eq!(request.read_to_end(64).await.unwrap(), b"still-control");
    reply.write_all(b"alive").await.unwrap();
    reply.finish().unwrap();
    assert_eq!(recv.read_to_end(64).await.unwrap(), b"alive");
    let vd = v.commit_data_path().await.unwrap();
    let cd = c.commit_data_path().await.unwrap();
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
            let cd = ca.commit_winner_path(c, &pair.c_gate).await.unwrap();
            let vd = va.commit_winner_path(v, &pair.v_gate).await.unwrap();
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
    let fallback = MappingSnapshot::fallback(&pair.c, reports.get()).unwrap();
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
        let cd2 = c2.commit_data_path().await.unwrap();
        let vd2 = v2.commit_data_path().await.unwrap();
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
    let snapshot = MappingSnapshot::fallback(&pair.c, original.clone()).unwrap();
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
    let (mut send, mut recv) = v.open_control_bi().await.unwrap();
    send.write_all(b"CPU-CONTROL-PING").await.unwrap();
    send.finish().unwrap();
    let (mut reply, mut request) = c.accept_control_bi().await.unwrap();
    assert_eq!(request.read_to_end(64).await.unwrap(), b"CPU-CONTROL-PING");
    reply.write_all(b"CPU-CONTROL-PONG").await.unwrap();
    reply.finish().unwrap();
    assert_eq!(recv.read_to_end(64).await.unwrap(), b"CPU-CONTROL-PONG");
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
