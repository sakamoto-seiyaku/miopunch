//! Formal native SDK actor for one controlled/device connection story.
//! Runtime material stays in private node directories; stdout is receipt-only.
use std::{
    collections::BTreeSet,
    error::Error,
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use iroh::{
    Endpoint, EndpointAddr, TransportAddr,
    endpoint::{Connection, EndpointHooks, presets},
};
use miopunch_native_probe::{
    admission_gate::{ADMISSION_REQUEST_ALPN, AdmissionGate, ProtocolScope},
    connection_coordinator::{AttemptContext, SessionPurpose},
    daily_management::ManagementAction,
    manual_admission::ManualExchange,
    mqtt_datagram::MqttFacility,
    native_network::NativeNetwork,
    resources::{Resource, ResourceUse},
};
use serde::{Deserialize, Serialize};

type Failure = Box<dyn Error + Send + Sync>;
const REQUEST: &[u8] = b"MIO79 formal native member request";
const RESPONSE: &[u8] = b"MIO79 formal native member response";
const RECORD_LIMIT: usize = 1024 * 1024;
#[path = "unified_native_member/socket_receipt.rs"]
mod socket_receipt;

#[derive(Serialize, Deserialize)]
struct RecordWire {
    header: Vec<u8>,
    body: Option<Vec<u8>>,
}

fn emit(value: serde_json::Value) {
    println!("{value}");
}
fn required(args: &[String], index: usize) -> Result<&str, Failure> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| "missing role arguments".into())
}

#[tokio::main]
async fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("prepare") => prepare(&args).await,
        Some("serve") | Some("connect") => run(&args).await,
        Some("configure-public") => configure_public(&args).await,
        _ => Err("use prepare DIRECTORY PROFILE [BROKER], configure-public NODEDIR FACILITYFILE, serve NODEDIR ROUTEFILE [PORT|BINDADDR], or connect NODEDIR TARGETFILE [PORT|BINDADDR]".into()),
    };
    if let Err(error) = result {
        emit(serde_json::json!({"event":"actor-failed","error":error.to_string()}));
        std::process::exit(1);
    }
}

async fn prepare(args: &[String]) -> Result<(), Failure> {
    let directory = Path::new(required(args, 1)?);
    let profile = required(args, 2)?;
    let broker = args
        .get(3)
        .map(String::as_str)
        .unwrap_or("mqtt://100.64.27.1:1883/");
    let mut a = NativeNetwork::create(directory.join("server")).await?;
    let mut b = NativeNetwork::join(directory.join("client"), a.manual_invitation()?).await?;
    for node in [&a, &b] {
        let mut config = node.resource_configuration()?;
        config.replacement = Some(vec![]);
        node.save_resource_configuration(&config)?;
    }
    let ae = a.bind_endpoint(Ipv4Addr::LOCALHOST).await?;
    let be = b.bind_endpoint(Ipv4Addr::LOCALHOST).await?;
    let deadline = n0_future::time::Instant::now() + Duration::from_secs(12);
    let options = iroh::endpoint::ConnectOptions::new().with_caller_deadline(deadline);
    let (client, server) = tokio::join!(
        async {
            be.connect_control_with_opts(ae.addr(), ADMISSION_REQUEST_ALPN, options)
                .await?
                .await
                .map_err(Failure::from)
        },
        async {
            ae.accept()
                .await
                .ok_or("admission endpoint closed")?
                .with_caller_deadline(deadline)
                .accept_control()?
                .await
                .map_err(Failure::from)
        }
    );
    let (client, server) = (Arc::new(client?), Arc::new(server?));
    let (request, served) = tokio::join!(
        b.request_manual_admission(&client),
        a.serve_manual_admission(&server)
    );
    if !matches!(request?, ManualExchange::AwaitingApproval) {
        return Err("unexpected admission stage".into());
    }
    served?;
    let request = a
        .manual_requests()
        .await?
        .pop()
        .ok_or("missing admission request")?;
    a.approve_manual_admission(&request).await?;
    let receiver = async {
        let ManualExchange::Materials(material) = b.request_manual_admission(&client).await? else {
            return Err(Failure::from("approved material unavailable"));
        };
        b.complete_manual_admission(material).await?;
        Ok::<_, Failure>(())
    };
    let (received, served) = tokio::join!(receiver, a.serve_manual_admission(&server));
    received?;
    served?;
    client.close(0u32.into(), b"prepared independent members");
    server.close(0u32.into(), b"prepared independent members");
    tokio::join!(ae.close(), be.close());
    for (node, server) in [(&a, true), (&b, false)] {
        let mut config = node.resource_configuration()?;
        let mut resources = vec![Resource {
            url: broker.into(),
            uses: BTreeSet::from([ResourceUse::Signaling]),
            limitation: "explicit test contact facility; ordinary data forbidden".into(),
        }];
        if matches!(profile, "tcp" | "fallback" | "blocked" | "asymmetric") {
            if profile != "asymmetric" || server {
                resources.push(Resource {
                    url: "stun+tcp://100.64.27.1:3478/".into(),
                    uses: BTreeSet::from([ResourceUse::Observation]),
                    limitation: "owned direct-socket TCP observer".into(),
                });
            }
            config.tcp_listen =
                Some((Ipv4Addr::UNSPECIFIED, if server { 43027 } else { 42027 }).into());
        }
        if matches!(profile, "udp" | "fallback" | "blocked" | "asymmetric") {
            for destination in [
                "100.64.27.1:3478",
                "100.64.27.1:3479",
                "100.64.27.5:3478",
                "100.64.27.5:3479",
            ] {
                resources.push(Resource {
                    url: format!("stun://{destination}/"),
                    uses: BTreeSet::from([ResourceUse::Observation]),
                    limitation: "owned actual native UDP socket observation".into(),
                });
            }
        }
        if !matches!(
            profile,
            "udp" | "tcp" | "fallback" | "blocked" | "asymmetric" | "public"
        ) {
            return Err("unknown native test profile".into());
        }
        config.udp_direct_disabled = profile == "tcp";
        config.replacement = Some(resources);
        node.save_resource_configuration(&config)?;
        let expected = if server {
            b.status().node_id
        } else {
            a.status().node_id
        };
        std::fs::write(
            directory
                .join(if server { "server" } else { "client" })
                .join("expected-peer"),
            expected.to_string(),
        )?;
    }
    let independent = a.status().node_id != b.status().node_id;
    tokio::join!(a.close(), b.close());
    emit(
        serde_json::json!({"event":"members-prepared","independentIdentities":independent,
        "actualManualAdmission":true,"durableMemberMaterials":true,"profile":profile}),
    );
    Ok(())
}

/// Root-authorized representative maintained observer group; explicit test
/// configuration only, never a new default table or availability scanner.
async fn configure_public(args: &[String]) -> Result<(), Failure> {
    let node = NativeNetwork::open(required(args, 1)?).await?;
    let facility: MqttFacility = serde_json::from_slice(&std::fs::read(required(args, 2)?)?)?;
    facility.validate()?;
    let mut config = node.resource_configuration()?;
    config.mqtt_facilities = vec![facility.clone()];
    let mut resources = config.defaults.resources.clone();
    resources.retain(|resource| {
        !resource.url.starts_with("stun:")
            && !resource.url.starts_with("stun+tcp:")
            && !resource.url.starts_with("mqtt")
            && !resource.url.starts_with("irc")
    });
    // Public relays remain independently permitted data fallback; observing
    // the selected native sockets uses only the explicit maintained group.
    for resource in &mut resources {
        resource.uses.remove(&ResourceUse::Observation);
    }
    resources.push(Resource {
        url: facility.native_url.clone(),
        uses: BTreeSet::from([ResourceUse::Signaling]),
        limitation: "explicit public MQTT contact; not a business-data relay".into(),
    });
    for destination in ["162.159.207.0:3478", "106.13.249.54:3478"] {
        resources.push(Resource {
            url: format!("stun://{destination}/"),
            uses: BTreeSet::from([ResourceUse::Observation]),
            limitation:
                "explicitly authorized representative maintained observer; actual SDK use only"
                    .into(),
        });
    }
    config.replacement = Some(resources);
    config.tcp_listen = None;
    config.udp_direct_disabled = false;
    node.save_resource_configuration(&config)?;
    node.close().await;
    emit(
        serde_json::json!({"event":"public-configured","ordinaryMqttForbidden":true,"tcpPunchingEnabled":false,
        "explicitMqttFacility":true,"observerGroup":"explicit-maintained-cn-plus-cloudflare","availabilityScan":false}),
    );
    Ok(())
}

#[derive(Clone, Debug)]
struct CaptureMember {
    gate: AdmissionGate,
    expected: iroh::EndpointId,
    captured: Arc<Mutex<Option<(iroh::endpoint::WeakConnectionHandle, bool)>>>,
}
impl EndpointHooks for CaptureMember {
    async fn after_handshake(
        &self,
        connection: &Connection,
    ) -> iroh::endpoint::AfterHandshakeOutcome {
        if connection.alpn() == miopunch_native_probe::connection_coordinator::MEMBER_ALPN {
            let matched = connection.remote_id() == self.expected
                && self.gate.allows(connection, ProtocolScope::Member);
            *self.captured.lock().unwrap() = Some((connection.weak_handle(), matched));
        }
        iroh::endpoint::AfterHandshakeOutcome::Accept
    }
}

async fn run(args: &[String]) -> Result<(), Failure> {
    let server = required(args, 0)? == "serve";
    let directory = PathBuf::from(required(args, 1)?);
    let target_file = PathBuf::from(required(args, 2)?);
    let bind: SocketAddr = match args.get(3) {
        Some(value) if value.contains(':') => value.parse()?,
        Some(value) => (Ipv4Addr::UNSPECIFIED, value.parse::<u16>()?).into(),
        None => (Ipv4Addr::UNSPECIFIED, 0).into(),
    };
    let expected: iroh::EndpointId =
        std::fs::read_to_string(directory.join("expected-peer"))?.parse()?;
    let captured = Arc::new(Mutex::new(None));
    let mut node = NativeNetwork::open(&directory).await?;
    if server {
        let mut management = node.begin_management_session();
        node.manage(
            &mut management,
            ManagementAction::AddMember(p2panda_core::SigningKey::generate().verifying_key()),
        )
        .await?;
    }
    let resources = node
        .bind_configured_endpoint_with(
            Endpoint::builder(presets::Minimal)
                .clear_ip_transports()
                .bind_addr(bind)?
                .hooks(CaptureMember {
                    gate: node.admission_gate(),
                    expected,
                    captured: captured.clone(),
                }),
        )
        .await?;
    // Advertised addresses omit disabled transports, while their physical
    // socket can still exist. Observe the actual endpoint-owned bind set.
    let sockets = resources.endpoint().bound_sockets();
    let tcp_sockets: Vec<_> = resources
        .endpoint()
        .custom_addrs()
        .into_iter()
        .filter(|address| address.id() == miopunch_native_probe::tcp_transport::DATA_TRANSPORT_ID)
        .map(|address| miopunch_native_probe::tcp_transport::TcpTransport::socket_addr(&address))
        .collect::<Result<_, _>>()?;
    let endpoint_id = resources.endpoint().id();
    let endpoint_identity = endpoint_id.as_bytes() == node.status().node_id.as_bytes();
    let coordinator = resources.coordinator();
    emit(socket_receipt::snapshot("resources-ready", &sockets, None));
    if server {
        let contact = resources
            .endpoint()
            .custom_addrs()
            .into_iter()
            .find(|address| address.id() == miopunch_native_probe::mqtt_transport::TRANSPORT_ID)
            .ok_or("no ready configured MQTT contact")?;
        std::fs::write(
            &target_file,
            serde_json::to_vec(
                &EndpointAddr::new(resources.endpoint().id())
                    .with_addrs([TransportAddr::Custom(contact)]),
            )?,
        )?;
        emit(
            serde_json::json!({"event":"native-ready","role":"responder","identityMatched":endpoint_identity}),
        );
    }
    let started = Instant::now();
    let deadline = n0_future::time::Instant::now() + Duration::from_secs(45);
    let context = AttemptContext::new(deadline, tokio_util::sync::CancellationToken::new());
    let result = if server {
        let incoming = tokio::time::timeout(Duration::from_secs(45), resources.endpoint().accept())
            .await?
            .ok_or("native endpoint closed")?;
        coordinator
            .accept(incoming, SessionPurpose::MemberBusiness, context)
            .await
    } else {
        let target: EndpointAddr = serde_json::from_slice(&std::fs::read(target_file)?)?;
        if target.id != expected {
            return Err("trusted target identity mismatch".into());
        }
        coordinator
            .connect(target, SessionPurpose::MemberBusiness, context)
            .await
    };
    let transfer;
    match result {
        Ok(connection) => {
            let matched = connection.remote_id() == expected
                && node
                    .admission_gate()
                    .allows(&connection, ProtocolScope::Member);
            transfer = exchange(&mut node, &connection, server).await;
            emit(
                serde_json::json!({"event":"native-result","ordinaryHandleReturned":true,
                "identityMatched":endpoint_identity && matched,"role":if server{"responder"}else{"initiator"},
                "callerMs":started.elapsed().as_millis(),"deadlineMs":45000,"streamMatched":transfer.is_ok(),
                "appliedRecords":node.status().applied_count,"paths":connection.paths().iter().map(|path|
                    serde_json::json!({"route":path.remote_addr().to_string(),"selected":path.is_selected(),
                        "pathId":path.id().to_string(),"localSocketId":path.local_socket_id().map(|id|id.get()),
                        "kind":if path.is_ip(){"native-udp"}else if path.is_relay(){"relay"}else{"native-tcp"}})).collect::<Vec<_>>() }),
            );
            let selected = connection.paths().iter().find(|path|path.is_selected()).map(|path| {
                let destination = match path.remote_addr() {
                    TransportAddr::Ip(address) => Some(serde_json::json!({"address":address.ip().to_string(),"port":address.port()})),
                    _ => None,
                };
                serde_json::json!({"route":path.remote_addr().to_string(),"pathId":path.id().to_string(),
                    "localSocketId":path.local_socket_id().map(|id|id.get()),"destination":destination})
            });
            emit(socket_receipt::snapshot(
                "member-active",
                &sockets,
                selected,
            ));
            if args.get(4).is_some_and(|value| value == "hold-for-route") && transfer.is_ok() {
                use tokio::io::AsyncBufReadExt;
                let held = Instant::now();
                let mut line = String::new();
                let command = tokio::time::timeout(
                    Duration::from_secs(15),
                    tokio::io::BufReader::new(tokio::io::stdin()).read_line(&mut line),
                )
                .await;
                emit(serde_json::json!({"event":"native-route-observation-ended",
                    "holdMs":held.elapsed().as_millis(),"callerBudgetExtended":false,
                    "releasedByController":matches!(command,Ok(Ok(_)))&&line.trim()=="release"}));
            }
            connection.close(0u32.into(), b"formal native story complete");
        }
        Err(error) => {
            let observed = captured.lock().unwrap();
            emit(
                serde_json::json!({"event":"native-result","ordinaryHandleReturned":false,
                "identityMatched":observed.as_ref().map(|(_,matched)|*matched),"error":error.to_string(),
                "role":if server{"responder"}else{"initiator"},"callerMs":started.elapsed().as_millis(),"deadlineMs":45000}),
            );
            transfer = Err(error.into());
        }
    }
    drop(coordinator);
    let cleanup = Instant::now();
    resources.close().await;
    node.close().await;
    let immediate: Vec<_> = sockets
        .iter()
        .map(|address| std::net::UdpSocket::bind(address).is_ok())
        .collect();
    let tcp_immediate: Vec<_> = tcp_sockets
        .iter()
        .map(|address| std::net::TcpListener::bind(address).is_ok())
        .collect();
    let cleanup_ms = cleanup.elapsed().as_millis();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let late: Vec<_> = sockets
        .iter()
        .map(|address| std::net::UdpSocket::bind(address).is_ok())
        .collect();
    let tcp_late: Vec<_> = tcp_sockets
        .iter()
        .map(|address| std::net::TcpListener::bind(address).is_ok())
        .collect();
    emit(
        serde_json::json!({"event":"native-cleanup","cleanupMs":cleanup_ms,"lateObservationMs":cleanup.elapsed().as_millis(),
        "graceful":true,"socketReleasedImmediately":immediate,"socketReleasedLate":late,
        "tcpListenerReleasedImmediately":tcp_immediate,"tcpListenerReleasedLate":tcp_late,
        "originalClosed":captured.lock().unwrap().as_ref().is_none_or(|(weak,_)|!weak.is_open())}),
    );
    let reopened = NativeNetwork::open(&directory).await?;
    let durable_records = reopened.management_history().await?;
    emit(serde_json::json!({"event":"native-material-reopened",
        "identityMatched":reopened.status().node_id.as_bytes()==endpoint_id.as_bytes(),
        "complete":reopened.admission_stage()==miopunch_native_probe::manual_admission::AdmissionStage::Complete,
        "appliedRecords":reopened.status().applied_count,"durableRecords":durable_records.len(),
        "ordinaryHandleReturned":transfer.is_ok(),"nonemptyVerifiedRecordSaved":transfer.is_ok()&&durable_records.len()>=3}));
    reopened.close().await;
    transfer
}

async fn exchange(
    node: &mut NativeNetwork,
    connection: &Connection,
    server: bool,
) -> Result<(), Failure> {
    let operation = async {
        if server {
            let (mut send, mut recv) = connection.accept_bi().await?;
            if recv.read_to_end(1024).await? != REQUEST {
                return Err("native business request mismatch".into());
            }
            send.write_all(RESPONSE).await?;
            send.finish()?;
            let (mut send, mut recv) = connection.accept_bi().await?;
            if recv.read_to_end(64).await? != b"verified management records" {
                return Err("record request mismatch".into());
            }
            let records: Vec<_> = node
                .management_history()
                .await?
                .into_iter()
                .map(|record| RecordWire {
                    header: record.header().to_vec(),
                    body: record.body().map(<[u8]>::to_vec),
                })
                .collect();
            send.write_all(&serde_json::to_vec(&records)?).await?;
            send.finish()?;
            let (_send, mut recv) = connection.accept_bi().await?;
            if recv.read_to_end(64).await? != b"verified and saved" {
                return Err("durable record acknowledgement missing".into());
            }
        } else {
            let (mut send, mut recv) = connection.open_bi().await?;
            send.write_all(REQUEST).await?;
            send.finish()?;
            if recv.read_to_end(1024).await? != RESPONSE {
                return Err("native business response mismatch".into());
            }
            let (mut send, mut recv) = connection.open_bi().await?;
            send.write_all(b"verified management records").await?;
            send.finish()?;
            let records: Vec<RecordWire> =
                serde_json::from_slice(&recv.read_to_end(RECORD_LIMIT).await?)?;
            if records.is_empty() || records.len() > 32 {
                return Err("bounded nonempty record scope missing".into());
            }
            for record in records {
                node.ingest_management_record(&record.header, record.body.as_deref())
                    .await?;
            }
            if node.status().applied_count < 3 {
                return Err("new peer record not verified and saved".into());
            }
            let (mut send, _recv) = connection.open_bi().await?;
            send.write_all(b"verified and saved").await?;
            send.finish()?;
        }
        Ok(())
    };
    tokio::time::timeout(Duration::from_secs(10), operation).await?
}
