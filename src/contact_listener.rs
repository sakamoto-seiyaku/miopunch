//! One selected MQTT contact carrier. NodeMaintenance owns retry/backoff;
//! this adapter owns only a generation's IO, readiness and terminal retirement.
use crate::mqtt_transport::MqttTransport;
use futures_util::task::AtomicWaker;
use iroh::endpoint::transports::{
    CustomEndpoint, CustomSender, CustomTransport, RecvInfo, Transmit,
};
use iroh_base::CustomAddr;
use n0_watcher::{Direct, Watchable, Watcher};
use serde::Serialize;
use std::{
    io,
    sync::{Arc, Mutex, Weak},
    task::{Context, Poll},
};
use tokio::{sync::watch, task::JoinHandle};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum ContactListenerPhase {
    Unbound,
    Starting,
    Ready,
    Unavailable,
    Stopped,
}

#[derive(Clone, Debug, Serialize)]
pub struct ContactListenerStatus {
    pub generation: u64,
    pub phase: ContactListenerPhase,
    pub ready_routes: usize,
    pub starts: u64,
    pub withdrawals: u64,
    pub failure: Option<String>,
}
impl Default for ContactListenerStatus {
    fn default() -> Self {
        Self {
            generation: 0,
            phase: ContactListenerPhase::Unbound,
            ready_routes: 0,
            starts: 0,
            withdrawals: 0,
            failure: None,
        }
    }
}

#[derive(Debug)]
struct Generation {
    endpoint: Box<dyn CustomEndpoint>,
    sender: Arc<dyn CustomSender>,
    observer: Option<JoinHandle<()>>,
}
impl Drop for Generation {
    fn drop(&mut self) {
        if let Some(observer) = &self.observer {
            observer.abort();
        }
    }
}
#[derive(Debug)]
struct State {
    bound: bool,
    status: ContactListenerStatus,
    current: Option<Generation>,
    retired_observers: Vec<JoinHandle<()>>,
}
#[derive(Debug)]
struct Shared {
    transport: Arc<MqttTransport>,
    state: Mutex<State>,
    addresses: Watchable<Vec<CustomAddr>>,
    status: watch::Sender<ContactListenerStatus>,
    receive_waker: AtomicWaker,
}

/// Installed once on the owning endpoint. A closed MQTT generation does not
/// destroy this receiver, so the existing maintenance task can restart it.
#[derive(Clone, Debug)]
pub struct ContactListener {
    shared: Arc<Shared>,
}
/// The Node owner keeps this control; no reconnect task runs in this handle.
#[derive(Clone, Debug)]
pub struct ContactListenerControl {
    shared: Arc<Shared>,
}

impl ContactListener {
    pub fn new(transport: Arc<MqttTransport>) -> Self {
        let status = ContactListenerStatus::default();
        Self {
            shared: Arc::new(Shared {
                transport,
                state: Mutex::new(State {
                    bound: false,
                    status: status.clone(),
                    current: None,
                    retired_observers: Vec::new(),
                }),
                addresses: Watchable::new(Vec::new()),
                status: watch::channel(status).0,
                receive_waker: AtomicWaker::new(),
            }),
        }
    }
    pub fn control(&self) -> ContactListenerControl {
        ContactListenerControl {
            shared: self.shared.clone(),
        }
    }
}
impl ContactListenerControl {
    pub fn resource_url(&self) -> iroh::RelayUrl {
        self.shared
            .transport
            .resource()
            .expect("MQTT resource descriptor")
            .0
    }
    pub fn status(&self) -> ContactListenerStatus {
        self.shared.status.borrow().clone()
    }
    pub fn watch(&self) -> watch::Receiver<ContactListenerStatus> {
        self.shared.status.subscribe()
    }
    pub fn local_addresses(&self) -> Vec<CustomAddr> {
        self.shared.addresses.get()
    }
    /// Called only by the existing maintenance selector after its budget/backoff.
    /// The carrier itself refuses rebinding until the previous driver ended.
    pub fn restart(&self) -> io::Result<()> {
        self.shared.start()
    }
    pub async fn shutdown(&self) -> io::Result<()> {
        let observers = {
            let mut state = self.shared.state.lock().unwrap();
            self.shared.retire_locked(&mut state, true, None);
            std::mem::take(&mut state.retired_observers)
        };
        for observer in observers {
            if let Err(error) = observer.await {
                if !error.is_cancelled() {
                    return Err(io::Error::other(error));
                }
            }
        }
        if self.shared.transport.traffic().snapshot()["generation"] != 0 {
            self.shared.transport.wait_driver_closed().await?;
        }
        Ok(())
    }
}
impl Shared {
    fn publish_locked(&self, state: &State) {
        self.status.send_replace(state.status.clone());
    }
    fn start(self: &Arc<Self>) -> io::Result<()> {
        let mut state = self.state.lock().unwrap();
        state
            .retired_observers
            .retain(|observer| !observer.is_finished());
        if !state.bound || state.status.phase == ContactListenerPhase::Stopped {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "contact listener is not an active endpoint resource",
            ));
        }
        if state.current.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "contact generation still active",
            ));
        }
        let generation = state
            .status
            .generation
            .checked_add(1)
            .ok_or_else(|| io::Error::other("contact generation exhausted"))?;
        let endpoint = self.transport.bind()?;
        let sender = endpoint.create_sender();
        let updates = endpoint.watch_local_addrs();
        state.status.generation = generation;
        state.status.starts += 1;
        state.status.phase = ContactListenerPhase::Starting;
        state.status.failure = None;
        state.current = Some(Generation {
            endpoint,
            sender,
            observer: None,
        });
        self.publish_locked(&state);
        let shared = Arc::downgrade(self);
        let transport = self.transport.clone();
        state.current.as_mut().unwrap().observer = Some(tokio::spawn(observe_generation(
            shared, transport, generation, updates,
        )));
        self.receive_waker.wake();
        Ok(())
    }
    fn ready(&self, generation: u64, addresses: Vec<CustomAddr>) -> bool {
        let mut state = self.state.lock().unwrap();
        if state.status.generation != generation
            || state.current.is_none()
            || !matches!(
                state.status.phase,
                ContactListenerPhase::Starting | ContactListenerPhase::Ready
            )
            || !self.transport.diagnostics().1
            || addresses.is_empty()
        {
            return false;
        }
        state.status.phase = ContactListenerPhase::Ready;
        state.status.ready_routes = addresses.len();
        self.addresses.set(addresses).ok();
        self.publish_locked(&state);
        self.receive_waker.wake();
        true
    }
    fn retire_locked(&self, state: &mut State, terminal: bool, failure: Option<String>) {
        if state.status.phase == ContactListenerPhase::Stopped {
            return;
        }
        // Withdraw eligibility while holding the same lock used for publication.
        self.addresses.set(Vec::new()).ok();
        if state.current.is_some() {
            state.status.withdrawals += 1;
        }
        state.status.ready_routes = 0;
        state.status.phase = if terminal {
            ContactListenerPhase::Stopped
        } else {
            ContactListenerPhase::Unavailable
        };
        state.status.failure = failure;
        if let Some(mut current) = state.current.take() {
            if let Some(observer) = current.observer.take() {
                observer.abort();
                state.retired_observers.push(observer);
            }
            drop(current);
        }
        self.publish_locked(state);
        self.receive_waker.wake();
    }
    fn failed(&self, generation: u64, failure: Option<String>) {
        let mut state = self.state.lock().unwrap();
        if state.status.generation == generation && state.current.is_some() {
            self.retire_locked(&mut state, false, failure);
        }
    }
}

async fn observe_generation(
    shared: Weak<Shared>,
    transport: Arc<MqttTransport>,
    generation: u64,
    mut updates: Direct<Vec<CustomAddr>>,
) {
    if let Err(error) = transport.wait_ready().await {
        if let Some(shared) = shared.upgrade() {
            shared.failed(generation, Some(error.to_string()));
        }
        return;
    }
    if let Some(shared) = shared.upgrade() {
        shared.ready(generation, updates.get());
    }
    loop {
        tokio::select! {
            biased;
            result = transport.wait_closed() => {
                let failure = result.err().map(|error|error.to_string()).or_else(|| {
                    transport.traffic().snapshot()["protocol"]["failure"].as_str().map(str::to_owned)
                }).or_else(|| Some("contact carrier closed".into()));
                if let Some(shared) = shared.upgrade() { shared.failed(generation, failure); }
                break;
            }
            result = updates.updated() => {
                let Some(shared) = shared.upgrade() else { break; };
                if result.is_err() || updates.get().is_empty() {
                    shared.failed(generation, Some("contact route withdrawn".into())); break;
                }
                shared.ready(generation, updates.get());
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/support/contact_broker.rs"]
mod test_broker;

#[cfg(test)]
mod tests {
    use super::test_broker::Broker;
    use super::*;
    use crate::mqtt_transport::Traffic;
    use std::{sync::atomic::Ordering, time::Duration};

    async fn phase(control: &ContactListenerControl, expected: ContactListenerPhase) {
        let mut updates = control.watch();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if updates.borrow_and_update().phase == expected {
                    return;
                }
                updates.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn ready_disconnect_restart_and_late_callback_preserve_exact_generation() {
        let broker = Broker::new().await;
        let transport = Arc::new(MqttTransport::new(
            broker.address,
            Arc::new(Traffic::default()),
        ));
        let listener = ContactListener::new(transport.clone());
        let control = listener.control();
        let endpoint = listener.bind().unwrap();
        phase(&control, ContactListenerPhase::Ready).await;
        let original_generation = control.status().generation;
        let original_addresses = control.local_addresses();
        let sender = endpoint.create_sender();
        assert_eq!(original_addresses.len(), 1);
        broker.disconnect();
        phase(&control, ContactListenerPhase::Unavailable).await;
        assert!(control.local_addresses().is_empty());
        assert!(!sender.is_valid_send_addr(&original_addresses[0]));
        transport.wait_driver_closed().await.unwrap();
        control.restart().unwrap();
        phase(&control, ContactListenerPhase::Ready).await;
        let current_addresses = control.local_addresses();
        assert!(control.status().generation > original_generation);
        assert!(current_addresses != original_addresses);
        // Inject the exact observer continuation after a real new generation is
        // ready. The current driver is valid, so token comparison must reject it.
        assert!(
            !listener
                .shared
                .ready(original_generation, original_addresses)
        );
        assert!(control.local_addresses() == current_addresses);
        drop(endpoint);
        let cleanup_started = tokio::time::Instant::now();
        control.shutdown().await.unwrap();
        assert_eq!(control.status().phase, ContactListenerPhase::Stopped);
        assert!(control.local_addresses().is_empty());
        assert_eq!(transport.traffic().snapshot()["driver_active"], 0);
        assert_eq!(transport.traffic().snapshot()["queued_packets"], 0);
        assert!(control.restart().is_err());
        let sockets = broker.no_sockets().await;
        println!(
            "CONTACT_GENERATION_PASS starts={} withdrawals={} stale_callback_rejected=true mqtt_immediate_empty={} mqtt_settled_ms={} mqtt_late_empty={} cleanup_ms={}",
            control.status().starts,
            control.status().withdrawals,
            sockets.immediate_empty,
            sockets.settled_ms,
            sockets.late_empty,
            cleanup_started.elapsed().as_millis()
        );
        broker.close().await;
    }

    #[tokio::test]
    async fn disabled_endpoint_resource_never_binds_the_listener_socket() {
        let broker = Broker::new().await;
        let listener = ContactListener::new(Arc::new(MqttTransport::new(
            broker.address,
            Arc::new(Traffic::default()),
        )));
        let control = listener.control();
        let endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
            .clear_ip_transports()
            .relay_mode(iroh::RelayMode::Disabled)
            .relay_resource_policy(iroh::endpoint::RelayResourcePolicy::new(|_, _| false))
            .add_custom_transport(Arc::new(listener))
            .bind()
            .await;
        assert!(endpoint.is_err());
        assert_eq!(control.status().phase, ContactListenerPhase::Unbound);
        assert!(control.restart().is_err());
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(broker.connections.load(Ordering::SeqCst), 0);
        control.shutdown().await.unwrap();
        let sockets = broker.no_sockets().await;
        println!(
            "CONTACT_DISABLED_PASS socket_count=0 stale_restart_rejected=true mqtt_immediate_empty={} mqtt_late_empty={}",
            sockets.immediate_empty, sockets.late_empty
        );
        broker.close().await;
    }
}

impl CustomTransport for ContactListener {
    fn resource(&self) -> Option<(iroh::RelayUrl, iroh::endpoint::RelayResourceUse)> {
        self.shared.transport.resource()
    }
    fn bind(&self) -> io::Result<Box<dyn CustomEndpoint>> {
        {
            let mut state = self.shared.state.lock().unwrap();
            if state.bound {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "contact listener already bound",
                ));
            }
            state.bound = true;
        }
        if let Err(error) = self.shared.start() {
            self.shared.state.lock().unwrap().bound = false;
            return Err(error);
        }
        Ok(Box::new(ListenerEndpoint(self.shared.clone())))
    }
}
#[derive(Debug)]
struct ListenerEndpoint(Arc<Shared>);
impl Drop for ListenerEndpoint {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap();
        self.0.retire_locked(&mut state, true, None);
    }
}
impl CustomEndpoint for ListenerEndpoint {
    fn watch_local_addrs(&self) -> Direct<Vec<CustomAddr>> {
        self.0.addresses.watch()
    }
    fn create_sender(&self) -> Arc<dyn CustomSender> {
        Arc::new(ListenerSender(self.0.clone()))
    }
    fn poll_recv(
        &mut self,
        cx: &mut Context,
        bufs: &mut [io::IoSliceMut<'_>],
        metas: &mut [noq_udp::RecvMeta],
        infos: &mut [RecvInfo],
    ) -> Poll<io::Result<usize>> {
        self.0.receive_waker.register(cx.waker());
        let mut state = self.0.state.lock().unwrap();
        let Some(current) = state.current.as_mut() else {
            return Poll::Pending;
        };
        match current.endpoint.poll_recv(cx, bufs, metas, infos) {
            Poll::Ready(Err(error)) => {
                self.0
                    .retire_locked(&mut state, false, Some(error.to_string()));
                // Keep the installed endpoint alive for a maintenance-owned restart.
                Poll::Pending
            }
            result => result,
        }
    }
}
#[derive(Debug)]
struct ListenerSender(Arc<Shared>);
impl CustomSender for ListenerSender {
    fn is_valid_send_addr(&self, address: &CustomAddr) -> bool {
        let state = self.0.state.lock().unwrap();
        state.status.phase == ContactListenerPhase::Ready
            && self.0.transport.diagnostics().1
            && state
                .current
                .as_ref()
                .is_some_and(|current| current.sender.is_valid_send_addr(address))
    }
    fn poll_send(
        &self,
        cx: &mut Context,
        dst: &CustomAddr,
        src: Option<&CustomAddr>,
        transmit: &Transmit<'_>,
    ) -> Poll<io::Result<()>> {
        let state = self.0.state.lock().unwrap();
        if state.status.phase != ContactListenerPhase::Ready
            || !self.0.transport.diagnostics().1
            || src.is_some_and(|address| !self.0.addresses.get().contains(address))
        {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "contact generation is not ready",
            )));
        }
        let Some(current) = &state.current else {
            return Poll::Pending;
        };
        current.sender.poll_send(cx, dst, src, transmit)
    }
}
