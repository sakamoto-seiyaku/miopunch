//! Public Native device operations. Credentials stay at the confirming host;
//! pending descriptions and original signed results survive retries/reopen.
use std::{path::Path, sync::Arc, time::Duration};

use iroh::{EndpointAddr, endpoint::Connection};
use p2panda_core::{Hash, SigningKey, VerifyingKey};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::{
    control_protocol::{AuthenticatedControl, ControlOwner},
    data_stream::DataStream,
    device_operation::{
        DEVICE_ALPN, DEVICE_RESULT_ALPN, DeviceAction, DeviceApproval, DeviceOperation,
        OperationJournal,
    },
    device_protocol::{
        DEPARTURE_ALPN, DepartureDelivery, DeviceReply, DeviceRequest, REMOVAL_ALPN, RemovalNotice,
        ResultReply, ResultRequest, read_message, write_exchange,
    },
    native_client::{ClientError, NativeClient},
    native_file_transfer::NativeFiles,
    native_migration::SourceExit,
    native_network::NativeNetwork,
    native_services::NativeServices,
    recovery_card::RecoveryCard,
    resources::{ContactClass, ResourceEndpoint},
    service_protocol::{ServiceBundle, next_member_sync_peer, read_guest, write_guest},
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
    node.store.changes.publish();
    Ok(())
}
pub(crate) fn operation_state(node: &NativeNetwork) -> Result<Value, ClientError> {
    let auth = node.store.authorization();
    let local = node.node_key.verifying_key();
    Ok(
        json!({"operations":load(node)?.entries.iter().map(|entry| entry.view(local, auth)).collect::<Vec<_>>(),
        "holds_genesis":node.directory.join("genesis.credential").exists()}),
    )
}
async fn install(
    node: &mut NativeNetwork,
    services: &NativeServices,
    files: &Arc<NativeFiles>,
    approval: &DeviceApproval,
) -> Result<(), ClientError> {
    node.ingest_management_record(&approval.record, None)
        .await?;
    services.refresh_access(node, files).await?;
    Ok(())
}

impl NativeClient {
    /// Propose one exact effect; this neither unlocks a key nor executes it.
    pub async fn propose_device_operation(
        &self,
        action: DeviceAction,
        target: VerifyingKey,
    ) -> Result<Value, ClientError> {
        let client_runtime = self.runtime();
        client_runtime.require_ready()?;
        let node = client_runtime.node.lock().await;
        let auth = client_runtime.services.lock().await.access(&node).await?.auth;
        let operation =
            DeviceOperation::propose(node.anchor, &auth, node.status().node_id, action, target)?;
        let mut journal = load(&node)?;
        journal.retain(operation.clone())?;
        save(&node, &journal)?;
        Ok(journal
            .get(operation.id)?
            .view(node.node_key.verifying_key(), &auth))
    }

    pub async fn device_operations(&self) -> Result<Value, ClientError> {
        let client_runtime = self.runtime();
        client_runtime.require_ready()?;
        let node = client_runtime.node.lock().await;
        operation_state(&node)
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
        let client_runtime = self.runtime();
        client_runtime.require_ready()?;
        let mut node = client_runtime.node.lock().await;
        let services = client_runtime.services.lock().await;
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
        install(&mut node, &services, &client_runtime.files, &approval).await?;
        Ok(journal
            .get(id)?
            .view(node.node_key.verifying_key(), node.store.authorization()))
    }

    /// Daily administrator removal or explicitly confirmed ordinary departure.
    /// It does not read the genesis credential, even on its custodian.
    pub async fn execute_device_operation(&self, id: Hash) -> Result<Value, ClientError> {
        let client_runtime = self.runtime();
        client_runtime.require_ready()?;
        let mut node = client_runtime.node.lock().await;
        let services = client_runtime.services.lock().await;
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
        install(&mut node, &services, &client_runtime.files, &approval).await?;
        Ok(journal
            .get(id)?
            .view(node.node_key.verifying_key(), node.store.authorization()))
    }

    /// Send/resume this one operation at a known custodian. It returns pending
    /// until that host explicitly confirms; the return contains no root key.
    pub async fn request_device_confirmation(
        &self,
        id: Hash,
        custodian: EndpointAddr,
    ) -> Result<Value, ClientError> {
        let client_runtime = self.runtime();
        client_runtime.require_ready()?;
        let (operation, history) = {
            let node = client_runtime.node.lock().await;
            let operation = load(&node)?.get(id)?.operation.clone();
            if operation.description.initiator != node.status().node_id
                || !operation.requires_genesis()
            {
                return Err("only the initiating node can request this confirmation".into());
            }
            (operation, client_runtime.services.lock().await.snapshot(&node).await?)
        };
        let connection = client_runtime.resources.connect(custodian, DEVICE_ALPN).await?;
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
            let mut node = client_runtime.node.lock().await;
            let mut services = client_runtime.services.lock().await;
            services.merge(&mut node, &client_runtime.files, history).await?;
            let mut journal = load(&node)?;
            if let Some(approval) = reply.approval {
                approval.verify(&operation, node.anchor)?;
                journal.approve(approval.clone(), receipt_history(&node, &approval).await?)?;
                save(&node, &journal)?;
                install(&mut node, &services, &client_runtime.files, &approval).await?;
            }
            stream.finish()?;
            Ok::<_, ClientError>(
                journal
                    .get(id)?
                    .view(node.node_key.verifying_key(), node.store.authorization()),
            )
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
        let client_runtime = self.runtime();
        client_runtime.require_ready()?;
        let operation = {
            let node = client_runtime.node.lock().await;
            let operation = load(&node)?.get(id)?.operation.clone();
            if operation.description.initiator != node.status().node_id {
                return Err("only the initiator can collect this result".into());
            }
            operation
        };
        let connection = client_runtime
            .resources
            .connect_control(custodian, DEVICE_RESULT_ALPN)
            .await?;
        let result = tokio::time::timeout(Duration::from_secs(60), async {
            use crate::control_protocol::AuthenticatedControl;
            let exchange = connection.open_control_exchange().await?;
            crate::service_protocol::write_guest(&exchange, &ResultRequest { operation: id })
                .await?;
            let reply: ResultReply = crate::service_protocol::read_guest(&exchange).await?;
            let mut node = client_runtime.node.lock().await;
            let mut services = client_runtime.services.lock().await;
            let mut journal = load(&node)?;
            if let Some(approval) = reply.approval {
                approval.verify(&operation, node.anchor)?;
                journal.approve(approval.clone(), reply.history.management.clone())?;
                save(&node, &journal)?;
                services
                    .merge(&mut node, &client_runtime.files, reply.history)
                    .await?;
                install(&mut node, &services, &client_runtime.files, &approval).await?;
            } else if !reply.history.management.is_empty() || !reply.history.services.is_empty() {
                return Err("pending result must not disclose history".into());
            }
            Ok::<_, ClientError>(
                journal
                    .get(id)?
                    .view(node.node_key.verifying_key(), node.store.authorization()),
            )
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

pub(crate) async fn recover_saved_departure(node: &mut NativeNetwork) -> Result<(), ClientError> {
    let journal = load(node)?;
    let local = node.node_key.verifying_key();
    // The journal is saved before management application. Finish an
    // interrupted local commit before opening an endpoint or reporting state.
    let pending = journal.entries.into_iter().rev().find(|entry| {
        let auth = node.store.authorization();
        entry.is_local_leave(local)
            && entry
                .approval
                .as_ref()
                .is_some_and(|approval| !auth.contains_operation(approval.record_id()))
            && entry.operation.check_current(node.anchor, auth).is_ok()
    });
    if let Some(entry) = pending {
        node.store.apply_batch(&entry.receipt_history).await?;
        node.refresh_authorization();
    }
    Ok(())
}

pub(crate) fn migration_departure(node: &NativeNetwork) -> Result<SourceExit, ClientError> {
    let local = node.status().node_id;
    let auth = node.store.authorization();
    for entry in load(node)?.entries.into_iter().rev().filter(|entry| entry.is_local_leave(local)) {
        if let Some(approval) = &entry.approval
            && approval.effect_current(auth)
        {
            let record = approval.record_id();
            return Ok(match entry.departure_recipient() {
                Some(recipient) => SourceExit::Delivered { record, recipient },
                None => SourceExit::PendingDelivery { record },
            });
        }
    }
    Ok(match auth.nodes().into_iter().find(|(id, _)| *id == local) {
        Some((_, role)) if role == p2panda_auth::Access::manage() => SourceExit::GenesisConfirmationRequired,
        Some(_) => SourceExit::NotStarted,
        None => SourceExit::MembershipAlreadyEnded,
    })
}

pub(crate) async fn commit_migration_departure(
    node: &mut NativeNetwork,
    genesis: Option<&SigningKey>,
) -> Result<(), ClientError> {
    recover_saved_departure(node).await?;
    let result = migration_departure(node)?;
    let signer = match (result, genesis) {
        (SourceExit::NotStarted, _) => &node.node_key,
        (SourceExit::GenesisConfirmationRequired, Some(root)) => root,
        _ => return Ok(()),
    };
    let auth = node.store.authorization();
    let local = node.status().node_id;
    let mut journal = load(node)?;
    let operation = match journal.entries.iter().rev().find(|entry| {
        entry.is_local_leave(local) && entry.operation.check_current(node.anchor, auth).is_ok()
    }) {
        Some(entry) => entry.operation.clone(),
        None => DeviceOperation::propose(node.anchor, auth, local, DeviceAction::Leave, local)?,
    };
    let approval = operation.approve(node.anchor, auth, signer)?;
    journal.retain(operation)?;
    journal.approve(approval.clone(), receipt_history(node, &approval).await?)?;
    save(node, &journal)?;
    node.ingest_management_record(&approval.record, None).await?;
    Ok(())
}

pub(crate) async fn deliver_departure(
    node: &Mutex<NativeNetwork>,
    resources: &ResourceEndpoint,
    last: &mut Option<iroh::EndpointId>,
) -> Result<(), ClientError> {
    deliver_departure_in_scope(node, resources, last, false).await
}

pub(crate) async fn deliver_migration_departure(
    node: &Mutex<NativeNetwork>,
    resources: &ResourceEndpoint,
    last: &mut Option<iroh::EndpointId>,
) -> Result<(), ClientError> {
    deliver_departure_in_scope(node, resources, last, true).await
}

async fn deliver_departure_in_scope(
    node: &Mutex<NativeNetwork>,
    resources: &ResourceEndpoint,
    last: &mut Option<iroh::EndpointId>,
    migration: bool,
) -> Result<(), ClientError> {
    let (delivery, address, source_gate) = {
        let node = node.lock().await;
        let local = node.node_key.verifying_key();
        if !node.store.known_removed(local) {
            return Ok(());
        }
        let journal = load(&node)?;
        let auth = node.store.authorization();
        let Some(entry) = journal.pending_departure(local, auth) else {
            return Ok(());
        };
        let peers = auth
            .nodes()
            .into_iter()
            .filter_map(|(id, _)| iroh::EndpointId::from_bytes(id.as_bytes()).ok())
            .filter(|peer| resources.contact_ready(*peer, ContactClass::Background));
        let Some(target) = next_member_sync_peer(peers, *last) else {
            return Ok(());
        };
        *last = Some(target);
        let mut address = EndpointAddr::new(target);
        let source_gate = if migration {
            let config = node.resource_configuration()?;
            let contacts = crate::member_contacts::MemberContacts::open(
                node.directory.clone(), node.gate.clone(), config,
            )?;
            contacts.merge_departure(&mut address);
            Some(node.gate.clone())
        } else {
            None
        };
        (DepartureDelivery::new(entry)?, address, source_gate)
    };
    let target = address.id;
    let deadline = resources.contact_deadline();
    let (connection, attempt) = if let Some(gate) = source_gate {
        let (delivery, attempt) = resources.connect_migration(address, DEPARTURE_ALPN, &gate, true).await?;
        let control = delivery.control().map_err(|error| attempt.failed(error.to_string()))?;
        (control, attempt)
    } else {
        resources.connect_control_attempt(address, DEPARTURE_ALPN, ContactClass::Background).await?
    };
    let receipt = tokio::time::timeout_at(deadline.into(), delivery.send(&connection)).await;
    connection.close(0u32.into(), b"departure delivery ended");
    let record = match receipt {
        Ok(Ok(record)) => record,
        Ok(Err(error)) => return Err(attempt.failed(error.to_string()).into()),
        Err(error) => return Err(attempt.failed(error.to_string()).into()),
    };
    let node = node.lock().await;
    let mut journal = load(&node)?;
    journal.confirm_departure_delivery(
        node.node_key.verifying_key(),
        delivery.approval.operation.id,
        record,
        VerifyingKey::from_bytes(target.as_bytes())?,
    )?;
    save(&node, &journal)?;
    attempt.succeeded();
    Ok(())
}

pub(crate) async fn serve_departure(
    connection: ControlOwner,
    node: &Mutex<NativeNetwork>,
    services: &Mutex<NativeServices>,
    files: &Arc<NativeFiles>,
    deadline: tokio::time::Instant,
) -> Result<(), ClientError> {
    let result = async {
        let peer = VerifyingKey::from_bytes(connection.remote_id().as_bytes())?;
        let (exchange, delivery) = tokio::time::timeout_at(deadline, async {
            let exchange = connection.accept_control_exchange().await?;
            let delivery: DepartureDelivery = read_guest(&exchange).await?;
            Ok::<_, ClientError>((exchange, delivery))
        })
        .await??;
        {
            let mut node = node.lock().await;
            if node.secret.is_none() {
                return Err("completed local admission is required for departure delivery".into());
            }
            if let Some(records) = delivery.verify(
                node.anchor,
                node.store.authorization(),
                node.node_key.verifying_key(),
                peer,
            )? {
                node.store.apply_batch(&records).await?;
                node.refresh_authorization();
                let services = services.lock().await;
                services.refresh_access(&node, files).await?;
                if !delivery.approval.effect_current(node.store.authorization()) {
                    return Err("departure decision did not apply".into());
                }
            }
        }
        tokio::time::timeout_at(deadline, async {
            write_guest(&exchange, &delivery.approval.record_id()).await?;
            connection.revocation_handle().closed().await;
            Ok::<_, ClientError>(())
        })
        .await?
    }
    .await;
    connection.close(0u32.into(), b"departure delivery ended");
    result
}

pub(crate) async fn serve_removal_notice(
    connection: ControlOwner,
    node: &Mutex<NativeNetwork>,
    deadline: tokio::time::Instant,
) -> Result<(), ClientError> {
    let result = tokio::time::timeout_at(deadline, async {
        let peer = VerifyingKey::from_bytes(connection.remote_id().as_bytes())?;
        let exchange = connection.accept_control_exchange().await?;
        let grant: Hash = read_guest(&exchange).await?;
        let notice = {
            let node = node.lock().await;
            if node.admission_stage() != crate::manual_admission::AdmissionStage::Complete {
                return Err("current membership required to serve a removal notice".into());
            }
            if let Some(decision) = node.store.authorization().removal_decision(grant, peer) {
                let records = node.store.applied_records().await?.iter()
                    .map(|record| (record.header().to_vec(), record.body().map(Vec::from)))
                    .collect();
                Some(RemovalNotice::new(decision, &records)?)
            } else {
                None
            }
        };
        write_guest(&exchange, &notice).await?;
        connection.revocation_handle().closed().await;
        Ok::<_, ClientError>(())
    }).await;
    connection.close(0u32.into(), b"removal notice exchange ended");
    result?
}

pub(crate) async fn receive_removal_notice(
    node: Arc<Mutex<NativeNetwork>>,
    services: Arc<Mutex<NativeServices>>,
    files: Arc<NativeFiles>,
    resources: &ResourceEndpoint,
    address: EndpointAddr,
    class: ContactClass,
    commits: &tokio_util::task::TaskTracker,
) -> Result<bool, ClientError> {
    let holder = VerifyingKey::from_bytes(address.id.as_bytes())?;
    let (local, grant) = {
        let node = node.lock().await;
        let auth = node.store.authorization();
        let local = node.node_key.verifying_key();
        if node.admission_stage() != crate::manual_admission::AdmissionStage::Complete
            || auth.known_removed(holder)
        {
            return Err("current membership required to request a removal notice".into());
        }
        (local, auth.authorization_origin_at(local, &auth.tips(), false)
            .ok_or("local membership grant unavailable")?)
    };
    let deadline = resources.contact_deadline();
    let timed_out = || std::io::Error::new(
        std::io::ErrorKind::TimedOut, "removal notice peer temporarily unreachable",
    );
    let contact = tokio::time::timeout_at(
        deadline.into(), resources.connect_control_attempt(address, REMOVAL_ALPN, class),
    ).await;
    let (connection, attempt) = match contact {
        Ok(Ok(contact)) => contact,
        Ok(Err(error)) if n0_future::time::Instant::now() < deadline => return Err(error.into()),
        _ => return Err(timed_out().into()),
    };
    let wire = tokio::time::timeout_at(deadline.into(), RemovalNotice::request(&connection, grant)).await;
    connection.close(0u32.into(), b"removal notice exchange ended");
    let notice = match wire {
        Ok(Ok(notice)) => notice,
        Ok(Err(error)) => return Err(attempt.failed(error.to_string()).into()),
        Err(error) => {
            attempt.failed(error.to_string());
            return Err(timed_out().into());
        }
    };
    attempt.succeeded();
    let Some(notice) = notice else { return Ok(false) };
    // Once a proof arrives, shutdown and wire expiry cannot interrupt the
    // local transaction between durable commit and gate installation.
    commits.spawn(async move {
        let mut node = node.lock().await;
        if node.store.known_removed(local) {
            return Ok(true);
        }
        if node.admission_stage() != crate::manual_admission::AdmissionStage::Complete {
            return Ok(false);
        }
        let auth = node.store.authorization();
        if auth.authorization_origin_at(local, &auth.tips(), false) != Some(grant) {
            return Ok(false);
        }
        let records = notice.verify(auth, grant, local, holder)?;
        node.store.apply_batch(&records).await?;
        node.refresh_authorization();
        let services = services.lock().await;
        services.refresh_access(&node, &files).await?;
        Ok(true)
    }).await?
}

impl NativeClient {
    /// Retrieve a signed decision concerning this device's current grant.
    /// A contact failure leaves the saved grant unchanged.
    pub async fn check_membership_removal(&self, address: EndpointAddr) -> Result<bool, ClientError> {
        let client_runtime = self.runtime();
        client_runtime.require_ready()?;
        receive_removal_notice(
            client_runtime.node.clone(), client_runtime.services.clone(), client_runtime.files.clone(), &client_runtime.resources,
            address, ContactClass::Foreground, &client_runtime.removal_commits,
        ).await
    }
}
