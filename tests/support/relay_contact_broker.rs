//! Owned routed MQTT and cold-relay TCP gate for SDK timing regressions.
use std::{
    collections::BTreeMap,
    io,
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Notify, mpsc},
    task::{JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;

type Routes = Arc<Mutex<BTreeMap<String, Vec<(usize, mpsc::Sender<Vec<u8>>)>>>>;
struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
struct Subscriptions {
    id: usize,
    routes: Routes,
}
impl Drop for Subscriptions {
    fn drop(&mut self) {
        self.routes.lock().unwrap().retain(|_, subscribers| {
            subscribers.retain(|(id, _)| *id != self.id);
            !subscribers.is_empty()
        });
    }
}

async fn packet(socket: &mut (impl AsyncRead + Unpin)) -> io::Result<(u8, Vec<u8>)> {
    let kind = socket.read_u8().await?;
    let mut length = 0usize;
    let mut shift = 0;
    loop {
        let byte = socket.read_u8().await?;
        length += usize::from(byte & 127) << shift;
        if length > 8192 || shift > 21 {
            return Err(io::Error::other("owned MQTT packet limit"));
        }
        if byte & 128 == 0 {
            break;
        }
        shift += 7;
    }
    let mut body = vec![0; length];
    socket.read_exact(&mut body).await?;
    Ok((kind, body))
}
fn topic(body: &[u8]) -> io::Result<(String, usize)> {
    let length = body
        .get(..2)
        .map(|b| usize::from(u16::from_be_bytes(b.try_into().unwrap())))
        .ok_or_else(|| io::Error::other("owned topic length"))?;
    let bytes = body
        .get(2..2 + length)
        .ok_or_else(|| io::Error::other("owned topic body"))?;
    Ok((
        std::str::from_utf8(bytes)
            .map_err(io::Error::other)?
            .to_owned(),
        2 + length,
    ))
}
fn publish(topic: &str, bytes: &[u8]) -> Vec<u8> {
    let length = 2 + topic.len() + bytes.len();
    let mut frame = vec![0x30];
    let mut remaining = length;
    loop {
        let mut byte = (remaining % 128) as u8;
        remaining /= 128;
        if remaining != 0 {
            byte |= 128;
        }
        frame.push(byte);
        if remaining == 0 {
            break;
        }
    }
    frame.extend_from_slice(&(topic.len() as u16).to_be_bytes());
    frame.extend_from_slice(topic.as_bytes());
    frame.extend_from_slice(bytes);
    frame
}
async fn serve(
    socket: TcpStream,
    id: usize,
    routes: Routes,
    stop: CancellationToken,
) -> io::Result<()> {
    let (mut reader, mut writer) = socket.into_split();
    let (decoded, mut packets) = mpsc::channel(64);
    // A publish to this connection must never cancel read_exact half-way
    // through another incoming MQTT frame. Only complete packets enter select.
    let read_task = tokio::spawn(async move {
        loop {
            let value = packet(&mut reader).await;
            let failed = value.is_err();
            if decoded.send(value).await.is_err() || failed {
                break;
            }
        }
    });
    let (outgoing, mut incoming) = mpsc::channel::<Vec<u8>>(64);
    let _subscriptions = Subscriptions {
        id,
        routes: routes.clone(),
    };
    let result = async {
        loop {
            let (kind, body) = tokio::select! {
                biased;
                _ = stop.cancelled() => return Ok(()),
                frame = incoming.recv() => { writer.write_all(&frame.ok_or_else(|| io::Error::other("owned subscriber stopped"))?).await?; continue; },
                value = packets.recv() => value.ok_or_else(|| io::Error::other("owned packet reader stopped"))??,
            };
        match kind >> 4 {
            1 => writer.write_all(&[0x20, 2, 0, 0]).await?,
            8 => {
                let identifier = body
                    .get(..2)
                    .ok_or_else(|| io::Error::other("owned subscribe ID"))?;
                let (name, end) = topic(&body[2..])?;
                if body.get(2 + end) != Some(&0) {
                    return Err(io::Error::other("owned fixture requires QoS0"));
                }
                routes
                    .lock()
                    .unwrap()
                    .entry(name)
                    .or_default()
                    .push((id, outgoing.clone()));
                writer
                    .write_all(&[0x90, 3, identifier[0], identifier[1], 0])
                    .await?;
            }
            3 => {
                if kind & 6 != 0 {
                    return Err(io::Error::other("owned publish QoS0 only"));
                }
                let (name, start) = topic(&body)?;
                let subscribers = routes
                    .lock()
                    .unwrap()
                    .get(&name)
                    .cloned()
                    .unwrap_or_default();
                for (_, subscriber) in subscribers {
                    let _ = subscriber.send(publish(&name, &body[start..])).await;
                }
            }
            10 => {
                let identifier = body
                    .get(..2)
                    .ok_or_else(|| io::Error::other("owned unsubscribe ID"))?;
                let (name, _) = topic(&body[2..])?;
                if let Some(subscribers) = routes.lock().unwrap().get_mut(&name) {
                    subscribers.retain(|(owner, _)| *owner != id);
                }
                writer
                    .write_all(&[0xb0, 2, identifier[0], identifier[1]])
                    .await?;
            }
            12 => writer.write_all(&[0xd0, 0]).await?,
            14 => return Ok(()),
            _ => return Err(io::Error::other("unexpected owned MQTT packet")),
        }
        }
    }.await;
    read_task.abort();
    let _ = read_task.await;
    result
}

pub struct Broker {
    pub address: SocketAddr,
    active: Arc<AtomicUsize>,
    stop: CancellationToken,
    task: JoinHandle<()>,
}
impl Broker {
    pub async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let active = Arc::new(AtomicUsize::new(0));
        let stop = CancellationToken::new();
        let active_task = active.clone();
        let stop_task = stop.clone();
        let task = tokio::spawn(async move {
            let routes = Routes::default();
            let mut sessions = JoinSet::new();
            let mut id = 0;
            loop {
                tokio::select! {
                    _ = stop_task.cancelled() => break,
                    connection = listener.accept() => { let (socket, _) = connection.unwrap(); id += 1; let id = id; let routes = routes.clone(); let stop = stop_task.clone(); let active = active_task.clone(); active.fetch_add(1, Ordering::SeqCst); sessions.spawn(async move { let _active = Active(active); let _ = serve(socket, id, routes, stop).await; }); },
                    Some(result) = sessions.join_next() => { result.unwrap(); },
                }
            }
            while let Some(result) = sessions.join_next().await {
                result.unwrap();
            }
        });
        Self {
            address,
            active,
            stop,
            task,
        }
    }
    pub fn active(&self) -> usize {
        self.active.load(Ordering::SeqCst)
    }
    pub async fn close(self) {
        self.stop.cancel();
        self.task.await.unwrap();
    }
}
pub struct RelayGate {
    pub address: SocketAddr,
    hold: Arc<AtomicBool>,
    wake: Arc<Notify>,
    active: Arc<AtomicUsize>,
    stop: CancellationToken,
    task: JoinHandle<()>,
}
impl RelayGate {
    pub async fn new(upstream: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let hold = Arc::new(AtomicBool::new(false));
        let wake = Arc::new(Notify::new());
        let active = Arc::new(AtomicUsize::new(0));
        let stop = CancellationToken::new();
        let hold_task = hold.clone();
        let wake_task = wake.clone();
        let active_task = active.clone();
        let stop_task = stop.clone();
        let task = tokio::spawn(async move {
            let mut sessions = JoinSet::new();
            loop {
                tokio::select! {
                    _ = stop_task.cancelled() => break,
                    connection = listener.accept() => {
                        let (mut incoming, _) = connection.unwrap(); let held = hold_task.load(Ordering::SeqCst); let wake = wake_task.clone(); let hold = hold_task.clone(); let stop = stop_task.clone(); let active = active_task.clone(); active.fetch_add(1, Ordering::SeqCst);
                        sessions.spawn(async move { let _active = Active(active); if held { loop { let notified = wake.notified(); tokio::pin!(notified); notified.as_mut().enable(); if !hold.load(Ordering::SeqCst) { break; } tokio::select! { _ = stop.cancelled() => return, _ = notified => {} } } }
                            let work = async { let mut outgoing = TcpStream::connect(upstream).await?; tokio::io::copy_bidirectional(&mut incoming, &mut outgoing).await.map(|_| ()) };
                            tokio::select! { _ = stop.cancelled() => {}, _ = work => {} }
                        });
                    },
                    Some(result) = sessions.join_next() => { result.unwrap(); },
                }
            }
            while let Some(result) = sessions.join_next().await {
                result.unwrap();
            }
        });
        Self {
            address,
            hold,
            wake,
            active,
            stop,
            task,
        }
    }
    pub fn hold_new(&self) {
        self.hold.store(true, Ordering::SeqCst);
    }
    pub fn release(&self) {
        self.hold.store(false, Ordering::SeqCst);
        self.wake.notify_waiters();
    }
    pub fn active(&self) -> usize {
        self.active.load(Ordering::SeqCst)
    }
    pub async fn close(self) {
        self.stop.cancel();
        self.task.await.unwrap();
    }
}
pub async fn retired(broker: &Broker, gate: &RelayGate) -> (bool, u128, bool) {
    let immediate = broker.active() == 0 && gate.active() == 0;
    let started = tokio::time::Instant::now();
    tokio::time::timeout(Duration::from_secs(1), async {
        while broker.active() != 0 || gate.active() != 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let settled = started.elapsed().as_millis();
    tokio::time::sleep(Duration::from_millis(100)).await;
    (
        immediate,
        settled,
        broker.active() == 0 && gate.active() == 0,
    )
}
