//! Browser adapter for the existing Native management-recovery wire. An
//! ordinary member serves the exporter-bound original-root proof; no old
//! administrator approves, and each retry retains the exact original grant.
use super::*;
use crate::{
    management_auth::ManagementExtension,
    management_recovery_wire::{proof_statement, recv_fixed, recv_frame, send_fixed, send_frame},
};
use n0_future::time::{Instant, timeout};
use p2panda_auth::{
    GroupsExtensionArgs,
    group::{GroupAction, GroupMember},
};
use p2panda_core::{Header, Signature};

pub(super) use crate::management_recovery_wire::{
    RECOVERY_ALPN, RecoveryIntent as BrowserRecovery, RecoveryProof as ServedRecovery,
};

impl SavedNode {
    pub(super) fn prepare_device_recovery(
        &mut self,
        card: crate::recovery_card::RecoveryCard,
        contact: iroh_base::EndpointId,
    ) -> Result<(), JsValue> {
        if self.invitation.is_some() || !self.records.is_empty() {
            return Err(error("recovery requires a new independent node"));
        }
        let network = card.anchor();
        let (seed, _) = card.credential();
        let mut invitation = network.root.as_bytes().to_vec();
        invitation.extend_from_slice(network.genesis.as_bytes());
        invitation.extend_from_slice(contact.as_bytes());
        self.invitation = Some(invitation);
        self.creator_seed = Some(seed);
        self.recovery_exported = true;
        self.recovery_saved = true;
        self.recovery = Some(BrowserRecovery {
            contact,
            attempt: rand::random(),
            authorization: None,
            complete: false,
        });
        self.verify()?;
        Ok(())
    }
    pub(super) fn verify_device_recovery(&self) -> Result<(), JsValue> {
        if self.served_recoveries.len() > 4096 {
            return Err(rejected());
        }
        let network = self.invitation()?.anchor;
        let mut ids = std::collections::BTreeSet::new();
        for proof in &self.served_recoveries {
            proof.verify(network, &self.records).map_err(error)?;
            if !ids.insert((proof.target, proof.attempt)) {
                return Err(rejected());
            }
        }
        if let Some(intent) = &self.recovery {
            intent
                .verify(
                    self.identity(),
                    &self.served_recoveries,
                    self.creator_seed.is_some(),
                )
                .map_err(error)?;
        }
        Ok(())
    }
    fn proof_index(&self, target: VerifyingKey, attempt: [u8; 16]) -> Option<usize> {
        self.served_recoveries
            .iter()
            .position(|r| r.target == target && r.attempt == attempt)
    }
    fn retain_live_proof(&mut self, proof: ServedRecovery) -> Result<usize, JsValue> {
        proof
            .verify(self.invitation()?.anchor, &self.records)
            .map_err(error)?;
        if let Some(index) = self.proof_index(proof.target, proof.attempt) {
            self.served_recoveries[index].statement = proof.statement;
            self.served_recoveries[index].signature = proof.signature;
            Ok(index)
        } else {
            if self.served_recoveries.len() >= 4096 {
                return Err(rejected());
            }
            self.served_recoveries.push(proof);
            Ok(self.served_recoveries.len() - 1)
        }
    }
}

fn recovery_deadline(connection: &ControlOwner) -> Instant {
    let limit = Instant::now() + Duration::from_secs(15);
    connection
        .caller_deadline()
        .map_or(limit, |deadline| deadline.min(limit))
}

async fn recovery_wire<T>(
    deadline: Instant,
    operation: impl Future<Output = std::io::Result<T>>,
) -> Result<T, JsValue> {
    if deadline <= Instant::now() {
        return Err(error("management recovery timed out"));
    }
    timeout(
        deadline.saturating_duration_since(Instant::now()),
        operation,
    )
    .await
    .map_err(|_| error("management recovery timed out"))?
    .map_err(error)
}

#[wasm_bindgen]
impl BrowserNetwork {
    pub async fn prepare_management_recovery(
        configuration: String,
        card: String,
        password: Option<String>,
        contact: String,
    ) -> Result<BrowserNetwork, JsValue> {
        let card = crate::recovery_card::RecoveryCard::import(&card, password.as_deref())
            .map_err(error)?;
        Self::start_onboarding(
            configuration,
            None,
            Startup::Recovery(card, contact.parse().map_err(error)?),
        )
        .await
    }
    pub fn management_recovery_status(&self) -> Result<String, JsValue> {
        let saved = self.gate.service_snapshot()?;
        let intent = saved
            .recovery
            .as_ref()
            .ok_or_else(|| error("not a management recovery operation"))?;
        let current = saved
            .verify()?
            .is_some_and(|a| a.nodes().contains(&(saved.identity(), Access::manage())));
        Ok(serde_json::json!({"stage":if intent.complete {"Complete"} else if intent.authorization.is_some() {"AuthorizationSaved"} else {"AwaitingProof"},
            "authorization":intent.authorization,"node":saved.identity(),"network":saved.invitation()?.anchor,
            "effective_administrator_locally":current}).to_string())
    }
    pub async fn restore_management_authority(
        &mut self,
        contact: String,
    ) -> Result<String, JsValue> {
        self.refresh_service_snapshot().await?;
        let intent = self
            .saved
            .recovery
            .clone()
            .ok_or_else(|| error("not a management recovery operation"))?;
        let target: iroh_base::EndpointId = contact.parse().map_err(error)?;
        if intent.contact != target {
            return Err(error("contact differs from this confirmed recovery"));
        }
        if let Some(grant) = intent.authorization {
            if !self
                .saved
                .verify()?
                .is_some_and(|a| a.administrator_authorization_valid(grant, self.saved.identity()))
            {
                return Err(error("this recovery authorization is no longer effective"));
            }
        }
        let connection = self.connect_device_control(target, RECOVERY_ALPN).await?;
        let _close = RequestConnection(Some(connection.clone()));
        self.recover_on(&connection, recovery_deadline(&connection))
            .await
    }
}

impl BrowserNetwork {
    async fn recover_on(
        &mut self,
        connection: &ControlOwner,
        deadline: Instant,
    ) -> Result<String, JsValue> {
        let intent = self.saved.recovery.clone().ok_or_else(rejected)?;
        let network = self.saved.invitation()?.anchor;
        let local =
            iroh_base::EndpointId::from_bytes(self.saved.identity().as_bytes()).map_err(error)?;
        let statement =
            proof_statement(connection, network, local, intent.contact).map_err(error)?;
        let root = SigningKey::from_bytes(
            &self
                .saved
                .creator_seed
                .ok_or_else(|| error("recovery credential not retained"))?,
        );
        let signature = root.sign(&statement).to_bytes();
        let exchange = recovery_wire(deadline, connection.open_control_exchange()).await?;
        recovery_wire(deadline, send_fixed(&exchange, &signature)).await?;
        recovery_wire(deadline, send_fixed(&exchange, &intent.attempt)).await?;
        let bytes = recovery_wire(deadline, recv_frame(&exchange, 16 * 1024 * 1024)).await?;
        let (server_grant, records): (Option<Hash>, Records) =
            p2panda_core::cbor::decode_cbor(bytes.as_slice()).map_err(error)?;
        if records.is_empty()
            || records.len() > 4096
            || Hash::digest(&records[0].0) != network.genesis
        {
            return Err(rejected());
        }
        let auth = ManagementAuth::replay_records(network, &records).map_err(error)?;
        if !auth
            .nodes()
            .iter()
            .any(|(id, _)| id.as_bytes() == intent.contact.as_bytes())
        {
            return Err(rejected());
        }
        if server_grant
            .is_some_and(|g| !auth.administrator_authorization_valid(g, self.saved.identity()))
        {
            return Err(rejected());
        }
        self.refresh_service_snapshot().await?;
        let mut next = self.saved.clone();
        let (merged, policy_error) =
            next.with_service_bundle(&crate::service_protocol::ServiceBundle {
                management: records,
                services: Vec::new(),
            })?;
        if let Some(policy_error) = policy_error {
            return Err(error(policy_error));
        }
        next = merged;
        let proof = ServedRecovery {
            target: next.identity(),
            attempt: intent.attempt,
            authorization: None,
            statement,
            signature: signature.to_vec(),
            phase: 1,
        };
        next.admissions_open()?;
        let index = next.retain_live_proof(proof)?;
        self.commit(next).await?;
        let auth = self.saved.verify()?.ok_or_else(rejected)?;
        let old = self.saved.served_recoveries[index].authorization;
        if old.is_some() && server_grant.is_some() && old != server_grant {
            return Err(rejected());
        }
        let grant = if let Some(id) = old.or(server_grant) {
            if !auth.administrator_authorization_valid(id, self.saved.identity()) {
                return Err(rejected());
            }
            self.saved
                .records
                .iter()
                .find(|(h, _)| Hash::digest(h) == id)
                .ok_or_else(rejected)?
                .0
                .clone()
        } else {
            let log_id = loop {
                let id = rand::random();
                if !auth.has_author_log_id(network.root, id) {
                    break id;
                }
            };
            Header::<ManagementExtension>::builder()
                .build(
                    &root,
                    ManagementExtension {
                        version: 1,
                        log_id,
                        group: GroupsExtensionArgs {
                            group_id: network.root,
                            action: GroupAction::Add {
                                member: GroupMember::Individual(self.saved.identity()),
                                access: Access::manage(),
                            },
                            dependencies: auth.tips(),
                        },
                    },
                )
                .encode()
        };
        let id = Hash::digest(&grant);
        let mut next = self.saved.clone();
        if !next.records.iter().any(|(h, _)| Hash::digest(h) == id) {
            next.records.push((grant.clone(), None));
        }
        next.authorization = Some(id);
        next.recovery.as_mut().unwrap().authorization = Some(id);
        next.served_recoveries[index].authorization = Some(id);
        // A fresh TLS proof refreshes a completed attempt without rolling its
        // durable completion back while the same grant is delivered again.
        next.served_recoveries[index].phase = next.served_recoveries[index].phase.max(2);
        self.commit(next).await?;
        self.files.enforce_access();
        recovery_wire(deadline, send_frame(&exchange, &grant)).await?;
        if recovery_wire(deadline, recv_fixed::<32>(&exchange)).await? != *id.as_bytes() {
            return Err(rejected());
        }
        recovery_wire(deadline, send_fixed(&exchange, b"AUTH")).await?;
        let material = recovery_wire(deadline, recv_fixed::<40>(&exchange)).await?;
        let generation = u64::from_be_bytes(material[..8].try_into().unwrap());
        self.gate.update_saved(|saved| {
            saved.admissions_open()?;
            if generation < saved.generation
                || generation > i64::MAX as u64
                || !saved.verify()?.is_some_and(|auth| {
                    auth.administrator_authorization_valid(id, saved.identity())
                })
            {
                return Err(rejected());
            }
            let index = saved
                .proof_index(saved.identity(), intent.attempt)
                .ok_or_else(rejected)?;
            saved.generation = generation;
            saved.secret = Some(material[8..].try_into().unwrap());
            saved.recovery.as_mut().unwrap().complete = true;
            saved.served_recoveries[index].phase = 3;
            Ok(())
        })
        .await?;
        self.refresh_service_snapshot().await?;
        recovery_wire(deadline, send_fixed(&exchange, b"SAVE")).await?;
        recovery_wire(deadline, exchange.finish_send()).await?;
        if recovery_wire(deadline, recv_fixed::<4>(&exchange)).await? != *b"DONE" {
            return Err(rejected());
        }
        Ok(serde_json::json!({"stage":"Complete","authorization":id,"node":self.saved.identity(),"network":network,
            "saved_locally":true,"old_administrators_automatically_revoked":false,
            "scope":"this original network; new node identity, no business data recovery"}).to_string())
    }
}

pub(super) async fn serve_recovery(
    connection: &ControlOwner,
    gate: &BrowserGate,
    files: &std::rc::Rc<files::BrowserFiles>,
) -> Result<(), JsValue> {
    serve_inner(connection, gate, files, recovery_deadline(connection)).await
}
async fn serve_inner(
    connection: &ControlOwner,
    gate: &BrowserGate,
    files: &std::rc::Rc<files::BrowserFiles>,
    deadline: Instant,
) -> Result<(), JsValue> {
    let saved = gate.installed().await?;
    saved.admissions_open()?;
    let network = saved.invitation()?.anchor;
    let local = iroh_base::EndpointId::from_bytes(saved.identity().as_bytes()).map_err(error)?;
    let target = VerifyingKey::from_bytes(connection.remote_id().as_bytes()).map_err(error)?;
    let auth = saved.verify()?.ok_or_else(rejected)?;
    if saved.stage() != "Complete"
        || saved.secret.is_none()
        || !auth.nodes().iter().any(|(id, _)| *id == saved.identity())
    {
        return Err(rejected());
    }
    let statement =
        proof_statement(connection, network, connection.remote_id(), local).map_err(error)?;
    let exchange = recovery_wire(deadline, connection.accept_control_exchange()).await?;
    let signature = recovery_wire(deadline, recv_fixed::<64>(&exchange)).await?;
    if !network
        .root
        .verify(&statement, &Signature::from_bytes(&signature))
    {
        return Err(rejected());
    }
    let attempt = recovery_wire(deadline, recv_fixed::<16>(&exchange)).await?;
    let (mut next, version) = gate.snapshot().await?;
    next.admissions_open()?;
    let index = next.retain_live_proof(ServedRecovery {
        target,
        attempt,
        authorization: None,
        statement,
        signature: signature.to_vec(),
        phase: 1,
    })?;
    let auth = next.verify()?.ok_or_else(rejected)?;
    let existing = next.served_recoveries[index].authorization;
    if existing.is_some_and(|id| !auth.administrator_authorization_valid(id, target)) {
        return Err(rejected());
    }
    version.commit(&next).await?;
    let bytes =
        p2panda_core::cbor::encode_cbor(&(existing, next.records.clone())).map_err(error)?;
    recovery_wire(deadline, send_frame(&exchange, &bytes)).await?;
    let grant = recovery_wire(deadline, recv_frame(&exchange, 256 * 1024)).await?;
    let header = Header::<ManagementExtension>::decode(&grant).map_err(error)?;
    let id = Hash::digest(&grant);
    if header.verifying_key != network.root
        || header.seq_num != 0
        || header.backlink.is_some()
        || !matches!(&header.extensions.group.action,GroupAction::Add {member:GroupMember::Individual(member),access} if *member==target && *access==Access::manage())
    {
        return Err(rejected());
    }
    let (mut next, version) = gate.snapshot().await?;
    let auth = next.verify()?.ok_or_else(rejected)?;
    next.admissions_open()?;
    let index = next.proof_index(target, attempt).ok_or_else(rejected)?;
    if let Some(existing) = next.served_recoveries[index].authorization {
        if existing != id || !auth.administrator_authorization_valid(id, target) {
            return Err(rejected());
        }
    } else {
        if auth.has_author_log_id(network.root, header.extensions.log_id)
            || header.extensions.group.dependencies != auth.tips()
        {
            return Err(rejected());
        }
        next.records.push((grant, None));
    }
    next.served_recoveries[index].authorization = Some(id);
    next.served_recoveries[index].phase = next.served_recoveries[index].phase.max(2);
    version.commit(&next).await?;
    files.enforce_access();
    recovery_wire(deadline, send_fixed(&exchange, id.as_bytes())).await?;
    if recovery_wire(deadline, recv_fixed::<4>(&exchange)).await? != *b"AUTH" {
        return Err(rejected());
    }
    {
        let (next, _write) = gate.write_node().await?;
        next.admissions_open()?;
        let auth = next.verify()?.ok_or_else(rejected)?;
        if !auth.administrator_authorization_valid(id, target)
            || !auth.nodes().iter().any(|(id, _)| *id == next.identity())
        {
            return Err(rejected());
        }
        let mut material = next.generation.to_be_bytes().to_vec();
        material.extend_from_slice(&next.secret.ok_or_else(rejected)?);
        recovery_wire(deadline, send_fixed(&exchange, &material)).await?;
    }
    if recovery_wire(deadline, recv_fixed::<4>(&exchange)).await? != *b"SAVE" {
        return Err(rejected());
    }
    let (mut next, version) = gate.snapshot().await?;
    let index = next.proof_index(target, attempt).ok_or_else(rejected)?;
    if !next
        .verify()?
        .is_some_and(|a| a.administrator_authorization_valid(id, target))
    {
        return Err(rejected());
    }
    next.served_recoveries[index].phase = 3;
    version.commit(&next).await?;
    recovery_wire(deadline, send_fixed(&exchange, b"DONE")).await?;
    recovery_wire(deadline, exchange.finish_send()).await?;
    let _ = n0_future::time::timeout(
        Duration::from_secs(5),
        connection.revocation_handle().closed(),
    )
    .await;
    Ok(())
}
