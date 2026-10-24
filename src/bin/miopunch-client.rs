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
fn socket_wait() -> u64 { 100 }
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Command {
    WebEnable { targets: Vec<String>, #[serde(default)] trust_roots_pem: Vec<String> },
    WebDisable,
    WebStatus,
    WebSocketOpen { address: iroh::EndpointAddr, url: String, #[serde(default)] headers: Vec<(String, String)> },
    WebSocketSend { socket: String, message: miopunch_native_probe::web::WebSocketMessage },
    WebSocketRead { socket: String, #[serde(default = "socket_wait")] timeout_ms: u64 },
    WebSocketClose { socket: String },
    WebOpen { address: iroh::EndpointAddr, url: String, #[serde(default)] method: Option<String>, #[serde(default)] headers: Vec<(String, String)>, #[serde(default)] body: String },
    WebBegin { address: iroh::EndpointAddr, url: String, #[serde(default)] method: Option<String>, #[serde(default)] headers: Vec<(String, String)>, #[serde(default)] body: String },
    WebResult { job: String },
    WebCancel { job: String },
    ForwardEnable { protocol: miopunch_native_probe::forwarding::Protocol, targets: Vec<std::net::SocketAddr> },
    ForwardDisable { protocol: miopunch_native_probe::forwarding::Protocol },
    ForwardStart { address: iroh::EndpointAddr, protocol: miopunch_native_probe::forwarding::Protocol, target: std::net::SocketAddr, bind: std::net::SocketAddr },
    ForwardStop { task: String },
    ForwardStatus,
    ForwardCancel { task: String },
    ClipboardEnable,
    ClipboardDisable,
    ClipboardStart {
        address: iroh::EndpointAddr,
    },
    ClipboardStop,
    ClipboardStatus,
    ReplacementPrepare {
        path: String,
    },
    ReplacementStatus,
    ReplacementComplete {
        address: iroh::EndpointAddr,
        receipt_path: String,
    },
    CustodyStatus,
    ManagementRecover {
        address: iroh::EndpointAddr,
    },
    ManagementRecoveryStatus,
    CustodyPropose {
        recipient: p2panda_core::VerifyingKey,
    },
    CustodyAccept {
        offer: p2panda_core::Hash,
    },
    CustodyTransfer {
        offer: p2panda_core::Hash,
        address: iroh::EndpointAddr,
    },
    CustodyCheck {
        #[serde(default)]
        addresses: Vec<iroh::EndpointAddr>,
    },
    CustodyStop {
        check: p2panda_core::Hash,
        #[serde(default)]
        force_without_confirmed_copy: bool,
    },
    GenesisPropose {
        action: miopunch_native_probe::device_operation::DeviceAction,
        target: p2panda_core::VerifyingKey,
    },
    DeviceOperations,
    GenesisConfirm {
        operation: p2panda_core::Hash,
        card_path: Option<String>,
        password_path: Option<String>,
    },
    GenesisRequest {
        operation: p2panda_core::Hash,
        address: iroh::EndpointAddr,
    },
    GenesisResult {
        operation: p2panda_core::Hash,
        address: iroh::EndpointAddr,
    },
    DeviceExecute {
        operation: p2panda_core::Hash,
    },
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
        #[serde(default)]
        service: Option<miopunch_native_probe::service_policy::Service>,
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
        "stage":if !status.recovery_saved {"AwaitingRecoverySave".into()} else if !status.replacement_ready {"RestoringAuthorization".into()} else if status.membership_ended {"Removed".into()} else if status.manual_rejected {"Rejected".into()} else {format!("{:?}",status.admission)},
        "automatic_retry":!status.manual_rejected,
        "recovery_saved":status.recovery_saved, "replacement_ready":status.replacement_ready, "records":status.node.applied_count,
        "members":status.node.members.iter().map(|(key,role)| json!({"node":key.to_string(),
            "role":if *role == p2panda_auth::Access::manage() {"admin"} else {"member"}})).collect::<Vec<_>>()});
    if let Some(guest) = guest {
        if status.replacement_ready && status.recovery_saved {
            result["stage"] = guest["stage"].clone();
        }
        result["guest"] = guest;
        result["members"] = json!([]);
    }
    result
}
async fn dispatch(client: &NativeClient, command: Command) -> Result<Value, ClientError> {
    match command {
        Command::WebEnable { targets, trust_roots_pem } => client.enable_web(targets, trust_roots_pem).await,
        Command::WebDisable => client.disable_web().await,
        Command::WebStatus => Ok(client.web_status()),
        Command::WebResult { job } => client.web_request_result(&job),
        Command::WebCancel { job } => client.cancel_web_request(&job).await,
        Command::WebBegin { address, url, method, headers, body } => {
            use base64::Engine;
            let request = miopunch_native_probe::web::WebRequest { url, method: method.unwrap_or_else(|| "GET".into()), headers,
                body: base64::engine::general_purpose::STANDARD.decode(body)? };
            client.begin_web_request(address, request).await
        },
        Command::WebSocketOpen { address, url, headers } => client.open_web_socket(address, url, headers).await,
        Command::WebSocketSend { socket, message } => { client.send_web_socket(&socket, message).await?; Ok(json!({"queued":true})) },
        Command::WebSocketRead { socket, timeout_ms } => client.read_web_socket(&socket, std::time::Duration::from_millis(timeout_ms)).await,
        Command::WebSocketClose { socket } => client.close_web_socket(&socket).await,
        Command::WebOpen { address, url, method, headers, body } => {
            use base64::Engine;
            let request = miopunch_native_probe::web::WebRequest { url, method: method.unwrap_or_else(|| "GET".into()), headers,
                body: base64::engine::general_purpose::STANDARD.decode(body)? };
            Ok(serde_json::to_value(client.request_web(address, request).await?)?)
        },
        Command::ForwardEnable { protocol, targets } => client.enable_forwarding(protocol, targets).await,
        Command::ForwardDisable { protocol } => client.disable_forwarding(protocol).await,
        Command::ForwardStart { address, protocol, target, bind } => client.start_forwarding(address, protocol, target, bind).await,
        Command::ForwardStop { task } => client.stop_forwarding(&task).await,
        Command::ForwardStatus => Ok(client.forwarding_status()),
        Command::ForwardCancel { task } => { client.cancel_forwarding_flow(&task).await?; Ok(client.forwarding_status()) },
        Command::ClipboardEnable => client.enable_clipboard().await,
        Command::ClipboardDisable => {
            client.disable_clipboard().await?;
            client.clipboard_status().await
        }
        Command::ClipboardStart { address } => client.start_clipboard(address).await,
        Command::ClipboardStop => {
            client.stop_clipboard().await;
            client.clipboard_status().await
        }
        Command::ClipboardStatus => client.clipboard_status().await,
        Command::ReplacementPrepare { .. } => {
            Err("replacement export consumes the running host".into())
        }
        Command::ReplacementStatus => client.node_replacement_status().await,
        Command::ReplacementComplete {
            address,
            receipt_path,
        } => {
            let result = client.complete_node_replacement(address).await?;
            let bytes = serde_json::to_vec(&result["receipt"])?;
            if std::path::Path::new(&receipt_path).exists() {
                if std::fs::read(&receipt_path)? != bytes {
                    return Err("another receipt already exists".into());
                }
            } else {
                save_private(&receipt_path, &bytes)?;
            }
            Ok(result)
        }
        Command::ManagementRecover { address } => {
            client.restore_management_authority(address).await
        }
        Command::ManagementRecoveryStatus => client.management_recovery_status().await,
        Command::CustodyStatus => client.credential_custody().await,
        Command::CustodyPropose { recipient } => client.propose_credential_custody(recipient).await,
        Command::CustodyAccept { offer } => client.accept_credential_custody(offer).await,
        Command::CustodyTransfer { offer, address } => {
            client.transfer_credential_custody(offer, address).await
        }
        Command::CustodyCheck { addresses } => {
            client.check_other_credential_custodians(addresses).await
        }
        Command::CustodyStop {
            check,
            force_without_confirmed_copy,
        } => {
            client
                .stop_credential_custody(check, force_without_confirmed_copy)
                .await
        }
        Command::GenesisPropose { action, target } => {
            client.propose_device_operation(action, target).await
        }
        Command::DeviceOperations => client.device_operations().await,
        Command::GenesisConfirm {
            operation,
            card_path,
            password_path,
        } => {
            if let Some(path) = card_path {
                let password = password_path.map(std::fs::read_to_string).transpose()?;
                let card = miopunch_native_probe::recovery_card::RecoveryCard::import(
                    &std::fs::read_to_string(path)?,
                    password.as_deref(),
                )?;
                client
                    .confirm_device_operation_offline(operation, &card)
                    .await
            } else {
                if password_path.is_some() {
                    return Err("card path required".into());
                }
                client.confirm_device_operation(operation).await
            }
        }
        Command::GenesisRequest { operation, address } => {
            client.request_device_confirmation(operation, address).await
        }
        Command::GenesisResult { operation, address } => {
            client.collect_device_result(operation, address).await
        }
        Command::DeviceExecute { operation } => client.execute_device_operation(operation).await,
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
            service,
            path,
            provider,
            seconds,
        } => {
            let expires = miopunch_native_probe::service_policy::now()
                .checked_add(seconds)
                .ok_or("invalid guest deadline")?;
            let invitation = client
                .issue_service_guest(
                    provider,
                    service.unwrap_or(miopunch_native_probe::service_policy::Service::Files),
                    expires,
                )
                .await?;
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
    if let [action, source, receipt] = args.as_slice() {
        if action == "finalize-replacement" {
            let receipt = serde_json::from_slice(&std::fs::read(receipt)?)?;
            emit(NativeClient::finalize_node_replacement(source, &receipt)?);
            return Ok(());
        }
    }
    if let [action, source, package, acknowledgement] = args.as_slice() {
        if action == "abort-replacement" && acknowledgement == "target-not-activated" {
            NativeClient::abort_node_replacement(source, package.parse()?, true)?;
            emit(json!({"source_resumption_authorized":true}));
            return Ok(());
        }
    }
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
        [action, path, backup] if action == "restore" => {
            NativeClient::restore_node_replacement(
                &std::fs::read_to_string(backup)?,
                path,
                Ipv4Addr::UNSPECIFIED,
            )
            .await?
        }
        [action, path, card, contact] if action == "recover" => {
            let card = miopunch_native_probe::recovery_card::RecoveryCard::import(
                &std::fs::read_to_string(card)?,
                None,
            )?;
            let contact: iroh::EndpointAddr = serde_json::from_slice(&std::fs::read(contact)?)?;
            NativeClient::prepare_management_recovery(
                path,
                &card,
                contact.id,
                Ipv4Addr::UNSPECIFIED,
                Default::default(),
            )
            .await?
        }
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
        [action, path, invitation, resources] if action == "join-guest" => {
            let invitation = miopunch_native_probe::service_policy::GuestInvitation::decode(
                &std::fs::read(invitation)?,
            )?;
            NativeClient::join_guest_with_resources(path, &invitation, Ipv4Addr::UNSPECIFIED, configuration(resources)?).await?
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
    enum Ending {
        Stop(u64),
        Replace(u64, String),
    }
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
                return Ok::<_, ClientError>(Some(Ending::Stop(request.id)));
            }
            if let Command::ReplacementPrepare { path } = request.command {
                return Ok(Some(Ending::Replace(request.id, path)));
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
    match result {
        Ok(Some(Ending::Replace(id, path))) => match client.prepare_node_replacement(path).await {
            Ok(value) => emit(json!({"id":id,"ok":true,"result":value})),
            Err(_) => {
                emit(json!({"id":id,"ok":false,"error":"replacement_export_failed_host_stopped"}))
            }
        },
        other => {
            client.close().await?;
            if let Some(Ending::Stop(id)) = other? {
                emit(json!({"id":id,"ok":true,"result":{"stopped":true}}));
            }
        }
    }
    Ok(())
}
