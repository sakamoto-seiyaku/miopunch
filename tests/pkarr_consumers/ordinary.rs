use iroh::{Endpoint, SecretKey};
use iroh::address_lookup::{AddressLookup, EndpointData, PkarrPublisher, PkarrRelayClient, PkarrResolver};
use iroh::endpoint::{presets, PortmapperConfig};
use iroh_dns::endpoint_info::EndpointInfo;
use n0_future::StreamExt;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let mode = &args[1];
    let url = args[2].parse()?;
    let endpoint = Endpoint::builder(presets::Minimal)
        .portmapper_config(PortmapperConfig::Disabled).bind().await?;
    let result: Result<(), Box<dyn std::error::Error>> = async {
        let key = SecretKey::generate();
        if mode == "raw-new" {
            let client = PkarrRelayClient::new(url, endpoint.tls_config().clone(), endpoint.dns_resolver()?.clone());
            let packet = EndpointInfo::from_parts(key.public(), EndpointData::default())
                .to_pkarr_signed_packet(&key, 30)?;
            client.publish(&packet).await?;
            let resolved = client.resolve(key.public()).await?;
            assert_eq!(resolved.to_relay_payload(), packet.to_relay_payload());
        } else {
            assert_eq!(mode, "publisher-resolver");
            let publisher = PkarrPublisher::builder(url.clone())
                .republish_interval(Duration::from_secs(60))
                .build(key.clone(), endpoint.tls_config().clone());
            publisher.publish(&EndpointData::default());
            let resolver = PkarrResolver::builder(url).build(endpoint.tls_config().clone());
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if let Some(Ok(item)) = resolver.resolve(key.public()).unwrap().next().await {
                        assert_eq!(item.endpoint_id(), key.public());
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(30)).await;
                }
            }).await?;
            drop(publisher);
        }
        println!("SIGNED_PUBLISH_RESOLVE_OK mode={mode}");
        Ok(())
    }.await;
    endpoint.close().await;
    println!("ENDPOINT_CLOSE_RETURNED");
    result
}
