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
use tokio::time::{self, Duration, Instant};
use tokio_util::sync::CancellationToken;

use crate::{RelayUrl, address_lookup::AddressLookupServices};

use super::{
    super::ConnectWithOptsError, ConnectError, ConnectOptions, ConnectingError, ConnectionError,
    EndpointAddr, MEMBER_ADMISSION_DENIED, RestrictedEndpoint, TransportAddr,
};

type RouteFinished = Box<dyn FnOnce(Result<usize, io::ErrorKind>) + Send>;
type RouteStarted =
    dyn Fn(&TransportAddr, Instant) -> ((usize, usize), RouteFinished) + Send + Sync;

/// Current contact inputs for a native member dial, evaluated by the owner.
#[derive(Default)]
pub struct MemberDialOptions {
    active: Vec<TransportAddr>,
    recent: Vec<TransportAddr>,
    relay_fallbacks: Vec<RelayUrl>,
    discover_ip: bool,
    route_started: Option<Box<RouteStarted>>,
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
            discover_ip: false,
            route_started: None,
        }
    }

    /// Look up additional IP contacts after the initial routes when direct
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
}

pub(super) struct Dial<'a> {
    pub(super) endpoint: &'a RestrictedEndpoint,
    pub(super) alpn: &'a [u8],
    pub(super) deadline: Instant,
    pub(super) cancel: &'a CancellationToken,
}

struct RelayBootstrap(Arc<AtomicBool>);

impl Drop for RelayBootstrap {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
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
        let recent_ip = options
            .recent
            .iter()
            .find(|route| route.is_ip() && target.addrs.contains(*route))
            .cloned();
        if let Some(route) = &recent_ip {
            target.addrs.remove(route);
        }
        let mut relays = target
            .addrs
            .iter()
            .filter(|route| route.is_relay())
            .cloned()
            .collect::<Vec<_>>();
        target.addrs.retain(|route| !route.is_relay());
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
        let cold = target.addrs.into_iter().collect::<Vec<_>>();
        let reporter = options.route_started.as_deref();
        let mut first = None;
        let has_race = recent_ip.is_some() || !relays.is_empty();
        let has_cold = !cold.is_empty() || lookup.is_some();
        if !active.is_empty() {
            let stages = 1 + u32::from(has_race) + u32::from(has_cold);
            let end = split_deadline(self.deadline, stages);
            match self.ordered(target.id, active, end, reporter).await {
                Err(error) if !stops_dial(&error) => first = Some(error),
                result => return result,
            }
        }
        if has_race {
            let end = split_deadline(self.deadline, 1 + u32::from(has_cold));
            let relay_work = self.relays(target.id, relays, end, reporter);
            let result = match recent_ip {
                Some(route) => race(self.route(target.id, route, end, reporter), relay_work).await,
                None => relay_work.await,
            };
            match result {
                Err(error) if !stops_dial(&error) => first = Some(prefer_error(first, error)),
                result => return result,
            }
        }
        if !cold.is_empty() {
            let end = split_deadline(self.deadline, 1 + u32::from(lookup.is_some()));
            match self.ordered(target.id, cold, end, reporter).await {
                Err(error) if !stops_dial(&error) => first = Some(prefer_error(first, error)),
                result => return result,
            }
        }
        if let Some(lookup) = lookup {
            return self
                .discover(target.id, lookup, known_ips, reporter, first)
                .await;
        }
        Err(first.unwrap_or_else(|| io::ErrorKind::NotConnected.into()))
    }

    async fn discover(
        &self,
        id: crate::EndpointId,
        lookup: &AddressLookupServices,
        mut seen: BTreeSet<TransportAddr>,
        reporter: Option<&RouteStarted>,
        mut failure: Option<io::Error>,
    ) -> io::Result<()> {
        let contacts = lookup.resolve(id);
        tokio::pin!(contacts);
        while let Some(contact) = contacts.next().await {
            let Ok(Ok(contact)) = contact else { continue };
            let routes = contact
                .into_endpoint_addr()
                .addrs
                .into_iter()
                .filter(|route| route.is_ip() && seen.insert(route.clone()))
                .collect::<Vec<_>>();
            if !routes.is_empty() {
                match self.ordered(id, routes, self.deadline, reporter).await {
                    Err(error) if !stops_dial(&error) => {
                        failure = Some(prefer_error(failure, error))
                    }
                    result => return result,
                }
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
        id: crate::EndpointId,
        route: TransportAddr,
        end: Instant,
        reporter: Option<&RouteStarted>,
    ) -> io::Result<()> {
        if Instant::now() >= end {
            return Err(io::ErrorKind::TimedOut.into());
        }
        let mut options = self.opts().with_bootstrap_route(route.clone());
        let _relay_scope = route.is_relay().then(|| {
            let released = Arc::new(AtomicBool::new(false));
            options.exact_relay_bootstrap = Some(released.clone());
            RelayBootstrap(released)
        });
        let (options, finished) = if let Some(start) = reporter {
            let ((attempt, phase), finish) = start(&route, self.deadline);
            (options.with_diagnostic_dial(attempt, phase), Some(finish))
        } else {
            (options, None)
        };
        let target = EndpointAddr::new(id).with_addrs([route]);
        let result = tokio::select! { biased;
            _ = time::sleep_until(end) => {
                // Dropping the observer marks the still-pending route cancelled.
                return Err(io::ErrorKind::TimedOut.into());
            }
            result = self.connect(target, options) => result.map_err(dial_error),
        };
        if let Some(finished) = finished {
            finished(result.as_ref().copied().map_err(io::Error::kind));
        }
        result.map(drop)
    }

    async fn ordered(
        &self,
        id: crate::EndpointId,
        routes: Vec<TransportAddr>,
        end: Instant,
        reporter: Option<&RouteStarted>,
    ) -> io::Result<()> {
        let mut routes = routes.into_iter();
        let mut failure = None;
        while let Some(route) = routes.next() {
            let share = split_deadline(end, (routes.len() + 1) as u32);
            match self.route(id, route, share, reporter).await {
                Err(error) if !stops_dial(&error) => failure = Some(prefer_error(failure, error)),
                result => return result,
            }
        }
        Err(failure.unwrap_or_else(|| io::ErrorKind::NotConnected.into()))
    }

    async fn relays(
        &self,
        id: crate::EndpointId,
        routes: Vec<TransportAddr>,
        end: Instant,
        reporter: Option<&RouteStarted>,
    ) -> io::Result<()> {
        let mut routes = routes.into_iter();
        let mut flying = FuturesUnordered::new();
        let mut failure = None;
        let start = Instant::now();
        let stagger =
            (self.deadline.saturating_duration_since(start) / 4).min(Duration::from_millis(250));
        let second = time::sleep_until(start + stagger);
        tokio::pin!(second);
        let mut second_started = false;

        // The first slot can expire before the second starts. Wait out the
        // stagger in that case as well. After it, pull in global route order.
        if let Some(route) = routes.next() {
            let share = split_deadline(end, (routes.len() + 1).div_ceil(2) as u32);
            flying.push(self.route(id, route, share, reporter));
        }
        while !flying.is_empty() || routes.len() != 0 {
            while second_started && flying.len() < 2 {
                let Some(route) = routes.next() else { break };
                let share = split_deadline(end, (routes.len() + 1).div_ceil(2) as u32);
                flying.push(self.route(id, route, share, reporter));
            }
            tokio::select! { biased;
                Some(result) = flying.next(), if !flying.is_empty() => match result {
                    Err(error) if !stops_dial(&error) => failure = Some(prefer_error(failure, error)),
                    result => return result,
                },
                _ = &mut second, if !second_started => second_started = true,
            }
        }
        Err(failure.unwrap_or_else(|| io::ErrorKind::NotConnected.into()))
    }
}

fn split_deadline(end: Instant, shares: u32) -> Instant {
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

fn stops_dial(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::PermissionDenied | io::ErrorKind::Interrupted
    )
}

fn prefer_error(previous: Option<io::Error>, next: io::Error) -> io::Error {
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
