//! Minimal owned MQTT fault boundary shared by listener lifecycle tests.
use std::{
    io,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
    task::{JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;
// Deliberately small owned MQTT fault boundary. Actual rumqttc and the
// production carrier process the packets. This is not public-facility,
// end-to-end QUIC, identity or application-session qualification.
pub struct Broker {
    pub address: SocketAddr,
    pub connections: Arc<AtomicUsize>,
    active: Arc<AtomicUsize>,
    cut: watch::Sender<u64>,
    stop: CancellationToken,
    task: JoinHandle<()>,
}
pub struct SocketRetirement {
    pub immediate_empty: bool,
    pub settled_ms: u128,
    pub late_empty: bool,
}
struct ActiveSocket(Arc<AtomicUsize>);
impl Drop for ActiveSocket {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
async fn packet(socket: &mut TcpStream) -> io::Result<(u8, Vec<u8>)> {
    let kind = socket.read_u8().await?;
    let mut length = 0usize;
    let mut shift = 0;
    loop {
        let byte = socket.read_u8().await?;
        length += usize::from(byte & 127) << shift;
        if length > 4096 || shift > 21 {
            return Err(io::Error::other("owned MQTT packet exceeds limit"));
        }
        if byte & 128 == 0 {
            break;
        }
        shift += 7;
    }
    let mut body = vec![0; length];
    socket.read_exact(&mut body).await?;
    Ok((kind >> 4, body))
}
async fn serve(
    mut socket: TcpStream,
    mut cut: watch::Receiver<u64>,
    stop: CancellationToken,
) -> io::Result<()> {
    loop {
        let (kind, body) = tokio::select! {
            biased;
            _ = stop.cancelled() => return Ok(()),
            _ = cut.changed() => return Ok(()),
            value = packet(&mut socket) => value?,
        };
        match kind {
            1 => socket.write_all(&[0x20, 2, 0, 0]).await?,
            8 => {
                if body.len() < 2 {
                    return Err(io::Error::other("owned subscribe missing identifier"));
                }
                socket.write_all(&[0x90, 3, body[0], body[1], 0]).await?;
            }
            10 => {
                if body.len() < 2 {
                    return Err(io::Error::other("owned unsubscribe missing identifier"));
                }
                socket.write_all(&[0xb0, 2, body[0], body[1]]).await?;
            }
            12 => socket.write_all(&[0xd0, 0]).await?,
            14 => return Ok(()),
            _ => {}
        }
    }
}
impl Broker {
    pub async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let connections = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let stop = CancellationToken::new();
        let (cut, _) = watch::channel(0);
        let task_connections = connections.clone();
        let task_active = active.clone();
        let task_stop = stop.clone();
        let task_cut = cut.clone();
        let task = tokio::spawn(async move {
            let mut sessions = JoinSet::new();
            loop {
                tokio::select! {
                    biased;
                    _ = task_stop.cancelled() => break,
                    value = listener.accept() => {
                        let (socket, _) = value.unwrap();
                        task_connections.fetch_add(1, Ordering::SeqCst);
                        task_active.fetch_add(1, Ordering::SeqCst);
                        let active = task_active.clone();
                        let stop = task_stop.clone();
                        let cut = task_cut.subscribe();
                        sessions.spawn(async move {
                            let _active = ActiveSocket(active);
                            let _ = serve(socket, cut, stop).await;
                        });
                    }
                    value = sessions.join_next(), if !sessions.is_empty() => { value.unwrap().unwrap(); }
                }
            }
            while let Some(value) = sessions.join_next().await {
                value.unwrap();
            }
        });
        Self {
            address,
            connections,
            active,
            cut,
            stop,
            task,
        }
    }
    pub fn disconnect(&self) {
        self.cut.send_modify(|generation| *generation += 1);
    }
    pub async fn no_sockets(&self) -> SocketRetirement {
        let immediate_empty = self.active.load(Ordering::SeqCst) == 0;
        let started = tokio::time::Instant::now();
        tokio::time::timeout(Duration::from_secs(1), async {
            while self.active.load(Ordering::SeqCst) != 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(self.active.load(Ordering::SeqCst), 0);
        let settled_ms = started.elapsed().as_millis();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(self.active.load(Ordering::SeqCst), 0);
        SocketRetirement {
            immediate_empty,
            settled_ms,
            late_empty: true,
        }
    }
    pub async fn close(self) {
        self.stop.cancel();
        self.task.await.unwrap();
    }
}
