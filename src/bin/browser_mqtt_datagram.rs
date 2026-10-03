//! JSON-lines driver for the real native carrier, with no QUIC or product UI.
use std::{error::Error, sync::Arc};

use miopunch_native_probe::{
    mqtt_datagram::MqttFacility,
    mqtt_transport::{MqttTransport, Traffic},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Deserialize)]
struct Config {
    facility: MqttFacility,
    #[serde(default)]
    disabled: Vec<String>,
}

fn emit(value: Value) {
    println!("{value}");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let args: Vec<_> = std::env::args().collect();
    let config: Config =
        serde_json::from_slice(&std::fs::read(args.get(1).ok_or("missing config")?)?)?;
    let ca = args.get(2).map(std::fs::read).transpose()?;
    let traffic = Arc::new(Traffic::default());
    let transport =
        MqttTransport::from_facility(&config.facility, &config.disabled, traffic.clone(), ca)?;
    let mut datagrams = transport.bind_datagrams()?;
    emit(
        json!({"event":"starting", "route_rejected":transport.route().is_err(), "address_count":datagrams.local_addresses().len(), "traffic":traffic.snapshot()}),
    );
    if let Err(err) = transport.wait_ready().await {
        let address_count = datagrams.local_addresses().len();
        drop(datagrams);
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        emit(
            json!({"event":"failed", "error":err.to_string(), "route_rejected":transport.route().is_err(), "address_count":address_count, "traffic":traffic.snapshot()}),
        );
        return Ok(());
    }
    emit(
        json!({"event":"ready", "route":transport.route()?, "address_count":datagrams.local_addresses().len(), "traffic":traffic.snapshot()}),
    );
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Some(line) = line? else { break; };
                let command: Value = serde_json::from_str(&line)?;
                match command["command"].as_str().ok_or("missing command")? {
                    "send" => {
                        let destination = MqttTransport::address(command["route"].as_str().ok_or("missing route")?)?;
                        let bytes = hex::decode(command["bytes"].as_str().ok_or("missing bytes")?)?;
                        let result = datagrams.send(&destination, &bytes).await;
                        emit(json!({"event":"sent", "id":command["id"], "accepted":result.is_ok(),
                            "error":result.err().map(|err|err.to_string()), "traffic":traffic.snapshot()}));
                    }
                    "status" => emit(json!({"event":"status", "route":transport.route().ok(), "address_count":datagrams.local_addresses().len(), "traffic":traffic.snapshot()})),
                    "stop" => break,
                    _ => return Err("unknown command".into()),
                }
            }
            packet = datagrams.receive() => match packet {
                Ok((source, bytes)) => emit(json!({"event":"received", "source":hex::encode(source.data()), "bytes":hex::encode(bytes), "traffic":traffic.snapshot()})),
                Err(err) => {
                    emit(json!({"event":"withdrawn", "error":err.to_string(), "route_rejected":transport.route().is_err(), "address_count":datagrams.local_addresses().len(), "traffic":traffic.snapshot()}));
                    break;
                }
            }
        }
    }
    drop(datagrams);
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    emit(
        json!({"event":"closed", "route_rejected":transport.route().is_err(), "traffic":traffic.snapshot()}),
    );
    Ok(())
}
