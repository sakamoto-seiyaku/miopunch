//! Qualified controlled member connection exposed by the browser SDK.
use super::*;
#[wasm_bindgen]
pub struct BrowserMemberConnection {
    pub(super) connection: Connection,
    pub(super) gate: BrowserGate,
    lease: u32,
    #[cfg(feature = "rtc-probe")]
    pub(super) rtc_active: Arc<std::sync::atomic::AtomicBool>,
    #[cfg(feature = "rtc-probe")]
    pub(super) rtc_transport: crate::rtc_transport::RtcTransport,
    pub(super) endpoint: RestrictedEndpoint,
    repair_contact: EndpointAddr,
    repair_active: std::sync::atomic::AtomicBool,
    repair_cancel: Mutex<Option<tokio_util::sync::CancellationToken>>,
    #[cfg(feature = "rtc-probe")]
    pub(super) rtc_live: std::rc::Rc<std::cell::RefCell<Option<super::rtc_member::OwnedRtc>>>,
    #[cfg(feature = "rtc-probe")]
    pub(super) rtc_continuity:
        std::rc::Rc<std::cell::RefCell<Option<super::rtc_member::Continuity>>>,
}
impl BrowserMemberConnection {
    fn recovery_qualified(&self, connection: &Connection) -> std::io::Result<()> {
        let gate = self.gate.0.lock().unwrap();
        let allowed = gate.saved.as_ref().is_some_and(|saved| {
            let Ok(invitation) = saved.invitation() else {
                return false;
            };
            let scope = AdmissionScope::new(*invitation.anchor.root.as_bytes(), saved.generation);
            connection.close_reason().is_none()
                && BrowserGate::permits(saved, connection.remote_id(), connection.alpn())
                && gate
                    .proofs
                    .iter()
                    .any(|proof| proof.applies_to(connection, &scope))
        });
        if allowed {
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "repair qualification changed",
            ))
        }
    }

    pub(super) async fn qualified_snapshot(&self) -> Result<SavedNode, JsValue> {
        if self.is_closed() {
            return Err(rejected());
        }
        let saved = self
            .gate
            .0
            .lock()
            .unwrap()
            .saved
            .clone()
            .ok_or_else(rejected)?;
        let current = serde_json::to_string(&saved).map_err(error)?;
        let scope = AdmissionScope::new(
            *saved.invitation()?.anchor.root.as_bytes(),
            saved.generation,
        );
        if read_node(self.lease).await?.as_string().as_deref() != Some(&current)
            || !BrowserGate::permits(&saved, self.connection.remote_id(), BUSINESS_ALPN)
            || !self
                .gate
                .0
                .lock()
                .unwrap()
                .proofs
                .iter()
                .any(|proof| proof.applies_to(&self.connection, &scope))
        {
            self.close();
            return Err(rejected());
        }
        Ok(saved)
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
        &self,
    ) -> Result<(tokio_util::sync::CancellationToken, RepairGuard<'_>), JsValue> {
        if self
            .repair_active
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return Err(error("member path improvement already active"));
        }
        let cancel = tokio_util::sync::CancellationToken::new();
        *self.repair_cancel.lock().unwrap() = Some(cancel.clone());
        let guard = RepairGuard {
            active: &self.repair_active,
            cancel: &self.repair_cancel,
        };
        Ok((cancel, guard))
    }

    async fn improve_path_until(
        &self,
        lease: &crate::session_maintenance::MaintenanceLease<()>,
        repair_contacts: bool,
    ) -> Result<(), JsValue> {
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
            return Err(error("active improvement has no current contact route"));
        }
        use crate::connection_coordinator::{AttemptRole, MemberPathAdapter};
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
                .connect_control_alpn(
                    target,
                    crate::connection_coordinator::ACTIVE_IMPROVEMENT_ALPN,
                    attempt.clone(),
                )
                .await
                .map_err(error)?;
                // Own exact C before any further await, including losing guard races.
                let _control = RequestConnection(Some(control.clone()));
                crate::connection_coordinator::improve_existing(
                    &self.endpoint,
                    &self.connection,
                    &control.path_handle(&self.endpoint),
                    &self.gate,
                    attempt,
                )
                .await
                .map_err(|cause| error(format!("active improvement failed: {cause:?}")))
            })
            .await;
        result.map_err(|cause| error(format!("active improvement failed: {cause:?}")))?
    }

    fn permits_repair_survival(&self, address: &iroh::TransportAddr) -> bool {
        if self.recovery_qualified(&self.connection).is_err() {
            return false;
        }
        match address {
            iroh::TransportAddr::Relay(url) => {
                let gate = self.gate.0.lock().unwrap();
                gate.saved.as_ref().is_some_and(|saved| {
                    origin(url.as_str())
                        .is_ok_and(|url| BrowserGate::permitted_relay_origins(saved).contains(&url))
                })
            }
            #[cfg(feature = "rtc-probe")]
            iroh::TransportAddr::Custom(address)
                if address.id() == crate::rtc_transport::TRANSPORT_ID =>
            {
                use iroh::endpoint::transports::CustomSender;
                self.rtc_transport
                    .is_data_path_qualified(address, self.connection.stable_id())
            }
            _ => false,
        }
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
        let (cancel, _guard) = self.begin_path_attempt()?;
        let attempt = crate::connection_coordinator::AttemptContext::new(deadline, cancel);
        let lease = self
            .gate
            .maintenance()
            .acquire(&self.connection, &attempt, ())
            .map_err(|cause| error(format!("active improvement reservation: {cause:?}")))?;
        self.improve_path_until(&lease, false).await?;
        Ok(
            serde_json::json!({"outcome":"committed","stableId":self.connection.stable_id()})
                .to_string(),
        )
    }

    /// Cancel only the outstanding explicit path-maintenance operation.
    pub fn cancel_path_recovery(&self) {
        if let Some(cancel) = self.repair_cancel.lock().unwrap().as_ref() {
            cancel.cancel();
        }
    }

    /// Repair this exact living M through the same restricted coordinator used
    /// for path improvement. A failed repair without a fresh surviving DATA
    /// path terminates this M; a later normal connect owns a new connection.
    pub async fn recover_path(&self, caller_ms: u32) -> Result<String, JsValue> {
        if self.is_closed() {
            return Ok(
                serde_json::json!({"outcome":"terminal","stableId":self.connection.stable_id()})
                    .to_string(),
            );
        }
        if caller_ms == 0 || u64::from(caller_ms) > crate::connection_budget::MAX_MS {
            return Err(error("repair caller budget must fit the browser budget"));
        }
        let started = n0_future::time::Instant::now();
        let deadline = (started + Duration::from_millis(caller_ms.into()))
            .min(started + crate::connection_recovery_core::REPAIR_TIMEOUT);
        // A concurrent operation refuses before taking outage ownership.
        let (cancel, _guard) = self.begin_path_attempt()?;
        let attempt = crate::connection_coordinator::AttemptContext::new(deadline, cancel);
        let lease = self
            .gate
            .maintenance()
            .acquire(&self.connection, &attempt, ())
            .map_err(|cause| error(format!("repair reservation: {cause:?}")))?;
        let surviving = crate::connection_coordinator::probe_existing_data_survival(
            &self.connection,
            |address| self.permits_repair_survival(address),
        );
        let result = n0_future::time::timeout(
            deadline.saturating_duration_since(n0_future::time::Instant::now()),
            self.improve_path_until(&lease, true),
        )
        .await
        .map_err(|_| error("repair caller deadline exceeded"))
        .and_then(|result| result);
        let outcome = if result.is_ok() && !self.is_closed() {
            "recovered"
        } else if self.is_closed() {
            "terminal"
        } else if surviving(&self.connection, &|address| {
            self.permits_repair_survival(address)
        }) || lease
            .run_ref(
                crate::connection_coordinator::validate_existing_data_survival(
                    &self.endpoint,
                    &self.connection,
                    deadline,
                    &|address| self.permits_repair_survival(address),
                    &|original| self.recovery_qualified(original),
                ),
            )
            .await
            .unwrap_or(false)
            || surviving(&self.connection, &|address| {
                self.permits_repair_survival(address)
            })
        {
            "unreachable"
        } else {
            self.connection.close(
                0u32.into(),
                b"same-session repair has no surviving data path",
            );
            self.connection.closed().await;
            "terminal"
        };
        Ok(serde_json::json!({"outcome":outcome,"stableId":self.connection.stable_id(),
            "controlAlpn":String::from_utf8_lossy(crate::connection_coordinator::ACTIVE_IMPROVEMENT_ALPN),
            "elapsedMs":started.elapsed().as_millis(),"callerBudgetMs":caller_ms,
            "failure":result.err().and_then(|cause|cause.as_string().or_else(||
                js_sys::Reflect::get(&cause,&JsValue::from_str("message")).ok()?.as_string()))}).to_string())
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
    active: &'a std::sync::atomic::AtomicBool,
    cancel: &'a Mutex<Option<tokio_util::sync::CancellationToken>>,
}
impl Drop for RepairGuard<'_> {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.lock().unwrap().take() {
            cancel.cancel();
        }
        self.active
            .store(false, std::sync::atomic::Ordering::Release);
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
        if self.saved.stage() != "Complete" {
            return Err(rejected());
        }
        Ok(BrowserMemberAcceptAttempt {
            endpoint: self.endpoint.clone(),
            gate: self.gate.clone(),
            incoming: self.incoming.clone(),
            lease: self.lease.as_ref().ok_or_else(rejected)?.0,
            saved: self.committed.clone(),
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
        if !BrowserGate::permits(&self.saved, target, BUSINESS_ALPN) {
            return Err(rejected());
        }
        Ok(BrowserMemberAttempt {
            endpoint: self.endpoint.clone(),
            gate: self.gate.clone(),
            lease: self.lease.as_ref().ok_or_else(rejected)?.0,
            saved: self.committed.clone(),
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
    lease: u32,
    saved: String,
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
            if read_node(self.lease).await?.as_string().as_deref() != Some(&self.saved) {
                return Err(error("browser materials changed during member attempt"));
            }
            use crate::connection_coordinator::{
                AttemptContext, AttemptRole, MemberPathAdapter, SessionPurpose,
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
                lease: self.lease,
                endpoint: self.endpoint.clone(),
                repair_active: std::sync::atomic::AtomicBool::new(false),
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
    pub(super) lease: u32,
    pub(super) saved: String,
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
            if read_node(self.lease).await?.as_string().as_deref() != Some(&self.saved) {
                return Err(error("browser materials changed during member attempt"));
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
            lease: self.lease,
            #[cfg(feature = "rtc-probe")]
            rtc_active: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            #[cfg(feature = "rtc-probe")]
            rtc_transport: self.rtc_transport.clone(),
            endpoint: self.endpoint.clone(),
            repair_contact: self.address.clone(),
            repair_active: std::sync::atomic::AtomicBool::new(false),
            repair_cancel: Mutex::new(None),
            #[cfg(feature = "rtc-probe")]
            rtc_live: std::rc::Rc::new(std::cell::RefCell::new(None)),
            #[cfg(feature = "rtc-probe")]
            rtc_continuity: std::rc::Rc::new(std::cell::RefCell::new(None)),
        })
    }
}
