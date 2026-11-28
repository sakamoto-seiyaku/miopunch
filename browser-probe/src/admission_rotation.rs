//! IndexedDB adapter for the shared admission-secret rotation protocol.
use super::*;
use crate::admission_rotation_wire::{
    ADMISSION_ROTATION_ALPN, COMMIT_DOMAIN, Completion, HistoryFrame, MAX_MEMBERS, PROPOSAL_DOMAIN,
    Proposal, ROTATION_DELIVERY_ALPN, RotationExchange, RotationProgress, RotationStage,
    SavedRotation, Signed, decode, encode, retained_members, rotation_deadline, verify_ack, verify_completion,
    verify_proposal, wire_io,
};
use crate::node_control_backup::{NativeRotationState, RetainedRotation, RetainedRotationMaterial};
use crate::recovery_card::RecoveryCard;
use iroh_base::EndpointId;
use n0_future::time::{Instant, sleep};
use p2panda_core::{Signature, SigningKey};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use tokio::sync::Notify;

const CONFIRMATION_WINDOW: Duration = Duration::from_secs(60);

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct RotationState {
    proposal: Signed,
    completion: Option<Signed>,
    staged: Option<[u8; 32]>,
    acknowledgements: BTreeMap<VerifyingKey, Signed>,
    predecessor: Option<(Signed, Signed)>,
}

impl RotationState {
    fn verify(&self, anchor: NetworkAnchor) -> Result<SavedRotation, JsValue> {
        SavedRotation::new(
            self.proposal.clone(),
            verify_proposal(anchor, &self.proposal).map_err(error)?,
            self.completion.clone(),
        )
        .map_err(error)
    }

    fn new(proposal: Signed, predecessor: Option<(Signed, Signed)>) -> Self {
        Self {
            proposal,
            completion: None,
            staged: None,
            acknowledgements: BTreeMap::new(),
            predecessor,
        }
    }
    pub(super) fn from_portable(
        ledger: &NativeRotationState,
        anchor: NetworkAnchor,
        generation: u64,
    ) -> Result<Option<Self>, JsValue> {
        let Some(row) = ledger.rotations.iter().max_by_key(|row| row.generation) else {
            return Ok(None);
        };
        let (signed, completion) = portable_decision(row, anchor)?;
        if row.generation > generation {
            return Err(rejected());
        }
        let previous = ledger
            .rotations
            .iter()
            .find(|prior| prior.generation.checked_add(1) == Some(row.generation))
            .map(|prior| portable_decision(prior, anchor))
            .transpose()?;
        let mut state = Self::new(signed, previous);
        state.completion = Some(completion);
        Ok(Some(state))
    }
}

fn portable_decision(
    row: &RetainedRotation,
    anchor: NetworkAnchor,
) -> Result<(Signed, Signed), JsValue> {
    let signed = decode(&row.proposal).map_err(error)?;
    let proposal = verify_proposal(anchor, &signed).map_err(error)?;
    let completion = decode(&row.completion).map_err(error)?;
    if proposal.generation != row.generation {
        return Err(rejected());
    }
    let rotation = SavedRotation::new(signed, proposal, None).map_err(error)?;
    verify_completion(&rotation, anchor.root, &completion).map_err(error)?;
    Ok((rotation.signed, completion))
}

impl SavedNode {
    pub(super) fn admissions_open(&self) -> Result<(), JsValue> {
        if self
            .rotation
            .as_ref()
            .is_some_and(|state| state.completion.is_none())
        {
            return Err(error(
                "admission and approved-material delivery are paused for secret rotation",
            ));
        }
        Ok(())
    }

    pub(super) fn portable_rotation(&self) -> Result<NativeRotationState, JsValue> {
        self.admissions_open()?;
        let mut ledger = self.native_rotation.clone();
        let Some(state) = &self.rotation else {
            return Ok(ledger);
        };
        let anchor = self.invitation()?.anchor;
        let rotation = state.verify(anchor)?;
        if rotation.proposal.generation == self.generation && self.secret.is_none() {
            return Ok(ledger);
        }
        let decisions = state
            .predecessor
            .iter()
            .map(|(proposal, completion)| (proposal, completion))
            .chain(
                state
                    .completion
                    .iter()
                    .map(|completion| (&state.proposal, completion)),
            );
        for (signed, completion) in decisions {
            let generation = verify_proposal(anchor, signed).map_err(error)?.generation;
            ledger.rotations.retain(|row| row.generation != generation);
            ledger.rotations.push(RetainedRotation {
                generation,
                proposal: encode(signed).map_err(error)?,
                completion: encode(completion).map_err(error)?,
            });
        }
        if let Some(secret) = self.secret
            && rotation.proposal.generation == self.generation
        {
            // The Native restore path rechecks staged material against the
            // signed fingerprint when opening the completed generation.
            ledger
                .materials
                .retain(|row| row.generation != self.generation);
            ledger.materials.push(RetainedRotationMaterial {
                generation: self.generation,
                bytes: secret.to_vec(),
            });
        }
        Ok(ledger)
    }

    fn retained(&self, auth: &ManagementAuth) -> Result<Vec<[u8; 32]>, JsValue> {
        let root = self.invitation()?.anchor.root;
        Ok(retained_members(auth, root))
    }

    fn local_membership(&self, auth: &ManagementAuth) -> bool {
        self.authorization
            .is_some_and(|id| auth.membership_authorization_valid(id, self.identity()))
    }

    fn rotation_auth(&self) -> Result<ManagementAuth, JsValue> {
        let auth = self.verify()?.ok_or_else(rejected)?;
        if !self.local_membership(&auth) {
            return Err(rejected());
        }
        Ok(auth)
    }

    pub(super) fn rotation_member_pair(&self, peer: EndpointId) -> bool {
        self.rotation_auth().is_ok_and(|auth| {
            auth.nodes()
                .into_iter()
                .any(|(id, _)| id.as_bytes() == peer.as_bytes())
        })
    }

    pub(super) fn rotation_credential(
        &self,
        generation: u64,
    ) -> Option<(AdmissionScope, AdmissionSecret)> {
        let anchor = self.invitation().ok()?.anchor;
        let secret = if self.generation == generation {
            self.secret?
        } else {
            let state = self.rotation.as_ref()?;
            let rotation = state.verify(anchor).ok()?;
            if state.completion.is_some()
                || generation <= self.generation
                || rotation.proposal.generation != generation
            {
                return None;
            }
            state.staged?
        };
        Some((
            AdmissionScope::new(*anchor.root.as_bytes(), generation),
            AdmissionSecret::from_saved(secret),
        ))
    }

    fn pending_rotation_profile(&self, peer: VerifyingKey) -> Option<u8> {
        let auth = self.rotation_auth().ok()?;
        self.pending_profile_for(&auth, peer)
    }

    fn pending_profile_for(&self, auth: &ManagementAuth, peer: VerifyingKey) -> Option<u8> {
        if self.approvals.iter().any(|approval| {
            approval.applicant == peer
                && !approval.complete
                && approval
                    .authorization
                    .is_some_and(|grant| auth.membership_authorization_valid(grant, peer))
        }) {
            return Some(1);
        }
        self.served_recoveries
            .iter()
            .any(|recovery| {
                recovery.target == peer
                    && recovery.phase < 3
                    && recovery
                        .authorization
                        .is_some_and(|grant| auth.administrator_authorization_valid(grant, peer))
            })
            .then_some(2)
    }

    fn is_original_provider(&self, profile: u8, peer: EndpointId) -> bool {
        match profile {
            1 => {
                self.creator_seed.is_none()
                    && self
                        .invitation()
                        .is_ok_and(|invitation| invitation.contact == peer)
            }
            2 => self
                .recovery
                .as_ref()
                .is_some_and(|intent| intent.contact == peer),
            _ => false,
        }
    }

    pub(super) fn rotation_contact(&self, peer: EndpointId, alpn: &[u8], client: bool) -> bool {
        if self.replacement_source.is_some()
            || self.restored.as_ref().is_some_and(|r| r.receipt.is_none())
            || self.creator_seed.is_some() && !self.recovery_saved
        {
            return false;
        }
        if alpn == ADMISSION_ROTATION_ALPN {
            self.rotation_member_pair(peer)
        } else if client {
            VerifyingKey::from_bytes(peer.as_bytes())
                .is_ok_and(|peer| self.pending_rotation_profile(peer).is_some())
        } else {
            self.is_original_provider(1, peer) || self.is_original_provider(2, peer)
        }
    }

    fn adopt_rotation_grant(&mut self) -> Result<(), JsValue> {
        if self.authorization.is_none() && !self.manual_retry_stopped() && self.guest.is_none() {
            let auth = self.verify()?.ok_or_else(rejected)?;
            let target = self.identity();
            let grant = auth
                .authorization_origin_at(target, &auth.tips(), false)
                .filter(|grant| auth.membership_authorization_valid(*grant, target))
                .ok_or_else(rejected)?;
            // This signed grant proves local membership. The admission wire
            // continues to track the individual request and its receipt.
            self.authorization = Some(grant);
        }
        Ok(())
    }

    fn current_rotation(&self) -> Result<Option<SavedRotation>, JsValue> {
        self.rotation
            .as_ref()
            .map(|state| state.verify(self.invitation()?.anchor))
            .transpose()
    }

    pub(super) fn validate_rotation(&self) -> Result<(), JsValue> {
        if let Some(rotation) = self.current_rotation()? {
            let state = self.rotation.as_ref().unwrap();
            if let Some(staged) = state.staged {
                let scope =
                    AdmissionScope::new(rotation.proposal.network, rotation.proposal.generation);
                if AdmissionSecret::from_saved(staged).fingerprint(&scope)
                    != rotation.proposal.fingerprint
                {
                    return Err(rejected());
                }
            }
            self.rotation_progress()?;
        }
        Ok(())
    }

    pub(super) fn rotation_progress(&self) -> Result<Option<RotationProgress>, JsValue> {
        let Some(rotation) = self.current_rotation()? else {
            return Ok(None);
        };
        let anchor = self.invitation()?.anchor;
        if let Some(progress) = rotation.completed_progress(anchor.root).map_err(error)? {
            return Ok(Some(progress));
        }
        let retained = self.retained(&self.verify()?.ok_or_else(rejected)?)?;
        let mut acknowledged = Vec::new();
        for (key, signed) in &self.rotation.as_ref().unwrap().acknowledgements {
            let peer = verify_ack(signed, &rotation).map_err(error)?;
            if key.as_bytes() != &peer { return Err(rejected()); }
            acknowledged.push(peer);
        }
        Ok(Some(rotation.progress(&retained, acknowledged).map_err(error)?))
    }

    fn genesis_key(&self, card: Option<&RecoveryCard>) -> Result<SigningKey, JsValue> {
        let anchor = self.invitation()?.anchor;
        let seed = match card {
            Some(card) if card.anchor() == anchor => card.credential().0,
            Some(_) => return Err(error("card belongs to another network")),
            None => self
                .creator_seed
                .ok_or_else(|| error("this node does not hold the genesis credential"))?,
        };
        let root = SigningKey::from_bytes(&seed);
        if root.verifying_key() != anchor.root {
            return Err(rejected());
        }
        Ok(root)
    }

    fn begin_rotation(&mut self, root: &SigningKey) -> Result<RotationProgress, JsValue> {
        let auth = self.rotation_auth()?;
        if self.stage() != "Complete" {
            return Err(rejected());
        }
        let predecessor = if let Some(existing) = self.current_rotation()? {
            if let Some(completion) = existing.completion {
                (existing.proposal.generation == self.generation)
                    .then_some((existing.signed, completion))
            } else {
                return Err(error("confirm the pending rotation by its ID"));
            }
        } else {
            None
        };
        let anchor = self.invitation()?.anchor;
        let generation = self
            .generation
            .checked_add(1)
            .filter(|generation| *generation <= i64::MAX as u64)
            .ok_or_else(rejected)?;
        let scope = AdmissionScope::new(*anchor.root.as_bytes(), generation);
        let secret = AdmissionSecret::generate();
        let proposal = Proposal {
            network: *anchor.root.as_bytes(),
            genesis: *anchor.genesis.as_bytes(),
            id: rand::random(),
            initiator: *self.identity().as_bytes(),
            previous_generation: self.generation,
            generation,
            retained: self.retained(&auth)?,
            fingerprint: secret.fingerprint(&scope),
        };
        let signed = Signed::sign(PROPOSAL_DOMAIN, &proposal, root).map_err(error)?;
        self.rotation = Some(RotationState::new(signed, predecessor));
        self.save_rotation_material(*secret.saved_bytes())?;
        self.rotation_progress()?.ok_or_else(rejected)
    }

    fn save_rotation_material(
        &mut self,
        bytes: [u8; 32],
    ) -> Result<(Signed, Vec<Signed>), JsValue> {
        let rotation = self.current_rotation()?.ok_or_else(rejected)?;
        let scope = AdmissionScope::new(rotation.proposal.network, rotation.proposal.generation);
        if rotation.completion.is_some()
            || AdmissionSecret::from_saved(bytes).fingerprint(&scope)
                != rotation.proposal.fingerprint
        {
            return Err(rejected());
        }
        let key = SigningKey::from_bytes(&self.key);
        let ack = rotation.sign_ack(&key).map_err(error)?;
        let retained = self.retained(&self.rotation_auth()?)?;
        let state = self.rotation.as_mut().unwrap();
        state.staged = Some(bytes);
        state
            .acknowledgements
            .insert(key.verifying_key(), ack.clone());
        let forwarded = state
            .acknowledgements
            .iter()
            .filter(|(id, _)| retained.contains(id.as_bytes()))
            .map(|(_, ack)| ack.clone())
            .collect();
        Ok((ack, forwarded))
    }

    fn complete_rotation(
        &mut self,
        rotation: &SavedRotation,
        signed: Signed,
    ) -> Result<(), JsValue> {
        let anchor = self.invitation()?.anchor;
        let completion = verify_completion(rotation, anchor.root, &signed).map_err(error)?;
        let identity = self.identity();
        if !completion.retained.contains(identity.as_bytes()) {
            return Err(rejected());
        }
        let state = self.rotation.as_mut().ok_or_else(rejected)?;
        if state
            .completion
            .as_ref()
            .is_some_and(|previous| previous != &signed)
        {
            return Err(rejected());
        }
        let activated = self.generation < rotation.proposal.generation;
        if activated {
            self.secret = Some(state.staged.take().ok_or_else(rejected)?);
            self.generation = rotation.proposal.generation;
        }
        state.acknowledgements.clear();
        state.completion = Some(signed);
        if activated {
            if let Some(recovery) = &mut self.recovery {
                if !recovery.complete {
                    let proof = self
                        .served_recoveries
                        .iter_mut()
                        .find(|proof| {
                            proof.target == identity
                                && proof.attempt == recovery.attempt
                                && proof.authorization == self.authorization
                                && proof.authorization.is_some()
                        })
                        .ok_or_else(rejected)?;
                    proof.phase = 3;
                    recovery.complete = true;
                }
            }
        }
        Ok(())
    }
}

impl BrowserGate {
    fn check_rotation_contact(
        &self,
        saved: &SavedNode,
        connection: &ControlOwner,
        peer: VerifyingKey,
        profile: Option<u8>,
    ) -> Result<(), JsValue> {
        if !saved.rotation_member_pair(connection.remote_id()) {
            return Err(rejected());
        }
        match profile {
            Some(profile) if connection.side().is_client() => {
                if saved.pending_rotation_profile(peer) != Some(profile) {
                    return Err(rejected());
                }
            }
            Some(profile) => {
                if !saved.is_original_provider(profile, connection.remote_id()) {
                    return Err(rejected());
                }
            }
            None => {}
        }
        let gate = self.0.lock().unwrap();
        let stable = connection.stable_id();
        if !gate.control_sessions.get(&stable).is_some_and(|session| {
            session.alpn == connection.alpn()
                && session.peer == connection.remote_id()
                && session.permitted(saved)
        }) || connection.alpn() == ADMISSION_ROTATION_ALPN
            && !Self::proved_id_current(
                &gate,
                saved,
                stable,
                connection.remote_id(),
                connection.alpn(),
            )
        {
            return Err(rejected());
        }
        Ok(())
    }
}

fn is_same_rotation(saved: &SavedNode, expected: &SavedRotation) -> Result<(), JsValue> {
    let present = saved.current_rotation()?.ok_or_else(rejected)?;
    if present.signed != expected.signed || present.stage() != expected.stage() {
        return Err(rejected());
    }
    Ok(())
}

async fn receive_rotation(
    accepting: iroh::endpoint::RestrictedAccepting,
    deadline: Instant,
    gate: &BrowserGate,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<(), JsValue> {
    let connection = n0_future::time::timeout(
        deadline.saturating_duration_since(Instant::now()),
        accepting.with_caller_deadline(deadline).into_control(),
    )
    .await
    .map_err(error)?
    .map_err(error)?;
    let connection = Arc::new(connection).retain_control();
    let _connection = RequestConnection(Some(connection.clone()));
    let deadline = rotation_deadline(&connection);
    let exchange = RotationExchange {
        control: wire_io(deadline, connection.accept_control_exchange())
            .await
            .map_err(error)?,
        deadline,
    };
    let peer = VerifyingKey::from_bytes(connection.remote_id().as_bytes()).map_err(error)?;
    let profile = if connection.alpn() == ROTATION_DELIVERY_ALPN {
        let mut profile = [0; 1];
        exchange.read_fixed(&mut profile).await.map_err(error)?;
        let saved = gate.service_snapshot()?;
        if !saved.is_original_provider(profile[0], connection.remote_id()) {
            return Err(rejected());
        }
        if profile[0] == 2 {
            let anchor = saved.invitation()?.anchor;
            let proof = crate::management_recovery_wire::proof_statement(
                &connection,
                anchor,
                SecretKey::from_bytes(&saved.key).public(),
                connection.remote_id(),
            )
            .map_err(error)?;
            let signature = saved.genesis_key(None)?.sign(&proof).to_bytes();
            exchange.write_fixed(&signature).await.map_err(error)?;
        }
        Some(profile[0])
    } else {
        None
    };

    let (signed, completion, history, predecessor): HistoryFrame =
        decode(&exchange.read_frame().await.map_err(error)?).map_err(error)?;
    let pending = gate.update_saved(|saved| {
        let proposal = verify_proposal(saved.invitation()?.anchor, &signed).map_err(error)?;
        if !proposal.retained.contains(peer.as_bytes())
            || !proposal.retained.contains(saved.identity().as_bytes())
            || history.len() > MAX_MEMBERS
        {
            return Err(rejected());
        }
        *saved = saved.merge_records(history)?;
        if profile == Some(1) {
            saved.adopt_rotation_grant()?;
        }
        gate.check_rotation_contact(saved, &connection, peer, profile)?;

        let existing = saved.current_rotation()?;
        if saved.generation < proposal.previous_generation
            && let Some(previous) = &existing
            && previous.stage() == RotationStage::Distributing
            && previous.proposal.generation == proposal.previous_generation
        {
            let (precursor, completion) = predecessor.ok_or_else(rejected)?;
            if previous.signed != precursor {
                return Err(rejected());
            }
            saved.complete_rotation(previous, completion)?;
        }
        let current = saved.current_rotation()?;
        if let Some(completion) = completion {
            let rotation = current.ok_or_else(rejected)?;
            if profile.is_some() || rotation.signed != signed {
                return Err(rejected());
            }
            saved.complete_rotation(&rotation, completion)?;
            Ok(None)
        } else {
            if profile.is_none() && proposal.previous_generation != saved.generation
                || profile.is_some() && proposal.generation <= saved.generation
                || current.as_ref().is_some_and(|old| {
                    old.proposal.generation >= proposal.generation && old.signed != signed
                })
            {
                return Err(rejected());
            }
            let rotation = SavedRotation::new(signed, proposal, None).map_err(error)?;
            if current
                .as_ref()
                .is_none_or(|old| old.signed != rotation.signed)
            {
                let predecessor = current.and_then(|old| {
                    (old.proposal.generation == rotation.proposal.previous_generation)
                        .then_some(old.completion)
                        .flatten()
                        .map(|completion| (old.signed, completion))
                });
                saved.rotation = Some(RotationState::new(rotation.signed.clone(), predecessor));
            }
            Ok(Some(rotation))
        }
    })
    .await?;

    if let Some(rotation) = pending {
        let mut bytes = [0; 32];
        exchange.read_fixed(&mut bytes).await.map_err(error)?;
        let acks = gate.update_saved(|saved| {
            is_same_rotation(saved, &rotation)?;
            gate.check_rotation_contact(saved, &connection, peer, profile)?;
            saved.save_rotation_material(bytes)
        })
        .await?;
        // The same strict transaction saved our staged key and signed ACK.
        exchange
            .write_frame(&encode(&acks).map_err(error)?)
            .await
            .map_err(error)?;
    } else {
        exchange.write_fixed(b"DONE").await.map_err(error)?;
    }
    let mut saved = [0; 4];
    exchange.read_fixed(&mut saved).await.map_err(error)?;
    if &saved != b"SAVE" {
        return Err(rejected());
    }
    exchange.write_fixed(b"RCVD").await.map_err(error)?;
    exchange.finish_send().await.map_err(error)?;
    tokio::select! {
        _ = cancel.cancelled() => {}
        _ = connection.revocation_handle().closed() => {}
        _ = sleep(deadline.saturating_duration_since(Instant::now())) => {}
    }
    Ok(())
}

async fn send_rotation(
    gate: &BrowserGate,
    connection: &ControlOwner,
    expected: ([u8; 16], RotationStage),
) -> Result<(), JsValue> {
    let peer = VerifyingKey::from_bytes(connection.remote_id().as_bytes()).map_err(error)?;
    let saved = gate.installed().await?;
    let rotation = saved.current_rotation()?.ok_or_else(rejected)?;
    if (rotation.proposal.id, rotation.stage()) != expected
        || !rotation.proposal.retained.contains(peer.as_bytes())
        || !rotation
            .proposal
            .retained
            .contains(saved.identity().as_bytes())
        || saved.records.len() > MAX_MEMBERS
        || rotation.completion.is_some() && saved.generation != rotation.proposal.generation
    {
        return Err(rejected());
    }
    let profile = if connection.alpn() == ROTATION_DELIVERY_ALPN {
        if rotation.completion.is_some() {
            return Err(rejected());
        }
        Some(saved.pending_rotation_profile(peer).ok_or_else(rejected)?)
    } else {
        None
    };
    gate.check_rotation_contact(&saved, connection, peer, profile)?;
    let deadline = rotation_deadline(connection);
    let wire = RotationExchange {
        control: wire_io(deadline, connection.open_control_exchange())
            .await
            .map_err(error)?,
        deadline,
    };
    if let Some(profile) = profile {
        wire.write_fixed(&[profile]).await.map_err(error)?;
        if profile == 2 {
            let anchor = saved.invitation()?.anchor;
            let statement = crate::management_recovery_wire::proof_statement(
                connection,
                anchor,
                connection.remote_id(),
                SecretKey::from_bytes(&saved.key).public(),
            )
            .map_err(error)?;
            let mut signature = [0; 64];
            wire.read_fixed(&mut signature).await.map_err(error)?;
            if !anchor
                .root
                .verify(&statement, &Signature::from_bytes(&signature))
            {
                return Err(rejected());
            }
        }
    }
    // Recovery proof awaits the peer, so refresh authorization before history
    // or private material is released.
    let history = {
        let current = gate.installed().await?;
        is_same_rotation(&current, &rotation)?;
        gate.check_rotation_contact(&current, connection, peer, profile)?;
        let frame: HistoryFrame = (
            rotation.signed.clone(),
            rotation.completion.clone(),
            current.records.to_vec(),
            current.rotation.unwrap().predecessor,
        );
        encode(&frame).map_err(error)?
    };
    wire.write_frame(&history).await.map_err(error)?;
    if rotation.completion.is_none() {
        {
            let (current, _write) = gate.write_node().await?;
            is_same_rotation(&current, &rotation)?;
            gate.check_rotation_contact(&current, connection, peer, profile)?;
            let staged = current.rotation.unwrap().staged.ok_or_else(rejected)?;
            wire.write_fixed(&staged).await.map_err(error)?;
        }
        let (ack, forwarded): (Signed, Vec<Signed>) =
            decode(&wire.read_frame().await.map_err(error)?).map_err(error)?;
        gate.update_saved(|saved| {
            is_same_rotation(saved, &rotation)?;
            gate.check_rotation_contact(saved, connection, peer, profile)?;
            if verify_ack(&ack, &rotation).map_err(error)? != *peer.as_bytes()
                || forwarded.len() > MAX_MEMBERS
            {
                return Err(rejected());
            }
            let retained = saved.retained(&saved.rotation_auth()?)?;
            let state = saved.rotation.as_mut().unwrap();
            for ack in std::iter::once(ack).chain(forwarded) {
                let member = verify_ack(&ack, &rotation).map_err(error)?;
                if !retained.contains(&member) {
                    return Err(rejected());
                }
                let member = VerifyingKey::from_bytes(&member).map_err(error)?;
                if state
                    .acknowledgements
                    .get(&member)
                    .is_some_and(|previous| previous != &ack)
                {
                    return Err(rejected());
                }
                state.acknowledgements.insert(member, ack);
            }
            Ok(())
        })
        .await?;
    } else {
        let mut done = [0; 4];
        wire.read_fixed(&mut done).await.map_err(error)?;
        if &done != b"DONE" {
            return Err(rejected());
        }
    }
    wire.write_fixed(b"SAVE").await.map_err(error)?;
    wire.finish_send().await.map_err(error)?;
    let mut receipt = [0; 4];
    wire.read_fixed(&mut receipt).await.map_err(error)?;
    if &receipt != b"RCVD" {
        return Err(rejected());
    }
    Ok(())
}

impl SavedNode {
    fn delivery_plan(
        &self,
    ) -> Result<Option<(RotationProgress, BTreeMap<EndpointId, &'static [u8]>)>, JsValue> {
        let Some(progress) = self.rotation_progress()? else {
            return Ok(None);
        };
        let local = self.identity();
        let Ok(auth) = self.rotation_auth() else {
            return Ok(None);
        };
        if !matches!(self.stage(), "Complete" | "AwaitingManagementRecovery")
            || !progress.retained.contains(&local)
            || progress.stage == RotationStage::Complete && self.generation != progress.generation
        {
            return Ok(None);
        }
        let current_members: BTreeSet<_> = auth.nodes().into_iter().map(|(id, _)| id).collect();
        let rotation = self.current_rotation()?.ok_or_else(rejected)?;
        let peers = progress.recipients(&rotation, local,
            |peer| current_members.contains(&peer),
            |peer| !progress.acknowledged.contains(&peer) && self.pending_profile_for(&auth, peer).is_some(),
        ).map_err(error)?;
        Ok(Some((progress, peers)))
    }

    fn commit_rotation(&mut self, confirmation: &Confirmation) -> Result<bool, JsValue> {
        let Some(rotation) = self.current_rotation()? else {
            return Ok(true);
        };
        if rotation.proposal.id != confirmation.id
            || rotation.completion.is_some()
            || confirmation.expires <= Instant::now()
        {
            return Ok(true);
        }
        let retained = self.retained(&self.rotation_auth()?)?;
        if rotation.proposal.initiator != *self.identity().as_bytes()
            || retained
                .iter()
                .any(|peer| !rotation.proposal.retained.contains(peer))
        {
            return Err(rejected());
        }
        let state = self.rotation.as_ref().unwrap();
        let acknowledgements: Vec<_> = state
            .acknowledgements
            .iter()
            .filter(|(member, _)| retained.contains(member.as_bytes()))
            .map(|(_, ack)| ack.clone())
            .collect();
        if acknowledgements.len() != retained.len() {
            return Ok(false);
        }
        let completion = Completion {
            proposal: rotation.proposal_hash,
            retained,
            acknowledgements,
        };
        if confirmation.expires <= Instant::now() {
            return Ok(true);
        }
        let signed = Signed::sign(COMMIT_DOMAIN, &completion, &confirmation.root).map_err(error)?;
        self.complete_rotation(&rotation, signed)?;
        Ok(true)
    }
}

struct Confirmation {
    id: [u8; 16],
    root: SigningKey,
    expires: Instant,
}

#[derive(Default)]
struct ConfirmationSlot {
    pending: std::cell::RefCell<Option<Rc<Confirmation>>>,
    ready: Notify,
}

#[derive(Default)]
pub(super) struct RotationRuntime {
    confirmation: Rc<ConfirmationSlot>,
    workers: std::cell::RefCell<Vec<tokio::sync::oneshot::Receiver<()>>>,
}

impl RotationRuntime {
    fn authorize(&self, id: [u8; 16], root: SigningKey, expires: Instant) {
        *self.confirmation.pending.borrow_mut() = Some(Rc::new(Confirmation { id, root, expires }));
        self.confirmation.ready.notify_one();
    }

    pub(super) async fn stopped(&self) {
        self.confirmation.pending.borrow_mut().take();
        let workers = std::mem::take(&mut *self.workers.borrow_mut());
        for worker in workers {
            let _ = worker.await;
        }
    }

    fn spawn(&self, work: impl Future<Output = ()> + 'static) {
        let (stopped, receiver) = tokio::sync::oneshot::channel();
        self.workers.borrow_mut().push(receiver);
        wasm_bindgen_futures::spawn_local(async move {
            work.await;
            let _ = stopped.send(());
        });
    }

    pub(super) fn start(&self, network: &BrowserNetwork) {
        let budget = network.caller_budget;
        let gate = network.gate.clone();
        let cancel = network.incoming.cancellation();
        for alpn in [ADMISSION_ROTATION_ALPN, ROTATION_DELIVERY_ALPN] {
            let gate = gate.clone();
            let cancel = cancel.clone();
            network.incoming.install_active(
                alpn,
                2,
                move || budget,
                move |accepting, deadline| {
                    let gate = gate.clone();
                    let cancel = cancel.clone();
                    async move {
                        let _ = receive_rotation(accepting, deadline, &gate, &cancel).await;
                    }
                },
            );
        }
        let policy = network
            .saved
            .resources
            .as_ref()
            .map(|resources| resources.maintenance.clone())
            .unwrap_or_default();
        let interval = Duration::from_millis(policy.exploration_interval_ms);
        let mut changes = gate.0.lock().unwrap().changes.subscribe();
        let confirmation = self.confirmation.clone();
        let commit_gate = gate.clone();
        let commit_cancel = cancel.clone();
        self.spawn(async move {
            loop {
                let pending = confirmation.pending.borrow().clone();
                let mut retry = false;
                if let Some(pending) = &pending {
                    let result = if pending.expires <= Instant::now() {
                        Ok(true)
                    } else {
                        commit_gate.update_saved(|saved| saved.commit_rotation(pending))
                            .await
                    };
                    retry = result.is_err();
                    if matches!(result, Ok(true))
                        && confirmation
                            .pending
                            .borrow()
                            .as_ref()
                            .is_some_and(|current| Rc::ptr_eq(current, pending))
                    {
                        confirmation.pending.borrow_mut().take();
                    }
                }
                let expiry = confirmation
                    .pending
                    .borrow()
                    .as_ref()
                    .map(|current| current.expires);
                let timer = async {
                    let retry_until = Instant::now() + interval;
                    match expiry.map(|expires| {
                        if retry {
                            expires.min(retry_until)
                        } else {
                            expires
                        }
                    }) {
                        Some(expires) => {
                            sleep(expires.saturating_duration_since(Instant::now())).await
                        }
                        None => std::future::pending().await,
                    }
                };
                tokio::select! {
                    biased;
                    _ = commit_cancel.cancelled() => break,
                    _ = confirmation.ready.notified() => {},
                    _ = changes.changed() => {},
                    _ = timer => {},
                }
            }
        });
        let endpoint = network.endpoint.clone();
        let mut changes = gate.0.lock().unwrap().changes.subscribe();
        self.spawn(async move {
            let delay = Duration::from_millis(policy.path_retry_base_ms);
            let mut operation = None;
            let mut revision = None;
            let mut plan = None;
            let mut delivered = BTreeSet::new();
            let mut retry_at = BTreeMap::new();
            let mut last = None;
            loop {
                let start = Instant::now();
                let current_revision = *changes.borrow_and_update();
                if revision != Some(current_revision) {
                    revision = Some(current_revision);
                    plan = gate.service_snapshot().and_then(|saved| saved.delivery_plan()).ok().flatten();
                }
                if let Some((progress, candidates)) = &plan {
                    let expected = (progress.id, progress.stage);
                    if operation != Some(expected) {
                        operation = Some(expected);
                        last = None;
                        delivered.clear();
                        retry_at.clear();
                    }
                    retry_at.retain(|peer, due| *due > start && candidates.contains_key(peer));
                    let candidates_filtered = candidates.keys().copied()
                        .filter(|peer| !delivered.contains(peer) && !retry_at.contains_key(peer));
                    if let Some(peer) = crate::service_protocol::next_member_sync_peer(candidates_filtered, last) {
                        last = Some(peer);
                        let deadline = start + budget;
                        let open = async {
                            let address = gate.service_snapshot()?.address_for(peer)?;
                            let connecting = endpoint.connect_control_with_opts(
                                address, candidates[&peer],
                                iroh::endpoint::ConnectOptions::new().with_caller_deadline(deadline),
                            ).await.map_err(error)?;
                            Ok::<_, JsValue>(Arc::new(connecting.await.map_err(error)?).retain_control())
                        };
                        let connection = tokio::select! {
                            biased;
                            _ = cancel.cancelled() => break,
                            result = n0_future::time::timeout(budget, open) => result.ok().and_then(Result::ok),
                        };
                        if let Some(connection) = connection {
                            let _close = RequestConnection(Some(connection.clone()));
                            // Stop transport on close while allowing any already
                            // validated IndexedDB write to finish and install.
                            let _ = tokio::join!(
                                async {
                                    if send_rotation(&gate, &connection, expected).await.is_ok()
                                        && progress.stage == RotationStage::Complete
                                    {
                                        delivered.insert(peer);
                                    }
                                    connection.close(0u32.into(), b"rotation exchange ended");
                                },
                                async {
                                    tokio::select! {
                                        _ = cancel.cancelled() => connection.close(0u32.into(), b"node closing"),
                                        _ = connection.revocation_handle().closed() => {},
                                    }
                                }
                            );
                        }
                        retry_at.insert(peer, Instant::now() + delay);
                    }
                }
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => break,
                    _ = changes.changed() => {},
                    _ = sleep((start + interval).saturating_duration_since(Instant::now())) => {},
                }
            }
        });
    }
}

#[wasm_bindgen]
impl BrowserNetwork {
    pub fn rotation_progress(&self) -> Result<String, JsValue> {
        let progress = self.gate.service_snapshot()?.rotation_progress()?;
        Ok(serde_json::json!(progress.map(|progress| progress.public_json())).to_string())
    }

    pub async fn begin_admission_rotation(
        &mut self,
        recovery_card: Option<String>,
        password: Option<String>,
    ) -> Result<String, JsValue> {
        self.require_device_ready()?;
        let card = recovery_card
            .as_deref()
            .map(|card| RecoveryCard::import(card, password.as_deref()).map_err(error))
            .transpose()?;
        let expires = Instant::now() + CONFIRMATION_WINDOW;
        let (progress, root) = self.gate.update_saved(|saved| {
            let root = saved.genesis_key(card.as_ref())?;
            if expires <= Instant::now() {
                return Err(error("rotation confirmation expired"));
            }
            let progress = saved.begin_rotation(&root)?;
            Ok((progress, root))
        })
        .await?;
        self.rotation.authorize(progress.id, root, expires);
        Ok(progress.public_json().to_string())
    }

    pub async fn confirm_admission_rotation(
        &mut self,
        rotation: String,
        recovery_card: Option<String>,
        password: Option<String>,
    ) -> Result<String, JsValue> {
        self.require_device_ready()?;
        let mut id = [0; 16];
        hex::decode_to_slice(rotation, &mut id).map_err(error)?;
        let card = recovery_card
            .as_deref()
            .map(|card| RecoveryCard::import(card, password.as_deref()).map_err(error))
            .transpose()?;
        let expires = Instant::now() + CONFIRMATION_WINDOW;
        let saved = self.gate.installed().await?;
        let progress = saved.rotation_progress()?.ok_or_else(rejected)?;
        if progress.id != id || progress.initiator != saved.identity() {
            return Err(rejected());
        }
        if progress.stage == RotationStage::Distributing {
            saved.rotation_auth()?;
            let root = saved.genesis_key(card.as_ref())?;
            if expires <= Instant::now() {
                return Err(error("rotation confirmation expired"));
            }
            self.rotation.authorize(id, root, expires);
        }
        Ok(progress.public_json().to_string())
    }
}
