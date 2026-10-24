//! Browser custody uses the same signed offers and receipts, with strict
//! IndexedDB save/readback before a recipient signs its saved-copy evidence.
use super::*;
use crate::{
    credential_custody::{CUSTODY_ALPN, CustodyOffer, CustodyProbe, CustodyProof, CustodyReceipt},
    custody_protocol::{CustodyReply, CustodyRequest},
    data_stream::DataStream,
    device_protocol::{read_message, write_exchange},
    service_protocol::ServiceBundle,
};

const CHECK_SECONDS: u64 = 60;
fn view(entry: &crate::credential_custody::CustodyEntry) -> serde_json::Value {
    serde_json::json!({"offer":entry.offer,"recipient_confirmed":entry.recipient_confirmed(),
        "stage":if entry.stopped {"stopped_locally"} else if entry.receipt.is_some() {"saved"} else if entry.accepted {"ready_to_deliver"} else {"awaiting_recipient_confirmation"},
        "saved_receipt":entry.receipt,
        "impact":"add a complete genesis copy; keep the source copy; do not grant daily administration; removal cannot revoke retained copies"})
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct CustodyCheck {
    id: Hash,
    network: NetworkAnchor,
    local: VerifyingKey,
    checked_at: u64,
    proofs: Vec<CustodyProof>,
    observations: Vec<serde_json::Value>,
}
impl CustodyCheck {
    fn view(&self) -> serde_json::Value {
        serde_json::json!({"check":self.id,"network":self.network,"observations":self.observations,
            "confirmed_other_member":!self.proofs.is_empty(),"valid_for_seconds":CHECK_SECONDS,
            "warning":"unconfirmed or offline members may still retain a copy; deletion can permanently lose genesis recovery if all copies are gone; an external card is not confirmed member custody"})
    }
}

impl BrowserNetwork {
    async fn custody_exchange(
        &mut self,
        target: iroh_base::EndpointId,
        request: CustodyRequest,
    ) -> Result<CustodyReply, JsValue> {
        self.refresh_service_snapshot().await?;
        let connection = self.connect_target(target, CUSTODY_ALPN).await?;
        let _close = RequestConnection(Some(connection.clone()));
        let mut stream = DataStream::open(&connection).await.map_err(error)?;
        write_exchange(&mut stream, &request, &self.saved.service_bundle())
            .await
            .map_err(error)?;
        let reply = read_message(&mut stream).await.map_err(error)?;
        let history = ServiceBundle::read(&mut stream).await.map_err(error)?;
        self.refresh_service_snapshot().await?;
        let (next, policy_error) = self.saved.with_service_bundle(&history)?;
        self.commit(next).await?;
        self.files.enforce_access();
        if let Some(policy_error) = policy_error {
            return Err(error(policy_error));
        }
        stream.finish().map_err(error)?;
        Ok(reply)
    }
    async fn retain_custody_reply(&mut self, id: Hash, reply: CustodyReply) -> Result<(), JsValue> {
        self.refresh_service_snapshot().await?;
        let mut next = self.saved.clone();
        let entry = next.custody.get_mut(id).map_err(error)?;
        if let Some(receipt) = reply.receipt {
            receipt.verify(&entry.offer).map_err(error)?;
            if !reply.accepted || entry.receipt.as_ref().is_some_and(|old| old != &receipt) {
                return Err(rejected());
            }
            entry.receipt = Some(receipt);
        }
        entry.accepted |= reply.accepted;
        self.commit(next).await
    }
}

#[wasm_bindgen]
impl BrowserNetwork {
    pub fn credential_custody(&self) -> Result<String, JsValue> {
        if self.lease.is_none() {
            return Err(error("this node host is stopped"));
        }
        let saved = self.gate.service_snapshot()?;
        Ok(serde_json::json!({"holds_genesis":saved.creator_seed.is_some(),
            "offers":saved.custody.entries.iter().map(view).collect::<Vec<_>>(),
            "warning":"member removal and local deletion cannot invalidate complete copies retained elsewhere"}).to_string())
    }
    pub async fn propose_credential_custody(
        &mut self,
        recipient: String,
    ) -> Result<String, JsValue> {
        self.refresh_service_snapshot().await?;
        self.require_device_ready()?;
        let root = SigningKey::from_bytes(
            &self
                .saved
                .creator_seed
                .ok_or_else(|| error("no saved genesis credential"))?,
        );
        let offer = CustodyOffer::new(
            self.saved.invitation()?.anchor,
            &self.saved.verify()?.ok_or_else(rejected)?,
            self.saved.identity(),
            recipient.parse().map_err(error)?,
            &root,
        )
        .map_err(error)?;
        let mut next = self.saved.clone();
        next.custody.retain(offer.clone()).map_err(error)?;
        self.commit(next).await?;
        Ok(view(self.saved.custody.get(offer.id).map_err(error)?).to_string())
    }
    pub async fn accept_credential_custody(&mut self, offer: String) -> Result<String, JsValue> {
        self.refresh_service_snapshot().await?;
        self.require_device_ready()?;
        let id = offer.parse().map_err(error)?;
        let auth = self.saved.verify()?.ok_or_else(rejected)?;
        let mut next = self.saved.clone();
        let entry = next.custody.get_mut(id).map_err(error)?;
        if entry.offer.description.recipient != self.saved.identity() {
            return Err(rejected());
        }
        entry
            .offer
            .check_current(self.saved.invitation()?.anchor, &auth)
            .map_err(error)?;
        entry.accept().map_err(error)?;
        self.commit(next).await?;
        Ok(view(self.saved.custody.get(id).map_err(error)?).to_string())
    }
    pub async fn transfer_credential_custody(
        &mut self,
        offer: String,
        recipient: String,
    ) -> Result<String, JsValue> {
        self.refresh_service_snapshot().await?;
        self.require_device_ready()?;
        let id = offer.parse().map_err(error)?;
        let target: iroh_base::EndpointId = recipient.parse().map_err(error)?;
        let offer = self.saved.custody.get(id).map_err(error)?.offer.clone();
        if offer.description.source != self.saved.identity()
            || offer.description.recipient.as_bytes() != target.as_bytes()
        {
            return Err(rejected());
        }
        let reply = self
            .custody_exchange(
                target,
                CustodyRequest::Offer {
                    offer: offer.clone(),
                },
            )
            .await?;
        if reply.accepted && reply.receipt.is_none() {
            self.refresh_service_snapshot().await?;
            offer
                .check_current(
                    self.saved.invitation()?.anchor,
                    &self.saved.verify()?.ok_or_else(rejected)?,
                )
                .map_err(error)?;
            let seed = self
                .saved
                .creator_seed
                .ok_or_else(|| error("no saved genesis credential"))?;
            let reply = self
                .custody_exchange(target, CustodyRequest::Deliver { offer, seed })
                .await?;
            self.retain_custody_reply(id, reply).await?;
        } else {
            self.retain_custody_reply(id, reply).await?;
        }
        Ok(view(self.saved.custody.get(id).map_err(error)?).to_string())
    }
    pub async fn check_other_credential_custodians(&mut self) -> Result<String, JsValue> {
        self.refresh_service_snapshot().await?;
        self.require_device_ready()?;
        // The same check also precedes cancelling pending custody, when this
        // recipient may not yet have a saved root.
        let network = self.saved.invitation()?.anchor;
        let local = self.saved.identity();
        let auth = self.saved.verify()?.ok_or_else(rejected)?;
        let members = auth.nodes();
        let mut proofs = Vec::new();
        let mut observations = Vec::new();
        for (member, _) in members.into_iter().filter(|(id, _)| *id != local) {
            let probe = CustodyProbe {
                network,
                member,
                member_authorization: auth
                    .authorization_origin_at(member, &auth.tips(), false)
                    .ok_or_else(rejected)?,
                challenge: rand::random(),
            };
            let target = iroh_base::EndpointId::from_bytes(member.as_bytes()).map_err(error)?;
            match self
                .custody_exchange(
                    target,
                    CustodyRequest::Probe {
                        probe: probe.clone(),
                    },
                )
                .await
            {
                Ok(reply) => match reply.proof {
                    Some(proof) => {
                        proof.verify(&probe).map_err(error)?;
                        proofs.push(proof);
                        observations.push(
                            serde_json::json!({"member":member,"status":"confirmed_holding"}),
                        );
                    }
                    None => observations
                        .push(serde_json::json!({"member":member,"status":"currently_not_stored"})),
                },
                Err(_) => observations
                    .push(serde_json::json!({"member":member,"status":"unconfirmed_unreachable"})),
            }
        }
        self.refresh_service_snapshot().await?;
        let check = CustodyCheck {
            id: Hash::digest(rand::random::<[u8; 32]>()),
            network,
            local,
            checked_at: crate::service_policy::now(),
            proofs,
            observations,
        };
        let result = check.view();
        let mut next = self.saved.clone();
        next.custody_check = Some(check);
        self.commit(next).await?;
        Ok(result.to_string())
    }
    pub async fn stop_credential_custody(
        &mut self,
        check: String,
        force_without_confirmed_copy: bool,
    ) -> Result<String, JsValue> {
        self.refresh_service_snapshot().await?;
        self.require_device_ready()?;
        let id: Hash = check.parse().map_err(error)?;
        let check = self.saved.custody_check.as_ref().ok_or_else(rejected)?;
        let network = self.saved.invitation()?.anchor;
        if check.id != id || check.network != network || check.local != self.saved.identity() {
            return Err(rejected());
        }
        let now = crate::service_policy::now();
        let auth = self.saved.verify()?.ok_or_else(rejected)?;
        let confirmed = now >= check.checked_at
            && now - check.checked_at <= CHECK_SECONDS
            && check.proofs.iter().any(|proof| {
                proof.probe.network == network
                    && proof.probe.member != check.local
                    && proof.verify(&proof.probe).is_ok()
                    && auth.nodes().iter().any(|(id, _)| *id == proof.probe.member)
                    && auth.authorization_origin_at(proof.probe.member, &auth.tips(), false)
                        == Some(proof.probe.member_authorization)
            });
        if !confirmed && !force_without_confirmed_copy {
            return Err(error(
                "no recent confirmed member copy; explicit risk force-delete required",
            ));
        }
        let mut next = self.saved.clone();
        next.custody.stop_received(self.saved.identity());
        next.creator_seed = None;
        self.commit(next).await?;
        Ok(serde_json::json!({"deleted_locally":true,"confirmed_other_member":confirmed,
            "forced_without_confirmation":!confirmed && force_without_confirmed_copy,"identity_membership_roles_retained":true,
            "warning":"only this software's saved root was deleted; retained external or remote copies remain valid"}).to_string())
    }
}

pub(super) async fn serve_custody(
    connection: &Connection,
    gate: &BrowserGate,
    files: &std::rc::Rc<files::BrowserFiles>,
) -> Result<(), JsValue> {
    gate.qualified_snapshot(connection).await?;
    let peer = VerifyingKey::from_bytes(connection.remote_id().as_bytes()).map_err(error)?;
    let mut stream = DataStream::accept(connection).await.map_err(error)?;
    let request: CustodyRequest = read_message(&mut stream).await.map_err(error)?;
    let history = ServiceBundle::read(&mut stream).await.map_err(error)?;
    let (saved, raw, token) = services::installed(gate).await?;
    let (next, policy_error) = saved.with_service_bundle(&history)?;
    services::commit_installed(gate, &raw, token, &next).await?;
    files.enforce_access();
    if let Some(policy_error) = policy_error {
        return Err(error(policy_error));
    }
    let (mut next, raw, token) = services::installed(gate).await?;
    let auth = next.verify()?.ok_or_else(rejected)?;
    let network = next.invitation()?.anchor;
    let local = next.identity();
    if !auth.nodes().iter().any(|(id, _)| *id == peer)
        || !auth.nodes().iter().any(|(id, _)| *id == local)
    {
        return Err(rejected());
    }
    let reply = match request {
        CustodyRequest::Offer { offer } => {
            offer.check_current(network, &auth).map_err(error)?;
            if offer.description.source != peer || offer.description.recipient != local {
                return Err(rejected());
            }
            next.custody.retain(offer.clone()).map_err(error)?;
            let entry = next.custody.get(offer.id).map_err(error)?;
            let reply = CustodyReply {
                accepted: entry.recipient_confirmed(),
                receipt: entry.receipt.clone(),
                proof: None,
            };
            services::commit_installed(gate, &raw, token, &next).await?;
            reply
        }
        CustodyRequest::Deliver { offer, seed } => {
            offer.check_current(network, &auth).map_err(error)?;
            if offer.description.source != peer || offer.description.recipient != local {
                return Err(rejected());
            }
            let entry = next.custody.get(offer.id).map_err(error)?;
            if entry.offer != offer {
                return Err(rejected());
            }
            if let Some(receipt) = entry.delivery().map_err(error)? {
                CustodyReply {
                    accepted: true,
                    receipt: Some(receipt.clone()),
                    proof: None,
                }
            } else {
                if SigningKey::from_bytes(&seed).verifying_key() != network.root
                    || next.creator_seed.is_some_and(|old| old != seed)
                {
                    return Err(rejected());
                }
                next.creator_seed = Some(seed);
                next.recovery_exported = true;
                next.recovery_saved = true;
                services::commit_installed(gate, &raw, token, &next).await?;
                // Independent readback follows the actual strict transaction.
                let (mut saved, raw, token) = services::installed(gate).await?;
                let root = SigningKey::from_bytes(&saved.creator_seed.ok_or_else(rejected)?);
                let receipt =
                    CustodyReceipt::saved(&offer, &root, &SigningKey::from_bytes(&saved.key))
                        .map_err(error)?;
                let entry = saved.custody.get_mut(offer.id).map_err(error)?;
                if entry.receipt.as_ref().is_some_and(|old| old != &receipt) {
                    return Err(rejected());
                }
                entry.receipt = Some(receipt.clone());
                services::commit_installed(gate, &raw, token, &saved).await?;
                next = saved;
                CustodyReply {
                    accepted: true,
                    receipt: Some(receipt),
                    proof: None,
                }
            }
        }
        CustodyRequest::Probe { probe } => {
            if probe.network != network
                || probe.member != local
                || auth.authorization_origin_at(local, &auth.tips(), false)
                    != Some(probe.member_authorization)
            {
                return Err(rejected());
            }
            let proof = next
                .creator_seed
                .map(|seed| {
                    CustodyProof::sign(probe, &SigningKey::from_bytes(&seed)).map_err(error)
                })
                .transpose()?;
            CustodyReply {
                accepted: false,
                receipt: None,
                proof,
            }
        }
    };
    write_exchange(&mut stream, &reply, &next.service_bundle())
        .await
        .map_err(error)?;
    stream.finish().map_err(error)?;
    let _ = n0_future::time::timeout(Duration::from_secs(5), connection.closed()).await;
    Ok(())
}
