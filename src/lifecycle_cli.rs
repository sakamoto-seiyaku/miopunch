//! Thin platform lifecycle inputs for the shared maintained native node.
//! Android app glue may forward callbacks here; no second recovery loop lives here.
use std::{error::Error, io::Write, net::Ipv4Addr};

use iroh::EndpointAddr;
use miopunch_native_probe::{
    native_network::NativeNetwork, network_maintenance::MaintainedNetwork,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

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
    Contact { address: EndpointAddr },
    NetworkChanged,
    Suspend,
    Resume,
    Stop,
}

fn emit(value: Value) {
    println!("{value}");
    let _ = std::io::stdout().flush();
}

struct Lifecycle<'a> {
    path: &'a str,
    bind_ip: Ipv4Addr,
    active: Option<MaintainedNetwork>,
    node: String,
    generation: String,
}

impl Lifecycle<'_> {
    async fn resume(&mut self) -> Result<Value> {
        if self.active.is_some() {
            return Err("already running".into());
        }
        let node = NativeNetwork::open(self.path).await?;
        let network = node.maintain_network(self.bind_ip).await?;
        self.node = network.node_status().node_id.to_string();
        // Connection stable IDs are local to this runtime, never durable identity.
        self.generation = format!("{:032x}", rand::random::<u128>());
        let ready =
            json!({"node": self.node, "address": network.addr(), "generation": self.generation});
        self.active = Some(network);
        Ok(ready)
    }

    fn network(&self) -> Result<&MaintainedNetwork> {
        self.active.as_ref().ok_or_else(|| "suspended".into())
    }

    fn status(&self) -> Value {
        let Some(network) = &self.active else {
            return json!({"state": "Suspended", "node": self.node, "generation": self.generation,
                "neighbors": [], "old_connections_ended": true});
        };
        let status = network.maintenance_status();
        json!({"state": format!("{:?}", status.state), "node": self.node,
            "generation": self.generation, "records": status.node.applied_count,
            "neighbors": status.neighbors.iter().map(|peer| json!({"node": peer.id.to_string(),
                "connection": peer.stable_id, "pings_sent": peer.pings_sent})).collect::<Vec<_>>(),
            "contact_attempts": status.contact_attempts, "retry_after_ms": status.retry_after.as_millis(),
            "cause": status.cause,
            "last_sync": status.last_sync.map(|sync| json!({"source": sync.source.to_string(), "complete": sync.complete()}))})
    }

    async fn suspend(&mut self) {
        if let Some(network) = self.active.take() {
            network.close().await;
        }
    }

    async fn dispatch(&mut self, command: Command) -> Result<Value> {
        match command {
            Command::Status => Ok(self.status()),
            Command::Contact { address } => {
                self.network()?.offer_contact(address).await?;
                Ok(json!({"hint_accepted": true}))
            }
            Command::NetworkChanged => {
                let network = self.network()?;
                // Android lacks netlink callbacks: forward both transport and
                // node-scheduler notifications without replacing either loop.
                network.endpoint().network_change().await;
                network.network_changed().await?;
                Ok(json!({"notified": true}))
            }
            Command::Suspend => {
                self.suspend().await;
                Ok(self.status())
            }
            Command::Resume => self.resume().await,
            Command::Stop => {
                self.suspend().await;
                Ok(json!({"stopped": true}))
            }
        }
    }
}

pub async fn run(path: &str, bind_ip: Ipv4Addr) -> Result<()> {
    let mut lifecycle = Lifecycle {
        path,
        bind_ip,
        active: None,
        node: String::new(),
        generation: String::new(),
    };
    let mut ready = lifecycle.resume().await?;
    ready["event"] = json!("ready");
    emit(ready);
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let result = async {
        while let Some(line) = lines.next_line().await? {
            let request = match serde_json::from_str::<Request>(&line) {
                Ok(request) => request,
                Err(_) => {
                    emit(json!({"id": null, "ok": false, "error": "invalid_command"}));
                    continue;
                }
            };
            let stop = matches!(request.command, Command::Stop);
            match lifecycle.dispatch(request.command).await {
                Ok(result) => emit(json!({"id": request.id, "ok": true, "result": result})),
                Err(_) => emit(
                    json!({"id": request.id, "ok": false, "error": "lifecycle_operation_failed"}),
                ),
            }
            if stop {
                break;
            }
        }
        Ok::<_, Box<dyn Error>>(())
    }
    .await;
    lifecycle.suspend().await;
    result
}
