mod desktop_cli;
mod lifecycle_cli;

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

use miopunch_native_probe::mqtt_transport::{MqttTransport, Traffic as MqttTraffic};
use miopunch_native_probe::native_network::{NativeNetwork, NodeStatus};
use miopunch_native_probe::tcp_transport;
use tcp_transport::{TcpOnly, TcpTransport, Traffic};

const PROBE_ALPN: &[u8] = b"miopunch/probe/1";
const MQTT_CONTROL_ALPN: &[u8] = b"miopunch/control-probe/1";
const PING: &[u8] = b"PING";
const PONG: &[u8] = b"PONG";
const USAGE: &str = "usage: miopunch-native-probe network create|status|resources DATA_DIR | network join DATA_DIR INVITATION_FILE | network backup|restore SOURCE_DIR TARGET_DIR | network session|maintain DATA_DIR [BIND_IPV4] | network resources-import|resources-update DATA_DIR JSON_FILE | network run DATA_DIR [BIND_IPV4] | identity DATA_DIR | serve raw|wrapped|tcp DATA_DIR [BIND_IPV4] | send raw|wrapped|tcp DATA_DIR TARGET_ID TARGET_ADDR [BIND_IPV4] | serve mqtt DATA_DIR BROKER_ADDR EXPECTED_PEER_ID | send mqtt DATA_DIR TARGET_ID TARGET_ROUTE BROKER_ADDR | cancel mqtt DATA_DIR BROKER_ADDR";

fn print_network_status(status: &NodeStatus) {
    println!("NETWORK {} {}", status.anchor.root, status.anchor.genesis);
    println!("NODE {}", status.node_id);
    for (node, access) in &status.members {
        let role = if *access == p2panda_auth::Access::manage() {
            "admin"
        } else {
            "member"
        };
        println!("MEMBER {node} {role}");
    }
    println!("RECORDS {}", status.applied_count);
}

#[derive(Debug, Clone)]
struct ProbeEcho {
    done: mpsc::UnboundedSender<()>,
    expected_peer: Option<PublicKey>,
}

impl ProtocolHandler for ProbeEcho {
    async fn accept(&self, connection: iroh::endpoint::Connection) -> Result<(), AcceptError> {
        if self
            .expected_peer
            .is_some_and(|expected| connection.remote_id() != expected)
        {
            connection.close(0u32.into(), b"unexpected peer");
            return Ok(());
        }
        let (mut send, mut recv) = connection.accept_bi().await?;
        let request = recv.read_to_end(64).await.map_err(std::io::Error::other)?;
        if request == PING {
            send.write_all(PONG).await.map_err(std::io::Error::other)?;
        }
        send.finish()?;
        connection.closed().await;
        let _ = self.done.send(());
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
        .accept(
            PROBE_ALPN,
            ProbeEcho {
                done: done_tx,
                expected_peer: None,
            },
        )
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

async fn tcp_endpoint(
    key: &SigningKey,
    transport: Arc<TcpTransport>,
) -> Result<Endpoint, Box<dyn Error>> {
    Ok(Endpoint::builder(presets::Minimal)
        .secret_key(iroh_secret(key))
        .transport_config(quic_config())
        .clear_ip_transports()
        .clear_relay_transports()
        .add_custom_transport(transport)
        .path_selector(Arc::new(TcpOnly))
        .bind()
        .await?)
}

async fn serve_tcp(key: &SigningKey, bind_ip: Ipv4Addr) -> Result<(), Box<dyn Error>> {
    let traffic = Arc::new(Traffic::default());
    let transport = Arc::new(TcpTransport::new(bind_ip, traffic.clone()));
    let endpoint = tcp_endpoint(key, transport.clone()).await?;
    let addr = transport.bound_addr().ok_or("no TCP carrier address")?;
    let (done_tx, mut done_rx) = mpsc::unbounded_channel();
    let router = Router::builder(endpoint)
        .accept(
            PROBE_ALPN,
            ProbeEcho {
                done: done_tx,
                expected_peer: None,
            },
        )
        .spawn();
    println!("READY {} {}", iroh_secret(key).public(), addr);
    tokio::time::timeout(Duration::from_secs(20), done_rx.recv())
        .await?
        .ok_or("server stopped before echo")?;
    router.shutdown().await?;
    eprintln!("{}", traffic.report());
    Ok(())
}

async fn send_tcp(
    key: &SigningKey,
    target: PublicKey,
    addr: SocketAddr,
    bind_ip: Ipv4Addr,
) -> Result<(), Box<dyn Error>> {
    let SocketAddr::V4(addr) = addr else {
        return Err("TCP probe requires IPv4".into());
    };
    let traffic = Arc::new(Traffic::default());
    let transport = Arc::new(TcpTransport::new(bind_ip, traffic.clone()));
    let endpoint = tcp_endpoint(key, transport).await?;
    let target = EndpointAddr::new(target).with_addrs([iroh_base::TransportAddr::Custom(
        TcpTransport::address(addr),
    )]);
    let connection =
        tokio::time::timeout(Duration::from_secs(8), endpoint.connect(target, PROBE_ALPN))
            .await??;
    if !connection.paths().iter().any(|path| {
        path.is_selected() && matches!(path.remote_addr(), iroh_base::TransportAddr::Custom(_))
    }) {
        return Err("TCP custom path was not selected".into());
    }
    tokio::time::timeout(Duration::from_secs(8), ping(connection)).await??;
    endpoint.close().await;
    eprintln!("{}", traffic.report());
    Ok(())
}

async fn mqtt_endpoint(
    key: &SigningKey,
    transport: Arc<MqttTransport>,
) -> Result<Endpoint, Box<dyn Error>> {
    Ok(Endpoint::builder(presets::Minimal)
        .secret_key(iroh_secret(key))
        .transport_config(quic_config())
        .clear_ip_transports()
        .clear_relay_transports()
        .add_custom_transport(transport)
        .bind()
        .await?)
}

async fn serve_mqtt(
    key: &SigningKey,
    broker: SocketAddr,
    expected_peer: PublicKey,
) -> Result<(), Box<dyn Error>> {
    let traffic = Arc::new(MqttTraffic::default());
    let transport = Arc::new(MqttTransport::new(broker, traffic.clone()));
    let endpoint = mqtt_endpoint(key, transport.clone()).await?;
    transport.wait_ready().await?;
    let (done_tx, mut done_rx) = mpsc::unbounded_channel();
    let router = Router::builder(endpoint)
        .accept(
            MQTT_CONTROL_ALPN,
            ProbeEcho {
                done: done_tx,
                expected_peer: Some(expected_peer),
            },
        )
        .spawn();
    println!("READY {} {}", iroh_secret(key).public(), transport.route()?);
    tokio::select! {
        result = done_rx.recv() => { result.ok_or("server stopped before control exchange")?; }
        _ = transport.wait_closed() => return Err("MQTT carrier closed before control exchange".into()),
        _ = tokio::time::sleep(Duration::from_secs(20)) => return Err("MQTT control exchange timed out".into()),
    }
    router.shutdown().await?;
    eprintln!("{}", traffic.report());
    Ok(())
}

async fn send_mqtt(
    key: &SigningKey,
    target: PublicKey,
    route: &str,
    broker: SocketAddr,
) -> Result<(), Box<dyn Error>> {
    let traffic = Arc::new(MqttTraffic::default());
    let transport = Arc::new(MqttTransport::new(broker, traffic.clone()));
    let endpoint = mqtt_endpoint(key, transport.clone()).await?;
    transport.wait_ready().await?;
    let target = EndpointAddr::new(target).with_addrs([iroh_base::TransportAddr::Custom(
        MqttTransport::address(route)?,
    )]);
    let connection = tokio::select! {
        result = tokio::time::timeout(Duration::from_secs(8), endpoint.connect(target, MQTT_CONTROL_ALPN)) => result??,
        _ = transport.wait_closed() => return Err("MQTT carrier closed during handshake".into()),
    };
    if !connection.paths().iter().any(|path| {
        path.is_selected() && matches!(path.remote_addr(), iroh_base::TransportAddr::Custom(_))
    }) {
        return Err("MQTT custom path was not selected".into());
    }
    tokio::select! {
        result = tokio::time::timeout(Duration::from_secs(8), ping(connection)) => result??,
        _ = transport.wait_closed() => return Err("MQTT carrier closed during control exchange".into()),
    }
    endpoint.close().await;
    eprintln!("{}", traffic.report());
    Ok(())
}

async fn cancel_mqtt(key: &SigningKey, broker: SocketAddr) -> Result<(), Box<dyn Error>> {
    let transport = Arc::new(MqttTransport::new(broker, Arc::new(MqttTraffic::default())));
    let endpoint = mqtt_endpoint(key, transport.clone()).await?;
    transport.wait_ready().await?;
    endpoint.close().await;
    drop(endpoint);
    tokio::time::timeout(Duration::from_secs(2), transport.wait_closed()).await??;
    println!("CANCELLED");
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
    endpoint
        .accept(
            PROBE_ALPN,
            ProbeEcho {
                done: done_tx,
                expected_peer: None,
            },
        )
        .await?;
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
    match args.as_slice() {
        [_, network, action, source, target] if network == "network" && action == "backup" => {
            NativeNetwork::open(source)
                .await?
                .create_backup(target)
                .await?;
            println!("SAVED backup");
            return Ok(());
        }
        [_, network, action, source, target] if network == "network" && action == "restore" => {
            let node = NativeNetwork::restore_backup(source, target).await?;
            print_network_status(&node.status());
            println!("PRECODE_COUNTS_UNTRUSTED old codes stopped for new redemption");
            node.close().await;
            return Ok(());
        }
        [_, network, action, data_dir, invitation] if network == "network" && action == "join" => {
            let node = desktop_cli::join(data_dir, invitation).await?;
            print_network_status(&node.status());
            node.close().await;
            return Ok(());
        }
        [_, network, action, data_dir, bind @ ..]
            if network == "network" && action == "session" =>
        {
            return desktop_cli::session(data_dir, optional_bind_ip(bind)?).await;
        }
        [_, network, action, data_dir, bind @ ..]
            if network == "network" && action == "maintain" =>
        {
            return lifecycle_cli::run(data_dir, optional_bind_ip(bind)?).await;
        }
        [_, network, action, data_dir] if network == "network" && action == "create" => {
            let node = NativeNetwork::create(data_dir).await?;
            print_network_status(&node.status());
            node.close().await;
            return Ok(());
        }
        [_, network, action, data_dir] if network == "network" && action == "status" => {
            let node = NativeNetwork::open(data_dir).await?;
            print_network_status(&node.status());
            node.close().await;
            return Ok(());
        }
        [_, network, action, data_dir] if network == "network" && action == "resources" => {
            let node = NativeNetwork::open(data_dir).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&node.resource_configuration()?)?
            );
            node.close().await;
            return Ok(());
        }
        [_, network, action, data_dir, json_file]
            if network == "network"
                && (action == "resources-import" || action == "resources-update") =>
        {
            let node = NativeNetwork::open(data_dir).await?;
            let mut config = node.resource_configuration()?;
            let bytes = std::fs::read(json_file)?;
            if action == "resources-import" {
                config.import_trusted_replacement(serde_json::from_slice(&bytes)?)?;
            } else {
                config.apply_signed_update(&serde_json::from_slice(&bytes)?)?;
            }
            node.save_resource_configuration(&config)?;
            println!("SAVED resources; effective on next endpoint start");
            node.close().await;
            return Ok(());
        }
        [_, network, action, data_dir, bind @ ..] if network == "network" && action == "run" => {
            let bind_ip = optional_bind_ip(bind)?;
            let node = NativeNetwork::open(data_dir).await?;
            let resources = node.bind_configured_endpoint(bind_ip).await?;
            let endpoint = resources.endpoint();
            let address = local_udp_addr(endpoint).ok().or_else(|| {
                resources.addr().addrs.iter().find_map(|addr| match addr {
                    iroh::TransportAddr::Custom(addr) => TcpTransport::socket_addr(addr).ok(),
                    _ => None,
                })
            });
            println!(
                "READY {} {}",
                node.status().node_id,
                address
                    .map(|addr| addr.to_string())
                    .unwrap_or_else(|| "no-direct-address".into())
            );
            tokio::signal::ctrl_c().await?;
            resources.close().await;
            node.close().await;
            return Ok(());
        }
        _ => {}
    }
    if let [_, action, data_dir] = args.as_slice() {
        if action == "identity" {
            println!(
                "{}",
                iroh_secret(&signing_key(Path::new(data_dir))?).public()
            );
            return Ok(());
        }
    }
    match args.as_slice() {
        [_, action, mode, data_dir, broker] if action == "cancel" && mode == "mqtt" => {
            return cancel_mqtt(&signing_key(Path::new(data_dir))?, broker.parse()?).await;
        }
        [_, action, mode, data_dir, broker, expected] if action == "serve" && mode == "mqtt" => {
            return serve_mqtt(
                &signing_key(Path::new(data_dir))?,
                broker.parse()?,
                expected.parse()?,
            )
            .await;
        }
        [_, action, mode, data_dir, target, route, broker]
            if action == "send" && mode == "mqtt" =>
        {
            return send_mqtt(
                &signing_key(Path::new(data_dir))?,
                target.parse()?,
                route,
                broker.parse()?,
            )
            .await;
        }
        _ => {}
    }
    if !matches!(
        args.get(2).map(String::as_str),
        Some("raw" | "wrapped" | "tcp")
    ) {
        return Err(USAGE.into());
    }
    match args.as_slice() {
        [_, action, mode, data_dir, bind @ ..] if action == "serve" => {
            let bind_ip = optional_bind_ip(bind)?;
            let key = signing_key(Path::new(data_dir))?;
            match mode.as_str() {
                "raw" => serve_raw(&key, bind_ip).await,
                "wrapped" => serve_wrapped(&key, bind_ip).await,
                "tcp" => serve_tcp(&key, bind_ip).await,
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
                "tcp" => send_tcp(&key, target, addr, bind_ip).await,
                _ => Err(USAGE.into()),
            }
        }
        _ => Err(USAGE.into()),
    }
}
