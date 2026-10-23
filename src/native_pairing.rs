//! Dedicated reverse-pairing receiver before a network has been selected.
use crate::{
    client_invitation::ClientInvitation,
    control_protocol::{AuthenticatedControl, OwnedControl},
    invitation_pairing::{PAIRING_ALPN, PairingGate, PairingInfo},
    native_client::{ClientError, NativeClient},
    native_network::{NativeNetwork, NodeError},
    resources::{ResourceConfig, ResourceUse},
};
use iroh::endpoint::presets;
use std::{
    net::Ipv4Addr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{sync::Mutex, task::JoinHandle};
use tokio_util::sync::CancellationToken;

const MAX_PAIRING_HANDLERS: usize = 4;

/// No member state is created until a capability-authorized, peer-bound full
/// invitation is durably published. `join` then opens that independent node;
/// ordinary approval/precode admission is still required.
pub struct NativePairing {
    info: PairingInfo,
    received: Arc<Mutex<Option<Vec<u8>>>>,
    path: PathBuf,
    cancel: CancellationToken,
    _drop_cancel: tokio_util::sync::DropGuard,
    listener: JoinHandle<()>,
}
impl NativePairing {
    pub async fn start(
        path: impl AsRef<Path>,
        bind: Ipv4Addr,
        config: ResourceConfig,
    ) -> Result<Self, ClientError> {
        if path.as_ref().exists() {
            return Err("pairing target already exists".into());
        }
        config.validate_contact_facilities()?;
        let key = iroh_base::SecretKey::generate();
        let builder = iroh::Endpoint::builder(presets::Minimal)
            .secret_key(key.clone())
            .bind_addr((bind, 0))?
            .alpns(vec![PAIRING_ALPN.to_vec()])
            .relay_mode(iroh::RelayMode::Custom(
                config.relay_map(ResourceUse::DataRelay)?,
            ))
            .observation_servers(config.relay_map(ResourceUse::Observation)?);
        let endpoint = config
            .configure(builder)?
            .restricted_application(PairingGate)
            .bind()
            .await?;
        let mut address = endpoint.addr();
        address.addrs.retain(|route| match route {
            iroh::TransportAddr::Ip(_) => !config.udp_direct_disabled,
            iroh::TransportAddr::Relay(url) => config.allows(url.as_str(), ResourceUse::DataRelay),
            _ => false,
        });
        address.addrs.extend(
            endpoint
                .custom_addrs()
                .into_iter()
                .filter(|a| config.allows_custom(a))
                .map(iroh::TransportAddr::Custom),
        );
        let bootstrap = crate::client_invitation::BootstrapContact {
            relays: config
                .relay_map(ResourceUse::DataRelay)?
                .urls::<Vec<_>>()
                .into_iter()
                .map(|url| url.to_string())
                .collect(),
            ..Default::default()
        };
        let info = PairingInfo::new(address, bootstrap);
        let received = Arc::new(Mutex::new(None::<Vec<u8>>));
        let cancel = CancellationToken::new();
        let path = path.as_ref().to_path_buf();
        let listener = {
            let cancel = cancel.clone();
            let received = received.clone();
            let info = info.clone();
            let path = path.clone();
            tokio::spawn(async move {
                let mut handlers = tokio::task::JoinSet::new();
                loop {
                    tokio::select! {
                        _=cancel.cancelled()=>break,
                        incoming=endpoint.accept()=> {
                            let Some(incoming)=incoming else {break};
                            if handlers.len()>=MAX_PAIRING_HANDLERS {incoming.refuse();continue;}
                            let received=received.clone();
                            let info=info.clone();
                            let path=path.clone();
                            let key=key.clone();
                            let config=config.clone();
                            let local=endpoint.id();
                            handlers.spawn(async move {
                                let _=tokio::time::timeout(Duration::from_secs(30),async {
                                    let connection=Arc::new(incoming.accept()?.into_control().await?).retain_control();
                                    let result=async {
                                        let delivery=crate::invitation_pairing::receive(&connection,local,&info).await?;
                                        {
                                            // Serialize publication only. An abandoned TLS C or
                                            // a peer's delayed close must not block another retry.
                                            let mut saved=received.lock().await;
                                            if saved.as_ref().is_some_and(|bytes|bytes!=&delivery.bytes) {return Err("paired invitation cannot be replaced".into());}
                                            if saved.is_none() {
                                                let config=crate::client_contact::import_contact(config,delivery.invitation.bootstrap())?;
                                                save_pending(&path,&delivery.invitation,&p2panda_core::SigningKey::from_bytes(&key.to_bytes()),&config).await?;
                                                *saved=Some(delivery.bytes.clone());
                                            }
                                        }
                                        delivery.acknowledge().await?;
                                        connection.revocation_handle().closed().await;
                                        Ok::<_,ClientError>(())
                                    }.await;
                                    connection.close(0u32.into(),b"invitation delivery ended");
                                    result
                                }).await;
                            });
                        },
                        _=handlers.join_next(), if !handlers.is_empty()=>{},
                    }
                }
                handlers.abort_all();
                while handlers.join_next().await.is_some() {}
                endpoint.close().await;
            })
        };
        let drop_cancel = cancel.clone().drop_guard();
        Ok(Self {
            info,
            received,
            path,
            cancel,
            _drop_cancel: drop_cancel,
            listener,
        })
    }
    /// Private return for trusted transfer to the administrator's scanner.
    pub fn information(&self) -> &PairingInfo {
        &self.info
    }
    pub async fn received(&self) -> bool {
        self.received.lock().await.is_some()
    }
    pub async fn invitation(&self) -> Result<ClientInvitation, ClientError> {
        Ok(ClientInvitation::decode(
            self.received
                .lock()
                .await
                .as_deref()
                .ok_or("no invitation received")?,
        )?)
    }
    pub async fn join(self, bind: Ipv4Addr) -> Result<NativeClient, ClientError> {
        if !self.received().await {
            return Err("no invitation received".into());
        }
        self.cancel.cancel();
        self.listener.await?;
        NativeClient::open(self.path, bind).await
    }
    pub async fn close(self) -> Result<(), ClientError> {
        self.cancel.cancel();
        self.listener.await?;
        Ok(())
    }
}

/// A canceled handler can have published the complete node before its in-memory
/// receipt was set. Retry may reopen that exact state, never overwrite another
/// identity or invitation that appeared at the target path.
async fn save_pending(
    path: &Path,
    invitation: &ClientInvitation,
    key: &p2panda_core::SigningKey,
    config: &ResourceConfig,
) -> Result<(), ClientError> {
    let node = match NativeNetwork::join_from_pairing(path, invitation, key, config).await {
        Ok(node) => node,
        Err(NodeError::AlreadyExists) => {
            let node = NativeNetwork::open(path).await?;
            let matches = (|| -> Result<bool, ClientError> {
                Ok(node.node_key.verifying_key() == key.verifying_key()
                    && node.anchor == invitation.anchor()?
                    && std::fs::read(path.join("client.invitation"))? == invitation.encode()?)
            })();
            node.close().await;
            if !matches? {
                return Err("pairing target contains different saved materials".into());
            }
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    node.close().await;
    Ok(())
}
