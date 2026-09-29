use std::error::Error;
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use iroh::address_lookup::memory::MemoryLookup;
use iroh::endpoint::QuicTransportConfig;
use iroh::endpoint::presets;
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr};
use iroh_base::{PublicKey, SecretKey};
use noq_proto::congestion::Bbr3Config;
use p2panda_core::{SigningKey, VerifyingKey};
use p2panda_net::iroh_endpoint::IrohConfig;
use p2panda_net::{AddressBook, Endpoint as PandaEndpoint};
use std::sync::Arc;
use tokio::sync::mpsc;

const PROBE_ALPN: &[u8] = b"miopunch/probe/1";
const PING: &[u8] = b"PING";
const PONG: &[u8] = b"PONG";
const USAGE: &str = "usage: miopunch-native-probe serve raw|wrapped DATA_DIR [BIND_IPV4] | send raw|wrapped DATA_DIR TARGET_ID TARGET_UDP_ADDR [BIND_IPV4]";

#[derive(Debug, Clone)]
struct Echo(mpsc::UnboundedSender<()>);

impl ProtocolHandler for Echo {
    async fn accept(&self, connection: iroh::endpoint::Connection) -> Result<(), AcceptError> {
        let (mut send, mut recv) = connection.accept_bi().await?;
        let request = recv.read_to_end(64).await.map_err(std::io::Error::other)?;
        if request == PING {
            send.write_all(PONG).await.map_err(std::io::Error::other)?;
        }
        send.finish()?;
        connection.closed().await;
        let _ = self.0.send(());
        Ok(())
    }
}

fn signing_key(data_dir: &Path) -> Result<SigningKey, Box<dyn Error>> {
    std::fs::create_dir_all(data_dir)?;
    let path = data_dir.join("node.key");
    if !path.exists() {
        let generated = SigningKey::generate();
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&path)?;
        file.write_all(generated.as_bytes())?;
        file.sync_all()?;
    }
    let bytes: [u8; 32] = std::fs::read(path)?
        .try_into()
        .map_err(|_| "invalid node key length")?;
    Ok(SigningKey::from_bytes(&bytes))
}

fn iroh_secret(key: &SigningKey) -> SecretKey {
    SecretKey::from_bytes(key.as_bytes())
}

fn quic_config() -> QuicTransportConfig {
    QuicTransportConfig::builder()
        .keep_alive_interval(Duration::from_secs(5))
        .max_idle_timeout(Some(Duration::from_secs(10).try_into().unwrap()))
        .congestion_controller_factory(Arc::new(Bbr3Config::default()))
        .build()
}

async fn raw_endpoint(key: &SigningKey, bind_ip: Ipv4Addr) -> Result<Endpoint, Box<dyn Error>> {
    let endpoint = Endpoint::builder(presets::Minimal)
        .secret_key(iroh_secret(key))
        .transport_config(quic_config())
        .bind_addr(SocketAddr::from((bind_ip, 0)))?
        .bind()
        .await?;
    Ok(endpoint)
}

fn local_udp_addr(endpoint: &Endpoint) -> Result<SocketAddr, Box<dyn Error>> {
    let addr = endpoint.addr();
    addr.ip_addrs()
        .find(|addr| addr.is_ipv4() && !addr.ip().is_loopback())
        .or_else(|| addr.ip_addrs().find(|addr| addr.is_ipv4()))
        .copied()
        .ok_or_else(|| "no IPv4 UDP address".into())
}

async fn serve_raw(key: &SigningKey, bind_ip: Ipv4Addr) -> Result<(), Box<dyn Error>> {
    let endpoint = raw_endpoint(key, bind_ip).await?;
    let (done_tx, mut done_rx) = mpsc::unbounded_channel();
    let router = Router::builder(endpoint.clone())
        .accept(PROBE_ALPN, Echo(done_tx))
        .spawn();
    println!(
        "READY {} {}",
        iroh_secret(key).public(),
        local_udp_addr(&endpoint)?
    );
    done_rx.recv().await.ok_or("server stopped before echo")?;
    router.shutdown().await?;
    Ok(())
}

async fn send_raw(
    key: &SigningKey,
    target: PublicKey,
    addr: SocketAddr,
    bind_ip: Ipv4Addr,
) -> Result<(), Box<dyn Error>> {
    let endpoint = raw_endpoint(key, bind_ip).await?;
    let target = EndpointAddr::new(target).with_ip_addr(addr);
    let connection = tokio::time::timeout(Duration::from_secs(8), async {
        endpoint.connect(target, PROBE_ALPN).await
    })
    .await??;
    ping(connection).await?;
    endpoint.close().await;
    Ok(())
}

async fn ping(connection: iroh::endpoint::Connection) -> Result<(), Box<dyn Error>> {
    let (mut send, mut recv) = connection.open_bi().await?;
    send.write_all(PING).await?;
    send.finish()?;
    let response = recv.read_to_end(64).await?;
    if response != PONG {
        return Err("unexpected echo response".into());
    }
    println!("PONG");
    connection.close(0u32.into(), b"done");
    Ok(())
}

fn panda_config(bind_ip: Ipv4Addr) -> IrohConfig {
    IrohConfig {
        bind_ip_v4: bind_ip,
        bind_port_v4: 0,
        bind_ip_v6: std::net::Ipv6Addr::LOCALHOST,
        bind_port_v6: 0,
    }
}

async fn serve_wrapped(key: &SigningKey, bind_ip: Ipv4Addr) -> Result<(), Box<dyn Error>> {
    let address_book = AddressBook::builder().spawn().await?;
    let endpoint = PandaEndpoint::builder(address_book)
        .signing_key(key.clone())
        .config(panda_config(bind_ip))
        .endpoint_builder_hook(|builder| builder.transport_config(quic_config()))
        .spawn()
        .await?;
    let (done_tx, mut done_rx) = mpsc::unbounded_channel();
    endpoint.accept(PROBE_ALPN, Echo(done_tx)).await?;
    let iroh_endpoint = endpoint.endpoint().await?;
    assert_eq!(endpoint.node_id().as_bytes(), iroh_endpoint.id().as_bytes());
    println!(
        "READY {} {}",
        iroh_endpoint.id(),
        local_udp_addr(&iroh_endpoint)?
    );
    done_rx.recv().await.ok_or("server stopped before echo")?;
    drop(endpoint);
    Ok(())
}

async fn send_wrapped(
    key: &SigningKey,
    target: PublicKey,
    addr: SocketAddr,
    bind_ip: Ipv4Addr,
) -> Result<(), Box<dyn Error>> {
    let address_book = AddressBook::builder().spawn().await?;
    let lookup = MemoryLookup::from_endpoint_info([EndpointAddr::new(target).with_ip_addr(addr)]);
    let endpoint = PandaEndpoint::builder(address_book)
        .signing_key(key.clone())
        .config(panda_config(bind_ip))
        .endpoint_builder_hook(move |builder| {
            builder
                .address_lookup(lookup.clone())
                .transport_config(quic_config())
        })
        .spawn()
        .await?;
    let target = VerifyingKey::from_bytes(target.as_bytes())?;
    let connection =
        tokio::time::timeout(Duration::from_secs(8), endpoint.connect(target, PROBE_ALPN))
            .await??;
    ping(connection).await?;
    drop(endpoint);
    Ok(())
}

fn optional_bind_ip(args: &[String]) -> Result<Ipv4Addr, Box<dyn Error>> {
    match args {
        [] => Ok(Ipv4Addr::LOCALHOST),
        [bind_ip] => Ok(bind_ip.parse()?),
        _ => Err(USAGE.into()),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    if !matches!(args.get(2).map(String::as_str), Some("raw" | "wrapped")) {
        return Err(USAGE.into());
    }
    match args.as_slice() {
        [_, action, mode, data_dir, bind @ ..] if action == "serve" => {
            let bind_ip = optional_bind_ip(bind)?;
            let key = signing_key(Path::new(data_dir))?;
            match mode.as_str() {
                "raw" => serve_raw(&key, bind_ip).await,
                "wrapped" => serve_wrapped(&key, bind_ip).await,
                _ => Err(USAGE.into()),
            }
        }
        [_, action, mode, data_dir, target, addr, bind @ ..] if action == "send" => {
            let bind_ip = optional_bind_ip(bind)?;
            let key = signing_key(Path::new(data_dir))?;
            let target = PublicKey::from_str(target)?;
            let addr = SocketAddr::from_str(addr)?;
            match mode.as_str() {
                "raw" => send_raw(&key, target, addr, bind_ip).await,
                "wrapped" => send_wrapped(&key, target, addr, bind_ip).await,
                _ => Err(USAGE.into()),
            }
        }
        _ => Err(USAGE.into()),
    }
}
