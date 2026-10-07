//! Ownership for one bounded improvement of an already permitted member M.
//! The coordinator supplies strict protected negotiation and candidate RAII;
//! this module never opens, selects, authenticates, or closes a data path/M.
use crate::connection_coordinator::AttemptContext;
use futures_util::FutureExt;
use iroh::endpoint::{Connection, WeakConnectionHandle};
use std::{
    collections::HashMap,
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Clone, Debug, Default)]
pub(crate) struct SessionMaintenance(Arc<Registry>);
#[derive(Debug, Default)]
struct Registry {
    next: AtomicU64,
    slots: Mutex<HashMap<usize, Slot>>,
    originals: Mutex<HashMap<usize, WeakConnectionHandle>>,
}
#[derive(Debug)]
struct Slot {
    epoch: u64,
    original: WeakConnectionHandle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MaintenanceError {
    Busy,
    Cancelled,
    DeadlineExceeded,
    SessionClosed,
    EpochExhausted,
}

impl SessionMaintenance {
    /// Register an ordinary member M after the common coordinator has released
    /// it.  The registry is deliberately weak: it observes the existing
    /// session owner and never keeps an application connection alive.
    pub(crate) fn register(&self, original: &Connection) {
        self.0
            .originals
            .lock()
            .unwrap()
            .insert(original.stable_id(), original.weak_handle());
    }

    /// Return currently live ordinary Ms for the existing NodeMaintenance
    /// owner.  Closed or dropped handles are removed as they are observed.
    pub(crate) fn live(&self) -> Vec<Connection> {
        let mut originals = self.0.originals.lock().unwrap();
        let mut live = Vec::new();
        originals.retain(|_, weak| {
            let Some(connection) = weak.upgrade() else {
                return false;
            };
            if connection.close_reason().is_some() {
                return false;
            }
            live.push(connection);
            true
        });
        live
    }

    /// Resolve a live M for an authenticated active-control binding.  The
    /// binding is checked against the exact weak handle before returning it;
    /// a late control event cannot attach to a replacement session.
    pub(crate) fn for_binding(
        &self,
        peer: iroh::EndpointId,
        binding: &[u8; 32],
    ) -> Option<Connection> {
        let mut originals = self.0.originals.lock().unwrap();
        let mut found = None;
        originals.retain(|_, weak| {
            let Some(connection) = weak.upgrade() else {
                return false;
            };
            if connection.close_reason().is_some() {
                return false;
            }
            if found.is_none()
                && connection.remote_id() == peer
                && crate::connection_recovery_core::binding(&connection)
                    .is_ok_and(|actual| &actual == binding)
            {
                found = Some(connection);
            }
            true
        });
        found
    }

    /// The caller passes its already-acquired existing Node budget guard. This
    /// owner holds it through the attempt; it creates no quota, retry or timer.
    pub(crate) fn acquire<B>(
        &self,
        original: &Connection,
        attempt: &AttemptContext,
        node_budget_guard: B,
    ) -> Result<MaintenanceLease<B>, MaintenanceError> {
        if attempt.cancellation.is_cancelled() {
            return Err(MaintenanceError::Cancelled);
        }
        if attempt.deadline <= n0_future::time::Instant::now() {
            return Err(MaintenanceError::DeadlineExceeded);
        }
        if original.close_reason().is_some() {
            return Err(MaintenanceError::SessionClosed);
        }
        let stable = original.stable_id();
        let mut slots = self.0.slots.lock().unwrap();
        if slots
            .get(&stable)
            .is_some_and(|slot| slot.original.is_open())
        {
            return Err(MaintenanceError::Busy);
        }
        let epoch = self
            .0
            .next
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .map_err(|_| MaintenanceError::EpochExhausted)?;
        let original = original.weak_handle();
        slots.insert(
            stable,
            Slot {
                epoch,
                original: original.clone(),
            },
        );
        Ok(MaintenanceLease {
            registry: self.0.clone(),
            stable,
            epoch,
            original,
            attempt: attempt.clone(),
            _node_budget_guard: node_budget_guard,
        })
    }

    pub(crate) fn active_attempts(&self) -> usize {
        self.0
            .slots
            .lock()
            .unwrap()
            .values()
            .filter(|slot| slot.original.is_open())
            .count()
    }
}

/// Dropping this lease only releases the local per-M reservation and current
/// Node budget. It has no original-M close operation or setup-lifetime holder.
pub(crate) struct MaintenanceLease<B> {
    registry: Arc<Registry>,
    stable: usize,
    epoch: u64,
    original: WeakConnectionHandle,
    attempt: AttemptContext,
    _node_budget_guard: B,
}
impl<B> MaintenanceLease<B> {
    pub(crate) fn stable_id(&self) -> usize {
        self.stable
    }
    pub(crate) fn context(&self) -> &AttemptContext {
        &self.attempt
    }

    /// `work` owns only this candidate and its private control streams. The
    /// common coordinator must finish fresh ACK, current permission and both
    /// confirmations before returning success; selection remains its existing
    /// timer/policy. Dropping work on any losing race runs candidate-only RAII.
    pub(crate) fn run<T>(
        self,
        work: impl Future<Output = T>,
    ) -> impl Future<Output = Result<T, MaintenanceError>> {
        let work = run_reserved_work(self.original.clone(), self.attempt.clone(), work);
        async move {
            let result = work.await;
            drop(self);
            result
        }
    }

    /// Explicit repair keeps this same reservation/quota through its failure
    /// decision. The candidate race finishes first; ownership ends when this
    /// caller drops the lease after settlement, including Terminal closure.
    pub(crate) fn run_ref<T>(
        &self,
        work: impl Future<Output = T>,
    ) -> impl Future<Output = Result<T, MaintenanceError>> {
        run_reserved_work(self.original.clone(), self.attempt.clone(), work)
    }
}

fn run_reserved_work<T>(
    original: WeakConnectionHandle,
    attempt: AttemptContext,
    work: impl Future<Output = T>,
) -> impl Future<Output = Result<T, MaintenanceError>> {
    // RTC preparation futures are large. Box before the async reservation race
    // so nested wrappers do not repeatedly move them on a Tokio worker's stack.
    let work = Box::pin(work);
    async move {
        if !original.is_open() {
            return Err(MaintenanceError::SessionClosed);
        }
        if attempt.cancellation.is_cancelled() {
            return Err(MaintenanceError::Cancelled);
        }
        if attempt.deadline <= n0_future::time::Instant::now() {
            return Err(MaintenanceError::DeadlineExceeded);
        }
        let cancelled = attempt.cancellation.cancelled().fuse();
        let closed = original.closed().fuse();
        let work = work.fuse();
        futures_util::pin_mut!(cancelled, closed, work);
        n0_future::time::timeout(
            attempt
                .deadline
                .saturating_duration_since(n0_future::time::Instant::now()),
            async {
                futures_util::select_biased! {
                    _ = closed => Err(MaintenanceError::SessionClosed),
                    _ = cancelled => Err(MaintenanceError::Cancelled),
                    value = work => Ok(value),
                }
            },
        )
        .await
        .map_err(|_| MaintenanceError::DeadlineExceeded)?
    }
}

impl<B> Drop for MaintenanceLease<B> {
    fn drop(&mut self) {
        let mut slots = self.registry.slots.lock().unwrap();
        // A late old guard cannot remove a replacement reservation even if a
        // closed connection's local numeric stable ID has been reused.
        if slots
            .get(&self.stable)
            .is_some_and(|slot| slot.epoch == self.epoch)
        {
            slots.remove(&self.stable);
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    //! Real loopback iroh ownership checks only: no SDK admission, RTC, relay,
    //! or path qualification is claimed by this transport-level unit fixture.
    use super::*;
    use iroh::{
        Endpoint, EndpointAddr, RelayMode, TransportAddr,
        endpoint::{PortmapperConfig, presets},
    };
    use std::{net::Ipv4Addr, sync::atomic::AtomicUsize, time::Duration};

    struct Dropped(Arc<AtomicUsize>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::AcqRel);
        }
    }

    async fn pair() -> (Endpoint, Endpoint, Connection, Connection) {
        let mut endpoints = Vec::new();
        for _ in 0..2 {
            endpoints.push(
                Endpoint::builder(presets::Minimal)
                    .clear_ip_transports()
                    .bind_addr((Ipv4Addr::LOCALHOST, 0))
                    .unwrap()
                    .relay_mode(RelayMode::Disabled)
                    .portmapper_config(PortmapperConfig::Disabled)
                    .alpns(vec![b"mio77/ownership-unit/1".to_vec()])
                    .bind()
                    .await
                    .unwrap(),
            );
        }
        let server = endpoints.pop().unwrap();
        let client = endpoints.pop().unwrap();
        let address = server.bound_sockets()[0];
        let target = EndpointAddr::new(server.id()).with_addrs([TransportAddr::Ip(address)]);
        let (client_m, server_m) = tokio::join!(
            async {
                client
                    .connect(target, b"mio77/ownership-unit/1")
                    .await
                    .unwrap()
            },
            async { server.accept().await.unwrap().await.unwrap() }
        );
        (client, server, client_m, server_m)
    }

    fn context(ms: u64) -> AttemptContext {
        AttemptContext {
            deadline: n0_future::time::Instant::now() + Duration::from_millis(ms),
            cancellation: tokio_util::sync::CancellationToken::new(),
        }
    }

    #[tokio::test]
    async fn cancelled_work_retires_only_owned_candidate_and_budget_not_original_stream() {
        let (client_ep, server_ep, client, server) = pair().await;
        let (mut send, mut reply) = client.open_bi().await.unwrap();
        send.write_all(b"before").await.unwrap();
        let (mut echo, mut receive) = server.accept_bi().await.unwrap();
        let mut first = [0; 6];
        receive.read_exact(&mut first).await.unwrap();
        assert_eq!(&first, b"before");
        let registry = SessionMaintenance::default();
        let attempt = context(1000);
        let budgets = Arc::new(AtomicUsize::new(0));
        let candidates = Arc::new(AtomicUsize::new(0));
        let lease = registry
            .acquire(&client, &attempt, Dropped(budgets.clone()))
            .unwrap();
        assert!(matches!(
            registry.acquire(&client, &attempt, ()),
            Err(MaintenanceError::Busy)
        ));
        let started = Arc::new(tokio::sync::Notify::new());
        let work_started = started.clone();
        let candidate = Dropped(candidates.clone());
        let task = tokio::spawn(lease.run(async move {
            let _candidate = candidate;
            work_started.notify_one();
            std::future::pending::<()>().await;
        }));
        started.notified().await;
        attempt.cancellation.cancel();
        assert_eq!(task.await.unwrap(), Err(MaintenanceError::Cancelled));
        assert_eq!(candidates.load(Ordering::Acquire), 1);
        assert_eq!(budgets.load(Ordering::Acquire), 1);
        assert_eq!(registry.active_attempts(), 0);
        assert!(client.close_reason().is_none());
        send.write_all(b"after").await.unwrap();
        let mut next = [0; 5];
        receive.read_exact(&mut next).await.unwrap();
        assert_eq!(&next, b"after");
        echo.write_all(b"same stream").await.unwrap();
        let mut answer = [0; 11];
        reply.read_exact(&mut answer).await.unwrap();
        assert_eq!(&answer, b"same stream");
        drop((send, reply, echo, receive));
        client.close(0u32.into(), b"ownership unit ended");
        server.close(0u32.into(), b"ownership unit ended");
        tokio::join!(client_ep.close(), server_ep.close());
    }

    #[tokio::test]
    async fn elapsed_context_does_not_poll_late_work_or_end_healthy_original() {
        let (client_ep, server_ep, client, server) = pair().await;
        let registry = SessionMaintenance::default();
        let attempt = context(10);
        let budgets = Arc::new(AtomicUsize::new(0));
        let lease = registry
            .acquire(&client, &attempt, Dropped(budgets.clone()))
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        let polled = Arc::new(AtomicUsize::new(0));
        let work_polled = polled.clone();
        assert_eq!(
            lease
                .run(async move {
                    work_polled.fetch_add(1, Ordering::AcqRel);
                })
                .await,
            Err(MaintenanceError::DeadlineExceeded)
        );
        assert_eq!(polled.load(Ordering::Acquire), 0);
        assert_eq!(budgets.load(Ordering::Acquire), 1);
        assert_eq!(registry.active_attempts(), 0);
        assert!(client.close_reason().is_none());
        client.close(0u32.into(), b"ownership unit ended");
        server.close(0u32.into(), b"ownership unit ended");
        tokio::join!(client_ep.close(), server_ep.close());
    }

    #[tokio::test]
    async fn registry_binds_active_control_to_the_same_live_original() {
        let (client_ep, server_ep, client, server) = pair().await;
        let registry = SessionMaintenance::default();
        registry.register(&client);
        let binding = crate::connection_recovery_core::binding(&client).unwrap();
        assert_eq!(registry.live().len(), 1);
        let original_peer = client.remote_id();
        let wrong_peer = server.remote_id();
        assert_ne!(original_peer, wrong_peer);
        assert!(registry.for_binding(wrong_peer, &binding).is_none());
        assert!(registry.for_binding(original_peer, &binding).is_some());
        client.close(0u32.into(), b"ownership registry ended");
        assert!(registry.live().is_empty());
        assert!(registry.for_binding(original_peer, &binding).is_none());
        server.close(0u32.into(), b"ownership registry ended");
        tokio::join!(client_ep.close(), server_ep.close());
    }
}
