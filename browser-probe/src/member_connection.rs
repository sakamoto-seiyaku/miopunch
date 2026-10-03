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
    #[cfg(feature = "rtc-probe")]
    pub(super) endpoint: Endpoint,
    #[cfg(feature = "rtc-probe")]
    pub(super) rtc_live: std::rc::Rc<std::cell::RefCell<Option<super::rtc_member::OwnedRtc>>>,
    #[cfg(feature = "rtc-probe")]
    pub(super) rtc_continuity:
        std::rc::Rc<std::cell::RefCell<Option<super::rtc_member::Continuity>>>,
}
impl BrowserMemberConnection {
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
            #[cfg(feature = "rtc-probe")]
            endpoint: self.endpoint.clone(),
            #[cfg(feature = "rtc-probe")]
            rtc_live: std::rc::Rc::new(std::cell::RefCell::new(None)),
            #[cfg(feature = "rtc-probe")]
            rtc_continuity: std::rc::Rc::new(std::cell::RefCell::new(None)),
        })
    }
}
