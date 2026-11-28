//! Browser platform adapter for the native manual-admission wire protocol.
//! Signatures, membership policy, SCRAM and TLS remain the shared implementations.
use crate::connection_coordinator::ControlFailure;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::control_protocol::{AuthenticatedControl, ControlOwner, OwnedControl};
use iroh::endpoint::{
    BeforeConnectOutcome, ControlExchange, ControlHandle, ControlMessageFormat, EndpointHooks,
    RestrictedEndpoint,
};
use iroh::endpoint::{
    CongestionControl, Connection, RecvStream, RelayResourcePolicy, RelayResourceUse, SendStream,
};
use iroh::{Endpoint, EndpointAddr, RelayMode, RelayUrl, endpoint::presets};
use iroh_base::SecretKey;
use p2panda_auth::Access;
use p2panda_core::{Hash, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use crate::admission_progress::{AdmissionAttempt, AdmissionReceipt, ApplicationProgress};
use crate::admission_proof::{AdmissionScope, AdmissionSecret, BootstrapProof};
use crate::admission_rotation_wire::{ADMISSION_ROTATION_ALPN, ROTATION_DELIVERY_ALPN};
use crate::connection_coordinator::ADMISSION_RECEIPT_ALPN;
use crate::device_protocol::{DEPARTURE_ALPN, REMOVAL_ALPN};
use crate::device_name::{DeviceNames, with_display_name};
use crate::management_auth::{ManagementAuth, NetworkAnchor};

const REPLICA_ALPN: &[u8] = crate::connection_coordinator::REPLICA_ALPN;
const REQUEST_ALPN: &[u8] = crate::connection_coordinator::ADMISSION_REQUEST_ALPN;
const PRECODE_REQUEST_ALPN: &[u8] = crate::connection_coordinator::PRECODE_REQUEST_ALPN;
fn is_request_alpn(alpn: &[u8]) -> bool {
    crate::connection_coordinator::SessionPurpose::from_alpn(alpn)
        == Some(crate::connection_coordinator::SessionPurpose::AdmissionRequest)
}
fn tracks_request_diagnostics(alpn: &[u8]) -> bool {
    matches!(alpn, REQUEST_ALPN | PRECODE_REQUEST_ALPN)
}
fn requires_member_proof(alpn: &[u8]) -> bool {
    !is_request_alpn(alpn)
        && !matches!(
            alpn,
            crate::service_policy::GUEST_ALPN
                | crate::device_operation::DEVICE_RESULT_ALPN
                | DEPARTURE_ALPN
                | REMOVAL_ALPN
                | device_recovery::RECOVERY_ALPN
                | ROTATION_DELIVERY_ALPN
        )
}
fn credential_bound_control(alpn: &[u8]) -> bool {
    // Recovery proves the root on this TLS session. Saving its newly received
    // admission secret must not close the session before SAVE/DONE completes.
    !is_request_alpn(alpn) && !matches!(
        alpn, crate::management_recovery_wire::RECOVERY_ALPN | ADMISSION_ROTATION_ALPN | ROTATION_DELIVERY_ALPN
    )
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
    #[wasm_bindgen(catch, js_name = releaseNodeAndWait)]
    async fn release_node_and_wait(token: u32) -> Result<JsValue, JsValue>;
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
fn admission_timeout() -> JsValue {
    error("admission attempt timed out; durable progress retained")
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
    #[serde(default, skip_serializing_if = "DeviceNames::is_empty")]
    device_names: DeviceNames,
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
    #[serde(default)]
    creator_seed: Option<[u8; 32]>,
    #[serde(default)]
    recovery_exported: bool,
    #[serde(default)]
    recovery_saved: bool,
    #[serde(default)]
    manual_rejected: bool,
    #[serde(default)]
    confirmed_admission: Option<AdmissionAttempt>,
    #[serde(default)]
    complete_invitation: Option<crate::client_invitation::ClientInvitation>,
    #[serde(default)]
    services: Vec<Vec<u8>>,
    #[serde(default)]
    guest: Option<crate::service_policy::GuestInvitation>,
    #[serde(default)]
    operations: crate::device_operation::OperationJournal,
    #[serde(default)]
    custody: crate::credential_custody::CustodyJournal,
    #[serde(default)]
    custody_check: Option<device_custody::CustodyCheck>,
    #[serde(default)]
    recovery: Option<device_recovery::BrowserRecovery>,
    #[serde(default)]
    served_recoveries: Vec<device_recovery::ServedRecovery>,
    #[serde(default)]
    replacement_source: Option<node_replacement::SourcePause>,
    #[serde(default)]
    restored: Option<crate::node_backup::RestoredNode>,
    #[serde(default)]
    portable_settings: Option<crate::node_backup::PortableSettings>,
    #[serde(default)]
    material_generations: Vec<crate::node_control_backup::MaterialGeneration>,
    #[serde(default)]
    native_rotation: crate::node_control_backup::NativeRotationState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rotation: Option<rotation::RotationState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    migration: Option<migration::SavedMigration>,
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
    fn fresh(key: [u8; 32]) -> Self {
        Self {
            version: 1,
            key,
            invitation: None,
            request: rand::random(),
            records: Vec::new(),
            authorization: None,
            generation: 0,
            secret: None,
            device_names: DeviceNames::default(),
            resources: None,
            pending: Vec::new(),
            precode: None,
            codes: Vec::new(),
            approvals: Vec::new(),
            creator_seed: None,
            recovery_exported: false,
            recovery_saved: false,
            manual_rejected: false,
            confirmed_admission: None,
            complete_invitation: None,
            services: Vec::new(),
            guest: None,
            operations: Default::default(),
            custody: Default::default(),
            custody_check: None,
            recovery: None,
            served_recoveries: Vec::new(),
            replacement_source: None,
            restored: None,
            portable_settings: None,
            material_generations: Vec::new(),
            native_rotation: Default::default(),
            rotation: None,
            migration: None,
        }
    }

    fn admission_request_id(&self) -> [u8; 16] {
        self.precode.map_or(self.request, |token| {
            crate::admission_request_id::precode_request_id(self.request, token)
        })
    }

    fn admission_attempt(&self) -> Option<AdmissionAttempt> {
        if self.invitation.is_none() || self.guest.is_some() || (self.confirmed_admission.is_none()
            && (self.creator_seed.is_some() || self.recovery.is_some() || self.restored.is_some()))
        {
            return None;
        }
        let request_id = self.admission_request_id();
        let progress = if self.manual_retry_stopped() {
            ApplicationProgress::Rejected
        } else {
            match self.confirmed_admission {
                Some(attempt) if attempt.request_id == request_id => attempt.progress,
                Some(_) => ApplicationProgress::Created,
                None if self.secret.is_some() => ApplicationProgress::Complete,
                None if self.authorization.is_some() => ApplicationProgress::Authorized,
                None => ApplicationProgress::Created,
            }
        };
        Some(AdmissionAttempt { request_id, progress })
    }

    fn record_admission_progress(&mut self, progress: ApplicationProgress) {
        let request_id = self.admission_request_id();
        let progress = self.admission_attempt().map_or(progress, |old| old.progress.max(progress));
        self.confirmed_admission = Some(AdmissionAttempt { request_id, progress });
    }

    fn reset_admission_progress(&mut self) {
        self.confirmed_admission = Some(AdmissionAttempt {
            request_id: self.admission_request_id(),
            progress: ApplicationProgress::Created,
        });
    }

    fn request_alpn(&self) -> &'static [u8] {
        if self.precode.is_some() {
            PRECODE_REQUEST_ALPN
        } else {
            REQUEST_ALPN
        }
    }

    async fn write_admission_request(&self, exchange: &ControlExchange) -> Result<u8, JsValue> {
        let mut binding = self.invitation.clone().ok_or_else(rejected)?;
        binding.extend_from_slice(&self.admission_request_id());
        let binding = binding.try_into().map_err(|_| rejected())?;
        let local = self.identity();
        crate::admission_request_wire::write(
            exchange,
            &binding,
            local,
            self.precode,
            self.device_names.record(local),
        )
        .await
        .map_err(error)?;
        Ok(recv_fixed::<1>(exchange).await?[0])
    }

    async fn authorized_admission(&self, exchange: &ControlExchange) -> Result<Self, JsValue> {
        let generation = u64::from_be_bytes(recv_fixed::<8>(exchange).await?);
        if generation < self.generation || generation > i64::MAX as u64 {
            return Err(rejected());
        }
        let history = exchange
            .recv_message(ControlMessageFormat::LengthPrefixed32 { maximum: MAX_HISTORY })
            .await
            .map_err(error)?;
        if history.is_empty() {
            return Err(rejected());
        }
        let (authorization, records): (Hash, Records) =
            p2panda_core::cbor::decode_cbor(history.as_slice()).map_err(error)?;
        let mut next = self.clone().merge_records(records)?;
        next.authorization = Some(authorization);
        next.generation = generation;
        if self.authorization != next.authorization || self.generation != generation {
            next.secret = None;
        }
        let auth = next.verify()?.ok_or_else(rejected)?;
        let provider = VerifyingKey::from_bytes(next.invitation()?.contact.as_bytes()).map_err(error)?;
        if !auth.nodes().contains(&(provider, Access::manage()))
            || !auth.membership_authorization_valid(authorization, next.identity())
        {
            return Err(rejected());
        }
        next.confirmed_admission = Some(AdmissionAttempt {
            request_id: self.admission_request_id(),
            progress: if next.secret.is_some() {
                ApplicationProgress::Complete
            } else {
                ApplicationProgress::Authorized
            },
        });
        Ok(next)
    }

    fn with_admission_secret(&self, material: [u8; 40]) -> Result<Self, JsValue> {
        if u64::from_be_bytes(material[..8].try_into().unwrap()) != self.generation {
            return Err(rejected());
        }
        let mut next = self.clone();
        next.secret = Some(material[8..].try_into().unwrap());
        next.record_admission_progress(ApplicationProgress::Complete);
        Ok(next)
    }

    fn admission_receipt(&self) -> Result<AdmissionReceipt, JsValue> {
        Ok(AdmissionReceipt {
            genesis: self.invitation()?.anchor.genesis,
            request_id: self.admission_request_id(),
            authorization: self.authorization.ok_or_else(rejected)?,
        })
    }

    fn pending_admission_receipt(&self) -> Result<String, JsValue> {
        let receipt = self.admission_receipt()?;
        serde_json::to_string(&(hex::encode(receipt.encode()), self.invitation()?.contact))
            .map_err(error)
    }

    fn manual_retry_stopped(&self) -> bool {
        self.precode.is_none() && self.manual_rejected
    }

    fn identity(&self) -> VerifyingKey {
        p2panda_core::SigningKey::from_bytes(&self.key).verifying_key()
    }
    fn invitation(&self) -> Result<Invitation, JsValue> {
        Invitation::decode(self.invitation.as_ref().ok_or_else(rejected)?)
    }
    fn verify(&self) -> Result<Option<ManagementAuth>, JsValue> {
        self.verify_ledger()?;
        if let Some(full) = &self.complete_invitation {
            full.encode().map_err(error)?;
            if self.invitation.as_deref() != Some(full.material_bytes().map_err(error)?.as_slice())
                || full
                    .precode_bytes()
                    .map_err(error)?
                    .map(|bytes| bytes[96..].try_into().unwrap())
                    != self.precode
            {
                return Err(rejected());
            }
        }
        if self.version != 1 || self.generation > i64::MAX as u64 {
            return Err(rejected());
        }
        if self.records.is_empty() {
            if self.secret.is_some() || self.authorization.is_some() || !self.services.is_empty() {
                return Err(rejected());
            }
            if let Some(bytes) = &self.invitation {
                let invitation = Invitation::decode(bytes)?;
                if self.creator_seed.is_some_and(|s| {
                    SigningKey::from_bytes(&s).verifying_key() != invitation.anchor.root
                }) {
                    return Err(rejected());
                }
                self.verify_device_recovery()?;
            }
            if !self.operations.entries.is_empty()
                || !self.custody.entries.is_empty()
                || self.restored.is_some()
                || self.replacement_source.is_some()
            {
                return Err(rejected());
            }
            return Ok(None);
        }
        let invitation = self.invitation()?;
        self.operations.verify(invitation.anchor).map_err(error)?;
        self.custody.verify(invitation.anchor).map_err(error)?;
        if let Some(source) = &self.replacement_source {
            source.package.verify().map_err(error)?;
            if source.package.identity != self.identity()
                || source.package.network != invitation.anchor
            {
                return Err(rejected());
            }
        }
        if let Some(restored) = &self.restored {
            restored
                .verify(self.identity(), invitation.anchor)
                .map_err(error)?;
        }
        self.verify_device_recovery()?;
        if self.recovery_saved && !self.recovery_exported
            || self.creator_seed.is_some_and(|seed| {
                p2panda_core::SigningKey::from_bytes(&seed).verifying_key()
                    != invitation.anchor.root
            })
        {
            return Err(rejected());
        }
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
        crate::service_policy::ServiceLedger::replay(invitation.anchor, &auth, &self.services)
            .map_err(error)?;
        if let Some(guest) = &self.guest {
            guest.encode().map_err(error)?;
            if guest.anchor != invitation.anchor {
                return Err(rejected());
            }
        }
        Ok(Some(auth))
    }
    fn stage(&self) -> &'static str {
        if self.replacement_source.is_some() {
            return "ReplacementPaused";
        }
        if self.restored.as_ref().is_some_and(|r| r.receipt.is_none()) {
            return "RestoringAuthorization";
        }
        if self.recovery.as_ref().is_some_and(|r| !r.complete) {
            return "AwaitingManagementRecovery";
        }
        if self.guest.is_some() {
            return self.guest_stage();
        }
        if self.creator_seed.is_some() && !self.recovery_saved {
            "AwaitingRecoverySave"
        } else if self.authorization.is_none() && self.manual_retry_stopped() {
            "Rejected"
        } else if self.authorization.is_none() {
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

    fn public_status(&self, pairing: bool, running: bool) -> serde_json::Value {
        let anchor = self.invitation().ok().map(|i| i.anchor);
        let stage = if pairing && self.complete_invitation.is_none() {
            "Pairing"
        } else {
            self.stage()
        };
        with_display_name(serde_json::json!({
            "id": self.identity().to_string(), "stage": stage,
            "network": anchor.map(|a| a.root.to_string()), "genesis": anchor.map(|a| a.genesis.to_string()),
            "recovery_exported": self.recovery_exported,
            "recovery_saved": self.creator_seed.is_none() || self.recovery_saved,
            "admission_attempt": if pairing { None } else { self.admission_attempt().map(|attempt| attempt.public_json()) },
            "automatic_retry": !self.manual_retry_stopped() && stage != "Removed", "authorization_saved": self.authorization.is_some(),
            "secret_saved": self.secret.is_some(), "running": running
        }), self.device_names.name(self.identity()))
    }
}

struct RelayCheckAttempt {
    stable_id: usize,
    original: iroh::endpoint::WeakControlConnection,
    sequence: u64,
    checks: Vec<serde_json::Value>,
}

#[derive(Default)]
struct GateState {
    clipboard: Option<Arc<crate::clipboard::ClipboardState>>,
    rtc_capability: Option<crate::rtc_capability::RtcCapability>,
    saved: Option<SavedNode>,
    proofs: Vec<BootstrapProof>,
    connections: Vec<iroh::endpoint::WeakConnectionHandle>,
    control_sessions: std::collections::BTreeMap<usize, BrowserControlSession>,
    qualification_revision: Arc<std::sync::atomic::AtomicU64>,
    qualification_updates: Option<n0_watcher::Watchable<u64>>,
    changes: crate::state_observation::StateChanges,
    snapshot_hash: Option<[u8; 32]>,
    request_handshake: Option<serde_json::Value>,
    receipt_delivery: Option<serde_json::Value>,
    migration_failure: Option<String>,
    request_deadline: Option<n0_future::time::Instant>,
    pairing_receiver: bool,
    pairing_peer: Option<iroh_base::EndpointId>,
    lease: Option<u32>,
    migration_owner: Option<std::sync::Weak<Mutex<GateState>>>,
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

#[derive(Clone)]
struct BrowserControlSession {
    connection: iroh::endpoint::WeakControlConnection,
    peer: iroh_base::EndpointId,
    alpn: Vec<u8>,
    client: bool,
}
impl BrowserControlSession {
    fn permitted(&self, saved: &SavedNode) -> bool {
        if !self.connection.is_open() {
            return false;
        }
        BrowserGate::control_permitted(saved, self.peer, &self.alpn, self.client)
    }
}

/// The effective permission of one proved original M. Saved record changes
/// still advance the source diagnostic revision, but cannot revoke another
/// still-permitted M's path. Retirement is terminal for every issued token.
#[cfg(feature = "rtc-probe")]
struct RtcQualificationSource {
    original: iroh::endpoint::WeakControlConnection,
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
    Arc<tokio::sync::Mutex<()>>,
);
impl Default for BrowserGate {
    fn default() -> Self {
        Self(
            Arc::new(Mutex::new(GateState::default())),
            crate::session_maintenance::SessionMaintenance::default(),
            Default::default(),
        )
    }
}
impl std::fmt::Debug for BrowserGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BrowserGate")
    }
}
impl BrowserGate {
    fn state_snapshot(&self, pairing: bool, running: bool) -> Result<String, JsValue> {
        let (saved, cursor) = {
            let state = self.0.lock().unwrap();
            (
                state.saved.clone().ok_or_else(rejected)?,
                state.changes.cursor(),
            )
        };
        let auth = saved.verify()?;
        let mut status = saved.public_status(pairing, running);
        let mut members = auth.as_ref().map(ManagementAuth::nodes).unwrap_or_default();
        members.sort_by_key(|(id, _)| *id.as_bytes());
        status["records"] = serde_json::json!(saved.records.len());
        status["members"] = serde_json::json!(members.iter().map(|(id, access)| with_display_name(
            serde_json::json!({"node":id.to_string(),"role":if *access == Access::manage() {"admin"} else {"member"}}),
            saved.device_names.name(*id),
        )).collect::<Vec<_>>());
        Ok(serde_json::json!({"cursor":cursor.to_string(),"status":status,
            "pending_admissions":saved.pending_admissions(),"admission_history":saved.admission_history(),
            "admission_rotation":saved.rotation_progress()?.map(|progress| progress.public_json()),
            "network_migration":saved.migration_progress()?,
            "device_operations":auth.as_ref().map(|auth| saved.operation_state(auth))}).to_string())
    }

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
    connection: &ControlHandle,
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
        let network_changed = gate.saved.as_ref().is_some_and(|previous| {
            previous.invitation().ok().map(|value| value.anchor)
                != saved.invitation().ok().map(|value| value.anchor)
        });
        if let Some(clipboard) = &gate.clipboard {
            if network_changed {
                clipboard.stop(true);
            } else if let Some(peer) = clipboard.status().peer {
                if !saved
                    .service_access()
                    .ok()
                    .is_some_and(|access| crate::clipboard::permitted(&access, peer))
                {
                    clipboard.stop(false);
                }
            }
        }
        let snapshot_hash =
            *blake3::hash(&serde_json::to_vec(saved).expect("serializable browser snapshot"))
                .as_bytes();
        let changed = gate.snapshot_hash != Some(snapshot_hash);
        if changed {
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
        let credential_changed = network_changed || gate.saved.as_ref().is_some_and(|previous| {
            previous.generation != saved.generation || previous.secret != saved.secret
        });
        #[cfg(feature = "rtc-probe")]
        {
            let mut qualifications = std::mem::take(&mut gate.rtc_qualifications);
            qualifications.retain(|stable, source| {
                let allowed = !credential_changed
                    && source.original.is_open()
                    && gate.control_sessions.get(stable).is_some_and(|session| {
                        session.permitted(saved)
                            && Self::proved_id_current(
                                &gate,
                                saved,
                                *stable,
                                session.peer,
                                &session.alpn,
                            )
                    });
                if !allowed {
                    source.retire();
                    source
                        .original
                        .close(0u32.into(), b"member path permission revoked");
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
            if network_changed || !Self::permits(saved, connection.remote_id(), connection.alpn())
                || (credential_changed && credential_bound_control(connection.alpn()))
            {
                connection.close(0u32.into(), b"membership revoked");
                return false;
            }
            true
        });
        let mut sessions = std::mem::take(&mut gate.control_sessions);
        sessions.retain(|stable, session| {
            let allowed = !network_changed && session.permitted(saved)
                && (session.alpn != ADMISSION_ROTATION_ALPN || Self::proved_id_current(
                    &gate, saved, *stable, session.peer, &session.alpn,
                ))
                && !(credential_changed && credential_bound_control(&session.alpn));
            if !allowed {
                session.connection.close(0u32.into(), b"membership revoked");
            }
            allowed
        });
        gate.control_sessions = sessions;
        gate.saved = Some(saved.clone());
        if changed {
            gate.changes.publish();
        }
    }
    fn control_permitted(saved: &SavedNode, peer: iroh_base::EndpointId, alpn: &[u8], client: bool) -> bool {
        if matches!(alpn, ADMISSION_ROTATION_ALPN | ROTATION_DELIVERY_ALPN) {
            return saved.rotation_contact(peer, alpn, client);
        }
        if alpn == REMOVAL_ALPN {
            return Self::removal_contact_allowed(saved, peer, client);
        }
        if matches!(alpn, crate::service_policy::GUEST_ALPN | DEPARTURE_ALPN) && !client {
            return saved.stage() == "Complete";
        }
        if is_request_alpn(alpn) && !client {
            return saved.stage() == "Complete"
                && saved.verify().ok().flatten().is_some_and(|auth| {
                    auth.nodes().contains(&(saved.identity(), Access::manage()))
                });
        }
        Self::permits(saved, peer, alpn)
    }

    fn removal_contact_allowed(saved: &SavedNode, peer: iroh_base::EndpointId, client: bool) -> bool {
        saved.stage() == "Complete"
            && saved.verify().ok().flatten().is_some_and(|auth| {
                VerifyingKey::from_bytes(peer.as_bytes()).is_ok_and(|peer| {
                    if client { !auth.known_removed(peer) } else { auth.known_removed(peer) }
                })
            })
    }

    fn permits(saved: &SavedNode, target: iroh_base::EndpointId, alpn: &[u8]) -> bool {
        if matches!(alpn, ADMISSION_ROTATION_ALPN | ROTATION_DELIVERY_ALPN) {
            return saved.rotation_contact(target, alpn, true);
        }
        let Ok(invitation) = saved.invitation() else {
            return false;
        };
        if saved.replacement_source.is_some() {
            return false;
        }
        if saved.restored.as_ref().is_some_and(|r| r.receipt.is_none())
            && !matches!(
                alpn,
                crate::service_policy::POLICY_ALPN
                    | crate::service_policy::GUEST_ALPN
                    | crate::device_operation::DEVICE_RESULT_ALPN
            )
        {
            return false;
        }
        if alpn == crate::device_operation::DEVICE_RESULT_ALPN {
            return true;
        }
        if alpn == DEPARTURE_ALPN {
            return saved.stage() == "Removed" && saved.verify().ok().flatten().is_some_and(|auth| {
                auth.nodes().iter().any(|(id, _)| id.as_bytes() == target.as_bytes())
            });
        }
        if alpn == REMOVAL_ALPN {
            return Self::removal_contact_allowed(saved, target, true);
        }
        if alpn == device_recovery::RECOVERY_ALPN {
            return saved.recovery.as_ref().is_some_and(|r| r.contact == target)
                || saved.stage() == "Complete"
                    && saved.verify().ok().flatten().is_some_and(|a| {
                        !a.known_removed(VerifyingKey::from_bytes(target.as_bytes()).unwrap())
                            && a.nodes().iter().any(|(id, _)| *id == saved.identity())
                    });
        }
        if alpn == ADMISSION_RECEIPT_ALPN {
            return saved.secret.is_some();
        }
        if is_request_alpn(alpn) {
            return target == invitation.contact;
        }
        if alpn == crate::service_policy::GUEST_ALPN {
            return saved
                .guest
                .as_ref()
                .is_some_and(|guest| target == guest.issuer.id || target == guest.provider.id)
                || saved.stage() == "Complete"
                    && saved.verify().ok().flatten().is_some_and(|auth| {
                        auth.nodes()
                            .iter()
                            .any(|(id, _)| id.as_bytes() == target.as_bytes())
                    });
        }
        if alpn == crate::file_transfer::FILE_ALPN {
            return saved.service_access().ok().is_some_and(|access| {
                VerifyingKey::from_bytes(target.as_bytes())
                    .ok()
                    .is_some_and(|peer| access.can_receive(peer) || access.can_send(peer))
            });
        }
        if alpn == crate::clipboard::CLIPBOARD_ALPN {
            return matches!(saved.stage(), "Complete" | "GuestActive")
                && saved.service_access().ok().is_some_and(|access| {
                    VerifyingKey::from_bytes(target.as_bytes())
                        .ok()
                        .is_some_and(|peer| crate::clipboard::permitted(&access, peer))
                });
        }
        if let Some(protocol) = crate::forwarding::Protocol::from_alpn(alpn) {
            return matches!(saved.stage(), "Complete" | "GuestActive")
                && saved.service_access().ok().is_some_and(|access| {
                    VerifyingKey::from_bytes(target.as_bytes())
                        .ok()
                        .is_some_and(|peer| {
                            crate::forwarding::peer_permitted(&access, peer, protocol)
                        })
                });
        }
        if alpn == crate::web::WEB_ALPN {
            return matches!(saved.stage(), "Complete" | "GuestActive")
                && saved.service_access().ok().is_some_and(|access| {
                    VerifyingKey::from_bytes(target.as_bytes())
                        .ok()
                        .is_some_and(|peer| crate::web::peer_permitted(&access, peer))
                });
        }
        if alpn == crate::terminal::TERMINAL_ALPN {
            return matches!(saved.stage(), "Complete" | "GuestActive")
                && saved.service_access().ok().is_some_and(|access| {
                    VerifyingKey::from_bytes(target.as_bytes())
                        .ok()
                        .is_some_and(|peer| crate::terminal::peer_permitted(&access, peer))
                });
        }
        if (alpn != BUSINESS_ALPN
            && alpn != crate::service_policy::POLICY_ALPN
            && alpn != crate::device_operation::DEVICE_ALPN
            && alpn != crate::credential_custody::CUSTODY_ALPN
            && alpn != REPLICA_ALPN
            && alpn != crate::connection_recovery_core::CONNECTION_RECOVERY_ALPN
            && alpn != crate::connection_coordinator::ACTIVE_IMPROVEMENT_ALPN)
            || saved.stage() != "Complete"
                && !(alpn == crate::service_policy::POLICY_ALPN
                    && saved.stage() == "RestoringAuthorization")
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
    // A checks its existing invitation/store scope without minting a member proof.
    async fn restricted_snapshot(
        &self,
        connection: &(impl AuthenticatedControl + ?Sized),
    ) -> Result<SavedNode, JsValue> {
        let (saved, _, _) = services::installed(self).await?;
        let auth = saved.verify()?;
        let permitted = if connection.side().is_client() {
            Self::permits(&saved, connection.remote_id(), connection.alpn())
        } else {
            saved.stage() == "Complete"
                && auth.is_some_and(|auth| {
                    auth.nodes().contains(&(saved.identity(), Access::manage()))
                })
        };
        if !is_request_alpn(connection.alpn())
            || connection.close_reason().is_some()
            || !permitted
        {
            return Err(rejected());
        }
        Ok(saved)
    }

    async fn qualified_snapshot(
        &self,
        connection: &(impl AuthenticatedControl + ?Sized),
    ) -> Result<SavedNode, JsValue> {
        let (saved, _, _) = services::installed(self).await?;
        let scope = AdmissionScope::new(
            *saved.invitation()?.anchor.root.as_bytes(),
            saved.generation,
        );
        if connection.close_reason().is_some()
            || !Self::permits(&saved, connection.remote_id(), connection.alpn())
            || !Self::proved_member_current(&self.0.lock().unwrap(), &saved, connection)
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
    fn remember_bootstrap(
        &self,
        connection: &(impl AuthenticatedControl + ?Sized),
        report: serde_json::Value,
    ) {
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
        connection: &ControlHandle,
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
        let home = connection.home_relay_status().get();
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
        details["localPublishedCount"] = serde_json::json!(connection.addr().relay_urls().count());
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
                original: connection.revocation_handle(),
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
        connection: &ControlHandle,
        log: crate::connection_coordinator::connection_diagnostic::AttemptLog,
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
        let local: Vec<RelayUrl> = connection
            .addr()
            .relay_urls()
            .filter(|url| permits(url))
            .take(4)
            .cloned()
            .collect();
        local_candidate_count = Some(local.len());
        self.remember_relay_check(connection, "begin", serde_json::json!({"localCandidateCount":local_candidate_count,"storageGateCheckPassed":storage_gate_checked}));
        let exchange = RelayValidation::begin(connection, local).await?;
        remote_candidate_count = Some(exchange.candidates.len());
        mutually_allowed_count = Some(exchange.candidates.iter().filter(|url| permits(url)).count());
        self.remember_relay_check(connection, "exchange", serde_json::json!({"localCandidateCount":local_candidate_count,"remoteCandidateCount":remote_candidate_count,"mutuallyAllowedCount":mutually_allowed_count,"storageGateCheckPassed":storage_gate_checked}));
        let stream_id = exchange.stream_id.clone();
        let remote = exchange.candidates.iter().find(|url| permits(url)).cloned();
        let mut opening = None;
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
                    if fresh.is_none() && opening.is_none() {
                        opening = Some(log.phase("path-open", serde_json::json!({"routeKind":"relay"})));
                    }
                    match connection.open_control_relay_path(remote.clone()).await {
                        Ok(()) => {},
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {},
                        Err(error) => return Err(error),
                    }
                }
                if let Some(path) = connection.paths().iter().find(|path| matches!(path.remote_addr(), iroh::TransportAddr::Relay(url) if url == &remote)) {
                    if fresh.is_none() {
                        if let Some(phase) = opening.take() {
                            phase.finish_with("succeeded", serde_json::json!({"pathId":path.id().to_string()}));
                        }
                        let probe = path.start_probe()?;
                        let mut details=relay_probe_debug_details(connection,Some(&path.id().to_string()),started.elapsed().as_millis(),None,started_utc_ms);
                        details["freshProbeIssued"]=serde_json::json!(true);details["probeWatermark"]=serde_json::json!(probe.watermark());details["storageGateCheckPassed"]=serde_json::json!(storage_gate_checked);
                        self.remember_relay_check(connection, "probe-start", details);
                        fresh = Some((path.id().to_string(), probe, started.elapsed().as_millis()));
                    }
                    let (path_id, probe, probe_started) = fresh.as_ref().unwrap();
                    if path.id().to_string() != *path_id { return Err(std::io::Error::other("relay probe path changed")); }
                    if probe.is_acknowledged().map_err(std::io::Error::other)? {
                        let mut details=relay_probe_debug_details(connection,Some(path_id),started.elapsed().as_millis(),None,started_utc_ms);
                        details["freshAck"]=serde_json::json!(true);details["probeWatermark"]=serde_json::json!(probe.watermark());details["storageGateCheckPassed"]=serde_json::json!(storage_gate_checked);
                        self.remember_relay_check(connection, "probe-ack", details);
                        return Ok(serde_json::json!({"pathId":path_id,"freshProbeStartedMs":probe_started,"freshAckMs":started.elapsed().as_millis(),"probeWatermark":probe.watermark(),"freshAck":true}));
                    }
                }
                if samples.get(sampled).is_some_and(|seconds| started.elapsed()>=Duration::from_secs(*seconds)) {
                    let mut details=relay_probe_debug_details(connection,fresh.as_ref().map(|(path_id,_,_)|path_id.as_str()),started.elapsed().as_millis(),None,started_utc_ms);
                    details["storageGateCheckPassed"]=serde_json::json!(storage_gate_checked);
                    if let Some((_,probe,_))=fresh.as_ref() {details["probeWatermark"]=serde_json::json!(probe.watermark());details["freshAck"]=serde_json::json!(probe.is_acknowledged().ok());}
                    self.remember_relay_check(connection, "probe-progress", details);sampled+=1;
                }
                n0_future::time::sleep(Duration::from_millis(40)).await;
            }
        }.await;
        if let Some(phase) = opening {
            phase.finish("failed");
        }
        self.remember_relay_check(connection, "validation-end", serde_json::json!({"validationOk":validation.is_ok(),"errorKind":validation.as_ref().err().map(|error|format!("{:?}",error.kind())),"storageGateCheckPassed":storage_gate_checked}));
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
        self.remember_relay_check(connection, "finish", serde_json::json!({"localCandidateCount":local_candidate_count,"remoteCandidateCount":remote_candidate_count,"mutuallyAllowedCount":mutually_allowed_count,"storageGateCheckPassed":storage_gate_checked,"finishOk":work.is_ok(),"errorKind":work.as_ref().err().map(|error|format!("{:?}",error.kind()))}));
        work
    }

    fn permits_relay_path(
        gate: &GateState,
        saved: &SavedNode,
        address: &iroh::TransportAddr,
    ) -> bool {
        match address {
            iroh::TransportAddr::Relay(url) => origin(url.as_str())
                .is_ok_and(|value| Self::permitted_relay_origins(saved).contains(&value)),
            iroh::TransportAddr::Custom(address) => gate
                .member_data
                .as_ref()
                .is_some_and(|transport| transport.permits_address(address)),
            _ => false,
        }
    }

    fn permits_repair_survival(
        &self,
        original: &Connection,
        address: &iroh::TransportAddr,
    ) -> bool {
        let gate = self.0.lock().unwrap();
        let Some(saved) = gate.saved.as_ref() else {
            return false;
        };
        if !Self::proved_member_current(&gate, saved, original) {
            return false;
        }
        #[cfg(feature = "rtc-probe")]
        if let iroh::TransportAddr::Custom(address) = address
            && address.id() == crate::rtc_transport::TRANSPORT_ID
        {
            use iroh::endpoint::transports::CustomSender;
            return gate.rtc.as_ref().is_some_and(|transport| {
                transport.is_data_path_qualified(address, original.stable_id())
            });
        }
        Self::permits_relay_path(&gate, saved, address)
    }

    fn proved_member_current(
        gate: &GateState,
        saved: &SavedNode,
        connection: &(impl AuthenticatedControl + ?Sized),
    ) -> bool {
        connection.close_reason().is_none()
            && Self::proved_id_current(
                gate,
                saved,
                connection.stable_id(),
                connection.remote_id(),
                connection.alpn(),
            )
    }
    fn proved_id_current(
        gate: &GateState,
        saved: &SavedNode,
        stable: usize,
        peer: iroh_base::EndpointId,
        alpn: &[u8],
    ) -> bool {
        if saved.guest_pair(peer, alpn) {
            return Self::permits(saved, peer, alpn)
                && gate
                    .control_sessions
                    .get(&stable)
                    .is_some_and(|s| s.peer == peer && s.alpn == alpn && s.connection.is_open());
        }
        if alpn == ADMISSION_ROTATION_ALPN {
            return saved.rotation_contact(peer, alpn, false)
                && gate.proofs.iter().any(|proof| {
                    saved.rotation_credential(proof.scope().secret_generation()).is_some_and(|(scope, secret)| {
                        proof.applies_to_id(stable, &scope)
                            && proof.credential_fingerprint() == &secret.fingerprint(&scope)
                    })
                });
        }
        let (Ok(invitation), Some(secret)) = (saved.invitation(), saved.secret) else {
            return false;
        };
        let scope = AdmissionScope::new(*invitation.anchor.root.as_bytes(), saved.generation);
        let fingerprint = AdmissionSecret::from_saved(secret).fingerprint(&scope);
        Self::permits(saved, peer, alpn)
            && gate.proofs.iter().any(|proof| {
                proof.applies_to_id(stable, &scope)
                    && proof.credential_fingerprint() == &fingerprint
            })
    }

    #[cfg(feature = "rtc-probe")]
    fn rtc_qualification(
        &self,
        connection: &(impl AuthenticatedControl + ?Sized),
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
                    original: connection.revocation_handle(),
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
impl crate::connection_coordinator::MemberPathAdapter<ControlHandle> for BrowserGate {
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
    fn existing_relay(&self, connection: &ControlHandle) -> Option<iroh::TransportAddr> {
        let gate = self.0.lock().unwrap();
        let saved = gate.saved.as_ref()?;
        connection
            .paths()
            .iter()
            .filter(|path| Self::permits_relay_path(&gate, saved, path.remote_addr()))
            .max_by_key(|path| path.is_selected())
            .map(|path| path.remote_addr().clone())
    }
    fn committed(&self, connection: &ControlHandle, report: serde_json::Value) {
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
            if tracks_request_diagnostics(connection.alpn())
                && let Some(handshake) = state.request_handshake.as_mut()
            {
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
        connection: &ControlHandle,
        purpose: crate::connection_coordinator::SessionPurpose,
    ) -> std::io::Result<u64> {
        if connection.close_reason().is_some() {
            return Err(ControlFailure::DataPathUnavailable.into_io());
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
                    ControlFailure::IdentityOrAdmissionRejected.into_io()
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
        connection: &'a ControlHandle,
        purpose: crate::connection_coordinator::SessionPurpose,
    ) -> impl std::future::Future<Output = std::io::Result<()>> + Send + 'a {
        send_wrapper::SendWrapper::new(async move {
            if connection.close_reason().is_some() {
                return Err(ControlFailure::DataPathUnavailable.into_io());
            }
            let snapshot =
                if purpose == crate::connection_coordinator::SessionPurpose::AdmissionRequest {
                    self.restricted_snapshot(connection).await
                } else {
                    self.qualified_snapshot(connection).await
                };
            snapshot.map(|_| ()).map_err(|_| {
                // The control connection can close while the saved membership is being read.
                if connection.close_reason().is_some() {
                    ControlFailure::DataPathUnavailable.into_io()
                } else {
                    ControlFailure::IdentityOrAdmissionRejected.into_io()
                }
            })
        })
    }
    fn prepare<'a>(
        &'a self,
        endpoint: Option<&'a Endpoint>,
        connection: &'a ControlHandle,
        attempt: crate::connection_coordinator::ProtectedAttempt,
        plan: crate::connection_coordinator::PathPlan,
    ) -> impl std::future::Future<
        Output = std::io::Result<crate::connection_coordinator::PreparedPath>,
    > + Send
    + 'a {
        send_wrapper::SendWrapper::new(async move {
            let _ = endpoint;
            use crate::connection_coordinator::{PathPlan, PreparedPath};
            match plan {
                #[cfg(feature = "rtc-probe")]
                PathPlan::Rtc => {
                    let saved = self
                        .qualified_snapshot(connection)
                        .await
                        .map_err(|_| ControlFailure::IdentityOrAdmissionRejected.into_io())?;
                    let transport = self
                        .0
                        .lock()
                        .unwrap()
                        .rtc
                        .clone()
                        .ok_or_else(|| ControlFailure::DataPathUnavailable.into_io())?;
                    let (mut report, prepared) = rtc_member::bootstrap_control_unified(
                        connection, self, &saved, &transport, attempt,
                    )
                    .await
                    .map_err(ControlFailure::into_io)?;
                    report["capabilitiesVersion"] = serde_json::json!(1);
                    self.remember_bootstrap(connection, report);
                    Ok(prepared)
                }
                PathPlan::ExistingRelay => {
                    let route = self
                        .existing_relay(connection)
                        .ok_or_else(|| ControlFailure::DataPathUnavailable.into_io())?;
                    let gate = self.clone();
                    let current_route = route.clone();
                    Ok(PreparedPath::candidate(connection, route, move || {
                        let state = gate.0.lock().unwrap();
                        let saved = state
                            .saved
                            .as_ref()
                            .ok_or_else(|| ControlFailure::IdentityOrAdmissionRejected.into_io())?;
                        if BrowserGate::permits_relay_path(&state, saved, &current_route) {
                            Ok(())
                        } else {
                            Err(ControlFailure::DataPathUnavailable.into_io())
                        }
                    }))
                }
                PathPlan::Relay => {
                    let mut report = self.prepare_relay(connection, attempt.log).await?;
                    report["capabilitiesVersion"] = serde_json::json!(1);
                    #[cfg(feature = "rtc-probe")]
                    self.remember_bootstrap(connection, report);
                    let paths = connection.paths();
                    let path = paths
                        .iter()
                        .filter(|path| path.is_relay())
                        .max_by_key(|path| path.is_selected())
                        .ok_or_else(|| ControlFailure::DataPathUnavailable.into_io())?;
                    Ok(PreparedPath::candidate(
                        connection,
                        path.remote_addr().clone(),
                        || Ok(()),
                    ))
                }
                _ => {
                    let _ = attempt;
                    Err(ControlFailure::DataPathUnavailable.into_io())
                }
            }
        })
    }

    fn prepare_active<'a>(
        &'a self,
        endpoint: Option<&'a Endpoint>,
        original: &'a ControlHandle,
        control: &'a ControlHandle,
        attempt: crate::connection_coordinator::ProtectedAttempt,
        plan: crate::connection_coordinator::PathPlan,
    ) -> impl std::future::Future<
        Output = std::io::Result<crate::connection_coordinator::PreparedPath>,
    > + Send
    + 'a {
        let _ = endpoint;
        send_wrapper::SendWrapper::new(async move {
            match plan {
                #[cfg(feature = "rtc-probe")]
                crate::connection_coordinator::PathPlan::Rtc => {
                    let saved = self.qualified_snapshot(original).await.map_err(|_| {
                        ControlFailure::IdentityOrAdmissionRejected.into_io()
                    })?;
                    let transport = self.0.lock().unwrap().rtc.clone().ok_or_else(|| {
                        ControlFailure::DataPathUnavailable.into_io()
                    })?;
                    rtc_member::bootstrap_control_active(
                        original, control, self, &saved, &transport, attempt,
                    )
                    .await
                    .map_err(ControlFailure::into_io)
                }
                _ => Err(ControlFailure::DataPathUnavailable.into_io()),
            }
        })
    }
}

#[derive(Clone, Debug)]
struct BrowserBeforeConnect(BrowserGate);
impl EndpointHooks for BrowserBeforeConnect {
    async fn before_connect(&self, target: &EndpointAddr, alpn: &[u8]) -> BeforeConnectOutcome {
        let state = self.0.0.lock().unwrap();
        let pairing = alpn == crate::invitation_pairing::PAIRING_ALPN
            && state.pairing_peer == Some(target.id);
        if pairing
            || state
                .saved
                .as_ref()
                .is_some_and(|saved| BrowserGate::permits(saved, target.id, alpn))
        {
            BeforeConnectOutcome::Accept
        } else {
            BeforeConnectOutcome::Reject
        }
    }

    async fn before_connect_with_context(
        &self,
        target: &EndpointAddr,
        alpn: &[u8],
        context: Option<&(dyn std::any::Any + Send + Sync)>,
    ) -> BeforeConnectOutcome {
        if let Some(context) = context.and_then(|value| value.downcast_ref::<migration::WireScope>()) {
            if !context.matches(target.id, alpn) {
                return BeforeConnectOutcome::Reject;
            }
            return BrowserBeforeConnect(context.gate.clone()).before_connect(target, alpn).await;
        }
        self.before_connect(target, alpn).await
    }
}
impl BrowserGate {
    async fn authenticate(
        &self,
        connection: &ControlHandle,
    ) -> std::io::Result<n0_future::time::Instant> {
        let saved = self.0.lock().unwrap().saved.clone();

        let Some(saved) = saved.filter(|saved| {
            Self::control_permitted(saved, connection.remote_id(), connection.alpn(), connection.side().is_client())
        }) else {
            return Err(ControlFailure::IdentityOrAdmissionRejected.into_io());
        };
        let handshake_started = n0_future::time::Instant::now();
        if tracks_request_diagnostics(connection.alpn()) {
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
        let proof = if requires_member_proof(connection.alpn())
            && !(saved.guest_pair(connection.remote_id(), connection.alpn()))
        {
            let generation = if connection.alpn() == ADMISSION_ROTATION_ALPN {
                crate::admission_rotation_wire::negotiate_generation(connection, saved.generation)
                    .await.map_err(|_| ControlFailure::IdentityOrAdmissionRejected.into_io())?
            } else {
                saved.generation
            };
            let Some((scope, secret)) = saved.rotation_credential(generation) else {
                return Err(ControlFailure::IdentityOrAdmissionRejected.into_io());
            };
            let result = if connection.side().is_client() {
                crate::admission_proof::prove_client(connection, &scope, &secret).await
            } else {
                crate::admission_proof::prove_server(connection, &scope, &secret).await
            };
            match result {
                Ok(proof) => Some(proof),
                Err(_) => {
                    return Err(ControlFailure::IdentityOrAdmissionRejected.into_io());
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
            .filter(|_| tracks_request_diagnostics(connection.alpn()));
        let agreement_budget = crate::connection_budget::remaining(
            match (connection.caller_deadline(), request_deadline) {
                (Some(caller), Some(request)) => Some(caller.min(request)),
                (caller, request) => caller.or(request),
            },
        );
        let agreed = is_request_alpn(connection.alpn())
            || n0_future::time::timeout(agreement_budget, async {
                let exchange = if connection.side().is_client() {
                    connection.open_control_exchange().await.ok()?
                } else {
                    connection.accept_control_exchange().await.ok()?
                };
                let format = ControlMessageFormat::Fixed { length: 1 };
                exchange.send_message(format, &[1]).await.ok()?;
                exchange.finish_send().await.ok()?;
                let response = exchange.recv_message(format).await.ok()?;
                (response == [1]).then_some(())
            })
            .await
            .ok()
            .flatten()
            .is_some();

        if tracks_request_diagnostics(connection.alpn()) {
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
            return Err(ControlFailure::IdentityOrAdmissionRejected.into_io());
        }
        if let Some(proof) = proof {
            let mut gate = self.0.lock().unwrap();
            if !gate.saved.as_ref().is_some_and(|current| {
                Self::control_permitted(current, connection.remote_id(), connection.alpn(), connection.side().is_client())
                    && (connection.alpn() == ADMISSION_ROTATION_ALPN || current.generation == saved.generation)
                    && current.rotation_credential(proof.scope().secret_generation())
                        .is_some_and(|(scope, secret)| {
                            proof.applies_to(connection, &scope)
                                && proof.credential_fingerprint() == &secret.fingerprint(&scope)
                        })
            }) {
                return Err(ControlFailure::IdentityOrAdmissionRejected.into_io());
            }
            gate.proofs.retain(BootstrapProof::is_live);
            gate.connections.retain(|weak| {
                weak.upgrade()
                    .is_some_and(|connection| connection.close_reason().is_none())
            });
            if gate.proofs.len() >= 32 {
                return Err(ControlFailure::IdentityOrAdmissionRejected.into_io());
            }
            gate.proofs.push(proof);
        }
        let mut gate = self.0.lock().unwrap();
        gate.control_sessions
            .retain(|_, session| session.connection.is_open());
        if gate.control_sessions.len() >= 32 {
            return Err(ControlFailure::IdentityOrAdmissionRejected.into_io());
        }
        gate.control_sessions.insert(
            connection.stable_id(),
            BrowserControlSession {
                connection: connection.revocation_handle(),
                peer: connection.remote_id(),
                alpn: connection.alpn().to_vec(),
                client: connection.side().is_client(),
            },
        );
        Ok(handshake_started)
    }
}
impl BrowserGate {
    async fn admit_in_scope(
        &self,
        connection: &iroh::endpoint::ControlConnection,
    ) -> std::io::Result<iroh::endpoint::AdmissionOutcome> {
        send_wrapper::SendWrapper::new(async {
            if connection.alpn() == crate::invitation_pairing::PAIRING_ALPN {
                if !self.pairing_permitted(connection) {
                    return Err(std::io::Error::other("invitation pairing rejected"));
                }
                return iroh::endpoint::EndpointAdmission::admit(
                    &crate::invitation_pairing::PairingGate,
                    connection,
                )
                .await;
            }
            use crate::connection_coordinator::SessionPurpose;
            let handle = connection.control_handle();
            let started = self.authenticate(&handle).await?;
            let purpose = SessionPurpose::from_alpn(connection.alpn());
            if purpose.is_none() || purpose == Some(SessionPurpose::ActiveImprovement) {
                let gate = self.clone();
                return Ok(
                    iroh::endpoint::AdmissionOutcome::ControlOnlyWithConfirmation(
                        iroh::endpoint::ControlConfirmation::with_control(move |control| {
                            Box::pin(async move {
                                if iroh::endpoint::EndpointAdmission::is_current(&gate, control) {
                                    Ok(())
                                } else {
                                    Err(ControlFailure::IdentityOrAdmissionRejected.into_io())
                                }
                            })
                        }),
                    ),
                );
            }
            let prepared =
                crate::connection_coordinator::prepare_control_session(None, &handle, self).await?;
            let permit = if purpose.is_some_and(|purpose| purpose.requires_data()) {
                let mut ids = Vec::new();
                let state = self.0.lock().unwrap();
                if state
                    .member_data
                    .as_ref()
                    .is_some_and(|data| data.permitted())
                {
                    ids.push(crate::member_data::DATA_TRANSPORT_ID);
                }
                #[cfg(feature = "rtc-probe")]
                if state.rtc.as_ref().is_some_and(|rtc| rtc.resource_allowed()) {
                    ids.push(crate::rtc_transport::TRANSPORT_ID);
                }
                drop(state);
                Some(
                    connection
                        .qualify_data_path(prepared.path_id(connection)?, ids)
                        .await?,
                )
            } else {
                None
            };
            let gate = self.clone();
            let confirmation = iroh::endpoint::ControlConfirmation::with_control(move |control| {
                Box::pin(send_wrapper::SendWrapper::new(async move {
                    let result = prepared.confirm(&control.control_handle(), &gate).await;
                    if tracks_request_diagnostics(control.alpn()) {
                        if let Some(report) = gate.0.lock().unwrap().request_handshake.as_mut() {
                            report["confirmation_elapsed_ms"] =
                                serde_json::json!(started.elapsed().as_millis());
                            report["stage"] = serde_json::json!(if result.is_ok() {
                                "restricted-A"
                            } else {
                                "unified-request-confirmation-failed"
                            });
                            report["stats"] = serde_json::json!(format!("{:?}", control.stats()));
                        }
                    }
                    result
                }))
            });
            Ok(match permit {
                Some(permit) => iroh::endpoint::AdmissionOutcome::AcceptWithConfirmation {
                    permit,
                    confirmation,
                },
                None => iroh::endpoint::AdmissionOutcome::ControlOnlyWithConfirmation(confirmation),
            })
        })
        .await
    }
}

impl iroh::endpoint::EndpointAdmission for BrowserGate {
    async fn admit(
        &self,
        connection: &iroh::endpoint::ControlConnection,
    ) -> std::io::Result<iroh::endpoint::AdmissionOutcome> {
        let gate = if let Some(scope) = connection.application_context::<migration::WireScope>() {
            if !connection.side().is_client() || !scope.matches(connection.remote_id(), connection.alpn()) {
                return Err(ControlFailure::IdentityOrAdmissionRejected.into_io());
            }
            &scope.gate
        } else {
            self
        };
        gate.admit_in_scope(connection).await
    }

    fn is_current(&self, connection: &iroh::endpoint::ControlConnection) -> bool {
        if connection.alpn() == crate::invitation_pairing::PAIRING_ALPN {
            return self.pairing_permitted(connection);
        }
        let gate = connection.application_context::<migration::WireScope>()
            .map_or(self, |scope| &scope.gate);
        let state = gate.0.lock().unwrap();
        let Some(saved) = state.saved.as_ref() else {
            return false;
        };
        state
            .control_sessions
            .get(&connection.stable_id())
            .is_some_and(|session| {
                session.peer == connection.remote_id()
                    && session.alpn == connection.alpn()
                    && session.permitted(saved)
                    && (!requires_member_proof(connection.alpn())
                        || Self::proved_member_current(&state, saved, connection))
            })
    }
    fn business_released(&self, connection: &Connection) {
        if connection.application_context::<migration::WireScope>().is_some() {
            // The caller owns the only provisional data connection.
            return;
        }
        let mut state = self.0.lock().unwrap();
        state.connections.retain(|weak| weak.is_open());
        state.connections.push(connection.weak_handle());
        drop(state);
        if connection.alpn() == BUSINESS_ALPN {
            self.maintenance().register(connection);
        }
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
fn legacy_resource_replacement() -> bool {
    // Older snapshots stored only the selected vectors. Preserve that saved
    // allow-list conservatively when no explicit preference marker exists.
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
    #[serde(default)]
    congestion_control: CongestionControl,
    relays: Vec<String>,
    disabled: Vec<String>,
    #[serde(default = "legacy_resource_replacement")]
    relay_replacement: bool,
    #[serde(default = "legacy_resource_replacement")]
    mqtt_replacement: bool,
    #[serde(default = "legacy_resource_replacement")]
    irc_replacement: bool,
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

use crate::maintenance_policy::MaintenancePolicy as BrowserMaintenancePolicy;

#[derive(Clone, Serialize, Deserialize)]
struct ContactRoute {
    contact: String,
    facility: String,
    address: String,
}
#[derive(Clone)]
struct RuntimeContactRoute {
    contact: iroh_base::EndpointId,
    address: iroh_base::CustomAddr,
}

fn target_address(
    target: iroh_base::EndpointId,
    relays: &[RelayUrl],
    routes: &[RuntimeContactRoute],
) -> Result<EndpointAddr, JsValue> {
    let mut address = EndpointAddr::new(target);
    address
        .addrs
        .extend(relays.iter().cloned().map(iroh_base::TransportAddr::Relay));
    address.addrs.extend(
        routes
            .iter()
            .filter(|route| route.contact == target)
            .map(|route| iroh_base::TransportAddr::Custom(route.address.clone())),
    );
    if address.addrs.is_empty() {
        return Err(error("temporarily unreachable: no allowed contact route"));
    }
    Ok(address)
}

impl Configuration {
    fn allowed_relays(&self) -> Result<Vec<RelayUrl>, JsValue> {
        if self.relays.len() > 16 || self.disabled.len() > 64 {
            return Err(error("too many relay resources"));
        }
        let disabled: Vec<_> = self.disabled.iter().filter_map(|url| origin(url).ok()).collect();
        let mut relays = Vec::new();
        for value in &self.relays {
            if !disabled.contains(&origin(value)?) {
                relays.push(value.parse::<RelayUrl>().map_err(error)?);
            }
        }
        Ok(relays)
    }

    fn validate_signaling(&self) -> Result<(), JsValue> {
        if self.mqtt.len() + self.irc.len() > 16
            || self.contact_routes.len() > 64
        {
            return Err(error("too many MQTT resources or contact routes"));
        }
        let mut logical_ids = std::collections::BTreeSet::new();
        for facility in &self.mqtt {
            facility.validate().map_err(error)?;
            if !logical_ids.insert(&facility.logical_id) {
                return Err(error("duplicate MQTT logical facility"));
            }
        }
        for entry in &self.irc {
            entry.facility.validate().map_err(error)?;
            entry.profile.validate().map_err(error)?;
            if !logical_ids.insert(&entry.facility.logical_id) {
                return Err(error("duplicate signaling logical facility"));
            }
        }
        self.runtime_contact_routes(None)?;
        Ok(())
    }

    fn runtime_contact_routes(
        &self,
        target: Option<iroh_base::EndpointId>,
    ) -> Result<Vec<RuntimeContactRoute>, JsValue> {
        let mut contact_routes = Vec::new();
        let target = target.map(|target| target.to_string());
        for route in self.contact_routes.iter().filter(|route| {
            target.as_ref().is_none_or(|target| &route.contact == target)
        }) {
            let contact = route
                .contact
                .parse::<iroh_base::EndpointId>()
                .map_err(error)?;
            let bytes = hex::decode(&route.address).map_err(error)?;
            let addr = if let Some(facility) = self
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
            } else if let Some(entry) = self
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
            if !self.mqtt.iter().any(|facility| facility.logical_id == route.facility && facility.check_allowed(&self.disabled).is_ok())
                && !self.irc.iter().any(|entry| {
                    entry.facility.logical_id == route.facility
                        && entry.facility.check_allowed(&self.disabled).is_ok()
                })
            {
                continue;
            }
            contact_routes.push(RuntimeContactRoute {
                contact,
                address: addr,
            });
        }
        Ok(contact_routes)
    }
}

impl SavedNode {
    fn address_for(&self, target: iroh_base::EndpointId) -> Result<EndpointAddr, JsValue> {
        let resources = self.resources.as_ref().ok_or_else(rejected)?;
        target_address(
            target,
            &resources.allowed_relays()?,
            &resources.runtime_contact_routes(Some(target))?,
        )
    }
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            rtc_enabled: true,
            member_data: None,
            path_selection: crate::path_selection::PathSelectionPolicy::default(),
            congestion_control: CongestionControl::default(),
            relays: iroh::defaults::prod::default_relay_map()
                .urls::<Vec<_>>()
                .into_iter()
                .map(|url| url.to_string())
                .collect(),
            disabled: Vec::new(),
            relay_replacement: false,
            mqtt_replacement: false,
            irc_replacement: false,
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
    host_device_name: Option<String>,
    member_data: Option<MemberDataConfiguration>,
    path_selection: Option<crate::path_selection::PathSelectionPolicy>,
    congestion_control: Option<CongestionControl>,
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

async fn send_fixed(exchange: &ControlExchange, bytes: &[u8]) -> Result<(), JsValue> {
    exchange
        .send_message(
            ControlMessageFormat::Fixed {
                length: bytes.len(),
            },
            bytes,
        )
        .await
        .map_err(error)
}
async fn recv_fixed<const N: usize>(exchange: &ControlExchange) -> Result<[u8; N], JsValue> {
    exchange
        .recv_message(ControlMessageFormat::Fixed { length: N })
        .await
        .map_err(error)?
        .try_into()
        .map_err(|_| rejected())
}

struct Delivery {
    connection: ControlOwner,
    deadline: n0_future::time::Instant,
    exchange: ControlExchange,
}
struct RequestConnection<C: AuthenticatedControl>(Option<C>);
impl<C: AuthenticatedControl> Drop for RequestConnection<C> {
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
    endpoint: RestrictedEndpoint,
    incoming: std::rc::Rc<incoming::IncomingDispatcher>,
    admission: admission::AdmissionService,
    rotation: rotation::RotationRuntime,
    migration: migration::Runtime,
    files: std::rc::Rc<files::BrowserFiles>,
    _file_owner: files::FileOwner,
    clipboard: std::rc::Rc<clipboard_host::BrowserClipboard>,
    forwarding: std::rc::Rc<forwarding_host::BrowserForwarding>,
    web: std::rc::Rc<web_host::BrowserWeb>,
    _web_owner: web_host::WebOwner,
    terminal: std::rc::Rc<terminal_host::BrowserTerminal>,
    _terminal_owner: terminal_host::TerminalOwner,
    _clipboard_owner: clipboard_host::ClipboardOwner,
    services: std::rc::Rc<services::BrowserServices>,
    _service_owner: services::ServiceOwner,
    member_data_transport: Option<crate::member_data::BrowserMemberDataTransport>,
    member_data_lease: Option<crate::member_data::MemberDataLease>,
    #[cfg(feature = "rtc-probe")]
    rtc_transport: crate::rtc_transport::RtcTransport,
    relay_map: iroh::RelayMap,
    mqtt: Vec<(
        crate::mqtt_datagram::MqttFacility,
        crate::mqtt::BrowserMqttTransport,
    )>,
    irc: Vec<(IrcConfiguration, crate::irc::BrowserIrcTransport)>,
    carrier_started: n0_future::time::Instant,
    startup_request_started: Option<n0_future::time::Instant>,
    caller_budget: std::time::Duration,
    last_request: Option<serde_json::Value>,
    last_replica: Option<serde_json::Value>,
    attempt_started: Option<n0_future::time::Instant>,
    gate: BrowserGate,
    lease: Option<Lease>,
    saved: SavedNode,
    committed: String,
    delivery: Option<Delivery>,
    management: Option<management::Session>,
    pairing: Option<crate::invitation_pairing::PairingInfo>,
}

enum Startup {
    Existing,
    Create,
    Join(crate::client_invitation::ClientInvitation),
    Pairing,
    Guest(crate::service_policy::GuestInvitation),
    Recovery(crate::recovery_card::RecoveryCard, iroh_base::EndpointId),
    Restore(crate::node_backup::NodeBackup),
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
        if network.irc.is_empty() || !network.mqtt.is_empty() || !network.relay_map.is_empty() {
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
        Self::start_onboarding(configuration, caller_override, Startup::Existing).await
    }
    async fn start_onboarding(
        configuration: String,
        caller_override: Option<std::time::Duration>,
        startup: Startup,
    ) -> Result<BrowserNetwork, JsValue> {
        let input: ConfigurationInput = serde_json::from_str(&configuration).map_err(error)?;
        if let Some(policy) = &input.maintenance {
            policy.validate().map_err(error)?;
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
            Self::start_inner(configuration, caller_override, carrier_started, startup),
        )
        .await
        .map_err(|_| error("temporarily unreachable: browser startup caller deadline"))?
    }
    async fn start_inner(
        configuration: String,
        caller_override: Option<std::time::Duration>,
        carrier_started: n0_future::time::Instant,
        startup: Startup,
    ) -> Result<BrowserNetwork, JsValue> {
        let input: ConfigurationInput = serde_json::from_str(&configuration).map_err(error)?;
        let lease = Lease(acquire_node().await?.as_f64().ok_or_else(rejected)? as u32);
        let raw = read_node(lease.0).await?;
        let mut saved: SavedNode = if let Some(raw) = raw.as_string() {
            if raw.len() > 80 * 1024 * 1024 {
                return Err(error("browser state too large"));
            }
            if serde_json::from_str::<serde_json::Value>(&raw)
                .ok()
                .is_some_and(|v| v["miopunch_replacement_retired"] == true)
            {
                return Err(error(
                    "source retired after node replacement; use a separate device identity",
                ));
            }
            serde_json::from_str(&raw).map_err(|_| {
                error("browser materials corrupt; restore backup or explicitly clear and rejoin")
            })?
        } else {
            SavedNode::fresh(SecretKey::generate().to_bytes())
        };
        saved.verify()?;
        saved.validate_rotation()?;
        saved.validate_migration()?;
        if !saved.device_names.is_valid() {
            return Err(error("browser device display names are invalid"));
        }
        if saved.replacement_source.is_some() {
            return Err(error(
                "source paused for node replacement; explicitly finalize or abort the replacement",
            ));
        }
        let mut configuration = saved.resources.clone().unwrap_or_default();
        if let Some(path_selection) = input.path_selection {
            configuration.path_selection = path_selection;
        }
        if let Some(congestion_control) = input.congestion_control {
            configuration.congestion_control = congestion_control;
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
            configuration.relay_replacement = true;
        }
        if let Some(disabled) = input.disabled {
            configuration.disabled = disabled;
        }
        if let Some(mqtt) = input.mqtt {
            configuration.mqtt = mqtt;
            configuration.mqtt_replacement = true;
        }
        if let Some(irc) = input.irc {
            configuration.irc = irc;
            configuration.irc_replacement = true;
        }
        if let Some(routes) = input.contact_routes {
            configuration.contact_routes = routes;
        }
        if let Some(maintenance) = input.maintenance {
            configuration.maintenance = maintenance;
        }
        let pairing_receiver = matches!(startup, Startup::Pairing);
        let reopening_application = if matches!(&startup, Startup::Existing) && caller_override.is_none() {
            saved.admission_attempt()
        } else {
            None
        };
        match startup {
            Startup::Recovery(card, contact) => saved.prepare_device_recovery(card, contact)?,
            Startup::Restore(backup) => {
                if saved.invitation.is_some() || !saved.records.is_empty() {
                    return Err(error("replacement requires empty target storage"));
                }
                let restored = SavedNode::from_node_backup(&backup)?;
                configuration = restored.resources.clone().ok_or_else(rejected)?;
                saved = restored;
            }
            Startup::Create => saved.create()?,
            Startup::Join(invitation) => {
                configuration.import_contact(&invitation)?;
                saved.select_invitation(invitation)?;
            }
            Startup::Guest(invitation) => {
                let ordinary = invitation.ordinary_contact().map_err(error)?;
                configuration.import_contact(&ordinary)?;
                saved.select_invitation(ordinary)?;
                saved.guest = Some(invitation);
            }
            Startup::Pairing if saved.invitation.is_some() => {
                return Err(error("a network is already selected"));
            }
            Startup::Existing | Startup::Pairing => {}
        }
        if let Some(name) = input.host_device_name {
            saved
                .device_names
                .seed_local(&SigningKey::from_bytes(&saved.key), &name);
        }
        configuration.maintenance.validate().map_err(error)?;
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
        let relays = configuration.allowed_relays()?;
        configuration.validate_signaling()?;
        let pending_migration = saved.reopen_migration_resources(&configuration);
        let signaling_config = pending_migration.as_ref().unwrap_or(&configuration);
        let mut mqtt = Vec::new();
        for facility in &signaling_config.mqtt {
            if facility.check_allowed(&configuration.disabled).is_ok() {
                mqtt.push((
                    facility.clone(),
                    crate::mqtt::BrowserMqttTransport::new(facility, &configuration.disabled)?,
                ));
            }
        }
        let mut irc = Vec::new();
        for entry in &signaling_config.irc {
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
        let congestion_control = configuration.congestion_control;
        saved.resources = Some(configuration);
        if saved.invitation.is_some() {
            saved = saved.declare_files(false)?;
            let capability = clipboard_host::capability().await;
            saved = saved.declare_clipboard(capability.read && capability.write, false)?;
        }
        let committed = serde_json::to_string(&saved).map_err(error)?;
        let previous = raw
            .as_string()
            .map_or(JsValue::NULL, |raw| JsValue::from_str(&raw));
        if raw.as_string().as_deref() != Some(&committed) {
            save_node(lease.0, previous, committed.clone()).await?;
        }
        let has_carriers = !(relays.is_empty() && mqtt.is_empty() && irc.is_empty());
        // Reopened applicants can read saved progress while paths reconnect.
        let resume_offline = reopening_application.is_some_and(|attempt| {
            has_carriers || attempt.progress != ApplicationProgress::Created
        }) || saved.rotation.is_some() || saved.migration.is_some();
        if !has_carriers && !resume_offline {
            return Err(error(
                "temporarily unreachable: no allowed browser relay or MQTT carrier",
            ));
        }
        let relay_map = RelayMode::custom(relays.clone()).relay_map();
        let allowed_relays = relay_map.clone();
        let gate = BrowserGate::default();
        gate.install(&saved);
        gate.0.lock().unwrap().rtc_capability = Some(rtc_capability.clone());
        gate.0.lock().unwrap().lease = Some(lease.0);
        gate.0.lock().unwrap().member_data = member_data_transport.clone();
        gate.0.lock().unwrap().pairing_receiver = pairing_receiver;
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
            .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
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
            .congestion_control(congestion_control)
            .secret_key(SecretKey::from_bytes(&saved.key))
            .connection_path_selector(path_selector)
            .relay_mode(RelayMode::Custom(relay_map.clone()))
            .relay_resource_policy(
                RelayResourcePolicy::new(move |url, usage| match usage {
                    RelayResourceUse::DataRelay => {
                        origin(url.as_str()).is_ok_and(|url_origin| {
                            allowed_relays.urls::<Vec<_>>().iter().any(|relay| origin(relay.as_str()).as_ref().is_ok_and(|origin| *origin == url_origin))
                        })
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
            .alpns({
                let mut alpns = vec![
                    BUSINESS_ALPN.to_vec(),
                    crate::file_transfer::FILE_ALPN.to_vec(),
                    crate::clipboard::CLIPBOARD_ALPN.to_vec(),
                    crate::service_policy::POLICY_ALPN.to_vec(),
                    crate::service_policy::GUEST_ALPN.to_vec(),
                    crate::device_operation::DEVICE_ALPN.to_vec(),
                    crate::device_operation::DEVICE_RESULT_ALPN.to_vec(),
                    DEPARTURE_ALPN.to_vec(),
                    REMOVAL_ALPN.to_vec(),
                    crate::credential_custody::CUSTODY_ALPN.to_vec(),
                    device_recovery::RECOVERY_ALPN.to_vec(),
                    crate::connection_coordinator::ACTIVE_IMPROVEMENT_ALPN.to_vec(),
                    REPLICA_ALPN.to_vec(),
                    REQUEST_ALPN.to_vec(),
                    PRECODE_REQUEST_ALPN.to_vec(),
                    ADMISSION_RECEIPT_ALPN.to_vec(),
                    ADMISSION_ROTATION_ALPN.to_vec(),
                    ROTATION_DELIVERY_ALPN.to_vec(),
                ];
                if pairing_receiver {
                    alpns.push(crate::invitation_pairing::PAIRING_ALPN.to_vec());
                }
                alpns
            })
            .hooks(BrowserBeforeConnect(gate.clone()));
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
        let endpoint = builder
            .restricted_application(gate.clone())
            .bind()
            .await
            .map_err(error)?;
        if !resume_offline && n0_future::time::timeout(
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
                1,
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
                            accepting.into_control(),
                        )
                        .await
                        else {
                            return;
                        };
                        let control = Arc::new(control).retain_control();
                        let _control = RequestConnection(Some(control.clone()));
                        let lookup_gate = gate.clone();
                        let maintenance = gate.maintenance();
                        let result = n0_future::time::timeout(
                            responder_deadline
                                .saturating_duration_since(n0_future::time::Instant::now()),
                            async {
                                if gate.qualified_snapshot(&control).await.is_err() {
                                    return Err(ControlFailure::IdentityOrAdmissionRejected.into_io());
                                }
                                crate::connection_coordinator::serve_active_session(
                                    &endpoint,
                                    &control.path_handle(&endpoint),
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
                        if let Err(error) = result
                            && control.close_reason().is_none()
                        {
                            crate::connection_coordinator::close_active_failure(&control, &error);
                        }
                    }
                },
            );
        }
        let files = files::BrowserFiles::start(
            endpoint.clone(),
            gate.clone(),
            incoming.clone(),
            caller_budget,
        );
        let file_owner = files::FileOwner(files.clone());
        let admission = admission::AdmissionService::start(&incoming, &gate, &files, caller_budget);
        let clipboard = clipboard_host::BrowserClipboard::start(
            endpoint.clone(),
            gate.clone(),
            incoming.clone(),
            caller_budget,
        );
        let clipboard_owner = clipboard_host::ClipboardOwner(clipboard.clone());
        let services = services::BrowserServices::start(
            endpoint.clone(),
            gate.clone(),
            incoming.clone(),
            files.clone(),
            clipboard.clone(),
            caller_budget,
        );
        let service_owner = services::ServiceOwner(services.clone());
        let web = std::rc::Rc::new(web_host::BrowserWeb::default());
        let terminal = std::rc::Rc::new(terminal_host::BrowserTerminal::default());
        let mut network = Self {
            endpoint,
            incoming,
            admission,
            rotation: Default::default(),
            migration: Default::default(),
            files,
            _file_owner: file_owner,
            clipboard,
            forwarding: Default::default(),
            _web_owner: web_host::WebOwner(web.clone()),
            web,
            _terminal_owner: terminal_host::TerminalOwner(terminal.clone()),
            terminal,
            _clipboard_owner: clipboard_owner,
            services,
            _service_owner: service_owner,
            member_data_transport,
            member_data_lease: None,
            #[cfg(feature = "rtc-probe")]
            rtc_transport,
            relay_map,
            mqtt,
            irc,
            carrier_started,
            startup_request_started: None,
            caller_budget,
            last_request: None,
            last_replica: None,
            attempt_started: None,
            gate,
            lease: Some(lease),
            saved,
            committed,
            delivery: None,
            management: None,
            pairing: None,
        };
        if pairing_receiver {
            let (address, bootstrap) = network.contact();
            network.pairing = Some(crate::invitation_pairing::PairingInfo::new(
                address, bootstrap,
            ));
        }
        network
            .admission
            .start_receipts(&network, network.lease.as_ref().unwrap().0);
        network.services.start_member_sync(&network);
        network.rotation.start(&network);
        network.start_migration_worker();
        Ok(network)
    }
}

#[wasm_bindgen]
impl BrowserNetwork {
    pub fn id(&self) -> String {
        self.endpoint.id().to_string()
    }
    pub fn status(&self) -> String {
        let saved = self
            .gate
            .service_snapshot()
            .unwrap_or_else(|_| self.saved.clone());
        saved
            .public_status(self.pairing.is_some(), self.lease.is_some())
            .to_string()
    }

    pub fn state_snapshot(&self) -> Result<String, JsValue> {
        self.gate
            .state_snapshot(self.pairing.is_some(), self.lease.is_some())
    }

    /// Persist this device's display name without contacting other members.
    pub async fn set_device_display_name(&mut self, display_name: String) -> Result<(), JsValue> {
        services::update_saved(&self.gate, |saved| {
            let key = SigningKey::from_bytes(&saved.key);
            saved.device_names.set_local(&key, &display_name).map_err(error)
        })
        .await?;
        self.refresh_service_snapshot().await
    }

    /// The promise owns its subscription, leaving the node available for other
    /// operations while the host waits for its next applied snapshot.
    pub fn observe_state(&self, after: String) -> Result<js_sys::Promise, JsValue> {
        let after = after.parse::<u64>().map_err(error)?;
        let mut updates = self.gate.0.lock().unwrap().changes.subscribe();
        let gate = self.gate.clone();
        let pairing = self.pairing.is_some();
        let cancellation = self.incoming.cancellation();
        Ok(wasm_bindgen_futures::future_to_promise(async move {
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => Err(error("browser node disconnected")),
                changed = updates.wait_for(|cursor| *cursor != after) => {
                    changed.map_err(error)?;
                    gate.state_snapshot(pairing, true).map(|snapshot| JsValue::from_str(&snapshot))
                }
            }
        }))
    }

    /// Redacted loaded material receipt, checked against the actual leased store.
    pub async fn admission_material_receipt(&self) -> Result<String, JsValue> {
        let (saved, _, _) = services::installed(&self.gate).await?;
        let invitation = saved.invitation()?;
        Ok(serde_json::json!({"id":self.id(),"contact":invitation.contact.to_string(),"stage":saved.stage(),
            "request":hex::encode(saved.admission_request_id()),
            "authorization":saved.authorization.map(|id|id.to_string()),"generation":saved.generation,
            "authorization_saved":saved.authorization.is_some(),"secret_saved":saved.secret.is_some(),
            "records":saved.records.iter().map(|(header,_)|Hash::digest(header).to_string()).collect::<Vec<_>>()}).to_string())
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
            .connect_control_target_until(self.saved.invitation()?.contact, REQUEST_ALPN, deadline)
            .await?;
        let _scope = RequestConnection(Some(connection.clone()));
        n0_future::time::timeout(deadline.saturating_duration_since(n0_future::time::Instant::now()),async {
            let exchange=connection.open_control_exchange().await.map_err(error)?;
            let mut wire=self.saved.invitation.clone().ok_or_else(rejected)?;
            wire.extend_from_slice(&request);wire.extend_from_slice(claimed.as_bytes());
            send_fixed(&exchange,&wire).await?;exchange.finish_send().await.map_err(error)?;
            let response=recv_fixed::<1>(&exchange).await?;
            if response!=[0] {return Err(error("identity probe unexpectedly obtained approved material"));}
            Ok(serde_json::json!({"local_id":self.id(),"remote_id":connection.remote_id().to_string(),"claimed":claimed.to_string(),
                "alpn":String::from_utf8_lossy(connection.alpn()),"stable_id":connection.stable_id(),"approved_material":false,"response":response[0]}).to_string())
        }).await.map_err(|_|error("identity probe caller deadline"))?
    }

    pub async fn request_admission(&mut self, invitation: String) -> Result<String, JsValue> {
        self.refresh_service_snapshot().await?;
        if self.saved.stage() == "Removed" {
            return Err(error("membership ended; start a new rejoin attempt"));
        }
        if self.saved.manual_rejected {
            return Ok("Rejected".into());
        }
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
        if self.saved.admission_request_id() != next.admission_request_id() {
            next.reset_admission_progress();
        }
        if next.complete_invitation.as_ref().is_some_and(|full| {
            full.material_bytes()
                .ok()
                .as_ref()
                .map(|bytes| bytes.as_slice())
                != next.invitation.as_deref()
                || full.precode_bytes().ok().flatten().is_some()
        }) {
            next.complete_invitation = None;
        }
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
        result.map_err(|_| admission_timeout())?
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
        serde_json::json!({"incoming":self.incoming.diagnostics(),"relayHome":relay_home,"rtcCapability":gate.rtc_capability,"relayChecks":gate.relay_checks.iter().flat_map(|attempt| attempt.checks.iter()).collect::<Vec<_>>(),"request":self.last_request,"gate":gate.request_handshake,"admissionReceipt":gate.receipt_delivery,"migrationFailure":gate.migration_failure,"memberBootstraps":bootstraps,"carriers":self.mqtt.iter().map(|(_,transport)|serde_json::from_str::<serde_json::Value>(&transport.diagnostics()).unwrap()).chain(self.irc.iter().map(|(_,transport)|serde_json::from_str::<serde_json::Value>(&transport.diagnostics()).unwrap())).collect::<Vec<_>>()}).to_string()
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
        self.refresh_service_snapshot().await?;
        if self.saved.stage() == "Removed" {
            return Err(error("membership ended; start a new rejoin attempt"));
        }
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
        if self.saved.admission_request_id() != next.admission_request_id() {
            next.reset_admission_progress();
        }
        if next.complete_invitation.as_ref().is_some_and(|full| {
            full.precode_bytes()
                .ok()
                .flatten()
                .as_ref()
                .map(|code| code.as_slice())
                != Some(bytes.as_slice())
        }) {
            next.complete_invitation = None;
        }
        self.last_request = Some(serde_json::json!({"stage":"connecting","business_handle":false}));
        n0_future::time::timeout(
            self.caller_budget.saturating_sub(started.elapsed()),
            async {
                self.commit(next).await?;
                self.request_inner().await
            },
        )
        .await
        .map_err(|_| admission_timeout())?
    }

    pub async fn complete_admission(&mut self) -> Result<String, JsValue> {
        self.gate.service_snapshot()?.admissions_open()?;
        let delivery = self.delivery.take().ok_or_else(rejected)?;
        let remaining = delivery
            .deadline
            .saturating_duration_since(n0_future::time::Instant::now());
        if remaining.is_zero() {
            return Err(admission_timeout());
        }
        if let Some(report) = self.last_request.as_mut() {
            report["completion_remaining_ms"] = serde_json::json!(remaining.as_millis());
            report["material_events"] = serde_json::json!([]);
        }
        let auth = self.saved.verify()?.ok_or_else(rejected)?;
        if !auth.membership_authorization_valid(
            self.saved.authorization.ok_or_else(rejected)?,
            self.saved.identity(),
        ) {
            return Err(rejected());
        }
        let material = n0_future::time::timeout(remaining, async {
            send_fixed(&delivery.exchange, b"AUTH").await?;
            self.material_event("AUTH-sent");
            recv_fixed::<40>(&delivery.exchange).await
        })
        .await
        .map_err(|_| admission_timeout())??;
        if u64::from_be_bytes(material[..8].try_into().unwrap()) != self.saved.generation {
            return Err(rejected());
        }
        self.material_event("secret-received-generation-verified");
        let next = self.saved.with_admission_secret(material)?;
        let receipt = next.admission_receipt()?;
        let lease = self.lease.as_ref().ok_or_else(rejected)?.0;
        let pending = next.pending_admission_receipt()?;
        // IndexedDB writes that have started can commit after their Rust future
        // is dropped. Await installation before returning a network timeout.
        let raw =
            services::commit_admission_material(&self.gate, &self.committed, lease, &next, pending)
                .await?;
        self.saved = next;
        self.committed = raw;
        self.material_event("secret-committed");
        let remaining = delivery
            .deadline
            .saturating_duration_since(n0_future::time::Instant::now());
        let received = async {
            if remaining.is_zero() {
                return Err(admission_timeout());
            }
            let reply = n0_future::time::timeout(remaining, async {
                send_fixed(&delivery.exchange, b"SAVE").await?;
                self.material_event("SAVE-sent");
                delivery.exchange.finish_send().await.map_err(error)?;
                recv_fixed::<4>(&delivery.exchange).await
            })
            .await
            .map_err(|_| admission_timeout())??;
            if &reply != b"DONE" {
                return Err(rejected());
            }
            self.material_event("DONE-received");
            admission::settle_receipt(lease, receipt).await
        }
        .await;
        if received.is_err() {
            self.admission.receipt_ready.notify_one();
        }
        received?;
        Ok(self.saved.stage().to_string())
    }

    /// Controlled member protocol, with a fresh native gate qualification.
    pub async fn exchange(&self, message: String) -> Result<String, JsValue> {
        let saved = self.gate.service_snapshot()?;
        if message.len() > 65536 || saved.stage() != "Complete" {
            return Err(rejected());
        }
        saved.verify()?;
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
        self.forwarding.close().await;
        self.stop_runtime().await;
        if let Some(lease) = self.lease.take() {
            let _ = release_node_and_wait(lease.0).await;
        }
    }
    pub(super) async fn stop_runtime(&mut self) {
        self.migration.stop().await;
        self.terminal.close().await;
        self.web.close().await;
        self.clipboard.close().await;
        self.services.close().await;
        self.files.close().await;
        self.delivery = None;
        self.management = None;
        self.gate.0.lock().unwrap().pairing_peer = None;
        self.gate.0.lock().unwrap().pairing_receiver = false;
        self.gate.retire_member_data();
        if let Some(mut lease) = self.member_data_lease.take() {
            let _ = lease.close().await;
        }
        self.incoming.close();
        self.incoming.stopped().await;
        self.rotation.stopped().await;
        self.endpoint.close().await;
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
        self.lease
            .as_ref()
            .ok_or_else(|| error("browser node disconnected"))?;
        services::installed(&self.gate).await?;
        Ok(())
    }
    async fn commit(&mut self, next: SavedNode) -> Result<(), JsValue> {
        let token = self.lease.as_ref().ok_or_else(rejected)?.0;
        let raw = services::commit_installed(&self.gate, &self.committed, token, &next).await?;
        self.saved = next;
        self.committed = raw;
        Ok(())
    }
    async fn connect(&self, alpn: &[u8]) -> Result<Connection, JsValue> {
        self.check_store().await?;
        let invitation = self.gate.service_snapshot()?.invitation()?;
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
    fn connection_coordinator(&self) -> crate::connection_coordinator::ConnectionCoordinator {
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
        crate::connection_coordinator::ConnectionCoordinator::new_restricted(
            self.endpoint.clone(),
            caps.permitted,
            contact_ids,
        )
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
            let coordinator = self.connection_coordinator();
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

    async fn connect_control_target_until(
        &self,
        target: iroh_base::EndpointId,
        alpn: &[u8],
        deadline: n0_future::time::Instant,
    ) -> Result<ControlOwner, JsValue> {
        self.check_store().await?;
        let addr = self.target_address(target)?;
        if crate::connection_coordinator::SessionPurpose::from_alpn(alpn).is_some() {
            let coordinator = self.connection_coordinator();
            return coordinator
                .connect_control_alpn(
                    addr,
                    alpn,
                    crate::connection_coordinator::AttemptContext::new(
                        deadline,
                        tokio_util::sync::CancellationToken::new(),
                    ),
                )
                .await
                .map_err(|failure| match failure {
                    crate::connection_coordinator::ConnectError::DeadlineExceeded
                        if is_request_alpn(alpn) => admission_timeout(),
                    _ => error(failure),
                });
        }
        Err(error("unsupported browser control purpose"))
    }

    fn target_address(&self, target: iroh_base::EndpointId) -> Result<EndpointAddr, JsValue> {
        self.gate.service_snapshot()?.address_for(target)
    }
    async fn request_inner(&mut self) -> Result<String, JsValue> {
        let started = n0_future::time::Instant::now();
        let deadline = self.attempt_started.unwrap_or(started) + self.caller_budget;
        let alpn = self.saved.request_alpn();
        let connection = self
            .connect_control_target_until(self.saved.invitation()?.contact, alpn, deadline)
            .await?;
        let mut request_scope = RequestConnection(Some(connection.clone()));
        self.last_request = Some(
            serde_json::json!({"local_id":self.endpoint.id().to_string(),"remote_id":connection.remote_id().to_string(),"alpn":String::from_utf8_lossy(connection.alpn()),"stable_id":connection.stable_id(),"restricted":true,"business_handle":false,"handshake_ms":started.elapsed().as_millis()}),
        );
        let result = async {
            let exchange = connection.open_control_exchange().await.map_err(error)?;
            let status = self.saved.write_admission_request(&exchange).await?;
            if matches!(status, 0 | 1) {
                let mut next = self.saved.clone();
                next.record_admission_progress(ApplicationProgress::Delivered);
                if next.confirmed_admission != self.saved.confirmed_admission {
                    self.commit(next).await?;
                }
            }
            if status == 0 {
                exchange.finish_send().await.map_err(error)?;
                return Ok("AwaitingApproval".to_string());
            }
            if status == 2 && self.saved.precode.is_none() {
                let mut next = self.saved.clone();
                next.manual_rejected = true;
                next.record_admission_progress(ApplicationProgress::Rejected);
                self.commit(next).await?;
                exchange.finish_send().await.map_err(error)?;
                return Ok("Rejected".into());
            }
            if status != 1 {
                return Err(rejected());
            }
            let next = self.saved.authorized_admission(&exchange).await?;
            self.commit(next).await?;
            self.delivery = Some(Delivery {
                connection: connection.clone(),
                deadline,
                exchange,
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
            report["stats"] = serde_json::json!(format!(
                "{:?}",
                connection.path_handle(&self.endpoint).stats()
            ));
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
    endpoint: RestrictedEndpoint,
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
#[path = "network_migration.rs"]
mod migration;

#[path = "member_connection.rs"]
mod member_connection;
pub use member_connection::{
    BrowserMemberAcceptAttempt, BrowserMemberAttempt, BrowserMemberConnection,
};

#[path = "incoming.rs"]
mod incoming;

#[path = "clipboard.rs"]
mod clipboard_host;
#[path = "device_custody.rs"]
mod device_custody;
#[path = "device_management.rs"]
mod device_management;
#[path = "device_recovery.rs"]
mod device_recovery;
#[path = "files.rs"]
mod files;
#[path = "forwarding.rs"]
mod forwarding_host;
#[path = "node_replacement.rs"]
mod node_replacement;
#[path = "services.rs"]
mod services;
#[path = "terminal.rs"]
mod terminal_host;
#[path = "web.rs"]
mod web_host;

#[path = "data_stream.rs"]
mod data_stream;

#[cfg(feature = "rtc-probe")]
#[path = "rtc.rs"]
mod rtc;
#[cfg(feature = "rtc-probe")]
#[path = "rtc_member.rs"]
mod rtc_member;

#[path = "member_data_sdk.rs"]
mod member_data_sdk;
use member_data_sdk::{MemberDataConfiguration, MemberDataQualificationSource};

#[path = "admission_rotation.rs"]
mod rotation;

#[path = "onboarding.rs"]
mod onboarding;
