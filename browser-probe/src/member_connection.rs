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
    pub(super) endpoint: Endpoint,
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
        if self
            .repair_active
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return Err(error("member path improvement already active"));
        }
        let cancel = tokio_util::sync::CancellationToken::new();
        *self.repair_cancel.lock().unwrap() = Some(cancel.clone());
        let _guard = RepairGuard {
            active: &self.repair_active,
            cancel: &self.repair_cancel,
        };
        let deadline = n0_future::time::Instant::now() + Duration::from_millis(caller_ms.into());
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
        if target.addrs.is_empty() {
            return Err(error("active improvement has no current contact route"));
        }
        use crate::connection_coordinator::{AttemptContext, AttemptRole, MemberPathAdapter};
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
        let attempt = AttemptContext::new(deadline, cancel.clone());
        let registry = self.gate.maintenance();
        let lease = registry
            .acquire(&self.connection, &attempt, ())
            .map_err(|cause| error(format!("active improvement reservation: {cause:?}")))?;
        let result = lease
            .run(async {
                let control = crate::connection_coordinator::ConnectionCoordinator::new(
                    self.endpoint.clone(),
                    caps.permitted,
                    contact_ids,
                )
                .with_member_data(crate::member_data::DATA_TRANSPORT_ID)
                .connect_alpn(
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
                    &control,
                    &self.gate,
                    attempt,
                )
                .await
                .map_err(|cause| error(format!("active improvement failed: {cause:?}")))
            })
            .await;
        result
            .map_err(|cause| error(format!("active improvement failed: {cause:?}")))?
            .map(|_| {
                serde_json::json!({"outcome":"committed","stableId":self.connection.stable_id()})
                    .to_string()
            })
    }

    /// Cancel only the outstanding explicit path-maintenance operation.
    pub fn cancel_path_recovery(&self) {
        if let Some(cancel) = self.repair_cancel.lock().unwrap().as_ref() {
            cancel.cancel();
        }
    }

    /// Repair a carrier on this existing M through the original protected
    /// recovery protocol. It never returns a new ordinary Connection.
    pub async fn recover_path(&self, caller_ms: u32) -> Result<String, JsValue> {
        use crate::connection_recovery_core as recovery;
        if self.is_closed() {
            return Ok(
                serde_json::json!({"outcome":"terminal", "stableId":self.connection.stable_id()})
                    .to_string(),
            );
        }
        if caller_ms == 0 || u64::from(caller_ms) > crate::connection_budget::MAX_MS {
            return Err(error("repair caller budget must fit the browser budget"));
        }
        if self
            .repair_active
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return Err(error("original M repair already active"));
        }
        let cancel = tokio_util::sync::CancellationToken::new();
        *self.repair_cancel.lock().unwrap() = Some(cancel.clone());
        let _guard = RepairGuard {
            active: &self.repair_active,
            cancel: &self.repair_cancel,
        };
        let started = n0_future::time::Instant::now();
        let deadline = started + Duration::from_millis(u64::from(caller_ms));
        let work = async {
            self.qualified_snapshot().await?;
            let mut contact = self.repair_contact.clone();
            // Prefer the explicit private signaling contact for repair when
            // available; the original ordinary M still cannot use it as data.
            if contact.addrs.iter().any(|path| matches!(path, iroh::TransportAddr::Custom(addr)
                if matches!(addr.id(), crate::mqtt_datagram::TRANSPORT_ID | crate::irc_datagram::TRANSPORT_ID))) {
                contact.addrs.retain(|path| matches!(path, iroh::TransportAddr::Custom(addr)
                    if matches!(addr.id(), crate::mqtt_datagram::TRANSPORT_ID | crate::irc_datagram::TRANSPORT_ID)));
            }
            let permits = |path: &iroh::TransportAddr| match path {
                iroh::TransportAddr::Relay(url) => {
                    let gate = self.gate.0.lock().unwrap();
                    gate.saved.as_ref().is_some_and(|saved| {
                        origin(url.as_str()).is_ok_and(|url| {
                            BrowserGate::permitted_relay_origins(saved).contains(&url)
                        })
                    })
                }
                _ => false,
            };
            let result = recovery::recover(
                &self.endpoint,
                &self.connection,
                contact,
                self.endpoint.addr(),
                deadline,
                cancel.clone(),
                true,
                permits,
                |original| self.recovery_qualified(original),
                |original, control| {
                    self.recovery_qualified(original)?;
                    if control.alpn() != recovery::CONNECTION_RECOVERY_ALPN
                        || control.remote_id() != original.remote_id()
                    {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            "repair control differs from original peer/scope",
                        ));
                    }
                    self.recovery_qualified(control)
                },
            )
            .await;
            match result {
                Ok(result) => {
                    self.qualified_snapshot().await?;
                    #[cfg(feature = "rtc-probe")]
                    if self.rtc_transport.retire_connection(&self.connection) {
                        for path in self.connection.paths().iter().filter(|path| matches!(path.remote_addr(),
                            iroh::TransportAddr::Custom(addr) if addr.id() == crate::rtc_transport::TRANSPORT_ID)) {
                            let _ = path.close();
                        }
                    }
                    Ok(serde_json::json!({"outcome":"recovered", "stableId":result.stable_id,
                        "pathId":result.path_id,"path":format!("{:?}",result.path),
                        "freshAck":true,"probeWatermark":result.probe_watermark,
                        "peerConfirmed":result.peer_confirmed,
                        "controlStableId":result.control_stable_id,"controlAlpn":String::from_utf8_lossy(recovery::CONNECTION_RECOVERY_ALPN),
                        "elapsedMs":started.elapsed().as_millis(),"callerBudgetMs":caller_ms}).to_string())
                },
                Err(cause) => Ok(serde_json::json!({"outcome":if self.is_closed(){"terminal"}else{"unreachable"},
                    "stableId":self.connection.stable_id(),"failure":cause.to_string(),
                    "elapsedMs":started.elapsed().as_millis(),"callerBudgetMs":caller_ms}).to_string()),
            }
        };
        match n0_future::time::timeout(
            deadline.saturating_duration_since(n0_future::time::Instant::now()),
            work,
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Ok(
                serde_json::json!({"outcome":if self.is_closed(){"terminal"}else{"unreachable"},
                "stableId":self.connection.stable_id(),"failure":"repair caller deadline exceeded",
                "elapsedMs":started.elapsed().as_millis(),"callerBudgetMs":caller_ms})
                .to_string(),
            ),
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
    endpoint: Endpoint,
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
            let coordinator = crate::connection_coordinator::ConnectionCoordinator::new(
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
    pub(super) endpoint: Endpoint,
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
            let coordinator = crate::connection_coordinator::ConnectionCoordinator::new(
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
