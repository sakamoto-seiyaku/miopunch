//! Owned loopback MQTT broker: forwards actual QoS0 datagrams, records no payloads.
use super::member_contact_observation::Events;
use std::{
    collections::BTreeMap,
    io,
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
    task::{JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;

type Topics = Arc<Mutex<BTreeMap<Vec<u8>, mpsc::Sender<Vec<u8>>>>>;
pub struct Broker {
    pub address: SocketAddr,
    pub active: Arc<AtomicUsize>,
    topics: Topics,
    stop: CancellationToken,
    task: JoinHandle<()>,
}
struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
async fn packet(socket: &mut (impl AsyncRead + Unpin)) -> io::Result<(u8, Vec<u8>)> {
    let kind = socket.read_u8().await?;
    let (mut length, mut shift) = (0, 0);
    loop {
        let byte = socket.read_u8().await?;
        length += usize::from(byte & 127) << shift;
        if length > 8192 || shift > 21 {
            return Err(io::Error::other("owned packet exceeds limit"));
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
fn wire(kind: u8, body: &[u8]) -> Vec<u8> {
    let mut result = vec![kind];
    let mut length = body.len();
    loop {
        let byte = (length % 128) as u8;
        length /= 128;
        result.push(byte | if length > 0 { 128 } else { 0 });
        if length == 0 {
            break;
        }
    }
    result.extend(body);
    result
}
fn topic(body: &[u8], offset: usize) -> io::Result<&[u8]> {
    let size = body
        .get(offset..offset + 2)
        .ok_or_else(|| io::Error::other("missing topic length"))?;
    let size = u16::from_be_bytes(size.try_into().unwrap()) as usize;
    body.get(offset + 2..offset + 2 + size)
        .ok_or_else(|| io::Error::other("missing topic"))
}
impl Broker {
    pub async fn new(events: Events) -> Self {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let active = Arc::new(AtomicUsize::new(0));
        let topics: Topics = Arc::default();
        let stop = CancellationToken::new();
        let task_active = active.clone();
        let task_topics = topics.clone();
        let task_stop = stop.clone();
        let task = tokio::spawn(async move {
            let mut sessions = JoinSet::new();
            loop {
                tokio::select! {
                    biased;
                    _ = task_stop.cancelled() => break,
                    value = listener.accept() => {
                        let (socket, _) = value.unwrap();
                        task_active.fetch_add(1, Ordering::SeqCst);
                        let active = task_active.clone();
                        let topics = task_topics.clone();
                        let stop = task_stop.clone();
                        let events = events.clone();
                        sessions.spawn(async move {
                            let _active = Active(active);
                            let (mut read, mut write) = socket.into_split();
                            let (sender, mut receiver) = mpsc::channel::<Vec<u8>>(64);
                            let (input, mut packets) = mpsc::channel(64);
                            let session_stop = CancellationToken::new();
                            let reader_stop = session_stop.clone();
                            // A packet read must not be cancelled when an outbound
                            // forward becomes ready: it may already own header bytes.
                            let reader = tokio::spawn(async move {
                                loop {
                                    let value = tokio::select! {
                                        _ = reader_stop.cancelled() => break,
                                        value = packet(&mut read) => value,
                                    };
                                    let ended = value.is_err();
                                    let sent = tokio::select! {
                                        _ = reader_stop.cancelled() => break,
                                        sent = input.send(value) => sent,
                                    };
                                    if ended || sent.is_err() { break; }
                                }
                            });
                            let mut own_topics = vec![];
                            let result = async {
                                loop {
                                    tokio::select! {
                                        biased;
                                        _ = stop.cancelled() => return Ok::<_, io::Error>(()),
                                        Some(value) = receiver.recv() => write.write_all(&value).await?,
                                        value = packets.recv() => {
                                            let (kind, body) = value.ok_or_else(|| io::Error::other("owned reader ended"))??;
                                            match kind >> 4 {
                                                1 => write.write_all(&[0x20,2,0,0]).await?,
                                                8 => {
                                                    let destination = topic(&body, 2)?.to_vec();
                                                    if body.last() != Some(&0) { return Err(io::Error::other("only owned QoS0 subscription supported")); }
                                                    topics.lock().unwrap().insert(destination.clone(), sender.clone());
                                                    own_topics.push(destination);
                                                    write.write_all(&[0x90,3,body[0],body[1],0]).await?;
                                                }
                                                3 => {
                                                    if kind != 0x30 { return Err(io::Error::other("only owned QoS0 publish supported")); }
                                                    let destination = topic(&body, 0)?;
                                                    let payload = &body[2 + destination.len()..];
                                                    events.record("mqttPublish", payload.len(), "receivedByBroker");
                                                    let recipient = topics.lock().unwrap().get(destination).cloned();
                                                    if let Some(recipient) = recipient {
                                                        recipient.try_send(wire(kind, &body)).map_err(io::Error::other)?;
                                                    }
                                                }
                                                10 => {
                                                    topics.lock().unwrap().remove(topic(&body, 2)?);
                                                    write.write_all(&[0xb0,2,body[0],body[1]]).await?;
                                                }
                                                12 => write.write_all(&[0xd0,0]).await?,
                                                14 => return Ok(()),
                                                _ => return Err(io::Error::other("unexpected owned MQTT packet")),
                                            }
                                        }
                                    }
                                }
                            }.await;
                            session_stop.cancel();
                            reader.await.unwrap();
                            for destination in own_topics { topics.lock().unwrap().remove(&destination); }
                            if let Err(error) = result {
                                if !matches!(error.kind(), io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset) {
                                    eprintln!("owned MQTT session failed: {error}");
                                }
                            }
                        });
                    }
                    Some(value) = sessions.join_next(), if !sessions.is_empty() => { value.unwrap(); }
                }
            }
            while let Some(value) = sessions.join_next().await {
                value.unwrap();
            }
        });
        Self {
            address,
            active,
            topics,
            stop,
            task,
        }
    }
    pub async fn retired(&self) -> serde_json::Value {
        let immediate = self.active.load(Ordering::SeqCst) == 0;
        let started = tokio::time::Instant::now();
        let settled = tokio::time::timeout(Duration::from_secs(1), async {
            while self.active.load(Ordering::SeqCst) != 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .is_ok();
        let settled_ms = started.elapsed().as_millis();
        tokio::time::sleep(Duration::from_millis(100)).await;
        serde_json::json!({"immediateEmpty":immediate,"settled":settled,"settledMs":settled_ms,
            "lateActiveSockets":self.active.load(Ordering::SeqCst),"lateSubscriptions":self.topics.lock().unwrap().len()})
    }
    pub async fn close(self) {
        self.stop.cancel();
        self.task.await.unwrap();
    }
}
