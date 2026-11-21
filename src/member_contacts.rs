//! Durable member route hints. A hint never establishes membership or permission.
use crate::{admission_gate::AdmissionGate, resources::ResourceConfig};
use iroh::{EndpointAddr, EndpointId, TransportAddr, endpoint::Connection};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    path::PathBuf,
    sync::{Arc, Mutex},
};

// A preference window, not an authorization lifetime. Older hints remain cold
// fallback candidates and have to pass the ordinary handshake again.
const RECENT_SECONDS: u64 = 300;
const MAX_PEERS: usize = 64;
const MAX_ROUTES: usize = 8;
#[derive(Clone, Serialize, Deserialize)]
struct SavedContact {
    id: EndpointId,
    routes: Vec<SavedRoute>,
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
struct SavedRoute {
    address: TransportAddr,
    observed_at: u64,
    verified: bool,
}
impl SavedRoute {
    fn is_recent(&self, now: u64) -> bool {
        now >= self.observed_at && now - self.observed_at <= RECENT_SECONDS
    }
}
pub(crate) struct MemberContacts {
    file: PathBuf,
    gate: AdmissionGate,
    config: ResourceConfig,
    saved: Mutex<BTreeMap<EndpointId, SavedContact>>,
}
impl MemberContacts {
    // The explicit remember API is synchronous. Async connection/maintenance
    // callers move the entire refresh + lock + flush operation off their
    // runtime thread, so their existing deadline/cancellation can wake.
    async fn blocking<T: Send + 'static>(
        self: &Arc<Self>,
        operation: impl FnOnce(&Self) -> io::Result<T> + Send + 'static,
    ) -> io::Result<T> {
        let store = self.clone();
        tokio::task::spawn_blocking(move || operation(&store))
            .await
            .map_err(io::Error::other)?
    }
    pub(crate) async fn merge_async(
        self: &Arc<Self>,
        target: &mut EndpointAddr,
    ) -> io::Result<Vec<TransportAddr>> {
        let mut merged = target.clone();
        let (merged, preferred) = self
            .blocking(move |store| {
                let preferred = store.merge(&mut merged)?;
                Ok((merged, preferred))
            })
            .await?;
        *target = merged;
        Ok(preferred)
    }
    pub(crate) async fn observe_async(self: &Arc<Self>, connection: &Connection) -> io::Result<()> {
        let connection = connection.clone();
        self.blocking(move |store| store.observe(&connection)).await
    }
    pub(crate) async fn observe_member(
        self: &Arc<Self>,
        connection: &iroh::endpoint::MemberConnection,
    ) -> io::Result<()> {
        let connection = connection.clone();
        self.blocking(move |store| {
            if store.gate.member_contact_allowed(connection.remote_id()) {
                store.observe_paths(connection.remote_id(), connection.paths())?;
            }
            Ok(())
        })
        .await
    }
    pub(crate) async fn refresh_async(self: &Arc<Self>) -> io::Result<()> {
        self.blocking(Self::refresh).await
    }
    pub(crate) fn open(
        directory: PathBuf,
        gate: AdmissionGate,
        config: ResourceConfig,
    ) -> io::Result<Self> {
        let file = directory.join("member.contacts.json");
        let saved: Vec<SavedContact> = match std::fs::read(&file) {
            Ok(bytes) if bytes.len() <= 64 * 1024 => {
                serde_json::from_slice(&bytes).map_err(io::Error::other)?
            }
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "member contacts too large",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error),
        };
        if saved.len() > MAX_PEERS || saved.iter().any(|entry| entry.routes.len() > MAX_ROUTES) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "member contacts exceed node bounds",
            ));
        }
        Ok(Self {
            file,
            gate,
            config,
            saved: Mutex::new(saved.into_iter().map(|s| (s.id, s)).collect()),
        })
    }
    fn permitted(&self, route: &TransportAddr) -> bool {
        match route {
            TransportAddr::Ip(_) => !self.config.udp_direct_disabled,
            TransportAddr::Relay(url) => self
                .config
                .allows(url.as_str(), crate::resources::ResourceUse::DataRelay),
            TransportAddr::Custom(address)
                if address.id() == crate::tcp_transport::DATA_TRANSPORT_ID =>
            {
                self.config.allows_custom(address)
                    && crate::tcp_transport::TcpTransport::socket_addr(address)
                        .is_ok_and(|address| !address.ip().is_unspecified() && address.port() != 0)
            }
            // Signaling and session-scoped custom routes are not redial hints.
            _ => false,
        }
    }
    pub(crate) fn observe(&self, connection: &Connection) -> io::Result<()> {
        if !self.gate.allows_member_path_upgrade(connection) {
            return Ok(());
        }
        self.observe_paths(connection.remote_id(), connection.paths())
    }
    fn observe_paths(&self, id: EndpointId, paths: iroh::endpoint::PathList<'_>) -> io::Result<()> {
        let now = crate::service_policy::now();
        let routes = paths
            .iter()
            .filter_map(|path| {
                let address = path.remote_addr();
                if !self.permitted(address) {
                    return None;
                }
                let ack = path.stats().acknowledgment?;
                Some(SavedRoute {
                    address: address.clone(),
                    observed_at: now.saturating_sub(ack.received_at.elapsed().as_secs()),
                    verified: true,
                })
            })
            .take(MAX_ROUTES)
            .collect();
        self.record_routes(id, routes)
    }
    pub(crate) fn advertised(
        &self,
        connection: &Connection,
        address: EndpointAddr,
    ) -> io::Result<()> {
        if address.id != connection.remote_id() || !self.gate.allows_member_path_upgrade(connection)
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "contact requires its current authenticated member connection",
            ));
        }
        if address.addrs.len() > MAX_ROUTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "too many member contact routes",
            ));
        }
        self.observe(connection)?;
        self.record(
            address.id,
            address
                .addrs
                .into_iter()
                .filter(|r| self.permitted(r))
                .collect(),
            false,
        )
    }
    fn record(&self, id: EndpointId, routes: Vec<TransportAddr>, verified: bool) -> io::Result<()> {
        let now = crate::service_policy::now();
        self.record_routes(
            id,
            routes
                .into_iter()
                .map(|address| SavedRoute {
                    address,
                    observed_at: now,
                    verified,
                })
                .collect(),
        )
    }
    fn record_routes(&self, id: EndpointId, routes: Vec<SavedRoute>) -> io::Result<()> {
        if routes.is_empty() {
            return Ok(());
        }
        let now = crate::service_policy::now();
        let mut saved = self.saved.lock().unwrap();
        let mut entry = saved.get(&id).cloned().unwrap_or(SavedContact {
            id,
            routes: Vec::new(),
        });
        let before = entry.routes.clone();
        for route in routes {
            if let Some(old) = entry
                .routes
                .iter_mut()
                .find(|old| old.address == route.address)
            {
                // Announcements never refresh evidence from an authenticated path.
                if (route.verified || !old.verified) && route.observed_at > old.observed_at {
                    *old = route;
                } else if route.verified && !old.verified {
                    *old = route;
                }
            } else {
                entry.routes.push(route);
            }
        }
        entry.routes.sort_by_key(|r| {
            (
                !r.is_recent(now),
                std::cmp::Reverse(r.verified),
                std::cmp::Reverse(r.observed_at),
            )
        });
        entry.routes.truncate(MAX_ROUTES);
        if entry.routes == before {
            return Ok(());
        }
        let mut next = saved.clone();
        if next.len() >= MAX_PEERS && !next.contains_key(&id) {
            let oldest = *next
                .iter()
                .min_by_key(|(_, s)| s.routes.iter().map(|r| r.observed_at).max().unwrap_or(0))
                .unwrap()
                .0;
            next.remove(&oldest);
        }
        next.insert(id, entry);
        let bytes =
            serde_json::to_vec(&next.values().collect::<Vec<_>>()).map_err(io::Error::other)?;
        crate::native_files::write_private_atomic(&self.file, &bytes)?;
        *saved = next;
        Ok(())
    }
    pub(crate) fn refresh(&self) -> io::Result<()> {
        let mut seen = std::collections::BTreeSet::new();
        for connection in self
            .gate
            .live_member_connections()
            .into_iter()
            .chain(self.gate.maintenance().live())
        {
            if seen.insert(connection.stable_id()) {
                self.observe(&connection)?;
            }
        }
        Ok(())
    }
    /// Extend the actual caller target; return recently verified routes for
    /// preference. Eligibility is rechecked against today's signed membership.
    pub(crate) fn merge(&self, target: &mut EndpointAddr) -> io::Result<Vec<TransportAddr>> {
        self.refresh()?;
        if !self.gate.member_contact_allowed(target.id) {
            return Ok(Vec::new());
        }
        let saved = self.saved.lock().unwrap();
        let Some(entry) = saved.get(&target.id) else {
            return Ok(Vec::new());
        };
        let now = crate::service_policy::now();
        let mut preferred = Vec::new();
        for route in &entry.routes {
            if !self.permitted(&route.address) {
                continue;
            }
            target.addrs.insert(route.address.clone());
            if route.is_recent(now)
                && (route.verified || matches!(route.address, TransportAddr::Relay(_)))
            {
                preferred.push(route);
            }
        }
        preferred.sort_by_key(|route| {
            (
                std::cmp::Reverse(route.verified),
                std::cmp::Reverse(route.observed_at),
            )
        });
        Ok(preferred
            .into_iter()
            .map(|route| route.address.clone())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        connection_coordinator::{AttemptContext, ConnectError, SessionPurpose},
        daily_management::ManagementAction,
        native_network::NativeNetwork,
    };
    use std::{
        net::Ipv4Addr,
        time::{Duration, Instant},
    };

    fn hold_writer(store: Arc<MemberContacts>) -> std::thread::JoinHandle<()> {
        let (entered, waiting) = std::sync::mpsc::sync_channel(0);
        let writer = std::thread::spawn(move || {
            // A flush keeps this same serialization lock until publication.
            let _writing = store.saved.lock().unwrap();
            entered.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(400));
        });
        waiting.recv().unwrap();
        writer
    }

    #[tokio::test]
    async fn cached_member_lookup_preserves_public_deadline_and_cancellation() {
        let directory = tempfile::tempdir().unwrap();
        let mut node = NativeNetwork::create(directory.path().join("node"))
            .await
            .unwrap();
        let peer = p2panda_core::SigningKey::generate().verifying_key();
        let mut management = node.begin_management_session();
        node.manage(&mut management, ManagementAction::AddMember(peer))
            .await
            .unwrap();
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(vec![]);
        config.tcp_direct_disabled = true;
        node.save_resource_configuration(&config).unwrap();
        let raw_alpn = b"miopunch/contact-clock-fixture/1";
        node.admission_gate()
            .register_protocol(raw_alpn, crate::admission_gate::ProtocolScope::Member);
        let resources = node
            .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap()
            .with_contact_timeout(Duration::from_millis(60))
            .unwrap();
        let target = EndpointAddr::new(iroh::EndpointId::from_bytes(peer.as_bytes()).unwrap())
            .with_addrs([TransportAddr::Ip("127.0.0.1:9".parse().unwrap())]);
        let writer = hold_writer(resources.member_contacts.as_ref().unwrap().clone());
        let start = Instant::now();
        assert!(resources.connect(target.clone(), raw_alpn).await.is_err());
        let elapsed = start.elapsed();
        writer.join().unwrap();
        assert!(
            elapsed < Duration::from_millis(250),
            "raw public contact waited for the filesystem writer: {elapsed:?}"
        );

        let writer = hold_writer(resources.member_contacts.as_ref().unwrap().clone());
        let cancellation = tokio_util::sync::CancellationToken::new();
        let cancel = cancellation.clone();
        let timer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(60)).await;
            cancel.cancel();
        });
        let start = Instant::now();
        let result = resources
            .coordinator()
            .connect(
                target,
                SessionPurpose::MemberBusiness,
                AttemptContext::new(
                    n0_future::time::Instant::now() + Duration::from_secs(1),
                    cancellation,
                ),
            )
            .await;
        let elapsed = start.elapsed();
        timer.await.unwrap();
        writer.join().unwrap();
        assert!(matches!(result, Err(ConnectError::Cancelled)), "{result:?}");
        assert!(
            elapsed < Duration::from_millis(250),
            "SDK cancellation waited for the filesystem writer: {elapsed:?}"
        );
        println!(
            "OBSERVED cache-writer held=400ms contact-deadline=60ms cancellation_elapsed_ms={}",
            elapsed.as_millis()
        );
        resources.endpoint().close().await;
        node.close().await;
    }
}
