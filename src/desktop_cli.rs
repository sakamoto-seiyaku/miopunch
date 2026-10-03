//! A thin, local JSON-lines test shell over the public native node interface.
//! Invitation capabilities are written to private files, never diagnostic output.
use std::{error::Error, fs::OpenOptions, io::Write, net::Ipv4Addr, path::Path, sync::Arc};

use iroh::{EndpointAddr, endpoint::Connection};
use miopunch_native_probe::{
    admission_gate::{ADMISSION_REQUEST_ALPN, PRECODE_REQUEST_ALPN, ProtocolScope},
    daily_management::{ManagementAction, ManagementError, ManagementSession},
    genesis_recovery::GenesisCredential,
    management_store::StoreError,
    manual_admission::{AdmissionRequest, ManualExchange, ManualInvitation},
    native_network::NativeNetwork,
    offline_replica::REPLICA_ALPN,
    precode::Precode,
    resources::ResourceEndpoint,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    sync::Mutex,
};

const BUSINESS: &[u8] = b"miopunch/desktop-test/1";
type CommandError = Box<dyn Error + Send + Sync>;

#[derive(Serialize, Deserialize)]
struct InvitationFile {
    version: u32,
    invitation: String,
    address: EndpointAddr,
    #[serde(default)]
    precode: Option<String>,
}

impl InvitationFile {
    fn read(path: &str) -> Result<Self, CommandError> {
        let saved: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        let invitation = saved.material()?;
        if saved.version != 1
            || saved.address.id != invitation.contact
            || saved
                .code()?
                .is_some_and(|code| code.invitation() != invitation)
        {
            return Err("invalid invitation binding".into());
        }
        Ok(saved)
    }

    fn code(&self) -> Result<Option<Precode>, CommandError> {
        self.precode
            .as_ref()
            .map(|encoded| {
                Ok(Precode::decode(
                    &hex::decode(encoded)?
                        .try_into()
                        .map_err(|_| "invalid precode length")?,
                )?)
            })
            .transpose()
    }

    fn material(&self) -> Result<ManualInvitation, CommandError> {
        Ok(ManualInvitation::decode(
            &hex::decode(&self.invitation)?
                .try_into()
                .map_err(|_| "invalid invitation length")?,
        )?)
    }
}

pub async fn join(path: &str, invitation_path: &str) -> Result<NativeNetwork, Box<dyn Error>> {
    let invitation =
        InvitationFile::read(invitation_path).map_err(|_| "invalid invitation file")?;
    Ok(NativeNetwork::join(
        path,
        invitation.material().map_err(|_| "invalid invitation")?,
    )
    .await?)
}

#[derive(Deserialize)]
struct Request {
    id: u64,
    #[serde(flatten)]
    command: Command,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Command {
    Invite {
        path: String,
    },
    IssuePrecode {
        path: String,
        seconds: u64,
        max_uses: Option<u64>,
    },
    PrecodeStatus {
        path: String,
    },
    RevokePrecode {
        path: String,
    },
    RefreshInvitation {
        path: String,
        destination: String,
    },
    Request {
        path: String,
        #[serde(default)]
        pause_before_save: bool,
    },
    Requests,
    Approve {
        applicant: String,
        request_id: String,
    },
    Status,
    Resources,
    Observe,
    SaveResources {
        config: miopunch_native_probe::resources::ResourceConfig,
    },
    Ping {
        address: EndpointAddr,
        #[serde(default)]
        hints_only: bool,
    },
    Sync {
        address: EndpointAddr,
        #[serde(default)]
        hints_only: bool,
    },
    Address,
    OpenBusiness {
        address: EndpointAddr,
        #[serde(default)]
        hints_only: bool,
    },
    Business,
    BusinessStatus,
    Manage {
        action: Action,
        target: p2panda_core::VerifyingKey,
    },
    Elevate {
        path: String,
        seconds: u64,
    },
    ExitElevation,
    Stop,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    AddMember,
    RemoveMember,
    AppointAdministrator,
    DemoteAdministrator,
}

fn error_code(error: &(dyn Error + Send + Sync + 'static)) -> &'static str {
    if let Some(error) = error.downcast_ref::<ManagementError>() {
        return match error {
            ManagementError::ElevationRequired => "elevation_required",
            ManagementError::InvalidCredential => "invalid_genesis_credential",
            ManagementError::InvalidSession => "invalid_management_session",
            ManagementError::Store(StoreError::Auth(_)) => "permission_denied",
            ManagementError::Store(_) | ManagementError::Node(_) => {
                "storage_or_verification_failed"
            }
            ManagementError::InvalidWindow => "invalid_elevation_window",
        };
    }
    if error.is::<miopunch_native_probe::resources::ResourceContactError>() {
        return "temporarily_unreachable";
    }
    if let Some(error) =
        error.downcast_ref::<miopunch_native_probe::manual_admission::ManualAdmissionError>()
    {
        use miopunch_native_probe::manual_admission::ManualAdmissionError;
        return match error {
            ManualAdmissionError::Rejected => "admission_rejected",
            ManualAdmissionError::PathUnavailable => "delivery_incomplete",
            ManualAdmissionError::Store(StoreError::AdmissionPaused) => "admission_paused",
            _ => "storage_or_verification_failed",
        };
    }
    "invalid_request_or_operation_failed"
}

fn status(node: &NativeNetwork) -> Value {
    let status = node.status();
    json!({"node": status.node_id.to_string(), "network": status.anchor.root.to_string(),
        "genesis": status.anchor.genesis.to_string(), "stage": format!("{:?}", node.admission_stage()),
        "records": status.applied_count, "members": status.members.iter().map(|(id, access)|
            json!({"node": id.to_string(), "role": if *access == p2panda_auth::Access::manage() {"admin"} else {"member"}})
        ).collect::<Vec<_>>()})
}

fn write_private(path: &str, value: &impl Serialize) -> Result<(), CommandError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(Path::new(path))?;
    file.write_all(&serde_json::to_vec(value)?)?;
    file.sync_all()?;
    Ok(())
}

fn emit(value: Value) {
    println!("{value}");
    let _ = std::io::stdout().flush();
}

fn paths(connection: &Connection) -> Value {
    json!({"connection": connection.stable_id(), "peer": connection.remote_id().to_string(),
        "paths": connection.paths().iter().map(|path| json!({"remote": path.remote_addr(),
            "local": format!("{:?}", path.local_addr()), "selected": path.is_selected(),
            "tx_bytes": path.stats().udp_tx.bytes, "rx_bytes": path.stats().udp_rx.bytes,
            "stream_tx": path.stats().frame_tx.stream, "stream_rx": path.stats().frame_rx.stream})).collect::<Vec<_>>()})
}

async fn dispatch(
    command: Command,
    node: &mut NativeNetwork,
    resources: &ResourceEndpoint,
    session: &mut ManagementSession,
    held: &mut Option<Connection>,
) -> Result<Value, CommandError> {
    match command {
        Command::Invite { path } => {
            write_private(
                &path,
                &InvitationFile {
                    version: 1,
                    invitation: hex::encode(node.manual_invitation()?.encode()),
                    address: resources.addr(),
                    precode: None,
                },
            )?;
            Ok(json!({"saved": true}))
        }
        Command::IssuePrecode {
            path,
            seconds,
            max_uses,
        } => {
            let expires = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs()
                .checked_add(seconds)
                .ok_or("invalid expiry")?;
            let code = node.issue_precode(expires, max_uses).await?;
            write_private(
                &path,
                &InvitationFile {
                    version: 1,
                    invitation: hex::encode(code.invitation().encode()),
                    address: resources.addr(),
                    precode: Some(hex::encode(code.encode())),
                },
            )?;
            Ok(json!({"saved": true}))
        }
        Command::PrecodeStatus { path } => {
            let code = InvitationFile::read(&path)?
                .code()?
                .ok_or("not a precode")?;
            let result = node.precode_usage_status(&code).await?;
            Ok(json!({"uses": result.saved_uses, "count_trusted": result.count_trusted}))
        }
        Command::RevokePrecode { path } => {
            let code = InvitationFile::read(&path)?
                .code()?
                .ok_or("not a precode")?;
            node.revoke_precode(&code).await?;
            Ok(json!({"revoked": true}))
        }
        Command::RefreshInvitation { path, destination } => {
            let mut invitation = InvitationFile::read(&path)?;
            if invitation.material()? != node.manual_invitation()? {
                return Err("wrong invitation contact".into());
            }
            invitation.address = resources.addr();
            write_private(&destination, &invitation)?;
            Ok(json!({"saved": true}))
        }
        Command::Request {
            path,
            pause_before_save,
        } => {
            let invitation = InvitationFile::read(&path)?;
            let code = invitation.code()?;
            if let Some(code) = &code {
                node.begin_precode_completion(code).await?;
            }
            let alpn = if code.is_some() {
                PRECODE_REQUEST_ALPN
            } else {
                ADMISSION_REQUEST_ALPN
            };
            let connection = resources.connect(invitation.address, alpn).await?;
            let result = async {
                let exchange = if let Some(code) = &code {
                    node.request_precode_admission(&connection, code).await?
                } else {
                    node.request_manual_admission(&connection).await?
                };
                match exchange {
                    ManualExchange::AwaitingApproval => {}
                    ManualExchange::Materials(materials) => {
                        if !pause_before_save {
                            node.complete_manual_admission(materials).await?;
                        }
                    }
                }
                Ok::<_, CommandError>(json!({"stage": format!("{:?}", node.admission_stage())}))
            }
            .await;
            connection.close(0u32.into(), b"desktop request ended");
            result
        }
        Command::Requests => Ok(
            json!({"requests": node.manual_requests().await?.iter().map(|request|
            json!({"applicant": request.applicant.to_string(), "request_id": hex::encode(request.request_id)})
        ).collect::<Vec<_>>()}),
        ),
        Command::Approve {
            applicant,
            request_id,
        } => {
            let request = AdmissionRequest {
                applicant: applicant.parse()?,
                request_id: hex::decode(request_id)?
                    .try_into()
                    .map_err(|_| "invalid request id")?,
            };
            let approved = node.approve_manual_admission(&request).await?;
            Ok(json!({"stage": format!("{:?}", approved.stage)}))
        }
        Command::Status => Ok(status(node)),
        Command::Resources => Ok(json!({"config": node.resource_configuration()?})),
        Command::Observe => {
            let mut results = Vec::new();
            for origin in node.resource_configuration()?.udp_stun_resources() {
                let started = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_millis();
                match resources.endpoint().observe_udp_stun(&origin).await {
                    Ok(observation) => results.push(json!({"service": origin, "protocol": "STUN/UDP/RFC8489",
                        "started_unix_ms": started, "destination": observation.destination,
                        "local_socket": observation.local_addr, "mapped": observation.mapped,
                        "transaction_and_source_validated": true, "fresh": observation.is_fresh(),
                        "valid_for_ms": observation.valid_until.saturating_duration_since(std::time::Instant::now()).as_millis()})),
                    Err(error) => results.push(json!({"service": origin, "protocol": "STUN/UDP/RFC8489",
                        "started_unix_ms": started, "fresh": false, "error": error.to_string()})),
                }
            }
            for origin in node.resource_configuration()?.tcp_stun_resources() {
                let started = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_millis();
                match resources.observe_tcp_stun(&origin).await {
                    Ok(observation) => results.push(json!({"service": origin, "protocol": "STUN/TCP/RFC8489",
                        "started_unix_ms": started, "destination": observation.destination,
                        "transport_listener": observation.listener, "local_socket": observation.local_addr, "mapped": observation.mapped,
                        "transaction_and_source_validated": true, "retained_connection": true, "fresh": observation.is_fresh(),
                        "valid_for_ms": observation.valid_until.saturating_duration_since(std::time::Instant::now()).as_millis()})),
                    Err(error) => results.push(json!({"service": origin, "protocol": "STUN/TCP/RFC8489",
                        "started_unix_ms": started, "fresh": false, "error": error.to_string()})),
                }
            }
            Ok(
                json!({"bound_sockets": resources.endpoint().bound_sockets(), "observations": results}),
            )
        }
        Command::SaveResources { config } => {
            node.save_resource_configuration(&config)?;
            Ok(json!({"effective_on_restart": true}))
        }
        Command::Address => Ok(
            json!({"address": resources.addr(), "custom": resources.endpoint().custom_addrs(), "mqtt": resources.mqtt_diagnostics()}),
        ),
        Command::OpenBusiness {
            address,
            hints_only,
        } => {
            if let Some(previous) = held.take() {
                previous.close(0u32.into(), b"replaced test connection");
            }
            let connection = if hints_only {
                tokio::time::timeout(
                    std::time::Duration::from_secs(25),
                    resources.endpoint().connect(address, BUSINESS),
                )
                .await??
            } else {
                resources.connect(address, BUSINESS).await?
            };
            let result = paths(&connection);
            *held = Some(connection);
            Ok(result)
        }
        Command::Business => {
            let connection = held.as_ref().ok_or("no held business connection")?;
            tokio::time::timeout(std::time::Duration::from_secs(3), async {
                let (mut send, mut recv) = connection.open_bi().await?;
                send.write_all(b"PING").await?;
                send.finish()?;
                if recv.read_to_end(64).await? != b"PONG" {
                    return Err("unexpected reply".into());
                }
                Ok::<_, CommandError>(json!({"reply": "PONG", "transport": paths(connection)}))
            })
            .await?
        }
        Command::BusinessStatus => Ok(json!({"transport": held.as_ref().map(paths),
            "close_reason": held.as_ref().and_then(|connection| connection.close_reason()).map(|reason| reason.to_string())})),
        Command::Ping {
            address,
            hints_only,
        } => {
            // Advanced Endpoint hints are useful for measuring a single carrier's
            // bootstrap. The configured Endpoint still enforces all resource and
            // admission rules; ordinary calls retain ResourceEndpoint recovery.
            let connection = if hints_only {
                tokio::time::timeout(
                    std::time::Duration::from_secs(25),
                    resources.endpoint().connect(address, BUSINESS),
                )
                .await??
            } else {
                resources.connect(address, BUSINESS).await?
            };
            let result = async {
                let (mut send, mut recv) = connection.open_bi().await?;
                send.write_all(b"PING").await?;
                send.finish()?;
                if recv.read_to_end(64).await? != b"PONG" {
                    return Err("unexpected reply".into());
                }
                Ok::<_, CommandError>(json!({"reply": "PONG", "transport": paths(&connection)}))
            }
            .await;
            connection.close(0u32.into(), b"desktop ping ended");
            result
        }
        Command::Sync {
            address,
            hints_only,
        } => {
            let connection = if hints_only {
                tokio::time::timeout(
                    std::time::Duration::from_secs(25),
                    resources.endpoint().connect(address, REPLICA_ALPN),
                )
                .await??
            } else {
                resources.connect(address, REPLICA_ALPN).await?
            };
            let result = node.sync_replica(&connection).await;
            let transport = paths(&connection);
            connection.close(0u32.into(), b"desktop sync ended");
            let result = result?;
            Ok(
                json!({"complete": result.complete(), "source": result.source.to_string(), "transport": transport}),
            )
        }
        Command::Manage { action, target } => {
            let action = match action {
                Action::AddMember => ManagementAction::AddMember(target),
                Action::RemoveMember => ManagementAction::RemoveMember(target),
                Action::AppointAdministrator => ManagementAction::AppointAdministrator(target),
                Action::DemoteAdministrator => ManagementAction::DemoteAdministrator(target),
            };
            let result = node.manage(session, action).await?;
            Ok(json!({"record": result.record.to_string()}))
        }
        Command::Elevate { path, seconds } => {
            let bytes: [u8; 64] = std::fs::read(path)?
                .try_into()
                .map_err(|_| "invalid credential file")?;
            let credential = GenesisCredential::from_saved(
                bytes[..32].try_into().unwrap(),
                p2panda_core::Hash::from_bytes(bytes[32..].try_into().unwrap()),
            );
            node.elevate_genesis(session, credential, std::time::Duration::from_secs(seconds))?;
            Ok(json!({"elevated": true, "seconds": seconds}))
        }
        Command::ExitElevation => {
            session.exit_genesis_elevation();
            Ok(json!({"elevated": false}))
        }
        Command::Stop => Ok(json!({"stopped": true})),
    }
}

async fn serve(
    connection: Connection,
    node: Arc<Mutex<NativeNetwork>>,
) -> Result<(), CommandError> {
    let result = async {
        if matches!(
            connection.alpn(),
            ADMISSION_REQUEST_ALPN | PRECODE_REQUEST_ALPN
        ) {
            let stage = if connection.alpn() == PRECODE_REQUEST_ALPN {
                node.lock()
                    .await
                    .serve_precode_admission(&connection)
                    .await?
            } else {
                node.lock()
                    .await
                    .serve_manual_admission(&connection)
                    .await?
            };
            emit(json!({"event": "admission", "stage": format!("{stage:?}")}));
        } else if connection.alpn() == REPLICA_ALPN {
            node.lock().await.serve_replica(&connection).await?;
        } else if connection.alpn() == BUSINESS {
            loop {
                let (mut send, mut recv) = tokio::select! {
                    streams = connection.accept_bi() => streams?,
                    _ = connection.closed() => break,
                };
                if recv.read_to_end(64).await? != b"PING" {
                    return Err("unexpected request".into());
                }
                send.write_all(b"PONG").await?;
                send.finish()?;
            }
        }
        connection.closed().await;
        Ok::<_, CommandError>(())
    }
    .await;
    if result.is_err() {
        connection.close(0u32.into(), b"desktop request failed");
    }
    result
}

pub async fn session(path: &str, bind_ip: Ipv4Addr) -> Result<(), Box<dyn Error>> {
    let node = NativeNetwork::open(path).await?;
    node.admission_gate()
        .register_protocol(BUSINESS, ProtocolScope::Member);
    node.admission_gate()
        .register_protocol(REPLICA_ALPN, ProtocolScope::Member);
    let mut session = node.begin_management_session();
    let resources = if let Ok(proxy) = std::env::var("MIOPUNCH_RELAY_PROXY") {
        node.bind_configured_endpoint_with(
            iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
                .bind_addr((bind_ip, 0))?
                .proxy_url(proxy.parse()?),
        )
        .await?
    } else {
        node.bind_configured_endpoint(bind_ip).await?
    };
    resources.endpoint().set_alpns(vec![
        ADMISSION_REQUEST_ALPN.to_vec(),
        PRECODE_REQUEST_ALPN.to_vec(),
        BUSINESS.to_vec(),
        REPLICA_ALPN.to_vec(),
    ]);
    emit(
        json!({"event": "ready", "node": node.status().node_id.to_string(), "address": resources.addr()}),
    );
    let node = Arc::new(Mutex::new(node));
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut handlers = tokio::task::JoinSet::new();
    let mut held = None;
    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Some(line) = line? else { break };
                let request = match serde_json::from_str::<Request>(&line) {
                    Ok(request) => request,
                    Err(_) => { emit(json!({"id": null, "ok": false, "error": "invalid_command"})); continue; }
                };
                let stop = matches!(request.command, Command::Stop);
                let result = dispatch(request.command, &mut *node.lock().await, &resources, &mut session, &mut held).await;
                match result {
                    Ok(result) => emit(json!({"id": request.id, "ok": true, "result": result})),
                    Err(error) => emit(json!({"id": request.id, "ok": false, "error": error_code(error.as_ref())})),
                }
                if stop { break }
            },
            incoming = resources.endpoint().accept() => {
                let Some(incoming) = incoming else { break };
                let node = node.clone();
                handlers.spawn(async move { serve(incoming.await?, node).await });
            },
            Some(result) = handlers.join_next(), if !handlers.is_empty() => {
                match result {
                    Ok(Err(error)) => emit(json!({"event": "incoming_failed", "error": error_code(error.as_ref())})),
                    Err(_) => emit(json!({"event": "incoming_failed", "error": "handler_failed"})),
                    Ok(Ok(())) => {},
                }
            },
        }
    }
    session.end();
    if let Some(connection) = held.take() {
        connection.close(0u32.into(), b"test shell stopped");
    }
    resources.close().await;
    handlers.abort_all();
    while handlers.join_next().await.is_some() {}
    Arc::try_unwrap(node)
        .map_err(|_| "node still owned")?
        .into_inner()
        .close()
        .await;
    Ok(())
}
