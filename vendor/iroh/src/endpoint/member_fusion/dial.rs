use std::{
    collections::BTreeSet,
    future::Future,
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use futures_util::{StreamExt, stream::FuturesUnordered};
use tokio::sync::{oneshot, watch};
use tokio::time::{self, Duration, Instant};
use tokio_util::sync::CancellationToken;

use crate::{
    EndpointId, RelayUrl, address_lookup::AddressLookupServices, socket::transports::RelayBootstrap,
};

use super::{
    super::ConnectWithOptsError, ConnectError, ConnectOptions, ConnectingError, ConnectionError,
    EndpointAddr, MEMBER_ADMISSION_DENIED, MEMBER_DATA_UNAVAILABLE, RelayResourceUse,
    RestrictedEndpoint, TransportAddr,
};

type RouteFinished = Box<dyn FnOnce(Result<usize, io::ErrorKind>) + Send>;
type RouteStarted =
    dyn Fn(&TransportAddr, Instant) -> ((usize, usize), RouteFinished) + Send + Sync;
type IdentityConfirmed = dyn Fn((usize, usize), EndpointId, usize) + Send + Sync;
type ContactFinished = Box<dyn FnOnce(&io::Result<()>) + Send>;
type ContactBudget =
    dyn Fn(crate::EndpointId, Instant) -> io::Result<ContactFinished> + Send + Sync;

/// Current contact inputs for a native member dial, evaluated by the owner.
#[derive(Default)]
pub struct MemberDialOptions {
    active: Vec<TransportAddr>,
    recent: Vec<TransportAddr>,
    relay_fallbacks: Vec<RelayUrl>,
    discover_ip: bool,
    route_started: Option<Box<RouteStarted>>,
    identity_confirmed: Option<Box<IdentityConfirmed>>,
    restricted: Vec<iroh_base::CustomAddr>,
    contact_budget: Option<Box<ContactBudget>>,
}

impl MemberDialOptions {
    /// Supply permitted active IP paths, recent contacts in observation order,
    /// and permitted local rendezvous relays in their preferred order.
    pub fn new(
        active: Vec<TransportAddr>,
        recent: Vec<TransportAddr>,
        relay_fallbacks: Vec<RelayUrl>,
    ) -> Self {
        Self {
            active,
            recent,
            relay_fallbacks,
            ..Self::default()
        }
    }

    /// Current restricted contact candidates, in application preference order.
    pub fn with_restricted_contacts(mut self, contacts: Vec<iroh_base::CustomAddr>) -> Self {
        self.restricted = contacts;
        self
    }

    /// Acquire an application contact allowance once, when the shared dial
    /// actually starts restricted contact. Cancellation drops the allowance.
    pub fn with_contact_budget<F, C>(mut self, acquire: F) -> Self
    where
        F: Fn(crate::EndpointId, Instant) -> io::Result<C> + Send + Sync + 'static,
        C: FnOnce(&io::Result<()>) + Send + 'static,
    {
        self.contact_budget = Some(Box::new(move |peer, deadline| {
            acquire(peer, deadline).map(|finished| Box::new(finished) as ContactFinished)
        }));
        self
    }

    /// Look up additional IP contacts alongside initial routes when direct
    /// UDP is currently allowed. Discovered routes are cold hints.
    pub fn with_ip_discovery(mut self) -> Self {
        self.discover_ip = true;
        self
    }

    /// Report each explicit route start and attach a local attempt and phase ID
    /// to its handshake. The completion observer is dropped on cancellation.
    pub fn with_route_diagnostics<F, C>(mut self, started: F) -> Self
    where
        F: Fn(&TransportAddr, Instant) -> ((usize, usize), C) + Send + Sync + 'static,
        C: FnOnce(Result<usize, io::ErrorKind>) + Send + 'static,
    {
        self.route_started = Some(Box::new(move |route, deadline| {
            let (ids, finished) = started(route, deadline);
            (ids, Box::new(finished))
        }));
        self
    }

    /// Observe a route's authenticated QUIC identity before member admission.
    pub fn with_identity_diagnostics(
        mut self,
        authenticated: impl Fn((usize, usize), EndpointId, usize) + Send + Sync + 'static,
    ) -> Self {
        self.identity_confirmed = Some(Box::new(authenticated));
        self
    }

    pub(super) fn relay_fallbacks(&self) -> &[RelayUrl] {
        &self.relay_fallbacks
    }
}

pub(super) struct Dial<'a> {
    pub(super) endpoint: &'a RestrictedEndpoint,
    pub(super) alpn: &'a [u8],
    pub(super) deadline: Instant,
    pub(super) cancel: &'a CancellationToken,
}

struct RouteProgress<'a> {
    started: Option<&'a RouteStarted>,
    identity_confirmed: Option<&'a IdentityConfirmed>,
    authenticated: AtomicBool,
}

#[derive(Clone, Copy)]
struct RelayStart {
    at: Instant,
    stagger: Duration,
}

type RelayClock = watch::Sender<Option<RelayStart>>;

struct ReleaseRelay(Arc<RelayBootstrap>);

impl Drop for ReleaseRelay {
    fn drop(&mut self) {
        self.0.released.store(true, Ordering::Release);
    }
}

impl Dial<'_> {
    pub(super) async fn run(
        self,
        mut target: EndpointAddr,
        options: Option<MemberDialOptions>,
    ) -> io::Result<()> {
        let Some(options) = options else {
            return self
                .connect(target, self.opts())
                .await
                .map(drop)
                .map_err(dial_error);
        };
        let lookup = options
            .discover_ip
            .then(|| self.endpoint.address_lookup().ok())
            .flatten()
            .filter(|lookup| !lookup.is_empty());
        let known_ips = if lookup.is_some() {
            target
                .addrs
                .iter()
                .filter(|route| route.is_ip())
                .cloned()
                .collect()
        } else {
            BTreeSet::new()
        };
        let active = options
            .active
            .into_iter()
            .filter(|route| route.is_ip() && target.addrs.remove(route))
            .collect::<Vec<_>>();
        let direct_transport = self
            .endpoint
            .inner
            .inner
            .static_config
            .admission
            .as_ref()
            .and_then(|policy| policy.member_direct_transport());
        let recent_direct = options
            .recent
            .iter()
            .filter(|route| {
                route.is_ip()
                    || matches!(route, TransportAddr::Custom(address)
                if Some(address.id()) == direct_transport)
            })
            .filter(|route| target.addrs.remove(*route))
            .cloned()
            .collect::<Vec<_>>();
        let endpoint = &self.endpoint.inner.inner;
        let member_relay = endpoint
            .static_config
            .admission
            .as_ref()
            .and_then(|policy| policy.member_relay_transport());
        let is_data_relay = |route: &TransportAddr| match route {
            TransportAddr::Relay(_) => true,
            TransportAddr::Custom(address) => member_relay == Some(address.id()),
            _ => false,
        };
        let mut relays = target
            .addrs
            .iter()
            .filter(|route| is_data_relay(route))
            .cloned()
            .collect::<Vec<_>>();
        target.addrs.retain(|route| !is_data_relay(route));
        relays.sort_by_key(|route| {
            options
                .recent
                .iter()
                .position(|recent| recent == route)
                .unwrap_or(usize::MAX)
        });
        for relay in options.relay_fallbacks {
            let relay = TransportAddr::Relay(relay);
            if !relays.contains(&relay) {
                relays.push(relay);
            }
        }
        let relay_policy = &endpoint.static_config.relay_resource_policy;
        relays.retain(|route| match route {
            TransportAddr::Relay(url) => relay_policy.allows(url, RelayResourceUse::DataRelay),
            TransportAddr::Custom(address) => {
                relay_policy.allows_custom(address) && endpoint.has_custom_bootstrap(address)
            }
            _ => false,
        });
        let mut seen = BTreeSet::new();
        let restricted = options
            .restricted
            .into_iter()
            .filter(|route| {
                target.addrs.remove(&TransportAddr::Custom(route.clone()));
                seen.insert(route.clone())
            })
            .collect::<Vec<_>>();
        let combined = !restricted.is_empty() && !relays.is_empty();
        let contact_budget = options.contact_budget.as_deref();
        let cold = target.addrs.into_iter().collect::<Vec<_>>();
        let progress = RouteProgress {
            started: options.route_started.as_deref(),
            identity_confirmed: options.identity_confirmed.as_deref(),
            authenticated: AtomicBool::new(false),
        };
        let id = target.id;
        let routes = async {
            let mut first = None;
            let has_race =
                !recent_direct.is_empty() || !relays.is_empty() || !restricted.is_empty();
            let has_cold = !cold.is_empty();
            if !active.is_empty() {
                let stages = 1 + u32::from(has_race) + u32::from(has_cold) + u32::from(combined);
                let end = split_deadline(self.deadline, stages);
                match self.ordered(id, active, end, &progress).await {
                    Err(error) if !stops_dial(&error) => first = Some(error),
                    result => return result,
                }
            }
            if has_race {
                let next_stage = split_deadline(self.deadline, 1 + u32::from(has_cold));
                // Fallback shares the original deadline with the live relay lanes.
                let end = if combined { self.deadline } else { next_stage };
                let alternative = async {
                    if relays.is_empty() {
                        self.contact(id, restricted, end, &progress, contact_budget)
                            .await
                    } else if combined {
                        self.relays_with_contact(id, relays, restricted, &progress, contact_budget)
                            .await
                    } else {
                        let (clock, _) = watch::channel(None);
                        self.relays(id, relays, end, &progress, &clock, false).await
                    }
                };
                let primary = async {
                    if recent_direct.is_empty() {
                        alternative.await
                    } else {
                        race(
                            self.parallel(id, recent_direct, end, &progress),
                            alternative,
                        )
                        .await
                    }
                };
                tokio::pin!(primary);
                let result = if combined && has_cold {
                    tokio::select! { biased;
                        result = &mut primary => result,
                        _ = time::sleep_until(next_stage) => {
                            return race(primary, self.cold(id, cold, &progress, first)).await;
                        }
                    }
                } else {
                    primary.await
                };
                match result {
                    Err(error) if !stops_dial(&error) => first = Some(prefer_error(first, error)),
                    result => return result,
                }
            }
            self.cold(id, cold, &progress, first).await
        };
        match lookup {
            Some(lookup) => {
                race(
                    routes,
                    self.discover(id, lookup, known_ips, &progress, None),
                )
                .await
            }
            None => routes.await,
        }
    }

    async fn relays_with_contact(
        &self,
        id: EndpointId,
        relays: Vec<TransportAddr>,
        restricted: Vec<iroh_base::CustomAddr>,
        progress: &RouteProgress<'_>,
        budget: Option<&ContactBudget>,
    ) -> io::Result<()> {
        let count = relays.len();
        let (clock, mut started) = watch::channel(None);
        let relay = self.relays(id, relays, self.deadline, progress, &clock, true);
        let contact = self.contact(id, restricted, self.deadline, progress, budget);
        let window = async {
            let first = first_relay(&mut started).await;
            time::sleep_until(first.at + first.stagger * (count as u32 + 1)).await;
        };
        tokio::pin!(relay);
        let authenticated = || progress.authenticated.load(Ordering::Acquire);
        tokio::select! { biased;
            result = &mut relay => {
                match &result {
                    Err(error) if !stops_dial(error) && !authenticated() => {
                        race(std::future::ready(result), contact).await
                    }
                    _ => result,
                }
            }
            _ = window => {
                if authenticated() {
                    relay.await
                } else {
                    race(relay, contact).await
                }
            }
        }
    }

    async fn cold(
        &self,
        id: EndpointId,
        cold: Vec<TransportAddr>,
        progress: &RouteProgress<'_>,
        mut first: Option<io::Error>,
    ) -> io::Result<()> {
        if !cold.is_empty() {
            match self.ordered(id, cold, self.deadline, progress).await {
                Err(error) if !stops_dial(&error) => first = Some(prefer_error(first, error)),
                result => return result,
            }
        }
        Err(first.unwrap_or_else(|| io::ErrorKind::NotConnected.into()))
    }

    async fn contact(
        &self,
        id: EndpointId,
        mut routes: Vec<iroh_base::CustomAddr>,
        end: Instant,
        progress: &RouteProgress<'_>,
        budget: Option<&ContactBudget>,
    ) -> io::Result<()> {
        let ready = |route: &iroh_base::CustomAddr| {
            self.endpoint.inner.inner.permits_custom_bootstrap(route)
        };
        let Some(mut next) = routes.iter().position(&ready) else {
            return Err(io::ErrorKind::NotConnected.into());
        };
        if self.cancel.is_cancelled() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if Instant::now() >= end {
            return Err(io::ErrorKind::TimedOut.into());
        }
        let finish = budget
            .map(|acquire| acquire(id, self.deadline))
            .transpose()?;
        let mut failure = None;
        let result = loop {
            let route = TransportAddr::Custom(routes.remove(next));
            let share = split_deadline(end, (routes.len() + 1) as u32);
            let error = match self
                .route(id, route, end, progress, None, Some(share))
                .await
            {
                Err(error) if !stops_dial(&error) => prefer_error(failure, error),
                result => break result,
            };
            let Some(index) = routes.iter().position(&ready) else {
                break Err(error);
            };
            failure = Some(error);
            next = index;
        };
        if let Some(finish) = finish {
            finish(&result);
        }
        result
    }

    async fn discover(
        &self,
        id: EndpointId,
        lookup: &AddressLookupServices,
        mut seen: BTreeSet<TransportAddr>,
        progress: &RouteProgress<'_>,
        mut failure: Option<io::Error>,
    ) -> io::Result<()> {
        let contacts = lookup.resolve(id);
        tokio::pin!(contacts);
        let mut flying = FuturesUnordered::new();
        let mut finished = false;
        while !finished || !flying.is_empty() {
            tokio::select! {
                contact = contacts.next(), if !finished => {
                    let Some(contact) = contact else { finished = true; continue };
                    let Ok(Ok(contact)) = contact else { continue };
                    for route in contact.into_endpoint_addr().addrs {
                        if route.is_ip() && seen.insert(route.clone()) {
                            flying.push(self.route(id, route, self.deadline, progress, None, None));
                        }
                    }
                }
                Some(result) = flying.next(), if !flying.is_empty() => match result {
                    Err(error) if !stops_dial(&error) => failure = Some(prefer_error(failure, error)),
                    result => return result,
                },
            }
        }
        Err(failure.unwrap_or_else(|| io::ErrorKind::NotConnected.into()))
    }

    fn opts(&self) -> ConnectOptions {
        ConnectOptions::default()
            .with_caller_deadline(self.deadline)
            .with_attempt_cancellation(self.cancel.clone())
    }

    async fn connect(
        &self,
        target: EndpointAddr,
        options: ConnectOptions,
    ) -> Result<usize, ConnectError> {
        let connecting = self
            .endpoint
            .connect_with_opts(target, self.alpn, options)
            .await?;
        Ok(connecting.await?.stable_id())
    }

    async fn route(
        &self,
        id: EndpointId,
        route: TransportAddr,
        end: Instant,
        progress: &RouteProgress<'_>,
        relay: Option<(&RelayClock, bool)>,
        silent_until: Option<Instant>,
    ) -> io::Result<()> {
        if self.cancel.is_cancelled() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if Instant::now() >= end {
            return Err(io::ErrorKind::TimedOut.into());
        }
        let mut options = self.opts().with_bootstrap_route(route.clone());
        let native_relay = relay.filter(|_| route.is_relay());
        let (watch_unavailable, unavailable) = if native_relay.is_some_and(|(_, track)| track) {
            let (send, receive) = oneshot::channel();
            (Some(send), Some(receive))
        } else {
            (None, None)
        };
        let _relay_scope = native_relay.map(|_| {
            let scope = Arc::new(RelayBootstrap {
                released: AtomicBool::new(false),
                watch_unavailable: std::sync::Mutex::new(watch_unavailable),
            });
            options.exact_relay_bootstrap = Some(scope.clone());
            ReleaseRelay(scope)
        });
        let (ids, options, finished) = if let Some(start) = progress.started {
            let ((attempt, phase), finish) = start(&route, self.deadline);
            (
                Some((attempt, phase)),
                options.with_diagnostic_dial(attempt, phase),
                Some(finish),
            )
        } else {
            (None, options, None)
        };
        let contact = match &route {
            TransportAddr::Custom(contact) if silent_until.is_some() || relay.is_some() => {
                Some(contact.clone())
            }
            _ => None,
        };
        let target = EndpointAddr::new(id).with_addrs([route]);
        let authenticated = AtomicBool::new(false);
        let work = async {
            let pending = self
                .endpoint
                .connect_with_opts(target, self.alpn, options)
                .await
                .map_err(ConnectError::from)
                .map_err(dial_error)?;
            if let Some((clock, _)) = relay {
                clock.send_if_modified(|current| {
                    if current.is_some() {
                        return false;
                    }
                    let at = Instant::now();
                    let stagger = (self.deadline.saturating_duration_since(at) / 4)
                        .min(Duration::from_millis(250));
                    *current = Some(RelayStart { at, stagger });
                    true
                });
            }
            let connection = pending
                .with_authenticated_identity(|peer, stable_id| {
                    authenticated.store(true, Ordering::Release);
                    progress.authenticated.store(true, Ordering::Release);
                    if let (Some(report), Some(ids)) = (progress.identity_confirmed, ids) {
                        report(ids, peer, stable_id);
                    }
                })
                .await
                .map_err(ConnectError::from)
                .map_err(dial_error)?;
            Ok(connection.stable_id())
        };
        let failed_relay = async {
            match unavailable {
                Some(receive) => {
                    if let Ok(mut status) = receive.await {
                        let _ = status.wait_for(|unavailable| *unavailable).await;
                    }
                }
                None => std::future::pending().await,
            }
            if authenticated.load(Ordering::Acquire) {
                std::future::pending().await
            }
            Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "relay registration unavailable",
            ))
        };
        let silent_contact = async {
            if let Some(until) = silent_until {
                time::sleep_until(until).await;
                if !authenticated.load(Ordering::Acquire) {
                    return Err(io::ErrorKind::TimedOut.into());
                }
            }
            std::future::pending().await
        };
        let unavailable_contact = async {
            if let Some(contact) = contact {
                self.endpoint
                    .inner
                    .inner
                    .custom_bootstrap_unavailable(&contact)
                    .await;
                if !authenticated.load(Ordering::Acquire) {
                    return Err(io::Error::new(
                        io::ErrorKind::NotConnected,
                        "contact sender unavailable",
                    ));
                }
            }
            std::future::pending().await
        };
        let result = tokio::select! { biased;
            _ = time::sleep_until(end) => {
                // Dropping the observer marks the still-pending route cancelled.
                return Err(io::ErrorKind::TimedOut.into());
            }
            result = work => result,
            result = failed_relay => result,
            result = unavailable_contact => result,
            result = silent_contact => result,
        };
        if let Some(finished) = finished {
            finished(result.as_ref().copied().map_err(io::Error::kind));
        }
        result.map(drop)
    }

    async fn parallel(
        &self,
        id: EndpointId,
        routes: Vec<TransportAddr>,
        end: Instant,
        progress: &RouteProgress<'_>,
    ) -> io::Result<()> {
        let mut flying = routes
            .into_iter()
            .map(|route| self.route(id, route, end, progress, None, None))
            .collect::<FuturesUnordered<_>>();
        let mut failure = None;
        while let Some(result) = flying.next().await {
            match result {
                Err(error) if !stops_dial(&error) => failure = Some(prefer_error(failure, error)),
                result => return result,
            }
        }
        Err(failure.unwrap_or_else(|| io::ErrorKind::NotConnected.into()))
    }

    async fn ordered(
        &self,
        id: EndpointId,
        routes: Vec<TransportAddr>,
        end: Instant,
        progress: &RouteProgress<'_>,
    ) -> io::Result<()> {
        let mut routes = routes.into_iter();
        let mut failure = None;
        while let Some(route) = routes.next() {
            let share = split_deadline(end, (routes.len() + 1) as u32);
            match self.route(id, route, share, progress, None, None).await {
                Err(error) if !stops_dial(&error) => failure = Some(prefer_error(failure, error)),
                result => return result,
            }
        }
        Err(failure.unwrap_or_else(|| io::ErrorKind::NotConnected.into()))
    }

    async fn relays(
        &self,
        id: EndpointId,
        routes: Vec<TransportAddr>,
        end: Instant,
        progress: &RouteProgress<'_>,
        clock: &RelayClock,
        track_failures: bool,
    ) -> io::Result<()> {
        let mut routes = routes.into_iter();
        let mut flying = FuturesUnordered::new();
        let mut failure = None;
        let mut start = clock.subscribe();
        let second = async {
            let first = first_relay(&mut start).await;
            time::sleep_until(first.at + first.stagger).await;
        };
        tokio::pin!(second);
        let mut slots = 1;

        // The first slot can expire before the second starts. Wait out the
        // stagger in that case as well. After it, pull in global route order.
        while !flying.is_empty() || routes.len() != 0 {
            while flying.len() < slots {
                let Some(route) = routes.next() else { break };
                let share = split_deadline(end, (routes.len() + 1).div_ceil(2) as u32);
                flying.push(self.route(
                    id,
                    route,
                    share,
                    progress,
                    Some((clock, track_failures)),
                    None,
                ));
            }
            tokio::select! { biased;
                Some(result) = flying.next(), if !flying.is_empty() => match result {
                    Err(error) if !stops_dial(&error) => {
                        failure = Some(prefer_error(failure, error));
                        if slots == 1 && clock.borrow().is_some() {
                            slots = 0;
                        }
                    },
                    result => return result,
                },
                _ = &mut second, if slots < 2 => slots = 2,
            }
        }
        Err(failure.unwrap_or_else(|| io::ErrorKind::NotConnected.into()))
    }
}

async fn first_relay(clock: &mut watch::Receiver<Option<RelayStart>>) -> RelayStart {
    clock
        .wait_for(Option::is_some)
        .await
        .expect("relay clock lives through the race")
        .unwrap()
}

pub(super) fn split_deadline(end: Instant, shares: u32) -> Instant {
    let now = Instant::now();
    now + end.saturating_duration_since(now) / shares
}

async fn race(
    left: impl Future<Output = io::Result<()>>,
    right: impl Future<Output = io::Result<()>>,
) -> io::Result<()> {
    tokio::pin!(left, right);
    let (failure, other) = tokio::select! { biased;
        result = &mut left => match result {
            Err(error) if !stops_dial(&error) => (error, right.await),
            result => return result,
        },
        result = &mut right => match result {
            Err(error) if !stops_dial(&error) => (error, left.await),
            result => return result,
        },
    };
    match other {
        Err(error) if !stops_dial(&error) => Err(prefer_error(Some(failure), error)),
        result => result,
    }
}

pub(super) fn stops_dial(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::PermissionDenied | io::ErrorKind::Interrupted
    )
}

pub(super) fn prefer_error(previous: Option<io::Error>, next: io::Error) -> io::Error {
    previous
        .filter(|error| error.kind() == io::ErrorKind::Unsupported)
        .unwrap_or(next)
}

fn dial_error(error: ConnectError) -> io::Error {
    let kind = match &error {
        ConnectError::Connect {
            source: ConnectWithOptsError::LocallyRejected { .. },
            ..
        } => io::ErrorKind::PermissionDenied,
        ConnectError::Connecting {
            source: ConnectingError::AdmissionFailed { kind, .. },
            ..
        } => match kind {
            io::ErrorKind::InvalidData => io::ErrorKind::InvalidInput,
            io::ErrorKind::Interrupted => io::ErrorKind::ConnectionAborted,
            kind => *kind,
        },
        ConnectError::Connecting {
            source:
                ConnectingError::ConnectionError {
                    source: ConnectionError::ApplicationClosed(closed),
                    ..
                },
            ..
        } if closed.reason.as_ref() == MEMBER_ADMISSION_DENIED => io::ErrorKind::PermissionDenied,
        ConnectError::Connecting {
            source:
                ConnectingError::ConnectionError {
                    source: ConnectionError::ApplicationClosed(closed),
                    ..
                },
            ..
        } if closed.reason.as_ref() == MEMBER_DATA_UNAVAILABLE => io::ErrorKind::NotConnected,
        ConnectError::Connecting {
            source:
                ConnectingError::ConnectionError {
                    source: ConnectionError::ConnectionClosed(closed),
                    ..
                },
            ..
        } if closed.error_code == noq::TransportErrorCode::crypto(0x78) => {
            io::ErrorKind::Unsupported
        }
        // The iroh TLS verifier reports unknown_ca when the actual raw public
        // key differs from the requested identity.
        ConnectError::Connecting {
            source:
                ConnectingError::ConnectionError {
                    source: ConnectionError::TransportError(transport),
                    ..
                },
            ..
        } if transport.code == noq::TransportErrorCode::crypto(0x30) => io::ErrorKind::InvalidData,
        ConnectError::Connect {
            source: ConnectWithOptsError::AttemptTimedOut { .. },
            ..
        }
        | ConnectError::Connecting {
            source: ConnectingError::AttemptTimedOut { .. },
            ..
        } => io::ErrorKind::TimedOut,
        ConnectError::Connect {
            source: ConnectWithOptsError::AttemptCancelled { .. },
            ..
        }
        | ConnectError::Connecting {
            source: ConnectingError::AttemptCancelled { .. },
            ..
        } => io::ErrorKind::Interrupted,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, error)
}
