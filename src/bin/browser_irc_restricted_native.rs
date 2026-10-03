//! Owned IRC facility fixture for the existing restricted admission protocol.
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
    profile: Profile,
    caller_budget_ms: u64,
    #[serde(default)]
    mqtt: Option<MqttFacility>,
    #[serde(default)]
    disabled: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err("usage: browser_irc_restricted_native CONFIG TEST_CA_PEM DATA_DIR".into());
    }
    let config: Configuration = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let node = NativeNetwork::create(&args[3]).await?;
    config.facility.check_allowed(&config.disabled)?;
    let mut resources = node.resource_configuration()?;
    let mut signaling = vec![miopunch_native_probe::resources::Resource {
        url: config.facility.native_url.clone(),
        uses: [miopunch_native_probe::resources::ResourceUse::Signaling].into(),
        limitation: "owned restricted IRC facility qualification".into(),
    }];
    if let Some(facility) = &config.mqtt {
        facility.check_allowed(&config.disabled)?;
        signaling.push(miopunch_native_probe::resources::Resource {
            url: facility.native_url.clone(),
            uses: [miopunch_native_probe::resources::ResourceUse::Signaling].into(),
            limitation: "owned MQTT mixed-carrier retry regression".into(),
        });
    }
    resources.import_trusted_replacement(signaling)?;
    resources.lan_discovery = miopunch_native_probe::resources::LanDiscovery::Disabled;
    node.save_resource_configuration(&resources)?;
    let _ = rustls::crypto::ring::default_provider().install_default();
    let transport = Arc::new(IrcTransport::from_facility(
        &config.facility,
        &config.disabled,
        config.profile.clone(),
        Some(std::fs::read(&args[2])?),
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
                Some(std::fs::read(&args[2])?),
            )
            .map(Arc::new)
        })
        .transpose()?;
    let mut builder = iroh::Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .relay_mode(RelayMode::Disabled)
        .alpns(vec![ADMISSION_REQUEST_ALPN.to_vec()])
        .add_custom_transport(transport.clone());
    if let Some(mqtt) = &mqtt {
        builder = builder.add_custom_transport(mqtt.clone());
    }
    let endpoint = node.bind_endpoint_with(builder).await?;
    tokio::time::timeout_at(startup_deadline, transport.wait_ready()).await??;
    if let Some(mqtt) = &mqtt {
        tokio::time::timeout_at(startup_deadline, mqtt.wait_ready()).await??;
    }
    let route = endpoint
        .custom_addrs()
        .iter()
        .find(|address| address.id() == miopunch_native_probe::irc_datagram::TRANSPORT_ID)
        .map(|address| hex::encode(address.data()))
        .ok_or("no qualified IRC custom route")?;
    println!(
        "{}",
        json!({"event":"ready","id":endpoint.id().to_string(),"route":route,"mqtt_traffic":mqtt.as_ref().map(|_|mqtt_traffic.snapshot()),"mqtt_route":mqtt.as_ref().map(|transport|transport.route()).transpose()?,"invitation":hex::encode(node.manual_invitation()?.encode())})
    );
    let gate = node.admission_gate().clone();
    let node = Arc::new(tokio::sync::Mutex::new(node));
    let mut tasks = tokio::task::JoinSet::new();
    let mut input = BufReader::new(tokio::io::stdin()).lines();
    loop {
        tokio::select! {
            incoming=endpoint.accept()=> {
                let Some(incoming)=incoming else {break};
                let node = node.clone();
                let gate = gate.clone();
                let endpoint = endpoint.clone();
                let traffic = traffic.clone();
                let budget = Duration::from_millis(config.caller_budget_ms);
                tasks.spawn(async move {
                    let started=tokio::time::Instant::now();
                    let deadline=started+budget;
                    match tokio::time::timeout_at(deadline,incoming).await {
                        Ok(Ok(connection))=> {
                            let handshake_ms=started.elapsed().as_millis();
                            println!("{}",json!({"event":"established","remote_id":connection.remote_id().to_string(),"alpn":String::from_utf8_lossy(connection.alpn()),"stable_id":connection.stable_id(),"handshake_ms":handshake_ms,"stats":format!("{:?}",connection.stats()),"traffic":traffic.snapshot()}));
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
                        result=>println!("{}",json!({"event":"handshake-rejected","error":format!("{result:?}"),"elapsed_ms":started.elapsed().as_millis(),"traffic":traffic.snapshot()})),
                    }
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
                    Some("stop")|None=>break,
                    _=>return Err("unknown fixture command".into()),
                }
            }
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    let cleanup_started = tokio::time::Instant::now();
    transport.close();
    tokio::time::timeout(Duration::from_secs(2), async {
        while traffic.snapshot()["driver_active"] != 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| "owned IRC driver did not finish cleanup")?;
    let graceful_close = tokio::time::timeout(Duration::from_millis(500), endpoint.close())
        .await
        .is_ok();
    drop(endpoint);
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
