//! Narrow SDK story support checks. These are not public Android/WASM or
//! common AdmissionRequest/Replica wire qualification.
use iroh::{Endpoint, EndpointAddr, endpoint::Connection};
use miopunch_native_probe::{
    admission_gate::{ADMISSION_REQUEST_ALPN, ProtocolScope},
    daily_management::ManagementAction,
    manual_admission::{AdmissionStage, ManualAdmissionError, ManualExchange},
    native_network::NativeNetwork,
    resources::{LanDiscovery, PortMapping, ResourceUse},
};
use std::{
    net::Ipv4Addr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

async fn local_endpoint(node: &NativeNetwork) -> Endpoint {
    let mut local = node.resource_configuration().unwrap();
    local.import_trusted_replacement(Vec::new()).unwrap();
    local.mqtt_facilities.clear();
    local.disabled_uses.extend([
        ResourceUse::Observation,
        ResourceUse::DataRelay,
        ResourceUse::Signaling,
        ResourceUse::Lookup,
    ]);
    local.lan_discovery = LanDiscovery::Disabled;
    local.port_mapping = PortMapping::Disabled;
    local.udp_direct_disabled = false;
    local.tcp_direct_disabled = true;
    local.member_data_relay_disabled = true;
    node.save_resource_configuration(&local).unwrap();
    let endpoint = node.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    endpoint.set_alpns(vec![
        ADMISSION_REQUEST_ALPN.to_vec(),
        miopunch_native_probe::admission_gate::PRECODE_REQUEST_ALPN.to_vec(),
    ]);
    endpoint
}

async fn connect(client: &Endpoint, server: &Endpoint) -> (Connection, Connection) {
    connect_alpn(client, server, ADMISSION_REQUEST_ALPN).await
}

async fn connect_alpn(
    client: &Endpoint,
    server: &Endpoint,
    alpn: &[u8],
) -> (Connection, Connection) {
    let address = server
        .addr()
        .ip_addrs()
        .find(|address| address.is_ipv4())
        .copied()
        .unwrap();
    let deadline = n0_future::time::Instant::now() + Duration::from_secs(3);
    tokio::time::timeout(Duration::from_secs(3), async {
        let (out, incoming) = tokio::join!(
            async {
                client
                    .connect_with_opts(
                        EndpointAddr::new(server.id()).with_ip_addr(address),
                        alpn,
                        iroh::endpoint::ConnectOptions::new().with_caller_deadline(deadline),
                    )
                    .await
                    .unwrap()
                    .await
            },
            async {
                server
                    .accept()
                    .await
                    .unwrap()
                    .with_caller_deadline(deadline)
                    .await
            }
        );
        (out.unwrap(), incoming.unwrap())
    })
    .await
    .expect("bounded owned loopback TLS")
}

async fn approve(
    admin: &mut NativeNetwork,
    joining: &mut NativeNetwork,
    ae: &Endpoint,
    je: &Endpoint,
) {
    let (client, server) = connect(je, ae).await;
    let (response, sent) = tokio::join!(
        joining.request_manual_admission(&client),
        admin.serve_manual_admission(&server)
    );
    assert!(matches!(
        response.unwrap(),
        ManualExchange::AwaitingApproval
    ));
    sent.unwrap();
    let id = joining.status().node_id;
    let requests = admin.manual_requests().await.unwrap();
    let exact: Vec<_> = requests
        .iter()
        .filter(|request| request.applicant == id)
        .collect();
    assert_eq!(exact.len(), 1);
    admin.approve_manual_admission(exact[0]).await.unwrap();
    client.close(0u32.into(), b"owned initial request ended");
    server.close(0u32.into(), b"owned initial request ended");
}

async fn complete(
    admin: &mut NativeNetwork,
    joining: &mut NativeNetwork,
    ae: &Endpoint,
    je: &Endpoint,
) {
    let (client, server) = connect(je, ae).await;
    let receive = async {
        let ManualExchange::Materials(delivery) =
            joining.request_manual_admission(&client).await.unwrap()
        else {
            panic!("approved material delivery")
        };
        joining.complete_manual_admission(delivery).await.unwrap()
    };
    let (received, sent) = tokio::join!(receive, admin.serve_manual_admission(&server));
    assert_eq!(received, AdmissionStage::Complete);
    assert_eq!(sent.unwrap(), AdmissionStage::Complete);
    client.close(0u32.into(), b"owned completion ended");
    server.close(0u32.into(), b"owned completion ended");
}

#[tokio::test]
async fn sdk_offline_author_record_survives_author_close_and_provider_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let provider_path = directory.path().join("provider");
    let mut provider = NativeNetwork::create(&provider_path).await.unwrap();
    let mut author = NativeNetwork::join(
        directory.path().join("independent-author"),
        provider.manual_invitation().unwrap(),
    )
    .await
    .unwrap();
    let author_id = author.status().node_id;
    assert_ne!(provider.status().node_id, author_id);
    let pe = local_endpoint(&provider).await;
    let ae = local_endpoint(&author).await;
    approve(&mut provider, &mut author, &pe, &ae).await;
    complete(&mut provider, &mut author, &pe, &ae).await;
    let mut session = author.begin_management_session();
    let authored = author
        .manage(&mut session, ManagementAction::RemoveMember(author_id))
        .await
        .unwrap();
    session.end();
    let record = author
        .management_record(authored.record)
        .await
        .unwrap()
        .unwrap();
    let decoded = p2panda_core::Header::<miopunch_native_probe::management_auth::ManagementExtension>::decode(record.header()).unwrap();
    assert_eq!(decoded.verifying_key, author_id);
    assert!(!record.header().is_empty());
    assert!(
        record.body().is_none(),
        "header-only record is explicit, not a payload-body qualification"
    );
    assert_eq!(
        provider
            .ingest_management_record(record.header(), record.body())
            .await
            .unwrap(),
        authored.record
    );
    assert!(
        !provider
            .status()
            .members
            .iter()
            .any(|(id, _)| *id == author_id)
    );
    ae.close().await;
    author.close().await;
    pe.close().await;
    provider.close().await;
    let provider = NativeNetwork::open(provider_path).await.unwrap();
    let reopened = provider
        .management_record(authored.record)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.header(), reopened.header());
    assert_eq!(record.body(), reopened.body());
    assert!(
        !provider
            .status()
            .members
            .iter()
            .any(|(id, _)| *id == author_id)
    );
    provider.close().await;
}

#[tokio::test]
async fn removed_authorization_only_grant_cannot_resume_after_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let applicant_path = directory.path().join("applicant");
    let mut provider = NativeNetwork::create(directory.path().join("provider"))
        .await
        .unwrap();
    let mut applicant = NativeNetwork::join(&applicant_path, provider.manual_invitation().unwrap())
        .await
        .unwrap();
    let applicant_id = applicant.status().node_id;
    let pe = local_endpoint(&provider).await;
    let ae = local_endpoint(&applicant).await;
    approve(&mut provider, &mut applicant, &pe, &ae).await;
    let request = provider.manual_requests().await.unwrap().pop().unwrap();
    let original = provider
        .approved_admission(&request)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(original.stage, AdmissionStage::Approved);
    let (client, server) = connect(&ae, &pe).await;
    let receive = async {
        let ManualExchange::Materials(delivery) =
            applicant.request_manual_admission(&client).await.unwrap()
        else {
            panic!("authorization-only delivery")
        };
        drop(delivery);
        assert_eq!(
            applicant.admission_stage(),
            AdmissionStage::PendingMaterials
        );
        client.close(0u32.into(), b"deliberate authorization-only interruption");
    };
    let (_, sent) = tokio::join!(receive, provider.serve_manual_admission(&server));
    assert!(sent.is_err());
    ae.close().await;
    applicant.close().await;
    let mut session = provider.begin_management_session();
    let removed = provider
        .manage(&mut session, ManagementAction::RemoveMember(applicant_id))
        .await
        .unwrap();
    session.end();
    assert!(removed.effective_access.is_none());
    let before = provider.status().applied_count;
    let mut applicant = NativeNetwork::open(&applicant_path).await.unwrap();
    assert_eq!(applicant.status().node_id, applicant_id);
    assert_eq!(
        applicant.admission_stage(),
        AdmissionStage::PendingMaterials
    );
    let ae = local_endpoint(&applicant).await;
    let (client, server) = connect(&ae, &pe).await;
    let (received, sent) = tokio::join!(
        applicant.request_manual_admission(&client),
        provider.serve_manual_admission(&server)
    );
    assert!(
        received.is_err() && sent.is_err(),
        "old invalidated request must not restore grant"
    );
    assert_eq!(provider.status().applied_count, before);
    assert!(
        !provider
            .status()
            .members
            .iter()
            .any(|(id, _)| *id == applicant_id)
    );
    let old = provider.approved_admission(&request).await;
    assert!(
        matches!(old, Err(ManualAdmissionError::Rejected)),
        "the public read boundary must reject the invalidated original grant"
    );
    ae.close().await;
    pe.close().await;
    applicant.close().await;
    provider.close().await;
}

#[tokio::test]
async fn interrupted_single_precode_keeps_same_grant_and_exhausts_other_holder() {
    let directory = tempfile::tempdir().unwrap();
    let provider_path = directory.path().join("provider");
    let applicant_path = directory.path().join("applicant");
    let mut provider = NativeNetwork::create(&provider_path).await.unwrap();
    let code = provider
        .issue_precode(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600,
            Some(1),
        )
        .await
        .unwrap();
    let mut applicant = NativeNetwork::join(&applicant_path, code.invitation())
        .await
        .unwrap();
    let original_id = applicant.status().node_id;
    let pe = local_endpoint(&provider).await;
    let ae = local_endpoint(&applicant).await;
    let (client, server) = connect_alpn(
        &ae,
        &pe,
        miopunch_native_probe::admission_gate::PRECODE_REQUEST_ALPN,
    )
    .await;
    let receive = async {
        let ManualExchange::Materials(delivery) = applicant
            .request_precode_admission(&client, &code)
            .await
            .unwrap()
        else {
            panic!("precode material delivery")
        };
        drop(delivery);
        client.close(0u32.into(), b"precode authorization-only interruption");
    };
    let (_, sent) = tokio::join!(receive, provider.serve_precode_admission(&server));
    assert!(sent.is_err());
    let original = provider
        .precode_redemptions(&code)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(provider.precode_usage(&code).await.unwrap(), 1);
    ae.close().await;
    pe.close().await;
    applicant.close().await;
    provider.close().await;
    let mut provider = NativeNetwork::open(&provider_path).await.unwrap();
    let mut applicant = NativeNetwork::open(&applicant_path).await.unwrap();
    assert_eq!(applicant.status().node_id, original_id);
    let pe = local_endpoint(&provider).await;
    let ae = local_endpoint(&applicant).await;
    let (client, server) = connect_alpn(
        &ae,
        &pe,
        miopunch_native_probe::admission_gate::PRECODE_REQUEST_ALPN,
    )
    .await;
    let receive = async {
        let ManualExchange::Materials(delivery) = applicant
            .request_precode_admission(&client, &code)
            .await
            .unwrap()
        else {
            panic!("same precode grant")
        };
        applicant.complete_manual_admission(delivery).await.unwrap()
    };
    let (received, sent) = tokio::join!(receive, provider.serve_precode_admission(&server));
    assert_eq!(received, AdmissionStage::Complete);
    assert_eq!(sent.unwrap(), AdmissionStage::Complete);
    let resumed = provider
        .precode_redemptions(&code)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(resumed.request, original.request);
    assert_eq!(resumed.authorization, original.authorization);
    assert_eq!(provider.precode_usage(&code).await.unwrap(), 1);
    assert_eq!(provider.status().applied_count, 2);
    client.close(0u32.into(), b"owned precode completion");
    server.close(0u32.into(), b"owned precode completion");
    let mut other = NativeNetwork::join(directory.path().join("other-holder"), code.invitation())
        .await
        .unwrap();
    let oe = local_endpoint(&other).await;
    let (client, server) = connect_alpn(
        &oe,
        &pe,
        miopunch_native_probe::admission_gate::PRECODE_REQUEST_ALPN,
    )
    .await;
    let (received, sent) = tokio::join!(
        other.request_precode_admission(&client, &code),
        provider.serve_precode_admission(&server)
    );
    assert!(received.is_err() && sent.is_err());
    assert_eq!(provider.precode_usage(&code).await.unwrap(), 1);
    assert_eq!(provider.precode_redemptions(&code).await.unwrap().len(), 1);
    oe.close().await;
    ae.close().await;
    pe.close().await;
    other.close().await;
    applicant.close().await;
    provider.close().await;
}

// Migration acceptance intentionally checks the actual old SDK wire after a
// genuine nonempty, verified and durable transfer. No new purpose/API is needed
// to compile the RED baseline; changing only an ALPN alias is not final GREEN.
#[tokio::test]
async fn nonempty_durable_replica_requires_strict_new_purpose_alpn() {
    let directory = tempfile::tempdir().unwrap();
    let provider_path = directory.path().join("replica-provider");
    let receiver_path = directory.path().join("replica-receiver");
    let mut provider = NativeNetwork::create(&provider_path).await.unwrap();
    let provider_id = provider.status().node_id;
    let mut receiver = NativeNetwork::join(&receiver_path, provider.manual_invitation().unwrap())
        .await
        .unwrap();
    let receiver_id = receiver.status().node_id;
    assert!(
        provider_id != receiver_id,
        "provider and receiver identities must differ"
    );
    let pe = local_endpoint(&provider).await;
    let re = local_endpoint(&receiver).await;
    approve(&mut provider, &mut receiver, &pe, &re).await;
    complete(&mut provider, &mut receiver, &pe, &re).await;

    // Receiver is disconnected before the independently identified SDK author
    // joins and authors its public self-removal. No private signer/key copy.
    re.close().await;
    receiver.close().await;
    let mut author = NativeNetwork::join(
        directory.path().join("replica-offline-author"),
        provider.manual_invitation().unwrap(),
    )
    .await
    .unwrap();
    let author_id = author.status().node_id;
    assert!(
        author_id != provider_id && author_id != receiver_id,
        "the offline SDK author must have its own identity"
    );
    let ae = local_endpoint(&author).await;
    approve(&mut provider, &mut author, &pe, &ae).await;
    complete(&mut provider, &mut author, &pe, &ae).await;
    let mut session = author.begin_management_session();
    let result = author
        .manage(&mut session, ManagementAction::RemoveMember(author_id))
        .await
        .unwrap();
    session.end();
    let record = author
        .management_record(result.record)
        .await
        .unwrap()
        .unwrap();
    assert!(!record.header().is_empty());
    assert!(
        record.body().is_none(),
        "nonempty signed record, not a payload-body qualification"
    );
    let decoded = p2panda_core::Header::<miopunch_native_probe::management_auth::ManagementExtension>
        ::decode(record.header()).unwrap();
    assert!(
        decoded.verifying_key == author_id,
        "record was not signed by the SDK author"
    );
    let target = provider
        .ingest_management_record(record.header(), record.body())
        .await
        .unwrap();
    assert_eq!(target, result.record);
    ae.close().await;
    author.close().await;
    pe.close().await;
    provider.close().await;

    // Both material stores and independent identities genuinely reopen before
    // fresh replica M. The target record is still absent at the consumer.
    let provider = NativeNetwork::open(&provider_path).await.unwrap();
    let mut receiver = NativeNetwork::open(&receiver_path).await.unwrap();
    assert!(
        provider.status().node_id == provider_id,
        "provider identity changed on reopen"
    );
    assert!(
        receiver.status().node_id == receiver_id,
        "receiver identity changed on reopen"
    );
    assert_eq!(receiver.admission_stage(), AdmissionStage::Complete);
    assert!(receiver.management_record(target).await.unwrap().is_none());
    let alpn = miopunch_native_probe::offline_replica::REPLICA_ALPN;
    provider
        .admission_gate()
        .register_protocol(alpn, ProtocolScope::Member);
    receiver
        .admission_gate()
        .register_protocol(alpn, ProtocolScope::Member);
    let pe = local_endpoint(&provider).await;
    let re = local_endpoint(&receiver).await;
    pe.set_alpns(vec![alpn.to_vec()]);
    re.set_alpns(vec![alpn.to_vec()]);
    let address = pe
        .addr()
        .ip_addrs()
        .find(|address| address.is_ipv4())
        .copied()
        .unwrap();
    // Keep the same existing three-second owned test ceiling through connect,
    // accept and the fresh provider/consumer exchange; never restart its clock.
    let deadline = n0_future::time::Instant::now() + Duration::from_secs(3);
    let cancellation = tokio_util::sync::CancellationToken::new();
    let options = iroh::endpoint::ConnectOptions::new()
        .with_caller_deadline(deadline)
        .with_attempt_cancellation(cancellation.clone());
    let (client, server) = tokio::time::timeout(
        deadline.saturating_duration_since(n0_future::time::Instant::now()),
        async {
            tokio::join!(
                async {
                    re.connect_with_opts(
                        EndpointAddr::new(pe.id()).with_ip_addr(address),
                        alpn,
                        options,
                    )
                    .await
                    .unwrap()
                    .await
                    .unwrap()
                },
                async {
                    pe.accept()
                        .await
                        .unwrap()
                        .with_caller_deadline(deadline)
                        .await
                        .unwrap()
                }
            )
        },
    )
    .await
    .expect("original bounded replica connect/accept");
    assert!(
        receiver
            .admission_gate()
            .allows(&client, ProtocolScope::Member)
    );
    assert!(
        provider
            .admission_gate()
            .allows(&server, ProtocolScope::Member)
    );
    let actual_alpn = String::from_utf8_lossy(client.alpn()).into_owned();
    assert_eq!(client.alpn(), server.alpn());
    let original = client.stable_id();
    let mut binding = [0; 32];
    client
        .export_keying_material(&mut binding, b"miopunch/replica-session-observation/1", &[])
        .unwrap();
    let (received, sent) = tokio::time::timeout(
        deadline.saturating_duration_since(n0_future::time::Instant::now()),
        async {
            tokio::join!(
                receiver.sync_replica_with_deadline(&client, deadline),
                provider.serve_replica_with_deadline(&server, deadline)
            )
        },
    )
    .await
    .expect("same original replica operation deadline");
    let received = received.unwrap();
    sent.unwrap(); // actual provider accepted the verified durable receipt
    assert!(received.complete());
    assert!(received.scope.contains(&target));
    assert!(received.saved.contains(&target));
    assert!(received.applied.contains(&target));
    assert_eq!(client.stable_id(), original);
    let mut after_binding = [0; 32];
    client
        .export_keying_material(
            &mut after_binding,
            b"miopunch/replica-session-observation/1",
            &[],
        )
        .unwrap();
    assert!(binding == after_binding, "original replica binding changed");
    let saved = receiver.management_record(target).await.unwrap().unwrap();
    assert!(
        saved.header() == record.header() && saved.body() == record.body(),
        "verified saved replica record differs from the SDK author's public record"
    );
    client.close(0u32.into(), b"owned replica exchange completed");
    server.close(0u32.into(), b"owned replica exchange completed");
    re.close().await;
    pe.close().await;
    receiver.close().await;
    provider.close().await;
    let receiver = NativeNetwork::open(&receiver_path).await.unwrap();
    assert!(
        receiver.status().node_id == receiver_id,
        "receiver identity changed after replica reopen"
    );
    assert_eq!(receiver.admission_stage(), AdmissionStage::Complete);
    let reopened = receiver.management_record(target).await.unwrap().unwrap();
    assert!(
        reopened.header() == record.header() && reopened.body() == record.body(),
        "replica record did not survive SDK reopen"
    );
    receiver.close().await;

    println!(
        "{}",
        serde_json::json!({"event":"replica-old-wire-observation",
        "actualNegotiatedAlpn":actual_alpn,"expectedStrictPurposeAlpn":"miopunch/session/replica/1",
        "independentIdentities":true,"persistedIdentitiesReopened":true,
        "offlineAuthorClosedBeforeSync":true,"targetAbsentBefore":true,
        "scopeContainsTarget":true,"savedContainsTarget":true,"appliedContainsTarget":true,
        "complete":received.complete(),"senderDurableReceiptAccepted":true,
        "targetSavedAfterReopen":true,"serializedRecordBytes":record.header().len(),
        "bodyPresent":false,"stableUnchanged":true,"bindingUnchanged":true,
        "clientStableId":original,"serverStableId":server.stable_id(),
        "commonPurposeQualification":"raw unified-path-ready events matched by driver","newWireQualified":false})
    );
    assert_eq!(
        actual_alpn, "miopunch/session/replica/1",
        "actual SDK durable replica still negotiated the old normal replica purpose"
    );
}
