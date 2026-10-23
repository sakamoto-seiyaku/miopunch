//! IndexedDB and background control adapter for the shared signed service
//! ledger. A successful control reply follows the real snapshot transaction.
use super::*;
use crate::service_policy::{
    GuestInvitation, Service, ServiceAccess, ServiceAction, ServiceLedger, ServiceRule,
};
use crate::service_protocol::{GuestRequest, ServiceBundle, read_guest, write_guest};
use futures_util::FutureExt;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use tokio::sync::Notify;

impl SavedNode {
    fn service_bundle(&self) -> ServiceBundle {
        ServiceBundle {
            management: self.records.clone(),
            services: self.services.clone(),
        }
    }
    pub(super) fn service_access(&self) -> Result<ServiceAccess, JsValue> {
        let anchor = self.invitation()?.anchor;
        let auth = self
            .verify()?
            .unwrap_or_else(|| ManagementAuth::new(anchor));
        let ledger = ServiceLedger::replay(anchor, &auth, &self.services).map_err(error)?;
        Ok(ServiceAccess {
            local: self.identity(),
            auth,
            ledger,
        })
    }
    pub(super) fn guest_pair(&self, peer: iroh_base::EndpointId) -> bool {
        self.service_access().ok().is_some_and(|access| {
            access
                .guest_pairs()
                .iter()
                .any(|(id, _)| id == peer.as_bytes())
        })
    }
    pub(super) fn guest_stage(&self) -> &'static str {
        let stage = self.guest.as_ref().and_then(|invitation| {
            self.service_access()
                .ok()
                .and_then(|access| access.guest_status(invitation).ok())
        });
        match stage.as_ref().and_then(|value| value["stage"].as_str()) {
            Some("GuestActive") => "GuestActive",
            Some("GuestEnded") => "GuestEnded",
            Some("GuestExpired") => "GuestExpired",
            _ => "GuestPending",
        }
    }
    fn with_service_record(&self, bytes: Vec<u8>) -> Result<Self, JsValue> {
        let access = self.service_access()?;
        let ledger = access
            .ledger
            .merge(self.invitation()?.anchor, &access.auth, &[bytes])
            .map_err(error)?;
        let mut next = self.clone();
        next.services = ledger.records().to_vec();
        next.verify()?;
        Ok(next)
    }
    fn with_service_action(&self, action: ServiceAction) -> Result<(Self, Vec<u8>), JsValue> {
        let access = self.service_access()?;
        let record = access
            .ledger
            .author(
                self.invitation()?.anchor,
                &access.auth,
                &p2panda_core::SigningKey::from_bytes(&self.key),
                action,
            )
            .map_err(error)?;
        Ok((self.with_service_record(record.clone())?, record))
    }
    fn with_service_bundle(
        &self,
        incoming: &ServiceBundle,
    ) -> Result<(Self, Option<std::io::Error>), JsValue> {
        let result = self
            .service_bundle()
            .reconcile(self.invitation()?.anchor, incoming)
            .map_err(error)?;
        let mut next = self.clone();
        next.records = result.bundle.management;
        next.services = result.bundle.services;
        next.verify()?;
        Ok((next, result.policy_error))
    }
    pub(super) fn declare_files(&self, enabled: bool) -> Result<Self, JsValue> {
        if self.guest.is_some() || self.invitation.is_none() {
            return Ok(self.clone());
        }
        let access = self.service_access()?;
        if !access.member(access.local) {
            return Ok(self.clone());
        }
        let previous = access.ledger.declaration(access.local, Service::Files);
        let capable = files::capable();
        if enabled && !capable {
            return Err(error("file storage unavailable in this browser"));
        }
        if previous
            .as_ref()
            .is_some_and(|d| d.capable == capable && d.enabled == enabled)
        {
            return Ok(self.clone());
        }
        self.with_service_action(ServiceAction::Declare {
            provider: access.local,
            service: Service::Files,
            previous: previous.map(|d| d.revision),
            capable,
            enabled,
        })
        .map(|(next, _)| next)
    }
}

impl BrowserGate {
    pub(super) fn service_snapshot(&self) -> Result<SavedNode, JsValue> {
        self.0.lock().unwrap().saved.clone().ok_or_else(rejected)
    }
}

async fn installed(gate: &BrowserGate) -> Result<(SavedNode, String, u32), JsValue> {
    let (saved, token) = {
        let state = gate.0.lock().unwrap();
        (
            state.saved.clone().ok_or_else(rejected)?,
            state.lease.ok_or_else(rejected)?,
        )
    };
    saved.verify()?;
    let raw = serde_json::to_string(&saved).map_err(error)?;
    if read_node(token).await?.as_string().as_deref() != Some(&raw) {
        return Err(error("browser service state changed or storage cleared"));
    }
    Ok((saved, raw, token))
}
async fn commit_installed(
    gate: &BrowserGate,
    previous: &str,
    token: u32,
    next: &SavedNode,
) -> Result<(), JsValue> {
    next.verify()?;
    let raw = serde_json::to_string(next).map_err(error)?;
    if raw.len() > 80 * 1024 * 1024 {
        return Err(error("browser state too large"));
    }
    save_node(token, JsValue::from_str(previous), raw).await?;
    gate.install(next);
    Ok(())
}

pub(super) struct BrowserServices {
    cancel: tokio_util::sync::CancellationToken,
    connections: RefCell<Vec<iroh::endpoint::WeakControlConnection>>,
    running: Cell<usize>,
    joined: Notify,
}
pub(super) struct ServiceOwner(pub Rc<BrowserServices>);
impl Drop for ServiceOwner {
    fn drop(&mut self) {
        self.0.stop();
    }
}
struct Running(Rc<BrowserServices>);
impl Running {
    fn new(service: &Rc<BrowserServices>) -> Self {
        service.running.set(service.running.get() + 1);
        Self(service.clone())
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        self.0.running.set(self.0.running.get() - 1);
        self.0.joined.notify_waiters();
    }
}
impl BrowserServices {
    pub fn start(
        endpoint: RestrictedEndpoint,
        gate: BrowserGate,
        incoming: Rc<incoming::IncomingDispatcher>,
        files: Rc<files::BrowserFiles>,
        budget: Duration,
    ) -> Rc<Self> {
        let service = Rc::new(Self {
            cancel: Default::default(),
            connections: Default::default(),
            running: Cell::new(0),
            joined: Notify::new(),
        });
        for alpn in [
            crate::service_policy::POLICY_ALPN,
            crate::service_policy::GUEST_ALPN,
        ] {
            let owner = Running::new(&service);
            let endpoint = endpoint.clone();
            let gate = gate.clone();
            let incoming = incoming.clone();
            let files = files.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let service = owner.0.clone();
                let _owner = owner;
                while !service.cancel.is_cancelled() {
                    let Ok((accepting, deadline)) = incoming
                        .receive_service(alpn, budget, &service.cancel)
                        .await
                    else {
                        if service.cancel.is_cancelled() {
                            break;
                        } else {
                            continue;
                        }
                    };
                    if alpn == crate::service_policy::GUEST_ALPN {
                        if let Ok(Ok(control)) = n0_future::time::timeout(
                            deadline.saturating_duration_since(n0_future::time::Instant::now()),
                            accepting.into_control(),
                        )
                        .await
                        {
                            let control = Arc::new(control).retain_control();
                            service
                                .connections
                                .borrow_mut()
                                .push(control.revocation_handle());
                            let _close = RequestConnection(Some(control.clone()));
                            let _ = service.serve_guest(&control, &gate, &files).await;
                        }
                    } else {
                        use crate::connection_coordinator::{AttemptContext, SessionPurpose};
                        let attempt = AttemptContext::new(deadline, service.cancel.clone());
                        let coordinator =
                            crate::connection_coordinator::ConnectionCoordinator::new_restricted(
                                endpoint.clone(),
                                crate::connection_coordinator::MemberPathAdapter::capabilities(
                                    &gate,
                                    crate::connection_coordinator::AttemptRole::Responder,
                                )
                                .permitted,
                                vec![],
                            );
                        if let Ok(connection) = coordinator
                            .accept_accepting(accepting, SessionPurpose::MemberBusiness, attempt)
                            .await
                        {
                            service
                                .connections
                                .borrow_mut()
                                .push(connection.revocation_handle());
                            let _close = RequestConnection(Some(connection.clone()));
                            let _ = service.serve_policy(&connection, &gate, &files).await;
                        }
                    }
                    service
                        .connections
                        .borrow_mut()
                        .retain(|connection| connection.is_open());
                }
            });
        }
        let owner = Running::new(&service);
        wasm_bindgen_futures::spawn_local(async move {
            let service = owner.0.clone();
            let _owner = owner;
            loop {
                let tick = n0_future::time::sleep(Duration::from_secs(1)).fuse();
                let stop = service.cancel.cancelled().fuse();
                futures_util::pin_mut!(tick, stop);
                futures_util::select_biased! { _ = stop => break, _ = tick => {} }
                if let Ok(saved) = gate.service_snapshot() {
                    gate.install(&saved);
                }
                files.enforce_access();
            }
        });
        service
    }
    async fn serve_policy(
        &self,
        connection: &Connection,
        gate: &BrowserGate,
        files: &Rc<files::BrowserFiles>,
    ) -> Result<(), JsValue> {
        gate.qualified_snapshot(connection).await?;
        let mut stream = crate::data_stream::DataStream::accept(connection)
            .await
            .map_err(error)?;
        let incoming = ServiceBundle::read(&mut stream).await.map_err(error)?;
        let (saved, raw, token) = installed(gate).await?;
        let (next, policy_error) = saved.with_service_bundle(&incoming)?;
        if self.cancel.is_cancelled() {
            return Err(rejected());
        }
        commit_installed(gate, &raw, token, &next).await?;
        files.enforce_access();
        if let Some(policy_error) = policy_error {
            return Err(error(policy_error));
        }
        next.service_bundle()
            .write(&mut stream)
            .await
            .map_err(error)?;
        stream.finish().map_err(error)?;
        let _ = n0_future::time::timeout(Duration::from_secs(5), connection.closed()).await;
        Ok(())
    }
    async fn serve_guest(
        &self,
        connection: &crate::control_protocol::ControlOwner,
        gate: &BrowserGate,
        files: &Rc<files::BrowserFiles>,
    ) -> Result<(), JsValue> {
        let exchange = connection.accept_control_exchange().await.map_err(error)?;
        let request: GuestRequest =
            n0_future::time::timeout(Duration::from_secs(60), read_guest(&exchange))
                .await
                .map_err(error)?
                .map_err(error)?;
        let redeem = matches!(&request, GuestRequest::Redeem { .. });
        let (saved, raw, token) = installed(gate).await?;
        let peer = VerifyingKey::from_bytes(connection.remote_id().as_bytes()).map_err(error)?;
        let next = match request {
            GuestRequest::Redeem { invitation, token } => {
                let access = saved.service_access()?;
                let guest = access
                    .ledger
                    .guests()
                    .into_iter()
                    .find(|g| g.invitation == invitation)
                    .ok_or_else(rejected)?;
                if guest.issuer != saved.identity() {
                    return Err(rejected());
                }
                if access
                    .ledger
                    .redeem(
                        invitation,
                        &token,
                        peer,
                        &access.auth,
                        crate::service_policy::now(),
                    )
                    .map_err(error)?
                {
                    saved
                        .with_service_action(ServiceAction::BindGuest {
                            invitation,
                            guest: peer,
                        })?
                        .0
                } else {
                    saved.clone()
                }
            }
            GuestRequest::End { record } => {
                if !ServiceLedger::is_guest_end(&record, peer) {
                    return Err(rejected());
                }
                saved.with_service_record(record)?
            }
        };
        if self.cancel.is_cancelled() {
            return Err(rejected());
        }
        commit_installed(gate, &raw, token, &next).await?;
        files.enforce_access();
        if redeem {
            write_guest(&exchange, &next.service_bundle())
                .await
                .map_err(error)?;
        } else {
            write_guest(&exchange, &serde_json::json!({"saved_locally":true}))
                .await
                .map_err(error)?;
        }
        exchange.finish_send().await.map_err(error)?;
        let _ = n0_future::time::timeout(
            Duration::from_secs(5),
            connection.revocation_handle().closed(),
        )
        .await;
        Ok(())
    }
    fn stop(&self) {
        self.cancel.cancel();
        for connection in self.connections.borrow().iter() {
            connection.close(0u32.into(), b"service host stopped");
        }
    }
    pub async fn close(&self) {
        self.stop();
        while self.running.get() != 0 {
            let done = self.joined.notified();
            if self.running.get() == 0 {
                break;
            }
            done.await;
        }
    }
}

#[wasm_bindgen]
impl BrowserNetwork {
    pub fn service_status(&self) -> Result<String, JsValue> {
        let saved = self.gate.service_snapshot()?;
        let access = saved.service_access()?;
        Ok(access
            .status(
                saved.guest.as_ref(),
                files::capable(),
                self.files.enabled.get(),
            )
            .map_err(error)?
            .to_string())
    }
    pub fn guest_status(&self) -> Result<String, JsValue> {
        let saved = self.gate.service_snapshot()?;
        match &saved.guest {
            Some(invitation) => Ok(saved
                .service_access()?
                .guest_status(invitation)
                .map_err(error)?
                .to_string()),
            None => Ok("null".into()),
        }
    }
    pub async fn set_service_rule(
        &mut self,
        provider: String,
        service: String,
        previous: Option<String>,
        rule: String,
    ) -> Result<String, JsValue> {
        self.refresh_service_snapshot().await?;
        let (next, record) = self.saved.with_service_action(ServiceAction::SetRule {
            provider: provider.parse().map_err(error)?,
            service: serde_json::from_value(serde_json::Value::String(service)).map_err(error)?,
            previous: previous.map(|id| id.parse()).transpose().map_err(error)?,
            rule: serde_json::from_str::<ServiceRule>(&rule).map_err(error)?,
        })?;
        self.commit(next).await?;
        self.files.enforce_access();
        Ok(Hash::digest(record).to_string())
    }
    pub async fn sync_services(&mut self, target: String) -> Result<String, JsValue> {
        self.refresh_service_snapshot().await?;
        let connection = self
            .connect_target(
                target.parse().map_err(error)?,
                crate::service_policy::POLICY_ALPN,
            )
            .await?;
        let _close = RequestConnection(Some(connection.clone()));
        let mut stream = crate::data_stream::DataStream::open(&connection)
            .await
            .map_err(error)?;
        self.saved
            .service_bundle()
            .write(&mut stream)
            .await
            .map_err(error)?;
        let incoming = ServiceBundle::read(&mut stream).await.map_err(error)?;
        self.refresh_service_snapshot().await?;
        let (next, policy_error) = self.saved.with_service_bundle(&incoming)?;
        self.commit(next).await?;
        self.files.enforce_access();
        if let Some(policy_error) = policy_error {
            return Err(error(policy_error));
        }
        Ok("{\"saved_locally\":true}".into())
    }
    pub async fn issue_guest(
        &mut self,
        provider_address: String,
        expires_unix_seconds: u64,
    ) -> Result<String, JsValue> {
        self.refresh_service_snapshot().await?;
        let created = crate::service_policy::now();
        if expires_unix_seconds <= created {
            return Err(error("guest deadline must be in the future"));
        }
        let provider: EndpointAddr = serde_json::from_str(&provider_address).map_err(error)?;
        let provider_id = VerifyingKey::from_bytes(provider.id.as_bytes()).map_err(error)?;
        let access = self.saved.service_access()?;
        if !access.ledger.rule(provider_id, Service::Files).rule.guests {
            return Err(rejected());
        }
        let token: [u8; 32] = rand::random();
        let (next, record) = self.saved.with_service_action(ServiceAction::InviteGuest {
            provider: provider_id,
            service: Service::Files,
            token_hash: Hash::digest(token),
            created,
            expires: expires_unix_seconds,
        })?;
        let (issuer, bootstrap) = self.contact();
        let invitation = GuestInvitation {
            anchor: self.saved.invitation()?.anchor,
            issuer,
            provider,
            bootstrap,
            record,
            token,
        };
        let text = invitation.text().map_err(error)?;
        self.commit(next).await?;
        Ok(text)
    }
    pub async fn redeem_guest(&mut self) -> Result<String, JsValue> {
        self.refresh_service_snapshot().await?;
        let invitation = self.saved.guest.clone().ok_or_else(rejected)?;
        let connection = self.connect_guest(invitation.issuer.id).await?;
        let _close = RequestConnection(Some(connection.clone()));
        let exchange = connection.open_control_exchange().await.map_err(error)?;
        write_guest(
            &exchange,
            &GuestRequest::Redeem {
                invitation: invitation.id(),
                token: invitation.token,
            },
        )
        .await
        .map_err(error)?;
        exchange.finish_send().await.map_err(error)?;
        let incoming: ServiceBundle = read_guest(&exchange).await.map_err(error)?;
        let (next, policy_error) = self.saved.with_service_bundle(&incoming)?;
        self.commit(next).await?;
        self.files.enforce_access();
        if let Some(policy_error) = policy_error {
            return Err(error(policy_error));
        }
        let access = self.saved.service_access()?;
        if !access.ledger.guest_access(
            VerifyingKey::from_bytes(invitation.provider.id.as_bytes()).map_err(error)?,
            Service::Files,
            access.local,
            &access.auth,
            crate::service_policy::now(),
        ) {
            return Err(rejected());
        }
        self.guest_status()
    }
    pub async fn revoke_guest(&mut self, invitation: String) -> Result<String, JsValue> {
        self.refresh_service_snapshot().await?;
        let (next, record) = self.saved.with_service_action(ServiceAction::EndGuest {
            invitation: invitation.parse().map_err(error)?,
        })?;
        self.commit(next).await?;
        self.files.enforce_access();
        Ok(Hash::digest(record).to_string())
    }
    pub async fn end_guest(&mut self) -> Result<String, JsValue> {
        self.refresh_service_snapshot().await?;
        let invitation = self.saved.guest.clone().ok_or_else(rejected)?;
        let ended = self
            .saved
            .service_access()?
            .ledger
            .guests()
            .into_iter()
            .find(|g| g.invitation == invitation.id())
            .is_some_and(|g| g.ended);
        if !ended {
            let next = self
                .saved
                .with_service_action(ServiceAction::EndGuest {
                    invitation: invitation.id(),
                })?
                .0;
            self.commit(next).await?;
            self.files.enforce_access();
        }
        let record = self
            .saved
            .service_access()?
            .ledger
            .guest_end_record(invitation.id())
            .ok_or_else(rejected)?;
        let mut targets = vec![invitation.issuer.id];
        if invitation.provider.id != invitation.issuer.id {
            targets.push(invitation.provider.id);
        }
        let mut results = Vec::new();
        for target in targets {
            let result = async {
                let connection = self.connect_guest(target).await?;
                let _close = RequestConnection(Some(connection.clone()));
                let exchange = connection.open_control_exchange().await.map_err(error)?;
                write_guest(
                    &exchange,
                    &GuestRequest::End {
                        record: record.clone(),
                    },
                )
                .await
                .map_err(error)?;
                exchange.finish_send().await.map_err(error)?;
                let reply: serde_json::Value = read_guest(&exchange).await.map_err(error)?;
                if reply["saved_locally"] != true {
                    return Err(error("guest exit was not acknowledged as saved"));
                }
                Ok::<_, JsValue>(())
            }
            .await;
            results.push(serde_json::json!({"node":target.to_string(),"saved":result.is_ok()}));
        }
        Ok(serde_json::json!({"ended_locally":true,"delivery":results}).to_string())
    }
    pub async fn refresh_service_state(&mut self) -> Result<(), JsValue> {
        self.refresh_service_snapshot().await
    }
}
impl BrowserNetwork {
    pub(super) async fn refresh_service_snapshot(&mut self) -> Result<(), JsValue> {
        if self.lease.is_none() {
            return Err(rejected());
        }
        let (saved, raw, _) = installed(&self.gate).await?;
        self.saved = saved;
        self.committed = raw;
        Ok(())
    }
    async fn connect_guest(
        &self,
        target: iroh_base::EndpointId,
    ) -> Result<crate::control_protocol::ControlOwner, JsValue> {
        self.check_store().await?;
        let address = self.target_address(target)?;
        let deadline = n0_future::time::Instant::now() + self.caller_budget;
        let connection = self
            .endpoint
            .connect_control_with_opts(
                address,
                crate::service_policy::GUEST_ALPN,
                iroh::endpoint::ConnectOptions::new().with_caller_deadline(deadline),
            )
            .await
            .map_err(error)?
            .await
            .map_err(error)?;
        Ok(Arc::new(connection).retain_control())
    }
}
