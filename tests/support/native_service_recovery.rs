//! A UDP-started member resumes its original service over newly traversed TCP.
use super::native_service_nat::wait_file;
use super::*;
use std::{fs, path::Path};

pub(super) async fn run(root: &Path, client: &NativeClient, role: &str, visitor: bool) {
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
        exchange(&mut stream).await;
        let original = flow(client, Protocol::Tcp)["transport"].clone();
        assert!(
            original["initial_data_path"].get("Ip").is_some(),
            "{original}"
        );
        println!("MIO164 FIRST {original}");
        fs::write(root.join("first"), []).unwrap();
        // Allow the first, intentionally blocked TCP attempt to reach its own deadline.
        let until = Instant::now() + Duration::from_secs(9);
        while Instant::now() < until {
            exchange(&mut stream).await;
            sleep(Duration::from_millis(100)).await;
        }
        let before = flow(client, Protocol::Tcp)["transport"].clone();
        assert!(
            !before["paths"]
                .as_array()
                .unwrap()
                .iter()
                .any(|path| path["remote"].get("Custom").is_some()),
            "no retained TCP before repair: {before}"
        );
        println!("MIO164 UDP_READY {before}");
        fs::write(root.join("udp-ready"), []).unwrap();
        wait_file(&root.join("fault")).await;
        let started = Instant::now();
        timeout(Duration::from_secs(12), async {
            loop {
                exchange(&mut stream).await;
                let current = flow(client, Protocol::Tcp)["transport"].clone();
                assert_eq!(current["connection"], original["connection"]);
                assert_eq!(current["stream"], original["stream"]);
                if current["paths"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|path| path["selected"] == true && path["remote"].get("Relay").is_some())
                {
                    assert!(
                        current["paths"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|path| path["remote"].get("Ip").is_some()),
                        "failed UDP is still retained"
                    );
                    if !root.join("relay").exists() {
                        println!("MIO164 RELAY {current}");
                        fs::write(root.join("relay"), []).unwrap();
                    }
                }
                if current["paths"].as_array().unwrap().iter().any(|path| {
                    path["selected"] == true
                        && path["remote"].get("Custom").is_some()
                        && path["tx_stream_frames"].as_u64().unwrap() > 0
                }) {
                    assert!(
                        current["paths"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|path| path["remote"] == original["initial_data_path"]),
                        "the failed original UDP path remains listed: {current}"
                    );
                    println!(
                        "MIO164 TCP elapsed_ms={} {current}",
                        started.elapsed().as_millis()
                    );
                    break;
                }
                sleep(Duration::from_millis(30)).await;
            }
        })
        .await
        .expect("new TCP must resume the UDP-started original stream");
        fs::write(root.join("tcp-used"), []).unwrap();
        wait_file(&root.join("tcp-committed")).await;
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
            matches!(address, iroh::TransportAddr::Custom(route)
            if route.id() == miopunch_native_probe::mqtt_datagram::TRANSPORT_ID)
        });
        fs::write(root.join("contact"), serde_json::to_vec(&contact).unwrap()).unwrap();
        fs::write(
            root.join("target"),
            serde_json::to_vec(&(contact, target)).unwrap(),
        )
        .unwrap();
        let (mut stream, _) = timeout(IO, sink.accept()).await.unwrap().unwrap();
        let finished = root.join("finished");
        loop {
            let mut bytes = [0; 5];
            tokio::select! {
                _ = wait_file(&finished) => break,
                result = stream.read_exact(&mut bytes) => { result.unwrap(); }
            }
            assert_eq!(&bytes, b"later");
            stream.write_all(b"reply").await.unwrap();
        }
        println!("MIO164 PEER {}", flow(client, Protocol::Tcp)["transport"]);
    }
}

async fn exchange(stream: &mut TcpStream) {
    stream.write_all(b"later").await.unwrap();
    let mut bytes = [0; 5];
    timeout(IO, stream.read_exact(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&bytes, b"reply");
}
