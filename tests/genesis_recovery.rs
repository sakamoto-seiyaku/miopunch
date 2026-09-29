use std::net::{Ipv4Addr, SocketAddr};

use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr};
use miopunch_native_probe::admission_proof::{AdmissionScope, AdmissionSecret};
use miopunch_native_probe::genesis_recovery::{
    GenesisCredential, RECOVERY_ALPN, recover_client, serve_recovery,
};
use miopunch_native_probe::management_auth::{ManagementAuth, ManagementExtension, NetworkAnchor};
use miopunch_native_probe::management_store::{ManagementRecord, ManagementStore};
use p2panda_auth::group::{GroupAction, GroupMember};
use p2panda_auth::{Access, GroupsExtensionArgs};
use p2panda_core::{Hash, Header, SigningKey, VerifyingKey};

fn signed(
    author: &SigningKey,
    root: VerifyingKey,
    action: GroupAction<VerifyingKey>,
    dependencies: Vec<Hash>,
    log_id: u8,
) -> ManagementRecord {
    ManagementRecord::new(
        Header::<ManagementExtension>::builder()
            .build(
                author,
                ManagementExtension {
                    version: 1,
                    log_id: [log_id; 16],
                    group: GroupsExtensionArgs {
                        group_id: root,
                        action,
                        dependencies,
                    },
                },
            )
            .encode(),
        None,
    )
}

async fn pair(
    server_key: &SigningKey,
    client_key: &SigningKey,
) -> (
    Endpoint,
    Endpoint,
    iroh::endpoint::Connection,
    iroh::endpoint::Connection,
) {
    let server = Endpoint::builder(presets::Minimal)
        .secret_key(iroh::SecretKey::from_bytes(server_key.as_bytes()))
        .alpns(vec![RECOVERY_ALPN.to_vec()])
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .bind()
        .await
        .unwrap();
    let client = Endpoint::builder(presets::Minimal)
        .secret_key(iroh::SecretKey::from_bytes(client_key.as_bytes()))
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .bind()
        .await
        .unwrap();
    let addr = server.addr();
    let target = EndpointAddr::new(server.id())
        .with_addrs(addr.ip_addrs().copied().map(iroh_base::TransportAddr::Ip));
    let (out, incoming) = tokio::join!(client.connect(target, RECOVERY_ALPN), async {
        server.accept().await.unwrap().await.unwrap()
    });
    (client, server, out.unwrap(), incoming)
}

async fn prepared_member() -> (
    tempfile::TempDir,
    SigningKey,
    SigningKey,
    SigningKey,
    NetworkAnchor,
    ManagementStore,
) {
    let root = SigningKey::generate();
    let old_admin = SigningKey::generate();
    let ordinary = SigningKey::generate();
    let create = signed(
        &root,
        root.verifying_key(),
        GroupAction::Create {
            initial_members: vec![
                (
                    GroupMember::Individual(root.verifying_key()),
                    Access::manage(),
                ),
                (
                    GroupMember::Individual(old_admin.verifying_key()),
                    Access::manage(),
                ),
            ],
        },
        vec![],
        1,
    );
    let anchor = NetworkAnchor {
        root: root.verifying_key(),
        genesis: Hash::digest(create.header()),
    };
    let add_ordinary = signed(
        &old_admin,
        anchor.root,
        GroupAction::Add {
            member: GroupMember::Individual(ordinary.verifying_key()),
            access: Access::read(),
        },
        vec![anchor.genesis],
        2,
    );
    let path = tempfile::tempdir().unwrap();
    let mut store = ManagementStore::open(path.path().join("member.db"), anchor)
        .await
        .unwrap();
    store.ingest(&create).await.unwrap();
    store.ingest(&add_ordinary).await.unwrap();
    assert_eq!(store.apply_ready().await.unwrap(), 2);
    (path, root, old_admin, ordinary, anchor, store)
}

#[tokio::test]
async fn pinned_root_credential_recovers_from_ordinary_member_without_secret_or_old_admin() {
    let (path, root, old_admin, ordinary, anchor, mut server_store) = prepared_member().await;
    let credential = GenesisCredential::from_saved(*root.as_bytes(), anchor.genesis);
    let recovering = SigningKey::generate();
    let secret = AdmissionSecret::generate();
    let scope = AdmissionScope::new(*anchor.root.as_bytes(), 3);
    let (client_endpoint, server_endpoint, client, server) = pair(&ordinary, &recovering).await;
    let (recovered, served) = tokio::join!(
        recover_client(&client_endpoint, &client, &credential, server_endpoint.id()),
        serve_recovery(
            &server_endpoint,
            &server,
            &mut server_store,
            &scope,
            &secret
        )
    );
    assert!(served.is_ok(), "server: {served:?}, client: {recovered:?}");
    let recovered = recovered.unwrap();
    assert_eq!(recovered.anchor, anchor);
    assert_eq!(recovered.secret.saved_bytes(), secret.saved_bytes());
    assert_eq!(recovered.scope, scope);
    assert_eq!(recovered.grant_id, served.unwrap());
    let view = ManagementAuth::replay_records(
        anchor,
        &recovered
            .records
            .iter()
            .map(|record| {
                (
                    record.header().to_vec(),
                    record.body().map(|body| body.to_vec()),
                )
            })
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert!(
        view.nodes()
            .contains(&(old_admin.verifying_key(), Access::manage()))
    );
    assert!(
        view.nodes()
            .contains(&(ordinary.verifying_key(), Access::read()))
    );
    assert!(
        view.nodes()
            .contains(&(recovering.verifying_key(), Access::manage()))
    );
    assert!(
        server_store
            .nodes()
            .contains(&(old_admin.verifying_key(), Access::manage()))
    );
    assert!(
        server_store
            .nodes()
            .contains(&(recovering.verifying_key(), Access::manage()))
    );
    let mut client_store = ManagementStore::open(path.path().join("recovered.db"), anchor)
        .await
        .unwrap();
    for record in &recovered.records {
        client_store.ingest(record).await.unwrap();
    }
    assert_eq!(client_store.apply_ready().await.unwrap(), 3);
    client_store.close().await;
    let reopened = ManagementStore::open(path.path().join("recovered.db"), anchor)
        .await
        .unwrap();
    assert!(
        reopened
            .nodes()
            .contains(&(recovering.verifying_key(), Access::manage()))
    );
    assert!(
        reopened
            .nodes()
            .contains(&(old_admin.verifying_key(), Access::manage()))
    );
    client_endpoint.close().await;
    server_endpoint.close().await;
}

fn proof_signature(
    root: &SigningKey,
    connection: &iroh::endpoint::Connection,
    claimed_anchor: NetworkAnchor,
    recovering: iroh_base::EndpointId,
    contact: iroh_base::EndpointId,
) -> [u8; 64] {
    let mut binding = [0; 32];
    connection
        .export_keying_material(&mut binding, b"EXPORTER-Channel-Binding", b"")
        .unwrap();
    let mut statement = b"miopunch/genesis-recovery/v1\0".to_vec();
    statement.extend_from_slice(claimed_anchor.root.as_bytes());
    statement.extend_from_slice(claimed_anchor.genesis.as_bytes());
    statement.extend_from_slice(recovering.as_bytes());
    statement.extend_from_slice(contact.as_bytes());
    statement.extend_from_slice(&binding);
    root.sign(&statement).to_bytes()
}

async fn rejected_proof(
    signature: [u8; 64],
    server_endpoint: &Endpoint,
    client: &iroh::endpoint::Connection,
    server: &iroh::endpoint::Connection,
    store: &mut ManagementStore,
    scope: &AdmissionScope,
    secret: &AdmissionSecret,
) {
    let (sent, served) = tokio::join!(
        async {
            let (mut send, mut recv) = client.open_bi().await.unwrap();
            send.write_all(&signature).await.unwrap();
            let mut first = [0; 1];
            recv.read_exact(&mut first).await
        },
        serve_recovery(server_endpoint, server, store, scope, secret)
    );
    assert!(sent.is_err(), "history was disclosed before proof");
    assert!(served.is_err(), "invalid root proof was accepted");
    assert_eq!(store.applied_count(), 2);
}

#[tokio::test]
async fn wrong_root_network_target_and_replayed_connection_get_no_material() {
    let (_path, root, _old_admin, ordinary, anchor, mut store) = prepared_member().await;
    let recovering = SigningKey::generate();
    let secret = AdmissionSecret::generate();
    let scope = AdmissionScope::new(*anchor.root.as_bytes(), 3);

    let (old_client_endpoint, old_server_endpoint, old_client, _old_server) =
        pair(&ordinary, &recovering).await;
    let old_signature = proof_signature(
        &root,
        &old_client,
        anchor,
        old_client_endpoint.id(),
        old_server_endpoint.id(),
    );
    old_client_endpoint.close().await;
    old_server_endpoint.close().await;

    for case in 0..4 {
        let (client_endpoint, server_endpoint, client, server) = pair(&ordinary, &recovering).await;
        let signature = match case {
            0 => proof_signature(
                &SigningKey::generate(),
                &client,
                anchor,
                client_endpoint.id(),
                server_endpoint.id(),
            ),
            1 => proof_signature(
                &root,
                &client,
                NetworkAnchor {
                    genesis: Hash::digest(b"different signed Create"),
                    ..anchor
                },
                client_endpoint.id(),
                server_endpoint.id(),
            ),
            2 => proof_signature(
                &root,
                &client,
                anchor,
                client_endpoint.id(),
                client_endpoint.id(),
            ),
            _ => old_signature,
        };
        rejected_proof(
            signature,
            &server_endpoint,
            &client,
            &server,
            &mut store,
            &scope,
            &secret,
        )
        .await;
        client_endpoint.close().await;
        server_endpoint.close().await;
    }
}

#[tokio::test]
async fn wrong_saved_create_hash_cannot_start_recovery() {
    let (_path, root, old_admin, ordinary, anchor, mut store) = prepared_member().await;
    let wrong =
        GenesisCredential::from_saved(*root.as_bytes(), Hash::digest(b"different signed Create"));
    let recovering = SigningKey::generate();
    let secret = AdmissionSecret::generate();
    let scope = AdmissionScope::new(*anchor.root.as_bytes(), 3);
    let (client_endpoint, server_endpoint, client, server) = pair(&ordinary, &recovering).await;
    let (recovered, served) = tokio::join!(
        recover_client(&client_endpoint, &client, &wrong, server_endpoint.id()),
        serve_recovery(&server_endpoint, &server, &mut store, &scope, &secret),
    );
    assert!(recovered.is_err());
    assert!(served.is_err());
    assert_eq!(store.applied_count(), 2);
    assert!(
        store
            .nodes()
            .contains(&(old_admin.verifying_key(), Access::manage()))
    );
    assert!(
        !store
            .nodes()
            .iter()
            .any(|(node, _)| *node == recovering.verifying_key())
    );
    client_endpoint.close().await;
    server_endpoint.close().await;
}

#[tokio::test]
async fn same_root_alternate_create_cannot_replace_saved_network_history() {
    let (_old_path, root, _old_admin, ordinary, original_anchor, _original_store) =
        prepared_member().await;
    let alternate_admin = SigningKey::generate();
    let alternate_create = signed(
        &root,
        original_anchor.root,
        GroupAction::Create {
            initial_members: vec![
                (
                    GroupMember::Individual(original_anchor.root),
                    Access::manage(),
                ),
                (
                    GroupMember::Individual(alternate_admin.verifying_key()),
                    Access::manage(),
                ),
            ],
        },
        vec![],
        7,
    );
    let alternate_anchor = NetworkAnchor {
        root: original_anchor.root,
        genesis: Hash::digest(alternate_create.header()),
    };
    assert_ne!(alternate_anchor.genesis, original_anchor.genesis);
    let add_ordinary = signed(
        &alternate_admin,
        alternate_anchor.root,
        GroupAction::Add {
            member: GroupMember::Individual(ordinary.verifying_key()),
            access: Access::read(),
        },
        vec![alternate_anchor.genesis],
        8,
    );
    let path = tempfile::tempdir().unwrap();
    let mut alternate_store =
        ManagementStore::open(path.path().join("alternate.db"), alternate_anchor)
            .await
            .unwrap();
    alternate_store.ingest(&alternate_create).await.unwrap();
    alternate_store.ingest(&add_ordinary).await.unwrap();
    assert_eq!(alternate_store.apply_ready().await.unwrap(), 2);

    let recovering = SigningKey::generate();
    let credential = GenesisCredential::from_saved(*root.as_bytes(), original_anchor.genesis);
    let secret = AdmissionSecret::generate();
    let scope = AdmissionScope::new(*alternate_anchor.root.as_bytes(), 4);
    let (client_endpoint, server_endpoint, client, server) = pair(&ordinary, &recovering).await;
    let (recovered, served) = tokio::join!(
        recover_client(&client_endpoint, &client, &credential, server_endpoint.id()),
        serve_recovery(
            &server_endpoint,
            &server,
            &mut alternate_store,
            &scope,
            &secret
        ),
    );
    assert!(recovered.is_err());
    assert!(served.is_err());
    assert_eq!(alternate_store.applied_count(), 2);
    assert!(
        !alternate_store
            .nodes()
            .iter()
            .any(|(node, _)| *node == recovering.verifying_key())
    );
    client_endpoint.close().await;
    server_endpoint.close().await;
}
