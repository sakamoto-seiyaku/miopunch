//! Opt-in public observation through the same driver factory as member RTC.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{
    future::Future,
    io::{self, IoSliceMut},
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
};
use webrtc::runtime::{
    self, AsyncInterval, AsyncTcpListener, AsyncTcpStream, AsyncUdpSocket, JoinHandle, RecvMeta,
    Runtime, Transmit,
};

#[derive(Debug)]
struct TraceRuntime {
    inner: runtime::TokioRuntime,
    started: Instant,
    packets: Arc<Mutex<Vec<Value>>>,
    live_sockets: Arc<AtomicUsize>,
}
#[derive(Debug)]
struct TraceSocket {
    inner: Arc<dyn AsyncUdpSocket>,
    started: Instant,
    packets: Arc<Mutex<Vec<Value>>>,
    live_sockets: Arc<AtomicUsize>,
    requests: Mutex<Vec<[u8; 12]>>,
}
impl Drop for TraceSocket {
    fn drop(&mut self) {
        self.live_sockets.fetch_sub(1, Ordering::AcqRel);
    }
}
impl TraceSocket {
    fn record(&self, direction: &str, bytes: &[u8], peer: SocketAddr) {
        if bytes.len() < 20 || bytes[4..8] != [0x21, 0x12, 0xa4, 0x42] {
            return;
        }
        let transaction: [u8; 12] = bytes[8..20].try_into().unwrap();
        let mut requests = self.requests.lock().unwrap();
        let matched = if direction == "send" {
            if !requests.contains(&transaction) {
                requests.push(transaction);
            }
            None
        } else {
            Some(requests.contains(&transaction))
        };
        let mut packets = self.packets.lock().unwrap();
        if packets.len() < 128 {
            packets.push(json!({"direction":direction,"atMs":self.started.elapsed().as_millis(),
                "local":self.local_addr().ok(),"peer":peer,"bytes":bytes.len(),
                "messageType":u16::from_be_bytes([bytes[0],bytes[1]]),"transactionMatched":matched}));
        }
    }
}
impl AsyncUdpSocket for TraceSocket {
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }
    fn poll_send(&self, cx: &mut Context<'_>, transmit: &Transmit<'_>) -> Poll<io::Result<usize>> {
        let result = self.inner.poll_send(cx, transmit);
        if let Poll::Ready(Ok(_)) = &result {
            self.record("send", transmit.contents, transmit.destination);
        }
        result
    }
    fn poll_recv(
        &self,
        cx: &mut Context<'_>,
        bufs: &mut [IoSliceMut<'_>],
        meta: &mut [RecvMeta],
    ) -> Poll<io::Result<usize>> {
        let result = self.inner.poll_recv(cx, bufs, meta);
        if let Poll::Ready(Ok(n)) = &result {
            for i in 0..*n {
                for bytes in bufs[i][..meta[i].len].chunks(meta[i].stride.max(1)) {
                    self.record("recv", bytes, meta[i].addr);
                }
            }
        }
        result
    }
    fn max_gso_segments(&self) -> usize {
        self.inner.max_gso_segments()
    }
    fn max_gro_segments(&self) -> usize {
        self.inner.max_gro_segments()
    }
}
impl Runtime for TraceRuntime {
    fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>) -> Box<dyn JoinHandle> {
        self.inner.spawn(future)
    }
    fn spawn_reactor(
        &self,
        size: usize,
        future: Pin<Box<dyn Future<Output = ()> + Send>>,
    ) -> Box<dyn JoinHandle> {
        self.inner.spawn_reactor(size, future)
    }
    fn wrap_udp_socket(&self, socket: std::net::UdpSocket) -> io::Result<Arc<dyn AsyncUdpSocket>> {
        let inner = self.inner.wrap_udp_socket(socket)?;
        self.live_sockets.fetch_add(1, Ordering::AcqRel);
        Ok(Arc::new(TraceSocket {
            inner,
            started: self.started,
            live_sockets: self.live_sockets.clone(),
            packets: self.packets.clone(),
            requests: Mutex::new(Vec::new()),
        }))
    }
    fn wrap_tcp_listener(
        &self,
        listener: std::net::TcpListener,
    ) -> io::Result<Arc<dyn AsyncTcpListener>> {
        self.inner.wrap_tcp_listener(listener)
    }
    fn connect_tcp<'a>(
        &'a self,
        remote: SocketAddr,
    ) -> Pin<Box<dyn Future<Output = io::Result<Arc<dyn AsyncTcpStream>>> + Send + 'a>> {
        self.inner.connect_tcp(remote)
    }
    fn resolve_host<'a>(
        &'a self,
        host: &'a str,
    ) -> Pin<Box<dyn Future<Output = io::Result<Vec<SocketAddr>>> + Send + 'a>> {
        self.inner.resolve_host(host)
    }
    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        self.inner.sleep(duration)
    }
    fn interval(&self, period: Duration) -> Box<dyn AsyncInterval> {
        self.inner.interval(period)
    }
    fn block_on(&self, future: Pin<Box<dyn Future<Output = ()> + '_>>) {
        self.inner.block_on(future)
    }
    fn yield_now(&self) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        self.inner.yield_now()
    }
}

#[tokio::test]
#[ignore = "explicit public STUN observation; run serially with network access"]
async fn public_stun_gather() -> Result<()> {
    let observer = std::env::var("MIO_RTC_STUN_OBSERVER")
        .unwrap_or_else(|_| "stun:74.125.250.129:19302".into());
    let bind = std::env::var("MIO_RTC_STUN_BIND").ok();
    let profile = crate::rtc_ice_profile::RtcIceProfile {
        stun_servers: vec![observer],
        candidate_addresses: vec![],
        local_bind_addresses: bind
            .as_deref()
            .map(str::parse)
            .transpose()?
            .into_iter()
            .collect(),
    };
    let before = sockets();
    let started = Instant::now();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
    let events = Arc::new(Mutex::new(Vec::new()));
    let (tx, mut rx) = mpsc::channel(1);
    let packets = Arc::new(Mutex::new(Vec::new()));
    let live_sockets = Arc::new(AtomicUsize::new(0));
    let runtime = Arc::new(TraceRuntime {
        inner: runtime::TokioRuntime,
        started,
        packets: packets.clone(),
        live_sockets: live_sockets.clone(),
    });
    let pc = tokio::time::timeout_at(
        deadline,
        peer_with_runtime(tx, events.clone(), &profile, Some(runtime)),
    )
    .await??;
    let stage = Arc::new(Mutex::new("create-channel"));
    let result = tokio::time::timeout_at(deadline, async {
        let channel = pc
            .create_data_channel(
                crate::rtc_association::DATAGRAM_CHANNEL_LABEL,
                Some(RTCDataChannelInit {
                    ordered: false,
                    max_retransmits: Some(0),
                    negotiated: Some(CHANNEL_ID),
                    protocol: crate::rtc_association::DATAGRAM_CHANNEL_PROTOCOL.into(),
                    ..Default::default()
                }),
            )
            .await?;
        *stage.lock().unwrap() = "create-offer";
        let offer = pc.create_offer(None).await?;
        *stage.lock().unwrap() = "set-local-description";
        pc.set_local_description(offer).await?;
        *stage.lock().unwrap() = "gather-complete";
        rx.recv().await.ok_or("gather completion absent")?;
        let sdp = pc
            .local_description()
            .await
            .ok_or("local description absent")?
            .sdp;
        let candidates: Vec<_> = sdp
            .lines()
            .filter(|line| line.starts_with("a=candidate:"))
            .map(|line| {
                let fields: Vec<_> = line.split_ascii_whitespace().collect();
                json!({"address":fields.get(4),"port":fields.get(5),"type":fields.get(7)})
            })
            .collect();
        drop(channel);
        Ok::<_, Box<dyn Error + Send + Sync>>(candidates)
    })
    .await;
    let caller_ms = started.elapsed().as_millis();
    let partial_types: Vec<_> = pc
        .local_description()
        .await
        .map(|description| {
            description
                .sdp
                .lines()
                .filter(|line| line.starts_with("a=candidate:"))
                .filter_map(|line| line.split_ascii_whitespace().nth(7).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let cleanup = Instant::now();
    let closed = pc.close().await.is_ok();
    drop(pc);
    let close_ms = cleanup.elapsed().as_millis();
    let immediate = sockets();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let late = sockets();
    println!(
        "{}",
        json!({"event":"rtc-stun-observation","observer":profile.stun_servers,
        "callerDeadlineMs":12000,"callerMs":caller_ms,"cleanupMs":close_ms,"lateObservationMs":cleanup.elapsed().as_millis(),"bind":bind,"partialCandidateTypes":partial_types,
        "closed":closed,"ownedSocketsLate":live_sockets.load(Ordering::Acquire),"packets":packets.lock().unwrap().clone(),"stage":*stage.lock().unwrap(),"socketsBefore":before,"socketsImmediate":immediate,"socketsLate":late,
        "events":events.lock().unwrap().clone(),"candidates":result.as_ref().ok().and_then(|r|r.as_ref().ok()),
        "failure":result.as_ref().err().map(ToString::to_string),"rtcRole":"offerer",
        "dataPath":null,"identityMatch":null,"bindingMatch":null})
    );
    let candidates = result??;
    assert!(
        candidates.iter().any(|value| value["type"] == "srflx"),
        "actual srflx required"
    );
    assert!(closed);
    assert_eq!(
        live_sockets.load(Ordering::Acquire),
        0,
        "owned driver sockets must retire"
    );
    Ok(())
}

#[tokio::test]
async fn silent_observer_deadline_and_retry_release_driver_sockets() -> Result<()> {
    // A real local observer accepts UDP but deliberately never replies. Holding
    // it bound prevents ICMP port-unreachable from changing the failure mode.
    let observer = tokio::net::UdpSocket::bind("127.0.0.1:0").await?;
    let profile = crate::rtc_ice_profile::RtcIceProfile {
        stun_servers: vec![format!("stun:{}", observer.local_addr()?)],
        ..Default::default()
    };
    for attempt in 0..3 {
        let started = Instant::now();
        let packets = Arc::new(Mutex::new(Vec::new()));
        let live_sockets = Arc::new(AtomicUsize::new(0));
        let runtime = Arc::new(TraceRuntime {
            inner: runtime::TokioRuntime,
            started,
            packets: packets.clone(),
            live_sockets: live_sockets.clone(),
        });
        let events = Arc::new(Mutex::new(Vec::new()));
        let (tx, mut rx) = mpsc::channel(1);
        let deadline = tokio::time::Instant::now() + Duration::from_millis(150);
        let pc = tokio::time::timeout_at(
            deadline,
            peer_with_runtime(tx, events, &profile, Some(runtime)),
        )
        .await??;
        let outcome = tokio::time::timeout_at(deadline, async {
            let channel = pc
                .create_data_channel(
                    crate::rtc_association::DATAGRAM_CHANNEL_LABEL,
                    Some(RTCDataChannelInit {
                        ordered: false,
                        max_retransmits: Some(0),
                        negotiated: Some(CHANNEL_ID),
                        protocol: crate::rtc_association::DATAGRAM_CHANNEL_PROTOCOL.into(),
                        ..Default::default()
                    }),
                )
                .await?;
            pc.set_local_description(pc.create_offer(None).await?)
                .await?;
            rx.recv().await.ok_or("gather completion absent")?;
            drop(channel);
            Ok::<_, Box<dyn Error + Send + Sync>>(())
        })
        .await;
        let caller_ms = started.elapsed().as_millis();
        let cleanup = Instant::now();
        assert!(
            outcome.is_err(),
            "silent observer must exhaust the caller budget"
        );
        assert!(caller_ms < 500, "caller deadline must bound gathering");
        let closed = pc.close().await.is_ok();
        drop(pc);
        let immediate = live_sockets.load(Ordering::Acquire);
        let cleanup_ms = cleanup.elapsed().as_millis();
        let retired = tokio::time::timeout(Duration::from_secs(1), async {
            while live_sockets.load(Ordering::Acquire) != 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .is_ok();
        let packets = packets.lock().unwrap().clone();
        assert!(
            packets.iter().any(|value| value["direction"] == "send"),
            "real driver must request STUN"
        );
        assert!(!packets.iter().any(|value| value["direction"] == "recv"));
        println!(
            "{}",
            json!({"event":"rtc-stun-silent-observer","attempt":attempt,
            "callerDeadlineMs":150,"callerMs":caller_ms,"cleanupMs":cleanup_ms,
            "lateObservationMs":cleanup.elapsed().as_millis(),"pcClosed":closed,
            "ownedSocketsImmediate":immediate,"ownedSocketsLate":live_sockets.load(Ordering::Acquire),
            "packets":packets,"callerDeadlineExceeded":outcome.is_err(),"graceful":closed && retired})
        );
        assert!(closed && retired);
    }
    drop(observer);
    Ok(())
}
