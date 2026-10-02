//! Qualified controlled member connection exposed by the browser SDK.
use super::*;
#[wasm_bindgen]
pub struct BrowserMemberConnection {
    connection: Connection,
    gate: BrowserGate,
    lease: u32,
}
#[wasm_bindgen]
impl BrowserMemberConnection {
    pub fn is_closed(&self) -> bool {
        self.connection.close_reason().is_some()
    }
    pub fn close(&self) {
        self.connection
            .close(0u32.into(), b"browser caller disconnected");
    }
    pub async fn exchange(&self, message: String) -> Result<String, JsValue> {
        if message.len() > 65536 || self.is_closed() {
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
        if read_node(self.lease).await?.as_string().as_deref() != Some(&current)
            || !BrowserGate::permits(&saved, self.connection.remote_id(), BUSINESS_ALPN)
        {
            self.close();
            return Err(rejected());
        }
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
        })
    }
}
