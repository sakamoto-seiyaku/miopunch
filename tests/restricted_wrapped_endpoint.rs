//! The p2panda actor receives business only after explicit restricted admission.
use iroh::{
    EndpointAddr,
    address_lookup::memory::MemoryLookup,
    endpoint::{AfterHandshakeOutcome, Connection, EndpointHooks, RestrictedEndpoint},
    protocol::{AcceptError, ProtocolHandler},
};
use miopunch_native_probe::{
    admission_gate::{AdmissionGate, ProtocolScope},
    admission_proof::{AdmissionScope, AdmissionSecret},
};
use p2panda_net::{AddressBook, Endpoint, iroh_endpoint::IrohConfig};
use std::{
    net::Ipv4Addr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
const NETWORK: [u8; 32] = [23; 32];
const PROTOCOL: &[u8] = b"miopunch/wrapped-restricted-member/1";

#[derive(Clone, Debug)]
struct LegacyCheck {
    gate: AdmissionGate,
    calls: Arc<AtomicUsize>,
}
impl EndpointHooks for LegacyCheck {
    async fn after_handshake(&self, connection: &Connection) -> AfterHandshakeOutcome {
        assert!(self.gate.allows(connection, ProtocolScope::Member));
        self.calls.fetch_add(1, Ordering::SeqCst);
        AfterHandshakeOutcome::Accept
    }
}
#[derive(Debug)]
struct Echo {
    gate: AdmissionGate,
    calls: Arc<AtomicUsize>,
}
impl ProtocolHandler for Echo {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        assert!(self.gate.allows(&connection, ProtocolScope::Member));
        self.calls.fetch_add(1, Ordering::SeqCst);
        let (mut send, mut recv) = connection.accept_bi().await?;
        assert_eq!(
            recv.read_to_end(4).await.map_err(std::io::Error::other)?,
            b"PING"
        );
        send.write_all(b"PONG")
            .await
            .map_err(std::io::Error::other)?;
        send.finish()?;
        connection.closed().await;
        Ok(())
    }
}
fn config() -> IrohConfig {
    IrohConfig {
        bind_ip_v4: Ipv4Addr::LOCALHOST,
        bind_port_v4: 0,
        bind_ip_v6: std::net::Ipv6Addr::LOCALHOST,
        bind_port_v6: 0,
    }
}

#[tokio::test]
async fn wrapped_actor_releases_member_after_checks_and_closes_on_revocation() {
    let server_key = p2panda_core::SigningKey::generate();
    let client_key = p2panda_core::SigningKey::generate();
    let server_id = iroh::SecretKey::from_bytes(server_key.as_bytes()).public();
    let client_id = iroh::SecretKey::from_bytes(client_key.as_bytes()).public();
    let server_gate = AdmissionGate::new(
        server_id,
        AdmissionScope::new(NETWORK, 0),
        Some(AdmissionSecret::from_saved([42; 32])),
    );
    let client_gate = AdmissionGate::new(
        client_id,
        AdmissionScope::new(NETWORK, 0),
        Some(AdmissionSecret::from_saved([42; 32])),
    );
    for gate in [&server_gate, &client_gate] {
        gate.register_p2panda_protocol(PROTOCOL, NETWORK, ProtocolScope::Member);
        gate.update_authorization([*server_id.as_bytes(), *client_id.as_bytes()], []);
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let business = Arc::new(AtomicUsize::new(0));
    let a = AddressBook::builder().spawn().await.unwrap();
    let b = AddressBook::builder().spawn().await.unwrap();
    let gate = server_gate.clone();
    let hook = LegacyCheck {
        gate: gate.clone(),
        calls: calls.clone(),
    };
    let server = Endpoint::builder(a)
        .network_id(NETWORK)
        .signing_key(server_key.clone())
        .config(config())
        .endpoint_builder_hook(move |builder| builder.hooks(hook.clone()))
        .spawn_restricted(move |builder| builder.restricted_application(gate.clone()))
        .await
        .unwrap();
    server
        .accept(
            PROTOCOL,
            Echo {
                gate: server_gate.clone(),
                calls: business.clone(),
            },
        )
        .await
        .unwrap();
    let se: RestrictedEndpoint = server.endpoint().await.unwrap();
    let lookup = MemoryLookup::from_endpoint_info([EndpointAddr::new(server_id)
        .with_addrs(se.addr().ip_addrs().copied().map(iroh::TransportAddr::Ip))]);
    let gate = client_gate.clone();
    let hook = LegacyCheck {
        gate: gate.clone(),
        calls: calls.clone(),
    };
    let client = Endpoint::builder(b)
        .network_id(NETWORK)
        .signing_key(client_key)
        .config(config())
        .endpoint_builder_hook(move |builder| {
            builder.address_lookup(lookup.clone()).hooks(hook.clone())
        })
        .spawn_restricted(move |builder| builder.restricted_application(gate.clone()))
        .await
        .unwrap();
    let connection = tokio::time::timeout(
        Duration::from_secs(5),
        client.connect(server_key.verifying_key(), PROTOCOL),
    )
    .await
    .unwrap()
    .unwrap();
    let (mut send, mut recv) = connection.open_bi().await.unwrap();
    send.write_all(b"PING").await.unwrap();
    send.finish().unwrap();
    assert_eq!(recv.read_to_end(4).await.unwrap(), b"PONG");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    server_gate.update_authorization([*server_id.as_bytes()], [*client_id.as_bytes()]);
    tokio::time::timeout(Duration::from_secs(2), connection.closed())
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(
            Duration::from_secs(5),
            client.connect(server_key.verifying_key(), PROTOCOL)
        )
        .await
        .unwrap()
        .is_err()
    );
    assert_eq!(business.load(Ordering::SeqCst), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let ce: RestrictedEndpoint = client.endpoint().await.unwrap();
    tokio::join!(ce.close(), se.close());
    drop((connection, client, server, ce, se));
}
