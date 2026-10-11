//! Explicit IRC facility fixture for the existing restricted admission protocol.
use iroh::{RelayMode, endpoint::presets};
use miopunch_native_probe::{
    admission_gate::{ADMISSION_REQUEST_ALPN, ProtocolScope},
    irc_datagram::{IrcFacility, Profile},
    irc_transport::IrcTransport,
    mqtt_datagram::MqttFacility,
    mqtt_transport::{MqttTransport, Traffic as MqttTraffic},
    native_network::NativeNetwork,
};
use serde::Deserialize;
use serde_json::json;
use std::{error::Error, sync::Arc, time::Duration};
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Deserialize)]
struct Configuration {
    facility: IrcFacility,
    relay: String,
    #[serde(default)]
    reopen: bool,
    profile: Profile,
    caller_budget_ms: u64,
    #[serde(default)]
    maintained: bool,
    #[serde(default)]
    quic_idle_timeout_ms: Option<u64>,
    #[serde(default)]
    quic_initial_rtt_ms: Option<u64>,
    #[serde(default)]
    mqtt: Option<MqttFacility>,
    #[serde(default)]
    disabled: Vec<String>,
}

// [DEBUG-m87] Private causal-observation hook. No gate, stream or wire changes.
#[derive(Clone, Debug, Default)]
struct StageObserver {
    connection: Arc<std::sync::Mutex<Option<iroh::endpoint::WeakConnectionHandle>>>,
}
impl iroh::endpoint::EndpointHooks for StageObserver {
    async fn after_handshake(&self, connection: &iroh::endpoint::Connection) -> iroh::endpoint::AfterHandshakeOutcome {
        *self.connection.lock().unwrap()=Some(connection.weak_handle());
        println!("{}",json!({"event":"[DEBUG-m87]native-tls","caller_remaining_ms":connection.caller_deadline().map(|deadline|deadline.saturating_duration_since(n0_future::time::Instant::now()).as_millis()),"stats":format!("{:?}",connection.stats())}));
        iroh::endpoint::AfterHandshakeOutcome::Accept
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err("usage: browser_irc_restricted_native CONFIG CA_PEM_OR_- DATA_DIR".into());
    }
    let config: Configuration = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let ca = if args[2] == "-" {
        None
    } else {
        Some(std::fs::read(&args[2])?)
    };
    let node = if config.reopen {NativeNetwork::open(&args[3]).await?} else {NativeNetwork::create(&args[3]).await?};
    config.facility.check_allowed(&config.disabled)?;
    let mut resources = node.resource_configuration()?;
    let mut signaling = vec![miopunch_native_probe::resources::Resource {
        url: config.facility.native_url.clone(),
        uses: [miopunch_native_probe::resources::ResourceUse::Signaling].into(),
        limitation: "explicit restricted IRC facility qualification".into(),
    }];
    if let Some(facility) = &config.mqtt {
        facility.check_allowed(&config.disabled)?;
        signaling.push(miopunch_native_probe::resources::Resource {
            url: facility.native_url.clone(),
            uses: [miopunch_native_probe::resources::ResourceUse::Signaling].into(),
            limitation: "owned MQTT mixed-carrier retry regression".into(),
        });
    }
    signaling.push(miopunch_native_probe::resources::Resource { url:config.relay.clone(), uses:[miopunch_native_probe::resources::ResourceUse::DataRelay].into(), limitation:"explicit synthetic approved-member business relay".into() });
    if config.reopen {signaling.retain(|resource|resource.uses.contains(&miopunch_native_probe::resources::ResourceUse::DataRelay));}
    resources.import_trusted_replacement(signaling)?;
    resources.udp_direct_disabled=true;resources.tcp_direct_disabled=true;
    resources.lan_discovery = miopunch_native_probe::resources::LanDiscovery::Disabled;
    node.save_resource_configuration(&resources)?;
    let _ = rustls::crypto::ring::default_provider().install_default();
    let create_transport = if config.maintained { IrcTransport::from_maintained_facility } else { IrcTransport::from_facility };
    let transport = Arc::new(create_transport(
        &config.facility,
        &config.disabled,
        config.profile.clone(),
        ca.clone(),
    )?);
    let traffic = transport.traffic().clone();
    let startup_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mqtt_traffic = Arc::new(MqttTraffic::default());
    let mqtt = config
        .mqtt
        .as_ref()
        .map(|facility| {
            MqttTransport::from_facility(
                facility,
                &config.disabled,
                mqtt_traffic.clone(),
                ca.clone(),
            )
            .map(Arc::new)
        })
        .transpose()?;
    let observer=StageObserver::default();
    let mut quic=iroh::endpoint::QuicTransportConfig::builder();
    if let Some(ms)=config.quic_idle_timeout_ms { quic=quic.max_idle_timeout(Some(Duration::from_millis(ms).try_into()?)); }
    if let Some(ms)=config.quic_initial_rtt_ms { quic=quic.initial_rtt(Duration::from_millis(ms)); }
    let mut builder = iroh::Endpoint::builder(presets::Minimal)
        .transport_config(quic.build())
        .hooks(observer.clone())
        .clear_ip_transports()
        .relay_mode(RelayMode::Custom(resources.relay_map(miopunch_native_probe::resources::ResourceUse::DataRelay)?))
        .alpns(vec![ADMISSION_REQUEST_ALPN.to_vec(),miopunch_native_probe::connection_coordinator::MEMBER_ALPN.to_vec(),miopunch_native_probe::connection_coordinator::REPLICA_ALPN.to_vec()]);
    if !config.reopen {builder=builder.add_custom_transport(transport.clone());}
    if let Some(mqtt) = &mqtt {
        builder = builder.add_custom_transport(mqtt.clone());
    }
    let (endpoint,assembly) = if config.reopen {
        let assembly=Arc::new(node.bind_configured_endpoint_with(builder).await?);
        (assembly.endpoint().clone(),Some(assembly))
    } else {(node.bind_endpoint_with(builder).await?,None)};
    if !config.reopen {tokio::time::timeout_at(startup_deadline, transport.wait_ready()).await??;}
    if let Some(mqtt) = &mqtt {
        tokio::time::timeout_at(startup_deadline, mqtt.wait_ready()).await??;
    }
    let route = endpoint
        .custom_addrs()
        .iter()
        .find(|address| address.id() == miopunch_native_probe::irc_datagram::TRANSPORT_ID)
        .map(|address| hex::encode(address.data()))
        .unwrap_or_default();
    println!(
        "{}",
        json!({"event":"ready","id":endpoint.id().to_string(),"route":route,"mqtt_traffic":mqtt.as_ref().map(|_|mqtt_traffic.snapshot()),"mqtt_route":mqtt.as_ref().map(|transport|transport.route()).transpose()?,"invitation":hex::encode(node.manual_invitation()?.encode())})
    );
    let gate = node.admission_gate().clone();
    let node = Arc::new(tokio::sync::Mutex::new(node));
    let mut tasks = tokio::task::JoinSet::new();
    let mut input = BufReader::new(tokio::io::stdin()).lines();
    let observation_started=tokio::time::Instant::now();
    let mut observation_tick=tokio::time::interval(Duration::from_secs(10));
    loop {
        tokio::select! {
            _=observation_tick.tick()=> {
                if let Some(connection)=observer.connection.lock().unwrap().as_ref().and_then(|weak|weak.upgrade()) {
                    println!("{}",json!({"event":"[DEBUG-m87]native-stage","elapsed_ms":observation_started.elapsed().as_millis(),"caller_remaining_ms":connection.caller_deadline().map(|deadline|deadline.saturating_duration_since(n0_future::time::Instant::now()).as_millis()),"closed":connection.close_reason().is_some(),"stats":format!("{:?}",connection.stats()),"traffic":traffic.snapshot()}));
                }
            }
            incoming=endpoint.accept()=> {
                let Some(incoming)=incoming else {break};
                let node = node.clone();
                let serving_coordinator=assembly.as_ref().map(|assembly|assembly.coordinator());
                let gate = gate.clone();
                let endpoint = endpoint.clone();
                let traffic = traffic.clone();
                let budget = Duration::from_millis(config.caller_budget_ms);
                tasks.spawn(async move {
                    let started=tokio::time::Instant::now();
                    let deadline=started+budget;
                    let accepted=tokio::time::timeout_at(deadline,async {
                    let mut accepting = incoming.accept().map_err(|e|std::io::Error::other(e.to_string()))?.with_caller_deadline(deadline);
                    let alpn = accepting.alpn().await.map_err(|e|std::io::Error::other(e.to_string()))?;
                    if let Some(coordinator)=serving_coordinator {
                        let purpose=miopunch_native_probe::connection_coordinator::SessionPurpose::from_alpn(&alpn).ok_or_else(||std::io::Error::other("unknown session purpose"))?;
                        coordinator.accept_accepting(accepting,purpose,miopunch_native_probe::connection_coordinator::AttemptContext::new(deadline,tokio_util::sync::CancellationToken::new())).await
                        .map_err(|e|std::io::Error::other(e.to_string()))
                    } else { accepting.await.map_err(|e|std::io::Error::other(e.to_string())) }
                    }).await;
                    match accepted {
                        Ok(Ok(connection))=> {
                            let handshake_ms=started.elapsed().as_millis();
                            println!("{}",json!({"event":"established","remote_id":connection.remote_id().to_string(),"alpn":String::from_utf8_lossy(connection.alpn()),"stable_id":connection.stable_id(),"handshake_ms":handshake_ms,"stats":format!("{:?}",connection.stats()),"traffic":traffic.snapshot()}));
                            if connection.alpn()==miopunch_native_probe::connection_coordinator::REPLICA_ALPN {
                                let result=node.lock().await.serve_replica_with_deadline(&connection,deadline).await;
                                println!("{}",json!({"event":"replica-finished","stable_id":connection.stable_id(),"succeeded":result.is_ok(),"failure":result.as_ref().err().map(ToString::to_string)}));
                                connection.close(0u32.into(),b"owned replica completed");
                            } else if connection.alpn()==miopunch_native_probe::connection_coordinator::MEMBER_ALPN {
                                let before=connection.stats();
                                let authorized=gate.allows(&connection,ProtocolScope::Member);
                                let selected_relay=connection.paths().iter().any(|p|p.is_selected()&&p.is_relay());
                                println!("{}",json!({"event":"member-released","stable_id":connection.stable_id(),"authorized":authorized,"selected_relay":selected_relay,"alpn":String::from_utf8_lossy(connection.alpn())}));
                                let result=tokio::time::timeout_at(deadline,async {
                                    let (mut send,mut recv)=connection.accept_bi().await?;
                                    let message=recv.read_to_end(65536).await.map_err(|e|std::io::Error::other(e.to_string()))?;
                                    if message!=b"PING"||!gate.allows(&connection,ProtocolScope::Member)||!selected_relay{return Err(std::io::Error::other("current authorized member relay PING required"));}
                                    miopunch_native_probe::member_message::respond(&message,&mut send).await?;
                                    println!("{}",json!({"event":"business","stable_id":connection.stable_id(),"reply":"PONG","member_message":true,"selected_relay":true,"stream_progress":connection.stats().frame_rx.stream>before.frame_rx.stream}));
                                    Ok::<_,std::io::Error>(())
                                }).await;
                                println!("{}",json!({"event":"business-ended","succeeded":matches!(result,Ok(Ok(())))}));
                                let _=tokio::time::timeout_at(deadline,connection.closed()).await;
                                connection.close(0u32.into(),b"owned member closed");
                            } else {
                            let mut node = node.lock().await;
                            let stage=tokio::time::timeout_at(deadline,node.serve_manual_admission(&connection)).await;
                            println!("{}",json!({"event":"admission","local_id":endpoint.id().to_string(),"remote_id":connection.remote_id().to_string(),"alpn":String::from_utf8_lossy(connection.alpn()),"stable_id":connection.stable_id(),"handshake_ms":handshake_ms,"exchange_ms":started.elapsed().as_millis(),"restricted":gate.allows(&connection,ProtocolScope::AdmissionRequest),"business_handle":gate.allows(&connection,ProtocolScope::Member),"stage":format!("{stage:?}"),"stats":format!("{:?}",connection.stats()),"traffic":traffic.snapshot()}));
                            drop(node);
                            // The unapproved status is buffered by QUIC. Let
                            // its caller consume it and close A, under this
                            // same server deadline, before retiring the fixture.
                            let peer_closed = tokio::time::timeout_at(deadline, connection.closed()).await;
                            println!("{}",json!({"event":"peer-closed","stable_id":connection.stable_id(),"elapsed_ms":started.elapsed().as_millis(),"result":format!("{peer_closed:?}")}));
                            connection.close(0u32.into(),b"restricted fixture exchange ended");
                            }
                        }
                        result=>println!("{}",json!({"event":"handshake-rejected","error":format!("{result:?}"),"elapsed_ms":started.elapsed().as_millis(),"traffic":traffic.snapshot()})),
                    }
                    Ok::<_,Box<dyn Error+Send+Sync>>(())
                });
            }
            completed=tasks.join_next(),if !tasks.is_empty()=> { completed.transpose()?; }

            line=input.next_line()=>{
                match line?.as_deref() {
                    Some("status")=>{
                        tasks.abort_all();
                        while tasks.join_next().await.is_some() {}
                        let mut node = node.lock().await;
                        let mut requests=Vec::new();
                        for request in node.manual_requests().await? {
                            let approved=node.approved_admission(&request).await?.is_some();
                            requests.push(json!({"applicant":request.applicant.to_string(),"request_id":hex::encode(request.request_id),"approved":approved}));
                        }
                        println!("{}",json!({"event":"status","requests":requests,"traffic":traffic.snapshot(),"mqtt_traffic":mqtt.as_ref().map(|_|mqtt_traffic.snapshot()),"members":node.status().members.iter().map(|(id,_)|id.to_string()).collect::<Vec<_>>() }));
                    }
                    Some("approve")=>{
                        let mut node=node.lock().await;
                        let requests=node.manual_requests().await?;
                        if requests.len()!=1{return Err("exactly one owned request required".into());}
                        let before=node.status().applied_count;
                        let approved=node.approve_manual_admission(&requests[0]).await?;
                        println!("{}",json!({"event":"approved","new_records":node.status().applied_count-before,"authorization":approved.authorization.to_string()}));
                    }
                    Some("stop")|None=>break,
                    _=>return Err("unknown fixture command".into()),
                }
            }
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    let cleanup_started = tokio::time::Instant::now();
    // Preserve the live carrier while the SDK flushes QUIC close notifications.
    // Explicit carrier retirement follows the same SDK shutdown ordering.
    let graceful_close = tokio::time::timeout(Duration::from_secs(120), endpoint.close())
        .await
        .is_ok();
    transport.close();
    tokio::time::timeout(Duration::from_secs(2), async {
        while traffic.snapshot()["driver_active"] != 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| "owned IRC driver did not finish cleanup")?;
    drop(endpoint);
    drop(assembly);
    if mqtt.is_some() {
        tokio::time::timeout_at(cleanup_started + Duration::from_secs(3), async {
            while mqtt_traffic.snapshot()["protocol"]["disconnect_sent"] != true {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .map_err(|_| "owned mixed MQTT driver did not disconnect")?;
    }
    println!(
        "{}",
        json!({"event":"closed","traffic":traffic.snapshot(),"mqtt_traffic":mqtt.as_ref().map(|_|mqtt_traffic.snapshot()),"graceful_close":graceful_close,"cleanup_elapsed_ms":cleanup_started.elapsed().as_millis()})
    );
    Arc::try_unwrap(node)
        .map_err(|_| "native fixture node still shared")?
        .into_inner()
        .close()
        .await;
    Ok(())
}
