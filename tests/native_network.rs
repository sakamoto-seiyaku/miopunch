use std::process::Command;

use miopunch_native_probe::management_auth::ManagementExtension;
use miopunch_native_probe::native_network::{NativeNetwork, NodeError};
use p2panda_auth::Access;
use p2panda_auth::GroupsExtensionArgs;
use p2panda_auth::group::{GroupAction, GroupMember};
use p2panda_core::{Hash, Header, SigningKey};

fn cli(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_miopunch-native-probe"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn relative_data_directory_is_a_public_entry() {
    let dir = tempfile::tempdir().unwrap();
    let binary = env!("CARGO_BIN_EXE_miopunch-native-probe");
    let created = Command::new(binary)
        .current_dir(dir.path())
        .args(["network", "create", "node"])
        .output()
        .unwrap();
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let resumed = Command::new(binary)
        .current_dir(dir.path())
        .args(["network", "status", "node"])
        .output()
        .unwrap();
    assert!(resumed.status.success());
    assert_eq!(resumed.stdout, created.stdout);
}

#[tokio::test]
async fn create_and_restart_through_public_node_entry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("alpha");
    let path_str = path.to_str().unwrap();

    let created = cli(&["network", "create", path_str]);
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let created = String::from_utf8(created.stdout).unwrap();
    assert!(created.contains("admin"));
    assert!(!created.contains("secret"));

    // Both calls are new processes; each closes its store before returning.
    for _ in 0..2 {
        let resumed = cli(&["network", "status", path_str]);
        assert!(
            resumed.status.success(),
            "{}",
            String::from_utf8_lossy(&resumed.stderr)
        );
        assert_eq!(String::from_utf8(resumed.stdout).unwrap(), created);
    }
    let opened = NativeNetwork::open(&path).await.unwrap();
    let status = opened.status();
    assert_eq!(status.applied_count, 1);
    assert_eq!(status.members, vec![(status.node_id, Access::manage())]);
    assert_ne!(status.anchor.root.as_bytes(), status.node_id.as_bytes());
    let endpoint = opened
        .bind_endpoint(std::net::Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    assert_eq!(endpoint.id().as_bytes(), status.node_id.as_bytes());
    endpoint.close().await;
    opened.close().await;
}

#[tokio::test]
async fn new_credential_is_new_network_and_secrets_are_not_status() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first");
    let second = dir.path().join("second");
    let first_node = NativeNetwork::create(&first).await.unwrap();
    let first_status = first_node.status();
    first_node.close().await;
    let second_node = NativeNetwork::create(&second).await.unwrap();
    assert_ne!(first_status.anchor.root, second_node.status().anchor.root);
    second_node.close().await;

    assert!(matches!(
        NativeNetwork::create(&first).await,
        Err(NodeError::AlreadyExists)
    ));
    let status = cli(&["network", "status", first.to_str().unwrap()]);
    assert!(status.status.success());
    let status = String::from_utf8(status.stdout).unwrap();
    let credential = std::fs::read(first.join("genesis.credential")).unwrap();
    let secret = std::fs::read(first.join("admission.secret")).unwrap();
    assert_eq!(&credential[32..], first_status.anchor.genesis.as_bytes());
    assert!(!status.contains(&hex::encode(&credential[..32])));
    assert!(!status.contains(&hex::encode(secret)));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for name in ["node.key", "genesis.credential", "admission.secret"] {
            assert_eq!(
                std::fs::metadata(first.join(name))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
}

#[tokio::test]
async fn wrong_root_and_alternate_create_cannot_replace_pinned_network() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("alpha");
    let opened = NativeNetwork::create(&path).await.unwrap();
    let anchor = opened.status().anchor;
    opened.close().await;

    let credential_path = path.join("genesis.credential");
    let original = std::fs::read(&credential_path).unwrap();
    let wrong = SigningKey::generate();
    let mut altered = original.clone();
    altered[..32].copy_from_slice(wrong.as_bytes());
    std::fs::write(&credential_path, altered).unwrap();
    assert!(matches!(
        NativeNetwork::open(&path).await,
        Err(NodeError::InvalidCredential)
    ));

    std::fs::write(&credential_path, &original).unwrap();
    let root = SigningKey::from_bytes(&original[..32].try_into().unwrap());
    let alternate_admin = SigningKey::generate();
    let alternate_create = Header::<ManagementExtension>::builder()
        .build(
            &root,
            ManagementExtension {
                version: 1,
                log_id: [77; 16],
                group: GroupsExtensionArgs {
                    group_id: anchor.root,
                    action: GroupAction::Create {
                        initial_members: vec![
                            (GroupMember::Individual(anchor.root), Access::manage()),
                            (
                                GroupMember::Individual(alternate_admin.verifying_key()),
                                Access::manage(),
                            ),
                        ],
                    },
                    dependencies: vec![],
                },
            },
        )
        .encode();
    let alternate_hash = Hash::digest(&alternate_create);
    let mut alternate = original.clone();
    alternate[32..].copy_from_slice(alternate_hash.as_bytes());
    std::fs::write(&credential_path, alternate).unwrap();
    assert!(matches!(
        NativeNetwork::open(&path).await,
        Err(NodeError::InvalidCredential)
    ));
    std::fs::write(&credential_path, original).unwrap();
    let mut network = NativeNetwork::open(&path).await.unwrap();
    assert!(matches!(
        network
            .ingest_management_record(&alternate_create, None)
            .await,
        Err(NodeError::NotApplied)
    ));
    assert_eq!(network.status().anchor, anchor);
    assert_eq!(network.status().applied_count, 1);
    network.close().await;
}

#[tokio::test]
async fn invalid_record_cannot_create_a_valid_view() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("alpha");
    let mut opened = NativeNetwork::create(&path).await.unwrap();
    let anchor = opened.status().anchor;
    let unauthorized = SigningKey::generate();
    let record = Header::<miopunch_native_probe::management_auth::ManagementExtension>::builder()
        .build(
            &unauthorized,
            miopunch_native_probe::management_auth::ManagementExtension {
                version: 1,
                log_id: [7; 16],
                group: p2panda_auth::GroupsExtensionArgs {
                    group_id: anchor.root,
                    action: p2panda_auth::group::GroupAction::Add {
                        member: p2panda_auth::group::GroupMember::Individual(
                            SigningKey::generate().verifying_key(),
                        ),
                        access: Access::manage(),
                    },
                    dependencies: vec![anchor.genesis],
                },
            },
        )
        .encode();
    assert!(matches!(
        opened.ingest_management_record(&record, None).await,
        Err(NodeError::NotApplied)
    ));
    assert_eq!(opened.status().applied_count, 1);
    opened.close().await;
    assert_eq!(
        NativeNetwork::open(&path)
            .await
            .unwrap()
            .status()
            .applied_count,
        1
    );
}
