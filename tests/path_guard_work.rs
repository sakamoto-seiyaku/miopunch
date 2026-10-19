#![cfg(not(target_arch = "wasm32"))]
#[path = "support/admitted_ip.rs"]
mod admitted_ip;

use futures_util::future::poll_fn;
use iroh::{
    Endpoint, TransportAddr,
    endpoint::{
        Connection, RestrictedEndpoint, WeakConnectionHandle, presets,
        transports::{
            CustomEndpoint, CustomEndpointShutdown, CustomSender, CustomTransport, RecvInfo,
            Transmit,
        },
    },
};
use iroh_base::CustomAddr;
use miopunch_native_probe::rtc_transport::{RtcDatagramIo, RtcTransport, TRANSPORT_ID};
use std::{
    io::{self, IoSliceMut},
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

const ALPN: &[u8] = b"miopunch/path-guard-work/1";
#[derive(Debug, Default)]
struct Calls {
    valid: AtomicU64,
    bootstrap: AtomicU64,
    guards: AtomicU64,
    owner: AtomicU64,
    qualified: AtomicU64,
    send: AtomicU64,
}
impl Calls {
    fn snapshot(&self) -> [u64; 6] {
        [
            &self.valid,
            &self.bootstrap,
            &self.guards,
            &self.owner,
            &self.qualified,
            &self.send,
        ]
        .map(|v| v.load(Ordering::SeqCst))
    }
}
// Count entries around the existing production RTC sender. Return values and
// resource lifecycle are forwarded; this fixture adds no transport policy.
#[derive(Debug)]
struct CountTransport {
    inner: RtcTransport,
    calls: Arc<Calls>,
}
#[derive(Debug)]
struct CountEndpoint {
    inner: Box<dyn CustomEndpoint>,
    calls: Arc<Calls>,
}
#[derive(Debug)]
struct CountSender {
    inner: Arc<dyn CustomSender>,
    calls: Arc<Calls>,
}
impl CustomTransport for CountTransport {
    fn bind(&self) -> io::Result<Box<dyn CustomEndpoint>> {
        Ok(Box::new(CountEndpoint {
            inner: self.inner.bind()?,
            calls: self.calls.clone(),
        }))
    }
}
impl CustomEndpoint for CountEndpoint {
    fn shutdown_handler(&self) -> Option<Arc<dyn CustomEndpointShutdown>> {
        self.inner.shutdown_handler()
    }
    fn watch_local_addrs(&self) -> n0_watcher::Direct<Vec<CustomAddr>> {
        self.inner.watch_local_addrs()
    }
    fn create_sender(&self) -> Arc<dyn CustomSender> {
        Arc::new(CountSender {
            inner: self.inner.create_sender(),
            calls: self.calls.clone(),
        })
    }
    fn poll_recv(
        &mut self,
        cx: &mut Context<'_>,
        bufs: &mut [IoSliceMut<'_>],
        metas: &mut [noq::udp::RecvMeta],
        infos: &mut [RecvInfo],
    ) -> Poll<io::Result<usize>> {
        self.inner.poll_recv(cx, bufs, metas, infos)
    }
    fn max_transmit_segments(&self) -> NonZeroUsize {
        self.inner.max_transmit_segments()
    }
}
impl CustomSender for CountSender {
    fn is_valid_send_addr(&self, a: &CustomAddr) -> bool {
        self.calls.valid.fetch_add(1, Ordering::SeqCst);
        self.inner.is_valid_send_addr(a)
    }
    fn can_bootstrap_connection(&self, a: &CustomAddr) -> bool {
        self.calls.bootstrap.fetch_add(1, Ordering::SeqCst);
        self.inner.can_bootstrap_connection(a)
    }
    fn guards_application_data(&self, a: &CustomAddr) -> bool {
        self.calls.guards.fetch_add(1, Ordering::SeqCst);
        self.inner.guards_application_data(a)
    }
    fn is_valid_connection_addr(&self, a: &CustomAddr, id: usize) -> bool {
        self.calls.owner.fetch_add(1, Ordering::SeqCst);
        self.inner.is_valid_connection_addr(a, id)
    }
    fn is_data_path_qualified(&self, a: &CustomAddr, id: usize) -> bool {
        self.calls.qualified.fetch_add(1, Ordering::SeqCst);
        self.inner.is_data_path_qualified(a, id)
    }
    fn poll_send(
        &self,
        cx: &mut Context<'_>,
        dst: &CustomAddr,
        src: Option<&CustomAddr>,
        t: &Transmit<'_>,
    ) -> Poll<io::Result<()>> {
        self.calls.send.fetch_add(1, Ordering::SeqCst);
        self.inner.poll_send(cx, dst, src, t)
    }
}
async fn endpoint(inner: &RtcTransport, calls: &Arc<Calls>) -> RestrictedEndpoint {
    Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .clear_ip_transports()
        .bind_addr("127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap())
        .unwrap()
        .clear_relay_transports()
        .add_custom_transport(Arc::new(CountTransport {
            inner: inner.clone(),
            calls: calls.clone(),
        }))
        .alpns(vec![ALPN.to_vec()])
        .restricted_application(admitted_ip::IpAdmission(vec![TRANSPORT_ID]))
        .bind()
        .await
        .unwrap()
}
async fn pair(a: &RestrictedEndpoint, b: &RestrictedEndpoint) -> (Connection, Connection) {
    tokio::time::timeout(Duration::from_secs(3), async {
        let (c, s) = tokio::join!(a.connect(b.addr(), ALPN), async {
            b.accept().await.unwrap().await
        });
        (c.unwrap(), s.unwrap())
    })
    .await
    .unwrap()
}
fn pump(from: RtcDatagramIo, to: RtcDatagramIo) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Ok(packet) = poll_fn(|cx| from.poll_outbound(cx)).await {
            if to.receive(packet).is_err() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
}
fn open_check(
    original: WeakConnectionHandle,
    calls: Arc<AtomicU64>,
) -> impl Fn() -> bool + Send + Sync + 'static {
    move || {
        calls.fetch_add(1, Ordering::SeqCst);
        original.is_open()
    }
}
#[tokio::test]
async fn ordinary_ip_is_permissive_and_explicit_rtc_guards_preserve_the_original_stream() {
    let at = RtcTransport::new();
    let bt = RtcTransport::new();
    let ac = Arc::new(Calls::default());
    let bc = Arc::new(Calls::default());
    let a = endpoint(&at, &ac).await;
    let b = endpoint(&bt, &bc).await;
    let (client, server) = pair(&a, &b).await;
    let (other, other_server) = pair(&a, &b).await;
    let (mut send, mut reply) = client.open_bi().await.unwrap();
    send.write_all(b"open").await.unwrap();
    let (mut response, mut incoming) = server.accept_bi().await.unwrap();
    let mut first = [0; 4];
    incoming.read_exact(&mut first).await.unwrap();
    assert_eq!(&first, b"open");
    response.write_all(b"open").await.unwrap();
    reply.read_exact(&mut first).await.unwrap();
    let payload: Vec<u8> = (0..32768).map(|i| (i % 251) as u8).collect();
    let mut received = vec![0; payload.len()];
    let ip_before = [ac.snapshot(), bc.snapshot()];
    for _ in 0..3 {
        tokio::join!(
            async {
                send.write_all(&payload).await.unwrap();
                reply.read_exact(&mut received).await.unwrap();
                assert_eq!(received, payload);
            },
            async {
                let mut bytes = vec![0; payload.len()];
                incoming.read_exact(&mut bytes).await.unwrap();
                assert_eq!(bytes, payload);
                response.write_all(&bytes).await.unwrap();
            }
        );
    }
    let ip_after = [ac.snapshot(), bc.snapshot()];
    assert_eq!(ip_before, [[0; 6]; 2]);
    assert_eq!(
        ip_after, ip_before,
        "ordinary IP never enters custom sender policy methods"
    );
    let weak = client.weak_handle();
    let authorization_calls = Arc::new(AtomicU64::new(0));
    let lease = at
        .reserve(
            &client,
            open_check(weak.clone(), authorization_calls.clone()),
        )
        .unwrap();
    let peer = bt
        .reserve(
            &server,
            open_check(server.weak_handle(), authorization_calls.clone()),
        )
        .unwrap();
    lease.set_remote(peer.local_addr()).unwrap();
    peer.set_remote(lease.local_addr()).unwrap();
    lease.activate().unwrap();
    peer.activate().unwrap();
    let ab = pump(lease.io(), peer.io());
    let ba = pump(peer.io(), lease.io());
    assert!(
        a.open_data_transport_path(&other, TransportAddr::Custom(peer.local_addr()))
            .await
            .is_err(),
        "a foreign original connection cannot use the association"
    );
    a.open_data_transport_path(&client, TransportAddr::Custom(peer.local_addr()))
        .await
        .unwrap();
    let (probe, remote_probe) = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let (Ok(x), Ok(y)) = (lease.start_probe(&client), peer.start_probe(&server)) {
                break (x, y);
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(lease.retain(&client, &remote_probe).is_err());
    assert!(lease.retain(&other, &probe).is_err());
    a.control_handle(&client)
        .test_limit_transport_paths(vec![TransportAddr::Custom(peer.local_addr())])
        .await
        .unwrap();
    b.control_handle(&server)
        .test_limit_transport_paths(vec![TransportAddr::Custom(lease.local_addr())])
        .await
        .unwrap();
    for p in client.paths().iter().filter(|p| p.is_ip()) {
        p.close().unwrap();
    }
    for p in server.paths().iter().filter(|p| p.is_ip()) {
        let _ = p.close();
    }
    send.write_all(b"pending").await.unwrap();
    let mut pending = [0; 7];
    assert!(
        tokio::time::timeout(
            Duration::from_millis(150),
            incoming.read_exact(&mut pending)
        )
        .await
        .is_err(),
        "unqualified RTC cannot carry ordinary bytes"
    );
    for p in client
        .paths()
        .iter()
        .filter(|p| p.remote_addr().is_custom())
    {
        assert_eq!(p.stats().frame_tx.stream, 0);
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        while !probe.is_acknowledged().unwrap() || !remote_probe.is_acknowledged().unwrap() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let pending_calls = [ac.snapshot(), bc.snapshot()];
    lease.retain(&client, &probe).unwrap();
    peer.retain(&server, &remote_probe).unwrap();
    a.control_handle(&client)
        .retain_data_transport_path(peer.local_addr())
        .await
        .unwrap();
    b.control_handle(&server)
        .retain_data_transport_path(lease.local_addr())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), incoming.read_exact(&mut pending))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&pending, b"pending");
    response.write_all(&pending).await.unwrap();
    reply.read_exact(&mut pending).await.unwrap();
    let bound_before = [ac.snapshot(), bc.snapshot()];
    tokio::time::timeout(Duration::from_secs(5), async {
        for _ in 0..3 {
            tokio::join!(
                async {
                    send.write_all(&payload).await.unwrap();
                    reply.read_exact(&mut received).await.unwrap();
                    assert_eq!(received, payload);
                },
                async {
                    let mut bytes = vec![0; payload.len()];
                    incoming.read_exact(&mut bytes).await.unwrap();
                    assert_eq!(bytes, payload);
                    response.write_all(&bytes).await.unwrap();
                }
            );
        }
    })
    .await
    .unwrap();
    let bound_after = [ac.snapshot(), bc.snapshot()];
    for side in 0..2 {
        for method in [2, 3, 4, 5] {
            assert!(
                bound_after[side][method] > bound_before[side][method],
                "actual guard and send entries measured during stream work"
            );
        }
    }
    lease.close();
    peer.close();
    assert!(lease.retain(&client, &probe).is_err());
    assert!(!at.is_valid_connection_addr(&peer.local_addr(), client.stable_id()));
    assert!(
        a.open_data_transport_path(&other, TransportAddr::Custom(peer.local_addr()))
            .await
            .is_err()
    );
    send.write_all(b"forbidden").await.unwrap();
    let mut forbidden = [0; 9];
    let forbidden_result = tokio::time::timeout(
        Duration::from_millis(150),
        incoming.read_exact(&mut forbidden),
    )
    .await;
    assert!(
        !matches!(forbidden_result, Ok(Ok(_))),
        "retired route must not deliver queued ordinary bytes"
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        ab.await.unwrap();
        ba.await.unwrap();
    })
    .await
    .unwrap();
    assert!(weak.is_open());
    client.close(0u32.into(), b"ended");
    assert!(
        !weak.is_open(),
        "terminal state is visible without reentering the runtime lock"
    );
    for c in [&server, &other, &other_server] {
        c.close(0u32.into(), b"ended");
    }
    tokio::join!(a.close(), b.close());
    assert_eq!(at.active_associations(), 0);
    assert_eq!(bt.active_associations(), 0);
    assert_eq!(at.active_pumps(), 0);
    assert_eq!(bt.active_pumps(), 0);
    assert!(
        authorization_calls.load(Ordering::SeqCst) > 0,
        "production route predicates actually ran their lock-free original-open checks"
    );
    println!(
        "GUARD_WORK {}",
        serde_json::json!({"units":["is_valid_send_addr_entries","can_bootstrap_entries","guards_application_data_entries","is_valid_connection_addr_entries","is_data_path_qualified_entries","poll_send_entries"],"bytesEachWayInEachMeasuredPhase":98304,"ipBefore":ip_before,"ipAfter":ip_after,"pending":pending_calls,"boundBefore":bound_before,"boundAfter":bound_after,"authorizationChecks":authorization_calls.load(Ordering::SeqCst),"sameStream":true,"unqualifiedBytesRejected":true,"wrongOwnerAndProbeRejected":true,"retiredQueuedBytesRejected":true,"pumpsJoinedNaturally":true,"scope":"method entries across actor/packet paths in the phase, not CPU time or per-packet counts"})
    );
}

#[derive(Debug, Default)]
struct HeldSocketState {
    hold: std::sync::atomic::AtomicBool,
    pending: AtomicU64,
    forbidden_repolls: AtomicU64,
    held_repolls: AtomicU64,
    sent: AtomicU64,
    packet: std::sync::Mutex<Vec<u8>>,
    forbidden_packet: std::sync::Mutex<Vec<u8>>,
    wake: std::sync::Mutex<Option<std::task::Waker>>,
    observed: tokio::sync::Notify,
}
#[derive(Debug)]
struct HeldSocket {
    inner: Box<dyn noq::AsyncUdpSocket>,
    state: Arc<HeldSocketState>,
}
#[derive(Debug)]
struct HeldSender {
    inner: std::pin::Pin<Box<dyn noq::UdpSender>>,
    state: Arc<HeldSocketState>,
}
impl noq::AsyncUdpSocket for HeldSocket {
    fn create_sender(&self) -> std::pin::Pin<Box<dyn noq::UdpSender>> {
        Box::pin(HeldSender {
            inner: self.inner.create_sender(),
            state: self.state.clone(),
        })
    }
    fn poll_recv(
        &mut self,
        cx: &mut Context<'_>,
        bufs: &mut [IoSliceMut<'_>],
        metas: &mut [noq::udp::RecvMeta],
    ) -> Poll<io::Result<usize>> {
        self.inner.poll_recv(cx, bufs, metas)
    }
    fn local_addr(&self) -> io::Result<std::net::SocketAddr> {
        self.inner.local_addr()
    }
}
impl noq::UdpSender for HeldSender {
    fn poll_send(
        mut self: std::pin::Pin<&mut Self>,
        transmit: &noq::udp::Transmit<'_>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        if self.state.hold.load(Ordering::Acquire) {
            self.state.pending.fetch_add(1, Ordering::SeqCst);
            *self.state.packet.lock().unwrap() = transmit.contents.to_vec();
            *self.state.wake.lock().unwrap() = Some(cx.waker().clone());
            self.state.observed.notify_one();
            return Poll::Pending;
        }
        let repeats_held_ciphertext =
            transmit.contents == self.state.packet.lock().unwrap().as_slice();
        if repeats_held_ciphertext {
            self.state.held_repolls.fetch_add(1, Ordering::SeqCst);
        }
        if transmit.contents == self.state.forbidden_packet.lock().unwrap().as_slice() {
            self.state.forbidden_repolls.fetch_add(1, Ordering::SeqCst);
        }
        let result = self.inner.as_mut().poll_send(transmit, cx);
        if matches!(result, Poll::Ready(Ok(()))) {
            self.state.sent.fetch_add(1, Ordering::SeqCst);
        }
        result
    }
}

#[tokio::test]
async fn newly_forbidden_noq_path_drops_the_actual_buffered_transmit_and_inbound_stream() {
    noq_buffered_permission(false).await;
}

#[tokio::test]
async fn changed_application_permission_drops_the_actual_buffered_noq_stream() {
    noq_buffered_permission(true).await;
}

async fn noq_buffered_permission(application_only: bool) {
    use noq::Runtime;
    use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let der = CertificateDer::from(cert.cert);
    let key = PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der());
    let server_config = noq::ServerConfig::with_single_cert(vec![der.clone()], key.into()).unwrap();
    let server_endpoint =
        noq::Endpoint::server(server_config, "127.0.0.1:0".parse().unwrap()).unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(der).unwrap();
    let client_config = noq::ClientConfig::with_root_certificates(Arc::new(roots)).unwrap();
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.set_nonblocking(true).unwrap();
    let state = Arc::new(HeldSocketState::default());
    let abstract_socket = HeldSocket {
        inner: noq::TokioRuntime.wrap_udp_socket(socket).unwrap(),
        state: state.clone(),
    };
    let mut client_endpoint = noq::Endpoint::new_with_abstract_socket(
        Default::default(),
        None,
        Box::new(abstract_socket),
        Arc::new(noq::TokioRuntime),
    )
    .unwrap();
    client_endpoint.set_default_client_config(client_config);
    let client_addr = client_endpoint.local_addr().unwrap();
    let server_addr = server_endpoint.local_addr().unwrap();
    let (client, server) = tokio::time::timeout(Duration::from_secs(3), async {
        let (c, s) = tokio::join!(
            client_endpoint.connect(server_addr, "localhost").unwrap(),
            async { server_endpoint.accept().await.unwrap().await }
        );
        (c.unwrap(), s.unwrap())
    })
    .await
    .unwrap();
    // No restriction installed: ordinary old sender methods carry a real stream.
    let mut warm = client.open_uni().await.unwrap();
    warm.write_all(b"ordinary").await.unwrap();
    warm.finish().unwrap();
    let mut received = server.accept_uni().await.unwrap();
    assert_eq!(received.read_to_end(64).await.unwrap(), b"ordinary");
    warm.stopped().await.unwrap();
    assert_eq!(client.path_filter_stats(), noq::PathFilterStats::default());
    let calls = Arc::new(AtomicU64::new(0));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let sent_before = state.sent.load(Ordering::SeqCst);
    assert!(sent_before > 0);
    let weak = client.weak_handle();
    let application_allowed = Arc::new(std::sync::atomic::AtomicBool::new(true));
    if application_only {
        let allowed = application_allowed.clone();
        let observed = calls.clone();
        let inside = weak.clone();
        client.restrict_application_data_paths(move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
            let _ = inside.is_open();
            allowed.load(Ordering::Acquire)
        });
    }
    let streams_before = client.stats().frame_tx.stream;
    state.hold.store(true, Ordering::Release);
    let mut forbidden = client.open_uni().await.unwrap();
    forbidden.write_all(b"buffered-forbidden").await.unwrap();
    forbidden.finish().unwrap();
    tokio::time::timeout(Duration::from_secs(1), state.observed.notified())
        .await
        .unwrap();
    assert!(state.pending.load(Ordering::SeqCst) > 0);
    assert!(!state.packet.lock().unwrap().is_empty());
    assert!(
        client.stats().frame_tx.stream > streams_before,
        "the held actual transmit contains a newly encoded STREAM"
    );
    let pending_ciphertext_bytes = state.packet.lock().unwrap().len();
    *state.forbidden_packet.lock().unwrap() = state.packet.lock().unwrap().clone();
    if application_only {
        application_allowed.store(false, Ordering::Release);
    } else {
        let inside = weak.clone();
        let observed = calls.clone();
        client.restrict_network_paths(move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
            let _ = inside.is_open();
            false
        });
    }
    assert!(
        calls.load(Ordering::SeqCst) > 0,
        "the explicitly installed predicate ran during encoding or restriction installation"
    );
    state.hold.store(false, Ordering::Release);
    if let Some(wake) = state.wake.lock().unwrap().take() {
        wake.wake();
    }
    let leaked = tokio::time::timeout(Duration::from_millis(150), async {
        let mut stream = server.accept_uni().await.unwrap();
        stream.read_to_end(64).await.unwrap()
    })
    .await
    .ok();
    let sent_after_permission_change = state.sent.load(Ordering::SeqCst);
    let mut held_probe_repolls = 0;
    if application_only {
        // A fresh PING, encoded after the rejected STREAM batch, must retain its
        // own classification even while pending on the same original sender.
        let ping_before = client.stats().frame_tx.ping;
        let received_ping_before = server.stats().frame_rx.ping;
        let ack_before = client.stats().frame_rx.acks + client.stats().frame_rx.path_acks;
        let pending_before = state.pending.load(Ordering::SeqCst);
        let repolls_before = state.held_repolls.load(Ordering::SeqCst);
        state.hold.store(true, Ordering::Release);
        client.path(noq::PathId::ZERO).unwrap().ping().unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while client.stats().frame_tx.ping <= ping_before
                || state.pending.load(Ordering::SeqCst) <= pending_before
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        state.hold.store(false, Ordering::Release);
        if let Some(wake) = state.wake.lock().unwrap().take() {
            wake.wake();
        }
        tokio::time::timeout(Duration::from_secs(1), async {
            while server.stats().frame_rx.ping <= received_ping_before
                || client.stats().frame_rx.acks + client.stats().frame_rx.path_acks <= ack_before
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("a held actual PING is sent and ACKed while application permission stays false");
        held_probe_repolls = state.held_repolls.load(Ordering::SeqCst) - repolls_before;
        assert!(held_probe_repolls > 0);
    }
    let mut reverse = server.open_uni().await.unwrap();
    reverse.write_all(b"inbound-forbidden").await.unwrap();
    reverse.finish().unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(150), client.accept_uni())
            .await
            .is_err()
    );
    let filters = client.path_filter_stats();
    if !application_only {
        assert!(
            filters.denied_inbound > 0,
            "an actual encrypted inbound packet is rejected before QUIC"
        );
    }
    assert!(weak.is_open());
    client.close(0u32.into(), b"closed");
    assert!(!weak.is_open());
    server.close(0u32.into(), b"closed");
    tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(client_endpoint.wait_idle(), server_endpoint.wait_idle());
    })
    .await
    .unwrap();
    drop(warm);
    drop(received);
    drop(forbidden);
    drop(reverse);
    drop(client);
    drop(server);
    drop(client_endpoint);
    drop(server_endpoint);
    for addr in [client_addr, server_addr] {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                match std::net::UdpSocket::bind(addr) {
                    Ok(socket) => {
                        drop(socket);
                        break;
                    }
                    Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
                        tokio::task::yield_now().await
                    }
                    Err(error) => panic!("owned Noq socket rebind failed: {error}"),
                }
            }
        })
        .await
        .expect("Noq endpoint driver releases its owned socket after the final reference drops");
    }
    println!(
        "NOQ_GUARD_WORK {}",
        serde_json::json!({"applicationOnly":application_only,"leakedPlaintext":leaked.as_ref().map(|v|String::from_utf8_lossy(v).into_owned()),"ordinaryUnrestrictedFilterCallbackCalls":0,"explicitFilterCallbackCalls":calls.load(Ordering::SeqCst),"actualCarrierSendsBeforeRestriction":sent_before,"actualCarrierSendsInPermissionWindow":sent_after_permission_change,"actualCarrierSendsAfterLocalClose":state.sent.load(Ordering::SeqCst),"forbiddenCiphertextRepolls":state.forbidden_repolls.load(Ordering::SeqCst),"heldProbeRepollsWhileApplicationForbidden":held_probe_repolls,"pendingSenderPolls":state.pending.load(Ordering::SeqCst),"pendingCiphertextBytes":pending_ciphertext_bytes,"deniedInboundDatagrams":filters.denied_inbound,"deniedOutboundTransmits":filters.denied_outbound,"terminalObservedWithoutRuntimeLock":true,"ownedUdpPortsRebound":true,"scope":"actual callback/poll/datagram/transmit entries, not byte-normalized CPU cost"})
    );
    assert!(
        leaked.is_none(),
        "forbidden buffered STREAM was delivered: {leaked:?}"
    );
    assert_eq!(
        state.forbidden_repolls.load(Ordering::SeqCst),
        0,
        "the already buffered forbidden ciphertext must never be handed back to its carrier"
    );
    if !application_only {
        assert_eq!(
            sent_after_permission_change, sent_before,
            "packet restriction blocks every carrier send after capacity returns"
        );
    }
}

#[allow(dead_code)]
mod ordinary_consumer {
    include!("path_guard_consumers/ordinary.rs");
    pub fn check() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                for case in ["ip", "relay", "legacy"] {
                    exchange(case).await;
                }
            });
    }
}

#[test]
fn fixed_original_consumer_keeps_ip_relay_and_legacy_sender_streams_usable() {
    ordinary_consumer::check();
}
