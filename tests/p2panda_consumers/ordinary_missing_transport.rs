use std::{net::{Ipv4Addr,Ipv6Addr},time::{Duration,Instant}};
use p2panda_net::{AddressBook,Endpoint,iroh_endpoint::IrohConfig};
#[tokio::main]
async fn main() {
 let book=AddressBook::builder().spawn().await.unwrap();
 let endpoint=Endpoint::builder(book).config(IrohConfig {bind_ip_v4:Ipv4Addr::LOCALHOST,bind_port_v4:0,bind_ip_v6:Ipv6Addr::LOCALHOST,bind_port_v6:0}).spawn().await.unwrap();
 let inner=endpoint.endpoint().await.unwrap();
 let target=p2panda_core::SigningKey::generate().verifying_key();
 let began=Instant::now();
 let result=tokio::time::timeout(Duration::from_millis(750),endpoint.connect(target,b"missing-transport/1")).await;
 let elapsed=began.elapsed();
 let expected=format!("address book does not have any iroh address info for node id {target}");
 let pass=matches!(&result,Ok(Err(p2panda_net::iroh_endpoint::EndpointError::Connect(err))) if err.to_string()==expected) && elapsed<Duration::from_millis(500);
 println!("MISSING_TRANSPORT result={result:?} elapsed_us={} pass={pass}",elapsed.as_micros());
 inner.close().await;drop((endpoint,inner));
 println!("ENDPOINT_CLOSE_RETURNED");
 assert!(pass,"ordinary missing-transport contract changed");
}
