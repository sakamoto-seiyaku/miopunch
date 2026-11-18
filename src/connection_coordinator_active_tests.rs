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
    production_native: bool,
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
            production_native: false,
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
            if self.production_native {
                let mut candidate = self
                    .inner
                    .prepare_active(_endpoint, original, control, _attempt, plan)
                    .await?;
                candidate.check(original, true)?;
                self.real_ack.store(true, Ordering::Release);
                let rollback = self.rollback.clone();
                let native_rollback = candidate.rollback.take();
                candidate.rollback = Some(Box::new(move || {
                    if let Some(native_rollback) = native_rollback {
                        native_rollback();
                    }
                    rollback.store(true, Ordering::Release);
                }));
                if self.fault == Some(Fault::PreCommitControlClose) {
                    control.close(0u32.into(), b"pre-commit control close fault");
                }
                return Ok(candidate);
            }
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

async fn production_native_active_scenario(udp_enabled: bool, reverse: bool) {
    let (_directory, a, b) = admitted_pair().await;
    let ae = Arc::new(
        a.bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap(),
    );
    let be = Arc::new(
        b.bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap(),
    );
    let (original, remote) = ordinary_pair(&ae, &be).await;
    let (a, b, ae, be, original, remote) = if reverse {
        (b, a, be, ae, remote, original)
    } else {
        (a, b, ae, be, original, remote)
    };
    let stable = original.stable_id();
    let binding = crate::connection_recovery_core::binding(&original).unwrap();
    let (mut send, mut recv) = original.open_bi().await.unwrap();
    send.write_all(b"before").await.unwrap();
    let (mut peer_send, mut peer_recv) = remote.accept_bi().await.unwrap();
    let mut before = [0; 6];
    peer_recv.read_exact(&mut before).await.unwrap();
    assert_eq!(&before, b"before");

    // Match a real application: it keeps accepting new business streams while
    // maintenance negotiates. Native control bytes must never reach this loop.
    let application_connection = remote.clone();
    let application = tokio::spawn(async move {
        while let Ok((mut send, mut recv)) = application_connection.accept_bi().await {
            let Ok(bytes) = recv.read_to_end(8).await else {
                application_connection.close(0u32.into(), b"control reached business handler");
                break;
            };
            if bytes.len() != 8 {
                break;
            }
            if send.write_all(&bytes).await.is_err() {
                break;
            }
            let _ = send.finish();
        }
    });

    let handler = b.active_improvement_handler(be.clone());
    let router = iroh::protocol::RestrictedRouter::builder(be.endpoint().clone())
        .accept_control(ACTIVE_IMPROVEMENT_ALPN, handler)
        .spawn();
    a.udp_connector().set_udp_enabled(udp_enabled);
    b.udp_connector().set_udp_enabled(udp_enabled);
    let coordinator = ae.active_coordinator(a.admission_gate());
    let (deadline, quota) = ae.begin_contact_attempt(original.remote_id()).unwrap();
    let attempt = AttemptContext::new(
        deadline.min(n0_future::time::Instant::now() + Duration::from_secs(5)),
        tokio_util::sync::CancellationToken::new(),
    );
    let lease = a
        .admission_gate()
        .maintenance()
        .acquire(&original, &attempt, quota)
        .unwrap();
    let improving = tokio::time::timeout(
        Duration::from_secs(7),
        lease.run(coordinator.improve(ae.endpoint(), &original, be.addr(), attempt)),
    );
    let business = tokio::time::timeout(Duration::from_secs(3), async {
        for _ in 0..8 {
            let (mut send, mut recv) = original.open_bi().await?;
            send.write_all(b"business").await?;
            send.finish()?;
            let bytes = recv.read_to_end(8).await.map_err(io::Error::other)?;
            if bytes != b"business" {
                return Err(io::Error::other("unexpected business reply"));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok::<_, io::Error>(())
    });
    let (result, business) = tokio::join!(improving, business);
    let same_original = original.stable_id() == stable
        && crate::connection_recovery_core::binding(&original)
            .is_ok_and(|actual| actual == binding);
    let continued = tokio::time::timeout(Duration::from_secs(2), async {
        send.write_all(b"after").await?;
        let mut after = [0; 5];
        peer_recv
            .read_exact(&mut after)
            .await
            .map_err(io::Error::other)?;
        peer_send.write_all(&after).await?;
        recv.read_exact(&mut after)
            .await
            .map_err(io::Error::other)?;
        Ok::<_, io::Error>(after == *b"after")
    })
    .await;
    original.close(0u32.into(), b"native active regression complete");
    remote.close(0u32.into(), b"native active regression complete");
    application.await.unwrap();
    drop((
        send,
        recv,
        peer_send,
        peer_recv,
        original,
        remote,
        coordinator,
    ));
    router.shutdown().await.unwrap();
    drop(router);
    let ae = Arc::try_unwrap(ae)
        .ok()
        .expect("initiator resource owner released");
    let be = Arc::try_unwrap(be)
        .ok()
        .expect("responder handler released");
    tokio::join!(ae.close(), be.close());
    tokio::join!(a.close(), b.close());
    if udp_enabled {
        assert!(
            matches!(result, Ok(Ok(Ok(())))),
            "production native active: {result:?}"
        );
    } else {
        assert!(
            matches!(result, Ok(Ok(Err(_)))),
            "disabled UDP active: {result:?}"
        );
    }
    assert!(same_original);
    assert!(
        matches!(business, Ok(Ok(()))),
        "concurrent business: {business:?}"
    );
    assert!(
        matches!(continued, Ok(Ok(true))),
        "original stream: {continued:?}"
    );
}

#[tokio::test]
async fn production_native_active_prepares_udp_on_the_original_member() {
    production_native_active_scenario(true, false).await;
}

#[tokio::test]
async fn production_native_active_respects_udp_disable_and_keeps_business() {
    production_native_active_scenario(false, false).await;
}

#[tokio::test]
async fn production_native_active_from_accepting_side_keeps_concurrent_business() {
    production_native_active_scenario(true, true).await;
}

#[tokio::test]
async fn qualified_original_can_own_traversal_but_ordinary_connection_cannot() {
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
    let mut traversal = ae
        .endpoint()
        .control_handle(&original)
        .register_udp_traversal([61; 16])
        .await;
    let mut retained = None;
    if let Ok(lease) = &mut traversal {
        lease.listen_random_ports(1).await.unwrap();
        let local = lease
            .local_sockets()
            .into_iter()
            .find(|socket| !socket.id.is_primary())
            .unwrap();
        retained = Some(
            lease
                .retain_winner(
                    local.id,
                    std::net::SocketAddr::from((Ipv4Addr::LOCALHOST, local.local_addr.port())),
                )
                .await,
        );
    }
    let retained_ok = matches!(&retained, Some(Ok(Some(_))));
    if let Some(Ok(Some(winner))) = &mut retained {
        assert!(
            winner.commit(&remote).is_err(),
            "winner belongs only to its original M"
        );
    }

    let builder = || {
        iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
            .clear_relay_transports()
            .bind_addr(std::net::SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .unwrap()
            .alpns(vec![b"ordinary-traversal-boundary".to_vec()])
    };
    let plain_a = builder().bind().await.unwrap();
    let plain_b = builder().bind().await.unwrap();
    let (plain, peer) = tokio::join!(
        plain_a.connect(plain_b.addr(), b"ordinary-traversal-boundary"),
        async { plain_b.accept().await.unwrap().await },
    );
    let (plain, peer) = (plain.unwrap(), peer.unwrap());
    let denied = plain_a
        .register_udp_traversal(&plain, [62; 16])
        .await
        .is_err();
    plain.close(0u32.into(), b"boundary regression done");
    peer.close(0u32.into(), b"boundary regression done");
    original.close(0u32.into(), b"boundary regression done");
    remote.close(0u32.into(), b"boundary regression done");
    drop((retained, plain, peer, original, remote));
    let allowed = traversal.is_ok();
    drop(traversal);
    tokio::join!(plain_a.close(), plain_b.close(), ae.close(), be.close());
    tokio::join!(a.close(), b.close());
    assert!(
        denied,
        "ordinary unqualified endpoints cannot gain traversal authority"
    );
    assert!(
        allowed && retained_ok,
        "qualified M needs connection-owned traversal and winner retention"
    );
}

async fn scenario(fault: Fault) {
    scenario_with_native(fault, false).await;
}

async fn scenario_with_native(fault: Fault, production_native: bool) {
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
    let mut initiator = ProbeAdapter::new(a.admission_gate(), None, None);
    let mut responder = ProbeAdapter::new(b_gate, Some(fault), Some(b.clone()));
    if production_native {
        initiator.inner = NativeControlAdapter(ae.active_coordinator(initiator.gate.clone()));
        responder.inner = NativeControlAdapter(be.active_coordinator(responder.gate.clone()));
        initiator.production_native = true;
        responder.production_native = true;
    }
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
async fn production_native_active_rollback_preserves_existing_udp_and_other_streams() {
    tokio::time::timeout(
        Duration::from_secs(12),
        scenario_with_native(Fault::PreCommitControlClose, true),
    )
    .await
    .expect("native rollback must preserve existing business paths");
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

async fn cold_admission_with_blackhole(include_bad_candidate: bool) -> &'static str {
    let directory = tempfile::tempdir().unwrap();
    let admin = NativeNetwork::create(directory.path().join("admin"))
        .await
        .unwrap();
    let member = NativeNetwork::join(
        directory.path().join("member"),
        admin.manual_invitation().unwrap(),
    )
    .await
    .unwrap();
    for node in [&admin, &member] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(vec![]);
        node.save_resource_configuration(&config).unwrap();
    }
    let server = admin
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let client = member
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    // Keep an owned UDP socket open so the first route silently drops QUIC
    // packets instead of returning an immediate closed-port error.
    let blackhole = tokio::net::UdpSocket::bind((Ipv4Addr::new(127, 0, 0, 0), 0))
        .await
        .unwrap();
    let bad = blackhole.local_addr().unwrap();
    let advertised = server.addr();
    let good = *advertised
        .ip_addrs()
        .find(|address| address.ip() == std::net::IpAddr::V4(Ipv4Addr::LOCALHOST))
        .expect("real server loopback route");
    let target = if include_bad_candidate {
        iroh::EndpointAddr::new(advertised.id)
            .with_addrs([iroh::TransportAddr::Ip(bad), iroh::TransportAddr::Ip(good)])
    } else {
        iroh::EndpointAddr::new(advertised.id).with_addrs([iroh::TransportAddr::Ip(good)])
    };
    if include_bad_candidate {
        assert_eq!(
            target.addrs.iter().next(),
            Some(&iroh::TransportAddr::Ip(bad))
        );
    }

    let deadline = n0_future::time::Instant::now() + Duration::from_secs(5);
    let cancellation = tokio_util::sync::CancellationToken::new();
    let caller = client.coordinator();
    let responder = server.coordinator();
    let result = tokio::time::timeout(Duration::from_secs(7), async {
        tokio::join!(
            caller.connect_control(
                target,
                SessionPurpose::AdmissionRequest,
                AttemptContext::new(deadline, cancellation.clone()),
            ),
            async {
                let incoming = server.endpoint().accept().await.unwrap();
                responder
                    .accept_control(
                        incoming,
                        SessionPurpose::AdmissionRequest,
                        AttemptContext::new(deadline, cancellation),
                    )
                    .await
            }
        )
    })
    .await;
    let outcome = match result {
        Ok((Ok(outgoing), Ok(incoming))) => {
            outgoing.close(0u32.into(), b"owned route test ended");
            incoming.close(0u32.into(), b"owned route test ended");
            "connected"
        }
        Ok((Err(_), _)) => "caller_failed",
        Ok((_, Err(_))) => "responder_failed",
        Err(_) => "scenario_timeout",
    };
    tokio::join!(client.close(), server.close());
    drop(blackhole);
    outcome
}

#[tokio::test]
async fn cold_admission_reaches_the_only_advertised_local_ip() {
    assert_eq!(cold_admission_with_blackhole(false).await, "connected");
}

#[tokio::test]
async fn cold_admission_uses_reachable_ip_after_a_blackholed_first_candidate() {
    assert_eq!(
        cold_admission_with_blackhole(true).await,
        "connected",
        "the working route must share the original deadline"
    );
}
