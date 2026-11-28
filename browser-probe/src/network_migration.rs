//! One leased snapshot selects the active network. The inactive snapshot can
//! only be used for a scoped outgoing admission, validation or departure.
use super::*;
use crate::{
    client_invitation::ClientInvitation,
    device_operation::{DeviceAction, DeviceOperation},
    device_protocol::DepartureDelivery,
    service_policy::POLICY_ALPN,
    service_protocol::{MemberSync, MemberSyncMembership, MemberSyncMode, next_member_sync_peer},
};
use n0_future::time::{Instant, sleep, timeout};
use std::rc::Rc;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

const RETRY: Duration = Duration::from_millis(500);

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct SavedMigration {
    source: NetworkAnchor,
    destination: NetworkAnchor,
    pub(super) inactive: Box<SavedNode>,
    verified: Option<VerifiedTarget>,
    receipt_pending: bool,
}

impl SavedMigration {
    fn switched(&self) -> bool {
        self.verified.is_some()
    }

    fn preparation_ended(&self) -> bool {
        !self.switched() && matches!(self.inactive.stage(), "Rejected" | "Removed")
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct VerifiedTarget {
    peer: iroh_base::EndpointId,
    admission: AdmissionAttempt,
}

#[derive(Serialize)]
#[serde(tag = "stage")]
enum SourceExit {
    NotStarted,
    GenesisConfirmationRequired,
    MembershipAlreadyEnded,
    PendingDelivery {
        record: Hash,
    },
    Delivered {
        record: Hash,
        recipient: VerifyingKey,
    },
}

impl Configuration {
    fn migration_target(
        &self,
        source: Option<&ClientInvitation>,
        destination: &ClientInvitation,
    ) -> Result<Self, JsValue> {
        let mut target = self.clone();
        if !target.relay_replacement
            && let Some(source) = source
        {
            let inherited: Vec<_> = source
                .bootstrap()
                .relays
                .iter()
                .cloned()
                .chain(source.address().relay_urls().map(ToString::to_string))
                .filter_map(|relay| origin(&relay).ok())
                .collect();
            let defaults: Vec<_> = Self::default()
                .relays
                .iter()
                .filter_map(|relay| origin(relay).ok())
                .collect();
            // Contact hints imported from the old invitation are not device
            // preferences. The new invitation can introduce the same relay.
            target.relays.retain(|relay| {
                origin(relay)
                    .is_ok_and(|relay| defaults.contains(&relay) || !inherited.contains(&relay))
            });
        }
        target.import_contact(destination)?;
        target.validate_signaling()?;
        target.allowed_relays()?;
        Ok(target)
    }
}

impl SavedNode {
    /// Bind browser signaling carriers during normal startup with current
    /// resource preferences and the original trusted destination invitation.
    pub(super) fn reopen_migration_resources(
        &mut self,
        active: &Configuration,
    ) -> Option<Configuration> {
        let migration = self.migration.as_mut()?;
        if migration.switched() {
            if let Some(old) = &mut migration.inactive.resources {
                old.disabled = active.disabled.clone();
                if active.relay_replacement {
                    old.relays = active.relays.clone();
                    old.relay_replacement = true;
                }
            }
            return None;
        }
        if migration.preparation_ended() {
            return None;
        }
        let invitation = migration.inactive.complete_invitation.as_ref()?;
        let target = match active.migration_target(self.complete_invitation.as_ref(), invitation) {
            Ok(target) => target,
            Err(_) => {
                migration.inactive.resources = None;
                return None;
            }
        };
        migration.inactive.resources = Some(target.clone());
        Some(target)
    }

    pub(super) fn validate_migration(&self) -> Result<(), JsValue> {
        let Some(migration) = &self.migration else {
            return Ok(());
        };
        let other = &migration.inactive;
        let (active, inactive) = if migration.switched() {
            (migration.destination, migration.source)
        } else {
            (migration.source, migration.destination)
        };
        if active.root == inactive.root
            || self.guest.is_some()
            || other.guest.is_some()
            || other.migration.is_some()
            || self.key != other.key
            || self.invitation()?.anchor != active
            || other.invitation()?.anchor != inactive
            || (!migration.switched() && other.complete_invitation.is_none())
        {
            return Err(error("invalid saved browser migration"));
        }
        other.verify()?;
        Ok(())
    }

    pub(super) fn migration_progress(&self) -> Result<Option<serde_json::Value>, JsValue> {
        let Some(migration) = &self.migration else {
            return Ok(None);
        };
        let (phase, admission, source_exit) = if let Some(verified) = &migration.verified {
            let exit = migration.inactive.migration_source_exit()?;
            let finished = matches!(
                exit,
                SourceExit::Delivered { .. } | SourceExit::MembershipAlreadyEnded
            );
            (
                if finished { "SourceExited" } else { "Switched" },
                Some(verified.admission),
                exit,
            )
        } else {
            let target = migration.inactive.as_ref();
            let phase = match target.stage() {
                "Complete" => "MaterialsSaved",
                "PendingMaterials" => "PendingMaterials",
                _ => "Preparing",
            };
            (phase, target.admission_attempt(), SourceExit::NotStarted)
        };
        Ok(Some(serde_json::json!({
            "source": migration.source, "destination": migration.destination,
            "phase": phase, "admission": admission.map(|attempt| attempt.public_json()),
            "switched": migration.switched(), "source_exit": source_exit,
        })))
    }

    pub(super) fn require_migration_settled_for_replacement(&self) -> Result<(), JsValue> {
        let Some(migration) = &self.migration else {
            return Ok(());
        };
        let in_progress = if migration.switched() {
            matches!(
                migration.inactive.migration_source_exit()?,
                SourceExit::NotStarted | SourceExit::PendingDelivery { .. }
            )
        } else {
            !migration.preparation_ended()
        };
        if in_progress {
            return Err(error(
                "finish the pending network migration before replacing this device",
            ));
        }
        Ok(())
    }

    fn migration_source_exit(&self) -> Result<SourceExit, JsValue> {
        let auth = self.verify()?.ok_or_else(rejected)?;
        let local = self.identity();
        for entry in self
            .operations
            .entries
            .iter()
            .rev()
            .filter(|entry| entry.is_local_leave(local))
        {
            if let Some(approval) = &entry.approval
                && approval.effect_current(&auth)
            {
                let record = approval.record_id();
                return Ok(match entry.departure_recipient() {
                    Some(recipient) => SourceExit::Delivered { record, recipient },
                    None => SourceExit::PendingDelivery { record },
                });
            }
        }
        Ok(
            match auth.nodes().into_iter().find(|(id, _)| *id == local) {
                Some((_, role)) if role == Access::manage() => {
                    SourceExit::GenesisConfirmationRequired
                }
                Some(_) => SourceExit::NotStarted,
                None => SourceExit::MembershipAlreadyEnded,
            },
        )
    }

    fn sign_migration_departure(&mut self, signer: &SigningKey) -> Result<(), JsValue> {
        let anchor = self.invitation()?.anchor;
        let auth = self.verify()?.ok_or_else(rejected)?;
        let local = self.identity();
        let existing = self.operations.entries.iter().rev().find(|entry| {
            entry.is_local_leave(local) && entry.operation.check_current(anchor, &auth).is_ok()
        });
        let operation = match existing {
            Some(entry) => entry.operation.clone(),
            None => DeviceOperation::propose(anchor, &auth, local, DeviceAction::Leave, local)
                .map_err(error)?,
        };
        let approval = operation.approve(anchor, &auth, signer).map_err(error)?;
        self.operations.retain(operation).map_err(error)?;
        *self = self.approved_operation(approval.clone(), self.approval_history(&approval))?;
        Ok(())
    }
}

/// ConnectOptions carries this local capability. It is never offered to any
/// incoming connection and cannot authorize a different peer or protocol.
pub(super) struct WireScope {
    peer: iroh_base::EndpointId,
    alpn: &'static [u8],
    pub(super) gate: BrowserGate,
}

impl WireScope {
    pub(super) fn matches(&self, peer: iroh_base::EndpointId, alpn: &[u8]) -> bool {
        self.peer == peer && self.alpn == alpn
    }
}

#[wasm_bindgen]
impl BrowserNetwork {
    /// Confirm this device's participation using a trusted new-network invitation.
    pub async fn begin_network_migration(&self, invitation: String) -> Result<String, JsValue> {
        let invitation = ClientInvitation::decode(invitation.as_bytes()).map_err(error)?;
        let progress = self.gate.update_saved(|active| {
            let source = active.invitation()?.anchor;
            let destination = invitation.anchor().map_err(error)?;
            if active.stage() != "Complete"
                || active.guest.is_some()
                || active
                    .migration
                    .as_ref()
                    .is_some_and(|migration| !migration.preparation_ended())
            {
                return Err(error("this device cannot start a new migration"));
            }
            if source.root == destination.root
                || invitation.contact().as_bytes() == active.identity().as_bytes()
            {
                return Err(error(
                    "migration requires another network and an independent contact",
                ));
            }
            let resources = active
                .resources
                .as_ref()
                .ok_or_else(rejected)?
                .migration_target(active.complete_invitation.as_ref(), &invitation)?;
            let mut target = SavedNode::fresh(active.key);
            target.device_names = active
                .device_names
                .for_members(&[(active.identity(), Access::read())]);
            target.resources = Some(resources);
            target.select_invitation(invitation)?;
            target.reset_admission_progress();
            active.migration = Some(SavedMigration {
                source,
                destination,
                inactive: Box::new(target),
                verified: None,
                receipt_pending: false,
            });
            active.migration_progress()?.ok_or_else(rejected)
        })
        .await?;
        self.migration.wake.notify_one();
        Ok(progress.to_string())
    }

    /// Read the current verified progress independently of the active network.
    pub fn migration_progress(&self) -> Result<String, JsValue> {
        Ok(self
            .gate
            .service_snapshot()?
            .migration_progress()?
            .unwrap_or(serde_json::Value::Null)
            .to_string())
    }

    /// Explicitly use the old genesis credential for this device's departure.
    pub async fn confirm_migration_source_exit(
        &self,
        recovery_card: Option<String>,
        password: Option<String>,
    ) -> Result<String, JsValue> {
        let card = recovery_card
            .as_deref()
            .map(|card| {
                crate::recovery_card::RecoveryCard::import(card, password.as_deref()).map_err(error)
            })
            .transpose()?;
        let progress = self.gate.update_saved(|active| {
            let migration = active
                .migration
                .as_mut()
                .filter(|item| item.switched())
                .ok_or_else(rejected)?;
            let source = &mut migration.inactive;
            if !matches!(
                source.migration_source_exit()?,
                SourceExit::GenesisConfirmationRequired
            ) {
                return Err(error(
                    "the old network does not need genesis confirmation for departure",
                ));
            }
            let seed = if let Some(card) = card {
                if card.anchor() != migration.source {
                    return Err(error("card belongs to another network"));
                }
                card.credential().0
            } else {
                source.creator_seed.ok_or_else(|| {
                    error("this node does not hold the old network genesis credential")
                })?
            };
            source.sign_migration_departure(&SigningKey::from_bytes(&seed))?;
            active.migration_progress()?.ok_or_else(rejected)
        })
        .await?;
        self.migration.wake.notify_one();
        Ok(progress.to_string())
    }
}

#[derive(Default)]
pub(super) struct Runtime {
    wake: Rc<Notify>,
    cancel: CancellationToken,
    joined: Option<tokio::sync::oneshot::Receiver<()>>,
}

impl Runtime {
    pub(super) async fn stop(&mut self) {
        self.cancel.cancel();
        if let Some(joined) = self.joined.take() {
            let _ = joined.await;
        }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl BrowserNetwork {
    pub(super) fn start_migration_worker(&mut self) {
        let (finished, joined) = tokio::sync::oneshot::channel();
        self.migration.joined = Some(joined);
        let worker = Worker {
            gate: self.gate.clone(),
            endpoint: self.endpoint.clone(),
            relay_map: self.relay_map.clone(),
            budget: self.caller_budget,
            cancel: self.migration.cancel.clone(),
            wake: self.migration.wake.clone(),
            receipt_ready: self.admission.receipt_ready.clone(),
            files: self.files.clone(),
        };
        wasm_bindgen_futures::spawn_local(async move {
            worker.run().await;
            let _ = finished.send(());
        });
    }
}

struct Worker {
    gate: BrowserGate,
    endpoint: RestrictedEndpoint,
    relay_map: iroh::RelayMap,
    budget: Duration,
    cancel: CancellationToken,
    wake: Rc<Notify>,
    receipt_ready: Rc<Notify>,
    files: Rc<files::BrowserFiles>,
}

struct ScopedDial {
    address: EndpointAddr,
    preferred: Option<EndpointAddr>,
    scope: Arc<WireScope>,
}

impl Worker {
    async fn run(self) {
        let mut last_peer = None;
        while !self.cancel.is_cancelled() {
            // Only network awaits are cancellable. A write already started
            // must finish installing its snapshot before explicit close.
            let step = self.step(&mut last_peer).await;
            self.gate.0.lock().unwrap().migration_failure =
                step.as_ref().err().and_then(JsValue::as_string);
            let wait = async {
                if step.unwrap_or(true) {
                    sleep(RETRY).await;
                } else {
                    self.wake.notified().await;
                }
            };
            tokio::select! {
                biased;
                _ = self.cancel.cancelled() => break,
                _ = wait => {},
            }
        }
    }

    async fn step(&self, last_peer: &mut Option<iroh_base::EndpointId>) -> Result<bool, JsValue> {
        let (saved, version) = self.gate.snapshot().await?;
        let lease = version.lease;
        let Some(migration) = saved.migration else {
            return Ok(false);
        };
        let other = &migration.inactive;
        if migration.switched() {
            match other.migration_source_exit()? {
                SourceExit::NotStarted => {
                    self.update_inactive(|source| {
                        source.sign_migration_departure(&SigningKey::from_bytes(&source.key))
                    })
                    .await?;
                }
                SourceExit::PendingDelivery { .. } => {
                    self.deliver_departure(other, lease, last_peer).await?
                }
                _ => return Ok(false),
            }
        } else {
            match other.stage() {
                "Rejected" | "Removed" => return Ok(false),
                "Complete" => {
                    if migration.receipt_pending && self.retry_receipt(other, lease).await.is_ok() {
                        self.mark_receipt_done().await?;
                    }
                    if self.validate_target(other, lease, last_peer).await? {
                        *last_peer = None;
                    }
                }
                _ => self.fetch_admission(other, lease).await?,
            }
        }
        Ok(true)
    }

    async fn network_until<T>(
        &self,
        deadline: Instant,
        work: impl Future<Output = Result<T, JsValue>>,
    ) -> Result<T, JsValue> {
        tokio::select! {
            biased;
            _ = self.cancel.cancelled() => Err(error("browser node disconnected")),
            result = timeout(deadline.saturating_duration_since(Instant::now()), work) => result.map_err(|_| admission_timeout())?,
        }
    }

    async fn update_inactive<T>(
        &self,
        edit: impl FnOnce(&mut SavedNode) -> Result<T, JsValue>,
    ) -> Result<T, JsValue> {
        self.gate.update_saved(|saved| {
            edit(&mut saved.migration.as_mut().ok_or_else(rejected)?.inactive)
        })
        .await
    }

    fn scoped_dial(
        &self,
        saved: &SavedNode,
        lease: u32,
        peer: iroh_base::EndpointId,
        alpn: &'static [u8],
    ) -> Result<ScopedDial, JsValue> {
        let gate = BrowserGate(Default::default(), Default::default(), self.gate.2.clone());
        {
            let active = self.gate.0.lock().unwrap();
            let mut state = gate.0.lock().unwrap();
            state.lease = Some(lease);
            state.migration_owner = Some(Arc::downgrade(&self.gate.0));
            state.rtc_capability = active.rtc_capability.clone();
            state.member_data = active.member_data.clone();
            #[cfg(feature = "rtc-probe")]
            {
                state.rtc = active.rtc.clone();
            }
        }
        gate.install(saved);
        let address = saved.address_for(peer)?;
        let preferred = saved
            .complete_invitation
            .as_ref()
            .filter(|value| value.contact() == peer)
            .and_then(|invitation| {
                let candidate = EndpointAddr::from_parts(
                    peer,
                    address
                        .addrs
                        .intersection(&invitation.address().addrs)
                        .cloned(),
                );
                (!candidate.addrs.is_empty() && candidate.addrs != address.addrs)
                    .then_some(candidate)
            });
        Ok(ScopedDial {
            address,
            preferred,
            scope: Arc::new(WireScope { peer, alpn, gate }),
        })
    }

    fn dial_options(
        &self,
        address: &EndpointAddr,
        deadline: Instant,
        scope: Arc<WireScope>,
    ) -> iroh::endpoint::ConnectOptions {
        let mut options = iroh::endpoint::ConnectOptions::new()
            .with_caller_deadline(deadline)
            .with_attempt_cancellation(self.cancel.clone())
            .with_application_context(scope);
        if address.addrs.len() == 1 {
            options = options.with_bootstrap_route(address.addrs.first().unwrap().clone());
        }
        options
    }

    async fn connect_scoped<T, F>(
        &self,
        dial: ScopedDial,
        deadline: Instant,
        connect: impl Fn(EndpointAddr, iroh::endpoint::ConnectOptions) -> F,
    ) -> Result<T, JsValue>
    where
        F: Future<Output = Result<T, JsValue>>,
    {
        if let Some(preferred) = dial.preferred {
            let now = Instant::now();
            let first_deadline = now + deadline.saturating_duration_since(now) / 2;
            let options = self.dial_options(&preferred, first_deadline, dial.scope.clone());
            if let Ok(connection) = connect(preferred, options).await {
                return Ok(connection);
            }
        }
        let options = self.dial_options(&dial.address, deadline, dial.scope);
        connect(dial.address, options).await
    }

    async fn prepare_relays(&self, saved: &SavedNode) -> Result<(), JsValue> {
        for url in saved
            .resources
            .as_ref()
            .ok_or_else(rejected)?
            .allowed_relays()?
        {
            let url_origin = origin(url.as_str())?;
            let registered = self
                .relay_map
                .urls::<Vec<_>>()
                .into_iter()
                .any(|relay| origin(relay.as_str()).is_ok_and(|other| other == url_origin));
            if !registered {
                let config = Arc::new(iroh::RelayConfig::from(url.clone()));
                self.relay_map.insert(url.clone(), config.clone());
                self.endpoint.insert_relay(url, config).await;
            }
        }
        Ok(())
    }

    async fn control(
        &self,
        saved: &SavedNode,
        lease: u32,
        peer: iroh_base::EndpointId,
        alpn: &'static [u8],
        deadline: Instant,
    ) -> Result<ControlOwner, JsValue> {
        self.prepare_relays(saved).await?;
        let dial = self.scoped_dial(saved, lease, peer, alpn)?;
        let connection = self
            .connect_scoped(dial, deadline, |address, options| async move {
                self.endpoint
                    .connect_control_with_opts(address, alpn, options)
                    .await
                    .map_err(error)?
                    .await
                    .map_err(error)
            })
            .await?;
        Ok(Arc::new(connection).retain_control())
    }

    async fn fetch_admission(&self, saved: &SavedNode, lease: u32) -> Result<(), JsValue> {
        let deadline = Instant::now() + self.budget;
        let peer = saved.invitation()?.contact;
        let connection = self
            .control(saved, lease, peer, saved.request_alpn(), deadline)
            .await?;
        let _close = RequestConnection(Some(connection.clone()));
        let (exchange, status) = self
            .network_until(deadline, async {
                let exchange = connection.open_control_exchange().await.map_err(error)?;
                let status = saved.write_admission_request(&exchange).await?;
                Ok((exchange, status))
            })
            .await?;
        if matches!(status, 0 | 1) {
            self.update_inactive(|target| {
                target.record_admission_progress(ApplicationProgress::Delivered);
                Ok(())
            })
            .await?;
        }
        if status == 2 && saved.precode.is_none() {
            return self
                .update_inactive(|target| {
                    target.manual_rejected = true;
                    target.record_admission_progress(ApplicationProgress::Rejected);
                    Ok(())
                })
                .await;
        }
        if status == 0 {
            return self
                .network_until(deadline, async {
                    exchange.finish_send().await.map_err(error)
                })
                .await;
        }
        if status != 1 {
            return Err(rejected());
        }
        let authorized = self
            .network_until(deadline, saved.authorized_admission(&exchange))
            .await?;
        self.update_inactive(|target| {
            *target = authorized.clone();
            Ok(())
        })
        .await?;
        let material = self
            .network_until(deadline, async {
                send_fixed(&exchange, b"AUTH").await?;
                recv_fixed::<40>(&exchange).await
            })
            .await?;
        let mut complete = authorized.with_admission_secret(material)?;
        complete = complete.declare_files(false)?;
        let clipboard = clipboard_host::capability().await;
        complete = complete.declare_clipboard(clipboard.read && clipboard.write, false)?;
        self.gate.update_saved(|active| {
            let migration = active.migration.as_mut().ok_or_else(rejected)?;
            *migration.inactive = complete;
            migration.receipt_pending = true;
            Ok(())
        })
        .await?;
        if self
            .network_until(deadline, async {
                send_fixed(&exchange, b"SAVE").await?;
                exchange.finish_send().await.map_err(error)?;
                if &recv_fixed::<4>(&exchange).await? == b"DONE" {
                    Ok(())
                } else {
                    Err(rejected())
                }
            })
            .await
            .is_ok()
        {
            self.mark_receipt_done().await?;
        }
        Ok(())
    }

    async fn mark_receipt_done(&self) -> Result<(), JsValue> {
        self.gate.update_saved(|node| {
            node.migration
                .as_mut()
                .ok_or_else(rejected)?
                .receipt_pending = false;
            Ok(())
        })
        .await
    }

    async fn retry_receipt(&self, target: &SavedNode, lease: u32) -> Result<(), JsValue> {
        let receipt = target.admission_receipt()?;
        let peer = target.invitation()?.contact;
        if let Some((scope, secret)) = target.receipt_material(receipt, peer)? {
            let deadline = Instant::now() + self.budget;
            let connection = self
                .control(target, lease, peer, ADMISSION_RECEIPT_ALPN, deadline)
                .await?;
            let _close = RequestConnection(Some(connection.clone()));
            admission::submit_receipt(&connection, receipt, &scope, &secret, deadline).await?;
        }
        Ok(())
    }

    async fn validate_target(
        &self,
        target: &SavedNode,
        lease: u32,
        last_peer: &mut Option<iroh_base::EndpointId>,
    ) -> Result<bool, JsValue> {
        let auth = target.verify()?.ok_or_else(rejected)?;
        let peer = candidate(target, &auth, last_peer)?;
        let capture =
            MemberSyncMembership::capture(&auth, target.identity(), peer).ok_or_else(rejected)?;
        let deadline = Instant::now() + self.budget;
        self.prepare_relays(target).await?;
        let dial = self.scoped_dial(target, lease, peer, POLICY_ALPN)?;
        let connection = self
            .connect_scoped(dial, deadline, |address, options| async move {
                self.endpoint
                    .connect_with_opts(address, POLICY_ALPN, options)
                    .await
                    .map_err(error)?
                    .await
                    .map_err(error)
            })
            .await?;
        let _close = RequestConnection(Some(connection.clone()));
        let outbound = target.member_sync(&auth.nodes(), MemberSyncMode::Management);
        let incoming = self
            .network_until(deadline, async {
                outbound.exchange(&connection).await.map_err(error)
            })
            .await?;
        self.apply_validation(target, peer, capture, incoming).await
    }

    async fn apply_validation(
        &self,
        tested: &SavedNode,
        peer: iroh_base::EndpointId,
        capture: MemberSyncMembership,
        incoming: MemberSync,
    ) -> Result<bool, JsValue> {
        let (mut active, mut write) = self.gate.write_node().await?;
        let mut migration = active
            .migration
            .take()
            .filter(|item| !item.switched())
            .ok_or_else(rejected)?;
        if serde_json::to_string(tested).map_err(error)?
            != serde_json::to_string(&migration.inactive).map_err(error)?
        {
            return Err(error(
                "browser migration destination changed during validation",
            ));
        }
        let (mut target, _, policy_error) = migration.inactive.with_member_sync(incoming)?;
        let current = target.verify()?.ok_or_else(rejected)?;
        if policy_error.is_some()
            || !capture.is_current(&current, target.identity())
            || target.stage() != "Complete"
        {
            // Preserve newly validated history even if this connection used a
            // membership grant that is no longer current in that history.
            migration.inactive = Box::new(target);
            active.migration = Some(migration);
            write.commit(&active).await?;
            return Ok(false);
        }
        target.device_names.merge_members(
            &[(target.identity(), Access::read())],
            active
                .device_names
                .for_members(&[(active.identity(), Access::read())]),
        );
        migration.inactive = Box::new(active);
        migration.verified = Some(VerifiedTarget {
            peer,
            admission: target.admission_attempt().ok_or_else(rejected)?,
        });
        let pending = if migration.receipt_pending {
            Some(target.pending_admission_receipt()?)
        } else {
            None
        };
        migration.receipt_pending = false;
        target.migration = Some(migration);
        write.switch(
            &target,
            pending.as_deref(),
            &self.files,
        )
        .await?;
        if pending.is_some() {
            self.receipt_ready.notify_one();
        }
        Ok(true)
    }

    async fn deliver_departure(
        &self,
        saved: &SavedNode,
        lease: u32,
        last_peer: &mut Option<iroh_base::EndpointId>,
    ) -> Result<(), JsValue> {
        let auth = saved.verify()?.ok_or_else(rejected)?;
        let entry = saved
            .operations
            .pending_departure(saved.identity(), &auth)
            .ok_or_else(rejected)?;
        let delivery = DepartureDelivery::new(entry).map_err(error)?;
        let peer = candidate(saved, &auth, last_peer)?;
        let deadline = Instant::now() + self.budget;
        let connection = self
            .control(saved, lease, peer, DEPARTURE_ALPN, deadline)
            .await?;
        let _close = RequestConnection(Some(connection.clone()));
        let record = self
            .network_until(deadline, async {
                delivery.send(&connection).await.map_err(error)
            })
            .await?;
        self.update_inactive(|source| {
            let recipient = VerifyingKey::from_bytes(peer.as_bytes()).map_err(error)?;
            source
                .operations
                .confirm_departure_delivery(
                    source.identity(),
                    delivery.approval.operation.id,
                    record,
                    recipient,
                )
                .map_err(error)
        })
        .await
    }
}

fn candidate(
    saved: &SavedNode,
    auth: &ManagementAuth,
    last: &mut Option<iroh_base::EndpointId>,
) -> Result<iroh_base::EndpointId, JsValue> {
    let local = saved.identity();
    let peers = auth
        .nodes()
        .into_iter()
        .filter(|(id, _)| *id != local)
        .filter_map(|(id, _)| iroh_base::EndpointId::from_bytes(id.as_bytes()).ok());
    let peer = next_member_sync_peer(peers, *last).ok_or_else(rejected)?;
    *last = Some(peer);
    Ok(peer)
}
