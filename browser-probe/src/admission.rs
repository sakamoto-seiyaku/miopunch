//! Browser administrator adapter. Approval, signed grant and precode count are
//! committed together by the same strict IndexedDB snapshot transaction.
use super::*;
use crate::management_action::ManagementAction;
use p2panda_core::SigningKey;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Code {
    token: [u8; 32],
    origin: Hash,
    expires: u64,
    limit: Option<u64>,
    used: u64,
    revoked: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Approval {
    applicant: VerifyingKey,
    request: [u8; 16],
    authorization: Option<Hash>,
    code: Option<[u8; 32]>,
    complete: bool,
}
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
            {
                return Err(rejected());
            }
        }
        Ok(())
    }
}

#[wasm_bindgen]
impl BrowserNetwork {
    pub fn manual_invitation(&self) -> Result<String, JsValue> {
        self.administrator()?;
        Ok(hex::encode(self.own_invitation()?))
    }
    pub fn admission_requests(&self) -> String {
        serde_json::json!(self.saved.approvals.iter().map(|a|serde_json::json!({"applicant":a.applicant.to_string(),"request":hex::encode(a.request),"approved":a.authorization.is_some(),"delivery_confirmed":a.complete})).collect::<Vec<_>>()).to_string()
    }
    pub async fn approve_admission(
        &mut self,
        applicant: String,
        request: String,
    ) -> Result<String, JsValue> {
        self.check_store().await?;
        let applicant = applicant.parse().map_err(error)?;
        let request: [u8; 16] = hex::decode(request)
            .map_err(error)?
            .try_into()
            .map_err(|_| rejected())?;
        let index = self
            .saved
            .approvals
            .iter()
            .position(|a| a.applicant == applicant && a.request == request)
            .ok_or_else(rejected)?;
        let next = self.approve_snapshot(self.saved.clone(), index)?;
        let id = next.approvals[index].authorization.ok_or_else(rejected)?;
        self.commit(next).await?;
        Ok(id.to_string())
    }
    /// Return bearer material only to the caller for trusted out-of-band delivery.
    pub async fn issue_precode(
        &mut self,
        expires_unix_seconds: u64,
        max_uses: Option<u64>,
    ) -> Result<String, JsValue> {
        self.check_store().await?;
        let auth = self.administrator()?;
        if !crate::precode_policy::valid_limits(expires_unix_seconds, max_uses, now()?)
            || self.saved.codes.len() >= 4096
        {
            return Err(rejected());
        }
        let origin = auth
            .administrator_authorization_origin(self.saved.identity())
            .ok_or_else(rejected)?;
        let token: [u8; 32] = rand::random();
        let mut code = self.own_invitation()?;
        code.extend_from_slice(&token);
        let mut next = self.saved.clone();
        next.codes.push(Code {
            token,
            origin,
            expires: expires_unix_seconds,
            limit: max_uses,
            used: 0,
            revoked: false,
        });
        self.commit(next).await?;
        Ok(hex::encode(code))
    }
    pub async fn revoke_precode(&mut self, code: String) -> Result<(), JsValue> {
        let token = self.own_code(&code)?;
        let mut next = self.saved.clone();
        next.codes
            .iter_mut()
            .find(|code| code.token == token)
            .ok_or_else(rejected)?
            .revoked = true;
        self.commit(next).await
    }
    pub fn precode_usage(&self, code: String) -> Result<u64, JsValue> {
        let token = self.own_code(&code)?;
        self.saved
            .codes
            .iter()
            .find(|code| code.token == token)
            .map(|code| code.used)
            .ok_or_else(rejected)
    }
    /// Process one restricted request. The foreground owner chooses the native
    /// manual or precode protocol mode, then drives the next accept explicitly.
    pub async fn serve_admission(&mut self, precode: bool) -> Result<String, JsValue> {
        let deadline = n0_future::time::Instant::now() + self.caller_budget;
        let attempt = crate::connection_coordinator::AttemptContext::new(
            deadline,
            tokio_util::sync::CancellationToken::new(),
        );
        self.check_store().await?;
        self.administrator()?;
        let accepting = self.incoming.receive(REQUEST_ALPN, &attempt).await?;
        let connection = n0_future::time::timeout(
            deadline.saturating_duration_since(n0_future::time::Instant::now()),
            accepting
                .with_caller_deadline(deadline)
                .with_attempt_cancellation(attempt.cancellation.clone()),
        )
        .await
        .map_err(error)?
        .map_err(error)?;
        let _scope = RequestConnection(Some(connection.clone()));
        let result = n0_future::time::timeout(
            deadline.saturating_duration_since(n0_future::time::Instant::now()),
            self.serve_admission_inner(&connection, precode),
        )
        .await
        .unwrap_or_else(|_| Err(error("admission delivery timed out")));
        connection.close(0u32.into(), b"browser admission ended");
        result
    }
}
impl BrowserNetwork {
    fn administrator(&self) -> Result<ManagementAuth, JsValue> {
        let auth = self.saved.verify()?.ok_or_else(rejected)?;
        if self.saved.stage() != "Complete"
            || !auth
                .nodes()
                .contains(&(self.saved.identity(), Access::manage()))
        {
            return Err(rejected());
        }
        Ok(auth)
    }
    fn own_invitation(&self) -> Result<Vec<u8>, JsValue> {
        let anchor = self.saved.invitation()?.anchor;
        let mut bytes = anchor.root.as_bytes().to_vec();
        bytes.extend_from_slice(anchor.genesis.as_bytes());
        bytes.extend_from_slice(self.endpoint.id().as_bytes());
        Ok(bytes)
    }
    fn own_code(&self, code: &str) -> Result<[u8; 32], JsValue> {
        let bytes = hex::decode(code).map_err(error)?;
        if bytes.len() != 128 || bytes[..96] != self.own_invitation()? {
            return Err(rejected());
        }
        Ok(bytes[96..].try_into().unwrap())
    }
    fn approve_snapshot(&self, mut next: SavedNode, index: usize) -> Result<SavedNode, JsValue> {
        let mut auth = self.administrator()?;
        let applicant = next.approvals[index].applicant;
        if applicant == next.invitation()?.anchor.root {
            return Err(rejected());
        }
        if let Some(id) = next.approvals[index].authorization {
            if !auth.membership_authorization_valid(id, applicant) {
                return Err(rejected());
            }
            return Ok(next);
        }
        let existing = next
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
                &SigningKey::from_bytes(&next.key),
                ManagementAction::AddMember(applicant),
                None,
            )?;
            let id = auth.accept(&header).map_err(error)?;
            next.records.push((header, None));
            id
        };
        next.approvals[index].authorization = Some(id);
        Ok(next)
    }
    async fn serve_admission_inner(
        &mut self,
        connection: &Connection,
        precode: bool,
    ) -> Result<String, JsValue> {
        self.check_store().await?;
        self.administrator()?;
        if connection.alpn() != REQUEST_ALPN {
            return Err(rejected());
        }
        let (mut send, mut recv) = connection.accept_bi().await.map_err(error)?;
        let mut wire = [0; 144];
        recv.read_exact(&mut wire).await.map_err(error)?;
        if wire[..96] != self.own_invitation()?
            || wire[112..] != connection.remote_id().as_bytes()[..]
        {
            return Err(rejected());
        }
        let applicant =
            VerifyingKey::from_bytes(&wire[112..].try_into().unwrap()).map_err(error)?;
        let request = wire[96..112].try_into().unwrap();
        let token = if precode {
            let mut token = [0; 32];
            recv.read_exact(&mut token).await.map_err(error)?;
            Some(token)
        } else {
            None
        };
        let mut next = self.saved.clone();
        let existing = next
            .approvals
            .iter()
            .position(|a| a.applicant == applicant && a.request == request);
        let index = if let Some(index) = existing {
            index
        } else {
            if next.approvals.len() >= 4096 {
                return Err(rejected());
            }
            next.approvals.push(Approval {
                applicant,
                request,
                authorization: None,
                code: token,
                complete: false,
            });
            next.approvals.len() - 1
        };
        if next.approvals[index].code != token {
            return Err(rejected());
        }
        if let Some(token) = token {
            if existing.is_none() {
                let auth = self.administrator()?;
                let code = next
                    .codes
                    .iter_mut()
                    .find(|code| code.token == token)
                    .ok_or_else(rejected)?;
                let actual = self
                    .saved
                    .approvals
                    .iter()
                    .filter(|a| a.code == Some(token) && a.authorization.is_some())
                    .count() as u64;
                let policy = crate::precode_policy::RedemptionPolicy {
                    expires: code.expires,
                    max_uses: code.limit,
                    used: code.used,
                    stopped: false,
                    revoked: code.revoked,
                    ledger_consistent: code.used == actual,
                    issuer_authorized: auth
                        .administrator_authorization_valid(code.origin, self.saved.identity()),
                };
                if !policy.permits_new(now()?) {
                    return Err(rejected());
                }
                code.used = code.used.checked_add(1).ok_or_else(rejected)?;
            }
            next = self.approve_snapshot(next, index)?;
        }
        self.commit(next).await?;
        let Some(authorization) = self.saved.approvals[index].authorization else {
            send.write_all(&[0]).await.map_err(error)?;
            send.finish().map_err(error)?;
            connection.closed().await;
            return Ok("AwaitingApproval".into());
        };
        if !self
            .administrator()?
            .membership_authorization_valid(authorization, applicant)
        {
            return Err(rejected());
        }
        let history = p2panda_core::cbor::encode_cbor(&(authorization, &self.saved.records))
            .map_err(error)?;
        if history.len() > MAX_HISTORY {
            return Err(rejected());
        }
        send.write_all(&[1]).await.map_err(error)?;
        send.write_all(&self.saved.generation.to_be_bytes())
            .await
            .map_err(error)?;
        send.write_all(&(history.len() as u32).to_be_bytes())
            .await
            .map_err(error)?;
        send.write_all(&history).await.map_err(error)?;
        let mut receipt = [0; 4];
        recv.read_exact(&mut receipt).await.map_err(error)?;
        if &receipt != b"AUTH" {
            return Err(rejected());
        }
        self.check_store().await?;
        if !self
            .administrator()?
            .membership_authorization_valid(authorization, applicant)
        {
            return Err(rejected());
        }
        send.write_all(&self.saved.generation.to_be_bytes())
            .await
            .map_err(error)?;
        send.write_all(&self.saved.secret.ok_or_else(rejected)?)
            .await
            .map_err(error)?;
        recv.read_exact(&mut receipt).await.map_err(error)?;
        if &receipt != b"SAVE" {
            return Err(rejected());
        }
        let mut next = self.saved.clone();
        next.approvals[index].complete = true;
        self.commit(next).await?;
        send.write_all(b"DONE").await.map_err(error)?;
        send.finish().map_err(error)?;
        connection.closed().await;
        Ok("Complete".into())
    }
}
