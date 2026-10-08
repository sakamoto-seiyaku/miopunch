//! Real persisted SDK members and ordinary streams for negotiated-context checks.
use iroh::endpoint::Connection;
use miopunch_native_probe::{
    admission_gate::ADMISSION_REQUEST_ALPN, manual_admission::ManualExchange,
    native_network::NativeNetwork,
};
use std::{
    net::{Ipv4Addr, SocketAddr},
    time::Duration,
};

pub async fn admitted_pair() -> (tempfile::TempDir, NativeNetwork, NativeNetwork) {
    let dir = tempfile::tempdir().unwrap();
    let a_path = dir.path().join("a");
    let b_path = dir.path().join("b");
    let mut a = NativeNetwork::create(&a_path).await.unwrap();
    let mut b = NativeNetwork::join(&b_path, a.manual_invitation().unwrap())
        .await
        .unwrap();
    for node in [&a, &b] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(vec![]);
        node.save_resource_configuration(&config).unwrap();
    }
    let ae = a.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let be = b.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let deadline = n0_future::time::Instant::now() + Duration::from_secs(5);
    let options = iroh::endpoint::ConnectOptions::new().with_caller_deadline(deadline);
    let (client, server) = tokio::join!(
        async {
            be.connect_with_opts(ae.addr(), ADMISSION_REQUEST_ALPN, options)
                .await
                .unwrap()
                .await
        },
        async {
            ae.accept()
                .await
                .unwrap()
                .with_caller_deadline(deadline)
                .await
        }
    );
    let (client, server) = (client.unwrap(), server.unwrap());
    let (request, served) = tokio::join!(
        b.request_manual_admission(&client),
        a.serve_manual_admission(&server)
    );
    assert!(matches!(request.unwrap(), ManualExchange::AwaitingApproval));
    served.unwrap();
    let request = a.manual_requests().await.unwrap().pop().unwrap();
    a.approve_manual_admission(&request).await.unwrap();
    let receiver = async {
        let ManualExchange::Materials(material) =
            b.request_manual_admission(&client).await.unwrap()
        else {
            panic!("approved materials")
        };
        b.complete_manual_admission(material).await.unwrap();
    };
    let (_, delivered) = tokio::join!(receiver, a.serve_manual_admission(&server));
    delivered.unwrap();
    client.close(0u32.into(), b"admission done");
    server.close(0u32.into(), b"admission done");
    tokio::join!(ae.close(), be.close());
    tokio::join!(a.close(), b.close());
    let a = NativeNetwork::open(&a_path).await.unwrap();
    let b = NativeNetwork::open(&b_path).await.unwrap();
    (dir, a, b)
}

pub async fn streams(a: &Connection, b: &Connection) {
    let send = async {
        let (mut tx, mut rx) = a.open_bi().await.unwrap();
        tx.write_all(b"durable native member request")
            .await
            .unwrap();
        tx.finish().unwrap();
        assert_eq!(
            rx.read_to_end(1024).await.unwrap(),
            b"durable native member response"
        );
    };
    let receive = async {
        let (mut tx, mut rx) = b.accept_bi().await.unwrap();
        assert_eq!(
            rx.read_to_end(1024).await.unwrap(),
            b"durable native member request"
        );
        tx.write_all(b"durable native member response")
            .await
            .unwrap();
        tx.finish().unwrap();
    };
    tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(send, receive);
    })
    .await
    .unwrap();
}

pub fn socket_release(sockets: &[SocketAddr]) -> Vec<bool> {
    sockets
        .iter()
        .map(|address| std::net::UdpSocket::bind(address).is_ok())
        .collect()
}
