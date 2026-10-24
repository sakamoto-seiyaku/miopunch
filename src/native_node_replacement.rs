//! Stop/export/import/qualify/finalize a complete logical node. An imported
//! snapshot cannot start ordinary service until a current peer exchange has
//! verified its effective authorization. Source resumption is always explicit.
use std::{collections::BTreeSet, net::Ipv4Addr, path::Path, sync::atomic::Ordering};

use iroh::EndpointAddr;
use p2panda_core::{Hash, SigningKey};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    admission_proof::{AdmissionScope, AdmissionSecret},
    data_stream::DataStream,
    native_client::{ClientError, NativeClient},
    native_network::NativeNetwork,
    node_backup::{
        NodeBackup, PortableNode, PortableSettings, ReplacementPackage, ReplacementReceipt,
        RestoredNode,
    },
    resources::{Resource, ResourceConfig, ResourceUse},
    service_protocol::ServiceBundle,
};

const SOURCE: &str = "replacement.source";
const RESTORED: &str = "replacement.restore";
const CODES_STOPPED: &str = "replacement.precodes-stopped";
const RETIRED: &str = "replacement.retired";
const SETTINGS: &str = "replacement.settings";

#[derive(Serialize, Deserialize)]
struct SourcePause {
    package: ReplacementPackage,
    settings: PortableSettings,
}

fn restored(node: &NativeNetwork) -> Result<Option<RestoredNode>, ClientError> {
    let state: RestoredNode = match std::fs::read(node.directory.join(RESTORED)) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Older known backup directories predate the restore marker.
            // Opening one must establish the same durable quarantine as an
            // explicit import, rather than treating its snapshot as current.
            let version = match std::fs::read(node.directory.join("backup.version")) {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(e.into()),
            };
            if version != b"miopunch node backup 1\n" {
                return Err("invalid node backup version".into());
            }
            let state =
                RestoredNode::new(ReplacementPackage::new(node.status().node_id, node.anchor));
            crate::native_files::write_private_atomic(
                &node.directory.join(RESTORED),
                &serde_json::to_vec(&state)?,
            )?;
            return Ok(Some(state));
        }
        Err(e) => return Err(e.into()),
    };
    state.verify(node.status().node_id, node.anchor)?;
    Ok(Some(state))
}
/// Commit restoration safety before exposing either public Native opener.
/// Return whether current-peer qualification has already completed.
pub(crate) async fn prepare_for_open(node: &NativeNetwork) -> Result<bool, ClientError> {
    let Some(state) = restored(node)? else {
        return Ok(true);
    };
    let stopped = match std::fs::read(node.directory.join(CODES_STOPPED)) {
        Ok(bytes) => bytes.as_slice() == state.package.id.as_bytes(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(e.into()),
    };
    if !stopped {
        // Signed peer history cannot establish continuity of private issuer
        // counters. Stop every inherited code before any qualification can
        // enable this copy, including older copies with an existing receipt.
        sqlx::query("UPDATE precodes SET stopped=1")
            .execute(&node.store.pool)
            .await?;
        // Publish only after the durable database update. This package-bound
        // local marker lets later fresh codes survive normal qualified reopen.
        crate::native_files::write_private_atomic(
            &node.directory.join(CODES_STOPPED),
            state.package.id.as_bytes(),
        )?;
    }
    Ok(state.receipt.is_some())
}

async fn export_paused(source: &Path, destination: &Path) -> Result<Value, ClientError> {
    let pause: SourcePause = serde_json::from_slice(&std::fs::read(source.join(SOURCE))?)?;
    let node = NativeNetwork::open_migration_context(source).await?;
    let result = async {
        if node.status().node_id != pause.package.identity || node.anchor != pause.package.network {
            return Err("paused replacement identity changed".into());
        }
        let services = crate::native_services::NativeServices::load(&node).await?;
        let bundle = services.snapshot(&node).await?;
        let auth = services.access(&node).await?.auth;
        let root = if node.directory.join("genesis.credential").exists() {
            Some(*node.saved_genesis_credential()?.root.as_bytes())
        } else { None };
        let recovery: Value = match std::fs::read(node.directory.join("client.recovery-save")) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({"exported":false,"saved":false}),
            Err(e) => return Err(e.into()),
        };
        let invitation = match std::fs::read(node.directory.join("client.invitation")) {
            Ok(bytes) => Some(crate::client_invitation::ClientInvitation::decode(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        let operations = read_or_default(&node.directory.join("device.operations"))?;
        let custody = read_or_default(&node.directory.join("credential.custody"))?;
        let recoveries=node.store.export_recovery_evidence().await?;
        let (admission,native_rotation)=crate::native_control_backup::export(&node).await?;
        let pending_records=node.store.unapplied_records().await?;
        let recovery_intent=node.recovery.map(|binding| {
            let proof=recoveries.iter().find(|p|p.target==node.status().node_id && p.attempt==binding.attempt);
            crate::management_recovery_wire::RecoveryIntent {contact:binding.contact,attempt:binding.attempt,
                authorization:proof.and_then(|p|p.authorization),complete:proof.is_some_and(|p|p.phase==3)}
        });
        let backup = NodeBackup { package: pause.package.clone(), node: PortableNode {
            key: *node.node_key.as_bytes(), network: node.anchor, records: bundle.management,pending_records,
            authorization: auth.authorization_origin_at(node.status().node_id, &auth.tips(), false),
            generation: node.scope.secret_generation(), secret: node.secret.as_ref().map(|s| *s.saved_bytes()),
            genesis_seed: root, recovery_exported: recovery["exported"].as_bool().unwrap_or(false),
            recovery_saved: recovery["saved"].as_bool().unwrap_or(false), complete_invitation: invitation,
            services: bundle.services, guest: services.guest, operations, custody,
            recovery:recovery_intent,recoveries,admission,native_rotation,settings: pause.settings,
        }};
        let text = backup.encode()?;
        crate::native_network::write_private(destination, text.as_bytes())?;
        crate::native_files::sync_directory(destination.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")))?;
        Ok::<_, ClientError>(json!({"package":backup.package,"saved":true,"source_stopped":true,
            "warning":"complete node backup contains identity, admission and any held genesis secrets; deliver privately"}))
    }.await;
    node.close().await;
    result
}

fn read_or_default<T: serde::de::DeserializeOwned + Default>(
    path: &Path,
) -> Result<T, ClientError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e.into()),
    }
}

impl NativeClient {
    /// Pause this source, join all services/endpoints, then export one coherent
    /// logical snapshot. This consumes the running host and publishes no Leave.
    pub async fn prepare_node_replacement(
        self,
        destination: impl AsRef<Path>,
    ) -> Result<Value, ClientError> {
        let destination = destination.as_ref();
        let prepared = async {
            self.require_ready()?;
            if destination.symlink_metadata().is_ok() {
                return Err("backup destination already exists".into());
            }
            let node = self.node.lock().await;
            node.store.ensure_admissions_open().await?;
            let mut settings: PortableSettings =
                read_or_default::<Option<PortableSettings>>(&node.directory.join(SETTINGS))?
                    .unwrap_or(PortableSettings {
                        source: crate::node_backup::SourceRuntime::Native,
                        native: None,
                        browser: None,
                        contact: Default::default(),
                        path_selection: Default::default(),
                        maintenance: Value::Null,
                    });
            let config = node.resource_configuration()?;
            settings.source = crate::node_backup::SourceRuntime::Native;
            settings.native = Some(serde_json::to_value(&config)?);
            settings.contact = self.resources.invitation_contact()?.1;
            settings.path_selection = config.path_selection.clone();
            settings.maintenance = serde_json::to_value(&config.maintenance)?;
            let pause = SourcePause {
                package: ReplacementPackage::new(node.status().node_id, node.anchor),
                settings,
            };
            crate::native_files::write_private_atomic(
                &node.directory.join(SOURCE),
                &serde_json::to_vec(&pause)?,
            )?;
            Ok::<_, ClientError>((node.directory.clone(), pause))
        }
        .await;
        // A failed close/export leaves a paused, retained source. Neither side
        // is unconditionally reopened after a partial handoff.
        let closed = self.close().await;
        let (source, pause) = prepared?;
        closed?;
        let result = export_paused(&source, destination).await?;
        debug_assert_eq!(result["package"]["id"], pause.package.id.to_string());
        Ok(result)
    }

    /// Resume only the export of an already stopped source, without running
    /// its network identity. A failed destination save can be retried safely.
    pub async fn retry_node_replacement_backup(
        source: impl AsRef<Path>,
        destination: impl AsRef<Path>,
    ) -> Result<Value, ClientError> {
        export_paused(source.as_ref(), destination.as_ref()).await
    }

    pub async fn restore_node_replacement(
        backup: &str,
        target: impl AsRef<Path>,
        bind: Ipv4Addr,
    ) -> Result<Self, ClientError> {
        let backup = NodeBackup::decode(backup)?;
        let node = import_native(&backup, target.as_ref()).await?;
        Self::start(node, bind).await
    }

    /// Complete only after an ordinary current peer has supplied/verified the
    /// current authorization view. A revoked old snapshot stays blocked.
    pub async fn complete_node_replacement(
        &self,
        contact: EndpointAddr,
    ) -> Result<Value, ClientError> {
        let state = {
            let node = self.node.lock().await;
            restored(&node)?.ok_or("not a restored replacement")?
        };
        if let Some(receipt) = state.receipt {
            return Ok(json!({"ready":true,"receipt":receipt,"reused":true}));
        }
        let guest = self.services.lock().await.guest.is_some();
        if guest {
            self.redeem_guest().await?;
        } else {
            let connection = self
                .resources
                .connect(contact, crate::service_policy::POLICY_ALPN)
                .await?;
            let result = async {
                let mut stream = DataStream::open(&connection).await?;
                let snapshot = {
                    let node = self.node.lock().await;
                    self.services.lock().await.snapshot(&node).await?
                };
                snapshot.write(&mut stream).await?;
                let remote = ServiceBundle::read(&mut stream).await?;
                let mut node = self.node.lock().await;
                self.services
                    .lock()
                    .await
                    .merge(&mut node, &self.files, remote)
                    .await?;
                stream.finish()?;
                Ok::<_, ClientError>(())
            }
            .await;
            connection.close(0u32.into(), b"replacement authorization check ended");
            result?;
        }
        let node = self.node.lock().await;
        let access = self.services.lock().await.access(&node).await?;
        if !guest && !access.member(access.local) {
            return Err("restored membership is no longer valid".into());
        }
        let mut state = restored(&node)?.ok_or("replacement state disappeared")?;
        let receipt = ReplacementReceipt::sign(
            state.package.clone(),
            state.destination,
            access.auth.tips(),
            &node.node_key,
        )?;
        state.receipt = Some(receipt.clone());
        crate::native_files::write_private_atomic(
            &node.directory.join(RESTORED),
            &serde_json::to_vec(&state)?,
        )?;
        self.replacement_ready.store(true, Ordering::Release);
        Ok(json!({"ready":true,"receipt":receipt,"reused":false,
            "scope":"verified and saved from this contact; no global latest-state or physical single-instance claim"}))
    }

    pub async fn node_replacement_status(&self) -> Result<Value, ClientError> {
        let node = self.node.lock().await;
        Ok(
            json!({"restoration":restored(&node)?,"ready":self.replacement_ready.load(Ordering::Acquire),
            "scope":"local state; a restored snapshot requires a current authorized peer before ordinary use"}),
        )
    }

    /// The target's signed success receipt authorizes cleanup of this paused
    /// source's known node materials. It never signs Leave/Remove.
    pub fn finalize_node_replacement(
        source: impl AsRef<Path>,
        receipt: &ReplacementReceipt,
    ) -> Result<Value, ClientError> {
        let source = source.as_ref();
        let mut already_finalized = false;
        if let Ok(bytes) = std::fs::read(source.join(RETIRED)) {
            let prior: ReplacementReceipt = serde_json::from_slice(&bytes)?;
            if serde_json::to_vec(&prior)? != serde_json::to_vec(receipt)? {
                return Err("another replacement already finalized".into());
            }
            already_finalized = true;
        }
        let pause: SourcePause = serde_json::from_slice(&std::fs::read(source.join(SOURCE))?)?;
        receipt.verify(&pause.package)?;
        if !already_finalized {
            let key: [u8; 32] = std::fs::read(source.join("node.key"))?
                .try_into()
                .map_err(|_| "invalid source key")?;
            if SigningKey::from_bytes(&key).verifying_key() != pause.package.identity {
                return Err("source identity changed".into());
            }
        }
        // Publish the non-secret tombstone first. Retrying cleanup remains
        // possible after a partial delete, while ordinary open stays blocked.
        crate::native_files::write_private_atomic(
            &source.join(RETIRED),
            &serde_json::to_vec(receipt)?,
        )?;
        let mut removed = Vec::new();
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if owned_node_file(&name) {
                std::fs::remove_file(entry.path())?;
                removed.push(name);
            }
        }
        crate::native_files::sync_directory(source)?;
        Ok(
            json!({"source_retired":true,"already_finalized":already_finalized,"removed_material_files":removed,
            "network_departure_published":false,"target_identity":pause.package.identity}),
        )
    }

    /// Explicit failure recovery. The caller confirms that no restored target
    /// has been activated; this does not establish a global single-instance lock.
    pub fn abort_node_replacement(
        source: impl AsRef<Path>,
        package: Hash,
        target_not_activated: bool,
    ) -> Result<(), ClientError> {
        let source = source.as_ref();
        if !target_not_activated || source.join(RETIRED).exists() {
            return Err("target activation must be ruled out before source resumption".into());
        }
        let pause: SourcePause = serde_json::from_slice(&std::fs::read(source.join(SOURCE))?)?;
        if pause.package.id != package {
            return Err("different replacement operation".into());
        }
        std::fs::remove_file(source.join(SOURCE))?;
        crate::native_files::sync_directory(source)?;
        Ok(())
    }
}

fn owned_node_file(name: &str) -> bool {
    matches!(
        name,
        "node.key"
            | "genesis.credential"
            | "network.anchor"
            | "admission.secret"
            | "management.sqlite"
            | "management.sqlite-wal"
            | "management.sqlite-shm"
            | "join.invitation"
            | "admission.material"
            | "recovery.binding"
            | "resources.json"
            | "maintenance.contacts.json"
            | "services.records"
            | "services.guest-invitation"
            | "client.invitation"
            | "client.recovery-save"
            | "device.operations"
            | "credential.custody"
            | "credential.custody-check"
            | "replacement.package"
            | "replacement.restore"
            | "replacement.precodes-stopped"
            | "replacement.settings"
    ) || name
        .strip_prefix(".mio-")
        .is_some_and(|n| n.len() == 32 && u128::from_str_radix(n, 16).is_ok())
        || name
            .strip_prefix("rotation.material.")
            .is_some_and(|n| n.len() == 16 && u64::from_str_radix(n, 16).is_ok())
}

fn native_configuration(settings: &PortableSettings) -> Result<ResourceConfig, ClientError> {
    let mut config: ResourceConfig = settings
        .native
        .clone()
        .map(serde_json::from_value)
        .transpose()?
        .unwrap_or_default();
    if settings.native.is_none() || settings.source == crate::node_backup::SourceRuntime::Browser {
        // Keep effective Native-only resources. The Browser's current common
        // contact policy becomes an explicit allow-list rather than falling
        // back to old defaults or an earlier Native permission snapshot.
        let browser_disabled = settings
            .browser
            .as_ref()
            .and_then(|v| v["disabled"].as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        let selected = config
            .replacement
            .as_ref()
            .unwrap_or(&config.defaults.resources);
        let mut native_only = Vec::new();
        for resource in selected {
            if browser_disabled.contains(&resource.url) {
                continue;
            }
            let mut uses = BTreeSet::new();
            if resource.uses.contains(&ResourceUse::Observation)
                && config.allows(&resource.url, ResourceUse::Observation)
            {
                uses.insert(ResourceUse::Observation);
            }
            if resource.uses.contains(&ResourceUse::Lookup)
                && config.allows(&resource.url, ResourceUse::Lookup)
            {
                uses.insert(ResourceUse::Lookup);
            }
            let mapped = config
                .mqtt_facilities
                .iter()
                .any(|f| f.native_url == resource.url)
                || config
                    .irc_facilities
                    .iter()
                    .any(|f| f.native_url == resource.url);
            if !mapped
                && resource.uses.contains(&ResourceUse::Signaling)
                && config.allows(&resource.url, ResourceUse::Signaling)
            {
                uses.insert(ResourceUse::Signaling);
            }
            if !uses.is_empty() {
                native_only.push(Resource {
                    url: resource.url.clone(),
                    uses,
                    limitation: resource.limitation.clone(),
                });
            }
        }
        let mut resources = settings
            .contact
            .relays
            .iter()
            .map(|url| Resource {
                url: url.clone(),
                uses: BTreeSet::from([ResourceUse::DataRelay]),
                limitation: "restored effective relay permission".into(),
            })
            .collect::<Vec<_>>();
        resources.extend(settings.contact.mqtt.iter().map(|f| Resource {
            url: f.native_url.clone(),
            uses: BTreeSet::from([ResourceUse::Signaling]),
            limitation: "restored trusted contact".into(),
        }));
        resources.extend(settings.contact.irc.iter().map(|f| Resource {
            url: f.native_url.clone(),
            uses: BTreeSet::from([ResourceUse::Signaling]),
            limitation: "restored trusted contact".into(),
        }));
        resources.extend(native_only);
        config.replacement = Some(resources);
        config
            .disabled
            .retain(|(_, usage)| matches!(usage, ResourceUse::Observation | ResourceUse::Lookup));
        config
            .disabled_uses
            .retain(|usage| matches!(usage, ResourceUse::Observation | ResourceUse::Lookup));
        // Replace inherited Native contact permissions with the Browser's
        // current policy, then retain its explicit denials for every usage.
        for value in browser_disabled {
            for usage in [
                ResourceUse::Observation,
                ResourceUse::Lookup,
                ResourceUse::Signaling,
                ResourceUse::DataRelay,
            ] {
                config.disabled.insert((value.clone(), usage));
            }
        }
        config.mqtt_facilities = settings.contact.mqtt.clone();
        config.irc_facilities = settings.contact.irc.clone();
        config.path_selection = settings.path_selection.clone();
        if !settings.maintenance.is_null() {
            config.maintenance = serde_json::from_value(settings.maintenance.clone())?;
        }
    }
    config.validate_contact_facilities()?;
    Ok(config)
}

async fn import_native(backup: &NodeBackup, target: &Path) -> Result<NativeNetwork, ClientError> {
    backup.verify()?;
    if target.symlink_metadata().is_ok() {
        return Err("replacement target already exists".into());
    }
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let staging = parent.join(format!(".miopunch-restore-{:032x}", rand::random::<u128>()));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&staging)?;
    let result = async {
        let n = &backup.node;
        let write = |name: &str, bytes: &[u8]| crate::native_network::write_private(&staging.join(name),bytes);
        write("node.key", &n.key)?;
        let mut anchor = n.network.root.as_bytes().to_vec(); anchor.extend_from_slice(n.network.genesis.as_bytes());
        write("network.anchor", &anchor)?;
        let contact = n.complete_invitation.as_ref().map(|i| i.address().id)
            .or_else(|| n.guest.as_ref().map(|g| g.issuer.id))
            .unwrap_or(iroh_base::EndpointId::from_bytes(n.identity().as_bytes())?);
        if n.recovery.is_none() {
            let own=n.admission.own_request.as_ref();
            let binding = crate::manual_admission::JoinBinding { invitation:crate::manual_admission::ManualInvitation {anchor:n.network,contact:own.map(|r|r.contact).unwrap_or(contact)},request_id:own.map(|r|r.request).unwrap_or_else(rand::random) };
            write("join.invitation", &binding.encode())?;
        }
        if let Some(seed) = n.genesis_seed { let mut bytes = seed.to_vec();bytes.extend_from_slice(n.network.genesis.as_bytes());write("genesis.credential", &bytes)?; }
        let mut store = crate::management_store::ManagementStore::open(staging.join("management.sqlite"),n.network).await?;
        let store_result = async {
            for (header,body) in &n.records { store.ingest(&crate::management_record::ManagementRecord::new(header.clone(),body.as_deref())).await?; }
            store.apply_ready().await?;
            for (header,body) in &n.pending_records {store.ingest(&crate::management_record::ManagementRecord::new(header.clone(),body.as_deref())).await?;}
            store.apply_ready().await?;
            store.import_recovery_evidence(&n.recoveries).await?;
            crate::native_control_backup::import(&store,&staging,&n.admission,&n.native_rotation).await?;
            if let Some(secret) = n.secret {
                let scope=AdmissionScope::new(*n.network.root.as_bytes(),n.generation);
                let secret=AdmissionSecret::from_saved(secret);
                store.set_current_admission(&scope,&secret).await?;
                let mut bytes=n.generation.to_be_bytes().to_vec();bytes.extend_from_slice(secret.saved_bytes());write("admission.material",&bytes)?;
            }
            Ok::<_,ClientError>(())
        }.await;
        // No old precode counters are imported into a redeemable code table.
        // Signed membership/authorization remains; newly issued codes work.
        store.close().await;
        store_result?;
        if let Some(intent)=&n.recovery {
            let binding=crate::management_recovery::RecoveryBinding {contact:intent.contact,attempt:intent.attempt};
            write("recovery.binding",&binding.encode())?;
        }
        write("resources.json", &serde_json::to_vec(&native_configuration(&n.settings)?)?)?;
        write(SETTINGS, &serde_json::to_vec(&n.settings)?)?;
        write("services.records", &serde_json::to_vec(&n.services)?)?;
        if let Some(guest)=&n.guest { write("services.guest-invitation",&guest.encode()?)?; }
        if let Some(invitation)=&n.complete_invitation { write("client.invitation",&invitation.encode()?)?; }
        write("client.recovery-save",&serde_json::to_vec(&json!({"genesis":n.network.genesis,"exported":n.recovery_exported,"saved":n.recovery_saved}))?)?;
        write("device.operations",&serde_json::to_vec(&n.operations)?)?;
        write("credential.custody",&serde_json::to_vec(&n.custody)?)?;
        write(RESTORED,&serde_json::to_vec(&RestoredNode::new(backup.package.clone()))?)?;
        let node=NativeNetwork::open_migration_context(&staging).await?;
        let validation = async {
            crate::native_services::NativeServices::load(&node).await?;
            crate::native_device_management::validate_saved_operations(&staging,&node)?;
            crate::native_custody::validate(&node)?;
            Ok::<_,ClientError>(())
        }.await;
        node.close().await;
        validation?;
        crate::native_files::sync_directory(&staging)?;
        crate::native_network::publish_new_directory(&staging,target,parent)?;
        crate::native_files::sync_directory(parent)?;
        Ok::<_,ClientError>(())
    }.await;
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result?;
    Ok(NativeNetwork::open(target).await?)
}
