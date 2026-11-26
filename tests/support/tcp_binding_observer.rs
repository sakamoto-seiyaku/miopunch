//! An owned RFC 8489 Binding observer using the accepted TCP socket's address.
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
};

pub async fn tcp_binding_observer(
    start: Option<Arc<tokio::sync::Notify>>,
    reject_first: usize,
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let (address, task, _) = tcp_binding_observer_with_sources(start, reject_first).await;
    (address, task)
}

pub async fn tcp_binding_observer_with_sources(
    start: Option<Arc<tokio::sync::Notify>>,
    reject_first: usize,
) -> (
    SocketAddr,
    tokio::task::JoinHandle<()>,
    tokio::sync::mpsc::UnboundedReceiver<SocketAddr>,
) {
    tcp_binding_observer_on(Ipv4Addr::LOCALHOST.into(), start, reject_first).await
}

pub async fn tcp_binding_observer_on(
    ip: IpAddr,
    start: Option<Arc<tokio::sync::Notify>>,
    reject_first: usize,
) -> (
    SocketAddr,
    tokio::task::JoinHandle<()>,
    tokio::sync::mpsc::UnboundedReceiver<SocketAddr>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind((ip, 0)).await.unwrap();
    let observer = listener.local_addr().unwrap();
    let (sources, observed) = tokio::sync::mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        let initial_round = Arc::new(tokio::sync::Semaphore::new(reject_first));
        let mut started = start.is_none();
        let mut bindings = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                _ = async { start.as_ref().unwrap().notified().await }, if !started => started = true,
                accepted = listener.accept(), if started => {
                    let (mut stream, peer) = accepted.unwrap();
                    let reject = initial_round.clone().try_acquire_owned().map(|permit| permit.forget()).is_ok();
                    let sources = sources.clone();
                    bindings.spawn(async move {
                        let mut frame = [0; 20];
                        stream.read_exact(&mut frame).await?;
                        assert_eq!(&frame[..8], &[0, 1, 0, 0, 0x21, 0x12, 0xa4, 0x42]);
                        let _ = sources.send(peer);
                        if reject {
                            return Ok(());
                        }
                        let size = if peer.is_ipv4() { 8 } else { 20 };
                        frame[..4].copy_from_slice(&[1, 1, 0, size + 4]);
                        stream.write_all(&frame).await?;
                        stream.write_all(&[0, 0x20, 0, size, 0, if peer.is_ipv4() { 1 } else { 2 }]).await?;
                        stream.write_u16(peer.port() ^ 0x2112).await?;
                        match peer.ip() {
                            IpAddr::V4(ip) => stream.write_u32(u32::from(ip) ^ 0x2112_a442).await?,
                            IpAddr::V6(ip) => {
                                let mut bytes = ip.octets();
                                for (index, byte) in bytes.iter_mut().enumerate() { *byte ^= frame[4 + index]; }
                                stream.write_all(&bytes).await?;
                            }
                        }
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
    (observer, task, observed)
}
