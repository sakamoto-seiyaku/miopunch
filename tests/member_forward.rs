//! Three persistent members, real admission and QUIC, one intermediate member.
use iroh::{
    Endpoint, EndpointAddr, TransportAddr,
    endpoint::{Connection, presets},
};
use miopunch_native_probe::{
    admission_gate::{ADMISSION_REQUEST_ALPN, ProtocolScope},
    manual_admission::ManualExchange,
    member_transport::{MemberForwarder, MemberSignalingTransport, SignalingLease},
    native_network::NativeNetwork,
};
use std::{net::Ipv4Addr, path::Path, sync::Arc, time::Duration};

const BUSINESS: &[u8] = b"miopunch/member-forward-business/1";
const LEASE: Duration = Duration::from_secs(60);

async fn connect(a: &Endpoint, b: &Endpoint, alpn: &[u8]) -> (Connection, Connection) {
    let addr = EndpointAddr::new(b.id()).with_ip_addr(*b.addr().ip_addrs().next().unwrap());
    tokio::time::timeout(Duration::from_secs(10), async {
        let (a, b) = tokio::join!(a.connect(addr, alpn), async {
            b.accept().await.unwrap().await
        });
        (a.unwrap(), b.unwrap())
    })
    .await
    .unwrap()
}
async fn join(admin: &mut NativeNetwork, path: &Path) -> NativeNetwork {
    let mut node = NativeNetwork::join(path, admin.manual_invitation().unwrap())
        .await
        .unwrap();
    let ae = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let ne = node.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = connect(&ne, &ae, ADMISSION_REQUEST_ALPN).await;
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
            panic!("saved approval")
        };
        node.complete_manual_admission(material).await.unwrap();
    };
    let (_, served) = tokio::join!(receive, admin.serve_manual_admission(&server));
    served.unwrap();
    client.close(0u32.into(), b"admitted");
    server.close(0u32.into(), b"admitted");
    ne.close().await;
    ae.close().await;
    node
}
async fn nodes() -> (
    tempfile::TempDir,
    NativeNetwork,
    NativeNetwork,
    NativeNetwork,
) {
    let directory = tempfile::tempdir().unwrap();
    let mut b = NativeNetwork::create(directory.path().join("b"))
        .await
        .unwrap();
    // No MQTT, native relay, address observer or member data-relay facility.
    let mut config = b.resource_configuration().unwrap();
    config.replacement = Some(vec![]);
    b.save_resource_configuration(&config).unwrap();
    let mut a = join(&mut b, &directory.path().join("a")).await;
    let c = join(&mut b, &directory.path().join("c")).await;
    // Public signed-record ingestion completes A's view of C's admission.
    for record in b.management_history().await.unwrap() {
        a.ingest_management_record(record.header(), record.body())
            .await
            .unwrap();
    }
    for node in [&a, &b, &c] {
        node.save_resource_configuration(&config).unwrap();
        node.admission_gate()
            .register_protocol(BUSINESS, ProtocolScope::Member);
    }
    (directory, a, b, c)
}
async fn endpoint(
    node: &NativeNetwork,
    transport: Option<Arc<MemberSignalingTransport>>,
) -> Endpoint {
    let mut builder = Endpoint::builder(presets::Minimal)
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .transport_config(
            iroh::endpoint::QuicTransportConfig::builder()
                .keep_alive_interval(Duration::from_secs(1))
                .max_idle_timeout(Some(Duration::from_secs(4).try_into().unwrap()))
                .build(),
        );
    if let Some(transport) = transport {
        builder = builder.add_custom_transport(transport);
    }
    let resources = node.bind_configured_endpoint_with(builder).await.unwrap();
    resources.endpoint().set_alpns(vec![BUSINESS.to_vec()]);
    resources.endpoint().clone()
}
async fn echo(client: &Connection, server: &Connection) {
    let (sent, received) = tokio::join!(
        async {
            let (mut send, mut recv) = client.open_bi().await.unwrap();
            send.write_all(b"private A-C business").await.unwrap();
            send.finish().unwrap();
            assert_eq!(recv.read_to_end(64).await.unwrap(), b"private ACK");
        },
        async {
            let (mut send, mut recv) = server.accept_bi().await.unwrap();
            assert_eq!(recv.read_to_end(64).await.unwrap(), b"private A-C business");
            send.write_all(b"private ACK").await.unwrap();
            send.finish().unwrap();
        }
    );
    let _ = (sent, received);
}
struct Topology {
    _directory: tempfile::TempDir,
    _nodes: (NativeNetwork, NativeNetwork, NativeNetwork),
    a: Endpoint,
    b: Endpoint,
    c: Endpoint,
    at: Arc<MemberSignalingTransport>,
    ct: Arc<MemberSignalingTransport>,
    ab: (Connection, Connection),
    cb: (Connection, Connection),
    _leases: Vec<SignalingLease>,
    forwarder: MemberForwarder,
}
impl Topology {
    async fn new() -> Self {
        Self::with_lease(LEASE).await
    }
    async fn with_lease(lifetime: Duration) -> Self {
        let (directory, a, b, c) = nodes().await;
        let be = endpoint(&b, None).await;
        let at = Arc::new(MemberSignalingTransport::new(be.id(), a.admission_gate()));
        let ct = Arc::new(MemberSignalingTransport::new(be.id(), c.admission_gate()));
        let ae = endpoint(&a, Some(at.clone())).await;
        let ce = endpoint(&c, Some(ct.clone())).await;
        let ab = connect(&ae, &be, BUSINESS).await;
        let cb = connect(&ce, &be, BUSINESS).await;
        let forwarder = MemberForwarder::new(b.admission_gate());
        let (al, bl) = tokio::join!(
            at.attach(&ab.0, lifetime),
            forwarder.accept_link(&ab.1, lifetime)
        );
        let (cl, dl) = tokio::join!(
            ct.attach(&cb.0, lifetime),
            forwarder.accept_link(&cb.1, lifetime)
        );
        Self {
            _directory: directory,
            _nodes: (a, b, c),
            a: ae,
            b: be,
            c: ce,
            at,
            ct,
            ab,
            cb,
            _leases: vec![al.unwrap(), bl.unwrap(), cl.unwrap(), dl.unwrap()],
            forwarder,
        }
    }
    fn target(&self) -> EndpointAddr {
        EndpointAddr::new(self.c.id())
            .with_addrs([TransportAddr::Custom(self.ct.address().unwrap())])
    }
    async fn close(self) {
        drop(self._leases);
        self.a.close().await;
        self.b.close().await;
        self.c.close().await;
    }
}

#[tokio::test]
async fn expired_forwarding_leases_remove_routes_without_ending_the_member_links() {
    let topology = Topology::with_lease(Duration::from_millis(250)).await;
    assert_eq!(topology.forwarder.active_links(), 2);
    let target = topology.target();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(topology.forwarder.active_links(), 0);
    assert!(topology.at.address().is_err() && topology.ct.address().is_err());
    assert!(topology._leases.iter().all(|lease| lease.is_finished()));
    assert!(
        tokio::time::timeout(Duration::from_secs(7), topology.a.connect(target, BUSINESS))
            .await
            .unwrap()
            .is_err()
    );
    echo(&topology.ab.0, &topology.ab.1).await;
    println!("MEMBER_EXPIRY_PASS registered_routes=0 stale_route_rejected=true ab_alive=true");
    topology.close().await;
}

#[tokio::test]
async fn stopping_a_forwarding_stream_fails_ac_control_and_keeps_ab_usable() {
    let mut topology = Topology::new().await;
    drop(topology._leases.remove(1));
    tokio::time::timeout(Duration::from_secs(2), async {
        while topology.at.address().is_ok() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(topology.forwarder.active_links(), 1);
    let target = topology.target();
    assert!(
        tokio::time::timeout(Duration::from_secs(7), topology.a.connect(target, BUSINESS))
            .await
            .unwrap()
            .is_err()
    );
    echo(&topology.ab.0, &topology.ab.1).await;
    println!(
        "MEMBER_INTERRUPTION_PASS forwarding_stream_stopped=true ac_unavailable=true ab_alive=true"
    );
    topology.close().await;
}

#[tokio::test]
async fn repeated_registration_does_not_add_forwarding_routes_or_confuse_member_identity() {
    let topology = Topology::new().await;
    let (client, server) = tokio::join!(
        topology.at.attach(&topology.ab.0, LEASE),
        topology.forwarder.accept_link(&topology.ab.1, LEASE)
    );
    assert!(client.is_err() && server.is_err());
    assert_eq!(topology.forwarder.active_links(), 2);
    echo(&topology.ab.0, &topology.ab.1).await;
    println!("MEMBER_DUPLICATE_REGISTRATION_PASS registered_routes=2 ab_alive=true");
    topology.close().await;
}

#[tokio::test]
async fn a_forwarding_route_cannot_substitute_the_inner_tls_target_identity() {
    let topology = Topology::new().await;
    let mut target = topology.target();
    target.id = iroh::SecretKey::generate().public();
    let (client, server) = tokio::time::timeout(Duration::from_secs(8), async {
        tokio::join!(topology.a.connect(target, BUSINESS), async {
            topology.c.accept().await.unwrap().await
        })
    })
    .await
    .unwrap();
    assert!(client.is_err() && server.is_err());
    echo(&topology.ab.0, &topology.ab.1).await;
    println!("MEMBER_WRONG_TARGET_PASS valid_c_route=true unrelated_identity_rejected=true");
    topology.close().await;
}

#[tokio::test]
async fn an_admitted_bootstrap_session_does_not_grant_member_forwarding() {
    use miopunch_native_probe::admission_gate::CONTROL_ALPN;
    let topology = Topology::new().await;
    topology
        .b
        .set_alpns(vec![BUSINESS.to_vec(), CONTROL_ALPN.to_vec()]);
    let (client, server) = connect(&topology.a, &topology.b, CONTROL_ALPN).await;
    assert!(
        topology
            ._nodes
            .0
            .admission_gate()
            .allows_member_path_upgrade(&client)
    );
    assert!(topology.at.attach(&client, LEASE).await.is_err());
    assert!(
        topology
            .forwarder
            .accept_link(&server, LEASE)
            .await
            .is_err()
    );
    assert_eq!(topology.forwarder.active_links(), 2);
    println!("MEMBER_SCOPE_PASS bootstrap_admitted=true member_forwarding_rejected=true");
    client.close(0u32.into(), b"done");
    server.close(0u32.into(), b"done");
    topology.close().await;
}

#[cfg(feature = "member-signaling-test-hooks")]
#[tokio::test]
async fn duplicated_opaque_datagrams_cannot_reveal_private_control_or_release_extra_connections() {
    let topology = Topology::new().await;
    topology.forwarder.test_duplicate_datagrams(true);
    let target = topology.target();
    let (client, server) = tokio::time::timeout(Duration::from_secs(12), async {
        tokio::join!(topology.a.connect(target, BUSINESS), async {
            topology.c.accept().await.unwrap().await
        })
    })
    .await
    .unwrap();
    let client = client.unwrap();
    let server = server.unwrap();
    echo(&client, &server).await;
    let observed = topology.forwarder.test_observed_datagrams();
    assert!(!observed.is_empty() && observed.len() <= 64);
    for packet in &observed {
        for secret in [
            b"127.0.0.1".as_slice(),
            b"private A-C business",
            b"private ACK",
        ] {
            assert!(!packet.windows(secret.len()).any(|window| window == secret));
        }
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(400), topology.c.accept())
            .await
            .is_err()
    );
    assert_eq!(topology.forwarder.active_links(), 2);
    println!(
        "MEMBER_DUPLICATE_PACKETS_PASS observed_ciphertext_datagrams={} private_candidates_visible=false extra_connections=0",
        observed.len()
    );
    client.close(0u32.into(), b"done");
    server.close(0u32.into(), b"done");
    topology.close().await;
}

#[tokio::test]
async fn closing_the_endpoint_releases_its_forwarding_lease_without_waiting_for_expiry() {
    let topology = Topology::new().await;
    topology.a.close().await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while topology.forwarder.active_links() != 1 || !topology._leases[0].is_finished() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("endpoint close must promptly release owned carrier tasks");
    echo(&topology.cb.0, &topology.cb.1).await;
    println!(
        "MEMBER_ENDPOINT_CLOSE_PASS owned_lease_finished=true remaining_links=1 cb_alive=true"
    );
    topology.close().await;
}

#[tokio::test]
async fn member_without_data_relay_forwards_protected_control_then_ac_uses_a_direct_path() {
    let topology = Topology::new().await;
    let target = topology.target();
    assert!(
        target
            .addrs
            .iter()
            .all(|a| matches!(a, TransportAddr::Custom(_)))
    );
    let (outgoing, incoming) = tokio::time::timeout(Duration::from_secs(12), async {
        tokio::join!(topology.a.connect(target, BUSINESS), async {
            topology.c.accept().await.unwrap().await
        })
    })
    .await
    .expect("member-only signaling must form A-C");
    let outgoing = outgoing.expect("member-only signaling must release the ordinary connection");
    let incoming = incoming.unwrap();
    assert_eq!(outgoing.remote_id(), topology.c.id());
    assert!(
        outgoing
            .paths()
            .iter()
            .any(|p| matches!(p.remote_addr(), TransportAddr::Ip(_)))
    );
    echo(&outgoing, &incoming).await;
    println!(
        "MEMBER_FIRST_PASS relay_enabled=false inner_tls_peer={} stable_id={} direct_data=true",
        outgoing.remote_id(),
        outgoing.stable_id()
    );
    // The original A-B member connection is still independently usable.
    echo(&topology.ab.0, &topology.ab.1).await;
    outgoing.close(0u32.into(), b"done");
    incoming.close(0u32.into(), b"done");
    topology.close().await;
}

#[tokio::test]
async fn signaling_only_cannot_release_business_without_another_permitted_ac_path() {
    let topology = Topology::new().await;
    // Withdraw A/C's runtime permission for new UDP paths, retaining the
    // already admitted A-B/B-C connections. No TCP or relay is configured.
    topology._nodes.0.udp_connector().set_udp_enabled(false);
    topology._nodes.2.udp_connector().set_udp_enabled(false);
    let target = topology.target();
    let (outgoing, incoming) = tokio::time::timeout(Duration::from_secs(8), async {
        tokio::join!(topology.a.connect(target, BUSINESS), async {
            topology.c.accept().await.unwrap().await
        })
    })
    .await
    .expect("signaling-only failure must be bounded");
    assert!(
        outgoing.is_err() && incoming.is_err(),
        "no application connection or stream may escape through B"
    );
    echo(&topology.ab.0, &topology.ab.1).await;
    echo(&topology.cb.0, &topology.cb.1).await;
    println!(
        "MEMBER_PRIORITY_PASS control_reachable=true permitted_ac_data_paths=0 business_released=false ab_unchanged=true cb_unchanged=true"
    );
    topology.close().await;
}
