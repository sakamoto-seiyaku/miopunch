use miopunch_native_probe::management_auth::{AuthError, ManagementExtension, NetworkAnchor};
use miopunch_native_probe::management_store::{ManagementRecord, ManagementStore, StoreError};
use p2panda_auth::group::{GroupAction, GroupMember};
use p2panda_auth::{Access, GroupsExtensionArgs};
use p2panda_core::{Hash, Header, SigningKey, VerifyingKey};
use std::process::Command;
use std::str::FromStr;

fn create(root: &SigningKey, admin: &SigningKey, body: Option<&[u8]>) -> ManagementRecord {
    let mut builder = Header::<ManagementExtension>::builder();
    if let Some(body) = body {
        builder = builder.body(body);
    }
    let header = builder
        .build(
            root,
            ManagementExtension {
                version: 1,
                log_id: [1; 16],
                group: GroupsExtensionArgs {
                    group_id: root.verifying_key(),
                    action: GroupAction::Create {
                        initial_members: vec![
                            (
                                GroupMember::Individual(root.verifying_key()),
                                Access::manage(),
                            ),
                            (
                                GroupMember::Individual(admin.verifying_key()),
                                Access::manage(),
                            ),
                        ],
                    },
                    dependencies: vec![],
                },
            },
        )
        .encode();
    ManagementRecord::new(header, body)
}

fn anchor(root: &SigningKey, record: &ManagementRecord) -> NetworkAnchor {
    NetworkAnchor {
        root: root.verifying_key(),
        genesis: Hash::digest(record.header()),
    }
}

fn has_admin(nodes: &[(VerifyingKey, Access)], admin: &SigningKey) -> bool {
    nodes
        .iter()
        .any(|(id, access)| *id == admin.verifying_key() && *access == Access::manage())
}

fn add_member(
    admin: &SigningKey,
    root: VerifyingKey,
    member: &SigningKey,
    dependency: Hash,
) -> ManagementRecord {
    let header = Header::<ManagementExtension>::builder()
        .body(b"approval evidence")
        .build(
            admin,
            ManagementExtension {
                version: 1,
                log_id: [2; 16],
                group: GroupsExtensionArgs {
                    group_id: root,
                    action: GroupAction::Add {
                        member: GroupMember::Individual(member.verifying_key()),
                        access: Access::read(),
                    },
                    dependencies: vec![dependency],
                },
            },
        )
        .encode();
    ManagementRecord::new(header, Some(b"approval evidence"))
}

#[tokio::test]
async fn full_signed_record_survives_process_restart_with_body_intact() {
    let root = SigningKey::generate();
    let admin = SigningKey::generate();
    let record = create(&root, &admin, Some(b"management attachment"));
    let anchor = anchor(&root, &record);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("records.sqlite");

    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "write_record_in_child_process"])
        .env("MIOPUNCH_MANAGEMENT_STORE_CHILD_PATH", &path)
        .env(
            "MIOPUNCH_MANAGEMENT_STORE_CHILD_ROOT",
            anchor.root.to_string(),
        )
        .env(
            "MIOPUNCH_MANAGEMENT_STORE_CHILD_GENESIS",
            anchor.genesis.to_string(),
        )
        .env(
            "MIOPUNCH_MANAGEMENT_STORE_CHILD_HEADER",
            hex::encode(record.header()),
        )
        .env(
            "MIOPUNCH_MANAGEMENT_STORE_CHILD_BODY",
            hex::encode(record.body().unwrap()),
        )
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "{}",
        String::from_utf8_lossy(&child.stderr)
    );

    let reopened = ManagementStore::open(&path, anchor).await.unwrap();
    let id = anchor.genesis;
    assert!(has_admin(&reopened.nodes(), &admin));
    assert_eq!(reopened.get_record(id).await.unwrap(), Some(record));
    assert_eq!(reopened.applied_count(), 1);
    reopened.close().await;
}

#[test]
fn write_record_in_child_process() {
    let Ok(path) = std::env::var("MIOPUNCH_MANAGEMENT_STORE_CHILD_PATH") else {
        return;
    };
    let anchor = NetworkAnchor {
        root: VerifyingKey::from_str(
            &std::env::var("MIOPUNCH_MANAGEMENT_STORE_CHILD_ROOT").unwrap(),
        )
        .unwrap(),
        genesis: Hash::from_str(&std::env::var("MIOPUNCH_MANAGEMENT_STORE_CHILD_GENESIS").unwrap())
            .unwrap(),
    };
    let header =
        hex::decode(std::env::var("MIOPUNCH_MANAGEMENT_STORE_CHILD_HEADER").unwrap()).unwrap();
    let body = hex::decode(std::env::var("MIOPUNCH_MANAGEMENT_STORE_CHILD_BODY").unwrap()).unwrap();
    let record = ManagementRecord::new(header, Some(&body));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async move {
        let mut store = ManagementStore::open(path, anchor).await.unwrap();
        store.ingest(&record).await.unwrap();
        assert_eq!(store.apply_ready().await.unwrap(), 1);
        store.close().await;
    });
}

#[tokio::test]
async fn missing_body_does_not_claim_hash_before_complete_retransmission() {
    let root = SigningKey::generate();
    let admin = SigningKey::generate();
    let record = create(&root, &admin, Some(b"required body"));
    let anchor = anchor(&root, &record);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("records.sqlite");
    let missing = ManagementRecord::new(record.header().to_vec(), None);

    let store = ManagementStore::open(&path, anchor).await.unwrap();
    assert!(matches!(
        store.ingest(&missing).await,
        Err(StoreError::Auth(AuthError::IncompleteBody))
    ));
    let wrong = ManagementRecord::new(record.header().to_vec(), Some(b"wrong body"));
    assert!(matches!(
        store.ingest(&wrong).await,
        Err(StoreError::Auth(AuthError::Operation(_)))
    ));
    store.close().await;

    let mut restarted = ManagementStore::open(&path, anchor).await.unwrap();
    let id = restarted.ingest(&record).await.unwrap();
    assert_eq!(restarted.apply_ready().await.unwrap(), 1);
    assert_eq!(restarted.get_record(id).await.unwrap(), Some(record));
    assert!(has_admin(&restarted.nodes(), &admin));
    restarted.close().await;
}

#[tokio::test]
async fn no_body_management_record_is_applied_and_saved() {
    let root = SigningKey::generate();
    let admin = SigningKey::generate();
    let record = create(&root, &admin, None);
    let anchor = anchor(&root, &record);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("records.sqlite");

    let mut store = ManagementStore::open(&path, anchor).await.unwrap();
    let id = store.ingest(&record).await.unwrap();
    assert_eq!(store.apply_ready().await.unwrap(), 1);
    store.close().await;

    let reopened = ManagementStore::open(&path, anchor).await.unwrap();
    assert!(has_admin(&reopened.nodes(), &admin));
    assert_eq!(reopened.get_record(id).await.unwrap(), Some(record));
    reopened.close().await;
}

#[tokio::test]
async fn out_of_order_and_duplicate_ingest_converges_after_restart() {
    let root = SigningKey::generate();
    let admin = SigningKey::generate();
    let member = SigningKey::generate();
    let genesis = create(&root, &admin, None);
    let anchor = anchor(&root, &genesis);
    let add = add_member(&admin, anchor.root, &member, anchor.genesis);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("records.sqlite");

    let mut store = ManagementStore::open(&path, anchor).await.unwrap();
    let add_id = store.ingest(&add).await.unwrap();
    assert_eq!(store.apply_ready().await.unwrap(), 0);
    assert_eq!(store.get_record(add_id).await.unwrap(), None);
    store.close().await;

    let mut restarted = ManagementStore::open(&path, anchor).await.unwrap();
    assert_eq!(restarted.ingest(&add).await.unwrap(), add_id);
    restarted.ingest(&genesis).await.unwrap();
    assert_eq!(restarted.apply_ready().await.unwrap(), 2);
    assert_eq!(restarted.apply_ready().await.unwrap(), 0);
    assert_eq!(restarted.applied_count(), 2);
    assert!(
        restarted
            .nodes()
            .iter()
            .any(|(id, access)| { *id == member.verifying_key() && *access == Access::read() })
    );
    restarted.close().await;

    let reopened = ManagementStore::open(&path, anchor).await.unwrap();
    assert_eq!(reopened.applied_count(), 2);
    assert_eq!(reopened.get_record(add_id).await.unwrap(), Some(add));
    reopened.close().await;
}

#[tokio::test]
async fn rejected_signed_record_does_not_block_later_valid_history() {
    let root = SigningKey::generate();
    let admin = SigningKey::generate();
    let other = SigningKey::generate();
    let genesis = create(&root, &admin, None);
    let alternate = create(&root, &other, None);
    let anchor = anchor(&root, &genesis);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("records.sqlite");

    let mut store = ManagementStore::open(&path, anchor).await.unwrap();
    let alternate_id = store.ingest(&alternate).await.unwrap();
    store.ingest(&genesis).await.unwrap();
    assert_eq!(store.apply_ready().await.unwrap(), 1);
    assert_eq!(store.get_record(alternate_id).await.unwrap(), None);
    assert!(has_admin(&store.nodes(), &admin));
    store.close().await;

    let mut recovered = ManagementStore::open(&path, anchor).await.unwrap();
    recovered.ingest(&alternate).await.unwrap();
    assert_eq!(recovered.apply_ready().await.unwrap(), 0);
    assert_eq!(recovered.applied_count(), 1);
    assert!(has_admin(&recovered.nodes(), &admin));
    recovered.close().await;
}
