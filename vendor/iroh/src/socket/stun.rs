//! RFC 8489 Binding, received and sent by the endpoint's retained UDP transport.
use std::{
    collections::{HashMap, HashSet},
    io,
    net::{Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use iroh_base::RelayUrl;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{EndpointInner, ObservationValidity, UdpMappingSample, UdpMappingSequence, transports};
use n0_watcher::Watcher as _;

const COOKIE: [u8; 4] = [0x21, 0x12, 0xa4, 0x42];
type Transaction = [u8; 12];

#[derive(Debug)]
struct Pending {
    destination: SocketAddr,
    sender: mpsc::Sender<(SocketAddr, Vec<u8>)>,
}

#[derive(Debug, Default)]
pub(super) struct Transactions {
    pending: Arc<Mutex<HashMap<Transaction, Pending>>>,
    generation: Arc<Mutex<u64>>,
}

struct Registration {
    pending: Arc<Mutex<HashMap<Transaction, Pending>>>,
    transaction: Transaction,
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.pending
            .lock()
            .expect("STUN transactions")
            .remove(&self.transaction);
    }
}

impl Transactions {
    pub(super) fn receive(&self, destination: SocketAddr, local: SocketAddr, bytes: &[u8]) {
        if bytes.len() < 20 || bytes.len() > 1044 || bytes[4..8] != COOKIE {
            return;
        }
        let transaction: Transaction = bytes[8..20].try_into().expect("header length");
        let pending = self.pending.lock().expect("STUN transactions");
        if let Some(entry) = pending.get(&transaction)
            && entry.destination == destination
        {
            let _ = entry.sender.try_send((local, bytes.to_vec()));
        }
    }
}

/// A validated STUN response from the endpoint's actual primary IPv4 socket.
/// This is an untrusted, short-lived address candidate, never reachability or
/// membership proof. It is separate from the endpoint's QAD observations.
#[derive(Debug, Clone)]
pub struct UdpStunObservation {
    /// Configured service origin, including its DNS name.
    pub observer: RelayUrl,
    /// Resolved request destination and validated response source.
    pub destination: SocketAddr,
    /// Actual receiving socket's destination IP and bound port.
    pub local_addr: SocketAddr,
    /// XOR-MAPPED-ADDRESS returned by the service.
    pub mapped: SocketAddr,
    /// Time at which the matching response arrived.
    pub observed_at: Instant,
    /// Candidate deadline, forty seconds after receipt.
    pub valid_until: Instant,
    validity: ObservationValidity,
    network_validity: ObservationValidity,
    closed: CancellationToken,
}

impl UdpStunObservation {
    /// Recheck before using a candidate. A new STUN request, network change or
    /// endpoint close invalidates previous copies, including after a failed query.
    pub fn is_fresh(&self) -> bool {
        Instant::now() < self.valid_until
            && self.validity.is_current()
            && self.network_validity.is_current()
            && !self.closed.is_cancelled()
    }
}

fn parse(bytes: &[u8], transaction: Transaction) -> io::Result<SocketAddr> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid STUN Binding response");
    if bytes.len() < 20
        || bytes[..2] != [1, 1]
        || bytes[4..8] != COOKIE
        || bytes[8..20] != transaction
    {
        return Err(invalid());
    }
    let length = u16::from_be_bytes(bytes[2..4].try_into().unwrap()) as usize;
    if length > 1024 || length % 4 != 0 || bytes.len() != 20 + length {
        return Err(invalid());
    }
    let mut offset = 20;
    let mut mapped = None;
    while offset < bytes.len() {
        if offset + 4 > bytes.len() {
            return Err(invalid());
        }
        let kind = u16::from_be_bytes(bytes[offset..offset + 2].try_into().unwrap());
        let size = u16::from_be_bytes(bytes[offset + 2..offset + 4].try_into().unwrap()) as usize;
        let end = offset + 4 + size;
        let padded = offset + 4 + ((size + 3) & !3);
        if padded > bytes.len() {
            return Err(invalid());
        }
        let value = &bytes[offset + 4..end];
        match kind {
            0x0020 => {
                if mapped.is_some() || size != 8 || value[..2] != [0, 1] {
                    return Err(invalid());
                }
                let port = u16::from_be_bytes(value[2..4].try_into().unwrap()) ^ 0x2112;
                let ip = Ipv4Addr::from(
                    u32::from_be_bytes(value[4..8].try_into().unwrap()) ^ 0x2112a442,
                );
                if port == 0 || ip.is_unspecified() || ip.is_multicast() || ip.is_broadcast() {
                    return Err(invalid());
                }
                mapped = Some(SocketAddr::from((ip, port)));
            }
            // MAPPED-ADDRESS is optional legacy information; XOR takes precedence.
            0x0001 => {}
            0x8028 => {
                if size != 4 || padded != bytes.len() {
                    return Err(invalid());
                }
                let mut crc = !0u32;
                for byte in &bytes[..offset] {
                    crc ^= u32::from(*byte);
                    for _ in 0..8 {
                        crc = (crc >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(crc & 1));
                    }
                }
                if u32::from_be_bytes(value.try_into().unwrap()) != (!crc ^ 0x5354554e) {
                    return Err(invalid());
                }
            }
            // Ignore optional attributes, reject unknown comprehension-required ones.
            kind if kind >= 0x8000 => {}
            _ => return Err(invalid()),
        }
        offset = padded;
    }
    mapped.ok_or_else(invalid)
}

impl EndpointInner {
    fn begin_udp_stun_observation(&self) -> ObservationValidity {
        let mut generation = self.sock.stun.generation.lock().expect("STUN generation");
        *generation = generation.wrapping_add(1);
        ObservationValidity {
            generation: self.sock.stun.generation.clone(),
            observed_generation: *generation,
        }
    }

    pub(crate) async fn observe_udp_stun(
        &self,
        observer: &RelayUrl,
    ) -> io::Result<UdpStunObservation> {
        self.observe_udp_stun_in_sequence(observer, self.begin_udp_stun_observation())
            .await
    }

    /// One scope for the entire ordered sequence; individual queries must not
    /// invalidate earlier samples. Distinct URL names never imply distinct IPs.
    pub(crate) async fn observe_udp_stun_mapping_sequence(
        &self,
        observers: &[RelayUrl],
    ) -> io::Result<UdpMappingSequence> {
        if observers.len() > 4 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "at most four STUN origins per sequence",
            ));
        }
        let validity = self.begin_udp_stun_observation();
        let network_validity = self.udp_network_generation();
        let mut destinations = HashSet::new();
        let mut samples = Vec::new();
        let mut local_addr = None;
        for observer in observers {
            if !validity.is_current()
                || !network_validity.is_current()
                || self.sock.shutdown.is_closed()
            {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "STUN sequence invalidated",
                ));
            }
            match self
                .observe_udp_stun_in_sequence(observer, validity.clone())
                .await
            {
                Ok(result) => {
                    if local_addr.is_some_and(|address| address != result.local_addr) {
                        return Err(io::Error::other("STUN sequence changed UDP socket"));
                    }
                    local_addr = Some(result.local_addr);
                    eprintln!(
                        "STUN_UDP_OBSERVED observer={} destination={} local_addr={} mapped={} current={}",
                        result.observer,
                        result.destination,
                        result.local_addr,
                        result.mapped,
                        result.is_fresh()
                    );
                    if destinations.insert(result.destination) {
                        samples.push(UdpMappingSample {
                            observer: result.observer.to_string(),
                            destination: result.destination,
                            mapped: result.mapped,
                            observed_at: result.observed_at.into(),
                        });
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied => return Err(error),
                Err(error) => {
                    eprintln!("STUN_UDP_OBSERVATION_FAILED observer={observer} error={error}")
                }
            }
        }
        if samples.len() < 2
            || !validity.is_current()
            || !network_validity.is_current()
            || self.sock.shutdown.is_closed()
        {
            return Err(io::Error::other(
                "insufficient current distinct STUN destinations",
            ));
        }
        let interfaces = self.local_interfaces_watcher.clone().get();
        let valid_until = samples[0].observed_at + Duration::from_secs(40);
        Ok(UdpMappingSequence {
            samples,
            local_ips: interfaces
                .local_addresses
                .regular
                .iter()
                .chain(interfaces.local_addresses.loopback.iter())
                .copied()
                .collect(),
            valid_until,
            validity: validity.clone(),
            network_validity,
            supersession_validity: Some(validity),
            closed: Some(self.sock.shutdown.at_close_start.child_token()),
        })
    }

    async fn observe_udp_stun_in_sequence(
        &self,
        observer: &RelayUrl,
        validity: ObservationValidity,
    ) -> io::Result<UdpStunObservation> {
        if observer.scheme() != "stun"
            || observer.username() != ""
            || observer.password().is_some()
            || !self
                .static_config
                .relay_resource_policy
                .allows(observer, crate::endpoint::RelayResourceUse::Observation)
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "STUN observation resource denied",
            ));
        }
        if !self.sock.ip_bind_addrs.iter().any(SocketAddr::is_ipv4) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "endpoint has no IPv4 UDP transport for STUN",
            ));
        }
        let network_validity = self.udp_network_generation();
        let closed = self.sock.shutdown.at_close_start.child_token();
        let mut sends = 0;
        let operation = async {
            let host = observer
                .host_str()
                .ok_or_else(|| io::Error::other("missing STUN host"))?;
            let ip = if let Ok(ip) = host.parse::<Ipv4Addr>() {
                std::net::IpAddr::V4(ip)
            } else {
                self.sock
                    .dns_resolver
                    .lookup_ipv4(host.to_owned(), Duration::from_secs(2))
                    .await
                    .map_err(io::Error::other)?
                    .next()
                    .ok_or_else(|| io::Error::other("STUN has no IPv4 address"))?
            };
            let destination = SocketAddr::from((ip, observer.port().unwrap_or(3478)));
            if destination.port() == 0 {
                return Err(io::Error::other("invalid STUN port"));
            }
            let transaction: Transaction = rand::random();
            let (sender, mut receiver) = mpsc::channel(8);
            {
                let mut pending = self.sock.stun.pending.lock().expect("STUN transactions");
                if pending.len() >= 8 {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "STUN query limit",
                    ));
                }
                pending.insert(
                    transaction,
                    Pending {
                        destination,
                        sender,
                    },
                );
            }
            let _registration = Registration {
                pending: self.sock.stun.pending.clone(),
                transaction,
            };
            let mut request = vec![0, 1, 0, 0];
            request.extend(COOKIE);
            request.extend(transaction);
            let mut retransmit = tokio::time::interval(Duration::from_millis(500));
            loop {
                tokio::select! {
                    _ = closed.cancelled() => return Err(io::Error::other("endpoint closed during STUN")),
                    _ = retransmit.tick(), if sends < 3 => {
                        if !network_validity.is_current() || !validity.is_current() { return Err(io::Error::other("network or observation changed during STUN")); }
                        let mut sender = self.sock.udp_probe_sender.lock().await;
                        let path = transports::FourTuple::Ip { remote: destination, local: None };
                        let packet = transports::Transmit { ecn: None, contents: &request, segment_size: None };
                        std::future::poll_fn(|cx| std::pin::Pin::new(&mut *sender).poll_send(cx, &path, &packet)).await?;
                        sends += 1;
                        eprintln!("STUN_UDP_SEND destination={destination} endpoint_binding={:?} attempt={sends} sent_to_kernel=true",
                            self.sock.ip_local_addrs().find(SocketAddr::is_ipv4));
                    },
                    packet = receiver.recv() => {
                        let (local_addr, bytes) = packet.ok_or_else(|| io::Error::other("STUN receiver stopped"))?;
                        let mapped = parse(&bytes, transaction)?;
                        let observed_at = Instant::now();
                        let result = UdpStunObservation { observer: observer.clone(), destination, local_addr, mapped,
                            observed_at, valid_until: observed_at + Duration::from_secs(40), validity: validity.clone(),
                            network_validity: network_validity.clone(), closed: closed.clone() };
                        if !result.is_fresh() { return Err(io::Error::other("STUN observation invalidated")); }
                        return Ok(result);
                    },
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(3), operation)
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("STUN Binding timed out after {sends} endpoint socket sends"),
                )
            })?
    }
}
