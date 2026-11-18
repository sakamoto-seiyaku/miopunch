//! MIOPUNCH-138: explicitly selected relay experiments through admitted Native SDK nodes.
//! No default routing policy is changed. This test executable is also the two-device actor.
#![cfg(feature = "nat-lab-test-hooks")]

use iroh::{Endpoint, EndpointAddr, RelayUrl, TransportAddr, endpoint::Connection};
use miopunch_native_probe::{
    admission_gate::{ADMISSION_REQUEST_ALPN, ProtocolScope},
    connection_coordinator::{AttemptContext, ConnectError, ConnectionCoordinator, SessionPurpose},
    manual_admission::ManualExchange,
    native_network::NativeNetwork,
    resources::{Resource, ResourceUse},
};
use n0_watcher::Watcher;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    error::Error,
    net::Ipv4Addr,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

type Failure = Box<dyn Error + Send + Sync>;
const REQUEST_BUDGETS: [u64; 2] = [3_000, 10_000];

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ActorMode {
    Prepare,
    Serve,
    Connect,
}

#[derive(Clone, Serialize, Deserialize)]
struct Actor {
    mode: ActorMode,
    directory: PathBuf,
    target_file: PathBuf,
    relays: Vec<RelayUrl>,
    home: Option<RelayUrl>,
    relay_only: bool,
    early_ms: Option<u64>,
    connect_ms: u64,
    lifetime_ms: u64,
    #[serde(default)]
    bind_port: u16,
}

fn emit(value: Value) {
    println!("{value}");
}
fn wall_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}
fn home(endpoint: &iroh::endpoint::RestrictedEndpoint) -> Value {
    json!(endpoint.home_relay_status().get().iter().map(|s| json!({"url":s.url().to_string(), "connected":s.is_connected(), "error":s.last_error().map(|error| format!("{error:?}"))})).collect::<Vec<_>>())
}

async fn prepare(directory: &Path) -> Result<(), Failure> {
    let mut a = NativeNetwork::create(directory.join("server")).await?;
    let mut b = NativeNetwork::join(directory.join("client"), a.manual_invitation()?).await?;
    for node in [&a, &b] {
        let mut config = node.resource_configuration()?;
        config.replacement = Some(vec![]);
        node.save_resource_configuration(&config)?;
    }
    let ae = a.bind_endpoint(Ipv4Addr::LOCALHOST).await?;
    let be = b.bind_endpoint(Ipv4Addr::LOCALHOST).await?;
    let deadline = n0_future::time::Instant::now() + Duration::from_secs(5);
    let (outgoing, incoming) = tokio::join!(
        async {
            be.connect_control_with_opts(
                ae.addr(),
                ADMISSION_REQUEST_ALPN,
                iroh::endpoint::ConnectOptions::new().with_caller_deadline(deadline),
            )
            .await?
            .await
            .map_err(Failure::from)
        },
        async {
            ae.accept()
                .await
                .ok_or("admission stopped")?
                .with_caller_deadline(deadline)
                .accept_control()?
                .await
                .map_err(Failure::from)
        }
    );
    let (outgoing, incoming) = (Arc::new(outgoing?), Arc::new(incoming?));
    let (request, served) = tokio::join!(
        b.request_manual_admission(&outgoing),
        a.serve_manual_admission(&incoming)
    );
    assert!(matches!(request?, ManualExchange::AwaitingApproval));
    served?;
    let request = a
        .manual_requests()
        .await?
        .pop()
        .ok_or("missing admission request")?;
    a.approve_manual_admission(&request).await?;
    let receive = async {
        let ManualExchange::Materials(material) = b.request_manual_admission(&outgoing).await?
        else {
            return Err(Failure::from("missing approved materials"));
        };
        b.complete_manual_admission(material).await?;
        Ok::<_, Failure>(())
    };
    let (received, served) = tokio::join!(receive, a.serve_manual_admission(&incoming));
    received?;
    served?;
    for (node, peer, side) in [
        (&a, b.status().node_id, "server"),
        (&b, a.status().node_id, "client"),
    ] {
        std::fs::write(directory.join(side).join("expected-peer"), peer.to_string())?;
        assert_ne!(node.status().node_id, peer);
    }
    outgoing.close(0u32.into(), b"isolated fixture provisioned");
    incoming.close(0u32.into(), b"isolated fixture provisioned");
    tokio::join!(ae.close(), be.close());
    tokio::join!(a.close(), b.close());
    emit(json!({"event":"mio138-prepared", "independentIdentities":true, "manualAdmission":true}));
    Ok(())
}

async fn request(
    connection: &Connection,
    expected: iroh::EndpointId,
    sequence: u64,
    budget: u64,
) -> Value {
    let started = Instant::now();
    let result = tokio::time::timeout(Duration::from_millis(budget), async {
        let (mut send, mut recv) = connection.open_bi().await?;
        send.write_all(&sequence.to_be_bytes()).await?;
        send.finish()?;
        let bytes = recv.read_to_end(128).await?;
        let reply: Value = serde_json::from_slice(&bytes)?;
        if reply["identity"] != expected.to_string() || reply["sequence"] != sequence {
            return Err(Failure::from("reply identity or sequence mismatch"));
        }
        Ok::<_, Failure>(())
    })
    .await;
    json!({"event":"mio138-request", "at":wall_ms(), "sequence":sequence, "budgetMs":budget,
        "elapsedMs":started.elapsed().as_millis(), "matched":matches!(&result, Ok(Ok(()))),
        "error":match result { Ok(Ok(()))=>None, Ok(Err(e))=>Some(e.to_string()), Err(e)=>Some(e.to_string()) },
        "stableId":connection.stable_id(), "paths":connection.paths().iter().map(|p| json!({"address":p.remote_addr().to_string(),"selected":p.is_selected(),"relay":p.is_relay()})).collect::<Vec<_>>()})
}

async fn echo(connection: Connection, identity: String) -> Result<(), Failure> {
    let mut streams = JoinSet::new();
    loop {
        tokio::select! {
            incoming = connection.accept_bi() => match incoming {
                Ok((mut send, mut recv)) => { let identity = identity.clone(); streams.spawn(async move {
                    let value = recv.read_to_end(8).await?;
                    let sequence = u64::from_be_bytes(value.try_into().map_err(|_| "invalid sequence")?);
                    send.write_all(&serde_json::to_vec(&json!({"identity":identity,"sequence":sequence}))?).await?;
                    send.finish()?; Ok::<_, Failure>(())
                }); },
                Err(_) => break,
            },
            Some(_) = streams.join_next(), if !streams.is_empty() => {},
        }
    }
    streams.abort_all();
    while streams.join_next().await.is_some() {}
    Ok(())
}

// A bounded experimental schedule. Each candidate uses the real coordinator's
// single-route admission/path contract. All child attempts share one absolute
// deadline; cancellation settles losers. This actor has no competing peer work.
async fn dial(
    coordinator: &ConnectionCoordinator,
    target: EndpointAddr,
    attempt: AttemptContext,
    early: Option<u64>,
) -> Result<Connection, ConnectError> {
    let Some(delay) = early else {
        return coordinator
            .connect(target, SessionPurpose::MemberBusiness, attempt)
            .await;
    };
    let mut pending = JoinSet::new();
    let started = Instant::now();
    let mut tokens = Vec::new();
    for (index, route) in target.addrs.into_iter().enumerate() {
        let coordinator = coordinator.clone();
        let id = target.id;
        let token = attempt.cancellation.child_token();
        tokens.push(token.clone());
        let child = AttemptContext::new(attempt.deadline, token.clone());
        pending.spawn(async move {
            tokio::select! {
                _ = token.cancelled() => return Err(ConnectError::Cancelled),
                _ = tokio::time::sleep(Duration::from_millis(delay.saturating_mul(index as u64))) => {},
            }
            emit(json!({"event":"mio138-candidate-start", "route":route.to_string(), "elapsedMs":started.elapsed().as_millis()}));
            coordinator.connect(EndpointAddr::new(id).with_addrs([route]), SessionPurpose::MemberBusiness, child).await.map(|connection| (index, connection))
        });
    }
    let result = loop {
        match pending.join_next().await {
            Some(Ok(Ok((index, connection)))) => break Ok((index, connection)),
            Some(Ok(Err(
                error @ (ConnectError::InvalidControl
                | ConnectError::IdentityOrAdmissionRejected
                | ConnectError::ProtocolUnsupported),
            ))) => break Err(error),
            Some(_) => {}
            None => break Err(ConnectError::DeadlineExceeded),
        }
    };
    for (index, token) in tokens.into_iter().enumerate() {
        if !result.as_ref().is_ok_and(|(winner, _)| *winner == index) {
            token.cancel();
        }
    }
    while let Some(outcome) = pending.join_next().await {
        if let Ok(Ok((_, connection))) = outcome {
            connection.close(0u32.into(), b"experimental route loser");
        }
    }
    result.map(|(_, connection)| connection)
}

async fn run_actor(config: Actor) -> Result<Value, Failure> {
    run_actor_with_request_barrier(config, None).await
}

// Controlled relay latency starts after admission has completed, so this one
// experiment measures the original 3s request separately from connection setup.
async fn run_actor_with_request_barrier(
    config: Actor,
    request_barrier: Option<(
        tokio::sync::oneshot::Sender<()>,
        tokio::sync::oneshot::Receiver<()>,
    )>,
) -> Result<Value, Failure> {
    if matches!(config.mode, ActorMode::Prepare) {
        prepare(&config.directory).await?;
        return Ok(json!({"prepared":true}));
    }
    let node = NativeNetwork::open(&config.directory).await?;
    let expected: iroh::EndpointId =
        std::fs::read_to_string(config.directory.join("expected-peer"))?.parse()?;
    let mut resources = node.resource_configuration()?;
    resources.replacement = Some(
        config
            .relays
            .iter()
            .map(|url| Resource {
                url: url.to_string(),
                uses: BTreeSet::from([ResourceUse::DataRelay]),
                limitation: "MIO138 explicit experiment; same permission in both groups".into(),
            })
            .collect(),
    );
    resources.udp_direct_disabled = config.relay_only;
    resources.tcp_direct_disabled = true;
    resources.member_data_relay_disabled = true;
    resources.disabled_uses.insert(ResourceUse::Observation);
    node.save_resource_configuration(&resources)?;
    let map = iroh::RelayMode::custom(
        config
            .home
            .clone()
            .map(|url| vec![url])
            .unwrap_or_else(|| config.relays.clone()),
    );
    let builder = Endpoint::builder(iroh::endpoint::presets::Minimal)
        .relay_mode(map)
        .alpns(vec![SessionPurpose::MemberBusiness.alpn().to_vec()])
        .bind_addr((Ipv4Addr::UNSPECIFIED, config.bind_port))?
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled);
    let endpoint = node.bind_endpoint_with(builder).await?;
    let coordinator =
        ConnectionCoordinator::relay_comparison_fixture(endpoint.clone(), !config.relay_only);
    let ready = Instant::now();
    emit(
        json!({"event":"mio138-endpoint-ready", "at":wall_ms(), "mode":config.mode, "home":home(&endpoint),
        "homePolicy":config.home, "permissions":config.relays, "identity":endpoint.id().to_string(), "sockets":endpoint.bound_sockets()}),
    );
    let stop = CancellationToken::new();
    let watch_stop = stop.clone();
    let watched = endpoint.clone();
    let watcher = tokio::spawn(async move {
        let mut previous = Value::Null;
        loop {
            let current = home(&watched);
            if current != previous {
                emit(
                    json!({"event":"mio138-home", "at":wall_ms(), "elapsedMs":ready.elapsed().as_millis(),"home":current}),
                );
                previous = current;
            }
            tokio::select! { _ = watch_stop.cancelled()=>break, _ = tokio::time::sleep(Duration::from_millis(100))=>{} }
        }
    });
    let registration = tokio::time::timeout(Duration::from_secs(10), endpoint.online()).await;
    emit(json!({"event":"mio138-registration-wait", "at":wall_ms(),
        "elapsedMs":ready.elapsed().as_millis(), "completed":registration.is_ok(), "home":home(&endpoint)}));
    let outcome = if matches!(config.mode, ActorMode::Serve) {
        let mut address = EndpointAddr::new(endpoint.id())
            .with_addrs(config.relays.iter().cloned().map(TransportAddr::Relay));
        if !config.relay_only {
            address
                .addrs
                .extend(endpoint.addr().ip_addrs().copied().map(TransportAddr::Ip));
        }
        std::fs::write(&config.target_file, serde_json::to_vec(&address)?)?;
        let deadline = tokio::time::Instant::now() + Duration::from_millis(config.lifetime_ms);
        let stop_file = config.target_file.with_extension("stop");
        let mut stop_tick = tokio::time::interval(Duration::from_millis(100));
        let mut handlers = JoinSet::new();
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline)=>break,
                _ = stop_tick.tick()=>{if stop_file.exists(){break}},
                incoming = endpoint.accept() => { let Some(incoming) = incoming else {break};
                    let co = coordinator.clone(); let identity = endpoint.id().to_string();
                    let gate = node.admission_gate();
                    handlers.spawn(async move {
                        let result = co.accept(incoming, SessionPurpose::MemberBusiness,
                            AttemptContext::new(n0_future::time::Instant::now()+Duration::from_millis(config.connect_ms), CancellationToken::new())).await;
                        match result { Ok(connection) => {
                            let matched = connection.remote_id()==expected && gate.allows(&connection, ProtocolScope::Member);
                            emit(json!({"event":"mio138-application-ready", "at":wall_ms(), "elapsedMs":ready.elapsed().as_millis(),"identityMatched":matched,"stableId":connection.stable_id()}));
                            if matched { let _=echo(connection, identity).await; }
                        }, Err(error)=>emit(json!({"event":"mio138-accept-error","error":error.to_string()})) }
                    });
                },
                Some(_) = handlers.join_next(), if !handlers.is_empty()=>{},
            }
        }
        endpoint.close().await;
        handlers.abort_all();
        while handlers.join_next().await.is_some() {}
        json!({"serverFinished":true})
    } else {
        let target: EndpointAddr = serde_json::from_slice(&std::fs::read(&config.target_file)?)?;
        if target.id != expected {
            return Err("target identity mismatch".into());
        }
        emit(
            json!({"event":"mio138-dial", "at":wall_ms(), "target":target,"earlyMs":config.early_ms,"budgetMs":config.connect_ms}),
        );
        let started = Instant::now();
        let result = dial(
            &coordinator,
            target,
            AttemptContext::new(
                n0_future::time::Instant::now() + Duration::from_millis(config.connect_ms),
                CancellationToken::new(),
            ),
            config.early_ms,
        )
        .await;
        match result {
            Ok(connection) => {
                let matched = connection.remote_id() == expected
                    && node
                        .admission_gate()
                        .allows(&connection, ProtocolScope::Member);
                let handshake_ms = started.elapsed().as_millis();
                emit(
                    json!({"event":"mio138-application-ready", "at":wall_ms(), "elapsedMs":handshake_ms, "identityMatched":matched,"stableId":connection.stable_id()}),
                );
                if let Some((ready, proceed)) = request_barrier {
                    let _ = ready.send(());
                    proceed.await?;
                }
                let mut replies = vec![];
                for (index, budget) in REQUEST_BUDGETS.iter().enumerate() {
                    let reply = request(&connection, expected, index as u64 + 1, *budget).await;
                    emit(reply.clone());
                    replies.push(reply);
                }
                connection.close(0u32.into(), b"comparison finished");
                json!({"connected":true,"handshakeMs":handshake_ms,"identityMatched":matched,"requests":replies})
            }
            Err(error) => {
                json!({"connected":false,"elapsedMs":started.elapsed().as_millis(),"error":error.to_string()})
            }
        }
    };
    stop.cancel();
    watcher.await?;
    endpoint.close().await;
    node.close().await;
    emit(json!({"event":"mio138-result","outcome":outcome}));
    Ok(outcome)
}

#[tokio::test]
async fn relay_probe_actor() {
    let Ok(path) = std::env::var("MIO138_ACTOR") else {
        return;
    };
    let config: Actor = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    run_actor(config).await.unwrap();
}

/// Transparent TCP fault injection around an actual Iroh relay, with no
/// protocol implementation or fabricated replies. Delay is per forwarded chunk.
struct Gate {
    url: RelayUrl,
    blackhole: Arc<std::sync::atomic::AtomicBool>,
    delay_ms: Arc<std::sync::atomic::AtomicU64>,
    connections: Arc<std::sync::atomic::AtomicUsize>,
    bytes: Arc<std::sync::atomic::AtomicU64>,
    stop: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}
impl Gate {
    async fn new(backend: std::net::SocketAddr) -> Self {
        use std::sync::atomic::Ordering::SeqCst;
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let blackhole = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let delay_ms = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let connections = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let bytes = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let stop = CancellationToken::new();
        let (drop_flag, delay, count, traffic, cancel) = (
            blackhole.clone(),
            delay_ms.clone(),
            connections.clone(),
            bytes.clone(),
            stop.clone(),
        );
        let task = tokio::spawn(async move {
            let mut workers = JoinSet::new();
            loop {
                tokio::select! {
                    _=cancel.cancelled()=>break,
                    incoming=listener.accept()=>{
                        let (incoming,_)=incoming.unwrap(); count.fetch_add(1,SeqCst);
                        let (drop_flag,delay,traffic)=(drop_flag.clone(),delay.clone(),traffic.clone());
                        workers.spawn(async move {
                            let Ok(outgoing)=tokio::net::TcpStream::connect(backend).await else {return};
                            let (ir,iw)=incoming.into_split(); let (or,ow)=outgoing.into_split();
                            let forward=async |mut reader:tokio::net::tcp::OwnedReadHalf,mut writer:tokio::net::tcp::OwnedWriteHalf| {
                                let mut buffer=[0u8;16384];
                                loop {let n=reader.read(&mut buffer).await?; if n==0 {return Ok::<_,std::io::Error>(());}
                                    if drop_flag.load(SeqCst) {continue;}
                                    tokio::time::sleep(Duration::from_millis(delay.load(SeqCst))).await;
                                    writer.write_all(&buffer[..n]).await?; traffic.fetch_add(n as u64,SeqCst);
                                }
                            };
                            tokio::select! {_=forward(ir,ow)=>{},_=forward(or,iw)=>{}}
                        });
                    },
                    Some(_)=workers.join_next(),if !workers.is_empty()=>{},
                }
            }
            workers.abort_all();
            while workers.join_next().await.is_some() {}
        });
        Self {
            url,
            blackhole,
            delay_ms,
            connections,
            bytes,
            stop,
            task,
        }
    }
    fn counts(&self) -> Value {
        use std::sync::atomic::Ordering::SeqCst;
        json!({"url":self.url,"tcpConnections":self.connections.load(SeqCst),"forwardedBytes":self.bytes.load(SeqCst)})
    }
    async fn close(self) {
        self.stop.cancel();
        self.task.await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controlled_relay_comparison() {
    use std::sync::atomic::Ordering::SeqCst;
    let directory = tempfile::tempdir().unwrap();
    prepare(directory.path()).await.unwrap();
    let mut config = iroh_relay::server::ServerConfig::default();
    config.relay = Some(iroh_relay::server::RelayConfig::new((
        Ipv4Addr::LOCALHOST,
        0,
    )));
    let relay = iroh_relay::server::Server::spawn(config).await.unwrap();
    let backend = (
        Ipv4Addr::LOCALHOST,
        relay.http_url().unwrap().port().unwrap(),
    )
        .into();
    let first = Gate::new(backend).await;
    let second = Gate::new(backend).await;
    let (primary, secondary) = if first.url < second.url {
        (first, second)
    } else {
        (second, first)
    };
    let relays = vec![primary.url.clone(), secondary.url.clone()];
    let target_file = directory.path().join("target.json");
    let mut results = vec![];
    // Repeat the same cut/recovery sequence; the home and start-time factors
    // are changed separately. Both URLs always retain identical permissions.
    for repetition in 0..2 {
        for (condition, relay_only, auto_home, early_ms) in [
            ("healthy-normal", false, false, None),
            ("healthy-relay-only", true, false, None),
            ("cut-auto", true, true, None),
            ("cut-fixed", true, false, None),
            ("cut-fixed-early", true, false, Some(250)),
            ("restored-fixed", true, false, None),
            ("slow-fixed", true, false, None),
            ("slow-fixed-early", true, false, Some(250)),
            ("business-slow-fixed", true, false, None),
        ] {
            primary
                .blackhole
                .store(condition.starts_with("cut"), SeqCst);
            primary.delay_ms.store(
                if condition.starts_with("slow") {
                    150
                } else {
                    0
                },
                SeqCst,
            );
            // A healthy fixed home lets the candidate-start experiment isolate
            // the dial schedule from registration failures at the first URL.
            let home = if auto_home {
                None
            } else {
                Some(secondary.url.clone())
            };
            let server_config = Actor {
                mode: ActorMode::Serve,
                directory: directory.path().join("server"),
                target_file: target_file.clone(),
                relays: relays.clone(),
                home: home.clone(),
                relay_only,
                early_ms: None,
                connect_ms: 4000,
                lifetime_ms: 60000,
                bind_port: 0,
            };
            let _ = std::fs::remove_file(&target_file);
            let _ = std::fs::remove_file(target_file.with_extension("stop"));
            let server = tokio::spawn(run_actor(server_config));
            tokio::time::timeout(Duration::from_secs(12), async {
                while !target_file.exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            let before = [primary.counts(), secondary.counts()];
            let client_config = Actor {
                mode: ActorMode::Connect,
                directory: directory.path().join("client"),
                target_file: target_file.clone(),
                relays: relays.clone(),
                home,
                relay_only,
                early_ms,
                connect_ms: 4000,
                lifetime_ms: 0,
                bind_port: 0,
            };
            let outcome = if condition == "business-slow-fixed" {
                let (ready, reached) = tokio::sync::oneshot::channel();
                let (proceed, wait) = tokio::sync::oneshot::channel();
                let client = tokio::spawn(run_actor_with_request_barrier(
                    client_config,
                    Some((ready, wait)),
                ));
                reached.await.unwrap();
                primary.delay_ms.store(1800, SeqCst);
                secondary.delay_ms.store(1800, SeqCst);
                proceed.send(()).unwrap();
                let outcome = client.await.unwrap().unwrap();
                primary.delay_ms.store(0, SeqCst);
                secondary.delay_ms.store(0, SeqCst);
                outcome
            } else {
                run_actor(client_config).await.unwrap()
            };
            std::fs::write(target_file.with_extension("stop"), b"comparison completed").unwrap();
            server.await.unwrap().unwrap();
            let record = json!({"event":"mio138-controlled-case","condition":condition,"repetition":repetition,
                "fault":"transparent TCP chunk delay/drop; real Iroh relay and admitted SDK members", "outcome":outcome,
                "before":before,"after":[primary.counts(),secondary.counts()]});
            emit(record.clone());
            results.push(record);
        }
    }
    for result in &results {
        let name = result["condition"].as_str().unwrap();
        if name.starts_with("healthy") || name.starts_with("restored") || name.ends_with("early") {
            assert_eq!(result["outcome"]["connected"], true, "{result}");
            assert_eq!(result["outcome"]["identityMatched"], true, "{result}");
            assert!(
                result["outcome"]["requests"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|r| r["matched"] == true),
                "{result}"
            );
        }
        if name == "cut-fixed" {
            assert_eq!(
                result["outcome"]["connected"], false,
                "the blocked first route must reproduce the original sequential deadline exhaustion: {result}"
            );
        }
        if name == "business-slow-fixed" {
            assert_eq!(result["outcome"]["connected"], true, "{result}");
            assert_eq!(result["outcome"]["identityMatched"], true, "{result}");
            let replies = result["outcome"]["requests"].as_array().unwrap();
            assert_eq!(replies[0]["budgetMs"], REQUEST_BUDGETS[0]);
            assert_eq!(replies[1]["budgetMs"], REQUEST_BUDGETS[1]);
        }
    }
    if let Ok(path) = std::env::var("MIO138_RESULTS") {
        std::fs::write(path, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    }
    primary.close().await;
    secondary.close().await;
    relay.shutdown().await.unwrap();
}
