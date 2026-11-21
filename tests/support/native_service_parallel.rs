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
    tcp_round_trip(&mut input, &mut output, b"late").await;
    consumer.close().await.unwrap();
    provider.close().await.unwrap();
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
        if case.contains("relay") {
            assert!(initial.is_relay(), "{original}");
        } else if case.ends_with("tcp") {
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
        wait_file(&root.join("release")).await;
        timeout(Duration::from_secs(7), async {
            loop {
                stream.write_all(b"later").await.unwrap();
                stream.read_exact(&mut bytes).await.unwrap();
                assert_eq!(&bytes, b"reply");
                let current = flow(client, Protocol::Tcp)["transport"].clone();
                assert_eq!(current["connection"], original["connection"]);
                assert_eq!(current["stream"], original["stream"]);
                if current["paths"].as_array().unwrap().iter().any(|path| {
                    path["selected"] == true
                        && if case.ends_with("tcp") {
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
            if case.contains("relay") {
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
            stream.write_all(b"reply").await.unwrap();
            let status = flow(client, Protocol::Tcp)["transport"].clone();
            if both_paths(&status) {
                println!("MIO161 BOTH {status}");
                fs::write(root.join("provider-both"), []).unwrap();
            }
        }
        let status = flow(client, Protocol::Tcp)["transport"].clone();
        assert!(both_paths(&status), "{status}");
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
