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

use crate::admission_proof::{AdmissionScope, AdmissionSecret, BootstrapProof};
use crate::management_auth::{ManagementAuth, NetworkAnchor};

const REPLICA_ALPN: &[u8] = crate::connection_coordinator::REPLICA_ALPN;
const REQUEST_ALPN: &[u8] = crate::connection_coordinator::ADMISSION_REQUEST_ALPN;
const PRECODE_REQUEST_ALPN: &[u8] = crate::connection_coordinator::PRECODE_REQUEST_ALPN;
fn is_request_alpn(alpn: &[u8]) -> bool {
    crate::connection_coordinator::SessionPurpose::from_alpn(alpn)
        == Some(crate::connection_coordinator::SessionPurpose::AdmissionRequest)
}
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

struct RelayCheckAttempt {
    stable_id: usize,
    original: iroh::endpoint::WeakConnectionHandle,
    sequence: u64,
    checks: Vec<serde_json::Value>,
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
    member_data: Option<crate::member_data::BrowserMemberDataTransport>,
    member_data_sources: std::collections::BTreeMap<usize, MemberDataQualificationSource>,
    // Temporary bounded causal-observation trace; no identities or wire values.
    relay_checks: Vec<RelayCheckAttempt>,
    relay_check_sequence: u64,
    #[cfg(feature = "rtc-probe")]
    rtc_qualifications: std::collections::BTreeMap<usize, RtcQualificationSource>,
    #[cfg(feature = "rtc-probe")]
    rtc_cleanup_observers: usize,
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
#[derive(Clone)]
struct BrowserGate(
    Arc<Mutex<GateState>>,
    crate::session_maintenance::SessionMaintenance,
);
impl Default for BrowserGate {
    fn default() -> Self {
        Self(
            Arc::new(Mutex::new(GateState::default())),
            crate::session_maintenance::SessionMaintenance::default(),
        )
    }
}
impl std::fmt::Debug for BrowserGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BrowserGate")
    }
}
impl BrowserGate {
    fn maintenance(&self) -> crate::session_maintenance::SessionMaintenance {
        self.1.clone()
    }

    /// The active-control responder uses the saved browser maintenance policy
    /// as its whole incoming operation budget. This is read before polling
    /// `Accepting`, so the same absolute deadline reaches the TLS hooks,
    /// qualification checks, and the restricted C protocol.
    fn active_control_timeout(&self) -> Duration {
        let state = self.0.lock().unwrap();
        state
            .saved
            .as_ref()
            .and_then(|saved| saved.resources.as_ref())
            .map(|resources| resources.maintenance.contact_timeout_ms)
            .map(Duration::from_millis)
            .unwrap_or_else(|| {
                Duration::from_millis(BrowserMaintenancePolicy::default().contact_timeout_ms)
            })
    }

    fn member_connection_for_binding(
        &self,
        peer: iroh_base::EndpointId,
        binding: &[u8; 32],
    ) -> Option<Connection> {
        if let Some(connection) = self.maintenance().for_binding(peer, binding) {
            return Some(connection);
        }
        let state = self.0.lock().unwrap();
        let saved = state.saved.as_ref()?;
        let invitation = saved.invitation().ok()?;
        let scope = AdmissionScope::new(*invitation.anchor.root.as_bytes(), saved.generation);
        state
            .connections
            .iter()
            .filter_map(|weak| weak.upgrade())
            .find(|connection| {
                connection.remote_id() == peer
                    && connection.close_reason().is_none()
                    && crate::connection_recovery_core::binding(connection)
                        .is_ok_and(|actual| &actual == binding)
                    && Self::permits(saved, peer, BUSINESS_ALPN)
                    && state
                        .proofs
                        .iter()
                        .any(|proof| proof.applies_to(connection, &scope))
            })
    }
}

fn relay_probe_debug_details(
    connection: &Connection,
    exact_path: Option<&str>,
    elapsed_ms: u128,
    stage_remaining_ms: Option<u128>,
    started_utc_ms: Option<u128>,
) -> serde_json::Value {
    let mut details = serde_json::json!({"elapsedMs":elapsed_ms});
    details["stageRemainingMs"] = serde_json::json!(stage_remaining_ms);
    details["startedUtcMs"] = serde_json::json!(started_utc_ms);
    details["callerRemainingMs"] =
        serde_json::json!(connection.caller_deadline().map(|deadline| {
            deadline
                .saturating_duration_since(n0_future::time::Instant::now())
                .as_millis()
        }));
    if let Some(exact_path) = exact_path {
        let paths = connection.paths();
        let path = paths
            .iter()
            .find(|path| path.id().to_string() == exact_path);
        details["exactProbePathPresent"] = serde_json::json!(path.is_some());
        if let Some(path) = path {
            let stats = path.stats();
            details["probePathSelected"] = serde_json::json!(path.is_selected());
            details["probePathRttMicros"] = serde_json::json!(stats.rtt.as_micros());
            details["probePingTx"] = serde_json::json!(stats.frame_tx.ping);
            details["probePingRx"] = serde_json::json!(stats.frame_rx.ping);
            details["probeAckTx"] = serde_json::json!(stats.frame_tx.acks);
            details["probeAckRx"] = serde_json::json!(stats.frame_rx.acks);
            details["probePathAckTx"] = serde_json::json!(stats.frame_tx.path_acks);
            details["probePathAckRx"] = serde_json::json!(stats.frame_rx.path_acks);
            details["probeDatagramsTx"] = serde_json::json!(stats.udp_tx.datagrams);
            details["probeDatagramsRx"] = serde_json::json!(stats.udp_rx.datagrams);
            details["probeBytesTx"] = serde_json::json!(stats.udp_tx.bytes);
            details["probeBytesRx"] = serde_json::json!(stats.udp_rx.bytes);
            details["probeLostPackets"] = serde_json::json!(stats.lost_packets);
        }
    }
    details
        .as_object_mut()
        .unwrap()
        .retain(|_, value| !value.is_null());
    details
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
        let mut data_sources = std::mem::take(&mut gate.member_data_sources);
        data_sources.retain(|_, source| {
            let allowed = source.original.upgrade().is_some_and(|original| {
                !credential_changed
                    && Self::proved_member_current(&gate, saved, &original)
                    && Self::member_data_allowed(saved, original.remote_id())
            });
            if !allowed {
                source.retire();
            }
            allowed
        });
        gate.member_data_sources = data_sources;
        gate.connections.retain(|weak| {
            let Some(connection) = weak.upgrade() else {
                return false;
            };
            if connection.close_reason().is_some() {
                return false;
            }
            if !Self::permits(saved, connection.remote_id(), connection.alpn())
                || (credential_changed && !is_request_alpn(connection.alpn()))
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
        if is_request_alpn(alpn) {
            return target == invitation.contact;
        }
        if (alpn != BUSINESS_ALPN
            && alpn != REPLICA_ALPN
            && alpn != crate::connection_recovery_core::CONNECTION_RECOVERY_ALPN
            && alpn != crate::connection_coordinator::ACTIVE_IMPROVEMENT_ALPN)
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

    // A checks its existing invitation/store scope without minting a member proof.
    async fn restricted_snapshot(&self, connection: &Connection) -> Result<SavedNode, JsValue> {
        let (saved, lease) = {
            let gate = self.0.lock().unwrap();
            (
                gate.saved.clone().ok_or_else(rejected)?,
                gate.lease.ok_or_else(rejected)?,
            )
        };
        let auth = saved.verify()?;
        let permitted = if connection.side().is_client() {
            saved.invitation()?.contact == connection.remote_id()
        } else {
            saved.stage() == "Complete"
                && auth.is_some_and(|auth| {
                    auth.nodes().contains(&(saved.identity(), Access::manage()))
                })
        };
        if !is_request_alpn(connection.alpn())
            || connection.close_reason().is_some()
            || !permitted
            || read_node(lease).await?.as_string().as_deref()
                != Some(&serde_json::to_string(&saved).map_err(error)?)
        {
            return Err(rejected());
        }
        Ok(saved)
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

    #[cfg(feature = "rtc-probe")]
    fn remember_rtc_cleanup(&self, stable: usize, sample: serde_json::Value) {
        let mut state = self.0.lock().unwrap();
        if state.member_reports.len() >= 32 && !state.member_reports.contains_key(&stable) {
            let live: std::collections::BTreeSet<_> = state
                .connections
                .iter()
                .filter_map(|original| original.upgrade())
                .filter(|original| original.close_reason().is_none())
                .map(|original| original.stable_id())
                .collect();
            state
                .member_reports
                .retain(|stable, _| live.contains(stable));
        }
        if state.member_reports.len() >= 32 && !state.member_reports.contains_key(&stable) {
            return;
        }
        let report = state
            .member_reports
            .entry(stable)
            .or_insert_with(|| serde_json::json!({}));
        let samples = report
            .as_object_mut()
            .unwrap()
            .entry("rtcCleanup")
            .or_insert_with(|| serde_json::json!([]))
            .as_array_mut()
            .unwrap();
        if samples.len() < 2 {
            samples.push(sample);
        }
    }

    fn remember_relay_check(
        &self,
        endpoint: &Endpoint,
        connection: &Connection,
        phase: &str,
        mut details: serde_json::Value,
    ) {
        use crate::connection_coordinator::{AttemptRole, MemberPathAdapter};
        use n0_watcher::Watcher;
        let caps = self.capabilities(AttemptRole::initial(connection));
        let paths = connection.paths();
        let selected = paths.iter().find(|path| path.is_selected());
        let selected_kind = match selected.as_ref().map(|path| path.remote_addr()) {
            Some(iroh::TransportAddr::Relay(_)) => "relay",
            Some(iroh::TransportAddr::Custom(address))
                if matches!(
                    address.id(),
                    crate::mqtt_datagram::TRANSPORT_ID | crate::irc_datagram::TRANSPORT_ID
                ) =>
            {
                "signal"
            }
            Some(iroh::TransportAddr::Ip(_)) => "ip",
            Some(_) => "other",
            None => "none",
        };
        let home = endpoint.home_relay_status().get();
        let mut gate = self.0.lock().unwrap();
        let allowed = gate.saved.as_ref().map(Self::permitted_relay_origins);
        let gate_snapshot_valid = gate
            .saved
            .as_ref()
            .is_some_and(|saved| Self::proved_member_current(&gate, saved, connection));
        details["tag"] = serde_json::json!("DEBUG-mio76-relay");
        details["phase"] = serde_json::json!(phase);
        details["sideClient"] = serde_json::json!(connection.side().is_client());
        details["memberOpen"] = serde_json::json!(connection.close_reason().is_none());
        details["cachedGateValid"] = serde_json::json!(gate_snapshot_valid);
        details["configuredPermittedCount"] = serde_json::json!(allowed.as_ref().map(Vec::len));
        details["localPublishedCount"] = serde_json::json!(endpoint.addr().relay_urls().count());
        details["localConnectedCount"] =
            serde_json::json!(home.iter().filter(|status| status.is_connected()).count());
        details["localRelayErrorCount"] = serde_json::json!(
            home.iter()
                .filter(|status| status.last_error().is_some())
                .count()
        );
        details["relayPathCount"] =
            serde_json::json!(paths.iter().filter(|path| path.is_relay()).count());
        details["permittedRelayPathCount"] = serde_json::json!(allowed.as_ref().map(|allowed| paths.iter().filter(|path| matches!(path.remote_addr(), iroh::TransportAddr::Relay(url) if origin(url.as_str()).is_ok_and(|origin| allowed.contains(&origin)))).count()));
        details["selectedRouteKind"] = serde_json::json!(selected_kind);
        details["localSupportedMask"] = serde_json::json!(caps.supported);
        details["localPermittedMask"] = serde_json::json!(caps.permitted);
        details["rtcRole"] = serde_json::json!(caps.rtc_role as u8);
        let index = if let Some(index) = gate
            .relay_checks
            .iter()
            .position(|attempt| attempt.stable_id == connection.stable_id())
        {
            index
        } else {
            if gate.relay_checks.len() == 4 {
                gate.relay_checks.remove(0);
            }
            gate.relay_check_sequence = gate.relay_check_sequence.saturating_add(1);
            let sequence = gate.relay_check_sequence;
            gate.relay_checks.push(RelayCheckAttempt {
                stable_id: connection.stable_id(),
                original: connection.weak_handle(),
                sequence,
                checks: Vec::new(),
            });
            gate.relay_checks.len() - 1
        };
        let attempt = &mut gate.relay_checks[index];
        details["attemptSequence"] = serde_json::json!(attempt.sequence);
        details["originalOpen"] = serde_json::json!(attempt.original.is_open());
        // Unobserved stages/optional observations remain absent, never fake 0.
        details
            .as_object_mut()
            .unwrap()
            .retain(|_, value| !value.is_null());
        if attempt.checks.len() == 16 {
            attempt.checks.remove(0);
        }
        attempt.checks.push(details);
    }

    async fn prepare_relay(
        &self,
        endpoint: &Endpoint,
        connection: &Connection,
    ) -> std::io::Result<serde_json::Value> {
        use crate::member_path_protocol::RelayValidation;
        let mut local_candidate_count: Option<usize> = None;
        let mut remote_candidate_count: Option<usize> = None;
        let mut mutually_allowed_count: Option<usize> = None;
        let mut storage_gate_checked = false;
        let work = async {
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
        storage_gate_checked = true;
        let allowed = Self::permitted_relay_origins(&saved);
        let permits =
            |url: &RelayUrl| origin(url.as_str()).is_ok_and(|value| allowed.contains(&value));
        let local: Vec<RelayUrl> = endpoint
            .addr()
            .relay_urls()
            .filter(|url| permits(url))
            .take(4)
            .cloned()
            .collect();
        local_candidate_count = Some(local.len());
        self.remember_relay_check(endpoint, connection, "begin", serde_json::json!({"localCandidateCount":local_candidate_count,"storageGateCheckPassed":storage_gate_checked}));
        let exchange = RelayValidation::begin(connection, local).await?;
        remote_candidate_count = Some(exchange.candidates.len());
        mutually_allowed_count = Some(exchange.candidates.iter().filter(|url| permits(url)).count());
        self.remember_relay_check(endpoint, connection, "exchange", serde_json::json!({"localCandidateCount":local_candidate_count,"remoteCandidateCount":remote_candidate_count,"mutuallyAllowedCount":mutually_allowed_count,"storageGateCheckPassed":storage_gate_checked}));
        let stream_id = exchange.stream_id.clone();
        let remote = exchange.candidates.iter().find(|url| permits(url)).cloned();
        let validation = async {
            let remote = remote.ok_or_else(|| std::io::Error::new(std::io::ErrorKind::PermissionDenied, "no mutually permitted relay"))?;
            let started = n0_future::time::Instant::now();
            let wall_time=js_sys::Date::now();
            let started_utc_ms=(wall_time.is_finite()&&wall_time>=0.0).then_some(wall_time as u128);
            let mut fresh = None;
            let samples=[1u64,2,4,6];
            let mut sampled=0;
            loop {
                storage_gate_checked = false;
                let current = check().await?;
                storage_gate_checked = true;
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
                    if fresh.is_none() {
                        let probe = path.start_probe()?;
                        let mut details=relay_probe_debug_details(connection,Some(&path.id().to_string()),started.elapsed().as_millis(),None,started_utc_ms);
                        details["freshProbeIssued"]=serde_json::json!(true);details["probeWatermark"]=serde_json::json!(probe.watermark());details["storageGateCheckPassed"]=serde_json::json!(storage_gate_checked);
                        self.remember_relay_check(endpoint, connection, "probe-start", details);
                        fresh = Some((path.id().to_string(), probe, started.elapsed().as_millis()));
                    }
                    let (path_id, probe, probe_started) = fresh.as_ref().unwrap();
                    if path.id().to_string() != *path_id { return Err(std::io::Error::other("relay probe path changed")); }
                    if probe.is_acknowledged().map_err(std::io::Error::other)? {
                        let mut details=relay_probe_debug_details(connection,Some(path_id),started.elapsed().as_millis(),None,started_utc_ms);
                        details["freshAck"]=serde_json::json!(true);details["probeWatermark"]=serde_json::json!(probe.watermark());details["storageGateCheckPassed"]=serde_json::json!(storage_gate_checked);
                        self.remember_relay_check(endpoint, connection, "probe-ack", details);
                        return Ok(serde_json::json!({"pathId":path_id,"freshProbeStartedMs":probe_started,"freshAckMs":started.elapsed().as_millis(),"probeWatermark":probe.watermark(),"freshAck":true}));
                    }
                }
                if samples.get(sampled).is_some_and(|seconds| started.elapsed()>=Duration::from_secs(*seconds)) {
                    let mut details=relay_probe_debug_details(connection,fresh.as_ref().map(|(path_id,_,_)|path_id.as_str()),started.elapsed().as_millis(),None,started_utc_ms);
                    details["storageGateCheckPassed"]=serde_json::json!(storage_gate_checked);
                    if let Some((_,probe,_))=fresh.as_ref() {details["probeWatermark"]=serde_json::json!(probe.watermark());details["freshAck"]=serde_json::json!(probe.is_acknowledged().ok());}
                    self.remember_relay_check(endpoint, connection, "probe-progress", details);sampled+=1;
                }
                n0_future::time::sleep(Duration::from_millis(40)).await;
            }
        }.await;
        self.remember_relay_check(endpoint, connection, "validation-end", serde_json::json!({"validationOk":validation.is_ok(),"errorKind":validation.as_ref().err().map(|error|format!("{:?}",error.kind())),"storageGateCheckPassed":storage_gate_checked}));
        let observation = validation.as_ref().ok().cloned();
        let continuity = exchange.finish(connection, validation.map(|_| ())).await?;
        storage_gate_checked = false;
        let current = check().await?;
        storage_gate_checked = true;
        if Self::permitted_relay_origins(&current) != allowed {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "browser relay resources changed before release",
            ));
        }
        Ok(
            serde_json::json!({"kind":"relay","capabilitiesVersion":1,"privateStreamId":stream_id,
            "memberStableId":connection.stable_id(),"stableUnchanged":continuity.stable_unchanged,"bindingUnchanged":continuity.binding_unchanged,"validatedRelay":observation,
            "relayFramesAfterConfirmation":connection.paths().iter().filter(|path|path.is_relay()).map(|path|serde_json::json!({"pathId":path.id().to_string(),"selected":path.is_selected(),"streamSent":path.stats().frame_tx.stream,"streamReceived":path.stats().frame_rx.stream})).collect::<Vec<_>>()}),
        )
        }.await;
        self.remember_relay_check(endpoint, connection, "finish", serde_json::json!({"localCandidateCount":local_candidate_count,"remoteCandidateCount":remote_candidate_count,"mutuallyAllowedCount":mutually_allowed_count,"storageGateCheckPassed":storage_gate_checked,"finishOk":work.is_ok(),"errorKind":work.as_ref().err().map(|error|format!("{:?}",error.kind()))}));
        work
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
    fn register_original(&self, connection: &Connection) {
        self.maintenance().register(connection);
    }

    fn capabilities(
        &self,
        role: crate::connection_coordinator::AttemptRole,
    ) -> crate::connection_coordinator::RuntimeCapabilities {
        use crate::connection_coordinator::{Platform, RELAY, RTC, RtcRole, RuntimeCapabilities};
        let state = self.0.lock().unwrap();
        let relay = state
            .saved
            .as_ref()
            .is_some_and(|saved| !Self::permitted_relay_origins(saved).is_empty())
            || state
                .member_data
                .as_ref()
                .is_some_and(|transport| transport.permitted());
        // Both role adapters share this runtime capability and resource permission.
        let supported = state
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
    fn existing_relay(&self, connection: &Connection) -> Option<iroh::TransportAddr> {
        let gate = self.0.lock().unwrap();
        let saved = gate.saved.as_ref()?;
        connection
            .paths()
            .iter()
            .filter(|path| match path.remote_addr() {
                iroh::TransportAddr::Relay(url) => origin(url.as_str())
                    .is_ok_and(|value| Self::permitted_relay_origins(saved).contains(&value)),
                iroh::TransportAddr::Custom(address) => gate
                    .member_data
                    .as_ref()
                    .is_some_and(|transport| transport.permits_address(address)),
                _ => false,
            })
            .max_by_key(|path| path.is_selected())
            .map(|path| path.remote_addr().clone())
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
        if is_request_alpn(connection.alpn()) {
            if let Some(handshake) = state.request_handshake.as_mut() {
                handshake["coordinator"] = report;
            }
            return;
        }
        let entry = state
            .member_reports
            .entry(connection.stable_id())
            .or_insert_with(|| serde_json::json!({}));
        if entry["finalized"] == true && entry["finalReleaseFreshAck"] == true {
            // Deferred RTC preparation remains provisional until this common
            // bilateral Ready and its post-finalizer authority checks commit.
            entry["switched"] = serde_json::json!(true);
        }
        entry["coordinator"] = report;
    }
    fn authorization_revision(
        &self,
        connection: &Connection,
        purpose: crate::connection_coordinator::SessionPurpose,
    ) -> std::io::Result<u64> {
        if connection.close_reason().is_some() {
            return Err(crate::connection_coordinator::control_error(
                "DataPathUnavailable",
            ));
        }
        if purpose == crate::connection_coordinator::SessionPurpose::AdmissionRequest {
            return Ok(self
                .0
                .lock()
                .unwrap()
                .qualification_revision
                .load(std::sync::atomic::Ordering::Acquire));
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
        purpose: crate::connection_coordinator::SessionPurpose,
    ) -> impl std::future::Future<Output = std::io::Result<()>> + Send + 'a {
        send_wrapper::SendWrapper::new(async move {
            if connection.close_reason().is_some() {
                return Err(crate::connection_coordinator::control_error(
                    "DataPathUnavailable",
                ));
            }
            let snapshot =
                if purpose == crate::connection_coordinator::SessionPurpose::AdmissionRequest {
                    self.restricted_snapshot(connection).await
                } else {
                    self.qualified_snapshot(connection).await
                };
            snapshot.map(|_| ()).map_err(|_| {
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
            use crate::connection_coordinator::{
                MemberPathAdapter, PathPlan, PreparedPath, control_error,
            };
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
                PathPlan::ExistingRelay => {
                    let route = self
                        .existing_relay(connection)
                        .ok_or_else(|| control_error("DataPathUnavailable"))?;
                    let gate = self.clone();
                    let current_route = route.clone();
                    Ok(PreparedPath::candidate(connection, route, move || {
                        let state = gate.0.lock().unwrap();
                        let saved = state
                            .saved
                            .as_ref()
                            .ok_or_else(|| control_error("IdentityOrAdmissionRejected"))?;
                        let allowed = match &current_route {
                            iroh::TransportAddr::Relay(url) => {
                                origin(url.as_str()).is_ok_and(|value| {
                                    BrowserGate::permitted_relay_origins(saved).contains(&value)
                                })
                            }
                            iroh::TransportAddr::Custom(address) => state
                                .member_data
                                .as_ref()
                                .is_some_and(|transport| transport.permits_address(address)),
                            _ => false,
                        };
                        if allowed {
                            Ok(())
                        } else {
                            Err(control_error("DataPathUnavailable"))
                        }
                    }))
                }
                PathPlan::Relay => {
                    let mut report = self.prepare_relay(endpoint, connection).await?;
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

    fn prepare_active<'a>(
        &'a self,
        endpoint: &'a Endpoint,
        original: &'a Connection,
        control: &'a Connection,
        attempt: crate::connection_coordinator::ProtectedAttempt,
        plan: crate::connection_coordinator::PathPlan,
    ) -> impl std::future::Future<
        Output = std::io::Result<crate::connection_coordinator::PreparedPath>,
    > + Send
    + 'a {
        send_wrapper::SendWrapper::new(async move {
            match plan {
                #[cfg(feature = "rtc-probe")]
                crate::connection_coordinator::PathPlan::Rtc => {
                    let saved = self.qualified_snapshot(original).await.map_err(|_| {
                        crate::connection_coordinator::control_error("IdentityOrAdmissionRejected")
                    })?;
                    let transport = self.0.lock().unwrap().rtc.clone().ok_or_else(|| {
                        crate::connection_coordinator::control_error("DataPathUnavailable")
                    })?;
                    rtc_member::bootstrap_control_active(
                        endpoint, original, control, self, &saved, &transport, attempt,
                    )
                    .await
                    .map_err(|cause| {
                        let text = cause
                            .as_string()
                            .unwrap_or_else(|| "RTC active candidate failed".into());
                        if text.starts_with("unified/") {
                            std::io::Error::other(text)
                        } else {
                            crate::connection_coordinator::control_error("DataPathUnavailable")
                        }
                    })
                }
                _ => Err(crate::connection_coordinator::control_error(
                    "DataPathUnavailable",
                )),
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
            let permits = if is_request_alpn(connection.alpn()) && !connection.side().is_client() {
                saved.stage() == "Complete"
                    && saved.verify().ok().flatten().is_some_and(|auth| {
                        auth.nodes().contains(&(saved.identity(), Access::manage()))
                    })
            } else {
                Self::permits(saved, connection.remote_id(), connection.alpn())
            };

            permits
        }) else {
            return Self::reject();
        };
        let handshake_started = n0_future::time::Instant::now();
        if is_request_alpn(connection.alpn()) {
            self.0.lock().unwrap().request_handshake = Some(serde_json::json!({
                "local_id": hex::encode(saved.identity().as_bytes()),
                "remote_id": connection.remote_id().to_string(),
                "alpn": String::from_utf8_lossy(connection.alpn()),
                "stable_id": connection.stable_id(),
                "expected_contact_verified": connection.side().is_client(),
                "tls_authenticated": true,
                "stage": "unified-request-confirmation",
                "business_handle": false,
                "stats": format!("{:?}", connection.stats()),
            }));
        }
        // No 0-RTT API is exposed. Members retain mutual proof and stage
        // agreement; requests wait for the shared restricted nonce confirmation.
        let proof = if !is_request_alpn(connection.alpn()) {
            let Ok(invitation) = saved.invitation() else {
                return Self::reject();
            };
            let scope = AdmissionScope::new(*invitation.anchor.root.as_bytes(), saved.generation);
            let Some(secret) = saved.secret else {
                return Self::reject();
            };
            let secret = AdmissionSecret::from_saved(secret);
            let result = if connection.side().is_client() {
                crate::admission_proof::prove_client(connection, &scope, &secret).await
            } else {
                crate::admission_proof::prove_server(connection, &scope, &secret).await
            };
            match result {
                Ok(proof) => Some(proof),
                Err(_) => {
                    return Self::reject();
                }
            }
        } else {
            None
        };
        let request_deadline = self
            .0
            .lock()
            .unwrap()
            .request_deadline
            .filter(|_| is_request_alpn(connection.alpn()));
        let agreement_budget = crate::connection_budget::remaining(
            match (connection.caller_deadline(), request_deadline) {
                (Some(caller), Some(request)) => Some(caller.min(request)),
                (caller, request) => caller.or(request),
            },
        );
        let agreed = is_request_alpn(connection.alpn())
            || n0_future::time::timeout(agreement_budget, async {
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

        if is_request_alpn(connection.alpn()) {
            let mut state = self.0.lock().unwrap();
            if let Some(report) = state.request_handshake.as_mut() {
                report["agreement_elapsed_ms"] = serde_json::json!(0);
                report["agreement_mode"] = serde_json::json!("unified-session/1");
                report["legacy_agreement_executed"] = serde_json::json!(false);
                report["stage"] = serde_json::json!("unified-request-confirmation");
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

            if connection.alpn() == crate::connection_recovery_core::CONNECTION_RECOVERY_ALPN
                || connection.alpn() == crate::connection_coordinator::ACTIVE_IMPROVEMENT_ALPN
            {
                // Current member proof authorizes only this protected repair
                // exchange. It must not run ordinary member RTC release hooks.
                return AfterHandshakeOutcome::AcceptRestricted;
            }
            AfterHandshakeOutcome::AcceptData {
                allowed_data_custom_ids: {
                    let mut ids = Vec::new();
                    let data = gate
                        .member_data
                        .as_ref()
                        .is_some_and(|transport| transport.permitted());
                    if data {
                        ids.push(crate::member_data::DATA_TRANSPORT_ID);
                    }
                    #[cfg(feature = "rtc-probe")]
                    if connection.paths().iter().any(|path| {
                        path.is_relay()
                            || matches!(path.remote_addr(), iroh::TransportAddr::Custom(address)
                            if data && address.id() == crate::member_data::DATA_TRANSPORT_ID)
                    }) {
                        ids.push(crate::rtc_transport::TRANSPORT_ID);
                    }
                    ids
                },
            }
        } else {
            let confirmed = send_wrapper::SendWrapper::new(
                crate::connection_coordinator::coordinate_session(None, connection, self),
            )
            .await
            .is_ok();
            if let Some(report) = self.0.lock().unwrap().request_handshake.as_mut() {
                report["confirmation_elapsed_ms"] =
                    serde_json::json!(handshake_started.elapsed().as_millis());
                report["stage"] = serde_json::json!(if confirmed {
                    "restricted-A"
                } else {
                    "unified-request-confirmation-failed"
                });
                report["stats"] = serde_json::json!(format!("{:?}", connection.stats()));
            }
            if !confirmed {
                return Self::reject();
            }
            AfterHandshakeOutcome::AcceptRestricted
        }
    }

    async fn before_data_release(
        &self,
        endpoint: &Endpoint,
        connection: &Connection,
    ) -> std::io::Result<()> {
        if crate::connection_coordinator::SessionPurpose::from_alpn(connection.alpn()).is_some() {
            if is_request_alpn(connection.alpn()) {
                return Err(crate::connection_coordinator::control_error(
                    "InvalidControl",
                ));
            }
            if crate::connection_coordinator::SessionPurpose::from_alpn(connection.alpn())
                == Some(crate::connection_coordinator::SessionPurpose::ActiveImprovement)
            {
                // Active C is a restricted control handle. It must never be
                // released as an ordinary data-bearing connection.
                return Err(crate::connection_coordinator::control_error(
                    "InvalidControl",
                ));
            }
            return send_wrapper::SendWrapper::new(
                crate::connection_coordinator::coordinate_session(Some(endpoint), connection, self),
            )
            .await;
        }
        Ok(())
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
    #[serde(default)]
    member_data: Option<MemberDataConfiguration>,
    #[serde(default)]
    path_selection: crate::path_selection::PathSelectionPolicy,
    relays: Vec<String>,
    disabled: Vec<String>,
    #[serde(default)]
    mqtt: Vec<crate::mqtt_datagram::MqttFacility>,
    #[serde(default)]
    contact_routes: Vec<ContactRoute>,
    #[serde(default)]
    maintenance: BrowserMaintenancePolicy,
    #[serde(default)]
    irc: Vec<IrcConfiguration>,
    #[cfg(feature = "rtc-probe")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rtc_ice: Option<crate::rtc_ice_profile::RtcIceProfile>,
}

/// Browser-persisted maintenance policy. It mirrors the shared native policy
/// fields so a saved configuration carries the complete operation budget,
/// while this WASM crate avoids pulling in the native maintenance runtime.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct BrowserMaintenancePolicy {
    exploration_interval_ms: u64,
    discovery_interval_ms: u64,
    max_concurrent_contacts: usize,
    max_neighbors: usize,
    quality_interval_ms: u64,
    contact_timeout_ms: u64,
}
impl Default for BrowserMaintenancePolicy {
    fn default() -> Self {
        Self {
            exploration_interval_ms: 5_000,
            discovery_interval_ms: 15_000,
            max_concurrent_contacts: 1,
            max_neighbors: 1,
            quality_interval_ms: 5_000,
            contact_timeout_ms: crate::connection_budget::DEFAULT_MS,
        }
    }
}
impl BrowserMaintenancePolicy {
    fn validate(&self) -> Result<(), JsValue> {
        if !(250..=60_000).contains(&self.exploration_interval_ms)
            || !(1_000..=120_000).contains(&self.discovery_interval_ms)
            || !(1..=8).contains(&self.max_concurrent_contacts)
            || !(1..=8).contains(&self.max_neighbors)
            || !(250..=30_000).contains(&self.quality_interval_ms)
            || !(100..=crate::connection_budget::MAX_MS).contains(&self.contact_timeout_ms)
        {
            return Err(error(
                "browser maintenance budget is outside bounded limits",
            ));
        }
        Ok(())
    }
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
            member_data: None,
            path_selection: crate::path_selection::PathSelectionPolicy::default(),
            relays: iroh::defaults::prod::default_relay_map()
                .urls::<Vec<_>>()
                .into_iter()
                .map(|url| url.to_string())
                .collect(),
            disabled: Vec::new(),
            mqtt: Vec::new(),
            irc: Vec::new(),
            contact_routes: Vec::new(),
            maintenance: BrowserMaintenancePolicy::default(),
            #[cfg(feature = "rtc-probe")]
            rtc_ice: None,
        }
    }
}
#[derive(Deserialize)]
struct ConfigurationInput {
    member_data: Option<MemberDataConfiguration>,
    path_selection: Option<crate::path_selection::PathSelectionPolicy>,
    rtc_enabled: Option<bool>,
    relays: Option<Vec<String>>,
    disabled: Option<Vec<String>>,
    mqtt: Option<Vec<crate::mqtt_datagram::MqttFacility>>,
    irc: Option<Vec<IrcConfiguration>>,
    contact_routes: Option<Vec<ContactRoute>>,
    maintenance: Option<BrowserMaintenancePolicy>,
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
    incoming: std::rc::Rc<incoming::IncomingDispatcher>,
    member_data_transport: Option<crate::member_data::BrowserMemberDataTransport>,
    member_data_lease: Option<crate::member_data::MemberDataLease>,
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
        Self::start_bounded(configuration, None).await
    }

    /// Owned-fixture diagnostic only. This explicit budget never changes the
    /// normal BrowserNetwork default caller budget. No business handle escapes.
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
            Some(std::time::Duration::from_millis(budget_ms.into())),
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
        caller_override: Option<std::time::Duration>,
    ) -> Result<BrowserNetwork, JsValue> {
        let input: ConfigurationInput = serde_json::from_str(&configuration).map_err(error)?;
        if let Some(policy) = &input.maintenance {
            policy.validate()?;
        }
        let startup_budget = caller_override.unwrap_or_else(|| {
            input
                .maintenance
                .as_ref()
                .map_or(crate::connection_budget::DEFAULT, |policy| {
                    std::time::Duration::from_millis(policy.contact_timeout_ms)
                })
        });
        let carrier_started = n0_future::time::Instant::now();
        n0_future::time::timeout(
            startup_budget,
            Self::start_inner(configuration, caller_override, carrier_started),
        )
        .await
        .map_err(|_| error("temporarily unreachable: browser startup caller deadline"))?
    }
    async fn start_inner(
        configuration: String,
        caller_override: Option<std::time::Duration>,
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
        if let Some(path_selection) = input.path_selection {
            configuration.path_selection = path_selection;
        }
        if let Some(member_data) = input.member_data {
            member_data.validate()?;
            configuration.member_data = Some(member_data);
        }
        if let Some(member_data) = &configuration.member_data {
            member_data.validate()?;
        }
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
        if let Some(maintenance) = input.maintenance {
            configuration.maintenance = maintenance;
        }
        configuration.maintenance.validate()?;
        let caller_budget = caller_override.unwrap_or_else(|| {
            std::time::Duration::from_millis(configuration.maintenance.contact_timeout_ms)
        });
        #[cfg(feature = "rtc-probe")]
        if let Some(profile) = input.rtc_ice {
            configuration.rtc_ice = Some(profile);
        }
        let rtc_capability =
            crate::rtc_capability::RtcCapability::probe(configuration.rtc_enabled)?;
        #[cfg(feature = "rtc-probe")]
        let ice_profile = {
            let configured = configuration.rtc_ice.clone().unwrap_or_default();
            configured.validate().map_err(error)?;
            if rtc_capability.effective {
                configured
                    .check_allowed(&configuration.disabled)
                    .map_err(error)?;
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
                if caller_override.is_some() {
                    profile.attempt_ms = profile.attempt_ms.min(
                        caller_budget
                            .saturating_sub(carrier_started.elapsed())
                            .as_millis() as u64,
                    );
                }
                irc.push((
                    entry.clone(),
                    crate::irc::BrowserIrcTransport::new(
                        &entry.facility,
                        &profile,
                        &configuration.disabled,
                        caller_override.is_some(),
                    )?,
                ));
            }
        }
        let member_data_transport = configuration
            .member_data
            .as_ref()
            .filter(|entry| entry.forward_data && !entry.disabled)
            .map(|entry| {
                entry
                    .via
                    .parse()
                    .map(crate::member_data::BrowserMemberDataTransport::new)
            })
            .transpose()
            .map_err(error)?;
        let mut installed_direct = Vec::new();
        #[cfg(feature = "rtc-probe")]
        if rtc_capability.effective {
            installed_direct.push(crate::rtc_transport::TRANSPORT_ID);
        }
        let installed_relay = if member_data_transport.is_some() {
            vec![crate::member_data::DATA_TRANSPORT_ID]
        } else {
            Vec::new()
        };
        let path_selector = configuration
            .path_selection
            .selector_with_carriers(crate::path_selection::CarrierClasses::new(
                installed_direct,
                installed_relay,
            ))
            .map_err(error)?;
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
        gate.0.lock().unwrap().member_data = member_data_transport.clone();
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
        let data_policy = member_data_transport.clone();
        let mut builder = Endpoint::builder(presets::Minimal)
            .transport_config(
                if irc.is_empty()
                    || !mqtt.is_empty()
                    || !relays.is_empty()
                    || rtc_effective
                    || member_data_transport.is_some()
                {
                    iroh::endpoint::QuicTransportConfig::default()
                } else {
                    iroh::endpoint::QuicTransportConfig::builder()
                        .active_connection_id_limit(2)
                        .build()
                },
            )
            .secret_key(SecretKey::from_bytes(&saved.key))
            .path_selector(path_selector)
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
                    if addr.id() == crate::member_data::DATA_TRANSPORT_ID {
                        return data_policy
                            .as_ref()
                            .is_some_and(|transport| transport.permits_address(addr));
                    }
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
                crate::connection_coordinator::ACTIVE_IMPROVEMENT_ALPN.to_vec(),
                REPLICA_ALPN.to_vec(),
                REQUEST_ALPN.to_vec(),
                PRECODE_REQUEST_ALPN.to_vec(),
            ])
            .admission_gate(gate.clone());
        if let Some(transport) = &member_data_transport {
            builder = builder.add_custom_transport(Arc::new(transport.clone()));
        }
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
        let incoming = incoming::IncomingDispatcher::start(endpoint.clone());
        {
            let active_endpoint = endpoint.clone();
            let active_gate = gate.clone();
            let budget_gate = gate.clone();
            incoming.install_active(
                crate::connection_coordinator::ACTIVE_IMPROVEMENT_ALPN,
                move || budget_gate.active_control_timeout(),
                move |accepting, responder_deadline| {
                    let endpoint = active_endpoint.clone();
                    let gate = active_gate.clone();
                    async move {
                        if responder_deadline <= n0_future::time::Instant::now() {
                            return;
                        }
                        let accepting = accepting.with_caller_deadline(responder_deadline);
                        let Ok(Ok(control)) = n0_future::time::timeout(
                            responder_deadline
                                .saturating_duration_since(n0_future::time::Instant::now()),
                            accepting,
                        )
                        .await
                        else {
                            return;
                        };
                        let _control = RequestConnection(Some(control.clone()));
                        let lookup_gate = gate.clone();
                        let maintenance = gate.maintenance();
                        let result = n0_future::time::timeout(
                            responder_deadline
                                .saturating_duration_since(n0_future::time::Instant::now()),
                            async {
                                if gate.qualified_snapshot(&control).await.is_err() {
                                    return Err(crate::connection_coordinator::control_error(
                                        "IdentityOrAdmissionRejected",
                                    ));
                                }
                                crate::connection_coordinator::serve_active_session(
                                    &endpoint,
                                    &control,
                                    &gate,
                                    &maintenance,
                                    responder_deadline,
                                    move |peer, binding| {
                                        lookup_gate.member_connection_for_binding(peer, binding)
                                    },
                                )
                                .await
                            },
                        )
                        .await
                        .map_err(|_| std::io::Error::other("active improvement deadline exceeded"))
                        .and_then(|result| result);
                        if result.is_err() {
                            control.close(0u32.into(), b"active improvement failed");
                        }
                    }
                },
            );
        }
        Ok(Self {
            endpoint,
            incoming,
            member_data_transport,
            member_data_lease: None,
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
        use n0_watcher::Watcher;
        let home = self.endpoint.home_relay_status().get();
        let relay_home = serde_json::json!({
            "publishedCount":self.endpoint.addr().relay_urls().count(),
            "connectedCount":home.iter().filter(|status|status.is_connected()).count(),
            "errorCount":home.iter().filter(|status|status.last_error().is_some()).count(),
        });
        let gate = self.gate.0.lock().unwrap();
        let bootstraps = serde_json::json!(gate.member_reports);
        serde_json::json!({"incoming":self.incoming.diagnostics(),"relayHome":relay_home,"rtcCapability":gate.rtc_capability,"relayChecks":gate.relay_checks.iter().flat_map(|attempt| attempt.checks.iter()).collect::<Vec<_>>(),"request":self.last_request,"gate":gate.request_handshake,"memberBootstraps":bootstraps,"carriers":self.mqtt.iter().map(|(_,transport)|serde_json::from_str::<serde_json::Value>(&transport.diagnostics()).unwrap()).chain(self.irc.iter().map(|(_,transport)|serde_json::from_str::<serde_json::Value>(&transport.diagnostics()).unwrap())).collect::<Vec<_>>()}).to_string()
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
        self.gate.retire_member_data();
        if let Some(mut lease) = self.member_data_lease.take() {
            let _ = lease.close().await;
        }
        self.incoming.close();
        self.incoming.stopped().await;
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
        if crate::connection_coordinator::SessionPurpose::from_alpn(alpn).is_some() {
            use crate::connection_coordinator::MemberPathAdapter;
            let caps = self
                .gate
                .capabilities(crate::connection_coordinator::AttemptRole::Initiator);
            let mut contact_ids = Vec::new();
            if !self.mqtt.is_empty() {
                contact_ids.push(crate::mqtt_datagram::TRANSPORT_ID);
            }
            if !self.irc.is_empty() {
                contact_ids.push(crate::irc_datagram::TRANSPORT_ID);
            }
            let coordinator = crate::connection_coordinator::ConnectionCoordinator::new(
                self.endpoint.clone(),
                caps.permitted,
                contact_ids,
            );
            return coordinator
                .connect_alpn(
                    addr,
                    alpn,
                    crate::connection_coordinator::AttemptContext::new(
                        deadline,
                        tokio_util::sync::CancellationToken::new(),
                    ),
                )
                .await
                .map_err(error);
        }
        let options = iroh::endpoint::ConnectOptions::new().with_caller_deadline(deadline);
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
        let alpn = if self.saved.precode.is_some() {
            PRECODE_REQUEST_ALPN
        } else {
            REQUEST_ALPN
        };
        let connection = self
            .connect_target_until(self.saved.invitation()?.contact, alpn, deadline)
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
pub use member_connection::{
    BrowserMemberAcceptAttempt, BrowserMemberAttempt, BrowserMemberConnection,
};

#[path = "incoming.rs"]
mod incoming;

#[cfg(feature = "rtc-probe")]
#[path = "rtc.rs"]
mod rtc;
#[cfg(feature = "rtc-probe")]
#[path = "rtc_member.rs"]
mod rtc_member;

#[path = "member_data_sdk.rs"]
mod member_data_sdk;
use member_data_sdk::{MemberDataConfiguration, MemberDataQualificationSource};
