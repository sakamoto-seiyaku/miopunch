//! Qualified controlled member connection exposed by the browser SDK.
use super::*;
use crate::connection_coordinator::{
    AttemptRole, MemberPathAdapter, RepairFailure, SessionPurpose,
    connection_diagnostic::{self, AttemptLog},
};
use crate::session_maintenance::{Completion, MaintenanceLease, Reservation};
use std::io;
#[wasm_bindgen]
pub struct BrowserMemberConnection {
    pub(super) connection: Connection,
    pub(super) gate: BrowserGate,
    #[cfg(feature = "rtc-probe")]
    pub(super) rtc_active: Arc<std::sync::atomic::AtomicBool>,
    #[cfg(feature = "rtc-probe")]
    pub(super) rtc_transport: crate::rtc_transport::RtcTransport,
    pub(super) endpoint: RestrictedEndpoint,
    repair_contact: EndpointAddr,
    improvement_cancel: Mutex<Option<tokio_util::sync::CancellationToken>>,
    repair_cancel: Mutex<Option<tokio_util::sync::CancellationToken>>,
    #[cfg(feature = "rtc-probe")]
    pub(super) rtc_live: std::rc::Rc<std::cell::RefCell<Option<super::rtc_member::OwnedRtc>>>,
    #[cfg(feature = "rtc-probe")]
    pub(super) rtc_continuity:
        std::rc::Rc<std::cell::RefCell<Option<super::rtc_member::Continuity>>>,
}
impl BrowserMemberConnection {
    pub(super) async fn qualified_snapshot(&self) -> Result<SavedNode, JsValue> {
        let result = self.gate.qualified_snapshot(&self.connection).await;
        if result.is_err() {
            self.close();
        }
        result
    }
    #[cfg(feature = "rtc-probe")]
    pub(super) fn rtc_qualification(
        &self,
    ) -> Result<
        (
            Arc<std::sync::atomic::AtomicU64>,
            u64,
            n0_watcher::Direct<u64>,
        ),
        JsValue,
    > {
        self.gate.rtc_qualification(&self.connection)
    }
    fn begin_path_attempt(
        cancel_slot: &Mutex<Option<tokio_util::sync::CancellationToken>>,
    ) -> Result<(tokio_util::sync::CancellationToken, RepairGuard<'_>), JsValue> {
        let mut slot = cancel_slot.lock().unwrap();
        if slot.is_some() {
            return Err(error("member path operation already active"));
        }
        let cancel = tokio_util::sync::CancellationToken::new();
        *slot = Some(cancel.clone());
        Ok((
            cancel,
            RepairGuard {
                cancel: cancel_slot,
            },
        ))
    }

    async fn improve_path_until(
        &self,
        lease: &mut MaintenanceLease<()>,
        repair_contacts: bool,
        log: AttemptLog,
    ) -> io::Result<()> {
        let mut target = EndpointAddr::new(self.connection.remote_id());
        for path in self
            .connection
            .paths()
            .iter()
            .map(|path| path.remote_addr().clone())
            .take(8)
        {
            target.addrs.insert(path);
        }
        if repair_contacts {
            target
                .addrs
                .extend(self.repair_contact.addrs.iter().cloned());
            // Hints address C only; common permitted direct/relay/contact
            // ordering decides its carrier. They are never a DATA offer.
        }
        if target.addrs.is_empty() {
            return Err(io::Error::other(
                "active improvement has no current contact route",
            ));
        }
        let caps = self
            .gate
            .capabilities(AttemptRole::initial(&self.connection));
        let contact_ids = target
            .addrs
            .iter()
            .filter_map(|path| match path {
                iroh::TransportAddr::Custom(address) => Some(address.id()),
                _ => None,
            })
            .collect();
        let attempt = lease.context().clone();
        let result = lease
            .run_ref(async {
                let control = crate::connection_coordinator::ConnectionCoordinator::new_restricted(
                    self.endpoint.clone(),
                    caps.permitted,
                    contact_ids,
                )
                .with_member_data(crate::member_data::DATA_TRANSPORT_ID)
                .connect_active_control(target, attempt.clone(), log)
                .await
                .map_err(|cause| io::Error::other(cause.to_string()))?;
                // Own exact C before any further await, including losing guard races.
                let _control = RequestConnection(Some(control.clone()));
                crate::connection_coordinator::improve_existing(
                    &self.endpoint,
                    &self.connection,
                    &control.path_handle(&self.endpoint),
                    &self.gate,
                    attempt,
                    log,
                    || lease.seal_outgoing_hello(),
                )
                .await
            })
            .await
            .map_err(|cause| io::Error::other(format!("active improvement failed: {cause:?}")))??;
        lease.completed(Completion::Active(result));
        Ok(())
    }
}
#[wasm_bindgen]
impl BrowserMemberConnection {
    /// Improve the existing ordinary M through a fresh restricted control C.
    /// The operation has one caller clock and retires only its candidate on
    /// failure; the original relay/stream remains owned by this handle.
    pub async fn improve_path(&self, caller_ms: u32) -> Result<String, JsValue> {
        if caller_ms == 0
            || u64::from(caller_ms) > crate::connection_budget::MAX_MS
            || self.is_closed()
        {
            return Err(error("active improvement caller budget is invalid"));
        }
        let deadline = n0_future::time::Instant::now() + Duration::from_millis(caller_ms.into());
        let (cancel, _guard) = Self::begin_path_attempt(&self.improvement_cancel)?;
        let attempt = crate::connection_coordinator::AttemptContext::new(deadline, cancel);
        let mut lease = self
            .gate
            .maintenance()
            .acquire(&self.connection, &attempt, ())
            .map_err(|cause| error(format!("active improvement reservation: {cause:?}")))?;
        let mut log = AttemptLog::new(AttemptRole::Initiator, "explicit_improvement");
        log.original = Some(self.connection.stable_id());
        self.improve_path_until(&mut lease, false, log)
            .await
            .map_err(error)?;
        Ok(
            serde_json::json!({"outcome":"committed","stableId":self.connection.stable_id()})
                .to_string(),
        )
    }

    /// Cancel only the outstanding explicit path-maintenance operation.
    pub fn cancel_path_recovery(&self) {
        if let Some(cancel) = self.repair_cancel.lock().unwrap().as_ref() {
            cancel.cancel();
        } else if let Some(cancel) = self.improvement_cancel.lock().unwrap().as_ref() {
            cancel.cancel();
        }
    }

    /// Repair this exact living M through the same restricted coordinator used
    /// for path improvement. A failed repair terminates this M if no permitted
    /// data path is still open; a later normal connect owns a new connection.
    pub async fn recover_path(&self, caller_ms: u32) -> Result<String, JsValue> {
        let started = n0_future::time::Instant::now();
        let mut log = AttemptLog::new(AttemptRole::Initiator, "foreground_repair");
        log.original = Some(self.connection.stable_id());
        if let Some(reason) = self.connection.close_reason() {
            log.connection_ended(&reason, None);
            return Ok(
                serde_json::json!({"outcome":"terminal","stableId":self.connection.stable_id()})
                    .to_string(),
            );
        }
        if caller_ms == 0 || u64::from(caller_ms) > crate::connection_budget::MAX_MS {
            return Err(error("repair caller budget must fit the browser budget"));
        }
        let deadline = (started + Duration::from_millis(caller_ms.into()))
            .min(started + crate::connection_recovery_core::REPAIR_TIMEOUT);
        let (cancel, _guard) = Self::begin_path_attempt(&self.repair_cancel)?;
        let attempt = crate::connection_coordinator::AttemptContext::new(deadline, cancel);
        let phase = log.phase(
            "repair",
            serde_json::json!({"budgetMs":deadline.duration_since(started).as_millis() as u64}),
        );
        let response = |outcome, failure: Option<String>| {
            Ok(serde_json::json!({"outcome":outcome,"stableId":self.connection.stable_id(),
                "controlAlpn":String::from_utf8_lossy(crate::connection_coordinator::ACTIVE_IMPROVEMENT_ALPN),
                "elapsedMs":started.elapsed().as_millis(),"callerBudgetMs":caller_ms,
                "failure":failure}).to_string())
        };
        let registry = self.gate.maintenance();
        let reservation = async {
            let lease = match registry.acquire_or_wait(&self.connection, &attempt, ())? {
                Reservation::Owned(lease) => lease,
                Reservation::Pending(wait) => {
                    if let Some(Completion::Active(path)) = wait.wait().await? {
                        let bound = self.endpoint.control_handle(&self.connection);
                        let current = tokio::select! {
                            biased;
                            _ = attempt.cancellation.cancelled() => false,
                            result = n0_future::time::timeout(
                                deadline.saturating_duration_since(n0_future::time::Instant::now()),
                                self.gate.check(&bound, SessionPurpose::MemberBusiness),
                            ) => result.is_ok_and(|result| result.is_ok()),
                        };
                        if current
                            && !attempt.cancellation.is_cancelled()
                            && deadline > n0_future::time::Instant::now()
                            && path.is_open(&self.connection)
                            && self
                                .gate
                                .permits_repair_survival(&self.connection, &path.address)
                            && self
                                .gate
                                .authorization_revision(&bound, SessionPurpose::MemberBusiness)
                                .is_ok_and(|revision| revision == path.revision)
                        {
                            return Ok(None);
                        }
                    }
                    registry.acquire(&self.connection, &attempt, ())?
                }
            };
            Ok::<_, crate::session_maintenance::MaintenanceError>(Some(lease))
        };
        let mut lease = match reservation.await {
            Ok(Some(lease)) => lease,
            Ok(None) => {
                phase.finish("reused");
                return response("recovered", None);
            }
            Err(cause) => {
                let outcome = if self.is_closed() {
                    "terminal"
                } else {
                    "unreachable"
                };
                phase.finish(outcome);
                return response(outcome, Some(format!("repair reservation: {cause:?}")));
            }
        };
        let result = self.improve_path_until(&mut lease, true, log).await;
        let outcome = match &result {
            Ok(()) if !self.is_closed() => "recovered",
            Ok(()) => {
                let reason = self.connection.close_reason().expect("original is closed");
                log.connection_ended(&reason, None);
                "terminal"
            }
            Err(cause) => match crate::connection_coordinator::settle_failed_repair(
                &self.connection,
                &attempt,
                cause,
                |address| self.gate.permits_repair_survival(&self.connection, address),
                log,
                "browser_sdk_repair",
            ) {
                RepairFailure::Terminal => "terminal",
                RepairFailure::Deferred | RepairFailure::Retryable => "unreachable",
            },
        };
        phase.finish(outcome);
        response(outcome, result.err().map(|cause| cause.to_string()))
    }

    /// Open an actual configured relay on the exact original MID1 member for
    /// the owned transport-withdrawal fixture. QUIC validates the new path.
    #[cfg(feature = "member-data-test-hooks")]
    pub async fn mid1_relay_backup_fixture(&self, relay: String) -> Result<(), JsValue> {
        self.qualified_snapshot().await?;
        let address = iroh::TransportAddr::Relay(relay.parse().map_err(error)?);
        if !self
            .gate
            .permits_repair_survival(&self.connection, &address)
            || !self.connection.paths().iter().any(|path| {
                matches!(path.remote_addr(), iroh::TransportAddr::Custom(addr)
                    if addr.id() == crate::member_data::DATA_TRANSPORT_ID)
                    && self
                        .gate
                        .permits_repair_survival(&self.connection, path.remote_addr())
            })
        {
            return Err(error("permitted relay and live MID1 original required"));
        }
        self.endpoint
            .control_handle(&self.connection)
            .open_data_transport_path(address)
            .await
            .map_err(error)
    }

    /// Bounded real-control refusal fixture. It changes no candidate permission
    /// and exposes no exporter. Terminal mode requires an already closed M and
    /// observes its retained stream's actual write failure while C remains live.
    #[cfg(feature = "rtc-probe")]
    pub async fn rtc_active_refusal_fixture(
        &self,
        mode: String,
        caller_ms: u32,
    ) -> Result<String, JsValue> {
        use crate::connection_coordinator::{AttemptContext, AttemptRole, MemberPathAdapter};
        let wrong = match mode.as_str() {
            "wrong-binding" if !self.is_closed() => true,
            "terminal-live-control" if self.is_closed() => false,
            _ => return Err(error("refusal fixture mode/original state differs")),
        };
        if caller_ms == 0 || caller_ms > 60000 {
            return Err(error("bounded refusal fixture caller required"));
        }
        let deadline = n0_future::time::Instant::now() + Duration::from_millis(caller_ms.into());
        let cancel = tokio_util::sync::CancellationToken::new();
        let attempt = AttemptContext::new(deadline, cancel);
        let mut target = self.repair_contact.clone();
        target.addrs.extend(
            self.connection
                .paths()
                .iter()
                .map(|path| path.remote_addr().clone()),
        );
        let caps = self
            .gate
            .capabilities(AttemptRole::initial(&self.connection));
        let contact_ids = target
            .addrs
            .iter()
            .filter_map(|address| match address {
                iroh::TransportAddr::Custom(address)
                    if matches!(
                        address.id(),
                        crate::mqtt_datagram::TRANSPORT_ID | crate::irc_datagram::TRANSPORT_ID
                    ) =>
                {
                    Some(address.id())
                }
                _ => None,
            })
            .collect();
        let control = crate::connection_coordinator::ConnectionCoordinator::new_restricted(
            self.endpoint.clone(),
            caps.permitted,
            contact_ids,
        )
        .with_member_data(crate::member_data::DATA_TRANSPORT_ID)
        .connect_control_alpn(
            target,
            crate::connection_coordinator::ACTIVE_IMPROVEMENT_ALPN,
            attempt,
        )
        .await
        .map_err(error)?;
        let _control = RequestConnection(Some(control.clone()));
        let before = control.close_reason().is_none();
        let original_id = self.connection.stable_id();
        if wrong {
            self.qualified_snapshot().await?;
            let result = crate::connection_coordinator::wrong_active_binding_fixture(
                &self.connection,
                &control,
                caps,
                deadline,
            )
            .await
            .map_err(error)?;
            Ok(serde_json::json!({"mode":mode,"actualRestrictedControlAuthenticated":before,
                "controlStableId":control.stable_id(),"controlAlpn":String::from_utf8_lossy(control.alpn()),
                "wrongExporterBindingSent":true,"actualOfferRefused":result.is_err(),
                "originalStableId":original_id,"originalStillOpen":!self.is_closed(),
                "failure":result.err().map(|cause|cause.to_string())}).to_string())
        } else {
            self.connection.closed().await;
            let retry: serde_json::Value =
                serde_json::from_str(&self.recover_path(250).await?).map_err(error)?;
            let mut stream = self
                .rtc_continuity
                .borrow_mut()
                .take()
                .ok_or_else(|| error("retained old stream required for Terminal fixture"))?;
            let (stream_id, failed) = stream.terminal_write_failure(&self.connection).await?;
            *self.rtc_continuity.borrow_mut() = Some(stream);
            Ok(serde_json::json!({"mode":mode,"actualRestrictedControlAuthenticated":before,
                "controlStableId":control.stable_id(),"controlAlpn":String::from_utf8_lossy(control.alpn()),
                "controlStillLiveAfterTerminalRetry":control.close_reason().is_none(),
                "oldOriginalClosed":self.is_closed(),"originalStableId":original_id,
                "terminalRetry":retry,"actualRetainedStreamId":stream_id,
                "actualOldStreamWriteFailed":failed,"offerSent":false}).to_string())
        }
    }

    /// Facts about this actual ordinary member M, not its earlier admission.
    pub fn session_diagnostics(&self) -> String {
        let state = self.gate.0.lock().unwrap();
        let member_authorized = state.saved.as_ref().is_some_and(|saved| {
            let Ok(invitation) = saved.invitation() else {
                return false;
            };
            let scope = AdmissionScope::new(*invitation.anchor.root.as_bytes(), saved.generation);
            BrowserGate::permits(saved, self.connection.remote_id(), self.connection.alpn())
                && state
                    .proofs
                    .iter()
                    .any(|proof| proof.applies_to(&self.connection, &scope))
        });
        serde_json::json!({"remoteId":self.connection.remote_id().to_string(),"newProtocol":self.connection.alpn()==BUSINESS_ALPN,
            "alpn":String::from_utf8_lossy(self.connection.alpn()),"ordinaryHandle":!self.is_closed(),
            "expectedPeerMatched":self.connection.remote_id()==self.repair_contact.id,"memberAuthorized":member_authorized,
            "coordinator":state.member_reports.get(&self.connection.stable_id()).map(|report|&report["coordinator"]),
            "bootstrap":state.member_reports.get(&self.connection.stable_id()),
            "paths":self.connection.paths().iter().map(|path|serde_json::json!({"pathId":path.id().to_string(),
                "route":path.remote_addr().to_string(),"selected":path.is_selected(),"isIp":path.is_ip(),"isRelay":path.is_relay(),
                "isRtc": ({
                    #[cfg(feature="rtc-probe")]
                    {matches!(path.remote_addr(),iroh::TransportAddr::Custom(address) if address.id()==crate::rtc_transport::TRANSPORT_ID)}
                    #[cfg(not(feature="rtc-probe"))]
                    {false}
                }),
                "isMemberData":matches!(path.remote_addr(),iroh::TransportAddr::Custom(address)
                    if address.id()==crate::member_data::DATA_TRANSPORT_ID),
                "streamSent":path.stats().frame_tx.stream,"streamReceived":path.stats().frame_rx.stream,
                "datagramSent":path.stats().frame_tx.datagram,"datagramReceived":path.stats().frame_rx.datagram,
                "isSignal":matches!(path.remote_addr(),iroh::TransportAddr::Custom(address) if matches!(address.id(),crate::mqtt_datagram::TRANSPORT_ID|crate::irc_datagram::TRANSPORT_ID)),
                "rttMicros":path.stats().rtt.as_micros(),
                "sentBytes":path.stats().udp_tx.bytes,"receivedBytes":path.stats().udp_rx.bytes})).collect::<Vec<_>>()}).to_string()
    }
    pub fn is_closed(&self) -> bool {
        self.connection.close_reason().is_some()
    }
    pub fn close(&self) {
        self.cancel_path_recovery();
        #[cfg(feature = "rtc-probe")]
        {
            self.rtc_live.borrow_mut().take();
            self.rtc_continuity.borrow_mut().take();
        }
        self.connection
            .close(0u32.into(), b"browser caller disconnected");
    }
    pub async fn exchange(&self, message: String) -> Result<String, JsValue> {
        if message.len() > 65536 || self.is_closed() {
            return Err(rejected());
        }
        self.qualified_snapshot().await?;
        n0_future::time::timeout(TIMEOUT, async {
            let (mut send, mut recv) = self.connection.open_bi().await.map_err(error)?;
            send.write_all(message.as_bytes()).await.map_err(error)?;
            send.finish().map_err(error)?;
            String::from_utf8(recv.read_to_end(65536).await.map_err(error)?).map_err(error)
        })
        .await
        .map_err(|_| error("member connection unavailable"))?
    }

    /// Serve one ordinary exchange on this qualified M using the same bounded
    /// stream protocol as the native member counterpart.
    pub async fn serve_exchange(&self) -> Result<String, JsValue> {
        self.qualified_snapshot().await?;
        n0_future::time::timeout(TIMEOUT, async {
            let (mut send, mut recv) = self.connection.accept_bi().await.map_err(error)?;
            let message = recv.read_to_end(65536).await.map_err(error)?;
            self.qualified_snapshot().await?;
            send.write_all(&message).await.map_err(error)?;
            send.finish().map_err(error)?;
            String::from_utf8(message).map_err(error)
        })
        .await
        .map_err(|_| error("member exchange unavailable"))?
    }
}

struct RepairGuard<'a> {
    cancel: &'a Mutex<Option<tokio_util::sync::CancellationToken>>,
}
impl Drop for RepairGuard<'_> {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.lock().unwrap().take() {
            cancel.cancel();
        }
    }
}
#[wasm_bindgen]
impl BrowserNetwork {
    /// Create one incoming operation before waiting for its particular M.
    /// Cancellation does not close this node or another member connection.
    pub fn member_accept_attempt(
        &self,
        caller_ms: u32,
    ) -> Result<BrowserMemberAcceptAttempt, JsValue> {
        if caller_ms == 0 || Duration::from_millis(u64::from(caller_ms)) > self.caller_budget {
            return Err(error(
                "member caller budget must fit the original browser budget",
            ));
        }
        if self.lease.is_none() || self.gate.service_snapshot()?.stage() != "Complete" {
            return Err(rejected());
        }
        Ok(BrowserMemberAcceptAttempt {
            endpoint: self.endpoint.clone(),
            gate: self.gate.clone(),
            incoming: self.incoming.clone(),
            deadline: n0_future::time::Instant::now() + Duration::from_millis(u64::from(caller_ms)),
            cancel: tokio_util::sync::CancellationToken::new(),
            started: std::sync::atomic::AtomicBool::new(false),
            #[cfg(feature = "rtc-probe")]
            rtc_transport: self.rtc_transport.clone(),
        })
    }

    pub async fn open_member_connection(
        &self,
        target: String,
    ) -> Result<BrowserMemberConnection, JsValue> {
        self.member_connection_attempt(target, self.caller_budget.as_millis() as u32)?
            .connect()
            .await
    }

    /// One pending operation, with the caller's original clock and cancellation.
    /// This never exposes an unadmitted Connection or allocates a second endpoint.
    pub fn member_connection_attempt(
        &self,
        target: String,
        caller_ms: u32,
    ) -> Result<BrowserMemberAttempt, JsValue> {
        if caller_ms == 0 || Duration::from_millis(u64::from(caller_ms)) > self.caller_budget {
            return Err(error(
                "member caller budget must fit the original browser budget",
            ));
        }
        let target = target.parse().map_err(error)?;
        if self.lease.is_none()
            || !BrowserGate::permits(&self.gate.service_snapshot()?, target, BUSINESS_ALPN)
        {
            return Err(rejected());
        }
        Ok(BrowserMemberAttempt {
            endpoint: self.endpoint.clone(),
            gate: self.gate.clone(),
            address: self.target_address(target)?,
            deadline: n0_future::time::Instant::now() + Duration::from_millis(u64::from(caller_ms)),
            cancel: tokio_util::sync::CancellationToken::new(),
            started: std::sync::atomic::AtomicBool::new(false),
            #[cfg(feature = "rtc-probe")]
            rtc_transport: self.rtc_transport.clone(),
        })
    }
}

#[wasm_bindgen]
pub struct BrowserMemberAcceptAttempt {
    endpoint: RestrictedEndpoint,
    gate: BrowserGate,
    incoming: std::rc::Rc<super::incoming::IncomingDispatcher>,
    deadline: n0_future::time::Instant,
    cancel: tokio_util::sync::CancellationToken,
    started: std::sync::atomic::AtomicBool,
    #[cfg(feature = "rtc-probe")]
    rtc_transport: crate::rtc_transport::RtcTransport,
}

impl Drop for BrowserMemberAcceptAttempt {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[wasm_bindgen]
impl BrowserMemberAcceptAttempt {
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub async fn accept(&self) -> Result<BrowserMemberConnection, JsValue> {
        use futures_util::FutureExt;
        if self.started.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return Err(error("member attempt already started"));
        }
        let work = async {
            let (saved, _, _) = services::installed(&self.gate).await?;
            if saved.stage() != "Complete" {
                return Err(rejected());
            }
            use crate::connection_coordinator::{
                AttemptContext, AttemptRole, MemberPathAdapter, RepairFailure, SessionPurpose,
            };
            let caps = self.gate.capabilities(AttemptRole::Responder);
            let coordinator = crate::connection_coordinator::ConnectionCoordinator::new_restricted(
                self.endpoint.clone(),
                caps.permitted,
                vec![],
            );
            let attempt = AttemptContext::new(self.deadline, self.cancel.clone());
            let accepting = self.incoming.receive(BUSINESS_ALPN, &attempt).await?;
            let connection = coordinator
                .accept_accepting(accepting, SessionPurpose::MemberBusiness, attempt)
                .await
                .map_err(error)?;
            let mut pending = RequestConnection(Some(connection.clone()));
            let owner = BrowserMemberConnection {
                repair_contact: EndpointAddr::new(connection.remote_id()),
                connection,
                gate: self.gate.clone(),
                endpoint: self.endpoint.clone(),
                improvement_cancel: Mutex::new(None),
                repair_cancel: Mutex::new(None),
                #[cfg(feature = "rtc-probe")]
                rtc_active: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                #[cfg(feature = "rtc-probe")]
                rtc_transport: self.rtc_transport.clone(),
                #[cfg(feature = "rtc-probe")]
                rtc_live: std::rc::Rc::new(std::cell::RefCell::new(None)),
                #[cfg(feature = "rtc-probe")]
                rtc_continuity: std::rc::Rc::new(std::cell::RefCell::new(None)),
            };
            owner.qualified_snapshot().await?;
            pending.0 = None;
            Ok(owner)
        }
        .fuse();
        let cancelled = self.cancel.cancelled().fuse();
        futures_util::pin_mut!(work, cancelled);
        n0_future::time::timeout(
            self.deadline
                .saturating_duration_since(n0_future::time::Instant::now()),
            async {
                futures_util::select_biased! {
                    _ = cancelled => Err(error("member attempt cancelled")),
                    result = work => result,
                }
            },
        )
        .await
        .map_err(|_| error("member connection caller deadline exceeded"))?
    }
}

#[wasm_bindgen]
pub struct BrowserMemberAttempt {
    pub(super) endpoint: RestrictedEndpoint,
    pub(super) gate: BrowserGate,
    pub(super) address: EndpointAddr,
    pub(super) deadline: n0_future::time::Instant,
    pub(super) cancel: tokio_util::sync::CancellationToken,
    pub(super) started: std::sync::atomic::AtomicBool,
    #[cfg(feature = "rtc-probe")]
    pub(super) rtc_transport: crate::rtc_transport::RtcTransport,
}
impl Drop for BrowserMemberAttempt {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[wasm_bindgen]
impl BrowserMemberAttempt {
    /// Cancels only this pending connect; the browser node and saved materials
    /// remain available for another independently bounded member attempt.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub async fn connect(&self) -> Result<BrowserMemberConnection, JsValue> {
        use futures_util::FutureExt;
        if self.started.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return Err(error("member attempt already started"));
        }
        let work = async {
            let (saved, _, _) = services::installed(&self.gate).await?;
            if !BrowserGate::permits(&saved, self.address.id, BUSINESS_ALPN) {
                return Err(rejected());
            }
            use crate::connection_coordinator::MemberPathAdapter;
            let caps = self
                .gate
                .capabilities(crate::connection_coordinator::AttemptRole::Initiator);
            let coordinator = crate::connection_coordinator::ConnectionCoordinator::new_restricted(
                self.endpoint.clone(),
                caps.permitted,
                self.address
                    .addrs
                    .iter()
                    .filter_map(|address| match address {
                        iroh::TransportAddr::Custom(address) => Some(address.id()),
                        _ => None,
                    })
                    .collect(),
            )
            .with_member_data(crate::member_data::DATA_TRANSPORT_ID);
            coordinator
                .connect(
                    self.address.clone(),
                    crate::connection_coordinator::SessionPurpose::MemberBusiness,
                    crate::connection_coordinator::AttemptContext::new(
                        self.deadline,
                        self.cancel.clone(),
                    ),
                )
                .await
                .map_err(error)
        }
        .fuse();
        let cancelled = self.cancel.cancelled().fuse();
        let connection = {
            futures_util::pin_mut!(work, cancelled);
            n0_future::time::timeout(
                self.deadline
                    .saturating_duration_since(n0_future::time::Instant::now()),
                async {
                    futures_util::select_biased! {
                        _ = cancelled => Err(error("member attempt cancelled")),
                        result = work => result,
                    }
                },
            )
            .await
            .map_err(|_| error("member connection caller deadline exceeded"))??
        };
        Ok(BrowserMemberConnection {
            connection,
            gate: self.gate.clone(),
            #[cfg(feature = "rtc-probe")]
            rtc_active: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            #[cfg(feature = "rtc-probe")]
            rtc_transport: self.rtc_transport.clone(),
            endpoint: self.endpoint.clone(),
            repair_contact: self.address.clone(),
            improvement_cancel: Mutex::new(None),
            repair_cancel: Mutex::new(None),
            #[cfg(feature = "rtc-probe")]
            rtc_live: std::rc::Rc::new(std::cell::RefCell::new(None)),
            #[cfg(feature = "rtc-probe")]
            rtc_continuity: std::rc::Rc::new(std::cell::RefCell::new(None)),
        })
    }
}
