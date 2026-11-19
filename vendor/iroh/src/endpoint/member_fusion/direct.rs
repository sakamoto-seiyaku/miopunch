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

use super::{Connection, RecvStream, RelayResourceUse, TransportAddr};
use crate::endpoint::{Endpoint, ObservationValidity};

const PREFACE: &[u8] = b"iroh/member-direct/1\0";
const MAX_CANDIDATES: usize = 2;
const MAX_DIRECT_ADDRESS: usize = 32;
const MAX_OFFER_ADDRESS: usize = 64;
const EXCHANGE_LIMIT: Duration = Duration::from_secs(6);
const VALIDATION_LIMIT: Duration = Duration::from_secs(6);
type Hint = (ObservationValidity, TransportAddr);

#[derive(Debug)]
pub(in crate::endpoint) struct PathCandidates {
    endpoint: Endpoint,
    direct: Option<u64>,
    member_relay: Option<u64>,
    native_relay: bool,
    dialer: bool,
    sender: mpsc::UnboundedSender<Hint>,
    learned: Mutex<(ObservationValidity, BTreeSet<TransportAddr>)>,
}

impl PathCandidates {
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
}

pub(in crate::endpoint) fn watch_path_candidates(
    endpoint: Endpoint,
    connection: Connection,
    activity: watch::Receiver<bool>,
    direct: Option<u64>,
    member_relay: Option<u64>,
    native_relay: bool,
) -> Option<Arc<PathCandidates>> {
    if direct.is_none() && member_relay.is_none() {
        return None;
    }
    // Contact inputs can arrive before any business stream. Queue each
    // permitted route once and bound the number of simultaneous QUIC probes.
    let (sender, receiver) = mpsc::unbounded_channel();
    let generation = endpoint.udp_network_generation();
    let candidates = Arc::new(PathCandidates {
        endpoint,
        direct,
        member_relay,
        native_relay,
        dialer: connection.side().is_client(),
        sender,
        learned: Mutex::new((generation, BTreeSet::new())),
    });
    tokio::spawn(watch(connection, activity, candidates.clone(), receiver));
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
            send_offer(&connection, routes, MAX_DIRECT_ADDRESS),
        )
        .await?
    };
    let receive = async {
        let mut received_direct = false;
        while let Ok(stream) = connection.accept_uni().await {
            let generation = candidates.endpoint.udp_network_generation();
            let Ok(Ok(addresses)) = time::timeout(EXCHANGE_LIMIT, read_offer(stream)).await else {
                continue;
            };
            if !generation.is_current() {
                continue;
            }
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
                    && candidates.permits(&address)
                    && peer_offers
                        .send((generation.clone(), address))
                        .await
                        .is_err()
                {
                    return;
                }
            }
        }
    };
    let known = async {
        let mut receiver = receiver;
        let mut generation = candidates.endpoint.udp_network_generation();
        let mut attempted = BTreeSet::new();
        let mut in_flight = FuturesUnordered::new();
        loop {
            let has_slot = in_flight.len() < MAX_CANDIDATES;
            let active = *activity.borrow_and_update();
            let (observed, address) = tokio::select! {
                biased;
                Ok(()) = activity.changed() => continue,
                Some(()) = in_flight.next() => continue,
                Some(address) = peer_receiver.recv(), if has_slot => address,
                Some(address) = receiver.recv(), if has_slot && active => address,
                else => break,
            };
            if !observed.is_current() {
                continue;
            }
            if generation != observed {
                generation = observed.clone();
                attempted.clear();
            }
            if candidates.permits(&address) && attempted.insert(address.clone()) {
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
            send_offer(connection, vec![local], MAX_OFFER_ADDRESS),
        )
        .await;
    }
}

async fn send_offer(
    connection: &Connection,
    addresses: Vec<CustomAddr>,
    max_address: usize,
) -> io::Result<()> {
    let mut offer = PREFACE.to_vec();
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

async fn read_offer(mut stream: RecvStream) -> io::Result<Vec<CustomAddr>> {
    let offer = stream
        .read_to_end(PREFACE.len() + MAX_CANDIDATES * (1 + MAX_OFFER_ADDRESS))
        .await
        .map_err(io::Error::other)?;
    let mut body = offer
        .strip_prefix(PREFACE)
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
    Ok(addresses)
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
            break path.start_probe()?;
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
