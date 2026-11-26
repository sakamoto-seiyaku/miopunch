//! RFC 8489 Binding, received and sent by the endpoint's retained UDP transport.
use std::{
    collections::{HashMap, HashSet},
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use iroh_base::RelayUrl;
use iroh_relay::ObservationFamily;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{
    EndpointInner, ObservationValidity, UdpMappingSample, UdpMappingSequence, net_report,
    transports,
};
use futures_util::{FutureExt as _, StreamExt as _, stream::FuturesUnordered};
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
    generation: [Arc<Mutex<u64>>; 2],
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
                if mapped.is_some() || size < 4 || value[0] != 0 {
                    return Err(invalid());
                }
                let port = u16::from_be_bytes(value[2..4].try_into().unwrap()) ^ 0x2112;
                let ip = match (value[1], size) {
                    (1, 8) => IpAddr::V4(Ipv4Addr::from(
                        u32::from_be_bytes(value[4..8].try_into().unwrap()) ^ 0x2112a442,
                    )),
                    (2, 20) => {
                        let mut ip = [0; 16];
                        for index in 0..16 {
                            ip[index] = value[index + 4] ^ bytes[index + 4];
                        }
                        IpAddr::V6(Ipv6Addr::from(ip))
                    }
                    _ => return Err(invalid()),
                };
                if port == 0
                    || ip.is_unspecified()
                    || ip.is_multicast()
                    || matches!(ip, IpAddr::V4(ip) if ip.is_broadcast())
                {
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
    fn begin_udp_stun_observation(&self, family: ObservationFamily) -> ObservationValidity {
        let state = &self.sock.stun.generation[usize::from(family == ObservationFamily::Ipv6)];
        let mut generation = state.lock().expect("STUN generation");
        *generation = generation.wrapping_add(1);
        ObservationValidity {
            generation: state.clone(),
            observed_generation: *generation,
        }
    }

    pub(crate) async fn resolve_observation_addr(
        &self,
        observer: &RelayUrl,
        family: ObservationFamily,
        default_port: u16,
    ) -> io::Result<SocketAddr> {
        let addresses = self
            .static_config
            .relay_resource_policy
            .observation_addresses(observer);
        let explicit = match family {
            ObservationFamily::Ipv4 => addresses.ipv4.map(IpAddr::V4),
            ObservationFamily::Ipv6 => addresses.ipv6.map(IpAddr::V6),
        };
        let ip = match explicit {
            Some(ip) => ip,
            None => match observer.host() {
                Some(url::Host::Ipv4(ip)) if family == ObservationFamily::Ipv4 => IpAddr::V4(ip),
                Some(url::Host::Ipv6(ip)) if family == ObservationFamily::Ipv6 => IpAddr::V6(ip),
                Some(url::Host::Domain(host)) => match family {
                    ObservationFamily::Ipv4 => self
                        .sock
                        .dns_resolver
                        .lookup_ipv4(host.to_owned(), Duration::from_secs(2))
                        .await
                        .map(|mut ips| ips.next()),
                    ObservationFamily::Ipv6 => self
                        .sock
                        .dns_resolver
                        .lookup_ipv6(host.to_owned(), Duration::from_secs(2))
                        .await
                        .map(|mut ips| ips.next()),
                }
                .map_err(io::Error::other)?
                .ok_or_else(|| io::Error::other("observer has no address for family"))?,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrNotAvailable,
                        "observer has no address for family",
                    ));
                }
            },
        };
        let destination = SocketAddr::new(ip, observer.port().unwrap_or(default_port));
        if destination.port() == 0 || ip.is_unspecified() || ip.is_multicast() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid observer destination",
            ));
        }
        Ok(destination)
    }

    pub(crate) async fn observe_udp_stun_for_family(
        &self,
        observer: &RelayUrl,
        family: ObservationFamily,
    ) -> io::Result<UdpStunObservation> {
        self.observe_udp_stun_in_sequence(
            observer,
            family,
            self.begin_udp_stun_observation(family),
            &|| {},
            None,
        )
        .await
    }

    pub(crate) async fn observe_udp_stun(
        &self,
        observer: &RelayUrl,
    ) -> io::Result<UdpStunObservation> {
        let family = if matches!(observer.host(), Some(url::Host::Ipv6(_)))
            || !self.sock.ip_bind_addrs.iter().any(SocketAddr::is_ipv4)
        {
            ObservationFamily::Ipv6
        } else {
            ObservationFamily::Ipv4
        };
        self.observe_udp_stun_for_family(observer, family).await
    }

    pub(crate) async fn observe_udp_mapping_round(
        &self,
        qad: &iroh_relay::RelayMap,
        stun: &[RelayUrl],
        on_work: &(impl Fn() + Sync),
        qad_submission: Option<&super::transports::QadSubmission>,
    ) -> io::Result<UdpMappingSequence> {
        self.observe_udp_mapping_round_for_family(
            qad,
            stun,
            ObservationFamily::Ipv4,
            on_work,
            qad_submission,
            crate::endpoint::nat_plan::MAX_OBSERVATIONS,
            tokio::time::Instant::now() + Duration::from_secs(8),
        )
        .await
    }

    pub(crate) async fn observe_udp_mapping_round_for_family(
        &self,
        qad: &iroh_relay::RelayMap,
        stun: &[RelayUrl],
        family: ObservationFamily,
        on_work: &(impl Fn() + Sync),
        qad_submission: Option<&super::transports::QadSubmission>,
        observation_limit: usize,
        deadline: tokio::time::Instant,
    ) -> io::Result<UdpMappingSequence> {
        // Both request types use the requested family of the endpoint transport. A single binding
        // makes that physical socket unambiguous even for wildcard addresses.
        let mut bindings = self
            .sock
            .ip_local_addrs()
            .filter(|addr| family.matches(*addr));
        let binding = bindings
            .next()
            .filter(|_| bindings.next().is_none())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    "mixed observation requires one binding for the requested family",
                )
            })?;
        let network_validity = self.udp_network_generation();
        let validity = self.udp_observation_generation();
        let closed = self.sock.shutdown.at_close_start.child_token();
        let allowed = |url: &RelayUrl| {
            self.static_config
                .relay_resource_policy
                .allows(url, crate::endpoint::RelayResourceUse::Observation)
        };
        let relays: Vec<_> = qad
            .relays::<Vec<_>>()
            .into_iter()
            .filter(|r| allowed(&r.url))
            .collect();
        let stun: Vec<_> = stun.iter().filter(|url| allowed(url)).collect();
        if relays.is_empty() && stun.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no available mapping observers",
            ));
        }
        let round_validity = self.begin_udp_stun_observation(family);
        let check = || -> io::Result<()> {
            if closed.is_cancelled()
                || !network_validity.is_current()
                || !round_validity.is_current()
            {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "mapping observation invalidated",
                ));
            }
            if relays.iter().any(|r| !allowed(&r.url)) || stun.iter().any(|url| !allowed(url)) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "mapping observation permission changed",
                ));
            }
            Ok(())
        };
        check()?;
        let destinations = Arc::new(Mutex::new((observation_limit, HashSet::new())));
        let mut pending_probes = Vec::new();
        for relay in &relays {
            let probe = net_report::observe_mapping_sample(
                relay.clone(),
                self.qad_client.clone(),
                self.sock.dns_resolver.clone(),
                closed.child_token(),
                qad_submission,
                family,
                Some(&destinations),
            );
            pending_probes.push(
                async move {
                    let sample = probe.await?;
                    Ok((
                        UdpMappingSample {
                            observer: sample.observer,
                            destination: sample.destination,
                            mapped: sample.mapped,
                            observed_at: sample.observed_at,
                        },
                        None,
                    ))
                }
                .boxed(),
            );
        }
        for observer in &stun {
            let probe = self.observe_udp_stun_in_sequence(
                observer,
                family,
                round_validity.clone(),
                on_work,
                Some(&destinations),
            );
            pending_probes.push(
                async move {
                    let sample = probe.await?;
                    if sample.local_addr.port() != binding.port()
                        || (!binding.ip().is_unspecified()
                            && sample.local_addr.ip() != binding.ip())
                    {
                        return Err(io::Error::other("STUN received on a different UDP binding"));
                    }
                    Ok((
                        UdpMappingSample {
                            observer: sample.observer.to_string(),
                            destination: sample.destination,
                            mapped: sample.mapped,
                            observed_at: sample.observed_at.into(),
                        },
                        Some(n0_future::time::Instant::from(sample.valid_until)),
                    ))
                }
                .boxed(),
            );
        }
        let waves = pending_probes.len().div_ceil(4).max(1) as u32;
        let probe_budget = deadline.saturating_duration_since(tokio::time::Instant::now()) / waves;
        let mut pending_probes = pending_probes.into_iter().map(|probe| async move {
            tokio::time::timeout(probe_budget, probe)
                .await
                .map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::TimedOut,
                        "observation station time slice expired",
                    )
                })?
        });
        let mut probes = FuturesUnordered::new();
        for probe in pending_probes.by_ref().take(4) {
            probes.push(probe);
        }
        let mut samples: Vec<(UdpMappingSample, Option<n0_future::time::Instant>)> =
            Vec::with_capacity(2);
        let mut recheck = tokio::time::interval(Duration::from_millis(10));
        while !probes.is_empty() {
            tokio::select! {
                biased;
                _ = closed.cancelled() => check()?,
                _ = recheck.tick() => check()?,
                result = probes.next() => {
                    check()?;
                    match result {
                        Some(Ok((sample, expiry))) if family.matches(sample.mapped)
                            && sample.mapped.port() != 0 && !sample.mapped.ip().is_unspecified()
                            && !samples.iter().any(|(s, _)| s.destination == sample.destination) =>
                        {
                            samples.push((sample, expiry));
                            if samples.len() == 2 {
                                break;
                            }
                        }
                        Some(Err(error)) if error.kind() == io::ErrorKind::PermissionDenied => return Err(error),
                        _ => {}
                    }
                    if let Some(probe) = pending_probes.next() { probes.push(probe); }
                }
            }
        }
        drop(probes);
        check()?;
        samples.sort_by_key(|(sample, _)| sample.observed_at);
        let stun_deadline = samples.iter().filter_map(|(_, expiry)| *expiry).min();
        let last = samples
            .last()
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "no current mapping observations")
            })?
            .0
            .observed_at;
        let interfaces = self.local_interfaces_watcher.clone().get();
        Ok(UdpMappingSequence {
            samples: samples.into_iter().map(|(sample, _)| sample).collect(),
            local_ips: interfaces
                .local_addresses
                .regular
                .iter()
                .chain(interfaces.local_addresses.loopback.iter())
                .copied()
                .collect(),
            valid_until: stun_deadline.unwrap_or(last + Duration::from_secs(2)),
            validity,
            network_validity,
            supersession_validity: Some(round_validity),
            traversal_deadline: stun_deadline,
            closed: Some(closed),
        })
    }

    /// One scope for the entire ordered sequence; individual queries must not
    /// invalidate earlier samples. Distinct URL names never imply distinct IPs.
    pub(crate) async fn observe_udp_stun_mapping_sequence(
        &self,
        observers: &[RelayUrl],
        on_work: &(impl Fn() + Sync),
    ) -> io::Result<UdpMappingSequence> {
        let validity = self.begin_udp_stun_observation(ObservationFamily::Ipv4);
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
                .observe_udp_stun_in_sequence(
                    observer,
                    ObservationFamily::Ipv4,
                    validity.clone(),
                    on_work,
                    None,
                )
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
            traversal_deadline: Some(valid_until),
            closed: Some(self.sock.shutdown.at_close_start.child_token()),
        })
    }

    async fn observe_udp_stun_in_sequence(
        &self,
        observer: &RelayUrl,
        family: ObservationFamily,
        validity: ObservationValidity,
        on_work: &(impl Fn() + Sync),
        destinations: Option<&Arc<Mutex<(usize, HashSet<SocketAddr>)>>>,
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
        if !self
            .sock
            .ip_bind_addrs
            .iter()
            .any(|addr| family.matches(*addr))
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "endpoint has no UDP transport for the observer family",
            ));
        }
        let network_validity = self.udp_network_generation();
        let closed = self.sock.shutdown.at_close_start.child_token();
        let mut sends = 0;
        let operation = async {
            let destination = self
                .resolve_observation_addr(observer, family, 3478)
                .await?;
            if let Some(destinations) = destinations {
                let mut destinations = destinations.lock().expect("mapping destinations");
                if destinations.1.len() >= destinations.0 || !destinations.1.insert(destination) {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "duplicate observation destination",
                    ));
                }
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
                        if sends == 0 { on_work(); }
                        sends += 1;
                        eprintln!("STUN_UDP_SEND destination={destination} endpoint_binding={:?} attempt={sends} sent_to_kernel=true",
                            self.sock.ip_local_addrs().find(|addr| family.matches(*addr)));
                    },
                    packet = receiver.recv() => {
                        let (local_addr, bytes) = packet.ok_or_else(|| io::Error::other("STUN receiver stopped"))?;
                        let mapped = parse(&bytes, transaction)?;
                        if !family.matches(mapped) { return Err(io::Error::other("STUN mapping family mismatch")); }
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
