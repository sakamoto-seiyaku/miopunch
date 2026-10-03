mod custom_datagram;
use iroh::{Endpoint, EndpointAddr, RelayMode, endpoint::presets};
use iroh_base::{PublicKey, RelayUrl, SecretKey};
use p2panda_core::SigningKey;
use wasm_bindgen::prelude::*;

// Use the native verifier and admission proof unchanged on the browser platform.
#[path = "../../src/admission_proof.rs"]
#[allow(dead_code)] // The browser uses the client qualification from this shared module.
pub mod admission_proof;
mod irc;
#[path = "../../src/irc_datagram.rs"]
pub mod irc_datagram;
#[path = "../../src/management_auth.rs"]
#[allow(dead_code)] // Durable native-store helpers are outside this platform adapter.
pub mod management_auth;
#[path = "../../src/member_path_protocol.rs"]
pub mod member_path_protocol;
mod mqtt;
#[path = "../../src/mqtt_datagram.rs"]
pub mod mqtt_datagram;
mod network;
pub use mqtt::BrowserMqttDatagram;
pub use network::{BrowserMemberConnection, BrowserNetwork};

const KEY_NAME: &str = "miopunch-browser-probe-node-key";
const ALPN: &[u8] = b"miopunch/browser-probe/1";

fn error(value: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&value.to_string())
}

fn browser_key() -> Result<SecretKey, JsValue> {
    let storage = web_sys::window()
        .ok_or_else(|| error("missing browser window"))?
        .local_storage()?
        .ok_or_else(|| error("browser localStorage is unavailable"))?;
    if let Some(saved) = storage.get_item(KEY_NAME)? {
        let raw = hex::decode(saved).map_err(error)?;
        let bytes: [u8; 32] = raw
            .try_into()
            .map_err(|_| error("invalid saved key length"))?;
        return Ok(SecretKey::from_bytes(&bytes));
    }

    let key = SecretKey::generate();
    storage.set_item(KEY_NAME, &hex::encode(key.to_bytes()))?;
    Ok(key)
}

#[wasm_bindgen]
pub struct BrowserNode {
    endpoint: Endpoint,
    relay_url: RelayUrl,
}

#[wasm_bindgen]
impl BrowserNode {
    pub async fn start(relay_url: String) -> Result<BrowserNode, JsValue> {
        let relay_url: RelayUrl = relay_url.parse().map_err(error)?;
        let key = browser_key()?;
        let panda_key = SigningKey::from_bytes(&key.to_bytes());
        if panda_key.verifying_key().as_bytes() != key.public().as_bytes() {
            return Err(error("p2panda and iroh identities disagree"));
        }
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(key)
            .relay_mode(RelayMode::custom([relay_url.clone()]))
            .bind()
            .await
            .map_err(error)?;
        endpoint.online().await;
        Ok(Self {
            endpoint,
            relay_url,
        })
    }

    pub fn id(&self) -> String {
        self.endpoint.id().to_string()
    }

    pub async fn ping(&self, target: String) -> Result<String, JsValue> {
        let target: PublicKey = target.parse().map_err(error)?;
        let addr = EndpointAddr::new(target).with_relay_url(self.relay_url.clone());
        let connection = self.endpoint.connect(addr, ALPN).await.map_err(error)?;
        let (mut send, mut recv) = connection.open_bi().await.map_err(error)?;
        send.write_all(b"PING").await.map_err(error)?;
        send.finish().map_err(error)?;
        let reply = recv.read_to_end(64).await.map_err(error)?;
        connection.close(0u32.into(), b"done");
        String::from_utf8(reply).map_err(error)
    }

    pub async fn close(&self) {
        self.endpoint.close().await;
    }
}

#[path = "../../vendor/p2panda-net/src/codec.rs"]
#[allow(dead_code)]
mod codec;
#[path = "../../src/management_record.rs"]
mod management_record;
#[path = "../../src/replica_snapshot.rs"]
mod replica_snapshot;

#[path = "../../src/admission_request_id.rs"]
mod admission_request_id;

#[path = "../../src/management_action.rs"]
mod management_action;

#[path = "../../src/precode_policy.rs"]
mod precode_policy;

#[cfg(feature = "rtc-probe")]
#[path = "../../src/rtc_association.rs"]
mod rtc_association;
#[cfg(feature = "rtc-probe")]
#[path = "../../src/rtc_transport.rs"]
mod rtc_transport;
#[cfg(feature = "rtc-probe")]
#[path = "../../src/rtc_wire.rs"]
mod rtc_wire;
