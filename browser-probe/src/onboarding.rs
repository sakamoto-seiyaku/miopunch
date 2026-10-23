//! Explicit product onboarding over the existing leased browser node. Legacy
//! `start`, raw invitations and management APIs retain their original contract.
use super::*;
use crate::client_invitation::{BootstrapContact, ClientInvitation};
use crate::invitation_pairing::{PAIRING_ALPN, PairingInfo};
use crate::management_auth::ManagementExtension;
use p2panda_auth::GroupsExtensionArgs;
use p2panda_auth::group::{GroupAction, GroupMember};
use p2panda_core::{Header, SigningKey};

impl SavedNode {
    pub(super) fn create(&mut self) -> Result<(), JsValue> {
        if self.invitation.is_some() || !self.records.is_empty() {
            return Err(error("a network is already selected"));
        }
        let root = SigningKey::generate();
        let header = Header::<ManagementExtension>::builder()
            .build(
                &root,
                ManagementExtension {
                    version: 1,
                    log_id: rand::random(),
                    group: GroupsExtensionArgs {
                        group_id: root.verifying_key(),
                        action: GroupAction::Create {
                            initial_members: vec![
                                (
                                    GroupMember::Individual(root.verifying_key()),
                                    Access::manage(),
                                ),
                                (GroupMember::Individual(self.identity()), Access::manage()),
                            ],
                        },
                        dependencies: Vec::new(),
                    },
                },
            )
            .encode();
        let genesis = Hash::digest(&header);
        let mut material = root.verifying_key().as_bytes().to_vec();
        material.extend_from_slice(genesis.as_bytes());
        material.extend_from_slice(self.identity().as_bytes());
        self.invitation = Some(material);
        self.records = vec![(header, None)];
        self.authorization = Some(genesis);
        self.secret = Some(rand::random());
        self.creator_seed = Some(*root.as_bytes());
        self.recovery_exported = false;
        self.recovery_saved = false;
        self.verify()?;
        Ok(())
    }

    pub(super) fn select_invitation(&mut self, full: ClientInvitation) -> Result<(), JsValue> {
        let material = full.material_bytes().map_err(error)?;
        let invitation = Invitation::decode(&material)?;
        if self.invitation.is_some() && self.invitation()?.anchor != invitation.anchor {
            return Err(error("another network is already selected"));
        }
        if self.creator_seed.is_some() {
            return Err(error("creator already belongs to this network"));
        }
        let precode = full.precode_bytes().map_err(error)?;
        if self.invitation.as_ref().is_some_and(|old| old != &material) && precode.is_none() {
            return Err(error(
                "changing an ordinary request requires a fresh explicit attempt",
            ));
        }
        self.invitation = Some(material.to_vec());
        self.precode = precode.map(|bytes| bytes[96..].try_into().unwrap());
        if self.precode.is_some() {
            self.manual_rejected = false;
        }
        self.complete_invitation = Some(full);
        self.verify()?;
        Ok(())
    }
}

impl Configuration {
    pub(super) fn import_contact(&mut self, invitation: &ClientInvitation) -> Result<(), JsValue> {
        let contact = invitation.bootstrap();
        contact.validate().map_err(error)?;
        if !self.relay_replacement {
            for relay in contact
                .relays
                .iter()
                .cloned()
                .chain(invitation.address().relay_urls().map(ToString::to_string))
            {
                origin(&relay)?;
                if !self.relays.contains(&relay) {
                    self.relays.push(relay);
                }
            }
        }
        if !self.mqtt_replacement {
            for facility in &contact.mqtt {
                if facility.check_allowed(&self.disabled).is_err() {
                    continue;
                }
                if let Some(old) = self
                    .mqtt
                    .iter()
                    .find(|old| old.logical_id == facility.logical_id)
                {
                    if old != facility {
                        return Err(error("invitation changes a saved MQTT mapping"));
                    }
                } else {
                    self.mqtt.push(facility.clone());
                }
            }
        }
        if !self.irc_replacement {
            for facility in &contact.irc {
                if facility.check_allowed(&self.disabled).is_err() {
                    continue;
                }
                if let Some(old) = self
                    .irc
                    .iter()
                    .find(|old| old.facility.logical_id == facility.logical_id)
                {
                    if old.facility != *facility {
                        return Err(error("invitation changes a saved IRC mapping"));
                    }
                } else {
                    self.irc.push(IrcConfiguration {
                        facility: facility.clone(),
                        profile: Default::default(),
                    });
                }
            }
        }
        // Native-only signaling URLs remain honest metadata in the invitation;
        // they never become an invented browser WebSocket mapping.
        for address in &invitation.address().addrs {
            let iroh_base::TransportAddr::Custom(address) = address else {
                continue;
            };
            let facility = self
                .mqtt
                .iter()
                .find(|f| {
                    f.check_allowed(&self.disabled).is_ok()
                        && f.service_tag().is_ok_and(|tag| {
                            crate::mqtt_datagram::route_from_addr(address, Some(tag)).is_ok()
                        })
                })
                .map(|f| f.logical_id.clone())
                .or_else(|| {
                    self.irc
                        .iter()
                        .find(|entry| {
                            entry.facility.check_allowed(&self.disabled).is_ok()
                                && entry.facility.service_tag().is_ok_and(|tag| {
                                    crate::irc_datagram::route_from_addr(address, tag).is_ok()
                                })
                        })
                        .map(|entry| entry.facility.logical_id.clone())
                });
            if let Some(facility) = facility {
                let route = ContactRoute {
                    contact: invitation.contact().to_string(),
                    facility,
                    address: hex::encode(address.data()),
                };
                if !self.contact_routes.iter().any(|old| {
                    old.contact == route.contact
                        && old.facility == route.facility
                        && old.address == route.address
                }) {
                    self.contact_routes.push(route);
                }
            }
        }
        Ok(())
    }
}

impl BrowserGate {
    pub(super) fn pairing_permitted(&self, connection: &iroh::endpoint::ControlConnection) -> bool {
        let state = self.0.lock().unwrap();
        if connection.side().is_client() {
            state.pairing_peer == Some(connection.remote_id())
        } else {
            state.pairing_receiver
        }
    }
}

struct PairingPermission(BrowserGate);
impl Drop for PairingPermission {
    fn drop(&mut self) {
        self.0.0.lock().unwrap().pairing_peer = None;
    }
}

#[wasm_bindgen]
impl BrowserNetwork {
    /// A new independently identified creator. The complete credential and
    /// signed original Create are committed together, before the endpoint opens.
    pub async fn create(configuration: String) -> Result<BrowserNetwork, JsValue> {
        Self::start_onboarding(configuration, None, Startup::Create).await
    }

    /// Full text/link/QR material is validated and its permitted contact
    /// facilities imported before the browser binds any transport.
    pub async fn join(
        configuration: String,
        invitation: String,
    ) -> Result<BrowserNetwork, JsValue> {
        let full = ClientInvitation::decode(invitation.as_bytes()).map_err(error)?;
        Self::start_onboarding(configuration, None, Startup::Join(full)).await
    }

    /// Pre-network receiver. Possessing a node ID or route is insufficient:
    /// delivery must prove the private scanned capability on this exact TLS C.
    pub async fn pair(configuration: String) -> Result<BrowserNetwork, JsValue> {
        Self::start_onboarding(configuration, None, Startup::Pairing).await
    }

    /// Secret-bearing return for an explicit private save/print operation.
    /// Export alone does not confirm the user saved the recovery card.
    pub async fn export_recovery_card(
        &mut self,
        password: Option<String>,
    ) -> Result<String, JsValue> {
        self.check_store().await?;
        let seed = self
            .saved
            .creator_seed
            .ok_or_else(|| error("no genesis credential"))?;
        let card =
            crate::recovery_card::RecoveryCard::new(seed, self.saved.invitation()?.anchor.genesis);
        let text = card.export(password.as_deref()).map_err(error)?;
        let mut next = self.saved.clone();
        next.recovery_exported = true;
        self.commit(next).await?;
        Ok(text)
    }

    pub fn inspect_recovery_card(
        text: String,
        password: Option<String>,
    ) -> Result<String, JsValue> {
        let anchor = crate::recovery_card::RecoveryCard::import(&text, password.as_deref())
            .map_err(error)?
            .anchor();
        Ok(serde_json::json!({"network":anchor.root.to_string(),"genesis":anchor.genesis.to_string()}).to_string())
    }

    /// Explicit user acknowledgment; no forced password or read-back step.
    pub async fn acknowledge_recovery_saved(&mut self) -> Result<(), JsValue> {
        self.check_store().await?;
        if self.saved.creator_seed.is_none() || !self.saved.recovery_exported {
            return Err(error("export the recovery card first"));
        }
        let mut next = self.saved.clone();
        next.recovery_saved = true;
        self.commit(next).await
    }

    pub fn invitation(&self) -> Result<String, JsValue> {
        self.administrator()?;
        self.full_invitation(None)?.text().map_err(error)
    }

    pub async fn precode_invitation(
        &mut self,
        expires_unix_seconds: u64,
        max_uses: Option<u64>,
    ) -> Result<String, JsValue> {
        let code: [u8; 128] =
            hex::decode(self.issue_precode(expires_unix_seconds, max_uses).await?)
                .map_err(error)?
                .try_into()
                .map_err(|_| rejected())?;
        self.full_invitation(Some(code))?.text().map_err(error)
    }

    /// Continue the saved full invitation. Authorization-only is useful for a
    /// host pausing before secret delivery; it never reports admission complete.
    pub async fn request_join(&mut self, authorization_only: bool) -> Result<String, JsValue> {
        if self.pairing.is_some() {
            return Err(error("finish pairing before requesting admission"));
        }
        let full = self
            .saved
            .complete_invitation
            .clone()
            .ok_or_else(|| error("no complete invitation selected"))?;
        if self.saved.stage() == "Complete" {
            self.check_store().await?;
            return Ok("Complete".into());
        }
        let stage = if let Some(code) = full.precode_bytes().map_err(error)? {
            self.request_precode(hex::encode(code)).await?
        } else {
            self.request_admission(hex::encode(full.material_bytes().map_err(error)?))
                .await?
        };
        if self.delivery.is_some() {
            if authorization_only {
                self.delivery = None;
            } else {
                return self.complete_admission().await;
            }
        }
        Ok(stage)
    }

    /// Rejection ends only that request. A user action creates a new durable
    /// request ID while retaining the independent node and selected network.
    pub async fn new_manual_attempt(&mut self) -> Result<(), JsValue> {
        self.check_store().await?;
        if !self.saved.manual_rejected
            || self.saved.precode.is_some()
            || self.saved.authorization.is_some()
        {
            return Err(error("no declined manual request"));
        }
        self.delivery = None;
        let mut next = self.saved.clone();
        next.request = rand::random();
        next.manual_rejected = false;
        self.commit(next).await
    }

    /// Private pairing material for trusted transfer to the administrator.
    pub fn pairing_information(&self) -> Result<String, JsValue> {
        self.pairing
            .as_ref()
            .ok_or_else(|| error("not a pairing receiver"))?
            .text()
            .map_err(error)
    }

    pub fn pairing_status(&self) -> String {
        serde_json::json!({"invitation_saved":self.pairing.is_some() && self.saved.complete_invitation.is_some()}).to_string()
    }

    pub async fn receive_invitation(&mut self) -> Result<(), JsValue> {
        self.check_store().await?;
        let info = self
            .pairing
            .clone()
            .ok_or_else(|| error("not a pairing receiver"))?;
        let deadline = n0_future::time::Instant::now() + self.caller_budget;
        let attempt = crate::connection_coordinator::AttemptContext::new(
            deadline,
            tokio_util::sync::CancellationToken::new(),
        );
        let accepting = self.incoming.receive(PAIRING_ALPN, &attempt).await?;
        n0_future::time::timeout(
            deadline.saturating_duration_since(n0_future::time::Instant::now()),
            async {
                let connection = Arc::new(
                    accepting
                        .with_caller_deadline(deadline)
                        .into_control()
                        .await
                        .map_err(error)?,
                )
                .retain_control();
                let _scope = RequestConnection(Some(connection.clone()));
                let delivery =
                    crate::invitation_pairing::receive(&connection, self.endpoint.id(), &info)
                        .await
                        .map_err(error)?;
                if let Some(saved) = &self.saved.complete_invitation {
                    if saved.encode().map_err(error)? != delivery.bytes {
                        return Err(error("paired invitation cannot be replaced"));
                    }
                } else {
                    let mut next = self.saved.clone();
                    let mut resources = next.resources.clone().ok_or_else(rejected)?;
                    resources.import_contact(&delivery.invitation)?;
                    next.select_invitation(delivery.invitation.clone())?;
                    next.resources = Some(resources);
                    self.commit(next).await?;
                }
                delivery.acknowledge().await.map_err(error)?;
                connection.revocation_handle().closed().await;
                Ok(())
            },
        )
        .await
        .map_err(|_| error("invitation delivery caller deadline"))?
    }

    /// Rebind only after SAVE, using the same saved identity and imported
    /// facilities. Ordinary approval or precode redemption remains necessary.
    pub async fn finish_pairing(&mut self) -> Result<BrowserNetwork, JsValue> {
        if self.pairing.is_none() || self.saved.complete_invitation.is_none() {
            return Err(error("no paired invitation saved"));
        }
        self.close().await;
        Self::start("{}".into()).await
    }

    pub async fn deliver_invitation(
        &mut self,
        pairing: String,
        invitation: String,
    ) -> Result<(), JsValue> {
        self.check_store().await?;
        self.administrator()?;
        let info = PairingInfo::decode(pairing.as_bytes()).map_err(error)?;
        let full = ClientInvitation::decode(invitation.as_bytes()).map_err(error)?;
        if full.contact() != self.endpoint.id()
            || full.anchor().map_err(error)? != self.saved.invitation()?.anchor
        {
            return Err(rejected());
        }
        self.gate.0.lock().unwrap().pairing_peer = Some(info.receiver());
        let _permission = PairingPermission(self.gate.clone());
        let deadline = n0_future::time::Instant::now() + self.caller_budget;
        n0_future::time::timeout(self.caller_budget, async {
            let connection = Arc::new(
                self.endpoint
                    .connect_control_with_opts(
                        info.address().clone(),
                        PAIRING_ALPN,
                        iroh::endpoint::ConnectOptions::new().with_caller_deadline(deadline),
                    )
                    .await
                    .map_err(error)?
                    .await
                    .map_err(error)?,
            )
            .retain_control();
            let _scope = RequestConnection(Some(connection.clone()));
            crate::invitation_pairing::deliver(&connection, self.endpoint.id(), &info, &full)
                .await
                .map_err(error)
        })
        .await
        .map_err(|_| error("invitation delivery caller deadline"))?
    }

    /// Public resource choices and route descriptors contain no credentials.
    pub fn resource_configuration(&self) -> Result<String, JsValue> {
        serde_json::to_string(&self.saved.resources).map_err(error)
    }
}

impl BrowserNetwork {
    pub(super) fn contact(&self) -> (EndpointAddr, BootstrapContact) {
        let mut address = self.endpoint.addr();
        address.addrs.extend(
            self.endpoint
                .custom_addrs()
                .into_iter()
                .filter(|address| {
                    self.mqtt.iter().any(|(facility, _)| {
                        facility.service_tag().is_ok_and(|tag| {
                            crate::mqtt_datagram::route_from_addr(address, Some(tag)).is_ok()
                        })
                    }) || self.irc.iter().any(|(entry, _)| {
                        entry.facility.service_tag().is_ok_and(|tag| {
                            crate::irc_datagram::route_from_addr(address, tag).is_ok()
                        })
                    })
                })
                .map(iroh_base::TransportAddr::Custom),
        );
        (
            address,
            BootstrapContact {
                relays: self.relays.iter().map(ToString::to_string).collect(),
                mqtt: self
                    .mqtt
                    .iter()
                    .map(|(facility, _)| facility.clone())
                    .collect(),
                irc: self
                    .irc
                    .iter()
                    .map(|(entry, _)| entry.facility.clone())
                    .collect(),
                native_signaling: Vec::new(),
            },
        )
    }
    fn full_invitation(&self, precode: Option<[u8; 128]>) -> Result<ClientInvitation, JsValue> {
        let (address, bootstrap) = self.contact();
        ClientInvitation::new(
            self.own_invitation()?.try_into().map_err(|_| rejected())?,
            address,
            precode,
            bootstrap,
        )
        .map_err(error)
    }
}
