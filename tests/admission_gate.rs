//! MIOPUNCH-29: real TLS connections enter only their permitted protocol phase.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use iroh::address_lookup::memory::MemoryLookup;
use iroh::endpoint::{Connection, EndpointHooks, presets};
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::{Endpoint, EndpointAddr};
use iroh_base::SecretKey;
use iroh_base::TransportAddr;
use miopunch_native_probe::admission_gate::{
    ADMISSION_REQUEST_ALPN, AdmissionGate, CONTROL_ALPN, ProtocolScope,
};
use miopunch_native_probe::admission_proof::{
    AdmissionScope, AdmissionSecret, ProofError, prove_server,
};
use miopunch_native_probe::genesis_recovery::RECOVERY_ALPN;
use miopunch_native_probe::member_bootstrap::MEMBER_BOOTSTRAP_ALPN;
use miopunch_native_probe::mqtt_transport::{MqttTransport, Traffic};
use miopunch_native_probe::nat_mapping::{
    MappingPattern, NodeProbeBudget, choose_regular_pair, choose_single_sequential,
};
use miopunch_native_probe::native_network::NativeNetwork;
use miopunch_native_probe::tcp_transport::{TcpOnly, TcpTransport, Traffic as TcpTraffic};
use p2panda_core::Hash;
use p2panda_core::SigningKey;
use p2panda_net::iroh_endpoint::IrohConfig;
use p2panda_net::{AddressBook, Endpoint as PandaEndpoint};

const NETWORK: [u8; 32] = [23; 32];
const SECRET: [u8; 32] = [42; 32];
const DISCOVERY: &[u8] = b"p2panda/confidential_discovery/v1";
const GOSSIP: &[u8] = b"/iroh-gossip/1";
const LOG_SYNC: &[u8] = b"p2panda/log_sync/v1";
const BUSINESS: &[u8] = b"miopunch/test-business/1";

fn mixed(protocol: &[u8]) -> Vec<u8> {
    let mut bytes = protocol.to_vec();
    bytes.extend_from_slice(&NETWORK);
    Hash::digest(&bytes).as_bytes().to_vec()
}

fn exporter(connection: &Connection) -> [u8; 32] {
    let mut bytes = [0; 32];
    connection
        .export_keying_material(&mut bytes, b"EXPORTER-Channel-Binding", b"")
        .unwrap();
    bytes
}

struct Pair {
    client: Endpoint,
    server: Endpoint,
    client_gate: AdmissionGate,
    server_gate: AdmissionGate,
}

impl Pair {
    async fn new(client_secret: Option<[u8; 32]>, server_secret: [u8; 32], member: bool) -> Self {
        let client_key = SecretKey::generate();
        let server_key = SecretKey::generate();
        let scope = AdmissionScope::new(NETWORK, 0);
        let client_gate = AdmissionGate::new(
            client_key.public(),
            scope.clone(),
            client_secret.map(AdmissionSecret::from_saved),
        );
        let server_gate = AdmissionGate::new(
            server_key.public(),
            scope,
            Some(AdmissionSecret::from_saved(server_secret)),
        );
        for gate in [&client_gate, &server_gate] {
            gate.register_p2panda_defaults(NETWORK);
            gate.register_protocol(BUSINESS, ProtocolScope::Member);
        }
        if member {
            let members = [
                *client_key.public().as_bytes(),
                *server_key.public().as_bytes(),
            ];
            client_gate.update_authorization(members, []);
            server_gate.update_authorization(members, []);
        } else {
            server_gate.update_authorization([*server_key.public().as_bytes()], []);
        }
        let alpns = [
            ADMISSION_REQUEST_ALPN.to_vec(),
            RECOVERY_ALPN.to_vec(),
            MEMBER_BOOTSTRAP_ALPN.to_vec(),
            CONTROL_ALPN.to_vec(),
            mixed(DISCOVERY),
            mixed(GOSSIP),
            mixed(LOG_SYNC),
            BUSINESS.to_vec(),
        ];
        let server = server_gate
            .configure(Endpoint::builder(presets::Minimal))
            .secret_key(server_key)
            .alpns(alpns.to_vec())
            .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .unwrap()
            .bind()
            .await
            .unwrap();
        let client = client_gate
            .configure(Endpoint::builder(presets::Minimal))
            .secret_key(client_key)
            .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .unwrap()
            .bind()
            .await
            .unwrap();
        Self {
            client,
            server,
            client_gate,
            server_gate,
        }
    }

    async fn connect(&self, alpn: &[u8]) -> (Option<Connection>, Option<Connection>) {
        let target = EndpointAddr::new(self.server.id()).with_addrs(
            self.server
                .addr()
                .ip_addrs()
                .copied()
                .map(iroh_base::TransportAddr::Ip),
        );
        let (outgoing, incoming) = tokio::time::timeout(Duration::from_secs(9), async {
            tokio::join!(self.client.connect(target, alpn), async {
                self.server.accept().await.unwrap().await
            })
        })
        .await
        .expect("bounded connection attempt");
        (outgoing.ok(), incoming.ok())
    }

    async fn close(self) {
        self.client.close().await;
        self.server.close().await;
    }
}

#[tokio::test]
async fn admitted_control_path_upgrade_requires_current_membership() {
    let pair = Pair::new(Some(SECRET), SECRET, true).await;
    let (client, server) = pair.connect(CONTROL_ALPN).await;
    let (client, server) = (client.unwrap(), server.unwrap());
    assert!(pair.client_gate.allows_member_path_upgrade(&client));
    assert!(pair.server_gate.allows_member_path_upgrade(&server));
    assert!(!pair.server_gate.allows(&server, ProtocolScope::Member));

    pair.server_gate.update_authorization(
        [*pair.server.id().as_bytes()],
        [*pair.client.id().as_bytes()],
    );
    assert!(!pair.server_gate.allows_member_path_upgrade(&server));
    tokio::time::timeout(Duration::from_secs(2), server.closed())
        .await
        .unwrap();
    pair.close().await;

    let pair = Pair::new(Some(SECRET), SECRET, false).await;
    let (client, server) = pair.connect(CONTROL_ALPN).await;
    let (client, server) = (client.unwrap(), server.unwrap());
    assert!(!pair.client_gate.allows_member_path_upgrade(&client));
    assert!(!pair.server_gate.allows_member_path_upgrade(&server));
    pair.close().await;
}

#[tokio::test]
async fn no_secret_applicant_only_reaches_the_restricted_application_protocol() {
    let pair = Pair::new(None, SECRET, false).await;
    let (client, server) = pair.connect(ADMISSION_REQUEST_ALPN).await;
    let client = client.expect("first application allowed without a long-lived secret");
    let server = server.unwrap();
    assert_eq!(client.remote_id(), pair.server.id());
    assert_eq!(server.remote_id(), pair.client.id());
    assert!(
        pair.server_gate
            .allows(&server, ProtocolScope::AdmissionRequest)
    );
    assert!(!pair.server_gate.allows(&server, ProtocolScope::Member));
    assert_eq!(server.test_local_qnt_candidate_count(), 0);

    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"APPLY").await.unwrap();
    send.finish().unwrap();
    let (_, mut recv) = server.accept_bi().await.unwrap();
    assert_eq!(recv.read_to_end(16).await.unwrap(), b"APPLY");

    for protocol in [DISCOVERY, GOSSIP, LOG_SYNC] {
        let (out, incoming) = pair.connect(&mixed(protocol)).await;
        assert!(
            out.is_none() && incoming.is_none(),
            "unexpected access: {protocol:?}"
        );
    }
    let (out, incoming) = pair.connect(BUSINESS).await;
    assert!(out.is_none() && incoming.is_none());
    pair.server_gate.update_authorization(
        [*pair.server.id().as_bytes()],
        [*pair.client.id().as_bytes()],
    );
    tokio::time::timeout(Duration::from_secs(2), server.closed())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), client.closed())
        .await
        .unwrap();
    pair.close().await;
}

#[tokio::test]
async fn secret_holder_without_membership_gets_bootstrap_but_no_normal_protocol() {
    let pair = Pair::new(Some(SECRET), SECRET, false).await;
    let (client, server) = pair.connect(MEMBER_BOOTSTRAP_ALPN).await;
    let client = client.expect("scoped proof allows bootstrap");
    let server = server.unwrap();
    assert!(pair.server_gate.allows(&server, ProtocolScope::Bootstrap));
    assert!(!pair.server_gate.allows(&server, ProtocolScope::Member));
    let fact = pair
        .server_gate
        .proof_for(&server)
        .expect("history handler gets same-connection proof");
    assert!(fact.applies_to(&server, &AdmissionScope::new(NETWORK, 0)));
    assert!(matches!(
        prove_server(
            &server,
            &AdmissionScope::new(NETWORK, 0),
            &AdmissionSecret::from_saved(SECRET),
        )
        .await,
        Err(ProofError::AlreadyAttempted)
    ));
    assert_eq!(client.test_local_qnt_candidate_count(), 0);
    assert_eq!(server.test_local_qnt_candidate_count(), 0);
    for protocol in [DISCOVERY, GOSSIP, LOG_SYNC] {
        let (out, incoming) = pair.connect(&mixed(protocol)).await;
        assert!(
            out.is_none() && incoming.is_none(),
            "unexpected access: {protocol:?}"
        );
    }
    client.close(0u32.into(), b"done");
    server.close(0u32.into(), b"done");
    pair.close().await;
}

#[tokio::test]
async fn verified_members_reach_every_normal_protocol_and_revocation_closes_existing_sessions() {
    let pair = Pair::new(Some(SECRET), SECRET, true).await;
    let mut connections = Vec::new();
    for alpn in [
        mixed(DISCOVERY),
        mixed(GOSSIP),
        mixed(LOG_SYNC),
        BUSINESS.to_vec(),
    ] {
        let (client, server) = pair.connect(&alpn).await;
        let client = client.expect("member connection allowed");
        let server = server.unwrap();
        assert!(pair.server_gate.allows(&server, ProtocolScope::Member));
        assert!(pair.client_gate.allows(&client, ProtocolScope::Member));
        assert!(server.test_local_qnt_candidate_count() > 0);
        connections.push((client, server));
    }
    let client_id = *pair.client.id().as_bytes();
    pair.server_gate
        .update_authorization([*pair.server.id().as_bytes()], [client_id]);
    for (client, server) in &connections {
        tokio::time::timeout(Duration::from_secs(2), server.closed())
            .await
            .unwrap();
        assert!(!pair.server_gate.allows(server, ProtocolScope::Member));
        tokio::time::timeout(Duration::from_secs(2), client.closed())
            .await
            .unwrap();
    }
    let (out, incoming) = pair.connect(BUSINESS).await;
    assert!(out.is_none() && incoming.is_none());
    pair.close().await;
}

#[tokio::test]
async fn proof_is_bound_to_each_connection_and_credential_generation() {
    let pair = Pair::new(Some(SECRET), SECRET, true).await;
    let ((one_out, one_in), (two_out, two_in)) =
        tokio::join!(pair.connect(BUSINESS), pair.connect(BUSINESS));
    let (one_out, two_out) = (one_out.unwrap(), two_out.unwrap());
    let (incoming_a, incoming_b) = (one_in.unwrap(), two_in.unwrap());
    let (one_in, two_in) = if exporter(&one_out) == exporter(&incoming_a) {
        (incoming_a, incoming_b)
    } else {
        assert_eq!(exporter(&one_out), exporter(&incoming_b));
        (incoming_b, incoming_a)
    };
    assert_ne!(one_in.stable_id(), two_in.stable_id());
    assert!(pair.server_gate.allows(&one_in, ProtocolScope::Member));
    assert!(pair.server_gate.allows(&two_in, ProtocolScope::Member));
    one_out.close(0u32.into(), b"first ended");
    tokio::time::timeout(Duration::from_secs(2), one_in.closed())
        .await
        .unwrap();
    assert!(pair.server_gate.allows(&two_in, ProtocolScope::Member));

    pair.server_gate.rotate_secret(
        AdmissionScope::new(NETWORK, 1),
        AdmissionSecret::from_saved([43; 32]),
    );
    tokio::time::timeout(Duration::from_secs(2), two_in.closed())
        .await
        .unwrap();
    assert!(!pair.server_gate.allows(&two_in, ProtocolScope::Member));
    let (out, incoming) = pair.connect(BUSINESS).await;
    assert!(out.is_none() && incoming.is_none());
    two_out.close(0u32.into(), b"done");
    pair.close().await;
}

#[tokio::test]
async fn a_wrong_secret_cannot_be_reused_from_another_member_connection() {
    let pair = Pair::new(Some([99; 32]), SECRET, true).await;
    let (out, incoming) = pair.connect(BUSINESS).await;
    assert!(out.is_none() && incoming.is_none());
    pair.close().await;
}

#[tokio::test]
async fn stale_member_view_does_not_return_an_outgoing_business_handle() {
    let pair = Pair::new(Some(SECRET), SECRET, true).await;
    pair.server_gate
        .update_authorization([*pair.server.id().as_bytes()], []);
    let (out, incoming) = pair.connect(BUSINESS).await;
    assert!(out.is_none() && incoming.is_none());
    pair.close().await;
}

#[tokio::test]
async fn genesis_recovery_stays_restricted_and_early_handles_are_unavailable() {
    let pair = Pair::new(None, SECRET, false).await;
    let (client, server) = pair.connect(RECOVERY_ALPN).await;
    let client = client.unwrap();
    let server = server.unwrap();
    assert!(
        pair.server_gate
            .allows(&server, ProtocolScope::GenesisRecovery)
    );
    assert!(!pair.server_gate.allows(&server, ProtocolScope::Member));
    assert_eq!(server.test_local_qnt_candidate_count(), 0);
    client.close(0u32.into(), b"done");
    server.close(0u32.into(), b"done");

    let target = EndpointAddr::new(pair.server.id()).with_addrs(
        pair.server
            .addr()
            .ip_addrs()
            .copied()
            .map(iroh_base::TransportAddr::Ip),
    );
    let connecting = pair
        .client
        .connect_with_opts(target, ADMISSION_REQUEST_ALPN, Default::default())
        .await
        .unwrap();
    assert!(connecting.into_0rtt().is_err());
    let accepting = pair.server.accept().await.unwrap().accept().unwrap();
    assert!(accepting.into_0rtt().is_err());
    pair.close().await;
}

#[tokio::test]
async fn the_persisted_native_network_installs_the_same_gate() {
    let temp = tempfile::tempdir().unwrap();
    let network = NativeNetwork::create(temp.path().join("network"))
        .await
        .unwrap();
    let status = network.status();
    let server = network.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    server.set_alpns(vec![ADMISSION_REQUEST_ALPN.to_vec(), BUSINESS.to_vec()]);
    network
        .admission_gate()
        .register_protocol(BUSINESS, ProtocolScope::Member);

    let applicant_key = SecretKey::generate();
    let applicant_gate = AdmissionGate::new(
        applicant_key.public(),
        AdmissionScope::new(*status.anchor.root.as_bytes(), 0),
        None,
    );
    applicant_gate.register_protocol(BUSINESS, ProtocolScope::Member);
    let applicant = applicant_gate
        .configure(Endpoint::builder(presets::Minimal))
        .secret_key(applicant_key)
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .bind()
        .await
        .unwrap();
    let target = EndpointAddr::new(server.id()).with_addrs(
        server
            .addr()
            .ip_addrs()
            .copied()
            .map(iroh_base::TransportAddr::Ip),
    );
    let (out, incoming) = tokio::time::timeout(Duration::from_secs(9), async {
        tokio::join!(
            applicant.connect(target.clone(), ADMISSION_REQUEST_ALPN),
            async { server.accept().await.unwrap().await }
        )
    })
    .await
    .unwrap();
    let (out, incoming) = (out.unwrap(), incoming.unwrap());
    assert!(
        network
            .admission_gate()
            .allows(&incoming, ProtocolScope::AdmissionRequest)
    );
    assert_eq!(incoming.test_local_qnt_candidate_count(), 0);
    out.close(0u32.into(), b"done");
    incoming.close(0u32.into(), b"done");
    let (out, incoming) = tokio::time::timeout(Duration::from_secs(9), async {
        tokio::join!(applicant.connect(target, BUSINESS), async {
            server.accept().await.unwrap().await
        })
    })
    .await
    .unwrap();
    assert!(out.is_err() && incoming.is_err());
    applicant.close().await;
    server.close().await;
    network.close().await;
}

#[derive(Debug)]
struct Echo;

impl ProtocolHandler for Echo {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let (mut send, mut recv) = connection.accept_bi().await?;
        let request = recv.read_to_end(16).await.map_err(std::io::Error::other)?;
        if request == b"PING" {
            send.write_all(b"PONG")
                .await
                .map_err(std::io::Error::other)?;
        }
        send.finish()?;
        connection.closed().await;
        Ok(())
    }
}

#[derive(Debug)]
struct NoOpHook;

impl EndpointHooks for NoOpHook {}

#[tokio::test]
async fn a_gated_endpoint_never_treats_a_default_hook_as_admission() {
    let server = Endpoint::builder(presets::Minimal)
        .admission_gate(NoOpHook)
        .alpns(vec![BUSINESS.to_vec()])
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .bind()
        .await
        .unwrap();
    let client = Endpoint::builder(presets::Minimal)
        .admission_gate(NoOpHook)
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .bind()
        .await
        .unwrap();
    let target = EndpointAddr::new(server.id())
        .with_addrs(server.addr().ip_addrs().copied().map(TransportAddr::Ip));
    let (out, incoming) = tokio::time::timeout(Duration::from_secs(9), async {
        tokio::join!(client.connect(target, BUSINESS), async {
            server.accept().await.unwrap().await
        })
    })
    .await
    .unwrap();
    assert!(out.is_err() && incoming.is_err());
    client.close().await;
    server.close().await;
}

fn panda_config() -> IrohConfig {
    IrohConfig {
        bind_ip_v4: Ipv4Addr::LOCALHOST,
        bind_port_v4: 0,
        bind_ip_v6: std::net::Ipv6Addr::LOCALHOST,
        bind_port_v6: 0,
    }
}

#[tokio::test]
async fn p2panda_endpoint_builder_hook_gates_the_actual_protocol_actor() {
    let server_key = SigningKey::generate();
    let client_key = SigningKey::generate();
    let server_id = SecretKey::from_bytes(server_key.as_bytes()).public();
    let client_id = SecretKey::from_bytes(client_key.as_bytes()).public();
    let scope = AdmissionScope::new(NETWORK, 0);
    let server_gate = AdmissionGate::new(
        server_id,
        scope.clone(),
        Some(AdmissionSecret::from_saved(SECRET)),
    );
    let client_gate =
        AdmissionGate::new(client_id, scope, Some(AdmissionSecret::from_saved(SECRET)));
    for gate in [&server_gate, &client_gate] {
        gate.register_p2panda_defaults(NETWORK);
        gate.update_authorization([*server_id.as_bytes(), *client_id.as_bytes()], []);
    }
    let server_address_book = AddressBook::builder().spawn().await.unwrap();
    let server_hook = server_gate.clone();
    let server = PandaEndpoint::builder(server_address_book)
        .network_id(NETWORK)
        .signing_key(server_key.clone())
        .config(panda_config())
        .endpoint_builder_hook(move |builder| server_hook.configure(builder))
        .spawn()
        .await
        .unwrap();
    server.accept(LOG_SYNC, Echo).await.unwrap();
    let iroh_server = server.endpoint().await.unwrap();
    let lookup = MemoryLookup::from_endpoint_info([EndpointAddr::new(server_id).with_addrs(
        iroh_server
            .addr()
            .ip_addrs()
            .copied()
            .map(iroh_base::TransportAddr::Ip),
    )]);
    let client_address_book = AddressBook::builder().spawn().await.unwrap();
    let client_hook = client_gate.clone();
    let client = PandaEndpoint::builder(client_address_book)
        .network_id(NETWORK)
        .signing_key(client_key)
        .config(panda_config())
        .endpoint_builder_hook(move |builder| {
            client_hook
                .configure(builder)
                .address_lookup(lookup.clone())
        })
        .spawn()
        .await
        .unwrap();
    let connection = tokio::time::timeout(
        Duration::from_secs(9),
        client.connect(server_key.verifying_key(), LOG_SYNC),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(client_gate.allows(&connection, ProtocolScope::Member));
    let (mut send, mut recv) = connection.open_bi().await.unwrap();
    send.write_all(b"PING").await.unwrap();
    send.finish().unwrap();
    assert_eq!(recv.read_to_end(16).await.unwrap(), b"PONG");
    connection.close(0u32.into(), b"done");
    drop(client);
    drop(server);
}

#[tokio::test]
async fn cancelled_proof_does_not_authorize_a_later_connection() {
    let pair = Pair::new(Some(SECRET), SECRET, true).await;
    let rogue = Endpoint::builder(presets::Minimal)
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .bind()
        .await
        .unwrap();
    let target = EndpointAddr::new(pair.server.id()).with_addrs(
        pair.server
            .addr()
            .ip_addrs()
            .copied()
            .map(iroh_base::TransportAddr::Ip),
    );
    let server = pair.server.clone();
    let pending = tokio::spawn(async move { server.accept().await.unwrap().await });
    let rogue_connection = rogue.connect(target, BUSINESS).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    pending.abort();
    let _ = pending.await;
    tokio::time::timeout(Duration::from_secs(2), rogue_connection.closed())
        .await
        .unwrap();
    let (out, incoming) = pair.connect(BUSINESS).await;
    let out = out.unwrap();
    let incoming = incoming.unwrap();
    assert!(pair.server_gate.allows(&incoming, ProtocolScope::Member));
    out.close(0u32.into(), b"done");
    incoming.close(0u32.into(), b"done");
    rogue.close().await;
    pair.close().await;
}

#[tokio::test]
async fn a_connection_to_an_uninvited_tls_identity_cannot_enter_application() {
    let pair = Pair::new(None, SECRET, false).await;
    let wrong_contact = SecretKey::generate().public();
    let target = EndpointAddr::new(wrong_contact).with_addrs(
        pair.server
            .addr()
            .ip_addrs()
            .copied()
            .map(iroh_base::TransportAddr::Ip),
    );
    let (out, incoming) = tokio::time::timeout(Duration::from_secs(9), async {
        tokio::join!(pair.client.connect(target, ADMISSION_REQUEST_ALPN), async {
            pair.server.accept().await.unwrap().await
        })
    })
    .await
    .unwrap();
    assert!(out.is_err() && incoming.is_err());
    pair.close().await;
}

#[tokio::test]
async fn known_removed_identity_only_reaches_fresh_restricted_admission_requests() {
    let pair = Pair::new(Some(SECRET), SECRET, false).await;
    pair.server_gate.update_authorization(
        [*pair.server.id().as_bytes()],
        [*pair.client.id().as_bytes()],
    );
    let (out, incoming) = pair.connect(ADMISSION_REQUEST_ALPN).await;
    let incoming = incoming.unwrap();
    assert!(out.is_some());
    assert!(
        pair.server_gate
            .allows(&incoming, ProtocolScope::AdmissionRequest)
    );
    assert!(!pair.server_gate.allows(&incoming, ProtocolScope::Member));
    incoming.close(0u32.into(), b"restricted request only");
    for alpn in [
        RECOVERY_ALPN.to_vec(),
        MEMBER_BOOTSTRAP_ALPN.to_vec(),
        mixed(DISCOVERY),
        mixed(GOSSIP),
        mixed(LOG_SYNC),
        BUSINESS.to_vec(),
    ] {
        let (out, incoming) = pair.connect(&alpn).await;
        assert!(out.is_none() && incoming.is_none(), "accepted {alpn:?}");
    }
    pair.close().await;
}

#[tokio::test]
async fn stalled_proof_times_out_and_fresh_connection_can_still_prove() {
    let pair = Pair::new(Some(SECRET), SECRET, true).await;
    let rogue = Endpoint::builder(presets::Minimal)
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .bind()
        .await
        .unwrap();
    let target = EndpointAddr::new(pair.server.id()).with_addrs(
        pair.server
            .addr()
            .ip_addrs()
            .copied()
            .map(iroh_base::TransportAddr::Ip),
    );
    let server = pair.server.clone();
    let stalled = tokio::spawn(async move { server.accept().await.unwrap().await });
    let rogue_connection = rogue.connect(target, BUSINESS).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(7), stalled)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    tokio::time::timeout(Duration::from_secs(2), rogue_connection.closed())
        .await
        .unwrap();
    let (out, incoming) = pair.connect(BUSINESS).await;
    let out = out.unwrap();
    let incoming = incoming.unwrap();
    assert!(pair.server_gate.allows(&incoming, ProtocolScope::Member));
    out.close(0u32.into(), b"done");
    incoming.close(0u32.into(), b"done");
    rogue.close().await;
    pair.close().await;
}

#[tokio::test]
async fn signaling_carrier_allows_scoped_control_but_refuses_business() {
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let broker: SocketAddr = broker.parse().unwrap();
    let client_key = SecretKey::generate();
    let server_key = SecretKey::generate();
    let scope = AdmissionScope::new(NETWORK, 0);
    let client_gate = AdmissionGate::new(
        client_key.public(),
        scope.clone(),
        Some(AdmissionSecret::from_saved(SECRET)),
    );
    let server_gate = AdmissionGate::new(
        server_key.public(),
        scope,
        Some(AdmissionSecret::from_saved(SECRET)),
    );
    for gate in [&client_gate, &server_gate] {
        gate.register_protocol(BUSINESS, ProtocolScope::Member);
        gate.update_authorization(
            [
                *client_key.public().as_bytes(),
                *server_key.public().as_bytes(),
            ],
            [],
        );
    }
    let server_carrier = Arc::new(MqttTransport::new(broker, Arc::new(Traffic::default())));
    let client_carrier = Arc::new(MqttTransport::new(broker, Arc::new(Traffic::default())));
    let server = server_gate
        .configure(Endpoint::builder(presets::Minimal))
        .secret_key(server_key)
        .alpns(vec![
            BUSINESS.to_vec(),
            miopunch_native_probe::admission_gate::CONTROL_ALPN.to_vec(),
        ])
        .control_only_alpns(vec![
            miopunch_native_probe::admission_gate::CONTROL_ALPN.to_vec(),
        ])
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .clear_relay_transports()
        .add_custom_transport(server_carrier.clone())
        .bind()
        .await
        .unwrap();
    let client = client_gate
        .configure(Endpoint::builder(presets::Minimal))
        .secret_key(client_key)
        .control_only_alpns(vec![
            miopunch_native_probe::admission_gate::CONTROL_ALPN.to_vec(),
        ])
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .clear_relay_transports()
        .add_custom_transport(client_carrier.clone())
        .bind()
        .await
        .unwrap();
    server_carrier.wait_ready().await.unwrap();
    client_carrier.wait_ready().await.unwrap();
    let target = EndpointAddr::new(server.id()).with_addrs([TransportAddr::Custom(
        MqttTransport::address(&server_carrier.route().unwrap()).unwrap(),
    )]);
    let (out, incoming) = tokio::time::timeout(Duration::from_secs(9), async {
        tokio::join!(client.connect(target.clone(), BUSINESS), async {
            server.accept().await.unwrap().await
        })
    })
    .await
    .unwrap();
    assert!(out.is_err() && incoming.is_err());
    let (out, incoming) = tokio::time::timeout(Duration::from_secs(9), async {
        tokio::join!(
            client.connect(target, miopunch_native_probe::admission_gate::CONTROL_ALPN),
            async { server.accept().await.unwrap().await }
        )
    })
    .await
    .unwrap();
    let (out, incoming) = (out.unwrap(), incoming.unwrap());
    assert!(server_gate.allows(&incoming, ProtocolScope::Bootstrap));
    assert_eq!(incoming.test_local_qnt_candidate_count(), 0);
    out.close(0u32.into(), b"done");
    incoming.close(0u32.into(), b"done");

    // The same member can use a real IP data connection, but a later path
    // attempt cannot redirect that admitted stream over MQTT.
    let data_target = EndpointAddr::new(server.id())
        .with_addrs(server.addr().ip_addrs().copied().map(TransportAddr::Ip));
    let (out, incoming) = tokio::time::timeout(Duration::from_secs(9), async {
        tokio::join!(client.connect(data_target, BUSINESS), async {
            server.accept().await.unwrap().await
        })
    })
    .await
    .unwrap();
    let (out, incoming) = (out.unwrap(), incoming.unwrap());
    assert!(server_gate.allows(&incoming, ProtocolScope::Member));
    let mqtt_addr = MqttTransport::address(&server_carrier.route().unwrap()).unwrap();
    let forbidden = client.test_open_admitted_custom_path(&out, mqtt_addr).await;
    assert!(forbidden.is_err(), "business gained a signaling path");
    let (mut send, _) = out.open_bi().await.unwrap();
    send.write_all(b"DATA").await.unwrap();
    send.finish().unwrap();
    let (_, mut recv) = incoming.accept_bi().await.unwrap();
    assert_eq!(recv.read_to_end(16).await.unwrap(), b"DATA");
    out.close(0u32.into(), b"done");
    incoming.close(0u32.into(), b"done");
    client.close().await;
    server.close().await;
}

#[tokio::test]
async fn an_admitted_tcp_data_connection_survives_an_unavailable_nat4_strategy() {
    let client_key = SecretKey::generate();
    let server_key = SecretKey::generate();
    let scope = AdmissionScope::new(NETWORK, 0);
    let client_gate = AdmissionGate::new(
        client_key.public(),
        scope.clone(),
        Some(AdmissionSecret::from_saved(SECRET)),
    );
    let server_gate = AdmissionGate::new(
        server_key.public(),
        scope,
        Some(AdmissionSecret::from_saved(SECRET)),
    );
    for gate in [&client_gate, &server_gate] {
        gate.register_protocol(BUSINESS, ProtocolScope::Member);
        gate.update_authorization(
            [
                *client_key.public().as_bytes(),
                *server_key.public().as_bytes(),
            ],
            [],
        );
    }
    let server_carrier = Arc::new(TcpTransport::new(
        Ipv4Addr::LOCALHOST,
        Arc::new(TcpTraffic::default()),
    ));
    let client_carrier = Arc::new(TcpTransport::new(
        Ipv4Addr::LOCALHOST,
        Arc::new(TcpTraffic::default()),
    ));
    let server = server_gate
        .configure(Endpoint::builder(presets::Minimal))
        .secret_key(server_key)
        .alpns(vec![BUSINESS.to_vec()])
        .clear_ip_transports()
        .clear_relay_transports()
        .add_custom_transport(server_carrier.clone())
        .path_selector(Arc::new(TcpOnly))
        .bind()
        .await
        .unwrap();
    let client = client_gate
        .configure(Endpoint::builder(presets::Minimal))
        .secret_key(client_key)
        .clear_ip_transports()
        .clear_relay_transports()
        .add_custom_transport(client_carrier)
        .path_selector(Arc::new(TcpOnly))
        .bind()
        .await
        .unwrap();
    let target = EndpointAddr::new(server.id()).with_addrs([TransportAddr::Custom(
        TcpTransport::address(server_carrier.bound_addr().unwrap()),
    )]);
    let (out, incoming) = tokio::time::timeout(Duration::from_secs(9), async {
        tokio::join!(client.connect(target, BUSINESS), async {
            server.accept().await.unwrap().await
        })
    })
    .await
    .unwrap();
    let (out, incoming) = (out.unwrap(), incoming.unwrap());
    assert!(server_gate.allows(&incoming, ProtocolScope::Member));
    let (mut send, _) = out.open_bi().await.unwrap();
    send.write_all(b"TCP DATA").await.unwrap();
    send.finish().unwrap();
    let (_, mut recv) = incoming.accept_bi().await.unwrap();
    assert_eq!(recv.read_to_end(16).await.unwrap(), b"TCP DATA");

    // A failed UDP mapping choice and exhausted enhancement budget do not
    // revoke another already-admitted data path or its current QUIC session.
    let stable_id = out.stable_id();
    assert!(client.bound_sockets().is_empty());
    let disabled = client.register_udp_traversal(&out, [59; 16]).await;
    assert_eq!(disabled.unwrap_err().to_string(), "IPv4 traversal disabled");
    assert_eq!(out.stable_id(), stable_id);
    assert!(
        choose_single_sequential(
            MappingPattern::Unknown,
            MappingPattern::Stable {
                address: "127.0.0.1:42027".parse().unwrap(),
            }
        )
        .is_none()
    );
    assert!(
        choose_regular_pair(
            MappingPattern::Sequential {
                last: "127.0.0.1:42034".parse().unwrap(),
                step: 1,
            },
            MappingPattern::Unknown,
            true,
        )
        .is_none()
    );
    let budget = NodeProbeBudget::new(0, 0);
    assert!(!budget.reserve_observations(4));
    assert!(!budget.spend_peer_datagram());
    let (mut send, _) = out.open_bi().await.unwrap();
    send.write_all(b"TCP AFTER UDP DECISION FAILURE")
        .await
        .unwrap();
    send.finish().unwrap();
    let (_, mut recv) = incoming.accept_bi().await.unwrap();
    assert_eq!(
        recv.read_to_end(64).await.unwrap(),
        b"TCP AFTER UDP DECISION FAILURE"
    );
    assert_eq!(out.stable_id(), stable_id);
    assert!(server_gate.allows(&incoming, ProtocolScope::Member));
    out.close(0u32.into(), b"done");
    incoming.close(0u32.into(), b"done");
    client.close().await;
    server.close().await;
}

#[tokio::test]
async fn removal_during_final_recovery_stage_must_reject_connection() {
    let client_key = SecretKey::generate();
    let gate = AdmissionGate::new(client_key.public(), AdmissionScope::new([23; 32], 0), None);
    let client = gate
        .configure(Endpoint::builder(presets::Minimal))
        .secret_key(client_key)
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .bind()
        .await
        .unwrap();
    let raw = Endpoint::builder(presets::Minimal)
        .alpns(vec![RECOVERY_ALPN.to_vec()])
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .bind()
        .await
        .unwrap();
    let target = EndpointAddr::new(raw.id()).with_ip_addr(*raw.addr().ip_addrs().next().unwrap());
    let endpoint = client.clone();
    let dialed = tokio::spawn(async move { endpoint.connect(target, RECOVERY_ALPN).await });
    let server = raw.accept().await.unwrap().await.unwrap();
    let (mut send, mut recv) = tokio::time::timeout(Duration::from_secs(3), server.accept_bi())
        .await
        .unwrap()
        .unwrap();
    let mut stage = [0];
    recv.read_exact(&mut stage).await.unwrap();
    assert_eq!(stage, [1]);
    // The gated client has passed its second policy check, sent its stage
    // byte and waits for ours. No session has entered the lifetime index yet.
    gate.update_authorization([*client.id().as_bytes()], [*raw.id().as_bytes()]);
    send.write_all(&[1]).await.unwrap();
    send.finish().unwrap();
    let result = tokio::time::timeout(Duration::from_secs(3), dialed)
        .await
        .unwrap()
        .unwrap();
    let rejected = result.is_err();
    client.close().await;
    raw.close().await;
    assert!(
        rejected,
        "recovery accepted after effective removal during final stage"
    );
}
