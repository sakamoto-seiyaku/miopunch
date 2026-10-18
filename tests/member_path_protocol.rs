//! Relay validation remains private to the authenticated original M.
//! Strict common member capabilities are exercised by unified_connection.
use iroh::{
    Endpoint,
    endpoint::{Connection, presets},
};
use miopunch_native_probe::member_path_protocol::RelayValidation;
use std::{
    net::{Ipv4Addr, SocketAddr},
    time::Duration,
};
const ALPN: &[u8] = b"mio84/relay-validation-test/1";
async fn endpoint(alpns: Vec<Vec<u8>>) -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .clear_relay_transports()
        .alpns(alpns)
        .bind()
        .await
        .unwrap()
}

async fn pair() -> (Endpoint, Endpoint, Connection, Connection) {
    let client = endpoint(Vec::new()).await;
    let server = endpoint(vec![ALPN.to_vec()]).await;
    let (local, remote) = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(client.connect(server.addr(), ALPN), async {
            server.accept().await.unwrap().await
        })
    })
    .await
    .unwrap();
    (client, server, local.unwrap(), remote.unwrap())
}

#[tokio::test]
async fn private_relay_exchange_confirms_both_sides_on_original_m() {
    let (a, b, local, remote) = pair().await;
    let before = (local.stable_id(), remote.stable_id());
    let left = vec!["https://relay-a.test/".parse().unwrap()];
    let right = vec!["https://relay-b.test/".parse().unwrap()];
    let (left_exchange, right_exchange) = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(
            RelayValidation::begin(&local, left.clone()),
            RelayValidation::begin(&remote, right.clone())
        )
    })
    .await
    .unwrap();
    let (left_exchange, right_exchange) = (left_exchange.unwrap(), right_exchange.unwrap());
    assert_eq!(left_exchange.candidates, right);
    assert_eq!(right_exchange.candidates, left);
    let (left_result, right_result) = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(
            left_exchange.finish(&local, Ok(())),
            right_exchange.finish(&remote, Ok(()))
        )
    })
    .await
    .unwrap();
    for result in [left_result, right_result] {
        let continuity = result.unwrap();
        assert!(continuity.stable_unchanged && continuity.binding_unchanged);
    }
    assert_eq!((local.stable_id(), remote.stable_id()), before);
    tokio::join!(a.close(), b.close());
}

#[tokio::test]
async fn failed_peer_validation_never_becomes_bilateral_success() {
    let (a, b, local, remote) = pair().await;
    let (left, right) = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(
            RelayValidation::begin(&local, vec![]),
            RelayValidation::begin(&remote, vec![])
        )
    })
    .await
    .unwrap();
    let (left, right) = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(
            left.unwrap().finish(&local, Ok(())),
            right.unwrap().finish(
                &remote,
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "candidate revoked"
                ))
            )
        )
    })
    .await
    .unwrap();
    assert!(left.is_err() && right.is_err());
    tokio::join!(a.close(), b.close());
}

#[tokio::test]
async fn five_peer_relay_candidates_are_rejected_before_path_preparation() {
    let (a, b, local, remote) = pair().await;
    let read = RelayValidation::begin(&local, vec![]);
    let write = async {
        let (mut send, mut recv) = remote.accept_bi().await.unwrap();
        let encoded = serde_json::to_vec(&vec!["https://relay.test/"; 5]).unwrap();
        send.write_all(&(encoded.len() as u16).to_be_bytes())
            .await
            .unwrap();
        send.write_all(&encoded).await.unwrap();
        send.finish().unwrap();
        let mut prefix = [0; 2];
        recv.read_exact(&mut prefix).await.unwrap();
        assert_eq!(u16::from_be_bytes(prefix), 2);
        assert_eq!(recv.read_to_end(2).await.unwrap(), b"[]");
    };
    let (result, _) =
        tokio::time::timeout(Duration::from_secs(3), async { tokio::join!(read, write) })
            .await
            .unwrap();
    assert!(result.is_err());
    tokio::join!(a.close(), b.close());
}
