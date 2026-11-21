//! Public node lifecycle, real admission and durable LogSync; no mocked peers.
#[path = "support/native_connections.rs"]
mod native_connections;
#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[path = "support/path_contention_baseline.rs"]
mod path_contention_baseline;
#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[path = "support/member_fusion_arbitration.rs"]
mod member_fusion_arbitration;
#[cfg(target_os = "linux")]
#[path = "support/tcp_binding_observer.rs"]
mod tcp_binding_observer;
use iroh::endpoint::RestrictedEndpoint;

use iroh::endpoint::Connection;
use miopunch_native_probe::{
    admission_gate::ADMISSION_REQUEST_ALPN,
    manual_admission::ManualExchange,
    native_network::NativeNetwork,
    network_recovery::RECOVERY_REPLICA_ALPN,
    resources::{Resource, ResourceUse},
};
use std::{collections::BTreeSet, net::Ipv4Addr, sync::Arc, time::Duration};

#[cfg(target_os = "linux")]
#[tokio::test]
async fn observed_tcp_moves_the_application_members_original_stream_after_fresh_ack() {
    observed_tcp_on_application_members(TcpRound::Commit).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn cancelling_application_tcp_traversal_retires_the_candidate_and_preserves_the_relay() {
    observed_tcp_on_application_members(TcpRound::Cancel).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn original_application_members_retry_a_failed_tcp_observation_after_network_change() {
    observed_tcp_on_application_members(TcpRound::RetryAfterNetworkChange).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn active_original_application_members_retry_tcp_after_same_generation_cooldown() {
    observed_tcp_on_application_members(TcpRound::RetryAfterCooldown).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn active_original_application_members_requalify_tcp_after_path_retirement() {
    observed_tcp_on_application_members(TcpRound::Requalify).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn application_standby_tcp_retry_is_independent_of_sync() {
    for interval in [1_000, 30_000] {
        observed_tcp_on_application_members(TcpRound::StandbyRetry(interval)).await;
    }
}

#[cfg(target_os = "linux")]
enum TcpRound {
    Commit,
    Cancel,
    RetryAfterNetworkChange,
    RetryAfterCooldown,
    StandbyRetry(u64),
    Requalify,
}

#[cfg(target_os = "linux")]
async fn observed_tcp_on_application_members(scenario: TcpRound) {
    use iroh::{
        EndpointAddr, TransportAddr,
        endpoint::{BeforeConnectOutcome, EndpointHooks, MemberConnection, MemberStream},
    };
    use miopunch_native_probe::{
        connection_coordinator::ACTIVE_IMPROVEMENT_ALPN,
        native_network::{FUSED_MEMBER_ALPN, member_fusion_attempt_id},
        path_selection::PathPreference,
        tcp_transport::TcpTransport,
    };
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
    use tokio::time::{Instant, timeout};

    #[derive(Clone, Debug, Default)]
    struct Dials(Arc<AtomicUsize>);
    impl EndpointHooks for Dials {
        async fn before_connect(&self, _: &EndpointAddr, alpn: &[u8]) -> BeforeConnectOutcome {
            if matches!(alpn, FUSED_MEMBER_ALPN | ACTIVE_IMPROVEMENT_ALPN) {
                self.0.fetch_add(1, Relaxed);
            }
            BeforeConnectOutcome::Accept
        }
    }
    async fn echo(a: &mut MemberStream, b: &mut MemberStream, sequence: u64) {
        let bytes = sequence.to_be_bytes();
        let (send, recv) = a.streams();
        let (send_back, recv_at_peer) = b.streams();
        let mut read = [0; 8];
        send.write_all(&bytes).await.unwrap();
        recv_at_peer.read_exact(&mut read).await.unwrap();
        assert_eq!(read, bytes);
        send_back.write_all(&read).await.unwrap();
        recv.read_exact(&mut read).await.unwrap();
        assert_eq!(read, bytes);
    }

    let dir = tempfile::tempdir().unwrap();
    let mut provider = NativeNetwork::create(dir.path().join("provider"))
        .await
        .unwrap();
    let consumer = join(&mut provider, &dir.path().join("consumer")).await;
    let mut relay_config = iroh_relay::server::ServerConfig::default();
    relay_config.relay = Some(iroh_relay::server::RelayConfig::new((
        Ipv4Addr::LOCALHOST,
        0,
    )));
    let relay = iroh_relay::server::Server::spawn(relay_config)
        .await
        .unwrap();
    let relay_url = relay.http_url().unwrap();
    let cooldown_retry = matches!(
        scenario,
        TcpRound::RetryAfterCooldown | TcpRound::StandbyRetry(_)
    );
    let requalify = matches!(scenario, TcpRound::Requalify);
    let retry = cooldown_retry || matches!(scenario, TcpRound::RetryAfterNetworkChange);
    let retry_interval = Duration::from_millis(600);
    let (observer, listener, mut sources) =
        tcp_binding_observer::tcp_binding_observer_with_sources(None, usize::from(retry) * 2).await;
    for node in [&consumer, &provider] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(vec![
            Resource {
                url: relay_url.to_string(),
                uses: [ResourceUse::DataRelay].into(),
                limitation: "owned healthy relay".into(),
            },
            Resource {
                url: format!("stun+tcp://{observer}/"),
                uses: [ResourceUse::Observation].into(),
                limitation: "owned TCP observer of the retained port".into(),
            },
        ]);
        config.udp_direct_disabled = true;
        config.tcp_listen = Some((Ipv4Addr::UNSPECIFIED, 0).into());
        config.path_selection.preference = PathPreference::PreferDirect;
        config.maintenance.quality_interval_ms = 250;
        if cooldown_retry || requalify {
            config.maintenance.exploration_interval_ms = match scenario {
                TcpRound::StandbyRetry(interval) => interval,
                _ => 30_000,
            };
            config.maintenance.path_retry_base_ms = retry_interval.as_millis() as u64;
        }
        if matches!(scenario, TcpRound::Cancel) {
            config.maintenance.max_standby_member_connections = 0;
        }
        config.maintenance.contact_timeout_ms = 8_000;
        node.save_resource_configuration(&config).unwrap();
    }
    let dials = [Dials::default(), Dials::default()];
    let consumer = consumer
        .maintain_application_network(Ipv4Addr::LOCALHOST, |b| b.hooks(dials[0].clone()))
        .await
        .unwrap();
    let provider = provider
        .maintain_application_network(Ipv4Addr::LOCALHOST, |b| b.hooks(dials[1].clone()))
        .await
        .unwrap();
    let bound = [&consumer, &provider].map(|node| {
        node.addr()
            .addrs
            .iter()
            .find_map(|addr| match addr {
                TransportAddr::Custom(addr) => TcpTransport::socket_addr(addr).ok(),
                _ => None,
            })
            .unwrap()
    });
    assert!(
        bound.iter().all(|addr| addr.ip().is_unspecified()),
        "{bound:?}"
    );
    let observed = bound.map(|addr| std::net::SocketAddr::from((Ipv4Addr::LOCALHOST, addr.port())));
    let relay_path = TransportAddr::Relay(relay_url.clone());
    let (out, entrance) = tokio::join!(
        consumer
            .connect_member(EndpointAddr::new(provider.endpoint().id()).with_relay_url(relay_url)),
        provider.accept_member()
    );
    let originals = [out.unwrap(), entrance.unwrap().unwrap()];
    let ids = originals.each_ref().map(MemberConnection::stable_id);
    assert_eq!(
        member_fusion_attempt_id(&originals[0]).unwrap(),
        member_fusion_attempt_id(&originals[1]).unwrap()
    );
    assert!(
        originals
            .iter()
            .all(|m| m.initial_data_path() == &relay_path)
    );
    if let TcpRound::StandbyRetry(_) = scenario {
        timeout(Duration::from_secs(6), async {
            for _ in 0..2 { assert!(observed.contains(&sources.recv().await.unwrap())); }
            let failed = Instant::now();
            for _ in 0..2 { assert!(observed.contains(&sources.recv().await.unwrap())); }
            assert!(failed.elapsed() >= retry_interval * 2 - Duration::from_millis(80));
            while !originals.iter().all(|conn| conn.paths().iter().any(|p|
                p.is_selected() && matches!(p.remote_addr(), TransportAddr::Custom(addr) if addr.id() == miopunch_native_probe::tcp_transport::DATA_TRANSPORT_ID) && p.stats().acknowledgment.is_some())) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let (a,b) = tokio::join!(originals[0].open_bi(), originals[1].accept_bi());
            echo(&mut a.unwrap(), &mut b.unwrap(), 158).await;
        }).await.expect("standby TCP retries without stream traffic or ping");
        tokio::join!(consumer.close(), provider.close());
        listener.abort();
        relay.shutdown().await.unwrap();
        return;
    }
    let suppressed = Arc::new(AtomicBool::new(true));
    let path_id = noq_proto::PathId::from(1u8);
    let generations =
        [consumer.endpoint(), provider.endpoint()].map(|e| e.udp_network_generation());
    for owner in &originals {
        owner.test_suppress_path_acks(path_id, suppressed.clone());
    }
    let started = Instant::now();
    // The accepting side's real business can trigger the client-controlled TCP round.
    let mut a = originals[1].open_bi().await.unwrap();
    a.streams().0.write_all(b"open original").await.unwrap();
    let mut b = originals[0].accept_bi().await.unwrap();
    let stream = u64::from(a.streams().0.id());
    assert_eq!(u64::from(b.streams().0.id()), stream);
    let mut open = [0; 13];
    b.streams().1.read_exact(&mut open).await.unwrap();
    assert_eq!(&open, b"open original");
    echo(&mut a, &mut b, 1).await;
    let mut cooldown_source = None;
    if retry {
        let first = timeout(Duration::from_secs(4), async {
            [sources.recv().await.unwrap(), sources.recv().await.unwrap()]
        })
        .await
        .expect("the original member connection must perform both initial TCP observations");
        assert_ne!(first[0], first[1]);
        assert!(first.into_iter().all(|source| observed.contains(&source)));
        assert!(
            originals
                .iter()
                .all(|member| member.paths().get(path_id).is_none())
        );
        echo(&mut a, &mut b, 10).await;
        if cooldown_retry {
            let first_round = Instant::now();
            let (source, request_count) = timeout(Duration::from_secs(4), async {
                let mut business = tokio::time::interval(Duration::from_millis(25));
                let mut sequence = 11;
                loop {
                    tokio::select! {
                        biased;
                        source = sources.recv() => break (source.unwrap(), sequence - 11),
                        _ = business.tick() => {
                            echo(&mut a, &mut b, sequence).await;
                            sequence += 1;
                        }
                    }
                }
            })
            .await
            .expect(
                "continuous business must retry an observed TCP failure in the same generation",
            );
            let elapsed = first_round.elapsed();
            assert!(
                elapsed >= retry_interval * 2 - Duration::from_millis(50),
                "{elapsed:?}"
            );
            assert!(request_count >= 2);
            assert!(generations.iter().all(|generation| generation.is_current()));
            cooldown_source = Some(source);
            eprintln!("MIO155 TCP cooldown={elapsed:?} sameGenerationRequests={request_count}");
        } else {
            for endpoint in [provider.endpoint(), consumer.endpoint()] {
                let previous = endpoint.udp_network_generation();
                endpoint.network_change().await;
                timeout(Duration::from_secs(1), async {
                    while previous.is_current() {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .expect("the socket actor must process the network notification");
            }
        }
    }
    let peer_routes = observed.map(TcpTransport::address);
    timeout(Duration::from_secs(5), async {
        let first = match cooldown_source {
            Some(source) => source,
            None => sources.recv().await.unwrap(),
        };
        let mut actual = [first, sources.recv().await.unwrap()];
        actual.sort();
        let mut expected = observed;
        expected.sort();
        assert_eq!(actual, expected, "observations use actual listener ports");
        while !originals.iter().enumerate().all(|(side, m)| {
            m.paths().get(path_id).is_some_and(|p| {
                p.remote_addr() == &TransportAddr::Custom(peer_routes[1 - side].clone())
            })
        }) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("configured enhanced TCP must observe and prime the original QUIC connection");
    for m in &originals {
        assert!(
            m.paths()
                .iter()
                .any(|p| p.is_selected() && p.remote_addr() == &relay_path)
        );
        assert!(
            m.paths()
                .get(path_id)
                .unwrap()
                .stats()
                .acknowledgment
                .is_none()
        );
    }
    echo(&mut a, &mut b, 2).await;
    if matches!(scenario, TcpRound::Cancel) {
        let mut maintenance = originals[0].open_maintenance_bi().await.unwrap();
        maintenance
            .streams()
            .0
            .write_all(b"same original")
            .await
            .unwrap();
        let mut peer = originals[1].accept_bi().await.unwrap();
        peer.streams().1.read_exact(&mut open).await.unwrap();
        assert_eq!(&open, b"same original");
        assert!(!maintenance.is_business() && !peer.is_business());
        drop((a, b));
        timeout(Duration::from_secs(3), async {
            while originals.iter().any(|m| m.paths().get(path_id).is_some()) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("last business release drops both uncommitted TCP candidates");
        for m in &originals {
            assert!(
                m.paths()
                    .iter()
                    .any(|p| p.is_selected() && p.remote_addr() == &relay_path)
            );
        }
        echo(&mut maintenance, &mut peer, 3).await;
        eprintln!("MIO155 TCP cancelled originals={ids:?} relayMaintenanceReply=3");
    } else {
        suppressed.store(false, Relaxed);
        timeout(Duration::from_secs(6), async {
            while !originals.iter().all(|m| {
                m.paths().get(path_id).is_some_and(|p| {
                    p.is_selected()
                        && p.stats()
                            .acknowledgment
                            .is_some_and(|ack| ack.sent_at >= started.into_std())
                })
            }) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("only the exact fresh-ACK TCP path can take the original stream");
        let received = [
            originals[1]
                .test_observe_stream_receive(path_id, &a)
                .unwrap(),
            originals[0]
                .test_observe_stream_receive(path_id, &b)
                .unwrap(),
        ];
        echo(&mut a, &mut b, 3).await;
        assert!(received.into_iter().all(|received| received.confirm()));
        for (side, original) in originals.iter().enumerate() {
            let paths = original.paths();
            let path = paths.get(path_id).unwrap();
            eprintln!(
                "MIO155 TCP side={side} stable={} stream={stream} binding={} observed={} path={path_id} ack_ms={}",
                ids[side],
                bound[side],
                observed[side],
                path.stats()
                    .acknowledgment
                    .unwrap()
                    .received_at
                    .duration_since(started.into_std())
                    .as_millis()
            );
        }
        if requalify {
            let replacement = path_id.saturating_add(1u8);
            suppressed.store(true, Relaxed);
            for owner in &originals {
                owner.test_suppress_path_acks(replacement, suppressed.clone());
            }
            let retired = Instant::now();
            originals[0].paths().get(path_id).unwrap().close().unwrap();
            timeout(Duration::from_secs(3), async {
                while !originals.iter().all(|m| {
                    let paths = m.paths();
                    paths.get(path_id).is_none()
                        && paths
                            .iter()
                            .any(|p| p.is_selected() && p.remote_addr() == &relay_path)
                }) {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("retiring the selected TCP path must return both originals to their relay");
            let relay = noq_proto::PathId::ZERO;
            let relay_bytes = [
                originals[1].test_observe_stream_receive(relay, &a).unwrap(),
                originals[0].test_observe_stream_receive(relay, &b).unwrap(),
            ];
            echo(&mut a, &mut b, 4).await;
            assert!(relay_bytes.into_iter().all(|received| received.confirm()));

            timeout(Duration::from_secs(5), async {
                let mut actual = [sources.recv().await.unwrap(), sources.recv().await.unwrap()];
                actual.sort();
                let mut expected = observed;
                expected.sort();
                assert_eq!(actual, expected, "observe both real listeners again");
                suppressed.store(false, Relaxed);
                while !originals.iter().enumerate().all(|(side, m)| {
                    m.paths().get(replacement).is_some_and(|p| {
                        p.is_selected()
                            && p.remote_addr()
                                == &TransportAddr::Custom(peer_routes[1 - side].clone())
                            && p.stats()
                                .acknowledgment
                                .is_some_and(|ack| ack.sent_at >= retired.into_std())
                    })
                }) {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect(
                "the original members must observe and requalify a new TCP path after retirement",
            );
            assert!(generations.iter().all(|generation| generation.is_current()));
            let tcp_bytes = [
                originals[1]
                    .test_observe_stream_receive(replacement, &a)
                    .unwrap(),
                originals[0]
                    .test_observe_stream_receive(replacement, &b)
                    .unwrap(),
            ];
            echo(&mut a, &mut b, 5).await;
            assert!(tcp_bytes.into_iter().all(|received| received.confirm()));
            eprintln!(
                "MIO155 TCP retired={path_id} relaySequence=4 replacement={replacement} tcpSequence=5 ids={ids:?} stream={stream} elapsed={:?}",
                retired.elapsed()
            );
        }
        drop((a, b));
    }
    assert_eq!(originals.each_ref().map(MemberConnection::stable_id), ids);
    assert_eq!(dials.each_ref().map(|d| d.0.load(Relaxed)), [1, 0]);
    assert!(sources.try_recv().is_err());
    listener.abort();
    tokio::join!(consumer.close(), provider.close());
    relay.shutdown().await.unwrap();
}

#[tokio::test]
async fn member_business_reaches_relay_missing_from_the_saved_contact() {
    use iroh::{
        EndpointAddr, TransportAddr,
        protocol::{AcceptError, ProtocolHandler},
    };
    use miopunch_native_probe::connection_coordinator::MEMBER_ALPN;

    #[derive(Debug)]
    struct Echo;
    impl ProtocolHandler for Echo {
        async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
            let (mut send, mut recv) = connection.accept_bi().await?;
            let request = recv.read_to_end(8).await.map_err(AcceptError::from_err)?;
            send.write_all(&request)
                .await
                .map_err(AcceptError::from_err)?;
            send.finish()?;
            send.stopped().await.map_err(AcceptError::from_err)?;
            Ok(())
        }
    }

    let directory = tempfile::tempdir().unwrap();
    let mut server = NativeNetwork::create(directory.path().join("server"))
        .await
        .unwrap();
    let client = join(&mut server, &directory.path().join("client")).await;
    let mut config = iroh_relay::server::ServerConfig::default();
    config.relay = Some(iroh_relay::server::RelayConfig::new((
        Ipv4Addr::LOCALHOST,
        0,
    )));
    let relay = iroh_relay::server::Server::spawn(config).await.unwrap();
    let healthy = relay.http_url().unwrap();
    // Keep the listener open and leave its WebSocket handshakes unanswered.
    // The first candidate therefore consumes the old serial dial's entire budget.
    let blackhole = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let stale: iroh::RelayUrl = format!("http://{}/", blackhole.local_addr().unwrap())
        .parse()
        .unwrap();
    for (node, relays) in [(&server, vec![&healthy]), (&client, vec![&stale, &healthy])] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(
            relays
                .into_iter()
                .map(|url| Resource {
                    url: url.to_string(),
                    uses: BTreeSet::from([ResourceUse::DataRelay]),
                    limitation: "member relay redial fixture".into(),
                })
                .collect(),
        );
        config.udp_direct_disabled = true;
        config.tcp_direct_disabled = true;
        config.member_data_relay_disabled = true;
        config.maintenance.contact_timeout_ms = 3000;
        node.save_resource_configuration(&config).unwrap();
    }
    let server = server
        .maintain_network_with(Ipv4Addr::LOCALHOST, |router| {
            router.accept(MEMBER_ALPN, Echo)
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), server.endpoint().online())
        .await
        .unwrap();
    let client = client.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    let peer = server.endpoint().id();
    let target = EndpointAddr::new(peer).with_addrs([TransportAddr::Relay(stale)]);
    let (connection, _activity) = client
        .connect(target, MEMBER_ALPN)
        .await
        .expect("a stale relay must leave time to reach the configured relay serving the peer");
    assert_eq!(connection.remote_id(), peer);
    assert!(
        connection
            .paths()
            .iter()
            .any(|path| path.remote_addr() == &TransportAddr::Relay(healthy.clone()))
    );
    tokio::time::timeout(Duration::from_secs(3), async {
        let (mut send, mut recv) = connection.open_bi().await.unwrap();
        send.write_all(&42u64.to_be_bytes()).await.unwrap();
        send.finish().unwrap();
        assert_eq!(recv.read_to_end(8).await.unwrap(), 42u64.to_be_bytes());
    })
    .await
    .unwrap();
    client.close().await;
    server.close().await;
}

async fn observe_queries(
    broker: &str,
) -> (
    Arc<std::sync::Mutex<Vec<std::time::Instant>>>,
    tokio::task::JoinHandle<()>,
) {
    use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
    let (host, port) = broker.rsplit_once(':').unwrap();
    let options = MqttOptions::new(
        format!("mio49-observer-{}", rand::random::<u64>()),
        host,
        port.parse().unwrap(),
    );
    let (client, mut events) = AsyncClient::new(options, 32);
    client
        .subscribe("miopunch/#", QoS::AtMostOnce)
        .await
        .unwrap();
    loop {
        if matches!(
            events.poll().await.unwrap(),
            Event::Incoming(Packet::SubAck(_))
        ) {
            break;
        }
    }
    let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let records = observed.clone();
    let task = tokio::spawn(async move {
        while let Ok(event) = events.poll().await {
            if let Event::Incoming(Packet::Publish(message)) = event {
                if message.payload.starts_with(b"MDQ1") {
                    records.lock().unwrap().push(std::time::Instant::now());
                }
            }
        }
    });
    (observed, task)
}

#[test]
fn explicit_repair_waits_for_transport_to_close_a_removed_relay() {
    // Assert the public operation and the actual SDK records from the same run.
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "explicit_repair_failed_relay_close_fixture",
            "--ignored",
            "--nocapture",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    println!("{stdout}");
    eprintln!("{stderr}");
    assert!(output.status.success());
    let stable_id = stdout
        .lines()
        .find_map(|line| {
            let record = serde_json::from_str::<serde_json::Value>(line).ok()?;
            (record["event"] == "repair-baseline-trigger")
                .then(|| record["stableId"].as_u64().unwrap())
        })
        .unwrap();
    let events: Vec<serde_json::Value> = stderr
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .filter(|record: &serde_json::Value| {
            record["stableId"] == stable_id && record["source"] == "foreground_repair"
        })
        .collect();
    assert!(!events.iter().any(|record| record["phase"] == "close"));
    assert!(events.iter().any(|record| record["phase"] == "repair"
        && record["result"] == "failed"
        && record["detail"]["output"]["originalOpen"] == true));
    let ended = events
        .iter()
        .find(|record| record["phase"] == "connection-ended")
        .unwrap();
    assert_eq!(ended["detail"]["owner"], "transport");
    assert_eq!(ended["detail"]["reason"], "transport_timeout");
}

#[tokio::test]
#[ignore = "run through explicit_repair_waits_for_transport_to_close_a_removed_relay to assert the emitted records"]
async fn explicit_repair_failed_relay_close_fixture() {
    use iroh::protocol::{AcceptError, ProtocolHandler};
    use miopunch_native_probe::{
        connection_coordinator::MEMBER_ALPN, connection_recovery::ConnectionPathRecoveryError,
    };

    #[derive(Clone, Debug)]
    struct ObserveMember(tokio::sync::mpsc::Sender<Connection>);
    impl ProtocolHandler for ObserveMember {
        async fn accept(&self, original: Connection) -> Result<(), AcceptError> {
            self.0.send(original.clone()).await.unwrap();
            original.closed().await;
            Ok(())
        }
    }

    tokio::time::timeout(Duration::from_secs(60), async {
        let dir = tempfile::tempdir().unwrap();
        let server_store = dir.path().join("server");
        let client_store = dir.path().join("client");
        let mut server = NativeNetwork::create(&server_store).await.unwrap();
        let client = join(&mut server, &client_store).await;
        let mut config = iroh_relay::server::ServerConfig::default();
        config.relay = Some(iroh_relay::server::RelayConfig::new((Ipv4Addr::LOCALHOST, 0)));
        let relay = iroh_relay::server::Server::spawn(config).await.unwrap();
        let relay_url = relay.http_url().unwrap();
        let relay_addr = relay.http_addr().unwrap();
        for node in [&client, &server] {
            let mut config = node.resource_configuration().unwrap();
            config.replacement = Some(vec![Resource {
                url: relay_url.to_string(),
                uses: BTreeSet::from([ResourceUse::DataRelay]),
                limitation: "local repair failure fixture".into(),
            }]);
            config.udp_direct_disabled = true;
            config.tcp_direct_disabled = true;
            config.member_data_relay_disabled = true;
            node.save_resource_configuration(&config).unwrap();
        }
        let (incoming, mut originals) = tokio::sync::mpsc::channel(1);
        let routes = |router: iroh::protocol::RestrictedRouterBuilder| {
            router.accept(MEMBER_ALPN, ObserveMember(incoming.clone()))
        };
        let server = server.maintain_network_with(Ipv4Addr::LOCALHOST, &routes).await.unwrap();
        let client = client.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
        let _server_business = server.business_activity();
        tokio::join!(client.endpoint().online(), server.endpoint().online());
        let target = iroh::EndpointAddr::new(server.endpoint().id())
            .with_addrs([iroh::TransportAddr::Relay(relay_url)]);
        let (original, _business) = client.connect(target.clone(), MEMBER_ALPN).await.unwrap();
        let remote = originals.recv().await.unwrap();
        let (mut send, mut recv) = original.open_bi().await.unwrap();
        send.write_all(b"live original").await.unwrap();
        let (mut reply, mut request) = remote.accept_bi().await.unwrap();
        let mut bytes = [0; 13];
        request.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"live original");
        reply.write_all(&bytes).await.unwrap();
        recv.read_exact(&mut bytes).await.unwrap();

        let stable_id = original.stable_id();
        let peer_stable_id = remote.stable_id();
        relay.shutdown().await.unwrap();
        assert!(original.close_reason().is_none(), "fault begins inside the original lifetime");
        let repaired = client.repair_member_connection_until(
            &original, target.clone(), n0_future::time::Instant::now() + Duration::from_millis(500),
        ).await;
        assert!(matches!(repaired, Err(ConnectionPathRecoveryError::Retryable { stable_id: actual, .. }) if actual == stable_id), "{repaired:?}");
        assert!(original.close_reason().is_none());
        // Noq can restart the idle timer on the first ack-eliciting send since
        // receiving a packet, including the existing five-second keepalive.
        let closed = tokio::time::timeout(Duration::from_secs(40), original.closed()).await.unwrap();
        assert!(matches!(closed, iroh::endpoint::ConnectionError::TimedOut));
        assert!(send.write_all(b"old stream").await.is_err());
        assert!(recv.read_exact(&mut [0]).await.is_err());
        let terminal = client.repair_member_connection_until(
            &original, target.clone(), n0_future::time::Instant::now() + Duration::from_millis(500),
        ).await;
        assert!(matches!(terminal, Err(ConnectionPathRecoveryError::Terminal { stable_id: actual }) if actual == stable_id));
        tokio::join!(client.close(), server.close());

        // Reestablish live relay registrations using the same saved identities.
        // The old M and its failed stream are still held by this caller.
        let mut relay_config = iroh_relay::server::ServerConfig::default();
        relay_config.relay = Some(iroh_relay::server::RelayConfig::new(relay_addr));
        let relay = iroh_relay::server::Server::spawn(relay_config).await.unwrap();
        let client = NativeNetwork::open(&client_store).await.unwrap().maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
        let server = NativeNetwork::open(&server_store).await.unwrap().maintain_network_with(Ipv4Addr::LOCALHOST, routes).await.unwrap();
        tokio::join!(client.endpoint().online(), server.endpoint().online());
        let (replacement, _business) = client.connect(target, MEMBER_ALPN).await.unwrap();
        let replacement_remote = originals.recv().await.unwrap();
        assert_ne!(replacement.stable_id(), stable_id);
        assert_ne!(replacement_remote.stable_id(), peer_stable_id);
        println!("{}", serde_json::json!({"event":"repair-baseline-trigger","stableId":stable_id,"trigger":"relay_shutdown","replacementStableId":replacement.stable_id()}));
        relay.shutdown().await.unwrap();
        client.close().await;
        server.close().await;
    }).await.unwrap();
}

async fn connect(
    client: &RestrictedEndpoint,
    server: &RestrictedEndpoint,
    alpn: &[u8],
) -> (Connection, Connection) {
    native_connections::member_pair(client, server, alpn).await
}

async fn join(admin: &mut NativeNetwork, path: &std::path::Path) -> NativeNetwork {
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
    let receiver = async {
        let ManualExchange::Materials(material) =
            node.request_manual_admission(&client).await.unwrap()
        else {
            panic!("approval")
        };
        node.complete_manual_admission(material).await.unwrap();
    };
    let (_, served) = tokio::join!(receiver, admin.serve_manual_admission(&server));
    served.unwrap();
    ne.close().await;
    ae.close().await;
    node
}

fn configure(node: &NativeNetwork, broker: &str) {
    let mut config = node.resource_configuration().unwrap();
    config.maintenance.exploration_interval_ms = 1_000;
    config.maintenance.discovery_interval_ms = 1_000;
    config.replacement = Some(vec![Resource {
        url: format!("mqtt://{broker}/"),
        uses: BTreeSet::from([ResourceUse::Signaling, ResourceUse::Lookup]),
        limitation: "owned maintenance fixture, never business relay".into(),
    }]);
    node.save_resource_configuration(&config).unwrap();
}

#[tokio::test]
async fn startup_automatically_recovers_and_saves_history_without_business() {
    let broker = std::env::var("MIOPUNCH_MQTT_BROKER")
        .expect("run scripts/check_network_maintenance.py for the real broker");
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let a = join(&mut admin, &dir.path().join("a")).await;
    a.close().await;
    let c = join(&mut admin, &dir.path().join("c")).await;
    admin.close().await;
    let a = NativeNetwork::open(dir.path().join("a")).await.unwrap();
    configure(&a, &broker);
    configure(&c, &broker);
    let c_id = c.status().node_id;
    assert!(!a.status().members.iter().any(|(id, _)| *id == c_id));
    let resources = Arc::new(
        c.bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap(),
    );
    resources
        .endpoint()
        .set_alpns(vec![RECOVERY_REPLICA_ALPN.to_vec()]);
    let serving_resources = resources.clone();
    let serving = tokio::spawn(async move {
        while let Some(incoming) = serving_resources.endpoint().accept().await {
            // Bootstrap history is control-only; ordinary acceptance rejects it.
            let Ok(incoming) = incoming.accept_control() else {
                continue;
            };
            if let Ok(connection) = incoming.await {
                let connection = Arc::new(connection);
                let _ = c
                    .serve_network_recovery(&serving_resources, &connection)
                    .await;
            }
        }
        c.close().await;
    });
    let a = a.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    let recovered = tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            if a.node_status().members.iter().any(|(id, _)| *id == c_id) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .is_ok();
    a.close().await;
    resources.endpoint().close().await;
    serving.await.unwrap();
    let reopened = NativeNetwork::open(dir.path().join("a")).await.unwrap();
    let saved = reopened.status().members.iter().any(|(id, _)| *id == c_id);
    reopened.close().await;
    assert!(
        recovered && saved,
        "startup maintenance must recover and durably save the unknown member without a business request"
    );
}

#[tokio::test]
async fn continuous_failure_backs_off_then_an_online_member_recovers_without_dial_storm() {
    let broker = std::env::var("MIOPUNCH_MQTT_BROKER").expect("owned real broker required");
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let a = join(&mut admin, &dir.path().join("a")).await;
    a.close().await;
    let c = join(&mut admin, &dir.path().join("c")).await;
    admin.close().await;
    let a = NativeNetwork::open(dir.path().join("a")).await.unwrap();
    configure(&a, &broker);
    configure(&c, &broker);
    let (observed, observer) = observe_queries(&broker).await;
    let a = a.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    tokio::time::sleep(Duration::from_secs(11)).await;
    let times = observed.lock().unwrap().clone();
    let backed_off = times.len() >= 3
        && times[2].duration_since(times[1])
            > times[1].duration_since(times[0]) + Duration::from_millis(500);
    let c_id = c.status().node_id;
    let resources = Arc::new(
        c.bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap(),
    );
    resources
        .endpoint()
        .set_alpns(vec![RECOVERY_REPLICA_ALPN.to_vec()]);
    let serving_resources = resources.clone();
    let serving = tokio::spawn(async move {
        while let Some(incoming) = serving_resources.endpoint().accept().await {
            // Bootstrap history is control-only; ordinary acceptance rejects it.
            let Ok(incoming) = incoming.accept_control() else {
                continue;
            };
            if let Ok(connection) = incoming.await {
                let connection = Arc::new(connection);
                let _ = c
                    .serve_network_recovery(&serving_resources, &connection)
                    .await;
            }
        }
        c.close().await;
    });
    let recovered = tokio::time::timeout(Duration::from_secs(14), async {
        loop {
            if a.node_status().members.iter().any(|(id, _)| *id == c_id) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .is_ok();
    a.close().await;
    resources.endpoint().close().await;
    serving.await.unwrap();
    observer.abort();
    eprintln!(
        "OBSERVED discovery_queries={} first_gaps_ms={:?}",
        times.len(),
        times
            .windows(2)
            .map(|w| w[1].duration_since(w[0]).as_millis())
            .collect::<Vec<_>>()
    );
    assert!(
        backed_off,
        "repeated real failed discovery must slow down instead of querying at a fixed interval"
    );
    assert!(
        recovered,
        "a later online member must recover without user business traffic"
    );
    assert!(times.len() <= 4, "no background query storm");
}

#[tokio::test]
async fn a_new_contact_wakes_waiting_maintenance_and_verified_contact_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let a = join(&mut admin, &dir.path().join("a")).await;
    a.close().await;
    let c = join(&mut admin, &dir.path().join("c")).await;
    admin.close().await;
    let a = NativeNetwork::open(dir.path().join("a")).await.unwrap();
    for node in [&a, &c] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(Vec::new());
        node.save_resource_configuration(&config).unwrap();
    }
    let c_id = c.status().node_id;
    let c = c.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    let a = a.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!a.node_status().members.iter().any(|(id, _)| *id == c_id));
    for _ in 0..32 {
        a.offer_contact(c.addr()).await.unwrap();
    }
    let recovered = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if a.node_status().members.iter().any(|(id, _)| *id == c_id)
                && a.maintenance_status().neighbors.len() == 1
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .is_ok();
    let attempts = a.maintenance_status().contact_attempts;
    eprintln!(
        "OBSERVED duplicate_hints=32 actual_contact_attempts={attempts} live_neighbors={}",
        a.maintenance_status().neighbors.len()
    );
    a.close().await;
    assert!(
        recovered,
        "a new allowed direct hint must wake maintenance without business"
    );
    assert!(
        attempts <= 2,
        "duplicate identical hints must not cause simultaneous or repeated dials"
    );
    let a = NativeNetwork::open(dir.path().join("a"))
        .await
        .unwrap()
        .maintain_network(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let restored = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if a.maintenance_status().neighbors.len() == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .is_ok();
    a.close().await;
    c.close().await;
    assert!(
        restored,
        "the verified saved contact must be tried at startup without external discovery"
    );
}

#[tokio::test]
async fn conservative_and_active_nodes_observe_distinct_frequency_and_concurrency_budgets() {
    use miopunch_native_probe::network_maintenance::MaintenancePolicy;
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let mut hints = Vec::new();
    let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut observers = Vec::new();
    for index in 0..4 {
        let peer = join(&mut admin, &dir.path().join(format!("peer{index}"))).await;
        let id = iroh::EndpointId::from_bytes(peer.status().node_id.as_bytes()).unwrap();
        peer.close().await;
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        hints.push(iroh::EndpointAddr::new(id).with_ip_addr(socket.local_addr().unwrap()));
        let observed = observed.clone();
        observers.push(tokio::spawn(async move {
            let mut bytes = [0; 2048];
            while socket.recv_from(&mut bytes).await.is_ok() {
                observed
                    .lock()
                    .unwrap()
                    .push((index, std::time::Instant::now()));
            }
        }));
    }
    let conservative = join(&mut admin, &dir.path().join("conservative")).await;
    let active = join(&mut admin, &dir.path().join("active")).await;
    admin.close().await;
    let mut results = Vec::new();
    for (node, mut policy) in [
        (conservative, MaintenancePolicy::conservative()),
        (active, MaintenancePolicy::active()),
    ] {
        policy.contact_timeout_ms = 500;
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(Vec::new());
        config.maintenance = policy;
        node.save_resource_configuration(&config).unwrap();
        observed.lock().unwrap().clear();
        let node = node.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
        for hint in &hints {
            node.offer_contact(hint.clone()).await.unwrap();
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        let packets = observed.lock().unwrap().clone();
        let peers: BTreeSet<_> = packets.iter().map(|(peer, _)| *peer).collect();
        let status = node.maintenance_status();
        results.push((status.contact_attempts, status.max_in_flight, peers.len()));
        assert!(status.neighbors.is_empty());
        node.close().await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    for observer in observers {
        observer.abort();
    }
    eprintln!(
        "OBSERVED (attempts,max_in_flight,UDP_targets) conservative={:?} active={:?}",
        results[0], results[1]
    );
    assert_eq!(results[0].0, 1);
    assert_eq!(results[0].1, 1);
    assert_eq!(results[0].2, 1);
    assert!(
        results[1].0 >= 4 && results[1].1 > 1 && results[1].1 <= 4 && results[1].2 == 4,
        "active effort must affect actual packets and parallel attempts"
    );
}

const BUSINESS: &[u8] = b"miopunch/maintenance-business/1";

/// Exercise the full active protocol through the public maintained-node entry.
/// First-connection NAT probes deliberately do not stand in for these old flows.
#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[tokio::test]
#[ignore = "owned MIOPUNCH-139 enhanced-UDP experiment; run in its isolated process"]
async fn enhanced_udp_experiment_preserves_original_member_business() {
    use iroh::{EndpointAddr, TransportAddr};
    use miopunch_native_probe::{
        connection_coordinator::MEMBER_ALPN, connection_recovery::ConnectionPathRecoveryError,
    };
    assert_eq!(
        std::env::var("MIOPUNCH_NAT_LAB_ENHANCED_ONLY").as_deref(),
        Ok("1")
    );
    tokio::time::timeout(Duration::from_secs(30), async {
        // These reflect the actual source tuple from the Endpoint UDP socket.
        let mut observers = Vec::new();
        let mut observer_tasks = Vec::new();
        for _ in 0..2 {
            let socket = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            observers.push(Resource { url: format!("stun://{}/", socket.local_addr().unwrap()), uses: BTreeSet::from([ResourceUse::Observation]), limitation: "owned ordered UDP Binding observer".into() });
            observer_tasks.push(tokio::spawn(async move {
                loop {
                    let mut request = [0u8; 1500];
                    let (length, source) = socket.recv_from(&mut request).await.unwrap();
                    assert_eq!(length, 20);
                    assert_eq!(&request[..8], &[0, 1, 0, 0, 0x21, 0x12, 0xa4, 0x42]);
                    let std::net::SocketAddr::V4(source) = source else { panic!("IPv4 fixture") };
                    let mut reply = [0u8; 32];
                    reply[..8].copy_from_slice(&[1, 1, 0, 12, 0x21, 0x12, 0xa4, 0x42]);
                    reply[8..20].copy_from_slice(&request[8..20]);
                    reply[20..26].copy_from_slice(&[0, 0x20, 0, 8, 0, 1]);
                    reply[26..28].copy_from_slice(&(source.port() ^ 0x2112).to_be_bytes());
                    reply[28..32].copy_from_slice(&(u32::from(*source.ip()) ^ 0x2112a442).to_be_bytes());
                    socket.send_to(&reply, source).await.unwrap();
                }
            }));
        }
        let dir = tempfile::tempdir().unwrap();
        let mut creator = NativeNetwork::create(dir.path().join("creator")).await.unwrap();
        let joining = join(&mut creator, &dir.path().join("joining")).await;
        let mut relay_config = iroh_relay::server::ServerConfig::default();
        relay_config.relay = Some(iroh_relay::server::RelayConfig::new((Ipv4Addr::LOCALHOST, 0)));
        let relay = iroh_relay::server::Server::spawn(relay_config).await.unwrap();
        let relay_url = relay.http_url().unwrap();
        let relay_path = TransportAddr::Relay(relay_url.clone());
        for node in [&creator, &joining] {
            let mut config = node.resource_configuration().unwrap();
            let mut resources = observers.clone();
            resources.push(Resource { url: relay_url.to_string(), uses: BTreeSet::from([ResourceUse::DataRelay]), limitation: "owned original member business relay".into() });
            config.replacement = Some(resources);
            config.tcp_direct_disabled = true;
            config.member_data_relay_disabled = true;
            config.maintenance.exploration_interval_ms = 60_000;
            config.maintenance.quality_interval_ms = 30_000;
            config.path_selection.preference = miopunch_native_probe::path_selection::PathPreference::PreferDirect;
            node.save_resource_configuration(&config).unwrap();
        }
        let (incoming, mut accepted) = tokio::sync::mpsc::channel(2);
        let server = creator.maintain_network_with(Ipv4Addr::LOCALHOST, |router| router.accept(MEMBER_ALPN, ObserveBusiness(incoming))).await.unwrap();
        let client = joining.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
        tokio::join!(client.endpoint().online(), server.endpoint().online());
        let target = EndpointAddr::new(server.endpoint().id()).with_addrs([relay_path.clone()]);
        let (original, original_business_lease) = client.connect(target.clone(), MEMBER_ALPN).await.unwrap();
        let remote = accepted.recv().await.unwrap();
        let ids = (original.stable_id(), remote.stable_id());
        let reject = std::env::var("MIOPUNCH_NAT_LAB_REJECT_CANDIDATE").as_deref() == Ok("1");
        for (owner, connection) in [(client.endpoint(), &original), (server.endpoint(), &remote)] {
            // The existing mask is one-way. Only the rejection case installs
            // it; the successful active attempt keeps ordinary UDP permission.
            if reject {
                owner.control_handle(connection).test_limit_transport_paths(vec![relay_path.clone()]).await.unwrap();
            }
            for path in connection.paths().iter().filter(|path| path.is_ip()) { path.close().unwrap(); }
            assert!(connection.paths().iter().any(|p| p.is_selected() && p.is_relay()));
        }
        let (mut send, mut reply) = original.open_bi().await.unwrap();
        send.write_all(b"first").await.unwrap();
        let (mut echo, mut receive) = remote.accept_bi().await.unwrap();
        let mut bytes = [0; 5];
        receive.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"first");
        echo.write_all(b"ready").await.unwrap();
        reply.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"ready");
        let outcome = client.repair_member_connection_until(&original, target, n0_future::time::Instant::now() + Duration::from_secs(3)).await;
        if reject {
            assert!(matches!(&outcome, Err(ConnectionPathRecoveryError::Retryable { stable_id, .. }) if *stable_id == ids.0), "candidate must really fail on original M: {outcome:?}");
            assert!(original.paths().iter().any(|p| p.is_selected() && p.is_relay()));
            assert!(remote.paths().iter().any(|p| p.is_selected() && p.is_relay()));
        } else {
            outcome.as_ref().unwrap(); // The public repair result is after the active ACK/commit barrier.
            assert!(original.paths().iter().any(|p| p.is_selected() && p.is_ip()));
            assert!(remote.paths().iter().any(|p| p.is_selected() && p.is_ip()));
        }
        send.write_all(b"after").await.unwrap();
        receive.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"after");
        echo.write_all(b"there").await.unwrap();
        reply.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"there");
        assert_eq!((original.stable_id(), remote.stable_id()), ids);
        assert!(original.close_reason().is_none() && remote.close_reason().is_none());
        println!("ORIGINAL_ACTIVE_BUSINESS_OK stable_ids={ids:?} rejected_candidate={reject} same_streams_both_directions=true repair={outcome:?}");
        drop(original_business_lease);
        tokio::join!(client.close(), server.close());
        relay.shutdown().await.unwrap();
        for task in observer_tasks { task.abort(); }
    }).await.expect("original active protocol and old streams stay bounded");
}

#[derive(Debug, Clone)]
struct ObserveBusiness(tokio::sync::mpsc::Sender<Connection>);
impl iroh::protocol::ProtocolHandler for ObserveBusiness {
    async fn accept(&self, connection: Connection) -> Result<(), iroh::protocol::AcceptError> {
        self.0.send(connection.clone()).await.unwrap();
        connection.closed().await;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
struct ObserveReplica {
    node: Arc<NativeNetwork>,
    syncs: Arc<std::sync::atomic::AtomicUsize>,
}
#[cfg(target_os = "linux")]
impl std::fmt::Debug for ObserveReplica {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ObserveReplica")
    }
}
#[cfg(target_os = "linux")]
impl iroh::protocol::ProtocolHandler for ObserveReplica {
    async fn accept(&self, connection: Connection) -> Result<(), iroh::protocol::AcceptError> {
        while self.node.serve_replica(&connection).await.is_ok() {
            self.syncs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn replica_sync_cadence_follows_only_exploration_interval() {
    use miopunch_native_probe::{admission_gate::ProtocolScope, offline_replica::REPLICA_ALPN};
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let path = dir.path().join("client");
    let node = join(&mut admin, &path).await;
    node.close().await;
    admin
        .admission_gate()
        .register_protocol(REPLICA_ALPN, ProtocolScope::Member);
    let mut config = admin.resource_configuration().unwrap();
    config.replacement = Some(vec![]);
    admin.save_resource_configuration(&config).unwrap();
    let endpoint = admin
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let address = endpoint.addr();
    let syncs = Arc::new(AtomicUsize::new(0));
    let router = iroh::protocol::RestrictedRouter::builder(endpoint.endpoint().clone())
        .accept(
            REPLICA_ALPN,
            ObserveReplica {
                node: Arc::new(admin),
                syncs: syncs.clone(),
            },
        )
        .spawn();
    for interval in [1_000, 30_000] {
        let client = NativeNetwork::open(&path).await.unwrap();
        config.maintenance.exploration_interval_ms = interval;
        client.save_resource_configuration(&config).unwrap();
        let before = syncs.load(SeqCst);
        let client = client
            .maintain_application_network(Ipv4Addr::LOCALHOST, |b| b)
            .await
            .unwrap();
        client.offer_contact(address.clone()).await.unwrap();
        wait_neighbors(&client, 1).await;
        assert!(syncs.load(SeqCst) > before);
        let first = syncs.load(SeqCst);
        tokio::time::sleep(Duration::from_millis(2500)).await;
        let repeats = syncs.load(SeqCst) - first;
        if interval == 1000 {
            assert!(repeats >= 1);
        } else {
            assert_eq!(repeats, 0);
        }
        client.close().await;
    }
    router.shutdown().await.unwrap();
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn relay_member_improves_during_the_original_business_lease() {
    use iroh::{EndpointAddr, TransportAddr};
    use miopunch_native_probe::{
        connection_coordinator::{ACTIVE_IMPROVEMENT_ALPN, MEMBER_ALPN},
        control_session::ControlSession,
        network_maintenance::MaintenanceState,
        offline_replica::REPLICA_ALPN,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug)]
    struct ObserveOrdinaryDial(Arc<AtomicUsize>);
    impl iroh::endpoint::EndpointHooks for ObserveOrdinaryDial {
        async fn before_connect(
            &self,
            _remote: &EndpointAddr,
            alpn: &[u8],
        ) -> iroh::endpoint::BeforeConnectOutcome {
            if alpn == REPLICA_ALPN {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
            iroh::endpoint::BeforeConnectOutcome::Accept
        }
    }

    let broker = std::env::var("MIOPUNCH_MQTT_BROKER").expect("owned real broker required");
    let dir = tempfile::tempdir().unwrap();
    let mut client = NativeNetwork::create(dir.path().join("client"))
        .await
        .unwrap();
    let server = join(&mut client, &dir.path().join("server")).await;
    let neighbor = join(&mut client, &dir.path().join("neighbor")).await;
    let mut relay_config = iroh_relay::server::ServerConfig::default();
    relay_config.relay = Some(iroh_relay::server::RelayConfig::new((
        Ipv4Addr::LOCALHOST,
        0,
    )));
    let relay = iroh_relay::server::Server::spawn(relay_config)
        .await
        .unwrap();
    let relay_url = relay.http_url().unwrap();
    let observer_ready = Arc::new(tokio::sync::Notify::new());
    let (observer_addr, observations) =
        tcp_binding_observer::tcp_binding_observer(Some(observer_ready.clone()), 2).await;
    let observer = Resource {
        url: format!("stun+tcp://{observer_addr}/"),
        uses: BTreeSet::from([ResourceUse::Observation]),
        limitation: "owned TCP Binding observer".into(),
    };
    for (node, floor) in [(&client, 250), (&server, 60_000)] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(vec![
            Resource {
                url: relay_url.to_string(),
                uses: BTreeSet::from([ResourceUse::DataRelay]),
                limitation: "owned native relay".into(),
            },
            observer.clone(),
        ]);
        config.tcp_listen = Some((Ipv4Addr::LOCALHOST, 0).into());
        config.maintenance.exploration_interval_ms = floor;
        config.maintenance.path_retry_base_ms = floor;
        config.maintenance.enhanced_udp_cooldown_ms = 32 * floor;
        config.maintenance.quality_interval_ms = 250;
        config.path_selection.preference =
            miopunch_native_probe::path_selection::PathPreference::PreferDirect;
        if floor == 250 {
            config.maintenance.discovery_interval_ms = 1_000;
            config.maintenance.max_neighbors = 2;
            config.replacement.as_mut().unwrap().push(Resource {
                url: format!("mqtt://{broker}/"),
                uses: BTreeSet::from([ResourceUse::Signaling, ResourceUse::Lookup]),
                limitation: "owned MQTT broker".into(),
            });
        }
        node.save_resource_configuration(&config).unwrap();
    }
    let mut config = neighbor.resource_configuration().unwrap();
    config.replacement = Some(Vec::new());
    neighbor.save_resource_configuration(&config).unwrap();
    let neighbor_resources = neighbor
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let (directs, mut accepted_direct) = tokio::sync::mpsc::channel(1);
    let syncs = Arc::new(AtomicUsize::new(0));
    let neighbor_router =
        iroh::protocol::RestrictedRouter::builder(neighbor_resources.endpoint().clone())
            .accept(
                REPLICA_ALPN,
                ObserveReplica {
                    node: Arc::new(neighbor),
                    syncs: syncs.clone(),
                },
            )
            .accept(MEMBER_ALPN, ObserveBusiness(directs))
            .spawn();
    let (incoming, mut accepted) = tokio::sync::mpsc::channel(2);
    let server = server
        .maintain_network_with(Ipv4Addr::LOCALHOST, |router| {
            router.accept(MEMBER_ALPN, ObserveBusiness(incoming))
        })
        .await
        .unwrap();
    let (queries, query_task) = observe_queries(&broker).await;
    let udp = client
        .udp_connector()
        .clone()
        .with_maintenance_policy(&client.resource_configuration().unwrap().maintenance)
        .unwrap();
    let gate = client.admission_gate();
    let ordinary_dials = Arc::new(AtomicUsize::new(0));
    let client = client
        .maintain_network_with_endpoint(
            Ipv4Addr::LOCALHOST,
            |builder| builder.hooks(ObserveOrdinaryDial(ordinary_dials.clone())),
            |router| router,
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(25), async {
        tokio::join!(client.endpoint().online(), server.endpoint().online());
        // Learn the neighbor's actual active IP before its ordinary contact
        // considers configured relay fallback for a cold address.
        let direct_target = EndpointAddr::new(neighbor_resources.endpoint().id())
            .with_ip_addr(neighbor_resources.endpoint().bound_sockets()[0]);
        let healthy = client
            .endpoint()
            .connect(direct_target.clone(), MEMBER_ALPN)
            .await
            .unwrap();
        let healthy_remote = accepted_direct.recv().await.unwrap();
        assert!(healthy.paths().iter().any(|path| path.is_ip()));
        client.offer_contact(direct_target).await.unwrap();
        wait_neighbors(&client, 1).await;
        let neighbor_id = client.maintenance_status().neighbors[0].stable_id;
        let setup_lease = client.business_activity();
        let before = client.maintenance_status();
        let before_queries = queries.lock().unwrap().len();
        let before_syncs = syncs.load(Ordering::SeqCst);
        let before_ordinary = ordinary_dials.load(Ordering::SeqCst);
        let relay_path = TransportAddr::Relay(relay_url);
        let target = EndpointAddr::new(server.endpoint().id()).with_addrs([relay_path.clone()]);
        let observation = ControlSession::from_control_connection(
            client
                .endpoint()
                .connect_control(target.clone(), ACTIVE_IMPROVEMENT_ALPN)
                .await
                .unwrap(),
        );
        let held = udp.observe(&observation, &gate).await.unwrap();
        assert!(held.is_current());
        let (original, activity) = client.connect(target.clone(), MEMBER_ALPN).await.unwrap();
        let remote = accepted.recv().await.unwrap();
        drop(setup_lease);
        let ids = (original.stable_id(), remote.stable_id());
        // Both mechanisms stay negotiated. Force the already admitted original
        // onto its relay by masking its UDP paths, while leaving TCP permitted.
        for (owner, connection, peer_addr) in [
            (client.endpoint(), &original, server.addr()),
            (server.endpoint(), &remote, client.addr()),
        ] {
            let tcp = peer_addr
                .addrs
                .into_iter()
                .find(|address| {
                    matches!(address, TransportAddr::Custom(addr)
                    if addr.id() == miopunch_native_probe::tcp_transport::DATA_TRANSPORT_ID)
                })
                .unwrap();
            owner
                .control_handle(connection)
                .test_limit_transport_paths(vec![relay_path.clone(), tcp])
                .await
                .unwrap();
            for path in connection.paths().iter().filter(|path| path.is_ip()) {
                path.close().unwrap();
            }
            assert!(connection.paths().iter().any(|path| path.is_relay()));
            assert!(connection.paths().iter().all(|path| path.is_relay()));
        }

        let (mut send, mut reply) = original.open_bi().await.unwrap();
        send.write_all(b"first").await.unwrap();
        let (mut echo, mut receive) = remote.accept_bi().await.unwrap();
        let mut bytes = [0; 5];
        receive.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"first");
        echo.write_all(b"ready").await.unwrap();
        reply.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"ready");

        client
            .offer_contact(
                EndpointAddr::new(server.endpoint().id()).with_addrs(
                    server
                        .addr()
                        .addrs
                        .into_iter()
                        .filter(|addr| matches!(addr, TransportAddr::Relay(_))),
                ),
            )
            .await
            .unwrap();
        observer_ready.notify_one();
        let direct_tcp = |connection: &Connection| {
            connection
                .paths()
                .iter()
                .find(|path| {
                    matches!(path.remote_addr(), TransportAddr::Custom(addr)
                    if addr.id() == miopunch_native_probe::tcp_transport::DATA_TRANSPORT_ID)
                })
                .map(|path| path.id())
        };
        let mut status = client.watch_maintenance();
        let first = tokio::time::timeout(
            Duration::from_secs(5),
            status.wait_for(|s| {
                s.contact_attempts == before.contact_attempts + 1
                    && s.cause
                        .as_deref()
                        .is_some_and(|cause| cause.contains("active improvement"))
            }),
        )
        .await
        .expect("the first TCP observation fails and UDP reaches the occupied round")
        .unwrap();
        assert!(first.cause.as_deref().unwrap().contains("WouldBlock"));
        drop(first);
        tokio::time::timeout(Duration::from_secs(4), async {
            while client.maintenance_status().contact_attempts < before.contact_attempts + 2
                || [&original, &remote]
                    .into_iter()
                    .any(|m| direct_tcp(m).is_none())
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the same relay original must retry TCP despite its occupied enhanced UDP round");
        assert!(
            held.is_current(),
            "TCP succeeds while the same node's enhanced UDP round is occupied"
        );
        assert_eq!(
            udp.observe(&observation, &gate).await.unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        drop((held, observation));
        let originals = [&original, &remote];
        let previous_paths = originals.map(|connection| direct_tcp(connection).unwrap());
        for (connection, previous) in originals.into_iter().zip(previous_paths) {
            let paths = connection.paths();
            assert!(paths.iter().any(|path| path.is_relay()));
            paths.get(previous).unwrap().close().unwrap();
            assert!(
                direct_tcp(connection).is_none(),
                "actual direct path absent after abandonment"
            );
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            while originals
                .into_iter()
                .zip(previous_paths)
                .any(|(connection, previous)| {
                    direct_tcp(connection).is_none_or(|id| id == previous)
                })
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("each original acquires a new direct path after abandonment");
        tokio::time::sleep(Duration::from_millis(1100)).await;
        let active = client.maintenance_status();
        assert_eq!(active.state, MaintenanceState::BusinessPriority);
        assert!((2..=3).contains(&(active.contact_attempts - before.contact_attempts)));
        assert_eq!(ordinary_dials.load(Ordering::SeqCst), before_ordinary);
        assert_eq!(active.discovery_rounds, before.discovery_rounds);
        assert_eq!(queries.lock().unwrap().len(), before_queries);
        assert_eq!(syncs.load(Ordering::SeqCst), before_syncs);
        assert_eq!(active.neighbors.len(), 1);
        assert_eq!((original.stable_id(), remote.stable_id()), ids);
        tokio::time::timeout(Duration::from_secs(3), async {
            while [&original, &remote].into_iter().any(|connection| {
                let paths = connection.paths();
                direct_tcp(connection)
                    .and_then(|id| paths.get(id))
                    .is_none_or(|path| !path.is_selected())
            }) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the new qualified path is selected on both original handles");
        send.write_all(b"after").await.unwrap();
        receive.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"after");
        echo.write_all(b"there").await.unwrap();
        reply.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"there");
        drop(activity);
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                let status = client.maintenance_status();
                if status.discovery_rounds > before.discovery_rounds
                    && queries.lock().unwrap().len() > before_queries
                    && syncs.load(Ordering::SeqCst) > before_syncs
                    && ordinary_dials.load(Ordering::SeqCst) > before_ordinary
                    && status.neighbors.len() == 2
                {
                    assert!(status.neighbors.iter().any(|n| n.stable_id == neighbor_id));
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("lease release resumes due sync, discovery and ordinary contacts");
        send.write_all(b"final").await.unwrap();
        receive.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"final");
        echo.write_all(b"again").await.unwrap();
        reply.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"again");
        assert!(healthy_remote.close_reason().is_none());
    })
    .await
    .unwrap();
    tokio::join!(client.close(), server.close());
    neighbor_router.shutdown().await.unwrap();
    neighbor_resources.close().await;
    relay.shutdown().await.unwrap();
    observations.abort();
    query_task.abort();
}

#[tokio::test]
async fn only_a_real_network_generation_restarts_failed_member_pacing() {
    use iroh::{
        EndpointAddr, TransportAddr,
        endpoint::ControlConnection,
        protocol::{AcceptError, ControlProtocolHandler},
    };
    use miopunch_native_probe::connection_coordinator::{ACTIVE_IMPROVEMENT_ALPN, MEMBER_ALPN};
    use tokio::{sync::mpsc, time::Instant};

    #[derive(Debug)]
    struct UnavailablePath(mpsc::UnboundedSender<Instant>);
    impl ControlProtocolHandler for UnavailablePath {
        async fn accept(&self, control: ControlConnection) -> Result<(), AcceptError> {
            self.0.send(Instant::now()).unwrap();
            control.close(0u32.into(), b"fixture path unavailable");
            Ok(())
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let mut client = NativeNetwork::create(dir.path().join("client"))
        .await
        .unwrap();
    let server = join(&mut client, &dir.path().join("server")).await;
    let mut relay_config = iroh_relay::server::ServerConfig::default();
    relay_config.relay = Some(iroh_relay::server::RelayConfig::new((
        Ipv4Addr::LOCALHOST,
        0,
    )));
    let relay = iroh_relay::server::Server::spawn(relay_config)
        .await
        .unwrap();
    let relay_url = relay.http_url().unwrap();
    for (node, floor) in [(&client, 250), (&server, 60_000)] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(vec![Resource {
            url: relay_url.to_string(),
            uses: BTreeSet::from([ResourceUse::DataRelay]),
            limitation: "owned native relay".into(),
        }]);
        config.udp_direct_disabled = floor == 60_000;
        config.maintenance.exploration_interval_ms = floor;
        config.maintenance.path_retry_base_ms = floor;
        config.maintenance.enhanced_udp_cooldown_ms = 32 * floor;
        config.maintenance.contact_timeout_ms = 5_000;
        node.save_resource_configuration(&config).unwrap();
    }
    let (incoming, mut accepted) = mpsc::channel(1);
    let (failures, mut failed) = mpsc::unbounded_channel();
    let server = server
        .maintain_network_with(Ipv4Addr::LOCALHOST, |router| {
            router
                .accept(MEMBER_ALPN, ObserveBusiness(incoming))
                .accept_control(ACTIVE_IMPROVEMENT_ALPN, UnavailablePath(failures))
        })
        .await
        .unwrap();
    let client = client.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    tokio::time::timeout(Duration::from_secs(20), async {
        tokio::join!(server.endpoint().online(), client.endpoint().online());
        let target =
            EndpointAddr::new(server.endpoint().id()).with_addrs([TransportAddr::Relay(relay_url)]);
        let (original, _lease) = client.connect(target.clone(), MEMBER_ALPN).await.unwrap();
        let remote = accepted.recv().await.unwrap();
        assert!(remote.paths().iter().all(|path| path.is_relay()));
        let first = failed.recv().await.unwrap();
        let generation = client.endpoint().udp_network_generation();
        for _ in 0..8 {
            client.network_changed().await.unwrap();
            client.offer_contact(target.clone()).await.unwrap();
        }
        let second = failed.recv().await.unwrap();
        assert!(second.duration_since(first) >= Duration::from_millis(500));
        for _ in 0..8 {
            client.network_changed().await.unwrap();
        }
        let third = failed.recv().await.unwrap();
        assert!(third.duration_since(second) >= Duration::from_secs(1));
        assert!(
            generation.is_current(),
            "selector notifications are not endpoint changes"
        );

        // Let the third actual failure settle with F × 8 due. Then produce
        // a real endpoint generation change through its normal entry point.
        let mut status = client.watch_maintenance();
        status.wait_for(|s| s.contact_attempts == 3).await.unwrap();
        client.endpoint().network_change().await;
        while generation.is_current() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let reset = tokio::time::timeout(Duration::from_millis(1750), failed.recv())
            .await
            .expect("generation invalidates the current session delay")
            .unwrap();
        assert!(reset.duration_since(third) < Duration::from_secs(2));
        for _ in 0..12 {
            client.network_changed().await.unwrap();
        }
        let after_reset = failed.recv().await.unwrap();
        assert!(after_reset.duration_since(reset) >= Duration::from_millis(500));
        assert!(original.close_reason().is_none());
        original.close(0u32.into(), b"original completed");
        remote.closed().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(1250), failed.recv())
                .await
                .is_err()
        );

        let (replacement, _replacement_lease) = client.connect(target, MEMBER_ALPN).await.unwrap();
        let replacement_remote = accepted.recv().await.unwrap();
        assert_ne!(replacement.stable_id(), original.stable_id());
        assert_ne!(replacement_remote.stable_id(), remote.stable_id());
        tokio::time::timeout(Duration::from_millis(1750), failed.recv())
            .await
            .expect("replacement gets its own first maintenance opportunity")
            .unwrap();
        eprintln!(
            "OBSERVED real_failure_gaps_ms={:?} first_after_generation_ms={} original={}",
            [
                second.duration_since(first).as_millis(),
                third.duration_since(second).as_millis()
            ],
            after_reset.duration_since(reset).as_millis(),
            original.stable_id()
        );
    })
    .await
    .unwrap();
    tokio::join!(client.close(), server.close());
    relay.shutdown().await.unwrap();
}

#[derive(Debug)]
struct Echo;
impl iroh::protocol::ProtocolHandler for Echo {
    async fn accept(&self, connection: Connection) -> Result<(), iroh::protocol::AcceptError> {
        while let Ok((mut send, mut recv)) = connection.accept_bi().await {
            let Ok(bytes) = recv.read_to_end(64 * 1024).await else {
                break;
            };
            if send.write_all(&bytes).await.is_err() {
                break;
            }
            if send.finish().is_err() {
                break;
            }
        }
        Ok(())
    }
}
async fn wait_neighbors(
    node: &miopunch_native_probe::network_maintenance::MaintainedNetwork,
    count: usize,
) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if node.maintenance_status().neighbors.len() == count {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("real neighbor state");
}

#[tokio::test]
async fn real_business_has_priority_and_shared_identity_guard_prevents_duplicate_dials() {
    use miopunch_native_probe::{
        admission_gate::ProtocolScope, network_maintenance::MaintenanceState,
    };
    let broker = std::env::var("MIOPUNCH_MQTT_BROKER").expect("owned real broker required");
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let a = join(&mut admin, &dir.path().join("a")).await;
    a.close().await;
    let c = join(&mut admin, &dir.path().join("c")).await;
    admin.close().await;
    let a = NativeNetwork::open(dir.path().join("a")).await.unwrap();
    for node in [&a, &c] {
        configure(node, &broker);
        node.admission_gate()
            .register_protocol(BUSINESS, ProtocolScope::Member);
    }
    let c = c
        .maintain_network_with(Ipv4Addr::LOCALHOST, |router| router.accept(BUSINESS, Echo))
        .await
        .unwrap();
    let c_priority = c.business_activity();
    let (queries, observer) = observe_queries(&broker).await;
    let a = a.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    a.offer_contact(c.addr()).await.unwrap();
    wait_neighbors(&a, 1).await;
    let original_neighbor = a.maintenance_status().neighbors[0].stable_id;
    let target = c.addr();
    let (first, duplicate) = tokio::join!(
        a.connect(target.clone(), BUSINESS),
        a.connect(target, BUSINESS)
    );
    assert!(
        first.is_ok() && duplicate.is_err(),
        "foreground business dials share one in-flight guard for each identity"
    );
    let (connection, lease) = first.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while a.maintenance_status().state != MaintenanceState::BusinessPriority {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let before = queries.lock().unwrap().len();
    let before_rounds = a.maintenance_status().discovery_rounds;
    let (mut send, mut recv) = connection.open_bi().await.unwrap();
    let bytes = vec![0x59; 32 * 1024];
    send.write_all(&bytes).await.unwrap();
    send.finish().unwrap();
    assert_eq!(recv.read_to_end(64 * 1024).await.unwrap(), bytes);
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(
        queries.lock().unwrap().len(),
        before,
        "a real business lease suppresses background lookup packets"
    );
    assert_eq!(a.maintenance_status().discovery_rounds, before_rounds);
    assert_eq!(
        a.maintenance_status().neighbors[0].stable_id,
        original_neighbor
    );
    connection.close(0u32.into(), b"business completed");
    drop(lease);
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if a.maintenance_status().discovery_rounds > before_rounds {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("maintenance resumes after business");
    drop(c_priority);
    observer.abort();
    a.close().await;
    c.close().await;
    eprintln!(
        "OBSERVED business_echo_bytes=32768 background_queries_during_business=0 duplicate_in_flight_rejected=true"
    );
}

#[tokio::test]
async fn partial_links_propagate_new_signed_history_on_the_same_neighbor_session() {
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let a = join(&mut admin, &dir.path().join("a")).await;
    a.close().await;
    let c = join(&mut admin, &dir.path().join("c")).await;
    let a = NativeNetwork::open(dir.path().join("a")).await.unwrap();
    for node in [&a, &c, &admin] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(Vec::new());
        config.maintenance.exploration_interval_ms = 250;
        node.save_resource_configuration(&config).unwrap();
    }
    let c = c.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    let a = a.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    a.offer_contact(c.addr()).await.unwrap();
    wait_neighbors(&a, 1).await;
    let original = a.maintenance_status().neighbors[0].stable_id;
    let a_priority = a.business_activity();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let d = join(&mut admin, &dir.path().join("d")).await;
    let d_id = d.status().node_id;
    d.close().await;
    let admin = admin.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    c.offer_contact(admin.addr()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if c.node_status().members.iter().any(|(id, _)| *id == d_id) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("C syncs the original signed D authorization");
    admin.close().await; // Neither original author nor D remains online for A.
    assert!(!a.node_status().members.iter().any(|(id, _)| *id == d_id));
    drop(a_priority);
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if a.node_status().members.iter().any(|(id, _)| *id == d_id) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("A gets durable D records over partial A-C connectivity");
    let status = a.maintenance_status();
    assert_eq!(status.neighbors.len(), 1);
    assert_eq!(status.neighbors[0].stable_id, original);
    assert_eq!(status.last_sync.as_ref().unwrap().source, c.endpoint().id());
    assert!(status.last_sync.as_ref().unwrap().complete());
    a.close().await;
    c.close().await;
    let a = NativeNetwork::open(dir.path().join("a")).await.unwrap();
    assert!(a.status().members.iter().any(|(id, _)| *id == d_id));
    a.close().await;
    eprintln!(
        "OBSERVED partial_link=A-C-Creator original_author_offline_at_A_sync=true same_neighbor_session=true saved_new_member=true"
    );
}

#[tokio::test]
async fn node_quality_budget_changes_actual_quic_probe_frequency() {
    use miopunch_native_probe::network_maintenance::MaintenancePolicy;
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let a = join(&mut admin, &dir.path().join("a")).await;
    let c = join(&mut admin, &dir.path().join("c")).await;
    // A must know C so its first ordinary session can be sampled directly.
    let a = {
        let mut a = a;
        for record in admin.management_history().await.unwrap() {
            a.ingest_management_record(record.header(), record.body())
                .await
                .unwrap();
        }
        a
    };
    for node in [&admin, &a, &c] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(Vec::new());
        config.maintenance.exploration_interval_ms = 250;
        node.save_resource_configuration(&config).unwrap();
    }
    admin.close().await;
    let c = c.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    a.close().await;
    let mut measured = Vec::new();
    // Reopen exactly the same durable A identity in the same controlled data
    // environment, varying only the maintenance quality interval.
    for interval in [5_000, 1_000] {
        let a = NativeNetwork::open(dir.path().join("a")).await.unwrap();
        let mut config = a.resource_configuration().unwrap();
        config.maintenance = MaintenancePolicy {
            exploration_interval_ms: 250,
            quality_interval_ms: interval,
            ..MaintenancePolicy::conservative()
        };
        a.save_resource_configuration(&config).unwrap();
        let a = a.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
        a.offer_contact(c.addr()).await.unwrap();
        wait_neighbors(&a, 1).await;
        let id = a.maintenance_status().neighbors[0].stable_id;
        let before = a.maintenance_status().neighbors[0].pings_sent;
        tokio::time::sleep(Duration::from_millis(3_500)).await;
        let status = a.maintenance_status();
        assert_eq!(status.neighbors[0].stable_id, id);
        measured.push(status.neighbors[0].pings_sent.saturating_sub(before));
        a.close().await;
        // C's peer-side connection promptly closes; keep its serving endpoint.
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    c.close().await;
    eprintln!(
        "OBSERVED actual_QUIC_ping_delta conservative={} active={} window_ms=3500",
        measured[0], measured[1]
    );
    assert!(
        measured[1] >= 3 && measured[1] > measured[0],
        "quality budget must affect QUIC probes, beyond saved configuration values"
    );
}

#[tokio::test]
async fn active_budget_never_enables_disabled_lookup_or_forbidden_data_carriers() {
    use miopunch_native_probe::network_maintenance::{MaintenancePolicy, MaintenanceState};
    let broker = std::env::var("MIOPUNCH_MQTT_BROKER").expect("owned real broker required");
    let dir = tempfile::tempdir().unwrap();
    let node = NativeNetwork::create(dir.path().join("node"))
        .await
        .unwrap();
    configure(&node, &broker);
    let mut config = node.resource_configuration().unwrap();
    config.maintenance = MaintenancePolicy::active();
    config.udp_direct_disabled = true;
    config.tcp_direct_disabled = true;
    config
        .disabled_uses
        .extend([ResourceUse::Lookup, ResourceUse::DataRelay]);
    node.save_resource_configuration(&config).unwrap();
    let (queries, observer) = observe_queries(&broker).await;
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let id = iroh::SecretKey::generate().public();
    let target = iroh::EndpointAddr::new(id).with_addrs([
        iroh::TransportAddr::Ip(socket.local_addr().unwrap()),
        iroh::TransportAddr::Custom(iroh_base::CustomAddr::from_parts(
            miopunch_native_probe::member_transport::DATA_TRANSPORT_ID,
            &[0; 32],
        )),
    ]);
    let node = node.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    for _ in 0..16 {
        node.offer_contact(target.clone()).await.unwrap();
        node.network_changed().await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let mut packet = [0; 2048];
    assert!(
        tokio::time::timeout(Duration::from_millis(100), socket.recv_from(&mut packet))
            .await
            .is_err()
    );
    let status = node.maintenance_status();
    assert_eq!(status.discovery_rounds, 0);
    assert_eq!(status.contact_attempts, 0);
    assert_eq!(queries.lock().unwrap().len(), 0);
    assert_eq!(status.state, MaintenanceState::TemporarilyUnreachable);
    assert!(status.cause.as_ref().unwrap().contains("no permitted"));
    assert!(status.node.members.len() == 1 && status.neighbors.is_empty());
    node.close().await;
    observer.abort();
    eprintln!(
        "OBSERVED active_budget=true disabled_lookup_queries=0 forbidden_data_dials=0 UDP_packets=0 waiting_retains_membership=true"
    );
}

#[tokio::test]
async fn stale_saved_contact_expands_to_discovery_and_recovers_an_unknown_member() {
    let broker = std::env::var("MIOPUNCH_MQTT_BROKER").expect("owned real broker required");
    let dir = tempfile::tempdir().unwrap();
    let mut b = NativeNetwork::create(dir.path().join("b")).await.unwrap();
    let a = join(&mut b, &dir.path().join("a")).await;
    for node in [&a, &b] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(Vec::new());
        config.maintenance.exploration_interval_ms = 250;
        config.maintenance.contact_timeout_ms = 500;
        node.save_resource_configuration(&config).unwrap();
    }
    let b = b.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    let a = a.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    a.offer_contact(b.addr()).await.unwrap();
    wait_neighbors(&a, 1).await;
    a.close().await;
    b.close().await;
    let mut b = NativeNetwork::open(dir.path().join("b")).await.unwrap();
    let c = join(&mut b, &dir.path().join("c")).await;
    let c_id = c.status().node_id;
    b.close().await;
    let a = NativeNetwork::open(dir.path().join("a")).await.unwrap();
    assert!(!a.status().members.iter().any(|(id, _)| *id == c_id));
    configure(&a, &broker);
    configure(&c, &broker);
    let c = c.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    let c_priority = c.business_activity(); // It serves but initiates no competing discovery.
    let a = a.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    tokio::time::timeout(Duration::from_secs(16), async {
        loop {
            if a.node_status().members.iter().any(|(id, _)| *id == c_id) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("the unavailable saved route expands to qualified discovery");
    let status = a.maintenance_status();
    assert!(status.contact_attempts >= 2 && status.discovery_rounds >= 1);
    assert_eq!(status.last_sync.as_ref().unwrap().source, c.endpoint().id());
    assert!(status.last_sync.as_ref().unwrap().complete());
    drop(c_priority);
    a.close().await;
    c.close().await;
    eprintln!(
        "OBSERVED saved_contact_unavailable=true discovery_expanded=true unknown_source_verified_and_saved=true"
    );
}

#[tokio::test]
async fn starting_business_does_not_abort_an_already_admitted_sync_exchange() {
    use miopunch_native_probe::{admission_gate::ProtocolScope, offline_replica::REPLICA_ALPN};
    struct HeldReplica {
        node: Arc<NativeNetwork>,
        accepted: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
        completed: Arc<tokio::sync::Notify>,
    }
    impl std::fmt::Debug for HeldReplica {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("HeldReplica")
        }
    }
    impl iroh::protocol::ProtocolHandler for HeldReplica {
        async fn accept(&self, connection: Connection) -> Result<(), iroh::protocol::AcceptError> {
            self.accepted.notify_one();
            self.release.notified().await;
            let result = self.node.serve_replica(&connection).await;
            if result.is_ok() {
                self.completed.notify_one();
            }
            while connection.close_reason().is_none() {
                if self.node.serve_replica(&connection).await.is_err() {
                    break;
                }
            }
            Ok(())
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let mut a = join(&mut admin, &dir.path().join("a")).await;
    let c = join(&mut admin, &dir.path().join("c")).await;
    for record in admin.management_history().await.unwrap() {
        a.ingest_management_record(record.header(), record.body())
            .await
            .unwrap();
    }
    admin.close().await;
    for node in [&a, &c] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(Vec::new());
        config.maintenance.exploration_interval_ms = 250;
        node.save_resource_configuration(&config).unwrap();
        node.admission_gate()
            .register_protocol(REPLICA_ALPN, ProtocolScope::Member);
    }
    let resources = c
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let accepted = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let completed = Arc::new(tokio::sync::Notify::new());
    let router = iroh::protocol::RestrictedRouter::builder(resources.endpoint().clone())
        .accept(
            REPLICA_ALPN,
            HeldReplica {
                node: Arc::new(c),
                accepted: accepted.clone(),
                release: release.clone(),
                completed: completed.clone(),
            },
        )
        .spawn();
    let a = a.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    a.offer_contact(resources.addr()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), accepted.notified())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let activity = a.business_activity();
    tokio::time::sleep(Duration::from_millis(100)).await;
    release.notify_one();
    let finished = tokio::time::timeout(Duration::from_secs(3), completed.notified())
        .await
        .is_ok();
    drop(activity);
    a.close().await;
    router.shutdown().await.unwrap();
    resources.close().await;
    assert!(
        finished,
        "business priority must stop new exploration while allowing the already admitted bounded LogSync exchange to finish"
    );
}

#[cfg(feature = "replica-test-hooks")]
#[tokio::test]
async fn a_full_neighbor_budget_still_explores_a_new_source_without_requiring_full_mesh() {
    use miopunch_native_probe::{
        admission_gate::ProtocolScope, management_store::ManagementRecord,
        offline_replica::REPLICA_ALPN,
    };
    struct FrozenSource {
        node: Arc<NativeNetwork>,
        records: Vec<ManagementRecord>,
    }
    impl std::fmt::Debug for FrozenSource {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("FrozenSource")
        }
    }
    impl iroh::protocol::ProtocolHandler for FrozenSource {
        async fn accept(&self, connection: Connection) -> Result<(), iroh::protocol::AcceptError> {
            while connection.close_reason().is_none() {
                if self
                    .node
                    .serve_replica_fixture(&connection, &self.records)
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Ok(())
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let mut b = NativeNetwork::create(dir.path().join("b")).await.unwrap();
    let mut a = join(&mut b, &dir.path().join("a")).await;
    let mut c = join(&mut b, &dir.path().join("c")).await;
    let frozen = b.management_history().await.unwrap();
    for record in &frozen {
        a.ingest_management_record(record.header(), record.body())
            .await
            .unwrap();
    }
    let d = join(&mut b, &dir.path().join("d")).await;
    let d_id = d.status().node_id;
    d.close().await;
    for record in b.management_history().await.unwrap() {
        c.ingest_management_record(record.header(), record.body())
            .await
            .unwrap();
    }
    for node in [&a, &b, &c] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(Vec::new());
        config.maintenance.exploration_interval_ms = 250;
        node.save_resource_configuration(&config).unwrap();
        node.admission_gate()
            .register_protocol(REPLICA_ALPN, ProtocolScope::Member);
    }
    let b_resources = b
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let b_router = iroh::protocol::RestrictedRouter::builder(b_resources.endpoint().clone())
        .accept(
            REPLICA_ALPN,
            FrozenSource {
                node: Arc::new(b),
                records: frozen,
            },
        )
        .spawn();
    let c = c.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    let a = a.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    a.offer_contact(b_resources.addr()).await.unwrap();
    wait_neighbors(&a, 1).await;
    let original = a.maintenance_status().neighbors[0].stable_id;
    assert!(!a.node_status().members.iter().any(|(id, _)| *id == d_id));
    a.offer_contact(c.addr()).await.unwrap();
    let recovered = tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if a.node_status().members.iter().any(|(id, _)| *id == d_id) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .is_ok();
    let status = a.maintenance_status();
    a.close().await;
    c.close().await;
    b_router.shutdown().await.unwrap();
    b_resources.close().await;
    assert!(
        recovered,
        "a full permanent neighbor view still samples a newly reachable source for better state coverage"
    );
    assert_eq!(status.neighbors.len(), 1);
    assert_eq!(status.neighbors[0].stable_id, original);
}

#[tokio::test]
async fn repeated_authenticated_replica_failures_increment_backoff_and_leave_connected_state() {
    use miopunch_native_probe::{
        admission_gate::ProtocolScope, network_maintenance::MaintenanceState,
        offline_replica::REPLICA_ALPN,
    };
    struct FailingReplica {
        node: Arc<NativeNetwork>,
        accepted: Arc<std::sync::Mutex<Vec<std::time::Instant>>>,
        failed: Arc<std::sync::atomic::AtomicBool>,
    }
    impl std::fmt::Debug for FailingReplica {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("FailingReplica")
        }
    }
    impl iroh::protocol::ProtocolHandler for FailingReplica {
        async fn accept(&self, connection: Connection) -> Result<(), iroh::protocol::AcceptError> {
            self.accepted
                .lock()
                .unwrap()
                .push(std::time::Instant::now());
            loop {
                if !self.failed.load(std::sync::atomic::Ordering::Acquire) {
                    if self.node.serve_replica(&connection).await.is_err() {
                        break;
                    }
                    self.failed
                        .store(true, std::sync::atomic::Ordering::Release);
                } else {
                    let Ok((mut send, mut recv)) = connection.accept_bi().await else {
                        break;
                    };
                    let mut request = [0; 4];
                    if recv.read_exact(&mut request).await.is_err() {
                        break;
                    }
                    assert_eq!(&request, b"REPL");
                    // A real authenticated peer sends an invalid manifest, not
                    // a mocked success or a failed TLS handshake.
                    if send.write_all(&[0, 0, 0, 1, 0xff]).await.is_err() {
                        break;
                    }
                    let _ = connection.closed().await;
                    break;
                }
            }
            Ok(())
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let mut a = join(&mut admin, &dir.path().join("a")).await;
    let c = join(&mut admin, &dir.path().join("c")).await;
    for record in admin.management_history().await.unwrap() {
        a.ingest_management_record(record.header(), record.body())
            .await
            .unwrap();
    }
    admin.close().await;
    for node in [&a, &c] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(Vec::new());
        config.maintenance.exploration_interval_ms = 250;
        node.save_resource_configuration(&config).unwrap();
        node.admission_gate()
            .register_protocol(REPLICA_ALPN, ProtocolScope::Member);
    }
    let accepted = Arc::new(std::sync::Mutex::new(Vec::new()));
    let resources = c
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let router = iroh::protocol::RestrictedRouter::builder(resources.endpoint().clone())
        .accept(
            REPLICA_ALPN,
            FailingReplica {
                node: Arc::new(c),
                accepted: accepted.clone(),
                failed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            },
        )
        .spawn();
    let a = a.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    a.offer_contact(resources.addr()).await.unwrap();
    wait_neighbors(&a, 1).await; // The source succeeds once, then fails LogSync.
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if a.maintenance_status().neighbors.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let no_stale_connected = a.maintenance_status().state != MaintenanceState::Connected;
    tokio::time::sleep(Duration::from_secs(5)).await;
    let times = accepted.lock().unwrap().clone();
    let gaps = times
        .windows(2)
        .map(|window| window[1].duration_since(window[0]))
        .collect::<Vec<_>>();
    a.close().await;
    router.shutdown().await.unwrap();
    resources.close().await;
    eprintln!(
        "OBSERVED authenticated_failed_replica_gaps_ms={:?} stale_connected={}",
        gaps.iter().map(Duration::as_millis).collect::<Vec<_>>(),
        !no_stale_connected
    );
    assert!(
        no_stale_connected,
        "the final failed neighbor must report waiting rather than Connected"
    );
    assert!(
        gaps.len() >= 2 && gaps[1] > gaps[0] + Duration::from_millis(500),
        "successful TLS followed by failed LogSync must not reset consecutive failure backoff"
    );
}
