//! Three persistent members, real admission and QUIC, one intermediate member.
#[path = "support/native_connections.rs"]
mod native_connections;
use iroh::endpoint::RestrictedEndpoint;

use iroh::{
    Endpoint, EndpointAddr, TransportAddr,
    endpoint::{Connection, presets},
};
use miopunch_native_probe::{
    admission_gate::{ADMISSION_REQUEST_ALPN, ProtocolScope},
    manual_admission::ManualExchange,
    member_transport::{MemberDataRelay, MemberDataTransport, SignalingLease},
    native_network::NativeNetwork,
};
use std::{net::Ipv4Addr, path::Path, sync::Arc, time::Duration};

const BUSINESS: &[u8] = b"miopunch/member-forward-business/1";
const LEASE: Duration = Duration::from_secs(60);

async fn connect(
    client: &RestrictedEndpoint,
    server: &RestrictedEndpoint,
    alpn: &[u8],
) -> (Connection, Connection) {
    native_connections::member_pair(client, server, alpn).await
}
async fn join(admin: &mut NativeNetwork, path: &Path) -> NativeNetwork {
    let mut node = NativeNetwork::join(path, admin.manual_invitation().unwrap())
        .await
        .unwrap();
    node.save_resource_configuration(&admin.resource_configuration().unwrap())
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
    config.tcp_listen = Some("127.0.0.1:0".parse().unwrap());
    config.udp_direct_disabled = true;
    b.save_resource_configuration(&config).unwrap();
    let mut a = join(&mut b, &directory.path().join("a")).await;
    let mut c = join(&mut b, &directory.path().join("c")).await;
    let saved = std::fs::read(directory.path().join("b/genesis.credential")).unwrap();
    let credential = miopunch_native_probe::genesis_recovery::GenesisCredential::from_saved(
        saved[..32].try_into().unwrap(),
        p2panda_core::Hash::from_bytes(saved[32..].try_into().unwrap()),
    );
    let mut session = b.begin_management_session();
    b.elevate_genesis(&mut session, credential, LEASE).unwrap();
    b.manage(
        &mut session,
        miopunch_native_probe::daily_management::ManagementAction::AppointAdministrator(
            a.status().node_id,
        ),
    )
    .await
    .unwrap();
    let b_id = b.status().node_id;
    b.manage(
        &mut session,
        miopunch_native_probe::daily_management::ManagementAction::DemoteAdministrator(b_id),
    )
    .await
    .unwrap();
    session.end();
    assert!(
        b.status()
            .members
            .iter()
            .any(|(id, access)| *id == b_id && *access == p2panda_auth::Access::read())
    );
    // Public signed-record ingestion completes A's view of C's admission.
    for record in b.management_history().await.unwrap() {
        a.ingest_management_record(record.header(), record.body())
            .await
            .unwrap();
        c.ingest_management_record(record.header(), record.body())
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
    transport: Option<Arc<MemberDataTransport>>,
) -> RestrictedEndpoint {
    endpoint_with_signal(node, transport, None).await
}
async fn endpoint_with_signal(
    node: &NativeNetwork,
    transport: Option<Arc<MemberDataTransport>>,
    signal: Option<Arc<miopunch_native_probe::member_transport::MemberSignalingTransport>>,
) -> RestrictedEndpoint {
    let mut builder = Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
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
    if let Some(signal) = signal {
        builder = builder.add_custom_transport(signal);
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
    a: RestrictedEndpoint,
    b: RestrictedEndpoint,
    c: RestrictedEndpoint,
    at: Arc<MemberDataTransport>,
    ct: Arc<MemberDataTransport>,
    ab: (Connection, Connection),
    cb: (Connection, Connection),
    _leases: Vec<SignalingLease>,
    forwarder: MemberDataRelay,
    signal: Option<(
        Arc<miopunch_native_probe::member_transport::MemberSignalingTransport>,
        Arc<miopunch_native_probe::member_transport::MemberSignalingTransport>,
        miopunch_native_probe::member_transport::MemberForwarder,
    )>,
}
impl Topology {
    async fn new() -> Self {
        Self::with_options(LEASE, false).await
    }
    async fn with_lease(lifetime: Duration) -> Self {
        Self::with_options(lifetime, false).await
    }
    async fn with_options(lifetime: Duration, backup: bool) -> Self {
        Self::with_configuration(lifetime, backup, false).await
    }
    async fn with_configuration(lifetime: Duration, backup: bool, consumer_disabled: bool) -> Self {
        Self::with_signal_configuration(lifetime, backup, consumer_disabled, false).await
    }
    async fn with_signal_configuration(
        lifetime: Duration,
        backup: bool,
        consumer_disabled: bool,
        signaling: bool,
    ) -> Self {
        let (directory, a, b, c) = nodes().await;
        if backup || consumer_disabled {
            for node in [&a, &c] {
                let mut config = node.resource_configuration().unwrap();
                config.udp_direct_disabled = !backup;
                config.member_data_relay_disabled = consumer_disabled;
                config.path_selection.preference =
                    miopunch_native_probe::path_selection::PathPreference::PreferRelay;
                node.save_resource_configuration(&config).unwrap();
            }
        }
        let be = endpoint(&b, None).await;
        let at = Arc::new(MemberDataTransport::new(be.id(), a.admission_gate()));
        let ct = Arc::new(MemberDataTransport::new(be.id(), c.admission_gate()));
        let signal = signaling.then(|| {
            (
                Arc::new(
                    miopunch_native_probe::member_transport::MemberSignalingTransport::new(
                        be.id(),
                        a.admission_gate(),
                    ),
                ),
                Arc::new(
                    miopunch_native_probe::member_transport::MemberSignalingTransport::new(
                        be.id(),
                        c.admission_gate(),
                    ),
                ),
                miopunch_native_probe::member_transport::MemberForwarder::new(b.admission_gate()),
            )
        });
        let ae =
            endpoint_with_signal(&a, Some(at.clone()), signal.as_ref().map(|s| s.0.clone())).await;
        let ce =
            endpoint_with_signal(&c, Some(ct.clone()), signal.as_ref().map(|s| s.1.clone())).await;
        let ab = connect(&ae, &be, BUSINESS).await;
        let cb = connect(&ce, &be, BUSINESS).await;
        let forwarder = MemberDataRelay::new(b.admission_gate());
        forwarder.set_enabled(true);
        let (al, bl) = tokio::join!(
            at.attach(&ab.0, lifetime),
            forwarder.accept_link(&ab.1, lifetime)
        );
        let (cl, dl) = tokio::join!(
            ct.attach(&cb.0, lifetime),
            forwarder.accept_link(&cb.1, lifetime)
        );
        let mut leases = vec![al.unwrap(), bl.unwrap(), cl.unwrap(), dl.unwrap()];
        if let Some((at, ct, forwarder)) = &signal {
            let (al, bl) =
                tokio::join!(at.attach(&ab.0, LEASE), forwarder.accept_link(&ab.1, LEASE));
            let (cl, dl) =
                tokio::join!(ct.attach(&cb.0, LEASE), forwarder.accept_link(&cb.1, LEASE));
            leases.extend([al.unwrap(), bl.unwrap(), cl.unwrap(), dl.unwrap()]);
        }
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
            _leases: leases,
            forwarder,
            signal,
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
async fn explicitly_enabled_member_relay_carries_a_real_end_to_end_business_session() {
    let topology = Topology::new().await;
    let (client, server) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(topology.a.connect(topology.target(), BUSINESS), async {
            topology.c.accept().await.unwrap().await
        })
    })
    .await
    .unwrap();
    let client = client.unwrap();
    let server = server.unwrap();
    assert_eq!(client.remote_id(), topology.c.id());
    assert_eq!(server.remote_id(), topology.a.id());
    assert!(client.paths().iter().any(|p| p.is_selected() && matches!(p.remote_addr(), TransportAddr::Custom(a) if a.id() == miopunch_native_probe::member_transport::DATA_TRANSPORT_ID)));
    echo(&client, &server).await;
    println!("MEMBER_DATA_FIRST_PASS real_ac_tls=true ac_business_via_b=true");
    topology.close().await;
}

async fn business(topology: &Topology) -> (Connection, Connection) {
    let (a, c) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(topology.a.connect(topology.target(), BUSINESS), async {
            topology.c.accept().await.unwrap().await
        })
    })
    .await
    .unwrap();
    (a.unwrap(), c.unwrap())
}
async fn routes_stopped(topology: &Topology) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while topology.forwarder.active_links() != 0
            || topology.at.address().is_ok()
            || topology.ct.address().is_ok()
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("retired data routes and advertisements");
}
#[tokio::test]
async fn disabling_data_relay_stops_new_forwarding_and_the_only_inner_path_fails() {
    let topology = Topology::new().await;
    let target = topology.target();
    let (client, server) = business(&topology).await;
    echo(&client, &server).await;
    assert!(client.paths().iter().all(|p| matches!(p.remote_addr(), TransportAddr::Custom(a) if a.id() == miopunch_native_probe::member_transport::DATA_TRANSPORT_ID)));
    topology.forwarder.set_enabled(false);
    routes_stopped(&topology).await;
    assert!(
        topology
            .forwarder
            .accept_link(&topology.ab.1, LEASE)
            .await
            .is_err()
    );
    tokio::time::timeout(Duration::from_secs(14), client.closed())
        .await
        .expect("original session must fail");
    assert!(client.open_bi().await.is_err());
    assert!(
        tokio::time::timeout(
            Duration::from_secs(14),
            topology.a.connect(target, BUSINESS)
        )
        .await
        .unwrap()
        .is_err()
    );
    echo(&topology.ab.0, &topology.ab.1).await;
    echo(&topology.cb.0, &topology.cb.1).await;
    println!(
        "MEMBER_DATA_PRIORITY_PASS disabled_routes=0 inner_original_failed=true outer_links_alive=true signal_business_fallback=false"
    );
    topology.close().await;
}

enum RelayLoss {
    Disabled,
    Disconnected,
    Removed,
    StreamInterrupted,
}
async fn backup_survives(loss: RelayLoss) {
    let mut topology = Topology::with_options(LEASE, true).await;
    let (client, server) = business(&topology).await;
    let ids = (client.stable_id(), server.stable_id());
    let (mut send, _recv) = client.open_bi().await.unwrap();
    send.write_all(b"before").await.unwrap();
    let (_send, mut recv) = server.accept_bi().await.unwrap();
    let mut bytes = [0; 6];
    recv.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"before");
    topology
        .a
        .open_data_path(&client, *topology.c.addr().ip_addrs().next().unwrap())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !client.paths().iter().any(|p| p.is_ip()) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(client.paths().iter().any(|p| p.is_selected() && matches!(p.remote_addr(), TransportAddr::Custom(a) if a.id() == miopunch_native_probe::member_transport::DATA_TRANSPORT_ID)));
    match loss {
        RelayLoss::Disabled => topology.forwarder.set_enabled(false),
        RelayLoss::Disconnected => topology.b.close().await,
        RelayLoss::Removed => {
            let target = topology._nodes.1.status().node_id;
            let mut session = topology._nodes.0.begin_management_session();
            let result = topology
                ._nodes
                .0
                .manage(
                    &mut session,
                    miopunch_native_probe::daily_management::ManagementAction::RemoveMember(target),
                )
                .await
                .unwrap();
            let record = topology
                ._nodes
                .0
                .management_record(result.record)
                .await
                .unwrap()
                .unwrap();
            topology
                ._nodes
                .2
                .ingest_management_record(record.header(), record.body())
                .await
                .unwrap();
            topology
                ._nodes
                .1
                .ingest_management_record(record.header(), record.body())
                .await
                .unwrap();
        }
        RelayLoss::StreamInterrupted => {
            // The owned B-facing A stream is the physical member-circuit carrier.
            drop(topology._leases.remove(1));
        }
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        while topology.at.address().is_ok() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the affected member carrier withdraws");
    tokio::time::timeout(Duration::from_secs(5), async {
        send.write_all(b"after").await.unwrap();
        let mut bytes = [0; 5];
        recv.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"after");
    })
    .await
    .expect("the same stream continues on its existing allowed backup");
    assert_eq!((client.stable_id(), server.stable_id()), ids);
    assert!(client.paths().iter().any(|p| p.is_ip() && p.is_selected()));
    println!(
        "MEMBER_DATA_BACKUP_PASS original_ids_unchanged=true same_stream=true retired_member_candidate=true"
    );
    topology.close().await;
}

#[tokio::test]
async fn disabling_member_relay_keeps_the_original_connection_and_stream_on_an_allowed_backup() {
    backup_survives(RelayLoss::Disabled).await;
}
#[tokio::test]
async fn disconnected_b_keeps_the_original_connection_and_stream_on_an_allowed_backup() {
    backup_survives(RelayLoss::Disconnected).await;
}
#[tokio::test]
async fn removing_b_retires_its_routes_and_preserves_the_original_ac_stream_on_an_allowed_backup() {
    backup_survives(RelayLoss::Removed).await;
}
#[tokio::test]
async fn interrupted_member_circuit_keeps_the_original_ac_stream_on_an_allowed_backup() {
    backup_survives(RelayLoss::StreamInterrupted).await;
}

#[tokio::test]
async fn ordinary_member_data_service_is_disabled_by_default_and_bootstrap_is_insufficient() {
    use miopunch_native_probe::admission_gate::CONTROL_ALPN;
    let topology = Topology::new().await;
    let default = MemberDataRelay::new(topology._nodes.1.admission_gate());
    assert!(default.accept_link(&topology.ab.1, LEASE).await.is_err());
    assert_eq!(default.active_links(), 0);
    topology
        .b
        .set_alpns(vec![BUSINESS.to_vec(), CONTROL_ALPN.to_vec()]);
    let (client, server) =
        native_connections::control_pair(&topology.a, &topology.b, CONTROL_ALPN).await;
    assert!(
        !topology
            ._nodes
            .1
            .admission_gate()
            .allows(&server, ProtocolScope::Member)
    );
    // Control capabilities cannot be passed to member links; the public compiler
    // consumer native_member_links_control preserves those rejection checks.
    assert!(
        !topology
            ._nodes
            .0
            .admission_gate()
            .allows(&client, ProtocolScope::Member)
    );
    echo(&topology.ab.0, &topology.ab.1).await;
    println!(
        "MEMBER_DATA_DEFAULT_PASS explicit_consent_required=true bootstrap_cannot_relay=true ordinary_b=true"
    );
    client.close(0u32.into(), b"done");
    server.close(0u32.into(), b"done");
    topology.close().await;
}
#[cfg(feature = "member-signaling-test-hooks")]
#[tokio::test]
async fn b_has_separate_outer_tls_keys_and_cannot_substitute_the_inner_endpoint_identity() {
    let topology = Topology::new().await;
    let (client, server) = business(&topology).await;
    let mut ac = [0; 32];
    let mut ca = [0; 32];
    let mut ba = [0; 32];
    let mut bc = [0; 32];
    client
        .export_keying_material(&mut ac, b"EXPORTER-member-data-relay-test", b"")
        .unwrap();
    server
        .export_keying_material(&mut ca, b"EXPORTER-member-data-relay-test", b"")
        .unwrap();
    topology
        .ab
        .1
        .export_keying_material(&mut ba, b"EXPORTER-member-data-relay-test", b"")
        .unwrap();
    topology
        .cb
        .1
        .export_keying_material(&mut bc, b"EXPORTER-member-data-relay-test", b"")
        .unwrap();
    assert_eq!(ac, ca);
    assert_ne!(ac, ba);
    assert_ne!(ac, bc);
    echo(&client, &server).await;
    assert!(topology.forwarder.test_counts().0 > 0);
    let observed = topology.forwarder.test_observed_datagrams();
    assert!(!observed.is_empty() && observed.len() <= 64);
    for packet in observed {
        assert!(
            !packet
                .windows(b"private A-C business".len())
                .any(|part| part == b"private A-C business")
        );
    }
    let mut forged = topology.target();
    forged.id = topology.b.id();
    let (out, incoming) = tokio::time::timeout(Duration::from_secs(12), async {
        tokio::join!(topology.a.connect(forged, BUSINESS), async {
            topology.c.accept().await.unwrap().await
        })
    })
    .await
    .unwrap();
    assert!(out.is_err() && incoming.is_err());
    echo(&client, &server).await;
    println!(
        "MEMBER_DATA_SECURITY_PASS real_ac_tls_identity=true inner_exporter_distinct_from_b_outer_sessions=true forged_target_rejected=true opaque_forwarding_observed=true"
    );
    topology.close().await;
}
#[tokio::test]
async fn expired_data_routes_cannot_revive_after_reenabling_the_service() {
    let topology = Topology::with_lease(Duration::from_millis(400)).await;
    let target = topology.target();
    tokio::time::sleep(Duration::from_millis(600)).await;
    routes_stopped(&topology).await;
    topology.forwarder.set_enabled(false);
    topology.forwarder.set_enabled(true);
    assert!(
        tokio::time::timeout(
            Duration::from_secs(14),
            topology.a.connect(target, BUSINESS)
        )
        .await
        .unwrap()
        .is_err()
    );
    echo(&topology.ab.0, &topology.ab.1).await;
    println!(
        "MEMBER_DATA_EXPIRY_PASS stale_route_rejected=true no_implicit_reregistration=true outer_link_alive=true"
    );
    topology.close().await;
}

#[tokio::test]
async fn an_inner_member_relay_connection_cannot_be_used_as_another_relay_outer_link() {
    let topology = Topology::new().await;
    let (client, server) = business(&topology).await;
    let nested = MemberDataRelay::new(topology._nodes.0.admission_gate());
    nested.set_enabled(true);
    assert!(nested.accept_link(&client, LEASE).await.is_err());
    let adapter = MemberDataTransport::new(topology.c.id(), topology._nodes.0.admission_gate());
    assert!(adapter.attach(&client, LEASE).await.is_err());
    echo(&client, &server).await;
    println!("MEMBER_DATA_ONE_HOP_PASS nested_circuit_denied=true original_business_alive=true");
    topology.close().await;
}
#[cfg(feature = "member-signaling-test-hooks")]
async fn release_a_data_registration(topology: &mut Topology) {
    drop(topology._leases.remove(1));
    drop(topology._leases.remove(0));
    tokio::time::timeout(Duration::from_secs(2), async {
        while topology.forwarder.active_links() != 1 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
#[cfg(feature = "member-signaling-test-hooks")]
async fn raw_link(
    topology: &Topology,
    token: [u8; 16],
) -> (
    iroh::endpoint::SendStream,
    iroh::endpoint::RecvStream,
    SignalingLease,
) {
    let (stream, lease) = tokio::join!(
        async {
            let (mut send, mut recv) = topology.ab.0.open_bi().await.unwrap();
            let mut hello = b"MID1".to_vec();
            hello.extend_from_slice(&token);
            send.write_all(&hello).await.unwrap();
            let mut ack = [0; 4];
            recv.read_exact(&mut ack).await.unwrap();
            assert_eq!(&ack, b"MID1");
            (send, recv)
        },
        topology.forwarder.accept_link(&topology.ab.1, LEASE)
    );
    let (send, recv) = stream;
    (send, recv, lease.unwrap())
}
#[cfg(feature = "member-signaling-test-hooks")]
#[tokio::test]
async fn forged_destinations_and_retired_tokens_cannot_redirect_member_data() {
    let mut topology = Topology::new().await;
    release_a_data_registration(&mut topology).await;
    let token = [0x41; 16];
    let (mut send, _recv, lease) = raw_link(&topology, token).await;
    let before = topology.forwarder.test_counts();
    let mut frame = vec![0xff; 16];
    frame.extend_from_slice(&1u16.to_be_bytes());
    frame.push(7);
    send.write_all(&frame).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while topology.forwarder.test_counts().1 == before.1 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(topology.forwarder.test_counts().0, before.0);
    drop(lease);
    tokio::time::timeout(Duration::from_secs(2), async {
        while topology.forwarder.active_links() != 1 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let (mut send, mut recv) = topology.ab.0.open_bi().await.unwrap();
    let mut hello = b"MID1".to_vec();
    hello.extend_from_slice(&token);
    send.write_all(&hello).await.unwrap();
    assert!(
        topology
            .forwarder
            .accept_link(&topology.ab.1, LEASE)
            .await
            .is_err()
    );
    let mut ack = [0; 4];
    assert!(recv.read_exact(&mut ack).await.is_err());
    assert_eq!(topology.forwarder.active_links(), 1);
    println!(
        "MEMBER_DATA_ROUTE_PASS unknown_destination_forwarded=0 retired_registration_rejected=true source_bound_to_outer_peer=true"
    );
    topology.close().await;
}
#[cfg(feature = "member-signaling-test-hooks")]
#[tokio::test]
async fn malformed_and_over_budget_member_frames_have_bounded_forwarding() {
    let mut topology = Topology::new().await;
    release_a_data_registration(&mut topology).await;
    let (mut send, _recv, lease) = raw_link(&topology, [0x51; 16]).await;
    let mut frame = vec![0xff; 16];
    frame.extend_from_slice(&2048u16.to_be_bytes());
    frame.extend_from_slice(&[7; 2048]);
    for _ in 0..2048 {
        send.write_all(&frame).await.unwrap();
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        while topology.forwarder.test_counts().2 == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("actual source rate budget drops excess datagrams");
    assert_eq!(topology.forwarder.test_counts().0, 0);
    // An oversized length ends the reserved stream before payload allocation.
    let mut oversized = vec![0xff; 16];
    oversized.extend_from_slice(&2049u16.to_be_bytes());
    send.write_all(&oversized).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !lease.is_finished() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(topology.forwarder.active_links(), 1);
    echo(&topology.ab.0, &topology.ab.1).await;
    println!(
        "MEMBER_DATA_BOUNDS_PASS rate_drops_observed=true unknown_target_forwarded=0 oversized_frame_rejected=true member_link_alive=true queue_bound=64 datagram_bound=2048 active_links_bound=16"
    );
    topology.close().await;
}

#[cfg(feature = "member-signaling-test-hooks")]
#[tokio::test]
async fn explicit_consumer_data_relay_restriction_blocks_the_route_at_actual_send() {
    let topology = Topology::with_configuration(LEASE, false, true).await;
    let before = topology.forwarder.test_counts();
    assert!(
        tokio::time::timeout(
            Duration::from_secs(14),
            topology.a.connect(topology.target(), BUSINESS)
        )
        .await
        .unwrap()
        .is_err()
    );
    assert_eq!(
        topology.forwarder.test_counts(),
        before,
        "forbidden candidate must cause no B data forwarding"
    );
    echo(&topology.ab.0, &topology.ab.1).await;
    println!(
        "MEMBER_DATA_RESOURCE_PASS explicit_candidate_denied=true b_forwarded_datagrams=0 outer_link_alive=true"
    );
    topology.close().await;
}

#[cfg(feature = "member-signaling-test-hooks")]
#[tokio::test]
async fn live_signaling_service_never_becomes_business_fallback_when_b_disables_data_relay() {
    let topology = Topology::with_signal_configuration(LEASE, false, false, true).await;
    let (client, server) = business(&topology).await;
    echo(&client, &server).await;
    let (at, ct, signal) = topology.signal.as_ref().unwrap();
    assert_eq!(signal.active_links(), 2);
    topology.forwarder.set_enabled(false);
    routes_stopped(&topology).await;
    let signal_count = signal.test_observed_datagrams().len();
    let (mut send, _recv) = client.open_bi().await.unwrap();
    send.write_all(&[0x53; 65536]).await.unwrap();
    tokio::time::timeout(Duration::from_secs(14), client.closed())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(14), server.closed())
        .await
        .unwrap();
    assert!(at.address().is_ok() && ct.address().is_ok());
    assert_eq!(signal.active_links(), 2);
    assert_eq!(
        signal.test_observed_datagrams().len(),
        signal_count,
        "queued business must not produce signaling-carrier datagrams"
    );
    // The independent real signaling path still reaches the inner TLS/control
    // exchange, but no permitted A-C data carrier remains for normal release.
    let target = EndpointAddr::new(topology.c.id())
        .with_addrs([TransportAddr::Custom(ct.address().unwrap())]);
    let (out, incoming) = tokio::time::timeout(Duration::from_secs(14), async {
        tokio::join!(topology.a.connect(target, BUSINESS), async {
            topology.c.accept().await.unwrap().await
        })
    })
    .await
    .unwrap();
    assert!(out.is_err() && incoming.is_err());
    assert!(signal.test_observed_datagrams().len() > signal_count);
    echo(&topology.ab.0, &topology.ab.1).await;
    println!(
        "MEMBER_DATA_SIGNAL_ISOLATION_PASS live_signaling_routes=2 queued_business_on_signal=0 restricted_attempt_reachable=true business_release=false"
    );
    topology.close().await;
}
