//! Common active-commit regression on admitted loopback Native SDK members.
//! Uses actual protected M/C and fresh Noq IP-path ACKs; no RTC/WAN claim.
use super::*;
use crate::{
    admission_gate::ProtocolScope, manual_admission::ManualExchange, native_network::NativeNetwork,
};
use std::{
    net::Ipv4Addr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

async fn admitted_pair() -> (tempfile::TempDir, NativeNetwork, NativeNetwork) {
    let directory = tempfile::tempdir().unwrap();
    let a_path = directory.path().join("a");
    let b_path = directory.path().join("b");
    let mut a = NativeNetwork::create(&a_path).await.unwrap();
    let mut b = NativeNetwork::join(&b_path, a.manual_invitation().unwrap())
        .await
        .unwrap();
    for node in [&a, &b] {
        let mut resources = node.resource_configuration().unwrap();
        resources.replacement = Some(vec![]);
        node.save_resource_configuration(&resources).unwrap();
    }
    let ae = a.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let be = b.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let deadline = n0_future::time::Instant::now() + Duration::from_secs(5);
    let options = iroh::endpoint::ConnectOptions::new().with_caller_deadline(deadline);
    let (client, server) = tokio::join!(
        async {
            be.connect_control_with_opts(ae.addr(), ADMISSION_REQUEST_ALPN, options)
                .await
                .unwrap()
                .await
                .unwrap()
        },
        async {
            ae.accept()
                .await
                .unwrap()
                .with_caller_deadline(deadline)
                .accept_control()
                .unwrap()
                .await
                .unwrap()
        }
    );
    let (client, server) = (Arc::new(client), Arc::new(server));
    let (request, served) = tokio::join!(
        b.request_manual_admission(&client),
        a.serve_manual_admission(&server)
    );
    assert!(matches!(request.unwrap(), ManualExchange::AwaitingApproval));
    served.unwrap();
    let request = a.manual_requests().await.unwrap().pop().unwrap();
    a.approve_manual_admission(&request).await.unwrap();
    let receiver = async {
        let ManualExchange::Materials(material) =
            b.request_manual_admission(&client).await.unwrap()
        else {
            panic!("approved admission material absent")
        };
        b.complete_manual_admission(material).await.unwrap();
    };
    let (_, served) = tokio::join!(receiver, a.serve_manual_admission(&server));
    served.unwrap();
    client.close(0u32.into(), b"admission complete");
    server.close(0u32.into(), b"admission complete");
    tokio::join!(ae.close(), be.close());
    tokio::join!(a.close(), b.close());
    (
        directory,
        NativeNetwork::open(&a_path).await.unwrap(),
        NativeNetwork::open(&b_path).await.unwrap(),
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Fault {
    CompletedControlClose,
    PreCommitControlClose,
    OriginalClose,
    CurrentProofInvalid,
}

struct ProbeAdapter {
    inner: NativeControlAdapter,
    gate: crate::admission_gate::AdmissionGate,
    fault: Option<Fault>,
    peer_committed: AtomicBool,
    reached: tokio::sync::Notify,
    post_check_entered: AtomicBool,
    control: Mutex<Option<iroh::endpoint::WeakControlConnection>>,
    node: Option<Arc<tokio::sync::Mutex<Option<NativeNetwork>>>>,
    rollback: Arc<AtomicBool>,
    real_ack: AtomicBool,
    report: Mutex<Option<serde_json::Value>>,
    stages: Mutex<Vec<&'static str>>,
}
impl ProbeAdapter {
    fn new(
        gate: crate::admission_gate::AdmissionGate,
        fault: Option<Fault>,
        node: Option<Arc<tokio::sync::Mutex<Option<NativeNetwork>>>>,
    ) -> Self {
        Self {
            inner: NativeControlAdapter(CoordinatorHook::new(gate.clone(), true)),
            gate,
            fault,
            peer_committed: AtomicBool::new(false),
            reached: tokio::sync::Notify::new(),
            post_check_entered: AtomicBool::new(false),
            control: Mutex::new(None),
            node,
            rollback: Arc::new(AtomicBool::new(false)),
            real_ack: AtomicBool::new(false),
            report: Mutex::new(None),
            stages: Mutex::new(Vec::new()),
        }
    }
}
impl MemberPathAdapter<iroh::endpoint::ControlHandle> for ProbeAdapter {
    fn capabilities(&self, role: AttemptRole) -> RuntimeCapabilities {
        self.inner.capabilities(role)
    }
    fn authorization_revision(
        &self,
        connection: &iroh::endpoint::ControlHandle,
        purpose: SessionPurpose,
    ) -> io::Result<u64> {
        self.inner.authorization_revision(connection, purpose)
    }
    fn committed(&self, _original: &iroh::endpoint::ControlHandle, report: serde_json::Value) {
        *self.report.lock().unwrap() = Some(report);
    }
    fn active_diagnostic(
        &self,
        _original: &iroh::endpoint::ControlHandle,
        _control: &iroh::endpoint::ControlHandle,
        stage: &'static str,
    ) {
        let mut stages = self.stages.lock().unwrap();
        if stages.len() < 16 {
            stages.push(stage);
        }
        drop(stages);
        if stage == "peer-committed-observed" {
            self.peer_committed.store(true, Ordering::Release);
            self.reached.notify_one();
        }
    }
    fn check<'a>(
        &'a self,
        connection: &'a iroh::endpoint::ControlHandle,
        purpose: SessionPurpose,
    ) -> impl Future<Output = io::Result<()>> + Send + 'a {
        async move {
            if purpose == SessionPurpose::MemberBusiness
                && self.peer_committed.load(Ordering::Acquire)
                && self.fault.is_some()
                && !self.post_check_entered.swap(true, Ordering::AcqRel)
            {
                // Wait for the actual peer's successful common protocol to return
                // and close its exact C. No timer, fake frame or ACK injection.
                let control = self.control.lock().unwrap().clone().unwrap();
                control.closed().await;
                match self.fault.unwrap() {
                    Fault::OriginalClose => connection.close(0u32.into(), b"original close fault"),
                    Fault::CurrentProofInvalid => {
                        // Existing SDK fixture publishes new durable credential material,
                        // invalidating the real proof/revision. It grants no membership.
                        #[cfg(feature = "rtc-probe")]
                        self.node
                            .as_ref()
                            .unwrap()
                            .lock()
                            .await
                            .as_mut()
                            .unwrap()
                            .invalidate_current_credential_fixture()
                            .await
                            .map_err(io::Error::other)?;
                        #[cfg(not(feature = "rtc-probe"))]
                        return Err(control_error("InvalidControl"));
                    }
                    _ => {}
                }
            }
            self.inner.check(connection, purpose).await
        }
    }
    fn prepare<'a>(
        &'a self,
        endpoint: Option<&'a Endpoint>,
        connection: &'a iroh::endpoint::ControlHandle,
        attempt: ProtectedAttempt,
        plan: PathPlan,
    ) -> impl Future<Output = io::Result<PreparedPath>> + Send + 'a {
        self.inner.prepare(endpoint, connection, attempt, plan)
    }
    fn prepare_active<'a>(
        &'a self,
        _endpoint: Option<&'a Endpoint>,
        original: &'a iroh::endpoint::ControlHandle,
        control: &'a iroh::endpoint::ControlHandle,
        _attempt: ProtectedAttempt,
        plan: PathPlan,
    ) -> impl Future<Output = io::Result<PreparedPath>> + Send + 'a {
        async move {
            assert!(matches!(plan, PathPlan::Native { udp: true, .. }));
            *self.control.lock().unwrap() = Some(control.weak_handle());
            let paths = original.paths();
            let path = paths
                .iter()
                .find(|path| path.is_ip() && path.is_selected())
                .ok_or_else(|| control_error("DataPathUnavailable"))?;
            let route = path.remote_addr().clone();
            let path_id = path.id().to_string();
            let probe = path.start_probe()?;
            let qualification = self
                .gate
                .path_qualification(original)
                .ok_or_else(|| control_error("IdentityOrAdmissionRejected"))?;
            while !probe.is_acknowledged().map_err(io::Error::other)? {
                self.inner
                    .check(original, SessionPurpose::MemberBusiness)
                    .await?;
                if n0_future::time::Instant::now() >= control.caller_deadline().unwrap() {
                    return Err(control_error("DeadlineExceeded"));
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            self.real_ack.store(true, Ordering::Release);
            let rollback = self.rollback.clone();
            // This path is already Available from the actual ordinary SDK dial.
            // No selector/retain emulation: exercise common commit ownership with
            // a genuine current connection-bound probe and a rollback observer.
            let candidate = PreparedPath::candidate(original, route, move || {
                if qualification.is_current() {
                    Ok(())
                } else {
                    Err(control_error("IdentityOrAdmissionRejected"))
                }
            })
            .with_proof(path_id, probe.watermark(), move || {
                probe.is_acknowledged().map_err(io::Error::other)
            })
            .with_rollback(move || {
                rollback.store(true, Ordering::Release);
            });
            if self.fault == Some(Fault::PreCommitControlClose) {
                control.close(0u32.into(), b"pre-commit control close fault");
            }
            Ok(candidate)
        }
    }
}

async fn ordinary_pair(
    a: &crate::resources::ResourceEndpoint,
    b: &crate::resources::ResourceEndpoint,
) -> (Connection, Connection) {
    let deadline = n0_future::time::Instant::now() + Duration::from_secs(5);
    let attempt = AttemptContext::new(deadline, tokio_util::sync::CancellationToken::new());
    let (ac, bc) = (a.coordinator(), b.coordinator());
    let (client, server) = tokio::join!(
        ac.connect(b.addr(), SessionPurpose::MemberBusiness, attempt.clone()),
        async {
            bc.accept(
                b.endpoint().accept().await.unwrap(),
                SessionPurpose::MemberBusiness,
                attempt,
            )
            .await
        }
    );
    (client.unwrap(), server.unwrap())
}

async fn scenario(fault: Fault) {
    let (_directory, a, b) = admitted_pair().await;
    let ae = a
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let be = b
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let (original, remote) = ordinary_pair(&ae, &be).await;
    let (other, other_remote) = ordinary_pair(&ae, &be).await;
    assert!(a.admission_gate().allows(&original, ProtocolScope::Member));
    assert!(b.admission_gate().allows(&remote, ProtocolScope::Member));
    let original_stable = original.stable_id();
    let binding = crate::connection_recovery_core::binding(&original).unwrap();
    let (mut stream, mut reply) = other.open_bi().await.unwrap();
    stream.write_all(b"before").await.unwrap();
    let (mut response, mut request) = other_remote.accept_bi().await.unwrap();
    let mut before = [0; 6];
    request.read_exact(&mut before).await.unwrap();
    assert_eq!(&before, b"before");
    response.write_all(b"before").await.unwrap();
    reply.read_exact(&mut before).await.unwrap();
    assert_eq!(&before, b"before");

    let (mut original_stream, mut original_reply) = original.open_bi().await.unwrap();
    original_stream.write_all(b"before").await.unwrap();
    let (mut original_response, mut original_request) = remote.accept_bi().await.unwrap();
    original_request.read_exact(&mut before).await.unwrap();
    assert_eq!(&before, b"before");
    original_response.write_all(b"before").await.unwrap();
    original_reply.read_exact(&mut before).await.unwrap();
    assert_eq!(&before, b"before");

    let b_gate = b.admission_gate();
    let b = Arc::new(tokio::sync::Mutex::new(Some(b)));
    let initiator = ProbeAdapter::new(a.admission_gate(), None, None);
    let responder = ProbeAdapter::new(b_gate, Some(fault), Some(b.clone()));
    let deadline = n0_future::time::Instant::now() + Duration::from_secs(5);
    let cancellation = tokio_util::sync::CancellationToken::new();
    let options = iroh::endpoint::ConnectOptions::new()
        .with_caller_deadline(deadline)
        .with_attempt_cancellation(cancellation.clone());
    let (control_owner, peer_control_owner) = tokio::join!(
        async {
            ae.endpoint()
                .connect_control_with_opts(be.addr(), ACTIVE_IMPROVEMENT_ALPN, options)
                .await
                .unwrap()
                .await
                .unwrap()
        },
        async {
            be.endpoint()
                .accept()
                .await
                .unwrap()
                .with_caller_deadline(deadline)
                .with_attempt_cancellation(cancellation.clone())
                .accept_control()
                .unwrap()
                .await
                .unwrap()
        }
    );
    let control = control_owner.control_handle();
    let peer_control = peer_control_owner.control_handle();
    let registry = responder.gate.maintenance();
    let lookup = registry.clone();
    let attempt = AttemptContext::new(deadline, cancellation);
    let lease = initiator
        .gate
        .maintenance()
        .acquire(&original, &attempt, ())
        .unwrap();
    let (client_result, server_result) = tokio::time::timeout(
        deadline.saturating_duration_since(n0_future::time::Instant::now()),
        async {
            tokio::join!(
                async {
                    let result = lease
                        .run(improve_existing(
                            ae.endpoint(),
                            &original,
                            &control,
                            &initiator,
                            attempt,
                        ))
                        .await;
                    if matches!(&result, Ok(Ok(()))) {
                        // A scheduler-only gate guarantees peer COMMITTED was
                        // read before the successful peer's real C cleanup.
                        responder.reached.notified().await;
                    }
                    control.close(0u32.into(), b"actual local active operation completed");
                    result
                },
                serve_active_session(
                    be.endpoint(),
                    &peer_control,
                    &responder,
                    &registry,
                    deadline,
                    move |peer, binding| lookup.for_binding(peer, binding)
                )
            )
        },
    )
    .await
    .expect("original active deadline bounds the real close gate");
    println!(
        "{}",
        serde_json::json!({"event":"common-active-commit-regression",
        "scope":"admitted Native SDK/IP probe common boundary only; not RTC/WAN/M77 acceptance",
        "fault":format!("{fault:?}"),"realFreshAck":responder.real_ack.load(Ordering::Acquire),
        "peerCommittedObserved":responder.peer_committed.load(Ordering::Acquire),
        "serverStages":*responder.stages.lock().unwrap(),
        "candidateRollbackObserved":responder.rollback.load(Ordering::Acquire),
        "serverOk":server_result.is_ok(),"serverError":server_result.as_ref().err().map(ToString::to_string)})
    );
    let real_ack = responder.real_ack.load(Ordering::Acquire);
    let rolled_back = responder.rollback.load(Ordering::Acquire);
    let responder_reported = responder.report.lock().unwrap().is_some();
    let both_reports_valid = [&initiator, &responder].iter().all(|adapter| {
        adapter
            .report
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|report| {
                report["event"] == "active-improvement-committed"
                    && report["freshAck"] == true
                    && report["bothReady"] == true
            })
    });
    if fault == Fault::CompletedControlClose {
        assert_eq!(original.stable_id(), original_stable);
        assert_eq!(
            crate::connection_recovery_core::binding(&original).unwrap(),
            binding
        );
        assert!(initiator.gate.allows(&original, ProtocolScope::Member));
        assert!(responder.gate.allows(&remote, ProtocolScope::Member));
    }
    if fault != Fault::PreCommitControlClose {
        assert!(responder.peer_committed.load(Ordering::Acquire));
        assert!(responder.post_check_entered.load(Ordering::Acquire));
    }
    if matches!(
        fault,
        Fault::CompletedControlClose | Fault::PreCommitControlClose
    ) {
        original_stream.write_all(b"after").await.unwrap();
        let mut after = [0; 5];
        original_request.read_exact(&mut after).await.unwrap();
        assert_eq!(&after, b"after");
        original_response.write_all(b"after").await.unwrap();
        original_reply.read_exact(&mut after).await.unwrap();
        assert_eq!(&after, b"after");
    }
    // Durable credential invalidation deliberately revokes every old proof on b;
    // continuity is required only while the unrelated M is still authorized.
    if fault != Fault::CurrentProofInvalid {
        stream.write_all(b"after").await.unwrap();
        let mut after = [0; 5];
        request.read_exact(&mut after).await.unwrap();
        assert_eq!(&after, b"after");
        response.write_all(b"after").await.unwrap();
        reply.read_exact(&mut after).await.unwrap();
        assert_eq!(&after, b"after");
        assert!(initiator.gate.allows(&other, ProtocolScope::Member));
        assert!(responder.gate.allows(&other_remote, ProtocolScope::Member));
    } else {
        assert!(!responder.gate.allows(&remote, ProtocolScope::Member));
    }
    control.close(0u32.into(), b"owned common commit regression finished");
    peer_control.close(0u32.into(), b"owned common commit regression finished");
    for connection in [&original, &remote, &other, &other_remote] {
        connection.close(0u32.into(), b"owned common commit regression finished");
    }
    drop((
        stream,
        reply,
        response,
        request,
        original_stream,
        original_reply,
        original_response,
        original_request,
        control,
        peer_control,
        original,
        remote,
        other,
        other_remote,
    ));
    drop((initiator, responder, registry));
    tokio::join!(ae.close(), be.close());
    let b = b.lock().await.take().unwrap();
    tokio::join!(a.close(), b.close());
    // The targeted pre-fix failure is asserted only after normal SDK cleanup.
    assert!(real_ack);
    if fault == Fault::CompletedControlClose {
        assert!(matches!(&client_result, Ok(Ok(()))), "{client_result:?}");
        assert!(
            server_result.is_ok(),
            "completed peer C cleanup cannot undo qualified commit: {server_result:?}"
        );
        assert!(!rolled_back);
        assert!(both_reports_valid);
    } else {
        assert!(server_result.is_err());
        assert!(rolled_back);
        assert!(!responder_reported);
    }
}

#[tokio::test]
async fn completed_peer_control_close_preserves_common_commit_and_other_stream() {
    scenario(Fault::CompletedControlClose).await;
}
#[tokio::test]
async fn pre_commit_control_close_still_rolls_back_candidate_and_preserves_other_stream() {
    scenario(Fault::PreCommitControlClose).await;
}
#[tokio::test]
async fn original_close_after_peer_committed_still_rolls_back_candidate() {
    scenario(Fault::OriginalClose).await;
}
#[cfg(feature = "rtc-probe")]
#[tokio::test]
async fn current_proof_invalid_after_peer_committed_still_rolls_back_candidate() {
    scenario(Fault::CurrentProofInvalid).await;
}
