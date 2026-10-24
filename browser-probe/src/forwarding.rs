//! Bounded browser access to a Native provider; no host socket/listener claims.
use super::*;
use crate::forwarding::{self, Protocol};
use futures_util::FutureExt;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
pub(super) struct BrowserForwarding {
    next: Cell<u64>,
    closed: Cell<bool>,
    active: RefCell<BTreeMap<u64, (CancellationToken, Option<Connection>)>>,
    joined: Notify,
}
struct ExchangeOwner {
    runtime: Rc<BrowserForwarding>,
    id: u64,
    cancel: CancellationToken,
}
impl Drop for ExchangeOwner {
    fn drop(&mut self) {
        if let Some((_, Some(connection))) = self.runtime.active.borrow_mut().remove(&self.id) {
            connection.close(0u32.into(), b"browser forwarding exchange ended");
        }
        self.runtime.joined.notify_waiters();
    }
}
impl BrowserForwarding {
    fn claim(self: &Rc<Self>) -> Result<ExchangeOwner, JsValue> {
        if self.closed.get() || self.active.borrow().len() >= 4 {
            return Err(error("forward exchange unavailable or busy"));
        }
        let id = self
            .next
            .get()
            .checked_add(1)
            .ok_or_else(|| error("forward identifier exhausted"))?;
        self.next.set(id);
        let cancel = CancellationToken::new();
        self.active.borrow_mut().insert(id, (cancel.clone(), None));
        Ok(ExchangeOwner {
            runtime: self.clone(),
            id,
            cancel,
        })
    }
    fn cancel(&self) {
        for (cancel, connection) in self.active.borrow().values() {
            cancel.cancel();
            if let Some(connection) = connection {
                connection.close(0u32.into(), b"browser forward stopped");
            }
        }
    }
    pub async fn stop(&self) {
        self.cancel();
        while !self.active.borrow().is_empty() {
            let joined = self.joined.notified();
            if self.active.borrow().is_empty() {
                break;
            }
            joined.await;
        }
    }
    pub async fn close(&self) {
        self.closed.set(true);
        self.stop().await;
    }
}
impl Drop for BrowserForwarding {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[wasm_bindgen]
impl BrowserNetwork {
    pub fn forwarding_status(&self) -> String {
        serde_json::json!({"native_listener":false,"provider":false,"bounded_exchange":true,
            "max_exchange_bytes":forwarding::MAX_EXCHANGE,"max_exchanges":4,"active_exchanges":self.forwarding.active.borrow().len(),
            "available":!self.forwarding.closed.get()}).to_string()
    }
    pub async fn stop_forwarding(&self) {
        self.forwarding.stop().await;
    }
    /// One bounded TCP request/response or one UDP datagram. This never binds
    /// a native host port for other applications and never offers a provider.
    pub async fn forward_exchange(
        &self,
        peer: String,
        protocol: String,
        target: String,
        payload: Vec<u8>,
        response_bytes: u32,
    ) -> Result<Vec<u8>, JsValue> {
        self.require_device_ready()?;
        self.check_store().await?;
        if payload.len() > forwarding::MAX_EXCHANGE
            || response_bytes as usize > forwarding::MAX_EXCHANGE
        {
            return Err(error("forward exchange too large"));
        }
        let protocol: Protocol =
            serde_json::from_value(serde_json::Value::String(protocol)).map_err(error)?;
        let target = target.parse().map_err(error)?;
        let peer = peer.parse().map_err(error)?;
        let owner = self.forwarding.claim()?;
        let work = async {
            let connection = self.connect_target(peer, protocol.alpn()).await?;
            if owner.cancel.is_cancelled() {
                connection.close(0u32.into(), b"browser forward stopped");
                return Err(error("forward stopped"));
            }
            self.forwarding
                .active
                .borrow_mut()
                .get_mut(&owner.id)
                .ok_or_else(rejected)?
                .1 = Some(connection.clone());
            let stream = forwarding::open(&connection, target).await.map_err(error)?;
            forwarding::exchange(stream, protocol, &payload, response_bytes as usize)
                .await
                .map_err(error)
        }
        .fuse();
        let stop = owner.cancel.cancelled().fuse();
        futures_util::pin_mut!(work, stop);
        futures_util::select_biased! {
            _ = stop => Err(error("forward stopped")),
            result = work => result,
        }
    }
}
