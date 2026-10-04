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
                    "activeRtcAssociations":rtc_transport.active_associations(),"activeRtcPumps":rtc_transport.active_pumps()})),
                _=>return Err("unknown fixture command".into()),
            },
            incoming=endpoint.accept()=>{
                let Some(incoming)=incoming else {break};
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
                        node.lock().await.serve_replica(&connection).await?;
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
