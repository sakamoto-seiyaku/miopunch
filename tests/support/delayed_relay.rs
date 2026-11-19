use std::{net::Ipv4Addr, time::Duration};

// Delay each relay TCP leg independently, leaving the relay protocol and
// encrypted QUIC packets intact. The reader keeps collecting bytes during delay.
pub async fn delay_relay(
    target: std::net::SocketAddr,
    delay: Duration,
) -> (iroh::RelayUrl, tokio_util::sync::CancellationToken) {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let stop = tokio_util::sync::CancellationToken::new();
    let cancelled = stop.clone();
    tokio::spawn(async move {
        let mut circuits = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                _ = cancelled.cancelled() => break,
                accepted = listener.accept() => {
                    let (peer, _) = accepted.unwrap();
                    circuits.spawn(async move {
                        let relay = tokio::net::TcpStream::connect(target).await?;
                        peer.set_nodelay(true)?;
                        relay.set_nodelay(true)?;
                        let (peer_read, peer_write) = peer.into_split();
                        let (relay_read, relay_write) = relay.into_split();
                        tokio::select! {
                            result = delay_relay_leg(peer_read, relay_write, delay) => result,
                            result = delay_relay_leg(relay_read, peer_write, delay) => result,
                        }
                    });
                }
                _ = circuits.join_next(), if !circuits.is_empty() => {}
            }
        }
    });
    (format!("http://{address}").parse().unwrap(), stop)
}

async fn delay_relay_leg(
    mut read: tokio::net::tcp::OwnedReadHalf,
    mut write: tokio::net::tcp::OwnedWriteHalf,
    delay: Duration,
) -> std::io::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (send, mut receive) = tokio::sync::mpsc::channel::<(tokio::time::Instant, Vec<u8>)>(128);
    let reader = async {
        let mut buffer = [0; 4096];
        loop {
            let len = read.read(&mut buffer).await?;
            if len == 0
                || send
                    .send((tokio::time::Instant::now() + delay, buffer[..len].to_vec()))
                    .await
                    .is_err()
            {
                return Ok(());
            }
        }
    };
    let writer = async {
        while let Some((ready, bytes)) = receive.recv().await {
            tokio::time::sleep_until(ready).await;
            write.write_all(&bytes).await?;
        }
        Ok(())
    };
    tokio::select! {
        result = reader => result,
        result = writer => result,
    }
}
