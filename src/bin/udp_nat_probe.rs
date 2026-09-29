//! MIOPUNCH-27: cross-process MQTT bootstrap and real dual-NAT UDP path probe.
use std::{
    error::Error,
    io,
    net::{Ipv4Addr, SocketAddr},
    path::Path,
    sync::Arc,
    time::Duration,
};

use iroh::{
    Endpoint, EndpointAddr, RelayConfig, RelayMap, Watcher as _,
    endpoint::{UdpObservation, presets},
};
use iroh_base::{PublicKey, SecretKey, TransportAddr};
use iroh_relay::{RelayQuicConfig, tls::CaTlsConfig};
use miopunch_native_probe::{
    control_session::{CommitError, ControlSession},
    mqtt_transport::{MqttTransport, Traffic},
};

const ALPN: &[u8] = b"miopunch/nat-probe/1";
const WAIT: Duration = Duration::from_secs(12);

struct ObservedCandidate {
    addr: SocketAddr,
    observation: UdpObservation,
}

impl ObservedCandidate {
    fn fresh_addr(&self) -> Result<SocketAddr, Box<dyn Error>> {
        if self.observation.is_fresh() {
            Ok(self.addr)
        } else {
            Err("observed UDP candidate is stale".into())
        }
    }
}

fn load_or_create_key(path: &Path) -> io::Result<SecretKey> {
    if !path.exists() {
        let key = SecretKey::generate();
        std::fs::write(path, key.to_bytes())?;
    }
    let bytes: [u8; 32] = std::fs::read(path)?
        .try_into()
        .map_err(|_| io::Error::other("bad key length"))?;
    Ok(SecretKey::from_bytes(&bytes))
}

async fn endpoint(
    key: SecretKey,
    broker: SocketAddr,
    observer: &str,
    observer_port: u16,
    cert: &Path,
) -> Result<(Endpoint, Arc<MqttTransport>, Arc<Traffic>), Box<dyn Error>> {
    let traffic = Arc::new(Traffic::default());
    let carrier = Arc::new(MqttTransport::new(broker, traffic.clone()));
    let roots = CaTlsConfig::custom_roots(vec![std::fs::read(cert)?.into()]);
    let url = format!("https://{observer}/").parse()?;
    let qad = RelayMap::from(RelayConfig::new(
        url,
        Some(RelayQuicConfig::new(observer_port)),
    ));
    let bind_port: u16 = std::env::var("MIOPUNCH_NAT_BIND_PORT")
        .unwrap_or_else(|_| "0".to_string())
        .parse()?;
    let endpoint = Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .alpns(vec![ALPN.to_vec()])
        .control_only_alpns(vec![ALPN.to_vec()])
        .bind_addr((Ipv4Addr::UNSPECIFIED, bind_port))?
        .clear_relay_transports()
        .observation_servers(qad)
        .ca_tls_config(roots)
        .add_custom_transport(carrier.clone())
        .bind()
        .await?;
    carrier.wait_ready().await?;
    Ok((endpoint, carrier, traffic))
}

async fn observed_addr(endpoint: &Endpoint) -> Result<ObservedCandidate, Box<dyn Error>> {
    let mut watcher = endpoint.udp_observation();
    let state = tokio::time::timeout(WAIT, async {
        loop {
            let state = watcher.get();
            if !matches!(state, UdpObservation::Pending) {
                return state;
            }
            watcher.updated().await.expect("observation watcher closed");
        }
    })
    .await?;
    let UdpObservation::Fresh { addresses, .. } = &state else {
        return Err(format!("no observed candidate: {state:?}").into());
    };
    if !state.is_fresh() {
        return Err("observation expired before exchange".into());
    }
    let addr = addresses
        .iter()
        .copied()
        .find(SocketAddr::is_ipv4)
        .ok_or("no IPv4 observed candidate")?;
    if !endpoint
        .addr()
        .ip_addrs()
        .any(|candidate| *candidate == addr)
    {
        return Err("observed candidate absent from endpoint address".into());
    }
    println!(
        "CANDIDATE id={} sockets={:?} observed={addr} mapping_varies={:?}",
        endpoint.id(),
        endpoint.bound_sockets(),
        match state {
            UdpObservation::Fresh {
                mapping_varies_by_dest,
                ..
            } => mapping_varies_by_dest,
            _ => unreachable!(),
        }
    );
    Ok(ObservedCandidate {
        addr,
        observation: state,
    })
}

async fn wait_ip(session: &ControlSession) -> Result<(), Box<dyn Error>> {
    tokio::time::timeout(WAIT, async {
        while !session.has_data_path() {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await?;
    Ok(())
}

async fn read_candidate(
    recv: &mut iroh::endpoint::RecvStream,
) -> Result<SocketAddr, Box<dyn Error>> {
    let bytes = tokio::time::timeout(WAIT, recv.read_to_end(128)).await??;
    Ok(std::str::from_utf8(&bytes)?.parse()?)
}

async fn run_server(
    endpoint: Endpoint,
    carrier: Arc<MqttTransport>,
    traffic: Arc<Traffic>,
    peer: PublicKey,
    own_candidate: ObservedCandidate,
) -> Result<(), Box<dyn Error>> {
    println!("READY id={} route={}", endpoint.id(), carrier.route()?);
    let control = tokio::time::timeout(WAIT, ControlSession::accept(&endpoint)).await??;
    if control.remote_id() != peer {
        return Err("unexpected authenticated peer".into());
    }
    let stable = control.stable_id();
    let (mut reply, mut request) =
        match tokio::time::timeout(WAIT, control.accept_control_bi()).await {
            Ok(Ok(streams)) => streams,
            other => {
                println!(
                    "CONTROL_UNAVAILABLE reason={other:?} mqtt={}",
                    traffic.report()
                );
                return Ok(());
            }
        };
    let remote = match read_candidate(&mut request).await {
        Ok(remote) => remote,
        Err(error) => {
            println!(
                "CONTROL_UNAVAILABLE reason={error} mqtt={}",
                traffic.report()
            );
            return Ok(());
        }
    };
    if std::env::var("MIOPUNCH_NAT_SKIP_PRIME").as_deref() == Ok("1") {
        println!("PRIME_SKIPPED peer_candidate={remote}");
    } else {
        control.prime_udp_path(remote).await?;
    }
    let own_addr = own_candidate.fresh_addr()?;
    reply.write_all(own_addr.to_string().as_bytes()).await?;
    reply.finish()?;
    println!("EXCHANGED peer={peer} peer_candidate={remote} stable_id={stable}");
    if let Err(error) = wait_ip(&control).await {
        println!("PATH_UNAVAILABLE reason={error} mqtt={}", traffic.report());
        return Ok(());
    }
    let data = control
        .commit_data_path()
        .await
        .map_err(|error| match error {
            CommitError::Retryable { error, .. } | CommitError::Terminal { error } => error,
        })?;
    let before = traffic.sent_packets();
    let (_, mut recv) = tokio::time::timeout(WAIT, data.connection().accept_bi()).await??;
    let business = tokio::time::timeout(WAIT, recv.read_to_end(64)).await??;
    if business != b"NAT_DATA" || traffic.sent_packets() != before {
        return Err("business crossed MQTT or had wrong content".into());
    }
    println!(
        "DATA_OK stable_id={stable} peer={peer} paths={:?} mqtt_after_commit={before}",
        data.connection().paths()
    );
    endpoint.close().await;
    Ok(())
}

async fn run_client(
    endpoint: Endpoint,
    traffic: Arc<Traffic>,
    peer: PublicKey,
    route: &str,
    own_candidate: ObservedCandidate,
    override_candidate: Option<SocketAddr>,
) -> Result<(), Box<dyn Error>> {
    let target =
        EndpointAddr::new(peer).with_addrs([TransportAddr::Custom(MqttTransport::address(route)?)]);
    let mut control =
        tokio::time::timeout(WAIT, ControlSession::connect(&endpoint, target, ALPN)).await??;
    if control.remote_id() != peer {
        return Err("wrong authenticated peer".into());
    }
    let stable = control.stable_id();
    control.queue_business(b"NAT_DATA".to_vec())?;
    if std::env::var("MIOPUNCH_NAT_FAULT").as_deref() == Ok("stale") {
        endpoint.network_change().await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while own_candidate.observation.is_fresh() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        println!("STALE_CANDIDATE rejected_before_exchange stable_id={stable}");
        endpoint.close().await;
        return Ok(());
    }
    if std::env::var("MIOPUNCH_NAT_FAULT").as_deref() == Ok("control_cut") {
        println!("CONTROL_READY stable_id={stable}");
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let own_addr = own_candidate.fresh_addr()?;
    let reported = match tokio::time::timeout(WAIT, async {
        let (mut send, mut recv) = control.open_control_bi().await?;
        send.write_all(own_addr.to_string().as_bytes()).await?;
        send.finish()?;
        read_candidate(&mut recv).await
    })
    .await
    {
        Ok(Ok(addr)) => addr,
        other => {
            let error = format!("{other:?}");
            println!(
                "CONTROL_UNAVAILABLE reason={error} mqtt={}",
                traffic.report()
            );
            endpoint.close().await;
            return Ok(());
        }
    };
    let remote = override_candidate.unwrap_or(reported);
    println!(
        "EXCHANGED peer={peer} peer_candidate={reported} attempted={remote} stable_id={stable}"
    );
    control.prime_udp_path(remote).await?;
    if std::env::var("MIOPUNCH_NAT_FAULT").as_deref() == Ok("path_id_retry") {
        #[cfg(feature = "nat-lab-test-hooks")]
        control.test_force_path_id_retry_once().await?;
        #[cfg(not(feature = "nat-lab-test-hooks"))]
        return Err("path_id_retry requires the nat-lab-test-hooks feature".into());
    }
    let result = tokio::time::timeout(WAIT, control.open_data_path_report(remote)).await;
    match result {
        Ok(Ok(report)) => {
            println!("PATH_OPEN path_id_retries={}", report.path_id_retries);
        }
        other => {
            println!(
                "PATH_UNAVAILABLE reason={other:?} mqtt={}",
                traffic.report()
            );
            endpoint.close().await;
            return Ok(());
        }
    }
    if let Err(error) = wait_ip(&control).await {
        println!("PATH_UNAVAILABLE reason={error} mqtt={}", traffic.report());
        endpoint.close().await;
        return Ok(());
    }
    match control.commit_data_path().await {
        Ok(mut data) => {
            let queued = data
                .take_pending_business()
                .pop_front()
                .ok_or("missing queued business")?;
            let before = traffic.sent_packets();
            let (mut send, _) = data.connection().open_bi().await?;
            send.write_all(&queued).await?;
            send.finish()?;
            tokio::time::sleep(Duration::from_millis(250)).await;
            if traffic.sent_packets() != before {
                return Err("business used MQTT".into());
            }
            println!(
                "DATA_OK stable_id={stable} peer={peer} paths={:?} mqtt_after_commit={before}",
                data.connection().paths()
            );
            endpoint.close().await;
            return Ok(());
        }
        Err(CommitError::Retryable { error, .. }) => {
            println!("PATH_UNAVAILABLE reason={error} mqtt={}", traffic.report());
        }
        Err(CommitError::Terminal { error }) => return Err(error.into()),
    }
    endpoint.close().await;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if let [mode, path] = args.as_slice() {
        if mode == "identity" {
            println!("{}", load_or_create_key(Path::new(path))?.public());
            return Ok(());
        }
    }
    let [
        mode,
        key_path,
        cert_path,
        broker,
        observer,
        port,
        peer,
        tail @ ..,
    ] = args.as_slice()
    else {
        return Err("usage: udp_nat_probe identity KEY | server|client KEY CERT BROKER OBSERVER_HOST OBSERVER_PORT PEER_ID [ROUTE [PEER_UDP_OVERRIDE]]".into());
    };
    let (endpoint, carrier, traffic) = endpoint(
        load_or_create_key(Path::new(key_path))?,
        broker.parse()?,
        observer,
        port.parse()?,
        Path::new(cert_path),
    )
    .await?;
    let own_candidate = observed_addr(&endpoint).await?;
    let peer = peer.parse()?;
    match (mode.as_str(), tail) {
        ("server", []) => run_server(endpoint, carrier, traffic, peer, own_candidate).await,
        ("client", [route]) => {
            run_client(endpoint, traffic, peer, route, own_candidate, None).await
        }
        ("client", [route, override_addr]) => {
            run_client(
                endpoint,
                traffic,
                peer,
                route,
                own_candidate,
                Some(override_addr.parse()?),
            )
            .await
        }
        _ => Err("invalid mode arguments".into()),
    }
}
