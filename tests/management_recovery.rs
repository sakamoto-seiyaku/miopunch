//! Original root backup recovery through public durable nodes and real TLS/QUIC.
#[path = "support/native_connections.rs"]
mod native_connections;
use iroh::endpoint::RestrictedEndpoint;
use native_connections::{receive_field, send_field};

use std::net::Ipv4Addr;
use std::path::Path;
use std::time::Duration;

use iroh::{EndpointAddr, endpoint::Connection};
use miopunch_native_probe::admission_gate::ADMISSION_REQUEST_ALPN;
use miopunch_native_probe::daily_management::ManagementAction;
use miopunch_native_probe::genesis_recovery::GenesisCredential;
use miopunch_native_probe::management_recovery::{
    MANAGEMENT_RECOVERY_ALPN, RecoveryRequest, RecoveryStage,
};
use miopunch_native_probe::manual_admission::{AdmissionStage, ManualExchange, ManualInvitation};
use miopunch_native_probe::native_network::NativeNetwork;
use p2panda_auth::Access;
use p2panda_core::Hash;

fn original_backup(path: &Path) -> GenesisCredential {
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

async fn ordinary_member(admin: &mut NativeNetwork, path: &Path) -> NativeNetwork {
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

async fn recovery_endpoint(node: &NativeNetwork) -> RestrictedEndpoint {
    let endpoint = node.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    endpoint.set_alpns(vec![MANAGEMENT_RECOVERY_ALPN.to_vec()]);
    endpoint
}

#[tokio::test]
async fn formal_entry_restores_original_network_when_all_old_administrators_are_offline() {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("original-device");
    let mp = dir.path().join("ordinary-member");
    let rp = dir.path().join("new-device");
    let mut creator = NativeNetwork::create(&cp).await.unwrap();
    let mut member = ordinary_member(&mut creator, &mp).await;
    let original = creator.status();
    let backup = original_backup(&cp);
    creator.close().await; // Every old administrator endpoint/store is offline.
    let me = recovery_endpoint(&member).await;
    let mut recovered = NativeNetwork::recover(&rp, &backup, me.id()).await.unwrap();
    let new_id = recovered.status().node_id;
    assert_ne!(new_id, original.node_id);
    assert_ne!(new_id, original.anchor.root);
    assert_eq!(recovered.status().anchor, original.anchor);
    assert_eq!(recovered.status().applied_count, 0);
    assert_eq!(
        recovered.admission_stage(),
        AdmissionStage::AwaitingApproval
    );
    assert_eq!(
        recovered.recovery_progress().await.unwrap().stage,
        RecoveryStage::AwaitingProof
    );
    let re = recovery_endpoint(&recovered).await;
    let (client, server) =
        native_connections::control_pair(&re, &me, MANAGEMENT_RECOVERY_ALPN).await;
    let (restored, served) = tokio::join!(
        recovered.restore_management(&client),
        member.serve_management_recovery(&server)
    );
    let restored = restored.unwrap();
    assert_eq!(served.unwrap().authorization, restored.authorization);
    assert_eq!(restored.stage, RecoveryStage::Complete);
    assert_eq!(recovered.admission_stage(), AdmissionStage::Complete);
    assert!(
        recovered
            .status()
            .members
            .contains(&(new_id, Access::manage()))
    );
    assert!(
        recovered
            .status()
            .members
            .contains(&(original.node_id, Access::manage()))
    );
    assert!(
        !recovered
            .status()
            .members
            .iter()
            .any(|(id, _)| *id == original.anchor.root)
    );
    assert!(
        member
            .status()
            .members
            .contains(&(member.status().node_id, Access::read()))
    );
    assert!(member.manual_requests().await.unwrap().is_empty());
    let mut session = recovered.begin_management_session();
    let added = recovered
        .manage(
            &mut session,
            ManagementAction::AddMember(p2panda_core::SigningKey::generate().verifying_key()),
        )
        .await
        .unwrap();
    assert_eq!(added.effective_access, Some(Access::read()));
    re.close().await;
    me.close().await;
    recovered.close().await;
    let recovered = NativeNetwork::open(&rp).await.unwrap();
    assert_eq!(recovered.status().node_id, new_id);
    assert_eq!(recovered.status().anchor, original.anchor);
    assert_eq!(
        recovered.recovery_progress().await.unwrap().stage,
        RecoveryStage::Complete
    );
    assert_eq!(recovered.admission_stage(), AdmissionStage::Complete);
    assert!(
        recovered
            .status()
            .members
            .contains(&(original.node_id, Access::manage()))
    );
    recovered.close().await;
    member.close().await;
}

#[tokio::test]
async fn ordinary_contact_requires_root_proof_but_no_secret_history_or_old_approval() {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("offline-original");
    let mp = dir.path().join("only-online-member");
    let mut creator = NativeNetwork::create(&cp).await.unwrap();
    let mut member = ordinary_member(&mut creator, &mp).await;
    let backup = original_backup(&cp);
    let anchor = creator.status().anchor;
    creator.close().await;
    let me = recovery_endpoint(&member).await;
    // An authenticated device knows the public network/contact but lacks the
    // root backup, current shared secret, history and ordinary approval.
    let attacker = NativeNetwork::join(
        dir.path().join("unproved"),
        ManualInvitation {
            anchor,
            contact: me.id(),
        },
    )
    .await
    .unwrap();
    let ae = recovery_endpoint(&attacker).await;
    let (client, server) =
        native_connections::control_pair(&ae, &me, MANAGEMENT_RECOVERY_ALPN).await;
    let request = RecoveryRequest {
        target: attacker.status().node_id,
        attempt: [7; 16],
    };
    let before = member.status();
    let (disclosed, served) = tokio::join!(
        async {
            let exchange = client.open_control_exchange().await.unwrap();
            let (send, recv) = (&exchange, &exchange);
            send_field(send, &[0; 64]).await.unwrap();
            let _ = send_field(send, &request.attempt).await;
            let mut first = [0; 1];
            receive_field(recv, &mut first).await
        },
        member.serve_management_recovery(&server)
    );
    assert!(
        disclosed.is_err(),
        "history/secret disclosed before root verification"
    );
    assert!(served.is_err());
    assert_eq!(member.status(), before);
    assert_eq!(
        member
            .served_management_recovery(request)
            .await
            .unwrap()
            .stage,
        RecoveryStage::AwaitingProof
    );
    ae.close().await;
    attacker.close().await;

    let mut recovered = NativeNetwork::recover(dir.path().join("recovering"), &backup, me.id())
        .await
        .unwrap();
    assert_eq!(recovered.status().applied_count, 0);
    assert_eq!(
        recovered.admission_stage(),
        AdmissionStage::AwaitingApproval
    );
    let re = recovery_endpoint(&recovered).await;
    let (client, server) =
        native_connections::control_pair(&re, &me, MANAGEMENT_RECOVERY_ALPN).await;
    let (restored, served) = tokio::join!(
        recovered.restore_management(&client),
        member.serve_management_recovery(&server)
    );
    assert_eq!(restored.unwrap().stage, RecoveryStage::Complete);
    assert_eq!(served.unwrap().stage, RecoveryStage::Complete);
    assert_eq!(recovered.admission_stage(), AdmissionStage::Complete);
    assert!(member.manual_requests().await.unwrap().is_empty());
    assert_eq!(member.status().applied_count, 3);
    re.close().await;
    me.close().await;
    recovered.close().await;
    member.close().await;
}

#[cfg(feature = "replica-test-hooks")]
#[test]
fn recovery_checkpoint_worker() {
    let Ok(node_path) = std::env::var("MIO36_NODE_PATH") else {
        return;
    };
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut node = NativeNetwork::open(node_path).await.unwrap();
        node.set_replica_checkpoint_observer(|stage| {
            use miopunch_native_probe::management_store::RecordCheckpoint;
            let wanted = match std::env::var("MIO36_CHECKPOINT").unwrap().as_str() {
                "RecoveryProofSaved" => RecordCheckpoint::RecoveryProofSaved,
                "RecoveryHistoryVerified" => RecordCheckpoint::RecoveryHistoryVerified,
                "RecoveryHistorySaved" => RecordCheckpoint::RecoveryHistorySaved,
                "RecoveryGrantCommitted" => RecordCheckpoint::RecoveryGrantCommitted,
                "RecoveryRejoinPrepared" => RecordCheckpoint::RecoveryRejoinPrepared,
                "RecoveryRejoinPublished" => RecordCheckpoint::RecoveryRejoinPublished,
                "AuthorizationSaved" => RecordCheckpoint::AuthorizationSaved,
                "AuthorizationSent" => RecordCheckpoint::AuthorizationSent,
                "AuthorizationAcknowledged" => RecordCheckpoint::AuthorizationAcknowledged,
                "SecretPrepared" => RecordCheckpoint::SecretPrepared,
                "SecretSent" => RecordCheckpoint::SecretSent,
                "SecretReceived" => RecordCheckpoint::SecretReceived,
                "SecretFilePublished" => RecordCheckpoint::SecretFilePublished,
                "SecretSaved" => RecordCheckpoint::SecretSaved,
                "CompletionSent" => RecordCheckpoint::CompletionSent,
                "CompletionAcknowledged" => RecordCheckpoint::CompletionAcknowledged,
                "DeliveryComplete" => RecordCheckpoint::DeliveryComplete,
                _ => panic!("unknown recovery checkpoint"),
            };
            if stage == wanted {
                eprintln!("MIO36_CRASH actual_process_exit_at={stage:?}");
                std::process::exit(86);
            }
        });
        if std::env::var("MIO36_ROLE").unwrap() == "rejoin" {
            let bytes: [u8; 96] =
                std::fs::read(std::env::var("MIO36_REJOIN_INVITATION_PATH").unwrap())
                    .unwrap()
                    .try_into()
                    .unwrap();
            node.begin_rejoin(ManualInvitation::decode(&bytes).unwrap())
                .await
                .unwrap();
            panic!("actual rejoin checkpoint was not reached");
        }
        let endpoint = recovery_endpoint(&node).await;
        if std::env::var("MIO36_ROLE").unwrap() == "contact" {
            let public = format!(
                "{}\n{}",
                endpoint.id(),
                endpoint.addr().ip_addrs().next().unwrap()
            );
            std::fs::write(std::env::var("MIO36_PUBLIC_PATH").unwrap(), public).unwrap();
            let c = std::sync::Arc::new(
                endpoint
                    .accept()
                    .await
                    .unwrap()
                    .accept_control()
                    .unwrap()
                    .await
                    .unwrap(),
            );
            node.serve_management_recovery(&c).await.unwrap();
        } else {
            let contact = std::env::var("MIO36_CONTACT").unwrap().parse().unwrap();
            let address = std::env::var("MIO36_ADDRESS").unwrap().parse().unwrap();
            let c = endpoint
                .connect_control(
                    EndpointAddr::new(contact).with_ip_addr(address),
                    MANAGEMENT_RECOVERY_ALPN,
                )
                .await
                .unwrap();
            let c = std::sync::Arc::new(c);
            node.restore_management(&c).await.unwrap();
        }
        panic!("actual recovery checkpoint was not reached");
    });
}

#[cfg(feature = "replica-test-hooks")]
async fn interruption(
    checkpoint: miopunch_native_probe::management_store::RecordCheckpoint,
    contact_crashes: bool,
) {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("offline-original");
    let mp = dir.path().join("ordinary-member");
    let rp = dir.path().join("recovering");
    let mut creator = NativeNetwork::create(&cp).await.unwrap();
    let mut member = ordinary_member(&mut creator, &mp).await;
    let original = creator.status();
    let backup = original_backup(&cp);
    creator.close().await;
    let me = recovery_endpoint(&member).await;
    let mut recovered = NativeNetwork::recover(&rp, &backup, me.id()).await.unwrap();
    let identity = recovered.status().node_id;
    let request = recovered.recovery_progress().await.unwrap().request;
    let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "recovery_checkpoint_worker", "--nocapture"])
        .env("MIO36_CHECKPOINT", format!("{checkpoint:?}"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if contact_crashes {
        let public_path = dir.path().join("public-contact");
        me.close().await;
        member.close().await;
        command
            .env("MIO36_NODE_PATH", &mp)
            .env("MIO36_ROLE", "contact")
            .env("MIO36_PUBLIC_PATH", &public_path);
        let child = command.spawn().unwrap();
        let public = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(text) = std::fs::read_to_string(&public_path) {
                    break text;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let mut lines = public.lines();
        let re = recovery_endpoint(&recovered).await;
        let connection = re
            .connect_control(
                EndpointAddr::new(lines.next().unwrap().parse().unwrap())
                    .with_ip_addr(lines.next().unwrap().parse().unwrap()),
                MANAGEMENT_RECOVERY_ALPN,
            )
            .await
            .unwrap();
        let connection = std::sync::Arc::new(connection);
        let (_, exit) = tokio::join!(
            recovered.restore_management(&connection),
            tokio::time::timeout(Duration::from_secs(20), child.wait_with_output())
        );
        let exit = exit.unwrap().unwrap();
        assert_eq!(
            exit.status.code(),
            Some(86),
            "{}",
            String::from_utf8_lossy(&exit.stderr)
        );
        eprintln!("{}", String::from_utf8_lossy(&exit.stderr));
        re.close().await;
        member = NativeNetwork::open(&mp).await.unwrap();
    } else {
        recovered.close().await;
        command
            .env("MIO36_NODE_PATH", &rp)
            .env("MIO36_ROLE", "recovering")
            .env("MIO36_CONTACT", me.id().to_string())
            .env(
                "MIO36_ADDRESS",
                me.addr().ip_addrs().next().unwrap().to_string(),
            );
        let child = command.spawn().unwrap();
        let serving = async {
            let c = me.accept().await.unwrap().await.unwrap();
            let _ = member.serve_management_recovery(&c).await;
        };
        let (_, exit) = tokio::join!(
            serving,
            tokio::time::timeout(Duration::from_secs(20), child.wait_with_output())
        );
        let exit = exit.unwrap().unwrap();
        assert_eq!(
            exit.status.code(),
            Some(86),
            "{}",
            String::from_utf8_lossy(&exit.stderr)
        );
        eprintln!("{}", String::from_utf8_lossy(&exit.stderr));
        me.close().await;
        recovered = NativeNetwork::open(&rp).await.unwrap();
    }
    let before = recovered.recovery_progress().await.unwrap();
    assert_eq!(before.request, request);
    assert_eq!(recovered.status().node_id, identity);
    assert_eq!(recovered.status().anchor, original.anchor);
    use miopunch_native_probe::management_store::RecordCheckpoint;
    let stage = recovered.admission_stage();
    match checkpoint {
        RecordCheckpoint::RecoveryProofSaved
        | RecordCheckpoint::RecoveryHistoryVerified
        | RecordCheckpoint::RecoveryHistorySaved => {
            assert_eq!(stage, AdmissionStage::AwaitingApproval);
            assert!(before.authorization.is_none());
        }
        RecordCheckpoint::SecretSent => assert!(matches!(
            stage,
            AdmissionStage::PendingMaterials | AdmissionStage::Complete
        )),
        RecordCheckpoint::SecretSaved
        | RecordCheckpoint::CompletionSent
        | RecordCheckpoint::CompletionAcknowledged
        | RecordCheckpoint::DeliveryComplete => assert_eq!(stage, AdmissionStage::Complete),
        _ => assert_eq!(stage, AdmissionStage::PendingMaterials),
    }
    let first_grant = before.authorization;
    // Stop and replay both durable stores, not only the crashed endpoint.
    recovered.close().await;
    member.close().await;
    let mut recovered = NativeNetwork::open(&rp).await.unwrap();
    let mut member = NativeNetwork::open(&mp).await.unwrap();
    let re = recovery_endpoint(&recovered).await;
    let me = recovery_endpoint(&member).await;
    let (c, s) = native_connections::control_pair(&re, &me, MANAGEMENT_RECOVERY_ALPN).await;
    let (restored, served) = tokio::join!(
        recovered.restore_management(&c),
        member.serve_management_recovery(&s)
    );
    let restored = restored.unwrap();
    let served = served.unwrap();
    assert_eq!(restored.request, request);
    if first_grant.is_some() {
        assert_eq!(restored.authorization, first_grant);
    }
    assert_eq!(restored.authorization, served.authorization);
    assert_eq!(restored.stage, RecoveryStage::Complete);
    assert_eq!(served.stage, RecoveryStage::Complete);
    assert_eq!(recovered.admission_stage(), AdmissionStage::Complete);
    assert_eq!(recovered.status().applied_count, 3);
    assert_eq!(member.status().applied_count, 3);
    assert!(
        recovered
            .status()
            .members
            .contains(&(original.node_id, Access::manage()))
    );
    assert!(
        member
            .status()
            .members
            .contains(&(member.status().node_id, Access::read()))
    );
    assert!(member.manual_requests().await.unwrap().is_empty());
    re.close().await;
    me.close().await;
    recovered.close().await;
    let recovered = NativeNetwork::open(&rp).await.unwrap();
    assert_eq!(recovered.recovery_progress().await.unwrap(), restored);
    assert_eq!(recovered.admission_stage(), AdmissionStage::Complete);
    recovered.close().await;
    member.close().await;
    eprintln!(
        "MIO36_RESUME checkpoint={checkpoint:?} contact_crashes={contact_crashes} actual_exit=86 same_request_and_grant=true applied=3 no_approval=true durable_complete=true"
    );
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_after_authorization_save_resumes_same_root_grant_without_approval() {
    interruption(
        miopunch_native_probe::management_store::RecordCheckpoint::AuthorizationSaved,
        false,
    )
    .await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_after_root_proof_save_requires_fresh_session_proof_and_resumes() {
    interruption(
        miopunch_native_probe::management_store::RecordCheckpoint::RecoveryProofSaved,
        true,
    )
    .await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_at_history_verified() {
    interruption(
        miopunch_native_probe::management_store::RecordCheckpoint::RecoveryHistoryVerified,
        false,
    )
    .await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_at_history_saved() {
    interruption(
        miopunch_native_probe::management_store::RecordCheckpoint::RecoveryHistorySaved,
        false,
    )
    .await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_at_local_grant_committed() {
    interruption(
        miopunch_native_probe::management_store::RecordCheckpoint::RecoveryGrantCommitted,
        false,
    )
    .await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_at_contact_grant_committed() {
    interruption(
        miopunch_native_probe::management_store::RecordCheckpoint::RecoveryGrantCommitted,
        true,
    )
    .await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_at_grant_sent() {
    interruption(
        miopunch_native_probe::management_store::RecordCheckpoint::AuthorizationSent,
        true,
    )
    .await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_at_authorization_acknowledged() {
    interruption(
        miopunch_native_probe::management_store::RecordCheckpoint::AuthorizationAcknowledged,
        false,
    )
    .await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_at_secret_prepared() {
    interruption(
        miopunch_native_probe::management_store::RecordCheckpoint::SecretPrepared,
        true,
    )
    .await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_at_secret_sent() {
    interruption(
        miopunch_native_probe::management_store::RecordCheckpoint::SecretSent,
        true,
    )
    .await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_at_secret_received() {
    interruption(
        miopunch_native_probe::management_store::RecordCheckpoint::SecretReceived,
        false,
    )
    .await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_at_secret_file_published() {
    interruption(
        miopunch_native_probe::management_store::RecordCheckpoint::SecretFilePublished,
        false,
    )
    .await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_at_secret_saved() {
    interruption(
        miopunch_native_probe::management_store::RecordCheckpoint::SecretSaved,
        false,
    )
    .await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_at_completion_sent() {
    interruption(
        miopunch_native_probe::management_store::RecordCheckpoint::CompletionSent,
        false,
    )
    .await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_at_completion_acknowledged() {
    interruption(
        miopunch_native_probe::management_store::RecordCheckpoint::CompletionAcknowledged,
        true,
    )
    .await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_at_delivery_complete() {
    interruption(
        miopunch_native_probe::management_store::RecordCheckpoint::DeliveryComplete,
        true,
    )
    .await;
}

// Additional adversarial checks use genuine root signatures and TLS streams.
async fn security_fixture() -> (
    tempfile::TempDir,
    p2panda_core::SigningKey,
    miopunch_native_probe::native_network::NodeStatus,
    NativeNetwork,
) {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("creator");
    let mut creator = NativeNetwork::create(&cp).await.unwrap();
    let member = ordinary_member(&mut creator, &dir.path().join("member")).await;
    let saved = std::fs::read(cp.join("genesis.credential")).unwrap();
    let root = p2panda_core::SigningKey::from_bytes(&saved[..32].try_into().unwrap());
    let original = creator.status();
    creator.close().await;
    (dir, root, original, member)
}

fn signed_proof(
    root: &p2panda_core::SigningKey,
    c: &(impl miopunch_native_probe::control_protocol::AuthenticatedControl + ?Sized),
    anchor: miopunch_native_probe::management_auth::NetworkAnchor,
    recovering: iroh_base::EndpointId,
    contact: iroh_base::EndpointId,
) -> [u8; 64] {
    root.sign(&root_statement(c, anchor, recovering, contact))
        .to_bytes()
}

fn root_statement(
    c: &(impl miopunch_native_probe::control_protocol::AuthenticatedControl + ?Sized),
    anchor: miopunch_native_probe::management_auth::NetworkAnchor,
    recovering: iroh_base::EndpointId,
    contact: iroh_base::EndpointId,
) -> Vec<u8> {
    let mut binding = [0; 32];
    c.export_keying_material(&mut binding, b"EXPORTER-Channel-Binding", b"")
        .unwrap();
    let mut statement = b"miopunch/genesis-recovery/v1\0".to_vec();
    statement.extend_from_slice(anchor.root.as_bytes());
    statement.extend_from_slice(anchor.genesis.as_bytes());
    statement.extend_from_slice(recovering.as_bytes());
    statement.extend_from_slice(contact.as_bytes());
    statement.extend_from_slice(&binding);
    statement
}

fn root_record(
    root: &p2panda_core::SigningKey,
    action: p2panda_auth::group::GroupAction<p2panda_core::VerifyingKey>,
    dependencies: Vec<Hash>,
    log_id: [u8; 16],
) -> miopunch_native_probe::management_store::ManagementRecord {
    use miopunch_native_probe::management_auth::ManagementExtension;
    miopunch_native_probe::management_store::ManagementRecord::new(
        p2panda_core::Header::<ManagementExtension>::builder()
            .build(
                root,
                ManagementExtension {
                    version: 1,
                    log_id,
                    group: p2panda_auth::GroupsExtensionArgs {
                        group_id: root.verifying_key(),
                        action,
                        dependencies,
                    },
                },
            )
            .encode(),
        None,
    )
}

async fn invalid_saved_credential(wrong_root: bool) {
    let (dir, root, original, mut member) = security_fixture().await;
    let me = recovery_endpoint(&member).await;
    let credential = if wrong_root {
        GenesisCredential::new(
            p2panda_core::SigningKey::generate(),
            original.anchor.genesis,
        )
    } else {
        let alternate = root_record(
            &root,
            p2panda_auth::group::GroupAction::Create {
                initial_members: vec![
                    (
                        p2panda_auth::group::GroupMember::Individual(original.anchor.root),
                        Access::manage(),
                    ),
                    (
                        p2panda_auth::group::GroupMember::Individual(original.node_id),
                        Access::manage(),
                    ),
                ],
            },
            vec![],
            rand::random(),
        );
        GenesisCredential::from_saved(*root.as_bytes(), Hash::digest(alternate.header()))
    };
    let mut node = NativeNetwork::recover(dir.path().join("recovering"), &credential, me.id())
        .await
        .unwrap();
    let re = recovery_endpoint(&node).await;
    let (c, s) = native_connections::control_pair(&re, &me, MANAGEMENT_RECOVERY_ALPN).await;
    let before = member.status();
    let (received, served) = tokio::join!(
        node.restore_management(&c),
        member.serve_management_recovery(&s)
    );
    assert!(received.is_err() && served.is_err());
    assert_eq!(node.status().applied_count, 0);
    assert_eq!(node.admission_stage(), AdmissionStage::AwaitingApproval);
    assert_eq!(
        node.recovery_progress().await.unwrap().stage,
        RecoveryStage::AwaitingProof
    );
    assert_eq!(member.status(), before);
    re.close().await;
    me.close().await;
    node.close().await;
    member.close().await;
}

#[derive(Clone, Copy)]
enum InvalidGrant {
    Target,
    OldLog,
    OldDependencies,
}

async fn rejected_signed_grant(fault: InvalidGrant) {
    use miopunch_native_probe::management_auth::ManagementExtension;
    use p2panda_auth::group::{GroupAction, GroupMember};
    let (dir, root, original, mut member) = security_fixture().await;
    let me = recovery_endpoint(&member).await;
    let node = NativeNetwork::join(
        dir.path().join("raw-device"),
        ManualInvitation {
            anchor: original.anchor,
            contact: me.id(),
        },
    )
    .await
    .unwrap();
    let re = recovery_endpoint(&node).await;
    let (c, s) = native_connections::control_pair(&re, &me, MANAGEMENT_RECOVERY_ALPN).await;
    let records = member.management_history().await.unwrap();
    let old_log = p2panda_core::Header::<ManagementExtension>::decode(records[0].header())
        .unwrap()
        .extensions
        .log_id;
    let heads = vec![Hash::digest(records.last().unwrap().header())];
    let target = if matches!(fault, InvalidGrant::Target) {
        p2panda_core::SigningKey::generate().verifying_key()
    } else {
        node.status().node_id
    };
    let grant = root_record(
        &root,
        GroupAction::Add {
            member: GroupMember::Individual(target),
            access: Access::manage(),
        },
        if matches!(fault, InvalidGrant::OldDependencies) {
            vec![original.anchor.genesis]
        } else {
            heads
        },
        if matches!(fault, InvalidGrant::OldLog) {
            old_log
        } else {
            rand::random()
        },
    );
    let signature = signed_proof(&root, &c, original.anchor, re.id(), me.id());
    let request = RecoveryRequest {
        target: node.status().node_id,
        attempt: [11; 16],
    };
    let before = member.status();
    let (read, served) = tokio::join!(
        async {
            let exchange = c.open_control_exchange().await.unwrap();
            let (send, recv) = (&exchange, &exchange);
            send_field(send, &signature).await.unwrap();
            send_field(send, &request.attempt).await.unwrap();
            let mut len = [0; 4];
            receive_field(recv, &mut len).await.unwrap();
            let mut history = vec![0; u32::from_be_bytes(len) as usize];
            receive_field(recv, &mut history).await.unwrap();
            send_field(send, &(grant.header().len() as u32).to_be_bytes())
                .await
                .unwrap();
            send_field(send, grant.header()).await.unwrap();
            let mut ack = [0; 32];
            receive_field(recv, &mut ack).await
        },
        member.serve_management_recovery(&s)
    );
    assert!(read.is_err() && served.is_err());
    assert_eq!(member.status(), before);
    let progress = member.served_management_recovery(request).await.unwrap();
    assert_eq!(progress.stage, RecoveryStage::ProofVerified);
    assert!(progress.authorization.is_none());
    re.close().await;
    me.close().await;
    node.close().await;
    member.close().await;
}

async fn send_history_fixture(
    connection: &(impl miopunch_native_probe::control_protocol::AuthenticatedControl + ?Sized),
    root: &p2panda_core::SigningKey,
    anchor: miopunch_native_probe::management_auth::NetworkAnchor,
    local: iroh_base::EndpointId,
    authorization: Option<Hash>,
    records: &[miopunch_native_probe::management_store::ManagementRecord],
) {
    let exchange = connection.accept_control_exchange().await.unwrap();
    let (send, recv) = (&exchange, &exchange);
    let mut signature = [0; 64];
    let mut attempt = [0; 16];
    receive_field(recv, &mut signature).await.unwrap();
    receive_field(recv, &mut attempt).await.unwrap();
    assert!(root.verifying_key().verify(
        &root_statement(connection, anchor, connection.remote_id(), local),
        &p2panda_core::Signature::from_bytes(&signature),
    ));
    let raw: Vec<_> = records
        .iter()
        .map(|record| (record.header().to_vec(), record.body().map(<[u8]>::to_vec)))
        .collect();
    let wire = p2panda_core::cbor::encode_cbor(&(authorization, raw)).unwrap();
    send_field(send, &(wire.len() as u32).to_be_bytes())
        .await
        .unwrap();
    send_field(send, &wire).await.unwrap();
    // The legitimate receiver rejects this genuine signed but unacceptable
    // snapshot before authoring/acknowledging a grant or accepting a secret.
    let mut ack = [0; 1];
    assert!(receive_field(recv, &mut ack).await.is_err());
}

async fn replicate(source: &NativeNetwork, receiver: &mut NativeNetwork) {
    use miopunch_native_probe::admission_gate::ProtocolScope;
    use miopunch_native_probe::offline_replica::REPLICA_ALPN;
    source
        .admission_gate()
        .register_protocol(REPLICA_ALPN, ProtocolScope::Member);
    receiver
        .admission_gate()
        .register_protocol(REPLICA_ALPN, ProtocolScope::Member);
    let se = source.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let re = receiver.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    se.set_alpns(vec![REPLICA_ALPN.to_vec()]);
    re.set_alpns(vec![REPLICA_ALPN.to_vec()]);
    let (c, s) = connect(&re, &se, REPLICA_ALPN).await;
    let (received, served) = tokio::join!(receiver.sync_replica(&c), source.serve_replica(&s));
    assert!(received.unwrap().complete());
    served.unwrap();
    re.close().await;
    se.close().await;
}

#[tokio::test]
async fn wrong_saved_root_cannot_read_history_or_gain_authority() {
    invalid_saved_credential(true).await;
}

#[tokio::test]
async fn same_root_other_signed_create_hash_cannot_replace_original_backup_pin() {
    invalid_saved_credential(false).await;
}

#[tokio::test]
async fn root_proof_for_substituted_target_cannot_read_history_or_secret() {
    let (dir, root, original, mut member) = security_fixture().await;
    let me = recovery_endpoint(&member).await;
    let node = NativeNetwork::join(
        dir.path().join("raw-device"),
        ManualInvitation {
            anchor: original.anchor,
            contact: me.id(),
        },
    )
    .await
    .unwrap();
    let re = recovery_endpoint(&node).await;
    let (c, s) = native_connections::control_pair(&re, &me, MANAGEMENT_RECOVERY_ALPN).await;
    let other = iroh::SecretKey::generate().public();
    let signature = signed_proof(&root, &c, original.anchor, other, me.id());
    let before = member.status();
    let (read, served) = tokio::join!(
        async {
            let exchange = c.open_control_exchange().await.unwrap();
            let (send, recv) = (&exchange, &exchange);
            send_field(send, &signature).await.unwrap();
            let _ = send_field(send, &[9; 16]).await;
            let mut first = [0; 1];
            receive_field(recv, &mut first).await
        },
        member.serve_management_recovery(&s)
    );
    assert!(read.is_err() && served.is_err());
    assert_eq!(member.status(), before);
    re.close().await;
    me.close().await;
    node.close().await;
    member.close().await;
}

#[tokio::test]
async fn signed_grant_for_replaced_target_is_rejected() {
    rejected_signed_grant(InvalidGrant::Target).await;
}

#[tokio::test]
async fn signed_grant_reusing_old_root_log_cannot_complete_recovery() {
    rejected_signed_grant(InvalidGrant::OldLog).await;
}

#[tokio::test]
async fn signed_grant_using_old_history_tips_cannot_complete_recovery() {
    rejected_signed_grant(InvalidGrant::OldDependencies).await;
}

#[tokio::test]
async fn persisted_root_proof_is_not_accepted_on_a_replayed_tls_session() {
    let (dir, root, original, mut member) = security_fixture().await;
    let me = recovery_endpoint(&member).await;
    let node = NativeNetwork::join(
        dir.path().join("raw-device"),
        ManualInvitation {
            anchor: original.anchor,
            contact: me.id(),
        },
    )
    .await
    .unwrap();
    let re = recovery_endpoint(&node).await;
    let request = RecoveryRequest {
        target: node.status().node_id,
        attempt: [13; 16],
    };
    let (c, s) = native_connections::control_pair(&re, &me, MANAGEMENT_RECOVERY_ALPN).await;
    let old_signature = signed_proof(&root, &c, original.anchor, re.id(), me.id());
    let (_, served) = tokio::join!(
        async {
            let exchange = c.open_control_exchange().await.unwrap();
            let (send, recv) = (&exchange, &exchange);
            send_field(send, &old_signature).await.unwrap();
            send_field(send, &request.attempt).await.unwrap();
            let mut len = [0; 4];
            receive_field(recv, &mut len).await.unwrap();
            let mut history = vec![0; u32::from_be_bytes(len) as usize];
            receive_field(recv, &mut history).await.unwrap();
            c.close(0u32.into(), b"interrupt after genuine proof");
        },
        member.serve_management_recovery(&s)
    );
    assert!(served.is_err());
    assert_eq!(
        member
            .served_management_recovery(request)
            .await
            .unwrap()
            .stage,
        RecoveryStage::ProofVerified
    );
    re.close().await;
    me.close().await;
    member.close().await;
    let mut member = NativeNetwork::open(dir.path().join("member"))
        .await
        .unwrap();
    let me = recovery_endpoint(&member).await;
    let re = recovery_endpoint(&node).await;
    let (c, s) = native_connections::control_pair(&re, &me, MANAGEMENT_RECOVERY_ALPN).await;
    let before = member.status();
    let (read, served) = tokio::join!(
        async {
            let exchange = c.open_control_exchange().await.unwrap();
            let (send, recv) = (&exchange, &exchange);
            send_field(send, &old_signature).await.unwrap();
            let _ = send_field(send, &request.attempt).await;
            let mut first = [0; 1];
            receive_field(recv, &mut first).await
        },
        member.serve_management_recovery(&s)
    );
    assert!(read.is_err() && served.is_err());
    assert_eq!(member.status(), before);
    assert_eq!(
        member
            .served_management_recovery(request)
            .await
            .unwrap()
            .stage,
        RecoveryStage::ProofVerified
    );
    re.close().await;
    me.close().await;
    node.close().await;
    member.close().await;
}

#[tokio::test]
async fn same_root_alternate_signed_history_cannot_replace_canonical_network_after_proof() {
    use miopunch_native_probe::management_auth::{ManagementAuth, NetworkAnchor};
    use p2panda_auth::group::{GroupAction, GroupMember};
    let (dir, root, original, member) = security_fixture().await;
    let me = recovery_endpoint(&member).await;
    let create = root_record(
        &root,
        GroupAction::Create {
            initial_members: vec![
                (
                    GroupMember::Individual(original.anchor.root),
                    Access::manage(),
                ),
                (GroupMember::Individual(original.node_id), Access::manage()),
            ],
        },
        vec![],
        rand::random(),
    );
    let alternate_anchor = NetworkAnchor {
        genesis: Hash::digest(create.header()),
        ..original.anchor
    };
    assert_ne!(alternate_anchor.genesis, original.anchor.genesis);
    let member_grant = root_record(
        &root,
        GroupAction::Add {
            member: GroupMember::Individual(member.status().node_id),
            access: Access::read(),
        },
        vec![alternate_anchor.genesis],
        rand::random(),
    );
    let fork = vec![create, member_grant];
    let raw: Vec<_> = fork
        .iter()
        .map(|r| (r.header().to_vec(), r.body().map(<[u8]>::to_vec)))
        .collect();
    let alternate = ManagementAuth::replay_records(alternate_anchor, &raw).unwrap();
    assert!(
        alternate
            .nodes()
            .contains(&(member.status().node_id, Access::read()))
    );
    let credential = GenesisCredential::from_saved(*root.as_bytes(), original.anchor.genesis);
    let mut node = NativeNetwork::recover(dir.path().join("recovering"), &credential, me.id())
        .await
        .unwrap();
    let re = recovery_endpoint(&node).await;
    let (c, s) = native_connections::control_pair(&re, &me, MANAGEMENT_RECOVERY_ALPN).await;
    let (restored, _) = tokio::join!(
        node.restore_management(&c),
        send_history_fixture(&s, &root, original.anchor, me.id(), None, &fork)
    );
    assert!(restored.is_err());
    assert_eq!(node.status().anchor, original.anchor);
    assert_eq!(node.status().applied_count, 0);
    assert_eq!(node.admission_stage(), AdmissionStage::AwaitingApproval);
    assert_eq!(
        node.recovery_progress().await.unwrap().stage,
        RecoveryStage::AwaitingProof
    );
    re.close().await;
    me.close().await;
    node.close().await;
    member.close().await;
}

#[tokio::test]
async fn old_signed_recovery_snapshot_cannot_reverse_current_verified_demotion() {
    let (dir, root, original, mut member) = security_fixture().await;
    let me = recovery_endpoint(&member).await;
    let credential = GenesisCredential::from_saved(*root.as_bytes(), original.anchor.genesis);
    let mut node = NativeNetwork::recover(dir.path().join("recovering"), &credential, me.id())
        .await
        .unwrap();
    let re = recovery_endpoint(&node).await;
    let (c, s) = native_connections::control_pair(&re, &me, MANAGEMENT_RECOVERY_ALPN).await;
    let (restored, served) = tokio::join!(
        node.restore_management(&c),
        member.serve_management_recovery(&s)
    );
    let old = restored.unwrap();
    served.unwrap();
    let old_history = node.management_history().await.unwrap();
    re.close().await;
    me.close().await;
    let target = node.status().node_id;
    let mut session = node.begin_management_session();
    node.elevate_genesis(
        &mut session,
        GenesisCredential::from_saved(*root.as_bytes(), original.anchor.genesis),
        Duration::from_secs(60),
    )
    .unwrap();
    node.manage(&mut session, ManagementAction::DemoteAdministrator(target))
        .await
        .unwrap();
    session.exit_genesis_elevation();
    replicate(&node, &mut member).await;
    let before = node.status();
    assert!(before.members.contains(&(target, Access::read())));
    assert!(node.recovery_progress().await.is_err());
    assert!(
        member
            .served_management_recovery(old.request)
            .await
            .is_err()
    );
    let me = recovery_endpoint(&member).await;
    let re = recovery_endpoint(&node).await;
    let (c, s) = native_connections::control_pair(&re, &me, MANAGEMENT_RECOVERY_ALPN).await;
    let (restored, _) = tokio::join!(
        node.restore_management(&c),
        send_history_fixture(
            &s,
            &root,
            original.anchor,
            me.id(),
            old.authorization,
            &old_history
        )
    );
    assert!(restored.is_err());
    assert_eq!(node.status(), before);
    assert!(node.recovery_progress().await.is_err());
    re.close().await;
    me.close().await;
    let me = recovery_endpoint(&member).await;
    let re = recovery_endpoint(&node).await;
    let (c, s) = native_connections::control_pair(&re, &me, MANAGEMENT_RECOVERY_ALPN).await;
    let (restored, served) = tokio::join!(
        node.restore_management(&c),
        member.serve_management_recovery(&s)
    );
    assert!(restored.is_err() && served.is_err());
    assert_eq!(node.status(), before);
    re.close().await;
    me.close().await;
    node.close().await;
    let node = NativeNetwork::open(dir.path().join("recovering"))
        .await
        .unwrap();
    assert!(node.status().members.contains(&(target, Access::read())));
    assert!(node.recovery_progress().await.is_err());
    node.close().await;
    member.close().await;
}

#[tokio::test]
async fn completed_recovery_retries_same_root_grant_without_duplicate_add() {
    let (dir, root, original, mut member) = security_fixture().await;
    let me = recovery_endpoint(&member).await;
    let credential = GenesisCredential::from_saved(*root.as_bytes(), original.anchor.genesis);
    let rp = dir.path().join("recovering");
    let mut node = NativeNetwork::recover(&rp, &credential, me.id())
        .await
        .unwrap();
    let re = recovery_endpoint(&node).await;
    let (c, s) = native_connections::control_pair(&re, &me, MANAGEMENT_RECOVERY_ALPN).await;
    let (restored, served) = tokio::join!(
        node.restore_management(&c),
        member.serve_management_recovery(&s)
    );
    let original_result = restored.unwrap();
    assert_eq!(served.unwrap().authorization, original_result.authorization);
    re.close().await;
    me.close().await;
    node.close().await;
    member.close().await;
    let mut node = NativeNetwork::open(&rp).await.unwrap();
    let mut member = NativeNetwork::open(dir.path().join("member"))
        .await
        .unwrap();
    let re = recovery_endpoint(&node).await;
    let me = recovery_endpoint(&member).await;
    let (c, s) = native_connections::control_pair(&re, &me, MANAGEMENT_RECOVERY_ALPN).await;
    let (restored, served) = tokio::join!(
        node.restore_management(&c),
        member.serve_management_recovery(&s)
    );
    assert_eq!(restored.unwrap(), original_result);
    assert_eq!(served.unwrap(), original_result);
    assert_eq!(node.status().applied_count, 3);
    assert_eq!(member.status().applied_count, 3);
    assert!(member.manual_requests().await.unwrap().is_empty());
    re.close().await;
    me.close().await;
    node.close().await;
    member.close().await;
}

#[tokio::test]
async fn both_offline_old_administrators_survive_recovery_and_root_elevation_is_explicit() {
    use miopunch_native_probe::daily_management::ManagementError;
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("original");
    let mut original = NativeNetwork::create(&cp).await.unwrap();
    let mut member = ordinary_member(&mut original, &dir.path().join("ordinary")).await;
    let mut old_admin = ordinary_member(&mut original, &dir.path().join("second-admin")).await;
    let first_id = original.status().node_id;
    let second_id = old_admin.status().node_id;
    let anchor = original.status().anchor;
    let mut session = original.begin_management_session();
    original
        .elevate_genesis(&mut session, original_backup(&cp), Duration::from_secs(60))
        .unwrap();
    original
        .manage(
            &mut session,
            ManagementAction::AppointAdministrator(second_id),
        )
        .await
        .unwrap();
    session.exit_genesis_elevation();
    replicate(&original, &mut member).await;
    replicate(&original, &mut old_admin).await;
    assert!(
        old_admin
            .status()
            .members
            .contains(&(second_id, Access::manage()))
    );
    let backup = original_backup(&cp);
    original.close().await;
    old_admin.close().await;
    let me = member.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let mut node = NativeNetwork::recover(dir.path().join("new-device"), &backup, me.id())
        .await
        .unwrap();
    let re = node.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (c, s) = native_connections::control_pair(&re, &me, MANAGEMENT_RECOVERY_ALPN).await;
    let (restored, served) = tokio::join!(
        node.restore_management(&c),
        member.serve_management_recovery(&s)
    );
    assert_eq!(restored.unwrap().stage, RecoveryStage::Complete);
    served.unwrap();
    assert_eq!(node.status().anchor, anchor);
    assert!(
        node.status()
            .members
            .contains(&(first_id, Access::manage()))
    );
    assert!(
        node.status()
            .members
            .contains(&(second_id, Access::manage()))
    );
    assert_ne!(node.status().node_id, anchor.root);
    let target = p2panda_core::SigningKey::generate().verifying_key();
    let mut session = node.begin_management_session();
    assert!(matches!(
        node.manage(&mut session, ManagementAction::AppointAdministrator(target))
            .await,
        Err(ManagementError::ElevationRequired)
    ));
    node.elevate_genesis(&mut session, original_backup(&cp), Duration::from_secs(60))
        .unwrap();
    node.manage(&mut session, ManagementAction::AppointAdministrator(target))
        .await
        .unwrap();
    assert!(
        node.status()
            .members
            .contains(&(first_id, Access::manage()))
    );
    assert!(
        node.status()
            .members
            .contains(&(second_id, Access::manage()))
    );
    re.close().await;
    me.close().await;
    node.close().await;
    member.close().await;
}

#[tokio::test]
async fn different_node_cannot_use_another_devices_recovery_connection() {
    let (dir, root, original, mut member) = security_fixture().await;
    let me = recovery_endpoint(&member).await;
    let credential = GenesisCredential::from_saved(*root.as_bytes(), original.anchor.genesis);
    let a = NativeNetwork::recover(dir.path().join("device-a"), &credential, me.id())
        .await
        .unwrap();
    let mut b = NativeNetwork::recover(dir.path().join("device-b"), &credential, me.id())
        .await
        .unwrap();
    let ae = recovery_endpoint(&a).await;
    let (c, s) = native_connections::control_pair(&ae, &me, MANAGEMENT_RECOVERY_ALPN).await;
    let before = member.status();
    let (restored, served) = tokio::join!(
        b.restore_management(&c),
        member.serve_management_recovery(&s)
    );
    assert!(restored.is_err() && served.is_err());
    assert_eq!(member.status(), before);
    assert_eq!(a.status().applied_count, 0);
    assert_eq!(b.status().applied_count, 0);
    assert_eq!(
        b.recovery_progress().await.unwrap().stage,
        RecoveryStage::AwaitingProof
    );
    ae.close().await;
    me.close().await;
    a.close().await;
    b.close().await;
    member.close().await;
}

#[tokio::test]
async fn another_legitimate_member_cannot_replace_pinned_recovery_contact() {
    let (dir, root, original, member) = security_fixture().await;
    let mut creator = NativeNetwork::open(dir.path().join("creator"))
        .await
        .unwrap();
    let mut alternate = ordinary_member(&mut creator, &dir.path().join("alternate")).await;
    creator.close().await;
    let me = recovery_endpoint(&member).await;
    let ae = recovery_endpoint(&alternate).await;
    let credential = GenesisCredential::from_saved(*root.as_bytes(), original.anchor.genesis);
    let mut node = NativeNetwork::recover(dir.path().join("recovering"), &credential, me.id())
        .await
        .unwrap();
    let re = recovery_endpoint(&node).await;
    let (c, s) = native_connections::control_pair(&re, &ae, MANAGEMENT_RECOVERY_ALPN).await;
    let before = alternate.status();
    let (restored, served) = tokio::join!(
        node.restore_management(&c),
        alternate.serve_management_recovery(&s)
    );
    assert!(restored.is_err() && served.is_err());
    assert_eq!(alternate.status(), before);
    assert_eq!(node.status().applied_count, 0);
    assert_eq!(
        node.recovery_progress().await.unwrap().stage,
        RecoveryStage::AwaitingProof
    );
    re.close().await;
    me.close().await;
    ae.close().await;
    node.close().await;
    member.close().await;
    alternate.close().await;
}

#[tokio::test]
async fn member_without_retained_current_secret_fails_closed_without_history_delivery() {
    let (dir, root, original, member) = security_fixture().await;
    member.close().await;
    // A genuine loss of the only online provider's private material is not
    // repaired by its signed membership history or the root backup alone.
    std::fs::remove_file(dir.path().join("member/admission.material")).unwrap();
    let mut member = NativeNetwork::open(dir.path().join("member"))
        .await
        .unwrap();
    assert_eq!(member.admission_stage(), AdmissionStage::PendingMaterials);
    let me = recovery_endpoint(&member).await;
    let credential = GenesisCredential::from_saved(*root.as_bytes(), original.anchor.genesis);
    let rp = dir.path().join("recovering");
    let mut node = NativeNetwork::recover(&rp, &credential, me.id())
        .await
        .unwrap();
    assert!(
        node.begin_rejoin(ManualInvitation {
            anchor: original.anchor,
            contact: me.id()
        })
        .await
        .is_err()
    );
    let re = recovery_endpoint(&node).await;
    let (c, s) = native_connections::control_pair(&re, &me, MANAGEMENT_RECOVERY_ALPN).await;
    let before = member.status();
    let (restored, served) = tokio::join!(
        node.restore_management(&c),
        member.serve_management_recovery(&s)
    );
    assert!(restored.is_err() && served.is_err());
    assert_eq!(member.status(), before);
    assert_eq!(node.status().applied_count, 0);
    assert_eq!(
        node.recovery_progress().await.unwrap().stage,
        RecoveryStage::AwaitingProof
    );
    assert_eq!(node.admission_stage(), AdmissionStage::AwaitingApproval);
    re.close().await;
    me.close().await;
    node.close().await;
    let node = NativeNetwork::open(&rp).await.unwrap();
    assert_eq!(
        node.recovery_progress().await.unwrap().stage,
        RecoveryStage::AwaitingProof
    );
    node.close().await;
    member.close().await;
}

#[tokio::test]
async fn completed_recovery_after_effective_removal_can_rejoin_with_fresh_ordinary_approval() {
    let (dir, root, original, mut member) = security_fixture().await;
    let me = recovery_endpoint(&member).await;
    let credential = GenesisCredential::from_saved(*root.as_bytes(), original.anchor.genesis);
    let rp = dir.path().join("recovering");
    let mut node = NativeNetwork::recover(&rp, &credential, me.id())
        .await
        .unwrap();
    let re = recovery_endpoint(&node).await;
    let (c, s) = native_connections::control_pair(&re, &me, MANAGEMENT_RECOVERY_ALPN).await;
    let (restored, served) = tokio::join!(
        node.restore_management(&c),
        member.serve_management_recovery(&s)
    );
    let old = restored.unwrap();
    served.unwrap();
    re.close().await;
    me.close().await;
    let identity = node.status().node_id;
    let mut session = node.begin_management_session();
    node.elevate_genesis(
        &mut session,
        GenesisCredential::from_saved(*root.as_bytes(), original.anchor.genesis),
        Duration::from_secs(60),
    )
    .unwrap();
    node.manage(
        &mut session,
        ManagementAction::AppointAdministrator(member.status().node_id),
    )
    .await
    .unwrap();
    replicate(&node, &mut member).await;
    node.manage(&mut session, ManagementAction::RemoveMember(identity))
        .await
        .unwrap();
    session.exit_genesis_elevation();
    // The valid new administrator learns the actual signed removal through
    // public record receipt; a removed node cannot open normal LogSync.
    let removal = node
        .management_history()
        .await
        .unwrap()
        .last()
        .unwrap()
        .clone();
    member
        .ingest_management_record(removal.header(), removal.body())
        .await
        .unwrap();
    assert!(node.recovery_progress().await.is_err());
    assert!(
        member
            .served_management_recovery(old.request)
            .await
            .is_err()
    );
    assert_eq!(node.admission_stage(), AdmissionStage::AwaitingApproval);
    node.close().await;
    let mut node = NativeNetwork::open(&rp).await.unwrap();
    let before = node.status();
    assert_eq!(before.node_id, identity);
    let invitation = member.manual_invitation().unwrap();
    node.begin_rejoin(invitation).await.unwrap();
    assert_eq!(node.status(), before); // Preparing a request grants no membership.
    node.close().await;
    let mut node = NativeNetwork::open(&rp).await.unwrap();
    assert_eq!(node.status(), before);
    let ne = node.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let me = member.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (c, s) = native_connections::control_pair(&ne, &me, ADMISSION_REQUEST_ALPN).await;
    let (requested, served) = tokio::join!(
        node.request_manual_admission(&c),
        member.serve_manual_admission(&s)
    );
    assert!(matches!(
        requested.unwrap(),
        ManualExchange::AwaitingApproval
    ));
    served.unwrap();
    let request = member.manual_requests().await.unwrap().pop().unwrap();
    assert_eq!(request.applicant, identity);
    assert_ne!(request.request_id, old.request.attempt);
    let fresh = member.approve_manual_admission(&request).await.unwrap();
    assert_ne!(Some(fresh.authorization), old.authorization);
    let receive = async {
        let ManualExchange::Materials(material) = node.request_manual_admission(&c).await.unwrap()
        else {
            panic!("fresh approval material")
        };
        node.complete_manual_admission(material).await.unwrap();
    };
    let (_, served) = tokio::join!(receive, member.serve_manual_admission(&s));
    served.unwrap();
    assert_eq!(node.admission_stage(), AdmissionStage::Complete);
    assert!(node.status().members.contains(&(identity, Access::read())));
    assert!(
        !node
            .status()
            .members
            .contains(&(identity, Access::manage()))
    );
    assert_eq!(node.status().anchor, original.anchor);
    assert!(node.recovery_progress().await.is_err());
    assert!(
        member
            .served_management_recovery(old.request)
            .await
            .is_err()
    );
    ne.close().await;
    me.close().await;
    node.close().await;
    let node = NativeNetwork::open(&rp).await.unwrap();
    assert_eq!(node.status().node_id, identity);
    assert_eq!(node.admission_stage(), AdmissionStage::Complete);
    assert!(node.status().members.contains(&(identity, Access::read())));
    assert!(node.recovery_progress().await.is_err());
    node.close().await;
    member.close().await;
}

#[cfg(feature = "replica-test-hooks")]
async fn removed_recovery_for_rejoin() -> (
    tempfile::TempDir,
    NativeNetwork,
    NativeNetwork,
    miopunch_native_probe::management_recovery::RecoveryProgress,
) {
    let (dir, root, original, mut member) = security_fixture().await;
    let me = recovery_endpoint(&member).await;
    let credential = GenesisCredential::from_saved(*root.as_bytes(), original.anchor.genesis);
    let rp = dir.path().join("recovering");
    let mut node = NativeNetwork::recover(&rp, &credential, me.id())
        .await
        .unwrap();
    let re = recovery_endpoint(&node).await;
    let (c, s) = native_connections::control_pair(&re, &me, MANAGEMENT_RECOVERY_ALPN).await;
    let (restored, served) = tokio::join!(
        node.restore_management(&c),
        member.serve_management_recovery(&s)
    );
    let old = restored.unwrap();
    served.unwrap();
    re.close().await;
    me.close().await;
    let identity = node.status().node_id;
    let mut session = node.begin_management_session();
    node.elevate_genesis(
        &mut session,
        GenesisCredential::from_saved(*root.as_bytes(), original.anchor.genesis),
        Duration::from_secs(60),
    )
    .unwrap();
    node.manage(
        &mut session,
        ManagementAction::AppointAdministrator(member.status().node_id),
    )
    .await
    .unwrap();
    replicate(&node, &mut member).await;
    node.manage(&mut session, ManagementAction::RemoveMember(identity))
        .await
        .unwrap();
    session.exit_genesis_elevation();
    // The valid new administrator learns the actual signed removal through
    // public record receipt; a removed node cannot open normal LogSync.
    let removal = node
        .management_history()
        .await
        .unwrap()
        .last()
        .unwrap()
        .clone();
    member
        .ingest_management_record(removal.header(), removal.body())
        .await
        .unwrap();
    assert!(node.recovery_progress().await.is_err());
    assert!(
        member
            .served_management_recovery(old.request)
            .await
            .is_err()
    );
    assert_eq!(node.admission_stage(), AdmissionStage::AwaitingApproval);
    (dir, node, member, old)
}

#[cfg(feature = "replica-test-hooks")]
async fn rejoin_publication_interruption(
    point: miopunch_native_probe::management_store::RecordCheckpoint,
) {
    use miopunch_native_probe::management_store::RecordCheckpoint;
    let (dir, node, mut admin, old) = removed_recovery_for_rejoin().await;
    let rp = dir.path().join("recovering");
    let before = node.status();
    let invitation = admin.manual_invitation().unwrap();
    let invitation_path = dir.path().join("fresh-trusted-invitation");
    std::fs::write(&invitation_path, invitation.encode()).unwrap();
    node.close().await;
    let output = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "recovery_checkpoint_worker", "--nocapture"])
        .env("MIO36_NODE_PATH", &rp)
        .env("MIO36_ROLE", "rejoin")
        .env("MIO36_CHECKPOINT", format!("{point:?}"))
        .env("MIO36_REJOIN_INVITATION_PATH", &invitation_path)
        .output()
        .await
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(86),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!("{}", String::from_utf8_lossy(&output.stderr));
    let mut node = NativeNetwork::open(&rp).await.unwrap();
    assert_eq!(node.status(), before);
    assert_eq!(node.admission_stage(), AdmissionStage::AwaitingApproval);
    assert!(node.recovery_progress().await.is_err());
    if point == RecordCheckpoint::RecoveryRejoinPrepared {
        node.begin_rejoin(invitation).await.unwrap();
    }
    let ne = node.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let ae = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (c, s) = native_connections::control_pair(&ne, &ae, ADMISSION_REQUEST_ALPN).await;
    let (requested, served) = tokio::join!(
        node.request_manual_admission(&c),
        admin.serve_manual_admission(&s)
    );
    assert!(matches!(
        requested.unwrap(),
        ManualExchange::AwaitingApproval
    ));
    served.unwrap();
    let request = admin.manual_requests().await.unwrap().pop().unwrap();
    assert_eq!(request.applicant, before.node_id);
    assert_ne!(request.request_id, old.request.attempt);
    ne.close().await;
    ae.close().await;
    node.close().await;
    let mut node = NativeNetwork::open(&rp).await.unwrap();
    let ne = node.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let ae = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (c, s) = native_connections::control_pair(&ne, &ae, ADMISSION_REQUEST_ALPN).await;
    let (requested, served) = tokio::join!(
        node.request_manual_admission(&c),
        admin.serve_manual_admission(&s)
    );
    assert!(matches!(
        requested.unwrap(),
        ManualExchange::AwaitingApproval
    ));
    served.unwrap();
    assert_eq!(
        admin.manual_requests().await.unwrap(),
        vec![request.clone()]
    );
    let approved = admin.approve_manual_admission(&request).await.unwrap();
    assert_ne!(Some(approved.authorization), old.authorization);
    let receive = async {
        let ManualExchange::Materials(material) = node.request_manual_admission(&c).await.unwrap()
        else {
            panic!("fresh approval")
        };
        node.complete_manual_admission(material).await.unwrap();
    };
    let (_, served) = tokio::join!(receive, admin.serve_manual_admission(&s));
    served.unwrap();
    assert!(
        node.status()
            .members
            .contains(&(before.node_id, Access::read()))
    );
    assert!(
        !node
            .status()
            .members
            .contains(&(before.node_id, Access::manage()))
    );
    assert!(admin.served_management_recovery(old.request).await.is_err());
    ne.close().await;
    ae.close().await;
    node.close().await;
    let node = NativeNetwork::open(&rp).await.unwrap();
    assert_eq!(node.admission_stage(), AdmissionStage::Complete);
    assert_eq!(node.status().node_id, before.node_id);
    assert_eq!(node.status().anchor, before.anchor);
    assert!(
        node.status()
            .members
            .contains(&(before.node_id, Access::read()))
    );
    node.close().await;
    admin.close().await;
    eprintln!(
        "MIO36_REJOIN_SWITCH actual_exit=86 boundary={point:?} stable_identity=true preserved_anchor_and_history=true fresh_request_and_grant=true old_root_grant_still_invalid=true"
    );
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_after_ordinary_binding_publication_reopens_fresh_rejoin() {
    rejoin_publication_interruption(
        miopunch_native_probe::management_store::RecordCheckpoint::RecoveryRejoinPublished,
    )
    .await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_after_historical_completion_seal_before_binding_retries_fresh_rejoin() {
    rejoin_publication_interruption(
        miopunch_native_probe::management_store::RecordCheckpoint::RecoveryRejoinPrepared,
    )
    .await;
}
