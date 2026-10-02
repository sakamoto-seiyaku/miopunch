//! Browser platform adapter for the native manual-admission wire protocol.
//! Signatures, membership policy, SCRAM and TLS remain the shared implementations.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::endpoint::{AfterHandshakeOutcome, BeforeConnectOutcome, EndpointHooks};
use iroh::endpoint::{Connection, RecvStream, RelayResourcePolicy, RelayResourceUse, SendStream};
use iroh::{Endpoint, EndpointAddr, RelayMode, RelayUrl, endpoint::presets};
use iroh_base::SecretKey;
use p2panda_auth::Access;
use p2panda_core::{Hash, VerifyingKey};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use crate::admission_proof::{AdmissionScope, AdmissionSecret, BootstrapProof, prove_client};
use crate::management_auth::{ManagementAuth, NetworkAnchor};

const REQUEST_ALPN: &[u8] = b"miopunch/admission-request/1";
const BUSINESS_ALPN: &[u8] = b"miopunch/browser-member/1";
const TIMEOUT: Duration = Duration::from_secs(12);
const MAX_HISTORY: usize = 16 * 1024 * 1024;
type Records = Vec<(Vec<u8>, Option<Vec<u8>>)>;

#[wasm_bindgen(module = "/src/storage.js")]
extern "C" {
    #[wasm_bindgen(catch, js_name = acquireNode)]
    async fn acquire_node() -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = releaseNode)]
    fn release_node(token: u32);
    #[wasm_bindgen(catch, js_name = readNode)]
    async fn read_node(token: u32) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_name = saveNode)]
    async fn save_node(token: u32, previous: JsValue, next: String) -> Result<JsValue, JsValue>;
}

fn error(value: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&value.to_string())
}
fn rejected() -> JsValue {
    error("admission rejected or materials incomplete")
}

struct Lease(u32);
impl Drop for Lease {
    fn drop(&mut self) {
        release_node(self.0);
    }
}

/// One strict transaction stores each complete snapshot. Authorization and
/// secret have distinct commits; the second cannot precede the verified first.
#[derive(Clone, Serialize, Deserialize)]
struct SavedNode {
    version: u8,
    key: [u8; 32],
    invitation: Option<Vec<u8>>,
    request: [u8; 16],
    records: Records,
    authorization: Option<Hash>,
    generation: u64,
    secret: Option<[u8; 32]>,
    #[serde(default)]
    resources: Option<Configuration>,
}

#[derive(Clone, Copy)]
struct Invitation {
    anchor: NetworkAnchor,
    contact: iroh_base::EndpointId,
}
impl Invitation {
    fn decode(bytes: &[u8]) -> Result<Self, JsValue> {
        if bytes.len() != 96 {
            return Err(rejected());
        }
        Ok(Self {
            anchor: NetworkAnchor {
                root: VerifyingKey::from_bytes(&bytes[..32].try_into().unwrap()).map_err(error)?,
                genesis: Hash::from_bytes(bytes[32..64].try_into().unwrap()),
            },
            contact: iroh_base::EndpointId::from_bytes(&bytes[64..].try_into().unwrap())
                .map_err(error)?,
        })
    }
}

impl SavedNode {
    fn identity(&self) -> VerifyingKey {
        p2panda_core::SigningKey::from_bytes(&self.key).verifying_key()
    }
    fn invitation(&self) -> Result<Invitation, JsValue> {
        Invitation::decode(self.invitation.as_ref().ok_or_else(rejected)?)
    }
    fn verify(&self) -> Result<Option<ManagementAuth>, JsValue> {
        if self.version != 1 || self.generation > i64::MAX as u64 {
            return Err(rejected());
        }
        if self.records.is_empty() {
            if self.secret.is_some() || self.authorization.is_some() {
                return Err(rejected());
            }
            if let Some(bytes) = &self.invitation {
                Invitation::decode(bytes)?;
            }
            return Ok(None);
        }
        let invitation = self.invitation()?;
        let auth =
            ManagementAuth::replay_records(invitation.anchor, &self.records).map_err(error)?;
        let provider = VerifyingKey::from_bytes(invitation.contact.as_bytes()).map_err(error)?;
        if self.records.len() > 4096
            || Hash::digest(&self.records[0].0) != invitation.anchor.genesis
            || !auth.nodes().contains(&(provider, Access::manage()))
            || !auth.membership_authorization_valid(
                self.authorization.ok_or_else(rejected)?,
                self.identity(),
            )
        {
            return Err(rejected());
        }
        Ok(Some(auth))
    }
    fn stage(&self) -> &'static str {
        if self.authorization.is_none() {
            "AwaitingApproval"
        } else if self.secret.is_none() {
            "PendingMaterials"
        } else {
            "Complete"
        }
    }
}

#[derive(Default)]
struct GateState {
    saved: Option<SavedNode>,
    proofs: Vec<BootstrapProof>,
}
#[derive(Clone, Default)]
struct BrowserGate(Arc<Mutex<GateState>>);
impl std::fmt::Debug for BrowserGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BrowserGate")
    }
}
impl BrowserGate {
    fn install(&self, saved: &SavedNode) {
        self.0.lock().unwrap().saved = Some(saved.clone());
    }
    fn permits(saved: &SavedNode, target: iroh_base::EndpointId, alpn: &[u8]) -> bool {
        let Ok(invitation) = saved.invitation() else {
            return false;
        };
        target == invitation.contact
            && (alpn == REQUEST_ALPN || (alpn == BUSINESS_ALPN && saved.stage() == "Complete"))
    }
    fn reject() -> AfterHandshakeOutcome {
        AfterHandshakeOutcome::Reject {
            error_code: 0u32.into(),
            reason: b"browser admission rejected".to_vec(),
        }
    }
}
impl EndpointHooks for BrowserGate {
    async fn before_connect(&self, target: &EndpointAddr, alpn: &[u8]) -> BeforeConnectOutcome {
        if self
            .0
            .lock()
            .unwrap()
            .saved
            .as_ref()
            .is_some_and(|saved| Self::permits(saved, target.id, alpn))
        {
            BeforeConnectOutcome::Accept
        } else {
            BeforeConnectOutcome::Reject
        }
    }
    async fn after_handshake(&self, connection: &Connection) -> AfterHandshakeOutcome {
        let saved = self.0.lock().unwrap().saved.clone();
        let Some(saved) =
            saved.filter(|saved| Self::permits(saved, connection.remote_id(), connection.alpn()))
        else {
            return Self::reject();
        };
        // No 0-RTT API is exposed. Normal data handles wait for the native
        // mutual proof and stage agreement, including after a fresh page start.
        let proof = if connection.alpn() == BUSINESS_ALPN {
            let Ok(invitation) = saved.invitation() else {
                return Self::reject();
            };
            let scope = AdmissionScope::new(*invitation.anchor.root.as_bytes(), saved.generation);
            let Some(secret) = saved.secret else {
                return Self::reject();
            };
            match prove_client(connection, &scope, &AdmissionSecret::from_saved(secret)).await {
                Ok(proof) => Some(proof),
                Err(_) => return Self::reject(),
            }
        } else {
            None
        };
        let agreed = n0_future::time::timeout(Duration::from_secs(5), async {
            let (mut send, mut recv) = connection.open_bi().await.ok()?;
            send.write_all(&[1]).await.ok()?;
            send.finish().ok()?;
            let mut response = [0];
            recv.read_exact(&mut response).await.ok()?;
            (response == [1]).then_some(())
        })
        .await
        .ok()
        .flatten()
        .is_some();
        if !agreed {
            return Self::reject();
        }
        if let Some(proof) = proof {
            let mut gate = self.0.lock().unwrap();
            gate.proofs.retain(BootstrapProof::is_live);
            if gate.proofs.len() >= 32 {
                return Self::reject();
            }
            gate.proofs.push(proof);
            AfterHandshakeOutcome::AcceptData {
                allowed_data_custom_ids: Vec::new(),
            }
        } else {
            AfterHandshakeOutcome::AcceptRestricted
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Configuration {
    relays: Vec<String>,
    disabled: Vec<String>,
}
impl Default for Configuration {
    fn default() -> Self {
        Self {
            relays: iroh::defaults::prod::default_relay_map()
                .urls::<Vec<_>>()
                .into_iter()
                .map(|url| url.to_string())
                .collect(),
            disabled: Vec::new(),
        }
    }
}
#[derive(Deserialize)]
struct ConfigurationInput {
    relays: Option<Vec<String>>,
    disabled: Option<Vec<String>>,
}

fn origin(value: &str) -> Result<String, JsValue> {
    let url: RelayUrl = value.parse().map_err(error)?;
    if !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.scheme(), "http" | "https")
    {
        return Err(error("invalid relay origin"));
    }
    Ok(format!(
        "{}://{}:{}",
        url.scheme(),
        url.host_str().ok_or_else(rejected)?.trim_end_matches('.'),
        url.port_or_known_default().ok_or_else(rejected)?
    ))
}

struct Delivery {
    connection: Connection,
    send: SendStream,
    recv: RecvStream,
}
impl Drop for Delivery {
    fn drop(&mut self) {
        self.connection
            .close(0u32.into(), b"browser delivery ended");
    }
}

#[wasm_bindgen]
pub struct BrowserNetwork {
    endpoint: Endpoint,
    relays: Vec<RelayUrl>,
    gate: BrowserGate,
    lease: Option<Lease>,
    saved: SavedNode,
    committed: String,
    delivery: Option<Delivery>,
}

#[wasm_bindgen]
impl BrowserNetwork {
    /// Configuration is a startup snapshot. Allowed relays are replaceable;
    /// no default facility is silently installed when none can be used.
    pub async fn start(configuration: String) -> Result<BrowserNetwork, JsValue> {
        let input: ConfigurationInput = serde_json::from_str(&configuration).map_err(error)?;
        let lease = Lease(acquire_node().await?.as_f64().ok_or_else(rejected)? as u32);
        let raw = read_node(lease.0).await?;
        let mut saved: SavedNode = if let Some(raw) = raw.as_string() {
            if raw.len() > 80 * 1024 * 1024 {
                return Err(error("browser state too large"));
            }
            serde_json::from_str(&raw).map_err(|_| {
                error("browser materials corrupt; restore backup or explicitly clear and rejoin")
            })?
        } else {
            SavedNode {
                version: 1,
                key: SecretKey::generate().to_bytes(),
                invitation: None,
                request: rand::random(),
                records: Vec::new(),
                authorization: None,
                generation: 0,
                secret: None,
                resources: None,
            }
        };
        saved.verify()?;
        let mut configuration = saved.resources.clone().unwrap_or_default();
        if let Some(relays) = input.relays {
            configuration.relays = relays;
        }
        if let Some(disabled) = input.disabled {
            configuration.disabled = disabled;
        }
        if configuration.relays.len() > 16 || configuration.disabled.len() > 64 {
            return Err(error("too many relay resources"));
        }
        let disabled: Vec<_> = configuration
            .disabled
            .iter()
            .map(|url| origin(url))
            .collect::<Result<_, _>>()?;
        let mut relays = Vec::new();
        for value in &configuration.relays {
            if !disabled.contains(&origin(value)?) {
                relays.push(value.parse::<RelayUrl>().map_err(error)?);
            }
        }
        saved.resources = Some(configuration);
        let committed = serde_json::to_string(&saved).map_err(error)?;
        let previous = raw
            .as_string()
            .map_or(JsValue::NULL, |raw| JsValue::from_str(&raw));
        if raw.as_string().as_deref() != Some(&committed) {
            save_node(lease.0, previous, committed.clone()).await?;
        }
        if relays.is_empty() {
            return Err(error("temporarily unreachable: no allowed browser relay"));
        }
        let allowed: Vec<_> = relays
            .iter()
            .map(|url| origin(url.as_str()))
            .collect::<Result<_, _>>()?;
        let gate = BrowserGate::default();
        gate.install(&saved);
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(SecretKey::from_bytes(&saved.key))
            .relay_mode(RelayMode::custom(relays.clone()))
            .relay_resource_policy(RelayResourcePolicy::new(move |url, usage| {
                matches!(usage, RelayResourceUse::DataRelay)
                    && origin(url.as_str()).is_ok_and(|origin| allowed.contains(&origin))
            }))
            .admission_gate(gate.clone())
            .bind()
            .await
            .map_err(error)?;
        if n0_future::time::timeout(TIMEOUT, endpoint.online())
            .await
            .is_err()
        {
            endpoint.close().await;
            return Err(error(
                "temporarily unreachable: allowed browser relays unavailable",
            ));
        }
        Ok(Self {
            endpoint,
            relays,
            gate,
            lease: Some(lease),
            saved,
            committed,
            delivery: None,
        })
    }

    pub fn id(&self) -> String {
        self.endpoint.id().to_string()
    }
    pub fn status(&self) -> String {
        serde_json::json!({"id":self.id(), "stage":self.saved.stage(), "authorization_saved":self.saved.authorization.is_some(), "secret_saved":self.saved.secret.is_some(), "running":self.lease.is_some()}).to_string()
    }

    pub async fn request_admission(&mut self, invitation: String) -> Result<String, JsValue> {
        self.delivery = None;
        let bytes = hex::decode(invitation).map_err(error)?;
        Invitation::decode(&bytes)?;
        if self
            .saved
            .invitation
            .as_ref()
            .is_some_and(|saved| saved != &bytes)
        {
            return Err(error("another network is already selected"));
        }
        let mut next = self.saved.clone();
        next.invitation = Some(bytes);
        self.commit(next).await?;
        let result = n0_future::time::timeout(TIMEOUT, self.request_inner()).await;
        result.map_err(|_| error("temporarily unreachable; durable admission progress retained"))?
    }

    pub async fn complete_admission(&mut self) -> Result<String, JsValue> {
        let mut delivery = self.delivery.take().ok_or_else(rejected)?;
        let result = n0_future::time::timeout(TIMEOUT, async {
            self.saved.verify()?;
            delivery.send.write_all(b"AUTH").await.map_err(error)?;
            let mut material = [0; 40];
            delivery
                .recv
                .read_exact(&mut material)
                .await
                .map_err(error)?;
            if u64::from_be_bytes(material[..8].try_into().unwrap()) != self.saved.generation {
                return Err(rejected());
            }
            let mut next = self.saved.clone();
            next.secret = Some(material[8..].try_into().unwrap());
            self.commit(next).await?;
            delivery.send.write_all(b"SAVE").await.map_err(error)?;
            delivery.send.finish().map_err(error)?;
            let mut reply = [0; 4];
            delivery.recv.read_exact(&mut reply).await.map_err(error)?;
            if &reply != b"DONE" {
                return Err(rejected());
            }
            Ok(self.saved.stage().to_string())
        })
        .await
        .map_err(|_| error("delivery interrupted; committed materials retained"))?;
        result
    }

    /// Controlled member protocol, with a fresh native gate qualification.
    pub async fn exchange(&self, message: String) -> Result<String, JsValue> {
        if message.len() > 65536 || self.saved.stage() != "Complete" {
            return Err(rejected());
        }
        self.saved.verify()?;
        self.check_store().await?;
        n0_future::time::timeout(TIMEOUT, async {
            let connection = self.connect(BUSINESS_ALPN).await?;
            let result = async {
                let (mut send, mut recv) = connection.open_bi().await.map_err(error)?;
                send.write_all(message.as_bytes()).await.map_err(error)?;
                send.finish().map_err(error)?;
                let reply = recv.read_to_end(65536).await.map_err(error)?;
                String::from_utf8(reply).map_err(error)
            }
            .await;
            connection.close(0u32.into(), b"browser exchange ended");
            result
        })
        .await
        .map_err(|_| error("temporarily unreachable: member data path unavailable"))?
    }

    /// Disconnect only. Closing, freezing or reclaiming a page never authors
    /// a membership Remove, and successful saved admission survives restart.
    pub async fn close(&mut self) {
        self.delivery = None;
        self.endpoint.close().await;
        self.lease = None;
    }
}

impl BrowserNetwork {
    async fn check_store(&self) -> Result<(), JsValue> {
        let token = self
            .lease
            .as_ref()
            .ok_or_else(|| error("browser node disconnected"))?
            .0;
        if read_node(token).await?.as_string().as_deref() != Some(&self.committed) {
            return Err(error(
                "browser materials cleared or changed; restore or rejoin",
            ));
        }
        Ok(())
    }
    async fn commit(&mut self, next: SavedNode) -> Result<(), JsValue> {
        next.verify()?;
        let token = self.lease.as_ref().ok_or_else(rejected)?.0;
        let raw = serde_json::to_string(&next).map_err(error)?;
        save_node(token, JsValue::from_str(&self.committed), raw.clone()).await?;
        self.saved = next;
        self.committed = raw;
        self.gate.install(&self.saved);
        Ok(())
    }
    async fn connect(&self, alpn: &[u8]) -> Result<Connection, JsValue> {
        self.check_store().await?;
        let invitation = self.saved.invitation()?;
        let mut last = error("temporarily unreachable: no allowed relay path");
        for relay in &self.relays {
            let addr = EndpointAddr::new(invitation.contact).with_relay_url(relay.clone());
            match self.endpoint.connect(addr, alpn).await {
                Ok(connection) => return Ok(connection),
                Err(_) => last = error("temporarily unreachable: allowed relay path failed"),
            }
        }
        Err(last)
    }
    async fn request_inner(&mut self) -> Result<String, JsValue> {
        let connection = self.connect(REQUEST_ALPN).await?;
        let result = async {
            let (mut send, mut recv) = connection.open_bi().await.map_err(error)?;
            let mut wire = self.saved.invitation.clone().ok_or_else(rejected)?;
            wire.extend_from_slice(&self.saved.request);
            wire.extend_from_slice(self.saved.identity().as_bytes());
            send.write_all(&wire).await.map_err(error)?;
            let mut status = [0];
            recv.read_exact(&mut status).await.map_err(error)?;
            if status == [0] {
                send.finish().map_err(error)?;
                return Ok("AwaitingApproval".to_string());
            }
            if status != [1] {
                return Err(rejected());
            }
            let mut generation = [0; 8];
            recv.read_exact(&mut generation).await.map_err(error)?;
            let generation = u64::from_be_bytes(generation);
            if generation < self.saved.generation || generation > i64::MAX as u64 {
                return Err(rejected());
            }
            let mut len = [0; 4];
            recv.read_exact(&mut len).await.map_err(error)?;
            let len = u32::from_be_bytes(len) as usize;
            if len == 0 || len > MAX_HISTORY {
                return Err(rejected());
            }
            let mut history = vec![0; len];
            recv.read_exact(&mut history).await.map_err(error)?;
            let (authorization, records): (Hash, Records) =
                p2panda_core::cbor::decode_cbor(history.as_slice()).map_err(error)?;
            let mut next = self.saved.clone();
            next.records = records;
            next.authorization = Some(authorization);
            next.generation = generation;
            // A newly received grant is never completed by a previously saved
            // secret, even when the identity is retained across a rejoin.
            next.secret = None;
            self.commit(next).await?;
            self.delivery = Some(Delivery {
                connection: connection.clone(),
                send,
                recv,
            });
            Ok(self.saved.stage().to_string())
        }
        .await;
        if self.delivery.is_none() {
            connection.close(0u32.into(), b"browser request ended");
        }
        result
    }
}
