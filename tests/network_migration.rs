use iroh::EndpointAddr;
use miopunch_native_probe::{
    admission_gate::ProtocolScope,
    genesis_recovery::GenesisCredential,
    native_network::NativeNetwork,
    network_migration::{MigrationPhase, NetworkMigration},
    offline_replica::REPLICA_ALPN,
};
use p2panda_core::Hash;
use std::{
    net::Ipv4Addr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn expiry() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600
}
fn address(e: &iroh::endpoint::RestrictedEndpoint) -> EndpointAddr {
    EndpointAddr::new(e.id()).with_ip_addr(*e.addr().ip_addrs().next().unwrap())
}
fn credential(p: &std::path::Path) -> GenesisCredential {
    let b = std::fs::read(p.join("genesis.credential")).unwrap();
    GenesisCredential::from_saved(
        b[..32].try_into().unwrap(),
        Hash::from_bytes(b[32..].try_into().unwrap()),
    )
}
async fn server(node: &NativeNetwork) -> iroh::endpoint::RestrictedEndpoint {
    node.admission_gate()
        .register_protocol(REPLICA_ALPN, ProtocolScope::Member);
    let e = node.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    e.set_alpns(vec![
        REPLICA_ALPN.to_vec(),
        miopunch_native_probe::admission_gate::ADMISSION_REQUEST_ALPN.to_vec(),
    ]);
    e
}

#[tokio::test]
async fn source_works_until_saved_target_member_data_then_switch_and_exit() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("device");
    let mut source = NativeNetwork::create(&p).await.unwrap();
    let mut config = source.resource_configuration().unwrap();
    config.replacement = Some(vec![]);
    source.save_resource_configuration(&config).unwrap();
    let mut peer = join(&mut source, &dir.path().join("source-peer")).await;
    let old = source.status();
    let mut target = NativeNetwork::create(dir.path().join("target-admin"))
        .await
        .unwrap();
    let code = target.issue_precode(expiry(), Some(1)).await.unwrap();
    let te = server(&target).await;
    let mut migration = NetworkMigration::prepare(source, code.invitation())
        .await
        .unwrap();
    assert_eq!(migration.phase(), MigrationPhase::Preparing);
    assert_eq!(migration.active().status(), old);
    assert_eq!(migration.target_status().node_id, old.node_id);
    let (r, s) = tokio::join!(migration.redeem_target(address(&te), &code), async {
        let c = std::sync::Arc::new(
            te.accept()
                .await
                .unwrap()
                .accept_control()
                .unwrap()
                .await
                .unwrap(),
        );
        let result = target.serve_precode_admission(&c).await;
        c.closed().await;
        result
    });
    r.unwrap();
    s.unwrap();
    assert_eq!(migration.phase(), MigrationPhase::MaterialsSaved);
    assert_eq!(migration.active().status(), old);
    replicate(migration.active(), &mut peer).await;
    let (r, s) = tokio::join!(migration.validate_target(address(&te)), async {
        let c = te.accept().await.unwrap().await.unwrap();
        let result = target.serve_replica(&c).await;
        c.closed().await;
        result
    });
    assert!(r.unwrap().complete());
    s.unwrap();
    assert_eq!(migration.phase(), MigrationPhase::Validated);
    let source_endpoint = server(migration.active()).await;
    let peer_endpoint = server(&peer).await;
    let (old_client, old_server) = tokio::join!(
        peer_endpoint.connect(address(&source_endpoint), REPLICA_ALPN),
        async { source_endpoint.accept().await.unwrap().await }
    );
    let old_client = old_client.unwrap();
    let old_server = old_server.unwrap();
    let mut session = migration.begin_source_management_session();
    migration
        .elevate_source(&mut session, credential(&p), Duration::from_secs(60))
        .unwrap();
    migration.switch(&mut session).await.unwrap();
    assert_eq!(migration.phase(), MigrationPhase::Switched);
    tokio::time::timeout(Duration::from_secs(3), old_client.closed())
        .await
        .unwrap();
    assert!(
        old_server.close_reason().is_some(),
        "source sessions end at switch"
    );
    source_endpoint.close().await;
    peer_endpoint.close().await;
    assert_eq!(migration.active().status().anchor, target.status().anchor);
    migration.finish_source_exit(&mut session).await.unwrap();
    assert_eq!(migration.phase(), MigrationPhase::SourceExited);
    assert_eq!(migration.active().status().node_id, old.node_id);
    assert_eq!(target.precode_usage(&code).await.unwrap(), 1);
    migration.close().await;
    let migration = NetworkMigration::open(&p).await.unwrap();
    assert_eq!(migration.phase(), MigrationPhase::SourceExited);
    assert_eq!(migration.active().status().anchor, target.status().anchor);
    assert!(NativeNetwork::open(&p).await.is_err());
    migration.close().await;
    te.close().await;
    target.close().await;
    peer.close().await;
}

async fn join(admin: &mut NativeNetwork, path: &std::path::Path) -> NativeNetwork {
    let code = admin.issue_precode(expiry(), Some(1)).await.unwrap();
    let mut node = NativeNetwork::join(path, code.invitation()).await.unwrap();
    let ae = server(admin).await;
    let ne = node.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (c, s) = tokio::join!(
        ne.connect_control(
            address(&ae),
            miopunch_native_probe::admission_gate::ADMISSION_REQUEST_ALPN
        ),
        async { ae.accept().await.unwrap().accept_control().unwrap().await }
    );
    let c = std::sync::Arc::new(c.unwrap());
    let s = std::sync::Arc::new(s.unwrap());
    let (r, t) = tokio::join!(
        async {
            let miopunch_native_probe::manual_admission::ManualExchange::Materials(d) =
                node.request_precode_admission(&c, &code).await.unwrap()
            else {
                panic!("materials")
            };
            node.complete_manual_admission(d).await
        },
        admin.serve_precode_admission(&s)
    );
    r.unwrap();
    t.unwrap();
    ne.close().await;
    ae.close().await;
    node
}
async fn replicate(source: &NativeNetwork, receiver: &mut NativeNetwork) {
    let se = server(source).await;
    let re = server(receiver).await;
    let (c, s) = tokio::join!(re.connect(address(&se), REPLICA_ALPN), async {
        se.accept().await.unwrap().await
    });
    let c = c.unwrap();
    let s = s.unwrap();
    let (r, t) = tokio::join!(receiver.sync_replica(&c), source.serve_replica(&s));
    assert!(r.unwrap().complete());
    t.unwrap();
    re.close().await;
    se.close().await;
}

#[tokio::test]
#[ignore = "requires owned MIOPUNCH_MQTT_BROKER; run scripts/check_network_migration.py"]
async fn signaling_only_target_cannot_switch_and_formal_exit_cannot_roll_back() {
    use miopunch_native_probe::resources::{Resource, ResourceUse};
    use std::collections::BTreeSet;
    let broker = std::env::var("MIOPUNCH_MQTT_BROKER").expect("real broker required");
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("source");
    let source = NativeNetwork::create(&p).await.unwrap();
    let old = source.status();
    let mut config = source.resource_configuration().unwrap();
    config.replacement = Some(vec![]);
    source.save_resource_configuration(&config).unwrap();
    let mut target = NativeNetwork::create(dir.path().join("target"))
        .await
        .unwrap();
    let code = target.issue_precode(expiry(), Some(1)).await.unwrap();
    let te = server(&target).await;
    let mut migration = NetworkMigration::prepare(source, code.invitation())
        .await
        .unwrap();
    let (r, s) = tokio::join!(migration.redeem_target(address(&te), &code), async {
        let c = std::sync::Arc::new(
            te.accept()
                .await
                .unwrap()
                .accept_control()
                .unwrap()
                .await
                .unwrap(),
        );
        let result = target.serve_precode_admission(&c).await;
        c.closed().await;
        result
    });
    r.unwrap();
    s.unwrap();
    te.close().await;
    config.replacement = Some(vec![Resource {
        url: format!("mqtt://{broker}/"),
        uses: BTreeSet::from([ResourceUse::Signaling]),
        limitation: "owned migration signaling-only fault".into(),
    }]);
    config.udp_direct_disabled = true;
    config.tcp_direct_disabled = true;
    config.member_data_relay_disabled = true;
    migration.configure_target_resources(&config).unwrap();
    target.save_resource_configuration(&config).unwrap();
    let te = target
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let route = EndpointAddr::new(te.endpoint().id()).with_addrs(
        te.endpoint()
            .custom_addrs()
            .into_iter()
            .filter(|a| a.id() == miopunch_native_probe::mqtt_transport::TRANSPORT_ID)
            .map(iroh::TransportAddr::Custom),
    );
    assert!(!route.addrs.is_empty(), "real signaling address");
    // Successful TLS invitation/result retry over MQTT proves the signal path
    // exists. It still cannot substitute for member DATA validation.
    let (r, s) = tokio::join!(migration.redeem_target(route.clone(), &code), async {
        let c = std::sync::Arc::new(
            te.endpoint()
                .accept()
                .await
                .unwrap()
                .accept_control()
                .unwrap()
                .await
                .unwrap(),
        );
        let result = target.serve_precode_admission(&c).await;
        c.closed().await;
        result
    });
    r.unwrap();
    s.unwrap();
    assert_eq!(target.precode_usage(&code).await.unwrap(), 1);
    let incoming = async {
        if let Some(incoming) = te.endpoint().accept().await {
            let _ = incoming.await;
        }
    };
    let (r, _) = tokio::join!(
        migration.validate_target(route),
        tokio::time::timeout(Duration::from_secs(18), incoming)
    );
    assert!(r.is_err());
    let mut session = migration.begin_source_management_session();
    migration
        .elevate_source(&mut session, credential(&p), Duration::from_secs(60))
        .unwrap();
    assert!(migration.switch(&mut session).await.is_err());
    assert_eq!(migration.active().status(), old);
    assert_eq!(migration.phase(), MigrationPhase::MaterialsSaved);
    te.close().await;
    config.replacement = Some(vec![]);
    config.udp_direct_disabled = false;
    migration.configure_target_resources(&config).unwrap();
    target.save_resource_configuration(&config).unwrap();
    let te = server(&target).await;
    let (r, s) = tokio::join!(migration.validate_target(address(&te)), async {
        let c = te.accept().await.unwrap().await.unwrap();
        let result = target.serve_replica(&c).await;
        c.closed().await;
        result
    });
    r.unwrap();
    s.unwrap();
    migration.switch(&mut session).await.unwrap();
    migration.finish_source_exit(&mut session).await.unwrap();
    migration.close().await;
    // Roll back only progress, retaining the signed durable departure. Never
    // restore membership from the old invitation or old active-state choice.
    std::fs::remove_file(p.join("migration.switched")).unwrap();
    std::fs::remove_file(p.join("migration.exited")).unwrap();
    assert!(NetworkMigration::open(&p).await.is_err());
    assert!(NativeNetwork::open(&p).await.is_err());
    te.close().await;
    target.close().await;
}

#[tokio::test]
async fn same_key_histories_and_permissions_remain_independent() {
    use miopunch_native_probe::daily_management::ManagementAction;
    use miopunch_native_probe::management_auth::ManagementExtension;
    use p2panda_auth::Access;
    use p2panda_core::{Header, SigningKey};
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("source");
    let tp = dir.path().join("target");
    let mut source = NativeNetwork::create(&p).await.unwrap();
    let id = source.status().node_id;
    let mut config = source.resource_configuration().unwrap();
    config.replacement = Some(vec![]);
    source.save_resource_configuration(&config).unwrap();
    let mut ss = source.begin_management_session();
    let source_record = source
        .manage(
            &mut ss,
            ManagementAction::AddMember(SigningKey::generate().verifying_key()),
        )
        .await
        .unwrap()
        .record;
    let mut target = NativeNetwork::create(&tp).await.unwrap();
    let mut target_peer = join(&mut target, &dir.path().join("target-peer")).await;
    let code = target.issue_precode(expiry(), Some(1)).await.unwrap();
    let te = server(&target).await;
    let mut m = NetworkMigration::prepare(source, code.invitation())
        .await
        .unwrap();
    let (r, s) = tokio::join!(m.redeem_target(address(&te), &code), async {
        let c = std::sync::Arc::new(
            te.accept()
                .await
                .unwrap()
                .accept_control()
                .unwrap()
                .await
                .unwrap(),
        );
        let r = target.serve_precode_admission(&c).await;
        c.closed().await;
        r
    });
    r.unwrap();
    s.unwrap();
    assert!(m.target_status().members.contains(&(id, Access::read())));
    assert!(!m.target_status().members.contains(&(id, Access::manage())));
    assert!(m.source_status().members.contains(&(id, Access::manage())));
    // Explicit destination genesis action, independent of the source role.
    let mut ts = target.begin_management_session();
    target
        .elevate_genesis(&mut ts, credential(&tp), Duration::from_secs(60))
        .unwrap();
    target
        .manage(&mut ts, ManagementAction::AppointAdministrator(id))
        .await
        .unwrap();
    let (r, s) = tokio::join!(m.validate_target(address(&te)), async {
        let c = te.accept().await.unwrap().await.unwrap();
        let r = target.serve_replica(&c).await;
        c.closed().await;
        r
    });
    r.unwrap();
    s.unwrap();
    let mut ss = m.begin_source_management_session();
    assert!(
        m.switch(&mut ss).await.is_err(),
        "old admin cannot bypass elevation"
    );
    m.elevate_source(&mut ss, credential(&p), Duration::from_secs(60))
        .unwrap();
    m.switch(&mut ss).await.unwrap();
    m.finish_source_exit(&mut ss).await.unwrap();
    let mut session = m.active().begin_management_session();
    let target_record = m
        .active_mut()
        .manage(
            &mut session,
            ManagementAction::AddMember(SigningKey::generate().verifying_key()),
        )
        .await
        .unwrap()
        .record;
    let old = m
        .source_history()
        .await
        .unwrap()
        .into_iter()
        .find(|r| Hash::digest(r.header()) == source_record)
        .unwrap();
    let new = m
        .active()
        .management_record(target_record)
        .await
        .unwrap()
        .unwrap();
    let old_header = Header::<ManagementExtension>::decode(old.header()).unwrap();
    let new_header = Header::<ManagementExtension>::decode(new.header()).unwrap();
    assert_eq!(old_header.verifying_key, new_header.verifying_key);
    assert_ne!(old_header.extensions.log_id, new_header.extensions.log_id);
    assert!(
        !new_header
            .extensions
            .group
            .dependencies
            .contains(&source_record)
    );
    assert!(
        m.active()
            .management_record(source_record)
            .await
            .unwrap()
            .is_none()
    );
    replicate(&target, &mut target_peer).await;
    replicate(m.active(), &mut target_peer).await;
    assert!(
        target_peer
            .management_record(target_record)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        target_peer
            .management_record(source_record)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        target_peer
            .ingest_management_record(old.header(), old.body())
            .await
            .is_err()
    );
    assert!(
        m.source_history()
            .await
            .unwrap()
            .iter()
            .all(|r| Hash::digest(r.header()) != target_record)
    );
    te.close().await;
    m.close().await;
    target.close().await;
    target_peer.close().await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn migration_crash_worker() {
    use miopunch_native_probe::management_store::RecordCheckpoint;
    let Ok(path) = std::env::var("MIO54_WORKER_DIR") else {
        return;
    };
    let path = std::path::PathBuf::from(path);
    let mode = std::env::var("MIO54_WORKER_MODE").unwrap();
    let code = miopunch_native_probe::precode::Precode::decode(
        &std::fs::read(path.parent().unwrap().join("code"))
            .unwrap()
            .try_into()
            .unwrap(),
    )
    .unwrap();
    if mode == "prepare" {
        let source = NativeNetwork::open(&path).await.unwrap();
        let _m = NetworkMigration::prepare(source, code.invitation())
            .await
            .unwrap();
        std::process::exit(77);
    }
    let mut m = NetworkMigration::open(&path).await.unwrap();
    m.set_checkpoint_observer(|checkpoint| {
        let mode = std::env::var("MIO54_WORKER_MODE").unwrap();
        if matches!(
            (mode.as_str(), checkpoint),
            ("authorize", RecordCheckpoint::AuthorizationSaved)
                | ("secret", RecordCheckpoint::SecretSaved)
                | ("validate", RecordCheckpoint::MigrationValidated)
                | ("switch", RecordCheckpoint::MigrationSwitched)
                | (
                    "exit_record",
                    RecordCheckpoint::MigrationSourceExitCommitted
                )
        ) {
            std::process::exit(77);
        }
    });
    if matches!(mode.as_str(), "authorize" | "secret" | "complete") {
        let addr = serde_json::from_str(&std::env::var("MIO54_WORKER_ADDRESS").unwrap()).unwrap();
        m.redeem_target(addr, &code).await.unwrap();
    } else if mode == "validate" {
        let addr = serde_json::from_str(&std::env::var("MIO54_WORKER_ADDRESS").unwrap()).unwrap();
        m.validate_target(addr).await.unwrap();
    } else {
        let mut session = m.begin_source_management_session();
        m.elevate_source(&mut session, credential(&path), Duration::from_secs(60))
            .unwrap();
        if mode == "switch" {
            m.switch(&mut session).await.unwrap();
        } else {
            m.finish_source_exit(&mut session).await.unwrap();
        }
    }
    std::process::exit(77);
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_death_at_each_durable_boundary_keeps_identity_progress_and_one_redemption() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("source");
    let source = NativeNetwork::create(&p).await.unwrap();
    let old = source.status();
    let mut config = source.resource_configuration().unwrap();
    config.replacement = Some(vec![]);
    source.save_resource_configuration(&config).unwrap();
    source.close().await;
    let mut target = NativeNetwork::create(dir.path().join("target"))
        .await
        .unwrap();
    let code = target.issue_precode(expiry(), Some(1)).await.unwrap();
    std::fs::write(dir.path().join("code"), code.encode()).unwrap();
    let te = server(&target).await;
    let stages = [
        ("prepare", MigrationPhase::Preparing),
        ("authorize", MigrationPhase::PendingMaterials),
        ("secret", MigrationPhase::MaterialsSaved),
        ("complete", MigrationPhase::MaterialsSaved),
        ("validate", MigrationPhase::Validated),
        ("switch", MigrationPhase::Switched),
        ("exit_record", MigrationPhase::Switched),
        ("exit", MigrationPhase::SourceExited),
    ];
    for (mode, phase) in stages {
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "migration_crash_worker", "--nocapture"])
            .env("MIO54_WORKER_DIR", &p)
            .env("MIO54_WORKER_MODE", mode)
            .env(
                "MIO54_WORKER_ADDRESS",
                serde_json::to_string(&address(&te)).unwrap(),
            )
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        if matches!(mode, "authorize" | "secret" | "complete" | "validate") {
            let serving = async {
                let incoming = te.accept().await.unwrap();
                if mode == "validate" {
                    let c = incoming.await.unwrap();
                    let result = target.serve_replica(&c).await.map_err(|e| e.to_string());
                    let _ = tokio::time::timeout(Duration::from_secs(5), c.closed()).await;
                    result
                } else {
                    let c = std::sync::Arc::new(incoming.accept_control().unwrap().await.unwrap());
                    let result = target
                        .serve_precode_admission(&c)
                        .await
                        .map(|_| ())
                        .map_err(|e| e.to_string());
                    let _ = tokio::time::timeout(Duration::from_secs(5), c.closed()).await;
                    result
                }
            };
            let (status, _served) = tokio::time::timeout(Duration::from_secs(25), async {
                tokio::join!(child.wait(), serving)
            })
            .await
            .unwrap();
            assert_eq!(status.unwrap().code(), Some(77), "{mode}");
        } else {
            assert_eq!(child.wait().await.unwrap().code(), Some(77), "{mode}");
        }
        let m = NetworkMigration::open(&p).await.unwrap();
        assert_eq!(m.phase(), phase, "{mode}");
        assert_eq!(m.active().status().node_id, old.node_id);
        assert_eq!(m.target_status().node_id, old.node_id);
        if matches!(
            phase,
            MigrationPhase::Preparing
                | MigrationPhase::PendingMaterials
                | MigrationPhase::MaterialsSaved
                | MigrationPhase::Validated
        ) {
            assert_eq!(m.active().status().anchor, old.anchor);
            assert_eq!(m.source_status().members, old.members);
        } else {
            assert_eq!(m.active().status().anchor, target.status().anchor);
        }
        if mode != "prepare" {
            assert_eq!(target.precode_usage(&code).await.unwrap(), 1, "{mode}");
        }
        println!("durable checkpoint {mode}: {phase:?}, identity preserved");
        m.close().await;
    }
    te.close().await;
    target.close().await;
}

#[tokio::test]
async fn offline_device_confirms_itself_with_fresh_invitation_and_ordinary_exit() {
    let dir = tempfile::tempdir().unwrap();
    let ap = dir.path().join("source-admin");
    let fp = dir.path().join("first");
    let lp = dir.path().join("late");
    let mut admin = NativeNetwork::create(&ap).await.unwrap();
    let first = join(&mut admin, &fp).await;
    let late = join(&mut admin, &lp).await;
    let late_status = late.status();
    late.close().await;
    let mut config = first.resource_configuration().unwrap();
    config.replacement = Some(vec![]);
    first.save_resource_configuration(&config).unwrap();
    let mut target = NativeNetwork::create(dir.path().join("target"))
        .await
        .unwrap();
    let code = target.issue_precode(expiry(), Some(1)).await.unwrap();
    let expired = target
        .issue_precode(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 1,
            Some(1),
        )
        .await
        .unwrap();
    let te = server(&target).await;
    let mut first = NetworkMigration::prepare(first, code.invitation())
        .await
        .unwrap();
    let (r, s) = tokio::join!(first.redeem_target(address(&te), &code), async {
        let c = std::sync::Arc::new(
            te.accept()
                .await
                .unwrap()
                .accept_control()
                .unwrap()
                .await
                .unwrap(),
        );
        let r = target.serve_precode_admission(&c).await;
        c.closed().await;
        r
    });
    r.unwrap();
    s.unwrap();
    let (r, s) = tokio::join!(first.validate_target(address(&te)), async {
        let c = te.accept().await.unwrap().await.unwrap();
        let r = target.serve_replica(&c).await;
        c.closed().await;
        r
    });
    r.unwrap();
    s.unwrap();
    let mut session = first.begin_source_management_session();
    first.switch(&mut session).await.unwrap();
    first.finish_source_exit(&mut session).await.unwrap();
    for record in first.source_history().await.unwrap() {
        admin
            .ingest_management_record(record.header(), record.body())
            .await
            .unwrap();
    }
    assert!(
        admin
            .status()
            .members
            .iter()
            .any(|(id, _)| *id == late_status.node_id)
    );
    assert!(
        admin
            .status()
            .members
            .iter()
            .any(|(id, _)| *id == admin.status().node_id)
    );
    let late = NativeNetwork::open(&lp).await.unwrap();
    assert_eq!(
        late.status(),
        late_status,
        "another node never migrates the offline device"
    );
    late.save_resource_configuration(&config).unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let mut late = NetworkMigration::prepare(late, expired.invitation())
        .await
        .unwrap();
    let (r, s) = tokio::join!(late.redeem_target(address(&te), &expired), async {
        let c = std::sync::Arc::new(
            te.accept()
                .await
                .unwrap()
                .accept_control()
                .unwrap()
                .await
                .unwrap(),
        );
        let r = target.serve_precode_admission(&c).await;
        c.closed().await;
        r
    });
    assert!(r.is_err());
    assert!(s.is_err());
    assert_eq!(late.active().status(), late_status);
    let fresh = target.issue_precode(expiry(), Some(1)).await.unwrap();
    late.use_new_target_precode(&fresh).await.unwrap();
    let (r, s) = tokio::join!(late.redeem_target(address(&te), &fresh), async {
        let c = std::sync::Arc::new(
            te.accept()
                .await
                .unwrap()
                .accept_control()
                .unwrap()
                .await
                .unwrap(),
        );
        let r = target.serve_precode_admission(&c).await;
        c.closed().await;
        r
    });
    r.unwrap();
    s.unwrap();
    assert_eq!(late.target_status().node_id, late_status.node_id);
    assert_eq!(target.precode_usage(&expired).await.unwrap(), 0);
    assert_eq!(target.precode_usage(&fresh).await.unwrap(), 1);
    late.close().await;
    first.close().await;
    te.close().await;
    target.close().await;
    admin.close().await;
}

#[tokio::test]
async fn explicit_cancel_leaves_target_without_changing_source() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("source");
    let source = NativeNetwork::create(&p).await.unwrap();
    let original = source.status();
    let mut config = source.resource_configuration().unwrap();
    config.replacement = Some(vec![]);
    source.save_resource_configuration(&config).unwrap();
    let mut target = NativeNetwork::create(dir.path().join("target"))
        .await
        .unwrap();
    let code = target.issue_precode(expiry(), Some(1)).await.unwrap();
    let te = server(&target).await;
    let mut m = NetworkMigration::prepare(source, code.invitation())
        .await
        .unwrap();
    // An unknown/in-flight approval cannot be discarded as if no target grant exists.
    assert!(m.cancel_preparation(None).await.is_err());
    let (r, s) = tokio::join!(m.redeem_target(address(&te), &code), async {
        let c = std::sync::Arc::new(
            te.accept()
                .await
                .unwrap()
                .accept_control()
                .unwrap()
                .await
                .unwrap(),
        );
        let r = target.serve_precode_admission(&c).await;
        c.closed().await;
        r
    });
    r.unwrap();
    s.unwrap();
    m.cancel_preparation(None).await.unwrap();
    assert_eq!(m.phase(), MigrationPhase::Cancelled);
    assert_eq!(m.active().status(), original);
    for record in m.cancelled_target_history().await.unwrap() {
        target
            .ingest_management_record(record.header(), record.body())
            .await
            .unwrap();
    }
    assert!(
        !target
            .status()
            .members
            .iter()
            .any(|(id, _)| *id == original.node_id)
    );
    m.close().await;
    std::fs::remove_file(p.join("migration.cancelled")).unwrap();
    let mut m = NetworkMigration::open(&p).await.unwrap();
    assert_eq!(m.phase(), MigrationPhase::Cancelling);
    let count = m.target_status().applied_count;
    m.cancel_preparation(None).await.unwrap();
    assert_eq!(
        m.target_status().applied_count,
        count,
        "resume does not sign a second leave"
    );
    assert_eq!(m.phase(), MigrationPhase::Cancelled);
    assert_eq!(m.active().status(), original);
    assert!(m.redeem_target(address(&te), &code).await.is_err());
    m.close().await;
    te.close().await;
    target.close().await;
}
