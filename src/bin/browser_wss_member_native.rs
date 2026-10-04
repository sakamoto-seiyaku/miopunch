//! Reopen genuinely saved members with only the owned MQTT signaling facility.
use std::{error::Error, sync::Arc, time::Duration};

use iroh::endpoint::presets;
use miopunch_native_probe::{
    admission_gate::ProtocolScope,
    member_path_protocol::MemberProtocol,
    mqtt_datagram::MqttFacility,
    mqtt_transport::{MqttTransport, Traffic},
    native_network::NativeNetwork,
    resources::{LanDiscovery, PortMapping, Resource, ResourceUse},
};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};

fn socket_count() -> usize {
    std::fs::read_dir("/proc/self/fd")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| std::fs::read_link(entry.path()).ok())
        .filter(|target| target.to_string_lossy().starts_with("socket:"))
        .count()
}

fn connection_paths(connection: &iroh::endpoint::Connection) -> Vec<serde_json::Value> {
    connection.paths().iter().map(|path| {
        let stats=path.stats();
        json!({"pathId":path.id().to_string(),"selected":path.is_selected(),"isRelay":path.is_relay(),"isIp":path.is_ip(),
            "isRtc":matches!(path.remote_addr(),iroh::TransportAddr::Custom(addr) if addr.id()==miopunch_native_probe::rtc_transport::TRANSPORT_ID),
            "isSignal":matches!(path.remote_addr(),iroh::TransportAddr::Custom(addr) if matches!(addr.id(),miopunch_native_probe::mqtt_transport::TRANSPORT_ID | miopunch_native_probe::irc_transport::TRANSPORT_ID)),
            "sentBytes":stats.udp_tx.bytes,"receivedBytes":stats.udp_rx.bytes,"streamFramesSent":stats.frame_tx.stream,"streamFramesReceived":stats.frame_rx.stream,
            "datagramFramesSent":stats.frame_tx.datagram,"datagramFramesReceived":stats.frame_rx.datagram})
    }).collect()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    if !matches!(args.len(), 4 | 5 | 6)
        || args.get(4).is_some_and(|mode| match mode.as_str() {
            "legacy-no-rtc" => args.len() != 5,
            "legacy-relay" => args.len() != 6,
            "rtc-relay" => args.len() != 6,
            _ => true,
        })
    {
        return Err(
            "usage: browser_wss_member_native DATA_DIR FACILITY_JSON TEST_CA_PEM [legacy-no-rtc | legacy-relay RELAY_URL]"
                .into(),
        );
    }
    let legacy_only = args.get(4).is_some_and(|mode| mode != "rtc-relay");
    let permitted_relay = args.get(5);
    // Preparation must have persisted the original identity and membership.
    let node = NativeNetwork::open(&args[1]).await?;
    let facility: MqttFacility = serde_json::from_slice(&std::fs::read(&args[2])?)?;
    let mut resources = node.resource_configuration()?;
    let mut replacement = vec![Resource {
        url: facility.native_url.clone(),
        uses: [ResourceUse::Signaling].into(),
        limitation: "owned saved-member MQTT/WSS qualification".into(),
    }];
    if let Some(relay) = permitted_relay {
        replacement.push(Resource {
            url: relay.clone(),
            uses: [ResourceUse::DataRelay].into(),
            limitation: "owned explicitly permitted legacy relay compatibility".into(),
        });
    }
    resources.import_trusted_replacement(replacement)?;
    resources.lan_discovery = LanDiscovery::Disabled;
    resources.port_mapping = PortMapping::Disabled;
    resources.udp_direct_disabled = true;
    resources.tcp_direct_disabled = true;
    resources.member_data_relay_disabled = true;
    resources.disabled_uses.insert(ResourceUse::Observation);
    if permitted_relay.is_none() {
        resources.disabled_uses.insert(ResourceUse::DataRelay);
    }
    node.save_resource_configuration(&resources)?;
    let mut alpns = Vec::new();
    for protocol in [MemberProtocol::Business, MemberProtocol::Replica] {
        for alpn in if legacy_only {
            vec![protocol.legacy_alpn().to_vec()]
        } else {
            protocol.preferred_alpns()
        } {
            node.admission_gate()
                .register_protocol(&alpn, ProtocolScope::Member);
            alpns.push(alpn);
        }
    }
    let traffic = Arc::new(Traffic::default());
    let transport = Arc::new(MqttTransport::from_facility(
        &facility,
        &[],
        traffic.clone(),
        Some(std::fs::read(&args[3])?),
    )?);
    let mut builder = iroh::Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .add_custom_transport(transport.clone());
    if permitted_relay.is_some() {
        let cert = rustls::pki_types::CertificateDer::from(std::fs::read(
            std::path::Path::new(&args[3]).with_file_name("ca.der"),
        )?);
        builder = builder.ca_tls_config(iroh_relay::tls::CaTlsConfig::custom_roots(vec![cert]));
    }
    alpns.push(miopunch_native_probe::connection_recovery::CONNECTION_RECOVERY_ALPN.to_vec());
    let rtc_transport = miopunch_native_probe::rtc_transport::RtcTransport::new();
    let assembly = if legacy_only {
        node.bind_configured_endpoint_with(builder).await?
    } else {
        node.bind_configured_endpoint_with_rtc(builder, rtc_transport.clone())
            .await?
    };
    let assembly = Arc::new(assembly);
    let endpoint = assembly.endpoint().clone();
    endpoint.set_alpns(alpns);
    tokio::time::timeout(Duration::from_secs(10), transport.wait_ready()).await??;
    if permitted_relay.is_some() {
        tokio::time::timeout(Duration::from_secs(10), endpoint.online()).await?;
    }
    println!(
        "{}",
        json!({"event":"ready", "id":endpoint.id().to_string(), "route":transport.route()?,
        "knownMembers":node.status().members.iter().map(|(id,_)|id.to_string()).collect::<Vec<_>>(),
        "ipAddresses":endpoint.addr().ip_addrs().count(), "relayAddresses":endpoint.addr().relay_urls().count(),
        "traffic":traffic.snapshot(), "socketCount":socket_count(),"legacyOnly":legacy_only})
    );
    let node = Arc::new(tokio::sync::Mutex::new(node));
    let mut input = BufReader::new(tokio::io::stdin()).lines();
    let mut handlers = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            line=input.next_line()=>match line?.as_deref() {
                Some("stop")|None=>break,
                Some("status")=>println!("{}",json!({"event":"status","traffic":traffic.snapshot(),"socketCount":socket_count(),
                    "activeRtcAssociations":rtc_transport.active_associations(),"activeRtcPumps":rtc_transport.active_pumps(),
                    "publishedRtcRoutes":rtc_transport.published_routes_fixture()})),
                Some("record")=>{
                    use p2panda_auth::{Access,GroupsExtensionArgs,group::{GroupAction,GroupMember}};
                    use p2panda_core::{Hash,Header,SigningKey};
                    use miopunch_native_probe::management_auth::ManagementExtension;
                    let mut node=node.lock().await;
                    let key=SigningKey::from_bytes(&std::fs::read(std::path::Path::new(&args[1]).join("node.key"))?
                        .try_into().map_err(|_|"invalid fixture key length")?);
                    let target=SigningKey::generate().verifying_key();
                    let history=node.management_history().await?;
                    let body=b"original-author browser replica body";
                    let header=Header::<ManagementExtension>::builder().body(body).build(&key,ManagementExtension {
                        version:1,log_id:rand::random(),group:GroupsExtensionArgs {group_id:node.status().anchor.root,
                            action:GroupAction::Add{member:GroupMember::Individual(target),access:Access::read()},
                            dependencies:history.iter().map(|record|Hash::digest(record.header())).collect()}
                    }).encode();
                    let id=node.ingest_management_record(&header,Some(body)).await?;
                    let stored=node.management_record(id).await?.ok_or("source record not applied")?;
                    let verified=Header::<ManagementExtension>::decode(stored.header())?;
                    println!("{}",json!({"event":"record","record":id.to_string(),"header":hex::encode(stored.header()),
                        "body":stored.body().map(hex::encode),"bodyBytes":body.len(),"author":verified.verifying_key.to_string(),
                        "logId":hex::encode(verified.extensions.log_id),"sequence":verified.seq_num,"backlink":verified.backlink.map(|id|id.to_string()),
                        "action":"Add","target":target.to_string(),"dependencies":verified.extensions.group.dependencies.iter().map(ToString::to_string).collect::<Vec<_>>(),
                        "applied":node.status().members.contains(&(target,Access::read()))}));
                },
                Some("arm-control")=>{
                    rtc_transport.arm_control_fixture(miopunch_native_probe::rtc_invalidation_fixture::ControlCheckpointStage::BeforeRelease)?;
                    println!("{}",json!({"event":"control-armed"}));
                },
                Some("arm-control-hold-cleanup")=>{
                    rtc_transport.arm_control_fixture_with_cleanup_hold(miopunch_native_probe::rtc_invalidation_fixture::ControlCheckpointStage::BeforeRelease)?;
                    println!("{}",json!({"event":"control-armed"}));
                },
                Some("control-status")=>{
                    let node=node.lock().await;
                    let (generation,durable)=node.credential_generation_fixture().await?;
                    println!("{}",json!({"event":"control-status","checkpoint":rtc_transport.control_fixture_snapshot(),
                        "source":node.admission_gate().qualification_fixture(),"secretGeneration":generation,"durableCurrent":durable,
                        "members":node.status().members.iter().map(|(id,_)|id.to_string()).collect::<Vec<_>>(),
                        "resourceRevision":rtc_transport.resource_revision(),"resourceAllowed":rtc_transport.resource_allowed(),
                        "activeRtcAssociations":rtc_transport.active_associations(),"activeRtcPumps":rtc_transport.active_pumps(),
                        "socketCount":socket_count()}));
                },
                Some("credential-invalidate")=>{
                    let mut node=node.lock().await;
                    let before=node.admission_gate().qualification_fixture();
                    let generation=node.invalidate_current_credential_fixture().await?;
                    let (_,durable)=node.credential_generation_fixture().await?;
                    println!("{}",json!({"event":"qualification-applied","kind":"credential-generation-invalidation",
                        "before":before,"after":node.admission_gate().qualification_fixture(),"generation":generation,"durableCurrent":durable,
                        "members":node.status().members.iter().map(|(id,_)|id.to_string()).collect::<Vec<_>>(),
                        "resourceRevision":rtc_transport.resource_revision(),"resourceAllowed":rtc_transport.resource_allowed(),
                        "checkpoint":rtc_transport.control_fixture_snapshot()}));
                },
                Some("rtc-resource-disable")=>{
                    let node=node.lock().await;
                    let source=node.admission_gate().qualification_fixture();
                    let before=rtc_transport.resource_revision();
                    rtc_transport.set_resource_allowed(false);
                    println!("{}",json!({"event":"qualification-applied","kind":"scoped-rtc-carrier-permission",
                        "before":source,"after":node.admission_gate().qualification_fixture(),
                        "previousResourceRevision":before,"resourceRevision":rtc_transport.resource_revision(),"resourceAllowed":rtc_transport.resource_allowed(),
                        "checkpoint":rtc_transport.control_fixture_snapshot()}));
                },
                Some("control-clear")=>{
                    rtc_transport.clear_control_fixture();
                    println!("{}",json!({"event":"control-cleared"}));
                },
                Some(command) if command.starts_with("control-release ")=>{
                    let result=rtc_transport.release_control_fixture(&command[16..]);
                    println!("{}",json!({"event":"control-release","released":result.is_ok(),"failure":result.err().map(|e|e.to_string())}));
                },
                Some(command) if command.starts_with("cleanup-release ")=>{
                    rtc_transport.release_cleanup_fixture(&command[16..])?;
                    println!("{}",json!({"event":"cleanup-release"}));
                },
                Some(command) if command.starts_with("remove-member ")=>{
                    let mut node=node.lock().await;
                    let before=node.admission_gate().qualification_fixture();
                    let target=command[14..].parse()?;
                    let mut session=node.begin_management_session();
                    let result=node.manage(&mut session,miopunch_native_probe::daily_management::ManagementAction::RemoveMember(target)).await?;
                    session.end();
                    let record=node.management_history().await?.into_iter()
                        .find(|record|p2panda_core::Hash::digest(record.header())==result.record)
                        .ok_or("applied removal record missing from actual history")?;
                    println!("{}",json!({"event":"qualification-applied","kind":"verified-applied-member-removal",
                        "record":result.record.to_string(),"target":target.to_string(),"effectiveAccessAbsent":result.effective_access.is_none(),
                        "header":hex::encode(record.header()),
                        "before":before,"after":node.admission_gate().qualification_fixture(),
                        "members":node.status().members.iter().map(|(id,_)|id.to_string()).collect::<Vec<_>>(),
                        "resourceRevision":rtc_transport.resource_revision(),"resourceAllowed":rtc_transport.resource_allowed(),
                        "checkpoint":rtc_transport.control_fixture_snapshot()}));
                },
                _=>return Err("unknown fixture command".into()),
            },
            incoming=endpoint.accept()=>{
                let Some(incoming)=incoming else {break};
                let incoming=incoming.with_caller_deadline(n0_future::time::Instant::now()+Duration::from_secs(12));
                let node=node.clone();
                let assembly=assembly.clone();
                let rtc_transport=rtc_transport.clone();
                let serving_endpoint=endpoint.clone();
                handlers.spawn(async move {
                    let connection=match incoming.await {
                        Ok(connection)=>connection,
                        Err(error)=>{
                            println!("{}",json!({"event":"handle-withheld","error":error.to_string()}));
                            return Ok::<_,Box<dyn Error+Send+Sync>>(());
                        }
                    };
                    if connection.alpn() == miopunch_native_probe::connection_recovery::CONNECTION_RECOVERY_ALPN {
                        println!("{}",json!({"event":"repair-control","stableId":connection.stable_id(),
                            "alpn":String::from_utf8_lossy(connection.alpn()),"restricted":!node.lock().await.admission_gate().allows(&connection,ProtocolScope::Member),
                            "paths":connection.paths().iter().map(|path|json!({"isRelay":path.is_relay(),"isIp":path.is_ip(),
                                "isMqtt":matches!(path.remote_addr(),iroh::TransportAddr::Custom(addr) if addr.id()==miopunch_native_probe::mqtt_transport::TRANSPORT_ID),
                                "isRtc":matches!(path.remote_addr(),iroh::TransportAddr::Custom(addr) if addr.id()==miopunch_native_probe::rtc_transport::TRANSPORT_ID)})).collect::<Vec<_>>()}));
                        let result=node.lock().await.serve_connection_path_recovery(&assembly,&connection).await;
                        println!("{}",json!({"event":"repair-control-ended","stableId":connection.stable_id(),"succeeded":result.is_ok(),"failure":result.err().map(|e|e.to_string())}));
                        connection.close(0u32.into(),b"repair serving ended");
                        return Ok::<_,Box<dyn Error+Send+Sync>>(());
                    }
                    println!("{}",json!({"event":"member-released","stableId":connection.stable_id(),
                        "remoteId":connection.remote_id().to_string(),"alpn":String::from_utf8_lossy(connection.alpn()),
                        "paths":connection.paths().iter().map(|path|json!({"isRelay":path.is_relay(),"isIp":path.is_ip(),"selected":path.is_selected(),"ackReceived":path.stats().frame_rx.acks,"streamSent":path.stats().frame_tx.stream})).collect::<Vec<_>>()}));
                    if MemberProtocol::from_wire(connection.alpn()).is_some_and(|(protocol,_)|protocol==MemberProtocol::Replica) {
                        let original=connection.stable_id();
                        let mut binding=[0;32];
                        connection.export_keying_material(&mut binding,b"miopunch/replica-session-observation/1",&[]).map_err(|_|"original replica binding unavailable")?;
                        let deadline=connection.caller_deadline().ok_or("owned replica caller deadline absent")?;
                        let before=connection_paths(&connection);
                        let result=node.lock().await.serve_replica_with_deadline(&connection,deadline).await;
                        let mut current=[0;32];
                        let matched=connection.export_keying_material(&mut current,b"miopunch/replica-session-observation/1",&[]).is_ok()&&current==binding;
                        println!("{}",json!({"event":"replica-finished","stableId":connection.stable_id(),"alpn":String::from_utf8_lossy(connection.alpn()),
                            "stableUnchanged":connection.stable_id()==original,"bindingUnchanged":matched,"durableReceiptAccepted":result.is_ok(),
                            "failure":result.as_ref().err().map(ToString::to_string),"before":before,"after":connection_paths(&connection),
                            "remainingCallerMs":deadline.saturating_duration_since(n0_future::time::Instant::now()).as_millis()}));
                        result?;
                    } else {
                        let mut streams=tokio::task::JoinSet::new();
                        loop {
                            let stream=tokio::select! {
                                _=connection.closed()=>break,
                                stream=connection.accept_bi()=>stream,
                                Some(_)=streams.join_next(),if !streams.is_empty()=>continue,
                            };
                            let (send,recv)=stream?;
                            let gate=node.lock().await.admission_gate();
                            streams.spawn(miopunch_native_probe::rtc_member::serve_stream(
                                serving_endpoint.clone(),connection.clone(),gate,rtc_transport.clone(),send,recv));
                        }
                        streams.abort_all();
                        while streams.join_next().await.is_some() {}
                    }
                    connection.closed().await;
                    Ok(())
                });
            },
            Some(result)=handlers.join_next(),if !handlers.is_empty()=>{
                if let Ok(Err(error))=result {eprintln!("owned saved-member handler: {error}");}
            }
        }
    }
    endpoint.close().await;
    handlers.abort_all();
    while handlers.join_next().await.is_some() {}
    drop(assembly);
    // Closing the endpoint stops its actor. Drop this last fixture clone too,
    // so the actor's custom endpoint can withdraw and report transport closure.
    drop(endpoint);
    tokio::time::timeout(Duration::from_secs(1), transport.wait_closed()).await??;
    println!("{}", json!({"event":"closed","traffic":traffic.snapshot()}));
    Arc::try_unwrap(node)
        .map_err(|_| "fixture node still owned")?
        .into_inner()
        .close()
        .await;
    Ok(())
}
