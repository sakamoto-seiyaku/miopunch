//! Management through the public node, real admission, TLS/QUIC and durable replicas.
#[path = "support/native_connections.rs"]
mod native_connections;
use iroh::endpoint::RestrictedEndpoint;

use std::net::Ipv4Addr;
use std::path::Path;
use std::time::Duration;

use iroh::{EndpointAddr, endpoint::Connection};
use miopunch_native_probe::admission_gate::{ADMISSION_REQUEST_ALPN, CONTROL_ALPN, ProtocolScope};
use miopunch_native_probe::daily_management::{ManagementAction, ManagementError};
use miopunch_native_probe::genesis_recovery::GenesisCredential;
use miopunch_native_probe::manual_admission::ManualExchange;
use miopunch_native_probe::native_network::NativeNetwork;
use miopunch_native_probe::offline_replica::REPLICA_ALPN;
use p2panda_auth::Access;
use p2panda_core::Hash;

fn credential(path: &Path) -> GenesisCredential {
    let saved = std::fs::read(path.join("genesis.credential")).unwrap();
    GenesisCredential::from_saved(
        saved[..32].try_into().unwrap(),
        Hash::from_bytes(saved[32..].try_into().unwrap()),
    )
}

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
    let receiver = async {
        let ManualExchange::Materials(material) =
            node.request_manual_admission(&client).await.unwrap()
        else {
            panic!("approved member must receive material")
        };
        node.complete_manual_admission(material).await.unwrap();
    };
    let (_, served) = tokio::join!(receiver, admin.serve_manual_admission(&server));
    served.unwrap();
    client.close(0u32.into(), b"admission done");
    server.close(0u32.into(), b"admission done");
    ne.close().await;
    ae.close().await;
    node
}

async fn endpoint(node: &NativeNetwork) -> RestrictedEndpoint {
    node.admission_gate()
        .register_protocol(REPLICA_ALPN, ProtocolScope::Member);
    let endpoint = node.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    endpoint.set_alpns(vec![REPLICA_ALPN.to_vec()]);
    endpoint
}

async fn replicate(source: &NativeNetwork, receiver: &mut NativeNetwork) {
    let se = endpoint(source).await;
    let re = endpoint(receiver).await;
    let (client, server) = connect(&re, &se, REPLICA_ALPN).await;
    let (received, served) = tokio::join!(
        receiver.sync_replica(&client),
        source.serve_replica(&server)
    );
    assert!(received.unwrap().complete());
    served.unwrap();
    client.close(0u32.into(), b"replica done");
    server.close(0u32.into(), b"replica done");
    re.close().await;
    se.close().await;
}

const BUSINESS: &[u8] = b"miopunch/revocation-business/1";

async fn business_endpoint(node: &NativeNetwork) -> RestrictedEndpoint {
    node.admission_gate()
        .register_protocol(BUSINESS, ProtocolScope::Member);
    let endpoint = node.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    node.admission_gate()
        .register_protocol(REPLICA_ALPN, ProtocolScope::Member);
    endpoint.set_alpns(vec![
        BUSINESS.to_vec(),
        ADMISSION_REQUEST_ALPN.to_vec(),
        CONTROL_ALPN.to_vec(),
        REPLICA_ALPN.to_vec(),
    ]);
    endpoint
}

#[tokio::test]
async fn received_removal_ends_actual_flow_and_rejects_new_connections_with_same_secret() {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("administrator");
    let ap = dir.path().join("receiver");
    let tp = dir.path().join("target");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let mut a = join(&mut c, &ap).await;
    let t = join(&mut c, &tp).await;
    replicate(&c, &mut a).await;
    let saved_secret = std::fs::read(tp.join("admission.material")).unwrap();
    let ae = business_endpoint(&a).await;
    let te = business_endpoint(&t).await;
    let (control_client, control_server) =
        native_connections::control_pair(&te, &ae, CONTROL_ALPN).await;
    let (replica_client, replica_server) = connect(&te, &ae, REPLICA_ALPN).await;
    let (request_client, request_server) =
        native_connections::control_pair(&te, &ae, ADMISSION_REQUEST_ALPN).await;
    let (client, server) = connect(&te, &ae, BUSINESS).await;
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"live").await.unwrap();
    let (_, mut recv) = server.accept_bi().await.unwrap();
    let mut bytes = [0; 4];
    recv.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"live");
    let mut session = c.begin_management_session();
    c.manage(
        &mut session,
        ManagementAction::RemoveMember(t.status().node_id),
    )
    .await
    .unwrap();
    replicate(&c, &mut a).await;
    tokio::time::timeout(Duration::from_secs(3), client.closed())
        .await
        .unwrap();
    assert!(server.close_reason().is_some());
    for (out, incoming) in [
        (control_client, control_server),
        (request_client, request_server),
    ] {
        tokio::time::timeout(Duration::from_secs(3), out.closed())
            .await
            .unwrap();
        assert!(incoming.close_reason().is_some());
    }
    tokio::time::timeout(Duration::from_secs(3), replica_client.closed())
        .await
        .unwrap();
    assert!(replica_server.close_reason().is_some());
    assert!(recv.read_exact(&mut bytes).await.is_err());
    assert!(!a.admission_gate().allows(&server, ProtocolScope::Member));
    let target = EndpointAddr::new(ae.id()).with_ip_addr(*ae.addr().ip_addrs().next().unwrap());
    let (out, incoming) = tokio::join!(te.connect(target, BUSINESS), async {
        ae.accept().await.unwrap().await
    });
    assert!(out.is_err() && incoming.is_err());
    for alpn in [CONTROL_ALPN, REPLICA_ALPN] {
        let target = EndpointAddr::new(ae.id()).with_ip_addr(*ae.addr().ip_addrs().next().unwrap());
        let (out, incoming) = tokio::join!(te.connect(target, alpn), async {
            ae.accept().await.unwrap().await
        });
        assert!(out.is_err() && incoming.is_err());
    }
    assert_eq!(
        std::fs::read(tp.join("admission.material")).unwrap(),
        saved_secret
    );
    println!(
        "actual established flow closed; stale secret held unchanged; new member connection rejected"
    );
    te.close().await;
    ae.close().await;
    t.close().await;
    a.close().await;
    c.close().await;
}

#[tokio::test]
async fn fresh_manual_approval_rejoins_stable_identity_but_old_request_cannot() {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("admin");
    let tp = dir.path().join("target");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let mut t = join(&mut c, &tp).await;
    let id = t.status().node_id;
    let old = c.manual_requests().await.unwrap().pop().unwrap();
    let old_result = c.approved_admission(&old).await.unwrap().unwrap();
    let saved = std::fs::read(tp.join("admission.material")).unwrap();
    let mut session = c.begin_management_session();
    let removed = c
        .manage(&mut session, ManagementAction::RemoveMember(id))
        .await
        .unwrap();
    let record = c.management_record(removed.record).await.unwrap().unwrap();
    t.ingest_management_record(record.header(), record.body())
        .await
        .unwrap();
    assert!(c.approve_manual_admission(&old).await.is_err());
    assert!(c.approved_admission(&old).await.is_err());
    let original = c
        .management_record(old_result.authorization)
        .await
        .unwrap()
        .unwrap();
    t.ingest_management_record(original.header(), original.body())
        .await
        .unwrap();
    assert!(!t.status().members.iter().any(|(node, _)| *node == id));
    let ce = business_endpoint(&c).await;
    let te = business_endpoint(&t).await;
    let (client, server) = native_connections::control_pair(&te, &ce, ADMISSION_REQUEST_ALPN).await;
    let (request, served) = tokio::join!(
        t.request_manual_admission(&client),
        c.serve_manual_admission(&server)
    );
    assert!(request.is_err() && served.is_err());
    t.begin_rejoin(c.manual_invitation().unwrap())
        .await
        .unwrap();
    te.close().await;
    t.close().await;
    let mut t = NativeNetwork::open(&tp).await.unwrap();
    assert_eq!(t.status().node_id, id);
    let te = business_endpoint(&t).await;
    let (client, server) = native_connections::control_pair(&te, &ce, ADMISSION_REQUEST_ALPN).await;
    let (request, served) = tokio::join!(
        t.request_manual_admission(&client),
        c.serve_manual_admission(&server)
    );
    assert!(matches!(request.unwrap(), ManualExchange::AwaitingApproval));
    served.unwrap();
    let fresh = c
        .manual_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.applicant == id && r.request_id != old.request_id)
        .unwrap();
    let approved = c.approve_manual_admission(&fresh).await.unwrap();
    assert_ne!(approved.authorization, old_result.authorization);
    let receiver = async {
        let ManualExchange::Materials(material) =
            t.request_manual_admission(&client).await.unwrap()
        else {
            panic!("new approval")
        };
        t.complete_manual_admission(material).await.unwrap();
    };
    let (_, served) = tokio::join!(receiver, c.serve_manual_admission(&server));
    served.unwrap();
    assert!(c.approved_admission(&old).await.is_err());
    assert_eq!(std::fs::read(tp.join("admission.material")).unwrap(), saved);
    assert_eq!(t.status().node_id, id);
    let (client, server) = connect(&te, &ce, BUSINESS).await;
    assert!(c.admission_gate().allows(&server, ProtocolScope::Member));
    client.close(0u32.into(), b"complete");
    te.close().await;
    ce.close().await;
    t.close().await;
    c.close().await;
    let t = NativeNetwork::open(&tp).await.unwrap();
    assert_eq!(t.status().node_id, id);
    assert_eq!(
        t.admission_stage(),
        miopunch_native_probe::manual_admission::AdmissionStage::Complete
    );
    t.close().await;
}
#[tokio::test]
async fn concurrent_admin_demotion_corrects_member_removal_in_both_wire_orders_and_replay() {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("root-device");
    let bp = dir.path().join("daily-admin");
    let ap = dir.path().join("removal-first");
    let dp = dir.path().join("demotion-first");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let mut b = join(&mut c, &bp).await;
    let mut a = join(&mut c, &ap).await;
    let mut d = join(&mut c, &dp).await;
    let t = join(&mut c, &dir.path().join("target")).await;
    let target = t.status().node_id;
    let b_id = b.status().node_id;
    let mut root_session = c.begin_management_session();
    c.elevate_genesis(&mut root_session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    c.manage(
        &mut root_session,
        ManagementAction::AppointAdministrator(b_id),
    )
    .await
    .unwrap();
    replicate(&c, &mut b).await;
    replicate(&c, &mut a).await;
    replicate(&c, &mut d).await;
    let ae = business_endpoint(&a).await;
    let te = business_endpoint(&t).await;
    let (old_client, old_server) = connect(&te, &ae, BUSINESS).await;
    let (mut send, _) = old_client.open_bi().await.unwrap();
    send.write_all(b"before correction").await.unwrap();
    send.finish().unwrap();
    let (_, mut recv) = old_server.accept_bi().await.unwrap();
    assert_eq!(recv.read_to_end(64).await.unwrap(), b"before correction");
    let mut daily_session = b.begin_management_session();
    let removed = b
        .manage(&mut daily_session, ManagementAction::RemoveMember(target))
        .await
        .unwrap();
    assert_eq!(removed.effective_access, None);
    // C has not received B's removal: its demotion is genuinely concurrent,
    // with signed dependencies taken from each node's own verified view.
    let demoted = c
        .manage(
            &mut root_session,
            ManagementAction::DemoteAdministrator(b_id),
        )
        .await
        .unwrap();
    replicate(&b, &mut a).await;
    assert!(!a.status().members.iter().any(|(id, _)| *id == target));
    tokio::time::timeout(Duration::from_secs(3), old_client.closed())
        .await
        .unwrap();
    assert!(old_server.close_reason().is_some());
    replicate(&c, &mut a).await;
    assert!(a.status().members.contains(&(target, Access::read())));
    let (fresh_client, fresh_server) = connect(&te, &ae, BUSINESS).await;
    assert_ne!(fresh_client.stable_id(), old_client.stable_id());
    assert!(
        a.admission_gate()
            .allows(&fresh_server, ProtocolScope::Member)
    );
    assert!(old_client.close_reason().is_some());
    assert!(old_server.close_reason().is_some());
    fresh_client.close(0u32.into(), b"new connection only");
    assert!(a.status().members.contains(&(b_id, Access::read())));
    replicate(&c, &mut d).await;
    assert!(d.status().members.contains(&(target, Access::read())));
    replicate(&b, &mut d).await;
    assert_eq!(a.status().members, d.status().members);
    let applied = a.status().applied_count;
    replicate(&b, &mut a).await;
    assert_eq!(a.status().applied_count, applied);
    assert!(
        a.management_record(removed.record).await.unwrap().is_some(),
        "a causally filtered, originally valid action remains verifiable history"
    );
    assert!(a.management_record(demoted.record).await.unwrap().is_some());
    te.close().await;
    ae.close().await;
    t.close().await;
    a.close().await;
    d.close().await;
    let a = NativeNetwork::open(&ap).await.unwrap();
    let d = NativeNetwork::open(&dp).await.unwrap();
    assert_eq!(a.status().members, d.status().members);
    assert!(a.status().members.contains(&(target, Access::read())));
    assert!(a.status().members.contains(&(b_id, Access::read())));
    a.close().await;
    d.close().await;
    b.close().await;
    c.close().await;
}

#[tokio::test]
async fn offline_retains_roles_but_self_departure_closes_flows_and_admin_requires_elevation() {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("admin");
    let tp = dir.path().join("member");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let t = join(&mut c, &tp).await;
    let c_id = c.status().node_id;
    let t_id = t.status().node_id;
    let expected = t.status();
    c.close().await;
    t.close().await;
    let mut c = NativeNetwork::open(&cp).await.unwrap();
    let mut t = NativeNetwork::open(&tp).await.unwrap();
    assert_eq!(t.status(), expected);
    assert!(c.status().members.contains(&(c_id, Access::manage())));
    let ce = business_endpoint(&c).await;
    let te = business_endpoint(&t).await;
    let (client, server) = connect(&te, &ce, BUSINESS).await;
    let mut session = t.begin_management_session();
    let departure = t
        .manage(&mut session, ManagementAction::RemoveMember(t_id))
        .await
        .unwrap();
    assert_eq!(departure.effective_access, None);
    tokio::time::timeout(Duration::from_secs(3), server.closed())
        .await
        .unwrap();
    assert!(client.close_reason().is_some());
    let record = t
        .management_record(departure.record)
        .await
        .unwrap()
        .unwrap();
    c.ingest_management_record(record.header(), record.body())
        .await
        .unwrap();
    assert!(!c.status().members.iter().any(|(id, _)| *id == t_id));
    let mut session = c.begin_management_session();
    assert!(matches!(
        c.manage(&mut session, ManagementAction::RemoveMember(c_id))
            .await,
        Err(ManagementError::ElevationRequired)
    ));
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_millis(20))
        .unwrap();
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert!(matches!(
        c.manage(&mut session, ManagementAction::RemoveMember(c_id))
            .await,
        Err(ManagementError::ElevationRequired)
    ));
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    assert_eq!(
        c.manage(&mut session, ManagementAction::RemoveMember(c_id))
            .await
            .unwrap()
            .effective_access,
        None
    );
    te.close().await;
    ce.close().await;
    t.close().await;
    c.close().await;
    let t = NativeNetwork::open(&tp).await.unwrap();
    let c = NativeNetwork::open(&cp).await.unwrap();
    assert!(!t.status().members.iter().any(|(id, _)| *id == t_id));
    assert!(!c.status().members.iter().any(|(id, _)| *id == c_id));
    t.close().await;
    c.close().await;
}

#[tokio::test]
async fn a_departed_creator_can_prepare_durable_rejoin_without_losing_recovery_material() {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("creator");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let invitation = c.manual_invitation().unwrap();
    let before = c.status();
    let saved_root = std::fs::read(cp.join("genesis.credential")).unwrap();
    assert!(c.begin_rejoin(invitation).await.is_err());
    let mut session = c.begin_management_session();
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    c.manage(&mut session, ManagementAction::RemoveMember(before.node_id))
        .await
        .unwrap();
    c.begin_rejoin(invitation).await.unwrap();
    c.close().await;
    let c = NativeNetwork::open(&cp).await.unwrap();
    assert_eq!(c.status().node_id, before.node_id);
    assert_eq!(c.status().anchor, before.anchor);
    assert_eq!(
        std::fs::read(cp.join("genesis.credential")).unwrap(),
        saved_root
    );
    assert_eq!(
        c.admission_stage(),
        miopunch_native_probe::manual_admission::AdmissionStage::AwaitingApproval
    );
    c.close().await;
}
