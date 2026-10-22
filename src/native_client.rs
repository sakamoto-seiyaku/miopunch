//! Controllable native client over the existing durable node and configured SDK.
//!
//! A host owns one client, calls operations, and awaits `close` before reopening
//! its directory. Invitations are capabilities: pass them through a trusted
//! channel and never include them in diagnostic output.
use std::{error::Error, net::Ipv4Addr, path::Path, sync::Arc, time::Duration};

use iroh::{EndpointAddr, endpoint::Connection};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{sync::Mutex, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::{
    admission_gate::{ADMISSION_REQUEST_ALPN, PRECODE_REQUEST_ALPN, ProtocolScope},
    control_protocol::{AuthenticatedControl, ControlOwner, OwnedControl},
    manual_admission::{AdmissionRequest, AdmissionStage, ManualExchange, ManualInvitation},
    native_network::{NativeNetwork, NodeStatus},
    precode::Precode,
    resources::ResourceEndpoint,
};

pub type ClientError = Box<dyn Error + Send + Sync>;
const MEMBER_MESSAGE: &[u8] = b"miopunch/client-message/1";

/// Trusted invitation plus current contact hints. Serialization is for private
/// delivery, not logs. The SDK still authenticates the contact and network.
#[derive(Serialize, Deserialize)]
pub struct ClientInvitation {
    version: u32,
    invitation: String,
    address: EndpointAddr,
    precode: Option<String>,
}
impl ClientInvitation {
    pub fn decode(bytes: &[u8]) -> Result<Self, ClientError> {
        let value: Self = serde_json::from_slice(bytes)?;
        let material = value.material()?;
        if value.version != 1
            || value.address.id != material.contact
            || value
                .code()?
                .is_some_and(|code| code.invitation() != material)
        {
            return Err("invalid invitation binding".into());
        }
        Ok(value)
    }
    pub fn encode(&self) -> Result<Vec<u8>, ClientError> {
        Ok(serde_json::to_vec(self)?)
    }
    fn material(&self) -> Result<ManualInvitation, ClientError> {
        Ok(ManualInvitation::decode(
            &hex::decode(&self.invitation)?
                .try_into()
                .map_err(|_| "invalid invitation length")?,
        )?)
    }
    fn code(&self) -> Result<Option<Precode>, ClientError> {
        self.precode
            .as_ref()
            .map(|value| {
                Ok(Precode::decode(
                    &hex::decode(value)?
                        .try_into()
                        .map_err(|_| "invalid precode length")?,
                )?)
            })
            .transpose()
    }
}

/// Secret-free local state, obtained after verifying the durable SDK records.
pub struct ClientStatus {
    pub node: NodeStatus,
    pub admission: AdmissionStage,
}

/// A bounded native host. Incoming admission and member exchanges run while the
/// caller controls the client; there is no UI, alternate transport or SSH path.
pub struct NativeClient {
    node: Arc<Mutex<NativeNetwork>>,
    resources: ResourceEndpoint,
    cancel: CancellationToken,
    _drop_cancel: tokio_util::sync::DropGuard,
    listener: JoinHandle<()>,
}
impl NativeClient {
    pub async fn create(path: impl AsRef<Path>, bind: Ipv4Addr) -> Result<Self, ClientError> {
        Self::start(NativeNetwork::create(path).await?, bind).await
    }
    pub async fn join(
        path: impl AsRef<Path>,
        invitation: &ClientInvitation,
        bind: Ipv4Addr,
    ) -> Result<Self, ClientError> {
        Self::start(
            NativeNetwork::join(path, invitation.material()?).await?,
            bind,
        )
        .await
    }
    pub async fn open(path: impl AsRef<Path>, bind: Ipv4Addr) -> Result<Self, ClientError> {
        Self::start(NativeNetwork::open(path).await?, bind).await
    }
    async fn start(node: NativeNetwork, bind: Ipv4Addr) -> Result<Self, ClientError> {
        node.admission_gate()
            .register_protocol(MEMBER_MESSAGE, ProtocolScope::Member);
        let resources = node.bind_configured_endpoint(bind).await?;
        resources.endpoint().set_alpns(vec![
            ADMISSION_REQUEST_ALPN.to_vec(),
            PRECODE_REQUEST_ALPN.to_vec(),
            MEMBER_MESSAGE.to_vec(),
        ]);
        let node = Arc::new(Mutex::new(node));
        let cancel = CancellationToken::new();
        let listener = {
            let endpoint = resources.endpoint().clone();
            let node = node.clone();
            let cancel = cancel.clone();
            tokio::spawn(async move {
                let mut handlers = tokio::task::JoinSet::new();
                loop {
                    tokio::select! {
                        _ = cancel.cancelled() => break,
                        incoming = endpoint.accept() => {
                            let Some(incoming) = incoming else { break };
                            if handlers.len() >= 16 { incoming.refuse(); continue; }
                            let node = node.clone();
                            handlers.spawn(async move {
                                let _ = tokio::time::timeout(Duration::from_secs(60), async {
                                    let mut accepting = incoming.accept()?;
                                    let alpn = accepting.alpn().await?;
                                    if matches!(alpn.as_slice(), ADMISSION_REQUEST_ALPN | PRECODE_REQUEST_ALPN) {
                                        let connection = Arc::new(accepting.into_control().await?);
                                        serve_admission(connection.retain_control(), node).await
                                    } else {
                                        let connection = accepting.await?;
                                        let result = async {
                                            let (send, recv) = connection.accept_bi().await?;
                                            crate::member_message::serve(send, recv).await?;
                                            // Await the caller's close, so the final reply is delivered.
                                            connection.closed().await;
                                            Ok::<_, ClientError>(())
                                        }.await;
                                        connection.close(0u32.into(), b"client exchange ended");
                                        result
                                    }
                                }).await;
                            });
                        },
                        _ = handlers.join_next(), if !handlers.is_empty() => {},
                    }
                }
                handlers.abort_all();
                while handlers.join_next().await.is_some() {}
            })
        };
        let drop_cancel = cancel.clone().drop_guard();
        Ok(Self {
            node,
            resources,
            cancel,
            _drop_cancel: drop_cancel,
            listener,
        })
    }
    pub async fn status(&self) -> ClientStatus {
        let node = self.node.lock().await;
        ClientStatus {
            node: node.status(),
            admission: node.admission_stage(),
        }
    }
    pub fn address(&self) -> EndpointAddr {
        self.resources.addr()
    }
    pub async fn invitation(&self) -> Result<ClientInvitation, ClientError> {
        Ok(ClientInvitation {
            version: 1,
            invitation: hex::encode(self.node.lock().await.manual_invitation()?.encode()),
            address: self.address(),
            precode: None,
        })
    }
    pub async fn issue_precode(
        &self,
        expires: u64,
        max_uses: Option<u64>,
    ) -> Result<ClientInvitation, ClientError> {
        let code = self
            .node
            .lock()
            .await
            .issue_precode(expires, max_uses)
            .await?;
        Ok(ClientInvitation {
            version: 1,
            invitation: hex::encode(code.invitation().encode()),
            address: self.address(),
            precode: Some(hex::encode(code.encode())),
        })
    }
    pub async fn precode_uses(&self, invitation: &ClientInvitation) -> Result<u64, ClientError> {
        let code = invitation.code()?.ok_or("not a precode")?;
        let status = self.node.lock().await.precode_usage_status(&code).await?;
        if !status.count_trusted {
            return Err("untrusted precode count".into());
        }
        status
            .saved_uses
            .ok_or_else(|| "precode count unavailable".into())
    }
    pub async fn requests(&self) -> Result<Vec<AdmissionRequest>, ClientError> {
        Ok(self.node.lock().await.manual_requests().await?)
    }
    pub async fn approve(&self, request: &AdmissionRequest) -> Result<AdmissionStage, ClientError> {
        Ok(self
            .node
            .lock()
            .await
            .approve_manual_admission(request)
            .await?
            .stage)
    }
    /// Resume the same durable SDK attempt. Complete is returned only after
    /// verified membership and the current admission secret have been saved.
    pub async fn request_admission(
        &self,
        invitation: &ClientInvitation,
    ) -> Result<AdmissionStage, ClientError> {
        self.receive_admission(invitation, true).await
    }
    /// Receive the verified authorization without finishing secret delivery.
    /// Hosts can stop here and resume `request_admission` after reopening. This
    /// state is deliberately never reported as completed admission.
    pub async fn receive_authorization(
        &self,
        invitation: &ClientInvitation,
    ) -> Result<AdmissionStage, ClientError> {
        self.receive_admission(invitation, false).await
    }
    async fn receive_admission(
        &self,
        invitation: &ClientInvitation,
        save: bool,
    ) -> Result<AdmissionStage, ClientError> {
        let code = invitation.code()?;
        if let Some(code) = &code {
            self.node
                .lock()
                .await
                .begin_precode_completion(code)
                .await?;
        }
        let connection = self
            .resources
            .connect_control(
                invitation.address.clone(),
                if code.is_some() {
                    PRECODE_REQUEST_ALPN
                } else {
                    ADMISSION_REQUEST_ALPN
                },
            )
            .await?;
        let result = async {
            let mut node = self.node.lock().await;
            let exchange = if let Some(code) = &code {
                node.request_precode_admission(&connection, code).await?
            } else {
                node.request_manual_admission(&connection).await?
            };
            if let ManualExchange::Materials(delivery) = exchange {
                if save {
                    node.complete_manual_admission(delivery).await?;
                }
            }
            Ok(node.admission_stage())
        }
        .await;
        connection.close(0u32.into(), b"client admission ended");
        result
    }
    /// Finite member business exchange through configured SDK admission and
    /// route selection. Diagnostics report the actual selected connection path.
    pub async fn ping(&self, address: EndpointAddr) -> Result<Value, ClientError> {
        let connection = self.resources.connect(address, MEMBER_MESSAGE).await?;
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            let (mut send, mut recv) = connection.open_bi().await?;
            send.write_all(b"PING").await?;
            send.finish()?;
            if recv.read_to_end(64).await? != b"PONG" {
                return Err("unexpected member response".into());
            }
            Ok::<_, ClientError>(json!({"reply": "PONG", "transport": path_status(&connection)}))
        })
        .await;
        connection.close(0u32.into(), b"client ping ended");
        result?
    }
    /// Stop this host's listener and handlers, close endpoint resources, then
    /// close the durable store. Identity and authorization remain on disk.
    pub async fn close(self) -> Result<(), ClientError> {
        self.cancel.cancel();
        self.listener.await?;
        self.resources.close().await;
        Arc::try_unwrap(self.node)
            .map_err(|_| "client node still owned")?
            .into_inner()
            .close()
            .await;
        Ok(())
    }
}
async fn serve_admission(
    connection: ControlOwner,
    node: Arc<Mutex<NativeNetwork>>,
) -> Result<(), ClientError> {
    let result = match connection.alpn() {
        PRECODE_REQUEST_ALPN => node.lock().await.serve_precode_admission(&connection).await,
        ADMISSION_REQUEST_ALPN => node.lock().await.serve_manual_admission(&connection).await,
        _ => return Err("unsupported admission operation".into()),
    };
    if let Err(error) = result {
        connection.close(0u32.into(), b"client admission failed");
        return Err(error.into());
    }
    connection.revocation_handle().closed().await;
    Ok(())
}

fn path_status(connection: &Connection) -> Value {
    json!({"peer": connection.remote_id().to_string(), "connection": connection.stable_id(),
        "paths": connection.paths().iter().map(|path| json!({"remote": path.remote_addr(),
            "local": format!("{:?}", path.local_addr()), "selected": path.is_selected(),
            "tx_bytes": path.stats().udp_tx.bytes, "rx_bytes": path.stats().udp_rx.bytes})).collect::<Vec<_>>()})
}
