//! Operator demo over the maintained application endpoint. HTTP is loopback-only;
//! all inter-device samples and remote observations use authenticated member QUIC.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::{
    collections::{BTreeMap, VecDeque},
    error::Error,
    io::{Read, Seek, SeekFrom, Write},
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use iroh::{
    EndpointAddr,
    endpoint::{MemberConnection, MemberStream},
};
use miopunch_native_probe::{
    admission_gate::{PRECODE_REQUEST_ALPN, ProtocolScope},
    client_commands::save_private,
    client_host::HostLease,
    client_invitation::BootstrapContact,
    connection_coordinator::connection_diagnostic,
    control_protocol::{AuthenticatedControl, OwnedControl},
    native_client::{ClientInvitation, NativeClient},
    native_network::{FUSED_MEMBER_ALPN, NativeNetwork},
    network_maintenance::{MaintainedNetwork, MaintenancePolicy},
    resources::{ResourceConfig, ResourceUse},
};
use n0_future::{FuturesUnorderedBounded, StreamExt};
use n0_watcher::Watcher;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Semaphore, oneshot},
    task::{JoinHandle, JoinSet},
    time::{Instant, timeout},
};
use tokio_util::sync::CancellationToken;

type Failure = Box<dyn Error + Send + Sync>;
const MAX_REPLY: usize = 256 * 1024;
const MAX_PENDING_PROBES: usize = 8;
const PROBE_SERVICE: u8 = 7;

#[derive(Deserialize)]
struct Peer {
    name: String,
    address: EndpointAddr,
}

#[derive(Deserialize)]
struct Configuration {
    name: String,
    listen: SocketAddr,
    peers: Vec<Peer>,
}

struct Observations {
    local: Value,
    peers: BTreeMap<String, Value>,
    connections: BTreeMap<String, MemberConnection>,
    events: VecDeque<Value>,
    remote: BTreeMap<String, Value>,
}

impl Observations {
    fn event(&mut self, kind: &str, detail: Value) {
        self.events
            .push_back(json!({"at":now_ms(),"monotonicMs":connection_diagnostic::monotonic_ms(),"kind":kind,"detail":detail}));
        if self.events.len() > 128 {
            self.events.pop_front();
        }
    }

    // Deliberately excludes other nodes' snapshots: remote data cannot recurse
    // into subsequent replies or turn a heartbeat into whole-network history.
    fn local_view(&self) -> Value {
        let mut view = self.local.clone();
        view["observedAt"] = json!(now_ms());
        view["peers"] = json!(self.peers);
        view
    }

    fn view(&self) -> Value {
        json!({"local":self.local_view(),"remote":self.remote,"events":self.events})
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

// The maintenance/contact protocol accepts at most eight hints. Hosts with
// many Docker/VPN interfaces can advertise far more. Keep transport diversity
// in the hint and keep the full live address list in diagnostics.
fn contact_hint(mut address: EndpointAddr) -> EndpointAddr {
    let mut relay = VecDeque::new();
    let mut ip = VecDeque::new();
    let mut custom = VecDeque::new();
    for route in address.addrs {
        match route {
            iroh::TransportAddr::Relay(_) => relay.push_back(route),
            iroh::TransportAddr::Ip(_) => ip.push_back(route),
            _ => custom.push_back(route),
        }
    }
    let public_ipv4 = ip.iter().position(|route| {
        let iroh::TransportAddr::Ip(SocketAddr::V4(address)) = route else {
            return false;
        };
        let ip = address.ip();
        let [first, second, ..] = ip.octets();
        (1..224).contains(&first)
            && !ip.is_private()
            && !ip.is_loopback()
            && !ip.is_link_local()
            && !ip.is_documentation()
            && !matches!((first, second), (100, 64..=127) | (198, 18..=19))
    });
    if let Some(route) = public_ipv4.and_then(|index| ip.remove(index)) {
        ip.push_front(route);
    }
    address.addrs = Default::default();
    while address.addrs.len() < 8 {
        let before = address.addrs.len();
        for source in [&mut relay, &mut ip, &mut custom] {
            if address.addrs.len() < 8
                && let Some(route) = source.pop_front()
            {
                address.addrs.insert(route);
            }
        }
        if address.addrs.len() == before {
            break;
        }
    }
    address
}

#[derive(Clone)]
struct ProbeServer {
    observations: Arc<Mutex<Observations>>,
    cancellation: CancellationToken,
}

impl ProbeServer {
    async fn accept(&self, connection: MemberConnection) {
        println!(
            "{}",
            json!({"event":"network-demo-incoming-ready","at":now_ms(),
            "peer":connection.remote_id().to_string(),"transport":path_view(&connection)})
        );
        let peer = connection.remote_id().to_string();
        // Keep stalled requests from blocking other streams while bounding the
        // number of snapshots held in memory. All probes belong to this handler.
        let mut probes = FuturesUnorderedBounded::new(MAX_PENDING_PROBES);
        let ended_by = loop {
            tokio::select! {
                _ = self.cancellation.cancelled() => break "stopped",
                stream = connection.accept_bi(), if probes.len() < probes.capacity() => {
                    match stream {
                        Ok(mut stream) => {
                            let stream_id = u64::from(stream.streams().0.id());
                            self.observations.lock().unwrap().event(
                                "probe-stream-accepted",
                                json!({"peer":peer,"stableId":connection.stable_id(),
                                "streamId":stream_id,"pending":probes.len()}),
                            );
                            probes.push(self.serve_probe(stream, &connection));
                        }
                        Err(_) => break "accept_stream",
                    }
                }
                result = probes.next(), if !probes.is_empty() => {
                    if let Some(Err(reason)) = result {
                        self.observations.lock().unwrap().event(
                            "probe-failed",
                            json!({"peer":peer,"stableId":connection.stable_id(),"reason":reason}),
                        );
                    }
                }
            }
        };
        drop(probes);
        if ended_by == "accept_stream" {
            self.observations.lock().unwrap().event(
                "probe-failed",
                json!({
                    "peer":peer,"stableId":connection.stable_id(),"reason":ended_by
                }),
            );
        }
        record_connection_end(&self.observations, &connection, &peer, ended_by);
    }

    async fn serve_probe(
        &self,
        mut stream: MemberStream,
        connection: &MemberConnection,
    ) -> Result<(), &'static str> {
        let (send, recv) = stream.streams();
        let start = Instant::now();
        let stream_id = u64::from(send.id());
        let request = timeout(Duration::from_secs(5), recv.read_to_end(9))
            .await
            .map_err(|_| "stream_read")?
            .map_err(|_| "stream_read")?;
        let Some((&PROBE_SERVICE, sequence)) = request.split_first() else {
            return Err("stream_decode");
        };
        let sequence = u64::from_be_bytes(sequence.try_into().map_err(|_| "stream_decode")?);
        self.observations.lock().unwrap().event(
            "probe-request-read",
            json!({"peer":connection.remote_id().to_string(),"stableId":connection.stable_id(),
            "streamId":stream_id,"sequence":sequence,"elapsedMs":start.elapsed().as_secs_f64()*1000.}),
        );
        let local = self.observations.lock().unwrap().local_view();
        let response = serde_json::to_vec(&json!({"sequence":sequence,"snapshot":local}))
            .map_err(|_| "stream_response")?;
        if response.len() > MAX_REPLY {
            return Err("stream_response");
        }
        timeout(Duration::from_secs(5), send.write_all(&response))
            .await
            .map_err(|_| "stream_write")?
            .map_err(|_| "stream_write")?;
        send.finish().map_err(|_| "stream_finish")?;
        self.observations.lock().unwrap().event(
            "probe-reply-queued",
            json!({"peer":connection.remote_id().to_string(),"stableId":connection.stable_id(),
            "streamId":stream_id,"sequence":sequence,"responseBytes":response.len(),
            "elapsedMs":start.elapsed().as_secs_f64()*1000.,
            "selectedRttMs":connection.paths().iter().find(|p|p.is_selected())
                .map(|p|p.stats().rtt.as_secs_f64()*1000.)}),
        );
        Ok(())
    }
}

async fn serve_probes(
    network: Arc<MaintainedNetwork>,
    handler: ProbeServer,
) -> Result<(), Failure> {
    let mut sessions = JoinSet::new();
    loop {
        tokio::select! {
            _ = handler.cancellation.cancelled() => break,
            _ = sessions.join_next(), if !sessions.is_empty() => {},
            incoming = network.accept_member() => match incoming {
                Some(Ok(connection)) => {
                    let handler = handler.clone();
                    sessions.spawn(async move { handler.accept(connection).await });
                }
                Some(Err(_)) => {},
                None => break,
            },
        }
    }
    while sessions.join_next().await.is_some() {}
    Ok(())
}

fn record_connection_end(
    state: &Mutex<Observations>,
    connection: &MemberConnection,
    peer: &str,
    cause: &'static str,
) {
    let close = connection.close_reason();
    let owner = close.as_ref().map_or("service", |reason| {
        connection_diagnostic::close_origin(Some(reason))
    });
    state.lock().unwrap().event(
        "connection-ended",
        json!({"peer":peer,"stableId":connection.stable_id(),"owner":owner,"reason":cause,
            "closeReason":connection_diagnostic::close_reason(close.as_ref())}),
    );
}

fn path_view(connection: &MemberConnection) -> Value {
    json!({"stableId":connection.stable_id(),"alpn":String::from_utf8_lossy(FUSED_MEMBER_ALPN),"open":connection.close_reason().is_none(),
        "side":format!("{:?}",connection.side()),
        "paths":connection.paths().iter().map(|path| {
            let kind = if path.is_relay() {"relay"} else if path.is_ip() {"udp"}
                else if matches!(path.remote_addr(),iroh::TransportAddr::Custom(addr)
                    if addr.id()==miopunch_native_probe::tcp_transport::DATA_TRANSPORT_ID) {"tcp"}
                else {"custom"};
            let stats = path.stats();
            json!({"id":path.id().to_string(),"kind":kind,"selected":path.is_selected(),
                "remote":path.remote_addr().to_string(),"local":format!("{:?}",path.local_addr()),
                "socket":format!("{:?}",path.local_socket_id()),
                "rttMs":stats.rtt.as_secs_f64()*1000.,
                "txBytes":stats.udp_tx.bytes,"rxBytes":stats.udp_rx.bytes,
                "streamTx":stats.frame_tx.stream,"streamRx":stats.frame_rx.stream,
                "lostPackets":stats.lost_packets})
        }).collect::<Vec<_>>()})
}

#[derive(Default)]
struct RequestProgress {
    stream_id: Option<u64>,
    sent_ms: Option<u64>,
}

async fn exchange(
    connection: &MemberConnection,
    sequence: u64,
    progress: &mut RequestProgress,
) -> Result<Value, Failure> {
    let mut stream = connection.open_maintenance_bi().await?;
    let (send, recv) = stream.streams();
    progress.stream_id = Some(u64::from(send.id()));
    send.write_all(&[PROBE_SERVICE]).await?;
    send.write_all(&sequence.to_be_bytes()).await?;
    send.finish()?;
    progress.sent_ms = Some(connection_diagnostic::monotonic_ms());
    let reply: Value = serde_json::from_slice(&recv.read_to_end(MAX_REPLY).await?)?;
    if reply["sequence"].as_u64() != Some(sequence)
        || reply["snapshot"]["id"].as_str() != Some(&connection.remote_id().to_string())
    {
        return Err("probe reply identity or sequence mismatch".into());
    }
    Ok(reply["snapshot"].clone())
}

async fn observe_peer(
    network: Arc<MaintainedNetwork>,
    peer: Peer,
    state: Arc<Mutex<Observations>>,
    cancellation: CancellationToken,
) {
    let mut sequence = 0;
    let mut connections = 0;
    let mut successes = 0;
    let mut failures = 0;
    let mut unavailable_since = Some(Instant::now());
    loop {
        {
            let mut state = state.lock().unwrap();
            state.peers.get_mut(&peer.name).unwrap()["phase"] = json!("connecting");
            state.event("connect-start", json!({"peer":peer.name,"alpn":String::from_utf8_lossy(FUSED_MEMBER_ALPN),"target":peer.address}));
        }
        let connected = tokio::select! {
            _ = cancellation.cancelled() => return,
            result = network.connect_member(peer.address.clone()) => result,
        };
        match connected {
            Ok(connection) => {
                connections += 1;
                println!(
                    "{}",
                    json!({"event":"network-demo-outgoing-ready","at":now_ms(),
                    "peer":peer.name,"peerId":connection.remote_id().to_string(),"transport":path_view(&connection)})
                );
                {
                    let mut state = state.lock().unwrap();
                    state
                        .connections
                        .insert(peer.name.clone(), connection.clone());
                    let entry = state.peers.get_mut(&peer.name).unwrap();
                    entry["phase"] = json!("checking");
                    entry["connections"] = json!(connections);
                    entry["transport"] = path_view(&connection);
                    state.event(
                        "connection-open",
                        json!({"peer":peer.name,"stableId":connection.stable_id()}),
                    );
                }
                let stopped = loop {
                    sequence += 1;
                    let start = Instant::now();
                    let mut progress = RequestProgress::default();
                    state.lock().unwrap().event(
                        "business-request-start",
                        json!({"peer":peer.name,"stableId":connection.stable_id(),"sequence":sequence}),
                    );
                    let result = tokio::select! {
                        _ = cancellation.cancelled() => break true,
                        result = timeout(Duration::from_secs(3), exchange(&connection, sequence, &mut progress)) => result,
                    };
                    match result {
                        Ok(Ok(remote)) => {
                            let round_trip_ms = start.elapsed().as_secs_f64() * 1000.;
                            if let Some(address) = remote.get("address")
                                && let Ok(address) =
                                    serde_json::from_value::<EndpointAddr>(address.clone())
                                && address.id == connection.remote_id()
                                && let Err(error) = network.offer_contact(contact_hint(address)).await
                            {
                                state.lock().unwrap().event(
                                    "contact-learning-failed",
                                    json!({"peer":peer.name,"error":error.to_string()}),
                                );
                            }
                            successes += 1;
                            let mut state = state.lock().unwrap();
                            if let Some(since) = unavailable_since.take() {
                                let elapsed = since.elapsed().as_millis() as u64;
                                state.peers.get_mut(&peer.name).unwrap()["lastRecoveryMs"] =
                                    json!(elapsed);
                                state.event(
                                    "business-restored",
                                    json!({"peer":peer.name,"elapsedMs":elapsed,
                                    "peerId":connection.remote_id().to_string(),"sequence":sequence,
                                    "roundTripMs":round_trip_ms,
                                    "stableId":connection.stable_id(),"connections":connections,
                                    "streamId":progress.stream_id,"requestSentMonotonicMs":progress.sent_ms}),
                                );
                            }
                            let entry = state.peers.get_mut(&peer.name).unwrap();
                            entry["phase"] = json!("online");
                            entry["lastOkAt"] = json!(now_ms());
                            entry["lastSequence"] = json!(sequence);
                            entry["roundTripMs"] = json!(round_trip_ms);
                            entry["successes"] = json!(successes);
                            entry["error"] = Value::Null;
                            state.remote.insert(
                                peer.name.clone(),
                                json!({"receivedAt":now_ms(),"snapshot":remote}),
                            );
                        }
                        result => {
                            failures += 1;
                            unavailable_since.get_or_insert(start);
                            let error = match result {
                                Ok(Err(error)) => error.to_string(),
                                Err(_) => "heartbeat deadline (3s)".into(),
                                _ => unreachable!(),
                            };
                            {
                                let mut state = state.lock().unwrap();
                                let entry = state.peers.get_mut(&peer.name).unwrap();
                                entry["phase"] = json!("recovering");
                                entry["failures"] = json!(failures);
                                entry["error"] = json!(error);
                                state.event(
                                    "heartbeat-failed",
                                    json!({"peer":peer.name,"error":error,
                                    "stableId":connection.stable_id(),"sequence":sequence,
                                    "streamId":progress.stream_id,"requestSentMonotonicMs":progress.sent_ms,
                                    "elapsedMs":start.elapsed().as_secs_f64()*1000.}),
                                );
                            }
                            if connection.close_reason().is_some() {
                                break false;
                            }
                        }
                    }
                    tokio::select! {
                        _ = cancellation.cancelled() => break true,
                        _ = tokio::time::sleep_until(start+Duration::from_secs(1)) => {},
                    }
                };
                record_connection_end(
                    &state,
                    &connection,
                    &peer.name,
                    if stopped {
                        "stopped"
                    } else {
                        "reconnect_cleanup"
                    },
                );
                state.lock().unwrap().connections.remove(&peer.name);
                if stopped {
                    return;
                }
            }
            Err(error) => {
                let error = format!("{error:?}");
                failures += 1;
                let mut state = state.lock().unwrap();
                let entry = state.peers.get_mut(&peer.name).unwrap();
                entry["phase"] = json!("unreachable");
                entry["error"] = json!(error);
                entry["failures"] = json!(failures);
                state.event("connect-failed", json!({"peer":peer.name,"error":error}));
            }
        }
        // Application reconnection is distinct from migration on the original M.
        // ResourceEndpoint retains its own per-peer contact budget and backoff.
        tokio::select! {
            _ = cancellation.cancelled() => return,
            _ = tokio::time::sleep(Duration::from_secs(1)) => {},
        }
    }
}

async fn sample(
    network: Arc<MaintainedNetwork>,
    state: Arc<Mutex<Observations>>,
    directory: PathBuf,
    cancellation: CancellationToken,
) -> Result<(), Failure> {
    let mut last_address = network.addr();
    let mut last_generation = network.endpoint().udp_network_generation();
    // Keep the same local samples while the office link is unavailable. The
    // latest snapshot and bounded event ring alone lose the switch history.
    let mut history = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("status.jsonl"))?;
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {_ = cancellation.cancelled()=>return Ok(()),_ = interval.tick()=>{}}
        let address = network.addr();
        if address != last_address {
            state.lock().unwrap().event(
                "local-address-changed",
                json!({"before":last_address,"after":address}),
            );
            last_address = address.clone();
        }
        let generation = network.endpoint().udp_network_generation();
        if generation != last_generation {
            network.network_changed().await?;
            last_generation = generation;
            state
                .lock()
                .unwrap()
                .event("network-generation-changed", json!({}));
        }
        let maintenance = network.maintenance_status();
        let relays = network
            .endpoint()
            .home_relay_status()
            .get()
            .iter()
            .map(|relay| {
                json!({"url":relay.url().to_string(),"connected":relay.is_connected(),
                "error":relay.last_error().map(|error|error.to_string())})
            })
            .collect::<Vec<_>>();
        let peers: Vec<_> = state
            .lock()
            .unwrap()
            .peers
            .iter()
            .map(|(name, peer)| (name.clone(), peer["id"].as_str().unwrap().parse().unwrap()))
            .collect();
        let mut contacts = BTreeMap::new();
        for (name, id) in peers {
            let info = network.endpoint().remote_info(id).await;
            contacts.insert(name,info.map(|info|info.addrs().map(|addr|
                json!({"address":addr.addr().to_string(),"usage":format!("{:?}",addr.usage())})).collect::<Vec<_>>()));
        }
        let view = {
            let mut state = state.lock().unwrap();
            state.local["at"] = json!(now_ms());
            state.local["sampleMonotonicMs"] = json!(connection_diagnostic::monotonic_ms());
            state.local["address"] = json!(address);
            state.local["boundSockets"] = json!(network.endpoint().bound_sockets());
            state.local["relays"] = json!(relays);
            state.local["maintenance"] = json!({"state":format!("{:?}",maintenance.state),
                "contactAttempts":maintenance.contact_attempts,"discoveryRounds":maintenance.discovery_rounds,
                "maxInFlight":maintenance.max_in_flight,"retryAfterMs":maintenance.retry_after.as_millis(),
                "cause":maintenance.cause,"contactListener":format!("{:?}",maintenance.contact_listener),
                "neighbors":maintenance.neighbors.iter().map(|n|json!({"id":n.id.to_string(),
                    "stableId":n.stable_id,"pingsSent":n.pings_sent})).collect::<Vec<_>>()});
            state.local["members"] = json!(
                maintenance
                    .node
                    .members
                    .iter()
                    .map(|(id, _)| id.to_string())
                    .collect::<Vec<_>>()
            );
            let paths: Vec<_> = state
                .connections
                .iter()
                .map(|(name, connection)| (name.clone(), path_view(connection)))
                .collect();
            for (name, transport) in paths {
                let selected = |v: &Value| {
                    v["paths"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|p| p["selected"] == true)
                        .map(|p| p["remote"].as_str().unwrap_or("").to_owned())
                        .collect::<Vec<_>>()
                };
                let before = selected(&state.peers[&name]["transport"]);
                let after = selected(&transport);
                if before != after {
                    state.event(
                        "selected-path-changed",
                        json!({"peer":name,
                        "before":before,"after":after,"stableId":transport["stableId"]}),
                    );
                }
                state.peers.get_mut(&name).unwrap()["transport"] = transport;
            }
            for (name, candidates) in contacts {
                state.peers.get_mut(&name).unwrap()["knownAddresses"] = json!(candidates);
            }
            state.view()
        };
        let temporary = directory.join("status.new");
        std::fs::write(&temporary, serde_json::to_vec(&view)?)?;
        std::fs::rename(temporary, directory.join("status.json"))?;
        serde_json::to_writer(&mut history, &view)?;
        history.write_all(b"\n")?;
    }
}

fn log_tail(path: &Path) -> String {
    let Ok(mut file) = std::fs::File::open(path) else {
        return String::new();
    };
    let _ = file
        .seek(SeekFrom::End(-32768))
        .or_else(|_| file.seek(SeekFrom::Start(0)));
    let mut bytes = Vec::new();
    let _ = file.take(32768).read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

struct NodeWorker {
    cancellation: CancellationToken,
    task: JoinHandle<Result<(), Failure>>,
}

/// The local diagnostic endpoint survives a stopped network node. Serialize
/// lifecycle operations until the actual SDK has released its state and sockets.
struct DemoHost {
    state_directory: PathBuf,
    config_path: PathBuf,
    observations: Arc<Mutex<Observations>>,
    worker: Arc<tokio::sync::Mutex<Option<NodeWorker>>>,
}

impl DemoHost {
    async fn start(&self) -> Result<(), Failure> {
        let mut owner = self.worker.lock().await;
        if owner
            .as_ref()
            .is_some_and(|worker| !worker.task.is_finished())
        {
            return Ok(());
        }
        if let Some(previous) = owner.take() {
            let _ = previous.task.await;
        }
        {
            let mut observations = self.observations.lock().unwrap();
            observations.local["hostState"] = json!("starting");
            observations.local["hostError"] = Value::Null;
            observations.event("node-starting", json!({}));
        }
        let cancellation = CancellationToken::new();
        let (ready, started) = oneshot::channel();
        let directory = self.state_directory.clone();
        let config_path = self.config_path.clone();
        let observations = self.observations.clone();
        let stop = cancellation.clone();
        let task = tokio::spawn(async move {
            let result =
                run_node(&directory, &config_path, observations.clone(), stop, ready).await;
            let mut state = observations.lock().unwrap();
            state.local["hostState"] = json!(if result.is_ok() { "stopped" } else { "failed" });
            state.local["hostError"] = if result.is_ok() {
                Value::Null
            } else {
                json!("node_failed; inspect SDK logs")
            };
            state.event("node-stopped", json!({"sdkClosed":result.is_ok()}));
            if let Err(error) = &result {
                eprintln!("NETWORK_DEMO_NODE_FAILED {error}");
            }
            result
        });
        *owner = Some(NodeWorker { cancellation, task });
        started
            .await
            .map_err(|_| "network node failed to start; inspect SDK logs")?;
        Ok(())
    }

    async fn stop(&self) -> Result<(), Failure> {
        let mut owner = self.worker.clone().lock_owned().await;
        let Some(worker) = owner.take() else {
            return Ok(());
        };
        {
            let mut state = self.observations.lock().unwrap();
            state.local["hostState"] = json!("stopping");
            state.event("node-stopping", json!({}));
        }
        worker.cancellation.cancel();
        // Retain ownership even if the browser closes or its request expires.
        tokio::spawn(async move {
            let result = worker.task.await;
            drop(owner);
            result?
        })
        .await?
    }
}

async fn http_request(
    mut stream: TcpStream,
    listen: SocketAddr,
    state: Arc<Mutex<Observations>>,
    directory: PathBuf,
    host: Arc<DemoHost>,
) -> Result<(), Failure> {
    let mut request = Vec::new();
    let mut buffer = [0; 1024];
    while !request.windows(4).any(|part| part == b"\r\n\r\n") {
        let size = stream.read(&mut buffer).await?;
        if size == 0 {
            return Ok(());
        }
        request.extend_from_slice(&buffer[..size]);
        if request.len() > 8192 {
            return Err("HTTP headers too large".into());
        }
    }
    let request = std::str::from_utf8(&request)?;
    let mut lines = request.split("\r\n");
    let first = lines.next().unwrap_or("");
    let parts: Vec<_> = first.split_whitespace().collect();
    let headers: Vec<_> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.to_ascii_lowercase(), value.trim()))
        .collect();
    let host_header = headers
        .iter()
        .find(|(key, _)| key == "host")
        .map(|(_, v)| *v);
    let origin = headers
        .iter()
        .find(|(key, _)| key == "origin")
        .map(|(_, v)| *v);
    let expected_host = listen.to_string();
    let expected_origin = format!("http://{listen}");
    let allowed = host_header == Some(expected_host.as_str())
        && (parts.first() != Some(&"POST") || origin == Some(expected_origin.as_str()));
    let (code, content_type, body) = if !allowed {
        (
            "403 Forbidden",
            "text/plain",
            b"loopback origin required".to_vec(),
        )
    } else {
        match parts.as_slice() {
            ["GET", "/", _] => (
                "200 OK",
                "text/html; charset=utf-8",
                include_bytes!("network_demo/index.html").to_vec(),
            ),
            ["GET", "/api/status", _] => (
                "200 OK",
                "application/json",
                serde_json::to_vec(&state.lock().unwrap().view())?,
            ),
            ["GET", "/api/logs", _] => (
                "200 OK",
                "application/json",
                serde_json::to_vec(&json!({
                "stdout":log_tail(&directory.join("stdout.log")),"stderr":log_tail(&directory.join("stderr.log"))}))?,
            ),
            ["POST", "/api/mark", _] => {
                state.lock().unwrap().event(
                    "operator-network-switch-marker",
                    json!({"doesNotChangeNetwork":true}),
                );
                ("200 OK", "application/json", b"{\"marked\":true}".to_vec())
            }
            ["POST", "/api/stop", _] => match host.stop().await {
                Ok(()) => ("200 OK", "application/json", b"{\"stopped\":true}".to_vec()),
                Err(_) => (
                    "503 Service Unavailable",
                    "application/json",
                    b"{\"error\":\"node_stop_failed\"}".to_vec(),
                ),
            },
            ["POST", "/api/start", _] => match host.start().await {
                Ok(()) => ("200 OK", "application/json", b"{\"started\":true}".to_vec()),
                Err(_) => (
                    "503 Service Unavailable",
                    "application/json",
                    b"{\"error\":\"node_start_failed\"}".to_vec(),
                ),
            },
            _ => ("404 Not Found", "text/plain", b"not found".to_vec()),
        }
    };
    let header = format!(
        "HTTP/1.1 {code}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'self'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(&body).await?;
    stream.shutdown().await?;
    Ok(())
}

async fn run_node(
    state_directory: &Path,
    config_path: &Path,
    observations: Arc<Mutex<Observations>>,
    cancellation: CancellationToken,
    ready: oneshot::Sender<()>,
) -> Result<(), Failure> {
    let config: Configuration = serde_json::from_slice(&std::fs::read(config_path)?)?;
    if !config.listen.ip().is_loopback() {
        return Err("diagnostic HTTP must bind loopback".into());
    }
    let _lease = HostLease::acquire(state_directory)?;
    let node = NativeNetwork::open(state_directory).await?;
    let identity = node.status().node_id.to_string();
    let root = state_directory
        .parent()
        .ok_or("missing runtime directory")?
        .to_path_buf();
    {
        let mut state = observations.lock().unwrap();
        let events = std::mem::take(&mut state.events);
        *state = Observations {
            local: json!({"name":config.name,"id":identity,"startedAt":now_ms(),"pid":std::process::id(),"hostState":"starting","hostError":null}),
            peers: config
                .peers
                .iter()
                .map(|peer| {
                    (
                        peer.name.clone(),
                        json!({"id":peer.address.id.to_string(),
            "phase":"starting","lastOkAt":null,"connections":0,"successes":0,"failures":0}),
                    )
                })
                .collect(),
            connections: BTreeMap::new(),
            events,
            remote: BTreeMap::new(),
        };
    }
    let handler = ProbeServer {
        observations: observations.clone(),
        cancellation: cancellation.clone(),
    };
    let network = Arc::new(
        node.maintain_application_network(Ipv4Addr::UNSPECIFIED, |builder| builder)
            .await?,
    );
    for peer in &config.peers {
        network.offer_contact(peer.address.clone()).await?;
    }
    std::fs::write(
        root.join("address.json"),
        serde_json::to_vec(&contact_hint(network.addr()))?,
    )?;
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_probes(network.clone(), handler));
    for peer in config.peers {
        let network = network.clone();
        let state = observations.clone();
        let stop = cancellation.clone();
        tasks.spawn(async move {
            observe_peer(network, peer, state, stop).await;
            Ok::<_, Failure>(())
        });
    }
    tasks.spawn(sample(
        network.clone(),
        observations.clone(),
        root.clone(),
        cancellation.clone(),
    ));
    {
        let mut state = observations.lock().unwrap();
        state.local["hostState"] = json!("running");
        state.event("node-started", json!({"id":identity}));
    }
    let _ = ready.send(());
    println!(
        "{}",
        json!({"event":"network-demo-ready","name":config.name,"id":identity,"url":format!("http://{}/",config.listen)})
    );
    let result: Result<(), Failure> = async {
        loop {
            tokio::select! {
                _=cancellation.cancelled()=>break,
                finished=tasks.join_next(),if !tasks.is_empty()=>{
                    finished.ok_or("missing worker")???;
                    if cancellation.is_cancelled() {break;}
                    return Err("network demo worker ended unexpectedly".into());
                },
            }
        }
        Ok(())
    }
    .await;
    cancellation.cancel();
    while tasks.join_next().await.is_some() {}
    Arc::try_unwrap(network)
        .map_err(|_| "network still owned")?
        .close()
        .await;
    result
}

async fn run(state_directory: &Path, config_path: &Path) -> Result<(), Failure> {
    let config: Configuration = serde_json::from_slice(&std::fs::read(config_path)?)?;
    if config_path.with_file_name("relay-trace").is_file() {
        iroh_relay::client::enable_relay_diagnostics();
    }
    if !config.listen.ip().is_loopback() {
        return Err("diagnostic HTTP must bind loopback".into());
    }
    let listener = TcpListener::bind(config.listen).await?;
    let directory = state_directory
        .parent()
        .ok_or("missing runtime directory")?
        .to_path_buf();
    let observations = Arc::new(Mutex::new(Observations {
        local: json!({"name":config.name,"pid":std::process::id(),"hostState":"stopped","hostError":null}),
        peers: BTreeMap::new(),
        connections: BTreeMap::new(),
        events: VecDeque::new(),
        remote: BTreeMap::new(),
    }));
    let host = Arc::new(DemoHost {
        state_directory: state_directory.to_owned(),
        config_path: config_path.to_owned(),
        observations: observations.clone(),
        worker: Arc::new(tokio::sync::Mutex::new(None)),
    });
    if host.start().await.is_err() {
        eprintln!("NETWORK_DEMO_START_FAILED; diagnostic endpoint remains available");
    }
    let permits = Arc::new(Semaphore::new(16));
    let mut http = JoinSet::new();
    let result:Result<(),Failure> = async {
        loop {
            tokio::select! {
                _=tokio::signal::ctrl_c()=>break,
                incoming=listener.accept()=>{
                    let (stream,_)=incoming?;
                    let Ok(permit)=permits.clone().try_acquire_owned() else {continue};
                    let state=observations.clone(); let directory=directory.clone(); let host=host.clone();
                    let listen=config.listen;
                    http.spawn(async move {let _permit=permit;
                        let _=timeout(Duration::from_secs(30),http_request(stream,listen,state,directory,host)).await;});
                },
                _=http.join_next(),if !http.is_empty()=>{},
            }
        }
        Ok(())
    }.await;
    while http.join_next().await.is_some() {}
    let stopped = host.stop().await;
    result?;
    stopped
}

async fn bootstrap(directory: &Path, invitation_path: &Path, create: bool) -> Result<(), Failure> {
    let _lease = HostLease::acquire(directory)?;
    // This is a separate operator test network, never an existing product node.
    // Provision it through the SDK fixture boundary; do not auto-ack a user's
    // recovery-card gate or import/copy any existing device identity.
    let mut node = if create {
        NativeNetwork::create(directory).await?
    } else {
        NativeNetwork::open(directory).await?
    };
    let config = if create {
        ResourceConfig {
            maintenance: MaintenancePolicy::active(),
            ..ResourceConfig::default()
        }
    } else {
        node.resource_configuration()?
    };
    node.save_resource_configuration(&config)?;
    node.admission_gate()
        .register_protocol(PRECODE_REQUEST_ALPN, ProtocolScope::AdmissionRequest);
    let resources = node.bind_configured_endpoint(Ipv4Addr::UNSPECIFIED).await?;
    if !config.relay_map(ResourceUse::DataRelay)?.is_empty() {
        timeout(
            Duration::from_millis(config.maintenance.contact_timeout_ms),
            resources.endpoint().online(),
        )
        .await
        .map_err(|_| "contact relay registration timed out before invitation export")?;
    }
    let code = node.issue_precode(now_ms() / 1000 + 86400, Some(8)).await?;
    let invitation = ClientInvitation::new(
        code.invitation().encode(),
        contact_hint(resources.addr()),
        Some(code.encode()),
        BootstrapContact {
            relays: config
                .relay_map(ResourceUse::DataRelay)?
                .urls::<Vec<_>>()
                .into_iter()
                .map(|url| url.to_string())
                .collect(),
            ..Default::default()
        },
    )?;
    save_private(
        invitation_path.to_str().ok_or("invalid invitation path")?,
        &serde_json::to_vec(&invitation)?,
    )?;
    println!(
        "{}",
        json!({"event":"bootstrap-ready","address":contact_hint(resources.addr())})
    );
    // Test provisioning uses the SDK admission protocol, independently of the
    // product creator UI and its explicit human recovery-save acknowledgment.
    let node = Arc::new(tokio::sync::Mutex::new(node));
    let mut handlers = JoinSet::new();
    let endpoint = resources.endpoint().clone();
    let mut stdin = tokio::io::stdin();
    let mut buffer = [0; 1];
    loop {
        tokio::select! {
            _=tokio::signal::ctrl_c()=>break,
            _=stdin.read(&mut buffer)=>break,
            incoming=endpoint.accept()=>{
                let Some(incoming)=incoming else {break};
                if handlers.len()>=8 {incoming.refuse();continue;}
                let node=node.clone();
                handlers.spawn(async move {
                    let result:Result<(),Failure>=async {
                        let connection=Arc::new(incoming.accept()?.into_control().await?);
                        let connection=connection.retain_control();
                        node.lock().await.serve_precode_admission(&connection).await?;
                        connection.revocation_handle().closed().await;
                        Ok(())
                    }.await;
                    if let Err(error)=result {eprintln!("admission exchange: {error}");}
                });
            },
            _=handlers.join_next(),if !handlers.is_empty()=>{},
        }
    }
    handlers.abort_all();
    while handlers.join_next().await.is_some() {}
    drop(endpoint);
    resources.close().await;
    Arc::try_unwrap(node)
        .map_err(|_| "bootstrap node still owned")?
        .into_inner()
        .close()
        .await;
    Ok(())
}

async fn join(directory: &Path, invitation_path: &Path, resume: bool) -> Result<(), Failure> {
    let _lease = HostLease::acquire(directory)?;
    let invitation = ClientInvitation::decode(&std::fs::read(invitation_path)?)?;
    let config = ResourceConfig {
        maintenance: MaintenancePolicy::active(),
        ..ResourceConfig::default()
    };
    let client = if resume {
        NativeClient::resume(directory, &invitation, Ipv4Addr::UNSPECIFIED).await?
    } else {
        NativeClient::join_with_resources(directory, &invitation, Ipv4Addr::UNSPECIFIED, config)
            .await?
    };
    let admitted = client.request_admission(&invitation).await;
    let address = contact_hint(client.address());
    client.close().await?;
    println!(
        "{}",
        json!({"event":"admitted","stage":format!("{:?}",admitted?),"address":address})
    );
    Ok(())
}

fn main() -> Result<(), Failure> {
    // Windows WebView/SSH startup stacks are not the node's async worker stack.
    std::thread::Builder::new().stack_size(8*1024*1024).spawn(|| {
        let runtime=tokio::runtime::Builder::new_multi_thread().worker_threads(2)
            .thread_stack_size(8*1024*1024).enable_all().build()?;
        runtime.block_on(async {
            let args:Vec<_>=std::env::args_os().skip(1).collect();
            if args.len()!=3 {return Err("usage: miopunch-network-demo bootstrap|invite|join|resume|run NODE_DIR INVITATION_OR_CONFIG".into());}
            let directory=Path::new(&args[1]);let argument=Path::new(&args[2]);
            match args[0].to_str() {
                Some("bootstrap")=>bootstrap(directory,argument,true).await,
                Some("invite")=>bootstrap(directory,argument,false).await,
                Some("join")=>join(directory,argument,false).await,
                Some("resume")=>join(directory,argument,true).await,
                Some("run")=>run(directory,argument).await,
                _=>Err("unknown network demo operation".into()),
            }
        })
    })?.join().map_err(|_|"network demo worker panicked")?
}

#[cfg(all(test, target_os = "linux"))]
#[path = "network_demo/network_notification_test.rs"]
mod network_notification_test;

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) struct FixtureDirectory(pub(super) PathBuf);

    impl FixtureDirectory {
        pub(super) fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("miopunch-demo-{}", rand::random::<u64>()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for FixtureDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    async fn admitted_nodes(path: &Path) -> (NativeNetwork, NativeNetwork) {
        use miopunch_native_probe::manual_admission::ManualExchange;

        let mut server = NativeNetwork::create(path.join("server")).await.unwrap();
        let code = server
            .issue_precode(now_ms() / 1000 + 300, Some(1))
            .await
            .unwrap();
        let mut client = NativeNetwork::join(path.join("client"), code.invitation())
            .await
            .unwrap();
        for node in [&client, &server] {
            let mut config = node.resource_configuration().unwrap();
            config.replacement = Some(vec![]);
            node.save_resource_configuration(&config).unwrap();
        }
        let listener = server.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
        listener.set_alpns(vec![PRECODE_REQUEST_ALPN.to_vec()]);
        let dialer = client.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
        let (requester, issuer) = tokio::join!(
            dialer.connect_control(listener.addr(), PRECODE_REQUEST_ALPN),
            async {
                listener
                    .accept()
                    .await
                    .unwrap()
                    .accept_control()
                    .unwrap()
                    .await
            },
        );
        let (requester, issuer) = (Arc::new(requester.unwrap()), Arc::new(issuer.unwrap()));
        let ((), served) = tokio::join!(
            async {
                let ManualExchange::Materials(material) = client
                    .request_precode_admission(&requester, &code)
                    .await
                    .unwrap()
                else {
                    panic!("precode did not grant membership");
                };
                client.complete_manual_admission(material).await.unwrap();
            },
            server.serve_precode_admission(&issuer),
        );
        served.unwrap();
        tokio::join!(dialer.close(), listener.close());
        (client, server)
    }

    async fn probe_pair(
        path: &Path,
    ) -> (
        MaintainedNetwork,
        MaintainedNetwork,
        MemberConnection,
        MemberConnection,
    ) {
        let (client, server) = admitted_nodes(path).await;
        let client = client
            .maintain_application_network(Ipv4Addr::LOCALHOST, |builder| builder)
            .await
            .unwrap();
        let server = server
            .maintain_application_network(Ipv4Addr::LOCALHOST, |builder| builder)
            .await
            .unwrap();
        let (outgoing, incoming) =
            tokio::join!(client.connect_member(server.addr()), server.accept_member());
        (
            client,
            server,
            outgoing.unwrap(),
            incoming.unwrap().unwrap(),
        )
    }

    pub(super) fn probe_server() -> ProbeServer {
        ProbeServer {
            cancellation: CancellationToken::new(),
            observations: Arc::new(Mutex::new(Observations {
                local: json!({}),
                peers: BTreeMap::new(),
                connections: BTreeMap::new(),
                events: VecDeque::new(),
                remote: BTreeMap::new(),
            })),
        }
    }

    #[tokio::test]
    async fn single_stream_failure_preserves_other_requests_and_stop() {
        timeout(Duration::from_secs(12), async {
            let directory = FixtureDirectory::new();
            let (client_host, server_host, client, server) = probe_pair(&directory.0).await;
            let delivery = server.stable_id();
            let origin = client.stable_id();
            let handler = probe_server();
            handler.observations.lock().unwrap().local["id"] =
                json!(client.remote_id().to_string());
            let abort_one_stream = async {
                let mut stream = client.open_bi().await.unwrap();
                let (send, recv) = stream.streams();
                send.write_all(b"x").await.unwrap();
                let stream_id = u64::from(send.id());
                timeout(Duration::from_secs(2), async {
                    loop {
                        if handler
                            .observations
                            .lock()
                            .unwrap()
                            .events
                            .iter()
                            .any(|event| {
                                event["kind"] == "probe-stream-accepted"
                                    && event["detail"]["streamId"] == stream_id
                            })
                        {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .unwrap();
                send.reset(73u32.into()).unwrap();
                assert!(recv.read_to_end(MAX_REPLY).await.unwrap().is_empty());

                let mut stream = client.open_bi().await.unwrap();
                let (send, recv) = stream.streams();
                send.write_all(b"malform").await.unwrap();
                send.finish().unwrap();
                assert!(recv.read_to_end(MAX_REPLY).await.unwrap().is_empty());

                let mut stalled_stream = client.open_bi().await.unwrap();
                let (stalled, recv) = stalled_stream.streams();
                stalled.write_all(b"x").await.unwrap();
                // This reply must arrive while the unrelated request is still
                // waiting for its five-second receive deadline.
                timeout(
                    Duration::from_secs(2),
                    exchange(&client, 42, &mut RequestProgress::default()),
                )
                .await
                .unwrap()
                .unwrap();
                assert!(recv.read_to_end(MAX_REPLY).await.unwrap().is_empty());
                drop(stalled_stream);
                exchange(&client, 43, &mut RequestProgress::default())
                    .await
                    .unwrap();
                assert_eq!(client.stable_id(), origin);
                assert!(client.close_reason().is_none());
                let mut outstanding = client.open_bi().await.unwrap();
                let (send, _) = outstanding.streams();
                send.write_all(b"x").await.unwrap();
                // Completing a later stream proves the handler has accepted
                // this earlier one, which still has no FIN.
                exchange(&client, 44, &mut RequestProgress::default())
                    .await
                    .unwrap();
                handler.cancellation.cancel();
            };
            tokio::join!(handler.accept(server), abort_one_stream);
            assert!(client.close_reason().is_none());
            tokio::join!(client_host.close(), server_host.close());
            let state = handler.observations.lock().unwrap();
            let events = state
                .events
                .iter()
                .filter(|event| {
                    !matches!(
                        event["kind"].as_str(),
                        Some("probe-stream-accepted" | "probe-request-read" | "probe-reply-queued")
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(events.len(), 4);
            for (event, reason) in
                events[..3]
                    .iter()
                    .zip(["stream_read", "stream_decode", "stream_read"])
            {
                assert_eq!(event["kind"], "probe-failed");
                assert_eq!(event["detail"]["reason"], reason);
            }
            assert_eq!(events[3]["kind"], "connection-ended");
            assert_eq!(events[3]["detail"]["reason"], "stopped");
            assert_eq!(events[3]["detail"]["owner"], "service");
            assert_eq!(events[3]["detail"]["closeReason"], "open");
            for event in events {
                assert_eq!(event["detail"]["stableId"], delivery);
                println!("{event}");
            }
        })
        .await
        .unwrap();
    }

    async fn receive_probe(connection: &MemberConnection) -> (u64, MemberStream) {
        let mut stream = connection.accept_bi().await.unwrap();
        let bytes = stream.streams().1.read_to_end(9).await.unwrap();
        assert_eq!(bytes[0], PROBE_SERVICE);
        (u64::from_be_bytes(bytes[1..].try_into().unwrap()), stream)
    }

    async fn reply_to_probe(identity: &str, sequence: u64, mut stream: MemberStream) {
        let data = json!({"sequence":sequence,"snapshot":{"id":identity}});
        let (reply, _) = stream.streams();
        reply.write_all(data.to_string().as_bytes()).await.unwrap();
        reply.finish().unwrap();
    }

    #[tokio::test]
    async fn heartbeat_learns_contact_from_a_full_address_snapshot() {
        use iroh::{TransportAddr, address_lookup::memory::MemoryLookup};
        use miopunch_native_probe::tcp_transport::TcpTransport;

        let directory = FixtureDirectory::new();
        let (client, server, outgoing, incoming) = probe_pair(&directory.0).await;
        let client = Arc::new(client);
        let probe = probe_server();
        let peer = Peer {
            name: "server".into(),
            address: server.addr(),
        };
        let public_ip = TransportAddr::Ip("182.48.107.243:8529".parse().unwrap());
        let relay = TransportAddr::Relay("https://relay.example.test/".parse().unwrap());
        let tcp = TransportAddr::Custom(TcpTransport::address((Ipv4Addr::LOCALHOST, 4100)));
        let advertised = EndpointAddr::new(peer.address.id).with_addrs(
            (1..=9)
                .map(|host| TransportAddr::Ip((Ipv4Addr::new(10, 0, 0, host), 4100).into()))
                .chain([TransportAddr::Ip("100.64.0.1:4100".parse().unwrap())])
                .chain([public_ip.clone(), relay.clone(), tcp.clone()]),
        );
        assert_eq!(advertised.addrs.len(), 13);
        {
            let mut state = probe.observations.lock().unwrap();
            state.local = json!({"id": advertised.id.to_string(), "address": advertised});
            state
                .peers
                .insert(peer.name.clone(), json!({"id": advertised.id}));
        }
        let observer = tokio::spawn(observe_peer(
            client.clone(),
            peer,
            probe.observations.clone(),
            probe.cancellation.clone(),
        ));
        let serving = tokio::spawn({
            let probe = probe.clone();
            async move { probe.accept(incoming).await }
        });

        let snapshot = timeout(Duration::from_secs(5), async {
            loop {
                if let Some(snapshot) = probe.observations.lock().unwrap().remote.get("server") {
                    break snapshot["snapshot"]["address"].clone();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("read a heartbeat reply from the admitted peer");
        assert_eq!(snapshot, json!(advertised));
        assert!(
            probe
                .observations
                .lock()
                .unwrap()
                .events
                .iter()
                .all(|event| event["kind"] != "contact-learning-failed"),
            "a full heartbeat snapshot should yield an accepted contact hint"
        );

        let learned = timeout(Duration::from_secs(2), async {
            let lookup = client.endpoint().address_lookup().unwrap();
            loop {
                let mut results = std::pin::pin!(lookup.resolve(advertised.id));
                while let Some(result) = results.next().await {
                    if let Ok(Ok(item)) = result
                        && item.provenance() == MemoryLookup::PROVENANCE
                        && item.addrs().any(|address| address == &tcp)
                    {
                        return item.into_endpoint_addr();
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the maintenance task published the heartbeat contact hint");
        assert_eq!(learned.addrs.len(), 8);
        assert!(learned.addrs.contains(&public_ip));
        assert!(learned.addrs.contains(&relay));
        assert!(learned.addrs.contains(&tcp));

        probe.cancellation.cancel();
        observer.await.unwrap();
        serving.await.unwrap();
        drop(outgoing);
        tokio::join!(
            Arc::try_unwrap(client).ok().unwrap().close(),
            server.close()
        );
    }

    #[tokio::test]
    async fn periodic_probes_reuse_the_standby_member_after_foreground_release() {
        let directory = FixtureDirectory::new();
        let (client, server) = admitted_nodes(&directory.0).await;
        for node in [&client, &server] {
            let mut config = node.resource_configuration().unwrap();
            config.maintenance.exploration_interval_ms = 3_000;
            node.save_resource_configuration(&config).unwrap();
        }
        let client = Arc::new(
            client
                .maintain_application_network(Ipv4Addr::LOCALHOST, |b| b)
                .await
                .unwrap(),
        );
        let server = server
            .maintain_application_network(Ipv4Addr::LOCALHOST, |b| b)
            .await
            .unwrap();
        let probe = probe_server();
        let peer = Peer {
            name: "server".into(),
            address: server.addr(),
        };
        probe
            .observations
            .lock()
            .unwrap()
            .peers
            .insert(peer.name.clone(), json!({"id":peer.address.id}));
        let observer = tokio::spawn(observe_peer(
            client.clone(),
            peer,
            probe.observations.clone(),
            probe.cancellation.clone(),
        ));
        let original = timeout(Duration::from_secs(5), server.accept_member())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        probe.observations.lock().unwrap().local["id"] = json!(server.endpoint().id().to_string());
        let serving = tokio::spawn({
            let probe = probe.clone();
            let original = original.clone();
            async move { probe.accept(original).await }
        });
        let foreground = original.open_bi().await.unwrap();
        assert!(foreground.is_business());
        let local = timeout(Duration::from_secs(2), async {
            loop {
                let connection = probe
                    .observations
                    .lock()
                    .unwrap()
                    .connections
                    .get("server")
                    .cloned();
                if let Some(connection) = connection {
                    break connection;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let incoming_foreground = local.accept_bi().await.unwrap();
        assert!(incoming_foreground.is_business());
        timeout(Duration::from_secs(6), async {
            loop {
                let successes = probe.observations.lock().unwrap().peers["server"]["successes"]
                    .as_u64()
                    .unwrap_or(0);
                if successes >= 4 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert!(original.close_reason().is_none());
        let successes_before_idle = probe.observations.lock().unwrap().peers["server"]["successes"]
            .as_u64()
            .unwrap();
        let idle = Instant::now();
        drop((foreground, incoming_foreground));
        tokio::time::sleep(Duration::from_millis(3_700)).await;
        assert!(original.close_reason().is_none());
        assert_eq!(
            client
                .connect_member(server.addr())
                .await
                .unwrap()
                .stable_id(),
            local.stable_id()
        );
        let successes = probe.observations.lock().unwrap().peers["server"]["successes"]
            .as_u64()
            .unwrap();
        assert!(successes >= successes_before_idle + 2);
        println!(
            "MIO155 demo idle {:?} original={} backgroundReplies={}",
            idle.elapsed(),
            original.stable_id(),
            successes - successes_before_idle
        );
        probe.cancellation.cancel();
        observer.await.unwrap();
        serving.await.unwrap();
        tokio::join!(
            Arc::try_unwrap(client).ok().unwrap().close(),
            server.close()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn heartbeat_timeout_preserves_member_until_actual_close() {
        let directory = FixtureDirectory::new();
        timeout(Duration::from_secs(30), async {
            let (client, server) = admitted_nodes(&directory.0).await;
            let server = server
                .maintain_application_network(Ipv4Addr::LOCALHOST, |builder| builder)
                .await
                .unwrap();
            let client = Arc::new(
                client
                    .maintain_application_network(Ipv4Addr::LOCALHOST, |builder| builder)
                    .await
                    .unwrap(),
            );
            let identity = server.endpoint().id().to_string();
            let peer = Peer {
                name: "server".into(),
                address: server.addr(),
            };
            let handler = probe_server();
            let state = handler.observations;
            let stop = handler.cancellation;
            state
                .lock()
                .unwrap()
                .peers
                .insert(peer.name.clone(), json!({"id":identity}));
            let observer = tokio::spawn(observe_peer(
                client.clone(),
                peer,
                state.clone(),
                stop.clone(),
            ));

            let original = server.accept_member().await.unwrap().unwrap();
            let (first_sequence, mut reply) = receive_probe(&original).await;
            let local = state.lock().unwrap().connections["server"].clone();
            let original_id = local.stable_id();
            assert_eq!(first_sequence, 1);
            assert_eq!(
                reply.streams().0.stopped().await.unwrap(),
                Some(0u32.into())
            );
            assert!(original.close_reason().is_none());
            drop(reply);

            let (sequence, reply) = receive_probe(&original).await;
            assert_eq!(sequence, 2);
            reply_to_probe(&identity, sequence, reply).await;
            let (sequence, _reply) = receive_probe(&original).await;
            assert_eq!(sequence, 3);
            {
                let state = state.lock().unwrap();
                assert_eq!(state.peers["server"]["failures"], 1);
                assert_eq!(state.peers["server"]["successes"], 1);
                assert_eq!(state.peers["server"]["connections"], 1);
                let deadline = state
                    .events
                    .iter()
                    .find(|e| e["kind"] == "heartbeat-failed")
                    .unwrap();
                assert_eq!(deadline["detail"]["error"], "heartbeat deadline (3s)");
                assert_eq!(deadline["detail"]["stableId"], original_id);
                let restored = state
                    .events
                    .iter()
                    .find(|e| e["kind"] == "business-restored")
                    .unwrap();
                assert_eq!(restored["detail"]["stableId"], original_id);
                assert!(state.events.iter().all(|e| e["kind"] != "path-repair"));
                println!("{deadline}\n{restored}");
            }

            original.test_close(4, b"test peer stopped original member");
            let reconnected = server.accept_member().await.unwrap().unwrap();
            assert_ne!(reconnected.stable_id(), original.stable_id());
            let (sequence, reply) = receive_probe(&reconnected).await;
            reply_to_probe(&identity, sequence, reply).await;
            let (_sequence, _reply) = receive_probe(&reconnected).await;
            let new_id = state.lock().unwrap().connections["server"].stable_id();
            assert_ne!(original_id, new_id);
            assert_eq!(state.lock().unwrap().peers["server"]["successes"], 2);
            stop.cancel();
            observer.await.unwrap();
            assert!(reconnected.close_reason().is_none());
            {
                let state = state.lock().unwrap();
                assert!(state.connections.is_empty());
                let ended: Vec<_> = state
                    .events
                    .iter()
                    .filter(|e| e["kind"] == "connection-ended")
                    .collect();
                assert_eq!(ended.len(), 2);
                assert_eq!(ended[0]["detail"]["owner"], "peer");
                assert_eq!(ended[0]["detail"]["stableId"], original_id);
                assert_eq!(ended[1]["detail"]["owner"], "service");
                assert_eq!(ended[1]["detail"]["stableId"], new_id);
                println!("{}\n{}", ended[0], ended[1]);
            }
            drop(local);
            tokio::join!(
                Arc::try_unwrap(client).ok().unwrap().close(),
                server.close()
            );
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn repeated_demo_close_does_not_reattribute_existing_closure() {
        timeout(Duration::from_secs(8), async {
            for (peer_close, expected_origin, expected_reason) in [
                (true, "peer", "peer_application_close"),
                (false, "unknown", "locally_closed"),
            ] {
                let directory = FixtureDirectory::new();
                let (client_host, server_host, client, server) = probe_pair(&directory.0).await;
                if peer_close {
                    client.test_close(4, b"arbitrary peer detail");
                } else {
                    server.test_close(4, b"unobserved local caller");
                }
                server.closed().await;
                let handler = probe_server();
                handler.accept(server).await;
                tokio::join!(client_host.close(), server_host.close());
                let state = handler.observations.lock().unwrap();
                let end = state.events.back().unwrap();
                assert_eq!(end["kind"], "connection-ended");
                assert_eq!(end["detail"]["owner"], expected_origin);
                assert_eq!(end["detail"]["closeReason"], expected_reason);
                println!("{end}");
            }
        })
        .await
        .unwrap();
    }
}
