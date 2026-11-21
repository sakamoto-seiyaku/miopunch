//! TCP-only first contact through the production client and two NAT gateways.

#[tokio::test]
#[ignore = "requires the isolated two-NAT lab"]
async fn restricted_contact_delivers_the_first_native_reply_through_tcp() {
    super::native_service_nat::restricted_contact_first_reply(true).await;
}

use super::*;

#[cfg(target_os = "windows")]
#[path = "tcp_binding_observer.rs"]
mod tcp_binding_observer;

#[cfg(target_os = "windows")]
#[tokio::test]
async fn windows_default_tcp_delivers_restricted_first_reply_and_honors_disable() {
    use miopunch_native_probe::tcp_transport::TcpTransport;
    let broker = relay_contact_broker::Broker::new().await;
    let (observer, observer_task, mut observations) =
        tcp_binding_observer::tcp_binding_observer_with_sources(None, 0).await;
    for disabled in [false, true] {
        let mut resources = ResourceConfig::with_defaults(ResourceTable {
            revision: 1,
            resources: vec![Resource {
                url: format!("mqtt://{}/", broker.address),
                uses: [ResourceUse::Signaling].into(),
                limitation: "owned protected Windows contact".into(),
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
        resources.udp_direct_disabled = true;
        resources.member_data_relay_disabled = true;
        resources.tcp_direct_disabled = disabled;
        resources.maintenance.contact_timeout_ms = 2000;
        resources.defaults.resources.push(Resource {
            url: format!("stun+tcp://{observer}/"),
            uses: [ResourceUse::Observation].into(),
            limitation: "owned source-port observer".into(),
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
        let mut ports = Vec::new();
        for client in [&provider, &consumer] {
            let addresses = client.address();
            let listener = addresses.addrs.iter().find_map(|address| match address {
                iroh::TransportAddr::Custom(route) => TcpTransport::socket_addr(route).ok(),
                _ => None,
            });
            assert_eq!(listener.is_none(), disabled);
            if let Some(listener) = listener {
                assert!(listener.ip().is_unspecified());
                ports.push(listener.port());
            }
        }
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
        let call = consumer.forward_exchange(contact, Protocol::Tcp, target, b"first request", 11);
        if disabled {
            assert!(
                timeout(Duration::from_secs(4), call)
                    .await
                    .unwrap()
                    .is_err()
            );
            assert!(observations.try_recv().is_err());
            assert!(
                timeout(Duration::from_millis(50), sink.accept())
                    .await
                    .is_err()
            );
        } else {
            let serve = async {
                let (mut socket, _) = timeout(IO, sink.accept()).await.unwrap().unwrap();
                let mut request = [0; 13];
                socket.read_exact(&mut request).await.unwrap();
                assert_eq!(&request, b"first request");
                let mut seen = Vec::new();
                for _ in 0..2 {
                    let source = timeout(IO, observations.recv()).await.unwrap().unwrap();
                    seen.push(source.port());
                }
                seen.sort();
                ports.sort();
                assert_eq!(seen, ports, "STUN must use the actual listening ports");
                for client in [&provider, &consumer] {
                    let status = flow(client, Protocol::Tcp);
                    let route: iroh::TransportAddr =
                        serde_json::from_value(status["transport"]["initial_data_path"].clone())
                            .unwrap();
                    assert!(
                        matches!(route, iroh::TransportAddr::Custom(ref route) if route.id() == miopunch_native_probe::tcp_transport::DATA_TRANSPORT_ID)
                    );
                    println!("MIO160 WINDOWS_ORIGINAL {}", status["transport"]);
                }
                socket.write_all(b"first reply").await.unwrap();
            };
            let (reply, ()) = tokio::join!(call, serve);
            assert_eq!(reply.unwrap(), b"first reply");
        }
        println!("MIO160 WINDOWS_FIRST_REPLY disabled={disabled}");
        consumer.close().await.unwrap();
        provider.close().await.unwrap();
    }
    observer_task.abort();
}

pub(super) async fn simultaneous_first_reply(
    root: &std::path::Path,
    client: &NativeClient,
    role: &str,
) {
    use super::native_service_nat::wait_file;
    let peer = if role == "a" { "b" } else { "a" };
    let sink = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let target = sink.local_addr().unwrap();
    client
        .enable_forwarding(Protocol::Tcp, vec![target])
        .await
        .unwrap();
    wait_file(&root.join(format!("contact-{role}"))).await;
    let contact: iroh::EndpointAddr =
        serde_json::from_slice(&std::fs::read(root.join(format!("contact-{role}"))).unwrap())
            .unwrap();
    std::fs::write(
        root.join(format!("target-{role}")),
        serde_json::to_vec(&(&contact, target)).unwrap(),
    )
    .unwrap();
    wait_file(&root.join(format!("target-{peer}"))).await;
    let (contact, target): (iroh::EndpointAddr, SocketAddr) =
        serde_json::from_slice(&std::fs::read(root.join(format!("target-{peer}"))).unwrap())
            .unwrap();
    std::fs::write(root.join(format!("ready-{role}")), []).unwrap();
    wait_file(&root.join(format!("ready-{peer}"))).await;
    let serve = async {
        let (mut socket, _) = timeout(Duration::from_secs(12), sink.accept())
            .await
            .expect("simultaneous TCP contact must release both business streams")
            .unwrap();
        let mut request = [0; 13];
        socket.read_exact(&mut request).await.unwrap();
        assert_eq!(&request, b"first request");
        std::fs::write(root.join(format!("request-{role}")), []).unwrap();
        for side in ["a", "b"] {
            wait_file(&root.join(format!("examined-{side}"))).await;
        }
        socket.write_all(b"first reply").await.unwrap();
    };
    let inspect = async {
        for side in ["a", "b"] {
            wait_file(&root.join(format!("request-{side}"))).await;
        }
        let flows = client.forwarding_status();
        let flows = flows["flows"].as_array().unwrap();
        assert_eq!(flows.len(), 2, "{flows:?}");
        let transports: Vec<_> = flows.iter().map(|flow| flow["transport"].clone()).collect();
        assert_eq!(transports[0]["connection"], transports[1]["connection"]);
        for transport in &transports {
            let route: iroh::TransportAddr =
                serde_json::from_value(transport["initial_data_path"].clone()).unwrap();
            assert!(matches!(route, iroh::TransportAddr::Custom(ref custom)
                if custom.id() == miopunch_native_probe::tcp_transport::DATA_TRANSPORT_ID));
            assert!(
                transport["paths"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|path| path["selected"] == true
                        && path["last_ack_age_ms"]
                            .as_u64()
                            .is_some_and(|age| age < 1000)
                        && path["rx_stream_frames"]
                            .as_u64()
                            .is_some_and(|frames| frames > 0)),
                "{transport}"
            );
        }
        println!(
            "MIO159 MUTUAL_ORIGINAL {}",
            serde_json::to_string(&transports).unwrap()
        );
        std::fs::write(root.join(format!("examined-{role}")), []).unwrap();
    };
    let (reply, _, _) = tokio::join!(
        client.forward_exchange(contact, Protocol::Tcp, target, b"first request", 11),
        serve,
        inspect
    );
    assert_eq!(reply.unwrap(), b"first reply");
    std::fs::write(root.join(format!("done-{role}")), []).unwrap();
    wait_file(&root.join(format!("done-{peer}"))).await;
    std::fs::write(root.join("finished"), []).unwrap();
    wait_file(&root.join("counted")).await;
}
