//! Production first contact through two translated native hosts.

use std::{env, fs, future::pending, path::Path};

use super::*;

#[path = "stun_observer.rs"]
mod stun_observer;

const WAN: Ipv4Addr = Ipv4Addr::new(100, 64, 27, 1);
const PRIVATE: [Ipv4Addr; 2] = [Ipv4Addr::new(10, 27, 1, 2), Ipv4Addr::new(10, 27, 2, 2)];
const OBSERVERS: [u16; 2] = [46761, 46762];

#[tokio::test]
#[ignore = "requires the isolated two-NAT lab"]
async fn restricted_contact_delivers_the_first_native_reply_through_enhanced_udp() {
    restricted_contact_first_reply(false).await;
}

pub(super) async fn restricted_contact_first_reply(tcp: bool) {
    let role = env::var("MIO150_WORKER").unwrap();
    let root = env::var("MIO150_ROOT").unwrap();
    let root = Path::new(&root);
    fs::create_dir_all(root).unwrap();
    let case = env::var("MIO150_CASE").unwrap();
    if role == "setup" {
        let broker = relay_contact_broker::Broker::bind((WAN, 0).into()).await;
        let mut resources = ResourceConfig::with_defaults(ResourceTable {
            revision: 1,
            resources: vec![Resource {
                url: format!("mqtt://{}/", broker.address),
                uses: [ResourceUse::Signaling].into(),
                limitation: "isolated contact without a data relay".into(),
            }],
        });
        resources.member_data_relay_disabled = true;
        resources.tcp_direct_disabled = !tcp;
        let Pair {
            root: pair_dir,
            consumer,
            provider,
            ..
        } = Pair::admit_with_resources(resources.clone()).await;
        consumer.close().await.unwrap();
        provider.close().await.unwrap();
        resources.udp_direct_disabled = tcp;
        if tcp && case == "disabled" {
            resources.tcp_direct_disabled = true;
        }
        if tcp && case == "observer-denied" {
            resources.disabled_uses.insert(ResourceUse::Observation);
        }
        if tcp {
            resources.defaults.resources.push(Resource {
                url: format!("stun+tcp://{WAN}:3478/"),
                uses: [ResourceUse::Observation].into(),
                limitation: "owned TCP observer for the actual listener".into(),
            });
        } else {
            resources
                .defaults
                .resources
                .extend(OBSERVERS.map(|port| Resource {
                    url: format!("stun://{WAN}:{port}/"),
                    uses: [ResourceUse::Observation].into(),
                    limitation: "independent UDP mapping destination".into(),
                }));
        }
        resources.maintenance.contact_timeout_ms = 8_000;
        for name in ["provider", "consumer"] {
            let origin = pair_dir.path().join(name);
            let node = NativeNetwork::open(&origin).await.unwrap();
            let mut saved = resources.clone();
            if tcp && case == "target-denied" {
                let side = usize::from(
                    (name == "consumer") != (env::var("MIO150_VISITOR").unwrap() == "a"),
                );
                saved.tcp_listen = Some((Ipv4Addr::UNSPECIFIED, [42027, 43027][side]).into());
                saved.disabled_tcp_addresses.insert(
                    (
                        [Ipv4Addr::new(100, 64, 27, 2), Ipv4Addr::new(100, 64, 27, 6)][1 - side],
                        [42027, 43027][1 - side],
                    )
                        .into(),
                );
            }
            node.save_resource_configuration(&saved).unwrap();
            node.close().await;
            fs::rename(origin, root.join(name)).unwrap();
        }
        println!("MIO150 LAB_READY");
        tokio::join!(
            async {
                if tcp {
                    pending::<()>().await;
                }
                let a = UdpSocket::bind((WAN, OBSERVERS[0])).await.unwrap();
                loop {
                    stun_observer::reply_one_stun(&a).await;
                }
            },
            async {
                if tcp {
                    pending::<()>().await;
                }
                let b = UdpSocket::bind((WAN, OBSERVERS[1])).await.unwrap();
                loop {
                    stun_observer::reply_one_stun(&b).await;
                }
            },
            async {
                wait_file(&root.join("finished")).await;
                let contact: iroh::EndpointAddr =
                    serde_json::from_slice(&fs::read(root.join("contact")).unwrap()).unwrap();
                let iroh::TransportAddr::Custom(route) = contact.addrs.first().unwrap() else {
                    panic!("MQTT route");
                };
                let dials = broker.initial_dials_to(route);
                println!("MIO155 MQTT_INITIALS {dials}");
                assert_eq!(dials, 1);
                fs::write(root.join("counted"), []).unwrap();
                pending::<()>().await;
            },
        );
        return;
    }

    let visitor = env::var("MIO150_VISITOR").unwrap() == role;
    let ip = PRIVATE[usize::from(role == "b")];
    let name = if visitor { "consumer" } else { "provider" };
    let blocked = if tcp {
        case != "positive"
    } else {
        case.ends_with("blocked")
    };
    let client = NativeClient::open(root.join(name), ip).await.unwrap();
    let address = client.address();
    assert!(address.relay_urls().next().is_none());
    let port = if tcp && case == "disabled" {
        assert!(!address.addrs.iter().any(|address| matches!(address,
            iroh::TransportAddr::Custom(route) if route.id() == miopunch_native_probe::tcp_transport::DATA_TRANSPORT_ID)));
        0
    } else if tcp {
        let listener = address
            .addrs
            .iter()
            .find_map(|address| {
                let iroh::TransportAddr::Custom(route) = address else {
                    return None;
                };
                miopunch_native_probe::tcp_transport::TcpTransport::socket_addr(route).ok()
            })
            .expect("default Linux TCP listener");
        assert!(listener.ip().is_unspecified());
        assert_ne!(listener.port(), 0);
        listener.port()
    } else {
        address
            .ip_addrs()
            .find(|address| address.ip() == ip)
            .unwrap()
            .port()
    };
    fs::write(root.join(format!("port-{role}")), port.to_string()).unwrap();
    wait_file(&root.join("start")).await;
    if visitor {
        wait_file(&root.join("contact")).await;
        let (contact, target): (iroh::EndpointAddr, SocketAddr) =
            serde_json::from_slice(&fs::read(root.join("target")).unwrap()).unwrap();
        let started = Instant::now();
        let call = client.forward_exchange(contact, Protocol::Tcp, target, b"first request", 11);
        if blocked {
            let reply = call.await;
            assert!(reply.is_err(), "{reply:?}");
            assert!(started.elapsed() < Duration::from_secs(10));
        } else {
            let request = root.join("request");
            tokio::pin!(call);
            tokio::select! {
                reply = &mut call => panic!("business ended before the first reply: {reply:?}"),
                _ = wait_file(&request) => {},
            }
            report_original("CONSUMER", &client, tcp);
            fs::write(root.join("examined"), []).unwrap();
            assert_eq!(call.await.unwrap(), b"first reply");
        }
        println!(
            "MIO155 FIRST_REPLY success={} elapsed_ms={}",
            !blocked,
            started.elapsed().as_millis()
        );
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
        contact.addrs.retain(|address| matches!(address,
            iroh::TransportAddr::Custom(route) if route.id() == miopunch_native_probe::mqtt_datagram::TRANSPORT_ID));
        assert_eq!(contact.addrs.len(), 1);
        fs::write(
            root.join("target"),
            serde_json::to_vec(&(&contact, target)).unwrap(),
        )
        .unwrap();
        fs::write(root.join("contact"), serde_json::to_vec(&contact).unwrap()).unwrap();
        if blocked {
            let finished = root.join("finished");
            tokio::select! {
                _ = wait_file(&finished) => {},
                result = sink.accept() => panic!("business escaped restricted contact: {result:?}"),
            }
        } else {
            let (mut socket, _) = timeout(Duration::from_secs(12), sink.accept())
                .await
                .expect("enhanced first contact must release the original forwarding stream")
                .unwrap();
            let mut request = [0; 13];
            socket.read_exact(&mut request).await.unwrap();
            assert_eq!(&request, b"first request");
            report_original("PROVIDER", &client, tcp);
            fs::write(root.join("request"), []).unwrap();
            wait_file(&root.join("examined")).await;
            socket.write_all(b"first reply").await.unwrap();
            wait_file(&root.join("finished")).await;
        }
    }
    client.close().await.unwrap();
}

fn report_original(side: &str, client: &NativeClient, tcp: bool) {
    let status = flow(client, Protocol::Tcp);
    let transport = &status["transport"];
    if tcp {
        let path: iroh::TransportAddr =
            serde_json::from_value(transport["initial_data_path"].clone()).unwrap();
        let iroh::TransportAddr::Custom(route) = path else {
            panic!("TCP first data path: {status}");
        };
        let peer = miopunch_native_probe::tcp_transport::TcpTransport::socket_addr(&route)
            .expect("TCP data route");
        assert!(["100.64.27.2", "100.64.27.6"].contains(&peer.ip().to_string().as_str()));
        assert!(
            transport["paths"]
                .as_array()
                .unwrap()
                .iter()
                .any(|candidate| candidate["selected"] == true
                    && candidate["remote"] == transport["initial_data_path"]
                    && candidate["last_ack_age_ms"]
                        .as_u64()
                        .is_some_and(|age| age < 1000)
                    && candidate["rx_stream_frames"]
                        .as_u64()
                        .is_some_and(|frames| frames > 0)),
            "{status}"
        );
        println!("MIO155 {side}_ORIGINAL {transport}");
        return;
    }
    assert!(transport["initial_data_path"]["Ip"].is_string(), "{status}");
    assert!(
        transport["paths"].as_array().unwrap().iter().any(
            |path| path["selected"] == true && path["remote"] == transport["initial_data_path"]
        ),
        "{status}"
    );
    let enhanced = &transport["enhanced_udp"];
    assert_eq!(enhanced["connection"], transport["connection"], "{status}");
    assert_eq!(enhanced["fresh_ack"], true, "{status}");
    assert_eq!(enhanced["selected_round"], true, "{status}");
    assert_eq!(enhanced["learned"], false, "{status}");
    assert_eq!(
        enhanced["peer"], transport["initial_data_path"]["Ip"],
        "{status}"
    );
    assert!(
        transport["paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path["selected"] == true && path["id"] == enhanced["path"]),
        "{status}"
    );
    println!("MIO155 {side}_ORIGINAL {transport}");
}

async fn wait_file(path: &Path) {
    timeout(Duration::from_secs(18), async {
        while !path.exists() {
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("missing lab signal {}", path.display()));
}
