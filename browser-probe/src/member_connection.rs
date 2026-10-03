//! Qualified controlled member connection exposed by the browser SDK.
use super::*;
#[wasm_bindgen]
pub struct BrowserMemberConnection {
    pub(super) connection: Connection,
    gate: BrowserGate,
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
        let connection = self
            .connect_target(target.parse().map_err(error)?, BUSINESS_ALPN)
            .await?;
        Ok(BrowserMemberConnection {
            connection,
            gate: self.gate.clone(),
            lease: self.lease.as_ref().ok_or_else(rejected)?.0,
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
