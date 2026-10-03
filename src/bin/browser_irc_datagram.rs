//! JSON-lines driver for the real native carrier, with no QUIC or product UI.
use std::error::Error;

use miopunch_native_probe::{
    irc_datagram::{IrcFacility, Profile, TRANSPORT_ID},
    irc_transport::IrcTransport,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Deserialize)]
struct Config {
    facility: IrcFacility,
    profile: Profile,
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
    let transport =
        IrcTransport::from_facility(&config.facility, &config.disabled, config.profile, ca)?;
    let traffic = transport.traffic().clone();
    let mut datagrams = transport.bind_datagrams()?;
    emit(
        json!({"event":"starting", "route_rejected":datagrams.local_addresses().is_empty(), "address_count":datagrams.local_addresses().len(), "traffic":traffic.snapshot()}),
    );
    if let Err(err) = transport.wait_ready().await {
        let address_count = datagrams.local_addresses().len();
        drop(datagrams);
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        emit(
            json!({"event":"failed", "error":err.to_string(), "route_rejected":true, "address_count":address_count, "traffic":traffic.snapshot()}),
        );
        return Ok(());
    }
    emit(
        json!({"event":"ready", "route":hex::encode(datagrams.local_addresses()[0].data()), "address_count":datagrams.local_addresses().len(), "traffic":traffic.snapshot()}),
    );
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Some(line) = line? else { break; };
                let command: Value = serde_json::from_str(&line)?;
                match command["command"].as_str().ok_or("missing command")? {
                    "send" => {
                        let destination = iroh_base::CustomAddr::from_parts(TRANSPORT_ID,&hex::decode(command["route"].as_str().ok_or("missing route")?)?);
                        let bytes = hex::decode(command["bytes"].as_str().ok_or("missing bytes")?)?;
                        let result = datagrams.send(&destination, &bytes).await;
                        emit(json!({"event":"sent", "id":command["id"], "accepted":result.is_ok(),
                            "error":result.err().map(|err|err.to_string()), "traffic":traffic.snapshot()}));
                    }
                    "status" => emit(json!({"event":"status", "route":datagrams.local_addresses().first().map(|address|hex::encode(address.data())), "address_count":datagrams.local_addresses().len(), "traffic":traffic.snapshot()})),
                    "stop" => break,
                    _ => return Err("unknown command".into()),
                }
            }
            packet = datagrams.receive() => match packet {
                Ok((source, bytes)) => emit(json!({"event":"received", "source":hex::encode(source.data()), "bytes":hex::encode(bytes), "traffic":traffic.snapshot()})),
                Err(err) => {
                    emit(json!({"event":"withdrawn", "error":err.to_string(), "route_rejected":datagrams.local_addresses().is_empty(), "address_count":datagrams.local_addresses().len(), "traffic":traffic.snapshot()}));
                    break;
                }
            }
        }
    }
    drop(datagrams);
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while traffic.snapshot()["driver_active"] != 0 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await?;
    emit(json!({"event":"closed", "route_rejected":true, "traffic":traffic.snapshot()}));
    Ok(())
}
