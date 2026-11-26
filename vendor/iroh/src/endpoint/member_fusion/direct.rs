//! Validate new carriers on the already admitted QUIC connection.

use std::{
    collections::BTreeSet,
    io,
    sync::{Arc, Mutex},
};

use futures_util::{FutureExt, StreamExt, future::BoxFuture, stream::FuturesUnordered};
use iroh_base::CustomAddr;
use tokio::{
    sync::{Notify, mpsc, watch},
    time::{self, Duration},
};

use super::{
    Connection, ControlExchange, ControlHandle, ControlMessageFormat, RecvStream, RelayResourceUse,
    StreamKind, TransportAddr,
};
use crate::endpoint::{Endpoint, ObservationValidity, PathProbe};

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

enum CandidateCompletion {
    Address,
    Traversal(io::Result<()>),
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
    activity: Arc<super::Activity>,
    last_traversal: Mutex<[Option<(ObservationValidity, time::Instant)>; 2]>,
    healthy_direct: watch::Receiver<bool>,
    data_failed: Option<watch::Receiver<bool>>,
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

    fn has_qualified_direct(&self) -> bool {
        *self.healthy_direct.borrow()
    }

    async fn initiate(&self, connection: &Connection) -> io::Result<()> {
        let policy = self
            .endpoint
            .inner
            .static_config
            .admission
            .as_ref()
            .expect("member policy");
        let mut families = policy
            .member_direct_bindings()
            .into_iter()
            .map(|binding| binding.is_ipv6())
            .collect::<BTreeSet<_>>();
        if families.is_empty() {
            families.insert(false);
        }
        let mut attempts = families
            .into_iter()
            .map(|ipv6| self.run_traversal(connection, None, ipv6))
            .collect::<FuturesUnordered<_>>();
        let mut error = io::ErrorKind::Unsupported.into();
        while let Some(result) = attempts.next().await {
            match result {
                Ok(()) => return Ok(()),
                Err(failed) => error = failed,
            }
        }
        Err(error)
    }

    pub(super) async fn respond(&self, connection: &Connection, exchange: ControlExchange, ipv6: bool) {
        if !self.dialer {
            let _ = self.run_traversal(connection, Some(exchange), ipv6).await;
        }
    }

    async fn physical_tcp(
        &self,
        connection: &Connection,
        wait: bool,
        deadline: time::Instant,
        ipv6: bool,
    ) -> io::Result<Option<super::physical::MemberPunching>> {
        let policy = self
            .endpoint
            .inner
            .static_config
            .admission
            .as_ref()
            .expect("member policy");
        let bindings = policy.member_direct_bindings();
        let Some(configured) = bindings.iter().copied().find(|binding| binding.is_ipv6() == ipv6) else {
            return Ok(bindings.is_empty().then(super::physical::MemberPunching::untracked));
        };
        let observers = policy.member_direct_observers();
        if observers.is_empty() {
            return Ok(Some(super::physical::MemberPunching::untracked()));
        }
        let family = if ipv6 {
            iroh_relay::ObservationFamily::Ipv6
        } else {
            iroh_relay::ObservationFamily::Ipv4
        };
        let binding = super::physical::observer_binding(
            &self.endpoint,
            family,
            observers.into_iter().map(|url| (url, 3478)),
            Some(configured),
            deadline,
        )
        .await?;
        let scope = self.endpoint.inner.punching_scope(binding, true)?;
        let pool = &policy
            .member_fusion()
            .expect("member fusion")
            .building
            .physical;
        if wait {
            Ok(Some(
                time::timeout_at(deadline, pool.acquire(scope, connection.remote_id()))
                    .await
                    .map_err(|_| io::ErrorKind::TimedOut)?
                    .with_binding(binding),
            ))
        } else {
            Ok(pool
                .try_acquire(scope, connection.remote_id())
                .map(|permit| permit.with_binding(binding)))
        }
    }

    async fn run_traversal(
        &self,
        connection: &Connection,
        incoming: Option<ControlExchange>,
        ipv6: bool,
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
            // Physical serialization and pacing happen before the member position.
            // Outgoing work waits for the existing node owner before opening
            // a stream. An incoming attempt never waits while its peer holds
            // that peer's slot, so crossed work cannot deadlock.
            let physical = self
                .physical_tcp(connection, incoming.is_none(), deadline, ipv6)
                .await?;
            time::timeout_at(deadline, async {
                let paced = self.last_traversal.lock().expect("direct traversal")
                    [usize::from(ipv6)]
                .as_ref()
                .is_some_and(|(scope, started)| scope.is_current() && started.elapsed() < interval);
                if physical.is_none() || paced {
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
                    send.write_all(&[if ipv6 { StreamKind::DirectTraversalIpv6 } else { StreamKind::DirectTraversal } as u8])
                        .await
                        .map_err(io::Error::other)?;
                    let exchange = ControlExchange::from_streams(send, recv);
                    match exchange.recv_message(TRAVERSAL_STATUS).await?.as_slice() {
                        [1] => exchange,
                        [0] => return Err(io::ErrorKind::WouldBlock.into()),
                        _ => return Err(io::ErrorKind::InvalidData.into()),
                    }
                };
                let fusion = self
                    .endpoint
                    .inner
                    .static_config
                    .admission
                    .as_ref()
                    .and_then(|policy| policy.member_fusion())
                    .expect("member fusion");
                let _position = fusion
                    .building
                    .try_acquire(connection.remote_id())
                    .ok_or(io::ErrorKind::WouldBlock)?;
                let started = time::Instant::now();
                let result = self
                    .traverse(
                        connection,
                        exchange,
                        physical.expect("physical TCP reservation"),
                    )
                    .await;
                if !result
                    .as_ref()
                    .is_err_and(|error| error.kind() == io::ErrorKind::WouldBlock)
                {
                    self.last_traversal.lock().expect("direct traversal")[usize::from(ipv6)] =
                        Some((generation.clone(), started));
                }
                result.map(|_| ())
            })
            .await?
        };
        let unavailable = async {
            if let Some(mut failed) = self.data_failed.clone() {
                let _ = failed.wait_for(|failed| *failed).await;
            } else {
                std::future::pending::<()>().await;
            }
        };
        tokio::select! { biased;
            _ = unavailable => Err(io::ErrorKind::NotConnected.into()),
            _ = self.endpoint.inner.wait_network_generation_change(&generation) => Err(io::ErrorKind::Interrupted.into()),
            _ = yield_standby => Err(io::ErrorKind::WouldBlock.into()),
            result = work => result,
        }
    }

    pub(super) async fn repair(
        &self,
        connection: &Connection,
        exchange: ControlExchange,
        deadline: time::Instant,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> io::Result<(TransportAddr, u64)> {
        let (budget, _) = self.traversal.ok_or(io::ErrorKind::Unsupported)?;
        let deadline = deadline.min(time::Instant::now() + budget);
        let generation = self.endpoint.udp_network_generation();
        let work = async {
            let physical = self
                .physical_tcp(
                    connection,
                    true,
                    deadline,
                    self.endpoint
                        .inner
                        .static_config
                        .admission
                        .as_ref()
                        .and_then(|policy| policy.member_direct_binding())
                        .is_some_and(|binding| binding.is_ipv6()),
                )
                .await?
                .ok_or(io::ErrorKind::WouldBlock)?;
            let fusion = self
                .endpoint
                .inner
                .static_config
                .admission
                .as_ref()
                .and_then(|policy| policy.member_fusion())
                .expect("member fusion");
            let _position = fusion
                .building
                .try_acquire(connection.remote_id())
                .ok_or(io::ErrorKind::WouldBlock)?;
            self.traverse(connection, exchange, physical).await
        };
        tokio::select! { biased;
            _ = cancel.cancelled() => Err(io::ErrorKind::Interrupted.into()),
            _ = connection.closed() => Err(io::ErrorKind::NotConnected.into()),
            _ = self.endpoint.inner.wait_network_generation_change(&generation) => Err(io::ErrorKind::Interrupted.into()),
            _ = time::sleep_until(deadline) => Err(io::ErrorKind::TimedOut.into()),
            result = work => result,
        }
    }

    async fn traverse(
        &self,
        connection: &Connection,
        exchange: ControlExchange,
        physical: super::physical::MemberPunching,
    ) -> io::Result<(TransportAddr, u64)> {
        let peer = exchange.send_control().await?;
        let control = ControlHandle::narrow(&self.endpoint, connection);
        let policy = self
            .endpoint
            .inner
            .static_config
            .admission
            .as_ref()
            .expect("member policy");
        let qualify = async {
            let prepared = policy
                .prepare_member_direct_reserved(&control, &exchange, physical)
                .await?;
            verify_direct(&control, prepared).await
        };
        let verified = tokio::select! { biased;
            _ = connection.closed() => return Err(io::ErrorKind::NotConnected.into()),
            _ = peer.stopped() => return Err(io::ErrorKind::Interrupted.into()),
            result = qualify => result?,
        };
        let watermark = verified.probe.watermark();
        verified
            .commit(&control, &exchange)
            .await
            .map(|remote| (remote, watermark))
    }
}

pub(super) struct VerifiedDirect {
    prepared: super::MemberDirectCandidate,
    remote: TransportAddr,
    generation: ObservationValidity,
    probe: PathProbe,
}

impl VerifiedDirect {
    fn check(&self, control: &ControlHandle) -> io::Result<()> {
        let config = &control.endpoint.inner.static_config;
        let policy = config.admission.as_ref().expect("member policy");
        if self.generation.is_current()
            && config
                .relay_resource_policy
                .allows_custom(&self.prepared.remote)
            && policy.permits_direct_candidate(&self.prepared.remote)
        {
            Ok(())
        } else {
            Err(io::ErrorKind::Interrupted.into())
        }
    }

    pub(super) async fn commit(
        self,
        control: &ControlHandle,
        exchange: &ControlExchange,
    ) -> io::Result<TransportAddr> {
        // A peer can finish its stream after reading Ready. Consume the framed
        // confirmation even if its send-control handle has already stopped.
        const READY: ControlMessageFormat = ControlMessageFormat::Fixed { length: 1 };
        let (_, ready) = tokio::try_join!(
            exchange.send_message(READY, &[1]),
            exchange.recv_message(READY)
        )?;
        if ready != [1] {
            return Err(io::ErrorKind::InvalidData.into());
        }
        self.check(control)?;
        if !self.probe.is_acknowledged().map_err(io::Error::other)? {
            return Err(io::ErrorKind::Interrupted.into());
        }
        exchange.finish_send().await?;
        (self.prepared.retain)()?;
        eprintln!(
            "MEMBER_DIRECT_RETAINED stable_id={} path={:?} remote={:?}",
            control.stable_id(),
            control
                .paths()
                .iter()
                .find(|path| path.remote_addr() == &self.remote)
                .map(|path| path.id()),
            self.remote
        );
        Ok(self.remote)
    }
}

/// Validate the candidate while its transport resources remain tentative.
pub(super) async fn verify_direct(
    control: &ControlHandle,
    prepared: super::MemberDirectCandidate,
) -> io::Result<VerifiedDirect> {
    let remote = TransportAddr::Custom(prepared.remote.clone());
    let generation = control.udp_network_generation();
    let probe = control
        .paths()
        .iter()
        .find(|path| path.remote_addr() == &remote)
        .ok_or(io::ErrorKind::NotConnected)?
        .start_fresh_probe()?;
    let verified = VerifiedDirect {
        prepared,
        remote,
        generation,
        probe,
    };
    verified.check(control)?;
    while !verified.probe.is_acknowledged().map_err(io::Error::other)? {
        verified.check(control)?;
        time::sleep(Duration::from_millis(15)).await;
    }
    Ok(verified)
}

pub(in crate::endpoint) fn watch_path_candidates(
    endpoint: Endpoint,
    connection: Connection,
    activity: Arc<super::Activity>,
    direct: Option<u64>,
    member_relay: Option<u64>,
    native_relay: bool,
    healthy_direct: watch::Receiver<bool>,
    data_failed: Option<watch::Receiver<bool>>,
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
        activity: activity.clone(),
        last_traversal: Default::default(),
        healthy_direct,
        data_failed,
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
        use n0_watcher::Watcher;
        if candidates.direct.is_none() {
            return Ok::<(), io::Error>(());
        }
        // The transport watcher includes all current TCP interfaces and mappings.
        let mut addresses = candidates.endpoint.inner.watch_local_addrs();
        tokio::select! {
            _ = direct_offer_received.notified() => {},
            active = offer_activity.wait_for(|active| *active) => { active.map_err(io::Error::other)?; }
        }
        let mut previous = BTreeSet::new();
        loop {
            let current: BTreeSet<_> = candidates
                .local_addresses(candidates.direct)
                .filter(|address| candidates.permits(&TransportAddr::Custom(address.clone())))
                .collect();
            #[cfg(feature = "miopunch-test-hooks")]
            let current = if candidates.endpoint.inner.static_config.suppress_member_direct_announcements {
                BTreeSet::new()
            } else { current };
            let added: Vec<_> = current.difference(&previous).cloned().collect();
            if !added.is_empty() || previous.is_empty() {
                // Stream credit backpressure must not retire the complete
                // candidate set or the watcher for later updates.
                send_offer(
                    &connection,
                    added,
                    MAX_DIRECT_ADDRESS,
                    candidates.supports_traversal(),
                )
                .await?;
            }
            previous = current;
            addresses.updated().await.map_err(io::Error::other)?;
        }
        #[allow(unreachable_code)]
        Ok::<(), io::Error>(())
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
            if !candidates.dialer {
                continue;
            }
            for address in addresses {
                let accepted = (candidates.direct == Some(address.id()))
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
        let mut in_flight: FuturesUnordered<BoxFuture<'_, CandidateCompletion>> =
            FuturesUnordered::new();
        let mut healthy_direct = candidates.healthy_direct.clone();
        let mut traversal_running = false;
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
            let qualified = can_traverse && candidates.has_qualified_direct();
            if can_traverse && (!previously_active || (previously_qualified && !qualified)) {
                next_traversal.get_or_insert(now);
            }
            previously_active = active;
            previously_qualified = qualified;
            let mut wake_at = next_traversal.filter(|_| active && !traversal_running);
            if let Some((due, _)) = wake_at.zip(traversal_interval) {
                if qualified {
                    next_traversal = None;
                    wake_at = None;
                    traversal_failures = 0;
                } else if due <= now {
                    next_traversal = None;
                    // TCP has its own node slot and need not wait for ordinary
                    // candidates. Only the QUIC dialer opens this private stream.
                    traversal_running = true;
                    let mut running_activity = activity.clone();
                    let owner = &candidates;
                    let original = &connection;
                    in_flight.push(async move {
                        CandidateCompletion::Traversal(tokio::select! { biased;
                            _ = running_activity.wait_for(|active| !*active) => Err(io::ErrorKind::WouldBlock.into()),
                            result = owner.initiate(original) => result,
                        })
                    }.boxed());
                    continue;
                }
            }
            let offer = tokio::select! {
                biased;
                _ = candidates.endpoint.inner.wait_network_generation_change(&generation), if traversal_interval.is_some() => continue,
                Ok(()) = activity.changed() => continue,
                Ok(()) = healthy_direct.changed() => continue,
                Some(_) = paths.next(), if can_traverse => continue,
                Some(result) = in_flight.next() => {
                    if let CandidateCompletion::Traversal(result) = result {
                        traversal_running = false;
                        match result {
                            Ok(()) => {
                                traversal_failures = 0;
                                next_traversal = None;
                                previously_qualified = candidates.has_qualified_direct();
                            }
                            Err(error) => {
                                let mut delay = traversal_interval.expect("running TCP traversal interval");
                                if error.kind() != io::ErrorKind::WouldBlock {
                                    traversal_failures = (traversal_failures + 1).min(5);
                                    delay *= 1 << traversal_failures;
                                }
                                next_traversal = Some(time::Instant::now() + delay);
                            }
                        }
                    }
                    continue;
                },
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
                in_flight.push(
                    async {
                        process_candidate(&connection, &candidates, address, observed).await;
                        CandidateCompletion::Address
                    }
                    .boxed(),
                );
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
        let fusion = candidates
            .endpoint
            .inner
            .static_config
            .admission
            .as_ref()
            .and_then(|policy| policy.member_fusion())
            .expect("member fusion");
        let _position = fusion.building.acquire(connection.remote_id()).await;
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
    // Keep the established per-message byte boundary and wire shape. The
    // validation concurrency bound does not truncate the advertised set.
    let limit = PREFACE.len() + 1 + MAX_CANDIDATES * (1 + MAX_OFFER_ADDRESS);
    let mut batches = vec![Vec::new()];
    let mut bytes = PREFACE.len() + 1;
    for address in addresses.into_iter().collect::<BTreeSet<_>>() {
        let encoded = address.to_vec();
        if encoded.len() > max_address {
            continue;
        }
        let batch = batches.last_mut().unwrap();
        if bytes + 1 + encoded.len() > limit || batch.len() == MAX_CANDIDATES {
            batches.push(Vec::new());
            bytes = PREFACE.len() + 1;
        }
        bytes += 1 + encoded.len();
        batches.last_mut().unwrap().push(encoded);
    }
    for batch in batches {
        let mut offer = PREFACE.to_vec();
        offer.push(u8::from(traversal));
        for encoded in batch {
            offer.push(encoded.len() as u8);
            offer.extend_from_slice(&encoded);
        }
        let mut stream = connection.open_uni().await.map_err(io::Error::other)?;
        stream.write_all(&offer).await.map_err(io::Error::other)?;
        stream.finish().map_err(io::Error::other)?;
    }
    Ok(())
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
