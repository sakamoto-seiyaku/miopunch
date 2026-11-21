//! TCP-only first contact through the production client and two NAT gateways.

#[tokio::test]
#[ignore = "requires the isolated two-NAT lab"]
async fn restricted_contact_delivers_the_first_native_reply_through_tcp() {
    super::native_service_nat::restricted_contact_first_reply(true).await;
}

use super::*;

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
