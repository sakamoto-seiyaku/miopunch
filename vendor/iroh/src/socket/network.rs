//! Lifetimes of actual local bindings across interface updates.
use std::{
    collections::{HashMap, HashSet},
    io,
    net::IpAddr,
    sync::{Arc, Mutex, Weak},
};

use netwatch::interfaces::{Interface, IpNet, State};

use super::ObservationValidity;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Binding {
    interface: Interface,
    address: IpNet,
}

fn binding(state: &State, ip: IpAddr) -> Option<Binding> {
    state
        .interfaces
        .values()
        .filter(|interface| interface.is_up())
        .find_map(|interface| {
            interface
                .addrs()
                .find(|address| {
                    address.addr() == ip
                        || (ip.is_loopback()
                            && matches!((address, ip),
                (IpNet::V4(network), IpAddr::V4(ip)) if network.contains(&ip)))
                })
                .map(|address| Binding {
                    interface: interface.clone(),
                    address,
                })
        })
}

pub(super) fn has_binding(state: &State, ip: IpAddr) -> bool {
    binding(state, ip).is_some()
}

#[derive(Debug)]
struct Generation {
    binding: Binding,
    route_source: Option<IpAddr>,
    epoch: Weak<Mutex<u64>>,
}

#[derive(Debug)]
pub(super) struct Bindings {
    state: State,
    generations: HashMap<(IpAddr, Option<std::net::SocketAddr>), Generation>,
    families: [Arc<Mutex<u64>>; 2],
}

impl Bindings {
    pub(super) fn new(state: State) -> Self {
        Self {
            state,
            generations: HashMap::new(),
            families: std::array::from_fn(|_| Arc::default()),
        }
    }

    pub(super) fn capture(&mut self, ip: IpAddr, state: &State) -> io::Result<ObservationValidity> {
        self.capture_route(ip, None, state)
    }

    pub(super) fn capture_route(
        &mut self,
        ip: IpAddr,
        destination: Option<std::net::SocketAddr>,
        state: &State,
    ) -> io::Result<ObservationValidity> {
        let current = binding(state, ip).ok_or(io::ErrorKind::AddrNotAvailable)?;
        let route_source = destination.map(route_source).transpose()?;
        let key = (ip, destination);
        if let Some(generation) = self.generations.get(&key)
            && generation.binding == current
            && generation.route_source == route_source
            && let Some(epoch) = generation.epoch.upgrade()
        {
            return Ok(ObservationValidity {
                observed_generation: *epoch.lock().expect("binding epoch"),
                generation: epoch.clone(),
            });
        }
        let epoch = Arc::new(Mutex::new(0));
        if let Some(previous) = self.generations.insert(
            key,
            Generation {
                binding: current,
                route_source,
                epoch: Arc::downgrade(&epoch),
            },
        ) && let Some(epoch) = previous.epoch.upgrade()
        {
            let mut value = epoch.lock().expect("binding epoch");
            *value = value.wrapping_add(1);
        }
        Ok(ObservationValidity {
            generation: epoch,
            observed_generation: 0,
        })
    }

    pub(super) fn family(&self, ipv6: bool) -> ObservationValidity {
        let generation = self.families[usize::from(ipv6)].clone();
        let observed_generation = *generation.lock().expect("family epoch");
        ObservationValidity {
            generation,
            observed_generation,
        }
    }

    pub(super) fn unchanged_ips(&self, state: &State) -> HashSet<IpAddr> {
        state
            .interfaces
            .values()
            .flat_map(Interface::addrs)
            .map(|address| address.addr())
            .filter(|ip| binding(&self.state, *ip) == binding(state, *ip))
            .collect()
    }

    /// Explicit notifications with no visible difference also report NAT-only changes.
    pub(super) fn update(&mut self, state: State, force_if_unchanged: bool) -> ([bool; 2], bool) {
        let same = self.state == state;
        let forced = same && force_if_unchanged;
        let mut changed = std::array::from_fn(|family| {
            forced
                || self.state.default_route_interface != state.default_route_interface
                || self.state.last_unsuspend != state.last_unsuspend
                || self.state.interfaces.iter().any(|(name, old)| {
                    let before: Vec<_> = old
                        .addrs()
                        .filter(|address| address.addr().is_ipv6() == (family == 1))
                        .collect();
                    state
                        .interfaces
                        .get(name)
                        .map_or(!before.is_empty(), |new| {
                            (!before.is_empty() && old != new)
                                || before
                                    != new
                                        .addrs()
                                        .filter(|address| address.addr().is_ipv6() == (family == 1))
                                        .collect::<Vec<_>>()
                        })
                })
                || state.interfaces.iter().any(|(name, new)| {
                    !self.state.interfaces.contains_key(name)
                        && new
                            .addrs()
                            .any(|address| address.addr().is_ipv6() == (family == 1))
                })
        });
        self.generations.retain(|(ip, destination), generation| {
            let Some(epoch) = generation.epoch.upgrade() else {
                return false;
            };
            if forced
                || binding(&state, *ip).as_ref() != Some(&generation.binding)
                || destination.is_some_and(|destination| {
                    route_source(destination).ok() != generation.route_source
                })
                || (destination.is_none()
                    && self.state.default_route_interface != state.default_route_interface)
                || self.state.last_unsuspend != state.last_unsuspend
            {
                changed[usize::from(ip.is_ipv6())] = true;
                let mut value = epoch.lock().expect("binding epoch");
                *value = value.wrapping_add(1);
                false
            } else {
                true
            }
        });
        for (changed, epoch) in changed.iter().zip(&self.families) {
            if *changed {
                let mut value = epoch.lock().expect("family epoch");
                *value = value.wrapping_add(1);
            }
        }
        self.state = state;
        (changed, forced)
    }
}

fn route_source(destination: std::net::SocketAddr) -> io::Result<IpAddr> {
    let source = if destination.is_ipv6() {
        "[::]:0"
    } else {
        "0.0.0.0:0"
    };
    let socket = std::net::UdpSocket::bind(source)?;
    socket.connect(destination)?;
    Ok(socket.local_addr()?.ip())
}

/// A transport with an unchanged actual source keeps its current QUIC path.
pub(super) fn path_is_affected(
    mapped: &super::MappedAddrs,
    path: noq_proto::FourTuple,
    unchanged: Option<&HashSet<IpAddr>>,
) -> bool {
    let Some(unchanged) = unchanged else {
        return true;
    };
    match mapped.to_transport_addr(path.remote()) {
        Some(super::transports::Addr::Ip(_)) => {
            path.local_ip().is_none_or(|ip| !unchanged.contains(&ip))
        }
        Some(super::transports::Addr::Custom(address)) => {
            !mapped.custom_network_path_is_current(&address, unchanged)
        }
        Some(super::transports::Addr::Relay(..)) => false,
        _ => true,
    }
}
