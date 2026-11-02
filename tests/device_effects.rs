//! Actual custody wire and backup activation effects using fresh owned state.
//! Credential bytes never enter diagnostics; all network actors use one local relay.
use miopunch_native_probe::{
    credential_custody::{CUSTODY_ALPN, CustodyOffer},
    custody_protocol::{CustodyReply, CustodyRequest},
    data_stream::DataStream,
    device_operation::DeviceAction,
    device_protocol::{read_message, write_exchange},
    manual_admission::AdmissionStage,
    native_client::{ClientError, NativeClient},
    native_network::NativeNetwork,
    node_backup::NodeBackup,
    recovery_card::RecoveryCard,
    resources::ResourceConfig,
    service_policy::{self, Service, ServiceRule},
    service_protocol::ServiceBundle,
};
use p2panda_core::Hash;
use serde_json::{Value, json};
use std::{net::Ipv4Addr, path::PathBuf, time::Duration};

fn require(condition: bool, message: &'static str) -> Result<(), ClientError> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}
fn id(value: &Value) -> Result<Hash, ClientError> {
    Ok(value.as_str().ok_or("identifier absent")?.parse()?)
}
struct Pair {
    root: PathBuf,
    clients: Vec<Option<NativeClient>>,
    card: RecoveryCard,
}
impl Pair {
    async fn start(label: &str) -> Result<Self, ClientError> {
        let root =
            PathBuf::from(std::env::var_os("M116_EFFECT_PRIVATE").ok_or("owned root required")?)
                .join(label);
        std::fs::create_dir(&root)?;
        let relay = std::env::var("M116_EFFECT_LOCAL_RELAY")?;
        let config: ResourceConfig = serde_json::from_value(json!({
            "defaults":{"revision":2,"resources":[]},
            "replacement":[{"url":relay,"uses":["DataRelay"],"limitation":"owned device effect fixture"}],
            "disabled":[],"disabled_uses":["Observation","Lookup","Signaling"],
            "lan_discovery":"Disabled","port_mapping":"Disabled"
        }))?;
        let source = NativeClient::create_with_resources(
            root.join("source"),
            Ipv4Addr::LOCALHOST,
            config.clone(),
        )
        .await?;
        let card_result = source.export_recovery_card(None).await;
        let card = match card_result.and_then(|text| Ok(RecoveryCard::import(&text, None)?)) {
            Ok(card) => card,
            Err(error) => {
                source.close().await?;
                return Err(error);
            }
        };
        let mut pair = Self {
            root,
            clients: vec![Some(source)],
            card,
        };
        let outcome = async {
            pair.at(0).acknowledge_recovery_saved().await?;
            let invitation = pair
                .at(0)
                .issue_precode(service_policy::now() + 600, Some(1))
                .await?;
            let peer = NativeClient::join_with_resources(
                pair.root.join("peer"),
                &invitation,
                Ipv4Addr::LOCALHOST,
                config,
            )
            .await?;
            pair.clients.push(Some(peer));
            require(
                pair.at(1).request_admission(&invitation).await? == AdmissionStage::Complete,
                "admission incomplete",
            )?;
            pair.at(0).sync_services(pair.at(1).address()).await?;
            require(
                pair.at(1).ping(pair.at(0).address()).await?["reply"] == "PONG",
                "member connection premise",
            )
        }
        .await;
        if let Err(error) = outcome {
            let cleanup = pair.close().await;
            return Err(format!("setup: {error}; cleanup: {cleanup:?}").into());
        }
        Ok(pair)
    }
    fn at(&self, index: usize) -> &NativeClient {
        self.clients[index].as_ref().unwrap()
    }
    async fn stop(&mut self, index: usize) -> Result<(), ClientError> {
        if let Some(client) = self.clients[index].take() {
            client.close().await?;
        }
        Ok(())
    }
    async fn close(&mut self) -> Result<(), ClientError> {
        let mut errors = Vec::new();
        for index in (0..self.clients.len()).rev() {
            if let Err(error) = self.stop(index).await {
                errors.push(error.to_string());
            }
        }
        require(errors.is_empty(), "client cleanup failed")?;
        std::fs::remove_dir_all(&self.root)?;
        Ok(())
    }
}

async fn wire_deliver(
    pair: &Pair,
    offer: CustodyOffer,
    history: &ServiceBundle,
) -> Result<Option<CustodyReply>, ClientError> {
    // The original high-level source is closed first. Rebind the same durable
    // identity through public NativeNetwork APIs, without concurrent owners.
    let source = NativeNetwork::open(pair.root.join("source")).await?;
    source.admission_gate().register_protocol(
        CUSTODY_ALPN,
        miopunch_native_probe::admission_gate::ProtocolScope::Member,
    );
    let endpoint_result = source.bind_configured_endpoint(Ipv4Addr::LOCALHOST).await;
    let endpoint = match endpoint_result {
        Ok(endpoint) => endpoint,
        Err(error) => {
            source.close().await;
            return Err(error.into());
        }
    };
    let outcome = async {
        let connection = endpoint.connect(pair.at(1).address(), CUSTODY_ALPN).await?;
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            let mut stream = DataStream::open(&connection).await?;
            write_exchange(
                &mut stream,
                &CustodyRequest::Deliver {
                    offer,
                    seed: pair.card.credential().0,
                },
                history,
            )
            .await?;
            let reply: CustodyReply = match read_message(&mut stream).await {
                Ok(reply) => reply,
                Err(error) => {
                    // DataStream maps transport errors to io::Error::Other.
                    // Require the server's actual custody close, not a dial
                    // failure, timeout, parser failure or arbitrary reset.
                    let reason = connection.closed().await;
                    if matches!(reason, iroh::endpoint::ConnectionError::ApplicationClosed(reason)
                        if reason.error_code == 0u32.into()
                            && reason.reason.as_ref() == b"credential custody exchange ended")
                    {
                        return Ok(None);
                    }
                    return Err(error.into());
                }
            };
            ServiceBundle::read(&mut stream).await?;
            stream.finish()?;
            Ok::<_, ClientError>(Some(reply))
        })
        .await;
        connection.close(0u32.into(), b"owned custody effect complete");
        result?
    }
    .await;
    endpoint.close().await;
    source.close().await;
    outcome
}

#[tokio::test]
async fn stopped_custody_wire_delivery_never_restores_the_saved_root() -> Result<(), ClientError> {
    let mut pair = Pair::start("custody").await?;
    let outcome = async {
        let recipient = pair.at(1).status().await.node.node_id;
        let mut offers = Vec::new();
        let mut saved_receipt = Value::Null;
        for stage in 0..3 {
            let proposed = pair.at(0).propose_credential_custody(recipient).await?;
            let offer: CustodyOffer = serde_json::from_value(proposed["offer"].clone())?;
            pair.at(0).transfer_credential_custody(offer.id, pair.at(1).address()).await?;
            if stage != 2 { pair.at(1).accept_credential_custody(offer.id).await?; }
            if stage == 0 {
                saved_receipt = pair.at(0).transfer_credential_custody(offer.id, pair.at(1).address()).await?["saved_receipt"].clone();
                require(!saved_receipt.is_null() && pair.root.join("peer/genesis.credential").is_file(), "actual saved custody premise")?;
            }
            offers.push(offer);
        }
        let before = pair.at(1).status().await.node.members;
        let check = pair.at(1).check_other_credential_custodians(vec![pair.at(0).address()]).await?;
        require(check["confirmed_other_member"] == true, "source holding proof missing")?;
        pair.at(1).stop_credential_custody(id(&check["check"])?, false).await?;
        require(!pair.root.join("peer/genesis.credential").exists(), "stop left a saved credential")?;
        let capsule = pair.root.join("wire-source-backup");
        let prepared = pair.clients[0].take().unwrap().prepare_node_replacement(&capsule).await?;
        let backup = NodeBackup::decode(&std::fs::read_to_string(capsule)?)?;
        let history = ServiceBundle { management: backup.node.records, services: backup.node.services };
        NativeClient::abort_node_replacement(pair.root.join("source"), id(&prepared["package"]["id"])?, true)?;
        for phase in ["stopped", "reopened"] {
            if phase == "reopened" {
                pair.stop(1).await?;
                pair.clients[1] = Some(NativeClient::open(pair.root.join("peer"), Ipv4Addr::LOCALHOST).await?);
            }
            let journal_before = std::fs::read(pair.root.join("peer/credential.custody"))?;
            for (index, offer) in offers.iter().enumerate() {
                let reply = wire_deliver(&pair, offer.clone(), &history).await?;
                if index == 0 {
                    let reply = reply.ok_or("historical receipt was rejected")?;
                    require(reply.accepted && serde_json::to_value(reply.receipt)? == saved_receipt, "completed delivery lost original receipt")?;
                } else {
                    require(reply.is_none(), "stopped pending or unaccepted delivery succeeded")?;
                }
                require(!pair.root.join("peer/genesis.credential").exists(), "old wire delivery resaved credential")?;
                require(pair.at(1).credential_custody().await?["holds_genesis"] == false, "live custody disagrees with disk")?;
                require(pair.at(1).status().await.node.members == before, "delivery changed membership")?;
                require(std::fs::read(pair.root.join("peer/credential.custody"))? == journal_before, "old wire delivery changed the journal")?;
                println!("M116_CUSTODY_EFFECT {}", json!({"phase":phase,"offer_case":index,"saved_root_absent":true,"journal_unchanged":true,"historical_receipt_only":index==0}));
            }
            require(pair.at(1).accept_credential_custody(offers[1].id).await.is_err(), "old consent can be reaccepted")?;
        }
        pair.clients[0] = Some(NativeClient::open(pair.root.join("source"), Ipv4Addr::LOCALHOST).await?);
        let fresh = pair.at(0).propose_credential_custody(recipient).await?;
        let offer = id(&fresh["offer"]["id"])?;
        require(offers.iter().all(|old| old.id != offer), "fresh offer reused old identity")?;
        pair.at(0).transfer_credential_custody(offer, pair.at(1).address()).await?;
        pair.at(1).accept_credential_custody(offer).await?;
        require(pair.at(0).transfer_credential_custody(offer, pair.at(1).address()).await?["stage"] == "saved", "new explicit custody failed")?;
        require(pair.root.join("peer/genesis.credential").is_file(), "new consent did not actually save")?;
        println!("M116_CUSTODY_FRESH_CONSENT saved=true");
        Ok::<_, ClientError>(())
    }.await;
    let cleanup = pair.close().await;
    outcome?;
    cleanup
}

#[tokio::test]
async fn composed_backup_cannot_apply_policy_before_or_after_current_demotion()
-> Result<(), ClientError> {
    let mut pair = Pair::start("backup").await?;
    let outcome = async {
        let source_id = pair.at(0).status().await.node.node_id;
        let peer_id = pair.at(1).status().await.node.node_id;
        let appointment = pair.at(0).propose_device_operation(DeviceAction::AppointAdministrator, peer_id).await?;
        pair.at(0).confirm_device_operation(id(&appointment["operation"]["id"])?).await?;
        pair.at(0).sync_services(pair.at(1).address()).await?;
        let rule = ServiceRule { members:false, guests:false, allowed:Default::default(), denied:Default::default() };
        // Positive control proves this exact policy operation is meaningful.
        pair.at(0).set_service_rule(peer_id, Service::Files, None, rule.clone()).await?;
        pair.at(0).sync_services(pair.at(1).address()).await?;
        let capsule = pair.root.join("portable");
        let prepared = pair.clients[0].take().unwrap().prepare_node_replacement(&capsule).await?;
        NativeClient::abort_node_replacement(pair.root.join("source"), id(&prepared["package"]["id"])?, true)?;
        pair.stop(0).await?;
        NativeNetwork::open(pair.root.join("source")).await?.create_backup(pair.root.join("legacy")).await?;
        let demotion = pair.at(1).propose_device_operation(DeviceAction::DemoteAdministrator, source_id).await?;
        pair.at(1).confirm_device_operation_offline(id(&demotion["operation"]["id"])?, &pair.card).await?;
        require(pair.at(1).status().await.node.members.contains(&(source_id, p2panda_auth::Access::read())), "current demotion premise")?;
        NativeNetwork::restore_backup(pair.root.join("legacy"), pair.root.join("restored")).await?.create_backup(pair.root.join("rebacked")).await?;
        let imported = NativeClient::restore_node_replacement(&std::fs::read_to_string(capsule)?, pair.root.join("portable-first"), Ipv4Addr::LOCALHOST).await?;
        imported.close().await?;
        NativeNetwork::open(pair.root.join("portable-first")).await?.create_backup(pair.root.join("portable-rebacked")).await?;
        for label in ["legacy", "rebacked", "portable-rebacked"] {
            let candidate = NativeClient::open(pair.root.join(label), Ipv4Addr::LOCALHOST).await?;
            pair.clients.push(Some(candidate));
            let index = pair.clients.len()-1;
            require(pair.at(index).status().await.node.node_id == source_id, "backup identity changed")?;
            require(!pair.at(index).status().await.replacement_ready, "backup activated before qualification")?;
            let local_before = pair.at(index).service_status().await?;
            let peer_before = pair.at(1).service_status().await?;
            let allow = ServiceRule { members:true, ..rule.clone() };
            require(pair.at(index).set_service_rule(peer_id, Service::Files, None, allow.clone()).await.is_err(), "unqualified backup wrote admin policy")?;
            require(pair.at(index).service_status().await? == local_before, "denied operation changed local policies")?;
            require(pair.at(1).service_status().await? == peer_before, "denied operation changed peer policies")?;
            pair.at(index).complete_node_replacement(pair.at(1).address()).await?;
            require(pair.at(index).status().await.node.members.contains(&(source_id, p2panda_auth::Access::read())), "backup revived old administrator")?;
            let qualified_before = pair.at(index).service_status().await?;
            require(pair.at(index).set_service_rule(peer_id, Service::Files, None, allow).await.is_err(), "demoted backup wrote admin policy")?;
            require(pair.at(index).service_status().await? == qualified_before, "denied demoted operation changed policy")?;
            require(pair.at(index).ping(pair.at(1).address()).await?["reply"] == "PONG", "demotion removed ordinary business")?;
            pair.at(index).sync_services(pair.at(1).address()).await?;
            require(pair.at(1).service_status().await?["policies"] == peer_before["policies"], "denied backup policy reached peer")?;
            println!("M116_BACKUP_EFFECT {}", json!({"composition":label,"unqualified_write_denied":true,"current_demotion_applied":true,"demoted_write_denied":true,"ordinary_ping":true,"independent_peer_policy_unchanged":true}));
            pair.stop(index).await?;
        }
        Ok::<_, ClientError>(())
    }.await;
    let cleanup = pair.close().await;
    outcome?;
    cleanup
}
