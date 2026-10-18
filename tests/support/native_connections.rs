//! Real public Native SDK control and member connections for durable node stories.
use iroh::{
    EndpointAddr, TransportAddr,
    endpoint::{Connection, ControlConnection, RestrictedEndpoint},
};
use std::{sync::Arc, time::Duration};
fn address(endpoint: &RestrictedEndpoint) -> EndpointAddr {
    EndpointAddr::new(endpoint.id())
        .with_addrs(endpoint.addr().ip_addrs().copied().map(TransportAddr::Ip))
}
pub async fn control_pair(
    client: &RestrictedEndpoint,
    server: &RestrictedEndpoint,
    alpn: &[u8],
) -> (Arc<ControlConnection>, Arc<ControlConnection>) {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (client, server) = tokio::join!(client.connect_control(address(server), alpn), async {
            server
                .accept()
                .await
                .unwrap()
                .accept_control()
                .unwrap()
                .await
        });
        (Arc::new(client.unwrap()), Arc::new(server.unwrap()))
    })
    .await
    .expect("bounded real Native control connection")
}
pub async fn member_pair(
    client: &RestrictedEndpoint,
    server: &RestrictedEndpoint,
    alpn: &[u8],
) -> (Connection, Connection) {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (client, server) = tokio::join!(client.connect(address(server), alpn), async {
            server.accept().await.unwrap().await
        });
        (client.unwrap(), server.unwrap())
    })
    .await
    .expect("bounded real Native member connection")
}

/// Send one deliberately chosen complete protocol field in a wire-adversary fixture.
pub async fn send_field(
    exchange: &iroh::endpoint::ControlExchange,
    bytes: &[u8],
) -> std::io::Result<()> {
    exchange
        .send_message(
            iroh::endpoint::ControlMessageFormat::Fixed {
                length: bytes.len(),
            },
            bytes,
        )
        .await
}
/// Read one exact existing protocol field; no byte-stream capability is exposed.
pub async fn receive_field(
    exchange: &iroh::endpoint::ControlExchange,
    bytes: &mut [u8],
) -> std::io::Result<()> {
    let message = exchange
        .recv_message(iroh::endpoint::ControlMessageFormat::Fixed {
            length: bytes.len(),
        })
        .await?;
    bytes.copy_from_slice(&message);
    Ok(())
}
