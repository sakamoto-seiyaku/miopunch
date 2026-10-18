//! Small native QAD probe for isolated kernel-NAT experiments.

use std::{
    error::Error,
    net::{Ipv4Addr, SocketAddr},
    path::Path,
    sync::Arc,
    time::Duration,
};

use iroh::{
    Endpoint, RelayConfig, RelayMap, Watcher as _,
    endpoint::{UdpObservation, presets},
};
use iroh_relay::{RelayQuicConfig, server, tls::CaTlsConfig};

fn usage() -> &'static str {
    "usage: udp-observation-probe serve CERT_DER_PATH [BIND_ADDR] | observe CERT_DER_PATH OBSERVER_HOST OBSERVER_QUIC_PORT [BIND_IP]"
}

async fn serve(cert_path: &Path, bind_addr: SocketAddr) -> Result<(), Box<dyn Error>> {
    let cert = rcgen::generate_simple_self_signed(vec![
        "localhost".into(),
        "qad.miopunch.test".into(),
        "qad2.miopunch.test".into(),
        "qad3.miopunch.test".into(),
        "qad4.miopunch.test".into(),
    ])?;
    std::fs::write(cert_path, cert.cert.der().as_ref())?;
    let key = rustls::pki_types::PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der());
    let tls = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_no_client_auth()
    .with_single_cert(vec![cert.cert.der().clone()], key.into())?;
    let mut bind_addrs = vec![bind_addr];
    if let Ok(extra) = std::env::var("MIOPUNCH_QAD_EXTRA_PORTS") {
        for port in extra.split(',') {
            bind_addrs.push(SocketAddr::new(bind_addr.ip(), port.parse()?));
        }
    }
    let mut servers = Vec::new();
    for addr in bind_addrs {
        let mut quic = server::QuicConfig::new(addr);
        quic.server_config = Some(tls.clone());
        let mut config = server::ServerConfig::default();
        config.quic = Some(quic);
        let server = server::Server::spawn(config).await?;
        println!("QAD_READY {}", server.quic_addr().ok_or("no QAD socket")?);
        servers.push(server);
    }
    tokio::signal::ctrl_c().await?;
    Ok(())
}

async fn observe(
    cert_path: &Path,
    host: &str,
    port: u16,
    bind_ip: Ipv4Addr,
) -> Result<(), Box<dyn Error>> {
    let cert = std::fs::read(cert_path)?;
    let roots = CaTlsConfig::custom_roots(vec![cert.into()]);
    let url = format!("https://{host}/").parse()?;
    let servers = RelayMap::from(RelayConfig::new(url, Some(RelayQuicConfig::new(port))));
    let endpoint = Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .clear_ip_transports()
        .bind_addr((bind_ip, 0))?
        .observation_servers(servers)
        .ca_tls_config(roots)
        .bind()
        .await?;
    println!("BOUND {:?}", endpoint.bound_sockets());
    let mut watcher = endpoint.udp_observation();
    let state = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let state = watcher.get();
            if !matches!(state, UdpObservation::Pending) {
                return state;
            }
            watcher.updated().await.expect("endpoint watcher closed");
        }
    })
    .await?;
    println!("OBSERVATION {state:?}");
    println!("DIRECT_ADDR {:?}", endpoint.addr());
    println!("RELAY_STATUS {:?}", endpoint.home_relay_status().get());
    endpoint.close().await;
    if state.is_fresh() {
        Ok(())
    } else {
        Err("no fresh observed UDP address".into())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.as_slice() {
        [mode, cert] if mode == "serve" => {
            serve(Path::new(cert), (Ipv4Addr::UNSPECIFIED, 0).into()).await
        }
        [mode, cert, addr] if mode == "serve" => serve(Path::new(cert), addr.parse()?).await,
        [mode, cert, host, port] if mode == "observe" => {
            observe(Path::new(cert), host, port.parse()?, Ipv4Addr::UNSPECIFIED).await
        }
        [mode, cert, host, port, ip] if mode == "observe" => {
            observe(Path::new(cert), host, port.parse()?, ip.parse()?).await
        }
        _ => Err(usage().into()),
    }
}
