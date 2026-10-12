//! Short-lived physical IPv4 bindings inside the existing Endpoint.
//!
//! Noq carries the socket ID independently of the real local IP. The existing
//! pool owns the descriptor; paths and pending packets retain its identity.
use std::{
    collections::BTreeMap,
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    pin::Pin,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    task::{Context, Poll},
};

use futures_util::task::AtomicWaker;
use ipnet::Ipv4Net;
use iroh_base::EndpointId;

use super::{Config, IpSender, IpTransport};
use crate::{
    metrics::SocketMetrics,
    socket::transports::{RecvInfo, Transmit},
};

/// A physical UDP binding in an Endpoint. IDs are never reused.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalSocketId(pub(crate) u64);
impl LocalSocketId {
    /// The Endpoint's original IPv4 transport.
    pub const PRIMARY: Self = Self(0);
    /// Whether this is the original physical transport.
    pub fn is_primary(self) -> bool {
        self == Self::PRIMARY
    }
    /// Stable diagnostic identity, distinct from the OS port and fd.
    pub fn get(self) -> u64 {
        self.0
    }
    pub(crate) fn to_noq(self) -> Option<noq::LocalSocketId> {
        (!self.is_primary()).then(|| noq::LocalSocketId::new(self.0))
    }
    pub(crate) fn from_noq(id: noq::LocalSocketId) -> Self {
        Self(id.get())
    }
}

/// Actual kernel binding, with identity separate from its network address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalSocketBinding {
    /// Unique physical binding identity.
    pub id: LocalSocketId,
    /// Bound port and last observed real destination IP, if available.
    pub local_addr: SocketAddr,
}

#[derive(Debug)]
pub(crate) struct PoolSocket {
    pub(crate) id: LocalSocketId,
    pub(crate) remote: EndpointId,
    pub(crate) connection_id: usize,
    io: Mutex<Option<SocketIo>>,
    real_ip: Mutex<Option<IpAddr>>,
    pinned_ip: Mutex<Option<IpAddr>>,
    port: u16,
    retained: AtomicBool,
    path_used: AtomicBool,
}
#[derive(Debug)]
struct SocketIo {
    transport: IpTransport,
    sender: IpSender,
}
impl PoolSocket {
    pub(crate) fn mark_path_used(&self) {
        self.path_used.store(true, Ordering::Release);
    }
    pub(crate) fn has_used_path(&self) -> bool {
        self.path_used.load(Ordering::Acquire)
    }
    pub(crate) fn retain(&self) {
        self.retained.store(true, Ordering::Release);
    }
    pub(crate) fn close(&self) {
        self.io.lock().expect("pool IO").take();
    }
    pub(crate) fn is_live(&self) -> bool {
        self.io.lock().expect("pool IO").is_some()
    }
    pub(crate) fn binding(&self) -> LocalSocketBinding {
        LocalSocketBinding {
            id: self.id,
            local_addr: SocketAddr::new(
                self.pinned_ip
                    .lock()
                    .expect("pool pinned IP")
                    .or(*self.real_ip.lock().expect("pool real IP"))
                    .unwrap_or(Ipv4Addr::UNSPECIFIED.into()),
                self.port,
            ),
        }
    }
    pub(crate) fn pin(&self, local: SocketAddr) -> io::Result<()> {
        if local.port() != self.port || !local.is_ipv4() || local.ip().is_unspecified() {
            return Err(io::Error::other(
                "winner has invalid physical destination binding",
            ));
        }
        let mut pinned = self.pinned_ip.lock().expect("pool pinned IP");
        if pinned.is_some_and(|ip| ip != local.ip()) {
            return Err(io::Error::other(
                "winner binding already pinned to another interface",
            ));
        }
        *pinned = Some(local.ip());
        Ok(())
    }
    pub(crate) fn is_pinned(&self) -> bool {
        self.pinned_ip.lock().expect("pool pinned IP").is_some()
    }
    pub(crate) fn poll_probe(
        &self,
        cx: &mut Context<'_>,
        remote: SocketAddr,
        bytes: &[u8],
        ttl: u8,
        source: Option<IpAddr>,
    ) -> Poll<io::Result<bool>> {
        let mut io = self.io.lock().expect("pool IO");
        let Some(io) = io.as_mut() else {
            return Poll::Ready(Err(io::Error::other("UDP binding closed")));
        };
        io.sender
            .poll_send_probe_from(cx, remote, bytes, ttl, source)
    }
    fn poll_send(
        &self,
        cx: &mut Context<'_>,
        remote: SocketAddr,
        transmit: &Transmit<'_>,
    ) -> Poll<io::Result<()>> {
        // The descriptor chooses the port and the authenticated winner chooses
        // its real source IP.
        let mut io = self.io.lock().expect("pool IO");
        let Some(io) = io.as_mut() else {
            return Poll::Ready(Err(io::Error::other("UDP binding closed")));
        };
        let source = *self.pinned_ip.lock().expect("pool pinned IP");
        if source.is_none() {
            return Poll::Ready(Err(io::Error::other("UDP winner interface not pinned")));
        }
        Pin::new(&mut io.sender).poll_send(cx, remote, source, transmit)
    }
    fn poll_recv(
        &self,
        cx: &mut Context<'_>,
        bufs: &mut [io::IoSliceMut<'_>],
        metas: &mut [noq_udp::RecvMeta],
        infos: &mut [RecvInfo],
    ) -> Poll<io::Result<usize>> {
        let mut io = self.io.lock().expect("pool IO");
        let Some(io) = io.as_mut() else {
            return Poll::Pending;
        };
        let result = io.transport.poll_recv(cx, bufs, metas, infos);
        if let Poll::Ready(Ok(n)) = result {
            for (meta, info) in metas[..n].iter_mut().zip(&mut infos[..n]) {
                let actual_ip = meta.dst_ip.map(|ip| ip.to_canonical());
                let pinned = *self.pinned_ip.lock().expect("pool pinned IP");
                if pinned.is_some() && pinned != actual_ip {
                    tracing::debug!(
                        local_socket_id = self.id.get(),
                        ?actual_ip,
                        ?pinned,
                        "discard packet addressed to another winner interface"
                    );
                    meta.len = 0;
                    meta.stride = 0;
                    continue;
                }
                // bind_full enables destination-address metadata. Keep its real
                // interface and pass the socket ID as independent I/O metadata.
                if let Some(ip) = meta.dst_ip {
                    *self.real_ip.lock().expect("pool real IP") = Some(ip.to_canonical());
                }
                info.ip_binding = Some(LocalSocketBinding {
                    id: self.id,
                    local_addr: SocketAddr::new(
                        meta.dst_ip
                            .unwrap_or(Ipv4Addr::UNSPECIFIED.into())
                            .to_canonical(),
                        self.port,
                    ),
                });
            }
        }
        result
    }
}

/// Weak registry: only a live round or its original Connection owns descriptors.
#[derive(Debug, Default)]
pub(crate) struct SocketPool {
    sockets: Mutex<BTreeMap<LocalSocketId, Weak<PoolSocket>>>,
    next: AtomicU64,
    recv_waker: AtomicWaker,
    cursor: AtomicU64,
    #[cfg(feature = "miopunch-test-hooks")]
    force_ttl_unavailable: AtomicBool,
}
impl std::fmt::Display for SocketPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IPv4ListenerPool")
    }
}
impl SocketPool {
    const MAX_SOCKETS: usize = 1024;
    pub(crate) fn register_recv_waker(&self, cx: &Context<'_>) {
        self.recv_waker.register(cx.waker());
    }
    pub(crate) fn has_sockets(&self) -> bool {
        self.sockets
            .lock()
            .expect("socket pool")
            .values()
            .any(|s| s.upgrade().is_some_and(|s| s.is_live()))
    }
    pub(crate) fn bind(
        &self,
        remote: EndpointId,
        connection_id: usize,
        metrics: Arc<SocketMetrics>,
    ) -> io::Result<Arc<PoolSocket>> {
        let mut sockets = self.sockets.lock().expect("socket pool");
        sockets.retain(|_, socket| socket.upgrade().is_some_and(|s| s.is_live()));
        if sockets.len() >= Self::MAX_SOCKETS {
            return Err(io::Error::other("Endpoint UDP listener capacity exhausted"));
        }
        let transport = IpTransport::bind(
            Config::V4 {
                ip_net: Ipv4Net::new(Ipv4Addr::UNSPECIFIED, 0).expect("IPv4 network"),
                port: 0,
                is_required: true,
                is_default: false,
            },
            metrics,
        )?;
        let port = transport.local_addr.get().port();
        let sender = transport.create_sender();
        #[cfg(feature = "miopunch-test-hooks")]
        if self.force_ttl_unavailable.load(Ordering::Acquire) {
            sender.test_force_ttl_unavailable();
        }
        let id = LocalSocketId(
            self.next
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
                .map_err(|_| io::Error::other("UDP socket IDs exhausted"))?
                + 1,
        );
        let socket = Arc::new(PoolSocket {
            id,
            remote,
            connection_id,
            io: Mutex::new(Some(SocketIo { transport, sender })),
            real_ip: Mutex::new(None),
            pinned_ip: Mutex::new(None),
            port,
            retained: AtomicBool::new(false),
            path_used: AtomicBool::new(false),
        });
        sockets.insert(id, Arc::downgrade(&socket));
        drop(sockets);
        self.recv_waker.wake();
        Ok(socket)
    }
    pub(crate) fn get(&self, id: LocalSocketId) -> Option<Arc<PoolSocket>> {
        self.sockets
            .lock()
            .expect("socket pool")
            .get(&id)?
            .upgrade()
            .filter(|s| s.is_live())
    }
    #[cfg(feature = "miopunch-test-hooks")]
    pub(crate) fn test_force_ttl_unavailable(&self) {
        self.force_ttl_unavailable.store(true, Ordering::Release);
        let sockets = self
            .sockets
            .lock()
            .expect("socket pool")
            .values()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>();
        for socket in sockets {
            if let Some(io) = socket.io.lock().expect("pool IO").as_mut() {
                io.sender.test_force_ttl_unavailable();
            }
        }
    }
    pub(crate) fn actual_ip(&self, id: LocalSocketId) -> Option<IpAddr> {
        let ip = self.get(id)?.binding().local_addr.ip();
        (!ip.is_unspecified()).then_some(ip)
    }
    /// Close pending rounds on a network change and retired interface bindings.
    /// A retained winner remains usable when its actual interface still exists.
    pub(crate) fn on_network_change(&self, actual_ips: &std::collections::HashSet<IpAddr>) {
        let sockets = self
            .sockets
            .lock()
            .expect("socket pool")
            .values()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>();
        for socket in sockets {
            if !socket.retained.load(Ordering::Acquire)
                || !actual_ips.contains(&socket.binding().local_addr.ip())
            {
                socket.close();
            }
        }
    }
    pub(crate) fn poll_send(
        &self,
        cx: &mut Context<'_>,
        id: LocalSocketId,
        source: Option<IpAddr>,
        remote: SocketAddr,
        transmit: &Transmit<'_>,
    ) -> Poll<io::Result<()>> {
        let Some(socket) = self.get(id) else {
            return Poll::Ready(Err(io::Error::other("UDP binding no longer live")));
        };
        if source.is_some_and(|ip| ip.to_canonical() != socket.binding().local_addr.ip()) {
            return Poll::Ready(Err(io::Error::other(
                "UDP source differs from pinned interface",
            )));
        }
        socket.poll_send(cx, remote, transmit)
    }
    pub(crate) fn poll_recv(
        &self,
        cx: &mut Context<'_>,
        bufs: &mut [io::IoSliceMut<'_>],
        metas: &mut [noq_udp::RecvMeta],
        infos: &mut [RecvInfo],
    ) -> Poll<io::Result<usize>> {
        self.recv_waker.register(cx.waker());
        let sockets = {
            let mut registry = self.sockets.lock().expect("socket pool");
            registry.retain(|_, s| s.upgrade().is_some_and(|s| s.is_live()));
            registry
                .values()
                .filter_map(Weak::upgrade)
                .collect::<Vec<_>>()
        };
        if sockets.is_empty() {
            return Poll::Pending;
        }
        let offset = self.cursor.fetch_add(1, Ordering::Relaxed) as usize % sockets.len();
        for socket in sockets[offset..].iter().chain(&sockets[..offset]) {
            if let Poll::Ready(result) = socket.poll_recv(cx, bufs, metas, infos) {
                return Poll::Ready(result);
            }
        }
        Poll::Pending
    }
}
