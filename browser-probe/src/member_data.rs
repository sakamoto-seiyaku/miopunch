//! Browser client of the existing, explicitly enabled one-hop MID1 data service.
//! The outer link is an already qualified member/1 M; inner QUIC datagrams stay
//! opaque. No signaling address or successful transport handshake grants membership.
#[path = "member_data_wire.rs"]
mod wire;
pub(super) const DATA_TRANSPORT_ID: u64 = wire::DATA_TRANSPORT_ID;

use crate::connection_coordinator::AttemptContext;
use futures_util::{FutureExt, future::poll_fn};
use iroh::{
    EndpointId, TransportAddr,
    endpoint::{
        Connection, RecvStream, SendStream, WeakConnectionHandle,
        transports::{
            CustomEndpoint, CustomEndpointShutdown, CustomSender, CustomTransport, RecvInfo,
            Transmit,
        },
    },
};
use iroh_base::CustomAddr;
use n0_watcher::{Direct, Watchable, Watcher};
use std::{
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Only the private gate/resource owner may mint this after checking the saved
/// explicit ForwardData permission and the actual outer member's current proof.
/// A JS boolean alone cannot construct a valid current-proof callback.
#[derive(Clone)]
pub(super) struct MemberDataPermission {
    original: WeakConnectionHandle,
    stable_id: usize,
    revision: u64,
    updates: Direct<u64>,
    current: Arc<dyn Fn() -> bool + Send + Sync>,
}
impl std::fmt::Debug for MemberDataPermission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemberDataPermission")
            .field("stable_id", &self.stable_id)
            .finish_non_exhaustive()
    }
}
impl MemberDataPermission {
    pub(super) fn qualified(
        original: &Connection,
        updates: Direct<u64>,
        current: impl Fn() -> bool + Send + Sync + 'static,
    ) -> io::Result<Self> {
        let permission = Self {
            original: original.weak_handle(),
            stable_id: original.stable_id(),
            revision: *updates.peek(),
            updates,
            current: Arc::new(current),
        };
        if original.alpn() != crate::connection_coordinator::MEMBER_ALPN
            || original.paths().iter().any(|path| {
                matches!(path.remote_addr(),
                TransportAddr::Custom(address) if address.id() == wire::DATA_TRANSPORT_ID)
            })
            || !permission.permitted()
        {
            return Err(denied(
                "MID1 requires current member proof and explicit ForwardData permission",
            ));
        }
        Ok(permission)
    }

    fn permitted(&self) -> bool {
        if *self.updates.peek() != self.revision || !(self.current)() {
            return false;
        }
        // Called from the inner QUIC transport predicate: never lock a
        // Connection or the Gate here. Proof/configuration retirement is a
        // private gate-owned atomic token; outer liveness is lock-free.
        self.original.is_open()
    }

    async fn invalidated(&self) {
        let nested = async {
            let mut paths = {
                let Some(original) = self.original.upgrade() else {
                    return;
                };
                original.path_events()
            };
            loop {
                let nested_or_closed = self.original.upgrade().is_none_or(|original| {
                    original.paths().iter().any(|path| {
                        matches!(path.remote_addr(),
                        TransportAddr::Custom(address) if address.id() == wire::DATA_TRANSPORT_ID)
                    })
                });
                if nested_or_closed {
                    break;
                }
                if futures_util::StreamExt::next(&mut paths).await.is_none() {
                    break;
                }
            }
        }
        .fuse();
        let mut updates = self.updates.clone();
        let changed = async {
            loop {
                if !self.permitted() || updates.updated().await.is_err() {
                    break;
                }
            }
        }
        .fuse();
        let closed = self.original.closed().fuse();
        futures_util::pin_mut!(changed, closed, nested);
        futures_util::select_biased! { _ = closed => {}, _ = changed => {}, _ = nested => {} }
    }
}

fn denied(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}
fn broken(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, message)
}

#[derive(Debug)]
struct Packet {
    peer: wire::Route,
    bytes: Vec<u8>,
}
#[derive(Debug)]
struct Queues {
    incoming: mpsc::Sender<Packet>,
    received: Mutex<mpsc::Receiver<Packet>>,
    outbound: mpsc::Sender<Packet>,
    sending: Mutex<mpsc::Receiver<Packet>>,
}
impl Queues {
    fn close(&self) {
        for receiver in [&self.received, &self.sending] {
            let mut receiver = receiver.lock().unwrap();
            receiver.close();
            while receiver.try_recv().is_ok() {}
        }
    }
}

struct Shared {
    via: EndpointId,
    own: wire::Route,
    addresses: Watchable<Vec<CustomAddr>>,
    bound: Mutex<Option<Arc<Queues>>>,
    permission: Mutex<Option<MemberDataPermission>>,
    attach_claimed: AtomicBool,
    stopped: CancellationToken,
    tasks: AtomicUsize,
    dropped: AtomicUsize,
}
impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MID1Client")
            .field("stopped", &self.stopped.is_cancelled())
            .finish_non_exhaustive()
    }
}
impl Shared {
    fn permitted(&self) -> bool {
        if self.stopped.is_cancelled() {
            return false;
        }
        let permission = self.permission.lock().unwrap().clone();
        permission.is_some_and(|permission| permission.permitted())
    }
    fn close(&self) {
        self.stopped.cancel();
        let _ = self.addresses.set(Vec::new());
        if let Some(queues) = self.bound.lock().unwrap().as_ref() {
            queues.close();
        }
    }
    fn address(&self, route: wire::Route) -> CustomAddr {
        CustomAddr::from_parts(
            wire::DATA_TRANSPORT_ID,
            &wire::address_data(self.via.as_bytes(), route),
        )
    }
}

/// Install before the inner A-C session, then attach a qualified outer A-B link.
#[derive(Clone, Debug)]
pub(super) struct BrowserMemberDataTransport(Arc<Shared>);
impl BrowserMemberDataTransport {
    pub(super) fn new(via: EndpointId) -> Self {
        Self(Arc::new(Shared {
            via,
            own: rand::random(),
            addresses: Watchable::new(Vec::new()),
            bound: Mutex::new(None),
            permission: Mutex::new(None),
            attach_claimed: AtomicBool::new(false),
            stopped: CancellationToken::new(),
            tasks: AtomicUsize::new(0),
            dropped: AtomicUsize::new(0),
        }))
    }
    pub(super) fn permitted(&self) -> bool {
        self.0.permitted()
    }
    pub(super) fn permits_address(&self, address: &CustomAddr) -> bool {
        self.0.permitted()
            && wire::route(address.id(), address.data(), self.0.via.as_bytes()).is_ok()
    }
    pub(super) fn address(&self) -> io::Result<CustomAddr> {
        if !self.0.permitted() {
            return Err(denied("MID1 route is not currently permitted"));
        }
        self.0
            .addresses
            .get()
            .first()
            .cloned()
            .ok_or_else(|| broken("MID1 link is not attached"))
    }

    pub(super) async fn attach(
        &self,
        original: &Connection,
        permission: MemberDataPermission,
        lifetime: Duration,
        attempt: &AttemptContext,
    ) -> io::Result<MemberDataLease> {
        if attempt.cancellation.is_cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "MID1 attach already cancelled",
            ));
        }
        if attempt.deadline <= n0_future::time::Instant::now() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "MID1 attach caller deadline already elapsed",
            ));
        }
        if lifetime.is_zero() || lifetime > Duration::from_secs(300) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "MID1 lifetime must be >0 and <=300s",
            ));
        }
        if original.remote_id() != self.0.via
            || original.stable_id() != permission.stable_id
            || !permission.permitted()
        {
            return Err(denied(
                "MID1 outer link or permission does not match forwarding member",
            ));
        }
        let queues = self
            .0
            .bound
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| broken("MID1 transport not bound"))?;
        if self.0.stopped.is_cancelled() || self.0.attach_claimed.swap(true, Ordering::AcqRel) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "MID1 generation already attached or retired",
            ));
        }
        let mut failed_attach = CloseOnDrop(Some(self.0.clone()));
        let remaining = attempt
            .deadline
            .saturating_duration_since(n0_future::time::Instant::now());
        let handshake = async {
            let (mut send, mut recv) = original.open_bi().await?;
            send.write_all(&wire::hello(self.0.own)).await?;
            let mut ack = [0; 4];
            recv.read_exact(&mut ack).await.map_err(io::Error::other)?;
            if &ack != wire::MAGIC {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid MID1 acknowledgement",
                ));
            }
            Ok::<_, io::Error>((send, recv))
        }
        .fuse();
        let cancelled = attempt.cancellation.cancelled().fuse();
        let invalidated = permission.invalidated().fuse();
        futures_util::pin_mut!(handshake, cancelled, invalidated);
        let (send, recv) = n0_future::time::timeout(remaining.min(Duration::from_secs(5)), async {
            futures_util::select_biased! {
                _ = cancelled => Err(io::Error::new(io::ErrorKind::Interrupted, "MID1 attach cancelled")),
                _ = invalidated => Err(denied("MID1 outer permission changed")),
                result = handshake => result,
            }
        }).await.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "MID1 attach caller deadline"))??;
        if !permission.permitted() || self.0.stopped.is_cancelled() {
            return Err(denied("MID1 permission changed after acknowledgement"));
        }
        *self.0.permission.lock().unwrap() = Some(permission.clone());
        let _ = self.0.addresses.set(vec![self.0.address(self.0.own)]);
        self.0.tasks.fetch_add(1, Ordering::AcqRel);
        let shared = self.0.clone();
        let finished = TaskFinished(shared.clone());
        let task_permission = permission.clone();
        let task = n0_future::task::spawn(async move {
            let _finished = finished;
            let work = forward(shared.clone(), queues, task_permission.clone(), send, recv).fuse();
            let invalidated = task_permission.invalidated().fuse();
            let stopped = shared.stopped.cancelled().fuse();
            futures_util::pin_mut!(work, invalidated, stopped);
            n0_future::time::timeout(lifetime, async {
                futures_util::select_biased! { _ = stopped => Ok(()), _ = invalidated => Err(denied("MID1 forwarding permission changed")), result = work => result }
            }).await.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "MID1 lease expired"))?
        });
        failed_attach.0 = None;
        Ok(MemberDataLease {
            shared: self.0.clone(),
            task: Some(task),
        })
    }

    pub(super) fn report(&self) -> serde_json::Value {
        let queues = self.0.bound.lock().unwrap().clone();
        serde_json::json!({"activeTasks":self.0.tasks.load(Ordering::Acquire),"stopped":self.0.stopped.is_cancelled(),
            "ready":self.0.permitted()&&!self.0.addresses.get().is_empty(),"droppedPackets":self.0.dropped.load(Ordering::Relaxed),
            "receivedQueue":queues.as_ref().map(|q|q.received.lock().unwrap().len()),
            "sendingQueue":queues.as_ref().map(|q|q.sending.lock().unwrap().len())})
    }
}

struct CloseOnDrop(Option<Arc<Shared>>);
impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        if let Some(shared) = self.0.take() {
            shared.close();
        }
    }
}
struct TaskFinished(Arc<Shared>);
impl Drop for TaskFinished {
    fn drop(&mut self) {
        self.0.close();
        self.0.tasks.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Dropping/closing this lease ends only the forwarding stream and its queues.
/// The admitted outer connection and every other application stream stay owned
/// by their original caller. Await close for a terminal cleanup observation.
pub(super) struct MemberDataLease {
    shared: Arc<Shared>,
    task: Option<n0_future::task::JoinHandle<io::Result<()>>>,
}
impl MemberDataLease {
    pub(super) async fn close(&mut self) -> io::Result<()> {
        self.shared.close();
        if let Some(task) = self.task.take() {
            task.await.map_err(io::Error::other)??;
        }
        Ok(())
    }
}
impl Drop for MemberDataLease {
    fn drop(&mut self) {
        self.shared.close();
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn read_packet(recv: &mut RecvStream) -> io::Result<Packet> {
    let mut header = [0; 18];
    recv.read_exact(&mut header)
        .await
        .map_err(io::Error::other)?;
    let (peer, length) = wire::parse_header(&header)?;
    let mut bytes = vec![0; length];
    recv.read_exact(&mut bytes)
        .await
        .map_err(io::Error::other)?;
    Ok(Packet { peer, bytes })
}
async fn forward(
    shared: Arc<Shared>,
    queues: Arc<Queues>,
    permission: MemberDataPermission,
    mut send: SendStream,
    mut recv: RecvStream,
) -> io::Result<()> {
    let receive = async {
        loop {
            let packet = read_packet(&mut recv).await?;
            if !permission.permitted() {
                return Err(denied("MID1 receive permission changed"));
            }
            if queues.incoming.try_send(packet).is_err() {
                shared.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    .fuse();
    let transmit = async {
        let mut budget = Budget::new();
        while let Some(packet) = poll_fn(|cx| queues.sending.lock().unwrap().poll_recv(cx)).await {
            if !permission.permitted() {
                return Err(denied("MID1 egress permission changed"));
            }
            if !budget.take(packet.bytes.len()) {
                shared.dropped.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            send.write_all(&wire::header(packet.peer, packet.bytes.len())?)
                .await?;
            send.write_all(&packet.bytes).await?;
        }
        Ok(())
    }
    .fuse();
    futures_util::pin_mut!(receive, transmit);
    futures_util::select_biased! { result = receive => result, result = transmit => result }
}

struct Budget {
    started: n0_future::time::Instant,
    bytes: usize,
    packets: usize,
}
impl Budget {
    fn new() -> Self {
        Self {
            started: n0_future::time::Instant::now(),
            bytes: 0,
            packets: 0,
        }
    }
    fn take(&mut self, bytes: usize) -> bool {
        if self.started.elapsed() >= Duration::from_secs(1) {
            *self = Self::new();
        }
        if self.bytes.saturating_add(bytes) > wire::BYTES_PER_SECOND
            || self.packets >= wire::PACKETS_PER_SECOND
        {
            return false;
        }
        self.bytes += bytes;
        self.packets += 1;
        true
    }
}

#[derive(Debug)]
struct MemberEndpoint {
    shared: Arc<Shared>,
    queues: Arc<Queues>,
}
impl Drop for MemberEndpoint {
    fn drop(&mut self) {
        self.shared.close();
    }
}
impl CustomTransport for BrowserMemberDataTransport {
    fn bind(&self) -> io::Result<Box<dyn CustomEndpoint>> {
        let mut binding = self.0.bound.lock().unwrap();
        if binding.is_some() || self.0.stopped.is_cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "MID1 transport already bound or stopped",
            ));
        }
        let (incoming, received) = mpsc::channel(wire::QUEUE_DEPTH);
        let (outbound, sending) = mpsc::channel(wire::QUEUE_DEPTH);
        let queues = Arc::new(Queues {
            incoming,
            received: Mutex::new(received),
            outbound,
            sending: Mutex::new(sending),
        });
        *binding = Some(queues.clone());
        Ok(Box::new(MemberEndpoint {
            shared: self.0.clone(),
            queues,
        }))
    }
}
impl CustomEndpoint for MemberEndpoint {
    fn watch_local_addrs(&self) -> Direct<Vec<CustomAddr>> {
        self.shared.addresses.watch()
    }
    fn create_sender(&self) -> Arc<dyn CustomSender> {
        Arc::new(MemberSender {
            shared: self.shared.clone(),
            queues: self.queues.clone(),
        })
    }
    fn poll_recv(
        &mut self,
        cx: &mut Context,
        bufs: &mut [io::IoSliceMut<'_>],
        metas: &mut [noq_udp::RecvMeta],
        infos: &mut [RecvInfo],
    ) -> Poll<io::Result<usize>> {
        if bufs.is_empty() {
            return Poll::Ready(Ok(0));
        }
        if self.shared.stopped.is_cancelled() {
            return Poll::Ready(Err(broken("MID1 endpoint stopped")));
        }
        loop {
            let received = self.queues.received.lock().unwrap().poll_recv(cx);
            match received {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => {
                    return Poll::Ready(Err(broken("MID1 receiving queue stopped")));
                }
                Poll::Ready(Some(packet)) if packet.bytes.len() <= bufs[0].len() => {
                    if !self.shared.permitted() {
                        self.shared.close();
                        return Poll::Ready(Err(denied("MID1 route permission changed")));
                    }
                    bufs[0][..packet.bytes.len()].copy_from_slice(&packet.bytes);
                    metas[0].len = packet.bytes.len();
                    metas[0].stride = packet.bytes.len();
                    infos[0] = RecvInfo::new(
                        self.shared.address(packet.peer),
                        self.shared.addresses.get().first().cloned(),
                    );
                    return Poll::Ready(Ok(1));
                }
                _ => {
                    self.shared.dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }
}
impl CustomEndpointShutdown for MemberEndpoint {
    fn close(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        self.shared.close();
        Box::pin(std::future::ready(()))
    }
}
#[derive(Debug)]
struct MemberSender {
    shared: Arc<Shared>,
    queues: Arc<Queues>,
}
impl CustomSender for MemberSender {
    fn is_valid_send_addr(&self, addr: &CustomAddr) -> bool {
        self.shared.permitted()
            && wire::route(addr.id(), addr.data(), self.shared.via.as_bytes()).is_ok()
    }
    fn poll_send(
        &self,
        _cx: &mut Context,
        dst: &CustomAddr,
        _src: Option<&CustomAddr>,
        transmit: &Transmit<'_>,
    ) -> Poll<io::Result<()>> {
        if !self.shared.permitted() {
            self.shared.close();
            return Poll::Ready(Err(denied("MID1 sending permission changed")));
        }
        let peer = match wire::route(dst.id(), dst.data(), self.shared.via.as_bytes()) {
            Ok(peer) => peer,
            Err(error) => return Poll::Ready(Err(error)),
        };
        if let Err(error) = wire::header(peer, transmit.contents.len()) {
            return Poll::Ready(Err(error));
        }
        Poll::Ready(
            match self.queues.outbound.try_send(Packet {
                peer,
                bytes: transmit.contents.to_vec(),
            }) {
                Ok(()) => Ok(()),
                Err(mpsc::error::TrySendError::Full(_)) => {
                    self.shared.dropped.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    Err(broken("MID1 sending queue stopped"))
                }
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_ceiling_limits_small_packets_and_large_payloads_independently() {
        let mut packets = Budget::new();
        for _ in 0..wire::PACKETS_PER_SECOND {
            assert!(packets.take(1));
        }
        assert!(!packets.take(1));
        let mut bytes = Budget::new();
        for _ in 0..wire::BYTES_PER_SECOND / wire::MAX_DATAGRAM {
            assert!(bytes.take(wire::MAX_DATAGRAM));
        }
        assert!(!bytes.take(1));
    }

    #[test]
    fn closing_both_bounded_queues_discards_pending_payload_and_refuses_late_sender() {
        let (incoming, received) = mpsc::channel(wire::QUEUE_DEPTH);
        let (outbound, sending) = mpsc::channel(wire::QUEUE_DEPTH);
        let queues = Queues {
            incoming,
            received: Mutex::new(received),
            outbound,
            sending: Mutex::new(sending),
        };
        for _ in 0..wire::QUEUE_DEPTH {
            queues
                .incoming
                .try_send(Packet {
                    peer: [1; 16],
                    bytes: vec![2; wire::MAX_DATAGRAM],
                })
                .unwrap();
            queues
                .outbound
                .try_send(Packet {
                    peer: [3; 16],
                    bytes: vec![4; wire::MAX_DATAGRAM],
                })
                .unwrap();
        }
        assert_eq!(queues.received.lock().unwrap().len(), wire::QUEUE_DEPTH);
        assert_eq!(queues.sending.lock().unwrap().len(), wire::QUEUE_DEPTH);
        queues.close();
        assert_eq!(queues.received.lock().unwrap().len(), 0);
        assert_eq!(queues.sending.lock().unwrap().len(), 0);
        assert!(matches!(
            queues.outbound.try_send(Packet {
                peer: [0; 16],
                bytes: vec![1]
            }),
            Err(mpsc::error::TrySendError::Closed(_))
        ));
    }
}
