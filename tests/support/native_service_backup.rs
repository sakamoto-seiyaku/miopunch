//! Same-connection TCP return confirmation in the retained dual-NAT lab.
use super::*;
use iroh::TransportAddr;
use serde_json::{Value, json};

fn tcp(address: &TransportAddr) -> bool {
    matches!(address, TransportAddr::Custom(address)
        if address.id() == miopunch_native_probe::tcp_transport::DATA_TRANSPORT_ID)
}

pub(super) async fn capture(root: &std::path::Path, client: &NativeClient, role: &str) {
    let file = root.join(format!("backup-ready-{role}"));
    if file.exists() {
        return;
    }
    let transport = flow(client, Protocol::Tcp)["transport"].clone();
    let peer = transport["peer"].as_str().unwrap().parse().unwrap();
    let original = client.test_member_connection(peer).await.unwrap();
    let paths = original.paths();
    assert!(
        paths.iter().any(|path| path.is_selected() && path.is_ip()),
        "{transport}"
    );
    let path = paths.iter().find(|path| tcp(path.remote_addr())).unwrap();
    assert!(
        !path.is_selected(),
        "TCP is the existing backup: {transport}"
    );
    let stats = path.stats();
    let snapshot = json!({"transport":transport, "path":path.id().to_string(),
        "remote":path.remote_addr(), "challenge":stats.frame_tx.path_challenge,
        "response":stats.frame_rx.path_response, "stream":stats.frame_rx.stream,
        "sent_stream":stats.frame_tx.stream});
    println!("MIO163 READY {snapshot}");
    std::fs::write(file, serde_json::to_vec(&snapshot).unwrap()).unwrap();
}

pub(super) async fn recover(
    root: &std::path::Path,
    client: &NativeClient,
    role: &str,
    stream: &mut TcpStream,
    original: &Value,
) {
    use super::native_service_nat::wait_file;
    wait_file(&root.join("backup-committed")).await;
    capture(root, client, role).await;
    let mut ready = [0; 5];
    stream.write_all(b"later").await.unwrap();
    timeout(IO, stream.read_exact(&mut ready))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&ready, b"reply");
    wait_file(&root.join("backup-aging")).await;
    pause_acks(root, client, role).await;
    let peer = original["peer"].as_str().unwrap().parse().unwrap();
    let connection = client.test_member_connection(peer).await.unwrap();
    let before: Value =
        serde_json::from_slice(&std::fs::read(root.join(format!("backup-ready-{role}"))).unwrap())
            .unwrap();
    timeout(Duration::from_secs(15), async {
        loop {
            let paths = connection.paths();
            let path = paths.iter().find(|path| tcp(path.remote_addr())).unwrap();
            assert_eq!(path.id().to_string(), before["path"].as_str().unwrap());
            assert!(!path.is_selected());
            let stats = path.stats();
            let window = 2 * Duration::from_secs(1).max(stats.probe_timeout);
            let age = stats.acknowledgment.unwrap().received_at.elapsed();
            if age > window {
                println!(
                    "MIO163 STALE path={} age_ms={} window_ms={}",
                    path.id(),
                    age.as_millis(),
                    window.as_millis()
                );
                break;
            }
            let mut reply = [0; 5];
            stream.write_all(b"later").await.unwrap();
            timeout(IO, stream.read_exact(&mut reply))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(&reply, b"reply");
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("TCP evidence expires while the original UDP stream continues");
    std::fs::write(root.join("backup-stale"), []).unwrap();
    wait_file(&root.join("backup-fault")).await;
    let start = Instant::now();
    let mut reply = [0; 5];
    stream.write_all(b"later").await.unwrap();
    timeout(IO, stream.read_exact(&mut reply))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&reply, b"reply");
    let blocked = std::env::var("MIO163_BACKUP").unwrap() == "blocked";
    if !blocked {
        timeout(IO, async {
            loop {
                let paths = connection.paths();
                let path = paths
                    .iter()
                    .find(|path| path.id().to_string() == before["path"])
                    .unwrap();
                let stats = path.stats();
                if path.is_selected()
                    && stats.frame_tx.path_challenge > before["challenge"].as_u64().unwrap()
                    && stats.frame_rx.path_response > before["response"].as_u64().unwrap()
                {
                    break;
                }
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "existing TCP must obtain a new authenticated return: {:?} tcp={:?}",
                flow(client, Protocol::Tcp),
                connection
                    .paths()
                    .iter()
                    .find(|path| tcp(path.remote_addr()))
                    .map(|path| path.stats())
            )
        });
        stream.write_all(b"later").await.unwrap();
        timeout(IO, stream.read_exact(&mut reply))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&reply, b"reply");
    }
    finish(root, client, role).await;
    println!(
        "MIO163 RECOVERED elapsed_ms={} blocked={blocked}",
        start.elapsed().as_millis()
    );
}

pub(super) async fn pause_acks(root: &std::path::Path, client: &NativeClient, role: &str) {
    let file = root.join(format!("backup-paused-{role}"));
    if !root.join("backup-aging").exists() || file.exists() {
        return;
    }
    let transport = flow(client, Protocol::Tcp)["transport"].clone();
    let peer = transport["peer"].as_str().unwrap().parse().unwrap();
    let original = client.test_member_connection(peer).await.unwrap();
    let paths = original.paths();
    let path = paths.iter().find(|path| tcp(path.remote_addr())).unwrap();
    original.test_suppress_path_acks(
        path.id(),
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
    );
    std::fs::write(file, []).unwrap();
}

pub(super) async fn finish(root: &std::path::Path, client: &NativeClient, role: &str) {
    let before: Value =
        serde_json::from_slice(&std::fs::read(root.join(format!("backup-ready-{role}"))).unwrap())
            .unwrap();
    let transport = flow(client, Protocol::Tcp)["transport"].clone();
    assert_eq!(transport["connection"], before["transport"]["connection"]);
    assert_eq!(transport["stream"], before["transport"]["stream"]);
    let peer = transport["peer"].as_str().unwrap().parse().unwrap();
    let original = client.test_member_connection(peer).await.unwrap();
    let paths = original.paths();
    let path = paths
        .iter()
        .find(|path| path.id().to_string() == before["path"])
        .unwrap();
    assert_eq!(
        serde_json::to_value(path.remote_addr()).unwrap(),
        before["remote"]
    );
    let stats = path.stats();
    if std::env::var("MIO163_BACKUP").unwrap() == "blocked" {
        assert!(!path.is_selected());
        assert_eq!(
            stats.frame_rx.path_response,
            before["response"].as_u64().unwrap()
        );
        assert!(
            paths
                .iter()
                .any(|path| path.is_selected() && path.is_relay())
        );
    } else if std::env::var("MIO150_VISITOR").unwrap() == role {
        assert!(path.is_selected(), "{transport}");
        assert!(stats.frame_tx.stream > before["sent_stream"].as_u64().unwrap());
    } else {
        assert!(stats.frame_rx.stream > before["stream"].as_u64().unwrap());
    }
    println!(
        "MIO163 FINAL {}",
        json!({"transport":transport, "path":path.id().to_string(),
        "challenge":stats.frame_tx.path_challenge, "response":stats.frame_rx.path_response,
        "stream":stats.frame_rx.stream, "sent_stream":stats.frame_tx.stream})
    );
}
