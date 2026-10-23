//! Minimal non-UI host for the public NativeClient operations.
use miopunch_native_probe::{
    manual_admission::AdmissionRequest,
    native_client::{ClientError, ClientInvitation, NativeClient},
    native_pairing::NativePairing,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{fs::OpenOptions, io::Write, net::Ipv4Addr};
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Deserialize)]
struct Request {
    id: u64,
    #[serde(flatten)]
    command: Command,
}
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Command {
    ExportRecoveryCard {
        path: String,
        password_path: Option<String>,
    },
    AcknowledgeRecoverySaved,
    InspectRecoveryCard {
        path: String,
        password_path: Option<String>,
    },
    Status,
    Address,
    Invite {
        path: String,
        #[serde(default)]
        text: bool,
    },
    PairingInformation {
        path: String,
    },
    PairingStatus,
    FinishPairing {
        path: String,
    },
    DeliverInvitation {
        pairing_path: String,
        invitation_path: String,
    },
    IssuePrecode {
        path: String,
        seconds: u64,
        max_uses: Option<u64>,
    },
    PrecodeStatus {
        path: String,
    },
    Request {
        path: String,
        #[serde(default)]
        authorization_only: bool,
    },
    Requests,
    Reject {
        applicant: String,
        request_id: String,
    },
    NewManualAttempt {
        path: String,
    },
    Approve {
        applicant: String,
        request_id: String,
    },
    Ping {
        address: iroh::EndpointAddr,
    },
    FilesEnable {
        directory: String,
    },
    FilesDisable,
    FilesStatus,
    ServiceStatus,
    ServiceRule {
        provider: p2panda_core::VerifyingKey,
        service: miopunch_native_probe::service_policy::Service,
        previous: Option<p2panda_core::Hash>,
        rule: miopunch_native_probe::service_policy::ServiceRule,
    },
    ServiceSync {
        address: iroh::EndpointAddr,
    },
    GuestIssue {
        path: String,
        provider: iroh::EndpointAddr,
        seconds: u64,
    },
    GuestRedeem,
    GuestStatus,
    GuestEnd,
    GuestRevoke {
        invitation: p2panda_core::Hash,
    },
    FileApprove {
        task: String,
    },
    FileReject {
        task: String,
    },
    FileSend {
        address: iroh::EndpointAddr,
        path: String,
    },
    FileCancel {
        task: String,
    },
    FileDisconnect {
        task: String,
    },
    Stop,
}
fn read_invitation(path: &str) -> Result<ClientInvitation, ClientError> {
    Ok(ClientInvitation::decode(&std::fs::read(path)?)?)
}
fn save_invitation(path: &str, invitation: &ClientInvitation) -> Result<Value, ClientError> {
    save_private(path, &invitation.encode()?)
}
fn save_private(path: &str, bytes: &[u8]) -> Result<Value, ClientError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(json!({"saved":true}))
}
async fn status(client: &NativeClient) -> Value {
    let status = client.status().await;
    let guest = client.guest_status().await.ok().flatten();
    let mut result = json!({"node":status.node.node_id.to_string(), "network":status.node.anchor.root.to_string(),
        "genesis":status.node.anchor.genesis.to_string(),
        "stage":if !status.recovery_saved {"AwaitingRecoverySave".into()} else if status.manual_rejected {"Rejected".into()} else {format!("{:?}",status.admission)},
        "automatic_retry":!status.manual_rejected,
        "recovery_saved":status.recovery_saved, "records":status.node.applied_count,
        "members":status.node.members.iter().map(|(key,role)| json!({"node":key.to_string(),
            "role":if *role == p2panda_auth::Access::manage() {"admin"} else {"member"}})).collect::<Vec<_>>()});
    if let Some(guest) = guest {
        result["stage"] = guest["stage"].clone();
        result["guest"] = guest;
        result["members"] = json!([]);
    }
    result
}
async fn dispatch(client: &NativeClient, command: Command) -> Result<Value, ClientError> {
    match command {
        Command::ExportRecoveryCard {
            path,
            password_path,
        } => {
            let password = password_path.map(std::fs::read_to_string).transpose()?;
            client
                .export_recovery_card_to(&path, password.as_deref())
                .await?;
            Ok(json!({"saved":true}))
        }
        Command::AcknowledgeRecoverySaved => {
            client.acknowledge_recovery_saved().await?;
            Ok(json!({"saved_acknowledged":true}))
        }
        Command::InspectRecoveryCard {
            path,
            password_path,
        } => {
            let password = password_path.map(std::fs::read_to_string).transpose()?;
            let card = miopunch_native_probe::recovery_card::RecoveryCard::import(
                &std::fs::read_to_string(path)?,
                password.as_deref(),
            )?;
            Ok(
                json!({"network":card.anchor().root.to_string(),"genesis":card.anchor().genesis.to_string()}),
            )
        }
        Command::Status => Ok(status(client).await),
        Command::Address => Ok(json!({"address":client.address()})),
        Command::Invite { path, text } => {
            let invitation = client.invitation().await?;
            if text {
                save_private(&path, invitation.text()?.as_bytes())
            } else {
                save_invitation(&path, &invitation)
            }
        }
        Command::DeliverInvitation {
            pairing_path,
            invitation_path,
        } => {
            let pairing = miopunch_native_probe::invitation_pairing::PairingInfo::decode(
                &std::fs::read(pairing_path)?,
            )?;
            client
                .deliver_invitation(&pairing, &read_invitation(&invitation_path)?)
                .await?;
            Ok(json!({"invitation_saved":true}))
        }
        Command::PairingInformation { .. }
        | Command::PairingStatus
        | Command::FinishPairing { .. } => Err("not a pairing receiver".into()),
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
            save_invitation(&path, &client.issue_precode(expires, max_uses).await?)
        }
        Command::PrecodeStatus { path } => {
            Ok(json!({"uses":client.precode_uses(&read_invitation(&path)?).await?}))
        }
        Command::Request {
            path,
            authorization_only,
        } => {
            let invitation = read_invitation(&path)?;
            let result = if authorization_only {
                client.receive_authorization(&invitation).await
            } else {
                client.request_admission(&invitation).await
            };
            if client.status().await.manual_rejected {
                return Ok(json!({"stage":"Rejected","automatic_retry":false}));
            }
            let stage = result?;
            Ok(json!({"stage":format!("{stage:?}")}))
        }
        Command::NewManualAttempt { path } => {
            client.new_manual_attempt(&read_invitation(&path)?).await?;
            Ok(json!({"stage":"AwaitingApproval"}))
        }
        Command::Reject {
            applicant,
            request_id,
        } => {
            let request = AdmissionRequest {
                applicant: applicant.parse()?,
                request_id: hex::decode(request_id)?
                    .try_into()
                    .map_err(|_| "invalid request identifier")?,
            };
            client.reject(&request).await?;
            Ok(json!({"rejected":true}))
        }
        Command::Requests => Ok(
            json!({"requests":client.requests().await?.iter().map(|request|
            json!({"applicant":request.applicant.to_string(),"request_id":hex::encode(request.request_id)})).collect::<Vec<_>>()}),
        ),
        Command::Approve {
            applicant,
            request_id,
        } => {
            let request = AdmissionRequest {
                applicant: applicant.parse()?,
                request_id: hex::decode(request_id)?
                    .try_into()
                    .map_err(|_| "invalid request identifier")?,
            };
            Ok(json!({"stage":format!("{:?}",client.approve(&request).await?)}))
        }
        Command::Ping { address } => client.ping(address).await,
        Command::FilesEnable { directory } => {
            client.enable_files(directory).await?;
            Ok(json!({"enabled":client.files_enabled()}))
        }
        Command::FilesDisable => {
            client.disable_files().await?;
            Ok(json!({"enabled":false}))
        }
        Command::FilesStatus => Ok(
            json!({"enabled":client.files_enabled(),"tasks":client.file_tasks(),
            "chunk_bytes":miopunch_native_probe::data_stream::CHUNK_BYTES,
            "active_limit":miopunch_native_probe::file_transfer::ACTIVE_TASKS,
            "history_limit":miopunch_native_probe::file_transfer::TASK_HISTORY}),
        ),
        Command::ServiceStatus => client.service_status().await,
        Command::ServiceRule {
            provider,
            service,
            previous,
            rule,
        } => Ok(
            json!({"revision":client.set_service_rule(provider, service, previous, rule).await?,"saved_locally":true}),
        ),
        Command::ServiceSync { address } => client.sync_services(address).await,
        Command::GuestIssue {
            path,
            provider,
            seconds,
        } => {
            let expires = miopunch_native_probe::service_policy::now()
                .checked_add(seconds)
                .ok_or("invalid guest deadline")?;
            let invitation = client.issue_guest(provider, expires).await?;
            save_private(&path, invitation.text()?.as_bytes())
        }
        Command::GuestRedeem => client.redeem_guest().await,
        Command::GuestStatus => Ok(client.guest_status().await?.unwrap_or(Value::Null)),
        Command::GuestEnd => client.end_guest().await,
        Command::GuestRevoke { invitation } => {
            Ok(json!({"revision":client.revoke_guest(invitation).await?,"saved_locally":true}))
        }
        Command::FileApprove { task } => Ok(json!({"approved":client.approve_file(&task)?})),
        Command::FileReject { task } => Ok(json!({"rejected":client.reject_file(&task)?})),
        Command::FileSend { address, path } => {
            Ok(json!({"task":client.send_file(address,path).await?}))
        }
        Command::FileCancel { task } => Ok(json!({"cancel_requested":client.cancel_file(&task)?})),
        Command::FileDisconnect { task } => {
            client.disconnect_file(&task)?;
            Ok(json!({"disconnected":true}))
        }
        Command::Stop => Ok(json!({"stopped":true})),
    }
}
fn emit(value: Value) {
    println!("{value}");
    let _ = std::io::stdout().flush();
}

enum ControlInput {
    Eof,
    Invalid,
    Request(Request),
}
async fn read_control(
    input: &mut BufReader<tokio::io::Stdin>,
) -> Result<ControlInput, ClientError> {
    let mut line = Vec::new();
    let mut oversized = false;
    loop {
        let bytes = input.fill_buf().await?;
        if bytes.is_empty() {
            break;
        }
        let consumed = bytes
            .iter()
            .position(|b| *b == b'\n')
            .map(|n| n + 1)
            .unwrap_or(bytes.len());
        let end = bytes[consumed - 1] == b'\n';
        if line.len() + consumed <= 65536 && !oversized {
            line.extend_from_slice(&bytes[..consumed]);
        } else {
            oversized = true;
        }
        input.consume(consumed);
        if end {
            break;
        }
    }
    if line.is_empty() && !oversized {
        return Ok(ControlInput::Eof);
    }
    if !oversized {
        if let Ok(request) = serde_json::from_slice::<Request>(&line) {
            return Ok(ControlInput::Request(request));
        }
    }
    Ok(ControlInput::Invalid)
}

async fn pairing_host(
    pairing: NativePairing,
    input: &mut BufReader<tokio::io::Stdin>,
) -> Result<Option<NativeClient>, ClientError> {
    emit(
        json!({"event":"ready","status":{"node":pairing.information().receiver().to_string(),"stage":"Pairing"},"address":pairing.information().address()}),
    );
    loop {
        let request = match read_control(input).await? {
            ControlInput::Eof => {
                pairing.close().await?;
                return Ok(None);
            }
            ControlInput::Invalid => {
                emit(json!({"id":null,"ok":false,"error":"invalid_command"}));
                continue;
            }
            ControlInput::Request(request) => request,
        };
        if matches!(request.command, Command::Stop) {
            pairing.close().await?;
            emit(json!({"id":request.id,"ok":true,"result":{"stopped":true}}));
            return Ok(None);
        }
        if let Command::FinishPairing { path } = request.command {
            if pairing.received().await {
                let result = save_invitation(&path, &pairing.invitation().await?);
                if result.is_ok() {
                    let client = pairing.join(Ipv4Addr::UNSPECIFIED).await?;
                    emit(
                        json!({"id":request.id,"ok":true,"result":{"status":status(&client).await,"address":client.address()}}),
                    );
                    return Ok(Some(client));
                }
            }
            emit(json!({"id":request.id,"ok":false,"error":"operation_failed"}));
            continue;
        }
        let result = match request.command {
            Command::PairingInformation { path } => {
                save_private(&path, &pairing.information().encode()?)
            }
            Command::PairingStatus => Ok(json!({"invitation_saved":pairing.received().await})),
            _ => Err("not a pairing operation".into()),
        };
        match result {
            Ok(value) => emit(json!({"id":request.id,"ok":true,"result":value})),
            Err(_) => emit(json!({"id":request.id,"ok":false,"error":"operation_failed"})),
        }
    }
}
fn configuration(
    path: &str,
) -> Result<miopunch_native_probe::resources::ResourceConfig, ClientError> {
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

#[tokio::main]
async fn main() -> Result<(), ClientError> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let mut input = BufReader::new(tokio::io::stdin());
    if args.first().is_some_and(|arg| arg == "pair") {
        let pairing = match args.as_slice() {
            [_, path] => {
                NativePairing::start(path, Ipv4Addr::UNSPECIFIED, Default::default()).await?
            }
            [_, path, resources] => {
                NativePairing::start(path, Ipv4Addr::UNSPECIFIED, configuration(resources)?).await?
            }
            _ => return Err("usage: miopunch-client pair DATA_DIR [RESOURCES_FILE]".into()),
        };
        let Some(client) = pairing_host(pairing, &mut input).await? else {
            return Ok(());
        };
        return run_client(client, input, false).await;
    }
    let client = match args.as_slice() {
        [action, path] if action == "create" => {
            NativeClient::create(path, Ipv4Addr::UNSPECIFIED).await?
        }
        [action, path] if action == "open" => {
            NativeClient::open(path, Ipv4Addr::UNSPECIFIED).await?
        }
        [action, path, invitation] if action == "join" => {
            NativeClient::join(path, &read_invitation(invitation)?, Ipv4Addr::UNSPECIFIED).await?
        }
        [action, path, invitation] if action == "join-guest" => {
            let invitation = miopunch_native_probe::service_policy::GuestInvitation::decode(
                &std::fs::read(invitation)?,
            )?;
            NativeClient::join_guest(path, &invitation, Ipv4Addr::UNSPECIFIED).await?
        }
        [action, path, resources] if action == "create" => {
            NativeClient::create_with_resources(
                path,
                Ipv4Addr::UNSPECIFIED,
                configuration(resources)?,
            )
            .await?
        }
        [action, path, invitation, resources] if action == "join" => {
            NativeClient::join_with_resources(
                path,
                &read_invitation(invitation)?,
                Ipv4Addr::UNSPECIFIED,
                configuration(resources)?,
            )
            .await?
        }
        [action, path, invitation] if action == "resume" => {
            NativeClient::resume(path, &read_invitation(invitation)?, Ipv4Addr::UNSPECIFIED).await?
        }
        _ => {
            return Err(
                "usage: miopunch-client create|open DATA_DIR | join DATA_DIR INVITATION_FILE"
                    .into(),
            );
        }
    };
    run_client(client, input, true).await
}
async fn run_client(
    client: NativeClient,
    mut input: BufReader<tokio::io::Stdin>,
    ready: bool,
) -> Result<(), ClientError> {
    if ready {
        emit(json!({"event":"ready","status":status(&client).await,"address":client.address()}));
    }
    // Limit control input as well as network messages; malformed/oversized
    // commands cannot force an unbounded JSON-lines allocation.
    let result = async {
        loop {
            let request = match read_control(&mut input).await? {
                ControlInput::Eof => break,
                ControlInput::Invalid => {
                    emit(json!({"id":null,"ok":false,"error":"invalid_command"}));
                    continue;
                }
                ControlInput::Request(request) => request,
            };
            if matches!(request.command, Command::Stop) {
                return Ok::<_, ClientError>(Some(request.id));
            }
            match dispatch(&client, request.command).await {
                Ok(value) => emit(json!({"id":request.id,"ok":true,"result":value})),
                Err(_) => emit(json!({"id":request.id,"ok":false,"error":"operation_failed"})),
            }
        }
        Ok(None)
    }
    .await;
    // A stop response is emitted only after the SDK and owned tasks close.
    client.close().await?;
    if let Some(id) = result? {
        emit(json!({"id":id,"ok":true,"result":{"stopped":true}}));
    }
    Ok(())
}
