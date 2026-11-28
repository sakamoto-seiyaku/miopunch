use crate::IpClass;
use hickory_proto::op::Message;
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    collections::HashMap,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4},
    sync::{Arc, RwLock},
};
use thiserror::Error;
use tokio::net::UdpSocket;

pub const MDNS_PORT: u16 = 5353;
pub const MDNS_IPV4: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);
pub const MDNS_IPV6: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 0xfb);

#[derive(Debug)]
#[doc(hidden)]
pub enum IP {
    Ipv4,
    Ipv6,
}

impl std::fmt::Display for IP {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IP::Ipv4 => write!(f, "IPv4"),
            IP::Ipv6 => write!(f, "IPv6"),
        }
    }
}

/// Errors that can occur when creating and configuring an IPv4 or IPv6 socket.
#[derive(Debug, Error)]
pub enum SocketError {
    #[error("{domain}: error creating new socket for")]
    NewSocket {
        domain: IP,
        #[source]
        source: std::io::Error,
    },
    #[error("{domain}: error setting the reuse address")]
    ReuseAddress {
        domain: IP,
        #[source]
        source: std::io::Error,
    },
    #[cfg(unix)]
    #[error("{domain}: error setting the reuse port")]
    ReusePort {
        domain: IP,
        #[source]
        source: std::io::Error,
    },
    #[error("{domain}: error binding the socket for")]
    Bind {
        domain: IP,
        #[source]
        source: std::io::Error,
    },
    #[error("{domain}: error setting multicast loop")]
    SetMulticastLoop {
        domain: IP,
        #[source]
        source: std::io::Error,
    },
    #[error("{domain}: error joining multicast")]
    JoinMulticast {
        domain: IP,
        #[source]
        source: std::io::Error,
    },
    #[error("{domain}: error setting the multicast ttl")]
    MulticastTtl {
        domain: IP,
        #[source]
        source: std::io::Error,
    },
    #[error("{domain}: error setting the multicast interface")]
    SetMulticastIf {
        domain: IP,
        #[source]
        source: std::io::Error,
    },
    #[error("{domain}: error setting the socket to non-blocking mode")]
    SetNonBlocking {
        domain: IP,
        #[source]
        source: std::io::Error,
    },
    #[error("{domain}: error creating a UDP socket from a standard socket")]
    UdpSocket {
        domain: IP,
        #[source]
        source: std::io::Error,
    },
    #[error("Cannot bind to IPv4 or IPv6")]
    CannotBind,
}

pub fn socket_v4(interface_addr: Option<Ipv4Addr>) -> Result<UdpSocket, SocketError> {
    // Make sure we bind to a specific interface if specified
    let bind_addr = match interface_addr {
        Some(addr) => SocketAddrV4::new(addr, MDNS_PORT).into(),
        None => SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, MDNS_PORT).into(),
    };

    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).map_err(|source| {
        SocketError::NewSocket {
            domain: IP::Ipv4,
            source,
        }
    })?;
    socket
        .set_reuse_address(true)
        .map_err(|source| SocketError::ReuseAddress {
            domain: IP::Ipv4,
            source,
        })?;
    // Best-effort: some platforms (e.g. esp-idf/lwIP) declare SO_REUSEPORT but
    // leave it unimplemented (setsockopt returns ENOPROTOOPT). It is not needed
    // for a single process binding the mDNS port, and a genuine "port in use"
    // still surfaces at bind(), so tolerate a failure here instead of aborting.
    #[cfg(unix)]
    if let Err(e) = socket.set_reuse_port(true) {
        tracing::debug!("set_reuse_port (IPv4) failed, continuing: {e}");
    }
    socket
        .bind(&bind_addr)
        .map_err(|source| SocketError::Bind {
            domain: IP::Ipv4,
            source,
        })?;
    socket
        .set_multicast_loop_v4(true)
        .map_err(|source| SocketError::SetMulticastLoop {
            domain: IP::Ipv4,
            source,
        })?;

    // Join the multicast group once, on the interface of the default route.
    //
    // Group membership is per interface, so packets arriving on any other
    // interface are not delivered to this socket. IP_MULTICAST_ALL would
    // paper over this on Linux when another membership exists, but that
    // socket option is Linux-only; on macOS and other BSD-derived systems
    // reception from non-default interfaces does not work. The per-interface
    // sockets cannot take over reception either: they are bound to a unicast
    // address and therefore never match multicast-destined packets. Reception
    // on further interfaces is instead set up by
    // `Sockets::join_group_on_main_v4`, which joins the group per interface on
    // this main wildcard socket.
    if let Err(source) =
        socket.join_multicast_v4(&MDNS_IPV4, &interface_addr.unwrap_or(Ipv4Addr::UNSPECIFIED))
    {
        // Offline LANs may have no default route. Keep the wildcard receiver
        // so explicit per-interface memberships can still receive multicast.
        if interface_addr.is_some() {
            return Err(SocketError::JoinMulticast {
                domain: IP::Ipv4,
                source,
            });
        }
        tracing::debug!("default IPv4 multicast membership unavailable: {source}");
    }

    // Pin multicast egress to the requested interface. Binding to the
    // interface address alone does not reliably select the outgoing
    // interface for multicast: the kernel falls back to the routing table,
    // which can pick a different interface or fail with "no route to host"
    // (for example for interfaces without a multicast route entry).
    if let Some(addr) = interface_addr {
        socket
            .set_multicast_if_v4(&addr)
            .map_err(|source| SocketError::SetMulticastIf {
                domain: IP::Ipv4,
                source,
            })?;
    }

    socket
        .set_multicast_ttl_v4(16)
        .map_err(|source| SocketError::MulticastTtl {
            domain: IP::Ipv4,
            source,
        })?;
    socket
        .set_nonblocking(true)
        .map_err(|source| SocketError::SetNonBlocking {
            domain: IP::Ipv4,
            source,
        })?;
    UdpSocket::from_std(std::net::UdpSocket::from(socket)).map_err(|source| {
        SocketError::UdpSocket {
            domain: IP::Ipv4,
            source,
        }
    })
}

pub fn socket_v6() -> Result<UdpSocket, SocketError> {
    let socket = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP)).map_err(|source| {
        SocketError::NewSocket {
            domain: IP::Ipv6,
            source,
        }
    })?;
    socket
        .set_reuse_address(true)
        .map_err(|source| SocketError::ReuseAddress {
            domain: IP::Ipv6,
            source,
        })?;
    // See the IPv4 path: set_reuse_port is best-effort (unimplemented on some
    // platforms, e.g. esp-idf/lwIP), so tolerate a failure here.
    #[cfg(unix)]
    if let Err(e) = socket.set_reuse_port(true) {
        tracing::debug!("set_reuse_port (IPv6) failed, continuing: {e}");
    }
    socket
        .bind(&SocketAddr::from((Ipv6Addr::UNSPECIFIED, MDNS_PORT)).into())
        .map_err(|source| SocketError::Bind {
            domain: IP::Ipv6,
            source,
        })?;
    socket
        .set_multicast_loop_v6(true)
        .map_err(|source| SocketError::SetMulticastLoop {
            domain: IP::Ipv6,
            source,
        })?;

    // A link-local-only host need not have a default multicast route.
    // Specific interface memberships are installed by set_ipv6_interfaces.
    if let Err(error) = socket.join_multicast_v6(&MDNS_IPV6, 0) {
        tracing::debug!("default IPv6 multicast membership unavailable: {error}");
    }

    socket
        .set_nonblocking(true)
        .map_err(|source| SocketError::SetNonBlocking {
            domain: IP::Ipv6,
            source,
        })?;
    UdpSocket::from_std(std::net::UdpSocket::from(socket)).map_err(|source| {
        SocketError::UdpSocket {
            domain: IP::Ipv6,
            source,
        }
    })
}

#[derive(Clone, Debug)]
pub struct Sockets {
    v4: Option<Arc<UdpSocket>>,
    v6: Option<Arc<UdpSocket>>,
    interfaces_v6: Arc<RwLock<Option<Vec<u32>>>>,
    joined_v6: Arc<RwLock<Vec<u32>>>,
    interface_sockets_v4: Arc<RwLock<HashMap<Ipv4Addr, Arc<UdpSocket>>>>,
}

impl Sockets {
    pub fn new(class: IpClass, multicast_interfaces: Vec<Ipv4Addr>) -> Result<Self, SocketError> {
        // Create interface-specific sockets for multi-interface mode
        let mut interface_sockets_v4 = HashMap::new();
        for addr in &multicast_interfaces {
            match socket_v4(Some(*addr)) {
                Ok(socket) => {
                    tracing::debug!("Created interface-specific socket for {}", addr);
                    interface_sockets_v4.insert(*addr, Arc::new(socket));
                }
                Err(e) => {
                    tracing::warn!("Failed to create interface socket for {}: {}", addr, e);
                }
            }
        }
        let interface_sockets_v4 = Arc::new(RwLock::new(interface_sockets_v4));

        let sockets = match class {
            IpClass::Auto => {
                let socket = Self {
                    interfaces_v6: Default::default(),
                    joined_v6: Default::default(),
                    v4: socket_v4(None).ok().map(Arc::new),
                    v6: socket_v6().ok().map(Arc::new),
                    interface_sockets_v4: interface_sockets_v4.clone(),
                };
                if socket.v4.is_none() && socket.v6.is_none() {
                    return Err(SocketError::CannotBind);
                }
                socket
            }
            _ => Self {
                interfaces_v6: Default::default(),
                joined_v6: Default::default(),
                v4: class
                    .has_v4()
                    .then(|| socket_v4(None).map(Arc::new))
                    .transpose()?,
                v6: class
                    .has_v6()
                    .then(|| socket_v6().map(Arc::new))
                    .transpose()?,
                interface_sockets_v4: interface_sockets_v4.clone(),
            },
        };
        for addr in sockets.get_all_interface_addresses_v4() {
            sockets.join_group_on_main_v4(addr);
        }
        Ok(sockets)
    }

    pub fn v4(&self) -> Option<Arc<UdpSocket>> {
        self.v4.as_ref().map(Arc::clone)
    }

    pub fn v6(&self) -> Option<Arc<UdpSocket>> {
        self.v6.as_ref().map(Arc::clone)
    }

    /// Joins the multicast group on the given interface on the main wildcard
    /// IPv4 socket.
    ///
    /// Group membership is per interface: the wildcard socket initially only
    /// joins the group on the interface of the default route, so multicast
    /// arriving on other interfaces is not delivered to it (except on
    /// platforms with `IP_MULTICAST_ALL` semantics and an existing
    /// system-wide membership). The per-interface sockets cannot take over
    /// reception either, because they are bound to a unicast address and
    /// therefore never match multicast-destined packets. Joining the group
    /// per interface on the wildcard socket makes reception work on all
    /// interfaces.
    fn join_group_on_main_v4(&self, addr: Ipv4Addr) {
        if let Some(v4) = &self.v4 {
            match v4.join_multicast_v4(MDNS_IPV4, addr) {
                Ok(()) => tracing::debug!("joined multicast group on interface {}", addr),
                // Already being a member on this interface (e.g. the default
                // interface joined at socket creation) is not a problem.
                Err(e) => tracing::debug!("could not join multicast group on {}: {}", addr, e),
            }
        }
    }

    /// Leaves the multicast group on the given interface on the main wildcard
    /// IPv4 socket. See [`Self::join_group_on_main_v4`].
    fn leave_group_on_main_v4(&self, addr: Ipv4Addr) {
        if let Some(v4) = &self.v4 {
            if let Err(e) = v4.leave_multicast_v4(MDNS_IPV4, addr) {
                tracing::debug!("could not leave multicast group on {}: {}", addr, e);
            }
        }
    }

    /// Add a new IPv4 interface for multicast operations.
    /// Returns Ok(()) if the socket was successfully created and added.
    pub fn add_interface_v4(&self, addr: Ipv4Addr) -> Result<(), SocketError> {
        // Check if interface already exists
        if self
            .interface_sockets_v4
            .read()
            .unwrap()
            .contains_key(&addr)
        {
            return Ok(());
        }

        // Create the interface-specific socket for sending
        let socket = socket_v4(Some(addr))?;

        {
            let mut interfaces = self.interface_sockets_v4.write().unwrap();
            // need to recheck since we dropped the lock in between
            interfaces.entry(addr).or_insert_with(|| {
                tracing::info!("Added interface {} for multicast", addr);
                Arc::new(socket)
            });
        }
        self.join_group_on_main_v4(addr);
        Ok(())
    }

    /// Remove an IPv4 interface from multicast operations.
    /// Returns true if the interface was found and removed.
    pub fn remove_interface_v4(&self, addr: Ipv4Addr) -> bool {
        let mut interfaces = self.interface_sockets_v4.write().unwrap();

        if interfaces.contains_key(&addr) {
            let socket = interfaces.remove(&addr);
            drop(interfaces);
            // drop socket outside the lock
            drop(socket);
            self.leave_group_on_main_v4(addr);
            tracing::info!("Removed interface {} from multicast", addr);

            true
        } else {
            false
        }
    }

    /// Get the socket for a specific IPv4 interface
    pub fn get_interface_socket_v4(&self, addr: Ipv4Addr) -> Option<Arc<UdpSocket>> {
        let interfaces = self.interface_sockets_v4.read().unwrap();
        match interfaces.get(&addr) {
            Some(sock) => Some(Arc::clone(sock)),
            None => None,
        }
    }

    /// Get all interface addresses that have sockets
    pub fn get_all_interface_addresses_v4(&self) -> Vec<Ipv4Addr> {
        let interfaces = self.interface_sockets_v4.read().unwrap();
        interfaces.keys().copied().collect()
    }

    pub fn set_ipv6_interfaces(&self, interfaces: &[u32]) {
        let mut joined = self.joined_v6.write().unwrap();
        let active = match &self.v6 {
            Some(socket) => reconcile_ipv6_interfaces(&mut joined, interfaces, |index, action| {
                let result = match action {
                    MembershipChange::Join => socket.join_multicast_v6(&MDNS_IPV6, index),
                    MembershipChange::Leave => socket.leave_multicast_v6(&MDNS_IPV6, index),
                };
                if let Err(error) = &result {
                    tracing::debug!(
                        index,
                        ?action,
                        "IPv6 multicast membership update failed: {error}"
                    );
                }
                result
            }),
            None => Vec::new(),
        };
        // None still means unconfigured/default-interface behavior. Once an
        // explicit snapshot arrives, send only on desired, successfully joined
        // interfaces; failed retirements remain tracked solely for retry.
        *self.interfaces_v6.write().unwrap() = Some(active);
    }

    pub async fn send_msg(&self, msg: &Message, mode: Mode) {
        let bytes = match msg.to_vec() {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!("error serializing mDNS: {error}");
                return;
            }
        };
        let multi_v4 = !self.interface_sockets_v4.read().unwrap().is_empty();
        if matches!(mode, Mode::V4 | Mode::Any) {
            if multi_v4 {
                self.send_msg_multi_interface_v4(&bytes, msg).await;
            } else if let Some(socket) = &self.v4 {
                if let Err(error) = socket.send_to(&bytes, (MDNS_IPV4, MDNS_PORT)).await {
                    tracing::debug!("IPv4 multicast send failed: {error}");
                }
            }
        }
        if matches!(mode, Mode::V6 | Mode::Any) {
            if let Some(socket) = &self.v6 {
                let interfaces = self.interfaces_v6.read().unwrap().clone();
                match interfaces {
                    Some(interfaces) => {
                        for index in interfaces {
                            let destination =
                                std::net::SocketAddrV6::new(MDNS_IPV6, MDNS_PORT, 0, index);
                            if let Err(error) = socket.send_to(&bytes, destination).await {
                                tracing::debug!(index, "IPv6 multicast send failed: {error}");
                            }
                        }
                    }
                    None if matches!(mode, Mode::V6) || self.v4.is_none() || multi_v4 => {
                        // Unconfigured library consumers retain their original
                        // default-interface multicast behavior.
                        if let Err(error) = socket.send_to(&bytes, (MDNS_IPV6, MDNS_PORT)).await {
                            tracing::debug!("default IPv6 multicast send failed: {error}");
                        }
                    }
                    None => {}
                }
            }
        }
    }

    async fn send_msg_multi_interface_v4(&self, bytes: &[u8], msg: &Message) {
        let mut sent_count = 0;

        // Send on all IPv4 interface-specific sockets
        let interfaces = self.interface_sockets_v4.read().unwrap().clone();
        for (addr, socket) in interfaces.iter() {
            if let Err(e) = socket.send_to(bytes, (MDNS_IPV4, MDNS_PORT)).await {
                tracing::error!("error sending mDNS on interface {}: {}", addr, e);
            } else {
                tracing::debug!(
                    addr = %addr,
                    q = msg.queries.len(),
                    an = msg.answers.len(),
                    ad = msg.additionals.len(),
                    "sent {} bytes on interface",
                    bytes.len()
                );
                sent_count += 1;
            }
        }

        if sent_count == 0 {
            tracing::error!("failed to send mDNS on any IPv4 interface in multi-interface mode");
        }
    }
}

#[derive(Debug)]
pub enum Mode {
    V4,
    V6,
    Any,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MembershipChange {
    Join,
    Leave,
}

fn reconcile_ipv6_interfaces(
    joined: &mut Vec<u32>,
    desired: &[u32],
    mut update: impl FnMut(u32, MembershipChange) -> std::io::Result<()>,
) -> Vec<u32> {
    joined.retain(|index| {
        if desired.contains(index) {
            return true;
        }
        match update(*index, MembershipChange::Leave) {
            Ok(()) => false,
            Err(error) => !membership_already_absent(&error),
        }
    });
    let mut active = Vec::new();
    for &index in desired {
        if active.contains(&index) {
            continue;
        }
        if !joined.contains(&index) {
            match update(index, MembershipChange::Join) {
                Ok(()) => joined.push(index),
                // The socket may already hold this membership through its
                // initial default-interface join (index 0).
                Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => joined.push(index),
                Err(_) => {}
            }
        }
        if joined.contains(&index) {
            active.push(index);
        }
    }
    active
}

fn membership_already_absent(error: &std::io::Error) -> bool {
    if matches!(
        error.kind(),
        std::io::ErrorKind::AddrNotAvailable | std::io::ErrorKind::NotFound
    ) {
        return true;
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        // Linux/Android ENODEV: the interface disappeared before retirement.
        const ENODEV: i32 = 19;
        if error.raw_os_error() == Some(ENODEV) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn explicit_empty_ipv6_snapshot_stays_configured() {
        let sockets = Sockets {
            v4: None,
            v6: None,
            interfaces_v6: Default::default(),
            joined_v6: Default::default(),
            interface_sockets_v4: Default::default(),
        };
        assert!(sockets.interfaces_v6.read().unwrap().is_none());
        sockets.set_ipv6_interfaces(&[]);
        assert_eq!(*sockets.interfaces_v6.read().unwrap(), Some(vec![]));
    }

    #[test]
    fn unchanged_ipv6_snapshot_retries_failed_joins_without_rejoining_successes() {
        let mut joined = Vec::new();
        let mut calls = Vec::new();
        let first = reconcile_ipv6_interfaces(&mut joined, &[2, 3], |index, action| {
            calls.push((index, action));
            if index == 3 {
                Err(std::io::ErrorKind::NetworkUnreachable.into())
            } else {
                Ok(())
            }
        });
        assert_eq!(first, [2]);
        assert_eq!(joined, [2]);
        let next = reconcile_ipv6_interfaces(&mut joined, &[2, 3], |index, action| {
            calls.push((index, action));
            Ok(())
        });
        assert_eq!(next, [2, 3]);
        assert_eq!(joined, [2, 3]);
        assert_eq!(
            calls,
            [
                (2, MembershipChange::Join),
                (3, MembershipChange::Join),
                (3, MembershipChange::Join)
            ]
        );
    }

    #[test]
    fn failed_ipv6_leave_stops_sending_and_retries_on_the_next_snapshot() {
        let mut joined = vec![2, 3];
        let mut calls = Vec::new();
        let active = reconcile_ipv6_interfaces(&mut joined, &[3, 4], |index, action| {
            calls.push((index, action));
            if action == MembershipChange::Leave {
                Err(std::io::ErrorKind::Interrupted.into())
            } else {
                Ok(())
            }
        });
        assert_eq!(active, [3, 4]);
        assert_eq!(joined, [2, 3, 4]);
        let active = reconcile_ipv6_interfaces(&mut joined, &[3, 4], |index, action| {
            calls.push((index, action));
            Ok(())
        });
        assert_eq!(active, [3, 4]);
        assert_eq!(joined, [3, 4]);
        assert_eq!(
            calls,
            [
                (2, MembershipChange::Leave),
                (4, MembershipChange::Join),
                (2, MembershipChange::Leave)
            ]
        );
    }

    #[test]
    fn default_ipv6_membership_already_joined_is_active_and_not_joined_twice() {
        let mut joined = Vec::new();
        let active = reconcile_ipv6_interfaces(&mut joined, &[2], |index, action| {
            assert_eq!((index, action), (2, MembershipChange::Join));
            Err(std::io::ErrorKind::AddrInUse.into())
        });
        assert_eq!(active, [2]);
        assert_eq!(joined, [2]);
        let active = reconcile_ipv6_interfaces(&mut joined, &[2], |_, _| {
            panic!("successful default membership must not be joined again")
        });
        assert_eq!(active, [2]);
    }

    #[test]
    fn absent_ipv6_membership_retires_and_reused_index_joins_again() {
        let mut errors = vec![
            std::io::Error::from(std::io::ErrorKind::AddrNotAvailable),
            std::io::Error::from(std::io::ErrorKind::NotFound),
        ];
        #[cfg(any(target_os = "linux", target_os = "android"))]
        errors.push(std::io::Error::from_raw_os_error(19));
        for error in errors {
            let mut joined = vec![2];
            let mut failure = Some(error);
            let active = reconcile_ipv6_interfaces(&mut joined, &[], |_, action| {
                assert_eq!(action, MembershipChange::Leave);
                Err(failure.take().unwrap())
            });
            assert!(active.is_empty());
            assert!(joined.is_empty());
            let mut joins = 0;
            let active = reconcile_ipv6_interfaces(&mut joined, &[2], |index, action| {
                assert_eq!((index, action), (2, MembershipChange::Join));
                joins += 1;
                Ok(())
            });
            assert_eq!(active, [2]);
            assert_eq!(joins, 1);
        }
    }
}
