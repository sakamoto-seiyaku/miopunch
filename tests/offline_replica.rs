//! Public node replicas over real admitted iroh TLS/QUIC connections.
#[path = "support/native_connections.rs"]
mod native_connections;

use std::net::Ipv4Addr;
use std::time::Duration;

use iroh::{EndpointAddr, endpoint::Connection};
use miopunch_native_probe::admission_gate::{ADMISSION_REQUEST_ALPN, ProtocolScope};
use miopunch_native_probe::management_auth::ManagementExtension;
use miopunch_native_probe::manual_admission::ManualExchange;
use miopunch_native_probe::native_network::NativeNetwork;
use miopunch_native_probe::offline_replica::REPLICA_ALPN;
use p2panda_auth::{
    Access, GroupsExtensionArgs,
    group::{GroupAction, GroupMember},
};
use p2panda_core::{Hash, Header, SigningKey};

fn add_record(
    c: &NativeNetwork,
    path: &std::path::Path,
    dependency: Hash,
    target: p2panda_core::VerifyingKey,
    body: Option<&[u8]>,
) -> Vec<u8> {
    add_record_with_log(c, path, dependency, target, body, rand::random())
}

fn add_record_with_log(
    c: &NativeNetwork,
    path: &std::path::Path,
    dependency: Hash,
    target: p2panda_core::VerifyingKey,
    body: Option<&[u8]>,
    log_id: [u8; 16],
) -> Vec<u8> {
    let author = SigningKey::from_bytes(
        &std::fs::read(path.join("node.key"))
            .unwrap()
            .try_into()
            .unwrap(),
    );
    let mut builder = Header::<ManagementExtension>::builder();
    if let Some(body) = body {
        builder = builder.body(body);
    }
    builder
        .build(
            &author,
            ManagementExtension {
                version: 1,
                log_id,
                group: GroupsExtensionArgs {
                    group_id: c.status().anchor.root,
                    action: GroupAction::Add {
                        member: GroupMember::Individual(target),
                        access: Access::read(),
                    },
                    dependencies: vec![dependency],
                },
            },
        )
        .encode()
}

async fn connect(
    a: &iroh::endpoint::RestrictedEndpoint,
    b: &iroh::endpoint::RestrictedEndpoint,
    alpn: &[u8],
) -> (Connection, Connection) {
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

#[tokio::test]
async fn replica_provider_consumes_the_original_caller_remainder_without_a_fresh_wait() {
    let dir = tempfile::tempdir().unwrap();
    let mut source = NativeNetwork::create(dir.path().join("source"))
        .await
        .unwrap();
    let receiver = join(&mut source, &dir.path().join("receiver")).await;
    let source_endpoint = endpoint(&source).await;
    let receiver_endpoint = endpoint(&receiver).await;
    let (client, server) = connect(&receiver_endpoint, &source_endpoint, REPLICA_ALPN).await;
    let (mut request, mut response) = client.open_bi().await.unwrap();
    request.write_all(b"REPL").await.unwrap();
    let deadline = n0_future::time::Instant::now() + Duration::from_millis(400);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let started = std::time::Instant::now();
    let (result, ()) = tokio::join!(
        tokio::time::timeout(
            Duration::from_secs(2),
            source.serve_replica_with_deadline(&server, deadline)
        ),
        async {
            // The admitted real stream receives a genuine nonempty manifest,
            // then intentionally sends no LogSync response/receipt.
            let mut length = [0; 4];
            response.read_exact(&mut length).await.unwrap();
            let length = u32::from_be_bytes(length) as usize;
            assert!(length > 0 && length < 4096);
            let mut manifest = vec![0; length];
            response.read_exact(&mut manifest).await.unwrap();
        },
    );
    assert!(matches!(
        result.expect("provider must not start another wait"),
        Err(miopunch_native_probe::offline_replica::ReplicaError::Transport)
    ));
    assert!(
        started.elapsed() < Duration::from_millis(300),
        "only the remaining original budget is available"
    );
    assert!(server.close_reason().is_some());
    assert_eq!(source.status().applied_count, 2);
    assert_eq!(receiver.status().applied_count, 2);
    client.close(0u32.into(), b"bounded replica test complete");
    source_endpoint.close().await;
    receiver_endpoint.close().await;
    source.close().await;
    receiver.close().await;
}

#[cfg(feature = "replica-test-hooks")]
#[test]
fn replica_crash_worker() {
    let Ok(path) = std::env::var("MIO31_CRASH_NODE") else {
        return;
    };
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut node = NativeNetwork::open(path).await.unwrap();
        let endpoint = endpoint(&node).await;
        let contact: iroh_base::EndpointId = std::env::var("MIO31_CRASH_CONTACT")
            .unwrap()
            .parse()
            .unwrap();
        let address = std::env::var("MIO31_CRASH_ADDRESS")
            .unwrap()
            .parse()
            .unwrap();
        let connection = endpoint
            .connect(
                EndpointAddr::new(contact).with_ip_addr(address),
                REPLICA_ALPN,
            )
            .await
            .unwrap();
        node.set_replica_checkpoint_observer(|stage| {
            if format!("{stage:?}") == std::env::var("MIO31_CRASH_STAGE").unwrap() {
                eprintln!("actual replica process exits at {stage:?}");
                std::process::exit(81);
            }
        });
        node.sync_replica(&connection).await.unwrap();
        panic!("requested real storage checkpoint must have fired");
    });
}

#[cfg(feature = "replica-test-hooks")]
async fn crash_case(stage: &str) {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("c");
    let bp = dir.path().join("b");
    let ap = dir.path().join("a");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let mut b = join(&mut c, &bp).await;
    let mut a = join(&mut c, &ap).await;
    let ce = endpoint(&c).await;
    let be = endpoint(&b).await;
    let (client, server) = connect(&be, &ce, REPLICA_ALPN).await;
    let (saved, served) = tokio::join!(b.sync_replica(&client), c.serve_replica(&server));
    assert!(saved.unwrap().complete());
    served.unwrap();
    client.close(0u32.into(), b"checkpoint setup");
    server.close(0u32.into(), b"setup");
    be.close().await;
    b.close().await;
    let target = SigningKey::generate().verifying_key();
    let history = c.management_history().await.unwrap();
    let dependency = Hash::digest(history.last().unwrap().header());
    let body = b"durable replica checkpoint attachment";
    let header = add_record(&c, &cp, dependency, target, Some(body));
    let id = c
        .ingest_management_record(&header, Some(body))
        .await
        .unwrap();
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "replica_crash_worker", "--nocapture"])
        .env("MIO31_CRASH_NODE", &bp)
        .env("MIO31_CRASH_CONTACT", ce.id().to_string())
        .env(
            "MIO31_CRASH_ADDRESS",
            ce.addr().ip_addrs().next().unwrap().to_string(),
        )
        .env("MIO31_CRASH_STAGE", stage)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let (child, served) = tokio::join!(
        tokio::task::spawn_blocking(move || child.wait_with_output().unwrap()),
        async {
            let connection = ce.accept().await.unwrap().await.unwrap();
            c.serve_replica(&connection).await
        }
    );
    let child = child.unwrap();
    eprintln!("{}", String::from_utf8_lossy(&child.stderr));
    assert_eq!(child.status.code(), Some(81));
    eprintln!("provider transport result at {stage}: {served:?}");
    let mut b = NativeNetwork::open(&bp).await.unwrap();
    let be = endpoint(&b).await;
    if stage == "BeforeStorageCommit" {
        assert!(
            b.management_record(id).await.unwrap().is_none(),
            "an uncommitted record cannot be advertised as saved"
        );
        let (client, server) = connect(&be, &ce, REPLICA_ALPN).await;
        let (saved, served) = tokio::join!(b.sync_replica(&client), c.serve_replica(&server));
        assert!(saved.unwrap().complete());
        served.unwrap();
        client.close(0u32.into(), b"retry");
        server.close(0u32.into(), b"retry");
    }
    // All other checkpoints recover exclusively from B's committed original
    // records, with C unavailable even for dependency or body fetches.
    ce.close().await;
    c.close().await;
    assert!(b.status().members.contains(&(target, Access::read())));
    assert_eq!(
        b.management_record(id).await.unwrap().unwrap().body(),
        Some(body.as_slice())
    );
    let ae = endpoint(&a).await;
    let (client, server) = connect(&ae, &be, REPLICA_ALPN).await;
    let (saved, served) = tokio::join!(a.sync_replica(&client), b.serve_replica(&server));
    let saved = saved.unwrap();
    served.unwrap();
    assert!(saved.complete());
    assert_eq!(a.status().applied_count, 4);
    assert_eq!(
        a.management_record(id).await.unwrap().unwrap().header(),
        header
    );
    assert!(a.status().members.contains(&(target, Access::read())));
    assert!(a.management_record(dependency).await.unwrap().is_some());
    let (again, served) = tokio::join!(a.sync_replica(&client), b.serve_replica(&server));
    assert!(again.unwrap().complete());
    served.unwrap();
    assert_eq!(a.status().applied_count, 4);
    eprintln!(
        "{stage}: recovered identical four-record view; offline replica still serves; duplicate replay adds no action"
    );
    client.close(0u32.into(), b"done");
    server.close(0u32.into(), b"done");
    ae.close().await;
    be.close().await;
    a.close().await;
    b.close().await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_after_causal_selection_replays_without_losing_authorization() {
    crash_case("AfterOrdererSelect").await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_before_record_commit_never_advertises_it_and_retry_completes() {
    crash_case("BeforeStorageCommit").await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_after_record_commit_replays_and_still_serves() {
    crash_case("AfterStorageCommit").await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_before_view_commit_replays_and_still_serves() {
    crash_case("BeforeGroupsCommit").await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn process_exit_after_view_commit_does_not_duplicate_application() {
    crash_case("AfterGroupsCommit").await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn required_body_missing_then_complete_retry_survives_restart() {
    use miopunch_native_probe::management_auth::AuthError;
    use miopunch_native_probe::management_store::{ManagementRecord, StoreError};
    use miopunch_native_probe::offline_replica::ReplicaError;
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("c");
    let bp = dir.path().join("b");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let mut b = join(&mut c, &bp).await;
    let target = SigningKey::generate().verifying_key();
    let body = b"required approval attachment";
    let header = add_record(&c, &cp, c.status().anchor.genesis, target, Some(body));
    let id = Hash::digest(&header);
    let ce = endpoint(&c).await;
    let be = endpoint(&b).await;
    let (client, server) = connect(&be, &ce, REPLICA_ALPN).await;
    let missing = [ManagementRecord::new(header.clone(), None)];
    let (rejected, served) = tokio::join!(
        b.sync_replica(&client),
        c.serve_replica_fixture(&server, &missing)
    );
    assert!(matches!(
        rejected,
        Err(ReplicaError::Store(StoreError::Auth(
            AuthError::IncompleteBody
        )))
    ));
    assert!(served.is_err());
    assert_eq!(b.status().applied_count, 2);
    assert!(b.management_record(id).await.unwrap().is_none());
    be.close().await;
    b.close().await;
    let mut b = NativeNetwork::open(&bp).await.unwrap();
    let be = endpoint(&b).await;
    c.ingest_management_record(&header, Some(body))
        .await
        .unwrap();
    let (client, server) = connect(&be, &ce, REPLICA_ALPN).await;
    let (saved, served) = tokio::join!(b.sync_replica(&client), c.serve_replica(&server));
    assert!(saved.unwrap().complete());
    served.unwrap();
    assert!(b.status().members.contains(&(target, Access::read())));
    assert_eq!(
        b.management_record(id).await.unwrap().unwrap().body(),
        Some(body.as_slice())
    );
    assert_eq!(b.status().applied_count, 3);
    client.close(0u32.into(), b"done");
    server.close(0u32.into(), b"done");
    be.close().await;
    ce.close().await;
    b.close().await;
    c.close().await;
}

#[tokio::test]
async fn signed_extension_only_action_is_applied_by_replica_consumer() {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("c");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let mut b = join(&mut c, &dir.path().join("b")).await;
    let target = SigningKey::generate().verifying_key();
    let header = add_record(&c, &cp, c.status().anchor.genesis, target, None);
    let id = c.ingest_management_record(&header, None).await.unwrap();
    let ce = endpoint(&c).await;
    let be = endpoint(&b).await;
    let (client, server) = connect(&be, &ce, REPLICA_ALPN).await;
    let (saved, served) = tokio::join!(b.sync_replica(&client), c.serve_replica(&server));
    let saved = saved.unwrap();
    served.unwrap();
    assert!(saved.complete());
    assert!(saved.applied.contains(&id));
    assert!(b.status().members.contains(&(target, Access::read())));
    assert_eq!(b.management_record(id).await.unwrap().unwrap().body(), None);
    assert_eq!(b.status().applied_count, 3);
    client.close(0u32.into(), b"done");
    server.close(0u32.into(), b"done");
    be.close().await;
    ce.close().await;
    b.close().await;
    c.close().await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn missing_dependency_is_saved_but_not_complete_then_out_of_order_duplicates_apply_once() {
    use miopunch_native_probe::management_store::ManagementRecord;
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("c");
    let bp = dir.path().join("b");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let mut b = join(&mut c, &bp).await;
    let parent_target = SigningKey::generate().verifying_key();
    let child_target = SigningKey::generate().verifying_key();
    // Distinct logs deliberately put the dependent operation first on the
    // actual LogSync wire, independently of the array's fixture order.
    let parent = add_record_with_log(
        &c,
        &cp,
        c.status().anchor.genesis,
        parent_target,
        None,
        [2; 16],
    );
    let parent_id = Hash::digest(&parent);
    let body = b"dependent original author attachment";
    let child = add_record_with_log(&c, &cp, parent_id, child_target, Some(body), [1; 16]);
    let child_id = Hash::digest(&child);
    let child = ManagementRecord::new(child, Some(body));
    let parent = ManagementRecord::new(parent, None);
    let ce = endpoint(&c).await;
    let be = endpoint(&b).await;
    let (client, server) = connect(&be, &ce, REPLICA_ALPN).await;
    let missing = [child.clone()];
    let (pending, served) = tokio::join!(
        b.sync_replica(&client),
        c.serve_replica_fixture(&server, &missing)
    );
    let pending = pending.unwrap();
    served.unwrap();
    assert!(pending.transport_done);
    assert_eq!(pending.saved, vec![child_id]);
    assert!(pending.applied.is_empty());
    assert!(
        !pending.complete(),
        "wire Done cannot hide missing causal evidence"
    );
    assert!(
        b.management_record(child_id).await.unwrap().is_none(),
        "unverified pending records are never supplied"
    );
    assert_eq!(b.status().applied_count, 2);
    client.close(0u32.into(), b"pending");
    server.close(0u32.into(), b"pending");
    be.close().await;
    b.close().await;
    let mut b = NativeNetwork::open(&bp).await.unwrap();
    assert_eq!(b.status().applied_count, 2);
    let be = endpoint(&b).await;
    let (client, server) = connect(&be, &ce, REPLICA_ALPN).await;
    let full = [child.clone(), parent, child.clone()];
    let (saved, served) = tokio::join!(
        b.sync_replica(&client),
        c.serve_replica_fixture(&server, &full)
    );
    assert!(saved.unwrap().complete());
    served.unwrap();
    assert_eq!(b.status().applied_count, 4);
    assert!(
        b.status()
            .members
            .contains(&(parent_target, Access::read()))
    );
    assert!(b.status().members.contains(&(child_target, Access::read())));
    assert_eq!(b.management_record(child_id).await.unwrap(), Some(child));
    assert!(b.management_record(parent_id).await.unwrap().is_some());
    client.close(0u32.into(), b"done");
    server.close(0u32.into(), b"done");
    be.close().await;
    ce.close().await;
    b.close().await;
    c.close().await;
}

#[tokio::test]
async fn unadmitted_identity_cannot_start_replication_or_read_history() {
    let dir = tempfile::tempdir().unwrap();
    let c = NativeNetwork::create(dir.path().join("c")).await.unwrap();
    let applicant =
        NativeNetwork::join(dir.path().join("applicant"), c.manual_invitation().unwrap())
            .await
            .unwrap();
    let ce = endpoint(&c).await;
    let ae = endpoint(&applicant).await;
    let address = EndpointAddr::new(ce.id()).with_ip_addr(*ce.addr().ip_addrs().next().unwrap());
    let (client, server) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(ae.connect(address, REPLICA_ALPN), async {
            ce.accept().await.unwrap().await
        })
    })
    .await
    .unwrap();
    assert!(
        client.is_err() && server.is_err(),
        "TLS identity and an invitation grant no replica access"
    );
    assert_eq!(applicant.status().applied_count, 0);
    assert!(applicant.management_history().await.unwrap().is_empty());
    ae.close().await;
    ce.close().await;
    applicant.close().await;
    c.close().await;
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn unauthorized_records_and_private_material_are_excluded_from_served_replicas() {
    use miopunch_native_probe::management_store::ManagementRecord;
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("c");
    let bp = dir.path().join("b");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let mut b = join(&mut c, &bp).await;
    let mut a = join(&mut c, &dir.path().join("a")).await;
    let denied_target = SigningKey::generate().verifying_key();
    let ordinary = SigningKey::from_bytes(
        &std::fs::read(bp.join("node.key"))
            .unwrap()
            .try_into()
            .unwrap(),
    );
    let dependency = Hash::digest(
        b.management_history()
            .await
            .unwrap()
            .last()
            .unwrap()
            .header(),
    );
    let header = Header::<ManagementExtension>::builder()
        .body(b"unauthorized record marker")
        .build(
            &ordinary,
            ManagementExtension {
                version: 1,
                log_id: [72; 16],
                group: GroupsExtensionArgs {
                    group_id: c.status().anchor.root,
                    action: GroupAction::Add {
                        member: GroupMember::Individual(denied_target),
                        access: Access::read(),
                    },
                    dependencies: vec![dependency],
                },
            },
        )
        .encode();
    let denied_id = Hash::digest(&header);
    let denied = [ManagementRecord::new(
        header,
        Some(b"unauthorized record marker"),
    )];
    let ce = endpoint(&c).await;
    let be = endpoint(&b).await;
    let (client, server) = connect(&be, &ce, REPLICA_ALPN).await;
    let mut exporter = [0; 32];
    client
        .export_keying_material(&mut exporter, b"EXPORTER-Channel-Binding", b"")
        .unwrap();
    let (received, served) = tokio::join!(
        b.sync_replica(&client),
        c.serve_replica_fixture(&server, &denied)
    );
    let received = received.unwrap();
    served.unwrap();
    assert!(!received.complete());
    assert!(received.applied.is_empty());
    assert_eq!(b.status().applied_count, 2);
    assert!(b.management_record(denied_id).await.unwrap().is_none());
    let (saved, served) = tokio::join!(b.sync_replica(&client), c.serve_replica(&server));
    assert!(saved.unwrap().complete());
    served.unwrap();
    let history = b.management_history().await.unwrap();
    let raw: Vec<_> = history
        .iter()
        .map(|record| (record.header(), record.body()))
        .collect();
    let bytes = p2panda_core::cbor::encode_cbor(&raw).unwrap();
    let node_key = std::fs::read(bp.join("node.key")).unwrap();
    let secret = std::fs::read(bp.join("admission.material")).unwrap();
    let root_backup = std::fs::read(cp.join("genesis.credential")).unwrap();
    for private in [
        &node_key[..],
        &secret[8..],
        &root_backup[..32],
        &exporter[..],
    ] {
        assert!(
            !bytes.windows(private.len()).any(|window| window == private),
            "private material cannot enter shareable history"
        );
    }
    assert!(
        !bytes
            .windows(b"unauthorized record marker".len())
            .any(|window| window == b"unauthorized record marker")
    );
    client.close(0u32.into(), b"offline");
    server.close(0u32.into(), b"offline");
    ce.close().await;
    c.close().await;
    let ae = endpoint(&a).await;
    let (client, server) = connect(&ae, &be, REPLICA_ALPN).await;
    let (saved, served) = tokio::join!(a.sync_replica(&client), b.serve_replica(&server));
    let saved = saved.unwrap();
    served.unwrap();
    assert!(saved.complete());
    assert!(!saved.scope.contains(&denied_id));
    assert!(a.management_record(denied_id).await.unwrap().is_none());
    assert!(
        !a.status()
            .members
            .contains(&(denied_target, Access::read()))
    );
    assert_eq!(a.status().applied_count, 3);
    client.close(0u32.into(), b"done");
    server.close(0u32.into(), b"done");
    ae.close().await;
    be.close().await;
    a.close().await;
    b.close().await;
}

async fn join(c: &mut NativeNetwork, path: &std::path::Path) -> NativeNetwork {
    let mut node = NativeNetwork::join(path, c.manual_invitation().unwrap())
        .await
        .unwrap();
    let ce = c.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let ne = node.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = native_connections::control_pair(&ne, &ce, ADMISSION_REQUEST_ALPN).await;
    let (request, served) = tokio::join!(
        node.request_manual_admission(&client),
        c.serve_manual_admission(&server)
    );
    assert!(matches!(request.unwrap(), ManualExchange::AwaitingApproval));
    served.unwrap();
    let request = c.manual_requests().await.unwrap().pop().unwrap();
    c.approve_manual_admission(&request).await.unwrap();
    let receiver = async {
        let ManualExchange::Materials(material) =
            node.request_manual_admission(&client).await.unwrap()
        else {
            panic!("approval")
        };
        node.complete_manual_admission(material).await.unwrap();
    };
    let (_, served) = tokio::join!(receiver, c.serve_manual_admission(&server));
    served.unwrap();
    client.close(0u32.into(), b"done");
    server.close(0u32.into(), b"done");
    ne.close().await;
    ce.close().await;
    node
}

async fn endpoint(node: &NativeNetwork) -> iroh::endpoint::RestrictedEndpoint {
    node.admission_gate()
        .register_protocol(REPLICA_ALPN, ProtocolScope::Member);
    let endpoint = node.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    endpoint.set_alpns(vec![REPLICA_ALPN.to_vec()]);
    endpoint
}

#[tokio::test]
async fn original_author_offline_restarted_member_supplies_body_and_dependencies() {
    let dir = tempfile::tempdir().unwrap();
    let cp = dir.path().join("c");
    let bp = dir.path().join("b");
    let ap = dir.path().join("a");
    let mut c = NativeNetwork::create(&cp).await.unwrap();
    let mut b = join(&mut c, &bp).await;
    let mut a = join(&mut c, &ap).await;
    let target = SigningKey::generate().verifying_key();
    let author = SigningKey::from_bytes(
        &std::fs::read(cp.join("node.key"))
            .unwrap()
            .try_into()
            .unwrap(),
    );
    let before = c.management_history().await.unwrap();
    let dependency = Hash::digest(before.last().unwrap().header());
    let body = b"original C approval attachment";
    let header = Header::<ManagementExtension>::builder()
        .body(body)
        .build(
            &author,
            ManagementExtension {
                version: 1,
                log_id: [71; 16],
                group: GroupsExtensionArgs {
                    group_id: c.status().anchor.root,
                    action: GroupAction::Add {
                        member: GroupMember::Individual(target),
                        access: Access::read(),
                    },
                    dependencies: vec![dependency],
                },
            },
        )
        .encode();
    let id = c
        .ingest_management_record(&header, Some(body))
        .await
        .unwrap();
    let ce = endpoint(&c).await;
    let be = endpoint(&b).await;
    let (client, server) = connect(&be, &ce, REPLICA_ALPN).await;
    let (saved, supplied) = tokio::join!(b.sync_replica(&client), c.serve_replica(&server));
    assert!(saved.unwrap().complete());
    supplied.unwrap();
    client.close(0u32.into(), b"C goes offline");
    server.close(0u32.into(), b"offline");
    ce.close().await;
    c.close().await;
    be.close().await;
    b.close().await;
    // B's durable identity, history and authorization view are reopened before A contacts it.
    let b = NativeNetwork::open(&bp).await.unwrap();
    let be = endpoint(&b).await;
    let ae = endpoint(&a).await;
    let (client, server) = connect(&ae, &be, REPLICA_ALPN).await;
    let (saved, supplied) = tokio::join!(a.sync_replica(&client), b.serve_replica(&server));
    let saved = saved.unwrap();
    supplied.unwrap();
    assert!(saved.transport_done && saved.complete());
    assert_eq!(saved.source.as_bytes(), b.status().node_id.as_bytes());
    assert!(a.status().members.contains(&(target, Access::read())));
    let received = a.management_record(id).await.unwrap().unwrap();
    assert_eq!(received.header(), header);
    assert_eq!(received.body(), Some(body.as_slice()));
    assert_eq!(
        Header::<ManagementExtension>::decode(received.header())
            .unwrap()
            .verifying_key,
        author.verifying_key()
    );
    assert!(a.management_record(dependency).await.unwrap().is_some());
    let count = a.status().applied_count;
    let (again, supplied) = tokio::join!(a.sync_replica(&client), b.serve_replica(&server));
    assert!(again.unwrap().complete());
    supplied.unwrap();
    assert_eq!(
        a.status().applied_count,
        count,
        "duplicates create no additional action"
    );
    client.close(0u32.into(), b"done");
    server.close(0u32.into(), b"done");
    ae.close().await;
    be.close().await;
    a.close().await;
    b.close().await;
}

#[cfg(feature = "replica-test-hooks")]
mod paged_history {
    use super::*;
    use iroh::endpoint::{ControlExchange, ControlMessageFormat};
    use miopunch_native_probe::management_auth::NetworkAnchor;
    use miopunch_native_probe::management_store::ManagementRecord;
    use p2panda_net::codec::Codec;
    use p2panda_sync::protocols::{LogSyncMessage, Logs};
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;
    use tokio_util::{
        bytes::BytesMut,
        codec::{Decoder, Encoder},
    };

    const FRAME_BYTES: usize = 16 * 1024 * 1024;
    const PAGE_RECORDS: usize = 4096;

    #[derive(Deserialize, Serialize)]
    struct WireManifest {
        anchor: NetworkAnchor,
        logs: Logs<[u8; 16]>,
        ids: Vec<Hash>,
        #[serde(default, skip_serializing_if = "is_false")]
        more: bool,
    }
    fn is_false(value: &bool) -> bool {
        !*value
    }

    #[derive(Deserialize, Serialize)]
    struct WireReceipt {
        records_saved: bool,
        view_applied: bool,
    }

    async fn send<M: Serialize + Send>(exchange: &ControlExchange, message: M, limit: usize) {
        let mut bytes = BytesMut::new();
        Codec::<M>::new()
            .max_frame_len(limit)
            .encode(message, &mut bytes)
            .unwrap();
        assert!(
            bytes.len() - 4 <= limit,
            "actual encoded frame exceeds its wire budget"
        );
        exchange
            .send_message(
                ControlMessageFormat::LengthPrefixed32 { maximum: limit },
                &bytes[4..],
            )
            .await
            .unwrap();
    }

    async fn receive<M: serde::de::DeserializeOwned>(
        exchange: &ControlExchange,
        limit: usize,
    ) -> M {
        let bytes = exchange
            .recv_message(ControlMessageFormat::LengthPrefixed32 { maximum: limit })
            .await
            .unwrap();
        assert!(bytes.len() <= limit);
        let mut frame = BytesMut::new();
        frame.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        frame.extend_from_slice(&bytes);
        Codec::<M>::new()
            .max_frame_len(limit)
            .decode(&mut frame)
            .unwrap()
            .unwrap()
    }

    enum MembershipHistory {
        DistinctMembers,
        SmallNetwork { tip: Hash },
    }

    fn records(
        node: &NativeNetwork,
        path: &std::path::Path,
        count: usize,
        body_bytes: usize,
        members: MembershipHistory,
    ) -> Vec<ManagementRecord> {
        let author = SigningKey::from_bytes(
            &std::fs::read(path.join("node.key"))
                .unwrap()
                .try_into()
                .unwrap(),
        );
        let mut previous = None;
        let log = [183; 16];
        let (repeated, first_dependency) = match members {
            MembershipHistory::DistinctMembers => (None, node.status().anchor.genesis),
            MembershipHistory::SmallNetwork { tip } => {
                (Some(SigningKey::generate().verifying_key()), tip)
            }
        };
        (0..count)
            .map(|seq| {
                let target = repeated.unwrap_or_else(|| SigningKey::generate().verifying_key());
                let body = (body_bytes != 0).then(|| vec![seq as u8; body_bytes]);
                let mut builder = Header::<ManagementExtension>::builder()
                    .seq_num(seq as u32)
                    .backlink(previous);
                if let Some(body) = &body {
                    builder = builder.body(body);
                }
                let header = builder.build(
                    &author,
                    ManagementExtension {
                        version: 1,
                        log_id: log,
                        group: GroupsExtensionArgs {
                            group_id: node.status().anchor.root,
                            action: if repeated.is_some() && seq % 2 != 0 {
                                GroupAction::Remove {
                                    member: GroupMember::Individual(target),
                                }
                            } else {
                                GroupAction::Add {
                                    member: GroupMember::Individual(target),
                                    access: Access::read(),
                                }
                            },
                            dependencies: vec![previous.unwrap_or(first_dependency)],
                        },
                    },
                );
                previous = Some(header.hash());
                ManagementRecord::from_signed_header_for_test(header, body.as_deref())
            })
            .collect()
    }

    async fn read_manifest(exchange: &ControlExchange) -> WireManifest {
        let bytes = exchange
            .recv_message(ControlMessageFormat::LengthPrefixed32 {
                maximum: FRAME_BYTES,
            })
            .await
            .unwrap();
        assert!(!bytes.is_empty() && bytes.len() <= FRAME_BYTES);
        let manifest: WireManifest = p2panda_core::cbor::decode_cbor(bytes.as_slice()).unwrap();
        assert!(manifest.ids.len() <= PAGE_RECORDS);
        assert!(manifest.logs.values().map(Vec::len).sum::<usize>() <= PAGE_RECORDS);
        manifest
    }

    // This reader exchanges actual p2panda messages with the production provider.
    // Its empty Have requests precisely each advertised page, even if its first
    // operation retains a nonzero original log sequence.
    async fn read_page(
        exchange: &ControlExchange,
        manifest: &WireManifest,
    ) -> Vec<ManagementRecord> {
        let have: LogSyncMessage<[u8; 16]> = receive(exchange, FRAME_BYTES).await;
        assert!(matches!(have, LogSyncMessage::Have(_)));
        send(
            exchange,
            LogSyncMessage::<[u8; 16]>::Have(BTreeMap::new()),
            FRAME_BYTES,
        )
        .await;
        send(exchange, LogSyncMessage::<[u8; 16]>::Done, FRAME_BYTES).await;
        let presync: LogSyncMessage<[u8; 16]> = receive(exchange, FRAME_BYTES).await;
        assert!(
            matches!(presync, LogSyncMessage::PreSync { total_operations, .. } if total_operations as usize == manifest.ids.len())
        );
        let mut records = Vec::new();
        let mut bytes = 0;
        loop {
            match receive::<LogSyncMessage<[u8; 16]>>(exchange, FRAME_BYTES).await {
                LogSyncMessage::Operation((header, body)) => {
                    bytes += header.len() + body.as_ref().map_or(0, Vec::len);
                    assert!(
                        bytes <= FRAME_BYTES,
                        "one LogSync page exceeds the record byte budget"
                    );
                    assert!(records.len() < PAGE_RECORDS);
                    records.push(ManagementRecord::new(header, body.as_deref()));
                }
                LogSyncMessage::Done => break,
                other => panic!("unexpected page message {other:?}"),
            }
        }
        assert_eq!(records.len(), manifest.ids.len());
        for record in &records {
            assert!(manifest.ids.contains(&Hash::digest(record.header())));
        }
        records
    }

    #[tokio::test]
    async fn small_history_preserves_the_legacy_manifest_and_receipt_wire() {
        #[derive(Serialize)]
        struct LegacyManifest {
            anchor: NetworkAnchor,
            logs: Logs<[u8; 16]>,
            ids: Vec<Hash>,
        }
        let directory = tempfile::tempdir().unwrap();
        let mut c = NativeNetwork::create(directory.path().join("c"))
            .await
            .unwrap();
        let b = join(&mut c, &directory.path().join("b")).await;
        let ce = endpoint(&c).await;
        let be = endpoint(&b).await;
        let (client, server) = connect(&be, &ce, REPLICA_ALPN).await;
        let legacy_reader = async {
            let (send_stream, recv) = client.open_bi().await.unwrap();
            let exchange = ControlExchange::from_streams(send_stream, recv);
            exchange
                .send_message(ControlMessageFormat::Fixed { length: 4 }, b"REPL")
                .await
                .unwrap();
            let bytes = exchange
                .recv_message(ControlMessageFormat::LengthPrefixed32 {
                    maximum: FRAME_BYTES,
                })
                .await
                .unwrap();
            let manifest: WireManifest = p2panda_core::cbor::decode_cbor(bytes.as_slice()).unwrap();
            assert!(!manifest.more);
            let legacy = LegacyManifest {
                anchor: manifest.anchor,
                logs: manifest.logs.clone(),
                ids: manifest.ids.clone(),
            };
            assert_eq!(
                p2panda_core::cbor::encode_cbor(&legacy).unwrap(),
                bytes,
                "one-page history retains the exact old manifest shape"
            );
            read_page(&exchange, &manifest).await;
            send(
                &exchange,
                WireReceipt {
                    records_saved: true,
                    view_applied: true,
                },
                64,
            )
            .await;
            let acknowledgement: WireReceipt = receive(&exchange, 64).await;
            assert!(acknowledgement.records_saved && acknowledgement.view_applied);
            exchange.finish_send().await.unwrap();
        };
        let ((), served) = tokio::join!(legacy_reader, c.serve_replica(&server));
        served.unwrap();
        client.close(0u32.into(), b"done");
        server.close(0u32.into(), b"done");
        ce.close().await;
        be.close().await;
        c.close().await;
        b.close().await;
    }

    #[tokio::test]
    async fn record_limit_pages_use_actual_logsync_and_keep_high_original_sequences() {
        let directory = tempfile::tempdir().unwrap();
        let cp = directory.path().join("c");
        let mut c = NativeNetwork::create(&cp).await.unwrap();
        let b = join(&mut c, &directory.path().join("b")).await;
        let records = records(
            &c,
            &cp,
            PAGE_RECORDS + 2,
            0,
            MembershipHistory::DistinctMembers,
        );
        let ce = endpoint(&c).await;
        let be = endpoint(&b).await;
        let (client, server) = connect(&be, &ce, REPLICA_ALPN).await;
        let reader = async {
            let (send_stream, recv) = client.open_bi().await.unwrap();
            let exchange = ControlExchange::from_streams(send_stream, recv);
            exchange
                .send_message(ControlMessageFormat::Fixed { length: 4 }, b"REPL")
                .await
                .unwrap();
            let mut actual = Vec::new();
            let mut lengths = Vec::new();
            loop {
                let manifest = read_manifest(&exchange).await;
                let more = manifest.more;
                let page = read_page(&exchange, &manifest).await;
                lengths.push(page.len());
                actual.extend(page);
                send(
                    &exchange,
                    WireReceipt {
                        records_saved: true,
                        view_applied: !more,
                    },
                    64,
                )
                .await;
                if !more {
                    break;
                }
            }
            let acknowledgement: WireReceipt = receive(&exchange, 64).await;
            assert!(acknowledgement.records_saved && acknowledgement.view_applied);
            exchange.finish_send().await.unwrap();
            assert_eq!(lengths.len(), 33);
            assert!(lengths[..32].iter().all(|count| *count == 128));
            assert_eq!(lengths.last(), Some(&2));
            assert_eq!(
                actual, records,
                "pages preserve signed originals and their order within the log"
            );
            let first_after_original_limit =
                Header::<ManagementExtension>::decode(actual[PAGE_RECORDS].header()).unwrap();
            assert_eq!(first_after_original_limit.seq_num, PAGE_RECORDS as u32);
        };
        let ((), served) = tokio::join!(reader, c.serve_replica_fixture(&server, &records));
        served.unwrap();
        client.close(0u32.into(), b"done");
        server.close(0u32.into(), b"done");

        // An old reader ignores the extra manifest field and expects its old
        // success ACK after one page. It must receive a non-Receipt continuation.
        let (client, server) = connect(&be, &ce, REPLICA_ALPN).await;
        let legacy_reader = async {
            let (send_stream, recv) = client.open_bi().await.unwrap();
            let exchange = ControlExchange::from_streams(send_stream, recv);
            exchange
                .send_message(ControlMessageFormat::Fixed { length: 4 }, b"REPL")
                .await
                .unwrap();
            let manifest = read_manifest(&exchange).await;
            assert!(manifest.more);
            read_page(&exchange, &manifest).await;
            send(
                &exchange,
                WireReceipt {
                    records_saved: true,
                    view_applied: true,
                },
                64,
            )
            .await;
            let continuation = exchange
                .recv_message(ControlMessageFormat::LengthPrefixed32 {
                    maximum: FRAME_BYTES,
                })
                .await
                .unwrap();
            let mut frame = BytesMut::new();
            frame.extend_from_slice(&(continuation.len() as u32).to_be_bytes());
            frame.extend_from_slice(&continuation);
            assert!(
                Codec::<WireReceipt>::new().decode(&mut frame).is_err(),
                "the first page must not look complete to the old reader"
            );
            client.close(0u32.into(), b"legacy reader rejects continuation");
        };
        let ((), served) = tokio::join!(legacy_reader, c.serve_replica_fixture(&server, &records));
        assert!(served.is_err());
        ce.close().await;
        be.close().await;
        c.close().await;
        b.close().await;
    }

    #[tokio::test]
    async fn truncated_multi_page_history_is_durable_and_complete_retry_keeps_originals() {
        let directory = tempfile::tempdir().unwrap();
        let cp = directory.path().join("c");
        let mut c = NativeNetwork::create(&cp).await.unwrap();
        let mut b = join(&mut c, &directory.path().join("b")).await;
        // Each attachment fits one frame; their total exceeds one 16MiB page.
        let additions = records(
            &c,
            &cp,
            3,
            8 * 1024 * 1024,
            MembershipHistory::DistinctMembers,
        );
        for record in &additions {
            c.ingest_management_record(record.header(), record.body())
                .await
                .unwrap();
        }
        let ce = endpoint(&c).await;
        let be = endpoint(&b).await;
        let (client, server) = connect(&be, &ce, REPLICA_ALPN).await;
        let truncated_provider = async {
            let (send_stream, recv) = server.accept_bi().await.unwrap();
            let exchange = ControlExchange::from_streams(send_stream, recv);
            assert_eq!(
                exchange
                    .recv_message(ControlMessageFormat::Fixed { length: 4 })
                    .await
                    .unwrap(),
                b"REPL"
            );
            let record = &additions[0];
            let header = Header::<ManagementExtension>::decode(record.header()).unwrap();
            let logs = BTreeMap::from([(header.verifying_key, vec![header.extensions.log_id])]);
            let heights = BTreeMap::from([(
                header.verifying_key,
                BTreeMap::from([(header.extensions.log_id, header.seq_num)]),
            )]);
            let manifest = WireManifest {
                anchor: c.status().anchor,
                logs,
                ids: vec![Hash::digest(record.header())],
                more: true,
            };
            let bytes = p2panda_core::cbor::encode_cbor(&manifest).unwrap();
            exchange
                .send_message(
                    ControlMessageFormat::LengthPrefixed32 {
                        maximum: FRAME_BYTES,
                    },
                    &bytes,
                )
                .await
                .unwrap();
            send(
                &exchange,
                LogSyncMessage::<[u8; 16]>::Have(heights),
                FRAME_BYTES,
            )
            .await;
            assert!(matches!(
                receive::<LogSyncMessage<[u8; 16]>>(&exchange, FRAME_BYTES).await,
                LogSyncMessage::Have(_)
            ));
            send(
                &exchange,
                LogSyncMessage::<[u8; 16]>::PreSync {
                    total_operations: 1,
                    total_bytes: (record.header().len() + record.body().unwrap().len()) as u32,
                },
                FRAME_BYTES,
            )
            .await;
            assert!(matches!(
                receive::<LogSyncMessage<[u8; 16]>>(&exchange, FRAME_BYTES).await,
                LogSyncMessage::Done
            ));
            send(
                &exchange,
                LogSyncMessage::<[u8; 16]>::Operation((
                    record.header().to_vec(),
                    record.body().map(ToOwned::to_owned),
                )),
                FRAME_BYTES,
            )
            .await;
            send(&exchange, LogSyncMessage::<[u8; 16]>::Done, FRAME_BYTES).await;
            let receipt: WireReceipt = receive(&exchange, 64).await;
            assert!(
                receipt.records_saved && !receipt.view_applied,
                "continuation receipt cannot claim the full scope applied"
            );
            server.close(0u32.into(), b"cut before promised next manifest");
        };
        let (result, ()) = tokio::join!(b.sync_replica(&client), truncated_provider);
        assert!(
            result.is_err(),
            "a promised missing page cannot complete the exchange"
        );
        assert_eq!(
            b.management_record(Hash::digest(additions[0].header()))
                .await
                .unwrap(),
            Some(additions[0].clone())
        );
        assert!(
            b.management_record(Hash::digest(additions[1].header()))
                .await
                .unwrap()
                .is_none()
        );
        let (client, server) = connect(&be, &ce, REPLICA_ALPN).await;
        let (result, served) = tokio::join!(b.sync_replica(&client), c.serve_replica(&server));
        served.unwrap();
        let result = result.unwrap();
        assert!(result.complete());
        assert_eq!(result.scope.len(), 5);
        assert_eq!(
            result.received, 2,
            "the saved attachment must not cross the wire again"
        );
        assert_eq!(result.reused, 3);
        assert_eq!(
            b.status().applied_count,
            5,
            "retry must not add an already saved action twice"
        );
        for (seq, original) in additions.iter().enumerate() {
            let stored = b
                .management_record(Hash::digest(original.header()))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(&stored, original);
            assert_eq!(
                Header::<ManagementExtension>::decode(stored.header())
                    .unwrap()
                    .seq_num,
                seq as u32
            );
        }
        client.close(0u32.into(), b"done");
        server.close(0u32.into(), b"done");
        ce.close().await;
        be.close().await;
        c.close().await;
        b.close().await;
    }

    #[tokio::test]
    async fn more_than_one_record_limit_resumes_in_normal_windows_and_finishes_full_scope() {
        use miopunch_native_probe::offline_replica::ReplicaError;
        let directory = tempfile::tempdir().unwrap();
        let cp = directory.path().join("c");
        let bp = directory.path().join("b");
        let mut c = NativeNetwork::create(&cp).await.unwrap();
        let mut b = join(&mut c, &bp).await;
        let mut frozen = c.management_history().await.unwrap();
        // Like daily management, extend the current causal head. The join's
        // approval grant is the sole tip of this frozen history.
        let tip = Hash::digest(frozen.last().unwrap().header());
        frozen.extend(records(
            &c,
            &cp,
            PAGE_RECORDS + 1,
            0,
            MembershipHistory::SmallNetwork { tip },
        ));
        let ce = endpoint(&c).await;
        let mut be = endpoint(&b).await;
        let mut previous = b.status().applied_count;
        let mut finished = None;
        let mut resumed = false;
        for attempt in 0..16 {
            let (client, server) = connect(&be, &ce, REPLICA_ALPN).await;
            // Use the ordinary operation budget. Only a later normal contact,
            // with its actual local Have, can continue after an interrupted run.
            let (result, served) = tokio::join!(
                b.sync_replica(&client),
                c.serve_replica_fixture(&server, &frozen),
            );
            client.close(0u32.into(), b"contact ended");
            server.close(0u32.into(), b"contact ended");
            match result {
                Ok(outcome) => {
                    served.unwrap();
                    assert!(outcome.complete());
                    assert_eq!(outcome.scope.len(), frozen.len());
                    assert!(
                        outcome.reused > 2,
                        "normal retry must advertise the durable prefix"
                    );
                    assert_eq!(outcome.received + outcome.reused, frozen.len());
                    finished = Some(outcome);
                    break;
                }
                Err(ReplicaError::Transport) => {
                    assert!(served.is_err());
                    let current = b.status().applied_count;
                    assert!(
                        current > previous,
                        "contact {attempt} repeated a prefix without saving more validated history"
                    );
                    assert!((current as usize) < frozen.len());
                    eprintln!(
                        "replica normal-window progress attempt={attempt} before={previous} after={current} total={}",
                        frozen.len()
                    );
                    previous = current;
                    if !resumed {
                        // A reopened node must derive the same trusted cursor
                        // from its durable records, without an in-memory token.
                        be.close().await;
                        b.close().await;
                        b = NativeNetwork::open(&bp).await.unwrap();
                        let restored = b.status().applied_count;
                        assert!(restored >= current);
                        assert_eq!(
                            b.management_history().await.unwrap(),
                            frozen[..restored as usize],
                            "recovery retains each original in the durable validated prefix"
                        );
                        previous = restored;
                        be = endpoint(&b).await;
                        resumed = true;
                    }
                }
                Err(other) => panic!("valid signed history rejected: {other:?}"),
            }
        }
        assert!(
            resumed,
            "the large first fetch must exercise a normal interrupted window"
        );
        assert!(
            finished.is_some(),
            "validated checkpoints must reach the frozen history's final receipt"
        );
        assert_eq!(b.management_history().await.unwrap(), frozen);
        be.close().await;
        ce.close().await;
        b.close().await;
        c.close().await;
    }
}
