//! Validate new carriers on the already admitted QUIC connection.

use std::{
    collections::BTreeSet,
    io,
    sync::{Arc, Mutex},
};

use futures_util::{StreamExt, stream::FuturesUnordered};
use iroh_base::CustomAddr;
use tokio::{
    sync::{Notify, mpsc, watch},
    time::{self, Duration},
};

use super::{
    Connection, ControlExchange, ControlHandle, ControlMessageFormat, RecvStream, RelayResourceUse,
    StreamKind, TransportAddr,
};
use crate::endpoint::{Endpoint, ObservationValidity};

const PREFACE: &[u8] = b"iroh/member-direct/2\0";
const MAX_CANDIDATES: usize = 2;
const MAX_DIRECT_ADDRESS: usize = 32;
const MAX_OFFER_ADDRESS: usize = 64;
const EXCHANGE_LIMIT: Duration = Duration::from_secs(6);
const VALIDATION_LIMIT: Duration = Duration::from_secs(6);
const TRAVERSAL_STATUS: ControlMessageFormat = ControlMessageFormat::Fixed { length: 1 };
type Hint = (ObservationValidity, TransportAddr);

enum PeerOffer {
    Candidate(Hint),
    Traversal,
}

#[derive(Debug)]
pub(in crate::endpoint) struct PathCandidates {
    endpoint: Endpoint,
    direct: Option<u64>,
    member_relay: Option<u64>,
    native_relay: bool,
    dialer: bool,
    sender: mpsc::UnboundedSender<Hint>,
    learned: Mutex<(ObservationValidity, BTreeSet<TransportAddr>)>,
    traversal: Option<(Duration, Duration)>,
    node_traversal_slot: Arc<tokio::sync::Mutex<()>>,
    activity: Arc<super::Activity>,
    last_traversal: Mutex<Option<(ObservationValidity, time::Instant)>>,
}

impl PathCandidates {
    pub(super) fn supports_traversal(&self) -> bool {
        self.traversal.is_some()
    }

    pub(super) fn learn(&self, address: &TransportAddr) {
        if !self.permits(address) {
            return;
        }
        if self.dialer {
            self.enqueue(address.clone());
        } else if let TransportAddr::Custom(remote) = address
            && self.member_relay == Some(remote.id())
        {
            // The peer already owns `remote`. Deduplicate by the current
            // local registration to be announced, which may rotate while
            // the peer keeps the same contact address.
            for local in self.local_member_routes(remote) {
                self.enqueue(TransportAddr::Custom(local));
            }
        }
    }

    fn enqueue(&self, address: TransportAddr) {
        let now = self.endpoint.udp_network_generation();
        let mut learned = self.learned.lock().expect("member path candidates");
        if learned.0 != now {
            *learned = (now.clone(), BTreeSet::new());
        }
        if learned.1.insert(address.clone()) {
            let _ = self.sender.send((now, address));
        }
    }

    fn permits(&self, address: &TransportAddr) -> bool {
        let config = &self.endpoint.inner.static_config;
        let policy = config.admission.as_ref().expect("member policy");
        match address {
            TransportAddr::Custom(custom) if Some(custom.id()) == self.direct => {
                config.relay_resource_policy.allows_custom(custom)
                    && policy.permits_direct_candidate(custom)
            }
            TransportAddr::Custom(custom) => {
                Some(custom.id()) == self.member_relay
                    && policy.permits_member_data_path(address)
                    && config.relay_resource_policy.allows_custom(custom)
                    && self.endpoint.inner.has_custom_bootstrap(custom)
            }
            TransportAddr::Relay(url) => {
                self.native_relay
                    && policy.permits_member_data_path(address)
                    && config
                        .relay_resource_policy
                        .allows(url, RelayResourceUse::DataRelay)
            }
            _ => false,
        }
    }

    fn local_addresses(&self, carrier: Option<u64>) -> impl Iterator<Item = CustomAddr> {
        self.endpoint
            .inner
            .local_addr()
            .into_iter()
            .filter_map(move |address| {
                let TransportAddr::Custom(address) = address.into() else {
                    return None;
                };
                (carrier == Some(address.id())).then_some(address)
            })
    }

    fn local_member_routes(&self, remote: &CustomAddr) -> impl Iterator<Item = CustomAddr> {
        let policy = self
            .endpoint
            .inner
            .static_config
            .admission
            .as_ref()
            .expect("member policy");
        self.local_addresses(self.member_relay).filter(|local| {
            policy.shares_member_relay(local, remote)
                && self.permits(&TransportAddr::Custom(local.clone()))
        })
    }

    fn has_qualified_direct(&self, connection: &Connection) -> bool {
        let policy = self
            .endpoint
            .inner
            .static_config
            .admission
            .as_ref()
            .expect("member policy");
        connection.paths().iter().any(|path| {
            (path.is_ip() || matches!(path.remote_addr(), TransportAddr::Custom(addr) if Some(addr.id()) == self.direct))
                && policy.permits_member_data_path(path.remote_addr())
                && path.stats().acknowledgment.is_some()
        })
    }

    async fn initiate(&self, connection: &Connection) -> io::Result<()> {
        self.run_traversal(connection, None).await
    }

    pub(super) async fn respond(&self, connection: &Connection, exchange: ControlExchange) {
        if !self.dialer {
            let _ = self.run_traversal(connection, Some(exchange)).await;
        }
    }

    async fn run_traversal(
        &self,
        connection: &Connection,
        incoming: Option<ControlExchange>,
    ) -> io::Result<()> {
        let (budget, interval) = self.traversal.ok_or(io::ErrorKind::Unsupported)?;
        let deadline = time::Instant::now() + budget;
        let generation = self.endpoint.udp_network_generation();
        let yield_standby = async {
            if self.activity.standby.is_none() {
                return std::future::pending().await;
            }
            let mut foreground = self.activity.planner.foreground();
            let mut local = self.activity.subscribe();
            loop {
                let busy = *foreground.borrow_and_update();
                if busy && self.activity.idle_since().is_some() {
                    break;
                }
                tokio::select! {
                    _ = foreground.changed() => {},
                    _ = local.changed() => {},
                }
            }
        };
        let work = async {
            // Outgoing work waits for the existing node owner before opening
            // a stream. An incoming attempt never waits while its peer holds
            // that peer's slot, so crossed work cannot deadlock.
            let node = if incoming.is_some() {
                self.node_traversal_slot.try_lock().ok()
            } else {
                Some(
                    time::timeout_at(deadline, self.node_traversal_slot.lock())
                        .await
                        .map_err(|_| io::ErrorKind::WouldBlock)?,
                )
            };
            time::timeout_at(deadline, async {
                let paced = self
                    .last_traversal
                    .lock()
                    .expect("direct traversal")
                    .as_ref()
                    .is_some_and(|(scope, started)| {
                        scope.is_current() && started.elapsed() < interval
                    });
                if node.is_none() || paced {
                    if let Some(exchange) = incoming {
                        exchange.send_message(TRAVERSAL_STATUS, &[0]).await?;
                        exchange.finish_send().await?;
                    }
                    return Err(io::ErrorKind::WouldBlock.into());
                }
                let exchange = if let Some(exchange) = incoming {
                    exchange.send_message(TRAVERSAL_STATUS, &[1]).await?;
                    exchange
                } else {
                    let (mut send, recv) = connection.open_bi().await.map_err(io::Error::other)?;
                    send.write_all(&[StreamKind::DirectTraversal as u8])
                        .await
                        .map_err(io::Error::other)?;
                    let exchange = ControlExchange::from_streams(send, recv);
                    match exchange.recv_message(TRAVERSAL_STATUS).await?.as_slice() {
                        [1] => exchange,
                        [0] => return Err(io::ErrorKind::WouldBlock.into()),
                        _ => return Err(io::ErrorKind::InvalidData.into()),
                    }
                };
                *self.last_traversal.lock().expect("direct traversal") =
                    Some((generation.clone(), time::Instant::now()));
                self.traverse(connection, exchange).await
            })
            .await?
        };
        tokio::select! { biased;
            _ = self.endpoint.inner.wait_network_generation_change(&generation) => Err(io::ErrorKind::Interrupted.into()),
            _ = yield_standby => Err(io::ErrorKind::WouldBlock.into()),
            result = work => result,
        }
    }

    async fn traverse(&self, connection: &Connection, exchange: ControlExchange) -> io::Result<()> {
        let peer = exchange.send_control().await?;
        let generation = self.endpoint.udp_network_generation();
        let control = ControlHandle::narrow(&self.endpoint, connection);
        let policy = self
            .endpoint
            .inner
            .static_config
            .admission
            .as_ref()
            .expect("member policy");
        let qualify = async {
            let prepared = policy.prepare_member_direct(&control, &exchange).await?;
            let remote = TransportAddr::Custom(prepared.remote);
            let check = || {
                if generation.is_current() && self.permits(&remote) {
                    Ok(())
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "direct member candidate changed",
                    ))
                }
            };
            check()?;
            let probe = connection
                .paths()
                .iter()
                .find(|path| path.remote_addr() == &remote)
                .ok_or(io::ErrorKind::NotConnected)?
                .start_fresh_probe()?;
            while !probe.is_acknowledged().map_err(io::Error::other)? {
                check()?;
                time::sleep(Duration::from_millis(15)).await;
            }
            Ok::<_, io::Error>((prepared.retain, remote, probe))
        };
        let (retain, remote, probe) = tokio::select! { biased;
            _ = connection.closed() => return Err(io::ErrorKind::NotConnected.into()),
            _ = peer.stopped() => return Err(io::ErrorKind::Interrupted.into()),
            result = qualify => result?,
        };
        // The adapter still guards business packets on this TCP circuit. A
        // peer may stop this stream after consuming Ready, so use the framed
        // exchange itself to decide whether it reached the same phase.
        const READY: ControlMessageFormat = ControlMessageFormat::Fixed { length: 1 };
        let (_, ready) = tokio::try_join!(
            exchange.send_message(READY, &[1]),
            exchange.recv_message(READY)
        )?;
        if ready != [1] {
            return Err(io::ErrorKind::InvalidData.into());
        }
        if !generation.is_current()
            || !self.permits(&remote)
            || !probe.is_acknowledged().map_err(io::Error::other)?
        {
            return Err(io::ErrorKind::Interrupted.into());
        }
        exchange.finish_send().await?;
        retain()
    }
}

pub(in crate::endpoint) fn watch_path_candidates(
    endpoint: Endpoint,
    connection: Connection,
    activity: Arc<super::Activity>,
    direct: Option<u64>,
    member_relay: Option<u64>,
    native_relay: bool,
    node_traversal_slot: Arc<tokio::sync::Mutex<()>>,
) -> Option<Arc<PathCandidates>> {
    if direct.is_none() && member_relay.is_none() {
        return None;
    }
    // Contact inputs can arrive before any business stream. Queue each
    // permitted route once and bound the number of simultaneous QUIC probes.
    let (sender, receiver) = mpsc::unbounded_channel();
    let generation = endpoint.udp_network_generation();
    let traversal = direct.and_then(|_| {
        endpoint
            .inner
            .static_config
            .admission
            .as_ref()?
            .member_direct_traversal()
    });
    let candidates = Arc::new(PathCandidates {
        endpoint,
        direct,
        member_relay,
        native_relay,
        dialer: connection.side().is_client(),
        sender,
        learned: Mutex::new((generation, BTreeSet::new())),
        traversal,
        node_traversal_slot,
        activity: activity.clone(),
        last_traversal: Default::default(),
    });
    tokio::spawn(watch(
        connection,
        activity.path_activity.subscribe(),
        candidates.clone(),
        receiver,
    ));
    Some(candidates)
}

async fn watch(
    connection: Connection,
    mut activity: watch::Receiver<bool>,
    candidates: Arc<PathCandidates>,
    receiver: mpsc::UnboundedReceiver<Hint>,
) {
    let direct_offer_received = Notify::new();
    let mut offer_activity = activity.clone();
    // Bound pending offers from the peer; a full queue stops reading the
    // control stream until an original-connection validation slot opens.
    let (peer_offers, mut peer_receiver) = mpsc::channel(MAX_CANDIDATES);
    let announce_direct = async {
        if candidates
            .local_addresses(candidates.direct)
            .next()
            .is_none()
        {
            return Ok(());
        }
        // Either side's business can trigger the existing TCP offer.
        tokio::select! {
            _ = direct_offer_received.notified() => {},
            active = offer_activity.wait_for(|active| *active) => { active.map_err(io::Error::other)?; }
        }
        let routes = candidates
            .local_addresses(candidates.direct)
            .filter(|address| candidates.permits(&TransportAddr::Custom(address.clone())))
            .take(MAX_CANDIDATES)
            .collect();
        time::timeout(
            EXCHANGE_LIMIT,
            send_offer(
                &connection,
                routes,
                MAX_DIRECT_ADDRESS,
                candidates.supports_traversal(),
            ),
        )
        .await?
    };
    let receive = async {
        let mut received_direct = false;
        while let Ok(stream) = connection.accept_uni().await {
            let generation = candidates.endpoint.udp_network_generation();
            let Ok(Ok((addresses, traversal))) =
                time::timeout(EXCHANGE_LIMIT, read_offer(stream)).await
            else {
                continue;
            };
            let first_direct = !received_direct
                && (addresses.is_empty()
                    || addresses
                        .iter()
                        .any(|address| candidates.direct == Some(address.id())));
            if first_direct {
                received_direct = true;
                direct_offer_received.notify_one();
            }
            if !candidates.dialer && received_direct {
                // Member-relay offers only go toward the dialing peer.
                break;
            }
            if !candidates.dialer {
                continue;
            }
            for address in addresses {
                let accepted = (first_direct && candidates.direct == Some(address.id()))
                    || (candidates.member_relay == Some(address.id())
                        && candidates.local_member_routes(&address).next().is_some());
                let address = TransportAddr::Custom(address);
                if accepted
                    && generation.is_current()
                    && candidates.permits(&address)
                    && peer_offers
                        .send(PeerOffer::Candidate((generation.clone(), address)))
                        .await
                        .is_err()
                {
                    return;
                }
            }
            if first_direct
                && traversal
                && candidates.supports_traversal()
                && peer_offers.send(PeerOffer::Traversal).await.is_err()
            {
                return;
            }
        }
    };
    let known = async {
        let mut receiver = receiver;
        let mut paths = connection.paths_stream();
        let mut generation = candidates.endpoint.udp_network_generation();
        let mut attempted = BTreeSet::new();
        let mut in_flight = FuturesUnordered::new();
        let mut traversal_interval = None;
        let mut next_traversal = None;
        let mut traversal_failures = 0;
        let mut previously_active = false;
        let mut previously_qualified = false;
        loop {
            let has_slot = in_flight.len() < MAX_CANDIDATES;
            let active = *activity.borrow_and_update();
            let now = time::Instant::now();
            let current = candidates.endpoint.udp_network_generation();
            if generation != current {
                generation = current;
                attempted.clear();
                traversal_failures = 0;
                next_traversal = traversal_interval.map(|_| now);
            }
            let can_traverse = active && traversal_interval.is_some();
            let qualified = can_traverse && candidates.has_qualified_direct(&connection);
            if can_traverse && (!previously_active || (previously_qualified && !qualified)) {
                next_traversal.get_or_insert(now);
            }
            previously_active = active;
            previously_qualified = qualified;
            let mut wake_at = next_traversal.filter(|_| {
                active && in_flight.is_empty() && receiver.is_empty() && peer_receiver.is_empty()
            });
            if let Some((due, interval)) = wake_at.zip(traversal_interval) {
                if qualified {
                    next_traversal = None;
                    wake_at = None;
                    traversal_failures = 0;
                } else if due <= now {
                    next_traversal = None;
                    // Reuse this watcher's slot after the ordinary candidates.
                    // The QUIC dialer is the only peer that opens the private stream.
                    let result = tokio::select! { biased;
                        _ = activity.wait_for(|active| !*active) => None,
                        result = candidates.initiate(&connection) => Some(result),
                    };
                    match result {
                        Some(Ok(())) => {
                            traversal_failures = 0;
                            previously_qualified = true;
                        }
                        Some(Err(error)) => {
                            let mut delay = interval;
                            if error.kind() != io::ErrorKind::WouldBlock {
                                traversal_failures = (traversal_failures + 1).min(5);
                                delay *= 1 << traversal_failures;
                            }
                            next_traversal = Some(time::Instant::now() + delay);
                        }
                        None => {}
                    }
                    continue;
                }
            }
            let offer = tokio::select! {
                biased;
                _ = candidates.endpoint.inner.wait_network_generation_change(&generation), if traversal_interval.is_some() => continue,
                Ok(()) = activity.changed() => continue,
                Some(_) = paths.next(), if can_traverse => continue,
                Some(()) = in_flight.next() => continue,
                Some(address) = peer_receiver.recv(), if has_slot => address,
                Some(address) = receiver.recv(), if has_slot && active => PeerOffer::Candidate(address),
                _ = async { time::sleep_until(wake_at.expect("scheduled direct traversal")).await }, if wake_at.is_some() => continue,
                else => break,
            };
            let (observed, address) = match offer {
                PeerOffer::Candidate(hint) => hint,
                PeerOffer::Traversal => {
                    traversal_interval = candidates.traversal.map(|(_, interval)| interval);
                    next_traversal.get_or_insert(now);
                    continue;
                }
            };
            if !observed.is_current() {
                continue;
            }
            if candidates.permits(&address) && attempted.insert(address.clone()) {
                if traversal_interval.is_some() {
                    next_traversal.get_or_insert(now);
                }
                in_flight.push(process_candidate(
                    &connection,
                    &candidates,
                    address,
                    observed,
                ));
            }
        }
    };
    tokio::select! {
        _ = connection.closed() => {}
        _ = async { let _ = tokio::join!(announce_direct, receive, known); } => {}
    }
}

async fn process_candidate(
    connection: &Connection,
    candidates: &PathCandidates,
    address: TransportAddr,
    generation: ObservationValidity,
) {
    if candidates.dialer {
        let _ = time::timeout(
            VALIDATION_LIMIT,
            validate(candidates, connection, address, generation),
        )
        .await;
    } else if let TransportAddr::Custom(local) = address
        && candidates
            .local_addresses(candidates.member_relay)
            .any(|current| current == local)
    {
        let _ = time::timeout(
            EXCHANGE_LIMIT,
            send_offer(connection, vec![local], MAX_OFFER_ADDRESS, false),
        )
        .await;
    }
}

async fn send_offer(
    connection: &Connection,
    addresses: Vec<CustomAddr>,
    max_address: usize,
    traversal: bool,
) -> io::Result<()> {
    let mut offer = PREFACE.to_vec();
    offer.push(u8::from(traversal));
    for address in addresses {
        let encoded = address.to_vec();
        if encoded.len() <= max_address {
            offer.push(encoded.len() as u8);
            offer.extend_from_slice(&encoded);
        }
    }
    let mut stream = connection.open_uni().await.map_err(io::Error::other)?;
    stream.write_all(&offer).await.map_err(io::Error::other)?;
    stream.finish().map_err(io::Error::other)
}

async fn read_offer(mut stream: RecvStream) -> io::Result<(Vec<CustomAddr>, bool)> {
    let offer = stream
        .read_to_end(PREFACE.len() + 1 + MAX_CANDIDATES * (1 + MAX_OFFER_ADDRESS))
        .await
        .map_err(io::Error::other)?;
    let (&traversal, mut body) = offer
        .strip_prefix(PREFACE)
        .and_then(|body| body.split_first())
        .filter(|(flag, _)| **flag <= 1)
        .ok_or(io::ErrorKind::InvalidData)?;
    let mut addresses = Vec::new();
    while let Some((&length, rest)) = body.split_first() {
        let length = usize::from(length);
        if length > MAX_OFFER_ADDRESS || addresses.len() == MAX_CANDIDATES {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let (bytes, next) = rest
            .split_at_checked(length)
            .ok_or(io::ErrorKind::InvalidData)?;
        let address = CustomAddr::from_bytes(bytes).map_err(io::Error::other)?;
        addresses.push(address);
        body = next;
    }
    addresses.dedup();
    Ok((addresses, traversal == 1))
}

async fn validate(
    candidates: &PathCandidates,
    connection: &Connection,
    remote: TransportAddr,
    generation: ObservationValidity,
) -> io::Result<()> {
    let endpoint = &candidates.endpoint;
    let Some(candidate) = endpoint
        .inner
        .request_admitted_data_path(
            connection.remote_id(),
            connection.stable_id(),
            remote.clone(),
            true,
        )
        .await?
    else {
        return Ok(());
    };
    let mut paths = connection.paths_stream();
    let probe = loop {
        let paths = paths.next().await.ok_or(io::ErrorKind::NotConnected)?;
        if let Some(path) = paths
            .iter()
            .find(|path| path.id() == candidate.id() && path.remote_addr() == &remote)
        {
            break path.start_fresh_probe()?;
        }
    };
    while !probe.is_acknowledged().map_err(io::Error::other)? {
        if !generation.is_current() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        time::sleep(Duration::from_millis(15)).await;
    }
    let custom = match &remote {
        TransportAddr::Custom(address) => Some(address.id()),
        _ => None,
    };
    if generation.is_current()
        && candidates.permits(&remote)
        && endpoint
            .inner
            .qualifies_admission_data_path(
                connection.remote_id(),
                connection.stable_id(),
                &remote,
                custom.as_slice(),
            )
            .await
        && generation.is_current()
    {
        candidate.retain();
    }
    Ok(())
}
