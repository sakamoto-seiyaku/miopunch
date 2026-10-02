//! Thin manual-admission fixture. The web server never handles node material.
use std::{error::Error, sync::Arc};

use iroh::{RelayMode, endpoint::presets};
use miopunch_native_probe::{
    admission_gate::{ADMISSION_REQUEST_ALPN, ProtocolScope},
    native_network::NativeNetwork,
    resources::{Resource, ResourceUse},
};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};

const BUSINESS: &[u8] = b"miopunch/browser-member/1";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    if !(args.len() == 3 || args.len() == 4) {
        return Err("usage: browser-network-native DATA_DIR RELAY_URL [TEST_CA_DER]".into());
    }
    let path = &args[1];
    let relay = &args[2];
    let node = if std::path::Path::new(path).exists() {
        NativeNetwork::open(path).await?
    } else {
        NativeNetwork::create(path).await?
    };
    let mut resources = node.resource_configuration()?;
    resources.import_trusted_replacement(vec![Resource {
        url: relay.clone(),
        uses: [ResourceUse::DataRelay].into(),
        limitation: "controlled self-hosted browser test relay".into(),
    }])?;
    node.save_resource_configuration(&resources)?;
    node.admission_gate()
        .register_protocol(BUSINESS, ProtocolScope::Member);
    let mut builder = iroh::Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .relay_mode(RelayMode::custom([relay.parse()?]))
        .alpns(vec![ADMISSION_REQUEST_ALPN.to_vec(), BUSINESS.to_vec()]);
    if let Some(path) = args.get(3) {
        let cert = rustls::pki_types::CertificateDer::from(std::fs::read(path)?);
        builder = builder.ca_tls_config(iroh_relay::tls::CaTlsConfig::custom_roots(vec![cert]));
    }
    let endpoint = node.bind_endpoint_with(builder).await?;
    endpoint.online().await;
    println!(
        "{}",
        json!({"event":"ready", "id":endpoint.id().to_string(), "invitation":hex::encode(node.manual_invitation()?.encode())})
    );
    let node = Arc::new(tokio::sync::Mutex::new(node));
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut handlers = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            line = lines.next_line() => {
                let Some(line) = line? else { break };
                let mut node = node.lock().await;
                match line.as_str() {
                    "approve" => {
                        for request in node.manual_requests().await? {
                            node.approve_manual_admission(&request).await?;
                        }
                        println!("{}", json!({"event":"approved", "count":node.status().applied_count}));
                    }
                    "status" => {
                        let status = node.status();
                        println!("{}", json!({"event":"status", "count":status.applied_count,
                            "members":status.members.iter().map(|(id,_)|id.to_string()).collect::<Vec<_>>(),
                            "requests":node.manual_requests().await?.len()}));
                    }
                    "stop" => break,
                    _ => return Err("unknown fixture command".into()),
                }
            },
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { break };
                let node = node.clone();
                handlers.spawn(async move {
                    let connection = incoming.await?;
                    let result: Result<(), Box<dyn Error + Send + Sync>> = async {
                        if connection.alpn() == ADMISSION_REQUEST_ALPN {
                            let stage = node.lock().await.serve_manual_admission(&connection).await?;
                            println!("{}", json!({"event":"admission", "stage":format!("{stage:?}")}));
                            connection.closed().await;
                        } else {
                            let (mut send, mut recv) = connection.accept_bi().await?;
                            let request = recv.read_to_end(65536).await?;
                            send.write_all(&request).await?;
                            send.finish()?;
                            println!("{}", json!({"event":"business", "bytes":request.len()}));
                            connection.closed().await;
                        }
                        Ok(())
                    }.await;
                    if result.is_err() { connection.close(0u32.into(), b"fixture request failed"); }
                    result
                });
            },
            Some(result) = handlers.join_next(), if !handlers.is_empty() => {
                if let Err(error) = result? { eprintln!("browser fixture request: {error}"); }
            }
        }
    }
    endpoint.close().await;
    handlers.abort_all();
    while handlers.join_next().await.is_some() {}
    Arc::try_unwrap(node)
        .map_err(|_| "fixture node still owned")?
        .into_inner()
        .close()
        .await;
    Ok(())
}
