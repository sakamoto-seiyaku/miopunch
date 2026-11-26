//! Browser action-specific decisions use the shared request/signature journal
//! and the same real member/control paths as the Native adapter.
use super::*;
use crate::{
    data_stream::DataStream,
    device_operation::{
        DEVICE_ALPN, DEVICE_RESULT_ALPN, DeviceAction, DeviceApproval, DeviceOperation,
    },
    device_protocol::{
        DeviceReply, DeviceRequest, ResultReply, ResultRequest, read_message, write_exchange,
    },
    recovery_card::RecoveryCard,
    service_protocol::{ServiceBundle, read_guest, write_guest},
};

fn view(
    entry: &crate::device_operation::OperationEntry,
    auth: &ManagementAuth,
) -> serde_json::Value {
    serde_json::json!({"operation":entry.operation,"impact":entry.operation.impact(),
        "requires_genesis":entry.operation.requires_genesis(),
        "stage":if entry.approval.is_some() {"confirmed"} else {"awaiting_confirmation"},
        "record":entry.approval.as_ref().map(DeviceApproval::record_id),
        "effective_locally":entry.approval.as_ref().is_some_and(|a| a.effect_current(auth)),
        "scope":"local verified state; other nodes apply after receipt"})
}

impl SavedNode {
    fn approved_operation(
        &self,
        approval: DeviceApproval,
        receipt: Records,
    ) -> Result<Self, JsValue> {
        let mut next = self.clone();
        next.operations
            .approve(approval.clone(), receipt.clone())
            .map_err(error)?;
        let (next, policy_error) = next.with_service_bundle(&ServiceBundle {
            management: receipt,
            services: Vec::new(),
        })?;
        if let Some(policy_error) = policy_error {
            return Err(error(policy_error));
        }
        let auth = next.verify()?.ok_or_else(rejected)?;
        if !auth.contains_operation(approval.record_id()) {
            return Err(rejected());
        }
        Ok(next)
    }
    fn approval_history(&self, approval: &DeviceApproval) -> Records {
        let mut records = self.records.clone();
        if !records
            .iter()
            .any(|(h, _)| Hash::digest(h) == approval.record_id())
        {
            records.push((approval.record.clone(), None));
        }
        records
    }
}

impl BrowserNetwork {
    pub(super) fn require_device_ready(&self) -> Result<(), JsValue> {
        let saved = self.gate.service_snapshot()?;
        if self.lease.is_none()
            || saved.invitation.is_none()
            || matches!(
                saved.stage(),
                "AwaitingRecoverySave"
                    | "ReplacementPaused"
                    | "RestoringAuthorization"
                    | "AwaitingManagementRecovery"
            )
        {
            return Err(error(
                "completed local setup required for this device action",
            ));
        }
        Ok(())
    }
    pub(super) async fn connect_device_control(
        &self,
        target: iroh_base::EndpointId,
        alpn: &[u8],
    ) -> Result<ControlOwner, JsValue> {
        self.check_store().await?;
        let address = self.target_address(target)?;
        let deadline = n0_future::time::Instant::now() + self.caller_budget;
        let control = self
            .endpoint
            .connect_control_with_opts(
                address,
                alpn,
                iroh::endpoint::ConnectOptions::new().with_caller_deadline(deadline),
            )
            .await
            .map_err(error)?
            .await
            .map_err(error)?;
        Ok(Arc::new(control).retain_control())
    }
    async fn confirm_operation(
        &mut self,
        id: Hash,
        card: Option<&RecoveryCard>,
    ) -> Result<String, JsValue> {
        self.require_device_ready()?;
        let result = services::update_saved(&self.gate, |saved| {
            let auth = saved.verify()?.ok_or_else(rejected)?;
            let entry = saved.operations.get(id).map_err(error)?;
            let approval = if let Some(approval) = entry.confirmation().map_err(error)? {
                approval.clone()
            } else {
                let seed = if let Some(card) = card {
                    if card.anchor() != saved.invitation()?.anchor {
                        return Err(error("card belongs to another network"));
                    }
                    card.credential().0
                } else {
                    saved
                        .creator_seed
                        .ok_or_else(|| error("this node does not hold the genesis credential"))?
                };
                entry
                    .operation
                    .approve(
                        saved.invitation()?.anchor,
                        &auth,
                        &SigningKey::from_bytes(&seed),
                    )
                    .map_err(error)?
            };
            *saved =
                saved.approved_operation(approval.clone(), saved.approval_history(&approval))?;
            saved.operation_view(id)
        })
        .await?;
        self.files.enforce_access();
        Ok(result)
    }
}

#[wasm_bindgen]
impl BrowserNetwork {
    pub async fn propose_device_operation(
        &mut self,
        action: String,
        target: String,
    ) -> Result<String, JsValue> {
        self.require_device_ready()?;
        let action: DeviceAction =
            serde_json::from_value(serde_json::Value::String(action)).map_err(error)?;
        let target = target.parse().map_err(error)?;
        services::update_saved(&self.gate, |saved| {
            let auth = saved.verify()?.ok_or_else(rejected)?;
            let operation = DeviceOperation::propose(
                saved.invitation()?.anchor,
                &auth,
                saved.identity(),
                action,
                target,
            )
            .map_err(error)?;
            let id = operation.id;
            saved.operations.retain(operation).map_err(error)?;
            Ok(view(saved.operations.get(id).map_err(error)?, &auth).to_string())
        })
        .await
    }
    pub fn device_operations(&self) -> Result<String, JsValue> {
        if self.lease.is_none() {
            return Err(error("this node host is stopped"));
        }
        let saved = self.gate.service_snapshot()?;
        let auth = saved.verify()?.ok_or_else(rejected)?;
        Ok(saved.operation_state(&auth).to_string())
    }
    pub async fn confirm_device_operation(&mut self, operation: String) -> Result<String, JsValue> {
        self.confirm_operation(operation.parse().map_err(error)?, None)
            .await
    }
    pub async fn confirm_device_operation_offline(
        &mut self,
        operation: String,
        card: String,
        password: Option<String>,
    ) -> Result<String, JsValue> {
        let card = RecoveryCard::import(&card, password.as_deref()).map_err(error)?;
        self.confirm_operation(operation.parse().map_err(error)?, Some(&card))
            .await
    }
    pub async fn execute_device_operation(&mut self, operation: String) -> Result<String, JsValue> {
        self.require_device_ready()?;
        let id = operation.parse().map_err(error)?;
        let result = services::update_saved(&self.gate, |saved| {
            let auth = saved.verify()?.ok_or_else(rejected)?;
            let entry = saved.operations.get(id).map_err(error)?;
            if entry.operation.description.initiator != saved.identity()
                || entry.operation.requires_genesis()
            {
                return Err(error("this action requires its own genesis confirmation"));
            }
            let approval = match &entry.approval {
                Some(approval) => approval.clone(),
                None => entry
                    .operation
                    .approve(
                        saved.invitation()?.anchor,
                        &auth,
                        &SigningKey::from_bytes(&saved.key),
                    )
                    .map_err(error)?,
            };
            *saved =
                saved.approved_operation(approval.clone(), saved.approval_history(&approval))?;
            saved.operation_view(id)
        })
        .await?;
        self.files.enforce_access();
        Ok(result)
    }
    pub async fn request_device_confirmation(
        &mut self,
        operation: String,
        custodian: String,
    ) -> Result<String, JsValue> {
        self.refresh_service_snapshot().await?;
        self.require_device_ready()?;
        let id = operation.parse().map_err(error)?;
        let operation = self
            .saved
            .operations
            .get(id)
            .map_err(error)?
            .operation
            .clone();
        if operation.description.initiator != self.saved.identity() || !operation.requires_genesis()
        {
            return Err(rejected());
        }
        let connection = self
            .connect_target(custodian.parse().map_err(error)?, DEVICE_ALPN)
            .await?;
        let _close = RequestConnection(Some(connection.clone()));
        let mut stream = DataStream::open(&connection).await.map_err(error)?;
        write_exchange(
            &mut stream,
            &DeviceRequest::Confirm {
                operation: operation.clone(),
            },
            &self.saved.service_bundle(),
        )
        .await
        .map_err(error)?;
        let reply: DeviceReply = read_message(&mut stream).await.map_err(error)?;
        let history = ServiceBundle::read(&mut stream).await.map_err(error)?;
        let policy_error = services::update_saved(&self.gate, |saved| {
            let (mut next, policy_error) = saved.with_service_bundle(&history)?;
            if let Some(approval) = reply.approval {
                approval
                    .verify(&operation, next.invitation()?.anchor)
                    .map_err(error)?;
                let receipt = next.approval_history(&approval);
                next = next.approved_operation(approval, receipt)?;
            }
            *saved = next;
            Ok(policy_error)
        })
        .await?;
        self.files.enforce_access();
        if let Some(policy_error) = policy_error {
            return Err(error(policy_error));
        }
        stream.finish().map_err(error)?;
        self.gate.service_snapshot()?.operation_view(id)
    }
    pub async fn collect_device_result(
        &mut self,
        operation: String,
        custodian: String,
    ) -> Result<String, JsValue> {
        self.refresh_service_snapshot().await?;
        let id = operation.parse().map_err(error)?;
        let operation = self
            .saved
            .operations
            .get(id)
            .map_err(error)?
            .operation
            .clone();
        if operation.description.initiator != self.saved.identity() {
            return Err(rejected());
        }
        let connection = self
            .connect_device_control(custodian.parse().map_err(error)?, DEVICE_RESULT_ALPN)
            .await?;
        let _close = RequestConnection(Some(connection.clone()));
        let exchange = connection.open_control_exchange().await.map_err(error)?;
        write_guest(&exchange, &ResultRequest { operation: id })
            .await
            .map_err(error)?;
        let reply: ResultReply = read_guest(&exchange).await.map_err(error)?;
        if let Some(approval) = reply.approval {
            services::update_saved(&self.gate, |saved| {
                approval
                    .verify(&operation, saved.invitation()?.anchor)
                    .map_err(error)?;
                *saved = saved.approved_operation(approval, reply.history.management)?;
                Ok(())
            })
            .await?;
            self.files.enforce_access();
        } else if !reply.history.management.is_empty() || !reply.history.services.is_empty() {
            return Err(rejected());
        }
        self.gate.service_snapshot()?.operation_view(id)
    }
}

impl SavedNode {
    fn operation_view(&self, id: Hash) -> Result<String, JsValue> {
        let auth = self.verify()?.ok_or_else(rejected)?;
        Ok(view(self.operations.get(id).map_err(error)?, &auth).to_string())
    }

    pub(super) fn operation_state(&self, auth: &ManagementAuth) -> serde_json::Value {
        serde_json::json!({"operations":self.operations.entries.iter().map(|e| view(e,auth)).collect::<Vec<_>>(),
            "holds_genesis":self.creator_seed.is_some()})
    }
}

pub(super) async fn serve_device(
    connection: &Connection,
    gate: &BrowserGate,
    files: &std::rc::Rc<files::BrowserFiles>,
) -> Result<(), JsValue> {
    gate.qualified_snapshot(connection).await?;
    let peer = VerifyingKey::from_bytes(connection.remote_id().as_bytes()).map_err(error)?;
    let mut stream = DataStream::accept(connection).await.map_err(error)?;
    let request: DeviceRequest = read_message(&mut stream).await.map_err(error)?;
    let history = ServiceBundle::read(&mut stream).await.map_err(error)?;
    let policy_error = services::update_saved(gate, |saved| {
        let (next, policy_error) = saved.with_service_bundle(&history)?;
        *saved = next;
        Ok(policy_error)
    })
    .await?;
    files.enforce_access();
    if let Some(policy_error) = policy_error {
        return Err(error(policy_error));
    }
    let (reply, history) = services::update_saved(gate, |next| {
        let DeviceRequest::Confirm { operation } = request;
        let auth = next.verify()?.ok_or_else(rejected)?;
        if operation.description.initiator != peer
            || !operation.requires_genesis()
            || !auth.nodes().iter().any(|(id, _)| *id == peer)
        {
            return Err(rejected());
        }
        match next.operations.get(operation.id) {
            Ok(entry) if entry.operation != operation => return Err(rejected()),
            Ok(_) => {}
            Err(_) => {
                operation.check_current(next.invitation()?.anchor, &auth).map_err(error)?;
                if next.creator_seed.is_none() {
                    return Err(error("this node does not hold the genesis credential"));
                }
                next.operations.retain(operation.clone()).map_err(error)?;
            }
        }
        let approval = next
            .operations
            .get(operation.id)
            .map_err(error)?
            .approval
            .clone();
        if let Some(approval) = &approval {
            let receipt = next.approval_history(approval);
            *next = next.approved_operation(approval.clone(), receipt)?;
        }
        Ok((DeviceReply { approval }, next.service_bundle()))
    })
    .await?;
    files.enforce_access();
    write_exchange(&mut stream, &reply, &history)
        .await
        .map_err(error)?;
    stream.finish().map_err(error)?;
    let _ = n0_future::time::timeout(Duration::from_secs(5), connection.closed()).await;
    Ok(())
}

pub(super) async fn serve_result(
    connection: &ControlOwner,
    gate: &BrowserGate,
) -> Result<(), JsValue> {
    let peer = VerifyingKey::from_bytes(connection.remote_id().as_bytes()).map_err(error)?;
    let exchange = connection.accept_control_exchange().await.map_err(error)?;
    let request: ResultRequest = read_guest(&exchange).await.map_err(error)?;
    let (saved, _, _) = services::installed(gate).await?;
    let entry = saved.operations.get(request.operation).map_err(error)?;
    if entry.operation.description.initiator != peer {
        return Err(rejected());
    }
    let reply = ResultReply {
        approval: entry.approval.clone(),
        history: ServiceBundle {
            management: entry.receipt_history.clone(),
            services: Vec::new(),
        },
    };
    write_guest(&exchange, &reply).await.map_err(error)?;
    exchange.finish_send().await.map_err(error)?;
    let _ = n0_future::time::timeout(
        Duration::from_secs(5),
        connection.revocation_handle().closed(),
    )
    .await;
    Ok(())
}
