//! Durable Native adapter for the shared service ledger and public operations.
use crate::{
    control_protocol::AuthenticatedControl,
    native_client::ClientError,
    native_file_transfer::NativeFiles,
    native_network::NativeNetwork,
    service_policy::{
        GuestInvitation, GuestStatus, Service, ServiceAccess, ServiceAction, ServiceRule,
    },
    service_protocol::ServiceBundle,
};
use p2panda_core::{Hash, VerifyingKey};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

pub(crate) struct NativeServices {
    path: PathBuf,
    records: Vec<Vec<u8>>,
    pub guest: Option<GuestInvitation>,
    pub clipboard: Option<std::sync::Weak<crate::native_clipboard::NativeClipboard>>,
    pub forwarding: Option<std::sync::Weak<crate::native_forwarding::NativeForwarding>>,
    pub web: Option<std::sync::Weak<crate::native_web::NativeWeb>>,
}
impl NativeServices {
    fn publish_access(&self, files: &Arc<NativeFiles>, access: ServiceAccess) {
        if let Some(web) = self.web.as_ref().and_then(std::sync::Weak::upgrade) {
            web.enforce_access(&access);
        }
        if let Some(forwarding) = self.forwarding.as_ref().and_then(std::sync::Weak::upgrade) {
            forwarding.enforce_access(&access);
        }
        if let Some(clipboard) = self.clipboard.as_ref().and_then(std::sync::Weak::upgrade) {
            clipboard.enforce_access(&access);
        }
        files.update_access(access);
    }
    pub async fn declare_clipboard(
        &mut self,
        node: &NativeNetwork,
        files: &Arc<NativeFiles>,
        capable: bool,
        enabled: bool,
    ) -> Result<(), ClientError> {
        let access = self.access(node).await?;
        if !access.member(access.local) || self.guest.is_some() {
            return Ok(());
        }
        if let Some(change) = access.declaration_change(Service::Clipboard, capable, enabled) {
            self.apply(node, files, change).await?;
        }
        Ok(())
    }
    pub async fn declare_files(
        &mut self,
        node: &NativeNetwork,
        files: &Arc<NativeFiles>,
        enabled: bool,
    ) -> Result<(), ClientError> {
        let access = self.access(node).await?;
        if !access.member(access.local) || self.guest.is_some() {
            return Ok(());
        }
        if let Some(change) = access.declaration_change(Service::Files, true, enabled) {
            self.apply(node, files, change).await?;
        }
        Ok(())
    }
    pub async fn load(node: &NativeNetwork) -> Result<Self, ClientError> {
        let path = node.directory.join("services.records");
        let records = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        let guest = match std::fs::read(node.directory.join("services.guest-invitation")) {
            Ok(bytes) => Some(GuestInvitation::decode(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        if guest.as_ref().is_some_and(|g| g.anchor != node.anchor) {
            return Err("guest belongs to another network".into());
        }
        let state = Self {
            path,
            records,
            guest,
            clipboard: None,
            forwarding: None,
            web: None,
        };
        state.snapshot(node).await?.verify(node.anchor)?;
        Ok(state)
    }
    pub async fn snapshot(&self, node: &NativeNetwork) -> Result<ServiceBundle, ClientError> {
        let management = node
            .store
            .applied_records()
            .await?
            .iter()
            .map(|r| (r.header().to_vec(), r.body().map(Vec::from)))
            .collect();
        Ok(ServiceBundle {
            management,
            services: self.records.clone(),
        })
    }
    pub async fn access(&self, node: &NativeNetwork) -> Result<ServiceAccess, ClientError> {
        let (auth, ledger) = self.snapshot(node).await?.verify(node.anchor)?;
        Ok(ServiceAccess {
            local: node.node_key.verifying_key(),
            auth,
            ledger,
        })
    }
    pub async fn apply(
        &mut self,
        node: &NativeNetwork,
        files: &Arc<NativeFiles>,
        command: ServiceAction,
    ) -> Result<Hash, ClientError> {
        let access = self.access(node).await?;
        let bytes = access
            .ledger
            .author(node.anchor, &access.auth, &node.node_key, command)?;
        let id = Hash::digest(&bytes);
        self.accept(node, files, bytes).await?;
        Ok(id)
    }
    pub async fn accept(
        &mut self,
        node: &NativeNetwork,
        files: &Arc<NativeFiles>,
        bytes: Vec<u8>,
    ) -> Result<(), ClientError> {
        let access = self.access(node).await?;
        let ledger = access.ledger.merge(node.anchor, &access.auth, &[bytes])?;
        crate::native_files::write_private_atomic(
            &self.path,
            &serde_json::to_vec(ledger.records())?,
        )?;
        self.records = ledger.records().to_vec();
        self.publish_access(files, ServiceAccess { ledger, ..access });
        Ok(())
    }
    pub async fn merge(
        &mut self,
        node: &mut NativeNetwork,
        files: &Arc<NativeFiles>,
        bundle: ServiceBundle,
    ) -> Result<(), ClientError> {
        // A rejected accompanying service head must never withhold a valid
        // management correction. Service changes themselves remain atomic.
        let result = self.snapshot(node).await?.reconcile(node.anchor, &bundle)?;
        let (auth, ledger) = result.bundle.verify(node.anchor)?;
        for (header, body) in bundle.management {
            node.ingest_management_record(&header, body.as_deref())
                .await?;
        }
        // This authority change is already durable even if publishing the
        // separate service file fails. Enforce it with the old saved policy.
        self.publish_access(files, self.access(node).await?);
        crate::native_files::write_private_atomic(
            &self.path,
            &serde_json::to_vec(ledger.records())?,
        )?;
        self.records = ledger.records().to_vec();
        self.publish_access(
            files,
            ServiceAccess {
                local: node.node_key.verifying_key(),
                auth,
                ledger,
            },
        );
        if let Some(error) = result.policy_error {
            return Err(error.into());
        }
        Ok(())
    }
}

pub(crate) async fn serve_policy(
    connection: iroh::endpoint::Connection,
    node: Arc<Mutex<NativeNetwork>>,
    services: Arc<Mutex<NativeServices>>,
    files: Arc<NativeFiles>,
) -> Result<(), ClientError> {
    let result = async {
        let mut stream = crate::data_stream::DataStream::accept(&connection).await?;
        let remote = ServiceBundle::read(&mut stream).await?;
        let response = {
            let mut node = node.lock().await;
            let mut services = services.lock().await;
            services.merge(&mut node, &files, remote).await?;
            services.snapshot(&node).await?
        };
        response.write(&mut stream).await?;
        stream.finish()?;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), connection.closed()).await;
        Ok(())
    }
    .await;
    connection.close(0u32.into(), b"service policy exchange ended");
    result
}

pub(crate) async fn serve_guest(
    connection: crate::control_protocol::ControlOwner,
    node: Arc<Mutex<NativeNetwork>>,
    services: Arc<Mutex<NativeServices>>,
    files: Arc<NativeFiles>,
) -> Result<(), ClientError> {
    use crate::control_protocol::AuthenticatedControl;
    use crate::service_protocol::{GuestRequest, read_guest, write_guest};
    let result = async {
        let exchange = connection.accept_control_exchange().await?;
        let request: GuestRequest = read_guest(&exchange).await?;
        let node = node.lock().await;
        let mut state = services.lock().await;
        let peer = VerifyingKey::from_bytes(connection.remote_id().as_bytes())?;
        match request {
            GuestRequest::Redeem { invitation, token } => {
                let access = state.access(&node).await?;
                let status = access
                    .ledger
                    .guests()
                    .into_iter()
                    .find(|g| g.invitation == invitation)
                    .ok_or("unknown guest invitation")?;
                if status.issuer != node.node_key.verifying_key() {
                    return Err("wrong guest redemption node".into());
                }
                if access.ledger.redeem(
                    invitation,
                    &token,
                    peer,
                    &access.auth,
                    crate::service_policy::now(),
                )? {
                    state
                        .apply(
                            &node,
                            &files,
                            ServiceAction::BindGuest {
                                invitation,
                                guest: peer,
                            },
                        )
                        .await?;
                }
                write_guest(&exchange, &state.snapshot(&node).await?).await?;
            }
            GuestRequest::End { record } => {
                if !crate::service_policy::ServiceLedger::is_guest_end(&record, peer) {
                    return Err("invalid guest exit".into());
                }
                state.accept(&node, &files, record).await?;
                write_guest(&exchange, &json!({"saved_locally":true})).await?;
            }
        }
        exchange.finish_send().await?;
        drop(state);
        drop(node);
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            connection.revocation_handle().closed(),
        )
        .await;
        Ok(())
    }
    .await;
    connection.close(0u32.into(), b"guest operation ended");
    result
}

impl crate::native_client::NativeClient {
    pub async fn join_guest(
        path: impl AsRef<std::path::Path>,
        invitation: &GuestInvitation,
        bind: std::net::Ipv4Addr,
    ) -> Result<Self, ClientError> {
        Self::join_guest_with_resources(path, invitation, bind, Default::default()).await
    }
    pub async fn join_guest_with_resources(
        path: impl AsRef<std::path::Path>,
        invitation: &GuestInvitation,
        bind: std::net::Ipv4Addr,
        local: crate::resources::ResourceConfig,
    ) -> Result<Self, ClientError> {
        let ordinary = invitation.ordinary_contact()?;
        let config = crate::client_contact::import_contact(local, ordinary.bootstrap())?;
        let node = NativeNetwork::join(
            path,
            crate::manual_admission::ManualInvitation::decode(&ordinary.material_bytes()?)?,
        )
        .await?;
        node.save_resource_configuration(&config)?;
        crate::native_files::write_private(
            &node.directory.join("services.guest-invitation"),
            &invitation.encode()?,
        )?;
        Self::start(node, bind).await
    }
    pub async fn guest_status(&self) -> Result<Option<Value>, ClientError> {
        let node = self.node.lock().await;
        let state = self.services.lock().await;
        let Some(invitation) = &state.guest else {
            return Ok(None);
        };
        let access = state.access(&node).await?;
        Ok(Some(access.guest_status(invitation)?))
    }
    pub async fn issue_guest(
        &self,
        provider: iroh::EndpointAddr,
        expires: u64,
    ) -> Result<GuestInvitation, ClientError> {
        self.issue_service_guest(provider, Service::Files, expires)
            .await
    }
    pub async fn issue_service_guest(
        &self,
        provider: iroh::EndpointAddr,
        service: Service,
        expires: u64,
    ) -> Result<GuestInvitation, ClientError> {
        self.require_ready()?;
        if !service.guest_supported() {
            return Err("service guest adapter unavailable".into());
        }
        let created = crate::service_policy::now();
        if expires <= created {
            return Err("guest deadline must be in the future".into());
        }
        let node = self.node.lock().await;
        let mut state = self.services.lock().await;
        let access = state.access(&node).await?;
        let provider_id = VerifyingKey::from_bytes(provider.id.as_bytes())?;
        if !access.ledger.rule(provider_id, service).rule.guests {
            return Err("guest sharing disabled by effective rule".into());
        }
        let token: [u8; 32] = rand::random();
        let id = state
            .apply(
                &node,
                &self.files,
                ServiceAction::InviteGuest {
                    provider: provider_id,
                    service,
                    token_hash: Hash::digest(token),
                    created,
                    expires,
                },
            )
            .await?;
        let record = state
            .records
            .iter()
            .find(|r| Hash::digest(r) == id)
            .ok_or("guest record was not saved")?
            .clone();
        let (issuer, bootstrap) = self.resources.invitation_contact()?;
        let invitation = GuestInvitation {
            anchor: node.anchor,
            issuer,
            provider,
            bootstrap,
            record,
            token,
        };
        invitation.encode()?;
        Ok(invitation)
    }
    pub async fn redeem_guest(&self) -> Result<Value, ClientError> {
        let invitation = self
            .services
            .lock()
            .await
            .guest
            .clone()
            .ok_or("not a guest invitation")?;
        let connection = self
            .resources
            .connect_control(invitation.issuer.clone(), crate::service_policy::GUEST_ALPN)
            .await?;
        let result = async {
            let exchange = connection.open_control_exchange().await?;
            crate::service_protocol::write_guest(
                &exchange,
                &crate::service_protocol::GuestRequest::Redeem {
                    invitation: invitation.id(),
                    token: invitation.token,
                },
            )
            .await?;
            exchange.finish_send().await?;
            let bundle: ServiceBundle = crate::service_protocol::read_guest(&exchange).await?;
            let mut node = self.node.lock().await;
            let mut state = self.services.lock().await;
            state.merge(&mut node, &self.files, bundle).await?;
            let access = state.access(&node).await?;
            if !access.ledger.guest_access(
                VerifyingKey::from_bytes(invitation.provider.id.as_bytes())?,
                invitation.details()?.service,
                access.local,
                &access.auth,
                crate::service_policy::now(),
            ) {
                return Err("guest authorization incomplete".into());
            }
            Ok::<_, ClientError>(json!({"saved_locally":true}))
        }
        .await;
        connection.close(0u32.into(), b"guest redemption ended");
        result?;
        self.guest_status()
            .await?
            .ok_or_else(|| "guest state missing".into())
    }
    pub async fn end_guest(&self) -> Result<Value, ClientError> {
        let invitation = self
            .services
            .lock()
            .await
            .guest
            .clone()
            .ok_or("not a guest")?;
        let record = {
            let node = self.node.lock().await;
            let mut state = self.services.lock().await;
            let access = state.access(&node).await?;
            let ended = access
                .ledger
                .guests()
                .into_iter()
                .find(|g| g.invitation == invitation.id())
                .is_some_and(|g| g.ended);
            if !ended {
                state
                    .apply(
                        &node,
                        &self.files,
                        ServiceAction::EndGuest {
                            invitation: invitation.id(),
                        },
                    )
                    .await?;
            }
            state
                .access(&node)
                .await?
                .ledger
                .guest_end_record(invitation.id())
                .ok_or("guest exit record missing")?
        };
        let mut delivered = Vec::new();
        let mut targets = vec![invitation.issuer.clone()];
        if invitation.provider.id != invitation.issuer.id {
            targets.push(invitation.provider);
        }
        for address in targets {
            let result = async {
                let connection = self
                    .resources
                    .connect_control(address.clone(), crate::service_policy::GUEST_ALPN)
                    .await?;
                let result = async {
                    let exchange = connection.open_control_exchange().await?;
                    crate::service_protocol::write_guest(
                        &exchange,
                        &crate::service_protocol::GuestRequest::End {
                            record: record.clone(),
                        },
                    )
                    .await?;
                    exchange.finish_send().await?;
                    let reply: Value = crate::service_protocol::read_guest(&exchange).await?;
                    if reply["saved_locally"] != true {
                        return Err("guest exit was not acknowledged as saved".into());
                    }
                    Ok::<_, ClientError>(())
                }
                .await;
                connection.close(0u32.into(), b"guest exit ended");
                result
            }
            .await;
            delivered.push(json!({"node":address.id.to_string(),"saved":result.is_ok()}));
        }
        Ok(json!({"ended_locally":true,"delivery":delivered}))
    }
    pub async fn service_status(&self) -> Result<Value, ClientError> {
        let clipboard_capability = crate::system_clipboard::current_capability().await;
        self.clipboard
            .state
            .capability(clipboard_capability.clone());
        let clipboard = self.clipboard.state.status();
        let node = self.node.lock().await;
        let state = self.services.lock().await;
        let access = state.access(&node).await?;
        let clipboard_guest = state
            .guest
            .as_ref()
            .map(GuestInvitation::details)
            .transpose()?
            .is_some_and(|scope| scope.service == Service::Clipboard);
        let mut status = access.status(
            state.guest.as_ref(),
            if clipboard_guest {
                clipboard_capability.read && clipboard_capability.write
            } else {
                true
            },
            if clipboard_guest {
                clipboard.enabled
            } else {
                self.files_enabled()
            },
        )?;
        if state.guest.is_none() {
            status["services"]
                .as_array_mut()
                .ok_or("invalid service status")?
                .push(access.local_service_status(
                    Service::Clipboard,
                    clipboard_capability.read && clipboard_capability.write,
                    clipboard.enabled,
                ));
            for protocol in [crate::forwarding::Protocol::Tcp, crate::forwarding::Protocol::Udp] {
                status["services"].as_array_mut().ok_or("invalid service status")?.push(access.local_service_status(
                    protocol.service(), true, self.forwarding_enabled(protocol)));
            }
            status["services"].as_array_mut().ok_or("invalid service status")?.push(
                access.local_service_status(Service::Web, true, self.web.status()["enabled"] == true));
        }
        Ok(status)
    }
    pub async fn set_service_rule(
        &self,
        provider: VerifyingKey,
        service: Service,
        previous: Option<Hash>,
        rule: ServiceRule,
    ) -> Result<Hash, ClientError> {
        self.require_ready()?;
        let node = self.node.lock().await;
        self.services
            .lock()
            .await
            .apply(
                &node,
                &self.files,
                ServiceAction::SetRule {
                    provider,
                    service,
                    previous,
                    rule,
                },
            )
            .await
    }
    pub async fn revoke_guest(&self, invitation: Hash) -> Result<Hash, ClientError> {
        self.require_ready()?;
        let node = self.node.lock().await;
        self.services
            .lock()
            .await
            .apply(&node, &self.files, ServiceAction::EndGuest { invitation })
            .await
    }
    pub async fn sync_services(&self, address: iroh::EndpointAddr) -> Result<Value, ClientError> {
        self.require_ready()?;
        let snapshot = {
            let node = self.node.lock().await;
            self.services.lock().await.snapshot(&node).await?
        };
        let connection = self
            .resources
            .connect(address, crate::service_policy::POLICY_ALPN)
            .await?;
        let result = async {
            let mut stream = crate::data_stream::DataStream::open(&connection).await?;
            snapshot.write(&mut stream).await?;
            let remote = ServiceBundle::read(&mut stream).await?;
            let mut node = self.node.lock().await;
            self.services
                .lock()
                .await
                .merge(&mut node, &self.files, remote)
                .await?;
            Ok::<_, ClientError>(json!({"saved_locally":true}))
        }
        .await;
        connection.close(0u32.into(), b"service policy exchange ended");
        result
    }
}
