//! Public backup boundary: signed service policy is part of the same node.
use std::{
    net::Ipv4Addr,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use miopunch_native_probe::{
    genesis_recovery::GenesisCredential,
    native_client::NativeClient,
    native_network::NativeNetwork,
    node_backup::{NodeBackup, ReplacementPackage, ReplacementReceipt},
    service_policy::{Service, ServiceRule},
};
use p2panda_core::{Hash, SigningKey};

fn owned_directory() -> PathBuf {
    std::env::temp_dir().join(format!(
        "miopunch-device-backup-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[tokio::test]
async fn backup_compositions_require_qualification_on_every_open() {
    let owned = owned_directory();
    std::fs::create_dir(&owned).unwrap();
    let source = owned.join("status-source");
    let client = NativeClient::create(&source, Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    client.export_recovery_card(None).await.unwrap();
    client.acknowledge_recovery_saved().await.unwrap();
    let identity = client.status().await.node.node_id;
    client.close().await.unwrap();
    NativeNetwork::open(&source)
        .await
        .unwrap()
        .create_backup(owned.join("first"))
        .await
        .unwrap();
    let mut observations = Vec::new();
    let first = NativeClient::open(owned.join("first"), Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    observations.push((
        "known backup direct open",
        first.status().await.replacement_ready,
    ));
    assert_eq!(first.status().await.node.node_id, identity);
    first.close().await.unwrap();
    let restored = NativeNetwork::restore_backup(owned.join("first"), owned.join("restored"))
        .await
        .unwrap();
    restored.create_backup(owned.join("second")).await.unwrap();
    let second = NativeClient::open(owned.join("second"), Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    observations.push((
        "restored then backed up",
        second.status().await.replacement_ready,
    ));
    second.close().await.unwrap();
    let original = NativeClient::open(&source, Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    assert!(original.status().await.replacement_ready);
    original
        .prepare_node_replacement(owned.join("portable"))
        .await
        .unwrap();
    let text = std::fs::read_to_string(owned.join("portable")).unwrap();
    let portable = NativeClient::restore_node_replacement(
        &text,
        owned.join("portable-target"),
        Ipv4Addr::LOCALHOST,
    )
    .await
    .unwrap();
    observations.push(("portable import", portable.status().await.replacement_ready));
    portable.close().await.unwrap();
    NativeNetwork::open(owned.join("portable-target"))
        .await
        .unwrap()
        .create_backup(owned.join("portable-legacy"))
        .await
        .unwrap();
    let composed = NativeClient::open(owned.join("portable-legacy"), Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    observations.push((
        "portable then legacy backup",
        composed.status().await.replacement_ready,
    ));
    composed.close().await.unwrap();
    std::fs::remove_dir_all(&owned).unwrap();
    for (composition, ready) in observations {
        assert!(
            !ready,
            "{composition} discarded the qualification requirement"
        );
    }
}

#[tokio::test]
async fn legacy_backup_activation_does_not_trust_inherited_precode_counts() {
    let owned = owned_directory();
    std::fs::create_dir(&owned).unwrap();
    let source_path = owned.join("counter-source");
    let mut source = NativeNetwork::create(&source_path).await.unwrap();
    let expiry = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    let code = source.issue_precode(expiry, Some(1)).await.unwrap();
    let initial = source.precode_usage_status(&code).await.unwrap();
    assert_eq!(initial.saved_uses, Some(0));
    assert!(initial.count_trusted);
    source
        .create_backup(owned.join("counter-copy"))
        .await
        .unwrap();
    let copy = NativeNetwork::open(owned.join("counter-copy"))
        .await
        .unwrap();
    let mut observations = vec![(
        "new snapshot direct open",
        copy.precode_usage_status(&code)
            .await
            .unwrap()
            .count_trusted,
    )];
    copy.close().await;
    let original = NativeNetwork::open(&source_path).await.unwrap();
    let original_trusted = original
        .precode_usage_status(&code)
        .await
        .unwrap()
        .count_trusted;
    original.close().await;
    let restored =
        NativeNetwork::restore_backup(owned.join("counter-copy"), owned.join("counter-restored"))
            .await
            .unwrap();
    observations.push((
        "explicit restore",
        restored
            .precode_usage_status(&code)
            .await
            .unwrap()
            .count_trusted,
    ));
    restored
        .create_backup(owned.join("counter-rebacked"))
        .await
        .unwrap();
    let rebacked = NativeNetwork::open(owned.join("counter-rebacked"))
        .await
        .unwrap();
    observations.push((
        "unqualified restore rebacked",
        rebacked
            .precode_usage_status(&code)
            .await
            .unwrap()
            .count_trusted,
    ));
    rebacked.close().await;
    // Optional compatibility fixtures are produced through the public backup
    // API of immutable older SDKs. Only their local count status is read; no
    // old invitation is ever submitted for redemption or sent over a network.
    if let Some(root) = std::env::var_os("MIOPUNCH_LEGACY_BACKUP_FIXTURES") {
        for label in ["native5", "native6"] {
            let fixture = PathBuf::from(&root).join(label);
            let bytes: [u8; 128] = std::fs::read(fixture.join("code.private"))
                .unwrap()
                .try_into()
                .unwrap();
            let code = miopunch_native_probe::precode::Precode::decode(&bytes).unwrap();
            let node = NativeNetwork::open(fixture.join("backup")).await.unwrap();
            observations.push((
                label,
                node.precode_usage_status(&code)
                    .await
                    .unwrap()
                    .count_trusted,
            ));
            node.close().await;
        }
    }
    std::fs::remove_dir_all(&owned).unwrap();
    assert!(
        original_trusted,
        "creating a copy must not stop the active source's codes"
    );
    for (entry, trusted) in observations {
        assert!(!trusted, "{entry} trusted an inherited precode count");
    }
}

#[tokio::test]
async fn qualified_legacy_copy_keeps_fresh_precode_counts_across_reopen() {
    let owned = owned_directory();
    std::fs::create_dir(&owned).unwrap();
    let source_path = owned.join("qualified-source");
    let source = NativeClient::create(&source_path, Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    source.export_recovery_card(None).await.unwrap();
    source.acknowledge_recovery_saved().await.unwrap();
    let expiry = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    let peer_invitation = source.issue_precode(expiry, Some(1)).await.unwrap();
    let peer = NativeClient::join(
        owned.join("current-peer"),
        &peer_invitation,
        Ipv4Addr::LOCALHOST,
    )
    .await
    .unwrap();
    assert!(matches!(
        peer.request_admission(&peer_invitation).await.unwrap(),
        miopunch_native_probe::manual_admission::AdmissionStage::Complete
    ));
    let inherited = source.issue_precode(expiry, Some(1)).await.unwrap();
    assert_eq!(source.precode_uses(&inherited).await.unwrap(), 0);
    source.close().await.unwrap();
    NativeNetwork::open(&source_path)
        .await
        .unwrap()
        .create_backup(owned.join("qualified-copy"))
        .await
        .unwrap();
    let copy = NativeClient::open(owned.join("qualified-copy"), Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    assert!(!copy.status().await.replacement_ready);
    assert!(copy.precode_uses(&inherited).await.is_err());
    assert!(
        copy.complete_node_replacement(peer.address())
            .await
            .unwrap()["ready"]
            .as_bool()
            .unwrap()
    );
    assert!(copy.precode_uses(&inherited).await.is_err());
    let fresh = copy.issue_precode(expiry, Some(1)).await.unwrap();
    assert_eq!(copy.precode_uses(&fresh).await.unwrap(), 0);
    copy.close().await.unwrap();
    let copy = NativeClient::open(owned.join("qualified-copy"), Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    assert!(copy.status().await.replacement_ready);
    assert_eq!(copy.precode_uses(&fresh).await.unwrap(), 0);
    let joined = NativeClient::join(owned.join("fresh-member"), &fresh, Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    assert!(matches!(
        joined.request_admission(&fresh).await.unwrap(),
        miopunch_native_probe::manual_admission::AdmissionStage::Complete
    ));
    assert_eq!(copy.precode_uses(&fresh).await.unwrap(), 1);
    assert!(copy.precode_uses(&inherited).await.is_err());
    joined.close().await.unwrap();
    copy.close().await.unwrap();
    peer.close().await.unwrap();
    std::fs::remove_dir_all(&owned).unwrap();
}

#[tokio::test]
async fn complete_node_backup_retains_effective_service_policy() {
    let owned = owned_directory();
    std::fs::create_dir(&owned).unwrap();
    let source = owned.join("source");
    let client = NativeClient::create(&source, Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let card = client.export_recovery_card(None).await.unwrap();
    assert!(!card.is_empty());
    client.acknowledge_recovery_saved().await.unwrap();
    let identity = client.status().await.node.node_id;
    let rule = ServiceRule {
        members: false,
        guests: false,
        allowed: Default::default(),
        denied: Default::default(),
    };
    let revision = client
        .set_service_rule(identity, Service::Files, None, rule.clone())
        .await
        .unwrap();
    client.close().await.unwrap();
    let node = NativeNetwork::open(&source).await.unwrap();
    node.create_backup(owned.join("backup")).await.unwrap();
    let restored = NativeNetwork::restore_backup(owned.join("backup"), owned.join("restored"))
        .await
        .unwrap();
    assert_eq!(restored.status().node_id, identity);
    restored.close().await;
    let restored = NativeClient::open(owned.join("restored"), Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    assert!(!restored.status().await.replacement_ready);
    let premature = owned.join("premature-receiver");
    assert!(restored.enable_files(&premature).await.is_err());
    assert!(!premature.exists());
    let status = restored.service_status().await.unwrap();
    let actual = status["policies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["provider"] == identity.to_string())
        .cloned();
    let recovery_saved = restored.status().await.recovery_saved;
    restored.close().await.unwrap();
    std::fs::remove_dir_all(owned).unwrap();
    let policy = actual.expect("complete node backup dropped the effective service policy");
    assert_eq!(policy["revision"], revision.to_string());
    assert_eq!(policy["rule"], serde_json::to_value(rule).unwrap());
    assert!(
        recovery_saved,
        "complete node backup dropped the original recovery-save confirmation"
    );
}

#[tokio::test]
async fn failed_capsule_save_retains_paused_source_until_explicit_abort() {
    let owned = owned_directory();
    std::fs::create_dir(&owned).unwrap();
    let source = owned.join("source");
    let client = NativeClient::create(&source, Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    client.export_recovery_card(None).await.unwrap();
    client.acknowledge_recovery_saved().await.unwrap();
    let identity = client.status().await.node.node_id;
    // A missing parent is a real filesystem failure, without fault hooks.
    assert!(
        client
            .prepare_node_replacement(owned.join("absent/capsule"))
            .await
            .is_err()
    );
    assert!(source.join("node.key").exists());
    assert!(source.join("genesis.credential").exists());
    assert!(
        NativeClient::open(&source, Ipv4Addr::LOCALHOST)
            .await
            .is_err()
    );
    let path = owned.join("retry-capsule");
    NativeClient::retry_node_replacement_backup(&source, &path)
        .await
        .unwrap();
    let backup = NodeBackup::decode(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(backup.package.identity, identity);
    let wrong_package = ReplacementPackage::new(identity, backup.package.network);
    let wrong = ReplacementReceipt::sign(
        wrong_package,
        [9; 16],
        vec![backup.package.network.genesis],
        &SigningKey::from_bytes(&backup.node.key),
    )
    .unwrap();
    assert!(NativeClient::finalize_node_replacement(&source, &wrong).is_err());
    assert!(source.join("node.key").exists());
    assert!(source.join("genesis.credential").exists());
    assert!(NativeClient::abort_node_replacement(&source, backup.package.id, false).is_err());
    assert!(
        NativeClient::abort_node_replacement(&source, Hash::digest(b"wrong operation"), true)
            .is_err()
    );
    assert!(
        NativeClient::open(&source, Ipv4Addr::LOCALHOST)
            .await
            .is_err()
    );
    NativeClient::abort_node_replacement(&source, backup.package.id, true).unwrap();
    let reopened = NativeClient::open(&source, Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    assert_eq!(reopened.status().await.node.node_id, identity);
    assert!(
        reopened.credential_custody().await.unwrap()["holds_genesis"]
            .as_bool()
            .unwrap()
    );
    reopened.close().await.unwrap();
    std::fs::remove_dir_all(owned).unwrap();
}

#[tokio::test]
async fn portable_backup_preserves_completed_rotation_and_refuses_pending_rotation() {
    let owned = owned_directory();
    std::fs::create_dir(&owned).unwrap();
    let source = owned.join("source");
    let mut node = NativeNetwork::create(&source).await.unwrap();
    let saved = std::fs::read(source.join("genesis.credential")).unwrap();
    let credential = GenesisCredential::from_saved(
        saved[..32].try_into().unwrap(),
        Hash::from_bytes(saved[32..].try_into().unwrap()),
    );
    let mut session = node.begin_management_session();
    node.elevate_genesis(&mut session, credential, std::time::Duration::from_secs(60))
        .unwrap();
    let rotation = node.begin_admission_rotation(&mut session).await.unwrap();
    node.close().await;
    let client = NativeClient::open(&source, Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    client.export_recovery_card(None).await.unwrap();
    client.acknowledge_recovery_saved().await.unwrap();
    assert!(
        client
            .prepare_node_replacement(owned.join("pending-capsule"))
            .await
            .is_err()
    );
    assert!(!source.join("replacement.source").exists());
    let mut node = NativeNetwork::open(&source).await.unwrap();
    let saved = std::fs::read(source.join("genesis.credential")).unwrap();
    let credential = GenesisCredential::from_saved(
        saved[..32].try_into().unwrap(),
        Hash::from_bytes(saved[32..].try_into().unwrap()),
    );
    let mut session = node.begin_management_session();
    node.elevate_genesis(&mut session, credential, std::time::Duration::from_secs(60))
        .unwrap();
    node.finish_admission_rotation(&mut session).await.unwrap();
    node.close().await;
    let client = NativeClient::open(&source, Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let path = owned.join("complete-capsule");
    client.prepare_node_replacement(&path).await.unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let backup = NodeBackup::decode(&text).unwrap();
    assert_eq!(backup.node.native_rotation.rotations.len(), 1);
    assert_eq!(backup.node.native_rotation.acknowledgements.len(), 1);
    assert!(!backup.node.native_rotation.materials.is_empty());
    assert_eq!(backup.node.generation, rotation.generation);
    let target = owned.join("target");
    let restored = NativeClient::restore_node_replacement(&text, &target, Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    assert!(!restored.status().await.replacement_ready);
    restored.close().await.unwrap();
    let restored = NativeNetwork::open(&target).await.unwrap();
    let progress = restored.rotation_progress().await.unwrap().unwrap();
    assert_eq!(progress.id, rotation.id);
    assert_eq!(
        progress.stage,
        miopunch_native_probe::admission_rotation::RotationStage::Complete
    );
    assert_eq!(progress.acknowledged.len(), 1);
    restored.close().await;
    std::fs::remove_dir_all(owned).unwrap();
}
