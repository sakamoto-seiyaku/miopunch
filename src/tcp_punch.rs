//! TCP traversal during the fork-owned admitted control stage.
use crate::{
    admission_gate::AdmissionGate, connection_coordinator::connection_diagnostic::AttemptLog,
    tcp_transport::TcpTransport,
};
use iroh::{
    TransportAddr,
    endpoint::{
        ControlExchange, ControlHandle, ControlMessageFormat, DataCandidatePath,
        WeakControlConnection,
    },
};

enum Preparation<'a> {
    Admission,
    Active(&'a ControlHandle),
    Fused {
        exchange: &'a ControlExchange,
        admission: bool,
        physical: Option<iroh::endpoint::MemberPunching>,
    },
}
use serde_json::json;
use std::{
    collections::HashMap,
    io,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone, Debug)]
pub(crate) struct TcpPunch {
    pub(crate) transport: TcpTransport,
    pub(crate) observers: Vec<iroh::RelayUrl>,
    pub(crate) resources: crate::resources::ResourceConfig,
    attempts: Arc<Mutex<HashMap<(iroh::EndpointId, Vec<u8>, bool), PeerAttempt>>>,
}

#[derive(Debug)]
struct PeerAttempt {
    preferred: bool,
    token: [u8; 16],
    connection: WeakControlConnection,
    released: bool,
}

struct AttemptClaim {
    attempts: Arc<Mutex<HashMap<(iroh::EndpointId, Vec<u8>, bool), PeerAttempt>>>,
    key: (iroh::EndpointId, Vec<u8>, bool),
    token: [u8; 16],
}

/// A validated restricted TCP candidate. Preparation owns its exact circuit
/// and deduplication claim; only the coordinator's final commit retains them.
pub(crate) struct PreparedTcpPath {
    punch: TcpPunch,
    claim: Option<AttemptClaim>,
    circuit: Option<crate::tcp_transport::TcpCircuitLease>,
    pub(crate) active: Option<ActiveTcpPath>,
    observation: crate::tcp_observation::TcpSocketObservation,
    generation: iroh::endpoint::ObservationValidity,
    route: iroh_base::CustomAddr,
    original: WeakControlConnection,
    connection_id: usize,
    physical_complete: bool,
}

pub(crate) struct ActiveTcpPath {
    original: ControlHandle,
    route: TransportAddr,
    opened: Option<DataCandidatePath>,
    received_candidate: bool,
    circuit: Option<crate::tcp_transport::TcpCircuitLease>,
    pending: Option<crate::tcp_transport::PendingTcpPath>,
}

impl ActiveTcpPath {
    pub(crate) fn retain(mut self) {
        self.received_candidate = false;
        if let Some(path) = self.opened.take() {
            path.retain();
        }
        if let Some(circuit) = self.circuit.take() {
            circuit.retain_in_transport();
        }
        if let Some(pending) = self.pending.take() {
            pending.commit();
        }
    }
}

impl Drop for ActiveTcpPath {
    fn drop(&mut self) {
        if self.received_candidate {
            for path in self
                .original
                .paths()
                .iter()
                .filter(|path| path.remote_addr() == &self.route)
            {
                let _ = path.close();
            }
        }
    }
}

impl PreparedTcpPath {
    pub(crate) fn network_validity(&self) -> iroh::endpoint::ObservationValidity { self.generation.clone() }
    pub(crate) fn route(&self) -> &iroh_base::CustomAddr {
        &self.route
    }

    pub(crate) fn check(&self, connection: &ControlHandle, gate: &AdmissionGate) -> io::Result<()> {
        if connection.stable_id() != self.connection_id
            || !self.original.is_open()
            || connection.close_reason().is_some()
            || !gate.allows_member_path_upgrade(connection)
            || !self.generation.is_current()
            || connection.is_closed()
            || if self.physical_complete {
                !self
                    .active
                    .as_ref()
                    .and_then(|active| active.circuit.as_ref())
                    .or(self.circuit.as_ref())
                    .is_some_and(|circuit| circuit.binding_is_live(self.observation.local_addr))
            } else {
                !self.observation.is_fresh()
            }
            || !self.punch.resources.allows_custom(&self.route)
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "TCP candidate scope changed before retention",
            ));
        }
        if !connection
            .paths()
            .iter()
            .any(|path| path.remote_addr() == &TransportAddr::Custom(self.route.clone()))
        {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "TCP candidate path disappeared",
            ));
        }
        Ok(())
    }

    pub(crate) fn retain_control(
        self,
        connection: &ControlHandle,
        gate: &AdmissionGate,
    ) -> io::Result<()> {
        self.check(connection, gate)?;
        if let Some(active) = self.active {
            active.retain();
        }
        let Some(claim) = self.claim else {
            return Ok(());
        };
        self.punch.release_claim(&claim, connection)?;
        let closed = connection.weak_handle();
        let drained = connection.transport_drained();
        let circuit = self.circuit;
        tokio::spawn(async move {
            closed.closed().await;
            drop(claim);
            drained.await;
            drop(circuit);
        });
        Ok(())
    }
}

impl Drop for AttemptClaim {
    fn drop(&mut self) {
        let mut attempts = self.attempts.lock().unwrap();
        if attempts
            .get(&self.key)
            .is_some_and(|attempt| attempt.token == self.token)
        {
            attempts.remove(&self.key);
        }
    }
}

impl TcpPunch {
    pub(crate) async fn retain_existing_control(
        &self,
        connection: &ControlHandle,
    ) -> io::Result<()> {
        let claim = self
            .claim(
                connection,
                self.transport
                    .bound_socket_addr()
                    .is_some_and(|addr| addr.is_ipv6()),
            )
            .await?;
        let routes: Vec<_> = connection
            .paths()
            .iter()
            .filter_map(|path| match path.remote_addr() {
                TransportAddr::Custom(address)
                    if address.id() == crate::tcp_transport::DATA_TRANSPORT_ID =>
                {
                    Some(address.clone())
                }
                _ => None,
            })
            .collect();
        if routes.is_empty() {
            return Err(io::Error::other("TCP path disappeared"));
        }
        let mut circuits = Vec::new();
        for address in routes {
            if !self.resources.allows_custom(&address) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "existing TCP route denied",
                ));
            }
            circuits.push(
                self.transport
                    .retain_existing(TcpTransport::socket_addr(&address)?)?,
            );
        }
        self.release_claim(&claim, connection)?;
        let closed = connection.weak_handle();
        let drained = connection.transport_drained();
        tokio::spawn(async move {
            closed.closed().await;
            drop(claim);
            drained.await;
            drop(circuits);
        });
        Ok(())
    }

    pub(crate) fn new(
        transport: TcpTransport,
        observers: Vec<iroh::RelayUrl>,
        resources: crate::resources::ResourceConfig,
    ) -> Self {
        Self {
            transport,
            observers,
            resources,
            attempts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn release_claim(&self, claim: &AttemptClaim, connection: &ControlHandle) -> io::Result<()> {
        let mut attempts = self.attempts.lock().unwrap();
        let attempt = attempts
            .get_mut(&claim.key)
            .filter(|attempt| attempt.token == claim.token && connection.close_reason().is_none())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "TCP peer attempt no longer owns release",
                )
            })?;
        attempt.released = true;
        Ok(())
    }

    async fn claim(&self, connection: &ControlHandle, ipv6: bool) -> io::Result<AttemptClaim> {
        let key = (connection.remote_id(), connection.alpn().to_vec(), ipv6);
        let preferred = connection.side().is_client() == (connection.id() < connection.remote_id());
        let mut token = [0; 16];
        connection
            .export_keying_material(&mut token, b"miopunch-tcp-attempt-choice-v1", b"")
            .map_err(|error| io::Error::other(format!("{error:?}")))?;
        {
            let mut attempts = self.attempts.lock().unwrap();
            if let Some(old) = attempts.get(&key)
                && old.connection.is_open()
            {
                if old.released
                    || (old.preferred && !preferred)
                    || (old.preferred == preferred && old.token < token)
                {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "duplicate TCP peer attempt",
                    ));
                }
                old.connection
                    .close(0u32.into(), b"superseded duplicate TCP peer attempt");
            }
            attempts.insert(
                key.clone(),
                PeerAttempt {
                    preferred,
                    token,
                    connection: connection.weak_handle(),
                    released: false,
                },
            );
        }
        let claim = AttemptClaim {
            attempts: self.attempts.clone(),
            key,
            token,
        };
        // Both endpoints choose the same TLS-exporter token when simultaneous
        // same-direction duplicates tie. No unauthenticated route chooses a peer.
        tokio::time::sleep(Duration::from_millis(200)).await;
        if connection.close_reason().is_some()
            || !self
                .attempts
                .lock()
                .unwrap()
                .get(&claim.key)
                .is_some_and(|attempt| attempt.token == token)
        {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "TCP peer attempt superseded",
            ));
        }
        Ok(claim)
    }

    /// Existing observation/coordination algorithm, without releasing the
    /// original connection's attempt claim before common path commitment.
    pub(crate) async fn prepare_control(
        &self,
        connection: &ControlHandle,
        gate: &AdmissionGate,
        log: AttemptLog,
    ) -> io::Result<PreparedTcpPath> {
        self.prepare_with_exchange(connection, gate, log, Preparation::Admission)
            .await
    }

    pub(crate) async fn prepare_active(
        &self,
        original: &ControlHandle,
        control: &ControlHandle,
        gate: &AdmissionGate,
        log: AttemptLog,
    ) -> io::Result<PreparedTcpPath> {
        self.prepare_with_exchange(original, gate, log, Preparation::Active(control))
            .await
    }

    pub(crate) async fn prepare_fused(
        &self,
        original: &ControlHandle,
        exchange: &ControlExchange,
        gate: &AdmissionGate,
        log: AttemptLog,
        admission: bool,
        physical: Option<iroh::endpoint::MemberPunching>,
    ) -> io::Result<PreparedTcpPath> {
        self.prepare_with_exchange(
            original,
            gate,
            log,
            Preparation::Fused {
                exchange,
                admission,
                physical,
            },
        )
        .await
    }

    async fn prepare_with_exchange(
        &self,
        connection: &ControlHandle,
        gate: &AdmissionGate,
        log: AttemptLog,
        preparation: Preparation<'_>,
    ) -> io::Result<PreparedTcpPath> {
        let exchange_control = match preparation {
            Preparation::Active(control) => control,
            _ => connection,
        };
        let initial = matches!(
            preparation,
            Preparation::Admission
                | Preparation::Fused {
                    admission: true,
                    ..
                }
        );
        let check = || {
            if connection.close_reason().is_some()
                || exchange_control.close_reason().is_some()
                || !gate.allows_member_path_upgrade(connection)
            {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "TCP member scope invalidated",
                ))
            } else {
                Ok(())
            }
        };
        check()?;
        let mut physical = match &preparation {
            Preparation::Fused { physical, .. } => physical.clone(),
            _ => None,
        };
        let binding = physical
            .as_ref()
            .and_then(|permit| permit.binding())
            .or_else(|| self.transport.bound_socket_addr());
        let ipv6 = binding.is_some_and(|addr| addr.is_ipv6());
        let family = if ipv6 {
            iroh::endpoint::ObservationFamily::Ipv6
        } else {
            iroh::endpoint::ObservationFamily::Ipv4
        };
        // Active work already holds the exact M's reservation. Initial
        // contact deduplication must never replace or pin another living M.
        let claim = if initial {
            Some(self.claim(connection, ipv6).await?)
        } else {
            None
        };
        check()?;
        let physical_required = physical.is_some();
        let mut observation = None;
        // Both peers consume the protected TCP phase even when only one can
        // punch. Unsupported platforms neither query observers nor prime a
        // circuit, and report unavailable before the shared UDP fallback.
        if self.transport.supports_punching() {
            for (index, observer) in self.observers.iter().enumerate() {
                check()?;
                let phase = log.phase("observation", json!({"method":"tcp","observerIndex":index}));
                let sample = async {
                    if physical_required {
                        let destination = connection
                            .resolve_observation_addr(observer, family, 3478)
                            .await?;
                        let listener = self.transport.bound_socket_addr_for(destination)?;
                        if let Some(previous) = physical.take() {
                            previous.finish();
                        }
                        let permit = connection.try_member_punching(listener, destination, true)?;
                        let source = permit.binding().expect("TCP physical source");
                        physical = Some(permit);
                        crate::tcp_observation::observe_from(
                            connection,
                            &self.transport,
                            observer,
                            source,
                        )
                        .await
                    } else {
                        crate::tcp_observation::observe_for_family(
                            connection,
                            &self.transport,
                            observer,
                            family,
                        )
                        .await
                    }
                }
                .await;
                match sample {
                    Ok(result) if result.is_fresh() && family.matches(result.mapped) => {
                        phase.finish("succeeded");
                        observation = Some(result);
                        break;
                    }
                    _ => {
                        phase.finish("failed");
                    }
                }
            }
        }
        if let Some(result) = &observation {
            println!(
                "TCP_OBSERVED stable_id={} listener={} source={} destination={} candidate={}",
                connection.stable_id(),
                result.listener,
                result.local_addr,
                result.destination,
                result.mapped
            );
        } else if let Some(permit) = physical.take() {
            permit.finish();
        }
        let generation = observation.as_ref().map(|result| result.generation.clone())
            .unwrap_or_else(|| connection.udp_network_generation());
        check()?;
        let owned_exchange;
        let exchange = match preparation {
            Preparation::Fused { exchange, .. } => exchange,
            _ => {
                owned_exchange = if connection.side().is_client() {
                    exchange_control.open_control_exchange().await?
                } else {
                    exchange_control.accept_control_exchange().await?
                };
                &owned_exchange
            }
        };
        // Keep both peers in the TCP phase even when only one observer fails.
        // The retained observation socket lives through the path decision.
        let preface: &[u8; 8] = if ipv6 { b"MIOTCP02" } else { b"MIOTCP01" };
        let length = if ipv6 { 27 } else { 15 };
        let mut candidate = preface.to_vec();
        if let Some(result) = &observation
            && result.is_fresh()
        {
            candidate.push(1);
            match result.mapped.ip() {
                std::net::IpAddr::V4(ip) => candidate.extend(ip.octets()),
                std::net::IpAddr::V6(ip) => candidate.extend(ip.octets()),
            }
            candidate.extend(result.mapped.port().to_be_bytes());
        } else {
            candidate.resize(length, 0);
        }
        let format = ControlMessageFormat::Fixed { length };
        let candidates = async {
            exchange.send_message(format, &candidate).await?;
            exchange.recv_message(format).await
        };
        let received = tokio::select! { biased;
            _ = async {
                while generation.is_current() {
                    connection.wait_network_generation_change(&generation).await;
                }
            }, if observation.is_some() => return Err(io::ErrorKind::Interrupted.into()),
            result = candidates => result?,
        };
        if received[..8] != *preface || received[8] > 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid protected TCP candidate",
            ));
        }
        if candidate[8] == 0 || received[8] == 0 {
            exchange.finish_send().await?;
            return Err(io::Error::other("TCP observation unavailable at a peer"));
        }
        let ip = if ipv6 {
            std::net::IpAddr::V6(std::net::Ipv6Addr::from(
                <[u8; 16]>::try_from(&received[9..25]).unwrap(),
            ))
        } else {
            std::net::IpAddr::V4(std::net::Ipv4Addr::from(
                <[u8; 4]>::try_from(&received[9..13]).unwrap(),
            ))
        };
        let remote = SocketAddr::new(
            ip,
            u16::from_be_bytes(received[length - 2..].try_into().unwrap()),
        );
        let route = TcpTransport::address(remote);
        let allowed = self.resources.allows_custom(&route)
            && !remote.ip().is_unspecified()
            && !remote.ip().is_multicast()
            && remote.port() != 0;
        let mut active = if !initial || matches!(preparation, Preparation::Fused { .. }) {
            let remote_addr = TransportAddr::Custom(route.clone());
            let local_opens = connection.side().is_client();
            let owned = allowed
                && !connection.paths().iter().any(|path| {
                    path.remote_addr() == &remote_addr
                        && self
                            .transport
                            .data_path_qualified(&route, connection.stable_id())
                });
            let pending = if owned {
                Some(self.transport.qualify_after_commit(connection, remote)?)
            } else {
                None
            };
            Some(ActiveTcpPath {
                original: connection.clone(),
                route: remote_addr,
                opened: None,
                received_candidate: owned && !local_opens,
                circuit: None,
                pending,
            })
        } else {
            None
        };
        exchange
            .send_message(
                iroh::endpoint::ControlMessageFormat::Fixed { length: 1 },
                &[u8::from(allowed)],
            )
            .await?;
        let peer_allowed = exchange
            .recv_message(iroh::endpoint::ControlMessageFormat::Fixed { length: 1 })
            .await?;
        if peer_allowed[0] > 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid TCP permission status",
            ));
        }
        if !allowed || peer_allowed[0] == 0 {
            exchange.finish_send().await?;
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "TCP candidate denied",
            ));
        }
        check()?;
        let mut circuit = None;
        let mut physical_finished = false;
        let validation = log
            .phase(
                "punch",
                json!({"routeKind":"tcp","validationBudgetMs":6000}),
            )
            .run(async {
                let primed = match &observation {
                    Some(observation) => {
                        self.transport.prime_from(remote, observation.local_addr)?
                    }
                    None => self.transport.prime(remote)?,
                };
                if let Some(active) = &mut active {
                    active.circuit = Some(primed);
                } else {
                    circuit = Some(primed);
                }
                println!(
                    "TCP_PUNCH_START stable_id={} peer={} remote={remote}",
                    connection.stable_id(),
                    connection.remote_id()
                );
                let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
                log.phase(
                    "path-open",
                    json!({"routeKind":"tcp","dialing":connection.side().is_client()}),
                )
                .run(async {
                    loop {
                        check()?;
                        if !generation.is_current()
                            || connection.is_closed()
                            || if physical.is_some()
                                && active
                                    .as_ref()
                                    .and_then(|active| active.circuit.as_ref())
                                    .or(circuit.as_ref())
                                    .is_some_and(|circuit| circuit.handshake_ready())
                            {
                                !observation.as_ref().is_some_and(|result| {
                                    active
                                        .as_ref()
                                        .and_then(|active| active.circuit.as_ref())
                                        .or(circuit.as_ref())
                                        .is_some_and(|circuit| {
                                            circuit.binding_is_live(result.local_addr)
                                        })
                                })
                            } else {
                                !observation.as_ref().is_some_and(|result| result.is_fresh())
                            }
                        {
                            return Err(io::Error::other(
                                "TCP observation invalidated during punching",
                            ));
                        }
                        if connection.side().is_client() {
                            let opened = if let Some(active) = &mut active {
                                if active.opened.is_none() {
                                    connection
                                        .open_data_transport_candidate(active.route.clone())
                                        .await
                                        .map(|path| active.opened = path)
                                } else {
                                    Ok(())
                                }
                            } else {
                                connection.open_control_path(route.clone()).await
                            };
                            match opened {
                                Ok(()) => {}
                                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                                Err(error) => return Err(error),
                            }
                        }
                        let handshake_ready = active
                            .as_ref()
                            .and_then(|active| active.circuit.as_ref())
                            .or(circuit.as_ref())
                            .is_some_and(|circuit| circuit.handshake_ready());
                        if handshake_ready && !physical_finished {
                            if let Some(physical) = &physical {
                                physical.finish();
                            }
                            physical_finished = true;
                            println!(
                                "TCP_PHYSICAL_RELEASED stable_id={} source={} remote={remote}",
                                connection.stable_id(),
                                observation.as_ref().expect("TCP observation").local_addr
                            );
                        }
                        if handshake_ready && connection.paths().iter().any(
                    |path| matches!(path.remote_addr(),TransportAddr::Custom(addr) if addr==&route),
                ) {
                    println!(
                        "TCP_PUNCH_VALIDATED stable_id={} peer={}",
                        connection.stable_id(),
                        connection.remote_id()
                    );
                    return Ok(());
                }
                        if tokio::time::Instant::now() >= deadline {
                            return Err(io::Error::new(
                                io::ErrorKind::TimedOut,
                                "TCP punching path validation timed out",
                            ));
                        }
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                })
                .await
            })
            .await;
        // A unilateral validation cannot release business while its peer is
        // entering fallback. Both endpoints commit the same phase result.
        let format = if matches!(preparation, Preparation::Fused { .. }) {
            ControlMessageFormat::Fixed { length: 1 }
        } else {
            ControlMessageFormat::EndTerminated { maximum: 1 }
        };
        exchange
            .send_message(format, &[u8::from(validation.is_ok())])
            .await?;
        let peer_validated = exchange.recv_message(format).await?;
        if peer_validated.len() != 1 || peer_validated[0] > 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid TCP validation status",
            ));
        }
        if let Err(error) = validation {
            if initial {
                let _ = connection.retire_control_path(route).await;
            }
            return Err(error);
        }
        if peer_validated[0] == 0 {
            if initial {
                let _ = connection.retire_control_path(route).await;
            }
            return Err(io::Error::other("TCP path unavailable at a peer"));
        }
        if !generation.is_current()
            || connection.is_closed()
            || if physical.is_some() {
                !active
                    .as_ref()
                    .and_then(|active| active.circuit.as_ref())
                    .or(circuit.as_ref())
                    .is_some_and(|circuit| {
                        observation
                            .as_ref()
                            .is_some_and(|result| circuit.binding_is_live(result.local_addr))
                    })
            } else {
                !observation.as_ref().is_some_and(|result| result.is_fresh())
            }
        {
            if initial {
                let _ = connection.retire_control_path(route).await;
            }
            return Err(io::Error::other(
                "TCP observation invalidated before release",
            ));
        }
        Ok(PreparedTcpPath {
            punch: self.clone(),
            claim,
            circuit,
            active,
            observation: observation.expect("validated retained TCP observation"),
            generation,
            route,
            original: connection.weak_handle(),
            connection_id: connection.stable_id(),
            physical_complete: physical.is_some(),
        })
    }
}
