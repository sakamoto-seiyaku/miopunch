//! Formal native TCP traversal through two real kernel NATs.
use iroh::{EndpointAddr, TransportAddr};
use miopunch_native_probe::{
    admission_gate::{ADMISSION_REQUEST_ALPN, ProtocolScope},
    manual_admission::ManualExchange,
    mqtt_transport::MqttTransport,
    native_network::NativeNetwork,
    offline_replica::REPLICA_ALPN,
    resources::{Resource, ResourceUse},
};

#[derive(Debug)]
struct StageIds(std::sync::Arc<std::sync::Mutex<Vec<usize>>>);
impl iroh::endpoint::EndpointHooks for StageIds {
    async fn before_data_release(
        &self,
        _: &iroh::Endpoint,
        connection: &iroh::endpoint::Connection,
    ) -> std::io::Result<()> {
        self.0.lock().unwrap().push(connection.stable_id());
        Ok(())
    }
}

fn peer_target(directory: &Path, server: bool) -> EndpointAddr {
    let id =
        std::fs::read_to_string(directory.join(if server { "joining-id" } else { "admin-id" }))
            .unwrap()
            .parse()
            .unwrap();
    let route = std::fs::read_to_string(directory.join(if server {
        "client-route"
    } else {
        "server-route"
    }))
    .unwrap();
    EndpointAddr::new(id).with_addrs([TransportAddr::Custom(
        MqttTransport::address(&route).unwrap(),
    )])
}

async fn wait_file(directory: &Path, name: &str) {
    tokio::time::timeout(Duration::from_secs(8), async {
        while !directory.join(name).exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
use std::{collections::BTreeSet, net::Ipv4Addr, path::Path, time::Duration};
async fn prepare(directory: &Path) {
    let mut admin = NativeNetwork::create(directory.join("admin"))
        .await
        .unwrap();
    let mut joining = NativeNetwork::join(
        directory.join("joining"),
        admin.manual_invitation().unwrap(),
    )
    .await
    .unwrap();
    for node in [&admin, &joining] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(vec![]);
        node.save_resource_configuration(&config).unwrap();
    }
    let a = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let b = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (outgoing, incoming) = tokio::join!(b.connect(a.addr(), ADMISSION_REQUEST_ALPN), async {
        a.accept().await.unwrap().await
    },);
    let outgoing = outgoing.unwrap();
    let incoming = incoming.unwrap();
    let (request, served) = tokio::join!(
        joining.request_manual_admission(&outgoing),
        admin.serve_manual_admission(&incoming),
    );
    assert!(matches!(request.unwrap(), ManualExchange::AwaitingApproval));
    served.unwrap();
    let request = admin.manual_requests().await.unwrap().pop().unwrap();
    admin.approve_manual_admission(&request).await.unwrap();
    let receive = async {
        let ManualExchange::Materials(materials) =
            joining.request_manual_admission(&outgoing).await.unwrap()
        else {
            panic!("saved approval");
        };
        joining.complete_manual_admission(materials).await.unwrap();
    };
    let (_, served) = tokio::join!(receive, admin.serve_manual_admission(&incoming));
    served.unwrap();
    outgoing.close(0u32.into(), b"prepared");
    incoming.close(0u32.into(), b"prepared");
    b.close().await;
    a.close().await;
    for node in [&admin, &joining] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(vec![
            Resource {
                url: "mqtt://100.64.27.1:1883/".into(),
                uses: BTreeSet::from([ResourceUse::Signaling]),
                limitation: "owned signaling broker".into(),
            },
            Resource {
                url: "stun+tcp://100.64.27.1:3478/".into(),
                uses: BTreeSet::from([ResourceUse::Observation]),
                limitation: "owned RFC8489 TCP binding observer; candidate only".into(),
            },
        ]);
        let fallback = std::env::var("MIO43_CASE").is_ok_and(|case| {
            matches!(
                case.as_str(),
                "udp_fallback" | "mixed_tcp" | "observer_failure" | "candidate_denied"
            )
        });
        if fallback {
            config.replacement.as_mut().unwrap().push(Resource {
                url: "https://qad.miopunch.test/".into(),
                uses: BTreeSet::from([ResourceUse::Observation]),
                limitation: "owned UDP QAD; independently permitted fallback".into(),
            });
        }
        if std::env::var("MIO43_CASE").is_ok_and(|case| case == "candidate_denied")
            && node.status().node_id == joining.status().node_id
        {
            config
                .disabled_tcp_addresses
                .insert("100.64.27.6:43027".parse().unwrap());
        }
        config.udp_direct_disabled = !fallback;
        config.tcp_direct_disabled = std::env::var("MIO43_CASE")
            .is_ok_and(|case| case == "mixed_tcp")
            && node.status().node_id == admin.status().node_id;
        config.tcp_listen = Some(
            (
                Ipv4Addr::UNSPECIFIED,
                if node.status().node_id == admin.status().node_id {
                    43027
                } else {
                    42027
                },
            )
                .into(),
        );
        if std::env::var("MIO43_CASE").is_ok_and(|case| case == "private_bind") {
            let address = if node.status().node_id == admin.status().node_id {
                "10.27.2.2:43027"
            } else {
                "10.27.1.2:42027"
            };
            config.tcp_listen = Some(address.parse().unwrap());
        }
        node.save_resource_configuration(&config).unwrap();
    }
    std::fs::write(
        directory.join("admin-id"),
        admin.status().node_id.to_string(),
    )
    .unwrap();
    std::fs::write(
        directory.join("joining-id"),
        joining.status().node_id.to_string(),
    )
    .unwrap();
    joining.close().await;
    admin.close().await;
}

#[test]
fn tcp_native_punch_worker() {
    let Ok(role) = std::env::var("MIO43_ROLE") else {
        return;
    };
    let directory = std::path::PathBuf::from(std::env::var("MIO43_DIRECTORY").unwrap());
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(run_worker(role, directory));
}

async fn run_worker(role: String, directory: std::path::PathBuf) {
    if role == "prepare" {
        prepare(&directory).await;
        return;
    }
    let server = role == "serve";
    let case = std::env::var("MIO43_CASE").unwrap();
    let stage_ids = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut node = NativeNetwork::open(directory.join(if server { "admin" } else { "joining" }))
        .await
        .unwrap();
    node.admission_gate()
        .register_protocol(REPLICA_ALPN, ProtocolScope::Member);
    let fallback = std::env::var("MIO43_CASE").is_ok_and(|case| {
        matches!(
            case.as_str(),
            "udp_fallback" | "mixed_tcp" | "observer_failure" | "candidate_denied"
        )
    });
    let builder = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
        .bind_addr((Ipv4Addr::UNSPECIFIED, if server { 43027 } else { 42027 }))
        .unwrap();
    let builder = if fallback {
        builder.ca_tls_config(iroh_relay::tls::CaTlsConfig::custom_roots(vec![
            std::fs::read(directory.join("cert.der")).unwrap().into(),
        ]))
    } else {
        builder
    };
    let builder = if case == "cancel_attempt" {
        builder.hooks(StageIds(stage_ids.clone()))
    } else {
        builder
    };
    let mut resources = node.bind_configured_endpoint_with(builder).await.unwrap();
    let endpoint = resources.endpoint().clone();
    endpoint.set_alpns(vec![REPLICA_ALPN.to_vec()]);
    assert_eq!(
        endpoint.bound_sockets().is_empty(),
        !fallback,
        "UDP requires separate local permission"
    );
    let route = endpoint
        .custom_addrs()
        .into_iter()
        .find(|a| a.id() == miopunch_native_probe::mqtt_transport::TRANSPORT_ID)
        .unwrap();
    std::fs::write(
        directory.join(if server {
            "server-route"
        } else {
            "client-route"
        }),
        hex::encode(route.data()),
    )
    .unwrap();
    println!(
        "READY id={} tcp={:?} mapping=false udp={fallback} relay=false",
        endpoint.id(),
        resources.addr()
    );
    let mut simultaneous_connection = None;
    if case == "cancel_attempt" {
        if server {
            assert!(endpoint.accept().await.unwrap().await.is_err());
            println!("CANCELED_ORIGINAL peer_closed=true endpoint_alive=true");
        } else {
            let dialing = endpoint.clone();
            let target = peer_target(&directory, false);
            let attempt = tokio::spawn(async move { dialing.connect(target, REPLICA_ALPN).await });
            wait_file(&directory, "cancel").await;
            attempt.abort();
            assert!(attempt.await.unwrap_err().is_cancelled());
            tokio::time::sleep(Duration::from_millis(100)).await;
            let pending = std::process::Command::new("ss")
                .args(["-Htan", "state", "syn-sent", "sport", "=", ":42027"])
                .output()
                .unwrap();
            assert!(
                pending.status.success(),
                "actual socket observation command failed"
            );
            assert!(
                pending.stdout.is_empty(),
                "canceled attempt must release socket while endpoint remains alive"
            );
            assert!(!endpoint.custom_addrs().is_empty());
            println!("CANCELED_ORIGINAL endpoint_alive=true pending_sockets=0");
            std::fs::write(directory.join("cancel-done"), b"").unwrap();
            wait_file(&directory, "retry").await;
        }
    }
    if case == "simultaneous_cancel" || case == "simultaneous" {
        while !directory
            .join(if server {
                "client-route"
            } else {
                "server-route"
            })
            .exists()
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let id =
            std::fs::read_to_string(directory.join(if server { "joining-id" } else { "admin-id" }))
                .unwrap()
                .parse()
                .unwrap();
        let route = std::fs::read_to_string(directory.join(if server {
            "client-route"
        } else {
            "server-route"
        }))
        .unwrap();
        let target = EndpointAddr::new(id).with_addrs([TransportAddr::Custom(
            MqttTransport::address(&route).unwrap(),
        )]);
        let dialing = endpoint.clone();
        let outgoing = tokio::spawn(async move { dialing.connect(target, REPLICA_ALPN).await });
        let accepting = endpoint.clone();
        let incoming = tokio::spawn(async move { accepting.accept().await.unwrap().await });
        if case == "simultaneous" {
            let (outgoing, incoming) = tokio::time::timeout(Duration::from_secs(10), async {
                tokio::join!(outgoing, incoming)
            })
            .await
            .unwrap();
            let mut winners = [outgoing.unwrap().ok(), incoming.unwrap().ok()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            assert_eq!(
                winners.len(),
                1,
                "simultaneous TCP attempts must release exactly one live authenticated session"
            );
            let winner = winners.pop().unwrap();
            assert_eq!(winner.remote_id(), id);
            println!(
                "SIMULTANEOUS_OK live_sessions=1 original_connection={} authenticated_peer={}",
                winner.stable_id(),
                winner.remote_id()
            );
            simultaneous_connection = Some(winner);
        } else if !server {
            tokio::time::timeout(Duration::from_secs(5), async {
                while !directory.join("cancel").exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            outgoing.abort();
            incoming.abort();
            let _ = outgoing.await;
            let _ = incoming.await;
            let before = std::time::Instant::now();
            endpoint.close().await;
            tokio::time::sleep(Duration::from_millis(100)).await;
            let sockets = std::process::Command::new("ss")
                .args(["-Htan", "state", "syn-sent", "sport", "=", ":42027"])
                .output()
                .unwrap();
            assert!(
                sockets.status.success(),
                "actual socket observation command failed"
            );
            let sockets = String::from_utf8(sockets.stdout).unwrap();
            println!(
                "CANCEL_SOCKET_SNAPSHOT elapsed_ms={} remaining={sockets:?}",
                before.elapsed().as_millis()
            );
            assert!(
                sockets.is_empty(),
                "cancelled endpoint may not retain pending same-port TCP dial sockets"
            );
            println!(
                "CANCEL_OK both_started=true node_identity={} pending_sockets=0",
                endpoint.id()
            );
        } else {
            let (outgoing, incoming) = tokio::time::timeout(Duration::from_secs(8), async {
                tokio::join!(outgoing, incoming)
            })
            .await
            .unwrap();
            assert!(outgoing.unwrap().is_err() && incoming.unwrap().is_err());
            endpoint.close().await;
            println!(
                "CANCEL_OK peer_cancel_closed_both=true node_identity={}",
                endpoint.id()
            );
        }
        if case == "simultaneous_cancel" {
            node.close().await;
            return;
        }
    }
    let started = std::time::Instant::now();
    let mut connection = tokio::time::timeout(Duration::from_secs(20), async {
        if let Some(connection) = simultaneous_connection {
            Ok(connection)
        } else if server {
            endpoint
                .accept()
                .await
                .unwrap()
                .await
                .map_err(|e| format!("{e:?}"))
        } else {
            let id = if case == "wrong_target" {
                iroh::SecretKey::generate().public()
            } else {
                std::fs::read_to_string(directory.join("admin-id"))
                    .unwrap()
                    .parse()
                    .unwrap()
            };
            let route = std::fs::read_to_string(directory.join("server-route")).unwrap();
            resources
                .connect(
                    EndpointAddr::new(id).with_addrs([TransportAddr::Custom(
                        MqttTransport::address(&route).unwrap(),
                    )]),
                    REPLICA_ALPN,
                )
                .await
                .map_err(|e| format!("{e:?}"))
        }
    })
    .await
    .unwrap();
    if case == "wrong_target" {
        assert!(
            connection.is_err(),
            "wrong expected node identity must fail before any TCP data path"
        );
        assert!(started.elapsed() < Duration::from_secs(3));
        println!(
            "WRONG_TARGET_REJECTED error={:?} data_released=false",
            connection.err()
        );
        connection = tokio::time::timeout(Duration::from_secs(10), async {
            if server {
                endpoint
                    .accept()
                    .await
                    .unwrap()
                    .await
                    .map_err(|error| format!("{error:?}"))
            } else {
                let id = std::fs::read_to_string(directory.join("admin-id"))
                    .unwrap()
                    .parse()
                    .unwrap();
                let route = std::fs::read_to_string(directory.join("server-route")).unwrap();
                resources
                    .connect(
                        EndpointAddr::new(id).with_addrs([TransportAddr::Custom(
                            MqttTransport::address(&route).unwrap(),
                        )]),
                        REPLICA_ALPN,
                    )
                    .await
                    .map_err(|error| format!("{error:?}"))
            }
        })
        .await
        .unwrap();
    }
    if case == "blocked" || case == "endpoint_dependent" {
        assert!(
            connection.is_err(),
            "unsupported topology must not release data"
        );
        assert!(
            started.elapsed() < Duration::from_secs(9),
            "unsupported topology must fail within its traversal bound"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
        let sockets = std::process::Command::new("ss")
            .args([
                "-Htan",
                "state",
                "syn-sent",
                "sport",
                "=",
                if server { ":43027" } else { ":42027" },
            ])
            .output()
            .unwrap();
        assert!(
            sockets.status.success(),
            "actual socket observation command failed"
        );
        assert!(
            sockets.stdout.is_empty(),
            "failed traversal must release pending socket"
        );
        if !server {
            let opened = endpoint.metrics().socket.num_conns_opened.get();
            let id = std::fs::read_to_string(directory.join("admin-id"))
                .unwrap()
                .parse()
                .unwrap();
            let route = std::fs::read_to_string(directory.join("server-route")).unwrap();
            let target = EndpointAddr::new(id).with_addrs([TransportAddr::Custom(
                MqttTransport::address(&route).unwrap(),
            )]);
            for _ in 0..100 {
                assert!(
                    resources
                        .connect(target.clone(), REPLICA_ALPN)
                        .await
                        .is_err()
                );
            }
            assert_eq!(
                endpoint.metrics().socket.num_conns_opened.get(),
                opened,
                "failed attempts must back off"
            );
        }
        println!(
            "UNSUPPORTED_BOUNDED case={case} elapsed_ms={} data_released=false pending_sockets=0",
            started.elapsed().as_millis()
        );
        resources.close().await;
        node.close().await;
        return;
    }
    let connection = connection.expect("dual NAT must return formal authenticated TCP connection");
    if case == "cancel_attempt" {
        let ids = stage_ids.lock().unwrap();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
        assert_eq!(ids[1], connection.stable_id());
        println!(
            "CANCEL_RETRY_OK old={} new={} same_node_identity={}",
            ids[0],
            ids[1],
            endpoint.id()
        );
    }
    if fallback {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let sockets = std::process::Command::new("ss")
            .args([
                "-Htan",
                "state",
                "syn-sent",
                "sport",
                "=",
                if server { ":43027" } else { ":42027" },
            ])
            .output()
            .unwrap();
        assert!(
            sockets.status.success(),
            "actual socket observation command failed"
        );
        println!(
            "LOSER_SOCKET_SNAPSHOT pending={:?}",
            String::from_utf8_lossy(&sockets.stdout)
        );
        assert!(
            sockets.stdout.is_empty(),
            "UDP winner must cancel all TCP loser sockets"
        );
        assert!(
            connection
                .paths()
                .iter()
                .any(|p| matches!(p.remote_addr(), TransportAddr::Ip(_)))
        );
        println!(
            "FALLBACK_OK permitted_udp=true original_connection={}",
            connection.stable_id()
        );
    } else {
        assert!(connection.paths().iter().any(|p| matches!(p.remote_addr(),TransportAddr::Custom(a) if a.id()==miopunch_native_probe::tcp_transport::DATA_TRANSPORT_ID)));
    }
    tokio::time::timeout(Duration::from_secs(2), async {
            while connection.paths().iter().any(|p| matches!(p.remote_addr(),TransportAddr::Custom(a) if a.id()==miopunch_native_probe::mqtt_transport::TRANSPORT_ID)) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.expect("forbidden MQTT path must finish closing");
    assert!(connection.paths().iter().all(|p| !matches!(p.remote_addr(),TransportAddr::Custom(a) if a.id()==miopunch_native_probe::mqtt_transport::TRANSPORT_ID)));
    let server = if case == "simultaneous" {
        connection.side().is_server()
    } else {
        server
    };
    if server {
        node.serve_replica(&connection).await.unwrap();
        let (mut send, mut recv) = connection.accept_bi().await.unwrap();
        assert_eq!(
            recv.read_to_end(256).await.unwrap(),
            b"actual business over punched TCP"
        );
        send.write_all(b"authorized ACK").await.unwrap();
        send.finish().unwrap();
        if case == "simultaneous" {
            let repeated = tokio::time::timeout(Duration::from_secs(5), async {
                endpoint.accept().await.unwrap().await
            })
            .await
            .unwrap();
            assert!(
                repeated.is_err(),
                "duplicate must not release another valid peer session"
            );
            println!(
                "DUPLICATE_OK first_connection={} still_live={}",
                connection.stable_id(),
                connection.close_reason().is_none()
            );
        }
        connection.closed().await;
    } else {
        assert!(node.sync_replica(&connection).await.unwrap().complete());
        let (mut send, mut recv) = connection.open_bi().await.unwrap();
        send.write_all(b"actual business over punched TCP")
            .await
            .unwrap();
        send.finish().unwrap();
        assert_eq!(recv.read_to_end(256).await.unwrap(), b"authorized ACK");
        if case == "simultaneous" {
            let role_server = role == "serve";
            let route = std::fs::read_to_string(directory.join(if role_server {
                "client-route"
            } else {
                "server-route"
            }))
            .unwrap();
            let target =
                EndpointAddr::new(connection.remote_id()).with_addrs([TransportAddr::Custom(
                    MqttTransport::address(&route).unwrap(),
                )]);
            assert!(
                resources.connect(target, REPLICA_ALPN).await.is_err(),
                "repeated contact may not create a second valid session while original TCP session is live"
            );
            assert!(connection.close_reason().is_none());
            println!(
                "DUPLICATE_OK first_connection={} still_live=true",
                connection.stable_id()
            );
        }
        connection.close(0u32.into(), b"done");
    }
    println!(
        "DATA_OK stable_id={} peer={} formal_member=true same_connection=true",
        connection.stable_id(),
        connection.remote_id()
    );
    resources.close().await;
    node.close().await;
}
