//! Public node replicas over real admitted iroh TLS/QUIC connections.
use std::net::Ipv4Addr;
use std::time::Duration;

use iroh::{Endpoint, EndpointAddr, endpoint::Connection};
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
    let (client, server) = connect(&ne, &ce, ADMISSION_REQUEST_ALPN).await;
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

async fn endpoint(node: &NativeNetwork) -> Endpoint {
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
