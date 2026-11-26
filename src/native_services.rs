//! Durable Native adapter for the shared service ledger and public operations.
use crate::{
    control_protocol::AuthenticatedControl,
    native_client::ClientError,
    native_file_transfer::NativeFiles,
    native_network::NativeNetwork,
    service_policy::{
        GuestInvitation, GuestStatus, Service, ServiceAccess, ServiceAction, ServiceRule,
    },
    service_protocol::{MemberSync, MemberSyncMembership, MemberSyncMode, ServiceBundle},
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
    pub terminal: Option<std::sync::Weak<crate::native_terminal::NativeTerminal>>,
}
impl NativeServices {
    pub(crate) fn publish_access(&self, files: &Arc<NativeFiles>, access: ServiceAccess) {
        if let Some(terminal) = self.terminal.as_ref().and_then(std::sync::Weak::upgrade) {
            terminal.enforce_access(&access);
        }
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
            terminal: None,
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

    async fn member_sync(
        &self,
        node: &NativeNetwork,
        mode: MemberSyncMode,
    ) -> Result<MemberSync, ClientError> {
        Ok(MemberSync::new(
            self.snapshot(node).await?,
            node.device_names.for_members(&node.store.nodes()),
            mode,
        ))
    }

    async fn refresh_local_declarations(
        &mut self,
        node: &NativeNetwork,
        files: &Arc<NativeFiles>,
    ) -> Result<(), ClientError> {
        use crate::forwarding::Protocol;
        use std::sync::Weak;

        let access = self.access(node).await?;
        if !access.has_stale_local_declarations() {
            return Ok(());
        }
        let clipboard_capable = crate::system_clipboard::current_capability().await.read;
        let clipboard_enabled = clipboard_capable && self.clipboard.as_ref().and_then(Weak::upgrade)
            .is_some_and(|clipboard| clipboard.state.status().enabled);
        let forwarding = self.forwarding.as_ref().and_then(Weak::upgrade);
        let web_enabled = self.web.as_ref().and_then(Weak::upgrade)
            .is_some_and(|web| web.provider_enabled());
        let (terminal_capable, terminal_enabled) = self.terminal.as_ref().and_then(Weak::upgrade)
            .map_or((false, false), |terminal| terminal.provider_capability());
        let declarations = [
            (Service::Files, true, files.enabled()),
            (Service::Clipboard, clipboard_capable, clipboard_enabled),
            (Service::TcpForward, true, forwarding.as_ref().is_some_and(|host| host.provider_enabled(Protocol::Tcp))),
            (Service::UdpForward, true, forwarding.as_ref().is_some_and(|host| host.provider_enabled(Protocol::Udp))),
            (Service::Web, true, web_enabled),
            (Service::Terminal, terminal_capable, terminal_enabled),
        ];
        for (service, capable, enabled) in declarations {
            if let Some(change) = access.refresh_declaration(service, capable, enabled) {
                self.apply(node, files, change).await?;
            }
        }
        Ok(())
    }

    async fn merge_member_sync(
        &mut self,
        node: &mut NativeNetwork,
        files: &Arc<NativeFiles>,
        sync: MemberSync,
    ) -> Result<(), ClientError> {
        let mode = sync.mode();
        self.merge(node, files, sync.bundle).await?;
        if mode == MemberSyncMode::Management {
            return Ok(());
        }
        let members = node.store.nodes();
        node.update_device_names(|names| {
            names.merge_members(&members, sync.device_names);
            Ok(())
        })?;
        Ok(())
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
        let current = self.snapshot(node).await?;
        let result = current.reconcile(node.anchor, &bundle)?;
        let (auth, ledger) = result.bundle.verify(node.anchor)?;
        let additions = &result.bundle.management[current.management.len()..];
        for (header, body) in additions {
            node.ingest_management_record(header, body.as_deref())
                .await?;
        }
        // This authority change is already durable even if publishing the
        // separate service file fails. Enforce it with the old saved policy.
        if self.records != ledger.records() {
            if !additions.is_empty() {
                self.publish_access(files, self.access(node).await?);
            }
            crate::native_files::write_private_atomic(
                &self.path,
                &serde_json::to_vec(ledger.records())?,
            )?;
            self.records = ledger.records().to_vec();
        }
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
    deadline: tokio::time::Instant,
) -> Result<(), ClientError> {
    let result = async {
        let admitted = {
            let node = node.lock().await;
            member_sync_admission(&node, &connection)?
        };
        let (mut stream, remote) = tokio::time::timeout_at(deadline, async {
            let mut stream = crate::data_stream::DataStream::accept(&connection).await?;
            let remote = MemberSync::read(&mut stream).await?;
            Ok::<_, std::io::Error>((stream, remote))
        })
        .await??;
        let mode = remote.mode();
        let response = {
            let mut node = node.lock().await;
            require_current_member_sync_access(&node, &admitted)?;
            let mut services = services.lock().await;
            services.merge_member_sync(&mut node, &files, remote).await?;
            if mode == MemberSyncMode::Full {
                services.refresh_local_declarations(&node, &files).await?;
            }
            services.member_sync(&node, mode).await?
        };
        tokio::time::timeout_at(deadline, async {
            response.write(&mut stream).await?;
            stream.finish()
        })
        .await??;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), connection.closed()).await;
        Ok(())
    }
    .await;
    connection.close(0u32.into(), b"service policy exchange ended");
    result
}

struct MemberSyncAdmission {
    proof: Arc<crate::admission_proof::BootstrapProof>,
    membership: MemberSyncMembership,
}

fn member_sync_admission(
    node: &NativeNetwork,
    connection: &iroh::endpoint::Connection,
) -> Result<MemberSyncAdmission, ClientError> {
    let proof = node
        .admission_gate()
        .proof_for(connection)
        .ok_or("member synchronization access revoked")?;
    let membership = MemberSyncMembership::capture(
        node.store.authorization(),
        node.node_key.verifying_key(),
        connection.remote_id(),
    )
    .ok_or("member synchronization access revoked")?;
    Ok(MemberSyncAdmission { proof, membership })
}

fn require_current_member_sync_access(
    node: &NativeNetwork,
    admitted: &MemberSyncAdmission,
) -> Result<(), ClientError> {
    if node
        .admission_gate()
        .accepts_member_history(&admitted.proof)
        && admitted
            .membership
            .is_current(node.store.authorization(), node.node_key.verifying_key())
    {
        Ok(())
    } else {
        Err("member synchronization access revoked".into())
    }
}

pub(crate) async fn sync_member(
    connection: &iroh::endpoint::Connection,
    node: &Mutex<NativeNetwork>,
    services: &Mutex<NativeServices>,
    files: &Arc<NativeFiles>,
    mode: MemberSyncMode,
    deadline: tokio::time::Instant,
) -> Result<(), ClientError> {
    let (snapshot, admitted) = {
        let node = node.lock().await;
        let admitted = member_sync_admission(&node, connection)?;
        let mut services = services.lock().await;
        if mode == MemberSyncMode::Full {
            services.refresh_local_declarations(&node, files).await?;
        }
        (services.member_sync(&node, mode).await?, admitted)
    };
    let remote = tokio::time::timeout_at(deadline, snapshot.exchange(connection)).await??;
    let mut node = node.lock().await;
    require_current_member_sync_access(&node, &admitted)?;
    services
        .lock()
        .await
        .merge_member_sync(&mut node, files, remote)
        .await
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
        let client_runtime = self.runtime();
        let node = client_runtime.node.lock().await;
        let state = client_runtime.services.lock().await;
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
        let provider = crate::resources::bounded_contact_hint(
            provider, crate::client_invitation::MAX_CONTACT_ADDRESSES,
        );
        let client_runtime = self.runtime();
        client_runtime.require_ready()?;
        if !service.guest_supported() {
            return Err("service guest adapter unavailable".into());
        }
        let created = crate::service_policy::now();
        if expires <= created {
            return Err("guest deadline must be in the future".into());
        }
        let node = client_runtime.node.lock().await;
        let mut state = client_runtime.services.lock().await;
        let access = state.access(&node).await?;
        let provider_id = VerifyingKey::from_bytes(provider.id.as_bytes())?;
        if !access.ledger.rule(provider_id, service).rule.guests {
            return Err("guest sharing disabled by effective rule".into());
        }
        let token: [u8; 32] = rand::random();
        let id = state
            .apply(
                &node,
                &client_runtime.files,
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
        let (issuer, bootstrap) = client_runtime.resources.invitation_contact()?;
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
        let client_runtime = self.runtime();
        let invitation = client_runtime
            .services
            .lock()
            .await
            .guest
            .clone()
            .ok_or("not a guest invitation")?;
        let connection = client_runtime
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
            let mut node = client_runtime.node.lock().await;
            let mut state = client_runtime.services.lock().await;
            state.merge(&mut node, &client_runtime.files, bundle).await?;
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
        let client_runtime = self.runtime();
        let invitation = client_runtime
            .services
            .lock()
            .await
            .guest
            .clone()
            .ok_or("not a guest")?;
        let record = {
            let node = client_runtime.node.lock().await;
            let mut state = client_runtime.services.lock().await;
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
                        &client_runtime.files,
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
                let connection = client_runtime
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
        let client_runtime = self.runtime();
        let clipboard_capability = crate::system_clipboard::current_capability().await;
        client_runtime.clipboard
            .state
            .capability(clipboard_capability.clone());
        let clipboard = client_runtime.clipboard.state.status();
        let node = client_runtime.node.lock().await;
        let state = client_runtime.services.lock().await;
        let access = state.access(&node).await?;
        let clipboard_guest = state
            .guest
            .as_ref()
            .map(GuestInvitation::details)
            .transpose()?
            .is_some_and(|scope| scope.service == Service::Clipboard);
        let terminal_guest = state.guest.as_ref().map(GuestInvitation::details).transpose()?
            .is_some_and(|scope| scope.service == Service::Terminal);
        let mut status = access.status(
            state.guest.as_ref(),
            if terminal_guest { false } else if clipboard_guest {
                clipboard_capability.read && clipboard_capability.write
            } else {
                true
            },
            if terminal_guest { false } else if clipboard_guest {
                clipboard.enabled
            } else {
                client_runtime.files.enabled()
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
                    protocol.service(), true, client_runtime.forwarding.provider_enabled(protocol)));
            }
            status["services"].as_array_mut().ok_or("invalid service status")?.push(
                access.local_service_status(Service::Web, true, client_runtime.web.status()["enabled"] == true));
            let terminal = client_runtime.terminal.status();
            status["services"].as_array_mut().ok_or("invalid service status")?.push(
                access.local_service_status(Service::Terminal, terminal["capable"] == true, terminal["enabled"] == true));
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
        let client_runtime = self.runtime();
        client_runtime.require_ready()?;
        let node = client_runtime.node.lock().await;
        client_runtime.services
            .lock()
            .await
            .apply(
                &node,
                &client_runtime.files,
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
        let client_runtime = self.runtime();
        client_runtime.require_ready()?;
        let node = client_runtime.node.lock().await;
        client_runtime.services
            .lock()
            .await
            .apply(&node, &client_runtime.files, ServiceAction::EndGuest { invitation })
            .await
    }
    pub async fn sync_services(&self, address: iroh::EndpointAddr) -> Result<Value, ClientError> {
        let client_runtime = self.runtime();
        use crate::resources::ContactClass;
        client_runtime.require_ready()?;
        let deadline = client_runtime.resources.contact_deadline();
        let (connection, attempt) = client_runtime
            .resources
            .connect_data_attempt(
                address,
                crate::service_policy::POLICY_ALPN,
                ContactClass::Foreground,
                deadline,
            )
            .await?;
        attempt.succeeded();
        let result = sync_member(
            &connection,
            &client_runtime.node,
            &client_runtime.services,
            &client_runtime.files,
            MemberSyncMode::Full,
            deadline.into(),
        )
        .await;
        connection.close(0u32.into(), b"service policy exchange ended");
        result?;
        Ok(json!({"saved_locally":true}))
    }
}
