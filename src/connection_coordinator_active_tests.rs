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

#[tokio::test]
async fn foreground_dials_while_background_contact_is_busy_or_backed_off() {
    use crate::resources::ContactClass::{Background, Foreground};

    let (_directory, a, b) = admitted_pair().await;
    let ae = a
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let be = b
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let id = be.addr().id;
    let (_, previous) = ae.begin_contact_attempt(id, Foreground).unwrap();
    previous.failed("foreground failed".into());
    drop(previous);
    assert!(ae.connect(be.addr(), MEMBER_ALPN).await.is_err());

    let attempt = AttemptContext::new(
        n0_future::time::Instant::now() + Duration::from_secs(5),
        tokio_util::sync::CancellationToken::new(),
    );
    let responder = be.coordinator();
    let accept_member = || async {
        responder
            .accept(
                be.endpoint().accept().await.unwrap(),
                SessionPurpose::MemberBusiness,
                attempt.clone(),
            )
            .await
            .unwrap()
    };
    let (background, incoming) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(
            ae.connect_for_recovery(be.addr(), MEMBER_ALPN, Background),
            accept_member()
        )
    })
    .await
    .unwrap();
    let (background_member, background) = background.unwrap();
    background.succeeded(); // Clear the earlier foreground failure, preserving this guard.
    let (first, duplicate, inbound) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(
            ae.connect(be.addr(), MEMBER_ALPN),
            ae.connect(be.addr(), MEMBER_ALPN),
            accept_member()
        )
    })
    .await
    .unwrap();
    assert!(first.is_ok() && duplicate.is_err());
    assert!(ae.begin_contact_attempt(id, Background).is_err());
    background.failed("later background failure".into());
    drop(background);
    assert!(!ae.contact_ready(id, Background));

    let (foreground_control, inbound_control) =
        tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(
                ae.connect_control(be.addr(), ADMISSION_REQUEST_ALPN),
                async {
                    let control = be.endpoint().accept().await.unwrap();
                    responder
                        .accept_control(control, SessionPurpose::AdmissionRequest, attempt)
                        .await
                }
            )
        })
        .await
        .unwrap();
    foreground_control
        .unwrap()
        .close(0u32.into(), b"foreground control complete");
    inbound_control
        .unwrap()
        .close(0u32.into(), b"foreground control complete");
    for connection in [
        first.unwrap(),
        inbound,
        background_member.business().unwrap(),
        incoming,
    ] {
        connection.close(0u32.into(), b"contact class fixture complete");
    }
    tokio::join!(ae.close(), be.close());
    tokio::join!(a.close(), b.close());
}

async fn production_native_active_scenario(udp_enabled: bool, reverse: bool) {
    use crate::connection_recovery::ConnectionPathRecoveryError;
    use crate::resources::ContactClass;
    use crate::session_maintenance::{Completion, MaintenanceDeferral, Reservation};
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
    let local_registry = a.admission_gate().maintenance();
    let peer_registry = b.admission_gate().maintenance();
    let (deadline, quota) = ae
        .begin_contact_attempt(original.remote_id(), ContactClass::Background)
        .unwrap();
    let attempt = AttemptContext::new(
        deadline.min(n0_future::time::Instant::now() + Duration::from_secs(5)),
        tokio_util::sync::CancellationToken::new(),
    );
    let mut lease = local_registry.acquire(&original, &attempt, ()).unwrap();
    if udp_enabled {
        let peer_lease = peer_registry.acquire(&remote, &attempt, ()).unwrap();
        let paths: Vec<_> = remote.paths().iter().map(|path| path.id()).collect();
        let error = lease
            .run_ref(coordinator.improve(ae.endpoint(), &original, be.addr(), attempt.clone()))
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(
            MaintenanceDeferral::from_error(&error),
            Some(&MaintenanceDeferral::Remote)
        );
        assert_eq!(
            remote
                .paths()
                .iter()
                .map(|path| path.id())
                .collect::<Vec<_>>(),
            paths
        );
        assert!(original.close_reason().is_none() && remote.close_reason().is_none());
        #[cfg(feature = "rtc-probe")]
        {
            let control = ae
                .endpoint()
                .connect_control_with_opts(
                    be.addr(),
                    ACTIVE_IMPROVEMENT_ALPN,
                    iroh::endpoint::ConnectOptions::new().with_caller_deadline(attempt.deadline),
                )
                .await
                .unwrap()
                .await
                .unwrap();
            let valid_caps =
                NativeControlAdapter(coordinator.clone()).capabilities(AttemptRole::Initiator);
            let refused =
                wrong_active_binding_fixture(&original, &control, valid_caps, attempt.deadline)
                    .await
                    .unwrap()
                    .unwrap_err();
            assert!(
                MaintenanceDeferral::from_error(&active_control_error(&control, refused)).is_none()
            );
            assert!(
                matches!(control.closed().await, iroh::endpoint::ConnectionError::ApplicationClosed(close) if close.error_code == 0u32.into())
            );
        }
        drop(peer_lease);
    }
    let (start, started) = tokio::sync::oneshot::channel();
    let improving = async {
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            started.await.unwrap();
            lease
                .run_ref(coordinator.improve(ae.endpoint(), &original, be.addr(), attempt.clone()))
                .await
        })
        .await;
        if let Ok(Ok(Ok(path))) = &result {
            quota.succeeded();
            lease.completed(Completion::Active(path.clone()));
        }
        drop(lease);
        result
    };
    let repairs = async {
        if !udp_enabled {
            start.send(()).unwrap();
            return;
        }
        let expired = coordinator
            .repair(
                &ae,
                &original,
                be.addr(),
                n0_future::time::Instant::now() + Duration::from_millis(30),
                tokio_util::sync::CancellationToken::new(),
            )
            .await;
        assert!(
            matches!(expired, Err(ConnectionPathRecoveryError::Retryable { cause, .. }) if cause.contains("DeadlineExceeded"))
        );
        let cancel = tokio_util::sync::CancellationToken::new();
        let cancelled =
            coordinator.repair(&ae, &original, be.addr(), attempt.deadline, cancel.clone());
        futures_util::pin_mut!(cancelled);
        assert!(futures_util::poll!(cancelled.as_mut()).is_pending());
        cancel.cancel();
        assert!(
            matches!(cancelled.await, Err(ConnectionPathRecoveryError::Retryable { cause, .. }) if cause.contains("Cancelled"))
        );
        assert!(original.close_reason().is_none());
        assert_eq!(local_registry.active_attempts(), 1);
        // A second C could not start while the foreground guard is occupied.
        let _foreground = ae
            .begin_contact_attempt(original.remote_id(), ContactClass::Foreground)
            .unwrap();
        let repaired = coordinator.repair(
            &ae,
            &original,
            be.addr(),
            attempt.deadline,
            tokio_util::sync::CancellationToken::new(),
        );
        futures_util::pin_mut!(repaired);
        assert!(futures_util::poll!(repaired.as_mut()).is_pending());
        start.send(()).unwrap();
        repaired.await.unwrap();
    };
    let peer_completion = async {
        if !udp_enabled {
            return;
        }
        let wait = n0_future::time::timeout(Duration::from_secs(3), async {
            loop {
                if peer_registry.active_attempts() != 0
                    && let Reservation::Pending(wait) = peer_registry
                        .acquire_or_wait(&remote, &attempt, ())
                        .unwrap()
                {
                    break wait;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            matches!(wait.wait().await, Ok(Some(Completion::Active(path))) if path.is_open(&remote))
        );
    };
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
    let (result, business, (), ()) = tokio::join!(improving, business, repairs, peer_completion);
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
            matches!(result, Ok(Ok(Ok(_)))),
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
async fn unmarked_active_control_close_is_not_deferred() {
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
    be.endpoint()
        .set_alpns(vec![ACTIVE_IMPROVEMENT_ALPN.to_vec()]);
    let attempt = AttemptContext::new(
        n0_future::time::Instant::now() + Duration::from_secs(2),
        tokio_util::sync::CancellationToken::new(),
    );
    let coordinator = ae.active_coordinator(a.admission_gate());
    let refuse = async {
        let control = be
            .endpoint()
            .accept()
            .await
            .unwrap()
            .accept_control()
            .unwrap()
            .await
            .unwrap();
        let stream = control.accept_control_exchange().await.unwrap();
        read_active_hello(&stream).await.unwrap();
        control.close(0u32.into(), b"end without active result");
    };
    let (result, ()) = tokio::time::timeout_at(attempt.deadline, async {
        tokio::join!(
            coordinator.repair(
                &ae,
                &original,
                be.addr(),
                attempt.deadline,
                attempt.cancellation
            ),
            refuse
        )
    })
    .await
    .unwrap();
    assert!(
        matches!(result, Err(crate::connection_recovery::ConnectionPathRecoveryError::Retryable { cause, .. }) if !cause.contains("deferred"))
    );
    assert!(ae.contact_ready(
        original.remote_id(),
        crate::resources::ContactClass::Foreground
    ));
    assert!(original.close_reason().is_none() && remote.close_reason().is_none());
    original.close(0u32.into(), b"fixture ended");
    remote.close(0u32.into(), b"fixture ended");
    tokio::join!(ae.close(), be.close());
    tokio::join!(a.close(), b.close());
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
                            connection_diagnostic::AttemptLog::new(
                                AttemptRole::Initiator,
                                "explicit_improvement",
                            ),
                        ))
                        .await;
                    if matches!(&result, Ok(Ok(_))) {
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
        assert!(matches!(&client_result, Ok(Ok(_))), "{client_result:?}");
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

#[derive(Clone, Debug)]
struct EchoMember(tokio::sync::mpsc::UnboundedSender<Connection>);

impl iroh::protocol::ProtocolHandler for EchoMember {
    async fn accept(&self, connection: Connection) -> Result<(), iroh::protocol::AcceptError> {
        self.0
            .send(connection.clone())
            .map_err(iroh::protocol::AcceptError::from_err)?;
        while let Ok((mut send, mut receive)) = connection.accept_bi().await {
            let bytes = receive
                .read_to_end(32)
                .await
                .map_err(iroh::protocol::AcceptError::from_err)?;
            send.write_all(&bytes)
                .await
                .map_err(iroh::protocol::AcceptError::from_err)?;
            send.finish()
                .map_err(iroh::protocol::AcceptError::from_err)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct ObserveControl(tokio::sync::mpsc::UnboundedSender<iroh::endpoint::ControlConnection>);

impl iroh::protocol::ControlProtocolHandler for ObserveControl {
    async fn accept(
        &self,
        connection: iroh::endpoint::ControlConnection,
    ) -> Result<(), iroh::protocol::AcceptError> {
        self.0
            .send(connection)
            .map_err(iroh::protocol::AcceptError::from_err)
    }
}

#[derive(Clone, Debug, Default)]
struct PauseProtectedFile {
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}

impl iroh::endpoint::EndpointHooks for PauseProtectedFile {
    async fn after_handshake(
        &self,
        connection: &Connection,
    ) -> iroh::endpoint::AfterHandshakeOutcome {
        if connection.alpn() == crate::file_transfer::FILE_ALPN {
            self.entered.notify_one();
            self.release.notified().await;
        }
        iroh::endpoint::AfterHandshakeOutcome::Accept
    }
}

async fn echo_member(connection: &Connection) {
    let (mut send, mut receive) = connection.open_bi().await.unwrap();
    send.write_all(b"member request").await.unwrap();
    send.finish().unwrap();
    assert_eq!(receive.read_to_end(32).await.unwrap(), b"member request");
}

#[test]
fn first_contact_relay_survives_a_blackholed_active_ip() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "connection_coordinator::active_commit_tests::first_contact_relay_fixture",
            "--ignored",
            "--nocapture",
        ])
        .output()
        .unwrap();
    println!("{}", String::from_utf8_lossy(&output.stdout));
    let stderr = String::from_utf8(output.stderr).unwrap();
    eprint!("{stderr}");
    assert!(output.status.success());

    let records: Vec<Value> = stderr
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    for (stage, winner) in [
        ("mixed-business", Some("relay")),
        ("mixed-control", Some("relay")),
        ("revoked-in-flight", None),
        ("mixed-cancelled", None),
        ("mixed-expired", None),
        ("foreground-dropped", None),
        ("direct-with-offline-relay", Some("ip")),
    ] {
        let start = records
            .iter()
            .position(|record| record["stage"] == stage)
            .unwrap();
        let attempt = &records[start + 1..];
        let end = attempt
            .iter()
            .position(|record| record["event"] == "first-contact-fixture")
            .unwrap();
        let events: Vec<_> = attempt[..end]
            .iter()
            .filter(|record| record["role"] == "initiator")
            .collect();
        let route_events: Vec<_> = events
            .iter()
            .copied()
            .filter(|event| event["phase"] == "route-dial")
            .collect();

        // Both routes start under the one attempt before either ends, and each
        // settles exactly once. A one-route retry cannot satisfy this assertion.
        assert_eq!(route_events.len(), 4, "{stage}");
        let [ip, relay, ..] = route_events.as_slice() else {
            unreachable!()
        };
        assert_eq!(
            ip["detail"]["input"]["routeHints"][0]["kind"], "ip",
            "{stage}"
        );
        assert_eq!(
            relay["detail"]["input"]["routeHints"][0]["kind"], "relay",
            "{stage}"
        );
        assert_eq!(ip["localAttemptId"], relay["localAttemptId"], "{stage}");
        for started in [*ip, *relay] {
            assert_eq!(started["result"], "started", "{stage}");
            assert_eq!(
                started["detail"]["input"]["routeMode"], "explicit",
                "{stage}"
            );
            let phase_id = &started["detail"]["phaseId"];
            let attempt_id = &started["localAttemptId"];
            let route_kind = started["detail"]["input"]["routeHints"][0]["kind"].as_str();
            let ended: Vec<_> = route_events[2..]
                .iter()
                .filter(|event| {
                    &event["localAttemptId"] == attempt_id
                        && &event["detail"]["phaseId"] == phase_id
                })
                .collect();
            assert_eq!(ended.len(), 1, "{stage}");
            let outcome = &ended[0]["result"];
            if winner == route_kind {
                assert_eq!(outcome, "succeeded", "{stage}");
            } else if stage == "revoked-in-flight" && route_kind == Some("relay") {
                assert_eq!(outcome, "failed", "{stage}");
                assert_ne!(
                    ended[0]["detail"]["output"]["reason"], "deadline",
                    "{stage}"
                );
            } else if stage == "mixed-expired" {
                assert!(outcome == "failed" || outcome == "cancelled", "{stage}");
            } else {
                assert_eq!(outcome, "cancelled", "{stage}");
            }
        }
        let delivered: Vec<_> = events
            .iter()
            .filter(|event| event["phase"] == "dial-delivered")
            .collect();
        if let Some(winner) = winner {
            assert_eq!(delivered.len(), 1, "{stage}");
            let started = if winner == "ip" { ip } else { relay };
            let delivery = delivered[0];
            assert_eq!(
                delivery["localAttemptId"], started["localAttemptId"],
                "{stage}"
            );
            assert_eq!(
                delivery["routePhaseId"], started["detail"]["phaseId"],
                "{stage}"
            );
            assert!(
                events.iter().any(|event| event["phase"] == "hello"
                    && event["result"] == "succeeded"
                    && event["localAttemptId"] == delivery["localAttemptId"]
                    && event["routePhaseId"] == delivery["routePhaseId"]),
                "{stage}"
            );
        } else {
            assert!(delivered.is_empty(), "{stage}");
        }
    }
}

#[tokio::test]
#[ignore = "run through first_contact_relay_survives_a_blackholed_active_ip to assert the timeline"]
async fn first_contact_relay_fixture() {
    let netem_ready = std::env::var_os("MIOPUNCH_134_NETEM_READY").map(std::path::PathBuf::from);
    let window = if netem_ready.is_some() {
        Duration::from_secs(20)
    } else {
        Duration::from_secs(2)
    };

    let (_directory, caller, responder) = admitted_pair().await;
    let mut config = iroh_relay::server::ServerConfig::default();
    config.relay = Some(iroh_relay::server::RelayConfig::new((
        Ipv4Addr::LOCALHOST,
        0,
    )));
    let relay = iroh_relay::server::Server::spawn(config).await.unwrap();
    let relay_route = TransportAddr::Relay(relay.http_url().unwrap());
    for node in [&caller, &responder] {
        node.admission_gate()
            .register_protocol(crate::file_transfer::FILE_ALPN, ProtocolScope::Member);
        let mut config = node.resource_configuration().unwrap();
        config.maintenance.contact_timeout_ms = window.as_millis() as u64;
        config.tcp_direct_disabled = true;
        config.member_data_relay_disabled = true;
        config.replacement = Some(vec![crate::resources::Resource {
            url: relay.http_url().unwrap().to_string(),
            uses: [crate::resources::ResourceUse::DataRelay].into(),
            limitation: "owned first contact relay".into(),
        }]);
        node.save_resource_configuration(&config).unwrap();
    }

    let gate = caller.admission_gate();
    let pause = PauseProtectedFile::default();
    let (control_tx, mut controls) = tokio::sync::mpsc::unbounded_channel();
    let (member_tx, mut members) = tokio::sync::mpsc::unbounded_channel();
    let responder = responder
        .maintain_network_with(Ipv4Addr::LOCALHOST, |router| {
            router
                .accept(MEMBER_ALPN, EchoMember(member_tx.clone()))
                .accept(crate::file_transfer::FILE_ALPN, EchoMember(member_tx))
                .accept_control(ADMISSION_REQUEST_ALPN, ObserveControl(control_tx))
        })
        .await
        .unwrap();
    let caller = caller
        .maintain_network_with_endpoint(
            Ipv4Addr::LOCALHOST,
            |builder| builder.hooks(pause.clone()),
            |router| router,
        )
        .await
        .unwrap();
    let _responder_activity = responder.business_activity();
    let _caller_activity = caller.business_activity();
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(caller.endpoint().online(), responder.endpoint().online());
    })
    .await
    .expect("the owned relay must be online for both members");

    // An owned UDP router allows the same previously working address to drop
    // packets silently while the original connection still has an active path.
    let udp = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let direct = TransportAddr::Ip(udp.local_addr().unwrap());
    let responder_addr = responder
        .addr()
        .ip_addrs()
        .copied()
        .find(|a| a.ip() == Ipv4Addr::LOCALHOST)
        .unwrap();
    let blackhole = Arc::new(AtomicBool::new(false));
    let dropped = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let router = tokio::spawn({
        let blackhole = blackhole.clone();
        let dropped = dropped.clone();
        async move {
            let mut caller_addr = None;
            let mut packet = vec![0; 65536];
            loop {
                let (length, from) = udp.recv_from(&mut packet).await.unwrap();
                if blackhole.load(Ordering::Acquire) {
                    dropped.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                if from == responder_addr {
                    if let Some(caller_addr) = caller_addr {
                        udp.send_to(&packet[..length], caller_addr).await.unwrap();
                    }
                } else {
                    caller_addr = Some(from);
                    udp.send_to(&packet[..length], responder_addr)
                        .await
                        .unwrap();
                }
            }
        }
    });
    let peer = iroh::EndpointAddr::new(responder.endpoint().id());
    let direct_only = peer.clone().with_addrs([direct.clone()]);
    let relay_only = peer.clone().with_addrs([relay_route.clone()]);
    let mixed = peer.clone().with_addrs([direct.clone(), relay_route]);
    let attempt = || {
        AttemptContext::new(
            n0_future::time::Instant::now() + window,
            tokio_util::sync::CancellationToken::new(),
        )
    };
    let require_active = || async {
        let remotes = caller.endpoint().remote_info(peer.id).await.unwrap();
        assert!(
            remotes.addrs().any(|route| route.addr() == &direct
                && matches!(route.usage(), iroh::endpoint::TransportAddrUsage::Active)),
            "the blackholed IP must still be an active hint: {remotes:?}",
        );
    };
    let mark = |stage| {
        eprintln!(
            "{}",
            serde_json::json!({
                "event":"first-contact-fixture", "stage":stage,
                "monotonicMs":connection_diagnostic::monotonic_ms(),
                "droppedPackets":dropped.load(Ordering::Relaxed),
            })
        );
    };

    let original = caller
        .coordinator()
        .connect(
            direct_only.clone(),
            SessionPurpose::MemberBusiness,
            attempt(),
        )
        .await
        .unwrap();
    let original_remote = tokio::time::timeout(window, members.recv())
        .await
        .unwrap()
        .unwrap();
    // Keep the warmup connection on the proxy rather than letting iroh
    // advertise and migrate to the responders' unproxied loopback sockets.
    let caller_handle = caller.endpoint().control_handle(&original);
    let responder_handle = responder.endpoint().control_handle(&original_remote);
    let (caller_mask, responder_mask) = tokio::join!(
        caller_handle.test_limit_transport_paths(vec![direct.clone()]),
        responder_handle.test_limit_transport_paths(vec![direct.clone()]),
    );
    caller_mask.unwrap();
    responder_mask.unwrap();
    tokio::time::timeout(window, echo_member(&original))
        .await
        .unwrap();
    require_active().await;
    if let Some(ready) = &netem_ready {
        // A disposable namespace adds latency after endpoint registration and
        // warming the original path. Relay-only and mixed retain one window.
        mark("netem-ready");
        tokio::time::timeout(Duration::from_secs(5), async {
            while !ready.exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("namespace latency was applied");
    } else {
        blackhole.store(true, Ordering::Release);

        mark("active-only");
        let result = caller
            .coordinator()
            .connect(
                direct_only.clone(),
                SessionPurpose::MemberBusiness,
                attempt(),
            )
            .await;
        assert!(
            matches!(result, Err(ConnectError::DeadlineExceeded)),
            "{result:?}"
        );
        assert!(dropped.load(Ordering::Relaxed) > 0);
        require_active().await;
    }

    let report_relay_rtt = |connection: &Connection| {
        if netem_ready.is_none() {
            return;
        }
        let rtt = connection
            .paths()
            .iter()
            .find_map(|path| {
                if matches!(path.remote_addr(), TransportAddr::Relay(_)) {
                    connection.rtt(path.id())
                } else {
                    None
                }
            })
            .expect("a measured data relay path");
        assert!(
            (Duration::from_secs(1)..=Duration::from_secs(2)).contains(&rtt),
            "{rtt:?}"
        );
        eprintln!(
            "{}",
            json!({"event":"first-contact-relay-rtt","rttMs":rtt.as_millis()})
        );
    };

    mark("relay-only-business");
    tokio::time::timeout(window, async {
        let (connection, _activity) = caller
            .connect(relay_only.clone(), MEMBER_ALPN)
            .await
            .unwrap();
        echo_member(&connection).await;
        report_relay_rtt(&connection);
        connection.close(0u32.into(), b"relay-only fixture complete");
    })
    .await
    .unwrap();
    if netem_ready.is_some() {
        tokio::time::timeout(window, echo_member(&original))
            .await
            .unwrap();
        blackhole.store(true, Ordering::Release);
    } else {
        mark("relay-only-control");
        let control = caller
            .coordinator()
            .connect_control_alpn(relay_only.clone(), ADMISSION_REQUEST_ALPN, attempt())
            .await
            .unwrap();
        assert_eq!(control.remote_id(), peer.id);
        let remote = tokio::time::timeout(window, controls.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(remote.remote_id(), caller.endpoint().id());
        control.close(0u32.into(), b"relay-only fixture complete");
    }

    require_active().await;
    mark("mixed-business");
    let business = tokio::time::timeout(window, async {
        let (connection, _activity) = caller.connect(mixed.clone(), MEMBER_ALPN).await?;
        assert_ne!(connection.stable_id(), original.stable_id());
        echo_member(&connection).await;
        report_relay_rtt(&connection);
        connection.close(0u32.into(), b"mixed fixture complete");
        Ok::<_, crate::resources::ResourceContactError>(())
    })
    .await;
    if netem_ready.is_some() {
        mark("finished");
        assert!(
            matches!(business, Ok(Ok(()))),
            "delayed mixed member business: {business:?}"
        );
        assert!(dropped.load(Ordering::Relaxed) > 0);
        assert!(original.close_reason().is_none());
        original.close(0u32.into(), b"delayed fixture complete");
        router.abort();
        tokio::join!(caller.close(), responder.close());
        relay.shutdown().await.unwrap();
        return;
    }
    require_active().await;
    mark("mixed-control");
    let control = caller
        .coordinator()
        .connect_control_alpn(mixed.clone(), ADMISSION_REQUEST_ALPN, attempt())
        .await;
    if let Ok(control) = &control {
        assert_eq!(control.remote_id(), peer.id);
        let remote = tokio::time::timeout(window, controls.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(remote.remote_id(), caller.endpoint().id());
        control.close(0u32.into(), b"mixed fixture complete");
    }
    mark("mixed-finished");
    assert!(
        matches!(business, Ok(Ok(()))),
        "mixed member business: {business:?}; mixed restricted control: {control:?}"
    );
    assert!(control.is_ok(), "mixed restricted control: {control:?}");
    assert!(original.close_reason().is_none());

    mark("ready-tie");
    blackhole.store(false, Ordering::Release);
    let coordinator = caller.coordinator();
    let active_attempt = attempt();
    let relay_attempt = attempt();
    let log = AttemptLog::new(AttemptRole::Initiator, "first-contact-tie");
    let (active, relayed) = tokio::join!(
        Box::pin(coordinator.dial(
            direct_only,
            SessionPurpose::MemberBusiness,
            &active_attempt,
            MEMBER_ALPN,
            false,
            log
        )),
        Box::pin(coordinator.dial(
            relay_only,
            SessionPurpose::MemberBusiness,
            &relay_attempt,
            MEMBER_ALPN,
            false,
            log
        )),
    );
    let active = active.ok().expect("real active route qualified");
    let relayed = relayed.ok().expect("real relay route qualified");
    let ConnectionDelivery::Business(loser) = &relayed.delivery else {
        unreachable!()
    };
    let loser = loser.clone();
    let active_id = active.delivery.stable_id();
    // Supply both already qualified connections to exercise the same-poll tie
    // deterministically. The actual dials and data admission ran above.
    let winner = race_active_and_relay(
        (&active_attempt, std::future::ready(Ok(active))),
        (&relay_attempt, std::future::ready(Ok(relayed))),
    )
    .await
    .ok()
    .expect("one route won");
    assert_eq!(winner.delivery.stable_id(), active_id);
    assert!(loser.close_reason().is_some());
    assert!(relay_attempt.cancellation.is_cancelled());
    assert!(!active_attempt.cancellation.is_cancelled());
    let ConnectionDelivery::Business(qualified) = &winner.delivery else {
        unreachable!()
    };
    let qualified = qualified.clone();
    tokio::time::timeout(window, echo_member(&qualified))
        .await
        .unwrap();

    // A ready success cannot hide a terminal authorization failure from the
    // other branch. The already qualified connection must also be retired.
    let rejected = race_active_and_relay(
        (&active_attempt, std::future::ready(Ok(winner))),
        (
            &attempt(),
            std::future::ready(Err(ContactDialFailure::Terminal(
                ConnectError::IdentityOrAdmissionRejected,
            ))),
        ),
    )
    .await;
    assert!(matches!(
        rejected,
        Err(ContactDialFailure::Terminal(
            ConnectError::IdentityOrAdmissionRejected
        ))
    ));
    assert!(qualified.close_reason().is_some());

    // Revoke this application ALPN after the real relay has authenticated
    // and qualified its path, with the active-IP dial still pending.
    blackhole.store(true, Ordering::Release);
    require_active().await;
    mark("revoked-in-flight");
    let coordinator_result = tokio::time::timeout(window, async {
        tokio::join!(
            coordinator.connect_alpn(mixed.clone(), crate::file_transfer::FILE_ALPN, attempt()),
            async {
                pause.entered.notified().await;
                gate.restrict_lifecycle(Some(
                    [MEMBER_ALPN.to_vec(), ADMISSION_REQUEST_ALPN.to_vec()].into(),
                ));
                pause.release.notify_one();
            },
        )
    })
    .await
    .expect("the live relay reached the protected admission hook")
    .0;
    assert!(coordinator_result.is_err(), "{coordinator_result:?}");
    assert!(original.close_reason().is_none());

    relay.shutdown().await.unwrap();
    blackhole.store(true, Ordering::Release);
    require_active().await;
    mark("mixed-cancelled");
    let ctx = attempt();
    let token = ctx.cancellation.clone();
    let (result, ()) = tokio::join!(
        coordinator.connect_control_alpn(mixed.clone(), ADMISSION_REQUEST_ALPN, ctx),
        async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            token.cancel();
        },
    );
    assert!(matches!(result, Err(ConnectError::Cancelled)), "{result:?}");

    require_active().await;
    mark("mixed-expired");
    let result = coordinator
        .connect_control_alpn(mixed.clone(), ADMISSION_REQUEST_ALPN, attempt())
        .await;
    assert!(
        matches!(result, Err(ConnectError::DeadlineExceeded)),
        "{result:?}"
    );
    assert!(controls.try_recv().is_err());

    require_active().await;
    mark("foreground-dropped");
    let dropped_call = tokio::time::timeout(
        Duration::from_millis(100),
        caller.connect(mixed.clone(), MEMBER_ALPN),
    )
    .await;
    assert!(dropped_call.is_err());

    blackhole.store(false, Ordering::Release);
    require_active().await;
    mark("direct-with-offline-relay");
    tokio::time::timeout(window, async {
        let (connection, _activity) = caller.connect(mixed.clone(), MEMBER_ALPN).await.unwrap();
        echo_member(&connection).await;
        connection.close(0u32.into(), b"direct fixture complete");
    })
    .await
    .unwrap();
    tokio::time::timeout(window, echo_member(&original))
        .await
        .unwrap();
    while let Ok(remote) = members.try_recv() {
        tokio::time::timeout(window, remote.closed()).await.unwrap();
    }

    mark("revoked");
    gate.update_authorization([*caller.endpoint().id().as_bytes()], [*peer.id.as_bytes()]);
    let result = coordinator
        .connect(mixed, SessionPurpose::MemberBusiness, attempt())
        .await;
    assert!(
        matches!(result, Err(ConnectError::IdentityOrAdmissionRejected)),
        "{result:?}"
    );
    tokio::time::timeout(window, original.closed())
        .await
        .unwrap();

    mark("finished");
    router.abort();
    tokio::join!(caller.close(), responder.close());
}
