//! Thin manual-admission fixture. The web server never handles node material.
use std::{error::Error, sync::Arc};

use iroh::{RelayMode, endpoint::presets};
use iroh_base::TransportAddr;
use miopunch_native_probe::{
    admission_gate::{ADMISSION_REQUEST_ALPN, ProtocolScope},
    native_network::NativeNetwork,
    offline_replica::REPLICA_ALPN,
    resources::{Resource, ResourceUse},
};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};

const BUSINESS: &[u8] = b"miopunch/browser-member/1";

// Public transport observations only; never emit admission or node secrets.
fn paths(connection: &iroh::endpoint::Connection) -> Vec<serde_json::Value> {
    connection
        .paths()
        .iter()
        .map(|path| {
            let stats = path.stats();
            let relay = match path.remote_addr() {
                TransportAddr::Relay(url) => Some(url.to_string()),
                _ => None,
            };
            json!({"relay":relay, "isRelay":path.is_relay(), "selected":path.is_selected(),
                "sentBytes":stats.udp_tx.bytes, "receivedBytes":stats.udp_rx.bytes})
        })
        .collect()
}

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
        limitation: "explicitly selected browser qualification relay; availability not guaranteed"
            .into(),
    }])?;
    node.save_resource_configuration(&resources)?;
    node.admission_gate()
        .register_protocol(BUSINESS, ProtocolScope::Member);
    node.admission_gate()
        .register_protocol(REPLICA_ALPN, ProtocolScope::Member);
    let mut builder = iroh::Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .relay_mode(RelayMode::custom([relay.parse()?]))
        .alpns(vec![
            ADMISSION_REQUEST_ALPN.to_vec(),
            BUSINESS.to_vec(),
            REPLICA_ALPN.to_vec(),
        ]);
    if let Some(path) = args.get(3) {
        let cert = rustls::pki_types::CertificateDer::from(std::fs::read(path)?);
        builder = builder.ca_tls_config(iroh_relay::tls::CaTlsConfig::custom_roots(vec![cert]));
    }
    let endpoint = node.bind_endpoint_with(builder).await?;
    tokio::time::timeout(std::time::Duration::from_secs(20), endpoint.online())
        .await
        .map_err(|_| "selected browser qualification relay unavailable")?;
    println!(
        "{}",
        json!({"event":"ready", "id":endpoint.id().to_string(), "relay":endpoint.addr().relay_urls().map(ToString::to_string).collect::<Vec<_>>(),
            "invitation":hex::encode(node.manual_invitation()?.encode())})
    );
    let node = Arc::new(tokio::sync::Mutex::new(node));
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut handlers = tokio::task::JoinSet::new();
    let precode_mode = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut issued = None;
    #[cfg(feature = "replica-test-hooks")]
    let replica_fixture = Arc::new(tokio::sync::Mutex::new(
        None::<Vec<miopunch_native_probe::management_store::ManagementRecord>>,
    ));
    #[cfg(feature = "replica-test-hooks")]
    let mut fault_records = Vec::new();
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
                    #[cfg(feature="replica-test-hooks")]
                    "prepare-faults" => {
                        use p2panda_auth::{Access,GroupsExtensionArgs,group::{GroupAction,GroupMember}};
                        use p2panda_core::{Hash,Header,SigningKey};
                        use miopunch_native_probe::{management_auth::ManagementExtension,management_store::ManagementRecord};
                        let key=SigningKey::from_bytes(&std::fs::read(std::path::Path::new(path).join("node.key"))?.try_into().map_err(|_|"key length")?);
                        let history=node.management_history().await?;
                        let mut dependency=Hash::digest(history.last().ok_or("empty history")?.header());
                        fault_records.clear();
                        for log_id in [[1;16],[0;16]] {
                            let body=b"original author body retained by browser";
                            let header=Header::<ManagementExtension>::builder().body(body).build(&key,ManagementExtension {
                                version:1,log_id,group:GroupsExtensionArgs {group_id:node.status().anchor.root,
                                action:GroupAction::Add{member:GroupMember::Individual(SigningKey::generate().verifying_key()),access:Access::read()},dependencies:vec![dependency]}
                            }).encode();
                            dependency=node.ingest_management_record(&header,Some(body)).await?;
                            fault_records.push(ManagementRecord::new(header,Some(body)));
                        }
                        println!("{}",json!({"event":"prepared","records":fault_records.iter().map(|record|Hash::digest(record.header()).to_string()).collect::<Vec<_>>() }));
                    }
                    #[cfg(feature="replica-test-hooks")]
                    "fixture-missing-body" | "fixture-child-only" | "fixture-reversed-duplicates" | "fixture-normal" => {
                        use miopunch_native_probe::management_store::ManagementRecord;
                        if fault_records.len()!=2 {return Err("prepare faults first".into());}
                        *replica_fixture.lock().await=match line.as_str() {
                            "fixture-missing-body"=>Some(vec![ManagementRecord::new(fault_records[0].header().to_vec(),None)]),
                            "fixture-child-only"=>Some(vec![fault_records[1].clone()]),
                            "fixture-reversed-duplicates"=>Some(vec![fault_records[1].clone(),fault_records[0].clone(),fault_records[1].clone()]),
                            _=>None,
                        };
                        println!("{}",json!({"event":"fixture","mode":line}));
                    }
                    "issue" => {
                        let expires=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs()+3600;
                        let code=node.issue_precode(expires,Some(1)).await?;
                        issued=Some(code);
                        precode_mode.store(true,std::sync::atomic::Ordering::SeqCst);
                        println!("{}",json!({"event":"issued","code":hex::encode(code.encode())}));
                    }
                    "usage" => {
                        let usage=node.precode_usage(&issued.ok_or("no code")?).await?;
                        println!("{}",json!({"event":"usage","uses":usage,"count":node.status().applied_count}));
                    }
                    "record" => {
                        use p2panda_auth::{Access,GroupsExtensionArgs,group::{GroupAction,GroupMember}};
                        use p2panda_core::{Hash,Header,SigningKey};
                        use miopunch_native_probe::management_auth::ManagementExtension;
                        let key=SigningKey::from_bytes(&std::fs::read(std::path::Path::new(path).join("node.key"))?.try_into().map_err(|_|"key length")?);
                        let history=node.management_history().await?;
                        let body=b"original-author browser replica body";
                        let header=Header::<ManagementExtension>::builder().body(body).build(&key,ManagementExtension {
                            version:1,log_id:rand::random(),group:GroupsExtensionArgs {group_id:node.status().anchor.root,
                            action:GroupAction::Add{member:GroupMember::Individual(SigningKey::generate().verifying_key()),access:Access::read()},dependencies:history.iter().map(|record|Hash::digest(record.header())).collect()}
                        }).encode();
                        let record=node.ingest_management_record(&header,Some(body)).await?;
                        println!("{}", json!({"event":"record", "record":record.to_string()}));
                    }
                    "status" => {
                        let status = node.status();
                        println!("{}", json!({"event":"status", "count":status.applied_count,
                            "members":status.members.iter().map(|(id,_)|id.to_string()).collect::<Vec<_>>(),
                            "requests":node.manual_requests().await?.len()}));
                    }
                    command if command.starts_with("remove-record ") => {
                        use p2panda_auth::{GroupsExtensionArgs,group::{GroupAction,GroupMember}};
                        use p2panda_core::{Header,SigningKey,Hash};
                        use miopunch_native_probe::management_auth::ManagementExtension;
                        let target=command[14..].parse()?;
                        let key=SigningKey::from_bytes(&std::fs::read(std::path::Path::new(path).join("node.key"))?.try_into().map_err(|_|"key length")?);
                        let history=node.management_history().await?;
                        let header=Header::<ManagementExtension>::builder().build(&key,ManagementExtension{version:1,log_id:rand::random(),group:GroupsExtensionArgs{
                            group_id:node.status().anchor.root,action:GroupAction::Remove{member:GroupMember::Individual(target)},dependencies:history.iter().map(|record|Hash::digest(record.header())).collect()
                        }}).encode();
                        println!("{}",json!({"event":"removal-record","header":hex::encode(header)}));
                    }
                    "stop" => break,
                    _ => return Err("unknown fixture command".into()),
                }
            },
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { break };
                let node = node.clone();
                #[cfg(feature = "rtc-probe")]
                let endpoint_id = endpoint.id().to_string();
                let precode_mode=precode_mode.clone();
                #[cfg(feature="replica-test-hooks")]
                let replica_fixture=replica_fixture.clone();
                handlers.spawn(async move {
                    let connection = incoming.await?;
                    let result: Result<(), Box<dyn Error + Send + Sync>> = async {
                        if connection.alpn() == ADMISSION_REQUEST_ALPN {
                            let stage = if precode_mode.load(std::sync::atomic::Ordering::SeqCst) {
                                node.lock().await.serve_precode_admission(&connection).await?
                            } else { node.lock().await.serve_manual_admission(&connection).await? };
                            println!("{}", json!({"event":"admission", "stage":format!("{stage:?}"), "paths":paths(&connection)}));
                            connection.closed().await;
                        } else if connection.alpn() == REPLICA_ALPN {
                            #[cfg(feature="replica-test-hooks")]
                            {
                                let fixture=replica_fixture.lock().await.clone();
                                if let Some(records)=fixture {
                                    node.lock().await.serve_replica_fixture(&connection,&records).await?;
                                } else {node.lock().await.serve_replica(&connection).await?;}
                            }
                            #[cfg(not(feature="replica-test-hooks"))]
                            node.lock().await.serve_replica(&connection).await?;
                            println!("{}", json!({"event":"replica", "paths":paths(&connection)}));
                            connection.closed().await;
                        } else {
                            let (mut send, mut recv) = connection.accept_bi().await?;
                            let request = recv.read_to_end(65536).await?;
                            #[cfg(feature = "rtc-probe")]
                            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&request) {
                                if value["protocol"] == "miopunch/rtc-probe/1" {
                                    let gate = node.lock().await.admission_gate().clone();
                                    miopunch_native_probe::rtc_probe::serve(&connection, &gate, endpoint_id, value, &mut send).await?;
                                    // The existing M owns the control receipt stream until its caller closes it.
                                    connection.closed().await;
                                    return Ok(());
                                }
                            }
                            send.write_all(&request).await?;
                            send.finish()?;
                            println!("{}", json!({"event":"business", "bytes":request.len(), "paths":paths(&connection)}));
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
