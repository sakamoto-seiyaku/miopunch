//! Worker for the two translated native members in check_member_fusion_nat.py.

use std::{env, io, net::UdpSocket, path::Path};

use iroh_relay::server::{RelayConfig, Server, ServerConfig};
use serde_json::json;

use super::*;

const WAN: Ipv4Addr = Ipv4Addr::new(100, 64, 27, 1);
const CLIENT: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 27, 1, 2)), 42027);
const SERVER: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 27, 2, 2)), 43027);
const STUN_PORTS: [u16; 2] = [46761, 46762];
const RELAY_PORT: u16 = 46760;
const FIXED_BUDGET: Duration = Duration::from_secs(5);
const RANDOM_BUDGET: Duration = Duration::from_secs(38);

// Keep the transport running while this worker independently rebinds up to 256 ports.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires the isolated two-NAT lab"]
async fn enhanced_udp_through_translated_members() {
    let role = env::var("MIO150_WORKER").unwrap();
    let root = env::var("MIO150_ROOT").unwrap();
    let root = Path::new(&root);
    let relay_url = format!("http://{WAN}:{RELAY_PORT}/")
        .parse::<RelayUrl>()
        .unwrap();
    if role == "setup" {
        let mut config = ServerConfig::default();
        config.relay = Some(RelayConfig::new((WAN, RELAY_PORT)));
        let relay = Server::spawn(config).await.unwrap();
        let (admin, _) = admitted_pair(root, &relay_url).await;
        std::fs::write(root.join("admin.id"), admin.status().node_id.to_string()).unwrap();
        admin.close().await;
        let [first, second] = STUN_PORTS;
        let a = tokio::net::UdpSocket::bind((WAN, first)).await.unwrap();
        let b = tokio::net::UdpSocket::bind((WAN, second)).await.unwrap();
        println!("MIO150 LAB_READY");
        tokio::join!(
            async {
                loop {
                    reply_one_stun(&a).await;
                }
            },
            async {
                loop {
                    reply_one_stun(&b).await;
                }
            }
        );
        relay.shutdown().await.unwrap();
        return;
    }

    let a = role == "a";
    let mode = env::var("MIO150_CASE").unwrap();
    let visitor = env::var("MIO150_VISITOR").unwrap() == role;
    let keep_backup = mode == "random-auto";
    let automatic = mode == "fixed-auto" || keep_backup;
    let cancel_during_probes = mode == "random-cancel";
    let blocked = mode.ends_with("blocked") || cancel_during_probes;
    let random = mode.starts_with("random");
    let budget = if random { RANDOM_BUDGET } else { FIXED_BUDGET };
    let node = NativeNetwork::open(root.join(if a { "joining" } else { "admin" }))
        .await
        .unwrap();
    let mut resources = node.resource_configuration().unwrap();
    resources.replacement = Some(
        [resource(&relay_url)]
            .into_iter()
            .chain(STUN_PORTS.map(|port| observation_resource(format!("stun://{WAN}:{port}/"))))
            .collect(),
    );
    resources.udp_direct_disabled = false;
    resources.path_selection.preference = if keep_backup {
        PathPreference::PreferRelay
    } else {
        PathPreference::Quality
    };
    resources.maintenance.contact_timeout_ms = budget.as_millis() as u64;
    if mode == "standby-retry" {
        resources.maintenance.exploration_interval_ms =
            if env::var("MIO150_VISITOR").unwrap() == "a" {
                1_000
            } else {
                30_000
            };
        resources.maintenance.quality_interval_ms = 250;
        resources.maintenance.enhanced_udp_cooldown_ms = 1_500;
        resources.maintenance.path_retry_base_ms = 250;
    }
    if automatic {
        resources.maintenance.quality_interval_ms = if visitor { 250 } else { 3_000 };
    }
    node.save_resource_configuration(&resources).unwrap();

    let primary = if a { CLIENT } else { SERVER };
    let dials = Dials::default();
    let endpoint = node
        .bind_member_fusion_endpoint_with(
            Endpoint::builder(presets::Minimal)
                .clear_ip_transports()
                .bind_addr(primary)
                .unwrap()
                .hooks(dials.clone()),
            SETUP,
        )
        .await
        .unwrap();
    let connection = time::timeout(SETUP, async {
        endpoint.online().await;
        if a {
            let peer = std::fs::read_to_string(root.join("admin.id"))
                .unwrap()
                .parse()
                .unwrap();
            endpoint
                .connect(EndpointAddr::new(peer).with_relay_url(relay_url.clone()))
                .await
                .unwrap()
        } else {
            endpoint.accept().await.unwrap().unwrap()
        }
    })
    .await
    .unwrap();
    let stable_id = connection.stable_id();
    assert!(relay_is_selected(&connection, &relay_url));
    if mode == "standby-retry" {
        let (failed, winner) = time::timeout(Duration::from_secs(40), async {
            let mut failed = None;
            loop {
                if let Some(report) = connection.udp_traversal() {
                    if report.outcome.is_some_and(|result| result.is_err()) && failed.is_none() {
                        std::fs::write(root.join(format!("failed-{role}")), []).unwrap();
                        failed = Some(report.round);
                    } else if let Some(first) = failed
                        && first != report.round
                        && report.validated.is_some()
                        && connection
                            .paths()
                            .iter()
                            .any(|p| p.is_ip() && p.is_selected())
                    {
                        break (first, report.round);
                    }
                }
                time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("standby retry on the original without any stream or ping");
        let mut stream = if a {
            connection.open_bi().await.unwrap()
        } else {
            connection.accept_bi().await.unwrap()
        };
        business(&mut stream, a, 158).await.unwrap();
        assert_eq!(connection.stable_id(), stable_id);
        assert!(
            connection
                .paths()
                .iter()
                .any(|p| p.is_ip() && p.is_selected())
        );
        println!(
            "MIO150 RESULT {}",
            json!({"success":true,"attempt":hex::encode(member_fusion_attempt_id(&connection).unwrap()),"failed":failed,"winner":winner,"stable":stable_id,"stream":u64::from(stream.streams().0.id())})
        );
        std::fs::write(root.join(format!("done-{role}")), []).unwrap();
        time::timeout(BUSINESS_LIMIT, async {
            while !root.join(if a { "done-b" } else { "done-a" }).exists() {
                time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        endpoint.close().await;
        node.close().await;
        return;
    }
    let mut stream = time::timeout(BUSINESS_LIMIT, async {
        if a {
            let mut stream = connection.open_bi().await.unwrap();
            business(&mut stream, a, 150).await.unwrap();
            stream
        } else {
            let mut stream = connection.accept_bi().await.unwrap();
            assert!(stream.is_business());
            business(&mut stream, a, 150).await.unwrap();
            stream
        }
    })
    .await
    .unwrap();
    let stream_id = stream.streams().0.id();
    let deadline = Instant::now() + budget;
    let cancel = CancellationToken::new();
    let peer_probing = root.join("peer-probing");
    let cancel_requested = root.join("cancel-requested");
    let operate = async {
        let mut plan = None;
        let mut saw_request = None;
        if blocked {
            business(&mut stream, a, 151).await.unwrap();
            assert!(relay_is_selected(&connection, &relay_url));
        }
        loop {
            if plan.is_none() {
                plan = connection.udp_plan();
            }
            if let Some(report) = connection.udp_traversal() {
                if report.outcome.is_some() {
                    business(&mut stream, a, 152).await.unwrap();
                    break;
                }
                if cancel_during_probes {
                    if visitor && peer_probing.exists() && !cancel.is_cancelled() {
                        assert!(report.deadline > Instant::now() + Duration::from_secs(20));
                        std::fs::write(&cancel_requested, []).unwrap();
                        cancel.cancel();
                    } else if !visitor && !peer_probing.exists() && report.trace.probes != 0 {
                        assert!(report.deadline > Instant::now() + Duration::from_secs(20));
                        println!(
                            "MIO150 CANCEL_READY side={role} probes={} listeners={}",
                            report.trace.probes,
                            report.listeners.len()
                        );
                        std::fs::write(&peer_probing, []).unwrap();
                    }
                    if !visitor && cancel_requested.exists() {
                        let since = saw_request.get_or_insert_with(Instant::now);
                        assert!(
                            since.elapsed() < Duration::from_secs(2),
                            "peer still probing after caller cancellation: sent={} bound={} remaining={:?}",
                            report.trace.probes,
                            report.listeners.len(),
                            report.deadline.saturating_duration_since(Instant::now())
                        );
                    }
                }
                if !automatic
                    && !blocked
                    && report.validated.is_some_and(|(id, _)| {
                        connection
                            .paths()
                            .iter()
                            .any(|path| path.id() == id && path.is_selected())
                    })
                {
                    connection
                        .confirm_udp_business(report.round, &mut stream, async |flow| {
                            business(flow, a, 152).await
                        })
                        .await
                        .unwrap();
                    break;
                }
            }
            time::sleep(Duration::from_millis(10)).await;
        }
        loop {
            let report = connection.udp_traversal().unwrap();
            if report.outcome.is_some() {
                break (report, plan);
            }
            time::sleep(Duration::from_millis(10)).await;
        }
    };

    let (result, (report, plan)) = time::timeout_at(deadline + Duration::from_secs(3), async {
        tokio::join!(
            async {
                if visitor && !automatic {
                    Some(connection.coordinate_udp_traversal(deadline, &cancel).await)
                } else {
                    None
                }
            },
            operate
        )
    })
    .await
    .expect("bounded original business and round cleanup");
    let success = report.outcome == Some(Ok(()));
    let plan = result
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .or(plan.as_ref());
    let (mode_id, index, phase_role, packet_limit, samples) =
        plan.as_ref()
            .map(|plan| {
                (
                    Some(plan.id.mode),
                    Some(plan.id.index),
                    Some(format!("{:?}", plan.local_phase.behavior.role)),
                    Some(plan.local_phase.peer_datagram_bound(
                        usize::from(plan.local_phase.behavior.listen_random) + 1,
                    )),
                    plan.local_samples()
                        .iter()
                        .map(|sample| sample.mapped)
                        .collect::<Vec<_>>(),
                )
            })
            .unwrap_or_default();
    assert_eq!(connection.stable_id(), stable_id);
    assert_eq!(report.connection_id, stable_id);
    assert_eq!(stream.streams().0.id(), stream_id);
    assert_eq!(dials.0.load(Ordering::Relaxed), usize::from(a));

    let mut retained = 0;
    let mut released = 0;
    for bind in &report.listeners {
        let won = success
            && report
                .winner
                .as_ref()
                .is_some_and(|winner| winner.local_socket_id == bind.id);
        match UdpSocket::bind(bind.local_addr) {
            Ok(_) => {
                assert!(!won, "won temporary socket must still be bound");
                released += 1;
            }
            Err(error) => {
                assert!(
                    won && error.kind() == io::ErrorKind::AddrInUse,
                    "{bind:?}: {error}"
                );
                retained += 1;
            }
        }
    }
    if keep_backup && success {
        let (id, _) = report.validated.unwrap();
        let winner = report.winner.as_ref().unwrap();
        let paths = connection.paths();
        let backup = paths.get(id).expect("retained UDP backup after round");
        assert!(relay_is_selected(&connection, &relay_url));
        assert!(!backup.is_selected());
        assert_eq!(backup.local_socket_id(), Some(winner.local_socket_id));
        assert_eq!(backup.remote_addr(), &TransportAddr::Ip(winner.remote));
        assert_eq!(retained, usize::from(a));
    }
    if random {
        // One peer may have 256 OS sockets to inspect while the other has none.
        std::fs::write(root.join(format!("ports-checked-{role}")), []).unwrap();
        let peer = if a { "b" } else { "a" };
        let checked = root.join(format!("ports-checked-{peer}"));
        wait_for("peer's port checks", SETUP, || checked.exists()).await;
    }
    if success {
        let (id, ack) = report.validated.unwrap();
        let winner = report.winner.as_ref().unwrap();
        if keep_backup {
            endpoint
                .test_limit_transport_paths(&connection, vec![TransportAddr::Ip(winner.remote)])
                .await
                .unwrap();
            for relay in connection.paths().iter().filter(|path| path.is_relay()) {
                // The peer may have closed this path after the snapshot.
                let _ = relay.close();
            }
        }
        if automatic {
            // Selection is asynchronous even when the quality policy prefers UDP.
            wait_for("selection of the retained winner", BUSINESS_LIMIT, || {
                connection
                    .paths()
                    .get(id)
                    .is_some_and(|path| path.is_selected())
            })
            .await;
            assert_eq!(connection.udp_traversal().unwrap().round, report.round);
        }
        assert!(connection.paths().iter().any(|path| path.id() == id
            && path.is_selected()
            && path.local_socket_id() == Some(winner.local_socket_id)
            && path.remote_addr() == &TransportAddr::Ip(winner.remote)));
        assert!(ack.sent_at >= report.first_match.as_ref().unwrap().1);
        if automatic {
            assert!(report.business.is_none() && !report.learned);
            let bytes = connection.test_observe_stream_receive(id, &stream).unwrap();
            time::timeout(BUSINESS_LIMIT, async {
                // Arm both original readers before either side puts bytes on that stream.
                let mut ready = if a {
                    connection.open_maintenance_bi().await?
                } else {
                    connection.accept_bi().await?
                };
                assert!(!ready.is_business());
                let (send, recv) = ready.streams();
                let mut received = [0];
                if a {
                    send.write_all(&[151]).await.unwrap();
                }
                recv.read_exact(&mut received).await.unwrap();
                assert_eq!(received, [151]);
                if !a {
                    send.write_all(&received).await.unwrap();
                }
                send.finish().unwrap();
                business(&mut stream, a, 154).await
            })
            .await
            .unwrap()
            .unwrap();
            assert!(
                bytes.confirm(),
                "ordinary business bytes came from the enhanced winner"
            );
        } else {
            assert_eq!(report.business.unwrap().0, stream_id);
            assert!(report.business.unwrap().1 >= ack.received_at);
            assert_eq!(report.learned, !visitor);
        }
    } else {
        assert!(report.business.is_none() && !report.learned);
        assert!(relay_is_selected(&connection, &relay_url));
        assert_eq!(retained, 0);
    }
    if blocked {
        assert!(!success);
    }
    if mode == "fixed" || automatic {
        assert!(success, "{report:?}");
    }
    if let Some(result) = result.as_ref() {
        assert_eq!(result.is_ok(), success);
    }
    if cancel_during_probes {
        assert!(cancel_requested.exists());
        assert_eq!(report.outcome, Some(Err(io::ErrorKind::Interrupted)));
        assert!(report.deadline.into_std() - report.finished.unwrap() > Duration::from_secs(20));
        if !visitor {
            assert!(report.trace.probes > 0);
        } else {
            // A new protected plan on the same original requires both nodes' slots.
            let renewed = time::timeout(
                BUSINESS_LIMIT,
                connection.coordinate_udp_plan(Instant::now() + budget, &CancellationToken::new()),
            )
            .await
            .expect("bounded new plan on the same original")
            .expect("both round slots released after cancellation");
            assert_ne!(renewed.round, report.round);
            println!("MIO150 SLOT_REUSED side={role}");
        }
    }
    let to_ms = |at: std::time::Instant| at.saturating_duration_since(report.started).as_millis();
    let first_match = report.first_match.as_ref().map(|(packet, at)| {
        json!({
            "ms": to_ms(*at), "socket": packet.local_socket_id.get(), "local": packet.local_addr,
            "remote": packet.remote, "response": packet.response,
        })
    });
    let winner = report.winner.as_ref().map(|packet| json!({
        "socket": packet.local_socket_id.get(), "local": packet.local_addr, "remote": packet.remote,
    }));
    println!(
        "MIO150 RESULT {}",
        json!({
            "side":role, "visitor":visitor, "case":mode, "round":hex::encode(report.round),
            "attempt":hex::encode(member_fusion_attempt_id(&connection).unwrap()),
            "stable":stable_id, "stream":u64::from(stream_id), "mode":mode_id, "index":index,
            "frp_role":phase_role, "samples":samples, "packet_limit":packet_limit,
            "match":first_match,"winner":winner,"ack_ms":report.validated.map(|(_, ack)| to_ms(ack.received_at)),
            "business_ms":report.business.map(|(_, at)| to_ms(at)),
            "automatic_business_verified":automatic && success,
            "duration_ms":to_ms(report.finished.unwrap()),"deadline_ms":to_ms(report.deadline.into_std()),
            "probes":report.trace.probes,"responses":report.trace.responses,
            "listener_attempted":report.trace.listener_attempted, "listener_bound":report.trace.listener_bound,
            "retained":retained,"released":released,"learned":report.learned,"success":success,
            "relay_after":relay_is_selected(&connection,&relay_url), "outcome":format!("{:?}",report.outcome),
        })
    );

    time::timeout(BUSINESS_LIMIT, business(&mut stream, a, 153))
        .await
        .unwrap()
        .unwrap();
    stream.streams().0.finish().unwrap();
    if a {
        connection.closed().await;
    } else {
        time::timeout(BUSINESS_LIMIT, async {
            while stream
                .streams()
                .1
                .read(&mut [0; 1])
                .await
                .is_ok_and(|result| result.is_some())
            {}
        })
        .await
        .ok();
    }
    node.close().await;
    endpoint.close().await;
}

async fn business(stream: &mut MemberStream, a: bool, nonce: u64) -> io::Result<()> {
    let (send, recv) = stream.streams();
    let expected = nonce.to_be_bytes();
    let mut actual = [0; 8];
    if a {
        send.write_all(&expected).await.map_err(io::Error::other)?;
    }
    recv.read_exact(&mut actual)
        .await
        .map_err(io::Error::other)?;
    assert_eq!(expected, actual);
    if !a {
        send.write_all(&actual).await.map_err(io::Error::other)?;
    }
    Ok(())
}
