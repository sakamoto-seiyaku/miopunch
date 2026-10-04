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
    ) -> (
        Arc<std::sync::atomic::AtomicU64>,
        u64,
        n0_watcher::Direct<u64>,
    ) {
        let gate = self.gate.0.lock().unwrap();
        let revision = gate.qualification_revision.clone();
        let observed = revision.load(std::sync::atomic::Ordering::Acquire);
        let updates = gate
            .qualification_updates
            .as_ref()
            .expect("installed browser qualification")
            .watch();
        (revision, observed, updates)
    }
}
#[wasm_bindgen]
impl BrowserMemberConnection {
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
        if caller_ms == 0 || caller_ms > TIMEOUT.as_millis() as u32 {
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
    pub fn is_closed(&self) -> bool {
        self.connection.close_reason().is_some()
    }
    pub fn close(&self) {
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
pub struct BrowserMemberAttempt {
    endpoint: Endpoint,
    gate: BrowserGate,
    lease: u32,
    saved: String,
    address: EndpointAddr,
    deadline: n0_future::time::Instant,
    cancel: tokio_util::sync::CancellationToken,
    started: std::sync::atomic::AtomicBool,
    #[cfg(feature = "rtc-probe")]
    rtc_transport: crate::rtc_transport::RtcTransport,
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
            let options = crate::member_path_protocol::MemberProtocol::Business
                .connect_options()
                .with_caller_deadline(self.deadline)
                .with_attempt_cancellation(self.cancel.clone());
            self.endpoint
                .connect_with_opts(self.address.clone(), BUSINESS_ALPN, options)
                .await
                .map_err(error)?
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
