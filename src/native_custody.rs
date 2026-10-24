//! Native custody delivery saves the complete credential before issuing its
//! receipt. Possession checks inspect current members and never infer absence
//! from offline status. All journals are secret-free.
use std::{collections::BTreeMap, sync::Arc, time::Duration};

use iroh::{EndpointAddr, endpoint::Connection};
use p2panda_core::{Hash, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::{
    credential_custody::{
        CUSTODY_ALPN, CustodyJournal, CustodyOffer, CustodyProbe, CustodyProof, CustodyReceipt,
    },
    custody_protocol::{CustodyReply, CustodyRequest},
    data_stream::DataStream,
    device_protocol::{read_message, write_exchange},
    native_client::{ClientError, NativeClient},
    native_file_transfer::NativeFiles,
    native_network::NativeNetwork,
    native_services::NativeServices,
    service_protocol::ServiceBundle,
};

const JOURNAL: &str = "credential.custody";
const CHECK: &str = "credential.custody-check";
const CHECK_SECONDS: u64 = 60;

fn load(node: &NativeNetwork) -> Result<CustodyJournal, ClientError> {
    let journal = match std::fs::read(node.directory.join(JOURNAL)) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => CustodyJournal::default(),
        Err(e) => return Err(e.into()),
    };
    journal.verify(node.anchor)?;
    Ok(journal)
}
fn save(node: &NativeNetwork, journal: &CustodyJournal) -> Result<(), ClientError> {
    journal.verify(node.anchor)?;
    crate::native_files::write_private_atomic(
        &node.directory.join(JOURNAL),
        &serde_json::to_vec(journal)?,
    )?;
    Ok(())
}
fn view(entry: &crate::credential_custody::CustodyEntry) -> Value {
    json!({"offer":entry.offer, "recipient_confirmed":entry.recipient_confirmed(),
        "stage":if entry.stopped {"stopped_locally"} else if entry.receipt.is_some() {"saved"} else if entry.accepted {"ready_to_deliver"} else {"awaiting_recipient_confirmation"},
        "saved_receipt":entry.receipt,
        "impact":"add a complete genesis copy; keep the source copy; do not grant daily administration; removal cannot revoke retained copies"})
}
fn current_member(node: &NativeNetwork, id: VerifyingKey) -> bool {
    node.status()
        .members
        .iter()
        .any(|(member, _)| *member == id)
}

impl NativeClient {
    /// The source confirms this specific recipient and retained-copy impact.
    pub async fn propose_credential_custody(
        &self,
        recipient: VerifyingKey,
    ) -> Result<Value, ClientError> {
        self.require_ready()?;
        let node = self.node.lock().await;
        let auth = self.services.lock().await.access(&node).await?.auth;
        let credential = node.saved_genesis_credential()?;
        let offer = CustodyOffer::new(
            node.anchor,
            &auth,
            node.status().node_id,
            recipient,
            &credential.root,
        )?;
        let mut journal = load(&node)?;
        journal.retain(offer.clone())?;
        save(&node, &journal)?;
        Ok(view(journal.get(offer.id)?))
    }

    pub async fn credential_custody(&self) -> Result<Value, ClientError> {
        self.require_ready()?;
        let node = self.node.lock().await;
        Ok(
            json!({"holds_genesis":node.directory.join("genesis.credential").exists(),
            "offers":load(&node)?.entries.iter().map(view).collect::<Vec<_>>(),
            "warning":"member removal and local deletion cannot invalidate complete copies retained elsewhere"}),
        )
    }

    pub async fn accept_credential_custody(&self, offer: Hash) -> Result<Value, ClientError> {
        self.require_ready()?;
        let node = self.node.lock().await;
        let auth = self.services.lock().await.access(&node).await?.auth;
        let mut journal = load(&node)?;
        let entry = journal.get_mut(offer)?;
        if entry.offer.description.recipient != node.status().node_id {
            return Err("not this custody recipient".into());
        }
        entry.offer.check_current(node.anchor, &auth)?;
        entry.accept()?;
        save(&node, &journal)?;
        Ok(view(journal.get(offer)?))
    }

    /// First call delivers only the source-confirmed offer. A later call, after
    /// recipient acceptance, sends the root and waits for durable-save evidence.
    pub async fn transfer_credential_custody(
        &self,
        id: Hash,
        recipient: EndpointAddr,
    ) -> Result<Value, ClientError> {
        self.require_ready()?;
        let offer = {
            let node = self.node.lock().await;
            let journal = load(&node)?;
            let offer = journal.get(id)?.offer.clone();
            if offer.description.source != node.status().node_id
                || offer.description.recipient.as_bytes() != recipient.id.as_bytes()
            {
                return Err("custody destination does not match confirmed recipient".into());
            }
            offer
        };
        let reply = self
            .custody_exchange(
                recipient.clone(),
                CustodyRequest::Offer {
                    offer: offer.clone(),
                },
            )
            .await?;
        if reply.accepted && reply.receipt.is_none() {
            let seed = {
                let node = self.node.lock().await;
                let auth = self.services.lock().await.access(&node).await?.auth;
                offer.check_current(node.anchor, &auth)?;
                *node.saved_genesis_credential()?.root.as_bytes()
            };
            let reply = self
                .custody_exchange(
                    recipient,
                    CustodyRequest::Deliver {
                        offer: offer.clone(),
                        seed,
                    },
                )
                .await?;
            self.retain_custody_reply(id, reply).await?;
        } else {
            self.retain_custody_reply(id, reply).await?;
        }
        let node = self.node.lock().await;
        Ok(view(load(&node)?.get(id)?))
    }

    async fn retain_custody_reply(&self, id: Hash, reply: CustodyReply) -> Result<(), ClientError> {
        let node = self.node.lock().await;
        let mut journal = load(&node)?;
        let entry = journal.get_mut(id)?;
        if let Some(receipt) = reply.receipt {
            receipt.verify(&entry.offer)?;
            if !reply.accepted {
                return Err("saved receipt without recipient confirmation".into());
            }
            if entry.receipt.as_ref().is_some_and(|old| old != &receipt) {
                return Err("custody receipt changed".into());
            }
            entry.receipt = Some(receipt);
        }
        entry.accepted |= reply.accepted;
        save(&node, &journal)
    }

    async fn custody_exchange(
        &self,
        target: EndpointAddr,
        request: CustodyRequest,
    ) -> Result<CustodyReply, ClientError> {
        let history = {
            let node = self.node.lock().await;
            self.services.lock().await.snapshot(&node).await?
        };
        let connection = self.resources.connect(target, CUSTODY_ALPN).await?;
        let result = tokio::time::timeout(Duration::from_secs(60), async {
            let mut stream = DataStream::open(&connection).await?;
            write_exchange(&mut stream, &request, &history).await?;
            let reply: CustodyReply = read_message(&mut stream).await?;
            let history = ServiceBundle::read(&mut stream).await?;
            let mut node = self.node.lock().await;
            self.services
                .lock()
                .await
                .merge(&mut node, &self.files, history)
                .await?;
            stream.finish()?;
            Ok::<_, ClientError>(reply)
        })
        .await;
        connection.close(0u32.into(), b"credential custody exchange ended");
        result?
    }

    /// Check all currently known other members, using supplied addresses as
    /// contact hints. Offline/unreachable is explicitly unconfirmed.
    pub async fn check_other_credential_custodians(
        &self,
        addresses: Vec<EndpointAddr>,
    ) -> Result<Value, ClientError> {
        self.require_ready()?;
        let (network, local, members, auth) = {
            let node = self.node.lock().await;
            // A recipient may stop pending custody before its first save.
            // Possession by other members is still checked in the same way.
            (
                node.anchor,
                node.status().node_id,
                node.status().members,
                self.services.lock().await.access(&node).await?.auth,
            )
        };
        let hints: BTreeMap<_, _> = addresses.into_iter().map(|a| (a.id, a)).collect();
        let mut observations = Vec::new();
        let mut proofs = Vec::new();
        for (member, _) in members.into_iter().filter(|(id, _)| *id != local) {
            let endpoint = iroh_base::EndpointId::from_bytes(member.as_bytes())?;
            let address = hints
                .get(&endpoint)
                .cloned()
                .unwrap_or_else(|| EndpointAddr::from(endpoint));
            let probe = CustodyProbe {
                network,
                member,
                member_authorization: auth
                    .authorization_origin_at(member, &auth.tips(), false)
                    .ok_or("custody member authorization disappeared")?,
                challenge: rand::random(),
            };
            let result = self
                .custody_exchange(
                    address,
                    CustodyRequest::Probe {
                        probe: probe.clone(),
                    },
                )
                .await;
            match result {
                Ok(reply) => match reply.proof {
                    Some(proof) => {
                        proof.verify(&probe)?;
                        proofs.push(proof);
                        observations.push(json!({"member":member,"status":"confirmed_holding"}));
                    }
                    None => {
                        observations.push(json!({"member":member,"status":"currently_not_stored"}))
                    }
                },
                Err(_) => {
                    observations.push(json!({"member":member,"status":"unconfirmed_unreachable"}))
                }
            }
        }
        let check = CustodyCheck {
            id: Hash::digest(rand::random::<[u8; 32]>()),
            network,
            local,
            checked_at: crate::service_policy::now(),
            proofs,
            observations,
        };
        let node = self.node.lock().await;
        crate::native_files::write_private_atomic(
            &node.directory.join(CHECK),
            &serde_json::to_vec(&check)?,
        )?;
        Ok(check.view())
    }

    /// Explicitly confirm deletion of this software's own saved root material.
    /// If there is no recent confirmed member copy, the separate force choice
    /// acknowledges the risk; an external recovery card is not a member proof.
    pub async fn stop_credential_custody(
        &self,
        check_id: Hash,
        force_without_confirmed_copy: bool,
    ) -> Result<Value, ClientError> {
        self.require_ready()?;
        let node = self.node.lock().await;
        let check: CustodyCheck =
            serde_json::from_slice(&std::fs::read(node.directory.join(CHECK))?)?;
        if check.id != check_id
            || check.network != node.anchor
            || check.local != node.status().node_id
        {
            return Err("custody check belongs to another operation".into());
        }
        let now = crate::service_policy::now();
        let auth = self.services.lock().await.access(&node).await?.auth;
        let recent = now >= check.checked_at && now - check.checked_at <= CHECK_SECONDS;
        let confirmed = recent
            && check.proofs.iter().any(|proof| {
                proof.probe.network == node.anchor
                    && proof.probe.member != check.local
                    && current_member(&node, proof.probe.member)
                    && auth.authorization_origin_at(proof.probe.member, &auth.tips(), false)
                        == Some(proof.probe.member_authorization)
                    && proof.verify(&proof.probe).is_ok()
            });
        if !confirmed && !force_without_confirmed_copy {
            return Err(
                "no recent confirmed member copy; explicit risk force-delete required".into(),
            );
        }
        // Persist consent cancellation before deleting material. If deletion
        // fails, retry remains safe and old pending consent cannot store again.
        let mut journal = load(&node)?;
        journal.stop_received(node.status().node_id);
        save(&node, &journal)?;
        // An interrupted private atomic write may leave an unreferenced vault
        // fragment. Delete only the writer's namespace and this exact original
        // root+Create pair; unrelated files are retained.
        let mut orphaned_credentials_removed = 0;
        for entry in std::fs::read_dir(&node.directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let owned = name
                .strip_prefix(".mio-")
                .is_some_and(|s| s.len() == 32 && u128::from_str_radix(s, 16).is_ok());
            if owned && entry.path().symlink_metadata()?.is_file() {
                let bytes = std::fs::read(entry.path())?;
                if bytes.len() == 64
                    && bytes[32..] == *node.anchor.genesis.as_bytes()
                    && p2panda_core::SigningKey::from_bytes(bytes[..32].try_into().unwrap())
                        .verifying_key()
                        == node.anchor.root
                {
                    std::fs::remove_file(entry.path())?;
                    orphaned_credentials_removed += 1;
                }
            }
        }
        match std::fs::remove_file(node.directory.join("genesis.credential")) {
            Ok(()) => crate::native_files::sync_directory(&node.directory)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        Ok(
            json!({"deleted_locally":true,"confirmed_other_member":confirmed,
            "orphaned_credentials_removed":orphaned_credentials_removed,
            "forced_without_confirmation":!confirmed && force_without_confirmed_copy,
            "identity_membership_roles_retained":true,
            "warning":"only this software's saved root was deleted; retained external or remote copies remain valid"}),
        )
    }
}

#[derive(Serialize, Deserialize)]
struct CustodyCheck {
    id: Hash,
    network: crate::management_auth::NetworkAnchor,
    local: VerifyingKey,
    checked_at: u64,
    proofs: Vec<CustodyProof>,
    observations: Vec<Value>,
}
impl CustodyCheck {
    fn view(&self) -> Value {
        json!({"check":self.id,"network":self.network,"observations":self.observations,
            "confirmed_other_member":!self.proofs.is_empty(),"valid_for_seconds":CHECK_SECONDS,
            "warning":"unconfirmed or offline members may still retain a copy; deletion can permanently lose genesis recovery if all copies are gone; an external card is not confirmed member custody"})
    }
}

pub(crate) async fn serve_custody(
    connection: Connection,
    node: Arc<Mutex<NativeNetwork>>,
    services: Arc<Mutex<NativeServices>>,
    files: Arc<NativeFiles>,
) -> Result<(), ClientError> {
    let result = async {
        let peer = VerifyingKey::from_bytes(connection.remote_id().as_bytes())?;
        let mut stream = DataStream::accept(&connection).await?;
        let request: CustodyRequest = read_message(&mut stream).await?;
        let history = ServiceBundle::read(&mut stream).await?;
        let (reply, history) = {
            let mut node = node.lock().await;
            let mut services = services.lock().await;
            services.merge(&mut node, &files, history).await?;
            let auth = services.access(&node).await?.auth;
            let local = node.status().node_id;
            if !current_member(&node, peer) || !current_member(&node, local) {
                return Err("custody requires current membership".into());
            }
            let mut journal = load(&node)?;
            let reply = match request {
                CustodyRequest::Offer { offer } => {
                    offer.check_current(node.anchor, &auth)?;
                    if offer.description.source != peer || offer.description.recipient != local {
                        return Err("wrong custody endpoints".into());
                    }
                    journal.retain(offer.clone())?;
                    save(&node, &journal)?;
                    let entry = journal.get(offer.id)?;
                    CustodyReply {
                        accepted: entry.recipient_confirmed(),
                        receipt: entry.receipt.clone(),
                        proof: None,
                    }
                }
                CustodyRequest::Deliver { offer, seed } => {
                    offer.check_current(node.anchor, &auth)?;
                    if offer.description.source != peer || offer.description.recipient != local {
                        return Err("wrong custody endpoints".into());
                    }
                    let entry = journal.get_mut(offer.id)?;
                    if entry.offer != offer {
                        return Err("recipient has not confirmed this custody offer".into());
                    }
                    if let Some(receipt) = entry.delivery()? {
                        CustodyReply {
                            accepted: true,
                            receipt: Some(receipt.clone()),
                            proof: None,
                        }
                    } else {
                        let root = p2panda_core::SigningKey::from_bytes(&seed);
                        if root.verifying_key() != node.anchor.root {
                            return Err("wrong genesis credential".into());
                        }
                        if node.directory.join("genesis.credential").exists()
                            && node.saved_genesis_credential()?.root.as_bytes() != root.as_bytes()
                        {
                            return Err("a different credential is already saved".into());
                        }
                        // The first-network recovery-card gate applies to creation,
                        // not to an already admitted custody recipient.
                        crate::native_client::record_received_credential(&node)?;
                        let mut material = seed.to_vec();
                        material.extend_from_slice(node.anchor.genesis.as_bytes());
                        crate::native_files::write_private_atomic(
                            &node.directory.join("genesis.credential"),
                            &material,
                        )?;
                        let saved = node.saved_genesis_credential()?;
                        let receipt = CustodyReceipt::saved(&offer, &saved.root, &node.node_key)?;
                        if entry.receipt.as_ref().is_some_and(|old| old != &receipt) {
                            return Err("saved custody receipt mismatch".into());
                        }
                        entry.receipt = Some(receipt.clone());
                        save(&node, &journal)?;
                        CustodyReply {
                            accepted: true,
                            receipt: Some(receipt),
                            proof: None,
                        }
                    }
                }
                CustodyRequest::Probe { probe } => {
                    if probe.network != node.anchor
                        || probe.member != local
                        || auth.authorization_origin_at(local, &auth.tips(), false)
                            != Some(probe.member_authorization)
                    {
                        return Err("wrong custody probe".into());
                    }
                    let proof = if node.directory.join("genesis.credential").exists() {
                        Some(CustodyProof::sign(
                            probe,
                            &node.saved_genesis_credential()?.root,
                        )?)
                    } else {
                        None
                    };
                    CustodyReply {
                        accepted: false,
                        receipt: None,
                        proof,
                    }
                }
            };
            (reply, services.snapshot(&node).await?)
        };
        write_exchange(&mut stream, &reply, &history).await?;
        stream.finish()?;
        let _ = tokio::time::timeout(Duration::from_secs(5), connection.closed()).await;
        Ok(())
    }
    .await;
    connection.close(0u32.into(), b"credential custody exchange ended");
    result
}

pub(crate) fn validate(node: &NativeNetwork) -> Result<(), ClientError> {
    load(node)?;
    Ok(())
}
