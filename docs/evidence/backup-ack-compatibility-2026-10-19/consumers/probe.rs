//! Fixed public Noq consumer: actual UDP/TLS, independently controlled carriers.
use std::{error::Error, net::SocketAddr, sync::{Arc, atomic::{AtomicU64, Ordering}}, time::{Duration, Instant}};
use noq::{ClientConfig, Endpoint, FourTuple, PathId, PathStatus, ServerConfig, TransportConfig};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use tokio::{net::UdpSocket, sync::watch, task::{JoinHandle, JoinSet}, time::{sleep, timeout}};
type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

struct Carrier {
    addr: SocketAddr,
    delay: Arc<AtomicU64>,
    drop: Arc<AtomicU64>,
    counts: Arc<[AtomicU64; 4]>,
    stop: watch::Sender<bool>,
    task: JoinHandle<Result<()>>,
}
impl Carrier {
    async fn start(server: SocketAddr, client: SocketAddr) -> Result<Self> {
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await?);
        let addr = socket.local_addr()?;
        let delay = Arc::new(AtomicU64::new(0));
        let drop = Arc::new(AtomicU64::new(0));
        let counts = Arc::new(std::array::from_fn(|_| AtomicU64::new(0)));
        let (stop, mut ending) = watch::channel(false);
        let (d, loss, c) = (delay.clone(), drop.clone(), counts.clone());
        let task = tokio::spawn(async move {
            let mut pending = JoinSet::new();
            let mut buf = [0u8; 65536];
            loop {
                tokio::select! {
                    changed = ending.changed() => { changed?; if *ending.borrow() { break; } }
                    result = socket.recv_from(&mut buf) => {
                        let (n, source) = result?;
                        let from_server = source == server;
                        assert!(from_server || source == client, "unexpected UDP sender");
                        let direction = usize::from(from_server);
                        c[direction].fetch_add(1, Ordering::SeqCst);
                        if loss.load(Ordering::SeqCst) != 0 {
                            c[direction+2].fetch_add(1, Ordering::SeqCst); continue;
                        }
                        let destination = if from_server { client } else { server };
                        let millis = if from_server { d.load(Ordering::SeqCst) } else { 0 };
                        if millis == 0 { socket.send_to(&buf[..n], destination).await?; }
                        else {
                            let socket = socket.clone(); let packet = buf[..n].to_vec();
                            pending.spawn(async move { sleep(Duration::from_millis(millis)).await; socket.send_to(&packet, destination).await });
                        }
                    }
                    result = pending.join_next(), if !pending.is_empty() => { result.unwrap()??; }
                }
            }
            while let Some(result) = pending.join_next().await { result??; }
            Ok(())
        });
        Ok(Self {addr, delay, drop, counts, stop, task})
    }
    async fn shutdown(self) -> Result<()> {
        self.stop.send(true)?; self.task.await??;
        let rebound = UdpSocket::bind(self.addr).await?; drop(rebound); Ok(())
    }
}

async fn exchange(client: &noq::Connection, server: &noq::Connection, label: &str) -> Result<u128> {
    let payload = format!("ordinary-noq-{label}-").repeat(256).into_bytes();
    let expected = payload.clone();
    let started = Instant::now();
    let send = async {
        let mut stream = client.open_uni().await?;
        stream.write_all(&payload).await?; stream.finish()?;
        assert_eq!(timeout(Duration::from_secs(4), stream.stopped()).await??, None);
        Result::Ok(())
    };
    let recv = async {
        let mut stream = server.accept_uni().await?;
        assert_eq!(stream.read_to_end(65536).await?, expected);
        Result::Ok(())
    };
    let (a,b) = timeout(Duration::from_secs(6), async { tokio::join!(send, recv) }).await?;
    a?; b?; Ok(started.elapsed().as_millis())
}

async fn scenario(mode: &str) -> Result<()> {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()])?;
    let der = CertificateDer::from(cert.cert);
    let key = PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der());
    let mut transport = TransportConfig::default();
    transport.max_concurrent_multipath_paths(4);
    transport.max_idle_timeout(Some(Duration::from_secs(5).try_into()?));
    let transport = Arc::new(transport);
    let mut server_config = ServerConfig::with_single_cert(vec![der.clone()], key.into())?;
    server_config.transport = transport.clone();
    let server_endpoint = Endpoint::server(server_config, "127.0.0.1:0".parse()?)?;
    let server_addr = server_endpoint.local_addr()?;
    let mut roots = rustls::RootCertStore::empty(); roots.add(der)?;
    let mut client_config = ClientConfig::with_root_certificates(Arc::new(roots))?;
    client_config.transport_config(transport);
    let mut client_endpoint = Endpoint::client("127.0.0.1:0".parse()?)?;
    client_endpoint.set_default_client_config(client_config);
    let client_addr = client_endpoint.local_addr()?;
    let a = Carrier::start(server_addr, client_addr).await?;
    let b = Carrier::start(server_addr, client_addr).await?;
    let connections = timeout(Duration::from_secs(3), async {
        tokio::join!(async {client_endpoint.connect(a.addr, "localhost").unwrap().await},
            async {server_endpoint.accept().await.unwrap().await})
    }).await?;
    let (client, server) = (connections.0?, connections.1?);
    let stable = client.stable_id();
    let base_ms = exchange(&client, &server, "baseline").await?;
    let mut opened = None;
    if mode != "single" {
        let path = timeout(Duration::from_secs(2), async {
            loop {
                match client.open_path(FourTuple::from_remote(b.addr), PathStatus::Available).await {
                    Ok(path) => break Ok::<_, noq::PathError>(path),
                    Err(noq::PathError::RemoteCidsExhausted) => sleep(Duration::from_millis(20)).await,
                    Err(error) => break Err(error),
                }
            }
        }).await??;
        let id = path.id();
        let remote = timeout(Duration::from_secs(2), async {
            loop { if let Some(path) = server.path(id) { break path; } sleep(Duration::from_millis(5)).await; }
        }).await?;
        // Ordinary users may assign opposite local priorities to the same paths.
        // Client's only Available data carrier is B; server prefers A for replies.
        client.path(PathId::ZERO).unwrap().set_status(PathStatus::Backup)?;
        remote.set_status(PathStatus::Backup)?;
        sleep(Duration::from_millis(120)).await;
        opened = Some((path, remote));
    }
    if mode == "slow_backup" { b.delay.store(250, Ordering::SeqCst); }
    if mode == "blackhole_available" || mode == "recovery" { a.drop.store(1, Ordering::SeqCst); }
    if let Some((local_path, remote_path)) = opened.as_ref() {
        let started = Instant::now();
        let local = local_path.start_probe()?;
        let remote = remote_path.start_probe()?;
        timeout(Duration::from_secs(2), async {
            loop {
                if local.is_acknowledged()? && remote.is_acknowledged()? {break Result::Ok(());}
                sleep(Duration::from_millis(5)).await;
            }
        }).await??;
        println!("{{\"case\":\"{mode}\",\"stage\":\"explicitProbe\",\"bothFreshAuthenticated\":true,\"elapsedMs\":{}}}",started.elapsed().as_millis());
    }
    let mut queued_server_stream = None;
    if mode == "blackhole_available" || mode == "recovery" {
        let mut stream = server.open_uni().await?;
        stream.write_all(b"must stay on Available data carrier").await?;
        stream.finish()?;
        queued_server_stream = Some(stream);
    }
    let before_a = server.path(PathId::ZERO).unwrap().stats();
    let before_b = opened.as_ref().map(|(_,p)| p.stats());
    let result = exchange(&client, &server, mode).await;
    let after_a = server.path(PathId::ZERO).unwrap().stats();
    let after_b = opened.as_ref().map(|(_,p)| p.stats());
    println!("{{\"case\":\"{mode}\",\"stage\":\"business\",\"baselineMs\":{base_ms},\"success\":{},\"elapsedMs\":{},\"ackCarrierA\":{},\"ackCarrierB\":{},\"aPackets\":{},\"bPackets\":{},\"aDropped\":{},\"lostA\":{},\"lostB\":{},\"sameConnection\":{}}}",
        result.is_ok(), result.as_ref().copied().unwrap_or(4000),
        after_a.frame_tx.acks+after_a.frame_tx.path_acks-before_a.frame_tx.acks-before_a.frame_tx.path_acks,
        after_b.zip(before_b).map(|(a,b)|a.frame_tx.acks+a.frame_tx.path_acks-b.frame_tx.acks-b.frame_tx.path_acks).unwrap_or(0),
        a.counts[1].load(Ordering::SeqCst), b.counts[1].load(Ordering::SeqCst), a.counts[3].load(Ordering::SeqCst),
        after_a.lost_packets, after_b.map(|p|p.lost_packets).unwrap_or(0), client.stable_id()==stable);
    if queued_server_stream.is_some() {
        assert_eq!(after_b.unwrap().frame_tx.stream, 0, "probe ACK must never enable Backup ordinary STREAM");
        assert!(timeout(Duration::from_millis(20), client.accept_uni()).await.is_err());
        println!("{{\"case\":\"{mode}\",\"stage\":\"backupPermission\",\"queuedStreamStayedOffBackup\":true}}");
    }
    if mode == "recovery" {
        a.drop.store(0, Ordering::SeqCst);
        let recovered = exchange(&client, &server, "recovered").await?;
        println!("{{\"case\":\"recovery\",\"stage\":\"restored\",\"elapsedMs\":{recovered},\"sameConnection\":{}}}", client.stable_id()==stable);
    }
    // Keep forwarders alive throughout actual QUIC close/drain, then join all tasks.
    b.delay.store(0, Ordering::SeqCst);
    let close_at = Instant::now();
    client.close(0u32.into(), b"ordinary complete");
    let closed = timeout(Duration::from_secs(2), server.closed()).await;
    println!("{{\"case\":\"{mode}\",\"stage\":\"close\",\"observed\":{},\"elapsedMs\":{}}}", closed.is_ok(), close_at.elapsed().as_millis());
    server.close(0u32.into(), b"ordinary complete");
    client_endpoint.close(0u32.into(), b"ordinary complete");
    server_endpoint.close(0u32.into(), b"ordinary complete");
    timeout(Duration::from_secs(4), async {tokio::join!(client_endpoint.wait_idle(), server_endpoint.wait_idle());}).await?;
    drop(queued_server_stream); drop(opened); drop(client); drop(server); drop(client_endpoint); drop(server_endpoint);
    a.shutdown().await?; b.shutdown().await?;
    for addr in [client_addr,server_addr] { let socket=UdpSocket::bind(addr).await?; drop(socket); }
    println!("{{\"case\":\"{mode}\",\"stage\":\"cleanup\",\"allActorsJoinedAndPortsRebound\":true}}");
    if mode == "single" || mode == "healthy" || mode == "slow_backup" {result?;}
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    for mode in std::env::args().skip(1) {scenario(&mode).await?;}
    Ok(())
}
