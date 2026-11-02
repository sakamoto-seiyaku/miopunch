//! Minimal non-UI host for the public NativeClient operations.
use miopunch_native_probe::{
    native_client::{ClientError, ClientInvitation, NativeClient},
    native_pairing::NativePairing,
};
use serde_json::{Value, json};
use std::{io::Write, net::Ipv4Addr};
use tokio::io::{AsyncBufReadExt, BufReader};

use miopunch_native_probe::client_commands::{
    Command, Request, dispatch, read_invitation, save_invitation, save_private, status,
};

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
    let mut args: Vec<_> = std::env::args().skip(1).collect();
    #[cfg(target_os = "linux")]
    if let [action, directory, socket] = args.as_slice() {
        if action == "serve" {
            return miopunch_native_probe::headless_host::serve(
                directory.as_ref(),
                socket.as_ref(),
            )
            .await;
        }
    }
    // Every application entry that reads/mutates a node participates in the
    // same lease, including pairing and offline replacement operations.
    let owns_directory = match args.as_slice() {
        [action, _] => matches!(action.as_str(), "create" | "open" | "pair"),
        [action, _, _] => matches!(
            action.as_str(),
            "create"
                | "pair"
                | "join"
                | "join-guest"
                | "resume"
                | "restore"
                | "finalize-replacement"
        ),
        [action, _, _, _] => matches!(
            action.as_str(),
            "join" | "join-guest" | "recover" | "abort-replacement"
        ),
        _ => false,
    };
    let _directory_lease = if owns_directory {
        let lease = miopunch_native_probe::client_host::HostLease::acquire(&args[1])?;
        args[1] = lease
            .directory()
            .to_str()
            .ok_or("node directory is not UTF-8")?
            .to_owned();
        Some(lease)
    } else {
        None
    };
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
            NativeClient::join_guest_with_resources(
                path,
                &invitation,
                Ipv4Addr::UNSPECIFIED,
                configuration(resources)?,
            )
            .await?
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
