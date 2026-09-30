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
        let (certs, tls) = server::testing::self_signed_tls_certs_and_config();
        let mut servers = Vec::new();
        let mut configs = Vec::new();
        for _ in 0..2 {
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
                .bind_addr((Ipv4Addr::LOCALHOST, 0))
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
