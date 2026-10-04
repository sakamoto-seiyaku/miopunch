//! Reopen genuinely saved members with only the owned MQTT signaling facility.
use std::{error::Error, sync::Arc, time::Duration};

use iroh::endpoint::presets;
use miopunch_native_probe::{
    admission_gate::{ADMISSION_REQUEST_ALPN, ProtocolScope},
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

struct FixtureOptions<'a> {
    admission: bool,
    legacy_only: bool,
    permitted_relay: Option<&'a str>,
    ice_profile: Option<&'a str>,
}

fn fixture_options(args: &[String]) -> Result<FixtureOptions<'_>, &'static str> {
    const USAGE: &str = "usage: browser_wss_member_native DATA_DIR FACILITY_JSON TEST_CA_PEM [admission | legacy-no-rtc | legacy-relay RELAY_URL | rtc-relay RELAY_URL] [--ice-profile FILE]";
    let tail = args.get(4..).ok_or(USAGE)?;
    let (mode, ice_profile) = if tail.len() >= 2 && tail[tail.len() - 2] == "--ice-profile" {
        (&tail[..tail.len() - 2], Some(tail[tail.len() - 1].as_str()))
    } else {
        (tail, None)
    };
    let (admission, legacy_only, permitted_relay) = match mode {
        [] => (false, false, None),
        [mode] if mode == "admission" => (true, false, None),
        [mode] if mode == "legacy-no-rtc" => (false, true, None),
        [mode, relay] if mode == "legacy-relay" => (false, true, Some(relay.as_str())),
        [mode, relay] if mode == "rtc-relay" => (false, false, Some(relay.as_str())),
        _ => return Err(USAGE),
    };
    Ok(FixtureOptions {
        admission,
        legacy_only,
        permitted_relay,
        ice_profile,
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    let options = fixture_options(&args)?;
    let admission_mode = options.admission;
    let legacy_only = options.legacy_only;
    let permitted_relay = options.permitted_relay;
    let ice_profile: miopunch_native_probe::rtc_ice_profile::RtcIceProfile =
        if let Some(path) = options.ice_profile {
            serde_json::from_slice(&std::fs::read(path)?)?
        } else {
            Default::default()
        };
    ice_profile.validate()?;
    // Preparation must have persisted the original identity and membership.
    let node = if admission_mode && !std::path::Path::new(&args[1]).exists() {
        NativeNetwork::create(&args[1]).await?
    } else {
        NativeNetwork::open(&args[1]).await?
    };
    let facility: MqttFacility = serde_json::from_slice(&std::fs::read(&args[2])?)?;
    let mut resources = node.resource_configuration()?;
    ice_profile.check_allowed(
        &resources
            .disabled
            .iter()
            .filter(|(_, usage)| *usage == ResourceUse::Observation)
            .map(|(url, _)| url.clone())
            .collect::<Vec<_>>(),
    )?;
    let mut replacement = vec![Resource {
        url: facility.native_url.clone(),
        uses: [ResourceUse::Signaling].into(),
        limitation: "owned saved-member MQTT/WSS qualification".into(),
    }];
    if let Some(relay) = permitted_relay {
        replacement.push(Resource {
            url: relay.to_owned(),
            uses: [ResourceUse::DataRelay].into(),
            limitation: "owned explicitly permitted legacy relay compatibility".into(),
        });
    }
    let observation_resources = ice_profile.observation_resources()?;
    for observer in &observation_resources {
        replacement.push(Resource {
            url: observer.clone(),
            uses: [ResourceUse::Observation].into(),
            limitation: "explicit owned RTC observation fixture; no data relay".into(),
        });
    }
    resources.import_trusted_replacement(replacement)?;
    resources.lan_discovery = LanDiscovery::Disabled;
    resources.port_mapping = PortMapping::Disabled;
    resources.udp_direct_disabled = true;
    resources.tcp_direct_disabled = true;
    resources.member_data_relay_disabled = true;
    resources.disabled_uses.insert(ResourceUse::Observation);
    if !ice_profile.stun_servers.is_empty() {
        resources.disabled_uses.remove(&ResourceUse::Observation);
    }
    if observation_resources
        .iter()
        .any(|url| !resources.allows(url, ResourceUse::Observation))
    {
        return Err("explicit RTC observer is not permitted by saved resources".into());
    }
    if permitted_relay.is_none() {
        resources.disabled_uses.insert(ResourceUse::DataRelay);
    } else {
        // This fixture's explicit allowed-relay case can reopen material saved
        // by its no-relay preparation; URL-specific disables remain enforced.
        resources.disabled_uses.remove(&ResourceUse::DataRelay);
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
    if admission_mode {
        alpns.push(ADMISSION_REQUEST_ALPN.to_vec());
    }
    let rtc_transport =
        miopunch_native_probe::rtc_transport::RtcTransport::with_ice_profile(ice_profile)?;
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
        "traffic":traffic.snapshot(), "socketCount":socket_count(),"legacyOnly":legacy_only,
        "invitation":if admission_mode {Some(hex::encode(node.manual_invitation()?.encode()))} else {None}})
    );
    let node = Arc::new(tokio::sync::Mutex::new(node));
    let mut input = BufReader::new(tokio::io::stdin()).lines();
    let mut handlers = tokio::task::JoinSet::new();
    let code_path = std::path::Path::new(&args[1]).join("fixture.precode");
    let mut issued = if admission_mode && code_path.exists() {
        Some(miopunch_native_probe::precode::Precode::decode(
            &std::fs::read(&code_path)?
                .try_into()
                .map_err(|_| "fixture precode length")?,
        )?)
    } else {
        None
    };
    let precode_mode = Arc::new(std::sync::atomic::AtomicBool::new(issued.is_some()));
    loop {
        tokio::select! {
            line=input.next_line()=>match line?.as_deref() {
                Some("stop")|None=>break,
                Some("status")=>println!("{}",json!({"event":"status","traffic":traffic.snapshot(),"socketCount":socket_count(),
                    "activeRtcAssociations":rtc_transport.active_associations(),"activeRtcPumps":rtc_transport.active_pumps(),
                    "publishedRtcRoutes":rtc_transport.published_routes_fixture()})),
                Some("admission-status") if admission_mode=>{
                    let mut node=node.lock().await;
                    let mut requests=Vec::new();
                    for request in node.manual_requests().await? {
                        let approved=node.approved_admission(&request).await?;
                        requests.push(json!({"applicant":request.applicant.to_string(),"request":hex::encode(request.request_id),
                            "authorization":approved.as_ref().map(|a|a.authorization.to_string()),"receiptStage":approved.map(|a|format!("{:?}",a.stage))}));
                    }
                    let records=node.management_history().await?;
                    println!("{}",json!({"event":"admission-status","id":node.status().node_id.to_string(),"requests":requests,
                        "records":records.iter().map(|r|p2panda_core::Hash::digest(r.header()).to_string()).collect::<Vec<_>>(),"count":node.status().applied_count}));
                },
                Some(command) if admission_mode && command.starts_with("approve-request ")=>{
                    let parts=command[16..].split_whitespace().collect::<Vec<_>>();
                    if parts.len()!=2 {return Err("exact applicant and request required".into());}
                    let request=miopunch_native_probe::manual_admission::AdmissionRequest {applicant:parts[0].parse()?,request_id:hex::decode(parts[1])?.try_into().map_err(|_|"request length")?};
                    let mut node=node.lock().await;
                    let before=node.status().applied_count;
                    let approved=node.approve_manual_admission(&request).await?;
                    println!("{}",json!({"event":"approved","applicant":request.applicant.to_string(),"request":hex::encode(request.request_id),
                        "authorization":approved.authorization.to_string(),"before":before,"after":node.status().applied_count,"receiptStage":format!("{:?}",approved.stage)}));
                },
                Some("issue") if admission_mode=>{
                    use std::os::unix::fs::OpenOptionsExt;
                    use std::io::Write;
                    let mut node=node.lock().await;
                    let expires=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs()+3600;
                    let code=node.issue_precode(expires,Some(1)).await?;
                    let mut file=std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&code_path)?;
                    file.write_all(&code.encode())?;file.sync_all()?;
                    issued=Some(code);precode_mode.store(true,std::sync::atomic::Ordering::Release);
                    println!("{}",json!({"event":"issued","code":hex::encode(code.encode())}));
                },
                Some("usage") if admission_mode=>{
                    let mut node=node.lock().await;
                    let code=issued.as_ref().ok_or("no owned code")?;
                    let status=node.precode_usage_status(code).await?;
                    let redemptions=node.precode_redemptions(code).await?;
                    println!("{}",json!({"event":"usage","uses":node.precode_usage(code).await?,"trusted":status.count_trusted,
                        "redemptions":redemptions.iter().map(|a|json!({"applicant":a.request.applicant.to_string(),"request":hex::encode(a.request.request_id),"authorization":a.authorization.to_string(),"receiptStage":format!("{:?}",a.stage)})).collect::<Vec<_>>(),"count":node.status().applied_count}));
                },
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
                let precode_mode=precode_mode.clone();
                handlers.spawn(async move {
                    let connection=match incoming.await {
                        Ok(connection)=>connection,
                        Err(error)=>{
                            println!("{}",json!({"event":"handle-withheld","error":error.to_string()}));
                            return Ok::<_,Box<dyn Error+Send+Sync>>(());
                        }
                    };
                    if admission_mode && connection.alpn()==ADMISSION_REQUEST_ALPN {
                        let deadline=connection.caller_deadline().ok_or("owned admission deadline absent")?;
                        let gate=node.lock().await.admission_gate();
                        println!("{}",json!({"event":"admission-accepted","stableId":connection.stable_id(),"remoteId":connection.remote_id().to_string(),
                            "alpn":String::from_utf8_lossy(connection.alpn()),"restricted":gate.allows(&connection,ProtocolScope::AdmissionRequest),
                            "businessHandle":gate.allows(&connection,ProtocolScope::Member),"paths":connection_paths(&connection)}));
                        let result=n0_future::time::timeout(deadline.saturating_duration_since(n0_future::time::Instant::now()),async {
                            let mut node=node.lock().await;
                            if precode_mode.load(std::sync::atomic::Ordering::Acquire) {node.serve_precode_admission(&connection).await} else {node.serve_manual_admission(&connection).await}
                        }).await;
                        println!("{}",json!({"event":"admission-ended","stableId":connection.stable_id(),"remoteId":connection.remote_id().to_string(),"stage":format!("{result:?}"),"paths":connection_paths(&connection)}));
                        if matches!(&result,Ok(Ok(_))) {let _=n0_future::time::timeout(deadline.saturating_duration_since(n0_future::time::Instant::now()),connection.closed()).await;}
                        connection.close(0u32.into(),b"owned admission ended");
                        return Ok::<_,Box<dyn Error+Send+Sync>>(());
                    }
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

#[cfg(test)]
mod tests {
    use super::fixture_options;

    fn arguments(tail: &[&str]) -> Vec<String> {
        ["fixture", "data", "facility", "ca"]
            .into_iter()
            .chain(tail.iter().copied())
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn admission_and_legacy_forms_keep_profile_separate_from_relay() {
        for (mode, admission, legacy, relay) in [
            (vec![], false, false, None),
            (vec!["admission"], true, false, None),
            (vec!["legacy-no-rtc"], false, true, None),
            (
                vec!["legacy-relay", "https://relay.test/"],
                false,
                true,
                Some("https://relay.test/"),
            ),
            (
                vec!["rtc-relay", "https://relay.test/"],
                false,
                false,
                Some("https://relay.test/"),
            ),
        ] {
            for profile in [false, true] {
                let mut tail = mode.clone();
                if profile {
                    tail.extend(["--ice-profile", "owned-profile.json"]);
                }
                let args = arguments(&tail);
                let parsed = fixture_options(&args).unwrap();
                assert_eq!(parsed.admission, admission);
                assert_eq!(parsed.legacy_only, legacy);
                assert_eq!(parsed.permitted_relay, relay);
                assert_eq!(parsed.ice_profile, profile.then_some("owned-profile.json"));
            }
        }
    }

    #[test]
    fn ambiguous_or_incomplete_profile_forms_are_rejected() {
        for tail in [
            vec!["--ice-profile"],
            vec!["admission", "--ice-profile"],
            vec!["--ice-profile", "one.json", "--ice-profile", "two.json"],
            vec!["--ice-profile", "owned-profile.json", "admission"],
            vec!["admission", "unrequested-relay"],
        ] {
            assert!(fixture_options(&arguments(&tail)).is_err(), "{tail:?}");
        }
    }
}
