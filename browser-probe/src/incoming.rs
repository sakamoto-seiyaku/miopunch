//! One incoming owner for the browser node. Foreground operations register
//! their purpose and original clock; TLS classification never runs gate hooks.
use super::*;
use futures_util::{FutureExt, StreamExt, stream::FuturesUnordered};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    pin::Pin,
    rc::Rc,
};
use tokio::sync::{Notify, oneshot};

struct Waiter {
    id: u64,
    alpn: Vec<u8>,
    attempt: crate::connection_coordinator::AttemptContext,
    sender: oneshot::Sender<iroh::endpoint::RestrictedAccepting>,
}

#[derive(Default)]
struct State {
    waiters: RefCell<VecDeque<Waiter>>,
    sequence: std::cell::Cell<u64>,
    pending_tls: std::cell::Cell<usize>,
    rejected: std::cell::Cell<u64>,
    stopped: std::cell::Cell<bool>,
    changed: Notify,
    joined: Notify,
    active: RefCell<Option<ActiveRoute>>,
    active_in_flight: Cell<usize>,
}

#[derive(Clone)]
struct ActiveRoute {
    alpn: Vec<u8>,
    budget: Rc<dyn Fn() -> Duration>,
    handler: Rc<
        dyn Fn(
            iroh::endpoint::RestrictedAccepting,
            n0_future::time::Instant,
        ) -> Pin<Box<dyn std::future::Future<Output = ()>>>,
    >,
}

pub(super) struct IncomingDispatcher {
    state: Rc<State>,
    cancel: tokio_util::sync::CancellationToken,
}

impl Drop for IncomingDispatcher {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

struct Registration {
    state: Rc<State>,
    id: u64,
}

struct ActiveHandler(Rc<State>);
impl Drop for ActiveHandler {
    fn drop(&mut self) {
        self.0.active_in_flight.set(0);
        self.0.changed.notify_one();
    }
}

enum Event {
    Closed,
    Changed,
    Classified(
        Option<
            Result<
                (
                    Vec<u8>,
                    iroh::endpoint::RestrictedAccepting,
                    Option<n0_future::time::Instant>,
                ),
                JsValue,
            >,
        >,
    ),
    Incoming(Option<iroh::endpoint::RestrictedIncoming>),
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.state
            .waiters
            .borrow_mut()
            .retain(|waiter| waiter.id != self.id);
        self.state.changed.notify_one();
    }
}

impl IncomingDispatcher {
    pub(super) fn start(endpoint: RestrictedEndpoint) -> Rc<Self> {
        let state = Rc::new(State::default());
        let cancel = tokio_util::sync::CancellationToken::new();
        let dispatcher = Rc::new(Self {
            state: state.clone(),
            cancel: cancel.clone(),
        });
        wasm_bindgen_futures::spawn_local(async move {
            let mut classifications = FuturesUnordered::new();
            loop {
                let now = n0_future::time::Instant::now();
                state.waiters.borrow_mut().retain(|waiter| {
                    !waiter.sender.is_closed()
                        && !waiter.attempt.cancellation.is_cancelled()
                        && waiter.attempt.deadline > now
                });
                let waiting = state.waiters.borrow().len();
                let active_route = state.active.borrow().is_some();
                // An installed active route owns incoming classification even
                // while no foreground operation is waiting. Retiring the
                // pending TLS futures here would drop an active-control C
                // that has already reached this one dispatcher.
                if waiting == 0 && !active_route {
                    classifications.clear();
                }
                state.pending_tls.set(classifications.len());
                let event = {
                    let changed = state.changed.notified().fuse();
                    let stopped = cancel.cancelled().fuse();
                    // Keep one active-control classification/handler owned by
                    // this dispatcher. Foreground callers still contribute
                    // one bounded classification each, but an installed
                    // route cannot turn endpoint.accept() into an unbounded
                    // queue when no foreground waiter exists.
                    let active_capacity =
                        usize::from(active_route && state.active_in_flight.get() == 0);
                    let accept_more =
                        classifications.len() < waiting.saturating_add(active_capacity);
                    let accepting = async {
                        if accept_more {
                            endpoint.accept().await
                        } else {
                            futures_util::future::pending().await
                        }
                    }
                    .fuse();
                    let classified = async {
                        if classifications.is_empty() {
                            futures_util::future::pending().await
                        } else {
                            classifications.next().await
                        }
                    }
                    .fuse();
                    futures_util::pin_mut!(changed, stopped, accepting, classified);
                    futures_util::select_biased! {
                        _ = stopped => Event::Closed,
                        _ = changed => Event::Changed,
                        item = classified => Event::Classified(item),
                        incoming = accepting => Event::Incoming(incoming),
                    }
                };
                match event {
                    Event::Closed => break,
                    Event::Changed => continue,
                    Event::Classified(item) => {
                        if let Some(Ok((alpn, accepting, active_deadline))) = item {
                            let waiter = {
                                let mut waiters = state.waiters.borrow_mut();
                                waiters
                                    .iter()
                                    .position(|waiter| waiter.alpn == alpn)
                                    .and_then(|position| waiters.remove(position))
                            };
                            if let Some(waiter) = waiter {
                                // Dropping the receiver on cancellation drops this exact raw M.
                                let _ = waiter.sender.send(accepting);
                            } else if let Some(route) = state
                                .active
                                .borrow()
                                .as_ref()
                                .filter(|route| route.alpn == alpn)
                                .cloned()
                            {
                                if state.active_in_flight.get() != 0 {
                                    state.rejected.set(state.rejected.get() + 1);
                                    continue;
                                }
                                let Some(deadline) = active_deadline else {
                                    state.rejected.set(state.rejected.get() + 1);
                                    continue;
                                };
                                state.active_in_flight.set(1);
                                let route_owner = ActiveHandler(state.clone());
                                let route_cancel = cancel.clone();
                                wasm_bindgen_futures::spawn_local(async move {
                                    // Declared before work: its Drop publishes completion only
                                    // after the handler's C/candidate future has been dropped.
                                    let _owner = route_owner;
                                    let work = (route.handler)(accepting, deadline).fuse();
                                    let stopped = route_cancel.cancelled().fuse();
                                    futures_util::pin_mut!(work, stopped);
                                    futures_util::select_biased! {
                                        _ = stopped => {},
                                        _ = work => {},
                                    }
                                });
                            } else {
                                state.rejected.set(state.rejected.get() + 1);
                            }
                        } else {
                            state.rejected.set(state.rejected.get() + 1);
                        }
                    }
                    Event::Incoming(incoming) => {
                        let Some(incoming) = incoming else {
                            break;
                        };
                        // Use only registered callers' existing clocks while the negotiated
                        // purpose is unknown. The selected caller's own clock remains final.
                        let deadline = state
                            .waiters
                            .borrow()
                            .iter()
                            .map(|waiter| waiter.attempt.deadline)
                            .max();
                        let active_deadline = state
                            .active
                            .borrow()
                            .as_ref()
                            .map(|route| incoming.received_at() + (route.budget)());
                        let deadline = deadline
                            .into_iter()
                            .chain(active_deadline)
                            .max()
                            .unwrap_or_else(|| {
                                n0_future::time::Instant::now() + crate::connection_budget::DEFAULT
                            });
                        if state.active.borrow().is_some() || waiting > 0 {
                            classifications.push(async move {
                                let mut accepting = incoming
                                    .accept()
                                    .map_err(error)?
                                    .with_caller_deadline(deadline);
                                let alpn = n0_future::time::timeout(
                                    deadline
                                        .saturating_duration_since(n0_future::time::Instant::now()),
                                    accepting.alpn(),
                                )
                                .await
                                .map_err(error)?
                                .map_err(error)?;
                                Ok::<_, JsValue>((alpn, accepting, active_deadline))
                            });
                        } else {
                            incoming.refuse();
                        }
                    }
                }
            }
            classifications.clear();
            state.waiters.borrow_mut().clear();
            state.pending_tls.set(0);
            cancel.cancel();
            while state.active_in_flight.get() != 0 {
                let changed = state.changed.notified();
                if state.active_in_flight.get() == 0 {
                    break;
                }
                changed.await;
            }
            state.stopped.set(true);
            state.joined.notify_waiters();
        });
        dispatcher
    }

    pub(super) async fn receive(
        &self,
        alpn: &[u8],
        attempt: &crate::connection_coordinator::AttemptContext,
    ) -> Result<iroh::endpoint::RestrictedAccepting, JsValue> {
        if self.cancel.is_cancelled() {
            return Err(error("browser incoming dispatcher closed"));
        }
        let id = self.state.sequence.get() + 1;
        self.state.sequence.set(id);
        let (sender, receiver) = oneshot::channel();
        self.state.waiters.borrow_mut().push_back(Waiter {
            id,
            alpn: alpn.to_vec(),
            attempt: attempt.clone(),
            sender,
        });
        let _registration = Registration {
            state: self.state.clone(),
            id,
        };
        self.state.changed.notify_one();
        let received = receiver.fuse();
        let cancelled = attempt.cancellation.cancelled().fuse();
        let closed = self.cancel.cancelled().fuse();
        futures_util::pin_mut!(received, cancelled, closed);
        n0_future::time::timeout(
            attempt.deadline.saturating_duration_since(n0_future::time::Instant::now()),
            async {
                futures_util::select_biased! {
                    _ = cancelled => Err(error("incoming attempt cancelled")),
                    _ = closed => Err(error("browser incoming dispatcher closed")),
                    result = received => result.map_err(|_| error("browser incoming dispatcher closed")),
                }
            },
        ).await.map_err(|_| error("incoming caller deadline exceeded"))?
    }

    pub(super) fn close(&self) {
        self.cancel.cancel();
    }

    pub(super) fn install_active<B, F, Fut>(&self, alpn: &[u8], budget: B, handler: F)
    where
        B: Fn() -> Duration + 'static,
        F: Fn(iroh::endpoint::RestrictedAccepting, n0_future::time::Instant) -> Fut + 'static,
        Fut: std::future::Future<Output = ()> + 'static,
    {
        let route = ActiveRoute {
            alpn: alpn.to_vec(),
            budget: Rc::new(budget),
            handler: Rc::new(move |accepting, deadline| Box::pin(handler(accepting, deadline))),
        };
        *self.state.active.borrow_mut() = Some(route);
        self.state.changed.notify_one();
    }

    pub(super) async fn stopped(&self) {
        loop {
            let joined = self.state.joined.notified();
            if self.state.stopped.get() {
                return;
            }
            joined.await;
        }
    }

    pub(super) fn diagnostics(&self) -> serde_json::Value {
        serde_json::json!({"pendingWaiters":self.state.waiters.borrow().len(),
            "pendingTls":self.state.pending_tls.get(),"activeInFlight":self.state.active_in_flight.get(),
            "rejectedIncoming":self.state.rejected.get(),
            "dispatcherStopped":self.state.stopped.get()})
    }
}
