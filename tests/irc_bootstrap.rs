use iroh::{
    Endpoint, EndpointAddr, TransportAddr,
    endpoint::{Connection, presets},
};
#[path = "support/native_connections.rs"]
mod native_connections;
use iroh::endpoint::RestrictedEndpoint;

use miopunch_native_probe::{
    admission_gate::{ADMISSION_REQUEST_ALPN, ProtocolScope},
    manual_admission::ManualExchange,
    native_network::NativeNetwork,
    resources::{Resource, ResourceUse},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::{collections::BTreeSet, net::Ipv4Addr, path::Path, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
const BUSINESS: &[u8] = b"miopunch/irc-business/1";
async fn connect(
    client: &RestrictedEndpoint,
    server: &RestrictedEndpoint,
    alpn: &[u8],
) -> (Connection, Connection) {
    native_connections::member_pair(client, server, alpn).await
}
async fn join(admin: &mut NativeNetwork, path: &Path) -> NativeNetwork {
    let mut node = NativeNetwork::join(path, admin.manual_invitation().unwrap())
        .await
        .unwrap();
    let ae = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let ne = node.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = native_connections::control_pair(&ne, &ae, ADMISSION_REQUEST_ALPN).await;
    let (request, served) = tokio::join!(
        node.request_manual_admission(&client),
        admin.serve_manual_admission(&server)
    );
    assert!(matches!(request.unwrap(), ManualExchange::AwaitingApproval));
    served.unwrap();
    let request = admin.manual_requests().await.unwrap().pop().unwrap();
    admin.approve_manual_admission(&request).await.unwrap();
    let receive = async {
        let ManualExchange::Materials(material) =
            node.request_manual_admission(&client).await.unwrap()
        else {
            panic!("saved approval")
        };
        node.complete_manual_admission(material).await.unwrap();
    };
    let (_, served) = tokio::join!(receive, admin.serve_manual_admission(&server));
    served.unwrap();
    client.close(0u32.into(), b"admitted");
    server.close(0u32.into(), b"admitted");
    ne.close().await;
    ae.close().await;
    node
}
async fn pair() -> (tempfile::TempDir, NativeNetwork, NativeNetwork) {
    pair_at(&std::env::var("MIOPUNCH_IRC_SERVER").expect("real owned IRCd required")).await
}
async fn pair_at(server: &str) -> (tempfile::TempDir, NativeNetwork, NativeNetwork) {
    let dir = tempfile::tempdir().unwrap();
    let mut b = NativeNetwork::create(dir.path().join("b")).await.unwrap();
    let mut config = b.resource_configuration().unwrap();
    config.replacement = Some(vec![]);
    b.save_resource_configuration(&config).unwrap();
    let a = join(&mut b, &dir.path().join("a")).await;
    config.replacement = Some(vec![Resource {
        url: format!("irc://{server}/"),
        uses: BTreeSet::from([ResourceUse::Signaling]),
        limitation: "owned real IRCd; signaling only".into(),
    }]);
    for node in [&a, &b] {
        node.save_resource_configuration(&config).unwrap();
        node.admission_gate()
            .register_protocol(BUSINESS, ProtocolScope::Member);
    }
    (dir, a, b)
}

/// Network fault boundary; all protocol processing remains in the real IRCd,
/// actual adapter and QUIC/TLS. The proxy only duplicates/drops wire lines.
struct Proxy {
    address: std::net::SocketAddr,
    drop_tail: Arc<AtomicBool>,
    duplicates: Arc<AtomicBool>,
    frames: Arc<AtomicU64>,
    long_packets: Arc<AtomicU64>,
    tasks: tokio::task::JoinHandle<()>,
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.tasks.abort();
    }
}
impl Proxy {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let drop_tail = Arc::new(AtomicBool::new(false));
        let duplicates = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(AtomicU64::new(0));
        let long_packets = Arc::new(AtomicU64::new(0));
        let (drop, duplicate, count, long) = (
            drop_tail.clone(),
            duplicates.clone(),
            frames.clone(),
            long_packets.clone(),
        );
        let server = std::env::var("MIOPUNCH_IRC_SERVER").unwrap();
        let tasks = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    socket=listener.accept()=>{
                        let(socket,_)=socket.unwrap();let upstream=tokio::net::TcpStream::connect(&server).await.unwrap();let(drop,duplicate,count,long)=(drop.clone(),duplicate.clone(),count.clone(),long.clone());
                        connections.spawn(async move {
                            let (read,mut write)=socket.into_split();let(upread,mut upwrite)=upstream.into_split();
                            let forward=async {
                                let mut lines=BufReader::new(read).lines();
                                while let Some(line)=lines.next_line().await.unwrap_or(None) {
                                    assert!(line.len()+2<=512,"client envelope must fit legacy IRC line");
                                    let fields=line.split_whitespace().collect::<Vec<_>>();
                                    let mut drop_line=false;let mut repeat=false;
                                    if fields.first()==Some(&"PRIVMSG") && fields.get(2)==Some(&":MIR1") {
                                        count.fetch_add(1,Ordering::Relaxed);
                                        let index=usize::from_str_radix(fields[6],16).unwrap();let total=usize::from_str_radix(fields[7],16).unwrap();
                                        if total>1 {long.fetch_add(1,Ordering::Relaxed);}
                                        drop_line=drop.load(Ordering::Relaxed) && index+1==total;repeat=duplicate.load(Ordering::Relaxed);
                                    }
                                    if !drop_line {if upwrite.write_all(format!("{line}\r\n").as_bytes()).await.is_err(){break;}if repeat {upwrite.write_all(format!("{line}\r\n").as_bytes()).await.ok();}}
                                }
                            };
                            let backward=async {tokio::io::copy(&mut BufReader::new(upread),&mut write).await.ok();};
                            tokio::select!{_=forward=>{},_=backward=>{}}
                        });
                    }
                    _=connections.join_next(),if !connections.is_empty()=>{}
                }
            }
        });
        Self {
            address,
            drop_tail,
            duplicates,
            frames,
            long_packets,
            tasks,
        }
    }
}

#[tokio::test]
async fn a_missing_fragment_of_the_real_long_handshake_cannot_release_a_connection() {
    let proxy = Proxy::new().await;
    let (_dir, a, b) = pair_at(&proxy.address.to_string()).await;
    let ae = bind(&a)
        .await
        .with_contact_timeout(Duration::from_secs(4))
        .unwrap();
    let be = bind(&b).await;
    proxy.drop_tail.store(true, Ordering::Relaxed);
    let route = be.endpoint().custom_addrs().into_iter().next().unwrap();
    let target = EndpointAddr::new(be.endpoint().id()).with_addrs([TransportAddr::Custom(route)]);
    let started = std::time::Instant::now();
    assert!(ae.connect(target, BUSINESS).await.is_err());
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(
        proxy.long_packets.load(Ordering::Relaxed) > 0,
        "actual QUIC handshake must require multiple IRC lines"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(250), be.endpoint().accept())
            .await
            .is_err()
    );
    println!(
        "IRC_PRIORITY_PASS long_handshake_fragments={} missing_final=true no_connection=true bounded_ms={}",
        proxy.long_packets.load(Ordering::Relaxed),
        started.elapsed().as_millis()
    );
    ae.close().await;
    be.close().await;
    a.close().await;
    b.close().await;
}

#[tokio::test]
async fn incomplete_public_envelopes_have_bounded_reassembly_and_expire() {
    use iroh::endpoint::transports::CustomTransport;
    use miopunch_native_probe::irc_transport::{CHANNEL, IrcTransport};
    let server = std::env::var("MIOPUNCH_IRC_SERVER").unwrap();
    let transport =
        IrcTransport::from_resource(format!("irc://{server}/").parse().unwrap()).unwrap();
    let endpoint = transport.bind().unwrap();
    transport.wait_ready().await.unwrap();
    let mut addresses = endpoint.watch_local_addrs();
    use n0_watcher::Watcher;
    let address = addresses.get().pop().unwrap();
    let target = hex::encode(&address.data()[16..]);
    let (mut write, _lines) = registered_irc_client(&server, "test").await;
    for id in 0..80u64 {
        write
            .write_all(
                format!(
                    "PRIVMSG {CHANNEL} :MIR1 {target} {} {id:016x} 00 10 {}\r\n",
                    "11".repeat(16),
                    "ab".repeat(128)
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(transport.traffic().pending_assemblies(), 32);
    tokio::time::sleep(Duration::from_millis(3100)).await;
    assert_eq!(transport.traffic().pending_assemblies(), 0);
    assert_eq!(transport.traffic().expired_assemblies(), 32);
    println!("IRC_REASSEMBLY_BOUND_PASS {}", transport.traffic().report());
    drop(endpoint);
}
async fn bind(node: &NativeNetwork) -> miopunch_native_probe::resources::ResourceEndpoint {
    let ep = node
        .bind_configured_endpoint_with(
            Endpoint::builder(presets::Minimal)
                .bind_addr((Ipv4Addr::LOCALHOST, 0))
                .unwrap()
                .transport_config(
                    iroh::endpoint::QuicTransportConfig::builder()
                        .keep_alive_interval(Duration::from_secs(1))
                        .max_idle_timeout(Some(Duration::from_secs(4).try_into().unwrap()))
                        .build(),
                ),
        )
        .await
        .unwrap();
    ep.endpoint().set_alpns(vec![BUSINESS.to_vec()]);
    ep
}

fn target(endpoint: &RestrictedEndpoint) -> EndpointAddr {
    EndpointAddr::new(endpoint.id()).with_addrs([TransportAddr::Custom(
        endpoint
            .custom_addrs()
            .into_iter()
            .find(|address| address.id() == miopunch_native_probe::irc_transport::TRANSPORT_ID)
            .unwrap(),
    )])
}
async fn formal_connect(
    client: &mut miopunch_native_probe::resources::ResourceEndpoint,
    server: &RestrictedEndpoint,
) -> (Connection, Connection) {
    let expected = client.endpoint().id();
    let (client, server) = tokio::time::timeout(Duration::from_secs(15), async {
        tokio::join!(client.connect(target(server), BUSINESS), async {
            loop {
                match server.accept().await.unwrap().await {
                    Ok(connection) => {
                        assert_eq!(connection.remote_id(), expected);
                        break connection;
                    }
                    Err(error) => eprintln!("IRC_REJECTED_INCOMING error={error}"),
                }
            }
        })
    })
    .await
    .unwrap();
    (client.unwrap(), server)
}
async fn echo(client: &Connection, server: &Connection) {
    tokio::join!(
        async {
            let (mut send, mut recv) = client.open_bi().await.unwrap();
            send.write_all(b"private business after IRC bootstrap")
                .await
                .unwrap();
            send.finish().unwrap();
            assert_eq!(recv.read_to_end(32).await.unwrap(), b"private ACK");
        },
        async {
            let (mut send, mut recv) = server.accept_bi().await.unwrap();
            assert_eq!(
                recv.read_to_end(128).await.unwrap(),
                b"private business after IRC bootstrap"
            );
            send.write_all(b"private ACK").await.unwrap();
            send.finish().unwrap();
        }
    );
}

async fn registered_irc_client(
    server: &str,
    prefix: &str,
) -> (
    tokio::net::tcp::OwnedWriteHalf,
    tokio::io::Lines<BufReader<tokio::net::tcp::OwnedReadHalf>>,
) {
    use miopunch_native_probe::irc_transport::CHANNEL;
    tokio::time::timeout(Duration::from_secs(8), async {
        let stream = tokio::net::TcpStream::connect(server).await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut lines = BufReader::new(read).lines();
        let nick = format!("{prefix}{}", rand::random::<u32>());
        write
            .write_all(format!("NICK {nick}\r\nUSER test 0 * :test\r\n").as_bytes())
            .await
            .unwrap();
        loop {
            let line = lines
                .next_line()
                .await
                .unwrap()
                .expect("owned IRCd must complete registration");
            if line.contains(" 001 ") {
                write
                    .write_all(format!("JOIN {CHANNEL}\r\n").as_bytes())
                    .await
                    .unwrap();
            }
            if line.contains(" 366 ") {
                return (write, lines);
            }
        }
    })
    .await
    .expect("owned IRCd registration and channel join are bounded")
}

struct Observer {
    count: Arc<AtomicU64>,
    packets: Arc<Mutex<Vec<String>>>,
    write: tokio::net::tcp::OwnedWriteHalf,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Observer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Observer {
    async fn new() -> Self {
        let (write, mut lines) =
            registered_irc_client(&std::env::var("MIOPUNCH_IRC_SERVER").unwrap(), "observe").await;
        let count = Arc::new(AtomicU64::new(0));
        let packets = Arc::new(Mutex::new(Vec::new()));
        let (c, p) = (count.clone(), packets.clone());
        let task = tokio::spawn(async move {
            while let Some(line) = lines.next_line().await.unwrap_or(None) {
                assert!(line.len() + 2 <= 512);
                if line.contains(" PRIVMSG ") && line.contains(" :MIR1 ") {
                    c.fetch_add(1, Ordering::Relaxed);
                    let mut packets = p.lock().unwrap();
                    if packets.len() < 128 {
                        packets.push(line);
                    }
                }
            }
        });
        Self {
            count,
            packets,
            write,
            task,
        }
    }
    async fn quiet_count(&self) -> u64 {
        let mut previous = 0;
        let mut quiet = 0;
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let current = self.count.load(Ordering::Relaxed);
                quiet = if current == previous { quiet + 1 } else { 0 };
                previous = current;
                if quiet == 5 {
                    assert!(current > 0);
                    return current;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .unwrap()
    }
    async fn inject(&mut self, target: &iroh_base::CustomAddr) {
        let destination = hex::encode(&target.data()[16..]);
        for id in 0..10u64 {
            self.write
                .write_all(
                    format!(
                        "PRIVMSG {} :MIR1 {destination} {} {id:016x} 00 01 deadbeef\r\n",
                        miopunch_native_probe::irc_transport::CHANNEL,
                        "aa".repeat(16)
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        }
    }
}

#[tokio::test]
async fn duplicate_fragments_and_injected_ciphertext_do_not_impersonate_or_leak() {
    let mut observer = Observer::new().await;
    let proxy = Proxy::new().await;
    let (_dir, a, b) = pair_at(&proxy.address.to_string()).await;
    let mut ae = bind(&a).await;
    let be = bind(&b).await;
    observer.inject(&be.endpoint().custom_addrs()[0]).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(250), be.endpoint().accept())
            .await
            .is_err()
    );
    proxy.duplicates.store(true, Ordering::Relaxed);
    let (client, server) = formal_connect(&mut ae, be.endpoint()).await;
    echo(&client, &server).await;
    let before = observer.quiet_count().await;
    echo(&client, &server).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        observer.count.load(Ordering::Relaxed),
        before,
        "ordinary business cannot be published to IRC"
    );
    assert!(proxy.frames.load(Ordering::Relaxed) > 0);
    let observed = observer.packets.lock().unwrap();
    assert!(!observed.is_empty());
    for line in observed.iter() {
        let ciphertext = hex::decode(line.split_whitespace().last().unwrap()).unwrap();
        for marker in [b"private business".as_slice(), b"private ACK", b"127.0.0.1"] {
            assert!(
                !ciphertext
                    .windows(marker.len())
                    .any(|window| window == marker)
            );
        }
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(250), be.endpoint().accept())
            .await
            .is_err()
    );
    println!(
        "IRC_DUPLICATE_INJECTION_PASS visible_frames={} captured={} extra_connections=0 normal_business_irc_frames=0",
        before,
        observed.len()
    );
    drop(observed);
    ae.close().await;
    be.close().await;
    a.close().await;
    b.close().await;
}

#[tokio::test]
async fn reachable_irc_cannot_release_business_without_a_permitted_data_path() {
    let observer = Observer::new().await;
    let (_dir, a, b) = pair().await;
    for node in [&a, &b] {
        let mut config = node.resource_configuration().unwrap();
        config.udp_direct_disabled = true;
        node.save_resource_configuration(&config).unwrap();
    }
    let ae = bind(&a).await;
    let be = bind(&b).await;
    let (client, server) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(ae.connect(target(be.endpoint()), BUSINESS), async {
            be.endpoint().accept().await.unwrap().await
        })
    })
    .await
    .unwrap();
    assert!(client.is_err() && server.is_err());
    assert!(!ae.endpoint().custom_addrs().is_empty() && !be.endpoint().custom_addrs().is_empty());
    assert!(observer.count.load(Ordering::Relaxed) > 0);
    println!("IRC_PURPOSE_PASS control_reachable=true udp_disabled=true application_connections=0");
    ae.close().await;
    be.close().await;
    a.close().await;
    b.close().await;
}

#[tokio::test]
async fn an_irc_route_cannot_substitute_tls_identity_or_unapproved_membership() {
    let (_dir, a, b) = pair().await;
    let mut ae = bind(&a).await;
    let be = bind(&b).await;
    let mut wrong = target(be.endpoint());
    wrong.id = iroh::SecretKey::generate().public();
    let (client, server) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(ae.connect(wrong, BUSINESS), async {
            be.endpoint().accept().await.unwrap().await
        })
    })
    .await
    .unwrap();
    assert!(client.is_err() && server.is_err());
    let dir = tempfile::tempdir().unwrap();
    let stranger = NativeNetwork::join(dir.path().join("stranger"), b.manual_invitation().unwrap())
        .await
        .unwrap();
    let config = b.resource_configuration().unwrap();
    stranger.save_resource_configuration(&config).unwrap();
    stranger
        .admission_gate()
        .register_protocol(BUSINESS, ProtocolScope::Member);
    let se = bind(&stranger).await;
    let (client, server) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(se.connect(target(be.endpoint()), BUSINESS), async {
            be.endpoint().accept().await.unwrap().await
        })
    })
    .await
    .unwrap();
    assert!(client.is_err() && server.is_err());
    let (client, server) = formal_connect(&mut ae, be.endpoint()).await;
    echo(&client, &server).await;
    println!(
        "IRC_IDENTITY_PASS wrong_tls_target_rejected=true invitation_holder_member_rejected=true approved_member_usable=true"
    );
    se.close().await;
    ae.close().await;
    be.close().await;
    stranger.close().await;
    a.close().await;
    b.close().await;
}

struct DataStageNotice {
    entered: Arc<tokio::sync::Notify>,
    id: tokio::sync::watch::Sender<Option<usize>>,
}
impl std::fmt::Debug for DataStageNotice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DataStageNotice")
    }
}
impl iroh::endpoint::EndpointHooks for DataStageNotice {
    async fn after_handshake(
        &self,
        connection: &Connection,
    ) -> iroh::endpoint::AfterHandshakeOutcome {
        self.id.send_replace(Some(connection.stable_id()));
        self.entered.notify_one();
        iroh::endpoint::AfterHandshakeOutcome::Accept
    }
}
#[tokio::test]
async fn canceled_handshake_releases_the_attempt_and_a_later_contact_succeeds() {
    let (_dir, a, b) = pair().await;
    let be = bind(&b).await;
    let entered = Arc::new(tokio::sync::Notify::new());
    let (id, original) = tokio::sync::watch::channel(None);
    let mut ae = a
        .bind_configured_endpoint_with(
            Endpoint::builder(presets::Minimal)
                .bind_addr((Ipv4Addr::LOCALHOST, 0))
                .unwrap()
                .hooks(DataStageNotice {
                    entered: entered.clone(),
                    id,
                }),
        )
        .await
        .unwrap();
    ae.endpoint().set_alpns(vec![BUSINESS.to_vec()]);
    let server_endpoint = be.endpoint().clone();
    let server_accept = tokio::spawn(async move { server_endpoint.accept().await.unwrap().await });
    let mut pending = Box::pin(ae.connect(target(be.endpoint()), BUSINESS));
    tokio::time::timeout(Duration::from_secs(10),async{tokio::select!{result=&mut pending=>panic!("returned before cancellation stage: {result:?}"),_=entered.notified()=>{}}}).await.unwrap();
    let original_id = original.borrow().unwrap();
    drop(pending);
    assert!(
        tokio::time::timeout(Duration::from_secs(3), server_accept)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while ae.endpoint().metrics().socket.num_conns_closed.get() != 1
            || be.endpoint().metrics().socket.num_conns_closed.get() != 1
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let (client, server) = formal_connect(&mut ae, be.endpoint()).await;
    assert_ne!(client.stable_id(), original_id);
    echo(&client, &server).await;
    assert_eq!(ae.endpoint().metrics().socket.num_conns_opened.get(), 2);
    println!(
        "IRC_CANCEL_PASS original_stable_id={original_id} later_stable_id={} original_closed=true opened_attempts=2",
        client.stable_id()
    );
    ae.close().await;
    be.close().await;
    a.close().await;
    b.close().await;
}

#[tokio::test]
async fn disconnected_irc_withdraws_routes_and_fresh_binding_preserves_identity() {
    let proxy = Proxy::new().await;
    let (_dir, a, b) = pair_at(&proxy.address.to_string()).await;
    let ae = bind(&a).await;
    let be = bind(&b).await;
    let ids = (ae.endpoint().id(), be.endpoint().id());
    let old = (
        ae.endpoint().custom_addrs()[0].clone(),
        be.endpoint().custom_addrs()[0].clone(),
    );
    drop(proxy);
    tokio::time::timeout(Duration::from_secs(3), async {
        while !ae.endpoint().custom_addrs().is_empty() || !be.endpoint().custom_addrs().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    ae.close().await;
    be.close().await;
    let server = std::env::var("MIOPUNCH_IRC_SERVER").unwrap();
    for node in [&a, &b] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement.as_mut().unwrap()[0].url = format!("irc://{server}/");
        node.save_resource_configuration(&config).unwrap();
    }
    let mut ae = bind(&a).await;
    let be = bind(&b).await;
    assert_eq!((ae.endpoint().id(), be.endpoint().id()), ids);
    assert_ne!(
        ae.endpoint().custom_addrs()[0].data()[16..],
        old.0.data()[16..]
    );
    assert_ne!(
        be.endpoint().custom_addrs()[0].data()[16..],
        old.1.data()[16..]
    );
    let (client, server) = formal_connect(&mut ae, be.endpoint()).await;
    echo(&client, &server).await;
    println!(
        "IRC_RECONNECT_PASS withdrawn_routes=true new_delivery_tokens=true stable_node_identity=true fresh_connection=true"
    );
    ae.close().await;
    be.close().await;
    a.close().await;
    b.close().await;
}

#[tokio::test]
async fn actual_server_rate_limiting_returns_bounded_unavailability() {
    let limited = std::env::var("MIOPUNCH_IRC_LIMITED_SERVER")
        .expect("owned IRCd with actual fakelag required");
    let (_dir, a, b) = pair_at(&limited).await;
    let ae = bind(&a)
        .await
        .with_contact_timeout(Duration::from_secs(4))
        .unwrap();
    let be = bind(&b).await;
    let started = std::time::Instant::now();
    assert!(ae.connect(target(be.endpoint()), BUSINESS).await.is_err());
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(
        tokio::time::timeout(Duration::from_millis(250), be.endpoint().accept())
            .await
            .is_err()
    );
    println!(
        "IRC_RATE_LIMIT_PASS actual_ircd_fakelag=true connection_unavailable=true bounded_ms={}",
        started.elapsed().as_millis()
    );
    ae.close().await;
    be.close().await;
    a.close().await;
    b.close().await;
}

#[tokio::test]
async fn saved_irc_disable_and_a_failed_service_preserve_other_resource_choices() {
    let (_dir, a, b) = pair().await;
    let closed = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let unavailable = closed.local_addr().unwrap();
    drop(closed);
    let mut config = a.resource_configuration().unwrap();
    config.replacement.as_mut().unwrap().push(Resource {
        url: format!("irc://{unavailable}/"),
        uses: BTreeSet::from([ResourceUse::Signaling]),
        limitation: "deliberately closed owned IRC origin".into(),
    });
    a.save_resource_configuration(&config).unwrap();
    let mut ae = bind(&a).await;
    let be = bind(&b).await;
    assert_eq!(ae.endpoint().custom_addrs().len(), 1);
    let (client, server) = formal_connect(&mut ae, be.endpoint()).await;
    echo(&client, &server).await;
    ae.close().await;
    be.close().await;
    config.disabled_uses.insert(ResourceUse::Signaling);
    a.save_resource_configuration(&config).unwrap();
    let ae = bind(&a).await;
    assert!(ae.endpoint().custom_addrs().is_empty());
    assert_eq!(ae.endpoint().id().as_bytes(), a.status().node_id.as_bytes());
    println!(
        "IRC_RESOURCES_PASS failed_origins=1 permitted_ready_routes=1 saved_signaling_disable_respected=true durable_identity_unchanged=true"
    );
    ae.close().await;
    a.close().await;
    b.close().await;
}

#[tokio::test]
#[ignore = "run in the owned privileged offline IRC container, never host netfilter"]
async fn losing_the_udp_data_path_does_not_send_business_back_to_irc() {
    assert_eq!(
        std::env::var("MIO45_ISOLATED_NETFILTER").as_deref(),
        Ok("1")
    );
    let observer = Observer::new().await;
    let (_dir, a, b) = pair().await;
    let mut ae = bind(&a).await;
    let be = bind(&b).await;
    let (client, server) = formal_connect(&mut ae, be.endpoint()).await;
    echo(&client, &server).await;
    let before = observer.quiet_count().await;
    let client_port = server
        .paths()
        .iter()
        .find_map(|path| {
            if let TransportAddr::Ip(address) = path.remote_addr() {
                Some(address.port())
            } else {
                None
            }
        })
        .unwrap();
    let server_port = client
        .paths()
        .iter()
        .find_map(|path| {
            if let TransportAddr::Ip(address) = path.remote_addr() {
                Some(address.port())
            } else {
                None
            }
        })
        .unwrap();
    println!(
        "IRC_DATA_CUT_START selected_initial_source={client_port} selected_initial_dest={server_port} all_owned_udp_paths=true"
    );
    for program in ["iptables", "ip6tables"] {
        assert!(
            std::process::Command::new(program)
                .args(["-I", "OUTPUT", "1", "-p", "udp", "-j", "DROP"])
                .status()
                .unwrap()
                .success()
        );
    }
    assert!(
        std::process::Command::new("iptables-save")
            .args(["-c"])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        std::process::Command::new("ip6tables-save")
            .args(["-c"])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        std::process::Command::new("ip6tables-save")
            .args(["-c"])
            .status()
            .unwrap()
            .success()
    );
    let stable = client.stable_id();
    let (mut send, mut recv) = client.open_bi().await.unwrap();
    send.write_all(&vec![0x42; 65536]).await.unwrap();
    send.finish().unwrap();
    let server_result = tokio::time::timeout(Duration::from_secs(7), async {
        let (_, mut recv) = server.accept_bi().await?;
        recv.read_to_end(65536)
            .await
            .map_err(|_| std::io::Error::other("data stream ended"))
    })
    .await;
    assert!(!matches!(server_result, Ok(Ok(_))));
    assert!(
        tokio::time::timeout(Duration::from_secs(7), recv.read_to_end(256))
            .await
            .unwrap()
            .is_err()
    );
    tokio::time::timeout(Duration::from_secs(7), client.closed())
        .await
        .unwrap();
    assert_eq!(client.stable_id(), stable);
    assert_eq!(observer.count.load(Ordering::Relaxed), before);
    assert_eq!(ae.endpoint().metrics().socket.num_conns_opened.get(), 1);
    assert!(!ae.endpoint().custom_addrs().is_empty());
    println!(
        "IRC_DATA_CUT_PASS stable_id={stable} irc_before={before} irc_after={} queued_business=65536 irc_still_reachable=true transparent_redial=false",
        observer.count.load(Ordering::Relaxed)
    );
    assert!(
        std::process::Command::new("iptables-save")
            .args(["-c"])
            .status()
            .unwrap()
            .success()
    );
    ae.close().await;
    be.close().await;
    a.close().await;
    b.close().await;
}
#[tokio::test]
async fn real_irc_replaces_mqtt_at_the_same_native_connection_entry() {
    let (_dir, a, b) = pair().await;
    let ae = bind(&a).await;
    let be = bind(&b).await;
    let route = be
        .endpoint()
        .custom_addrs()
        .into_iter()
        .next()
        .expect("configured real IRC must publish a temporary route");
    let target = EndpointAddr::new(be.endpoint().id()).with_addrs([TransportAddr::Custom(route)]);
    let (client, server) = tokio::time::timeout(Duration::from_secs(20), async {
        tokio::join!(ae.connect(target, BUSINESS), async {
            be.endpoint().accept().await.unwrap().await
        })
    })
    .await
    .unwrap();
    let client = client.unwrap();
    let server = server.unwrap();
    assert_eq!(client.remote_id(), be.endpoint().id());
    assert!(a.admission_gate().allows(&client, ProtocolScope::Member));
    assert!(
        client
            .paths()
            .iter()
            .all(|p| matches!(p.remote_addr(), TransportAddr::Ip(_)))
    );
    let business = b"private business after IRC bootstrap";
    tokio::join!(
        async {
            let (mut send, mut recv) = client.open_bi().await.unwrap();
            send.write_all(business).await.unwrap();
            send.finish().unwrap();
            assert_eq!(recv.read_to_end(32).await.unwrap(), b"private ACK");
        },
        async {
            let (mut send, mut recv) = server.accept_bi().await.unwrap();
            assert_eq!(recv.read_to_end(128).await.unwrap(), business);
            send.write_all(b"private ACK").await.unwrap();
            send.finish().unwrap();
        }
    );
    println!(
        "IRC_FIRST_PASS stable_id={} same_connection=true real_member=true udp_data=true",
        client.stable_id()
    );
    ae.close().await;
    be.close().await;
    a.close().await;
    b.close().await;
}
