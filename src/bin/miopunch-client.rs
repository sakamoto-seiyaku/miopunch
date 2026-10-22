//! Minimal non-UI host for the public NativeClient operations.
use miopunch_native_probe::{
    manual_admission::AdmissionRequest,
    native_client::{ClientError, ClientInvitation, NativeClient},
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
    Status,
    Address,
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
    Request {
        path: String,
        #[serde(default)]
        authorization_only: bool,
    },
    Requests,
    Approve {
        applicant: String,
        request_id: String,
    },
    Ping {
        address: iroh::EndpointAddr,
    },
    Stop,
}
fn read_invitation(path: &str) -> Result<ClientInvitation, ClientError> {
    ClientInvitation::decode(&std::fs::read(path)?)
}
fn save_invitation(path: &str, invitation: &ClientInvitation) -> Result<Value, ClientError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(&invitation.encode()?)?;
    file.sync_all()?;
    Ok(json!({"saved":true}))
}
async fn status(client: &NativeClient) -> Value {
    let status = client.status().await;
    json!({"node":status.node.node_id.to_string(), "network":status.node.anchor.root.to_string(),
        "stage":format!("{:?}",status.admission), "records":status.node.applied_count,
        "members":status.node.members.iter().map(|(key,role)| json!({"node":key.to_string(),
            "role":if *role == p2panda_auth::Access::manage() {"admin"} else {"member"}})).collect::<Vec<_>>()})
}
async fn dispatch(client: &NativeClient, command: Command) -> Result<Value, ClientError> {
    match command {
        Command::Status => Ok(status(client).await),
        Command::Address => Ok(json!({"address":client.address()})),
        Command::Invite { path } => save_invitation(&path, &client.invitation().await?),
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
            let stage = if authorization_only {
                client.receive_authorization(&invitation).await?
            } else {
                client.request_admission(&invitation).await?
            };
            Ok(json!({"stage":format!("{stage:?}")}))
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
        Command::Stop => Ok(json!({"stopped":true})),
    }
}
fn emit(value: Value) {
    println!("{value}");
    let _ = std::io::stdout().flush();
}

#[tokio::main]
async fn main() -> Result<(), ClientError> {
    let args: Vec<_> = std::env::args().skip(1).collect();
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
        _ => {
            return Err(
                "usage: miopunch-client create|open DATA_DIR | join DATA_DIR INVITATION_FILE"
                    .into(),
            );
        }
    };
    emit(json!({"event":"ready","status":status(&client).await,"address":client.address()}));
    // Limit control input as well as network messages; malformed/oversized
    // commands cannot force an unbounded JSON-lines allocation.
    let mut input = BufReader::new(tokio::io::stdin());
    let result = async {
        loop {
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
                break;
            }
            let request = if oversized {
                None
            } else {
                serde_json::from_slice::<Request>(&line).ok()
            };
            let Some(request) = request else {
                emit(json!({"id":null,"ok":false,"error":"invalid_command"}));
                continue;
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
