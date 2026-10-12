use iroh_relay::server::{RelayConfig,Server,ServerConfig};
use std::net::Ipv4Addr;
use tokio::io::{AsyncBufReadExt,BufReader};
#[tokio::main]
async fn main()->Result<(),Box<dyn std::error::Error>>{
 let mut config=ServerConfig::default();config.relay=Some(RelayConfig::new((Ipv4Addr::LOCALHOST,0)));
 let server=Server::spawn(config).await?;
 println!("{}",serde_json::json!({"event":"ready","url":format!("http://localhost:{}/",server.http_addr().unwrap().port())}));
 let mut input=BufReader::new(tokio::io::stdin()).lines();let _=input.next_line().await?;
 server.shutdown().await?;println!("{}",serde_json::json!({"event":"closed","graceful":true}));Ok(())
}
