//! An owned RFC 8489 Binding observer using the accepted TCP socket's address.
use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
};

pub async fn tcp_binding_observer(
    start: Option<Arc<tokio::sync::Notify>>,
    reject_first: usize,
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let observer = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let initial_round = Arc::new(tokio::sync::Semaphore::new(reject_first));
        let mut started = start.is_none();
        let mut bindings = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                _ = async { start.as_ref().unwrap().notified().await }, if !started => started = true,
                accepted = listener.accept(), if started => {
                    let (mut stream, std::net::SocketAddr::V4(peer)) = accepted.unwrap() else { unreachable!() };
                    let reject = initial_round.clone().try_acquire_owned().map(|permit| permit.forget()).is_ok();
                    bindings.spawn(async move {
                        let mut frame = [0; 20];
                        stream.read_exact(&mut frame).await?;
                        assert_eq!(&frame[..8], &[0, 1, 0, 0, 0x21, 0x12, 0xa4, 0x42]);
                        if reject {
                            return Ok(());
                        }
                        frame[..4].copy_from_slice(&[1, 1, 0, 12]);
                        stream.write_all(&frame).await?;
                        stream.write_all(&[0, 0x20, 0, 8, 0, 1]).await?;
                        stream.write_u16(peer.port() ^ 0x2112).await?;
                        stream.write_u32(u32::from(*peer.ip()) ^ 0x2112_a442).await?;
                        let _ = stream.read_u8().await;
                        std::io::Result::Ok(())
                    });
                }
                result = bindings.join_next(), if !bindings.is_empty() => {
                    result.unwrap().unwrap().unwrap();
                }
            }
        }
    });
    (observer, task)
}
