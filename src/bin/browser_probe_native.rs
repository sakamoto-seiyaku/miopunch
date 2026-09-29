use std::{error::Error, net::SocketAddr, str::FromStr};

use iroh::{Endpoint, RelayMode, endpoint::presets};
use iroh_base::RelayUrl;
use iroh_relay::server::{RelayConfig, Server, ServerConfig};

const ALPN: &[u8] = b"miopunch/browser-probe/1";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    match args.as_slice() {
        [_, action, port] if action == "relay" => {
            let addr = SocketAddr::from_str(&format!("127.0.0.1:{port}"))?;
            let mut config = ServerConfig::default();
            config.relay = Some(RelayConfig::new(addr));
            let server = Server::spawn(config).await?;
            println!("RELAY {}", server.http_addr().ok_or("no relay address")?);
            tokio::signal::ctrl_c().await?;
            server.shutdown().await?;
        }
        [_, action, relay_url] if action == "serve" => {
            let relay_url: RelayUrl = relay_url.parse()?;
            let endpoint = Endpoint::builder(presets::Minimal)
                .relay_mode(RelayMode::custom([relay_url]))
                .clear_ip_transports()
                .alpns(vec![ALPN.to_vec()])
                .bind()
                .await?;
            endpoint.online().await;
            println!("READY {}", endpoint.id());
            loop {
                let incoming = tokio::select! {
                    incoming = endpoint.accept() => incoming.ok_or("endpoint closed")?,
                    _ = tokio::signal::ctrl_c() => break,
                };
                let connection = incoming.await?;
                let (mut send, mut recv) = connection.accept_bi().await?;
                let request = recv.read_to_end(64).await?;
                if request != b"PING" {
                    return Err("unexpected request".into());
                }
                send.write_all(b"PONG").await?;
                send.finish()?;
                connection.closed().await;
                println!("PONG");
            }
            endpoint.close().await;
        }
        _ => return Err("usage: browser-probe-native relay <port> | serve <relay-url>".into()),
    }
    Ok(())
}
