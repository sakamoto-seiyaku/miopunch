//! Reopen genuinely saved members with only the owned MQTT signaling facility.
use std::{error::Error, sync::Arc, time::Duration};

use iroh::endpoint::presets;
use miopunch_native_probe::{
    admission_gate::{ADMISSION_REQUEST_ALPN, PRECODE_REQUEST_ALPN, ProtocolScope},
    mqtt_datagram::MqttFacility,
    mqtt_transport::{MqttTransport, Traffic},
    native_network::NativeNetwork,
    resources::{LanDiscovery, PortMapping, Resource, ResourceUse},
};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};

fn socket_count() -> Option<usize> {
    let entries = std::fs::read_dir("/proc/self/fd").ok()?;
    let mut count = 0;
    for entry in entries {
        let entry = entry.ok()?;
        match std::fs::read_link(entry.path()) {
            Ok(target) if target.to_string_lossy().starts_with("socket:") => count += 1,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return None,
        }
    }
    Some(count)
}

fn startup_observation(
    stage: &str,
    phase: &str,
    endpoint: &iroh::endpoint::RestrictedEndpoint,
    transport: &MqttTransport,
    traffic: &Traffic,
    started: tokio::time::Instant,
    deadline: tokio::time::Instant,
    started_utc_ms: Option<u128>,
    result: Option<&Result<Result<(), std::io::Error>, tokio::time::error::Elapsed>>,
) {
    use n0_watcher::Watcher;
    let home = endpoint.home_relay_status().get();
    let actual = traffic.snapshot();
    let mut details = json!({"tag":"DEBUG-mio76-startup","stage":stage,"phase":phase,
        "startedUtcMs":started_utc_ms,"elapsedMs":started.elapsed().as_millis(),
        "budgetRemainingMs":deadline.saturating_duration_since(tokio::time::Instant::now()).as_millis(),
        "budgetMs":10000,"mqttRouteReady":transport.route().is_ok(),
        "mqttDriverActive":actual.get("driver_active"),"mqttQueuedPackets":actual.get("queued_packets"),
        "mqttSentPackets":actual.get("sent_packets"),"mqttReceivedPackets":actual.get("received_packets"),
        "localPublishedCount":endpoint.addr().relay_urls().count(),
        "localConnectedCount":home.iter().filter(|status|status.is_connected()).count(),
        "localRelayErrorCount":home.iter().filter(|status|status.last_error().is_some()).count(),
        "socketCount":socket_count()});
    if let Some(result) = result {
        details["waitOk"] = json!(matches!(result, Ok(Ok(()))));
        details["outerTimedOut"] = json!(result.is_err());
        details["errorKind"] = match result {
            Err(_) => json!("TimedOut"),
            Ok(Err(error)) => json!(format!("{:?}", error.kind())),
            Ok(Ok(())) => serde_json::Value::Null,
        };
    }
    details
        .as_object_mut()
        .unwrap()
        .retain(|_, value| !value.is_null());
    eprintln!("[DEBUG-mio76-startup] {details}");
}

async fn startup_wait(
    stage: &str,
    endpoint: &iroh::endpoint::RestrictedEndpoint,
    transport: &MqttTransport,
    traffic: &Traffic,
    work: impl std::future::Future<Output = Result<(), std::io::Error>>,
) -> Result<(), Box<dyn Error>> {
    // Keep the existing ten-second ceiling. Diagnostic samples do not refill it.
    let started = tokio::time::Instant::now();
    let deadline = started + Duration::from_secs(10);
    let started_utc_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|value| value.as_millis());
    let work = tokio::time::timeout_at(deadline, work);
    tokio::pin!(work);
    startup_observation(
        stage,
        "begin",
        endpoint,
        transport,
        traffic,
        started,
        deadline,
        started_utc_ms,
        None,
    );
    let samples = [1u64, 4, 8];
    let mut sampled = 0;
    let result = loop {
        let next = started + Duration::from_secs(*samples.get(sampled).unwrap_or(&10));
        tokio::select! {biased;
            result=&mut work=>break result,
            _=tokio::time::sleep_until(next),if sampled<samples.len()=>{
                startup_observation(stage,"progress",endpoint,transport,traffic,started,deadline,started_utc_ms,None);
                sampled+=1;
            }
        }
    };
    startup_observation(
        stage,
        "end",
        endpoint,
        transport,
        traffic,
        started,
        deadline,
        started_utc_ms,
        Some(&result),
    );
    result??;
    Ok(())
}

struct FixtureCleanup {
    node: Arc<tokio::sync::Mutex<NativeNetwork>>,
    assembly: Arc<miopunch_native_probe::resources::ResourceEndpoint>,
    endpoint: iroh::endpoint::RestrictedEndpoint,
    transport: Arc<MqttTransport>,
    rtc_transport: miopunch_native_probe::rtc_transport::RtcTransport,
    traffic: Arc<Traffic>,
    handlers: tokio::task::JoinSet<Result<(), Box<dyn Error + Send + Sync>>>,
    sockets_before: Option<usize>,
}

// Only the controlled fixture's named caller handles live here. No Connection,
// PC, transport, node allowance or peer-wide candidate is owned by this ledger.
struct NamedMemberAttempt {
    cancellation: tokio_util::sync::CancellationToken,
    pending: bool,
    original: Option<iroh::endpoint::WeakConnectionHandle>,
}
type NamedMemberAttempts =
    Arc<std::sync::Mutex<std::collections::BTreeMap<String, NamedMemberAttempt>>>;

struct NamedAttemptGuard {
    ledger: NamedMemberAttempts,
    label: String,
}
impl Drop for NamedAttemptGuard {
    fn drop(&mut self) {
        if let Some(entry) = self.ledger.lock().unwrap().get_mut(&self.label) {
            entry.pending = false;
        }
    }
}
// Fixture-only owned-socket observation; no dependency/config change or SDK getter.
// The selected Android socket2 0.6.5 graph enables `all`.
#[cfg(any(target_os = "android", target_os = "linux"))]
fn observe_owned_udp_mark(selected: std::net::SocketAddrV4) -> serde_json::Value {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    // Raw descriptor input avoids creating a BorrowedFd whose validity could
    // race another task's close. Kernel fcntl atomically duplicates or fails.
    unsafe extern "C" {
        fn fcntl(fd: std::ffi::c_int, command: std::ffi::c_int, ...) -> std::ffi::c_int;
    }
    // Existing Android NDK29 linux/fcntl.h: F_LINUX_SPECIFIC_BASE(1024)+6.
    const DUPFD_CLOEXEC: std::ffi::c_int = 1030;
    let read = || -> std::io::Result<Vec<u32>> {
        let mut directory = std::fs::read_dir("/proc/self/fd")?;
        let entries = directory.by_ref().collect::<Result<Vec<_>, _>>()?;
        let mut marks = Vec::new();
        for entry in entries {
            let original = std::fs::read_link(entry.path())?;
            if !original.to_string_lossy().starts_with("socket:[") {
                continue;
            }
            let fd = entry
                .file_name()
                .to_string_lossy()
                .parse::<std::ffi::c_int>()
                .map_err(|_| std::io::Error::other("invalid own descriptor"))?;
            // Does not create a socket or change the original file description.
            let duplicate = unsafe { fcntl(fd, DUPFD_CLOEXEC, 0 as std::ffi::c_int) };
            if duplicate < 0 {
                return Err(std::io::Error::last_os_error());
            }
            // Successful raw duplication gives this scope sole RAII ownership.
            let owned = unsafe { OwnedFd::from_raw_fd(duplicate) };
            let socket = socket2::Socket::from(owned);
            let duplicated = std::fs::read_link(format!("/proc/self/fd/{}", socket.as_raw_fd()))?;
            let still_original = std::fs::read_link(entry.path())?;
            if original != duplicated || original != still_original {
                return Err(std::io::Error::other(
                    "own socket descriptor changed during duplication",
                ));
            }
            if socket.r#type()? == socket2::Type::DGRAM
                && socket.local_addr()?.as_socket_ipv4() == Some(selected)
            {
                // Actual getsockopt(SOL_SOCKET, SO_MARK). Ok(0) is observed zero;
                // no match/error/multiple descriptors never manufacture zero.
                if socket.protocol()? != Some(socket2::Protocol::UDP) {
                    return Err(std::io::Error::other("matching datagram tuple is not UDP"));
                }
                marks.push(socket.mark()?);
            }
            drop(socket); // Every observer duplicate closes immediately.
        }
        drop(directory);
        Ok(marks)
    };
    match read() {
        Ok(marks) if marks.len() == 1 => serde_json::json!({
            "observed":true,"matchingFdCount":1,
            "socketEvidence":{"operation":"getsockopt(SO_MARK)","success":true,
                "value":marks[0],"pid":std::process::id(),
                "localAddress":selected.ip().to_string(),"localPort":selected.port()},
            "actualSocketUid":null,
            "scope":"unique sampled own-process SOCK_DGRAM exact local tuple; no PC registry",
            "duplicatesClosed":true}),
        Ok(marks) => serde_json::json!({"observed":false,"socketEvidence":null,
            "matchingFdCount":marks.len(),"reason":"exact own UDP tuple absent or ambiguous",
            "duplicatesClosed":true,"actualSocketUid":null}),
        Err(error) => serde_json::json!({"observed":false,"socketEvidence":null,
            "errno":error.raw_os_error(),"reason":"own fd/SO_TYPE/getsockname/SO_MARK observation failed",
            "duplicatesClosed":true,"actualSocketUid":null}),
    }
}

#[cfg(not(any(target_os = "android", target_os = "linux")))]
fn observe_owned_udp_mark(_selected: std::net::SocketAddrV4) -> serde_json::Value {
    serde_json::json!({"observed":false,"socketEvidence":null,
        "reason":"own UDP mark observation unsupported on this platform","actualSocketUid":null})
}

impl FixtureCleanup {
    async fn close(mut self) -> Result<(), Box<dyn Error>> {
        let cleanup_started = std::time::Instant::now();

        self.endpoint.close().await;

        self.handlers.abort_all();
        while self.handlers.join_next().await.is_some() {}
        drop(self.handlers);

        drop(self.assembly);
        drop(self.endpoint);

        let mut status_closed = false;
        let mut driver_joined = false;
        // The original one-second transport cleanup ceiling includes both facts.
        let transport_result = tokio::time::timeout(Duration::from_secs(1), async {
            self.transport.wait_closed().await?;
            status_closed = true;

            self.transport.wait_driver_closed().await?;
            driver_joined = true;

            Ok::<_, std::io::Error>(())
        })
        .await;

        let node_closed = if let Ok(node) = Arc::try_unwrap(self.node) {
            node.into_inner().close().await;
            true
        } else {
            false
        };

        let sockets_immediate = socket_count();
        let rtc_cleanup_started = std::time::Instant::now();

        while (self.rtc_transport.active_associations() != 0
            || self.rtc_transport.active_pumps() != 0)
            && rtc_cleanup_started.elapsed() < Duration::from_secs(2)
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;

        let active_associations = self.rtc_transport.active_associations();
        let active_pumps = self.rtc_transport.active_pumps();
        let cleanup_ok = matches!(&transport_result, Ok(Ok(())))
            && node_closed
            && active_associations == 0
            && active_pumps == 0;
        println!(
            "{}",
            json!({"event":"closed","traffic":self.traffic.snapshot(),
            "cleanupMs":cleanup_started.elapsed().as_millis(),"socketsBefore":self.sockets_before,
            "socketsImmediate":sockets_immediate,"socketsLate":socket_count(),
            "endpointClosed":true,"nodeClosed":node_closed,"mqttStatusClosed":status_closed,
            "mqttDriverJoined":driver_joined,"cleanupSucceeded":cleanup_ok,
            "activeRtcAssociations":active_associations,"activeRtcPumps":active_pumps})
        );
        transport_result??;
        if !node_closed {
            return Err("fixture node still owned".into());
        }
        if active_associations != 0 || active_pumps != 0 {
            return Err("owned RTC cleanup did not complete".into());
        }
        Ok(())
    }
}

fn connection_paths(connection: &iroh::endpoint::Connection) -> Vec<serde_json::Value> {
    connection.paths().iter().map(|path| {
        let stats=path.stats();
        json!({"pathId":path.id().to_string(),"selected":path.is_selected(),"isRelay":path.is_relay(),"isIp":path.is_ip(),
            "isRtc":matches!(path.remote_addr(),iroh::TransportAddr::Custom(addr) if addr.id()==miopunch_native_probe::rtc_transport::TRANSPORT_ID),
            "isSignal":matches!(path.remote_addr(),iroh::TransportAddr::Custom(addr) if matches!(addr.id(),miopunch_native_probe::mqtt_transport::TRANSPORT_ID | miopunch_native_probe::irc_transport::TRANSPORT_ID)),
            "sentBytes":stats.udp_tx.bytes,"receivedBytes":stats.udp_rx.bytes,"streamFramesSent":stats.frame_tx.stream,"streamFramesReceived":stats.frame_rx.stream,
            "datagramFramesSent":stats.frame_tx.datagram,"datagramFramesReceived":stats.frame_rx.datagram})
    }).collect()
}

struct OwnedFixtureMember(iroh::endpoint::Connection);
impl Drop for OwnedFixtureMember {
    fn drop(&mut self) {
        self.0.close(0u32.into(), b"owned member handler ended");
    }
}

async fn serve_member_streams(
    endpoint: iroh::endpoint::RestrictedEndpoint,
    connection: iroh::endpoint::Connection,
    gate: miopunch_native_probe::admission_gate::AdmissionGate,
    rtc: miopunch_native_probe::rtc_transport::RtcTransport,
    first_exchange: Option<n0_future::time::Instant>,
    label: Option<String>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let owner = OwnedFixtureMember(connection);
    let connection = &owner.0;
    if let Some(deadline) = first_exchange {
        let stable = connection.stable_id();
        let mut before = [0; 32];
        connection
            .export_keying_material(&mut before, b"miopunch/mio74-fixture-session/1", &[])
            .map_err(|_| "actual ordinary M exporter unavailable")?;
        let started = std::time::Instant::now();
        let (matched, stream_id) = n0_future::time::timeout(
            deadline.saturating_duration_since(n0_future::time::Instant::now()),
            async {
                let (mut send, mut recv) = connection.open_bi().await?;
                let stream_id = send.id().to_string();
                let message = b"CURRENT74 native to browser original M exchange";
                send.write_all(message).await?;
                send.finish()?;
                let reply = recv.read_to_end(65536).await?;
                Ok::<_, Box<dyn Error + Send + Sync>>((reply == message, stream_id))
            },
        )
        .await??;
        let mut after = [0; 32];
        let unchanged = connection
            .export_keying_material(&mut after, b"miopunch/mio74-fixture-session/1", &[])
            .is_ok()
            && before == after;
        println!(
            "{}",
            json!({"event":"member-native-exchange","label":label,"matched":matched,"streamId":stream_id,
            "stableId":stable,"stableUnchanged":connection.stable_id()==stable,"bindingUnchanged":unchanged,
            "memberAuthorized":gate.allows(connection,ProtocolScope::Member),"ordinaryStream":true,
            "elapsedMs":started.elapsed().as_millis(),"paths":connection_paths(connection)})
        );
        if !matched || !unchanged {
            return Err("actual native to browser stream mismatch".into());
        }
    }
    let mut streams = tokio::task::JoinSet::new();
    let result = loop {
        let stream = tokio::select! {
            _=connection.closed()=>break Ok(()),
            stream=connection.accept_bi()=>stream,
            Some(_)=streams.join_next(),if !streams.is_empty()=>continue,
        };
        let (send, recv) = match stream {
            Ok(stream) => stream,
            Err(error) => break Err(error.into()),
        };
        streams.spawn(miopunch_native_probe::rtc_member::serve_stream(
            endpoint.id(),
            connection.clone(),
            gate.clone(),
            rtc.clone(),
            send,
            recv,
        ));
    };
    streams.abort_all();
    while streams.join_next().await.is_some() {}
    result
}

struct FixtureOptions<'a> {
    admission: bool,
    unified_member: bool,
    permitted_relay: Option<&'a str>,
    ice_profile: Option<&'a str>,
}

fn fixture_options(args: &[String]) -> Result<FixtureOptions<'_>, &'static str> {
    const USAGE: &str = "usage: browser_wss_member_native DATA_DIR FACILITY_JSON TEST_CA_PEM [admission | unified-member [RELAY_URL]] [--ice-profile FILE]";
    let tail = args.get(4..).ok_or(USAGE)?;
    let (mode, ice_profile) = if tail.len() >= 2 && tail[tail.len() - 2] == "--ice-profile" {
        (&tail[..tail.len() - 2], Some(tail[tail.len() - 1].as_str()))
    } else {
        (tail, None)
    };
    let (admission, unified_member, permitted_relay) = match mode {
        [] => (false, true, None),
        [mode] if mode == "unified-member" => (false, true, None),
        [mode, relay] if mode == "unified-member" => (false, true, Some(relay.as_str())),
        [mode] if mode == "admission" => (true, false, None),
        _ => return Err(USAGE),
    };
    Ok(FixtureOptions {
        admission,
        unified_member,
        permitted_relay,
        ice_profile,
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let sockets_before = socket_count();
    let args: Vec<_> = std::env::args().collect();
    let options = fixture_options(&args)?;
    let admission_mode = options.admission;
    let unified_member = options.unified_member;
    let permitted_relay = options.permitted_relay;
    let ice_profile: miopunch_native_probe::rtc_ice_profile::RtcIceProfile =
        if let Some(path) = options.ice_profile {
            serde_json::from_slice(&std::fs::read(path)?)?
        } else {
            Default::default()
        };
    ice_profile.validate()?;
    // Preparation must have persisted the original identity and membership.
    let node = if admission_mode && !std::path::Path::new(&args[1]).exists() {
        NativeNetwork::create(&args[1]).await?
    } else {
        NativeNetwork::open(&args[1]).await?
    };
    let facility: MqttFacility = serde_json::from_slice(&std::fs::read(&args[2])?)?;
    let mut resources = node.resource_configuration()?;
    resources.mqtt_facilities = vec![facility.clone()];
    ice_profile.check_allowed(
        &resources
            .disabled
            .iter()
            .filter(|(_, usage)| *usage == ResourceUse::Observation)
            .map(|(url, _)| url.clone())
            .collect::<Vec<_>>(),
    )?;
    let mut replacement = vec![Resource {
        url: facility.native_url.clone(),
        uses: [ResourceUse::Signaling].into(),
        limitation: "owned saved-member MQTT/WSS qualification".into(),
    }];
    if let Some(relay) = permitted_relay {
        replacement.push(Resource {
            url: relay.to_owned(),
            uses: [ResourceUse::DataRelay].into(),
            limitation: "owned explicitly permitted common relay path".into(),
        });
    }
    let observation_resources = ice_profile.observation_resources()?;
    for observer in &observation_resources {
        replacement.push(Resource {
            url: observer.clone(),
            uses: [ResourceUse::Observation].into(),
            limitation: "explicit owned RTC observation fixture; no data relay".into(),
        });
    }
    resources.import_trusted_replacement(replacement)?;
    resources.lan_discovery = LanDiscovery::Disabled;
    resources.port_mapping = PortMapping::Disabled;
    resources.udp_direct_disabled = true;
    resources.tcp_direct_disabled = true;
    resources.member_data_relay_disabled = true;
    resources.disabled_uses.insert(ResourceUse::Observation);
    if !ice_profile.stun_servers.is_empty() {
        resources.disabled_uses.remove(&ResourceUse::Observation);
    }
    if observation_resources
        .iter()
        .any(|url| !resources.allows(url, ResourceUse::Observation))
    {
        return Err("explicit RTC observer is not permitted by saved resources".into());
    }
    if permitted_relay.is_none() {
        resources.disabled_uses.insert(ResourceUse::DataRelay);
    } else {
        // This fixture's explicit allowed-relay case can reopen material saved
        // by its no-relay preparation; URL-specific disables remain enforced.
        resources.disabled_uses.remove(&ResourceUse::DataRelay);
    }
    node.save_resource_configuration(&resources)?;
    let mut alpns = Vec::new();
    for alpn in [
        miopunch_native_probe::connection_coordinator::MEMBER_ALPN,
        miopunch_native_probe::connection_coordinator::REPLICA_ALPN,
    ] {
        node.admission_gate()
            .register_protocol(alpn, ProtocolScope::Member);
        alpns.push(alpn.to_vec());
    }
    let traffic = Arc::new(Traffic::default());
    let transport = Arc::new(MqttTransport::from_facility(
        &facility,
        &resources
            .disabled
            .iter()
            .filter(|(_, purpose)| *purpose == ResourceUse::Signaling)
            .map(|(url, _)| url.clone())
            .collect::<Vec<_>>(),
        traffic.clone(),
        if args[3] == "-" {
            None
        } else {
            Some(std::fs::read(&args[3])?)
        },
    )?);
    let mut builder = iroh::Endpoint::builder(presets::Minimal).clear_ip_transports();
    if permitted_relay.is_some() && args[3] != "-" {
        let cert = rustls::pki_types::CertificateDer::from(std::fs::read(
            std::path::Path::new(&args[3]).with_file_name("ca.der"),
        )?);
        builder = builder.ca_tls_config(iroh_relay::tls::CaTlsConfig::custom_roots(vec![cert]));
    }
    alpns.push(miopunch_native_probe::connection_recovery::CONNECTION_RECOVERY_ALPN.to_vec());
    if admission_mode {
        alpns.extend([
            ADMISSION_REQUEST_ALPN.to_vec(),
            PRECODE_REQUEST_ALPN.to_vec(),
        ]);
    }
    let rtc_transport =
        miopunch_native_probe::rtc_transport::RtcTransport::with_ice_profile(ice_profile)?;
    let assembly = node
        .bind_configured_endpoint_with_rtc_and_contact(
            builder,
            rtc_transport.clone(),
            transport.clone(),
        )
        .await?;
    let assembly = Arc::new(assembly);
    let endpoint = assembly.endpoint().clone();
    endpoint.set_alpns(alpns);
    let startup = async {
        startup_wait(
            "mqtt-ready",
            &endpoint,
            &transport,
            &traffic,
            transport.wait_ready(),
        )
        .await?;
        if permitted_relay.is_some() {
            startup_wait("relay-online", &endpoint, &transport, &traffic, async {
                endpoint.online().await;
                Ok(())
            })
            .await?;
        }
        Ok::<_, Box<dyn Error>>(())
    }
    .await;
    if let Err(original) = startup {
        let cleanup = FixtureCleanup {
            node: Arc::new(tokio::sync::Mutex::new(node)),
            assembly,
            endpoint,
            transport,
            rtc_transport,
            traffic,
            handlers: tokio::task::JoinSet::new(),
            sockets_before,
        }
        .close()
        .await;
        if cleanup.is_err() {
            eprintln!(
                "[DEBUG-mio76-startup] {}",
                json!({"tag":"DEBUG-mio76-startup","stage":"cleanup","phase":"end","cleanupSucceeded":false})
            );
        }
        // Cleanup cannot replace the original startup error or turn it into success.
        return Err(original);
    }
    println!(
        "{}",
        json!({"event":"ready", "pid":std::process::id(), "id":endpoint.id().to_string(), "route":transport.route()?,
        "knownMembers":node.status().members.iter().map(|(id,_)|id.to_string()).collect::<Vec<_>>(),
        "ipAddresses":endpoint.addr().ip_addrs().count(), "relayAddresses":endpoint.addr().relay_urls().count(),
        "traffic":traffic.snapshot(), "socketCount":socket_count(),"unifiedMember":unified_member,"normalPublicTrust":args[3]=="-",
        "invitation":if admission_mode {Some(hex::encode(node.manual_invitation()?.encode()))} else {None}})
    );
    let node = Arc::new(tokio::sync::Mutex::new(node));
    let mut input = BufReader::new(tokio::io::stdin()).lines();
    let mut handlers = tokio::task::JoinSet::new();
    let named_attempts: NamedMemberAttempts = Arc::default();
    let code_path = std::path::Path::new(&args[1]).join("fixture.precode");
    let mut issued = if admission_mode && code_path.exists() {
        Some(miopunch_native_probe::precode::Precode::decode(
            &std::fs::read(&code_path)?
                .try_into()
                .map_err(|_| "fixture precode length")?,
        )?)
    } else {
        None
    };
    loop {
        tokio::select! {
            line=input.next_line()=>match line?.as_deref() {
                Some("stop")|None=>{

                    break;
                },
                Some("status")=>println!("{}",json!({"event":"status","traffic":traffic.snapshot(),"socketCount":socket_count(),
                    "activeRtcAssociations":rtc_transport.active_associations(),"activeRtcPumps":rtc_transport.active_pumps(),
                    "publishedRtcRoutes":rtc_transport.published_routes_fixture()})),
                Some("member-attempts") if unified_member=>{
                    let attempts=named_attempts.lock().unwrap();
                    println!("{}",json!({"event":"member-attempts","attempts":attempts.iter().map(|(label,entry)|json!({
                        "label":label,"pending":entry.pending,"cancelled":entry.cancellation.is_cancelled(),
                        "originalOpen":entry.original.as_ref().map(|original|original.is_open())})).collect::<Vec<_>>(),
                        "activeRtcAssociations":rtc_transport.active_associations(),"activeRtcPumps":rtc_transport.active_pumps()}));
                },
                Some(command) if unified_member && command.starts_with("member-socket-mark ") => {
                    let result = async {
                        let fields = command.strip_prefix("member-socket-mark ").unwrap()
                            .split_whitespace().collect::<Vec<_>>();
                        if fields.len() != 4 { return Err("exact original M and observed local tuple required"); }
                        let stable = fields[1].parse::<usize>().map_err(|_| "invalid stable ID")?;
                        let selected = std::net::SocketAddrV4::new(
                            fields[2].parse().map_err(|_| "actual IPv4 required")?,
                            fields[3].parse().map_err(|_| "actual UDP port required")?);
                        if selected.ip().is_unspecified() || selected.ip().is_loopback()
                            || selected.ip().is_multicast() || selected.port() == 0 {
                            return Err("actual physical tuple unavailable");
                        }
                        let weak = {
                            let ledger = named_attempts.lock().map_err(|_| "owned ledger unavailable")?;
                            let entry = ledger.get(fields[0]).ok_or("unknown exact owned attempt")?;
                            if entry.pending { return Err("ordinary M has not returned"); }
                            entry.original.clone().ok_or("ordinary M absent")?
                        };
                        let connection = weak.upgrade().ok_or("ordinary M no longer live")?;
                        let gate = node.lock().await.admission_gate();
                        let selected_rtc = || connection.paths().iter().any(|path| path.is_selected()
                            && matches!(path.remote_addr(),iroh::TransportAddr::Custom(address)
                                if address.id()==miopunch_native_probe::rtc_transport::TRANSPORT_ID
                                && iroh::endpoint::transports::CustomSender::is_data_path_qualified(
                                    &rtc_transport,address,connection.stable_id())));
                        if connection.stable_id()!=stable || connection.close_reason().is_some()
                            || !gate.allows(&connection,ProtocolScope::Member) || !selected_rtc() {
                            return Err("same original ordinary M/current gate/selected RTC unavailable");
                        }
                        let observation = observe_owned_udp_mark(selected);
                        if connection.stable_id()!=stable || connection.close_reason().is_some()
                            || !gate.allows(&connection,ProtocolScope::Member) || !selected_rtc() {
                            return Err("ordinary M/current gate/RTC changed during read");
                        }
                        Ok(observation)
                    }.await;
                    println!("{}", match result {
                        Ok(observation) => json!({"event":"member-socket-mark", "observation":observation}),
                        Err(reason) => json!({"event":"member-socket-mark", "observation":{
                            "observed":false,"socketEvidence":null,"reason":reason,"actualSocketUid":null}}),
                    });
                },
                Some(command) if unified_member && command.starts_with("member-cancel ")=>{
                    let label=&command[14..];
                    let attempts=named_attempts.lock().unwrap();
                    let attempt=attempts.get(label).ok_or("unknown owned member attempt")?;
                    if !attempt.pending {return Err("member attempt already settled".into());}
                    attempt.cancellation.cancel();
                    println!("{}",json!({"event":"member-cancelled","label":label,"pending":attempt.pending}));
                },
                Some(command) if unified_member && (command.starts_with("member-connect ") || command.starts_with("member-connect-named "))=>{
                    let (label,parts)=if let Some(named)=command.strip_prefix("member-connect-named ") {
                        let mut parts=named.split_whitespace();
                        let label=parts.next().ok_or("owned member attempt label missing")?;
                        if label.len()>16 || !label.bytes().all(|byte|byte.is_ascii_alphanumeric()) {return Err("invalid fixture label".into());}
                        (Some(label.to_string()),parts.collect::<Vec<_>>())
                    } else {(None,command[15..].split_whitespace().collect::<Vec<_>>())};
                    if parts.len()!=3 {return Err("member-connect requires peer, contact route and caller milliseconds".into());}
                    let peer=parts[0].parse()?;
                    let route=MqttTransport::address(parts[1])?;
                    let caller_ms=parts[2].parse::<u64>()?;
                    if caller_ms==0 || caller_ms>12000 {return Err("member caller budget exceeds fixture caller budget".into());}
                    let started=n0_future::time::Instant::now();
                    let deadline=started+Duration::from_millis(caller_ms);
                    let cancellation=tokio_util::sync::CancellationToken::new();
                    let named_guard=if let Some(label)=label.as_ref() {
                        let mut attempts=named_attempts.lock().unwrap();
                        // Four retained labels bound this diagnostic ledger, not SDK admission or Node policy.
                        if attempts.len()>=4 || attempts.contains_key(label) {return Err("fixture label ledger full or already owned".into());}
                        attempts.insert(label.clone(),NamedMemberAttempt {cancellation:cancellation.clone(),pending:true,original:None});
                        Some(NamedAttemptGuard {ledger:named_attempts.clone(),label:label.clone()})
                    } else {None};
                    let assembly=assembly.clone();
                    let gate=node.lock().await.admission_gate();
                    let rtc=rtc_transport.clone();
                    let serving_endpoint=endpoint.clone();
                    handlers.spawn(async move {
                        let mut named_guard=named_guard;
                        let result=assembly.coordinator().connect(
                            iroh::EndpointAddr::new(peer).with_addrs([iroh::TransportAddr::Custom(route)]),
                            miopunch_native_probe::connection_coordinator::SessionPurpose::MemberBusiness,
                            miopunch_native_probe::connection_coordinator::AttemptContext::new(deadline,cancellation)
                        ).await;
                        if let Some(guard)=named_guard.as_ref() {
                            if let Some(entry)=guard.ledger.lock().unwrap().get_mut(&guard.label) {
                                entry.original=result.as_ref().ok().map(|connection|connection.weak_handle());
                            }
                        }
                        drop(named_guard.take());
                        println!("{}",json!({"event":"member-connect-result","label":label,"returned":result.is_ok(),
                            "stableId":result.as_ref().ok().map(|connection|connection.stable_id()),
                            "expectedPeerMatched":result.as_ref().ok().is_some_and(|connection|connection.remote_id()==peer),
                            "remoteId":result.as_ref().ok().map(|connection|connection.remote_id().to_string()),
                            "memberAuthorized":result.as_ref().ok().is_some_and(|connection|gate.allows(connection,ProtocolScope::Member)),
                            "paths":result.as_ref().ok().map(connection_paths),
                            "quicRole":result.as_ref().ok().map(|connection|if connection.side().is_client(){"client"}else{"server"}),
                            "callerMs":started.elapsed().as_millis(),"callerBudgetMs":caller_ms,
                            "failure":result.as_ref().err().map(ToString::to_string)}));
                        if let Ok(connection)=result {
                            serve_member_streams(serving_endpoint,connection,gate,rtc,Some(deadline),label).await?;
                        }
                        Ok::<_,Box<dyn Error+Send+Sync>>(())
                    });
                },
                Some("admission-status") if admission_mode=>{
                    let mut node=node.lock().await;
                    let mut requests=Vec::new();
                    for request in node.manual_requests().await? {
                        let approved=node.approved_admission(&request).await?;
                        requests.push(json!({"applicant":request.applicant.to_string(),"request":hex::encode(request.request_id),
                            "authorization":approved.as_ref().map(|a|a.authorization.to_string()),"receiptStage":approved.map(|a|format!("{:?}",a.stage))}));
                    }
                    let records=node.management_history().await?;
                    println!("{}",json!({"event":"admission-status","id":node.status().node_id.to_string(),"requests":requests,
                        "records":records.iter().map(|r|p2panda_core::Hash::digest(r.header()).to_string()).collect::<Vec<_>>(),"count":node.status().applied_count}));
                },
                Some(command) if admission_mode && command.starts_with("approve-request ")=>{
                    let parts=command[16..].split_whitespace().collect::<Vec<_>>();
                    if parts.len()!=2 {return Err("exact applicant and request required".into());}
                    let request=miopunch_native_probe::manual_admission::AdmissionRequest {applicant:parts[0].parse()?,request_id:hex::decode(parts[1])?.try_into().map_err(|_|"request length")?};
                    let mut node=node.lock().await;
                    let before=node.status().applied_count;
                    let approved=node.approve_manual_admission(&request).await?;
                    println!("{}",json!({"event":"approved","applicant":request.applicant.to_string(),"request":hex::encode(request.request_id),
                        "authorization":approved.authorization.to_string(),"before":before,"after":node.status().applied_count,"receiptStage":format!("{:?}",approved.stage)}));
                },
                Some("issue") if admission_mode=>{
                    use std::os::unix::fs::OpenOptionsExt;
                    use std::io::Write;
                    let mut node=node.lock().await;
                    let expires=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs()+3600;
                    let code=node.issue_precode(expires,Some(1)).await?;
                    let mut file=std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&code_path)?;
                    file.write_all(&code.encode())?;file.sync_all()?;
                    issued=Some(code);
                    println!("{}",json!({"event":"issued","code":hex::encode(code.encode())}));
                },
                Some("usage") if admission_mode=>{
                    let mut node=node.lock().await;
                    let code=issued.as_ref().ok_or("no owned code")?;
                    let status=node.precode_usage_status(code).await?;
                    let redemptions=node.precode_redemptions(code).await?;
                    println!("{}",json!({"event":"usage","uses":node.precode_usage(code).await?,"trusted":status.count_trusted,
                        "redemptions":redemptions.iter().map(|a|json!({"applicant":a.request.applicant.to_string(),"request":hex::encode(a.request.request_id),"authorization":a.authorization.to_string(),"receiptStage":format!("{:?}",a.stage)})).collect::<Vec<_>>(),"count":node.status().applied_count}));
                },
                Some("record")=>{
                    use p2panda_auth::{Access,GroupsExtensionArgs,group::{GroupAction,GroupMember}};
                    use p2panda_core::{Hash,Header,SigningKey};
                    use miopunch_native_probe::management_auth::ManagementExtension;
                    let mut node=node.lock().await;
                    let key=SigningKey::from_bytes(&std::fs::read(std::path::Path::new(&args[1]).join("node.key"))?
                        .try_into().map_err(|_|"invalid fixture key length")?);
                    let target=SigningKey::generate().verifying_key();
                    let history=node.management_history().await?;
                    let body=b"original-author browser replica body";
                    let header=Header::<ManagementExtension>::builder().body(body).build(&key,ManagementExtension {
                        version:1,log_id:rand::random(),group:GroupsExtensionArgs {group_id:node.status().anchor.root,
                            action:GroupAction::Add{member:GroupMember::Individual(target),access:Access::read()},
                            dependencies:history.iter().map(|record|Hash::digest(record.header())).collect()}
                    }).encode();
                    let id=node.ingest_management_record(&header,Some(body)).await?;
                    let stored=node.management_record(id).await?.ok_or("source record not applied")?;
                    let verified=Header::<ManagementExtension>::decode(stored.header())?;
                    println!("{}",json!({"event":"record","record":id.to_string(),"header":hex::encode(stored.header()),
                        "body":stored.body().map(hex::encode),"bodyBytes":body.len(),"author":verified.verifying_key.to_string(),
                        "logId":hex::encode(verified.extensions.log_id),"sequence":verified.seq_num,"backlink":verified.backlink.map(|id|id.to_string()),
                        "action":"Add","target":target.to_string(),"dependencies":verified.extensions.group.dependencies.iter().map(ToString::to_string).collect::<Vec<_>>(),
                        "applied":node.status().members.contains(&(target,Access::read()))}));
                },
                Some("arm-control")=>{
                    rtc_transport.arm_control_fixture(miopunch_native_probe::rtc_invalidation_fixture::ControlCheckpointStage::BeforeRelease)?;
                    println!("{}",json!({"event":"control-armed"}));
                },
                Some("arm-control-hold-cleanup")=>{
                    rtc_transport.arm_control_fixture_with_cleanup_hold(miopunch_native_probe::rtc_invalidation_fixture::ControlCheckpointStage::BeforeRelease)?;
                    println!("{}",json!({"event":"control-armed"}));
                },
                Some("control-status")=>{
                    let node=node.lock().await;
                    let (generation,durable)=node.credential_generation_fixture().await?;
                    println!("{}",json!({"event":"control-status","checkpoint":rtc_transport.control_fixture_snapshot(),
                        "source":node.admission_gate().qualification_fixture(),"secretGeneration":generation,"durableCurrent":durable,
                        "members":node.status().members.iter().map(|(id,_)|id.to_string()).collect::<Vec<_>>(),
                        "resourceRevision":rtc_transport.resource_revision(),"resourceAllowed":rtc_transport.resource_allowed(),
                        "activeRtcAssociations":rtc_transport.active_associations(),"activeRtcPumps":rtc_transport.active_pumps(),
                        "socketCount":socket_count()}));
                },
                Some("credential-invalidate")=>{
                    let mut node=node.lock().await;
                    let before=node.admission_gate().qualification_fixture();
                    let generation=node.invalidate_current_credential_fixture().await?;
                    let (_,durable)=node.credential_generation_fixture().await?;
                    println!("{}",json!({"event":"qualification-applied","kind":"credential-generation-invalidation",
                        "before":before,"after":node.admission_gate().qualification_fixture(),"generation":generation,"durableCurrent":durable,
                        "members":node.status().members.iter().map(|(id,_)|id.to_string()).collect::<Vec<_>>(),
                        "resourceRevision":rtc_transport.resource_revision(),"resourceAllowed":rtc_transport.resource_allowed(),
                        "checkpoint":rtc_transport.control_fixture_snapshot()}));
                },
                Some("rtc-resource-disable")=>{
                    let node=node.lock().await;
                    let source=node.admission_gate().qualification_fixture();
                    let before=rtc_transport.resource_revision();
                    rtc_transport.set_resource_allowed(false);
                    println!("{}",json!({"event":"qualification-applied","kind":"scoped-rtc-carrier-permission",
                        "before":source,"after":node.admission_gate().qualification_fixture(),
                        "previousResourceRevision":before,"resourceRevision":rtc_transport.resource_revision(),"resourceAllowed":rtc_transport.resource_allowed(),
                        "checkpoint":rtc_transport.control_fixture_snapshot()}));
                },
                Some("control-clear")=>{
                    rtc_transport.clear_control_fixture();
                    println!("{}",json!({"event":"control-cleared"}));
                },
                Some(command) if command.starts_with("control-release ")=>{
                    let result=rtc_transport.release_control_fixture(&command[16..]);
                    println!("{}",json!({"event":"control-release","released":result.is_ok(),"failure":result.err().map(|e|e.to_string())}));
                },
                Some(command) if command.starts_with("cleanup-release ")=>{
                    rtc_transport.release_cleanup_fixture(&command[16..])?;
                    println!("{}",json!({"event":"cleanup-release"}));
                },
                Some(command) if command.starts_with("remove-member ")=>{
                    let mut node=node.lock().await;
                    let before=node.admission_gate().qualification_fixture();
                    let target=command[14..].parse()?;
                    let mut session=node.begin_management_session();
                    let result=node.manage(&mut session,miopunch_native_probe::daily_management::ManagementAction::RemoveMember(target)).await?;
                    session.end();
                    let record=node.management_history().await?.into_iter()
                        .find(|record|p2panda_core::Hash::digest(record.header())==result.record)
                        .ok_or("applied removal record missing from actual history")?;
                    println!("{}",json!({"event":"qualification-applied","kind":"verified-applied-member-removal",
                        "record":result.record.to_string(),"target":target.to_string(),"effectiveAccessAbsent":result.effective_access.is_none(),
                        "header":hex::encode(record.header()),
                        "before":before,"after":node.admission_gate().qualification_fixture(),
                        "members":node.status().members.iter().map(|(id,_)|id.to_string()).collect::<Vec<_>>(),
                        "resourceRevision":rtc_transport.resource_revision(),"resourceAllowed":rtc_transport.resource_allowed(),
                        "checkpoint":rtc_transport.control_fixture_snapshot()}));
                },
                _=>return Err("unknown fixture command".into()),
            },
            incoming=endpoint.accept()=>{
                let Some(incoming)=incoming else {break};
                let caller_deadline=n0_future::time::Instant::now()+Duration::from_secs(12);
                let incoming=incoming.with_caller_deadline(caller_deadline);
                let node=node.clone();
                let assembly=assembly.clone();
                let rtc_transport=rtc_transport.clone();
                let serving_endpoint=endpoint.clone();
                handlers.spawn(async move {
                    let accepted = if unified_member {
                        async {
                            let mut accepting=incoming.accept().map_err(|error|error.to_string())?
                                .with_caller_deadline(caller_deadline);
                            let alpn=accepting.alpn().await.map_err(|error|error.to_string())?;
                            if let Some(purpose)=miopunch_native_probe::connection_coordinator::SessionPurpose::from_alpn(&alpn) {
                                assembly.coordinator().accept_accepting(accepting,purpose,
                                    miopunch_native_probe::connection_coordinator::AttemptContext::new(
                                        caller_deadline,tokio_util::sync::CancellationToken::new())).await
                                    .map_err(|error|error.to_string())
                            } else {accepting.await.map_err(|error|error.to_string())}
                        }.await
                    } else {incoming.await.map_err(|error|error.to_string())};
                    let connection=match accepted {
                        Ok(connection)=>connection,
                        Err(error)=>{
                            println!("{}",json!({"event":"handle-withheld","error":error.to_string()}));
                            return Ok::<_,Box<dyn Error+Send+Sync>>(());
                        }
                    };
                    if admission_mode && matches!(connection.alpn(), ADMISSION_REQUEST_ALPN | PRECODE_REQUEST_ALPN) {
                        let deadline=connection.caller_deadline().ok_or("owned admission deadline absent")?;
                        let gate=node.lock().await.admission_gate();
                        println!("{}",json!({"event":"admission-accepted","stableId":connection.stable_id(),"remoteId":connection.remote_id().to_string(),
                            "alpn":String::from_utf8_lossy(connection.alpn()),"restricted":gate.allows(&connection,ProtocolScope::AdmissionRequest),
                            "businessHandle":gate.allows(&connection,ProtocolScope::Member),"paths":connection_paths(&connection)}));
                        let result=n0_future::time::timeout(deadline.saturating_duration_since(n0_future::time::Instant::now()),async {
                            let mut node=node.lock().await;
                            if connection.alpn()==PRECODE_REQUEST_ALPN {node.serve_precode_admission(&connection).await} else {node.serve_manual_admission(&connection).await}
                        }).await;
                        println!("{}",json!({"event":"admission-ended","stableId":connection.stable_id(),"remoteId":connection.remote_id().to_string(),"stage":format!("{result:?}"),"paths":connection_paths(&connection)}));
                        if matches!(&result,Ok(Ok(_))) {let _=n0_future::time::timeout(deadline.saturating_duration_since(n0_future::time::Instant::now()),connection.closed()).await;}
                        connection.close(0u32.into(),b"owned admission ended");
                        return Ok::<_,Box<dyn Error+Send+Sync>>(());
                    }
                    if connection.alpn() == miopunch_native_probe::connection_recovery::CONNECTION_RECOVERY_ALPN {
                        println!("{}",json!({"event":"repair-control","stableId":connection.stable_id(),
                            "alpn":String::from_utf8_lossy(connection.alpn()),"restricted":!node.lock().await.admission_gate().allows(&connection,ProtocolScope::Member),
                            "paths":connection.paths().iter().map(|path|json!({"isRelay":path.is_relay(),"isIp":path.is_ip(),
                                "isMqtt":matches!(path.remote_addr(),iroh::TransportAddr::Custom(addr) if addr.id()==miopunch_native_probe::mqtt_transport::TRANSPORT_ID),
                                "isRtc":matches!(path.remote_addr(),iroh::TransportAddr::Custom(addr) if addr.id()==miopunch_native_probe::rtc_transport::TRANSPORT_ID)})).collect::<Vec<_>>()}));
                        let result=node.lock().await.serve_connection_path_recovery(&assembly,&connection).await;
                        println!("{}",json!({"event":"repair-control-ended","stableId":connection.stable_id(),"succeeded":result.is_ok(),"failure":result.err().map(|e|e.to_string())}));
                        connection.close(0u32.into(),b"repair serving ended");
                        return Ok::<_,Box<dyn Error+Send+Sync>>(());
                    }
                    println!("{}",json!({"event":"member-released","stableId":connection.stable_id(),
                        "remoteId":connection.remote_id().to_string(),"alpn":String::from_utf8_lossy(connection.alpn()),
                        "memberAuthorized":node.lock().await.admission_gate().allows(&connection,ProtocolScope::Member),
                        "quicRole":if connection.side().is_client(){"client"}else{"server"},
                        "paths":connection.paths().iter().map(|path|json!({"isRelay":path.is_relay(),"isIp":path.is_ip(),"selected":path.is_selected(),"ackReceived":path.stats().frame_rx.acks,"streamSent":path.stats().frame_tx.stream})).collect::<Vec<_>>()}));
                    if connection.alpn() == miopunch_native_probe::connection_coordinator::REPLICA_ALPN {
                        let original=connection.stable_id();
                        let mut binding=[0;32];
                        connection.export_keying_material(&mut binding,b"miopunch/replica-session-observation/1",&[]).map_err(|_|"original replica binding unavailable")?;
                        let deadline=connection.caller_deadline().ok_or("owned replica caller deadline absent")?;
                        let before=connection_paths(&connection);
                        let result=node.lock().await.serve_replica_with_deadline(&connection,deadline).await;
                        let mut current=[0;32];
                        let matched=connection.export_keying_material(&mut current,b"miopunch/replica-session-observation/1",&[]).is_ok()&&current==binding;
                        println!("{}",json!({"event":"replica-finished","stableId":connection.stable_id(),"alpn":String::from_utf8_lossy(connection.alpn()),
                            "stableUnchanged":connection.stable_id()==original,"bindingUnchanged":matched,"durableReceiptAccepted":result.is_ok(),
                            "failure":result.as_ref().err().map(ToString::to_string),"before":before,"after":connection_paths(&connection),
                            "remainingCallerMs":deadline.saturating_duration_since(n0_future::time::Instant::now()).as_millis()}));
                        result?;
                    } else {
                        let gate=node.lock().await.admission_gate();
                        serve_member_streams(serving_endpoint,connection.clone(),gate,rtc_transport,None,None).await?;
                    }
                    connection.closed().await;
                    Ok(())
                });
            },
            Some(result)=handlers.join_next(),if !handlers.is_empty()=>{
                if let Ok(Err(error))=result {eprintln!("owned saved-member handler: {error}");}
            }
        }
    }
    for attempt in named_attempts.lock().unwrap().values() {
        if attempt.pending {
            attempt.cancellation.cancel();
        }
    }
    drop(named_attempts);
    FixtureCleanup {
        node,
        assembly,
        endpoint,
        transport,
        rtc_transport,
        traffic,
        handlers,
        sockets_before,
    }
    .close()
    .await
}

#[cfg(test)]
mod tests {
    use super::fixture_options;

    fn arguments(tail: &[&str]) -> Vec<String> {
        ["fixture", "data", "facility", "ca"]
            .into_iter()
            .chain(tail.iter().copied())
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn common_forms_keep_profile_separate_from_relay() {
        for (mode, admission, relay) in [
            (vec![], false, None),
            (vec!["admission"], true, None),
            (vec!["unified-member"], false, None),
            (
                vec!["unified-member", "https://relay.test/"],
                false,
                Some("https://relay.test/"),
            ),
        ] {
            for profile in [false, true] {
                let mut tail = mode.clone();
                if profile {
                    tail.extend(["--ice-profile", "owned-profile.json"]);
                }
                let args = arguments(&tail);
                let parsed = fixture_options(&args).unwrap();
                assert_eq!(parsed.admission, admission);
                assert_eq!(parsed.permitted_relay, relay);
                assert_eq!(parsed.ice_profile, profile.then_some("owned-profile.json"));
            }
        }
    }

    #[test]
    fn ambiguous_or_incomplete_profile_forms_are_rejected() {
        for tail in [
            vec!["legacy-no-rtc"],
            vec!["legacy-relay", "https://relay.test/"],
            vec!["rtc-relay", "https://relay.test/"],
            vec!["--ice-profile"],
            vec!["admission", "--ice-profile"],
            vec!["--ice-profile", "one.json", "--ice-profile", "two.json"],
            vec!["--ice-profile", "owned-profile.json", "admission"],
            vec!["admission", "unrequested-relay"],
        ] {
            assert!(fixture_options(&arguments(&tail)).is_err(), "{tail:?}");
        }
    }
}
