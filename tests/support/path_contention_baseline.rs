//! Ordinary native nodes, using the same relay/STUN and business entry as the
//! legacy enhanced UDP experiment. Keep the operation phases in the witness
//! when switching the driver to the fused endpoint.
#[path = "delayed_relay.rs"]
pub(super) mod delayed_relay;

use super::{ObserveBusiness, join};
use iroh::endpoint::{BeforeConnectOutcome, EndpointHooks};
use iroh::{EndpointAddr, TransportAddr};
use miopunch_native_probe::{
    connection_coordinator::{ACTIVE_IMPROVEMENT_ALPN, MEMBER_ALPN},
    native_network::NativeNetwork,
    network_maintenance::MaintainedNetwork,
    resources::{Resource, ResourceUse},
};
use serde_json::json;
use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed},
    },
    time::Duration,
};
use tokio::{net::UdpSocket, sync::Barrier, task::JoinSet, time::Instant};
use tokio_util::sync::CancellationToken;

pub(super) const FLOOR: Duration = Duration::from_millis(250);
const COOLDOWN: Duration = FLOOR.saturating_mul(32);
pub(super) const DEADLINE: Duration = Duration::from_secs(3);

tokio::task_local! {
    static EXPLICIT_REPAIR: bool;
}

#[derive(Debug, Clone)]
struct ControlStart {
    outgoing: Arc<Barrier>,
    automatic: bool,
    first_automatic: Arc<AtomicBool>,
}

impl EndpointHooks for ControlStart {
    async fn before_connect(&self, _: &EndpointAddr, alpn: &[u8]) -> BeforeConnectOutcome {
        if alpn != ACTIVE_IMPROVEMENT_ALPN {
            return BeforeConnectOutcome::Accept;
        }
        match EXPLICIT_REPAIR.try_with(|crossed| *crossed) {
            Ok(true) => self.outgoing.wait().await,
            Ok(false) => return BeforeConnectOutcome::Accept,
            Err(_) if self.automatic && !self.first_automatic.swap(true, Relaxed) => {
                self.outgoing.wait().await
            }
            Err(_) => return BeforeConnectOutcome::Reject,
        };
        BeforeConnectOutcome::Accept
    }
}

pub(super) async fn observe_stun(
    observers: &mut JoinSet<()>,
    requests: &Arc<AtomicUsize>,
) -> Vec<Resource> {
    let mut allowed = Vec::new();
    for _ in 0..2 {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        allowed.push(Resource {
            url: format!("stun://{}/", socket.local_addr().unwrap()),
            uses: [ResourceUse::Observation].into(),
            limitation: "isolated contention fixture".into(),
        });
        let requests = requests.clone();
        observers.spawn(async move {
            loop {
                let mut request = [0u8; 1500];
                let (length, source) = socket.recv_from(&mut request).await.unwrap();
                assert_eq!(length, 20);
                assert_eq!(&request[..8], &[0, 1, 0, 0, 0x21, 0x12, 0xa4, 0x42]);
                let SocketAddr::V4(source) = source else {
                    panic!("IPv4 STUN source")
                };
                let mut reply = [0; 32];
                reply[..8].copy_from_slice(&[1, 1, 0, 12, 0x21, 0x12, 0xa4, 0x42]);
                reply[8..20].copy_from_slice(&request[8..20]);
                reply[20..26].copy_from_slice(&[0, 0x20, 0, 8, 0, 1]);
                reply[26..28].copy_from_slice(&(source.port() ^ 0x2112).to_be_bytes());
                reply[28..32]
                    .copy_from_slice(&(u32::from(*source.ip()) ^ 0x2112a442).to_be_bytes());
                requests.fetch_add(1, Relaxed);
                socket.send_to(&reply, source).await.unwrap();
            }
        });
    }
    allowed
}

async fn stream(
    dialed: &iroh::endpoint::Connection,
    accepted: &iroh::endpoint::Connection,
    stop: CancellationToken,
    pulses: Arc<AtomicUsize>,
) {
    let (mut from_dialer, mut to_dialer) = dialed.open_bi().await.unwrap();
    from_dialer.write_all(b"mio156").await.unwrap();
    let (mut from_acceptor, mut to_acceptor) = accepted.accept_bi().await.unwrap();
    let mut header = [0; 6];
    to_acceptor.read_exact(&mut header).await.unwrap();
    assert_eq!(&header, b"mio156");
    let mut tick = tokio::time::interval(Duration::from_millis(75));
    for sequence in 1u64.. {
        tokio::select! {
            _ = stop.cancelled() => break,
            _ = tick.tick() => {}
        }
        let left = sequence.to_be_bytes();
        let right = (!sequence).to_be_bytes();
        let mut received_left = [0; 8];
        let mut received_right = [0; 8];
        let (send_left, send_right, read_left, read_right) = tokio::join!(
            from_dialer.write_all(&left),
            from_acceptor.write_all(&right),
            to_dialer.read_exact(&mut received_right),
            to_acceptor.read_exact(&mut received_left),
        );
        send_left.unwrap();
        send_right.unwrap();
        read_left.unwrap();
        read_right.unwrap();
        assert_eq!((received_left, received_right), (left, right));
        pulses.fetch_add(1, Relaxed);
    }
}

async fn relay_only(
    owner: &MaintainedNetwork,
    connection: &iroh::endpoint::Connection,
    relay: &TransportAddr,
) {
    owner
        .endpoint()
        .control_handle(connection)
        .test_limit_transport_paths(vec![relay.clone()])
        .await
        .unwrap();
    for path in connection.paths().iter().filter(|path| !path.is_relay()) {
        path.close().unwrap();
    }
    assert!(
        connection
            .paths()
            .iter()
            .any(|path| path.is_relay() && path.is_selected())
    );
}

async fn repair(
    node: &MaintainedNetwork,
    original: &iroh::endpoint::Connection,
    target: &EndpointAddr,
    crossed: bool,
) -> String {
    let started = n0_future::time::Instant::now();
    let result = EXPLICIT_REPAIR
        .scope(
            crossed,
            node.repair_member_connection_until(original, target.clone(), started + DEADLINE),
        )
        .await;
    assert!(
        original.close_reason().is_none(),
        "repair closed the healthy business connection"
    );
    assert!(started.elapsed() <= DEADLINE + Duration::from_millis(100));
    format!("{result:?}")
}

#[tokio::test]
#[ignore = "run in its own process with the enhanced-only lab input"]
async fn same_original_hello_hands_off_live_business() {
    run(false).await;
}

#[tokio::test]
#[ignore = "run in its own process with the enhanced-only lab input"]
async fn independent_originals_contend_for_node_rounds() {
    run(true).await;
}

async fn run(independent: bool) {
    assert_eq!(
        std::env::var("MIOPUNCH_NAT_LAB_ENHANCED_ONLY").as_deref(),
        Ok("1")
    );
    tokio::time::timeout(Duration::from_secs(30), scenario(independent))
        .await
        .expect("the real relay, business, overlap and retry windows are bounded");
}

async fn scenario(independent: bool) {
    let directory = tempfile::tempdir().unwrap();
    let mut left = NativeNetwork::create(directory.path().join("left"))
        .await
        .unwrap();
    let right = join(&mut left, &directory.path().join("right")).await;
    let mut relay_config = iroh_relay::server::ServerConfig::default();
    relay_config.relay = Some(iroh_relay::server::RelayConfig::new((
        Ipv4Addr::LOCALHOST,
        0,
    )));
    let relay = iroh_relay::server::Server::spawn(relay_config)
        .await
        .unwrap();
    let (delayed, relay_stop) = delayed_relay::delay_relay(
        relay
            .http_url()
            .unwrap()
            .as_str()
            .parse::<url::Url>()
            .unwrap()
            .socket_addrs(|| None)
            .unwrap()[0],
        Duration::from_millis(35),
    )
    .await;
    let relay_path = TransportAddr::Relay(delayed.clone());
    let requests: [Arc<AtomicUsize>; 2] = std::array::from_fn(|_| Arc::default());
    let mut observers = JoinSet::new();
    let outgoing = Arc::new(Barrier::new(2));
    let mut nodes = Vec::new();
    let mut connectors = Vec::new();
    let mut incoming = Vec::new();
    for (node, requests) in [left, right].into_iter().zip(&requests) {
        let mut config = node.resource_configuration().unwrap();
        let mut resources = observe_stun(&mut observers, requests).await;
        resources.push(Resource {
            url: delayed.to_string(),
            uses: [ResourceUse::DataRelay].into(),
            limitation: "isolated business relay".into(),
        });
        config.replacement = Some(resources);
        config.tcp_direct_disabled = true;
        config.member_data_relay_disabled = true;
        config.maintenance.exploration_interval_ms = FLOOR.as_millis() as u64;
        node.save_resource_configuration(&config).unwrap();
        let connector = node.udp_connector().clone();
        connector.set_udp_enabled(false);
        connectors.push(connector);
        let (send, receive) = tokio::sync::mpsc::channel(2);
        incoming.push(receive);
        let hook = ControlStart {
            outgoing: outgoing.clone(),
            automatic: !independent,
            first_automatic: Arc::default(),
        };
        nodes.push(
            node.maintain_network_with_endpoint(
                Ipv4Addr::LOCALHOST,
                |builder| builder.hooks(hook),
                |router| router.accept(MEMBER_ALPN, ObserveBusiness(send)),
            )
            .await
            .unwrap(),
        );
    }
    let [left, right] = nodes.try_into().ok().unwrap();
    let [mut to_left, mut to_right] = incoming.try_into().ok().unwrap();
    let _business = [left.business_activity(), right.business_activity()];
    tokio::join!(left.endpoint().online(), right.endpoint().online());
    let to_right_target = EndpointAddr::new(right.endpoint().id()).with_addrs([relay_path.clone()]);
    let to_left_target = EndpointAddr::new(left.endpoint().id()).with_addrs([relay_path.clone()]);
    let (left_first, _) = left
        .connect(to_right_target.clone(), MEMBER_ALPN)
        .await
        .unwrap();
    let right_first = to_right.recv().await.unwrap();
    assert_eq!(left_first.remote_id(), right.endpoint().id());
    assert_eq!(right_first.remote_id(), left.endpoint().id());
    relay_only(&left, &left_first, &relay_path).await;
    relay_only(&right, &right_first, &relay_path).await;
    let mut pairs = vec![(left_first.clone(), right_first)];
    let right_second = if independent {
        let (dialed, _) = right
            .connect(to_left_target.clone(), MEMBER_ALPN)
            .await
            .unwrap();
        let accepted = to_left.recv().await.unwrap();
        assert_eq!(dialed.remote_id(), left.endpoint().id());
        assert_eq!(accepted.remote_id(), right.endpoint().id());
        relay_only(&right, &dialed, &relay_path).await;
        relay_only(&left, &accepted, &relay_path).await;
        pairs.push((accepted, dialed.clone()));
        Some(dialed)
    } else {
        None
    };

    let stop = CancellationToken::new();
    let pulses: Vec<_> = pairs
        .iter()
        .map(|_| Arc::<AtomicUsize>::default())
        .collect();
    let mut streams = JoinSet::new();
    for (index, ((left, right), pulses)) in pairs.iter().zip(&pulses).enumerate() {
        let (dialed, accepted) = if index == 0 {
            (left, right)
        } else {
            (right, left)
        };
        let (dialed, accepted, stop, pulses) = (
            dialed.clone(),
            accepted.clone(),
            stop.clone(),
            pulses.clone(),
        );
        streams.spawn(async move { stream(&dialed, &accepted, stop, pulses).await });
    }
    while pulses.iter().any(|counter| counter.load(Relaxed) == 0) {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let marker = |phase: &str, outcomes: &[String]| {
        eprintln!(
            "{}",
            json!({
                "event":"contention-fixture","phase":phase,
                "originals":pairs.iter().map(|(l, r)| [l.stable_id(), r.stable_id()]).collect::<Vec<_>>(),
                "stunRequests":requests.each_ref().map(|counter| counter.load(Relaxed)),
                "businessSequences":pulses.iter().map(|counter| counter.load(Relaxed)).collect::<Vec<_>>(),
                "outcomes":outcomes,
            })
        );
    };
    marker("crossed-start", &[]);
    for connector in &connectors {
        connector.set_udp_enabled(true);
    }
    if let Some(right_second) = right_second {
        let (left_result, right_result) = tokio::join!(
            repair(&left, &left_first, &to_right_target, true),
            repair(&right, &right_second, &to_left_target, true),
        );
        marker("crossed-end", &[left_result, right_result]);
        let next = repair(&left, &left_first, &to_right_target, false).await;
        marker("early-retry-end", &[next]);
        tokio::time::sleep(COOLDOWN).await;
        let probe = repair(&left, &left_first, &to_right_target, false).await;
        marker("probe-end", &[probe]);
        let next = repair(&left, &left_first, &to_right_target, false).await;
        marker("probe-retry-end", &[next]);
    } else {
        tokio::time::sleep(DEADLINE + FLOOR).await;
        marker("crossed-end", &[]);
    }
    for (l, r) in &pairs {
        assert!(l.close_reason().is_none() && r.close_reason().is_none());
    }
    stop.cancel();
    while let Some(result) = streams.join_next().await {
        result.unwrap();
    }
    tokio::join!(left.close(), right.close());
    relay_stop.cancel();
    relay.shutdown().await.unwrap();
    observers.shutdown().await;
}
