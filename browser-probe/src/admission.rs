//! Browser administrator adapter. Approval, signed grant and precode count are
//! committed together by the same strict IndexedDB snapshot transaction.
use super::*;
use crate::management_action::ManagementAction;
use crate::admission_progress::{
    AdmissionHistoryEntry, AdmissionReceipt, RECEIPT_DONE, RECEIPT_STOP, RECENT_ADMISSION_ATTEMPTS,
};
use p2panda_core::SigningKey;
use std::rc::Rc;

pub(super) use crate::node_control_backup::{SavedApproval as Approval, SavedCode as Code};
fn now() -> Result<u64, JsValue> {
    n0_future::time::SystemTime::now()
        .duration_since(n0_future::time::SystemTime::UNIX_EPOCH)
        .map(|time| time.as_secs())
        .map_err(error)
}

impl SavedNode {
    pub(super) fn verify_ledger(&self) -> Result<(), JsValue> {
        if self.codes.len() > 4096 || self.approvals.len() > 4096 {
            return Err(rejected());
        }
        let mut codes = std::collections::BTreeSet::new();
        for code in &self.codes {
            let actual = self
                .approvals
                .iter()
                .filter(|a| a.code == Some(code.token) && a.authorization.is_some())
                .count() as u64;
            if !codes.insert(code.token)
                || code.used != actual
                || code.limit == Some(0)
                || code.limit.is_some_and(|limit| code.used > limit)
            {
                return Err(error("precode ledger inconsistent; old codes stopped"));
            }
        }
        let mut requests = std::collections::BTreeSet::new();
        for approval in &self.approvals {
            if !requests.insert((approval.applicant, approval.request))
                || approval.code.is_some_and(|token| !codes.contains(&token))
                || approval.complete && approval.authorization.is_none()
                || approval.rejected
                    && (approval.authorization.is_some() || approval.code.is_some())
            {
                return Err(rejected());
            }
        }
        Ok(())
    }
}

// Admission exchanges share the dispatcher with other protocols. No handler
// holds the state writer while waiting for the applicant on the network.
pub(super) struct AdmissionService {
    manual: tokio::sync::broadcast::Sender<Result<&'static str, String>>,
    precode: tokio::sync::broadcast::Sender<Result<&'static str, String>>,
    pub(super) receipts: Rc<tokio::sync::Mutex<Option<AdmissionReceipt>>>,
}

impl AdmissionService {
    pub(super) fn start(
        incoming: &incoming::IncomingDispatcher,
        gate: &BrowserGate,
        files: &std::rc::Rc<files::BrowserFiles>,
        budget: Duration,
    ) -> Self {
        let (manual, _) = tokio::sync::broadcast::channel(1);
        let (precode, _) = tokio::sync::broadcast::channel(1);
        for (protocol, results) in [(REQUEST_ALPN, &manual), (PRECODE_REQUEST_ALPN, &precode)] {
            let gate = gate.clone();
            let files = files.clone();
            let results = results.clone();
            incoming.install_active(
                protocol,
                4,
                move || budget,
                move |accepting, deadline| {
                    let gate = gate.clone();
                    let files = files.clone();
                    let results = results.clone();
                    async move {
                        let result = receive_admission(accepting, deadline, &gate, &files).await;
                        let _ = results.send(result.map_err(|cause| {
                            cause
                                .as_string()
                                .unwrap_or_else(|| "admission failed".into())
                        }));
                    }
                },
            );
        }
        let receipt_gate = gate.clone();
        incoming.install_active(
            ADMISSION_RECEIPT_ALPN,
            4,
            move || budget,
            move |accepting, deadline| {
                let gate = receipt_gate.clone();
                async move {
                    let _ = receive_receipt(accepting, deadline, &gate).await;
                }
            },
        );
        Self {
            manual,
            precode,
            receipts: Default::default(),
        }
    }

    pub(super) fn start_receipts(&self, network: &BrowserNetwork) {
        let gate = network.gate.clone();
        let settled = self.receipts.clone();
        let coordinator = network.connection_coordinator();
        let relays = network.relays.clone();
        let routes = network.contact_routes.clone();
        let budget = network.caller_budget;
        let cancel = network.incoming.cancellation();
        let mut changes = gate.0.lock().unwrap().changes.subscribe();
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                let work = async {
                    let mut settled = settled.lock().await;
                    let saved = gate.service_snapshot()?;
                    let Some((receipt, contact)) = saved.pending_receipt()? else {
                        return Ok(());
                    };
                    if *settled == Some(receipt) {
                        return Ok(());
                    }
                    let (_, _, lease) = services::installed(&gate).await?;
                    let previous = read_admission_receipt(lease).await?;
                    let next = hex::encode(receipt.encode());
                    if previous.as_string().as_deref() != Some(next.as_str()) {
                        let target = target_address(contact, &relays, &routes)?;
                        let deadline = n0_future::time::Instant::now() + budget;
                        let attempt = crate::connection_coordinator::AttemptContext::new(
                            deadline,
                            cancel.clone(),
                        );
                        let connection = coordinator
                            .connect_control_alpn(target, ADMISSION_RECEIPT_ALPN, attempt)
                            .await
                            .map_err(error)?;
                        let _scope = RequestConnection(Some(connection.clone()));
                        if gate.service_snapshot()?.pending_receipt()? != Some((receipt, contact)) {
                            return Ok(());
                        }
                        until(deadline, async {
                            let exchange =
                                connection.open_control_exchange().await.map_err(error)?;
                            send_fixed(&exchange, &receipt.encode()).await?;
                            exchange.finish_send().await.map_err(error)?;
                            match &recv_fixed::<4>(&exchange).await? {
                                RECEIPT_DONE | RECEIPT_STOP => Ok(()),
                                _ => Err(rejected()),
                            }
                        })
                        .await?;
                        save_admission_receipt(lease, previous, next).await?;
                    }
                    *settled = Some(receipt);
                    Ok::<(), JsValue>(())
                };
                let result = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => break,
                    result = work => result,
                };
                if result.is_err() {
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => break,
                        _ = n0_future::time::sleep(Duration::from_secs(2)) => {}
                    }
                } else {
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => break,
                        _ = changes.changed() => {}
                    }
                }
            }
        });
    }
}

#[wasm_bindgen(module = "/src/storage.js")]
extern "C" {
    #[wasm_bindgen(catch, js_name = readAdmissionReceipt)]
    async fn read_admission_receipt(lease: u32) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_name = saveAdmissionReceipt)]
    async fn save_admission_receipt(
        lease: u32,
        previous: JsValue,
        next: String,
    ) -> Result<JsValue, JsValue>;
}

pub(super) async fn settle_receipt(lease: u32, receipt: AdmissionReceipt) -> Result<(), JsValue> {
    let previous = read_admission_receipt(lease).await?;
    let next = hex::encode(receipt.encode());
    if previous.as_string().as_deref() != Some(next.as_str()) {
        save_admission_receipt(lease, previous, next).await?;
    }
    Ok(())
}

impl SavedNode {
    pub(super) fn pending_receipt(
        &self,
    ) -> Result<Option<(AdmissionReceipt, iroh_base::EndpointId)>, JsValue> {
        let Some(attempt) = self.admission_attempt() else {
            return Ok(None);
        };
        let Some(authorization) = self.authorization else {
            return Ok(None);
        };
        if attempt.progress != ApplicationProgress::Complete
            || self.replacement_source.is_some()
            || self.secret.is_none()
        {
            return Ok(None);
        }
        let invitation = self.invitation()?;
        let provider = VerifyingKey::from_bytes(invitation.contact.as_bytes()).map_err(error)?;
        if !self.verify()?.is_some_and(|auth| {
            auth.nodes().contains(&(provider, Access::manage()))
                && auth.membership_authorization_valid(authorization, self.identity())
        }) {
            return Ok(None);
        }
        Ok(Some((
            AdmissionReceipt {
                genesis: invitation.anchor.genesis,
                request_id: attempt.request_id,
                authorization,
            },
            invitation.contact,
        )))
    }
}

#[wasm_bindgen]
impl BrowserNetwork {
    pub fn manual_invitation(&self) -> Result<String, JsValue> {
        let saved = self.gate.service_snapshot()?;
        saved.administrator()?;
        Ok(hex::encode(saved.own_invitation()?))
    }

    pub fn admission_requests(&self) -> String {
        self.gate
            .service_snapshot()
            .expect("installed browser node")
            .admission_requests()
            .to_string()
    }

    /// Recent locally handled requests, including historical delivery outcomes.
    pub fn admission_history(&self) -> String {
        self.gate
            .service_snapshot()
            .expect("installed browser node")
            .admission_history()
            .to_string()
    }

    pub async fn approve_admission(
        &mut self,
        applicant: String,
        request: String,
    ) -> Result<String, JsValue> {
        let (applicant, request) = parse_request(&applicant, &request)?;
        let id = services::update_saved(&self.gate, |saved| {
            let index = saved
                .approval_index(applicant, request)
                .ok_or_else(rejected)?;
            let auth = saved.administrator()?;
            saved.approve(index, auth)
        })
        .await?;
        self.files.enforce_access();
        Ok(id.to_string())
    }

    pub async fn reject_admission(
        &mut self,
        applicant: String,
        request: String,
    ) -> Result<(), JsValue> {
        let (applicant, request) = parse_request(&applicant, &request)?;
        services::update_saved(&self.gate, |saved| {
            saved.administrator()?;
            let index = saved
                .approval_index(applicant, request)
                .ok_or_else(rejected)?;
            let approval = &mut saved.approvals[index];
            if approval.authorization.is_some() || approval.code.is_some() {
                return Err(rejected());
            }
            approval.rejected = true;
            Ok(())
        })
        .await
    }

    /// Return bearer material only to the caller for trusted out-of-band delivery.
    pub async fn issue_precode(
        &mut self,
        expires_unix_seconds: u64,
        max_uses: Option<u64>,
    ) -> Result<String, JsValue> {
        services::update_saved(&self.gate, |saved| {
            let auth = saved.administrator()?;
            if !crate::precode_policy::valid_limits(expires_unix_seconds, max_uses, now()?)
                || saved.codes.len() >= 4096
            {
                return Err(rejected());
            }
            let origin = auth
                .administrator_authorization_origin(saved.identity())
                .ok_or_else(rejected)?;
            let token: [u8; 32] = rand::random();
            let mut code = saved.own_invitation()?;
            code.extend_from_slice(&token);
            saved.codes.push(Code {
                token,
                origin,
                expires: expires_unix_seconds,
                limit: max_uses,
                used: 0,
                revoked: false,
                stopped: false,
            });
            Ok(hex::encode(code))
        })
        .await
    }

    pub async fn revoke_precode(&mut self, code: String) -> Result<(), JsValue> {
        services::update_saved(&self.gate, |saved| {
            let token = saved.own_code(&code)?;
            saved
                .codes
                .iter_mut()
                .find(|code| code.token == token)
                .ok_or_else(rejected)?
                .revoked = true;
            Ok(())
        })
        .await
    }

    pub fn precode_usage(&self, code: String) -> Result<u64, JsValue> {
        let saved = self.gate.service_snapshot()?;
        let token = saved.own_code(&code)?;
        saved
            .codes
            .iter()
            .find(|code| code.token == token)
            .map(|code| code.used)
            .ok_or_else(rejected)
    }

    /// Compatibility hook for hosts waiting on one completed exchange.
    /// Incoming requests are accepted independently of these calls.
    pub fn serve_admission(&self, precode: bool) -> Result<js_sys::Promise, JsValue> {
        self.administrator()?;
        let source = if precode {
            &self.admission.precode
        } else {
            &self.admission.manual
        };
        let mut results = source.subscribe();
        let budget = self.caller_budget;
        let cancellation = self.incoming.cancellation();
        Ok(wasm_bindgen_futures::future_to_promise(async move {
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => Err(error("browser node disconnected")),
                result = n0_future::time::timeout(budget, results.recv()) => {
                    let result = result.map_err(error)?.map_err(error)?.map_err(error)?;
                    Ok(JsValue::from_str(result))
                }
            }
        }))
    }
}

impl BrowserNetwork {
    pub(super) fn administrator(&self) -> Result<ManagementAuth, JsValue> {
        self.gate.service_snapshot()?.administrator()
    }

    pub(super) fn own_invitation(&self) -> Result<Vec<u8>, JsValue> {
        self.gate.service_snapshot()?.own_invitation()
    }
}

fn parse_request(applicant: &str, request: &str) -> Result<(VerifyingKey, [u8; 16]), JsValue> {
    let applicant = applicant.parse().map_err(error)?;
    let request = hex::decode(request)
        .map_err(error)?
        .try_into()
        .map_err(|_| rejected())?;
    Ok((applicant, request))
}

impl SavedNode {
    pub(super) fn admission_requests(&self) -> serde_json::Value {
        serde_json::json!(self.approvals.iter().map(|a| serde_json::json!({
            "applicant":a.applicant.to_string(),"request":hex::encode(a.request),
            "approved":a.authorization.is_some(),"rejected":a.rejected,"delivery_confirmed":a.complete
        })).collect::<Vec<_>>())
    }

    pub(super) fn pending_admissions(&self) -> serde_json::Value {
        serde_json::json!(self.approvals.iter().filter(|a| a.authorization.is_none() && !a.rejected)
            .map(|a| serde_json::json!({"applicant":a.applicant.to_string(),"request_id":hex::encode(a.request)}))
            .collect::<Vec<_>>())
    }

    pub(super) fn admission_history(&self) -> serde_json::Value {
        serde_json::json!(
            self.approvals
                .iter()
                .rev()
                .take(RECENT_ADMISSION_ATTEMPTS)
                .map(|approval| AdmissionHistoryEntry::from_evidence(
                    approval.applicant,
                    approval.request,
                    approval.authorization,
                    approval.rejected,
                    approval.complete
                )
                .public_json())
                .collect::<Vec<_>>()
        )
    }

    pub(super) fn administrator(&self) -> Result<ManagementAuth, JsValue> {
        let auth = self.verify()?.ok_or_else(rejected)?;
        if self.stage() != "Complete"
            || !auth.nodes().contains(&(self.identity(), Access::manage()))
        {
            return Err(rejected());
        }
        Ok(auth)
    }

    fn own_invitation(&self) -> Result<Vec<u8>, JsValue> {
        let anchor = self.invitation()?.anchor;
        let mut bytes = anchor.root.as_bytes().to_vec();
        bytes.extend_from_slice(anchor.genesis.as_bytes());
        bytes.extend_from_slice(self.identity().as_bytes());
        Ok(bytes)
    }

    fn own_code(&self, code: &str) -> Result<[u8; 32], JsValue> {
        let bytes = hex::decode(code).map_err(error)?;
        if bytes.len() != 128 || bytes[..96] != self.own_invitation()? {
            return Err(rejected());
        }
        Ok(bytes[96..].try_into().unwrap())
    }

    fn approval_index(&self, applicant: VerifyingKey, request: [u8; 16]) -> Option<usize> {
        self.approvals
            .iter()
            .position(|a| (a.applicant, a.request) == (applicant, request))
    }

    fn approve(&mut self, index: usize, mut auth: ManagementAuth) -> Result<Hash, JsValue> {
        let approval = &self.approvals[index];
        let applicant = approval.applicant;
        if approval.rejected || applicant == self.invitation()?.anchor.root {
            return Err(rejected());
        }
        if let Some(id) = approval.authorization {
            return auth
                .membership_authorization_valid(id, applicant)
                .then_some(id)
                .ok_or_else(rejected);
        }
        let existing = self
            .records
            .iter()
            .rev()
            .map(|(h, _)| Hash::digest(h))
            .find(|id| auth.membership_authorization_valid(*id, applicant));
        let id = if let Some(id) = existing {
            id
        } else {
            let header = self.author_action(
                &auth,
                &SigningKey::from_bytes(&self.key),
                ManagementAction::AddMember(applicant),
                None,
            )?;
            let id = auth.accept(&header).map_err(error)?;
            self.records.push((header, None));
            id
        };
        self.approvals[index].authorization = Some(id);
        self.approvals[index].delivery_phase = Some(1);
        Ok(id)
    }

    fn receive_request(
        &mut self,
        wire: &[u8; 144],
        applicant: VerifyingKey,
        request: [u8; 16],
        token: Option<[u8; 32]>,
    ) -> Result<Approval, JsValue> {
        let auth = self.administrator()?;
        if wire[..96] != self.own_invitation()? {
            return Err(rejected());
        }
        let index = if let Some(index) = self.approval_index(applicant, request) {
            if self.approvals[index].code != token {
                return Err(rejected());
            }
            index
        } else {
            if self.approvals.len() >= 4096 {
                return Err(rejected());
            }
            if let Some(token) = token {
                self.redeem_code(token, &auth)?;
            }
            self.approvals.push(Approval {
                applicant,
                request,
                authorization: None,
                code: token,
                complete: false,
                rejected: false,
                delivery_phase: Some(0),
            });
            self.approvals.len() - 1
        };
        if token.is_some() {
            self.approve(index, auth)?;
        }
        Ok(self.approvals[index].clone())
    }

    fn redeem_code(&mut self, token: [u8; 32], auth: &ManagementAuth) -> Result<(), JsValue> {
        let identity = self.identity();
        let code = self
            .codes
            .iter_mut()
            .find(|code| code.token == token)
            .ok_or_else(rejected)?;
        let actual = self
            .approvals
            .iter()
            .filter(|a| a.code == Some(token) && a.authorization.is_some())
            .count() as u64;
        let policy = crate::precode_policy::RedemptionPolicy {
            expires: code.expires,
            max_uses: code.limit,
            used: code.used,
            stopped: code.stopped,
            revoked: code.revoked,
            ledger_consistent: code.used == actual,
            issuer_authorized: auth.administrator_authorization_valid(code.origin, identity),
        };
        if !policy.permits_new(now()?) {
            return Err(rejected());
        }
        code.used = code.used.checked_add(1).ok_or_else(rejected)?;
        Ok(())
    }

    fn verify_delivery(&self, authorization: Hash, applicant: VerifyingKey) -> Result<(), JsValue> {
        self.administrator()?
            .membership_authorization_valid(authorization, applicant)
            .then_some(())
            .ok_or_else(rejected)
    }
}

async fn until<T>(
    deadline: n0_future::time::Instant,
    work: impl std::future::Future<Output = Result<T, JsValue>>,
) -> Result<T, JsValue> {
    n0_future::time::timeout(
        deadline.saturating_duration_since(n0_future::time::Instant::now()),
        work,
    )
    .await
    .map_err(|_| error("admission delivery timed out"))?
}

async fn receive_receipt(
    accepting: iroh::endpoint::RestrictedAccepting,
    deadline: n0_future::time::Instant,
    gate: &BrowserGate,
) -> Result<(), JsValue> {
    until(deadline, async {
        let connection = Arc::new(
            accepting
                .with_caller_deadline(deadline)
                .into_control()
                .await
                .map_err(error)?,
        )
        .retain_control();
        let _scope = RequestConnection(Some(connection.clone()));
        let exchange = connection.accept_control_exchange().await.map_err(error)?;
        let receipt = AdmissionReceipt::decode(recv_fixed::<80>(&exchange).await?);
        let applicant =
            VerifyingKey::from_bytes(connection.remote_id().as_bytes()).map_err(error)?;
        let response = services::update_saved(gate, |saved| {
            if saved.invitation()?.anchor.genesis != receipt.genesis {
                return Err(rejected());
            }
            let index = saved
                .approval_index(applicant, receipt.request_id)
                .ok_or_else(rejected)?;
            if saved.approvals[index].authorization != Some(receipt.authorization) {
                return Err(rejected());
            }
            if saved
                .verify_delivery(receipt.authorization, applicant)
                .is_err()
            {
                return Ok(RECEIPT_STOP);
            }
            saved.approvals[index].complete = true;
            saved.approvals[index].delivery_phase = Some(3);
            Ok(RECEIPT_DONE)
        })
        .await?;
        send_fixed(&exchange, response).await?;
        exchange.finish_send().await.map_err(error)?;
        connection.revocation_handle().closed().await;
        Ok(())
    })
    .await
}

async fn receive_admission(
    accepting: iroh::endpoint::RestrictedAccepting,
    deadline: n0_future::time::Instant,
    gate: &BrowserGate,
    files: &std::rc::Rc<files::BrowserFiles>,
) -> Result<&'static str, JsValue> {
    let connection = until(deadline, async {
        accepting
            .with_caller_deadline(deadline)
            .into_control()
            .await
            .map_err(error)
    })
    .await?;
    let connection = Arc::new(connection).retain_control();
    let _scope = RequestConnection(Some(connection.clone()));
    let (exchange, wire, applicant, request, token) = until(deadline, async {
        let exchange = connection.accept_control_exchange().await.map_err(error)?;
        let wire = recv_fixed::<144>(&exchange).await?;
        if wire[112..] != connection.remote_id().as_bytes()[..] {
            return Err(rejected());
        }
        let applicant =
            VerifyingKey::from_bytes(&wire[112..].try_into().unwrap()).map_err(error)?;
        let request: [u8; 16] = wire[96..112].try_into().unwrap();
        let token = if connection.alpn() == PRECODE_REQUEST_ALPN {
            Some(recv_fixed::<32>(&exchange).await?)
        } else {
            None
        };
        Ok((exchange, wire, applicant, request, token))
    })
    .await?;

    let approval = services::update_saved(gate, |saved| {
        saved.receive_request(&wire, applicant, request, token)
    })
    .await?;
    files.enforce_access();
    let result = if let Some(authorization) = approval.authorization {
        let (saved, _, _) = services::installed(gate).await?;
        saved.verify_delivery(authorization, applicant)?;
        let history =
            p2panda_core::cbor::encode_cbor(&(authorization, &saved.records)).map_err(error)?;
        if history.len() > MAX_HISTORY {
            return Err(rejected());
        }
        until(deadline, async {
            send_fixed(&exchange, &[1]).await?;
            send_fixed(&exchange, &saved.generation.to_be_bytes()).await?;
            exchange
                .send_message(
                    ControlMessageFormat::LengthPrefixed32 {
                        maximum: MAX_HISTORY,
                    },
                    &history,
                )
                .await
                .map_err(error)?;
            if &recv_fixed::<4>(&exchange).await? != b"AUTH" {
                return Err(rejected());
            }
            Ok(())
        })
        .await?;
        let (current, _, _) = services::installed(gate).await?;
        current.verify_delivery(authorization, applicant)?;
        if current.generation != saved.generation {
            return Err(rejected());
        }
        until(deadline, async {
            send_fixed(&exchange, &current.generation.to_be_bytes()).await?;
            send_fixed(&exchange, &current.secret.ok_or_else(rejected)?).await?;
            if &recv_fixed::<4>(&exchange).await? != b"SAVE" {
                return Err(rejected());
            }
            Ok(())
        })
        .await?;
        services::update_saved(gate, |saved| {
            saved.verify_delivery(authorization, applicant)?;
            let index = saved
                .approval_index(applicant, request)
                .ok_or_else(rejected)?;
            saved.approvals[index].complete = true;
            saved.approvals[index].delivery_phase = Some(3);
            Ok(())
        })
        .await?;
        until(deadline, send_fixed(&exchange, b"DONE")).await?;
        "Complete"
    } else if approval.rejected {
        until(deadline, send_fixed(&exchange, &[2])).await?;
        "Rejected"
    } else {
        until(deadline, send_fixed(&exchange, &[0])).await?;
        "AwaitingApproval"
    };
    until(deadline, async {
        exchange.finish_send().await.map_err(error)?;
        connection.revocation_handle().closed().await;
        Ok(result)
    })
    .await
}
