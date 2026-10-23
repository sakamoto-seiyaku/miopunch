//! Bounded technical host for actual public maintenance-resource regressions.
//! Product onboarding continues through NativeClient; this exercises the
//! existing lower-level SDK's live resource retirement and discovery owner.
use miopunch_native_probe::{native_network::NativeNetwork, resources::ResourceConfig};
use serde_json::{Value, json};
use std::{io::Write, net::Ipv4Addr};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

fn emit(value: Value) {
    println!("{value}");
    std::io::stdout().flush().expect("probe stdout");
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    let node = match arguments.as_slice() {
        [action, path, configuration] if action == "create" => {
            let node = NativeNetwork::create(path).await?;
            node.save_resource_configuration(&ResourceConfig::load(std::path::Path::new(
                configuration,
            ))?)?;
            node
        }
        [action, path] if action == "open" => NativeNetwork::open(path).await?,
        _ => return Err("expected create STATE CONFIG or open STATE".into()),
    };
    let owner = node.maintain_network(Ipv4Addr::UNSPECIFIED).await?;
    let mut priority = Some(owner.business_activity());
    let status = owner.node_status();
    emit(
        json!({"event":"ready", "status":{"node":status.node_id.to_string(),
                "network":status.anchor.root.to_string(), "genesis":status.anchor.genesis.to_string()},
                "address":owner.addr()}),
    );
    let mut input = BufReader::new(tokio::io::stdin().take(64 * 1024)).lines();
    while let Some(line) = input.next_line().await? {
        let command: Value = serde_json::from_str(&line)?;
        let id = command.get("id").ok_or("missing command id")?;
        let result = match command["op"].as_str().ok_or("missing operation")? {
            "disable_contact_signaling" => {
                owner.disable_contact_signaling().await?;
                json!({"disabled":true})
            }
            "explore" => {
                drop(priority.take());
                owner.network_changed().await?;
                json!({"exploration_enabled":true})
            }
            "status" => {
                let status = owner.maintenance_status();
                json!({"discovery_rounds":status.discovery_rounds,
                    "contact_listener":status.contact_listener.map(|listener| format!("{:?}", listener.phase))})
            }
            "stop" => {
                emit(json!({"id":id,"ok":true,"result":{"stopping":true}}));
                break;
            }
            _ => return Err("unknown probe operation".into()),
        };
        emit(json!({"id":id,"ok":true,"result":result}));
    }
    drop(priority.take());
    owner.close().await;
    Ok(())
}
