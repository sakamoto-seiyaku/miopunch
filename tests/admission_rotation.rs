//! Admission-secret rotation through durable public nodes and real TLS/QUIC.
use iroh::{Endpoint, EndpointAddr, endpoint::Connection};
use miopunch_native_probe::admission_gate::ADMISSION_REQUEST_ALPN;
use miopunch_native_probe::admission_rotation::{ADMISSION_ROTATION_ALPN, RotationStage};
use miopunch_native_probe::genesis_recovery::GenesisCredential;
use miopunch_native_probe::manual_admission::ManualExchange;
use miopunch_native_probe::native_network::NativeNetwork;
use p2panda_core::Hash;
use std::net::Ipv4Addr;
use std::path::Path;
use std::time::Duration;

fn credential(path: &Path) -> GenesisCredential {
    let saved = std::fs::read(path.join("genesis.credential")).unwrap();
    GenesisCredential::from_saved(
        saved[..32].try_into().unwrap(),
        Hash::from_bytes(saved[32..].try_into().unwrap()),
    )
}

async fn connect(a: &Endpoint, b: &Endpoint, alpn: &[u8]) -> (Connection, Connection) {
    tokio::time::timeout(Duration::from_secs(10), async {
        let address = EndpointAddr::new(b.id()).with_ip_addr(*b.addr().ip_addrs().next().unwrap());
        let (a, b) = tokio::join!(a.connect(address, alpn), async {
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

async fn distribute(initiator: &mut NativeNetwork, member: &mut NativeNetwork) {
    let ie = initiator.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let me = member.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = connect(&ie, &me, ADMISSION_ROTATION_ALPN).await;
    let (sent, received) = tokio::join!(
        initiator.distribute_admission_rotation(&client),
        member.receive_admission_rotation(&server)
    );
    sent.unwrap();
    received.unwrap();
    ie.close().await;
    me.close().await;
}

#[tokio::test]
async fn two_retained_members_save_ack_switch_and_reopen_with_stable_identities() {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("creator");
    let ap = dir.path().join("member-a");
    let bp = dir.path().join("member-b");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let mut a = join(&mut c, &ap).await;
    let mut b = join(&mut c, &bp).await;
    let identities = [c.status().node_id, a.status().node_id, b.status().node_id];
    let anchor = c.status().anchor;
    let mut session = c.begin_management_session();
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    let initial = c.begin_admission_rotation(&mut session).await.unwrap();
    assert_eq!(initial.generation, 1);
    assert_eq!(
        initial.retained.len(),
        3,
        "virtual root is not an online member"
    );
    assert_eq!(initial.acknowledged, vec![identities[0]]);
    distribute(&mut c, &mut a).await;
    distribute(&mut c, &mut b).await;
    let ready = c.rotation_progress().await.unwrap().unwrap();
    assert_eq!(ready.acknowledged.len(), 3);
    let complete = c.finish_admission_rotation(&mut session).await.unwrap();
    assert_eq!(complete.stage, RotationStage::Complete);
    distribute(&mut c, &mut a).await;
    distribute(&mut c, &mut b).await;
    c.close().await;
    a.close().await;
    b.close().await;
    let mut c = NativeNetwork::open(&cp).await.unwrap();
    let mut a = NativeNetwork::open(&ap).await.unwrap();
    let b = NativeNetwork::open(&bp).await.unwrap();
    for (node, id) in [
        (&c, identities[0]),
        (&a, identities[1]),
        (&b, identities[2]),
    ] {
        assert_eq!(node.status().node_id, id);
        assert_eq!(node.status().anchor, anchor);
        let progress = node.rotation_progress().await.unwrap().unwrap();
        assert_eq!(progress.id, initial.id);
        assert_eq!(progress.stage, RotationStage::Complete);
    }
    // Idempotent completion on a fresh, new-generation SCRAM-bound connection.
    distribute(&mut c, &mut a).await;
    eprintln!(
        "MIO37_FIRST retained=3 external_retained=2 durable_ack=3 generation=1 reopened=true stable_identity=true new_secret_connection=true"
    );
    c.close().await;
    a.close().await;
    b.close().await;
}

#[cfg(feature = "replica-test-hooks")]
#[test]
fn rotation_checkpoint_worker() {
    let Ok(path) = std::env::var("MIO37_NODE_PATH") else {
        return;
    };
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut node = NativeNetwork::open(&path).await.unwrap();
        node.set_replica_checkpoint_observer(|checkpoint| {
            if format!("{checkpoint:?}") == std::env::var("MIO37_CHECKPOINT").unwrap() {
                eprintln!("MIO37_CRASH actual_exit=87 checkpoint={checkpoint:?}");
                std::process::exit(87);
            }
        });
        let endpoint = node.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
        let public = format!(
            "{}\n{}",
            endpoint.id(),
            endpoint.addr().ip_addrs().next().unwrap()
        );
        std::fs::write(std::env::var("MIO37_PUBLIC_PATH").unwrap(), public).unwrap();
        let connection = endpoint.accept().await.unwrap().await.unwrap();
        if std::env::var("MIO37_ROLE").unwrap() == "begin" {
            let mut session = node.begin_management_session();
            node.elevate_genesis(
                &mut session,
                credential(Path::new(&path)),
                Duration::from_secs(60),
            )
            .unwrap();
            node.begin_admission_rotation(&mut session).await.unwrap();
        } else if std::env::var("MIO37_ROLE").unwrap() == "finish" {
            let mut session = node.begin_management_session();
            node.elevate_genesis(
                &mut session,
                credential(Path::new(&path)),
                Duration::from_secs(60),
            )
            .unwrap();
            node.finish_admission_rotation(&mut session).await.unwrap();
        } else if std::env::var("MIO37_ROLE").unwrap() == "root-recovery" {
            node.restore_management(&connection).await.unwrap();
        } else if std::env::var("MIO37_ROLE").unwrap() == "initiator" {
            node.distribute_admission_rotation(&connection)
                .await
                .unwrap();
        } else {
            node.receive_admission_rotation(&connection).await.unwrap();
        }
        endpoint.close().await;
        node.close().await;
    });
}

#[cfg(feature = "replica-test-hooks")]
async fn rotation_worker(
    path: &Path,
    public: &Path,
    checkpoint: &str,
    role: &str,
) -> (tokio::process::Child, EndpointAddr) {
    use sha2::{Digest, Sha256};
    let executable = std::env::current_exe().unwrap();
    eprintln!(
        "MIO37_WORKER binary_sha256={} checkpoint={checkpoint}",
        hex::encode(Sha256::digest(std::fs::read(&executable).unwrap()))
    );
    let mut child = tokio::process::Command::new(executable)
        .args(["--exact", "rotation_checkpoint_worker", "--nocapture"])
        .env("MIO37_NODE_PATH", path)
        .env("MIO37_PUBLIC_PATH", public)
        .env("MIO37_CHECKPOINT", checkpoint)
        .env("MIO37_ROLE", role)
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let address = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(public) {
                let mut lines = text.lines();
                if let (Some(id), Some(address)) = (lines.next(), lines.next()) {
                    if let (Ok(id), Ok(address)) = (
                        id.parse::<iroh_base::EndpointId>(),
                        address.parse::<std::net::SocketAddr>(),
                    ) {
                        break EndpointAddr::new(id).with_ip_addr(address);
                    }
                }
            }
            assert!(
                child.try_wait().unwrap().is_none(),
                "worker exited before publishing address"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    (child, address)
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn received_before_commit_process_crash_never_acknowledges_or_switches() {
    use miopunch_native_probe::admission_rotation::RotationError;
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("creator");
    let mp = dir.path().join("member");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let member = join(&mut c, &mp).await;
    let id = member.status().node_id;
    member.close().await;
    let mut session = c.begin_management_session();
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    let initial = c.begin_admission_rotation(&mut session).await.unwrap();
    let (mut child, target) = rotation_worker(
        &mp,
        &dir.path().join("worker.public"),
        "RotationSecretReceived",
        "receiver",
    )
    .await;
    let endpoint = c.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let connection = endpoint
        .connect(target, ADMISSION_ROTATION_ALPN)
        .await
        .unwrap();
    assert!(c.distribute_admission_rotation(&connection).await.is_err());
    let exit = tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(exit.code(), Some(87));
    endpoint.close().await;
    let after = c.rotation_progress().await.unwrap().unwrap();
    assert_eq!(after.id, initial.id);
    assert_eq!(after.acknowledged.len(), 1);
    assert_eq!(after.stage, RotationStage::Distributing);
    assert!(matches!(
        c.finish_admission_rotation(&mut session).await,
        Err(RotationError::Waiting)
    ));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        c.status().members.iter().any(|(member, _)| *member == id),
        "waiting never automatically removes a member"
    );
    let mut member = NativeNetwork::open(&mp).await.unwrap();
    let pending = member.rotation_progress().await.unwrap().unwrap();
    assert_eq!(pending.id, initial.id);
    assert!(
        pending.acknowledged.is_empty(),
        "root pause is durable; secret receipt is not acknowledged"
    );
    assert!(
        matches!(
            member.begin_rejoin(c.manual_invitation().unwrap()).await,
            Err(miopunch_native_probe::native_network::NodeError::Store(
                miopunch_native_probe::management_store::StoreError::AdmissionPaused
            ))
        ),
        "reopened recipient enforces its durable admission pause"
    );
    distribute(&mut c, &mut member).await;
    c.finish_admission_rotation(&mut session).await.unwrap();
    distribute(&mut c, &mut member).await;
    assert_eq!(
        member.rotation_progress().await.unwrap().unwrap().stage,
        RotationStage::Complete
    );
    eprintln!(
        "MIO37_PRIORITY receiver_received=true before_commit_actual_exit=87 ack_not_counted=true no_early_switch=true reopened_unacknowledged_plan=true reopened_pause_enforced=true explicit_retry_completed=true"
    );
    c.close().await;
    member.close().await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn saved_replacement_lost_ack_is_retryable_after_member_and_initiator_restart() {
    use miopunch_native_probe::admission_rotation::RotationError;
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("creator");
    let mp = dir.path().join("member");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let member = join(&mut c, &mp).await;
    let id = member.status().node_id;
    member.close().await;
    let mut session = c.begin_management_session();
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    let initial = c.begin_admission_rotation(&mut session).await.unwrap();
    let (mut child, target) = rotation_worker(
        &mp,
        &dir.path().join("worker.public"),
        "RotationSecretSaved",
        "receiver",
    )
    .await;
    let endpoint = c.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let connection = endpoint
        .connect(target, ADMISSION_ROTATION_ALPN)
        .await
        .unwrap();
    assert!(c.distribute_admission_rotation(&connection).await.is_err());
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(10), child.wait())
            .await
            .unwrap()
            .unwrap()
            .code(),
        Some(87)
    );
    endpoint.close().await;
    assert!(matches!(
        c.finish_admission_rotation(&mut session).await,
        Err(RotationError::Waiting)
    ));
    c.close().await;
    let mut c = NativeNetwork::open(&cp).await.unwrap();
    let mut member = NativeNetwork::open(&mp).await.unwrap();
    assert_eq!(member.status().node_id, id);
    assert_eq!(
        member.rotation_progress().await.unwrap().unwrap().id,
        initial.id
    );
    let before = c.rotation_progress().await.unwrap().unwrap();
    assert_eq!(before.id, initial.id);
    assert_eq!(before.acknowledged.len(), 1);
    let mut session = c.begin_management_session();
    assert!(
        c.finish_admission_rotation(&mut session).await.is_err(),
        "reopen never transfers the old elevation"
    );
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    assert_eq!(
        c.begin_admission_rotation(&mut session).await.unwrap().id,
        initial.id,
        "restart resumes the exact rotation"
    );
    distribute(&mut c, &mut member).await;
    distribute(&mut c, &mut member).await;
    assert_eq!(
        c.rotation_progress()
            .await
            .unwrap()
            .unwrap()
            .acknowledged
            .len(),
        2,
        "duplicates never increase the retained count"
    );
    c.finish_admission_rotation(&mut session).await.unwrap();
    distribute(&mut c, &mut member).await;
    eprintln!(
        "MIO37_LOST_ACK actual_exit=87 durable_secret_before_ack=true initiator_reopened=true same_rotation=true duplicate_ack_idempotent=true"
    );
    c.close().await;
    member.close().await;
}

#[cfg(feature = "replica-test-hooks")]
async fn initiator_ack_boundary(checkpoint: &str, expected_saved: usize) {
    use miopunch_native_probe::admission_rotation::RotationError;
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("creator");
    let mp = dir.path().join("member");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let mut member = join(&mut c, &mp).await;
    let mut session = c.begin_management_session();
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    let initial = c.begin_admission_rotation(&mut session).await.unwrap();
    c.close().await;
    let (mut child, target) = rotation_worker(
        &cp,
        &dir.path().join("worker.public"),
        checkpoint,
        "initiator",
    )
    .await;
    let endpoint = member.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let connection = endpoint
        .connect(target, ADMISSION_ROTATION_ALPN)
        .await
        .unwrap();
    assert!(
        member
            .receive_admission_rotation(&connection)
            .await
            .is_err()
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(10), child.wait())
            .await
            .unwrap()
            .unwrap()
            .code(),
        Some(87)
    );
    endpoint.close().await;
    member.close().await;
    let mut member = NativeNetwork::open(&mp).await.unwrap();
    let mut c = NativeNetwork::open(&cp).await.unwrap();
    let progress = c.rotation_progress().await.unwrap().unwrap();
    assert_eq!(progress.id, initial.id);
    assert_eq!(progress.stage, RotationStage::Distributing);
    assert_eq!(progress.acknowledged.len(), expected_saved);
    let mut session = c.begin_management_session();
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    if expected_saved == 1 {
        assert!(matches!(
            c.finish_admission_rotation(&mut session).await,
            Err(RotationError::Waiting)
        ));
    }
    distribute(&mut c, &mut member).await;
    assert_eq!(
        c.rotation_progress()
            .await
            .unwrap()
            .unwrap()
            .acknowledged
            .len(),
        2
    );
    c.finish_admission_rotation(&mut session).await.unwrap();
    distribute(&mut c, &mut member).await;
    eprintln!(
        "MIO37_INITIATOR actual_exit=87 checkpoint={checkpoint} saved_acknowledgements={expected_saved} same_rotation=true retry_completed=true"
    );
    c.close().await;
    member.close().await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn initiator_ack_received_before_database_commit_does_not_count_after_restart() {
    initiator_ack_boundary("RotationAcknowledgementReceived", 1).await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn initiator_ack_database_commit_survives_restart_before_sender_result() {
    initiator_ack_boundary("RotationAcknowledgementSaved", 2).await;
}

#[tokio::test]
async fn new_manual_approval_waits_for_rotation_without_creating_membership() {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("creator");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let mut applicant =
        NativeNetwork::join(dir.path().join("applicant"), c.manual_invitation().unwrap())
            .await
            .unwrap();
    let ce = c.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let ae = applicant.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = connect(&ae, &ce, ADMISSION_REQUEST_ALPN).await;
    let (sent, served) = tokio::join!(
        applicant.request_manual_admission(&client),
        c.serve_manual_admission(&server)
    );
    assert!(matches!(sent.unwrap(), ManualExchange::AwaitingApproval));
    served.unwrap();
    ae.close().await;
    ce.close().await;
    let request = c.manual_requests().await.unwrap().pop().unwrap();
    let before = c.status();
    let mut session = c.begin_management_session();
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    c.begin_admission_rotation(&mut session).await.unwrap();
    assert!(
        c.approve_manual_admission(&request).await.is_err(),
        "rotation pauses new manual approvals"
    );
    assert_eq!(c.status(), before);
    assert!(c.approved_admission(&request).await.unwrap().is_none());
    c.finish_admission_rotation(&mut session).await.unwrap();
    c.approve_manual_admission(&request).await.unwrap();
    let ce = c.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let ae = applicant.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = connect(&ae, &ce, ADMISSION_REQUEST_ALPN).await;
    let receive = async {
        let ManualExchange::Materials(material) =
            applicant.request_manual_admission(&client).await.unwrap()
        else {
            panic!("approval")
        };
        applicant.complete_manual_admission(material).await.unwrap();
    };
    let (_, served) = tokio::join!(receive, c.serve_manual_admission(&server));
    served.unwrap();
    ae.close().await;
    ce.close().await;
    eprintln!(
        "MIO37_PAUSE new_manual_approval=true no_side_effect=true resumed_current_generation=true"
    );
    c.close().await;
    applicant.close().await;
}

#[tokio::test]
async fn already_approved_member_without_old_secret_receives_only_rotation_replacement() {
    use miopunch_native_probe::admission_rotation::{ROTATION_DELIVERY_ALPN, RotationError};
    use miopunch_native_probe::manual_admission::AdmissionStage;
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("creator");
    let ap = dir.path().join("approved-member");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let mut a = NativeNetwork::join(&ap, c.manual_invitation().unwrap())
        .await
        .unwrap();
    let ce = c.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let ae = a.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = connect(&ae, &ce, ADMISSION_REQUEST_ALPN).await;
    let (sent, served) = tokio::join!(
        a.request_manual_admission(&client),
        c.serve_manual_admission(&server)
    );
    assert!(matches!(sent.unwrap(), ManualExchange::AwaitingApproval));
    served.unwrap();
    let request = c.manual_requests().await.unwrap().pop().unwrap();
    let original = c.approve_manual_admission(&request).await.unwrap();
    assert_eq!(a.admission_stage(), AdmissionStage::AwaitingApproval);
    let mut session = c.begin_management_session();
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    let progress = c.begin_admission_rotation(&mut session).await.unwrap();
    assert!(
        progress.retained.contains(&a.status().node_id),
        "current approved membership must not be silently excluded"
    );
    assert!(matches!(
        c.finish_admission_rotation(&mut session).await,
        Err(RotationError::Waiting)
    ));
    let (requested, served) = tokio::join!(
        a.request_manual_admission(&client),
        c.serve_manual_admission(&server)
    );
    assert!(
        requested.is_err() && served.is_err(),
        "ordinary approved-material resumption stays paused"
    );
    ae.close().await;
    ce.close().await;
    let ce = c.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let ae = a.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = connect(&ce, &ae, ROTATION_DELIVERY_ALPN).await;
    let (sent, received) = tokio::join!(
        c.distribute_admission_rotation(&client),
        a.receive_admission_rotation(&server)
    );
    sent.unwrap();
    received.unwrap();
    assert_eq!(
        a.admission_stage(),
        AdmissionStage::PendingMaterials,
        "staged replacement does not activate before commit"
    );
    assert_eq!(
        c.approved_admission(&request)
            .await
            .unwrap()
            .unwrap()
            .authorization,
        original.authorization
    );
    ae.close().await;
    ce.close().await;
    c.finish_admission_rotation(&mut session).await.unwrap();
    distribute(&mut c, &mut a).await;
    assert_eq!(a.admission_stage(), AdmissionStage::Complete);
    a.close().await;
    let a = NativeNetwork::open(&ap).await.unwrap();
    assert_eq!(a.admission_stage(), AdmissionStage::Complete);
    eprintln!(
        "MIO37_PENDING_MEMBER original_grant_reused=true retained_not_excluded=true ordinary_resumption_paused=true replacement_staged_without_old_secret=true complete_after_switch=true"
    );
    a.close().await;
    c.close().await;
}

#[cfg(feature = "replica-test-hooks")]
async fn pending_root_generation(
    generation: u64,
) -> (
    tempfile::TempDir,
    std::path::PathBuf,
    std::path::PathBuf,
    NativeNetwork,
    NativeNetwork,
) {
    use miopunch_native_probe::management_recovery::{MANAGEMENT_RECOVERY_ALPN, RecoveryStage};
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("creator");
    let rp = dir.path().join("recovering");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    for _ in 0..generation {
        let mut session = c.begin_management_session();
        c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
            .unwrap();
        c.begin_admission_rotation(&mut session).await.unwrap();
        c.finish_admission_rotation(&mut session).await.unwrap();
    }
    let ce = c.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let root = NativeNetwork::recover(&rp, &credential(&cp), ce.id())
        .await
        .unwrap();
    root.close().await;
    let (mut child, target) = rotation_worker(
        &rp,
        &dir.path().join("root-worker.public"),
        "SecretReceived",
        "root-recovery",
    )
    .await;
    let connection = ce.connect(target, MANAGEMENT_RECOVERY_ALPN).await.unwrap();
    assert!(c.serve_management_recovery(&connection).await.is_err());
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(10), child.wait())
            .await
            .unwrap()
            .unwrap()
            .code(),
        Some(87)
    );
    ce.close().await;
    let root = NativeNetwork::open(&rp).await.unwrap();
    assert_eq!(
        root.recovery_progress().await.unwrap().stage,
        RecoveryStage::AuthorizationSaved
    );
    assert!(
        c.status()
            .members
            .contains(&(root.status().node_id, p2panda_auth::Access::manage())),
        "provider has durably verified the grant before this interruption"
    );
    assert!(
        c.manual_requests().await.unwrap().is_empty(),
        "root proof never becomes an ordinary approval"
    );
    (dir, cp, rp, c, root)
}

#[cfg(feature = "replica-test-hooks")]
async fn pending_root() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    std::path::PathBuf,
    NativeNetwork,
    NativeNetwork,
) {
    pending_root_generation(0).await
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn pending_root_recovery_delivery_pauses_but_fresh_root_proof_receives_replacement_once() {
    use miopunch_native_probe::admission_rotation::ROTATION_DELIVERY_ALPN;
    use miopunch_native_probe::management_recovery::{MANAGEMENT_RECOVERY_ALPN, RecoveryStage};
    let (_dir, cp, rp, mut c, mut root) = pending_root().await;
    let id = root.status().node_id;
    let old = root.recovery_progress().await.unwrap();
    let mut session = c.begin_management_session();
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    c.begin_admission_rotation(&mut session).await.unwrap();
    let ce = c.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let re = root.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = connect(&re, &ce, MANAGEMENT_RECOVERY_ALPN).await;
    let (restored, served) = tokio::join!(
        root.restore_management(&client),
        c.serve_management_recovery(&server)
    );
    assert!(
        restored.is_err() && served.is_err(),
        "ordinary root restoration material delivery pauses"
    );
    assert!(c.manual_requests().await.unwrap().is_empty());
    let (client, server) = connect(&ce, &re, ROTATION_DELIVERY_ALPN).await;
    let (sent, received) = tokio::join!(
        c.distribute_admission_rotation(&client),
        root.receive_admission_rotation(&server)
    );
    sent.unwrap();
    received.unwrap();
    tokio::time::timeout(Duration::from_secs(2), client.closed())
        .await
        .unwrap();
    assert!(
        root.receive_admission_rotation(&server).await.is_err(),
        "root authentication cannot repeat on this same TLS session"
    );
    assert_eq!(
        root.recovery_progress().await.unwrap().stage,
        RecoveryStage::AuthorizationSaved
    );
    ce.close().await;
    re.close().await;
    c.finish_admission_rotation(&mut session).await.unwrap();
    distribute(&mut c, &mut root).await;
    assert_eq!(
        root.recovery_progress().await.unwrap().stage,
        RecoveryStage::Complete
    );
    root.close().await;
    let root = NativeNetwork::open(&rp).await.unwrap();
    assert_eq!(root.status().node_id, id);
    let current = root.recovery_progress().await.unwrap();
    assert_eq!(current.request, old.request);
    assert_eq!(current.authorization, old.authorization);
    assert_eq!(current.stage, RecoveryStage::Complete);
    assert!(c.manual_requests().await.unwrap().is_empty());
    eprintln!(
        "MIO37_ROOT_PENDING original_grant=true fresh_original_root_exporter_proof=true one_authentication_per_tls=true ordinary_delivery_paused=true replacement_only=true stable_identity=true reopened=true no_ordinary_approval=true"
    );
    root.close().await;
    c.close().await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn forged_ack_signature_is_rejected_without_counting_the_member() {
    use miopunch_native_probe::admission_rotation::{RotationAckFault, RotationError};
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("creator");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let mut a = join(&mut c, &dir.path().join("member")).await;
    let mut session = c.begin_management_session();
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    c.begin_admission_rotation(&mut session).await.unwrap();
    let ce = c.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let ae = a.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = connect(&ce, &ae, ADMISSION_ROTATION_ALPN).await;
    let (sent, received) = tokio::join!(
        c.distribute_admission_rotation(&client),
        a.receive_admission_rotation_fixture(&server, RotationAckFault::ForgedSignature)
    );
    assert!(sent.is_err() && received.is_err());
    assert_eq!(
        c.rotation_progress()
            .await
            .unwrap()
            .unwrap()
            .acknowledged
            .len(),
        1
    );
    assert!(matches!(
        c.finish_admission_rotation(&mut session).await,
        Err(RotationError::Waiting)
    ));
    ce.close().await;
    ae.close().await;
    distribute(&mut c, &mut a).await;
    c.finish_admission_rotation(&mut session).await.unwrap();
    distribute(&mut c, &mut a).await;
    eprintln!(
        "MIO37_FORGED_ACK real_tls=true actual_signature_tampered=true counted=false honest_retry_completed=true"
    );
    a.close().await;
    c.close().await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn ack_signed_by_another_retained_identity_cannot_count_on_this_tls_connection() {
    use miopunch_native_probe::admission_rotation::{RotationAckFault, RotationError};
    use p2panda_core::SigningKey;
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("creator");
    let bp = dir.path().join("other-retained");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let mut a = join(&mut c, &dir.path().join("tls-peer")).await;
    let mut b = join(&mut c, &bp).await;
    let wrong = SigningKey::from_bytes(
        &std::fs::read(bp.join("node.key"))
            .unwrap()
            .try_into()
            .unwrap(),
    );
    assert_eq!(
        wrong.verifying_key(),
        b.status().node_id,
        "a genuine other retained node signs the wrong-identity acknowledgement"
    );
    let mut session = c.begin_management_session();
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    let initial = c.begin_admission_rotation(&mut session).await.unwrap();
    assert!(initial.retained.contains(&b.status().node_id));
    let ce = c.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let ae = a.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = connect(&ce, &ae, ADMISSION_ROTATION_ALPN).await;
    assert_eq!(client.remote_id().as_bytes(), a.status().node_id.as_bytes());
    let (sent, received) = tokio::join!(
        c.distribute_admission_rotation(&client),
        a.receive_admission_rotation_fixture(&server, RotationAckFault::WrongIdentity(wrong))
    );
    assert!(sent.is_err() && received.is_err());
    assert_eq!(
        c.rotation_progress()
            .await
            .unwrap()
            .unwrap()
            .acknowledged
            .len(),
        1
    );
    assert!(matches!(
        c.finish_admission_rotation(&mut session).await,
        Err(RotationError::Waiting)
    ));
    ce.close().await;
    ae.close().await;
    distribute(&mut c, &mut a).await;
    distribute(&mut c, &mut b).await;
    c.finish_admission_rotation(&mut session).await.unwrap();
    distribute(&mut c, &mut a).await;
    distribute(&mut c, &mut b).await;
    eprintln!(
        "MIO37_WRONG_ID_ACK genuine_other_retained_signer=true real_different_tls_peer=true counted=false honest_all_retained_completed=true"
    );
    a.close().await;
    b.close().await;
    c.close().await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn actual_previous_rotation_signed_ack_is_rejected_in_the_next_generation() {
    use miopunch_native_probe::admission_rotation::{RotationAckFault, RotationError};
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("creator");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let mut a = join(&mut c, &dir.path().join("member")).await;
    let mut session = c.begin_management_session();
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    let old = c.begin_admission_rotation(&mut session).await.unwrap();
    distribute(&mut c, &mut a).await;
    c.finish_admission_rotation(&mut session).await.unwrap();
    distribute(&mut c, &mut a).await;
    assert_eq!(
        a.rotation_progress().await.unwrap().unwrap().stage,
        RotationStage::Complete
    );
    let next = c.begin_admission_rotation(&mut session).await.unwrap();
    assert_ne!(next.id, old.id);
    assert_eq!(next.generation, 2);
    let ce = c.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let ae = a.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = connect(&ce, &ae, ADMISSION_ROTATION_ALPN).await;
    let (sent, received) = tokio::join!(
        c.distribute_admission_rotation(&client),
        a.receive_admission_rotation_fixture(&server, RotationAckFault::PreviousRotation)
    );
    assert!(sent.is_err() && received.is_err());
    let progress = c.rotation_progress().await.unwrap().unwrap();
    assert_eq!(progress.id, next.id);
    assert_eq!(progress.acknowledged.len(), 1);
    assert!(matches!(
        c.finish_admission_rotation(&mut session).await,
        Err(RotationError::Waiting)
    ));
    ce.close().await;
    ae.close().await;
    distribute(&mut c, &mut a).await;
    c.finish_admission_rotation(&mut session).await.unwrap();
    distribute(&mut c, &mut a).await;
    eprintln!(
        "MIO37_STALE_ACK prior_real_rotation_complete=true literal_prior_durable_signed_ack=true current_generation=2 counted=false honest_retry_completed=true"
    );
    a.close().await;
    c.close().await;
}

#[tokio::test]
async fn explicit_removal_changes_retained_set_and_takes_effect_before_rotation_switch() {
    use miopunch_native_probe::admission_gate::ProtocolScope;
    use miopunch_native_probe::admission_rotation::RotationError;
    use miopunch_native_probe::daily_management::ManagementAction;
    use miopunch_native_probe::offline_replica::REPLICA_ALPN;
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("creator");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let mut a = join(&mut c, &dir.path().join("acknowledged")).await;
    let b = join(&mut c, &dir.path().join("unacknowledged")).await;
    let bid = b.status().node_id;
    for node in [&c, &b] {
        node.admission_gate()
            .register_protocol(REPLICA_ALPN, ProtocolScope::Member);
    }
    let ce = c.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let be = b.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    ce.set_alpns(vec![REPLICA_ALPN.to_vec()]);
    be.set_alpns(vec![REPLICA_ALPN.to_vec()]);
    let (old_client, old_server) = connect(&ce, &be, REPLICA_ALPN).await;
    let mut session = c.begin_management_session();
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    let initial = c.begin_admission_rotation(&mut session).await.unwrap();
    distribute(&mut c, &mut a).await;
    assert!(matches!(
        c.finish_admission_rotation(&mut session).await,
        Err(RotationError::Waiting)
    ));
    session.exit_genesis_elevation();
    let removed = c
        .manage(&mut session, ManagementAction::RemoveMember(bid))
        .await
        .unwrap();
    assert_eq!(
        removed.effective_access, None,
        "ordinary member removal does not wait for secret switch"
    );
    tokio::time::timeout(Duration::from_secs(2), old_client.closed())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), old_server.closed())
        .await
        .unwrap();
    let pending = c.rotation_progress().await.unwrap().unwrap();
    assert_eq!(pending.id, initial.id);
    assert_eq!(pending.stage, RotationStage::Distributing);
    assert!(
        !pending.retained.contains(&bid),
        "progress reflects the actual retained set after explicit removal"
    );
    assert_eq!(pending.acknowledged.len(), 2);
    assert!(
        c.finish_admission_rotation(&mut session).await.is_err(),
        "switch rechecks genesis elevation"
    );
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    let complete = c.finish_admission_rotation(&mut session).await.unwrap();
    assert_eq!(complete.retained.len(), 2);
    distribute(&mut c, &mut a).await;
    assert!(!a.status().members.iter().any(|(id, _)| *id == bid));
    ce.close().await;
    be.close().await;
    eprintln!(
        "MIO37_EXPLICIT_REMOVE missing_ack_waited=true daily_removal_effective_before_switch=true existing_session_closed=true actual_retained_rechecked=true fresh_genesis_required=true no_automatic_removal=true"
    );
    a.close().await;
    b.close().await;
    c.close().await;
}

async fn replicate(source: &NativeNetwork, receiver: &mut NativeNetwork) {
    use miopunch_native_probe::admission_gate::ProtocolScope;
    use miopunch_native_probe::offline_replica::REPLICA_ALPN;
    for node in [source, &*receiver] {
        node.admission_gate()
            .register_protocol(REPLICA_ALPN, ProtocolScope::Member);
    }
    let se = source.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let re = receiver.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    se.set_alpns(vec![REPLICA_ALPN.to_vec()]);
    re.set_alpns(vec![REPLICA_ALPN.to_vec()]);
    let (client, server) = connect(&re, &se, REPLICA_ALPN).await;
    let (received, sent) = tokio::join!(
        receiver.sync_replica(&client),
        source.serve_replica(&server)
    );
    assert!(received.unwrap().complete());
    sent.unwrap();
    re.close().await;
    se.close().await;
}

#[tokio::test]
async fn completed_root_recovery_rotates_reopens_and_rejoins_ordinary_at_a_later_generation() {
    use miopunch_native_probe::daily_management::ManagementAction;
    use miopunch_native_probe::management_recovery::{MANAGEMENT_RECOVERY_ALPN, RecoveryStage};
    use miopunch_native_probe::manual_admission::AdmissionStage;
    use p2panda_auth::Access;
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("creator");
    let rp = dir.path().join("recovered");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let ce = c.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let mut r = NativeNetwork::recover(&rp, &credential(&cp), ce.id())
        .await
        .unwrap();
    let re = r.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = connect(&re, &ce, MANAGEMENT_RECOVERY_ALPN).await;
    let (received, served) = tokio::join!(
        r.restore_management(&client),
        c.serve_management_recovery(&server)
    );
    let old = received.unwrap();
    served.unwrap();
    let id = r.status().node_id;
    let anchor = r.status().anchor;
    let original = r
        .management_record(old.authorization.unwrap())
        .await
        .unwrap()
        .unwrap();
    re.close().await;
    ce.close().await;
    let mut session = c.begin_management_session();
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    let first = c.begin_admission_rotation(&mut session).await.unwrap();
    assert_eq!(first.generation, 1);
    distribute(&mut c, &mut r).await;
    c.finish_admission_rotation(&mut session).await.unwrap();
    distribute(&mut c, &mut r).await;
    c.close().await;
    r.close().await;
    let mut c = NativeNetwork::open(&cp).await.unwrap();
    let mut r = NativeNetwork::open(&rp).await.unwrap();
    assert_eq!(
        r.recovery_progress().await.unwrap().stage,
        RecoveryStage::Complete
    );
    assert_eq!(r.status().node_id, id);
    assert_eq!(r.status().anchor, anchor);
    replicate(&c, &mut r).await;
    let mut session = c.begin_management_session();
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    let removed = c
        .manage(&mut session, ManagementAction::RemoveMember(id))
        .await
        .unwrap();
    assert_eq!(removed.effective_access, None);
    let record = c.management_record(removed.record).await.unwrap().unwrap();
    r.ingest_management_record(record.header(), record.body())
        .await
        .unwrap();
    assert!(
        r.recovery_progress().await.is_err(),
        "old root grant remains currently invalid"
    );
    r.begin_rejoin(c.manual_invitation().unwrap())
        .await
        .unwrap();
    assert!(
        r.recovery_progress().await.is_err(),
        "historical root evidence is inert in the ordinary flow"
    );
    let second = c.begin_admission_rotation(&mut session).await.unwrap();
    assert_eq!(second.generation, 2);
    assert_eq!(second.retained, vec![c.status().node_id]);
    c.finish_admission_rotation(&mut session).await.unwrap();
    let ce = c.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let re = r.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = connect(&re, &ce, ADMISSION_REQUEST_ALPN).await;
    let (requested, served) = tokio::join!(
        r.request_manual_admission(&client),
        c.serve_manual_admission(&server)
    );
    assert!(matches!(
        requested.unwrap(),
        ManualExchange::AwaitingApproval
    ));
    served.unwrap();
    let request = c.manual_requests().await.unwrap().pop().unwrap();
    let fresh = c.approve_manual_admission(&request).await.unwrap();
    assert_ne!(fresh.authorization, old.authorization.unwrap());
    let receive = async {
        let ManualExchange::Materials(material) =
            r.request_manual_admission(&client).await.unwrap()
        else {
            panic!("fresh ordinary material")
        };
        assert_eq!(
            r.admission_stage(),
            AdmissionStage::PendingMaterials,
            "an old-generation secret cannot report fresh admission complete"
        );
        r.complete_manual_admission(material).await.unwrap();
    };
    let (_, served) = tokio::join!(receive, c.serve_manual_admission(&server));
    served.unwrap();
    re.close().await;
    ce.close().await;
    assert!(c.served_management_recovery(old.request).await.is_err());
    r.close().await;
    let mut r = NativeNetwork::open(&rp).await.unwrap();
    assert_eq!(r.status().node_id, id);
    assert_eq!(r.status().anchor, anchor);
    assert!(r.status().members.contains(&(id, Access::read())));
    assert!(!r.status().members.contains(&(id, Access::manage())));
    assert!(r.recovery_progress().await.is_err());
    assert_eq!(r.admission_stage(), AdmissionStage::Complete);
    assert_eq!(
        r.management_record(old.authorization.unwrap())
            .await
            .unwrap(),
        Some(original)
    );
    replicate(&c, &mut r).await;
    eprintln!(
        "MIO37_ROOT_LIFECYCLE gen1_root_complete_reopened=true explicit_remove=true gen2_fresh_ordinary_material=true old_secret_not_success=true stable_identity_anchor_history=true old_root_authority_invalid=true only_read=true new_generation_logsync_after_restart=true"
    );
    r.close().await;
    c.close().await;
}

#[tokio::test]
async fn standalone_rotation_requires_current_device_session_elevation_at_start_and_switch() {
    use miopunch_native_probe::admission_rotation::RotationError;
    use miopunch_native_probe::daily_management::ManagementError;
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("creator");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let mut session = c.begin_management_session();
    assert!(matches!(
        c.begin_admission_rotation(&mut session).await,
        Err(RotationError::Management(
            ManagementError::ElevationRequired
        ))
    ));
    assert!(c.rotation_progress().await.unwrap().is_none());
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    let started = c.begin_admission_rotation(&mut session).await.unwrap();
    assert_eq!(started.retained, vec![c.status().node_id]);
    let mut foreign_session = c.begin_management_session();
    assert!(matches!(
        c.finish_admission_rotation(&mut foreign_session).await,
        Err(RotationError::Management(
            ManagementError::ElevationRequired
        ))
    ));
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_millis(5))
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(matches!(
        c.finish_admission_rotation(&mut session).await,
        Err(RotationError::Management(
            ManagementError::ElevationRequired
        ))
    ));
    assert_eq!(
        c.rotation_progress().await.unwrap().unwrap().stage,
        RotationStage::Distributing
    );
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    c.finish_admission_rotation(&mut session).await.unwrap();
    c.close().await;
    let c = NativeNetwork::open(&cp).await.unwrap();
    assert_eq!(
        c.rotation_progress().await.unwrap().unwrap().stage,
        RotationStage::Complete
    );
    eprintln!(
        "MIO37_STANDALONE no_elevation_rejected=true other_session_rejected=true expired_switch_rejected=true valid_standalone_complete_reopened=true"
    );
    c.close().await;
}

#[tokio::test]
async fn explicit_combined_remove_and_rotate_removes_first_and_waits_for_other_retained_member() {
    use miopunch_native_probe::admission_rotation::RotationError;
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("creator");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let mut a = join(&mut c, &dir.path().join("retained")).await;
    let b = join(&mut c, &dir.path().join("removed")).await;
    let bid = b.status().node_id;
    let mut session = c.begin_management_session();
    assert!(c.remove_member_and_rotate(&mut session, bid).await.is_err());
    assert!(
        c.status().members.iter().any(|(id, _)| *id == bid),
        "unauthorized combined operation has no removal side effect"
    );
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    let progress = c.remove_member_and_rotate(&mut session, bid).await.unwrap();
    assert!(!c.status().members.iter().any(|(id, _)| *id == bid));
    assert!(!progress.retained.contains(&bid));
    assert_eq!(progress.retained.len(), 2);
    assert!(matches!(
        c.finish_admission_rotation(&mut session).await,
        Err(RotationError::Waiting)
    ));
    distribute(&mut c, &mut a).await;
    c.finish_admission_rotation(&mut session).await.unwrap();
    distribute(&mut c, &mut a).await;
    eprintln!(
        "MIO37_COMBINED genesis_required=true explicit_removal_first=true other_retained_ack_required=true complete=true"
    );
    a.close().await;
    b.close().await;
    c.close().await;
}

#[tokio::test]
async fn staged_proof_stays_restricted_and_switch_closes_old_member_sessions() {
    use miopunch_native_probe::admission_gate::ProtocolScope;
    use miopunch_native_probe::offline_replica::REPLICA_ALPN;
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("creator");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let mut a = join(&mut c, &dir.path().join("member")).await;
    for node in [&c, &a] {
        node.admission_gate()
            .register_protocol(REPLICA_ALPN, ProtocolScope::Member);
    }
    let ce = c.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let ae = a.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    ce.set_alpns(vec![
        REPLICA_ALPN.to_vec(),
        ADMISSION_ROTATION_ALPN.to_vec(),
    ]);
    ae.set_alpns(vec![
        REPLICA_ALPN.to_vec(),
        ADMISSION_ROTATION_ALPN.to_vec(),
    ]);
    let (old_client, old_server) = connect(&ce, &ae, REPLICA_ALPN).await;
    let mut session = c.begin_management_session();
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    c.begin_admission_rotation(&mut session).await.unwrap();
    distribute(&mut c, &mut a).await;
    assert!(
        c.admission_gate()
            .allows(&old_client, ProtocolScope::Member),
        "staging does not switch the active generation early"
    );
    c.finish_admission_rotation(&mut session).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), old_client.closed())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), old_server.closed())
        .await
        .unwrap();
    let target = EndpointAddr::new(ae.id()).with_ip_addr(*ae.addr().ip_addrs().next().unwrap());
    let (client, server) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(ce.connect(target, REPLICA_ALPN), async {
            ae.accept().await.unwrap().await
        })
    })
    .await
    .unwrap();
    assert!(
        client.is_err() && server.is_err(),
        "staged new credential cannot start the normal member protocol"
    );
    let (client, server) = connect(&ce, &ae, ADMISSION_ROTATION_ALPN).await;
    assert!(a.admission_gate().allows(&server, ProtocolScope::Rotation));
    assert!(!a.admission_gate().allows(&server, ProtocolScope::Member));
    assert!(!a.admission_gate().allows_member_path_upgrade(&server));
    let (sent, received) = tokio::join!(
        c.distribute_admission_rotation(&client),
        a.receive_admission_rotation(&server)
    );
    sent.unwrap();
    received.unwrap();
    assert!(
        !a.admission_gate().allows(&server, ProtocolScope::Member),
        "completion does not promote the restricted TLS session"
    );
    ce.close().await;
    ae.close().await;
    replicate(&c, &mut a).await;
    eprintln!(
        "MIO37_STAGED_SCOPE old_session_valid_before_switch=true old_session_closed=true staged_new_member_protocol_denied=true staged_rotation_allowed=true no_path_upgrade_or_scope_promotion=true fresh_normal_logsync=true"
    );
    a.close().await;
    c.close().await;
}

#[cfg(feature = "replica-test-hooks")]
async fn switch_boundary(checkpoint: &str, receiver_crashes: bool) {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("creator");
    let mp = dir.path().join("member");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let mut a = join(&mut c, &mp).await;
    let identities = [c.status().node_id, a.status().node_id];
    let mut session = c.begin_management_session();
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    let original = c.begin_admission_rotation(&mut session).await.unwrap();
    distribute(&mut c, &mut a).await;
    if receiver_crashes {
        c.finish_admission_rotation(&mut session).await.unwrap();
        a.close().await;
        let (mut child, target) = rotation_worker(
            &mp,
            &dir.path().join("worker.public"),
            checkpoint,
            "receiver",
        )
        .await;
        let endpoint = c.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
        let connection = endpoint
            .connect(target, ADMISSION_ROTATION_ALPN)
            .await
            .unwrap();
        assert!(c.distribute_admission_rotation(&connection).await.is_err());
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(10), child.wait())
                .await
                .unwrap()
                .unwrap()
                .code(),
            Some(87)
        );
        endpoint.close().await;
        a = NativeNetwork::open(&mp).await.unwrap();
    } else {
        c.close().await;
        let (mut child, target) =
            rotation_worker(&cp, &dir.path().join("worker.public"), checkpoint, "finish").await;
        let endpoint = a.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
        let _connection = endpoint
            .connect(target, ADMISSION_ROTATION_ALPN)
            .await
            .unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(10), child.wait())
                .await
                .unwrap()
                .unwrap()
                .code(),
            Some(87)
        );
        endpoint.close().await;
        c = NativeNetwork::open(&cp).await.unwrap();
        distribute(&mut c, &mut a).await;
    }
    for (node, identity) in [(&c, identities[0]), (&a, identities[1])] {
        let current = node.rotation_progress().await.unwrap().unwrap();
        assert_eq!(current.id, original.id);
        assert_eq!(current.stage, RotationStage::Complete);
        assert_eq!(node.status().node_id, identity);
    }
    replicate(&c, &mut a).await;
    eprintln!(
        "MIO37_SWITCH_CRASH actual_exit=87 checkpoint={checkpoint} receiver_crashes={receiver_crashes} committed_decision_recovered=true stable_identity=true new_generation_normal_logsync=true"
    );
    a.close().await;
    c.close().await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn initiator_switch_commit_before_material_publication_recovers_after_actual_exit() {
    switch_boundary("RotationCommitSaved", false).await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn initiator_switch_material_publication_before_pin_recovers_after_actual_exit() {
    switch_boundary("RotationMaterialPublished", false).await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn initiator_switch_pin_before_runtime_install_recovers_after_actual_exit() {
    switch_boundary("RotationCredentialPinned", false).await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn receiver_switch_commit_before_material_publication_recovers_after_actual_exit() {
    switch_boundary("RotationCommitSaved", true).await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn receiver_switch_material_publication_before_pin_recovers_after_actual_exit() {
    switch_boundary("RotationMaterialPublished", true).await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn receiver_switch_pin_before_runtime_install_recovers_after_actual_exit() {
    switch_boundary("RotationCredentialPinned", true).await;
}

fn expiry() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600
}

async fn redeem(
    c: &mut NativeNetwork,
    a: &mut NativeNetwork,
    code: &miopunch_native_probe::precode::Precode,
) -> bool {
    let ce = c.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let ae = a.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = connect(&ae, &ce, ADMISSION_REQUEST_ALPN).await;
    let receive = async {
        match a.request_precode_admission(&client, code).await {
            Ok(ManualExchange::Materials(material)) => {
                a.complete_manual_admission(material).await.is_ok()
            }
            _ => false,
        }
    };
    let (received, served) = tokio::join!(receive, c.serve_precode_admission(&server));
    assert_eq!(received, served.is_ok());
    ae.close().await;
    ce.close().await;
    received
}

#[tokio::test]
async fn new_precode_redemption_pauses_without_consumption_and_resumes_current_generation() {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("creator");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let mut retained = join(&mut c, &dir.path().join("retained")).await;
    let code = c.issue_precode(expiry(), Some(1)).await.unwrap();
    let mut a = NativeNetwork::join(dir.path().join("new-applicant"), code.invitation())
        .await
        .unwrap();
    let before = c.status();
    let mut session = c.begin_management_session();
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    c.begin_admission_rotation(&mut session).await.unwrap();
    assert!(!redeem(&mut c, &mut a, &code).await);
    assert_eq!(c.precode_usage(&code).await.unwrap(), 0);
    assert_eq!(c.status(), before);
    distribute(&mut c, &mut retained).await;
    c.finish_admission_rotation(&mut session).await.unwrap();
    distribute(&mut c, &mut retained).await;
    assert!(redeem(&mut c, &mut a, &code).await);
    assert_eq!(c.precode_usage(&code).await.unwrap(), 1);
    replicate(&c, &mut a).await;
    eprintln!(
        "MIO37_NEW_PRECODE_PAUSE no_consumption_or_grant=true resumed_generation=1 normal_logsync=true"
    );
    a.close().await;
    retained.close().await;
    c.close().await;
}

#[tokio::test]
async fn old_precode_material_resumption_pauses_but_retained_replacement_preserves_consumption() {
    use miopunch_native_probe::admission_rotation::ROTATION_DELIVERY_ALPN;
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("creator");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let code = c.issue_precode(expiry(), Some(1)).await.unwrap();
    let mut a = NativeNetwork::join(dir.path().join("approved"), code.invitation())
        .await
        .unwrap();
    let ce = c.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let ae = a.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = connect(&ae, &ce, ADMISSION_REQUEST_ALPN).await;
    let receive = async {
        let ManualExchange::Materials(material) =
            a.request_precode_admission(&client, &code).await.unwrap()
        else {
            panic!("approved")
        };
        client.close(0u32.into(), b"before initial secret");
        drop(material);
    };
    let (_, served) = tokio::join!(receive, c.serve_precode_admission(&server));
    assert!(served.is_err());
    ae.close().await;
    ce.close().await;
    assert_eq!(c.precode_usage(&code).await.unwrap(), 1);
    let original = c.precode_redemptions(&code).await.unwrap()[0].authorization;
    let mut session = c.begin_management_session();
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    c.begin_admission_rotation(&mut session).await.unwrap();
    assert!(!redeem(&mut c, &mut a, &code).await);
    assert_eq!(c.precode_usage(&code).await.unwrap(), 1);
    let ce = c.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let ae = a.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = connect(&ce, &ae, ROTATION_DELIVERY_ALPN).await;
    let (sent, received) = tokio::join!(
        c.distribute_admission_rotation(&client),
        a.receive_admission_rotation(&server)
    );
    sent.unwrap();
    received.unwrap();
    ae.close().await;
    ce.close().await;
    c.finish_admission_rotation(&mut session).await.unwrap();
    distribute(&mut c, &mut a).await;
    assert!(redeem(&mut c, &mut a, &code).await);
    assert_eq!(c.precode_usage(&code).await.unwrap(), 1);
    assert_eq!(
        c.precode_redemptions(&code).await.unwrap()[0].authorization,
        original
    );
    eprintln!(
        "MIO37_OLD_PRECODE_PAUSE original_result_paused=true replacement_allowed=true original_grant_and_single_consumption_preserved=true"
    );
    a.close().await;
    c.close().await;
}

#[tokio::test]
async fn unapproved_tls_identity_cannot_receive_retained_replacement_or_history() {
    use miopunch_native_probe::admission_rotation::ROTATION_DELIVERY_ALPN;
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("creator");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let mut a = NativeNetwork::join(
        dir.path().join("unapproved"),
        c.manual_invitation().unwrap(),
    )
    .await
    .unwrap();
    let mut session = c.begin_management_session();
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    c.begin_admission_rotation(&mut session).await.unwrap();
    let ce = c.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let ae = a.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = connect(&ce, &ae, ROTATION_DELIVERY_ALPN).await;
    let (sent, received) = tokio::join!(
        c.distribute_admission_rotation(&client),
        a.receive_admission_rotation(&server)
    );
    assert!(sent.is_err() && received.is_err());
    assert!(a.management_history().await.unwrap().is_empty());
    assert!(a.rotation_progress().await.unwrap().is_none());
    assert_eq!(
        c.rotation_progress()
            .await
            .unwrap()
            .unwrap()
            .acknowledged
            .len(),
        1
    );
    ae.close().await;
    ce.close().await;
    eprintln!(
        "MIO37_UNAPPROVED tls_identity_alone_insufficient=true no_control_history_or_replacement=true"
    );
    a.close().await;
    c.close().await;
}

#[cfg(feature = "replica-test-hooks")]
async fn initial_boundary(checkpoint: &str) {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("creator");
    let c = NativeNetwork::create(&cp).await.unwrap();
    let node_id = c.status().node_id;
    let applicant =
        NativeNetwork::join(dir.path().join("applicant"), c.manual_invitation().unwrap())
            .await
            .unwrap();
    c.close().await;
    let (mut child, target) =
        rotation_worker(&cp, &dir.path().join("worker.public"), checkpoint, "begin").await;
    let endpoint = applicant.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let _connection = endpoint
        .connect(target, ADMISSION_REQUEST_ALPN)
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(10), child.wait())
            .await
            .unwrap()
            .unwrap()
            .code(),
        Some(87)
    );
    endpoint.close().await;
    applicant.close().await;
    let mut c = NativeNetwork::open(&cp).await.unwrap();
    let original = c
        .rotation_progress()
        .await
        .unwrap()
        .map(|progress| progress.id);
    let mut session = c.begin_management_session();
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    let resumed = c.begin_admission_rotation(&mut session).await.unwrap();
    assert_eq!(resumed.acknowledged, vec![node_id]);
    if let Some(original) = original {
        assert_eq!(resumed.id, original);
    }
    c.finish_admission_rotation(&mut session).await.unwrap();
    c.close().await;
    let c = NativeNetwork::open(&cp).await.unwrap();
    assert_eq!(
        c.rotation_progress().await.unwrap().unwrap().stage,
        RotationStage::Complete
    );
    assert_eq!(c.status().node_id, node_id);
    eprintln!(
        "MIO37_INITIALIZER checkpoint={checkpoint} actual_exit=87 durable_self_ack_repaired=true complete_after_reopen=true"
    );
    c.close().await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn initiator_initial_material_before_proposal_recovers_after_actual_exit() {
    initial_boundary("RotationInitialMaterialSaved").await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn initiator_initial_proposal_before_self_ack_recovers_after_actual_exit() {
    initial_boundary("RotationProposalSaved").await;
}

#[tokio::test]
async fn missed_predecessor_completion_recovers_when_initiator_starts_next_rotation() {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("creator");
    let ap = dir.path().join("member");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let mut a = join(&mut c, &ap).await;
    let identities = (c.status().node_id, a.status().node_id);
    let mut session = c.begin_management_session();
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    c.begin_admission_rotation(&mut session).await.unwrap();
    distribute(&mut c, &mut a).await;
    c.finish_admission_rotation(&mut session).await.unwrap();
    let next = c.begin_admission_rotation(&mut session).await.unwrap();
    assert_eq!(next.generation, 2);
    c.close().await;
    a.close().await;
    let mut c = NativeNetwork::open(&cp).await.unwrap();
    let mut a = NativeNetwork::open(&ap).await.unwrap();
    assert_eq!(
        a.rotation_progress().await.unwrap().unwrap().stage,
        RotationStage::Distributing
    );
    distribute(&mut c, &mut a).await;
    let mut session = c.begin_management_session();
    c.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    c.finish_admission_rotation(&mut session).await.unwrap();
    distribute(&mut c, &mut a).await;
    a.close().await;
    let mut a = NativeNetwork::open(&ap).await.unwrap();
    assert_eq!(a.status().node_id, identities.1);
    assert_eq!(c.status().node_id, identities.0);
    assert_eq!(a.rotation_progress().await.unwrap().unwrap().generation, 2);
    assert_eq!(
        a.rotation_progress().await.unwrap().unwrap().stage,
        RotationStage::Complete
    );
    replicate(&c, &mut a).await;
    eprintln!(
        "MIO37_PREDECESSOR missed_completion_generation1=true generation2_started=true both_reopened=true signed_predecessor_catchup=true stable_identity=true generation2_logsync=true"
    );
    a.close().await;
    c.close().await;
}

async fn replacement(provider: &mut NativeNetwork, recipient: &mut NativeNetwork) {
    use miopunch_native_probe::admission_rotation::ROTATION_DELIVERY_ALPN;
    let pe = provider.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let re = recipient.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = connect(&pe, &re, ROTATION_DELIVERY_ALPN).await;
    let (sent, received) = tokio::join!(
        provider.distribute_admission_rotation(&client),
        recipient.receive_admission_rotation(&server)
    );
    sent.unwrap();
    received.unwrap();
    pe.close().await;
    re.close().await;
}

#[tokio::test]
async fn original_provider_relays_different_initiator_replacement_and_signed_ack_for_pending_member()
 {
    use miopunch_native_probe::admission_rotation::RotationError;
    let dir = tempfile::tempdir().unwrap();
    let ap = dir.path().join("original-provider");
    let bp = dir.path().join("initiator");
    let xp = dir.path().join("pending");
    let mut a = NativeNetwork::create(&ap).await.unwrap();
    let mut b = join(&mut a, &bp).await;
    let mut x = NativeNetwork::join(&xp, a.manual_invitation().unwrap())
        .await
        .unwrap();
    let ae = a.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let xe = x.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = connect(&xe, &ae, ADMISSION_REQUEST_ALPN).await;
    let (requested, served) = tokio::join!(
        x.request_manual_admission(&client),
        a.serve_manual_admission(&server)
    );
    assert!(matches!(
        requested.unwrap(),
        ManualExchange::AwaitingApproval
    ));
    served.unwrap();
    let request = a.manual_requests().await.unwrap().pop().unwrap();
    let original = a.approve_manual_admission(&request).await.unwrap();
    xe.close().await;
    ae.close().await;
    replicate(&a, &mut b).await;
    let mut session = b.begin_management_session();
    b.elevate_genesis(&mut session, credential(&ap), Duration::from_secs(60))
        .unwrap();
    let pending = b.begin_admission_rotation(&mut session).await.unwrap();
    assert!(pending.retained.contains(&x.status().node_id));
    distribute(&mut b, &mut a).await;
    replacement(&mut a, &mut x).await;
    #[cfg(feature = "replica-test-hooks")]
    {
        use miopunch_native_probe::admission_rotation::RotationAckFault;
        let be = b.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
        let ae = a.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
        let (client, server) = connect(&be, &ae, ADMISSION_ROTATION_ALPN).await;
        let (sent, received) = tokio::join!(
            b.distribute_admission_rotation(&client),
            a.receive_admission_rotation_fixture(
                &server,
                RotationAckFault::ForgedForwardedSignature
            )
        );
        assert!(sent.is_err() && received.is_err());
        assert_eq!(
            b.rotation_progress()
                .await
                .unwrap()
                .unwrap()
                .acknowledged
                .len(),
            2,
            "actual forged forwarded recipient signature cannot count"
        );
        be.close().await;
        ae.close().await;
    }
    assert_eq!(
        b.rotation_progress()
            .await
            .unwrap()
            .unwrap()
            .acknowledged
            .len(),
        2,
        "provider receipt is not assumed at the initiator"
    );
    assert!(matches!(
        b.finish_admission_rotation(&mut session).await,
        Err(RotationError::Waiting)
    ));
    a.close().await;
    b.close().await;
    x.close().await;
    let mut a = NativeNetwork::open(&ap).await.unwrap();
    let mut b = NativeNetwork::open(&bp).await.unwrap();
    let mut x = NativeNetwork::open(&xp).await.unwrap();
    distribute(&mut b, &mut a).await;
    assert_eq!(b.rotation_progress().await.unwrap().unwrap().id, pending.id);
    assert_eq!(
        b.rotation_progress()
            .await
            .unwrap()
            .unwrap()
            .acknowledged
            .len(),
        3
    );
    let mut session = b.begin_management_session();
    b.elevate_genesis(&mut session, credential(&ap), Duration::from_secs(60))
        .unwrap();
    b.finish_admission_rotation(&mut session).await.unwrap();
    distribute(&mut b, &mut a).await;
    distribute(&mut a, &mut x).await;
    assert_eq!(
        a.approved_admission(&request)
            .await
            .unwrap()
            .unwrap()
            .authorization,
        original.authorization
    );
    x.close().await;
    let mut x = NativeNetwork::open(&xp).await.unwrap();
    assert_eq!(
        x.admission_stage(),
        miopunch_native_probe::manual_admission::AdmissionStage::Complete
    );
    replicate(&b, &mut x).await;
    eprintln!(
        "MIO37_PROVIDER_RELAY original_contact_preserved=true different_elevated_initiator=true original_grant=true signed_forwarded_ack_durable_after_all_reopen=true forged_forwarded_signature_not_counted=true no_early_count=true new_generation_logsync=true"
    );
    x.close().await;
    b.close().await;
    a.close().await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn original_root_provider_relays_other_initiator_but_node_key_cannot_replace_fresh_root_proof()
 {
    use miopunch_native_probe::admission_rotation::{ROTATION_DELIVERY_ALPN, RotationError};
    let (dir, cp, rp, mut a, mut root) = pending_root_generation(1).await;
    let bp = dir.path().join("other-initiator");
    let mut b = join(&mut a, &bp).await;
    let original = root.recovery_progress().await.unwrap();
    let requests_before = a.manual_requests().await.unwrap();
    let original_id = root.status().node_id;
    let mut session = b.begin_management_session();
    b.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    b.begin_admission_rotation(&mut session).await.unwrap();
    distribute(&mut b, &mut a).await;
    let ae = a.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let re = root.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = connect(&ae, &re, ROTATION_DELIVERY_ALPN).await;
    let node_key = p2panda_core::SigningKey::from_bytes(
        &std::fs::read(rp.join("node.key"))
            .unwrap()
            .try_into()
            .unwrap(),
    );
    let anchor = root.status().anchor;
    let recovering = re.id();
    let contact = ae.id();
    let invalid = async {
        let (mut send, mut recv) = server.accept_bi().await.unwrap();
        let mut profile = [0];
        recv.read_exact(&mut profile).await.unwrap();
        assert_eq!(profile, [2]);
        let mut exporter = [0; 32];
        server
            .export_keying_material(&mut exporter, b"EXPORTER-Channel-Binding", b"")
            .unwrap();
        let mut statement = b"miopunch/genesis-recovery/v1\0".to_vec();
        statement.extend_from_slice(anchor.root.as_bytes());
        statement.extend_from_slice(anchor.genesis.as_bytes());
        statement.extend_from_slice(recovering.as_bytes());
        statement.extend_from_slice(contact.as_bytes());
        statement.extend_from_slice(&exporter);
        send.write_all(&node_key.sign(&statement).to_bytes())
            .await
            .unwrap();
        let mut first_history_length = [0; 4];
        assert!(
            recv.read_exact(&mut first_history_length).await.is_err(),
            "valid TLS node key must not disclose root history"
        );
    };
    let (sent, _) = tokio::join!(a.distribute_admission_rotation(&client), invalid);
    assert!(sent.is_err());
    assert!(root.rotation_progress().await.unwrap().is_none());
    assert_eq!(
        b.rotation_progress()
            .await
            .unwrap()
            .unwrap()
            .acknowledged
            .len(),
        2
    );
    ae.close().await;
    re.close().await;
    replacement(&mut a, &mut root).await;
    assert!(matches!(
        b.finish_admission_rotation(&mut session).await,
        Err(RotationError::Waiting)
    ));
    a.close().await;
    b.close().await;
    root.close().await;
    let mut a = NativeNetwork::open(&cp).await.unwrap();
    let mut b = NativeNetwork::open(&bp).await.unwrap();
    let mut root = NativeNetwork::open(&rp).await.unwrap();
    distribute(&mut b, &mut a).await;
    assert_eq!(
        b.rotation_progress()
            .await
            .unwrap()
            .unwrap()
            .acknowledged
            .len(),
        3
    );
    let mut session = b.begin_management_session();
    b.elevate_genesis(&mut session, credential(&cp), Duration::from_secs(60))
        .unwrap();
    b.finish_admission_rotation(&mut session).await.unwrap();
    distribute(&mut b, &mut a).await;
    distribute(&mut a, &mut root).await;
    root.close().await;
    let mut root = NativeNetwork::open(&rp).await.unwrap();
    let recovered = root.recovery_progress().await.unwrap();
    assert_eq!(root.status().node_id, original_id);
    assert_eq!(recovered.request, original.request);
    assert_eq!(recovered.authorization, original.authorization);
    assert_eq!(
        recovered.stage,
        miopunch_native_probe::management_recovery::RecoveryStage::Complete
    );
    assert_eq!(a.manual_requests().await.unwrap(), requests_before);
    replicate(&b, &mut root).await;
    eprintln!(
        "MIO37_ROOT_PROVIDER_RELAY generation=2 no_old_generation_secret=true other_initiator=true original_root_contact_grant_preserved=true actual_node_signature_rejected_before_history=true fresh_exporter_original_root_proof_required=true durable_forwarded_ack=true all_reopened=true no_ordinary_approval=true normal_logsync=true"
    );
    root.close().await;
    b.close().await;
    a.close().await;
}
