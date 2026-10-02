//! Public node/Connection/stream behavior for same-session path maintenance.
use iroh::{EndpointAddr, TransportAddr, endpoint::Connection};
use miopunch_native_probe::{
    admission_gate::{ADMISSION_REQUEST_ALPN, ProtocolScope},
    manual_admission::{AdmissionStage, ManualExchange},
    native_network::NativeNetwork,
    resources::{ResourceConfig, ResourceEndpoint},
};
use std::{net::Ipv4Addr, time::Duration};
const BUSINESS: &[u8] = b"miopunch/path-maintenance/1";

struct Pair {
    _dir: tempfile::TempDir,
    admin: NativeNetwork,
    joining: NativeNetwork,
    client: ResourceEndpoint,
    server: ResourceEndpoint,
}
impl Pair {
    async fn new(udp: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut admin = NativeNetwork::create(dir.path().join("admin"))
            .await
            .unwrap();
        let mut joining = NativeNetwork::join(
            dir.path().join("joining"),
            admin.manual_invitation().unwrap(),
        )
        .await
        .unwrap();
        for node in [&admin, &joining] {
            let mut config = ResourceConfig::default();
            config.replacement = Some(vec![]);
            config.tcp_listen = Some("127.0.0.1:0".parse().unwrap());
            config.udp_direct_disabled = !udp;
            node.save_resource_configuration(&config).unwrap();
            node.admission_gate()
                .register_protocol(BUSINESS, ProtocolScope::Member);
        }
        let server = admin
            .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap();
        server
            .endpoint()
            .set_alpns(vec![ADMISSION_REQUEST_ALPN.to_vec(), BUSINESS.to_vec()]);
        let mut client = joining
            .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap();
        let (out, incoming) = connect(
            &mut client,
            &server,
            tcp_target(&server),
            ADMISSION_REQUEST_ALPN,
        )
        .await;
        let (request, served) = tokio::join!(
            joining.request_manual_admission(&out),
            admin.serve_manual_admission(&incoming)
        );
        assert!(matches!(request.unwrap(), ManualExchange::AwaitingApproval));
        assert_eq!(served.unwrap(), AdmissionStage::AwaitingApproval);
        let request = admin.manual_requests().await.unwrap().pop().unwrap();
        admin.approve_manual_admission(&request).await.unwrap();
        let receive = async {
            let ManualExchange::Materials(materials) =
                joining.request_manual_admission(&out).await.unwrap()
            else {
                panic!("approved materials")
            };
            joining.complete_manual_admission(materials).await.unwrap()
        };
        let (saved, served) = tokio::join!(receive, admin.serve_manual_admission(&incoming));
        assert_eq!(saved, AdmissionStage::Complete);
        assert_eq!(served.unwrap(), AdmissionStage::Complete);
        out.close(0u32.into(), b"admitted");
        incoming.close(0u32.into(), b"admitted");
        // Rebind the admitted nodes so the initial contact is not replaced by
        // a cached route from the admission fixture.
        client.close().await;
        server.close().await;
        let server = admin
            .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap();
        server.endpoint().set_alpns(vec![BUSINESS.to_vec()]);
        let client = joining
            .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap();
        Self {
            _dir: dir,
            admin,
            joining,
            client,
            server,
        }
    }
    async fn reconfigure(&mut self, resources: Vec<miopunch_native_probe::resources::Resource>) {
        for node in [&self.admin, &self.joining] {
            let mut config = node.resource_configuration().unwrap();
            config.replacement = Some(resources.clone());
            node.save_resource_configuration(&config).unwrap();
        }
        // Only the configured endpoints restart; durable admission is retained.
        let new_server = self
            .admin
            .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap();
        new_server
            .endpoint()
            .set_alpns(vec![ADMISSION_REQUEST_ALPN.to_vec(), BUSINESS.to_vec()]);
        let new_client = self
            .joining
            .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap();
        std::mem::replace(&mut self.client, new_client)
            .close()
            .await;
        std::mem::replace(&mut self.server, new_server)
            .close()
            .await;
    }
    async fn business(&mut self) -> (Connection, Connection) {
        let target = tcp_target(&self.server);
        connect(&mut self.client, &self.server, target, BUSINESS).await
    }
    async fn close(mut self) {
        self.client.close().await;
        self.server.close().await;
        self.joining.close().await;
        self.admin.close().await;
    }
}
fn tcp_target(endpoint: &ResourceEndpoint) -> EndpointAddr {
    EndpointAddr::new(endpoint.endpoint().id()).with_addrs(
        endpoint
            .addr()
            .addrs
            .into_iter()
            .filter(|a| matches!(a, TransportAddr::Custom(_))),
    )
}
async fn connect(
    client: &mut ResourceEndpoint,
    server: &ResourceEndpoint,
    target: EndpointAddr,
    alpn: &[u8],
) -> (Connection, Connection) {
    tokio::time::timeout(Duration::from_secs(15), async {
        let (out, incoming) = tokio::join!(client.connect(target, alpn), async {
            server.endpoint().accept().await.unwrap().await
        });
        (out.unwrap(), incoming.unwrap())
    })
    .await
    .expect("bounded authenticated connection")
}
async fn wait_paths(connection: &Connection, condition: impl Fn(&Connection) -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !condition(connection) {
            assert!(
                connection.close_reason().is_none(),
                "original connection terminated"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|error| {
        panic!(
            "qualified path condition: {error:?}; paths={:?}",
            connection
                .paths()
                .iter()
                .map(|p| (p.id(), p.remote_addr().clone(), p.rtt(), p.is_selected()))
                .collect::<Vec<_>>()
        )
    });
}
async fn chunk(
    send: &mut iroh::endpoint::SendStream,
    receive: &mut iroh::endpoint::RecvStream,
    bytes: &[u8],
) {
    send.write_all(bytes).await.unwrap();
    let mut actual = vec![0; bytes.len()];
    tokio::time::timeout(Duration::from_secs(5), receive.read_exact(&mut actual))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(actual, bytes);
}

#[tokio::test]
async fn first_same_stream_survives_validated_tcp_to_udp_switch() {
    let mut pair = Pair::new(true).await;
    let (client, server) = pair.business().await;
    let ids = (client.stable_id(), server.stable_id());
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"before").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut before = [0; 6];
    receive.read_exact(&mut before).await.unwrap();
    assert_eq!(&before, b"before");
    let udp = pair.server.endpoint().bound_sockets()[0];
    pair.client
        .endpoint()
        .open_data_path(&client, udp)
        .await
        .unwrap();
    wait_paths(&client, |c| c.paths().iter().any(|p| p.is_ip())).await;
    assert!(
        client
            .paths()
            .iter()
            .any(|p| matches!(p.remote_addr(), TransportAddr::Custom(_)))
    );
    for path in client
        .paths()
        .iter()
        .filter(|p| matches!(p.remote_addr(), TransportAddr::Custom(_)))
    {
        path.close().unwrap();
    }
    wait_paths(&client, |c| {
        c.paths().iter().any(|p| p.is_ip() && p.is_selected())
    })
    .await;
    chunk(
        &mut send,
        &mut receive,
        b"same stream after TCP to UDP switch",
    )
    .await;
    assert_eq!((client.stable_id(), server.stable_id()), ids);
    eprintln!(
        "MIO47_FIRST stable_ids={ids:?} same_stream=true content_complete=true paths={:?}",
        client.paths()
    );
    pair.close().await;
}

#[tokio::test]
async fn priority_closed_connection_fails_old_stream_and_reports_new_identity() {
    let mut pair = Pair::new(false).await;
    let (client, server) = pair.business().await;
    let old = (client.stable_id(), server.stable_id());
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"old stream").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut before = [0; 10];
    receive.read_exact(&mut before).await.unwrap();
    assert_eq!(&before, b"old stream");
    client.close(0u32.into(), b"original connection terminated");
    let reason = client.closed().await;
    tokio::time::timeout(Duration::from_secs(3), server.closed())
        .await
        .unwrap();
    assert!(send.write_all(b"must fail").await.is_err());
    let mut after = [0; 1];
    assert!(!matches!(receive.read_exact(&mut after).await, Ok(())));
    let (replacement, replacement_server) = pair.business().await;
    let new = (replacement.stable_id(), replacement_server.stable_id());
    assert_ne!(new.0, old.0);
    assert_ne!(new.1, old.1);
    assert!(client.close_reason().is_some());
    let (mut send, _) = replacement.open_bi().await.unwrap();
    send.write_all(b"new connection").await.unwrap();
    let (_, mut receive) = replacement_server.accept_bi().await.unwrap();
    let mut content = [0; 14];
    receive.read_exact(&mut content).await.unwrap();
    assert_eq!(&content, b"new connection");
    eprintln!("MIO47_PRIORITY old={old:?} new={new:?} old_stream_failed=true close={reason:?}");
    pair.close().await;
}

// Controlled byte carrier: route prefaces contain the proxy address; inner
// QUIC/TLS packets are unchanged. Delay/loss are real socket fault injection.
struct TcpProxy {
    address: std::net::SocketAddr,
    delay_ms: std::sync::Arc<std::sync::atomic::AtomicU64>,
    stop: tokio_util::sync::CancellationToken,
    task: tokio::task::JoinHandle<()>,
}
impl TcpProxy {
    async fn new(upstream: std::net::SocketAddr, delay_ms: u64) -> Self {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let delay_ms = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(delay_ms));
        let delay = delay_ms.clone();
        let stop = tokio_util::sync::CancellationToken::new();
        let cancel = stop.clone();
        let task = tokio::spawn(async move {
            let mut circuits = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    accepted = listener.accept() => {
                        let (mut downstream, _) = accepted.unwrap();
                        let delay = delay.clone();
                        circuits.spawn(async move {
                            let mut upstream = tokio::net::TcpStream::connect(upstream).await.unwrap();
                            let mut preface = [0; 6];
                            downstream.read_exact(&mut preface).await.unwrap();
                            let route = miopunch_native_probe::tcp_transport::TcpTransport::address(address);
                            upstream.write_all(route.data()).await.unwrap();
                            upstream.read_exact(&mut preface).await.unwrap();
                            downstream.write_all(route.data()).await.unwrap();
                            let (dr, dw) = downstream.into_split(); let (ur, uw) = upstream.into_split();
                            let _ = tokio::join!(forward_frames(dr, uw, delay.clone()), forward_frames(ur, dw, delay));
                        });
                    }
                    Some(result) = circuits.join_next(), if !circuits.is_empty() => { result.unwrap(); }
                }
            }
            circuits.shutdown().await;
        });
        Self {
            address,
            delay_ms,
            stop,
            task,
        }
    }
    fn custom(&self) -> iroh_base::CustomAddr {
        miopunch_native_probe::tcp_transport::TcpTransport::address(self.address)
    }
    fn delay(&self, ms: u64) {
        self.delay_ms
            .store(ms, std::sync::atomic::Ordering::Relaxed);
    }
    async fn close(self) {
        self.stop.cancel();
        self.task.await.unwrap();
    }
}
async fn forward_frames(
    mut read: tokio::net::tcp::OwnedReadHalf,
    mut write: tokio::net::tcp::OwnedWriteHalf,
    delay: std::sync::Arc<std::sync::atomic::AtomicU64>,
) -> std::io::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    loop {
        let len = read.read_u16().await?;
        assert!((1..=2048).contains(&len));
        let mut bytes = vec![0; len as usize];
        read.read_exact(&mut bytes).await?;
        tokio::time::sleep(Duration::from_millis(
            delay.load(std::sync::atomic::Ordering::Relaxed),
        ))
        .await;
        write.write_u16(len).await?;
        write.write_all(&bytes).await?;
    }
}

#[tokio::test]
async fn living_paths_are_reevaluated_when_real_latency_changes() {
    let mut pair = Pair::new(false).await;
    let target = tcp_target(&pair.server);
    let TransportAddr::Custom(address) = target.addrs.iter().next().unwrap() else {
        unreachable!()
    };
    let upstream =
        miopunch_native_probe::tcp_transport::TcpTransport::socket_addr(address).unwrap();
    let first = TcpProxy::new(upstream, 2).await;
    let second = TcpProxy::new(upstream, 30).await;
    let target = EndpointAddr::new(pair.server.endpoint().id())
        .with_addrs([TransportAddr::Custom(first.custom())]);
    let (client, server) = connect(&mut pair.client, &pair.server, target, BUSINESS).await;
    let ids = (client.stable_id(), server.stable_id());
    eprintln!(
        "MIO47_FIXTURE first={} second={} initial={:?}",
        first.address,
        second.address,
        client.paths()
    );
    pair.client
        .endpoint()
        .open_data_transport_path(&client, TransportAddr::Custom(second.custom()))
        .await
        .unwrap();
    wait_paths(&client, |c| {
        c.paths()
            .iter()
            .any(|p| p.remote_addr() == &TransportAddr::Custom(second.custom()))
    })
    .await;
    eprintln!("MIO47_FIXTURE both_established={:?}", client.paths());
    let path_ids = client
        .paths()
        .iter()
        .map(|p| p.id())
        .collect::<std::collections::BTreeSet<_>>();
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"before").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut before = [0; 6];
    receive.read_exact(&mut before).await.unwrap();
    assert_eq!(&before, b"before");
    wait_paths(&client, |c| {
        c.paths()
            .iter()
            .any(|p| p.remote_addr() == &TransportAddr::Custom(first.custom()) && p.is_selected())
    })
    .await;
    first.delay(80);
    second.delay(0);
    for _ in 0..12 {
        chunk(&mut send, &mut receive, b"same stream real latency change").await;
    }
    eprintln!(
        "MIO47_QUALITY before_wait paths={:?}",
        client
            .paths()
            .iter()
            .map(|p| (p.id(), p.remote_addr().clone(), p.rtt(), p.is_selected()))
            .collect::<Vec<_>>()
    );
    wait_paths(&client, |c| {
        c.paths()
            .iter()
            .any(|p| p.remote_addr() == &TransportAddr::Custom(second.custom()) && p.is_selected())
    })
    .await;
    assert_eq!(
        client
            .paths()
            .iter()
            .map(|p| p.id())
            .collect::<std::collections::BTreeSet<_>>(),
        path_ids,
        "quality reevaluation must not depend on opening/replacing paths"
    );
    chunk(&mut send, &mut receive, b"after quality switch").await;
    assert_eq!((client.stable_id(), server.stable_id()), ids);
    eprintln!(
        "MIO47_QUALITY stable_ids={ids:?} same_stream=true same_paths=true selected={:?}",
        client
            .paths()
            .iter()
            .filter(|p| p.is_selected())
            .map(|p| p.remote_addr().clone())
            .collect::<Vec<_>>()
    );
    pair.close().await;
    first.close().await;
    second.close().await;
}

#[tokio::test]
async fn restricted_control_paths_do_not_displace_business_quality_selection() {
    use miopunch_native_probe::{
        mqtt_transport::TRANSPORT_ID,
        resources::{Resource, ResourceUse},
    };
    let broker = std::env::var("MIOPUNCH_MQTT_BROKER").expect("owned MQTT broker required");
    let mut pair = Pair::new(false).await;
    pair.reconfigure(vec![Resource {
        url: format!("mqtt://{broker}"),
        uses: std::collections::BTreeSet::from([ResourceUse::Signaling]),
        limitation: "owned controlled test broker".into(),
    }])
    .await;
    let route = pair
        .server
        .endpoint()
        .custom_addrs()
        .into_iter()
        .find(|a| a.id() == TRANSPORT_ID)
        .unwrap();
    let target = EndpointAddr::new(pair.server.endpoint().id())
        .with_addrs([TransportAddr::Custom(route.clone())]);
    let (control, control_server) = tokio::time::timeout(Duration::from_secs(10), async {
        let (client, server) = tokio::join!(
            pair.client
                .endpoint()
                .connect(target, ADMISSION_REQUEST_ALPN),
            async { pair.server.endpoint().accept().await.unwrap().await }
        );
        (client.unwrap(), server.unwrap())
    })
    .await
    .unwrap();
    let target = tcp_target(&pair.server);
    let TransportAddr::Custom(address) = target.addrs.iter().next().unwrap() else {
        unreachable!()
    };
    let upstream =
        miopunch_native_probe::tcp_transport::TcpTransport::socket_addr(address).unwrap();
    let first = TcpProxy::new(upstream, 2).await;
    let second = TcpProxy::new(upstream, 30).await;
    let target = EndpointAddr::new(pair.server.endpoint().id())
        .with_addrs([TransportAddr::Custom(first.custom())]);
    let (client, server) = connect(&mut pair.client, &pair.server, target, BUSINESS).await;
    pair.client
        .endpoint()
        .open_data_transport_path(&client, TransportAddr::Custom(second.custom()))
        .await
        .unwrap();
    wait_paths(&client, |c| {
        c.paths()
            .iter()
            .any(|p| p.remote_addr() == &TransportAddr::Custom(second.custom()))
    })
    .await;
    let ids = (client.stable_id(), server.stable_id());
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"before").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut before = [0; 6];
    receive.read_exact(&mut before).await.unwrap();
    wait_paths(&client, |c| {
        c.paths()
            .iter()
            .any(|p| p.remote_addr() == &TransportAddr::Custom(first.custom()) && p.is_selected())
    })
    .await;
    first.delay(100);
    second.delay(0);
    for _ in 0..12 {
        chunk(
            &mut send,
            &mut receive,
            b"business beside live protected control",
        )
        .await;
    }
    wait_paths(&client, |c| {
        c.paths()
            .iter()
            .any(|p| p.remote_addr() == &TransportAddr::Custom(second.custom()) && p.is_selected())
    })
    .await;
    assert_eq!((client.stable_id(), server.stable_id()), ids);
    assert!(control.close_reason().is_none());
    assert!(control_server.close_reason().is_none());
    assert!(
        client.paths().iter().all(
            |p| !matches!(p.remote_addr(), TransportAddr::Custom(a) if a.id() == TRANSPORT_ID)
        )
    );
    assert!(
        pair.client
            .endpoint()
            .open_data_transport_path(&client, TransportAddr::Custom(route))
            .await
            .is_err()
    );
    chunk(
        &mut send,
        &mut receive,
        b"signaling remains forbidden for business",
    )
    .await;
    eprintln!(
        "MIO47_ISOLATION same_stream=true stable_ids={ids:?} control_alive=true signaling_business_paths=0"
    );
    pair.close().await;
    first.close().await;
    second.close().await;
}

async fn native_relay() -> iroh_relay::server::Server {
    let mut config = iroh_relay::server::ServerConfig::default();
    config.relay = Some(iroh_relay::server::RelayConfig::new((
        Ipv4Addr::LOCALHOST,
        0,
    )));
    iroh_relay::server::Server::spawn(config).await.unwrap()
}
fn relay_resource(url: &iroh::RelayUrl) -> miopunch_native_probe::resources::Resource {
    miopunch_native_probe::resources::Resource {
        url: url.to_string(),
        uses: std::collections::BTreeSet::from([
            miopunch_native_probe::resources::ResourceUse::DataRelay,
        ]),
        limitation: "owned native relay; real endpoint TLS, no public availability claim".into(),
    }
}
async fn open_carrier(endpoint: &ResourceEndpoint, connection: &Connection, remote: TransportAddr) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match endpoint
                .endpoint()
                .open_data_transport_path(connection, remote.clone())
                .await
            {
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    tokio::time::sleep(Duration::from_millis(50)).await
                }
                result => {
                    result.unwrap();
                    break;
                }
            }
        }
    })
    .await
    .unwrap();
    wait_paths(connection, |c| {
        c.paths().iter().any(|p| p.remote_addr() == &remote)
    })
    .await;
}

#[tokio::test]
async fn quality_selects_better_native_relay_and_keeps_tcp_as_existing_backup() {
    let service = native_relay().await;
    let url = service.http_url().unwrap();
    let mut pair = Pair::new(false).await;
    pair.reconfigure(vec![relay_resource(&url)]).await;
    let target = tcp_target(&pair.server);
    let TransportAddr::Custom(address) = target.addrs.iter().next().unwrap() else {
        unreachable!()
    };
    let upstream =
        miopunch_native_probe::tcp_transport::TcpTransport::socket_addr(address).unwrap();
    let proxy = TcpProxy::new(upstream, 30).await;
    let target = EndpointAddr::new(pair.server.endpoint().id())
        .with_addrs([TransportAddr::Custom(proxy.custom())]);
    let (client, server) = tokio::time::timeout(Duration::from_secs(15), async {
        let (out, incoming) =
            tokio::join!(pair.client.endpoint().connect(target, BUSINESS), async {
                pair.server.endpoint().accept().await.unwrap().await
            });
        (out.unwrap(), incoming.unwrap())
    })
    .await
    .unwrap();
    let ids = (client.stable_id(), server.stable_id());
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"TCP").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut before = [0; 3];
    receive.read_exact(&mut before).await.unwrap();
    assert_eq!(&before, b"TCP");
    open_carrier(&pair.client, &client, TransportAddr::Relay(url.clone())).await;
    wait_paths(&client, |c| {
        c.paths().iter().any(|p| p.is_relay() && p.is_selected())
    })
    .await;
    let paths = client
        .paths()
        .iter()
        .map(|p| (p.remote_addr().clone(), p.rtt(), p.is_selected()))
        .collect::<Vec<_>>();
    eprintln!("MIO47_RELAY_QUALITY before_cut={paths:?}");
    assert!(
        client
            .paths()
            .iter()
            .any(|p| matches!(p.remote_addr(), TransportAddr::Custom(_)))
    );
    chunk(
        &mut send,
        &mut receive,
        b"same stream on better permitted native relay",
    )
    .await;
    // Removing the selected carrier retains the already validated TCP backup.
    for path in client.paths().iter().filter(|p| p.is_relay()) {
        path.close().unwrap();
    }
    wait_paths(&client, |c| {
        c.paths()
            .iter()
            .any(|p| matches!(p.remote_addr(), TransportAddr::Custom(_)) && p.is_selected())
    })
    .await;
    chunk(
        &mut send,
        &mut receive,
        b"same stream on surviving TCP backup",
    )
    .await;
    assert_eq!((client.stable_id(), server.stable_id()), ids);
    eprintln!(
        "MIO47_MATRIX TCP->Relay->TCP stable_ids={ids:?} same_stream=true content_complete=true"
    );
    pair.close().await;
    proxy.close().await;
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn control_paths_recover_a_new_tcp_carrier_within_the_original_connection_window() {
    use miopunch_native_probe::{
        connection_recovery::CONNECTION_RECOVERY_ALPN,
        mqtt_transport::TRANSPORT_ID,
        resources::{Resource, ResourceUse},
    };
    let broker = std::env::var("MIOPUNCH_MQTT_BROKER").expect("owned MQTT broker required");
    let mut pair = Pair::new(false).await;
    pair.reconfigure(vec![Resource {
        url: format!("mqtt://{broker}"),
        uses: std::collections::BTreeSet::from([ResourceUse::Signaling]),
        limitation: "owned broker for protected path repair".into(),
    }])
    .await;
    pair.server
        .endpoint()
        .set_alpns(vec![BUSINESS.to_vec(), CONNECTION_RECOVERY_ALPN.to_vec()]);
    let route = pair
        .server
        .endpoint()
        .custom_addrs()
        .into_iter()
        .find(|a| a.id() == TRANSPORT_ID)
        .unwrap();
    let control_target =
        EndpointAddr::new(pair.server.endpoint().id()).with_addrs([TransportAddr::Custom(route)]);
    let target = tcp_target(&pair.server);
    let TransportAddr::Custom(address) = target.addrs.iter().next().unwrap() else {
        unreachable!()
    };
    let upstream =
        miopunch_native_probe::tcp_transport::TcpTransport::socket_addr(address).unwrap();
    let proxy = TcpProxy::new(upstream, 0).await;
    let target = EndpointAddr::new(pair.server.endpoint().id())
        .with_addrs([TransportAddr::Custom(proxy.custom())]);
    let (client, server) = connect(&mut pair.client, &pair.server, target, BUSINESS).await;
    let ids = (client.stable_id(), server.stable_id());
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"before cut").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut before = [0; 10];
    receive.read_exact(&mut before).await.unwrap();
    assert_eq!(&before, b"before cut");
    assert_eq!(client.paths().len(), 1, "no pre-established backup");
    proxy.close().await;
    assert!(
        client.close_reason().is_none(),
        "repair starts only within original QUIC lifetime"
    );
    let started = std::time::Instant::now();
    let served = async {
        let control = pair
            .server
            .endpoint()
            .accept()
            .await
            .unwrap()
            .await
            .unwrap();
        let result = pair
            .admin
            .serve_connection_path_recovery(&pair.server, &control)
            .await;
        control.close(0u32.into(), b"path repair complete");
        result
    };
    let (recovered, served) = tokio::join!(
        pair.joining
            .recover_connection_path(&pair.client, &client, control_target),
        served
    );
    eprintln!("MIO47_CONTROL_RESULT recovered={recovered:?} served={served:?}");
    let recovered = recovered.unwrap();
    served.unwrap();
    assert_eq!(recovered.stable_id, ids.0);
    chunk(
        &mut send,
        &mut receive,
        b"same original stream after protected path recovery",
    )
    .await;
    assert_eq!((client.stable_id(), server.stable_id()), ids);
    assert!(
        client.paths().iter().all(
            |p| !matches!(p.remote_addr(), TransportAddr::Custom(a) if a.id() == TRANSPORT_ID)
        )
    );
    eprintln!(
        "MIO47_CONTROL_RECOVERY stable_ids={ids:?} same_stream=true new_carrier={:?} elapsed_ms={}",
        recovered.path,
        started.elapsed().as_millis()
    );
    pair.close().await;
}

#[tokio::test]
async fn control_paths_never_recover_a_closed_business_connection() {
    use miopunch_native_probe::{
        connection_recovery::ConnectionPathRecoveryError,
        mqtt_transport::TRANSPORT_ID,
        resources::{Resource, ResourceUse},
    };
    let broker = std::env::var("MIOPUNCH_MQTT_BROKER").expect("owned MQTT broker required");
    let mut pair = Pair::new(false).await;
    pair.reconfigure(vec![Resource {
        url: format!("mqtt://{broker}"),
        uses: std::collections::BTreeSet::from([ResourceUse::Signaling]),
        limitation: "owned control-only broker".into(),
    }])
    .await;
    let route = pair
        .server
        .endpoint()
        .custom_addrs()
        .into_iter()
        .find(|a| a.id() == TRANSPORT_ID)
        .unwrap();
    let contact =
        EndpointAddr::new(pair.server.endpoint().id()).with_addrs([TransportAddr::Custom(route)]);
    let (control, control_server) = tokio::time::timeout(Duration::from_secs(10), async {
        let (out, incoming) = tokio::join!(
            pair.client
                .endpoint()
                .connect(contact.clone(), ADMISSION_REQUEST_ALPN),
            async { pair.server.endpoint().accept().await.unwrap().await }
        );
        (out.unwrap(), incoming.unwrap())
    })
    .await
    .unwrap();
    let (client, server) = pair.business().await;
    let old = client.stable_id();
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"old").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut old_bytes = [0; 3];
    receive.read_exact(&mut old_bytes).await.unwrap();
    client.close(0u32.into(), b"terminal old session");
    tokio::time::timeout(Duration::from_secs(3), server.closed())
        .await
        .unwrap();
    let result = pair
        .joining
        .recover_connection_path(&pair.client, &client, contact)
        .await;
    assert!(
        matches!(result, Err(ConnectionPathRecoveryError::Terminal { stable_id }) if stable_id == old)
    );
    assert!(control.close_reason().is_none());
    assert!(control_server.close_reason().is_none());
    assert!(send.write_all(b"old must fail").await.is_err());
    let (replacement, replacement_server) = pair.business().await;
    assert_ne!(replacement.stable_id(), old);
    let (mut new_send, _) = replacement.open_bi().await.unwrap();
    new_send.write_all(b"new").await.unwrap();
    let (_, mut new_receive) = replacement_server.accept_bi().await.unwrap();
    let mut bytes = [0; 3];
    new_receive.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"new");
    eprintln!(
        "MIO47_CLOSED_WITH_CONTROL old={old} new={} terminal=true control_alive=true old_stream_failed=true",
        replacement.stable_id()
    );
    pair.close().await;
}

#[tokio::test]
async fn explicit_relay_preference_switches_the_same_udp_stream_and_retains_udp_backup() {
    use miopunch_native_probe::path_selection::PathPreference;
    let service = native_relay().await;
    let url = service.http_url().unwrap();
    let mut pair = Pair::new(true).await;
    for node in [&pair.admin, &pair.joining] {
        let mut config = node.resource_configuration().unwrap();
        config.path_selection.preference = PathPreference::PreferRelay;
        node.save_resource_configuration(&config).unwrap();
    }
    pair.reconfigure(vec![relay_resource(&url)]).await;
    let address = pair.server.endpoint().bound_sockets()[0];
    let target =
        EndpointAddr::new(pair.server.endpoint().id()).with_addrs([TransportAddr::Ip(address)]);
    let (client, server) = tokio::time::timeout(Duration::from_secs(15), async {
        let (out, incoming) =
            tokio::join!(pair.client.endpoint().connect(target, BUSINESS), async {
                pair.server.endpoint().accept().await.unwrap().await
            });
        (out.unwrap(), incoming.unwrap())
    })
    .await
    .unwrap();
    let ids = (client.stable_id(), server.stable_id());
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"UDP").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut before = [0; 3];
    receive.read_exact(&mut before).await.unwrap();
    assert_eq!(&before, b"UDP");
    open_carrier(&pair.client, &client, TransportAddr::Relay(url.clone())).await;
    wait_paths(&client, |c| {
        c.paths().iter().any(|p| p.is_relay() && p.is_selected())
    })
    .await;
    assert!(client.paths().iter().any(|p| p.is_ip()));
    chunk(
        &mut send,
        &mut receive,
        b"same UDP stream on preferred relay",
    )
    .await;
    for path in client.paths().iter().filter(|p| p.is_relay()) {
        path.close().unwrap();
    }
    wait_paths(&client, |c| {
        c.paths().iter().any(|p| p.is_ip() && p.is_selected())
    })
    .await;
    chunk(
        &mut send,
        &mut receive,
        b"same stream restored to UDP backup",
    )
    .await;
    assert_eq!((client.stable_id(), server.stable_id()), ids);
    eprintln!(
        "MIO47_MATRIX UDP->Relay->UDP stable_ids={ids:?} preference=PreferRelay same_stream=true content_complete=true"
    );
    pair.close().await;
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn explicit_direct_preference_wins_even_when_permitted_relay_is_faster() {
    use miopunch_native_probe::path_selection::PathPreference;
    let service = native_relay().await;
    let url = service.http_url().unwrap();
    let mut pair = Pair::new(false).await;
    for node in [&pair.admin, &pair.joining] {
        let mut config = node.resource_configuration().unwrap();
        config.path_selection.preference = PathPreference::PreferDirect;
        node.save_resource_configuration(&config).unwrap();
    }
    pair.reconfigure(vec![relay_resource(&url)]).await;
    let target = tcp_target(&pair.server);
    let TransportAddr::Custom(address) = target.addrs.iter().next().unwrap() else {
        unreachable!()
    };
    let proxy = TcpProxy::new(
        miopunch_native_probe::tcp_transport::TcpTransport::socket_addr(address).unwrap(),
        30,
    )
    .await;
    let target = EndpointAddr::new(pair.server.endpoint().id())
        .with_addrs([TransportAddr::Custom(proxy.custom())]);
    let (client, server) = tokio::time::timeout(Duration::from_secs(15), async {
        let (out, incoming) =
            tokio::join!(pair.client.endpoint().connect(target, BUSINESS), async {
                pair.server.endpoint().accept().await.unwrap().await
            });
        (out.unwrap(), incoming.unwrap())
    })
    .await
    .unwrap();
    let ids = (client.stable_id(), server.stable_id());
    open_carrier(&pair.client, &client, TransportAddr::Relay(url)).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let tcp = client
        .paths()
        .iter()
        .find(|p| matches!(p.remote_addr(), TransportAddr::Custom(_)))
        .unwrap()
        .rtt();
    let relay = client.paths().iter().find(|p| p.is_relay()).unwrap().rtt();
    assert!(
        relay < tcp,
        "actual relay must be better in this fixture: {relay:?} vs {tcp:?}"
    );
    assert!(
        client
            .paths()
            .iter()
            .any(|p| matches!(p.remote_addr(), TransportAddr::Custom(_)) && p.is_selected())
    );
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"explicit preference").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut bytes = [0; 19];
    receive.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"explicit preference");
    assert_eq!((client.stable_id(), server.stable_id()), ids);
    eprintln!(
        "MIO47_PREFERENCE direct_rtt={tcp:?} relay_rtt={relay:?} selected=TCP stable_ids={ids:?}"
    );
    pair.close().await;
    proxy.close().await;
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn disabled_relay_is_never_opened_or_selected_even_with_explicit_relay_preference() {
    use miopunch_native_probe::{path_selection::PathPreference, resources::ResourceUse};
    let service = native_relay().await;
    let url = service.http_url().unwrap();
    let mut pair = Pair::new(false).await;
    for node in [&pair.admin, &pair.joining] {
        let mut config = node.resource_configuration().unwrap();
        config.path_selection.preference = PathPreference::PreferRelay;
        config
            .disabled
            .insert((url.to_string(), ResourceUse::DataRelay));
        node.save_resource_configuration(&config).unwrap();
    }
    pair.reconfigure(vec![relay_resource(&url)]).await;
    let (client, server) = pair.business().await;
    let ids = (client.stable_id(), server.stable_id());
    pair.client
        .endpoint()
        .open_data_transport_path(&client, TransportAddr::Relay(url))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(client.paths().iter().all(|p| !p.is_relay()));
    assert_eq!(
        service.metrics().server.accepts.get(),
        0,
        "no access to disabled origin"
    );
    assert_eq!(service.metrics().server.bytes_recv.get(), 0);
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"allowed TCP").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut bytes = [0; 11];
    receive.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"allowed TCP");
    assert_eq!((client.stable_id(), server.stable_id()), ids);
    eprintln!(
        "MIO47_DISABLED preference=PreferRelay actual_relay_accesses=0 relay_paths=0 TCP_stream=complete stable_ids={ids:?}"
    );
    pair.close().await;
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn control_paths_stay_live_while_physical_business_loss_ends_queued_bytes() {
    use miopunch_native_probe::{
        connection_recovery::ConnectionPathRecoveryError,
        mqtt_transport::TRANSPORT_ID,
        resources::{Resource, ResourceUse},
    };
    let broker = std::env::var("MIOPUNCH_MQTT_BROKER").expect("owned MQTT broker required");
    let mut pair = Pair::new(false).await;
    pair.reconfigure(vec![Resource {
        url: format!("mqtt://{broker}"),
        uses: std::collections::BTreeSet::from([ResourceUse::Signaling]),
        limitation: "owned control-only broker".into(),
    }])
    .await;
    let route = pair
        .server
        .endpoint()
        .custom_addrs()
        .into_iter()
        .find(|a| a.id() == TRANSPORT_ID)
        .unwrap();
    let contact =
        EndpointAddr::new(pair.server.endpoint().id()).with_addrs([TransportAddr::Custom(route)]);
    let (control, control_server) = tokio::time::timeout(Duration::from_secs(10), async {
        let (out, incoming) = tokio::join!(
            pair.client
                .endpoint()
                .connect(contact.clone(), ADMISSION_REQUEST_ALPN),
            async { pair.server.endpoint().accept().await.unwrap().await }
        );
        (out.unwrap(), incoming.unwrap())
    })
    .await
    .unwrap();
    let target = tcp_target(&pair.server);
    let TransportAddr::Custom(address) = target.addrs.iter().next().unwrap() else {
        unreachable!()
    };
    let proxy = TcpProxy::new(
        miopunch_native_probe::tcp_transport::TcpTransport::socket_addr(address).unwrap(),
        0,
    )
    .await;
    let target = EndpointAddr::new(pair.server.endpoint().id())
        .with_addrs([TransportAddr::Custom(proxy.custom())]);
    let (client, server) = connect(&mut pair.client, &pair.server, target, BUSINESS).await;
    let old = client.stable_id();
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"before cut").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut before = [0; 10];
    receive.read_exact(&mut before).await.unwrap();
    assert_eq!(client.paths().len(), 1);
    proxy.close().await;
    let started = std::time::Instant::now();
    send.write_all(&vec![0x5a; 65_536]).await.unwrap();
    send.finish().unwrap();
    let reason = tokio::time::timeout(Duration::from_secs(16), client.closed())
        .await
        .expect("actual noq idle/path lifetime ends business");
    assert!(send.stopped().await.is_err());
    let mut bytes = [0; 1];
    assert!(receive.read_exact(&mut bytes).await.is_err());
    assert!(control.close_reason().is_none());
    assert!(control_server.close_reason().is_none());
    let result = pair
        .joining
        .recover_connection_path(&pair.client, &client, contact)
        .await;
    assert!(
        matches!(result, Err(ConnectionPathRecoveryError::Terminal { stable_id }) if stable_id == old)
    );
    let (new, _) = pair.business().await;
    assert_ne!(new.stable_id(), old);
    eprintln!(
        "MIO47_PHYSICAL_TERMINAL old={old} new={} close={reason:?} queued_bytes=65536 control_alive=true elapsed_ms={}",
        new.stable_id(),
        started.elapsed().as_millis()
    );
    pair.close().await;
}

#[tokio::test]
async fn control_paths_reject_an_unknown_original_tls_binding_without_changing_business() {
    use miopunch_native_probe::{
        connection_recovery::CONNECTION_RECOVERY_ALPN,
        mqtt_transport::TRANSPORT_ID,
        resources::{Resource, ResourceUse},
    };
    let broker = std::env::var("MIOPUNCH_MQTT_BROKER").expect("owned MQTT broker required");
    let mut pair = Pair::new(false).await;
    pair.reconfigure(vec![Resource {
        url: format!("mqtt://{broker}"),
        uses: std::collections::BTreeSet::from([ResourceUse::Signaling]),
        limitation: "owned broker for binding rejection".into(),
    }])
    .await;
    pair.server
        .endpoint()
        .set_alpns(vec![BUSINESS.to_vec(), CONNECTION_RECOVERY_ALPN.to_vec()]);
    let (client, server) = pair.business().await;
    let ids = (client.stable_id(), server.stable_id());
    let route = pair
        .server
        .endpoint()
        .custom_addrs()
        .into_iter()
        .find(|a| a.id() == TRANSPORT_ID)
        .unwrap();
    let contact =
        EndpointAddr::new(pair.server.endpoint().id()).with_addrs([TransportAddr::Custom(route)]);
    let (control, control_server) = tokio::time::timeout(Duration::from_secs(10), async {
        let (out, incoming) = tokio::join!(
            pair.client
                .endpoint()
                .connect(contact, CONNECTION_RECOVERY_ALPN),
            async { pair.server.endpoint().accept().await.unwrap().await }
        );
        (out.unwrap(), incoming.unwrap())
    })
    .await
    .unwrap();
    let request = async {
        let (mut send, mut recv) = control.open_bi().await.unwrap();
        let request = serde_json::json!({"binding": vec![0u8; 32], "offer": pair.client.addr()});
        send.write_all(&serde_json::to_vec(&request).unwrap())
            .await
            .unwrap();
        send.finish().unwrap();
        let result = recv.read_to_end(4096).await;
        assert!(
            !matches!(result, Ok(ref bytes) if !bytes.is_empty()),
            "invalid binding must not disclose a path offer"
        );
    };
    let serve = async {
        let error = pair
            .admin
            .serve_connection_path_recovery(&pair.server, &control_server)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::NotConnected);
        control_server.close(0u32.into(), b"unknown original binding");
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(request, serve);
    })
    .await
    .unwrap();
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"original still usable").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut bytes = [0; 21];
    receive.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"original still usable");
    assert_eq!((client.stable_id(), server.stable_id()), ids);
    eprintln!(
        "MIO47_BINDING unknown_binding=rejected offered_candidates=0 original_business=usable stable_ids={ids:?}"
    );
    pair.close().await;
}

#[tokio::test]
async fn control_paths_recover_when_the_original_accepting_side_initiates() {
    use miopunch_native_probe::{
        connection_recovery::CONNECTION_RECOVERY_ALPN,
        mqtt_transport::TRANSPORT_ID,
        resources::{Resource, ResourceUse},
    };
    let broker = std::env::var("MIOPUNCH_MQTT_BROKER").expect("owned MQTT broker required");
    let mut pair = Pair::new(false).await;
    pair.reconfigure(vec![Resource {
        url: format!("mqtt://{broker}"),
        uses: std::collections::BTreeSet::from([ResourceUse::Signaling]),
        limitation: "owned broker for accepting-side repair".into(),
    }])
    .await;
    pair.client
        .endpoint()
        .set_alpns(vec![CONNECTION_RECOVERY_ALPN.to_vec()]);
    let route = pair
        .client
        .endpoint()
        .custom_addrs()
        .into_iter()
        .find(|a| a.id() == TRANSPORT_ID)
        .unwrap();
    let contact =
        EndpointAddr::new(pair.client.endpoint().id()).with_addrs([TransportAddr::Custom(route)]);
    let target = tcp_target(&pair.server);
    let TransportAddr::Custom(address) = target.addrs.iter().next().unwrap() else {
        unreachable!()
    };
    let proxy = TcpProxy::new(
        miopunch_native_probe::tcp_transport::TcpTransport::socket_addr(address).unwrap(),
        0,
    )
    .await;
    let target = EndpointAddr::new(pair.server.endpoint().id())
        .with_addrs([TransportAddr::Custom(proxy.custom())]);
    let (client, server) = connect(&mut pair.client, &pair.server, target, BUSINESS).await;
    let ids = (client.stable_id(), server.stable_id());
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"before cut").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut before = [0; 10];
    receive.read_exact(&mut before).await.unwrap();
    assert_eq!(client.paths().len(), 1);
    proxy.close().await;
    let served = async {
        let control = pair
            .client
            .endpoint()
            .accept()
            .await
            .unwrap()
            .await
            .unwrap();
        pair.joining
            .serve_connection_path_recovery(&pair.client, &control)
            .await
    };
    let (recovered, served) = tokio::join!(
        pair.admin
            .recover_connection_path(&pair.server, &server, contact),
        served
    );
    eprintln!("MIO47_REVERSE_CONTROL_RESULT recovered={recovered:?} served={served:?}");
    let recovered = recovered.unwrap();
    served.unwrap();
    assert_eq!(recovered.stable_id, ids.1);
    chunk(
        &mut send,
        &mut receive,
        b"same stream after accepting-side control coordination",
    )
    .await;
    assert_eq!((client.stable_id(), server.stable_id()), ids);
    eprintln!(
        "MIO47_REVERSE_CONTROL same_stream=true original_stable_ids={ids:?} repaired_by=original_dialing_peer"
    );
    pair.close().await;
}

// This boundary fixture cuts the real TCP listener/circuits while preserving
// the endpoint's advertised address, as happens during an unchanged-IP outage.
#[derive(Debug)]
struct CuttableTcp {
    inner: miopunch_native_probe::tcp_transport::TcpTransport,
    shutdown: std::sync::Mutex<
        Option<std::sync::Arc<dyn iroh::endpoint::transports::CustomEndpointShutdown>>,
    >,
}
impl iroh::endpoint::transports::CustomTransport for CuttableTcp {
    fn bind(&self) -> std::io::Result<Box<dyn iroh::endpoint::transports::CustomEndpoint>> {
        let endpoint = self.inner.bind()?;
        *self.shutdown.lock().unwrap() = endpoint.shutdown_handler();
        Ok(endpoint)
    }
}

#[tokio::test]
async fn control_paths_do_not_report_stale_same_address_as_recovered() {
    use miopunch_native_probe::{
        connection_recovery::{CONNECTION_RECOVERY_ALPN, ConnectionPathRecoveryError},
        mqtt_transport::TRANSPORT_ID,
        resources::{Resource, ResourceUse},
        tcp_transport::{TcpTransport, Traffic},
    };
    let broker = std::env::var("MIOPUNCH_MQTT_BROKER").expect("owned MQTT broker required");
    let mut pair = Pair::new(false).await;
    pair.reconfigure(vec![Resource {
        url: format!("mqtt://{broker}"),
        uses: std::collections::BTreeSet::from([ResourceUse::Signaling]),
        limitation: "owned broker for unchanged-address outage".into(),
    }])
    .await;
    let mut config = pair.admin.resource_configuration().unwrap();
    config.tcp_listen = None;
    pair.admin.save_resource_configuration(&config).unwrap();
    let carrier = std::sync::Arc::new(CuttableTcp {
        inner: TcpTransport::new(Ipv4Addr::LOCALHOST, std::sync::Arc::new(Traffic::default())),
        shutdown: std::sync::Mutex::new(None),
    });
    let new_server = pair
        .admin
        .bind_configured_endpoint_with(
            iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
                .clear_ip_transports()
                .bind_addr((Ipv4Addr::LOCALHOST, 0))
                .unwrap()
                .add_custom_transport(carrier.clone()),
        )
        .await
        .unwrap();
    std::mem::replace(&mut pair.server, new_server)
        .close()
        .await;
    pair.server
        .endpoint()
        .set_alpns(vec![BUSINESS.to_vec(), CONNECTION_RECOVERY_ALPN.to_vec()]);
    let route = pair
        .server
        .endpoint()
        .custom_addrs()
        .into_iter()
        .find(|a| a.id() == TRANSPORT_ID)
        .unwrap();
    let contact =
        EndpointAddr::new(pair.server.endpoint().id()).with_addrs([TransportAddr::Custom(route)]);
    let (client, server) = pair.business().await;
    let old = client.stable_id();
    let original_address = client.paths().iter().next().unwrap().remote_addr().clone();
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"before outage").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut before = [0; 13];
    receive.read_exact(&mut before).await.unwrap();
    // First confirm that a living unchanged path really can be acknowledged.
    let serve = async {
        let control = pair
            .server
            .endpoint()
            .accept()
            .await
            .unwrap()
            .await
            .unwrap();
        pair.admin
            .serve_connection_path_recovery(&pair.server, &control)
            .await
    };
    let (result, served) = tokio::join!(
        pair.joining
            .recover_connection_path(&pair.client, &client, contact.clone()),
        serve
    );
    assert_eq!(result.unwrap().path, original_address);
    served.unwrap();
    chunk(&mut send, &mut receive, b"fresh unchanged path").await;
    let shutdown = carrier.shutdown.lock().unwrap().clone().unwrap();
    shutdown.close().await;
    tokio::time::timeout(Duration::from_secs(1), async {
        while tokio::net::TcpStream::connect(carrier.inner.bound_addr().unwrap())
            .await
            .is_ok()
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("real listener stopped");
    assert!(client.close_reason().is_none());
    assert!(
        client
            .paths()
            .iter()
            .any(|p| p.remote_addr() == &original_address)
    );
    assert!(
        pair.server.addr().addrs.contains(&original_address),
        "unchanged peer offer"
    );
    let serve = async {
        let control = pair
            .server
            .endpoint()
            .accept()
            .await
            .unwrap()
            .await
            .unwrap();
        assert!(
            matches!(control.paths().iter().next().unwrap().remote_addr(), TransportAddr::Custom(a) if a.id() == TRANSPORT_ID)
        );
        pair.admin
            .serve_connection_path_recovery(&pair.server, &control)
            .await
    };
    let (result, served) = tokio::join!(
        pair.joining
            .recover_connection_path(&pair.client, &client, contact.clone()),
        serve
    );
    eprintln!("MIO47_SAME_ADDRESS result={result:?} served={served:?} control_contacted=true");
    assert!(
        matches!(result, Err(ConnectionPathRecoveryError::Retryable { stable_id, .. }) if stable_id == old),
        "stale listed path must not prove delivery"
    );
    assert!(client.close_reason().is_none());
    send.write_all(b"queued during outage").await.unwrap();
    let reason = tokio::time::timeout(Duration::from_secs(16), client.closed())
        .await
        .expect("actual original lifetime expires");
    let result = pair
        .joining
        .recover_connection_path(&pair.client, &client, contact)
        .await;
    assert!(
        matches!(result, Err(ConnectionPathRecoveryError::Terminal { stable_id }) if stable_id == old)
    );
    assert!(receive.read_exact(&mut [0; 1]).await.is_err());
    eprintln!(
        "MIO47_SAME_ADDRESS_FINAL original={old} close={reason:?} retryable_then_terminal=true false_success=false"
    );
    pair.close().await;
}
