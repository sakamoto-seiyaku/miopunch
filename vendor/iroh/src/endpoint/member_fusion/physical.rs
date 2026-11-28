//! Mapping-sensitive work is serialized on its actual local interface.
use super::{Endpoint, EndpointId, Instant, time};
use std::{
    collections::HashMap,
    io,
    net::SocketAddr,
    sync::{Arc, Mutex, Weak},
};
use tokio::sync::watch;

/// Select a usable observation route within the original attempt's deadline.
pub(crate) async fn observer_binding(
    endpoint: &crate::socket::EndpointInner,
    family: iroh_relay::ObservationFamily,
    observers: impl IntoIterator<Item = (crate::RelayUrl, u16)>,
    configured: Option<SocketAddr>,
    deadline: Instant,
) -> io::Result<(SocketAddr, SocketAddr)> {
    let mut last_error = io::Error::from(io::ErrorKind::AddrNotAvailable);
    for (observer, port) in observers {
        let result = time::timeout_at(deadline, async {
            let destination = endpoint
                .resolve_observation_addr(&observer, family, port)
                .await?;
            let binding = match configured {
                Some(binding) => Scope::route_binding(binding, destination)?,
                None => endpoint.udp_punching_binding_for(destination).await?,
            };
            endpoint.punching_scope(binding, configured.is_some())?;
            Ok((binding, destination))
        })
        .await
        .map_err(|_| io::ErrorKind::TimedOut)?;
        match result {
            Ok(binding) => return Ok(binding),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Scope {
    interface: String,
    ipv6: bool,
    tcp: bool,
}
impl Scope {
    pub(crate) fn is_ipv6(&self) -> bool {
        self.ipv6
    }
    pub(crate) fn route_binding(
        binding: SocketAddr,
        destination: SocketAddr,
    ) -> io::Result<SocketAddr> {
        if !binding.ip().is_unspecified() {
            return Ok(binding);
        }
        // UDP connect selects a route without transmitting or creating a mapping.
        let socket = std::net::UdpSocket::bind(SocketAddr::new(binding.ip(), 0))?;
        socket.connect(destination)?;
        let mut source = socket.local_addr()?;
        source.set_port(binding.port());
        Ok(source)
    }
    pub(crate) fn for_binding(
        binding: SocketAddr,
        tcp: bool,
        interfaces: &netwatch::interfaces::State,
    ) -> io::Result<Self> {
        let interface = if binding.ip().is_unspecified() {
            None
        } else {
            interfaces
                .interfaces
                .iter()
                .find(|(_, interface)| {
                    interface.addrs().any(|addr| {
                        addr.addr() == binding.ip()
                            || (binding.ip().is_loopback()
                                && addr.addr().is_loopback()
                                && matches!((&addr, binding.ip()),
                                    (netwatch::interfaces::IpNet::V4(net), std::net::IpAddr::V4(ip))
                                        if net.contains(&ip)))
                    })
                })
                .map(|(name, _)| name.clone())
        }
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                "punching binding has no current local interface",
            )
        })?;
        Ok(Self {
            interface,
            ipv6: binding.is_ipv6(),
            tcp,
        })
    }
}
#[derive(Debug, Default)]
pub(crate) struct Pool {
    active: Mutex<HashMap<Scope, Weak<Owner>>>,
    changed: watch::Sender<()>,
}
#[derive(Debug)]
struct Owner {
    peer: EndpointId,
    changed: watch::Sender<()>,
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.changed.send_modify(|_| {});
    }
}
/// Physical mapping reservation. Finish it after the winning binding is retained.
#[derive(Debug, Clone)]
pub struct MemberPunching {
    owner: Arc<Mutex<Option<Arc<Owner>>>>,
    binding: Option<(SocketAddr, crate::socket::ObservationValidity)>,
}
impl MemberPunching {
    /// Release mapping exclusivity while the original QUIC path validates.
    pub(crate) fn untracked() -> Self {
        Self {
            owner: Arc::new(Mutex::new(None)),
            binding: None,
        }
    }
    pub(crate) fn with_binding(mut self, binding: SocketAddr, validity: crate::socket::ObservationValidity) -> Self {
        self.binding = Some((binding, validity));
        self
    }
    /// Source selected for this physical attempt.
    pub fn binding(&self) -> Option<SocketAddr> {
        self.binding.as_ref().map(|(binding, _)| *binding)
    }

    pub(super) async fn binding_lost_while_held(&self, endpoint: &Endpoint) {
        if let Some((_, validity)) = &self.binding {
            endpoint.inner.wait_for_binding_loss(validity).await;
            if self.owner.lock().expect("physical punching").is_some() {
                return;
            }
        }
        std::future::pending().await
    }
    /// Release the physical mapping reservation after retaining the winning binding.
    pub fn finish(&self) {
        self.owner.lock().expect("physical punching").take();
    }
}
impl Pool {
    #[cfg(feature = "miopunch-test-hooks")]
    pub(super) fn tcp_held(&self, peer: EndpointId, ipv6: bool) -> bool {
        self.active.lock().expect("physical punching pool").iter().any(|(scope, owner)| {
            scope.tcp && scope.ipv6 == ipv6 && owner.upgrade().is_some_and(|owner| owner.peer == peer)
        })
    }

    pub(crate) fn new(changed: watch::Sender<()>) -> Self {
        Self {
            active: Mutex::default(),
            changed,
        }
    }
    pub(crate) fn try_acquire(&self, scope: Scope, peer: EndpointId) -> Option<MemberPunching> {
        let mut active = self.active.lock().expect("physical punching pool");
        let owner = if let Some(owner) = active.get(&scope).and_then(Weak::upgrade) {
            if owner.peer != peer {
                return None;
            }
            owner
        } else {
            active.retain(|_, owner| owner.strong_count() != 0);
            let owner = Arc::new(Owner {
                peer,
                changed: self.changed.clone(),
            });
            active.insert(scope, Arc::downgrade(&owner));
            owner
        };
        Some(MemberPunching {
            owner: Arc::new(Mutex::new(Some(owner))),
            binding: None,
        })
    }
    pub(crate) async fn acquire(&self, scope: Scope, peer: EndpointId) -> MemberPunching {
        let mut changed = self.changed.subscribe();
        loop {
            if let Some(permit) = self.try_acquire(scope.clone(), peer) {
                return permit;
            }
            changed.changed().await.expect("physical pool stays open");
        }
    }
}
