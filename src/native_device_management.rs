//! Public Native device operations. Credentials stay at the confirming host;
//! pending descriptions and original signed results survive retries/reopen.
use std::{path::Path, sync::Arc, time::Duration};

use iroh::{EndpointAddr, endpoint::Connection};
use p2panda_core::{Hash, SigningKey, VerifyingKey};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::{
    control_protocol::AuthenticatedControl,
    data_stream::DataStream,
    device_operation::{
        DEVICE_ALPN, DEVICE_RESULT_ALPN, DeviceAction, DeviceApproval, DeviceOperation,
        OperationJournal,
    },
    device_protocol::{
        DeviceReply, DeviceRequest, ResultReply, ResultRequest, read_message, write_exchange,
    },
    native_client::{ClientError, NativeClient},
    native_file_transfer::NativeFiles,
    native_network::NativeNetwork,
    native_services::NativeServices,
    recovery_card::RecoveryCard,
    service_protocol::ServiceBundle,
};

const JOURNAL: &str = "device.operations";

async fn receipt_history(
    node: &NativeNetwork,
    approval: &DeviceApproval,
) -> Result<crate::service_protocol::ManagementRecords, ClientError> {
    let mut records = node
        .store
        .applied_records()
        .await?
        .iter()
        .map(|r| (r.header().to_vec(), r.body().map(Vec::from)))
        .collect::<Vec<_>>();
    if !records
        .iter()
        .any(|(header, _)| Hash::digest(header) == approval.record_id())
    {
        records.push((approval.record.clone(), None));
    }
    Ok(records)
}

fn load(node: &NativeNetwork) -> Result<OperationJournal, ClientError> {
    let journal = match std::fs::read(node.directory.join(JOURNAL)) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => OperationJournal::default(),
        Err(e) => return Err(e.into()),
    };
    journal.verify(node.anchor)?;
    Ok(journal)
}
fn save(node: &NativeNetwork, journal: &OperationJournal) -> Result<(), ClientError> {
    journal.verify(node.anchor)?;
    crate::native_files::write_private_atomic(
        &node.directory.join(JOURNAL),
        &serde_json::to_vec(journal)?,
    )?;
    Ok(())
}
fn view(
    entry: &crate::device_operation::OperationEntry,
    auth: &crate::management_auth::ManagementAuth,
) -> Value {
    json!({"operation": entry.operation, "impact": entry.operation.impact(),
        "requires_genesis": entry.operation.requires_genesis(),
        "stage": if entry.approval.is_some() {"confirmed"} else {"awaiting_confirmation"},
        "record": entry.approval.as_ref().map(DeviceApproval::record_id),
        "effective_locally": entry.approval.as_ref().is_some_and(|a| a.effect_current(auth)),
        "scope":"local verified state; other nodes apply after receipt"})
}
async fn install(
    node: &mut NativeNetwork,
    services: &NativeServices,
    files: &Arc<NativeFiles>,
    approval: &DeviceApproval,
) -> Result<(), ClientError> {
    node.ingest_management_record(&approval.record, None)
        .await?;
    services.publish_access(files, services.access(node).await?);
    Ok(())
}

impl NativeClient {
    /// Propose one exact effect; this neither unlocks a key nor executes it.
    pub async fn propose_device_operation(
        &self,
        action: DeviceAction,
        target: VerifyingKey,
    ) -> Result<Value, ClientError> {
        self.require_ready()?;
        let node = self.node.lock().await;
        let auth = self.services.lock().await.access(&node).await?.auth;
        let operation =
            DeviceOperation::propose(node.anchor, &auth, node.status().node_id, action, target)?;
        let mut journal = load(&node)?;
        journal.retain(operation.clone())?;
        save(&node, &journal)?;
        Ok(view(journal.get(operation.id)?, &auth))
    }

    pub async fn device_operations(&self) -> Result<Value, ClientError> {
        self.require_ready()?;
        let node = self.node.lock().await;
        let auth = self.services.lock().await.access(&node).await?.auth;
        Ok(
            json!({"operations":load(&node)?.entries.iter().map(|e| view(e, &auth)).collect::<Vec<_>>(),
            "holds_genesis":node.directory.join("genesis.credential").exists()}),
        )
    }

    /// Confirm exactly this descriptor with this host's retained credential.
    /// A retry returns and installs the original result, never a new signature.
    pub async fn confirm_device_operation(&self, id: Hash) -> Result<Value, ClientError> {
        self.confirm_device_operation_inner(id, None).await
    }

    /// Explicit offline fallback. The supplied card is used for this operation
    /// and is neither persisted nor converted into a general elevation window.
    pub async fn confirm_device_operation_offline(
        &self,
        id: Hash,
        card: &RecoveryCard,
    ) -> Result<Value, ClientError> {
        self.confirm_device_operation_inner(id, Some(card)).await
    }

    async fn confirm_device_operation_inner(
        &self,
        id: Hash,
        card: Option<&RecoveryCard>,
    ) -> Result<Value, ClientError> {
        self.require_ready()?;
        let mut node = self.node.lock().await;
        let services = self.services.lock().await;
        let access = services.access(&node).await?;
        let mut journal = load(&node)?;
        let entry = journal.get(id)?;
        let approval = if let Some(approval) = entry.confirmation()? {
            approval.clone()
        } else {
            let operation = &entry.operation;
            let root = if let Some(card) = card {
                if card.anchor() != node.anchor {
                    return Err("card belongs to another network".into());
                }
                SigningKey::from_bytes(&card.credential().0)
            } else {
                node.saved_genesis_credential()?.root
            };
            operation.approve(node.anchor, &access.auth, &root)?
        };
        // Save the signature before application. A crash between these writes
        // resumes the same record rather than authoring a second decision.
        journal.approve(approval.clone(), receipt_history(&node, &approval).await?)?;
        save(&node, &journal)?;
        install(&mut node, &services, &self.files, &approval).await?;
        Ok(view(journal.get(id)?, &services.access(&node).await?.auth))
    }

    /// Daily administrator removal or explicitly confirmed ordinary departure.
    /// It does not read the genesis credential, even on its custodian.
    pub async fn execute_device_operation(&self, id: Hash) -> Result<Value, ClientError> {
        self.require_ready()?;
        let mut node = self.node.lock().await;
        let services = self.services.lock().await;
        let access = services.access(&node).await?;
        let mut journal = load(&node)?;
        let entry = journal.get(id)?;
        if entry.operation.description.initiator != node.status().node_id
            || entry.operation.requires_genesis()
        {
            return Err("this operation requires its own genesis confirmation".into());
        }
        let approval = match &entry.approval {
            Some(approval) => approval.clone(),
            None => entry
                .operation
                .approve(node.anchor, &access.auth, &node.node_key)?,
        };
        journal.approve(approval.clone(), receipt_history(&node, &approval).await?)?;
        save(&node, &journal)?;
        install(&mut node, &services, &self.files, &approval).await?;
        Ok(view(journal.get(id)?, &services.access(&node).await?.auth))
    }

    /// Send/resume this one operation at a known custodian. It returns pending
    /// until that host explicitly confirms; the return contains no root key.
    pub async fn request_device_confirmation(
        &self,
        id: Hash,
        custodian: EndpointAddr,
    ) -> Result<Value, ClientError> {
        self.require_ready()?;
        let (operation, history) = {
            let node = self.node.lock().await;
            let operation = load(&node)?.get(id)?.operation.clone();
            if operation.description.initiator != node.status().node_id
                || !operation.requires_genesis()
            {
                return Err("only the initiating node can request this confirmation".into());
            }
            (operation, self.services.lock().await.snapshot(&node).await?)
        };
        let connection = self.resources.connect(custodian, DEVICE_ALPN).await?;
        let result = tokio::time::timeout(Duration::from_secs(60), async {
            let mut stream = DataStream::open(&connection).await?;
            write_exchange(
                &mut stream,
                &DeviceRequest::Confirm {
                    operation: operation.clone(),
                },
                &history,
            )
            .await?;
            let reply: DeviceReply = read_message(&mut stream).await?;
            let history = ServiceBundle::read(&mut stream).await?;
            let mut node = self.node.lock().await;
            let mut services = self.services.lock().await;
            services.merge(&mut node, &self.files, history).await?;
            let mut journal = load(&node)?;
            if let Some(approval) = reply.approval {
                approval.verify(&operation, node.anchor)?;
                journal.approve(approval.clone(), receipt_history(&node, &approval).await?)?;
                save(&node, &journal)?;
                install(&mut node, &services, &self.files, &approval).await?;
            }
            stream.finish()?;
            Ok::<_, ClientError>(view(journal.get(id)?, &services.access(&node).await?.auth))
        })
        .await;
        connection.close(0u32.into(), b"device confirmation exchange ended");
        result?
    }

    /// Read this initiating node's frozen result, including after its own
    /// membership ended. No history newer than that decision is released.
    pub async fn collect_device_result(
        &self,
        id: Hash,
        custodian: EndpointAddr,
    ) -> Result<Value, ClientError> {
        self.require_ready()?;
        let operation = {
            let node = self.node.lock().await;
            let operation = load(&node)?.get(id)?.operation.clone();
            if operation.description.initiator != node.status().node_id {
                return Err("only the initiator can collect this result".into());
            }
            operation
        };
        let connection = self
            .resources
            .connect_control(custodian, DEVICE_RESULT_ALPN)
            .await?;
        let result = tokio::time::timeout(Duration::from_secs(60), async {
            use crate::control_protocol::AuthenticatedControl;
            let exchange = connection.open_control_exchange().await?;
            crate::service_protocol::write_guest(&exchange, &ResultRequest { operation: id })
                .await?;
            let reply: ResultReply = crate::service_protocol::read_guest(&exchange).await?;
            let mut node = self.node.lock().await;
            let mut services = self.services.lock().await;
            let mut journal = load(&node)?;
            if let Some(approval) = reply.approval {
                approval.verify(&operation, node.anchor)?;
                journal.approve(approval.clone(), reply.history.management.clone())?;
                save(&node, &journal)?;
                services
                    .merge(&mut node, &self.files, reply.history)
                    .await?;
                install(&mut node, &services, &self.files, &approval).await?;
            } else if !reply.history.management.is_empty() || !reply.history.services.is_empty() {
                return Err("pending result must not disclose history".into());
            }
            Ok::<_, ClientError>(view(journal.get(id)?, &services.access(&node).await?.auth))
        })
        .await;
        connection.close(0u32.into(), b"device result exchange ended");
        result?
    }
}

pub(crate) async fn serve_result(
    connection: crate::control_protocol::ControlOwner,
    node: Arc<Mutex<NativeNetwork>>,
) -> Result<(), ClientError> {
    use crate::control_protocol::AuthenticatedControl;
    let result = async {
        let peer = VerifyingKey::from_bytes(connection.remote_id().as_bytes())?;
        let exchange = connection.accept_control_exchange().await?;
        let request: ResultRequest = crate::service_protocol::read_guest(&exchange).await?;
        let reply = {
            let node = node.lock().await;
            let journal = load(&node)?;
            let entry = journal.get(request.operation)?;
            if entry.operation.description.initiator != peer {
                return Err("result belongs to another initiating node".into());
            }
            ResultReply {
                approval: entry.approval.clone(),
                history: ServiceBundle {
                    management: entry.receipt_history.clone(),
                    services: Vec::new(),
                },
            }
        };
        crate::service_protocol::write_guest(&exchange, &reply).await?;
        connection.revocation_handle().closed().await;
        Ok(())
    }
    .await;
    connection.close(0u32.into(), b"device result exchange ended");
    result
}

pub(crate) async fn serve_device(
    connection: Connection,
    node: Arc<Mutex<NativeNetwork>>,
    services: Arc<Mutex<NativeServices>>,
    files: Arc<NativeFiles>,
) -> Result<(), ClientError> {
    let result = async {
        let peer = VerifyingKey::from_bytes(connection.remote_id().as_bytes())?;
        let mut stream = DataStream::accept(&connection).await?;
        let request: DeviceRequest = read_message(&mut stream).await?;
        let history = ServiceBundle::read(&mut stream).await?;
        let (reply, history) = {
            let mut node = node.lock().await;
            let mut services = services.lock().await;
            services.merge(&mut node, &files, history).await?;
            match request {
                DeviceRequest::Confirm { operation } => {
                    let auth = services.access(&node).await?.auth;
                    if operation.description.initiator != peer
                        || !operation.requires_genesis()
                        || !auth.nodes().iter().any(|(id, _)| *id == peer)
                    {
                        return Err("invalid confirmation requester".into());
                    }
                    let mut journal = load(&node)?;
                    match journal.get(operation.id) {
                        Ok(entry) if entry.operation != operation => {
                            return Err("operation mismatch".into());
                        }
                        Ok(_) => {}
                        Err(_) => {
                            operation.check_current(node.anchor, &auth)?;
                            // A host without custody cannot accept a request
                            // on behalf of some other offline root holder.
                            node.saved_genesis_credential()?;
                            journal.retain(operation.clone())?;
                            save(&node, &journal)?;
                        }
                    }
                    let approval = journal.get(operation.id)?.approval.clone();
                    if let Some(approval) = &approval {
                        install(&mut node, &services, &files, approval).await?;
                    }
                    (DeviceReply { approval }, services.snapshot(&node).await?)
                }
            }
        };
        write_exchange(&mut stream, &reply, &history).await?;
        stream.finish()?;
        let _ = tokio::time::timeout(Duration::from_secs(5), connection.closed()).await;
        Ok(())
    }
    .await;
    connection.close(0u32.into(), b"device management exchange ended");
    result
}

pub(crate) fn validate_saved_operations(
    directory: &Path,
    node: &NativeNetwork,
) -> Result<(), ClientError> {
    debug_assert_eq!(directory, node.directory);
    load(node)?;
    Ok(())
}
