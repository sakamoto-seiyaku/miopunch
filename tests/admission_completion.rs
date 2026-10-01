//! Real public-node precode behavior.
use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointAddr};
use miopunch_native_probe::admission_gate::ADMISSION_REQUEST_ALPN;
use miopunch_native_probe::manual_admission::{AdmissionStage, ManualExchange};
use miopunch_native_probe::native_network::NativeNetwork;
use std::net::Ipv4Addr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
async fn connect(client: &Endpoint, server: &Endpoint, alpn: &[u8]) -> (Connection, Connection) {
    let target = EndpointAddr::new(server.id()).with_addrs(
        server
            .addr()
            .ip_addrs()
            .copied()
            .map(iroh_base::TransportAddr::Ip),
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        let (out, incoming) = tokio::join!(client.connect(target, alpn), async {
            server.accept().await.unwrap().await
        });
        (out.unwrap(), incoming.unwrap())
    })
    .await
    .expect("bounded real TLS connection")
}

fn expiry() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600
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
    let (received, sent) = tokio::join!(receiver.sync_replica(&c), source.serve_replica(&s));
    assert!(received.unwrap().complete());
    sent.unwrap();
    re.close().await;
    se.close().await;
}

async fn redeem(
    admin: &mut NativeNetwork,
    joining: &mut NativeNetwork,
    code: &miopunch_native_probe::precode::Precode,
) -> bool {
    let ae = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let je = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    ae.set_alpns(vec![ADMISSION_REQUEST_ALPN.to_vec()]);
    let (c, s) = connect(&je, &ae, ADMISSION_REQUEST_ALPN).await;
    let receive = async {
        match joining.request_precode_admission(&c, code).await {
            Ok(ManualExchange::Materials(d)) => joining.complete_manual_admission(d).await.is_ok(),
            _ => false,
        }
    };
    let (received, sent) = tokio::join!(receive, admin.serve_precode_admission(&s));
    assert_eq!(received, sent.is_ok());
    je.close().await;
    ae.close().await;
    received
}

async fn second_administrator(
    original: &mut NativeNetwork,
    root_path: &std::path::Path,
    path: &std::path::Path,
) -> NativeNetwork {
    use miopunch_native_probe::daily_management::ManagementAction;
    use miopunch_native_probe::genesis_recovery::GenesisCredential;
    use p2panda_core::Hash;
    let seed = original.issue_precode(expiry(), Some(1)).await.unwrap();
    let mut alternate = NativeNetwork::join(path, seed.invitation()).await.unwrap();
    assert!(redeem(original, &mut alternate, &seed).await);
    let bytes = std::fs::read(root_path.join("genesis.credential")).unwrap();
    let credential = GenesisCredential::from_saved(
        bytes[..32].try_into().unwrap(),
        Hash::from_bytes(bytes[32..].try_into().unwrap()),
    );
    let mut session = original.begin_management_session();
    original
        .elevate_genesis(&mut session, credential, Duration::from_secs(60))
        .unwrap();
    original
        .manage(
            &mut session,
            ManagementAction::AppointAdministrator(alternate.status().node_id),
        )
        .await
        .unwrap();
    replicate(original, &mut alternate).await;
    alternate
}

#[tokio::test]
async fn unavailable_original_new_contact_consumes_new_code_reuses_verified_grant() {
    let dir = tempfile::tempdir().unwrap();
    let root_path = dir.path().join("original");
    let join_path = dir.path().join("joining");
    let mut original = NativeNetwork::create(&root_path).await.unwrap();
    let mut alternate =
        second_administrator(&mut original, &root_path, &dir.path().join("alternate")).await;
    let old = original.issue_precode(expiry(), Some(1)).await.unwrap();
    let mut joining = NativeNetwork::join(&join_path, old.invitation())
        .await
        .unwrap();
    let id = joining.status().node_id;
    let oe = original.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let je = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (c, s) = connect(&je, &oe, ADMISSION_REQUEST_ALPN).await;
    let receiver = async {
        let ManualExchange::Materials(delivery) =
            joining.request_precode_admission(&c, &old).await.unwrap()
        else {
            panic!("grant")
        };
        assert_eq!(joining.admission_stage(), AdmissionStage::PendingMaterials);
        c.close(0u32.into(), b"secret interrupted");
        drop(delivery);
    };
    let (_, sent) = tokio::join!(receiver, original.serve_precode_admission(&s));
    assert!(sent.is_err());
    let grant = original.precode_redemptions(&old).await.unwrap()[0].authorization;
    assert_eq!(original.precode_usage(&old).await.unwrap(), 1);
    je.close().await;
    oe.close().await;
    // The second administrator learns the original signed grant through real
    // authenticated LogSync, never through an injected membership boolean.
    replicate(&original, &mut alternate).await;
    let before = alternate.status().applied_count;
    assert!(
        alternate
            .status()
            .members
            .iter()
            .any(|(peer, _)| *peer == id)
    );
    original.close().await; // no original endpoint/store is available now
    let fresh = alternate.issue_precode(expiry(), Some(1)).await.unwrap();
    assert!(alternate.precode_usage(&old).await.is_err());
    assert!(joining.begin_rejoin(fresh.invitation()).await.is_err());
    joining.begin_precode_completion(&fresh).await.unwrap();
    joining.close().await;
    let mut joining = NativeNetwork::open(&join_path).await.unwrap();
    assert_eq!(joining.status().node_id, id);
    assert_eq!(joining.admission_stage(), AdmissionStage::PendingMaterials);
    joining.begin_precode_completion(&fresh).await.unwrap(); // idempotent binding
    assert!(redeem(&mut alternate, &mut joining, &fresh).await);
    assert_eq!(alternate.precode_usage(&fresh).await.unwrap(), 1);
    assert_eq!(
        alternate.precode_redemptions(&fresh).await.unwrap()[0].authorization,
        grant
    );
    assert_eq!(alternate.status().applied_count, before);
    assert!(redeem(&mut alternate, &mut joining, &fresh).await);
    assert_eq!(alternate.precode_usage(&fresh).await.unwrap(), 1);
    eprintln!(
        "MIO35_PRIORITY original_offline=true learned_grant=real_logsync new_uses=1 same_grant=true duplicate_add=false stable_identity=true"
    );
    joining.close().await;
    alternate.close().await;
}

#[cfg(feature = "replica-test-hooks")]
#[test]
fn delivery_checkpoint_worker() {
    let Ok(node_path) = std::env::var("MIO35_NODE_PATH") else {
        return;
    };
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut node = NativeNetwork::open(node_path).await.unwrap();
        node.set_replica_checkpoint_observer(|stage| {
            if format!("{stage:?}") == std::env::var("MIO35_CHECKPOINT").unwrap() {
                eprintln!("MIO35_CRASH real_process_exit_at={stage:?}");
                std::process::exit(83);
            }
        });
        let endpoint = node.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
        if std::env::var("MIO35_ROLE").unwrap() == "sender" {
            let public = format!(
                "{}\n{}",
                endpoint.id(),
                endpoint.addr().ip_addrs().next().unwrap()
            );
            std::fs::write(std::env::var("MIO35_PUBLIC_PATH").unwrap(), public).unwrap();
            let c = endpoint.accept().await.unwrap().await.unwrap();
            node.serve_precode_admission(&c).await.unwrap();
        } else {
            let contact = std::env::var("MIO35_CONTACT").unwrap().parse().unwrap();
            let address = std::env::var("MIO35_ADDRESS").unwrap().parse().unwrap();
            let code_bytes: [u8; 128] = std::fs::read(std::env::var("MIO35_CODE_PATH").unwrap())
                .unwrap()
                .try_into()
                .unwrap();
            let code = miopunch_native_probe::precode::Precode::decode(&code_bytes).unwrap();
            let c = endpoint
                .connect(
                    EndpointAddr::new(contact).with_ip_addr(address),
                    ADMISSION_REQUEST_ALPN,
                )
                .await
                .unwrap();
            let ManualExchange::Materials(d) =
                node.request_precode_admission(&c, &code).await.unwrap()
            else {
                panic!("approved")
            };
            node.complete_manual_admission(d).await.unwrap();
        }
        panic!("actual checkpoint was not reached");
    });
}

#[cfg(feature = "replica-test-hooks")]
async fn interruption(checkpoint: &str, sender_crashes: bool) {
    let dir = tempfile::tempdir().unwrap();
    let admin_path = dir.path().join("admin");
    let join_path = dir.path().join("joining");
    let mut admin = NativeNetwork::create(&admin_path).await.unwrap();
    let code = admin.issue_precode(expiry(), Some(1)).await.unwrap();
    let mut joining = NativeNetwork::join(&join_path, code.invitation())
        .await
        .unwrap();
    let id = joining.status().node_id;
    let code_path = dir.path().join("private-code");
    std::fs::write(&code_path, code.encode()).unwrap();
    let public_path = dir.path().join("public-contact");
    let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "delivery_checkpoint_worker", "--nocapture"])
        .env("MIO35_CHECKPOINT", checkpoint)
        .env("MIO35_CODE_PATH", &code_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if sender_crashes {
        admin.close().await;
        command
            .env("MIO35_NODE_PATH", &admin_path)
            .env("MIO35_ROLE", "sender")
            .env("MIO35_PUBLIC_PATH", &public_path);
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
        let contact = lines.next().unwrap().parse().unwrap();
        let address = lines.next().unwrap().parse().unwrap();
        let je = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
        let c = je
            .connect(
                EndpointAddr::new(contact).with_ip_addr(address),
                ADMISSION_REQUEST_ALPN,
            )
            .await
            .unwrap();
        let request = async {
            if let Ok(ManualExchange::Materials(d)) =
                joining.request_precode_admission(&c, &code).await
            {
                let _ = joining.complete_manual_admission(d).await;
            }
        };
        let (_, exited) = tokio::join!(
            request,
            tokio::time::timeout(Duration::from_secs(20), child.wait_with_output())
        );
        let exited = exited.unwrap().unwrap();
        assert_eq!(
            exited.status.code(),
            Some(83),
            "{}",
            String::from_utf8_lossy(&exited.stderr)
        );
        eprintln!("{}", String::from_utf8_lossy(&exited.stderr));
        je.close().await;
        joining.close().await;
    } else {
        joining.close().await;
        let ae = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
        command
            .env("MIO35_NODE_PATH", &join_path)
            .env("MIO35_ROLE", "receiver")
            .env("MIO35_CONTACT", ae.id().to_string())
            .env(
                "MIO35_ADDRESS",
                ae.addr().ip_addrs().next().unwrap().to_string(),
            );
        let child = command.spawn().unwrap();
        let serve = async {
            let c = ae.accept().await.unwrap().await.unwrap();
            let _ = admin.serve_precode_admission(&c).await;
        };
        let (_, exited) = tokio::join!(
            serve,
            tokio::time::timeout(Duration::from_secs(20), child.wait_with_output())
        );
        let exited = exited.unwrap().unwrap();
        assert_eq!(
            exited.status.code(),
            Some(83),
            "{}",
            String::from_utf8_lossy(&exited.stderr)
        );
        eprintln!("{}", String::from_utf8_lossy(&exited.stderr));
        ae.close().await;
        admin.close().await;
    }
    let mut admin = NativeNetwork::open(&admin_path).await.unwrap();
    let original = admin
        .precode_redemptions(&code)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(admin.precode_usage(&code).await.unwrap(), 1);
    let mut joining = NativeNetwork::open(&join_path).await.unwrap();
    assert_eq!(joining.status().node_id, id);
    let expected = match checkpoint {
        "AuthorizationPrepared" | "AuthorizationVerified" => Some(AdmissionStage::AwaitingApproval),
        "AuthorizationSaved"
        | "AuthorizationAcknowledged"
        | "SecretPrepared"
        | "SecretReceived"
        | "SecretFilePublished" => Some(AdmissionStage::PendingMaterials),
        "SecretSaved" | "CompletionSent" | "CompletionAcknowledged" | "DeliveryComplete" => {
            Some(AdmissionStage::Complete)
        }
        "AuthorizationSent" | "SecretSent" => None, // queued bytes may or may not have arrived
        _ => panic!("unknown checkpoint"),
    };
    if let Some(expected) = expected {
        assert_eq!(joining.admission_stage(), expected);
    }
    assert!(redeem(&mut admin, &mut joining, &code).await);
    let resumed = admin
        .precode_redemptions(&code)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(resumed.authorization, original.authorization);
    assert_eq!(resumed.request, original.request);
    assert_eq!(resumed.stage, AdmissionStage::Complete);
    assert_eq!(admin.precode_usage(&code).await.unwrap(), 1);
    assert_eq!(admin.status().applied_count, 2);
    assert_eq!(joining.admission_stage(), AdmissionStage::Complete);
    eprintln!(
        "MIO35_RESUME checkpoint={checkpoint} real_exit=83 stable_identity=true same_result=true uses=1 applied=2 durable_complete=true"
    );
    joining.close().await;
    admin.close().await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_after_secret_send_resumes_original_code_once() {
    interruption("SecretSent", true).await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_at_authorization_prepared() {
    interruption("AuthorizationPrepared", true).await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_at_authorization_sent() {
    interruption("AuthorizationSent", true).await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_at_authorization_acknowledged() {
    interruption("AuthorizationAcknowledged", true).await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_at_secret_prepared() {
    interruption("SecretPrepared", true).await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_at_completion_acknowledged() {
    interruption("CompletionAcknowledged", true).await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_at_delivery_complete() {
    interruption("DeliveryComplete", true).await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_at_authorization_verified() {
    interruption("AuthorizationVerified", false).await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_at_authorization_saved() {
    interruption("AuthorizationSaved", false).await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_at_secret_received() {
    interruption("SecretReceived", false).await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_at_secret_file_published() {
    interruption("SecretFilePublished", false).await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_at_secret_saved() {
    interruption("SecretSaved", false).await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_at_completion_sent() {
    interruption("CompletionSent", false).await;
}

#[tokio::test]
async fn new_contact_rejects_wrong_network_tls_contact_and_stale_delivery_handle() {
    let dir = tempfile::tempdir().unwrap();
    let root_path = dir.path().join("original");
    let mut original = NativeNetwork::create(&root_path).await.unwrap();
    let mut alternate =
        second_administrator(&mut original, &root_path, &dir.path().join("alternate")).await;
    let old = original.issue_precode(expiry(), Some(1)).await.unwrap();
    let fresh = alternate.issue_precode(expiry(), Some(1)).await.unwrap();
    let mut unrelated = NativeNetwork::create(dir.path().join("unrelated"))
        .await
        .unwrap();
    let foreign = unrelated.issue_precode(expiry(), Some(1)).await.unwrap();
    let mut joining = NativeNetwork::join(dir.path().join("joining"), old.invitation())
        .await
        .unwrap();
    let oe = original.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let je = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (c, s) = connect(&je, &oe, ADMISSION_REQUEST_ALPN).await;
    let receiver = async {
        let ManualExchange::Materials(d) =
            joining.request_precode_admission(&c, &old).await.unwrap()
        else {
            panic!("grant")
        };
        assert!(joining.begin_precode_completion(&foreign).await.is_err());
        joining.begin_precode_completion(&fresh).await.unwrap();
        // A new contact binding cannot authorize a late delivery from the old
        // connection, even though that connection originally authenticated.
        assert!(joining.complete_manual_admission(d).await.is_err());
        assert_eq!(joining.admission_stage(), AdmissionStage::PendingMaterials);
    };
    let (_, sent) = tokio::join!(receiver, original.serve_precode_admission(&s));
    assert!(sent.is_err());
    assert_eq!(original.precode_usage(&old).await.unwrap(), 1);
    let (wrong_contact, peer) = connect(&je, &oe, ADMISSION_REQUEST_ALPN).await;
    assert!(
        joining
            .request_precode_admission(&wrong_contact, &fresh)
            .await
            .is_err()
    );
    peer.close(0u32.into(), b"wrong contact");
    je.close().await;
    oe.close().await;
    // Returning to the original contact preserves the original code attempt.
    joining.begin_precode_completion(&old).await.unwrap();
    assert!(redeem(&mut original, &mut joining, &old).await);
    assert_eq!(original.precode_usage(&old).await.unwrap(), 1);
    assert_eq!(alternate.precode_usage(&fresh).await.unwrap(), 0);
    eprintln!(
        "MIO35_CONTACT wrong_network_rejected=true wrong_tls_contact_rejected=true stale_handle_rejected=true original_attempt_preserved=true"
    );
    joining.close().await;
    original.close().await;
    alternate.close().await;
    unrelated.close().await;
}

#[tokio::test]
async fn replacing_applicant_cannot_complete_another_identity_delivery() {
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let code = admin.issue_precode(expiry(), Some(1)).await.unwrap();
    let mut joining = NativeNetwork::join(dir.path().join("joining"), code.invitation())
        .await
        .unwrap();
    let mut substitute = NativeNetwork::join(dir.path().join("substitute"), code.invitation())
        .await
        .unwrap();
    let ae = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let je = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (c, s) = connect(&je, &ae, ADMISSION_REQUEST_ALPN).await;
    let receiver = async {
        let ManualExchange::Materials(d) =
            joining.request_precode_admission(&c, &code).await.unwrap()
        else {
            panic!("grant")
        };
        assert!(substitute.complete_manual_admission(d).await.is_err());
        assert_eq!(
            substitute.admission_stage(),
            AdmissionStage::AwaitingApproval
        );
        assert_eq!(joining.admission_stage(), AdmissionStage::PendingMaterials);
    };
    let (_, sent) = tokio::join!(receiver, admin.serve_precode_admission(&s));
    assert!(sent.is_err());
    je.close().await;
    ae.close().await;
    assert!(redeem(&mut admin, &mut joining, &code).await);
    assert!(!redeem(&mut admin, &mut substitute, &code).await);
    assert_eq!(admin.precode_usage(&code).await.unwrap(), 1);
    assert_eq!(admin.precode_redemptions(&code).await.unwrap().len(), 1);
    eprintln!("MIO35_IDENTITY wrong_node_rejected=true rightful_resume=true uses=1");
    joining.close().await;
    substitute.close().await;
    admin.close().await;
}

#[tokio::test]
async fn new_contact_after_removal_gets_new_authorization_and_old_result_stays_invalid() {
    use miopunch_native_probe::daily_management::ManagementAction;
    let dir = tempfile::tempdir().unwrap();
    let root_path = dir.path().join("original");
    let mut original = NativeNetwork::create(&root_path).await.unwrap();
    let mut alternate =
        second_administrator(&mut original, &root_path, &dir.path().join("alternate")).await;
    let old = original.issue_precode(expiry(), Some(1)).await.unwrap();
    let mut joining = NativeNetwork::join(dir.path().join("joining"), old.invitation())
        .await
        .unwrap();
    let id = joining.status().node_id;
    let oe = original.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let je = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (c, s) = connect(&je, &oe, ADMISSION_REQUEST_ALPN).await;
    let receiver = async {
        let ManualExchange::Materials(d) =
            joining.request_precode_admission(&c, &old).await.unwrap()
        else {
            panic!("grant")
        };
        c.close(0u32.into(), b"secret interrupted");
        drop(d);
    };
    let (_, sent) = tokio::join!(receiver, original.serve_precode_admission(&s));
    assert!(sent.is_err());
    let approval = original.precode_redemptions(&old).await.unwrap()[0].clone();
    je.close().await;
    oe.close().await;
    let mut session = original.begin_management_session();
    original
        .manage(&mut session, ManagementAction::RemoveMember(id))
        .await
        .unwrap();
    replicate(&original, &mut alternate).await;
    // An application receiving the real signed removal through the public
    // management ingest seam gets the same causal verifier and durable view.
    for record in original.management_history().await.unwrap() {
        joining
            .ingest_management_record(record.header(), record.body())
            .await
            .unwrap();
    }
    assert_eq!(joining.admission_stage(), AdmissionStage::AwaitingApproval);
    assert!(!redeem(&mut original, &mut joining, &old).await);
    assert!(
        original
            .approved_admission(&approval.request)
            .await
            .is_err()
    );
    let fresh = alternate.issue_precode(expiry(), Some(1)).await.unwrap();
    let before = alternate.status().applied_count;
    joining.begin_precode_completion(&fresh).await.unwrap();
    assert!(redeem(&mut alternate, &mut joining, &fresh).await);
    let renewed = alternate.precode_redemptions(&fresh).await.unwrap()[0].authorization;
    assert_ne!(renewed, approval.authorization);
    assert_eq!(alternate.status().applied_count, before + 1);
    assert_eq!(alternate.precode_usage(&fresh).await.unwrap(), 1);
    replicate(&alternate, &mut original).await;
    assert!(
        original
            .approved_admission(&approval.request)
            .await
            .is_err()
    );
    assert_eq!(original.precode_usage(&old).await.unwrap(), 1);
    assert_eq!(joining.admission_stage(), AdmissionStage::Complete);
    eprintln!(
        "MIO35_REVOKED old_result_rejected=true new_contact_new_grant=true new_uses=1 original_uses=1"
    );
    joining.close().await;
    alternate.close().await;
    original.close().await;
}

#[tokio::test]
async fn signed_old_response_cannot_restore_locally_verified_removal() {
    use miopunch_native_probe::daily_management::ManagementAction;
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let code = admin.issue_precode(expiry(), Some(1)).await.unwrap();
    let mut joining = NativeNetwork::join(dir.path().join("joining"), code.invitation())
        .await
        .unwrap();
    let ae = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let je = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (c, s) = connect(&je, &ae, ADMISSION_REQUEST_ALPN).await;
    let receive = async {
        let ManualExchange::Materials(d) =
            joining.request_precode_admission(&c, &code).await.unwrap()
        else {
            panic!("grant")
        };
        c.close(0u32.into(), b"before secret");
        drop(d);
    };
    let (_, sent) = tokio::join!(receive, admin.serve_precode_admission(&s));
    assert!(sent.is_err());
    let authorization = admin.precode_redemptions(&code).await.unwrap()[0].authorization;
    let old = admin.management_history().await.unwrap();
    let mut session = admin.begin_management_session();
    admin
        .manage(
            &mut session,
            ManagementAction::RemoveMember(joining.status().node_id),
        )
        .await
        .unwrap();
    for record in admin.management_history().await.unwrap() {
        joining
            .ingest_management_record(record.header(), record.body())
            .await
            .unwrap();
    }
    let before = joining.status().applied_count;
    let old: Vec<_> = old.iter().map(|r| (r.header(), r.body())).collect();
    let stale = p2panda_core::cbor::encode_cbor(&(authorization, old)).unwrap();
    let (c, s) = connect(&je, &ae, ADMISSION_REQUEST_ALPN).await;
    let delayed_response = async {
        let (mut send, mut recv) = s.accept_bi().await.unwrap();
        let mut request = [0; 176];
        recv.read_exact(&mut request).await.unwrap();
        send.write_all(&[1]).await.unwrap();
        send.write_all(&0_u64.to_be_bytes()).await.unwrap();
        send.write_all(&(stale.len() as u32).to_be_bytes())
            .await
            .unwrap();
        send.write_all(&stale).await.unwrap();
        send.finish().unwrap();
        // Keep the real QUIC sender alive until the receiving API rejects it.
        s.closed().await;
    };
    let (received, _) = tokio::join!(
        joining.request_precode_admission(&c, &code),
        delayed_response
    );
    assert!(received.is_err());
    assert_eq!(joining.admission_stage(), AdmissionStage::AwaitingApproval);
    assert_eq!(joining.status().applied_count, before);
    assert_eq!(admin.precode_usage(&code).await.unwrap(), 1);
    eprintln!(
        "MIO35_REPLAY original_signed_response_replayed=true local_removal_still_effective=true no_secret_saved=true"
    );
    je.close().await;
    ae.close().await;
    joining.close().await;
    admin.close().await;
}

#[tokio::test]
async fn duplicate_save_ack_does_not_consume_twice_or_prove_recipient_persistence() {
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let code = admin.issue_precode(expiry(), Some(1)).await.unwrap();
    let mut joining = NativeNetwork::join(dir.path().join("joining"), code.invitation())
        .await
        .unwrap();
    let ae = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let je = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (c, s) = connect(&je, &ae, ADMISSION_REQUEST_ALPN).await;
    let receive = async {
        let ManualExchange::Materials(d) =
            joining.request_precode_admission(&c, &code).await.unwrap()
        else {
            panic!("grant")
        };
        c.close(0u32.into(), b"before secret");
        drop(d);
    };
    let (_, sent) = tokio::join!(receive, admin.serve_precode_admission(&s));
    assert!(sent.is_err());
    let approval = admin.precode_redemptions(&code).await.unwrap()[0].clone();
    let (c, s) = connect(&je, &ae, ADMISSION_REQUEST_ALPN).await;
    let duplicated_ack = async {
        let (mut send, mut recv) = c.open_bi().await.unwrap();
        let mut wire = code.invitation().encode().to_vec();
        wire.extend_from_slice(&approval.request.request_id);
        wire.extend_from_slice(joining.status().node_id.as_bytes());
        wire.extend_from_slice(&code.encode()[96..]);
        send.write_all(&wire).await.unwrap();
        let mut status = [0];
        recv.read_exact(&mut status).await.unwrap();
        assert_eq!(status, [1]);
        let mut generation = [0; 8];
        recv.read_exact(&mut generation).await.unwrap();
        assert_eq!(u64::from_be_bytes(generation), 0);
        let mut len = [0; 4];
        recv.read_exact(&mut len).await.unwrap();
        let mut history = vec![0; u32::from_be_bytes(len) as usize];
        recv.read_exact(&mut history).await.unwrap();
        send.write_all(b"AUTH").await.unwrap();
        let mut private_bytes = [0; 40];
        recv.read_exact(&mut private_bytes).await.unwrap();
        // Raw authenticated peer deliberately acknowledges twice without
        // calling the node's durable-save operation. Never log these bytes.
        send.write_all(b"SAVESAVE").await.unwrap();
        send.finish().unwrap();
        let mut done = [0; 4];
        recv.read_exact(&mut done).await.unwrap();
        assert_eq!(&done, b"DONE");
    };
    let (_, sent) = tokio::join!(duplicated_ack, admin.serve_precode_admission(&s));
    assert_eq!(sent.unwrap(), AdmissionStage::Complete);
    assert_eq!(admin.precode_usage(&code).await.unwrap(), 1);
    assert_eq!(
        admin.precode_redemptions(&code).await.unwrap()[0].stage,
        AdmissionStage::Complete
    );
    assert_eq!(joining.admission_stage(), AdmissionStage::PendingMaterials);
    je.close().await;
    ae.close().await;
    assert!(redeem(&mut admin, &mut joining, &code).await);
    assert_eq!(joining.admission_stage(), AdmissionStage::Complete);
    assert_eq!(admin.precode_usage(&code).await.unwrap(), 1);
    assert_eq!(admin.status().applied_count, 2);
    eprintln!(
        "MIO35_ACK duplicate_save=true sender_ack_complete=true recipient_pending_until_real_save=true retry_uses=1"
    );
    joining.close().await;
    admin.close().await;
}
