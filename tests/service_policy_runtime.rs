//! Actual Native hosts and an ordinary member carrying a signed correction.
//! No private node key, process interposition, alternate data plane or SDK hook.
use miopunch_native_probe::{
    admission_gate::ProtocolScope,
    data_stream::DataStream,
    file_transfer::{FileState, FileTaskStatus},
    management_auth::{ManagementAuth, ManagementExtension, NetworkAnchor},
    management_store::ManagementStore,
    native_client::NativeClient,
    native_network::NativeNetwork,
    recovery_card::RecoveryCard,
    service_policy::{POLICY_ALPN, now},
    service_protocol::{ManagementRecords, ServiceBundle},
};
use p2panda_auth::{
    Access, GroupsExtensionArgs,
    group::{GroupAction, GroupMember},
};
use p2panda_core::{Hash, Header, SigningKey, VerifyingKey};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fs,
    io::{Read, Write},
    net::Ipv4Addr,
    path::{Path, PathBuf},
    time::Duration,
};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
fn ensure(condition: bool, message: &str) -> Result<()> {
    if !condition {
        return Err(std::io::Error::other(message).into());
    }
    Ok(())
}
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        use std::os::unix::fs::DirBuilderExt;
        let path = std::env::temp_dir().join(format!(
            "miopunch-service-runtime-{}",
            Hash::digest(SigningKey::generate().as_bytes())
        ));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

async fn records(path: &Path, anchor: NetworkAnchor) -> Result<ManagementRecords> {
    let store = ManagementStore::open(path.join("management.sqlite"), anchor).await?;
    let records = store
        .applied_records()
        .await?
        .iter()
        .map(|record| (record.header().to_vec(), record.body().map(Vec::from)))
        .collect();
    store.close().await;
    Ok(records)
}
fn correction(
    card: &RecoveryCard,
    records: &ManagementRecords,
    administrator: VerifyingKey,
) -> Result<Vec<u8>> {
    let auth = ManagementAuth::replay_records(card.anchor(), records)?;
    let key = SigningKey::from_bytes(&card.credential().0);
    Ok(Header::builder()
        .build(
            &key,
            ManagementExtension {
                version: 1,
                log_id: *SigningKey::generate()
                    .as_bytes()
                    .first_chunk::<16>()
                    .unwrap(),
                group: GroupsExtensionArgs {
                    group_id: card.anchor().root,
                    dependencies: auth.tips(),
                    action: GroupAction::Demote {
                        member: GroupMember::Individual(administrator),
                        access: Access::read(),
                    },
                },
            },
        )
        .encode())
}
async fn joined(issuer: &NativeClient, path: &Path) -> Result<NativeClient> {
    let invitation = issuer.issue_precode(now() + 3600, Some(1)).await?;
    let node = NativeClient::join(path, &invitation, Ipv4Addr::LOCALHOST).await?;
    ensure(
        node.request_admission(&invitation).await?
            == miopunch_native_probe::manual_admission::AdmissionStage::Complete,
        "ordinary admission incomplete",
    )?;
    Ok(node)
}
async fn task(node: &NativeClient, id: &str, progress: bool) -> Result<FileTaskStatus> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        if let Some(task) = node.file_tasks().into_iter().find(|task| task.id == id) {
            if if progress {
                task.bytes > 0 && task.state == FileState::Transferring
            } else {
                task.state.terminal()
            } {
                return Ok(task);
            }
            if progress && task.state.terminal() {
                return Err(std::io::Error::other(format!(
                    "progress prerequisite ended: {:?}",
                    task.state
                ))
                .into());
            }
        }
        ensure(
            tokio::time::Instant::now() < deadline,
            "real file task did not reach the required state",
        )?;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
fn hash(path: &Path) -> Result<String> {
    let mut digest = Sha256::new();
    let mut file = fs::File::open(path)?;
    let mut buffer = vec![0; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
async fn saved(
    source: &NativeClient,
    destination: &NativeClient,
    input: &Path,
    directory: &Path,
) -> Result<()> {
    let id = source.send_file(destination.address(), input).await?;
    let sent = task(source, &id, false).await?;
    let received = task(destination, &id, false).await?;
    ensure(
        sent.state == FileState::Complete && received.receipt == sent.receipt,
        "real saved receipt missing",
    )?;
    let receipt = received
        .receipt
        .ok_or_else(|| std::io::Error::other("receiver receipt absent"))?;
    let output = directory.join(&receipt.saved_name);
    ensure(
        hash(&output)? == hash(input)? && receipt.sha256 == hash(input)?,
        "independent saved SHA256 differs",
    )?;
    fs::remove_file(output)?;
    Ok(())
}

#[derive(Default)]
struct Hosts {
    receiver: Option<NativeClient>,
    provider: Option<NativeClient>,
    carrier: Option<NativeClient>,
    carrier_network: Option<NativeNetwork>,
    carrier_resources: Option<miopunch_native_probe::resources::ResourceEndpoint>,
}
impl Hosts {
    async fn cleanup(&mut self) -> Vec<String> {
        let mut errors = vec![];
        for node in [&self.provider, &self.receiver, &self.carrier]
            .into_iter()
            .flatten()
        {
            for task in node
                .file_tasks()
                .into_iter()
                .filter(|task| !task.state.terminal())
            {
                let _ = node.cancel_file(&task.id);
            }
        }
        if let Some(resources) = self.carrier_resources.take() {
            resources.close().await;
        }
        if let Some(node) = self.carrier_network.take() {
            node.close().await;
        }
        for node in [&mut self.provider, &mut self.carrier, &mut self.receiver] {
            if let Some(node) = node.take() {
                if let Err(error) = node.close().await {
                    errors.push(error.to_string());
                }
            }
        }
        errors
    }
}
async fn native_case(hosts: &mut Hosts, directory: &Path, obsolete_policy: bool) -> Result<Value> {
    let receiver_path = directory.join("receiver");
    let carrier_path = directory.join("carrier");
    let provider_path = directory.join("provider");
    let received = directory.join("received");
    hosts.receiver = Some(NativeClient::create(&receiver_path, Ipv4Addr::LOCALHOST).await?);
    let receiver = hosts.receiver.as_ref().unwrap();
    let card = RecoveryCard::import(&receiver.export_recovery_card(None).await?, None)?;
    receiver.acknowledge_recovery_saved().await?;
    receiver.enable_files(&received).await?;
    let receiver_id = receiver.status().await.node.node_id;
    hosts.carrier = Some(joined(receiver, &carrier_path).await?);
    hosts.carrier.take().unwrap().close().await?;
    let common = records(&carrier_path, card.anchor()).await?;
    let demotion = correction(&card, &common, receiver_id)?;
    hosts.provider = Some(joined(receiver, &provider_path).await?);
    let provider = hosts.provider.as_ref().unwrap();
    provider
        .enable_files(directory.join("provider-received"))
        .await?;
    provider.sync_services(receiver.address()).await?;
    let provider_id = provider.status().await.node.node_id;
    let small = directory.join("source-small.bin");
    fs::write(&small, b"actual save")?;
    saved(provider, receiver, &small, &received).await?;
    let incoming_services = if obsolete_policy {
        provider.disable_files().await?;
        serde_json::from_slice(&fs::read(provider_path.join("services.records"))?)?
    } else {
        vec![]
    };
    hosts.carrier_network = Some(NativeNetwork::open(&carrier_path).await?);
    let network = hosts.carrier_network.as_mut().unwrap();
    network.ingest_management_record(&demotion, None).await?;
    network
        .admission_gate()
        .register_protocol(POLICY_ALPN, ProtocolScope::Member);
    hosts.carrier_resources = Some(
        network
            .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await?,
    );
    let resources = hosts.carrier_resources.as_ref().unwrap();
    let connection = resources.connect(receiver.address(), POLICY_ALPN).await?;
    let mut stream = DataStream::open(&connection).await?;
    let large = directory.join("source-live.bin");
    let block: Vec<_> = (0..65536).map(|index| (index % 251) as u8).collect();
    {
        let mut file = fs::File::create(&large)?;
        for _ in 0..8192 {
            file.write_all(&block)?;
        }
        file.sync_all()?;
    }
    let id = provider.send_file(receiver.address(), &large).await?;
    let progress = task(receiver, &id, true).await?;
    ensure(
        progress.bytes < progress.size,
        "large source completed before correction",
    )?;
    let mut management = common;
    management.push((demotion.clone(), None));
    ServiceBundle {
        management,
        services: incoming_services,
    }
    .write(&mut stream)
    .await?;
    stream.finish()?;
    let reply = ServiceBundle::read(&mut stream).await;
    connection.close(0u32.into(), b"correction probe ended");
    let status = receiver.status().await;
    ensure(
        !status.node.members.iter().any(|(id, _)| *id == provider_id),
        "valid StrongRemove correction was blocked by service history",
    )?;
    ensure(
        !status
            .node
            .members
            .contains(&(receiver_id, Access::manage())),
        "demotion did not enter the actual host",
    )?;
    ensure(
        reply.is_ok() != obsolete_policy,
        "control reply does not distinguish accepted/rejected accompanying policy",
    )?;
    let sent = task(provider, &id, false).await?;
    let stopped = task(receiver, &id, false).await?;
    ensure(
        sent.state != FileState::Complete
            && stopped.state != FileState::Complete
            && sent.receipt.is_none()
            && stopped.receipt.is_none(),
        "revoked transfer acquired a receipt",
    )?;
    ensure(
        stopped.bytes > 0
            && stopped.bytes < stopped.size
            && fs::read_dir(&received)?.next().is_none(),
        "real stopped transfer retained saved or partial bytes",
    )?;
    ensure(
        receiver.files_enabled(),
        "revocation disabled the entire service",
    )?;
    hosts.carrier_resources.take().unwrap().close().await;
    hosts.carrier_network.take().unwrap().close().await;
    hosts.carrier = Some(NativeClient::open(&carrier_path, Ipv4Addr::LOCALHOST).await?);
    saved(hosts.carrier.as_ref().unwrap(), receiver, &small, &received).await?;
    Ok(
        json!({"case":if obsolete_policy {"incoming_obsolete_policy"} else {"retained_declaration"},
        "actualBytesBeforeCorrection":progress.bytes,"actualBytesAtStop":stopped.bytes,"noReceipt":true,
        "independentDirectoryEmpty":true,"serviceStillEnabled":true,"baselineAndPostCorrectionSavedHashes":true,
        "managementSavedDespitePolicyRejection":obsolete_policy,"controlReplyAccepted":reply.is_ok()}),
    )
}
async fn run_native(obsolete: bool) {
    let directory = Directory::new();
    let mut hosts = Hosts::default();
    let result = native_case(&mut hosts, &directory.0, obsolete).await;
    let cleanup = hosts.cleanup().await;
    println!(
        "{}",
        json!({"runtimeQualification":result.as_ref().ok(),"error":result.as_ref().err().map(ToString::to_string),"cleanupErrors":cleanup})
    );
    assert!(cleanup.is_empty(), "owned Native host cleanup failed");
    assert!(result.is_ok(), "{}", result.err().unwrap());
}
#[tokio::test]
async fn native_retained_declaration_cannot_block_a_live_strong_remove() {
    run_native(false).await;
}
#[tokio::test]
async fn native_rejected_policy_cannot_block_a_live_strong_remove() {
    run_native(true).await;
}

struct SmallSource(Vec<u8>);
impl miopunch_native_probe::file_transfer::FileSource for SmallSource {
    fn read_at(
        &mut self,
        offset: u64,
        length: usize,
    ) -> miopunch_native_probe::file_transfer::FileFuture<'_, Vec<u8>> {
        Box::pin(async move { Ok(self.0[offset as usize..offset as usize + length].to_vec()) })
    }
}

async fn metadata_case(hosts: &mut Hosts, directory: &Path) -> Result<Value> {
    use miopunch_native_probe::file_transfer::{FILE_ALPN, FileError, FileOffer, send_file};
    let received = directory.join("received");
    let peer_received = directory.join("peer-received");
    let peer_path = directory.join("peer");
    let requester_path = directory.join("requester");
    hosts.receiver =
        Some(NativeClient::create(directory.join("receiver"), Ipv4Addr::LOCALHOST).await?);
    let receiver = hosts.receiver.as_ref().unwrap();
    receiver.export_recovery_card(None).await?;
    receiver.acknowledge_recovery_saved().await?;
    receiver.enable_files(&received).await?;
    hosts.carrier = Some(joined(receiver, &peer_path).await?);
    hosts
        .carrier
        .as_ref()
        .unwrap()
        .enable_files(&peer_received)
        .await?;
    hosts.provider = Some(joined(receiver, &requester_path).await?);
    hosts.provider.take().unwrap().close().await?;
    hosts.carrier_network = Some(NativeNetwork::open(&requester_path).await?);
    let network = hosts.carrier_network.as_ref().unwrap();
    network
        .admission_gate()
        .register_protocol(FILE_ALPN, ProtocolScope::Member);
    hosts.carrier_resources = Some(
        network
            .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await?,
    );
    let source = directory.join("source-small.bin");
    fs::write(&source, b"actual save")?;
    let original_id = hosts
        .carrier
        .as_ref()
        .unwrap()
        .send_file(receiver.address(), &source)
        .await?;
    task(hosts.carrier.as_ref().unwrap(), &original_id, false).await?;
    let original = task(receiver, &original_id, false)
        .await?
        .receipt
        .ok_or_else(|| std::io::Error::other("original committed receipt absent"))?;
    let original_path = received.join(&original.saved_name);
    ensure(
        hash(&original_path)? == hash(&source)?,
        "original saved bytes differ",
    )?;
    // Outgoing real saves evict the receive summary without creating another
    // incoming partial at the provider. Every delivery is independently read.
    for _ in 0..63 {
        saved(
            receiver,
            hosts.carrier.as_ref().unwrap(),
            &source,
            &peer_received,
        )
        .await?;
    }
    // Establish the independent authorized requester before the eviction.
    // Opening another identity after eviction can let ordinary maintenance
    // remove the completed cache before this negative-case prerequisite.
    let connection = hosts
        .carrier_resources
        .as_ref()
        .unwrap()
        .connect(receiver.address(), FILE_ALPN)
        .await?;
    let mut stream = DataStream::open(&connection).await?;
    let last_delivery = receiver
        .send_file(hosts.carrier.as_ref().unwrap().address(), &source)
        .await?;
    ensure(
        !receiver
            .file_tasks()
            .iter()
            .any(|task| task.id == original_id),
        "old receive summary was not evicted",
    )?;
    let offer = FileOffer {
        id: original_id.clone(),
        name: "different-name.bin".into(),
        size: 11,
        sha256: hash(&source)?,
    };
    let transfer = send_file(
        &mut stream,
        &offer,
        &mut SmallSource(fs::read(&source)?),
        |_, _| {},
    )
    .await;
    connection.close(0u32.into(), b"metadata case ended");
    let sent = task(receiver, &last_delivery, false).await?;
    let delivered = task(hosts.carrier.as_ref().unwrap(), &last_delivery, false).await?;
    ensure(
        sent.state == FileState::Complete && sent.receipt == delivered.receipt,
        "last summary-evicting delivery was not saved",
    )?;
    let delivered_path = peer_received.join(&delivered.receipt.unwrap().saved_name);
    ensure(
        hash(&delivered_path)? == hash(&source)?,
        "last summary-evicting saved hash differs",
    )?;
    fs::remove_file(delivered_path)?;
    let retained_negative = match &transfer {
        Ok(_) => false,
        Err(FileError::Rejected) => true,
        Err(error) => {
            return Err(std::io::Error::other(format!(
                "mismatched offer failed without a retained-cache rejection: {error}"
            ))
            .into());
        }
    };
    let status = task(receiver, &original_id, false).await?;
    if retained_negative {
        ensure(
            status.state == FileState::Failed
                && status.error == Some(FileError::Rejected)
                && status.receipt.is_none(),
            "mismatched offer was not a failed new task",
        )?;
    } else {
        // Once both bounded summaries and completed cache have retired this
        // ID, it is a new transfer. Its new filename/receipt must match actual
        // new saved bytes; it must never adopt the earlier saved receipt.
        let receipt = transfer.as_ref().unwrap();
        ensure(
            status.state == FileState::Complete
                && status.receipt.as_ref() == Some(receipt)
                && receipt.saved_name == offer.saved_name(),
            "retired cache supplied a mismatched receipt",
        )?;
        ensure(
            hash(&received.join(&receipt.saved_name))? == hash(&source)?,
            "fresh post-retirement saved hash differs",
        )?;
    }
    receiver.disable_files().await?;
    let status = receiver
        .file_tasks()
        .into_iter()
        .find(|task| task.id == original_id)
        .unwrap();
    if retained_negative {
        ensure(
            status.state == FileState::Failed
                && status.error == Some(FileError::Rejected)
                && status.receipt.is_none(),
            "disable cleanup rewrote a mismatched new task using the old committed receipt",
        )?;
    } else {
        ensure(
            status.state == FileState::Complete
                && status.receipt.as_ref() == transfer.as_ref().ok(),
            "disable cleanup rewrote a fresh post-retirement receipt",
        )?;
        ensure(
            hash(&received.join(&offer.saved_name()))? == hash(&source)?,
            "disable removed a new committed file",
        )?;
    }
    ensure(
        hash(&original_path)? == hash(&source)?,
        "mismatched request or disable removed the committed file",
    )?;
    Ok(
        json!({"actualSavedDeliveries":if retained_negative {65} else {66},"allSavedHashesIndependent":true,"evictedSummary":true,
        "retainedCacheNegativeObserved":retained_negative,"retainedNegativeError":transfer.as_ref().err().map(ToString::to_string),
        "currentResultSurvivedDisable":true,"originalSavedFilePreserved":true}),
    )
}

#[tokio::test]
async fn completed_cache_retirement_preserves_current_task_and_saved_files() {
    let directory = Directory::new();
    let mut hosts = Hosts::default();
    let result = metadata_case(&mut hosts, &directory.0).await;
    let cleanup = hosts.cleanup().await;
    println!(
        "{}",
        json!({"metadataQualification":result.as_ref().ok(),"error":result.as_ref().err().map(ToString::to_string),"cleanupErrors":cleanup})
    );
    assert!(cleanup.is_empty());
    assert!(result.is_ok(), "{}", result.err().unwrap());
}

#[tokio::test]
#[ignore = "ordinary carrier for an externally orchestrated real Browser/Native receiver"]
async fn external_correction_carrier() {
    let path = std::env::var_os("MIOPUNCH_CORRECTION_CONFIG").expect("private owned config path");
    let config: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let node_path = PathBuf::from(config["carrierNode"].as_str().unwrap());
    let card = RecoveryCard::import(
        &fs::read_to_string(config["recoveryCard"].as_str().unwrap()).unwrap(),
        None,
    )
    .unwrap();
    let common = records(&node_path, card.anchor()).await.unwrap();
    let administrator = config["administrator"].as_str().unwrap().parse().unwrap();
    let demotion = correction(&card, &common, administrator).unwrap();
    let mut node = NativeNetwork::open(&node_path).await.unwrap();
    node.ingest_management_record(&demotion, None)
        .await
        .unwrap();
    node.admission_gate()
        .register_protocol(POLICY_ALPN, ProtocolScope::Member);
    let resources = node
        .bind_configured_endpoint(Ipv4Addr::UNSPECIFIED)
        .await
        .unwrap();
    let address = serde_json::from_value::<iroh::EndpointAddr>(config["receiver"].clone()).unwrap();
    let connection = resources.connect(address, POLICY_ALPN).await.unwrap();
    let mut stream = DataStream::open(&connection).await.unwrap();
    let mut management = common;
    management.push((demotion, None));
    let services = config["incomingServices"]
        .as_str()
        .map(|path| serde_json::from_slice(&fs::read(path).unwrap()).unwrap())
        .unwrap_or_default();
    let sent = ServiceBundle {
        management,
        services,
    }
    .write(&mut stream)
    .await;
    if sent.is_ok() {
        let _ = stream.finish();
    }
    let reply = if sent.is_ok() {
        ServiceBundle::read(&mut stream).await.ok()
    } else {
        None
    };
    connection.close(0u32.into(), b"external correction carrier ended");
    resources.close().await;
    node.close().await;
    fs::write(config["report"].as_str().unwrap(), json!({"sent":sent.is_ok(),"replyAccepted":reply.is_some(),"ordinaryMemberCarrier":true,"originalSignedCorrection":true}).to_string()).unwrap();
    assert!(sent.is_ok(), "signed correction could not be delivered");
}
