// Fixed ordinary consumer: compile and run the identical bytes on official iroh and the fork.
use std::{net::{Ipv4Addr, SocketAddr}, sync::Arc, time::Duration};
use iroh::{Endpoint, EndpointAddr, endpoint::{Connection, EndpointHooks, PortmapperConfig, presets}};
const ALPN: &[u8] = b"test/ordinary-compatibility/1";

use iroh::endpoint::{Accepting, AfterHandshakeOutcome, IncomingZeroRttConnection};

fn incoming(accepting: Accepting) -> IncomingZeroRttConnection {
    accepting.into_0rtt()
}

fn outcome(result: AfterHandshakeOutcome) -> bool {
    match result {
        AfterHandshakeOutcome::Accept => true,
        AfterHandshakeOutcome::Reject { .. } => false,
    }
}

fn original_compile_contracts() {
    let _ = incoming;
    assert!(outcome(AfterHandshakeOutcome::Accept));
}

#[derive(Debug)]
struct OrdinaryHandshakeCount(Arc<std::sync::atomic::AtomicUsize>);
impl EndpointHooks for OrdinaryHandshakeCount {
    async fn after_handshake(&self, _connection: &Connection) -> AfterHandshakeOutcome {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        AfterHandshakeOutcome::Accept
    }
}

async fn ordinary_early_data() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let server = Endpoint::builder(presets::Minimal)
        .portmapper_config(PortmapperConfig::Disabled)
        .alpns(vec![ALPN.to_vec()])
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .hooks(OrdinaryHandshakeCount(calls.clone()))
        .bind()
        .await
        .unwrap();
    let client = Endpoint::builder(presets::Minimal)
        .portmapper_config(PortmapperConfig::Disabled)
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .bind()
        .await
        .unwrap();
    let target = EndpointAddr::new(server.id()).with_addrs(
        server
            .addr()
            .ip_addrs()
            .copied()
            .map(iroh_base::TransportAddr::Ip),
    );
    let receiver = server.clone();
    let server_calls = calls.clone();
    let mut task = tokio::spawn(async move {
        let mut rounds = Vec::new();
        for _ in 0..2 {
            let early = receiver
                .accept()
                .await
                .unwrap()
                .accept()
                .unwrap()
                .into_0rtt();
            let (mut send, mut recv) = early.accept_bi().await.unwrap();
            let is_zero_rtt = recv.is_0rtt();
            let message = recv.read_to_end(64).await.unwrap();
            send.write_all(&message).await.unwrap();
            send.finish().unwrap();
            let before = server_calls.load(std::sync::atomic::Ordering::SeqCst);
            let connection = early.handshake_completed().await.unwrap();
            let after = server_calls.load(std::sync::atomic::Ordering::SeqCst);
            rounds.push((is_zero_rtt, before, after, message));
            connection.closed().await;
        }
        rounds
    });
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        let first = client
            .connect_with_opts(target.clone(), ALPN, Default::default())
            .await
            .unwrap()
            .into_0rtt()
            .expect_err("fresh client has no resumption ticket")
            .await
            .unwrap();
        let (mut send, mut recv) = first.open_bi().await.unwrap();
        send.write_all(b"ordinary-first").await.unwrap();
        send.finish().unwrap();
        let first_reply = recv.read_to_end(64).await.unwrap();
        first.close(0u32.into(), b"first complete");
        let early = client
            .connect_with_opts(target, ALPN, Default::default())
            .await
            .unwrap()
            .into_0rtt()
            .expect("original ordinary resumption remains enabled");
        let (mut send, mut recv) = early.open_bi().await.unwrap();
        send.write_all(b"ordinary-zero-rtt").await.unwrap();
        send.finish().unwrap();
        let status = early.handshake_completed().await.unwrap();
        let accepted = matches!(&status, iroh::endpoint::ZeroRttStatus::Accepted(_));
        let connection = match status {
            iroh::endpoint::ZeroRttStatus::Accepted(connection)
            | iroh::endpoint::ZeroRttStatus::Rejected(connection) => connection,
        };
        let resumed_reply = recv.read_to_end(64).await.unwrap();
        connection.close(0u32.into(), b"resumed complete");
        (first, connection, accepted, first_reply, resumed_reply)
    })
    .await;
    client.close().await;
    server.close().await;
    let served = tokio::time::timeout(Duration::from_secs(2), &mut task).await;
    if served.is_err() {
        task.abort();
        let _ = task.await;
    }
    let (_, _, accepted, first_reply, resumed_reply) =
        result.expect("ordinary early-data exchange is bounded");
    assert!(accepted);
    assert_eq!(first_reply, b"ordinary-first");
    assert_eq!(resumed_reply, b"ordinary-zero-rtt");
    let rounds = served.expect("ordinary server completed").unwrap();
    assert_eq!(rounds.len(), 2);
    assert_eq!((rounds[0].0, rounds[0].1, rounds[0].2), (false, 0, 1));
    assert_eq!((rounds[1].0, rounds[1].1, rounds[1].2), (true, 1, 2));
}

// Ordinary public struct literals and the original required custom transport method.
#[derive(Debug)]
struct ExistingTransport;
impl iroh::endpoint::transports::CustomTransport for ExistingTransport {
    fn bind(&self) -> std::io::Result<Box<dyn iroh::endpoint::transports::CustomEndpoint>> {
        Err(std::io::ErrorKind::Unsupported.into())
    }
}
fn main() {
    original_compile_contracts();
    let _ = iroh::endpoint::HandshakeCompleted {};
    let _ = iroh::endpoint::IncomingZeroRtt {};
    let _ = iroh::endpoint::OutgoingZeroRtt {};
    let _ = ExistingTransport;
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(ordinary_early_data());
    println!("ORDINARY_EARLY_DATA_PASS direct_incoming=true fallible_outgoing=true resumed_0rtt=true hooks=true natural_close=true");
}
