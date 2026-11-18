//! Operator demo over the actual maintained SDK node. HTTP is loopback-only;
//! all inter-device samples and remote observations use authenticated member QUIC.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::{
    collections::{BTreeMap, VecDeque},
    error::Error,
    fmt,
    io::{Read, Seek, SeekFrom},
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use iroh::{
    EndpointAddr,
    endpoint::{Connection, WeakConnectionHandle},
    protocol::{AcceptError, ProtocolHandler},
};
use miopunch_native_probe::{
    admission_gate::PRECODE_REQUEST_ALPN,
    admission_gate::ProtocolScope,
    client_commands::save_private,
    client_host::HostLease,
    client_invitation::BootstrapContact,
    connection_coordinator::{MEMBER_ALPN, connection_diagnostic},
    control_protocol::{AuthenticatedControl, OwnedControl},
    native_client::{ClientInvitation, NativeClient},
    native_network::NativeNetwork,
    network_maintenance::{MaintainedNetwork, MaintenancePolicy},
    resources::{ResourceConfig, ResourceUse},
};
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
// Use the SDK member-business wire so this demo exercises unified admission
// and path preparation before the retained application connection is released.
const ALPN: &[u8] = MEMBER_ALPN;
const MAX_REPLY: usize = 256 * 1024;

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
    connections: BTreeMap<String, WeakConnectionHandle>,
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
// in this initial hint; the full live address list remains in diagnostics.
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

impl fmt::Debug for ProbeServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NetworkDemoMemberProbe")
    }
}

impl ProtocolHandler for ProbeServer {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        println!(
            "{}",
            json!({"event":"network-demo-incoming-ready","at":now_ms(),
            "peer":connection.remote_id().to_string(),"transport":path_view(&connection)})
        );
        let mut ended_by = "accept_stream";
        let result: Result<(), Failure> = async {
            loop {
                ended_by = "accept_stream";
                let stream = tokio::select! {
                    _ = self.cancellation.cancelled() => {
                        ended_by = "stopped";
                        break;
                    },
                    stream = connection.accept_bi() => stream?,
                };
                let (mut send, mut recv) = stream;
                ended_by = "stream_read";
                let request = timeout(Duration::from_secs(5), recv.read_to_end(8)).await??;
                ended_by = "stream_decode";
                let sequence = u64::from_be_bytes(request.try_into().map_err(|_| "invalid probe")?);
                let local = self.observations.lock().unwrap().local_view();
                ended_by = "stream_response";
                let response = serde_json::to_vec(&json!({"sequence":sequence,"snapshot":local}))?;
                if response.len() > MAX_REPLY {
                    return Err("probe snapshot exceeds bound".into());
                }
                ended_by = "stream_write";
                timeout(Duration::from_secs(5), send.write_all(&response)).await??;
                ended_by = "stream_finish";
                send.finish()?;
            }
            Ok(())
        }
        .await;
        let peer = connection.remote_id().to_string();
        if result.is_err() {
            self.observations.lock().unwrap().event(
                "probe-failed",
                json!({
                    "peer":peer,"stableId":connection.stable_id(),"reason":ended_by
                }),
            );
        }
        close_probe_connection(
            &self.observations,
            &connection,
            &peer,
            "demo_receive",
            ended_by,
            b"network demo probe ended",
        );
        Ok(())
    }
}

fn close_probe_connection(
    state: &Mutex<Observations>,
    connection: &Connection,
    peer: &str,
    owner: &'static str,
    cause: &'static str,
    wire_reason: &[u8],
) {
    let prior = connection.close_with_previous_reason(0u32.into(), wire_reason);
    state.lock().unwrap().event(
        "connection-close-invoked",
        json!({
            "peer":peer,"stableId":connection.stable_id(),"owner":owner,"reason":cause,
            "priorCloseReason":connection_diagnostic::close_reason(prior.as_ref()),
        }),
    );
    let origin = prior.as_ref().map_or(owner, |reason| {
        connection_diagnostic::close_origin(Some(reason))
    });
    let end_reason = prior
        .as_ref()
        .unwrap_or(&iroh::endpoint::ConnectionError::LocallyClosed);
    state.lock().unwrap().event(
        "connection-ended",
        json!({
            "peer":peer,"stableId":connection.stable_id(),"owner":origin,"reason":cause,
            "closeReason":connection_diagnostic::close_reason(Some(end_reason)),
            "priorCloseReason":connection_diagnostic::close_reason(prior.as_ref()),
        }),
    );
}

fn path_view(connection: &Connection) -> Value {
    json!({"stableId":connection.stable_id(),"alpn":String::from_utf8_lossy(connection.alpn()),"open":connection.close_reason().is_none(),
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

async fn exchange(connection: &Connection, sequence: u64) -> Result<Value, Failure> {
    let (mut send, mut recv) = connection.open_bi().await?;
    send.write_all(&sequence.to_be_bytes()).await?;
    send.finish()?;
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
            state.event("connect-start", json!({"peer":peer.name,"alpn":String::from_utf8_lossy(ALPN),"target":peer.address}));
        }
        let connected = tokio::select! {
            _ = cancellation.cancelled() => return,
            result = network.connect(peer.address.clone(),ALPN) => result,
        };
        match connected {
            Ok((connection, activity)) => {
                // These are finite diagnostic exchanges on a retained session.
                // Idle time between samples must remain available to the SDK's
                // membership synchronization and path-maintenance scheduler.
                drop(activity);
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
                        .insert(peer.name.clone(), connection.weak_handle());
                    let entry = state.peers.get_mut(&peer.name).unwrap();
                    entry["phase"] = json!("checking");
                    entry["connections"] = json!(connections);
                    entry["transport"] = path_view(&connection);
                    state.event(
                        "connection-open",
                        json!({"peer":peer.name,"stableId":connection.stable_id()}),
                    );
                }
                loop {
                    sequence += 1;
                    let start = Instant::now();
                    let activity = network.business_activity();
                    let result = tokio::select! {
                        _ = cancellation.cancelled() => {
                            close_probe_connection(&state, &connection, &peer.name, "demo_stop",
                                "stopped", b"demo stopped");
                            return;
                        },
                        result = timeout(Duration::from_secs(3),exchange(&connection,sequence)) => result,
                    };
                    drop(activity);
                    match result {
                        Ok(Ok(remote)) => {
                            successes += 1;
                            let mut state = state.lock().unwrap();
                            if let Some(since) = unavailable_since.take() {
                                let elapsed = since.elapsed().as_millis() as u64;
                                state.peers.get_mut(&peer.name).unwrap()["lastRecoveryMs"] =
                                    json!(elapsed);
                                state.event(
                                    "business-restored",
                                    json!({"peer":peer.name,"elapsedMs":elapsed,
                                    "stableId":connection.stable_id(),"connections":connections}),
                                );
                            }
                            let entry = state.peers.get_mut(&peer.name).unwrap();
                            entry["phase"] = json!("online");
                            entry["lastOkAt"] = json!(now_ms());
                            entry["roundTripMs"] = json!(start.elapsed().as_secs_f64() * 1000.);
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
                                    "stableId":connection.stable_id()}),
                                );
                            }
                            if connection.close_reason().is_some() {
                                break;
                            }
                            // The existing SDK owns path repair and its absolute budget.
                            let repair = tokio::select! {
                                _ = cancellation.cancelled() => {
                                    close_probe_connection(&state, &connection, &peer.name, "demo_stop",
                                        "stopped", b"demo stopped");
                                    return;
                                },
                                result = network.repair_member_connection_until(&connection,peer.address.clone(),
                                    n0_future::time::Instant::now()+Duration::from_secs(3)) => result,
                            };
                            state.lock().unwrap().event(
                                "path-repair",
                                json!({"peer":peer.name,
                                "stableId":connection.stable_id(),"ok":repair.is_ok(),
                                "error":repair.err().map(|e|e.to_string())}),
                            );
                            if connection.close_reason().is_some() {
                                break;
                            }
                        }
                    }
                    tokio::select! {
                        _ = cancellation.cancelled() => {
                            close_probe_connection(&state, &connection, &peer.name, "demo_stop",
                                "stopped", b"demo stopped");
                            return;
                        },
                        _ = tokio::time::sleep_until(start+Duration::from_secs(1)) => {},
                    }
                }
                close_probe_connection(
                    &state,
                    &connection,
                    &peer.name,
                    "demo_reconnect",
                    "reconnect_cleanup",
                    b"demo reconnecting after terminal connection",
                );
                let mut state = state.lock().unwrap();
                state.connections.remove(&peer.name);
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
            network.network_changed().await?;
            last_address = address.clone();
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
                .filter_map(|(name, weak)| {
                    weak.upgrade()
                        .map(|connection| (name.clone(), path_view(&connection)))
                })
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
    node.admission_gate()
        .register_protocol(ALPN, ProtocolScope::Member);
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
        node.maintain_network_with(Ipv4Addr::UNSPECIFIED, |router| router.accept(ALPN, handler))
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

#[cfg(test)]
mod tests {
    use super::*;

    async fn probe_pair() -> (iroh::Endpoint, iroh::Endpoint, Connection, Connection) {
        let bind = || {
            iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
                .clear_relay_transports()
                .bind_addr((Ipv4Addr::LOCALHOST, 0))
                .unwrap()
                .alpns(vec![ALPN.to_vec()])
                .bind()
        };
        let client = bind().await.unwrap();
        let server = bind().await.unwrap();
        let (outgoing, incoming) = tokio::join!(client.connect(server.addr(), ALPN), async {
            server.accept().await.unwrap().await
        },);
        (client, server, outgoing.unwrap(), incoming.unwrap())
    }

    fn probe_server() -> ProbeServer {
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
    async fn single_stream_failure_close_baseline() {
        timeout(Duration::from_secs(8), async {
            let (_client, _server, client, server) = probe_pair().await;
            let delivery = server.stable_id();
            let handler = probe_server();
            let abort_one_stream = async {
                let (mut send, _recv) = client.open_bi().await.unwrap();
                send.write_all(b"x").await.unwrap();
                send.reset(73u32.into()).unwrap();
                client.closed().await
            };
            let (served, closure) = tokio::join!(handler.accept(server), abort_one_stream);
            served.unwrap();
            assert!(matches!(
                closure,
                iroh::endpoint::ConnectionError::ApplicationClosed(_)
            ));
            let state = handler.observations.lock().unwrap();
            let events = state.events.iter().collect::<Vec<_>>();
            assert_eq!(events.len(), 3);
            assert_eq!(events[0]["kind"], "probe-failed");
            assert_eq!(events[0]["detail"]["reason"], "stream_read");
            assert_eq!(events[1]["kind"], "connection-close-invoked");
            assert_eq!(events[1]["detail"]["priorCloseReason"], "open");
            assert_eq!(events[2]["kind"], "connection-ended");
            assert_eq!(events[2]["detail"]["owner"], "demo_receive");
            assert_eq!(events[2]["detail"]["closeReason"], "locally_closed");
            for event in events {
                assert_eq!(event["detail"]["stableId"], delivery);
                println!("{event}");
            }
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
                let (_client, _server, client, server) = probe_pair().await;
                if peer_close {
                    client.close(4u32.into(), b"arbitrary peer detail");
                } else {
                    server.close(4u32.into(), b"unobserved local caller");
                }
                server.closed().await;
                let handler = probe_server();
                handler.accept(server).await.unwrap();
                let state = handler.observations.lock().unwrap();
                let end = state.events.back().unwrap();
                assert_eq!(end["kind"], "connection-ended");
                assert_eq!(end["detail"]["owner"], expected_origin);
                assert_eq!(end["detail"]["priorCloseReason"], expected_reason);
                println!("{end}");
            }
        })
        .await
        .unwrap();
    }
}
