//! IndexedDB and background control adapter for the shared signed service
//! ledger. A successful control reply follows the real snapshot transaction.
use super::*;
use crate::service_policy::{
    GuestInvitation, Service, ServiceAccess, ServiceAction, ServiceLedger, ServiceRule,
};
use crate::service_protocol::{
    GuestRequest, MemberSync, MemberSyncMembership, MemberSyncMode, ServiceBundle,
    next_member_sync_peer, read_guest, write_guest,
};
use futures_util::FutureExt;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use tokio::sync::Notify;

const POLICY_WORKERS: usize = 4;

impl SavedNode {
    pub(super) fn declare_clipboard(&self, capable: bool, enabled: bool) -> Result<Self, JsValue> {
        if self.guest.is_some() || self.invitation.is_none() {
            return Ok(self.clone());
        }
        let access = self.service_access()?;
        if !access.member(access.local) {
            return Ok(self.clone());
        }
        if enabled && !capable {
            return Err(error("system clipboard API unavailable"));
        }
        match access.declaration_change(Service::Clipboard, capable, enabled) {
            Some(change) => self.with_service_action(change).map(|(next, _)| next),
            None => Ok(self.clone()),
        }
    }
    pub(super) fn service_bundle(&self) -> ServiceBundle {
        ServiceBundle {
            management: self.records.to_vec(),
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
    pub(super) fn guest_pair(&self, peer: iroh_base::EndpointId, alpn: &[u8]) -> bool {
        let service = match alpn {
            crate::file_transfer::FILE_ALPN => Service::Files,
            crate::clipboard::CLIPBOARD_ALPN => Service::Clipboard,
            crate::forwarding::TCP_ALPN => Service::TcpForward,
            crate::forwarding::UDP_ALPN => Service::UdpForward,
            crate::web::WEB_ALPN => Service::Web,
            crate::terminal::TERMINAL_ALPN => Service::Terminal,
            _ => return false,
        };
        self.service_access().ok().is_some_and(|access| {
            access
                .guest_pairs_for(service)
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
    pub(super) fn with_service_bundle(
        &self,
        incoming: &ServiceBundle,
    ) -> Result<(Self, Option<std::io::Error>), JsValue> {
        let (next, _, policy_error) = self.reconcile_service_bundle(incoming)?;
        Ok((next, policy_error))
    }

    fn reconcile_service_bundle(
        &self,
        incoming: &ServiceBundle,
    ) -> Result<(Self, Option<ManagementAuth>, Option<std::io::Error>), JsValue> {
        let result = self
            .service_bundle()
            .reconcile(self.invitation()?.anchor, incoming)
            .map_err(error)?;
        let mut next = self.clone();
        next.records = result.bundle.management.into();
        next.services = result.bundle.services;
        let auth = next.verify()?;
        Ok((next, auth, result.policy_error))
    }

    pub(super) fn member_sync(&self, members: &[(VerifyingKey, Access)], mode: MemberSyncMode) -> MemberSync {
        MemberSync::new(
            self.service_bundle(),
            self.device_names.for_members(members),
            mode,
        )
    }

    pub(super) fn with_member_sync(
        &self,
        incoming: MemberSync,
    ) -> Result<(Self, Vec<(VerifyingKey, Access)>, Option<std::io::Error>), JsValue> {
        let (mut next, auth, policy_error) = self.reconcile_service_bundle(&incoming.bundle)?;
        let members = auth.ok_or_else(rejected)?.nodes();
        if policy_error.is_none() && incoming.mode() == MemberSyncMode::Full {
            next.device_names.merge_members(&members, incoming.device_names);
        }
        Ok((next, members, policy_error))
    }
    pub(super) fn declare_files(&self, enabled: bool) -> Result<Self, JsValue> {
        if self.guest.is_some() || self.invitation.is_none() {
            return Ok(self.clone());
        }
        let access = self.service_access()?;
        if !access.member(access.local) {
            return Ok(self.clone());
        }
        let capable = files::capable();
        if enabled && !capable {
            return Err(error("file storage unavailable in this browser"));
        }
        match access.declaration_change(Service::Files, capable, enabled) {
            Some(change) => self.with_service_action(change).map(|(next, _)| next),
            None => Ok(self.clone()),
        }
    }
}

impl BrowserGate {
    pub(super) fn service_snapshot(&self) -> Result<SavedNode, JsValue> {
        self.0.lock().unwrap().saved.clone().ok_or_else(rejected)
    }
}

#[derive(Clone)]
struct MemberSyncAdmission {
    anchor: NetworkAnchor,
    generation: u64,
    secret: [u8; 32],
    membership: MemberSyncMembership,
}

impl MemberSyncAdmission {
    fn capture(
        saved: &SavedNode,
        auth: &ManagementAuth,
        connection: &Connection,
    ) -> Result<Self, JsValue> {
        Ok(Self {
            anchor: saved.invitation()?.anchor,
            generation: saved.generation,
            secret: saved.secret.ok_or_else(rejected)?,
            membership: MemberSyncMembership::capture(
                auth,
                saved.identity(),
                connection.remote_id(),
            )
            .ok_or_else(rejected)?,
        })
    }

    fn is_current(&self, saved: &SavedNode, connection: &Connection) -> Result<bool, JsValue> {
        Ok(saved.generation == self.generation
            && saved.secret == Some(self.secret)
            && saved.invitation()?.anchor == self.anchor
            && BrowserGate::permits(saved, connection.remote_id(), connection.alpn())
            && saved
                .verify()?
                .is_some_and(|auth| self.membership.is_current(&auth, saved.identity())))
    }
}

async fn retain_member_page(
    gate: &BrowserGate,
    files: &Rc<files::BrowserFiles>,
    clipboard: &Rc<clipboard_host::BrowserClipboard>,
    admitted: &MemberSyncAdmission,
    connection: &Connection,
    cancel: &tokio_util::sync::CancellationToken,
    records: crate::service_protocol::ManagementRecords,
) -> std::io::Result<()> {
    let gate = gate.clone();
    let files = files.clone();
    let clipboard = clipboard.clone();
    let admitted = admitted.clone();
    let connection = connection.clone();
    let cancel = cancel.clone();
    let (send, receive) = tokio::sync::oneshot::channel();
    // Started IndexedDB commits retain their runtime publication even if the
    // caller's network deadline expires while awaiting this page.
    wasm_bindgen_futures::spawn_local(async move {
        let result = gate
            .update_saved(|saved| {
                if cancel.is_cancelled() || !admitted.is_current(saved, &connection)? {
                    return Err(rejected());
                }
                *saved = saved.merge_records(records)?;
                Ok(())
            })
            .await;
        files.enforce_access();
        clipboard.enforce_access().await;
        let _ = send.send(result.map_err(|value| {
            std::io::Error::other(
                value
                    .as_string()
                    .unwrap_or_else(|| "member page commit failed".into()),
            )
        }));
    });
    receive
        .await
        .map_err(|_| std::io::Error::other("member page commit ended"))?
}

async fn member_sync_wire<T>(
    deadline: n0_future::time::Instant,
    work: impl std::future::Future<Output = Result<T, JsValue>>,
) -> Result<T, JsValue> {
    n0_future::time::timeout(
        deadline.saturating_duration_since(n0_future::time::Instant::now()),
        work,
    )
    .await
    .map_err(|_| error("member synchronization timed out"))?
}

pub(super) struct BrowserServices {
    clipboard: Rc<clipboard_host::BrowserClipboard>,
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
        clipboard: Rc<clipboard_host::BrowserClipboard>,
        budget: Duration,
    ) -> Rc<Self> {
        let service = Rc::new(Self {
            clipboard,
            cancel: Default::default(),
            connections: Default::default(),
            running: Cell::new(0),
            joined: Notify::new(),
        });
        // An admitted background exchange can occupy one receiver while an
        // explicit sync arrives. Keep multiple bounded policy receivers.
        let policy = std::iter::repeat_n(crate::service_policy::POLICY_ALPN, POLICY_WORKERS);
        for alpn in policy.chain([
            crate::service_policy::GUEST_ALPN,
            crate::device_operation::DEVICE_ALPN,
            crate::device_operation::DEVICE_RESULT_ALPN,
            DEPARTURE_ALPN,
            REMOVAL_ALPN,
            crate::credential_custody::CUSTODY_ALPN,
            device_recovery::RECOVERY_ALPN,
        ]) {
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
                    if matches!(
                        alpn,
                        crate::service_policy::GUEST_ALPN
                            | crate::device_operation::DEVICE_RESULT_ALPN
                            | DEPARTURE_ALPN
                            | REMOVAL_ALPN
                            | device_recovery::RECOVERY_ALPN
                    ) {
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
                            if alpn == crate::device_operation::DEVICE_RESULT_ALPN {
                                let _ = device_management::serve_result(&control, &gate).await;
                            } else if alpn == DEPARTURE_ALPN {
                                let _ = device_management::serve_departure(&control, &gate, &files, deadline).await;
                            } else if alpn == REMOVAL_ALPN {
                                let _ = device_management::serve_removal_notice(&control, &gate, deadline).await;
                            } else if alpn == device_recovery::RECOVERY_ALPN {
                                let _ =
                                    device_recovery::serve_recovery(&control, &gate, &files).await;
                            } else {
                                let _ = service.serve_guest(&control, &gate, &files).await;
                            }
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
                            if alpn == crate::device_operation::DEVICE_ALPN {
                                let _ = device_management::serve_device(&connection, &gate, &files)
                                    .await;
                            } else if alpn == crate::credential_custody::CUSTODY_ALPN {
                                let _ =
                                    device_custody::serve_custody(&connection, &gate, &files).await;
                            } else {
                                let _ = service
                                    .serve_policy(&connection, &gate, &files, deadline)
                                    .await;
                            }
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
                service.clipboard.enforce_access().await;
            }
        });
        service
    }

    pub(super) fn start_member_sync(self: &Rc<Self>, network: &BrowserNetwork) {
        use crate::connection_coordinator::AttemptContext;
        use n0_future::time::{Instant, sleep};

        let owner = Running::new(self);
        let gate = network.gate.clone();
        let files = network.files.clone();
        let coordinator = network.connection_coordinator();
        let endpoint = network.endpoint.clone();
        let policy = network
            .saved
            .resources
            .as_ref()
            .map(|resources| resources.maintenance.clone())
            .unwrap_or_default();
        let interval = Duration::from_millis(policy.exploration_interval_ms);
        let budget = Duration::from_millis(policy.contact_timeout_ms);
        let retry_delay = Duration::from_millis(policy.path_retry_base_ms);
        wasm_bindgen_futures::spawn_local(async move {
            let service = &owner.0;
            let mut last = None;
            let mut retry_at = std::collections::BTreeMap::new();
            let mut check_removal = std::collections::BTreeSet::new();
            while !service.cancel.is_cancelled() {
                let now = Instant::now();
                let next = now + interval;
                retry_at.retain(|_, due| *due > now);
                let mut peer = None;
                let attempt = async {
                    let saved = gate.service_snapshot()?;
                    let stage = saved.stage();
                    if !matches!(stage, "Complete" | "Removed") {
                        return Ok(());
                    }
                    let auth = saved.verify()?.ok_or_else(rejected)?;
                    check_removal.retain(|peer: &iroh_base::EndpointId| {
                        auth.nodes().iter().any(|(id, _)| id.as_bytes() == peer.as_bytes())
                    });
                    let departure = if stage == "Removed" {
                        let Some(entry) = saved.operations.pending_departure(saved.identity(), &auth) else {
                            return Ok(());
                        };
                        Some(crate::device_protocol::DepartureDelivery::new(entry).map_err(error)?)
                    } else {
                        None
                    };
                    let peers = auth
                        .nodes()
                        .into_iter()
                        .filter(|(id, _)| *id != saved.identity())
                        .filter_map(|(id, _)| iroh_base::EndpointId::from_bytes(id.as_bytes()).ok())
                        .filter(|id| !retry_at.contains_key(id));
                    let Some(target) = next_member_sync_peer(peers, last) else {
                        return Ok(());
                    };
                    peer = Some(target);
                    last = peer;
                    let address = saved.address_for(target)?;
                    let deadline = Instant::now() + budget;
                    if let Some(delivery) = departure {
                        let wire = member_sync_wire(deadline, async {
                            let options = iroh::endpoint::ConnectOptions::new()
                                .with_caller_deadline(deadline);
                            let control = endpoint.connect_control_with_opts(address, DEPARTURE_ALPN, options)
                                .await.map_err(error)?.await.map_err(error)?;
                            let control = Arc::new(control).retain_control();
                            let _close = RequestConnection(Some(control.clone()));
                            delivery.send(&control).await.map_err(error)
                        });
                        let record = tokio::select! {
                            biased;
                            _ = service.cancel.cancelled() => return Ok(()),
                            record = wire => record?,
                        };
                        return gate.update_saved(|current| {
                            let recipient = VerifyingKey::from_bytes(target.as_bytes()).map_err(error)?;
                            current.operations.confirm_departure_delivery(
                                saved.identity(), delivery.approval.operation.id, record, recipient,
                            ).map_err(error)
                        }).await;
                    }
                    if check_removal.remove(&target) {
                        service.receive_removal_notice(&endpoint, address, &gate, &files, deadline)
                            .await?;
                        return Ok(());
                    }
                    check_removal.insert(target);
                    let connection = coordinator
                        .connect_alpn(
                            address,
                            crate::service_policy::POLICY_ALPN,
                            AttemptContext::new(deadline, service.cancel.clone()),
                        )
                        .await
                        .map_err(error)?;
                    let _close = RequestConnection(Some(connection.clone()));
                    service
                        .sync_member(
                            &connection,
                            &gate,
                            &files,
                            MemberSyncMode::Management,
                            deadline,
                        )
                        .await?;
                    check_removal.remove(&target);
                    Ok(())
                };
                if attempt.await.is_err()
                    && let Some(peer) = peer
                {
                    retry_at.insert(peer, Instant::now() + retry_delay);
                }
                tokio::select! {
                    biased;
                    _ = service.cancel.cancelled() => break,
                    _ = sleep(next.saturating_duration_since(Instant::now())) => {}
                }
            }
        });
    }

    async fn receive_removal_notice(
        &self,
        endpoint: &RestrictedEndpoint,
        address: EndpointAddr,
        gate: &BrowserGate,
        files: &Rc<files::BrowserFiles>,
        deadline: n0_future::time::Instant,
    ) -> Result<bool, JsValue> {
        use crate::device_protocol::RemovalNotice;
        use iroh::endpoint::{ConnectWithOptsError, ConnectingError};
        use n0_future::time::{Instant, timeout};

        let saved = gate.service_snapshot()?;
        if !BrowserGate::permits(&saved, address.id, REMOVAL_ALPN) {
            return Err(error("current membership required to request a removal notice"));
        }
        let holder = VerifyingKey::from_bytes(address.id.as_bytes()).map_err(error)?;
        let grant = saved.authorization.ok_or_else(rejected)?;
        let unreachable = || error("removal notice peer temporarily unreachable");
        let wire = timeout(deadline.saturating_duration_since(Instant::now()), async {
            let options = iroh::endpoint::ConnectOptions::new().with_caller_deadline(deadline);
            let control = endpoint.connect_control_with_opts(address, REMOVAL_ALPN, options)
                .await.map_err(|cause| match cause {
                    ConnectWithOptsError::AttemptTimedOut { .. } => unreachable(),
                    _ => error(cause),
                })?
                .await.map_err(|cause| match cause {
                    ConnectingError::AttemptTimedOut { .. } => unreachable(),
                    _ => error(cause),
                })?;
            let control = Arc::new(control).retain_control();
            let _close = RequestConnection(Some(control.clone()));
            RemovalNotice::request(&control, grant).await.map_err(error)
        });
        let notice = tokio::select! {
            biased;
            _ = self.cancel.cancelled() => return Err(error("browser node disconnected")),
            result = wire => result.map_err(|_| unreachable())??,
        };
        let Some(notice) = notice else { return Ok(false) };
        let removed = gate.update_saved(|saved| {
            if saved.authorization != Some(grant) {
                return Ok(false);
            }
            if saved.stage() == "Removed" {
                return Ok(true);
            }
            if saved.stage() != "Complete" {
                return Ok(false);
            }
            let auth = saved.verify()?.ok_or_else(rejected)?;
            let management = notice.verify(&auth, grant, saved.identity(), holder).map_err(error)?;
            let (next, policy_error) = saved.with_service_bundle(&ServiceBundle {
                management,
                services: Vec::new(),
            })?;
            if let Some(policy_error) = policy_error {
                return Err(error(policy_error));
            }
            *saved = next;
            Ok(true)
        }).await?;
        files.enforce_access();
        self.clipboard.enforce_access().await;
        Ok(removed)
    }

    async fn refresh_local_declarations(
        &self,
        gate: &BrowserGate,
        files: &Rc<files::BrowserFiles>,
    ) -> Result<bool, JsValue> {
        if !gate.service_snapshot()?.service_access()?.has_stale_local_declarations() {
            return Ok(false);
        }
        let clipboard = clipboard_host::capability().await;
        let clipboard_capable = clipboard.read && clipboard.write;
        let files_capable = files::capable();
        let declarations = [
            (Service::Files, files_capable, files_capable && files.enabled.get()),
            (Service::Clipboard, clipboard_capable, clipboard_capable && self.clipboard.state.status().enabled),
        ];
        gate.update_saved(|saved| {
            let mut refreshed = false;
            if saved.stage() == "Complete" {
                let access = saved.service_access()?;
                for (service, capable, enabled) in declarations {
                    if let Some(change) = access.refresh_declaration(service, capable, enabled) {
                        *saved = saved.with_service_action(change)?.0;
                        refreshed = true;
                    }
                }
            }
            Ok(refreshed)
        }).await
    }

    async fn serve_policy(
        &self,
        connection: &Connection,
        gate: &BrowserGate,
        files: &Rc<files::BrowserFiles>,
        deadline: n0_future::time::Instant,
    ) -> Result<(), JsValue> {
        let admitted = {
            let saved = gate.qualified_snapshot(connection).await?;
            let auth = saved.verify()?.ok_or_else(rejected)?;
            MemberSyncAdmission::capture(&saved, &auth, connection)?
        };
        let anchor = gate.service_snapshot()?.invitation()?.anchor;
        let (local, known) = {
            let saved = gate.service_snapshot()?;
            (saved.records.to_vec(), saved.verify()?.ok_or_else(rejected)?)
        };
        let (mut stream, incoming) = member_sync_wire(deadline, async {
            let mut stream = crate::data_stream::DataStream::accept(connection)
                .await
                .map_err(error)?;
            let incoming = MemberSync::read_resuming(&mut stream, anchor, local, known,
                |page| retain_member_page(gate, files, &self.clipboard, &admitted, connection, &self.cancel, page)).await.map_err(error)?;
            Ok((stream, incoming))
        })
        .await?;
        let mode = incoming.mode();
        let (mut response, policy_error) = gate.update_saved(|saved| {
            if self.cancel.is_cancelled() || !admitted.is_current(saved, connection)? {
                return Err(rejected());
            }
            let (next, members, policy_error) = saved.with_member_sync(incoming)?;
            let response = next.member_sync(&members, mode);
            *saved = next;
            Ok((response, policy_error))
        })
        .await?;
        files.enforce_access();
        self.clipboard.enforce_access().await;
        if let Some(policy_error) = policy_error {
            return Err(error(policy_error));
        }
        if mode == MemberSyncMode::Full && self.refresh_local_declarations(gate, files).await? {
            let saved = gate.qualified_snapshot(connection).await?;
            let auth = saved.verify()?.ok_or_else(rejected)?;
            response = saved.member_sync(&auth.nodes(), mode);
        }
        member_sync_wire(deadline, async {
            response.write_resuming(&mut stream).await.map_err(error)?;
            stream.finish().map_err(error)
        })
        .await?;
        let _ = n0_future::time::timeout(Duration::from_secs(5), connection.closed()).await;
        Ok(())
    }

    async fn sync_member(
        &self,
        connection: &Connection,
        gate: &BrowserGate,
        files: &Rc<files::BrowserFiles>,
        mode: MemberSyncMode,
        deadline: n0_future::time::Instant,
    ) -> Result<(), JsValue> {
        if mode == MemberSyncMode::Full {
            self.refresh_local_declarations(gate, files).await?;
        }
        let (snapshot, admitted, known) = {
            let saved = gate.qualified_snapshot(connection).await?;
            let auth = saved.verify()?.ok_or_else(rejected)?;
            (
                saved.member_sync(&auth.nodes(), mode),
                MemberSyncAdmission::capture(&saved, &auth, connection)?,
                auth,
            )
        };
        let anchor = gate.service_snapshot()?.invitation()?.anchor;
        let transfer = member_sync_wire(deadline, async {
            snapshot.exchange_resuming(connection, anchor, known,
                |page| retain_member_page(gate, files, &self.clipboard, &admitted, connection, &self.cancel, page)).await.map_err(error)
        });
        let incoming = tokio::select! {
            biased;
            _ = self.cancel.cancelled() => return Err(rejected()),
            incoming = transfer => incoming?,
        };
        let policy_error = gate.update_saved(|saved| {
            if self.cancel.is_cancelled() || !admitted.is_current(saved, connection)? {
                return Err(rejected());
            }
            let (next, _, policy_error) = saved.with_member_sync(incoming)?;
            *saved = next;
            Ok(policy_error)
        })
        .await?;
        files.enforce_access();
        self.clipboard.enforce_access().await;
        if let Some(policy_error) = policy_error {
            return Err(error(policy_error));
        }
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
        let (saved, version) = gate.snapshot().await?;
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
        version.commit(&next).await?;
        files.enforce_access();
        self.clipboard.enforce_access().await;
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
    pub async fn service_status(&self) -> Result<String, JsValue> {
        let capability = clipboard_host::capability().await;
        let clipboard = self.clipboard.state.status();
        let saved = self.gate.service_snapshot()?;
        let access = saved.service_access()?;
        let clipboard_guest = saved
            .guest
            .as_ref()
            .map(GuestInvitation::details)
            .transpose()
            .map_err(error)?
            .is_some_and(|scope| scope.service == Service::Clipboard);
        let terminal_guest = saved.guest.as_ref().map(GuestInvitation::details).transpose().map_err(error)?
            .is_some_and(|scope| scope.service == Service::Terminal);
        let mut status = access
            .status(
                saved.guest.as_ref(),
                if terminal_guest { false } else if clipboard_guest {
                    capability.read && capability.write
                } else {
                    files::capable()
                },
                if terminal_guest { false } else if clipboard_guest {
                    clipboard.enabled
                } else {
                    self.files.enabled.get()
                },
            )
            .map_err(error)?;
        if saved.guest.is_none() {
            status["services"]
                .as_array_mut()
                .ok_or_else(rejected)?
                .push(access.local_service_status(
                    Service::Clipboard,
                    capability.read && capability.write,
                    clipboard.enabled,
                ));
            for service in [Service::TcpForward, Service::UdpForward, Service::Web, Service::Terminal] {
                status["services"].as_array_mut().ok_or_else(rejected)?.push(access.local_service_status(service, false, false));
            }
        }
        Ok(status.to_string())
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
        self.require_device_ready()?;
        let action = ServiceAction::SetRule {
            provider: provider.parse().map_err(error)?,
            service: serde_json::from_value(serde_json::Value::String(service)).map_err(error)?,
            previous: previous.map(|id| id.parse()).transpose().map_err(error)?,
            rule: serde_json::from_str::<ServiceRule>(&rule).map_err(error)?,
        };
        let record = self.gate.update_saved(|saved| {
            let (next, record) = saved.with_service_action(action)?;
            *saved = next;
            Ok(record)
        })
        .await?;
        self.files.enforce_access();
        self.clipboard.enforce_access().await;
        Ok(Hash::digest(record).to_string())
    }
    pub async fn sync_services(&mut self, target: String) -> Result<String, JsValue> {
        self.require_device_ready()?;
        let deadline = n0_future::time::Instant::now() + self.caller_budget;
        let connection = self
            .connect_target_until(
                target.parse().map_err(error)?,
                crate::service_policy::POLICY_ALPN,
                deadline,
            )
            .await?;
        let _close = RequestConnection(Some(connection.clone()));
        self.services
            .sync_member(
                &connection,
                &self.gate,
                &self.files,
                MemberSyncMode::Full,
                deadline,
            )
            .await?;
        Ok("{\"saved_locally\":true}".into())
    }
    /// A path failure leaves the previously saved membership grant intact.
    pub async fn check_membership_removal(&mut self, target: String) -> Result<bool, JsValue> {
        self.require_device_ready()?;
        self.services.receive_removal_notice(
            &self.endpoint, self.target_address(target.parse().map_err(error)?)?,
            &self.gate, &self.files, n0_future::time::Instant::now() + self.caller_budget,
        ).await
    }
    pub async fn issue_guest(
        &mut self,
        provider_address: String,
        expires_unix_seconds: u64,
    ) -> Result<String, JsValue> {
        self.issue_service_guest(provider_address, "files".into(), expires_unix_seconds)
            .await
    }
    pub async fn issue_service_guest(
        &mut self,
        provider_address: String,
        service: String,
        expires_unix_seconds: u64,
    ) -> Result<String, JsValue> {
        self.require_device_ready()?;
        let service: Service =
            serde_json::from_value(serde_json::Value::String(service)).map_err(error)?;
        if !service.guest_supported() {
            return Err(error("service guest adapter unavailable"));
        }
        let created = crate::service_policy::now();
        if expires_unix_seconds <= created {
            return Err(error("guest deadline must be in the future"));
        }
        let provider: EndpointAddr = serde_json::from_str(&provider_address).map_err(error)?;
        let provider_id = VerifyingKey::from_bytes(provider.id.as_bytes()).map_err(error)?;
        let (issuer, bootstrap) = self.contact();
        self.gate.update_saved(|saved| {
            let access = saved.service_access()?;
            if !access.ledger.rule(provider_id, service).rule.guests {
                return Err(rejected());
            }
            let token: [u8; 32] = rand::random();
            let (next, record) = saved.with_service_action(ServiceAction::InviteGuest {
                provider: provider_id,
                service,
                token_hash: Hash::digest(token),
                created,
                expires: expires_unix_seconds,
            })?;
            let invitation = GuestInvitation {
                anchor: saved.invitation()?.anchor,
                issuer,
                provider,
                bootstrap,
                record,
                token,
            };
            let text = invitation.text().map_err(error)?;
            *saved = next;
            Ok(text)
        })
        .await
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
        self.clipboard.enforce_access().await;
        if let Some(policy_error) = policy_error {
            return Err(error(policy_error));
        }
        let access = self.saved.service_access()?;
        if !access.ledger.guest_access(
            VerifyingKey::from_bytes(invitation.provider.id.as_bytes()).map_err(error)?,
            invitation.details().map_err(error)?.service,
            access.local,
            &access.auth,
            crate::service_policy::now(),
        ) {
            return Err(rejected());
        }
        self.guest_status()
    }
    pub async fn revoke_guest(&mut self, invitation: String) -> Result<String, JsValue> {
        self.require_device_ready()?;
        let invitation = invitation.parse().map_err(error)?;
        let record = self.gate.update_saved(|saved| {
            let (next, record) =
                saved.with_service_action(ServiceAction::EndGuest { invitation })?;
            *saved = next;
            Ok(record)
        })
        .await?;
        self.files.enforce_access();
        self.clipboard.enforce_access().await;
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
            self.clipboard.enforce_access().await;
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
