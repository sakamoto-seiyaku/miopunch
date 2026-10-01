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

#[tokio::test]
async fn single_use_redemption_survives_restart_and_delayed_delivery() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("admin");
    let mut admin = NativeNetwork::create(&a).await.unwrap();
    let code = admin.issue_precode(expiry(), Some(1)).await.unwrap();
    let joining_path = dir.path().join("joining");
    let mut joining = NativeNetwork::join(&joining_path, code.invitation())
        .await
        .unwrap();
    let ce = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let se = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    se.set_alpns(vec![ADMISSION_REQUEST_ALPN.to_vec()]);
    let (c, s) = connect(&ce, &se, ADMISSION_REQUEST_ALPN).await;
    let (received, sent) = tokio::join!(
        joining.request_precode_admission(&c, &code),
        admin.serve_precode_admission(&s)
    );
    assert!(matches!(received.unwrap(), ManualExchange::Materials(_)));
    assert!(sent.is_err()); // applicant drops delivery before secret save
    let original = admin
        .precode_redemptions(&code)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(original.stage, AdmissionStage::Approved);
    assert_eq!(admin.precode_usage(&code).await.unwrap(), 1);
    se.close().await;
    ce.close().await;
    joining.close().await;
    admin.close().await;
    let mut joining = NativeNetwork::open(&joining_path).await.unwrap();
    let ce = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let mut admin = NativeNetwork::open(&a).await.unwrap();
    let se = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    se.set_alpns(vec![ADMISSION_REQUEST_ALPN.to_vec()]);
    let (c, s) = connect(&ce, &se, ADMISSION_REQUEST_ALPN).await;
    let receive = async {
        let ManualExchange::Materials(d) =
            joining.request_precode_admission(&c, &code).await.unwrap()
        else {
            panic!("approved")
        };
        joining.complete_manual_admission(d).await.unwrap()
    };
    let (r, s) = tokio::join!(receive, admin.serve_precode_admission(&s));
    assert_eq!(r, AdmissionStage::Complete);
    assert_eq!(s.unwrap(), AdmissionStage::Complete);
    let retry = admin
        .precode_redemptions(&code)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(retry.authorization, original.authorization);
    assert_eq!(retry.request, original.request);
    assert_eq!(retry.stage, AdmissionStage::Complete);
    assert_eq!(admin.precode_usage(&code).await.unwrap(), 1);
    assert_eq!(admin.status().applied_count, 2);
    eprintln!(
        "MIO33_FIRST stable_request=true stable_authorization=true uses=1 applied=2 complete=true"
    );
    ce.close().await;
    se.close().await;
    joining.close().await;
    admin.close().await;
}

#[tokio::test]
async fn independent_tls_identities_competing_for_last_use_admit_only_one() {
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let code = admin.issue_precode(expiry(), Some(1)).await.unwrap();
    let mut first = NativeNetwork::join(dir.path().join("first"), code.invitation())
        .await
        .unwrap();
    let mut second = NativeNetwork::join(dir.path().join("second"), code.invitation())
        .await
        .unwrap();
    assert_ne!(first.status().node_id, second.status().node_id);
    let ae = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let fe = first.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let se = second.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    ae.set_alpns(vec![ADMISSION_REQUEST_ALPN.to_vec()]);
    // Both independently authenticated connections exist before either starts.
    let (fc, fs) = connect(&fe, &ae, ADMISSION_REQUEST_ALPN).await;
    let (sc, ss) = connect(&se, &ae, ADMISSION_REQUEST_ALPN).await;
    let admin = tokio::sync::Mutex::new(admin);
    let first_request = async {
        match first.request_precode_admission(&fc, &code).await {
            Ok(ManualExchange::Materials(d)) => first.complete_manual_admission(d).await.is_ok(),
            _ => false,
        }
    };
    let second_request = async {
        match second.request_precode_admission(&sc, &code).await {
            Ok(ManualExchange::Materials(d)) => second.complete_manual_admission(d).await.is_ok(),
            _ => false,
        }
    };
    let (f, s, sf, ss) = tokio::join!(
        first_request,
        second_request,
        async { admin.lock().await.serve_precode_admission(&fs).await },
        async { admin.lock().await.serve_precode_admission(&ss).await }
    );
    assert_eq!(usize::from(f) + usize::from(s), 1);
    assert_eq!(usize::from(sf.is_ok()) + usize::from(ss.is_ok()), 1);
    let mut admin = admin.into_inner();
    assert_eq!(admin.precode_usage(&code).await.unwrap(), 1);
    assert_eq!(admin.precode_redemptions(&code).await.unwrap().len(), 1);
    assert_eq!(admin.status().applied_count, 2);
    assert_eq!(
        usize::from(first.admission_stage() == AdmissionStage::Complete)
            + usize::from(second.admission_stage() == AdmissionStage::Complete),
        1
    );
    eprintln!("MIO33_PRIORITY independent_tls_identities=2 successes=1 uses=1 applied=2");
    fe.close().await;
    se.close().await;
    ae.close().await;
    first.close().await;
    second.close().await;
    admin.close().await;
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

#[tokio::test]
async fn time_only_codes_and_single_code_revocation_preserve_existing_membership() {
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let code = admin.issue_precode(expiry(), None).await.unwrap();
    let mut first = NativeNetwork::join(dir.path().join("first"), code.invitation())
        .await
        .unwrap();
    assert!(redeem(&mut admin, &mut first, &code).await);
    let authorization = admin.precode_redemptions(&code).await.unwrap()[0].authorization;
    let other = NativeNetwork::create(dir.path().join("other"))
        .await
        .unwrap();
    assert!(other.revoke_precode(&code).await.is_err());
    admin.revoke_precode(&code).await.unwrap();
    let mut second = NativeNetwork::join(dir.path().join("second"), code.invitation())
        .await
        .unwrap();
    assert!(!redeem(&mut admin, &mut second, &code).await);
    assert!(
        admin
            .status()
            .members
            .iter()
            .any(|(id, _)| *id == first.status().node_id)
    );
    assert!(redeem(&mut admin, &mut first, &code).await);
    assert_eq!(admin.precode_usage(&code).await.unwrap(), 1);
    assert_eq!(
        admin.precode_redemptions(&code).await.unwrap()[0].authorization,
        authorization
    );
    eprintln!(
        "MIO33_REVOKE time_only=true designated_only=true new_rejected=true prior_valid=true retry_uses=1"
    );
    first.close().await;
    second.close().await;
    other.close().await;
    admin.close().await;
}

#[tokio::test]
async fn fresh_code_consumes_once_for_existing_member_without_adding_twice() {
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let first_code = admin.issue_precode(expiry(), Some(1)).await.unwrap();
    let mut joining = NativeNetwork::join(dir.path().join("joining"), first_code.invitation())
        .await
        .unwrap();
    assert!(redeem(&mut admin, &mut joining, &first_code).await);
    let original = admin.precode_redemptions(&first_code).await.unwrap()[0].clone();
    let fresh = admin.issue_precode(expiry(), Some(1)).await.unwrap();
    assert!(redeem(&mut admin, &mut joining, &fresh).await);
    let new = admin.precode_redemptions(&fresh).await.unwrap()[0].clone();
    assert_eq!(new.authorization, original.authorization);
    assert_ne!(
        new.request.request_id, original.request.request_id,
        "redemption identity is scoped to the code"
    );
    assert_eq!(admin.precode_usage(&first_code).await.unwrap(), 1);
    assert_eq!(admin.precode_usage(&fresh).await.unwrap(), 1);
    assert_eq!(admin.status().applied_count, 2);
    assert!(redeem(&mut admin, &mut joining, &fresh).await);
    assert_eq!(admin.precode_usage(&fresh).await.unwrap(), 1);
    eprintln!(
        "MIO33_FRESH existing_grant_reused=true each_code_uses=1 distinct_redemption=true applied=2"
    );
    joining.close().await;
    admin.close().await;
}

#[tokio::test]
async fn administrator_regrant_never_revives_codes_from_the_previous_authority_origin() {
    use miopunch_native_probe::daily_management::ManagementAction;
    use miopunch_native_probe::genesis_recovery::GenesisCredential;
    use p2panda_auth::Access;
    use p2panda_core::Hash;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("admin");
    let mut admin = NativeNetwork::create(&path).await.unwrap();
    let old = admin.issue_precode(expiry(), None).await.unwrap();
    let mut admitted = NativeNetwork::join(dir.path().join("admitted"), old.invitation())
        .await
        .unwrap();
    assert!(redeem(&mut admin, &mut admitted, &old).await);
    let saved = std::fs::read(path.join("genesis.credential")).unwrap();
    let credential = GenesisCredential::from_saved(
        saved[..32].try_into().unwrap(),
        Hash::from_bytes(saved[32..].try_into().unwrap()),
    );
    let mut session = admin.begin_management_session();
    admin
        .elevate_genesis(&mut session, credential, Duration::from_secs(60))
        .unwrap();
    let me = admin.status().node_id;
    admin
        .manage(&mut session, ManagementAction::DemoteAdministrator(me))
        .await
        .unwrap();
    assert!(admin.issue_precode(expiry(), None).await.is_err());
    assert!(
        admin
            .status()
            .members
            .contains(&(admitted.status().node_id, Access::read()))
    );
    admin
        .manage(&mut session, ManagementAction::AppointAdministrator(me))
        .await
        .unwrap();
    assert!(admin.status().members.contains(&(me, Access::manage())));
    admin.close().await;
    let mut admin = NativeNetwork::open(&path).await.unwrap();
    let mut new = NativeNetwork::join(dir.path().join("new"), old.invitation())
        .await
        .unwrap();
    assert!(
        !redeem(&mut admin, &mut new, &old).await,
        "current is_manager must not revive the old code"
    );
    assert_eq!(admin.precode_usage(&old).await.unwrap(), 1);
    let fresh = admin.issue_precode(expiry(), Some(1)).await.unwrap();
    assert!(redeem(&mut admin, &mut new, &fresh).await);
    assert_eq!(admin.precode_usage(&fresh).await.unwrap(), 1);
    assert!(
        admin
            .status()
            .members
            .contains(&(admitted.status().node_id, Access::read()))
    );
    eprintln!(
        "MIO33_ORIGIN root_signed_loss_regrant=true reopened=true old_new_redemption_rejected=true fresh_accepted=true prior_member_valid=true"
    );
    admitted.close().await;
    new.close().await;
    admin.close().await;
}

#[tokio::test]
async fn supported_restore_of_coherent_pre_consumption_backup_stops_old_codes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("admin");
    let mut admin = NativeNetwork::create(&path).await.unwrap();
    let old = admin.issue_precode(expiry(), Some(1)).await.unwrap();
    let identity = admin.status().node_id;
    let anchor = admin.status().anchor;
    let backup = dir.path().join("before-consumption-backup");
    admin.create_backup(&backup).await.unwrap(); // consumes/closes original node
    let mut admin = NativeNetwork::open(&path).await.unwrap();
    let mut first = NativeNetwork::join(dir.path().join("first"), old.invitation())
        .await
        .unwrap();
    assert!(redeem(&mut admin, &mut first, &old).await);
    assert_eq!(admin.precode_usage(&old).await.unwrap(), 1);
    admin.close().await;
    let mut restored = NativeNetwork::restore_backup(&backup, dir.path().join("restored"))
        .await
        .unwrap();
    assert_eq!(restored.status().node_id, identity);
    assert_eq!(restored.status().anchor, anchor);
    assert_eq!(
        restored.status().applied_count,
        1,
        "backup really predates consumption"
    );
    assert_eq!(
        restored.precode_usage(&old).await.unwrap(),
        0,
        "coherent old counter was actually restored"
    );
    let mut second = NativeNetwork::join(dir.path().join("second"), old.invitation())
        .await
        .unwrap();
    assert!(
        !redeem(&mut restored, &mut second, &old).await,
        "supported restore cannot trust the rolled-back counter"
    );
    let fresh = restored.issue_precode(expiry(), Some(1)).await.unwrap();
    assert!(redeem(&mut restored, &mut second, &fresh).await);
    assert_eq!(restored.precode_usage(&old).await.unwrap(), 0);
    assert_eq!(restored.precode_usage(&fresh).await.unwrap(), 1);
    eprintln!(
        "MIO33_RESTORE coherent_before_consumption=true old_saved_counter=0 old_new_redemption_stopped=true fresh_uses=1 stable_identity_anchor=true"
    );
    restored.close().await;
    let mut restored = NativeNetwork::open(dir.path().join("restored"))
        .await
        .unwrap();
    let mut third = NativeNetwork::join(dir.path().join("third"), old.invitation())
        .await
        .unwrap();
    assert!(
        !redeem(&mut restored, &mut third, &old).await,
        "restored code stop survives normal restart"
    );
    assert!(redeem(&mut restored, &mut second, &fresh).await);
    assert_eq!(restored.precode_usage(&fresh).await.unwrap(), 1);
    first.close().await;
    second.close().await;
    third.close().await;
    restored.close().await;
}

#[tokio::test]
async fn expiry_stops_new_redemption_but_not_prior_materials_or_membership() {
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let expires = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 2;
    let code = admin.issue_precode(expires, None).await.unwrap();
    let mut first = NativeNetwork::join(dir.path().join("first"), code.invitation())
        .await
        .unwrap();
    assert!(redeem(&mut admin, &mut first, &code).await);
    let original = admin.precode_redemptions(&code).await.unwrap()[0].authorization;
    let until = UNIX_EPOCH + Duration::from_secs(expires);
    if let Ok(wait) = until.duration_since(SystemTime::now()) {
        tokio::time::sleep(wait).await;
    }
    let mut second = NativeNetwork::join(dir.path().join("second"), code.invitation())
        .await
        .unwrap();
    assert!(!redeem(&mut admin, &mut second, &code).await);
    assert!(redeem(&mut admin, &mut first, &code).await);
    assert_eq!(admin.precode_usage(&code).await.unwrap(), 1);
    assert_eq!(
        admin.precode_redemptions(&code).await.unwrap()[0].authorization,
        original
    );
    assert_eq!(first.admission_stage(), AdmissionStage::Complete);
    eprintln!(
        "MIO33_EXPIRY actual_wall_deadline=true new_rejected=true prior_materials=true uses=1"
    );
    first.close().await;
    second.close().await;
    admin.close().await;
}

#[tokio::test]
async fn damaged_counter_stops_old_code_and_allows_fresh_issuance() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("admin");
    let mut admin = NativeNetwork::create(&path).await.unwrap();
    let old = admin.issue_precode(expiry(), Some(2)).await.unwrap();
    let mut first = NativeNetwork::join(dir.path().join("first"), old.invitation())
        .await
        .unwrap();
    assert!(redeem(&mut admin, &mut first, &old).await);
    admin.close().await;
    // A real storage fault changes the persisted counter, not a trust flag.
    // Assertions below use public-node results, never database observations.
    let options =
        sqlx::sqlite::SqliteConnectOptions::new().filename(path.join("management.sqlite"));
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::query("UPDATE precodes SET used=0")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let mut admin = NativeNetwork::open(&path).await.unwrap();
    let mut second = NativeNetwork::join(dir.path().join("second"), old.invitation())
        .await
        .unwrap();
    assert!(
        !redeem(&mut admin, &mut second, &old).await,
        "inconsistent consumption evidence must stop new redemption"
    );
    assert_eq!(first.admission_stage(), AdmissionStage::Complete);
    let fresh = admin.issue_precode(expiry(), Some(1)).await.unwrap();
    assert!(redeem(&mut admin, &mut second, &fresh).await);
    assert_eq!(admin.precode_usage(&fresh).await.unwrap(), 1);
    eprintln!(
        "MIO33_DAMAGE real_counter_corruption=true old_stopped=true prior_valid=true fresh_uses=1"
    );
    first.close().await;
    second.close().await;
    admin.close().await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn precode_commit_crash_child() {
    let Ok(path) = std::env::var("MIO33_CRASH_NODE") else {
        return;
    };
    let mut node = NativeNetwork::open(path).await.unwrap();
    node.set_replica_checkpoint_observer(|checkpoint| {
        use miopunch_native_probe::management_store::RecordCheckpoint;
        let expected = std::env::var("MIO33_CRASH_STAGE").unwrap();
        if (expected == "before" && checkpoint == RecordCheckpoint::BeforePrecodeCommit)
            || (expected == "after" && checkpoint == RecordCheckpoint::AfterPrecodeCommit)
        {
            eprintln!("MIO33_CRASH actual_process_exit_at={checkpoint:?}");
            std::process::exit(if expected == "before" { 91 } else { 92 });
        }
    });
    let endpoint = node.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    endpoint.set_alpns(vec![ADMISSION_REQUEST_ALPN.to_vec()]);
    let address = *endpoint.addr().ip_addrs().next().unwrap();
    std::fs::write(
        std::env::var("MIO33_CRASH_ADDRESS").unwrap(),
        address.to_string(),
    )
    .unwrap();
    let connection = endpoint.accept().await.unwrap().await.unwrap();
    node.serve_precode_admission(&connection).await.unwrap();
    panic!("checkpoint did not terminate the real server");
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn quota_signed_grant_and_pending_result_recover_at_both_commit_boundaries() {
    for stage in ["before", "after"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("admin");
        let mut admin = NativeNetwork::create(&path).await.unwrap();
        let code = admin.issue_precode(expiry(), Some(1)).await.unwrap();
        let mut joining = NativeNetwork::join(dir.path().join("joining"), code.invitation())
            .await
            .unwrap();
        admin.close().await;
        let address_file = dir.path().join("address");
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "precode_commit_crash_child", "--nocapture"])
            .env("MIO33_CRASH_NODE", &path)
            .env("MIO33_CRASH_STAGE", stage)
            .env("MIO33_CRASH_ADDRESS", &address_file)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let address = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(text) = std::fs::read_to_string(&address_file) {
                    break text.parse::<std::net::SocketAddr>().unwrap();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let endpoint = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
        let target = EndpointAddr::new(code.invitation().contact).with_ip_addr(address);
        let connection = endpoint
            .connect(target, ADMISSION_REQUEST_ALPN)
            .await
            .unwrap();
        assert!(
            joining
                .request_precode_admission(&connection, &code)
                .await
                .is_err()
        );
        let output = child.wait_with_output().unwrap();
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(
            output.status.code(),
            Some(if stage == "before" { 91 } else { 92 })
        );
        endpoint.close().await;
        let mut admin = NativeNetwork::open(&path).await.unwrap();
        assert_eq!(
            admin.precode_usage(&code).await.unwrap(),
            if stage == "before" { 0 } else { 1 }
        );
        let saved = admin.precode_redemptions(&code).await.unwrap();
        assert_eq!(saved.len(), if stage == "before" { 0 } else { 1 });
        assert_eq!(
            admin.status().applied_count,
            if stage == "before" { 1 } else { 2 }
        );
        assert!(redeem(&mut admin, &mut joining, &code).await);
        let retried = admin
            .precode_redemptions(&code)
            .await
            .unwrap()
            .pop()
            .unwrap();
        if stage == "after" {
            assert_eq!(retried.authorization, saved[0].authorization);
            assert_eq!(retried.request, saved[0].request);
        }
        assert_eq!(admin.precode_usage(&code).await.unwrap(), 1);
        assert_eq!(admin.status().applied_count, 2);
        eprintln!(
            "MIO33_COMMIT stage={stage} real_process_exit=true recoverable_atomic_tuple=true retry_uses=1 applied=2"
        );
        joining.close().await;
        admin.close().await;
    }
}

#[tokio::test]
async fn code_is_only_redeemable_and_revocable_at_its_designated_same_network_administrator() {
    use miopunch_native_probe::daily_management::ManagementAction;
    use miopunch_native_probe::genesis_recovery::GenesisCredential;
    use miopunch_native_probe::precode::Precode;
    use p2panda_core::Hash;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("admin");
    let mut admin = NativeNetwork::create(&path).await.unwrap();
    let seed = admin.issue_precode(expiry(), Some(1)).await.unwrap();
    let mut alternate = NativeNetwork::join(dir.path().join("alternate"), seed.invitation())
        .await
        .unwrap();
    assert!(redeem(&mut admin, &mut alternate, &seed).await);
    let saved = std::fs::read(path.join("genesis.credential")).unwrap();
    let credential = GenesisCredential::from_saved(
        saved[..32].try_into().unwrap(),
        Hash::from_bytes(saved[32..].try_into().unwrap()),
    );
    let mut session = admin.begin_management_session();
    admin
        .elevate_genesis(&mut session, credential, Duration::from_secs(60))
        .unwrap();
    admin
        .manage(
            &mut session,
            ManagementAction::AppointAdministrator(alternate.status().node_id),
        )
        .await
        .unwrap();
    for record in admin.management_history().await.unwrap() {
        alternate
            .ingest_management_record(record.header(), record.body())
            .await
            .unwrap();
    }
    let old = admin.issue_precode(expiry(), Some(1)).await.unwrap();
    let encoded = old.encode();
    let decoded = Precode::decode(&encoded).unwrap();
    assert!(decoded == old);
    assert!(alternate.revoke_precode(&old).await.is_err());
    // Trusted invitation replacement is outside scope, but even deliberately
    // retargeting the capability cannot make another administrator consume it.
    let mut retargeted = encoded;
    retargeted[..96].copy_from_slice(&alternate.manual_invitation().unwrap().encode());
    let forged = Precode::decode(&retargeted).unwrap();
    let mut holder = NativeNetwork::join(dir.path().join("holder"), forged.invitation())
        .await
        .unwrap();
    assert!(!redeem(&mut alternate, &mut holder, &forged).await);
    assert_eq!(admin.precode_usage(&old).await.unwrap(), 0);
    let mut legitimate = NativeNetwork::join(dir.path().join("legitimate"), old.invitation())
        .await
        .unwrap();
    assert!(redeem(&mut admin, &mut legitimate, &old).await);
    assert_eq!(admin.precode_usage(&old).await.unwrap(), 1);
    admin.close().await; // designated node offline; alternate stays available
    let mut third = NativeNetwork::join(dir.path().join("third"), forged.invitation())
        .await
        .unwrap();
    assert!(
        !redeem(&mut alternate, &mut third, &forged).await,
        "offline never transfers the old code"
    );
    let independent = alternate.issue_precode(expiry(), Some(1)).await.unwrap();
    assert!(redeem(&mut alternate, &mut third, &independent).await);
    eprintln!(
        "MIO33_DESIGNATED same_network_admins=2 other_revoke=false retargeted_redemption=false offline_takeover=false fresh_other_code=true"
    );
    holder.close().await;
    legitimate.close().await;
    third.close().await;
    alternate.close().await;
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

#[tokio::test]
async fn late_signed_loss_and_regrant_history_invalidates_the_old_code_origin() {
    use miopunch_native_probe::daily_management::ManagementAction;
    use miopunch_native_probe::genesis_recovery::GenesisCredential;
    use p2panda_auth::Access;
    use p2panda_core::Hash;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("root-device");
    let mut root = NativeNetwork::create(&path).await.unwrap();
    let seed = root.issue_precode(expiry(), Some(1)).await.unwrap();
    let mut manager = NativeNetwork::join(dir.path().join("manager"), seed.invitation())
        .await
        .unwrap();
    assert!(redeem(&mut root, &mut manager, &seed).await);
    let bytes = std::fs::read(path.join("genesis.credential")).unwrap();
    let credential = GenesisCredential::from_saved(
        bytes[..32].try_into().unwrap(),
        Hash::from_bytes(bytes[32..].try_into().unwrap()),
    );
    let mut session = root.begin_management_session();
    root.elevate_genesis(&mut session, credential, Duration::from_secs(60))
        .unwrap();
    let id = manager.status().node_id;
    root.manage(&mut session, ManagementAction::AppointAdministrator(id))
        .await
        .unwrap();
    replicate(&root, &mut manager).await;
    let old = manager.issue_precode(expiry(), Some(1)).await.unwrap();
    root.manage(&mut session, ManagementAction::DemoteAdministrator(id))
        .await
        .unwrap();
    root.manage(&mut session, ManagementAction::AppointAdministrator(id))
        .await
        .unwrap();
    assert!(
        manager.status().members.contains(&(id, Access::manage())),
        "receiver has not seen the loss yet"
    );
    replicate(&root, &mut manager).await;
    assert!(
        manager.status().members.contains(&(id, Access::manage())),
        "final view is still Manage; current bool is insufficient"
    );
    let mut holder = NativeNetwork::join(dir.path().join("holder"), old.invitation())
        .await
        .unwrap();
    assert!(!redeem(&mut manager, &mut holder, &old).await);
    let fresh = manager.issue_precode(expiry(), Some(1)).await.unwrap();
    assert!(redeem(&mut manager, &mut holder, &fresh).await);
    eprintln!(
        "MIO33_LATE_ORIGIN actual_logsync=true manage_before_and_after=true old_origin_rejected=true fresh_accepted=true"
    );
    holder.close().await;
    manager.close().await;
    root.close().await;
}

#[tokio::test]
async fn holder_of_the_same_code_cannot_redeem_as_a_different_tls_identity() {
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let code = admin.issue_precode(expiry(), Some(1)).await.unwrap();
    let mut rightful = NativeNetwork::join(dir.path().join("rightful"), code.invitation())
        .await
        .unwrap();
    let attacker = NativeNetwork::join(dir.path().join("attacker"), code.invitation())
        .await
        .unwrap();
    let ae = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let te = attacker.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    ae.set_alpns(vec![ADMISSION_REQUEST_ALPN.to_vec()]);
    let (c, s) = connect(&te, &ae, ADMISSION_REQUEST_ALPN).await;
    assert_ne!(
        c.remote_id().as_bytes(),
        attacker.status().node_id.as_bytes()
    );
    let forged = async {
        let (mut send, mut recv) = c.open_bi().await.unwrap();
        let mut wire = code.invitation().encode().to_vec();
        wire.extend_from_slice(&[7; 16]);
        wire.extend_from_slice(rightful.status().node_id.as_bytes());
        wire.extend_from_slice(&code.encode()[96..]);
        send.write_all(&wire).await.unwrap();
        send.finish().unwrap();
        recv.read_to_end(1024).await
    };
    let (received, sent) = tokio::join!(forged, admin.serve_precode_admission(&s));
    assert!(sent.is_err());
    assert!(received.is_err());
    assert_eq!(admin.precode_usage(&code).await.unwrap(), 0);
    assert!(admin.precode_redemptions(&code).await.unwrap().is_empty());
    assert_eq!(admin.status().applied_count, 1);
    te.close().await;
    ae.close().await;
    assert!(redeem(&mut admin, &mut rightful, &code).await);
    assert_eq!(admin.precode_usage(&code).await.unwrap(), 1);
    eprintln!(
        "MIO33_IDENTITY actual_forged_claim=true tls_mismatch_rejected=true forged_uses=0 rightful_uses=1"
    );
    rightful.close().await;
    attacker.close().await;
    admin.close().await;
}

#[tokio::test]
async fn old_redemption_cannot_restore_removed_membership_or_replace_a_fresh_grant() {
    use miopunch_native_probe::daily_management::ManagementAction;
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let old = admin.issue_precode(expiry(), Some(1)).await.unwrap();
    let mut joining = NativeNetwork::join(dir.path().join("joining"), old.invitation())
        .await
        .unwrap();
    assert!(redeem(&mut admin, &mut joining, &old).await);
    let old_grant = admin.precode_redemptions(&old).await.unwrap()[0].authorization;
    let mut session = admin.begin_management_session();
    admin
        .manage(
            &mut session,
            ManagementAction::RemoveMember(joining.status().node_id),
        )
        .await
        .unwrap();
    // MIOPUNCH-34 permits only a fresh restricted request connection, never
    // old redemption material or member access after effective removal.
    assert!(!redeem(&mut admin, &mut joining, &old).await);
    assert_eq!(admin.precode_usage(&old).await.unwrap(), 1);
    // A separately valid signed grant restores membership. New-code material
    // completion reuses that grant; old redemption must still remain invalid.
    let regrant = admin
        .manage(
            &mut session,
            ManagementAction::AddMember(joining.status().node_id),
        )
        .await
        .unwrap();
    let fresh = admin.issue_precode(expiry(), Some(1)).await.unwrap();
    assert!(redeem(&mut admin, &mut joining, &fresh).await);
    let fresh_grant = admin.precode_redemptions(&fresh).await.unwrap()[0].authorization;
    assert_ne!(old_grant, fresh_grant);
    assert_eq!(fresh_grant, regrant.record);
    assert!(!redeem(&mut admin, &mut joining, &old).await);
    assert_eq!(admin.precode_usage(&old).await.unwrap(), 1);
    assert_eq!(admin.precode_usage(&fresh).await.unwrap(), 1);
    eprintln!(
        "MIO33_REJOIN old_redemption_rejected_after_remove=true old_result_invalid_after_separate_valid_regrant=true fresh_reuses_grant=true each_code_uses=1"
    );
    joining.close().await;
    admin.close().await;
}
