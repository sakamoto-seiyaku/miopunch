//! Owned MQTT facility fixture for the existing restricted admission protocol.
use iroh::{RelayMode, endpoint::presets};
use miopunch_native_probe::{
    admission_gate::{ADMISSION_REQUEST_ALPN, ProtocolScope},
    mqtt_datagram::MqttFacility,
    mqtt_transport::{MqttTransport, Traffic},
    native_network::NativeNetwork,
};
use serde::Deserialize;
use serde_json::json;
use std::{error::Error, sync::Arc, time::Duration};
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Deserialize)]
struct Configuration {
    facility: MqttFacility,
    #[serde(default)]
    disabled: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err("usage: browser_restricted_native CONFIG TEST_CA_PEM DATA_DIR".into());
    }
    let config: Configuration = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let mut node = NativeNetwork::create(&args[3]).await?;
    config.facility.check_allowed(&config.disabled)?;
    let mut resources = node.resource_configuration()?;
    resources.import_trusted_replacement(vec![miopunch_native_probe::resources::Resource {
        url: config.facility.native_url.clone(),
        uses: [miopunch_native_probe::resources::ResourceUse::Signaling].into(),
        limitation: "owned restricted MQTT facility qualification".into(),
    }])?;
    resources.lan_discovery = miopunch_native_probe::resources::LanDiscovery::Disabled;
    node.save_resource_configuration(&resources)?;
    let traffic = Arc::new(Traffic::default());
    let transport = Arc::new(MqttTransport::from_facility(
        &config.facility,
        &config.disabled,
        traffic.clone(),
        Some(std::fs::read(&args[2])?),
    )?);
    let endpoint = node
        .bind_endpoint_with(
            iroh::Endpoint::builder(presets::Minimal)
                .clear_ip_transports()
                .relay_mode(RelayMode::Disabled)
                .alpns(vec![ADMISSION_REQUEST_ALPN.to_vec()])
                .add_custom_transport(transport.clone()),
        )
        .await?;
    tokio::time::timeout(Duration::from_secs(10), transport.wait_ready()).await??;
    println!(
        "{}",
        json!({"event":"ready","id":endpoint.id().to_string(),"route":transport.route()?,"invitation":hex::encode(node.manual_invitation()?.encode())})
    );
    let mut input = BufReader::new(tokio::io::stdin()).lines();
    loop {
        tokio::select! {
            incoming=endpoint.accept()=> {
                let Some(incoming)=incoming else {break};
                let started=tokio::time::Instant::now();
                let deadline=started+Duration::from_secs(12);
                match tokio::time::timeout(Duration::from_secs(12),incoming).await {
                    Ok(Ok(connection))=> {
                        let handshake_ms=started.elapsed().as_millis();
                        let stage=tokio::time::timeout_at(deadline,node.serve_manual_admission(&connection)).await;
                        println!("{}",json!({"event":"admission","local_id":endpoint.id().to_string(),"remote_id":connection.remote_id().to_string(),"alpn":String::from_utf8_lossy(connection.alpn()),"stable_id":connection.stable_id(),"handshake_ms":handshake_ms,"exchange_ms":started.elapsed().as_millis(),"restricted":node.admission_gate().allows(&connection,ProtocolScope::AdmissionRequest),"business_handle":node.admission_gate().allows(&connection,ProtocolScope::Member),"stage":format!("{stage:?}"),"stats":format!("{:?}",connection.stats()),"traffic":traffic.snapshot()}));
                        connection.close(0u32.into(),b"restricted fixture exchange ended");
                    }
                    result=>println!("{}",json!({"event":"handshake-rejected","error":format!("{result:?}"),"elapsed_ms":started.elapsed().as_millis()})),
                }
            }
            line=input.next_line()=>{
                match line?.as_deref() {
                    Some("status")=>{
                        let mut requests=Vec::new();
                        for request in node.manual_requests().await? {
                            let approved=node.approved_admission(&request).await?.is_some();
                            requests.push(json!({"applicant":request.applicant.to_string(),"request_id":hex::encode(request.request_id),"approved":approved}));
                        }
                        println!("{}",json!({"event":"status","requests":requests,"members":node.status().members.iter().map(|(id,_)|id.to_string()).collect::<Vec<_>>() }));
                    }
                    Some("stop")|None=>break,
                    _=>return Err("unknown fixture command".into()),
                }
            }
        }
    }
    endpoint.close().await;
    drop(endpoint);
    tokio::time::timeout(Duration::from_secs(1), transport.wait_closed()).await??;
    // Closed withdraws route eligibility before the broker driver's final
    // DISCONNECT write. This is a fixture cleanup receipt, not a contact wait.
    tokio::time::timeout(Duration::from_secs(1), async {
        while !traffic.snapshot()["protocol"]["disconnect_sent"]
            .as_bool()
            .unwrap_or(false)
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .map_err(|_| "owned MQTT driver did not finish disconnect")?;
    println!("{}", json!({"event":"closed","traffic":traffic.snapshot()}));
    node.close().await;
    Ok(())
}
