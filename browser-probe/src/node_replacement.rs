//! Coherent Browser/Native portable node replacement, real storage ownership
//! and authorization quarantine. Exported text contains complete node secrets.
use super::*;
use crate::{
    node_backup::{
        NodeBackup, PortableNode, PortableSettings, ReplacementPackage, ReplacementReceipt,
        RestoredNode, SourceRuntime,
    },
    node_control_backup::{AdmissionLedger, JoinRequest},
    service_protocol::ServiceBundle,
};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct SourcePause {
    pub package: ReplacementPackage,
    settings: PortableSettings,
}

fn browser_configuration(settings: &PortableSettings) -> Result<Configuration, JsValue> {
    let mut config: Configuration = settings
        .browser
        .clone()
        .map(serde_json::from_value)
        .transpose()
        .map_err(error)?
        .unwrap_or_default();
    if settings.source == SourceRuntime::Native || settings.browser.is_none() {
        config.relays = settings.contact.relays.clone();
        config.disabled = Vec::new();
        config.mqtt = settings.contact.mqtt.clone();
        config.irc = settings
            .contact
            .irc
            .iter()
            .map(|facility| IrcConfiguration {
                facility: facility.clone(),
                profile: Default::default(),
            })
            .collect();
        config.relay_replacement = true;
        config.mqtt_replacement = true;
        config.irc_replacement = true;
        config.path_selection = settings.path_selection.clone();
        if !settings.maintenance.is_null() {
            config.maintenance =
                serde_json::from_value(settings.maintenance.clone()).map_err(error)?;
        }
        config.contact_routes.clear();
    }
    config.maintenance.validate()?;
    Ok(config)
}

impl SavedNode {
    fn node_backup(&self, source: &SourcePause) -> Result<NodeBackup, JsValue> {
        let admission = AdmissionLedger {
            codes: self.codes.clone(),
            approvals: self.approvals.clone(),
            material_generations: self.material_generations.clone(),
            own_request: if self.recovery.is_some() {
                None
            } else {
                Some(JoinRequest {
                    contact: self.invitation()?.contact,
                    request: self.request,
                })
            },
        };
        let backup = NodeBackup {
            package: source.package.clone(),
            node: PortableNode {
                key: self.key,
                network: self.invitation()?.anchor,
                records: self.records.clone(),
                pending_records: self.pending.clone(),
                authorization: self.authorization,
                generation: self.generation,
                secret: self.secret,
                genesis_seed: self.creator_seed,
                recovery_exported: self.recovery_exported,
                recovery_saved: self.recovery_saved,
                complete_invitation: self.complete_invitation.clone(),
                services: self.services.clone(),
                guest: self.guest.clone(),
                operations: self.operations.clone(),
                custody: self.custody.clone(),
                recovery: self.recovery.clone(),
                recoveries: self.served_recoveries.clone(),
                admission,
                native_rotation: self.native_rotation.clone(),
                settings: source.settings.clone(),
            },
        };
        backup.verify().map_err(error)?;
        Ok(backup)
    }
    pub(super) fn from_node_backup(backup: &NodeBackup) -> Result<Self, JsValue> {
        backup.verify().map_err(error)?;
        let node = &backup.node;
        let mut admission = node.admission.clone();
        admission.stop_restored_codes();
        let contact = admission
            .own_request
            .as_ref()
            .map(|r| r.contact)
            .or_else(|| node.recovery.as_ref().map(|r| r.contact))
            .or_else(|| node.complete_invitation.as_ref().map(|i| i.address().id))
            .or_else(|| node.guest.as_ref().map(|g| g.issuer.id))
            .unwrap_or(
                iroh_base::EndpointId::from_bytes(node.identity().as_bytes()).map_err(error)?,
            );
        let mut invitation = node.network.root.as_bytes().to_vec();
        invitation.extend_from_slice(node.network.genesis.as_bytes());
        invitation.extend_from_slice(contact.as_bytes());
        let precode = node
            .complete_invitation
            .as_ref()
            .map(|i| i.precode_bytes())
            .transpose()
            .map_err(error)?
            .flatten()
            .map(|bytes| bytes[96..].try_into().unwrap());
        let saved = Self {
            version: 1,
            key: node.key,
            invitation: Some(invitation),
            request: admission
                .own_request
                .as_ref()
                .map(|r| r.request)
                .unwrap_or_else(rand::random),
            records: node.records.clone(),
            authorization: node.authorization,
            generation: node.generation,
            secret: node.secret,
            resources: Some(browser_configuration(&node.settings)?),
            pending: node.pending_records.clone(),
            precode,
            codes: admission.codes,
            approvals: admission.approvals,
            creator_seed: node.genesis_seed,
            recovery_exported: node.recovery_exported,
            recovery_saved: node.recovery_saved,
            manual_rejected: false,
            complete_invitation: node.complete_invitation.clone(),
            services: node.services.clone(),
            guest: node.guest.clone(),
            operations: node.operations.clone(),
            custody: node.custody.clone(),
            custody_check: None,
            recovery: node.recovery.clone(),
            served_recoveries: node.recoveries.clone(),
            replacement_source: None,
            restored: Some(RestoredNode::new(backup.package.clone())),
            portable_settings: Some(node.settings.clone()),
            material_generations: admission.material_generations,
            native_rotation: node.native_rotation.clone(),
        };
        saved.verify()?;
        Ok(saved)
    }
}

#[wasm_bindgen]
impl BrowserNetwork {
    /// Explicit secret-bearing export. Services and endpoint owners are joined
    /// before the coherent snapshot; the source is durably paused before return.
    pub async fn prepare_node_replacement(&mut self) -> Result<String, JsValue> {
        self.refresh_service_snapshot().await?;
        self.require_device_ready()?;
        let result = async {
            self.disable_files().await?;
            self.stop_runtime().await;
            self.refresh_service_snapshot().await?;
            let mut settings = self
                .saved
                .portable_settings
                .clone()
                .unwrap_or(PortableSettings {
                    source: SourceRuntime::Browser,
                    native: None,
                    browser: None,
                    contact: Default::default(),
                    path_selection: Default::default(),
                    maintenance: serde_json::Value::Null,
                });
            settings.source = SourceRuntime::Browser;
            let config = self.saved.resources.clone().ok_or_else(rejected)?;
            settings.browser = Some(serde_json::to_value(&config).map_err(error)?);
            settings.contact = self.contact().1;
            settings.path_selection = config.path_selection.clone();
            settings.maintenance = serde_json::to_value(&config.maintenance).map_err(error)?;
            let source = SourcePause {
                package: ReplacementPackage::new(
                    self.saved.identity(),
                    self.saved.invitation()?.anchor,
                ),
                settings,
            };
            let text = self.saved.node_backup(&source)?.encode().map_err(error)?;
            let mut next = self.saved.clone();
            next.replacement_source = Some(source);
            self.commit(next).await?;
            Ok::<_, JsValue>(text)
        }
        .await;
        self.close().await;
        result
    }

    pub async fn retry_node_replacement_backup() -> Result<String, JsValue> {
        let lease = Lease(acquire_node().await?.as_f64().ok_or_else(rejected)? as u32);
        let result = async {
            let raw = read_node(lease.0).await?.as_string().ok_or_else(rejected)?;
            let saved: SavedNode = serde_json::from_str(&raw).map_err(error)?;
            saved.verify()?;
            let source = saved
                .replacement_source
                .as_ref()
                .ok_or_else(|| error("source is not paused for replacement"))?;
            saved.node_backup(source)?.encode().map_err(error)
        }
        .await;
        let released = release_node_and_wait(lease.0).await;
        drop(lease);
        released?;
        result
    }

    pub async fn restore_node_replacement(
        configuration: String,
        backup: String,
    ) -> Result<BrowserNetwork, JsValue> {
        let backup = NodeBackup::decode(&backup).map_err(error)?;
        Self::start_onboarding(configuration, None, Startup::Restore(backup)).await
    }

    pub async fn complete_node_replacement(&mut self, contact: String) -> Result<String, JsValue> {
        self.refresh_service_snapshot().await?;
        let restored = self
            .saved
            .restored
            .clone()
            .ok_or_else(|| error("not a restored replacement"))?;
        if let Some(receipt) = restored.receipt {
            return Ok(serde_json::json!({"ready":matches!(self.saved.stage(),"Complete"|"GuestActive"),"receipt":receipt,"reused":true}).to_string());
        }
        if self.saved.guest.is_some() {
            self.redeem_guest().await?;
        } else {
            let connection = self
                .connect_target(
                    contact.parse().map_err(error)?,
                    crate::service_policy::POLICY_ALPN,
                )
                .await?;
            let _close = RequestConnection(Some(connection.clone()));
            let mut stream = crate::data_stream::DataStream::open(&connection)
                .await
                .map_err(error)?;
            self.saved
                .service_bundle()
                .write(&mut stream)
                .await
                .map_err(error)?;
            let incoming = ServiceBundle::read(&mut stream).await.map_err(error)?;
            self.refresh_service_snapshot().await?;
            let (next, policy_error) = self.saved.with_service_bundle(&incoming)?;
            self.commit(next).await?;
            self.files.enforce_access();
            if let Some(policy_error) = policy_error {
                return Err(error(policy_error));
            }
            stream.finish().map_err(error)?;
        }
        self.refresh_service_snapshot().await?;
        let auth = self.saved.verify()?.ok_or_else(rejected)?;
        if self.saved.guest.is_none()
            && !auth
                .nodes()
                .iter()
                .any(|(id, _)| *id == self.saved.identity())
        {
            return Err(error("restored membership is no longer valid"));
        }
        let mut next = self.saved.clone();
        let state = next.restored.as_mut().ok_or_else(rejected)?;
        let receipt = ReplacementReceipt::sign(
            state.package.clone(),
            state.destination,
            auth.tips(),
            &SigningKey::from_bytes(&self.saved.key),
        )
        .map_err(error)?;
        state.receipt = Some(receipt.clone());
        self.commit(next).await?;
        Ok(serde_json::json!({"ready":matches!(self.saved.stage(),"Complete"|"GuestActive"),"receipt":receipt,"reused":false,
            "scope":"verified and saved from this contact; no global latest-state or physical single-instance claim"}).to_string())
    }

    pub fn node_replacement_status(&self) -> Result<String, JsValue> {
        if self.lease.is_none() {
            return Err(error("this node host is stopped"));
        }
        let saved = self.gate.service_snapshot()?;
        Ok(serde_json::json!({"restoration":saved.restored,
            "ready":matches!(saved.stage(),"Complete"|"GuestActive"),
            "scope":"local state; a restored snapshot requires a current authorized peer before ordinary use"}).to_string())
    }

    /// A strict replacement of the paused snapshot removes its software-held
    /// identity/root/admission/code secrets. External copies remain independent.
    pub async fn finalize_node_replacement(receipt: String) -> Result<String, JsValue> {
        let receipt: ReplacementReceipt = serde_json::from_str(&receipt).map_err(error)?;
        let lease = Lease(acquire_node().await?.as_f64().ok_or_else(rejected)? as u32);
        let result=async {
            let raw=read_node(lease.0).await?.as_string().ok_or_else(rejected)?;
            let value:serde_json::Value=serde_json::from_str(&raw).map_err(error)?;
            if value["miopunch_replacement_retired"]==true {
                let prior:ReplacementReceipt=serde_json::from_value(value["receipt"].clone()).map_err(error)?;
                if serde_json::to_vec(&prior).map_err(error)?!=serde_json::to_vec(&receipt).map_err(error)? {return Err(rejected());}
                receipt.verify(&receipt.package).map_err(error)?;
                return Ok(serde_json::json!({"source_retired":true,"already_finalized":true,"network_departure_published":false}).to_string());
            }
            let saved:SavedNode=serde_json::from_str(&raw).map_err(error)?;saved.verify()?;
            let source=saved.replacement_source.as_ref().ok_or_else(||error("source is not paused for replacement"))?;
            receipt.verify(&source.package).map_err(error)?;
            let tombstone=serde_json::json!({"miopunch_replacement_retired":true,"receipt":receipt}).to_string();
            save_node(lease.0,JsValue::from_str(&raw),tombstone).await?;
            Ok(serde_json::json!({"source_retired":true,"already_finalized":false,"network_departure_published":false}).to_string())
        }.await;
        let released = release_node_and_wait(lease.0).await;
        drop(lease);
        released?;
        result
    }

    pub async fn abort_node_replacement(
        package: String,
        target_not_activated: bool,
    ) -> Result<(), JsValue> {
        if !target_not_activated {
            return Err(error(
                "target activation must be ruled out before source resumption",
            ));
        }
        let id: Hash = package.parse().map_err(error)?;
        let lease = Lease(acquire_node().await?.as_f64().ok_or_else(rejected)? as u32);
        let result = async {
            let raw = read_node(lease.0).await?.as_string().ok_or_else(rejected)?;
            let mut saved: SavedNode = serde_json::from_str(&raw).map_err(error)?;
            saved.verify()?;
            if saved
                .replacement_source
                .as_ref()
                .is_none_or(|s| s.package.id != id)
            {
                return Err(rejected());
            }
            saved.replacement_source = None;
            save_node(
                lease.0,
                JsValue::from_str(&raw),
                serde_json::to_string(&saved).map_err(error)?,
            )
            .await?;
            Ok(())
        }
        .await;
        let released = release_node_and_wait(lease.0).await;
        drop(lease);
        released?;
        result
    }
}
