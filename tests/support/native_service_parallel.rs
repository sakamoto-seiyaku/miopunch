//! First business and late path completion through the production client.
use super::*;
use std::sync::Arc;

#[path = "tcp_binding_observer.rs"]
mod tcp_binding_observer;

#[tokio::test]
async fn udp_first_reply_keeps_the_started_tcp_round() {
    let broker = relay_contact_broker::Broker::new().await;
    let start_tcp = Arc::new(tokio::sync::Notify::new());
    let (observer, task, mut observations) =
        tcp_binding_observer::tcp_binding_observer_with_sources(Some(start_tcp.clone()), 0).await;
    let mut resources = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![Resource {
            url: format!("mqtt://{}/", broker.address),
            uses: [ResourceUse::Signaling].into(),
            limitation: "owned first-contact control".into(),
        }],
    });
    resources.tcp_direct_disabled = true;
    let Pair {
        root,
        provider,
        consumer,
        ..
    } = Pair::admit_with_resources(resources.clone()).await;
    consumer.close().await.unwrap();
    provider.close().await.unwrap();
    resources.tcp_direct_disabled = false;
    resources.member_data_relay_disabled = true;
    resources.maintenance.contact_timeout_ms = 8_000;
    resources.defaults.resources.push(Resource {
        url: format!("stun+tcp://{observer}/"),
        uses: [ResourceUse::Observation].into(),
        limitation: "held until first UDP business bytes".into(),
    });
    for name in ["provider", "consumer"] {
        let node = NativeNetwork::open(root.path().join(name)).await.unwrap();
        node.save_resource_configuration(&resources).unwrap();
        node.close().await;
    }
    let provider = NativeClient::open(root.path().join("provider"), Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let consumer = NativeClient::open(root.path().join("consumer"), Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let sink = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let target = sink.local_addr().unwrap();
    provider
        .enable_forwarding(Protocol::Tcp, vec![target])
        .await
        .unwrap();
    let mut contact = provider.invitation().await.unwrap().address().clone();
    contact.addrs.retain(|address| {
        matches!(address, iroh::TransportAddr::Custom(route)
        if route.id() == miopunch_native_probe::mqtt_datagram::TRANSPORT_ID)
    });
    assert_eq!(contact.addrs.len(), 1);
    let listener = consumer
        .start_forwarding(
            contact,
            Protocol::Tcp,
            target,
            (Ipv4Addr::LOCALHOST, 0).into(),
        )
        .await
        .unwrap();
    let local: SocketAddr = serde_json::from_value(listener["listen"].clone()).unwrap();
    let mut input = TcpStream::connect(local).await.unwrap();
    input.write_all(b"first").await.unwrap();
    let (mut output, _) = timeout(Duration::from_secs(3), sink.accept())
        .await
        .expect("UDP business must not wait for the TCP observation")
        .unwrap();
    let mut first = [0; 5];
    output.read_exact(&mut first).await.unwrap();
    assert_eq!(&first, b"first");
    output.write_all(b"reply").await.unwrap();
    input.read_exact(&mut first).await.unwrap();
    assert_eq!(&first, b"reply");
    start_tcp.notify_one();
    for _ in 0..2 {
        timeout(IO, observations.recv()).await.unwrap().unwrap();
    }
    let original = flow(&consumer, Protocol::Tcp)["transport"].clone();
    assert!(
        original["initial_data_path"]["Ip"].is_string(),
        "{original}"
    );
    timeout(IO, async {
        loop {
            let current = flow(&consumer, Protocol::Tcp)["transport"].clone();
            assert_eq!(current["connection"], original["connection"]);
            assert_eq!(current["stream"], original["stream"]);
            if current["paths"].as_array().unwrap().iter().any(|path| {
                let Ok(iroh::TransportAddr::Custom(route)) =
                    serde_json::from_value(path["remote"].clone())
                else {
                    return false;
                };
                route.id() == miopunch_native_probe::tcp_transport::DATA_TRANSPORT_ID
                    && path["last_ack_age_ms"]
                        .as_u64()
                        .is_some_and(|age| age < 1000)
            }) {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("started TCP must qualify on the same original after UDP delivery");
    let learned = flow(&consumer, Protocol::Tcp)["transport"].clone();
    let unused_tcp = learned["paths"]
        .as_array()
        .unwrap()
        .iter()
        .find(|path| {
            matches!(serde_json::from_value::<iroh::TransportAddr>(path["remote"].clone()),
            Ok(iroh::TransportAddr::Custom(address))
                if address.id() == miopunch_native_probe::tcp_transport::DATA_TRANSPORT_ID)
        })
        .unwrap();
    assert_eq!(
        unused_tcp["selected"], false,
        "TCP remains a verified backup: {learned}"
    );
    assert_eq!(
        unused_tcp["tx_stream_frames"], 0,
        "no business used this candidate: {learned}"
    );
    println!("MIO162 VERIFIED_UNUSED {learned}");
    #[cfg(feature = "nat-lab-test-hooks")]
    {
        let remote: iroh::TransportAddr =
            serde_json::from_value(unused_tcp["remote"].clone()).unwrap();
        let original = consumer
            .test_member_connection(provider.address().id)
            .await
            .unwrap();
        if std::env::var_os("MIO162_CLOSE_ORIGINAL").is_some() {
            original.test_close(0, b"verified backup lifecycle fixture");
            original.closed().await;
        } else {
            original
                .paths()
                .iter()
                .find(|path| path.remote_addr() == &remote)
                .unwrap()
                .close()
                .unwrap();
            if std::env::var_os("MIO162_CLOSE_CLIENT").is_none() {
                timeout(IO, async {
                    while original
                        .paths()
                        .iter()
                        .any(|path| path.remote_addr() == &remote)
                    {
                        sleep(Duration::from_millis(5)).await;
                    }
                })
                .await
                .unwrap();
            }
        }
        if std::env::var_os("MIO162_CLOSE_CLIENT").is_none() {
            timeout(IO, async {
                loop {
                    if read_member_routes(&root.path().join("consumer"), provider.address().id)
                        .iter()
                        .any(|route| {
                            route["address"] == unused_tcp["remote"] && route["verified"] == true
                        })
                    {
                        break;
                    }
                    sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .expect("retired verified backup must be saved while the client remains open");
        }
        println!("MIO162 UNUSED_RETIRED {remote:?}");
    }
    if std::env::var_os("MIO162_CLOSE_ORIGINAL").is_none()
        && std::env::var_os("MIO162_CLOSE_CLIENT").is_none()
    {
        tcp_round_trip(&mut input, &mut output, b"late").await;
    }
    consumer.close().await.unwrap();
    let saved: Vec<_> = read_member_routes(&root.path().join("consumer"), provider.address().id)
        .into_iter()
        .filter(|route| route["verified"] == true && (
            route["address"] == unused_tcp["remote"] || route["address"] == original["initial_data_path"]
        ))
        .collect();
    assert_eq!(saved.len(), 2, "both the original UDP path and unused TCP backup were saved");
    let tcp: iroh::TransportAddr = serde_json::from_value(unused_tcp["remote"].clone()).unwrap();
    if std::env::var_os("MIO162_LIFECYCLE_ONLY").is_some() {
        provider.close().await.unwrap();
        task.abort();
        return;
    }
    // Isolate the two verified inputs for the cached-route replay on multihomed hosts.
    write_member_routes(&root.path().join("consumer"), provider.address().id, saved.clone());
    let reopened = reopen_with_resources(&root.path().join("consumer"), |resources| {
        resources.udp_direct_disabled = true;
    })
    .await;
    first_pong(
        &reopened,
        iroh::EndpointAddr::new(provider.address().id),
        tcp.clone(),
    )
    .await;
    reopened.close().await.unwrap();
    let peer = provider.address().id;
    let udp_address: SocketAddr = saved
        .iter()
        .find_map(|route| {
            serde_json::from_value::<iroh::TransportAddr>(route["address"].clone())
                .ok()
                .and_then(|route| match route {
                    iroh::TransportAddr::Ip(address) => Some(address),
                    _ => None,
                })
        })
        .unwrap();
    let iroh::TransportAddr::Custom(ref tcp_route) = tcp else {
        unreachable!()
    };
    let tcp_address =
        miopunch_native_probe::tcp_transport::TcpTransport::socket_addr(tcp_route).unwrap();
    provider.close().await.unwrap();
    let old_udp = UdpSocket::bind(udp_address).await.unwrap();
    let old_tcp = TcpListener::bind(tcp_address).await.unwrap();
    let relay_server = relay().await;
    let relay_url = relay_server.http_url().unwrap();
    for with_relay in [true, false] {
        let configure = |resources: &mut ResourceConfig| {
            resources.udp_direct_disabled = false;
            if with_relay {
                resources
                    .defaults
                    .resources
                    .push(relay_resource(&relay_url));
            } else {
                resources
                    .defaults
                    .resources
                    .retain(|resource| !resource.uses.contains(&ResourceUse::DataRelay));
            }
        };
        let provider = reopen_with_resources(&root.path().join("provider"), &configure).await;
        // Restore the real, previously validated pair after the last run learned new paths.
        write_member_routes(&root.path().join("consumer"), peer, saved.clone());
        let consumer = reopen_with_resources(&root.path().join("consumer"), &configure).await;
        let mut contact = iroh::EndpointAddr::new(peer);
        if !with_relay {
            contact = provider.invitation().await.unwrap().address().clone();
            contact.addrs.retain(|route| {
                matches!(route, iroh::TransportAddr::Custom(address)
                if address.id() == miopunch_native_probe::mqtt_datagram::TRANSPORT_ID)
            });
        }
        let mut packet = [0; 1500];
        let (reply, tcp_started, udp_started) = timeout(Duration::from_secs(4), async {
            tokio::join!(
                consumer.ping(contact),
                old_tcp.accept(),
                old_udp.recv_from(&mut packet)
            )
        })
        .await
        .expect("both cached protocols must start without holding up relay or restricted contact");
        let reply = reply.unwrap();
        assert_eq!(reply["reply"], "PONG");
        assert_eq!(reply["transport"]["peer"], peer.to_string());
        assert!(tcp_started.is_ok(), "TCP cached candidate was not dialed");
        assert!(
            udp_started.unwrap().0 >= 1200,
            "UDP cached candidate did not send Initial"
        );
        if with_relay {
            assert!(
                reply["transport"]["initial_data_path"]
                    .get("Relay")
                    .is_some(),
                "{reply}"
            );
        }
        println!("MIO162 CACHED_PARALLEL relay={with_relay} {reply}");
        let second = consumer.ping(iroh::EndpointAddr::new(peer)).await.unwrap();
        assert_eq!(
            second["transport"]["connection"],
            reply["transport"]["connection"]
        );
        consumer.close().await.unwrap();
        provider.close().await.unwrap();
        while old_udp.try_recv_from(&mut packet).is_ok() {}
    }
    relay_server.shutdown().await.unwrap();
    task.abort();
}

#[tokio::test]
#[ignore = "requires the isolated two-NAT lab"]
async fn parallel_paths_through_translated_native_clients() {
    super::native_service_nat::restricted_contact_first_reply(true).await;
}

pub(super) async fn lab_first_reply(
    root: &std::path::Path,
    client: &NativeClient,
    role: &str,
    visitor: bool,
    case: &str,
) {
    use super::native_service_nat::wait_file;
    use std::fs;
    let relay_start = case.contains("relay");
    let tcp_first = case.ends_with("tcp");
    fs::write(root.join(format!("port-{role}")), "ready").unwrap();
    wait_file(&root.join("start")).await;
    if visitor {
        wait_file(&root.join("target")).await;
        let (contact, target): (iroh::EndpointAddr, SocketAddr) =
            serde_json::from_slice(&fs::read(root.join("target")).unwrap()).unwrap();
        let listener = client
            .start_forwarding(
                contact,
                Protocol::Tcp,
                target,
                (Ipv4Addr::LOCALHOST, 0).into(),
            )
            .await
            .unwrap();
        let listen: SocketAddr = serde_json::from_value(listener["listen"].clone()).unwrap();
        let mut stream = TcpStream::connect(listen).await.unwrap();
        let started = Instant::now();
        stream.write_all(b"first").await.unwrap();
        let mut bytes = [0; 5];
        timeout(IO, stream.read_exact(&mut bytes))
            .await
            .expect("first available path must carry business")
            .unwrap();
        assert_eq!(&bytes, b"reply");
        let original = flow(client, Protocol::Tcp)["transport"].clone();
        let initial: iroh::TransportAddr =
            serde_json::from_value(original["initial_data_path"].clone()).unwrap();
        if relay_start {
            assert!(initial.is_relay(), "{original}");
        } else if tcp_first {
            assert!(
                matches!(initial, iroh::TransportAddr::Custom(_)),
                "{original}"
            );
        } else {
            assert!(initial.is_ip(), "{original}");
        }
        println!(
            "MIO161 FIRST {}",
            serde_json::json!({"elapsedMs":started.elapsed().as_millis(),"transport":original})
        );
        fs::write(root.join("first"), []).unwrap();
        if !relay_start {
            wait_file(&root.join("release")).await;
        }
        timeout(Duration::from_secs(7), async {
            loop {
                let check_relay = relay_start && root.join("check-relay").exists();
                stream.write_all(b"later").await.unwrap();
                stream.read_exact(&mut bytes).await.unwrap();
                assert_eq!(&bytes, b"reply");
                let current = flow(client, Protocol::Tcp)["transport"].clone();
                assert_eq!(current["connection"], original["connection"]);
                assert_eq!(current["stream"], original["stream"]);
                if check_relay && !root.join("relay-held").exists() {
                    assert!(
                        current["paths"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|path| path["selected"] == true
                                && path["remote"].get("Relay").is_some()),
                        "{current}"
                    );
                    println!("MIO161 RELAY_PENDING {current}");
                    fs::write(root.join("relay-held"), []).unwrap();
                }
                if current["paths"].as_array().unwrap().iter().any(|path| {
                    path["selected"] == true
                        && if tcp_first {
                            matches!(
                                serde_json::from_value(path["remote"].clone()),
                                Ok(iroh::TransportAddr::Custom(_))
                            )
                        } else {
                            path["remote"]["Ip"].is_string()
                        }
                }) {
                    fs::write(root.join("fast"), []).unwrap();
                }
                if both_paths(&current) {
                    println!("MIO161 BOTH {current}");
                    fs::write(root.join("both"), []).unwrap();
                    break;
                }
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("both started mechanisms must qualify on the same stream");
        let steady = Instant::now() + Duration::from_secs(1);
        while Instant::now() < steady {
            stream.write_all(b"later").await.unwrap();
            timeout(IO, stream.read_exact(&mut bytes))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(&bytes, b"reply");
            sleep(Duration::from_millis(20)).await;
        }
        wait_file(&root.join("provider-both")).await;
        let retained = flow(client, Protocol::Tcp)["transport"].clone();
        assert_eq!(retained["connection"], original["connection"]);
        assert_eq!(retained["stream"], original["stream"]);
        assert!(both_paths(&retained), "{retained}");
        println!("MIO161 RETAINED {retained}");
        #[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
        if std::env::var_os("MIO163_BACKUP").is_some() {
            super::native_service_backup::recover(root, client, role, &mut stream, &original).await;
        }
        fs::write(root.join("finished"), []).unwrap();
        wait_file(&root.join("counted")).await;
    } else {
        let sink = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let target = sink.local_addr().unwrap();
        client
            .enable_forwarding(Protocol::Tcp, vec![target])
            .await
            .unwrap();
        let mut contact = client.invitation().await.unwrap().address().clone();
        contact.addrs.retain(|address| {
            if relay_start {
                address.is_relay()
            } else {
                matches!(address, iroh::TransportAddr::Custom(route)
                if route.id() == miopunch_native_probe::mqtt_datagram::TRANSPORT_ID)
            }
        });
        assert_eq!(contact.addrs.len(), 1);
        fs::write(
            root.join("target"),
            serde_json::to_vec(&(&contact, target)).unwrap(),
        )
        .unwrap();
        // The setup worker counts the signal initial only for signaling cells.
        fs::write(root.join("contact"), serde_json::to_vec(&contact).unwrap()).unwrap();
        let (mut stream, _) = timeout(Duration::from_secs(12), sink.accept())
            .await
            .unwrap()
            .unwrap();
        let mut bytes = [0; 5];
        let finished = root.join("finished");
        loop {
            tokio::select! {
                _ = wait_file(&finished) => break,
                read = timeout(Duration::from_secs(10), stream.read_exact(&mut bytes)) => { read.unwrap().unwrap(); }
            }
            assert!(bytes == *b"first" || bytes == *b"later");
            #[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
            if std::env::var_os("MIO163_BACKUP").is_some() {
                super::native_service_backup::pause_acks(root, client, role).await;
            }
            stream.write_all(b"reply").await.unwrap();
            let status = flow(client, Protocol::Tcp)["transport"].clone();
            if both_paths(&status) {
                println!("MIO161 BOTH {status}");
                #[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
                if std::env::var_os("MIO163_BACKUP").is_some()
                    && root.join("backup-committed").exists() {
                    super::native_service_backup::capture(root, client, role).await;
                }
                fs::write(root.join("provider-both"), []).unwrap();
            }
        }
        let status = flow(client, Protocol::Tcp)["transport"].clone();
        if std::env::var_os("MIO163_BACKUP").is_none() {
            assert!(both_paths(&status), "{status}");
        }
        #[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
        if std::env::var_os("MIO163_BACKUP").is_some() {
            super::native_service_backup::finish(root, client, role).await;
        }
        fs::write(root.join("provider-both"), []).unwrap();
        wait_file(&root.join("finished")).await;
    }
}

fn both_paths(transport: &serde_json::Value) -> bool {
    let paths = transport["paths"].as_array().unwrap();
    let fresh = |path: &&serde_json::Value| {
        path["last_ack_age_ms"]
            .as_u64()
            .is_some_and(|age| age < 1000)
    };
    let udp = paths
        .iter()
        .filter(fresh)
        .any(|path| path["remote"]["Ip"].is_string());
    let tcp = paths.iter().filter(fresh).any(|path| matches!(serde_json::from_value(path["remote"].clone()),
        Ok(iroh::TransportAddr::Custom(route)) if route.id() == miopunch_native_probe::tcp_transport::DATA_TRANSPORT_ID));
    udp && tcp
}
