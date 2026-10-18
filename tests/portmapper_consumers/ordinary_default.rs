use iroh::endpoint::{presets, PortmapperConfig};
use iroh::{Endpoint, EndpointAddr};
use std::path::PathBuf;
use std::time::Duration;

const ALPN: &[u8] = b"m98-original-portmapper-consumer/1";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let role = arguments.next().expect("role");
    let directory = PathBuf::from(arguments.next().expect("owned fixture directory"));
    // The serving consumer intentionally leaves the ordinary mapping default untouched.
    let mut builder = Endpoint::builder(presets::Minimal).alpns(vec![ALPN.to_vec()]);
    if role == "connect" { builder = builder.portmapper_config(PortmapperConfig::Disabled); }
    if role != "connect" {
        // Upstream starts net-report/portmapping only with a nonempty relay map.
        // This isolated lab address has no public or real LAN route.
        builder = builder.relay_mode(iroh::RelayMode::custom(["http://10.39.0.1:3340/".parse()?]));
    }
    let endpoint = builder.bind().await?;
    let result: Result<(), Box<dyn std::error::Error>> = async {
        if role == "quiet" {
            tokio::time::sleep(Duration::from_secs(2)).await;
            assert!(!endpoint.addr().ip_addrs().any(|address| address.ip().to_string() == "100.64.39.1"));
            println!("NO_FEATURE_QUIET");
            return Ok(());
        }
        if role == "serve" {
            let mapped = tokio::time::timeout(Duration::from_secs(8), async {
                loop {
                    if let Some(address) = endpoint.addr().ip_addrs().find(|address| address.ip().to_string() == "100.64.39.1").copied() { break address; }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            }).await?;
            std::fs::write(directory.join("mapped"), format!("{}\n{mapped}\n", endpoint.id()))?;
            let connection = tokio::time::timeout(Duration::from_secs(8), endpoint.accept()).await?.ok_or("endpoint closed")?.await?;
            let (mut send, mut recv) = connection.accept_bi().await?;
            assert_eq!(recv.read_to_end(32).await?, b"PING");
            send.write_all(b"PONG").await?;
            send.finish()?;
            tokio::time::timeout(Duration::from_secs(4), connection.closed()).await?;
            println!("DEFAULT_MAPPING_DATA_OK");
        } else {
            assert_eq!(role, "connect");
            let text = std::fs::read_to_string(directory.join("mapped"))?;
            let mut lines = text.lines();
            let id = lines.next().ok_or("missing identity")?.parse()?;
            let address = lines.next().ok_or("missing mapped address")?.parse()?;
            let connection = endpoint.connect(EndpointAddr::new(id).with_ip_addr(address), ALPN).await?;
            let (mut send, mut recv) = connection.open_bi().await?;
            send.write_all(b"PING").await?;
            send.finish()?;
            assert_eq!(recv.read_to_end(32).await?, b"PONG");
            connection.close(0u32.into(), b"default mapping consumer complete");
            println!("MAPPED_REMOTE_DATA_OK");
        }
        Ok(())
    }.await;
    let started = std::time::Instant::now();
    endpoint.close().await;
    println!("ENDPOINT_CLOSE_RETURNED seconds={:.3}", started.elapsed().as_secs_f64());
    // Retain the runtime briefly to observe upstream's best-effort release too.
    tokio::time::sleep(Duration::from_millis(500)).await;
    result
}
