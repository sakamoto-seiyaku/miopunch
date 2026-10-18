//! Management through the public node, real admission, TLS/QUIC and durable replicas.
#[path = "support/native_connections.rs"]
mod native_connections;
use iroh::endpoint::RestrictedEndpoint;

use std::net::Ipv4Addr;
use std::path::Path;
use std::time::Duration;

use iroh::endpoint::Connection;
use miopunch_native_probe::admission_gate::{ADMISSION_REQUEST_ALPN, ProtocolScope};
use miopunch_native_probe::daily_management::{ManagementAction, ManagementError};
use miopunch_native_probe::genesis_recovery::GenesisCredential;
use miopunch_native_probe::management_auth::ManagementExtension;
use miopunch_native_probe::manual_admission::ManualExchange;
use miopunch_native_probe::native_network::NativeNetwork;
use miopunch_native_probe::offline_replica::REPLICA_ALPN;
use p2panda_auth::Access;
use p2panda_core::{Hash, Header, SigningKey};

#[cfg(feature = "replica-test-hooks")]
fn signed_action(
    node: &NativeNetwork,
    path: &Path,
    action: p2panda_auth::group::GroupAction<p2panda_core::VerifyingKey, ()>,
    dependencies: Vec<Hash>,
) -> Vec<u8> {
    let author = SigningKey::from_bytes(
        &std::fs::read(path.join("node.key"))
            .unwrap()
            .try_into()
            .unwrap(),
    );
    Header::<ManagementExtension>::builder()
        .build(
            &author,
            ManagementExtension {
                version: 1,
                log_id: rand::random(),
                group: p2panda_auth::GroupsExtensionArgs {
                    group_id: node.status().anchor.root,
                    action,
                    dependencies,
                },
            },
        )
        .encode()
}

#[cfg(feature = "replica-test-hooks")]
async fn causal_heads(node: &NativeNetwork) -> Vec<Hash> {
    let history = node.management_history().await.unwrap();
    let mut ids: Vec<_> = history
        .iter()
        .map(|record| Hash::digest(record.header()))
        .collect();
    for record in history {
        let header = Header::<ManagementExtension>::decode(record.header()).unwrap();
        ids.retain(|id| !header.extensions.group.dependencies.contains(id));
    }
    ids
}

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

#[tokio::test]
async fn daily_management_and_elevated_appointment_are_verified_on_remote_node() {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("creator");
    let bp = dir.path().join("member");
    let mut creator = NativeNetwork::create(&cp).await.unwrap();
    let mut member = join(&mut creator, &bp).await;
    let target = SigningKey::generate().verifying_key();
    let mut session = creator.begin_management_session();
    let added = creator
        .manage(&mut session, ManagementAction::AddMember(target))
        .await
        .unwrap();
    assert_eq!(added.effective_access, Some(Access::read()));
    let record = creator
        .management_record(added.record)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        Header::<ManagementExtension>::decode(record.header())
            .unwrap()
            .verifying_key,
        creator.status().node_id
    );
    let removed = creator
        .manage(&mut session, ManagementAction::RemoveMember(target))
        .await
        .unwrap();
    assert_eq!(removed.effective_access, None);
    creator
        .elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    let appointed = creator
        .manage(
            &mut session,
            ManagementAction::AppointAdministrator(member.status().node_id),
        )
        .await
        .unwrap();
    assert_eq!(appointed.effective_access, Some(Access::manage()));
    let record = creator
        .management_record(appointed.record)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        Header::<ManagementExtension>::decode(record.header())
            .unwrap()
            .verifying_key,
        creator.status().anchor.root
    );
    replicate(&creator, &mut member).await;
    assert!(
        member
            .status()
            .members
            .contains(&(member.status().node_id, Access::manage()))
    );
    member.close().await;
    let member = NativeNetwork::open(&bp).await.unwrap();
    assert!(
        member
            .status()
            .members
            .contains(&(member.status().node_id, Access::manage()))
    );
    member.close().await;
    creator.close().await;
}

#[tokio::test]
async fn expired_elevation_and_other_device_or_session_cannot_reuse_the_window() {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("creator");
    let bp = dir.path().join("other-device");
    let mut creator = NativeNetwork::create(&cp).await.unwrap();
    let mut other = join(&mut creator, &bp).await;
    let mut session = creator.begin_management_session();
    creator
        .elevate_genesis(&mut session, credential(&cp), Duration::from_millis(30))
        .unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
    let before = creator.status();
    assert!(matches!(
        creator
            .manage(
                &mut session,
                ManagementAction::AppointAdministrator(other.status().node_id)
            )
            .await,
        Err(ManagementError::ElevationRequired)
    ));
    assert_eq!(creator.status(), before);
    assert_eq!(session.elevation_remaining(), None);

    creator
        .elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    creator
        .manage(
            &mut session,
            ManagementAction::AppointAdministrator(other.status().node_id),
        )
        .await
        .unwrap();
    let target = SigningKey::generate().verifying_key();
    creator
        .manage(&mut session, ManagementAction::AppointAdministrator(target))
        .await
        .unwrap();
    // The same verified window signs repeated actions; its root signatures
    // propagate, but its local signing ability does not.
    replicate(&creator, &mut other).await;
    let before = other.status();
    let another_target = SigningKey::generate().verifying_key();
    assert!(matches!(
        other
            .manage(
                &mut session,
                ManagementAction::AppointAdministrator(another_target)
            )
            .await,
        Err(ManagementError::InvalidSession)
    ));
    let mut other_session = other.begin_management_session();
    assert!(matches!(
        other
            .manage(
                &mut other_session,
                ManagementAction::AppointAdministrator(another_target)
            )
            .await,
        Err(ManagementError::ElevationRequired)
    ));
    assert_eq!(other.status(), before);
    let mut second_session = creator.begin_management_session();
    assert!(matches!(
        creator
            .manage(
                &mut second_session,
                ManagementAction::AppointAdministrator(another_target)
            )
            .await,
        Err(ManagementError::ElevationRequired)
    ));

    session.exit_genesis_elevation();
    assert!(matches!(
        creator
            .manage(
                &mut session,
                ManagementAction::AppointAdministrator(another_target)
            )
            .await,
        Err(ManagementError::ElevationRequired)
    ));
    creator
        .elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    session.end();
    assert!(matches!(
        creator
            .manage(
                &mut session,
                ManagementAction::AppointAdministrator(another_target)
            )
            .await,
        Err(ManagementError::InvalidSession)
    ));
    assert!(matches!(
        creator.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60)),
        Err(ManagementError::InvalidSession)
    ));

    let mut live_session = creator.begin_management_session();
    creator
        .elevate_genesis(&mut live_session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    creator.close().await;
    let mut creator = NativeNetwork::open(&cp).await.unwrap();
    assert!(matches!(
        creator
            .manage(
                &mut live_session,
                ManagementAction::AppointAdministrator(another_target)
            )
            .await,
        Err(ManagementError::InvalidSession)
    ));
    let mut reopened_session = creator.begin_management_session();
    assert!(matches!(
        creator
            .manage(
                &mut reopened_session,
                ManagementAction::AppointAdministrator(another_target)
            )
            .await,
        Err(ManagementError::ElevationRequired)
    ));
    other.close().await;
    creator.close().await;
}

#[tokio::test]
async fn demotion_keeps_membership_and_separate_admin_removal_survives_offline_forwarding() {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("creator");
    let bp = dir.path().join("forwarder");
    let ap = dir.path().join("offline-member");
    let mut creator = NativeNetwork::create(&cp).await.unwrap();
    let mut b = join(&mut creator, &bp).await;
    let mut a = join(&mut creator, &ap).await;
    let b_id = b.status().node_id;
    let removed_admin = SigningKey::generate().verifying_key();
    let mut session = creator.begin_management_session();
    assert!(
        matches!(
            creator
                .manage(
                    &mut session,
                    ManagementAction::RemoveMember(creator.status().node_id)
                )
                .await,
            Err(ManagementError::ElevationRequired)
        ),
        "an administrator cannot use daily self-removal to bypass genesis"
    );
    creator
        .elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    creator
        .manage(&mut session, ManagementAction::AppointAdministrator(b_id))
        .await
        .unwrap();
    creator
        .manage(
            &mut session,
            ManagementAction::AppointAdministrator(removed_admin),
        )
        .await
        .unwrap();
    session.exit_genesis_elevation();
    assert!(matches!(
        creator
            .manage(&mut session, ManagementAction::RemoveMember(removed_admin))
            .await,
        Err(ManagementError::ElevationRequired)
    ));
    creator
        .elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    let demoted = creator
        .manage(&mut session, ManagementAction::DemoteAdministrator(b_id))
        .await
        .unwrap();
    assert_eq!(demoted.action, ManagementAction::DemoteAdministrator(b_id));
    assert_eq!(demoted.effective_access, Some(Access::read()));
    let removed = creator
        .manage(&mut session, ManagementAction::RemoveMember(removed_admin))
        .await
        .unwrap();
    assert_eq!(
        removed.action,
        ManagementAction::RemoveMember(removed_admin)
    );
    assert_eq!(removed.effective_access, None);
    session.exit_genesis_elevation();
    replicate(&creator, &mut b).await;
    let original_demotion = creator
        .management_record(demoted.record)
        .await
        .unwrap()
        .unwrap();
    let original_removal = creator
        .management_record(removed.record)
        .await
        .unwrap()
        .unwrap();
    creator.close().await;
    b.close().await;
    let mut b = NativeNetwork::open(&bp).await.unwrap();
    assert!(!bp.join("genesis.credential").exists());
    assert!(b.status().members.contains(&(b_id, Access::read())));
    let mut b_session = b.begin_management_session();
    assert!(
        b.manage(
            &mut b_session,
            ManagementAction::AddMember(SigningKey::generate().verifying_key())
        )
        .await
        .is_err(),
        "demoted device retains membership but cannot perform daily administration"
    );
    replicate(&b, &mut a).await;
    assert_eq!(
        a.management_record(demoted.record).await.unwrap(),
        Some(original_demotion)
    );
    assert_eq!(
        a.management_record(removed.record).await.unwrap(),
        Some(original_removal)
    );
    assert!(a.status().members.contains(&(b_id, Access::read())));
    assert!(
        !a.status()
            .members
            .iter()
            .any(|(id, _)| *id == removed_admin)
    );
    a.close().await;
    let a = NativeNetwork::open(&ap).await.unwrap();
    assert!(a.status().members.contains(&(b_id, Access::read())));
    assert!(
        !a.status()
            .members
            .iter()
            .any(|(id, _)| *id == removed_admin)
    );
    a.close().await;
    b.close().await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn directly_signed_daily_admin_privilege_records_fail_locally_on_wire_and_after_restart() {
    use miopunch_native_probe::management_store::ManagementRecord;
    use p2panda_auth::group::{GroupAction, GroupMember};
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("daily-admin");
    let ap = dir.path().join("receiver");
    let bp = dir.path().join("third-party");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let mut a = join(&mut c, &ap).await;
    let mut b = join(&mut c, &bp).await;
    let mut session = c.begin_management_session();
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    c.manage(
        &mut session,
        ManagementAction::AppointAdministrator(b.status().node_id),
    )
    .await
    .unwrap();
    session.exit_genesis_elevation();
    let target = SigningKey::generate().verifying_key();
    let actions = [
        GroupAction::Add {
            member: GroupMember::Individual(target),
            access: Access::manage(),
        },
        GroupAction::Promote {
            member: GroupMember::Individual(a.status().node_id),
            access: Access::manage(),
        },
        GroupAction::Demote {
            member: GroupMember::Individual(b.status().node_id),
            access: Access::read(),
        },
        GroupAction::Remove {
            member: GroupMember::Individual(b.status().node_id),
        },
        GroupAction::Remove {
            member: GroupMember::Individual(c.status().node_id),
        },
    ];
    let expected = c.status();
    let mut denied_ids = Vec::new();
    for action in actions {
        // Bypass the convenience API and construct a genuine Ed25519 signed
        // header by a current daily administrator. Its author is still not root.
        let raw = signed_action(&c, &cp, action, causal_heads(&c).await);
        let id = Hash::digest(&raw);
        denied_ids.push(id);
        assert!(c.ingest_management_record(&raw, None).await.is_err());
        assert_eq!(c.status(), expected);
        assert!(c.management_record(id).await.unwrap().is_none());
        let mut fixture = c.management_history().await.unwrap();
        fixture.push(ManagementRecord::new(raw, None));
        let ce = endpoint(&c).await;
        let ae = endpoint(&a).await;
        let (client, server) = connect(&ae, &ce, REPLICA_ALPN).await;
        let (received, served) = tokio::join!(
            a.sync_replica(&client),
            c.serve_replica_fixture(&server, &fixture)
        );
        let outcome = received.unwrap();
        served.unwrap();
        assert!(outcome.transport_done && outcome.saved.contains(&id));
        assert!(!outcome.complete() && !outcome.applied.contains(&id));
        assert_eq!(a.status().members, expected.members);
        assert_eq!(a.status().applied_count, expected.applied_count);
        assert!(a.management_record(id).await.unwrap().is_none());
        client.close(0u32.into(), b"one rejected record");
        server.close(0u32.into(), b"one rejected record");
        ae.close().await;
        ce.close().await;
        a.close().await;
        a = NativeNetwork::open(&ap).await.unwrap();
        assert_eq!(a.status().members, expected.members);
        assert_eq!(a.status().applied_count, expected.applied_count);
    }
    c.close().await;
    replicate(&a, &mut b).await;
    assert_eq!(b.status().members, expected.members);
    for id in denied_ids {
        assert!(b.management_record(id).await.unwrap().is_none());
        assert!(
            !b.management_history()
                .await
                .unwrap()
                .iter()
                .any(|record| Hash::digest(record.header()) == id)
        );
    }
    b.close().await;
    let b = NativeNetwork::open(&bp).await.unwrap();
    assert_eq!(b.status().members, expected.members);
    a.close().await;
    b.close().await;
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
    let target = SigningKey::generate().verifying_key();
    let b_id = b.status().node_id;
    let mut root_session = c.begin_management_session();
    c.manage(&mut root_session, ManagementAction::AddMember(target))
        .await
        .unwrap();
    c.elevate_genesis(&mut root_session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    c.manage(
        &mut root_session,
        ManagementAction::AppointAdministrator(b_id),
    )
    .await
    .unwrap();
    replicate(&c, &mut b).await;
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
    replicate(&c, &mut a).await;
    assert!(a.status().members.contains(&(target, Access::read())));
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
async fn elevation_requires_independent_root_and_canonical_create_not_node_key_or_admission_secret()
{
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("creator");
    let mut node = NativeNetwork::create(&path).await.unwrap();
    let mut session = node.begin_management_session();
    let anchor = node.status().anchor;
    for filename in ["node.key", "admission.secret"] {
        let bytes = std::fs::read(path.join(filename))
            .unwrap()
            .try_into()
            .unwrap();
        assert!(matches!(
            node.elevate_genesis(
                &mut session,
                GenesisCredential::from_saved(bytes, anchor.genesis),
                Duration::from_secs(60)
            ),
            Err(ManagementError::InvalidCredential)
        ));
    }
    let root = std::fs::read(path.join("genesis.credential")).unwrap();
    assert!(matches!(
        node.elevate_genesis(
            &mut session,
            GenesisCredential::from_saved(
                root[..32].try_into().unwrap(),
                Hash::digest(b"another Create")
            ),
            Duration::from_secs(60)
        ),
        Err(ManagementError::InvalidCredential)
    ));
    assert!(matches!(
        node.elevate_genesis(&mut session, credential(&path), Duration::ZERO),
        Err(ManagementError::InvalidWindow)
    ));
    assert!(matches!(
        node.elevate_genesis(&mut session, credential(&path), Duration::MAX),
        Err(ManagementError::InvalidWindow)
    ));
    assert_eq!(session.elevation_remaining(), None);
    assert!(matches!(
        node.manage(
            &mut session,
            ManagementAction::AppointAdministrator(SigningKey::generate().verifying_key())
        )
        .await,
        Err(ManagementError::ElevationRequired)
    ));
    node.close().await;
}
