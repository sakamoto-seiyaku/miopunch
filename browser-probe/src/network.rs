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

use crate::admission_proof::{
    AdmissionScope, AdmissionSecret, BootstrapProof, prove_client, prove_server,
};
use crate::management_auth::{ManagementAuth, NetworkAnchor};

const REPLICA_ALPN: &[u8] = b"miopunch/control-replica/1";
const REQUEST_ALPN: &[u8] = b"miopunch/admission-request/1";
const BUSINESS_ALPN: &[u8] = crate::connection_coordinator::MEMBER_ALPN;
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
    #[serde(default)]
    pending: Records,
    #[serde(default)]
    precode: Option<[u8; 32]>,
    #[serde(default)]
    codes: Vec<admission::Code>,
    #[serde(default)]
    approvals: Vec<admission::Approval>,
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
        self.verify_ledger()?;
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
        if self.records.len() + self.pending.len() > 4096 {
            return Err(rejected());
        }
        for (header, body) in &self.pending {
            crate::management_auth::decode_complete(header, body.as_deref(), invitation.anchor)
                .map_err(error)?;
        }
        let auth =
            ManagementAuth::replay_records(invitation.anchor, &self.records).map_err(error)?;
        if self.records.len() > 4096
            || Hash::digest(&self.records[0].0) != invitation.anchor.genesis
        {
            return Err(rejected());
        }
        Ok(Some(auth))
    }
    fn stage(&self) -> &'static str {
        if self.authorization.is_none() {
            "AwaitingApproval"
        } else if self.verify().ok().flatten().is_some_and(|auth| {
            !auth.membership_authorization_valid(self.authorization.unwrap(), self.identity())
        }) {
            "Removed"
        } else if self.secret.is_none() {
            "PendingMaterials"
        } else {
            "Complete"
        }
    }
}

#[derive(Default)]
struct GateState {
    rtc_capability: Option<crate::rtc_capability::RtcCapability>,
    saved: Option<SavedNode>,
    proofs: Vec<BootstrapProof>,
    connections: Vec<iroh::endpoint::WeakConnectionHandle>,
    qualification_revision: Arc<std::sync::atomic::AtomicU64>,
    qualification_updates: Option<n0_watcher::Watchable<u64>>,
    snapshot_hash: Option<[u8; 32]>,
    request_handshake: Option<serde_json::Value>,
    request_deadline: Option<n0_future::time::Instant>,
    lease: Option<u32>,
    #[cfg(feature = "rtc-probe")]
    rtc: Option<crate::rtc_transport::RtcTransport>,
    member_reports: std::collections::BTreeMap<usize, serde_json::Value>,
    #[cfg(feature = "rtc-probe")]
    rtc_qualifications: std::collections::BTreeMap<usize, RtcQualificationSource>,
}

/// The effective permission of one proved original M. Saved record changes
/// still advance the source diagnostic revision, but cannot revoke another
/// still-permitted M's path. Retirement is terminal for every issued token.
#[cfg(feature = "rtc-probe")]
struct RtcQualificationSource {
    original: iroh::endpoint::WeakConnectionHandle,
    revision: Arc<std::sync::atomic::AtomicU64>,
    updates: n0_watcher::Watchable<u64>,
    retired: bool,
}
#[cfg(feature = "rtc-probe")]
impl RtcQualificationSource {
    fn retire(&mut self) {
        if self.retired {
            return;
        }
        self.retired = true;
        let previous = self
            .revision
            .fetch_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |revision| revision.checked_add(1),
            )
            .expect("browser M permission revision exhausted");
        let _ = self.updates.set(previous + 1);
    }
}
#[cfg(feature = "rtc-probe")]
impl Drop for RtcQualificationSource {
    fn drop(&mut self) {
        self.retire();
    }
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
        let mut gate = self.0.lock().unwrap();
        let snapshot_hash =
            *blake3::hash(&serde_json::to_vec(saved).expect("serializable browser snapshot"))
                .as_bytes();
        if gate.snapshot_hash != Some(snapshot_hash) {
            gate.qualification_revision
                .fetch_update(
                    std::sync::atomic::Ordering::AcqRel,
                    std::sync::atomic::Ordering::Acquire,
                    |revision| revision.checked_add(1),
                )
                .expect("browser path revision exhausted");
            let revision = gate
                .qualification_revision
                .load(std::sync::atomic::Ordering::Acquire);
            if let Some(updates) = &gate.qualification_updates {
                let _ = updates.set(revision);
            } else {
                gate.qualification_updates = Some(n0_watcher::Watchable::new(revision));
            }
            gate.snapshot_hash = Some(snapshot_hash);
        }
        let credential_changed = gate.saved.as_ref().is_some_and(|previous| {
            previous.generation != saved.generation || previous.secret != saved.secret
        });
        #[cfg(feature = "rtc-probe")]
        {
            let mut qualifications = std::mem::take(&mut gate.rtc_qualifications);
            qualifications.retain(|_, source| {
                let Some(connection) = source.original.upgrade() else {
                    source.retire();
                    return false;
                };
                if credential_changed || !Self::proved_member_current(&gate, saved, &connection) {
                    // Publish loss before closing the M or changing SavedNode.
                    // Queued/idle pumps and Noq predicates see only this atomic
                    // token and never need the gate/Connection lock.
                    source.retire();
                    if let Some(transport) = &gate.rtc {
                        transport.retire_connection(&connection);
                    }
                    connection.close(0u32.into(), b"member path permission revoked");
                    return false;
                }
                true
            });
            gate.rtc_qualifications = qualifications;
        }
        gate.connections.retain(|weak| {
            let Some(connection) = weak.upgrade() else {
                return false;
            };
            if connection.close_reason().is_some() {
                return false;
            }
            if !Self::permits(saved, connection.remote_id(), connection.alpn())
                || (credential_changed && connection.alpn() != REQUEST_ALPN)
            {
                connection.close(0u32.into(), b"membership revoked");
                return false;
            }
            true
        });
        gate.saved = Some(saved.clone());
    }
    fn permits(saved: &SavedNode, target: iroh_base::EndpointId, alpn: &[u8]) -> bool {
        let Ok(invitation) = saved.invitation() else {
            return false;
        };
        if alpn == REQUEST_ALPN {
            return target == invitation.contact;
        }
        if (alpn != BUSINESS_ALPN
            && !crate::member_path_protocol::MemberProtocol::from_wire(alpn).is_some_and(
                |(purpose, _)| purpose == crate::member_path_protocol::MemberProtocol::Replica,
            )
            && alpn != crate::connection_recovery_core::CONNECTION_RECOVERY_ALPN)
            || saved.stage() != "Complete"
        {
            return false;
        }
        saved.verify().ok().flatten().is_some_and(|auth| {
            auth.nodes()
                .iter()
                .any(|(id, _)| id.as_bytes() == target.as_bytes())
                && auth.nodes().iter().any(|(id, _)| *id == saved.identity())
        })
    }
    fn reject() -> AfterHandshakeOutcome {
        AfterHandshakeOutcome::Reject {
            error_code: 0u32.into(),
            reason: b"browser admission rejected".to_vec(),
        }
    }

    async fn qualified_snapshot(&self, connection: &Connection) -> Result<SavedNode, JsValue> {
        let (saved, lease) = {
            let gate = self.0.lock().unwrap();
            (
                gate.saved.clone().ok_or_else(rejected)?,
                gate.lease.ok_or_else(rejected)?,
            )
        };
        let scope = AdmissionScope::new(
            *saved.invitation()?.anchor.root.as_bytes(),
            saved.generation,
        );
        if connection.close_reason().is_some()
            || read_node(lease).await?.as_string().as_deref()
                != Some(&serde_json::to_string(&saved).map_err(error)?)
            || !Self::permits(&saved, connection.remote_id(), connection.alpn())
            || !self
                .0
                .lock()
                .unwrap()
                .proofs
                .iter()
                .any(|proof| proof.applies_to(connection, &scope))
        {
            return Err(rejected());
        }
        Ok(saved)
    }

    fn permitted_relay_origins(saved: &SavedNode) -> Vec<String> {
        let Some(resources) = saved.resources.as_ref() else {
            return Vec::new();
        };
        let disabled: Vec<_> = resources
            .disabled
            .iter()
            .filter_map(|url| origin(url).ok())
            .collect();
        resources
            .relays
            .iter()
            .filter_map(|url| origin(url).ok())
            .filter(|url| !disabled.contains(url))
            .collect()
    }

    #[cfg(feature = "rtc-probe")]
    fn remember_bootstrap(&self, connection: &Connection, report: serde_json::Value) {
        let mut gate = self.0.lock().unwrap();
        // The existing admission proof bound is 32 concurrent member Ms.
        // Closed diagnostics must not grow with a long-running page's retries.
        if gate.member_reports.len() >= 32 {
            let live: std::collections::BTreeSet<_> = gate
                .connections
                .iter()
                .filter_map(|weak| weak.upgrade())
                .filter(|connection| connection.close_reason().is_none())
                .map(|connection| connection.stable_id())
                .collect();
            gate.member_reports
                .retain(|stable, _| live.contains(stable));
        }
        gate.member_reports.insert(connection.stable_id(), report);
    }

    async fn legacy_relay_continuation(
        &self,
        endpoint: &Endpoint,
        connection: &Connection,
    ) -> std::io::Result<serde_json::Value> {
        use crate::member_path_protocol::RelayValidation;
        let check = || async {
            self.qualified_snapshot(connection).await.map_err(|cause| {
                std::io::Error::other(
                    cause
                        .as_string()
                        .unwrap_or_else(|| "browser relay qualification failed".into()),
                )
            })
        };
        let saved = check().await?;
        let allowed = Self::permitted_relay_origins(&saved);
        let permits =
            |url: &RelayUrl| origin(url.as_str()).is_ok_and(|value| allowed.contains(&value));
        let local = endpoint
            .addr()
            .relay_urls()
            .filter(|url| permits(url))
            .take(4)
            .cloned()
            .collect();
        let exchange = RelayValidation::begin(connection, local).await?;
        let stream_id = exchange.stream_id.clone();
        let remote = exchange.candidates.iter().find(|url| permits(url)).cloned();
        let validation = async {
            let remote = remote.ok_or_else(|| std::io::Error::new(std::io::ErrorKind::PermissionDenied, "no mutually permitted relay"))?;
            let started = n0_future::time::Instant::now();
            let mut fresh = None;
            loop {
                let current = check().await?;
                if !origin(remote.as_str()).is_ok_and(|url| Self::permitted_relay_origins(&current).contains(&url)) {
                    return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "browser relay permission changed"));
                }
                if connection.side().is_client() {
                    match endpoint.open_control_relay_path(connection, remote.clone()).await {
                        Ok(()) => {},
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {},
                        Err(error) => return Err(error),
                    }
                }
                if let Some(path) = connection.paths().iter().find(|path| matches!(path.remote_addr(), iroh::TransportAddr::Relay(url) if url == &remote)) {
                    if fresh.is_none() { fresh = Some((path.id().to_string(), path.start_probe()?, started.elapsed().as_millis())); }
                    let (path_id, probe, probe_started) = fresh.as_ref().unwrap();
                    if path.id().to_string() != *path_id { return Err(std::io::Error::other("relay probe path changed")); }
                    if probe.is_acknowledged().map_err(std::io::Error::other)? {
                        return Ok(serde_json::json!({"pathId":path_id,"freshProbeStartedMs":probe_started,"freshAckMs":started.elapsed().as_millis(),"probeWatermark":probe.watermark(),"freshAck":true}));
                    }
                }
                n0_future::time::sleep(Duration::from_millis(40)).await;
            }
        }.await;
        let observation = validation.as_ref().ok().cloned();
        let continuity = exchange.finish(connection, validation.map(|_| ())).await?;
        let current = check().await?;
        if Self::permitted_relay_origins(&current) != allowed {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "browser relay resources changed before release",
            ));
        }
        Ok(
            serde_json::json!({"kind":"legacy-relay","capabilitiesVersion":2,"privateStreamId":stream_id,
            "memberStableId":connection.stable_id(),"stableUnchanged":continuity.stable_unchanged,"bindingUnchanged":continuity.binding_unchanged,"validatedRelay":observation,
            "relayFramesAfterConfirmation":connection.paths().iter().filter(|path|path.is_relay()).map(|path|serde_json::json!({"pathId":path.id().to_string(),"selected":path.is_selected(),"streamSent":path.stats().frame_tx.stream,"streamReceived":path.stats().frame_rx.stream})).collect::<Vec<_>>()}),
        )
    }

    fn proved_member_current(gate: &GateState, saved: &SavedNode, connection: &Connection) -> bool {
        let (Ok(invitation), Some(secret)) = (saved.invitation(), saved.secret) else {
            return false;
        };
        let scope = AdmissionScope::new(*invitation.anchor.root.as_bytes(), saved.generation);
        let fingerprint = AdmissionSecret::from_saved(secret).fingerprint(&scope);
        connection.close_reason().is_none()
            && Self::permits(saved, connection.remote_id(), connection.alpn())
            && gate.proofs.iter().any(|proof| {
                proof.applies_to(connection, &scope)
                    && proof.credential_fingerprint() == &fingerprint
            })
    }

    #[cfg(feature = "rtc-probe")]
    fn rtc_qualification(
        &self,
        connection: &Connection,
    ) -> Result<
        (
            Arc<std::sync::atomic::AtomicU64>,
            u64,
            n0_watcher::Direct<u64>,
        ),
        JsValue,
    > {
        let mut gate = self.0.lock().unwrap();
        let saved = gate.saved.as_ref().ok_or_else(rejected)?;
        if !Self::proved_member_current(&gate, saved, connection) {
            return Err(rejected());
        }
        gate.rtc_qualifications
            .retain(|_, source| source.original.is_open());
        let stable = connection.stable_id();
        if !gate.rtc_qualifications.contains_key(&stable) {
            if gate.rtc_qualifications.len() >= 32 {
                return Err(error("too many concurrent proved member paths"));
            }
            let current = gate
                .qualification_revision
                .load(std::sync::atomic::Ordering::Acquire);
            gate.rtc_qualifications.insert(
                stable,
                RtcQualificationSource {
                    original: connection.weak_handle(),
                    revision: Arc::new(std::sync::atomic::AtomicU64::new(current)),
                    updates: n0_watcher::Watchable::new(current),
                    retired: false,
                },
            );
        }
        let source = &gate.rtc_qualifications[&stable];
        let revision = source.revision.clone();
        let observed = revision.load(std::sync::atomic::Ordering::Acquire);
        Ok((revision, observed, source.updates.watch()))
    }
}
impl crate::connection_coordinator::MemberPathAdapter for BrowserGate {
    fn capabilities(
        &self,
        role: crate::connection_coordinator::AttemptRole,
    ) -> crate::connection_coordinator::RuntimeCapabilities {
        use crate::connection_coordinator::{
            AttemptRole, Platform, RELAY, RTC, RtcRole, RuntimeCapabilities,
        };
        let state = self.0.lock().unwrap();
        let relay = state
            .saved
            .as_ref()
            .is_some_and(|saved| !Self::permitted_relay_origins(saved).is_empty());
        // This slice installs the existing offerer adapter only; later slices
        // supply its opposite role. Support and permission stay separate.
        let supported = role == AttemptRole::Initiator
            && state
                .rtc_capability
                .as_ref()
                .is_some_and(|capability| capability.supported == Some(true));
        let allowed = {
            #[cfg(feature = "rtc-probe")]
            {
                supported
                    && state
                        .rtc_capability
                        .as_ref()
                        .is_some_and(|capability| capability.effective)
                    && state.rtc.as_ref().is_some_and(|rtc| rtc.resource_allowed())
            }
            #[cfg(not(feature = "rtc-probe"))]
            {
                false
            }
        };
        RuntimeCapabilities {
            runtime: Platform::Browser,
            supported: RELAY | if supported { RTC } else { 0 },
            permitted: if relay { RELAY } else { 0 } | if allowed { RTC } else { 0 },
            rtc_role: RtcRole::for_attempt(role, supported),
        }
    }
    fn committed(&self, connection: &Connection, report: serde_json::Value) {
        let mut state = self.0.lock().unwrap();
        if state.member_reports.len() >= 32 {
            let live: Vec<_> = state
                .connections
                .iter()
                .filter_map(|weak| weak.upgrade())
                .map(|connection| connection.stable_id())
                .collect();
            state
                .member_reports
                .retain(|stable, _| live.contains(stable));
        }
        let entry = state
            .member_reports
            .entry(connection.stable_id())
            .or_insert_with(|| serde_json::json!({}));
        entry["coordinator"] = report;
    }
    fn authorization_revision(&self, connection: &Connection) -> std::io::Result<u64> {
        if connection.close_reason().is_some() {
            return Err(crate::connection_coordinator::control_error("DataPathUnavailable"));
        }
        #[cfg(feature = "rtc-probe")]
        {
            self.rtc_qualification(connection)
                .map(|(_, revision, _)| revision)
                .map_err(|_| {
                    crate::connection_coordinator::control_error("IdentityOrAdmissionRejected")
                })
        }
        #[cfg(not(feature = "rtc-probe"))]
        {
            let _ = connection;
            Ok(self
                .0
                .lock()
                .unwrap()
                .qualification_revision
                .load(std::sync::atomic::Ordering::Acquire))
        }
    }
    fn check<'a>(
        &'a self,
        connection: &'a Connection,
    ) -> impl std::future::Future<Output = std::io::Result<()>> + Send + 'a {
        send_wrapper::SendWrapper::new(async move {
            if connection.close_reason().is_some() {
                return Err(crate::connection_coordinator::control_error("DataPathUnavailable"));
            }
            self.qualified_snapshot(connection)
                .await
                .map(|_| ())
                .map_err(|_| {
                    crate::connection_coordinator::control_error("IdentityOrAdmissionRejected")
                })
        })
    }
    fn prepare<'a>(
        &'a self,
        endpoint: &'a Endpoint,
        connection: &'a Connection,
        attempt: crate::connection_coordinator::ProtectedAttempt,
        plan: crate::connection_coordinator::PathPlan,
    ) -> impl std::future::Future<
        Output = std::io::Result<crate::connection_coordinator::PreparedPath>,
    > + Send
    + 'a {
        send_wrapper::SendWrapper::new(async move {
            use crate::connection_coordinator::{PathPlan, PreparedPath, control_error};
            match plan {
                #[cfg(feature = "rtc-probe")]
                PathPlan::Rtc => {
                    let saved = self
                        .qualified_snapshot(connection)
                        .await
                        .map_err(|_| control_error("IdentityOrAdmissionRejected"))?;
                    let transport = self
                        .0
                        .lock()
                        .unwrap()
                        .rtc
                        .clone()
                        .ok_or_else(|| control_error("DataPathUnavailable"))?;
                    let (mut report, prepared) = rtc_member::bootstrap_control_unified(
                        endpoint, connection, self, &saved, &transport, attempt,
                    )
                    .await
                    .map_err(|cause| {
                        let text = cause
                            .as_string()
                            .unwrap_or_else(|| "RTC candidate failed".into());
                        if text.starts_with("unified/") {
                            std::io::Error::other(text)
                        } else {
                            control_error("DataPathUnavailable")
                        }
                    })?;
                    report["capabilitiesVersion"] = serde_json::json!(1);
                    self.remember_bootstrap(connection, report);
                    Ok(prepared)
                }
                PathPlan::Relay => {
                    let mut report = self.legacy_relay_continuation(endpoint, connection).await?;
                    report["capabilitiesVersion"] = serde_json::json!(1);
                    #[cfg(feature = "rtc-probe")]
                    self.remember_bootstrap(connection, report);
                    let paths = connection.paths();
                    let path = paths
                        .iter()
                        .filter(|path| path.is_relay())
                        .max_by_key(|path| path.is_selected())
                        .ok_or_else(|| control_error("DataPathUnavailable"))?;
                    Ok(PreparedPath::candidate(
                        connection,
                        path.remote_addr().clone(),
                        || Ok(()),
                    ))
                }
                _ => {
                    let _ = attempt;
                    Err(control_error("DataPathUnavailable"))
                }
            }
        })
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
        let Some(saved) = saved.filter(|saved| {
            if connection.alpn() == REQUEST_ALPN && !connection.side().is_client() {
                saved.stage() == "Complete"
                    && saved.verify().ok().flatten().is_some_and(|auth| {
                        auth.nodes().contains(&(saved.identity(), Access::manage()))
                    })
            } else {
                Self::permits(saved, connection.remote_id(), connection.alpn())
            }
        }) else {
            return Self::reject();
        };
        let handshake_started = n0_future::time::Instant::now();
        if connection.alpn() == REQUEST_ALPN {
            self.0.lock().unwrap().request_handshake = Some(serde_json::json!({
                "local_id": hex::encode(saved.identity().as_bytes()),
                "remote_id": connection.remote_id().to_string(),
                "alpn": String::from_utf8_lossy(connection.alpn()),
                "stable_id": connection.stable_id(),
                "expected_contact_verified": connection.side().is_client(),
                "tls_authenticated": true,
                "stage": "admission-stage-agreement",
                "business_handle": false,
                "stats": format!("{:?}", connection.stats()),
            }));
        }
        // No 0-RTT API is exposed. Normal data handles wait for the native
        // mutual proof and stage agreement, including after a fresh page start.
        let proof = if connection.alpn() != REQUEST_ALPN {
            let Ok(invitation) = saved.invitation() else {
                return Self::reject();
            };
            let scope = AdmissionScope::new(*invitation.anchor.root.as_bytes(), saved.generation);
            let Some(secret) = saved.secret else {
                return Self::reject();
            };
            let secret = AdmissionSecret::from_saved(secret);
            let result = if connection.side().is_client() {
                prove_client(connection, &scope, &secret).await
            } else {
                prove_server(connection, &scope, &secret).await
            };
            match result {
                Ok(proof) => Some(proof),
                Err(_) => return Self::reject(),
            }
        } else {
            None
        };
        let agreement_budget = self
            .0
            .lock()
            .unwrap()
            .request_deadline
            .filter(|_| connection.alpn() == REQUEST_ALPN)
            .map_or(Duration::from_secs(5), |deadline| {
                Duration::from_secs(5)
                    .min(deadline.saturating_duration_since(n0_future::time::Instant::now()))
            });
        let agreed = n0_future::time::timeout(agreement_budget, async {
            let (mut send, mut recv) = if connection.side().is_client() {
                connection.open_bi().await.ok()?
            } else {
                connection.accept_bi().await.ok()?
            };
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
        if connection.alpn() == REQUEST_ALPN {
            let mut state = self.0.lock().unwrap();
            if let Some(report) = state.request_handshake.as_mut() {
                report["agreement_elapsed_ms"] =
                    serde_json::json!(handshake_started.elapsed().as_millis());
                report["stage"] = serde_json::json!(if agreed {
                    "restricted-A"
                } else {
                    "admission-stage-agreement-failed"
                });
                report["stats"] = serde_json::json!(format!("{:?}", connection.stats()));
            }
        }
        if !agreed {
            return Self::reject();
        }
        if let Some(proof) = proof {
            let mut gate = self.0.lock().unwrap();
            if !gate.saved.as_ref().is_some_and(|current| {
                current.generation == saved.generation
                    && current.secret == saved.secret
                    && Self::permits(current, connection.remote_id(), connection.alpn())
            }) {
                return Self::reject();
            }
            gate.proofs.retain(BootstrapProof::is_live);
            gate.connections.retain(|weak| {
                weak.upgrade()
                    .is_some_and(|connection| connection.close_reason().is_none())
            });
            if gate.proofs.len() >= 32 {
                return Self::reject();
            }
            gate.proofs.push(proof);
            gate.connections.push(connection.weak_handle());
            if connection.alpn() == crate::connection_recovery_core::CONNECTION_RECOVERY_ALPN {
                // Current member proof authorizes only this protected repair
                // exchange. It must not run ordinary member RTC release hooks.
                return AfterHandshakeOutcome::AcceptRestricted;
            }
            AfterHandshakeOutcome::AcceptData {
                allowed_data_custom_ids: {
                    #[cfg(feature = "rtc-probe")]
                    {
                        // Existing admitted relay experiments retain their
                        // S5 grant. Restricted MQTT M must seal one exact route.
                        if connection.paths().iter().any(|path| path.is_relay()) {
                            vec![crate::rtc_transport::TRANSPORT_ID]
                        } else {
                            Vec::new()
                        }
                    }
                    #[cfg(not(feature = "rtc-probe"))]
                    {
                        Vec::new()
                    }
                },
            }
        } else {
            AfterHandshakeOutcome::AcceptRestricted
        }
    }

    async fn before_data_release(
        &self,
        endpoint: &Endpoint,
        connection: &Connection,
    ) -> std::io::Result<()> {
        if connection.alpn() == BUSINESS_ALPN {
            return send_wrapper::SendWrapper::new(
                crate::connection_coordinator::coordinate_member(endpoint, connection, self),
            )
            .await;
        }
        use crate::member_path_protocol::{PathCapabilities, RtcRole, exchange_capabilities};
        if crate::member_path_protocol::MemberProtocol::from_wire(connection.alpn()).is_none()
            || !connection.paths().iter().any(|path| matches!(path.remote_addr(), iroh::TransportAddr::Custom(addr) if addr.id() == crate::mqtt_datagram::TRANSPORT_ID))
        { return Ok(()); }
        let deadline = connection
            .caller_deadline()
            .ok_or_else(|| std::io::Error::other("browser member caller deadline absent"))?;
        // WASM hooks run on their originating browser thread. Wrap the complete
        // local JS future; no Rc escapes into a transport callback or Noq lock.
        send_wrapper::SendWrapper::new(async {
            let relay = self
                .0
                .lock()
                .unwrap()
                .saved
                .as_ref()
                .is_some_and(|saved| !Self::permitted_relay_origins(saved).is_empty());
            #[cfg(feature = "rtc-probe")]
            let transport = self.0.lock().unwrap().rtc.clone();
            #[cfg(feature = "rtc-probe")]
            let qualified = self.qualified_snapshot(connection).await.map_err(|cause| {
                std::io::Error::other(
                    cause
                        .as_string()
                        .unwrap_or_else(|| "browser qualification failed".into()),
                )
            })?;
            let role = {
                #[cfg(feature = "rtc-probe")]
                {
                    RtcRole::browser_adapter(
                        connection,
                        transport.as_ref().is_some_and(|rtc| rtc.resource_allowed()),
                    )
                }
                #[cfg(not(feature = "rtc-probe"))]
                {
                    RtcRole::None
                }
            };
            let operation = async {
                let paths = exchange_capabilities(
                    connection,
                    PathCapabilities {
                        tcp: false,
                        udp: false,
                        relay,
                        rtc: role,
                    },
                )
                .await?
                .require_usable()?;
                #[cfg(feature = "rtc-probe")]
                if paths.rtc {
                    let mut report = rtc_member::bootstrap_control(
                        endpoint,
                        connection,
                        self,
                        &qualified,
                        transport
                            .as_ref()
                            .ok_or_else(|| std::io::Error::other("browser RTC adapter absent"))?,
                    )
                    .await
                    .map_err(|cause| {
                        std::io::Error::other(
                            cause
                                .as_string()
                                .unwrap_or_else(|| "browser RTC setup failed".into()),
                        )
                    })?;
                    report["capabilitiesVersion"] = serde_json::json!(paths.version.number());
                    self.remember_bootstrap(connection, report);
                    return Ok(());
                }
                if paths.relay {
                    let mut report = self.legacy_relay_continuation(endpoint, connection).await?;
                    report["capabilitiesVersion"] = serde_json::json!(paths.version.number());
                    #[cfg(feature = "rtc-probe")]
                    self.remember_bootstrap(connection, report);
                    return Ok(());
                }
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotConnected,
                    "temporarily unreachable: browser member data path unavailable",
                ))
            };
            n0_future::time::timeout(
                deadline.saturating_duration_since(n0_future::time::Instant::now()),
                operation,
            )
            .await
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "browser member caller deadline exceeded",
                )
            })?
        })
        .await
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct IrcConfiguration {
    facility: crate::irc_datagram::IrcFacility,
    profile: crate::irc_datagram::Profile,
}
fn rtc_enabled_by_default() -> bool {
    true
}

#[derive(Clone, Serialize, Deserialize)]
struct Configuration {
    #[serde(default = "rtc_enabled_by_default")]
    rtc_enabled: bool,
    relays: Vec<String>,
    disabled: Vec<String>,
    #[serde(default)]
    mqtt: Vec<crate::mqtt_datagram::MqttFacility>,
    #[serde(default)]
    contact_routes: Vec<ContactRoute>,
    #[serde(default)]
    irc: Vec<IrcConfiguration>,
    #[cfg(feature = "rtc-probe")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rtc_ice: Option<crate::rtc_ice_profile::RtcIceProfile>,
}
#[derive(Clone, Serialize, Deserialize)]
struct ContactRoute {
    contact: String,
    facility: String,
    address: String,
}
struct RuntimeContactRoute {
    contact: iroh_base::EndpointId,
    facility: String,
    address: iroh_base::CustomAddr,
}
impl Default for Configuration {
    fn default() -> Self {
        Self {
            rtc_enabled: true,
            relays: iroh::defaults::prod::default_relay_map()
                .urls::<Vec<_>>()
                .into_iter()
                .map(|url| url.to_string())
                .collect(),
            disabled: Vec::new(),
            mqtt: Vec::new(),
            irc: Vec::new(),
            contact_routes: Vec::new(),
            #[cfg(feature = "rtc-probe")]
            rtc_ice: None,
        }
    }
}
#[derive(Deserialize)]
struct ConfigurationInput {
    rtc_enabled: Option<bool>,
    relays: Option<Vec<String>>,
    disabled: Option<Vec<String>>,
    mqtt: Option<Vec<crate::mqtt_datagram::MqttFacility>>,
    irc: Option<Vec<IrcConfiguration>>,
    contact_routes: Option<Vec<ContactRoute>>,
    #[cfg(feature = "rtc-probe")]
    rtc_ice: Option<crate::rtc_ice_profile::RtcIceProfile>,
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
    deadline: n0_future::time::Instant,
    send: SendStream,
    recv: RecvStream,
}
struct RequestConnection(Option<Connection>);
impl Drop for RequestConnection {
    fn drop(&mut self) {
        if let Some(connection) = self.0.take() {
            connection.close(0u32.into(), b"browser request ended or cancelled");
        }
    }
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
    #[cfg(feature = "rtc-probe")]
    rtc_transport: crate::rtc_transport::RtcTransport,
    relays: Vec<RelayUrl>,
    mqtt: Vec<(
        crate::mqtt_datagram::MqttFacility,
        crate::mqtt::BrowserMqttTransport,
    )>,
    irc: Vec<(IrcConfiguration, crate::irc::BrowserIrcTransport)>,
    carrier_started: n0_future::time::Instant,
    startup_request_started: Option<n0_future::time::Instant>,
    caller_budget: std::time::Duration,
    contact_routes: Vec<RuntimeContactRoute>,
    last_request: Option<serde_json::Value>,
    last_replica: Option<serde_json::Value>,
    attempt_started: Option<n0_future::time::Instant>,
    gate: BrowserGate,
    lease: Option<Lease>,
    saved: SavedNode,
    committed: String,
    delivery: Option<Delivery>,
    management: Option<management::Session>,
}

#[wasm_bindgen]
impl BrowserNetwork {
    /// Configuration is a startup snapshot. Allowed relays are replaceable;
    /// no default facility is silently installed when none can be used.
    pub async fn start(configuration: String) -> Result<BrowserNetwork, JsValue> {
        Self::start_bounded(configuration, TIMEOUT).await
    }

    /// Owned-fixture diagnostic only. This explicit budget never changes the
    /// normal BrowserNetwork 12-second caller budget. No business handle escapes.
    pub async fn irc_restricted_probe(
        configuration: String,
        invitation: String,
        budget_ms: u32,
    ) -> Result<String, JsValue> {
        if !(1..=45000).contains(&budget_ms) {
            return Err(error("invalid IRC diagnostic budget"));
        }
        let mut network = Self::start_bounded(
            configuration,
            std::time::Duration::from_millis(budget_ms.into()),
        )
        .await?;
        if network.irc.is_empty() || !network.mqtt.is_empty() || !network.relays.is_empty() {
            network.close().await;
            return Err(error(
                "IRC diagnostic requires IRC-only explicit facilities",
            ));
        }
        // Only this explicitly startup-inclusive fixture attempt shares the
        // outer caller clock. Ordinary public calls begin their own attempt.
        network.startup_request_started = Some(network.carrier_started);
        let result = network.request_admission(invitation).await;
        let diagnostics: serde_json::Value =
            serde_json::from_str(&network.path_diagnostics()).map_err(error)?;
        let status: serde_json::Value = serde_json::from_str(&network.status()).map_err(error)?;
        for (_, transport) in &network.irc {
            transport.close();
        }
        // Fixture retirement is separate from the caller's attempt. Closing a
        // QUIC endpoint may drain for several PTOs; the owned browser context
        // is destroyed by the runner if this bounded graceful drain expires.
        let cleanup_started = n0_future::time::Instant::now();
        let graceful_close =
            n0_future::time::timeout(std::time::Duration::from_secs(2), network.close())
                .await
                .is_ok();
        Ok(serde_json::json!({"result":result.as_ref().ok(),"failure":result.err().map(|e|format!("{e:?}")),"status":status,"diagnostics":diagnostics,"graceful_close":graceful_close,"cleanup_elapsed_ms":cleanup_started.elapsed().as_millis(),"cleanup":serde_json::from_str::<serde_json::Value>(&network.path_diagnostics()).map_err(error)?}).to_string())
    }
}

impl BrowserNetwork {
    async fn start_bounded(
        configuration: String,
        caller_budget: std::time::Duration,
    ) -> Result<BrowserNetwork, JsValue> {
        let carrier_started = n0_future::time::Instant::now();
        n0_future::time::timeout(
            caller_budget,
            Self::start_inner(configuration, caller_budget, carrier_started),
        )
        .await
        .map_err(|_| error("temporarily unreachable: browser startup caller deadline"))?
    }
    async fn start_inner(
        configuration: String,
        caller_budget: std::time::Duration,
        carrier_started: n0_future::time::Instant,
    ) -> Result<BrowserNetwork, JsValue> {
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
                pending: Vec::new(),
                precode: None,
                codes: Vec::new(),
                approvals: Vec::new(),
            }
        };
        saved.verify()?;
        let mut configuration = saved.resources.clone().unwrap_or_default();
        if let Some(enabled) = input.rtc_enabled {
            configuration.rtc_enabled = enabled;
        }
        if let Some(relays) = input.relays {
            configuration.relays = relays;
        }
        if let Some(disabled) = input.disabled {
            configuration.disabled = disabled;
        }
        if let Some(mqtt) = input.mqtt {
            configuration.mqtt = mqtt;
        }
        if let Some(irc) = input.irc {
            configuration.irc = irc;
        }
        if let Some(routes) = input.contact_routes {
            configuration.contact_routes = routes;
        }
        #[cfg(feature = "rtc-probe")]
        if let Some(profile) = input.rtc_ice {
            configuration.rtc_ice = Some(profile);
        }
        let rtc_capability = crate::rtc_capability::RtcCapability::probe(configuration.rtc_enabled)?;
        #[cfg(feature = "rtc-probe")]
        let ice_profile = {
            let configured = configuration.rtc_ice.clone().unwrap_or_default();
            configured.validate().map_err(error)?;
            if rtc_capability.effective {
                configured.check_allowed(&configuration.disabled).map_err(error)?;
                configured
            } else {
                // Keep the saved preference for a later enabled startup, but
                // never authorize or install observers for an absent adapter.
                crate::rtc_ice_profile::RtcIceProfile::default()
            }
        };
        if configuration.relays.len() > 16 || configuration.disabled.len() > 64 {
            return Err(error("too many relay resources"));
        }
        let disabled: Vec<_> = configuration
            .disabled
            .iter()
            .filter_map(|url| origin(url).ok())
            .collect();
        let mut relays = Vec::new();
        for value in &configuration.relays {
            if !disabled.contains(&origin(value)?) {
                relays.push(value.parse::<RelayUrl>().map_err(error)?);
            }
        }
        if configuration.mqtt.len() + configuration.irc.len() > 16
            || configuration.contact_routes.len() > 64
        {
            return Err(error("too many MQTT resources or contact routes"));
        }
        let mut logical_ids = std::collections::BTreeSet::new();
        for facility in &configuration.mqtt {
            facility.validate().map_err(error)?;
            if !logical_ids.insert(&facility.logical_id) {
                return Err(error("duplicate MQTT logical facility"));
            }
        }
        for entry in &configuration.irc {
            entry.facility.validate().map_err(error)?;
            entry.profile.validate().map_err(error)?;
            if !logical_ids.insert(&entry.facility.logical_id) {
                return Err(error("duplicate signaling logical facility"));
            }
        }
        let mut contact_routes = Vec::new();
        for route in &configuration.contact_routes {
            let contact = route
                .contact
                .parse::<iroh_base::EndpointId>()
                .map_err(error)?;
            let bytes = hex::decode(&route.address).map_err(error)?;
            let addr = if let Some(facility) = configuration
                .mqtt
                .iter()
                .find(|item| item.logical_id == route.facility)
            {
                let addr =
                    iroh_base::CustomAddr::from_parts(crate::mqtt_datagram::TRANSPORT_ID, &bytes);
                crate::mqtt_datagram::route_from_addr(
                    &addr,
                    Some(facility.service_tag().map_err(error)?),
                )
                .map_err(error)?;
                addr
            } else if let Some(entry) = configuration
                .irc
                .iter()
                .find(|item| item.facility.logical_id == route.facility)
            {
                let addr =
                    iroh_base::CustomAddr::from_parts(crate::irc_datagram::TRANSPORT_ID, &bytes);
                crate::irc_datagram::route_from_addr(
                    &addr,
                    entry.facility.service_tag().map_err(error)?,
                )
                .map_err(error)?;
                addr
            } else {
                return Err(error("contact route has no trusted facility"));
            };
            contact_routes.push(RuntimeContactRoute {
                contact,
                facility: route.facility.clone(),
                address: addr,
            });
        }
        let mut mqtt = Vec::new();
        for facility in &configuration.mqtt {
            if facility.check_allowed(&configuration.disabled).is_ok() {
                mqtt.push((
                    facility.clone(),
                    crate::mqtt::BrowserMqttTransport::new(facility, &configuration.disabled)?,
                ));
            }
        }
        let mut irc = Vec::new();
        for entry in &configuration.irc {
            if entry
                .facility
                .check_allowed(&configuration.disabled)
                .is_ok()
            {
                let mut profile = entry.profile.clone();
                profile.attempt_ms = profile.attempt_ms.min(
                    caller_budget
                        .saturating_sub(carrier_started.elapsed())
                        .as_millis() as u64,
                );
                irc.push((
                    entry.clone(),
                    crate::irc::BrowserIrcTransport::new(
                        &entry.facility,
                        &profile,
                        &configuration.disabled,
                    )?,
                ));
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
        if relays.is_empty() && mqtt.is_empty() && irc.is_empty() {
            return Err(error(
                "temporarily unreachable: no allowed browser relay or MQTT carrier",
            ));
        }
        let allowed: Vec<_> = relays
            .iter()
            .map(|url| origin(url.as_str()))
            .collect::<Result<_, _>>()?;
        let gate = BrowserGate::default();
        gate.install(&saved);
        gate.0.lock().unwrap().rtc_capability = Some(rtc_capability.clone());
        gate.0.lock().unwrap().lease = Some(lease.0);
        let mut signaling: Vec<iroh::RelayUrl> = mqtt
            .iter()
            .map(|(facility, _)| facility.wss_url.parse().unwrap())
            .collect();
        signaling.extend(
            irc.iter()
                .map(|(entry, _)| entry.facility.wss_url.parse().unwrap()),
        );
        let irc_services: Vec<_> = irc
            .iter()
            .map(|(entry, _)| entry.facility.service_tag().unwrap())
            .collect();
        let custom_services: Vec<_> = mqtt
            .iter()
            .map(|(facility, _)| facility.service_tag().unwrap())
            .collect();
        let rtc_effective = rtc_capability.effective;
        let mut builder = Endpoint::builder(presets::Minimal)
            .secret_key(SecretKey::from_bytes(&saved.key))
            .relay_mode(RelayMode::custom(relays.clone()))
            .relay_resource_policy(
                RelayResourcePolicy::new(move |url, usage| match usage {
                    RelayResourceUse::DataRelay => {
                        origin(url.as_str()).is_ok_and(|origin| allowed.contains(&origin))
                    }
                    RelayResourceUse::Signaling => signaling.contains(url),
                    _ => false,
                })
                .with_custom_addresses(move |addr| {
                    #[cfg(feature = "rtc-probe")]
                    if addr.id() == crate::rtc_transport::TRANSPORT_ID && addr.data().len() == 16 {
                        return rtc_effective;
                    }
                    custom_services.iter().any(|service| {
                        crate::mqtt_datagram::route_from_addr(addr, Some(*service)).is_ok()
                    }) || irc_services
                        .iter()
                        .any(|service| crate::irc_datagram::route_from_addr(addr, *service).is_ok())
                }),
            )
            .alpns(vec![
                BUSINESS_ALPN.to_vec(),
                REPLICA_ALPN.to_vec(),
                REQUEST_ALPN.to_vec(),
            ])
            .admission_gate(gate.clone());
        for (_, transport) in &mqtt {
            builder = builder.add_custom_transport(Arc::new(transport.clone()));
        }
        for (_, transport) in &irc {
            builder = builder.add_custom_transport(Arc::new(transport.clone()));
        }
        #[cfg(feature = "rtc-probe")]
        let rtc_transport =
            crate::rtc_transport::RtcTransport::with_ice_profile(ice_profile).map_err(error)?;
        #[cfg(feature = "rtc-probe")]
        {
            rtc_transport.set_resource_allowed(rtc_capability.effective);
            if rtc_capability.effective {
                builder = builder.add_custom_transport(Arc::new(rtc_transport.clone()));
                gate.0.lock().unwrap().rtc = Some(rtc_transport.clone());
            }
        }
        let endpoint = builder.bind().await.map_err(error)?;
        if n0_future::time::timeout(
            caller_budget.saturating_sub(carrier_started.elapsed()),
            async {
                use futures_util::{FutureExt, future::select_all};
                let mut candidates = Vec::new();
                for (_, transport) in &mqtt {
                    candidates.push(
                        async move {
                            transport.wait_ready().await?;
                            transport.route()?;
                            Ok::<_, JsValue>(())
                        }
                        .boxed_local(),
                    );
                }
                for (_, transport) in &irc {
                    candidates.push(
                        async move {
                            transport.wait_ready().await?;
                            transport.route()?;
                            Ok::<_, JsValue>(())
                        }
                        .boxed_local(),
                    );
                }
                if !relays.is_empty() {
                    candidates.push(
                        async {
                            endpoint.online().await;
                            Ok::<_, JsValue>(())
                        }
                        .boxed_local(),
                    );
                }
                while !candidates.is_empty() {
                    let (result, _, remaining) = select_all(candidates).await;
                    if result.is_ok() {
                        return Ok(());
                    }
                    candidates = remaining;
                }
                Err(error("no browser carrier became ready"))
            },
        )
        .await
        .map_or(true, |result| result.is_err())
        {
            endpoint.close().await;
            return Err(error(
                "temporarily unreachable: allowed browser carriers unavailable",
            ));
        }
        Ok(Self {
            endpoint,
            #[cfg(feature = "rtc-probe")]
            rtc_transport,
            relays,
            mqtt,
            irc,
            carrier_started,
            startup_request_started: None,
            caller_budget,
            contact_routes,
            last_request: None,
            last_replica: None,
            attempt_started: None,
            gate,
            lease: Some(lease),
            saved,
            committed,
            delivery: None,
            management: None,
        })
    }
}

#[wasm_bindgen]
impl BrowserNetwork {
    pub fn id(&self) -> String {
        self.endpoint.id().to_string()
    }
    pub fn status(&self) -> String {
        serde_json::json!({"id":self.id(), "stage":self.saved.stage(), "authorization_saved":self.saved.authorization.is_some(), "secret_saved":self.saved.secret.is_some(), "running":self.lease.is_some()}).to_string()
    }

    /// Redacted loaded material receipt, checked against the actual leased store.
    pub async fn admission_material_receipt(&self) -> Result<String, JsValue> {
        self.check_store().await?;
        self.saved.verify()?;
        let invitation = self.saved.invitation()?;
        Ok(serde_json::json!({"id":self.id(),"contact":invitation.contact.to_string(),"stage":self.saved.stage(),
            "request":hex::encode(self.saved.precode.map_or(self.saved.request,|token|crate::admission_request_id::precode_request_id(self.saved.request,token))),
            "authorization":self.saved.authorization.map(|id|id.to_string()),"generation":self.saved.generation,
            "authorization_saved":self.saved.authorization.is_some(),"secret_saved":self.saved.secret.is_some(),
            "records":self.saved.records.iter().map(|(header,_)|Hash::digest(header).to_string()).collect::<Vec<_>>()}).to_string())
    }

    /// Owned identity-binding diagnostic over a genuine restricted TLS A.
    /// Claims affect only this existing request wire; durable node state is untouched.
    #[cfg(feature = "rtc-probe")]
    pub async fn admission_identity_probe(
        &self,
        request: String,
        claimed: String,
    ) -> Result<String, JsValue> {
        let request = hex::decode(request).map_err(error)?;
        if request.len() != 16 {
            return Err(rejected());
        }
        let claimed: iroh_base::EndpointId = claimed.parse().map_err(error)?;
        let deadline = n0_future::time::Instant::now() + self.caller_budget;
        let connection = self
            .connect_target_until(self.saved.invitation()?.contact, REQUEST_ALPN, deadline)
            .await?;
        let _scope = RequestConnection(Some(connection.clone()));
        n0_future::time::timeout(deadline.saturating_duration_since(n0_future::time::Instant::now()),async {
            let (mut send,mut recv)=connection.open_bi().await.map_err(error)?;
            let mut wire=self.saved.invitation.clone().ok_or_else(rejected)?;
            wire.extend_from_slice(&request);wire.extend_from_slice(claimed.as_bytes());
            send.write_all(&wire).await.map_err(error)?;send.finish().map_err(error)?;
            let mut response=[0];recv.read_exact(&mut response).await.map_err(error)?;
            if response!=[0] {return Err(error("identity probe unexpectedly obtained approved material"));}
            Ok(serde_json::json!({"local_id":self.id(),"remote_id":connection.remote_id().to_string(),"claimed":claimed.to_string(),
                "alpn":String::from_utf8_lossy(connection.alpn()),"stable_id":connection.stable_id(),"approved_material":false,"response":response[0]}).to_string())
        }).await.map_err(|_|error("identity probe caller deadline"))?
    }

    pub async fn request_admission(&mut self, invitation: String) -> Result<String, JsValue> {
        let started = self
            .startup_request_started
            .take()
            .unwrap_or_else(n0_future::time::Instant::now);
        self.attempt_started = Some(started);
        self.gate.0.lock().unwrap().request_deadline = Some(started + self.caller_budget);
        self.last_request = Some(serde_json::json!({"stage":"connecting","business_handle":false}));
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
        next.precode = None;
        let result = n0_future::time::timeout(
            self.caller_budget.saturating_sub(started.elapsed()),
            async {
                self.commit(next).await?;
                self.request_inner().await
            },
        )
        .await;
        if let Some(report) = self.last_request.as_mut() {
            report["attempt_elapsed_ms"] = serde_json::json!(started.elapsed().as_millis());
        }
        result.map_err(|_| error("temporarily unreachable; durable admission progress retained"))?
    }

    /// Public protocol diagnostics contain identities and counters only.
    pub fn path_diagnostics(&self) -> String {
        let gate = self.gate.0.lock().unwrap();
        let bootstraps = serde_json::json!(gate.member_reports);
        serde_json::json!({"rtcCapability":gate.rtc_capability,"request":self.last_request,"gate":gate.request_handshake,"memberBootstraps":bootstraps,"carriers":self.mqtt.iter().map(|(_,transport)|serde_json::from_str::<serde_json::Value>(&transport.diagnostics()).unwrap()).chain(self.irc.iter().map(|(_,transport)|serde_json::from_str::<serde_json::Value>(&transport.diagnostics()).unwrap())).collect::<Vec<_>>()}).to_string()
    }

    /// A separate handle can cancel while WASM owns the mutable request future.
    /// Cancellation disconnects this endpoint; reopen retains durable identity.
    pub fn cancellation_handle(&self) -> BrowserCancellation {
        BrowserCancellation {
            endpoint: self.endpoint.clone(),
            irc: self
                .irc
                .iter()
                .map(|(_, transport)| transport.clone())
                .collect(),
            carriers: self
                .mqtt
                .iter()
                .map(|(_, transport)| transport.clone())
                .collect(),
        }
    }

    /// Redeem through the designated node. A new code changes the request
    /// namespace, preserving this device and original durable attempt.
    pub async fn request_precode(&mut self, code: String) -> Result<String, JsValue> {
        let started = n0_future::time::Instant::now();
        self.attempt_started = Some(started);
        self.gate.0.lock().unwrap().request_deadline = Some(started + self.caller_budget);
        self.delivery = None;
        let bytes = hex::decode(code).map_err(error)?;
        if bytes.len() != 128 {
            return Err(rejected());
        }
        let invitation = Invitation::decode(&bytes[..96])?;
        if self.saved.invitation.is_some() && self.saved.invitation()?.anchor != invitation.anchor {
            return Err(error("another network is already selected"));
        }
        let mut next = self.saved.clone();
        next.invitation = Some(bytes[..96].to_vec());
        next.precode = Some(bytes[96..].try_into().unwrap());
        self.last_request = Some(serde_json::json!({"stage":"connecting","business_handle":false}));
        n0_future::time::timeout(
            self.caller_budget.saturating_sub(started.elapsed()),
            async {
                self.commit(next).await?;
                self.request_inner().await
            },
        )
        .await
        .map_err(|_| error("precode contact unavailable; progress retained"))?
    }

    pub async fn complete_admission(&mut self) -> Result<String, JsValue> {
        let mut delivery = self.delivery.take().ok_or_else(rejected)?;
        let remaining = delivery
            .deadline
            .saturating_duration_since(n0_future::time::Instant::now());
        if remaining.is_zero() {
            return Err(error(
                "delivery caller deadline expired; committed materials retained",
            ));
        }
        if let Some(report) = self.last_request.as_mut() {
            report["completion_remaining_ms"] = serde_json::json!(remaining.as_millis());
            report["material_events"] = serde_json::json!([]);
        }
        let result = n0_future::time::timeout(remaining, async {
            let auth = self.saved.verify()?.ok_or_else(rejected)?;
            if !auth.membership_authorization_valid(
                self.saved.authorization.ok_or_else(rejected)?,
                self.saved.identity(),
            ) {
                return Err(rejected());
            }
            delivery.send.write_all(b"AUTH").await.map_err(error)?;
            self.material_event("AUTH-sent");
            let mut material = [0; 40];
            delivery
                .recv
                .read_exact(&mut material)
                .await
                .map_err(error)?;
            if u64::from_be_bytes(material[..8].try_into().unwrap()) != self.saved.generation {
                return Err(rejected());
            }
            self.material_event("secret-received-generation-verified");
            let mut next = self.saved.clone();
            next.secret = Some(material[8..].try_into().unwrap());
            self.commit(next).await?;
            self.material_event("secret-committed");
            if delivery.deadline <= n0_future::time::Instant::now() {
                return Err(error(
                    "delivery caller deadline expired; committed materials retained",
                ));
            }
            delivery.send.write_all(b"SAVE").await.map_err(error)?;
            self.material_event("SAVE-sent");
            delivery.send.finish().map_err(error)?;
            let mut reply = [0; 4];
            delivery.recv.read_exact(&mut reply).await.map_err(error)?;
            if &reply != b"DONE" {
                return Err(rejected());
            }
            self.material_event("DONE-received");
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
        self.management = None;
        self.endpoint.close().await;
        self.lease = None;
    }
}

impl BrowserNetwork {
    fn material_event(&mut self, stage: &str) {
        if let Some(events) = self
            .last_request
            .as_mut()
            .and_then(|report| report["material_events"].as_array_mut())
        {
            events.push(serde_json::json!({"stage":stage,"sequence":events.len(),"elapsed_ms":self.attempt_started.map(|start|start.elapsed().as_millis())}));
        }
    }
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
        if raw.len() > 80 * 1024 * 1024 {
            return Err(error("browser state too large"));
        }
        save_node(token, JsValue::from_str(&self.committed), raw.clone()).await?;
        self.saved = next;
        self.committed = raw;
        self.gate.install(&self.saved);
        Ok(())
    }
    async fn connect(&self, alpn: &[u8]) -> Result<Connection, JsValue> {
        self.check_store().await?;
        let invitation = self.saved.invitation()?;
        self.connect_target(invitation.contact, alpn).await
    }
    async fn connect_target(
        &self,
        target: iroh_base::EndpointId,
        alpn: &[u8],
    ) -> Result<Connection, JsValue> {
        self.connect_target_until(
            target,
            alpn,
            n0_future::time::Instant::now() + self.caller_budget,
        )
        .await
    }
    async fn connect_target_until(
        &self,
        target: iroh_base::EndpointId,
        alpn: &[u8],
        deadline: n0_future::time::Instant,
    ) -> Result<Connection, JsValue> {
        self.check_store().await?;
        let addr = self.target_address(target)?;
        if alpn == BUSINESS_ALPN {
            use crate::connection_coordinator::MemberPathAdapter;
            let caps = self
                .gate
                .capabilities(crate::connection_coordinator::AttemptRole::Initiator);
            let coordinator = crate::connection_coordinator::ConnectionCoordinator::new(
                self.endpoint.clone(),
                caps.permitted,
                if self.mqtt.is_empty() {
                    vec![]
                } else {
                    vec![crate::mqtt_datagram::TRANSPORT_ID]
                },
            );
            return coordinator
                .connect(
                    addr,
                    crate::connection_coordinator::SessionPurpose::MemberBusiness,
                    crate::connection_coordinator::AttemptContext::new(
                        deadline,
                        tokio_util::sync::CancellationToken::new(),
                    ),
                )
                .await
                .map_err(error);
        }
        let options = crate::member_path_protocol::MemberProtocol::from_wire(alpn)
            .map_or_else(iroh::endpoint::ConnectOptions::new, |(protocol, _)| {
                protocol.connect_options()
            })
            .with_caller_deadline(deadline);
        n0_future::time::timeout(
            deadline.saturating_duration_since(n0_future::time::Instant::now()),
            async {
                self.endpoint
                    .connect_with_opts(addr, alpn, options)
                    .await
                    .map_err(error)?
                    .await
                    .map_err(error)
            },
        )
        .await
        .map_err(|_| error("browser connection caller deadline exceeded"))?
    }

    fn target_address(&self, target: iroh_base::EndpointId) -> Result<EndpointAddr, JsValue> {
        let mut addr = EndpointAddr::new(target);
        for relay in &self.relays {
            addr.addrs
                .insert(iroh_base::TransportAddr::Relay(relay.clone()));
        }
        for route in &self.contact_routes {
            if route.contact == target
                && self
                    .mqtt
                    .iter()
                    .any(|(facility, _)| facility.logical_id == route.facility)
                || route.contact == target
                    && self
                        .irc
                        .iter()
                        .any(|(entry, _)| entry.facility.logical_id == route.facility)
            {
                addr.addrs
                    .insert(iroh_base::TransportAddr::Custom(route.address.clone()));
            }
        }
        if addr.addrs.is_empty() {
            return Err(error("temporarily unreachable: no allowed contact route"));
        }
        Ok(addr)
    }
    async fn request_inner(&mut self) -> Result<String, JsValue> {
        let started = n0_future::time::Instant::now();
        let deadline = self.attempt_started.unwrap_or(started) + self.caller_budget;
        let connection = self
            .connect_target_until(self.saved.invitation()?.contact, REQUEST_ALPN, deadline)
            .await?;
        let mut request_scope = RequestConnection(Some(connection.clone()));
        self.last_request = Some(
            serde_json::json!({"local_id":self.endpoint.id().to_string(),"remote_id":connection.remote_id().to_string(),"alpn":String::from_utf8_lossy(connection.alpn()),"stable_id":connection.stable_id(),"restricted":true,"business_handle":false,"handshake_ms":started.elapsed().as_millis()}),
        );
        let result = async {
            let (mut send, mut recv) = connection.open_bi().await.map_err(error)?;
            let mut wire = self.saved.invitation.clone().ok_or_else(rejected)?;
            let request = self.saved.precode.map_or(self.saved.request, |token| {
                crate::admission_request_id::precode_request_id(self.saved.request, token)
            });
            wire.extend_from_slice(&request);
            wire.extend_from_slice(self.saved.identity().as_bytes());
            if let Some(token) = self.saved.precode {
                wire.extend_from_slice(&token);
            }
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
            next = next.merge_records(records)?;
            next.authorization = Some(authorization);
            next.generation = generation;
            // A newly received grant is never completed by a previously saved
            // secret, even when the identity is retained across a rejoin.
            next.secret = None;
            let auth = next.verify()?.ok_or_else(rejected)?;
            let provider =
                VerifyingKey::from_bytes(next.invitation()?.contact.as_bytes()).map_err(error)?;
            if !auth.nodes().contains(&(provider, Access::manage()))
                || !auth.membership_authorization_valid(authorization, next.identity())
            {
                return Err(rejected());
            }
            self.commit(next).await?;
            self.delivery = Some(Delivery {
                connection: connection.clone(),
                deadline,
                send,
                recv,
            });
            Ok(self.saved.stage().to_string())
        }
        .await;
        if let Some(report) = self.last_request.as_mut() {
            report["exchange_ms"] = serde_json::json!(started.elapsed().as_millis());
            report["remaining_caller_ms"] = serde_json::json!(
                self.caller_budget
                    .saturating_sub(self.attempt_started.unwrap_or(started).elapsed())
                    .as_millis()
            );
            report["stage"] = serde_json::json!(result.as_ref().ok());
            report["stats"] = serde_json::json!(format!("{:?}", connection.stats()));
            report["paths"] = serde_json::json!(
                connection
                    .paths()
                    .iter()
                    .map(|path| format!("{:?}", path.remote_addr()))
                    .collect::<Vec<_>>()
            );
        }
        if self.delivery.is_some() {
            request_scope.0 = None;
        }
        result
    }
}

#[wasm_bindgen]
pub struct BrowserCancellation {
    endpoint: Endpoint,
    carriers: Vec<crate::mqtt::BrowserMqttTransport>,
    irc: Vec<crate::irc::BrowserIrcTransport>,
}
#[wasm_bindgen]
impl BrowserCancellation {
    pub fn cancel(&self) {
        for carrier in &self.irc {
            carrier.close();
        }
        for carrier in &self.carriers {
            carrier.close();
        }
        let endpoint = self.endpoint.clone();
        wasm_bindgen_futures::spawn_local(async move {
            endpoint.close().await;
        });
    }
}

#[path = "replica.rs"]
mod replica;

#[path = "management.rs"]
mod management;

#[path = "admission.rs"]
mod admission;

#[path = "member_connection.rs"]
mod member_connection;
pub use member_connection::BrowserMemberConnection;

#[cfg(feature = "rtc-probe")]
#[path = "rtc.rs"]
mod rtc;
#[cfg(feature = "rtc-probe")]
#[path = "rtc_member.rs"]
mod rtc_member;
