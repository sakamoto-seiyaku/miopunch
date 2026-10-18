//! TCP traversal during the fork-owned admitted control stage.
use crate::{admission_gate::AdmissionGate, tcp_transport::TcpTransport};
use iroh::{
    TransportAddr,
    endpoint::{ControlHandle, WeakControlConnection},
};
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
    attempts: Arc<Mutex<HashMap<(iroh::EndpointId, Vec<u8>), PeerAttempt>>>,
}

#[derive(Debug)]
struct PeerAttempt {
    preferred: bool,
    token: [u8; 16],
    connection: WeakControlConnection,
    released: bool,
}

struct AttemptClaim {
    attempts: Arc<Mutex<HashMap<(iroh::EndpointId, Vec<u8>), PeerAttempt>>>,
    key: (iroh::EndpointId, Vec<u8>),
    token: [u8; 16],
}

/// A validated restricted TCP candidate. Preparation owns its exact circuit
/// and deduplication claim; only the coordinator's final commit retains them.
pub(crate) struct PreparedTcpPath {
    punch: TcpPunch,
    claim: AttemptClaim,
    circuit: Option<crate::tcp_transport::TcpCircuitLease>,
    observation: crate::tcp_observation::TcpSocketObservation,
    generation: iroh::endpoint::ObservationValidity,
    route: iroh_base::CustomAddr,
    original: WeakControlConnection,
    connection_id: usize,
}

impl PreparedTcpPath {
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
            || !self.observation.is_fresh()
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
        self.punch.release_claim(&self.claim, connection)?;
        let closed = connection.weak_handle();
        let circuit = self.circuit;
        let claim = self.claim;
        tokio::spawn(async move {
            closed.closed().await;
            drop(circuit);
            drop(claim);
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
        let claim = self.claim(connection).await?;
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
        tokio::spawn(async move {
            closed.closed().await;
            drop(circuits);
            drop(claim);
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

    async fn claim(&self, connection: &ControlHandle) -> io::Result<AttemptClaim> {
        let key = (connection.remote_id(), connection.alpn().to_vec());
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
    ) -> io::Result<PreparedTcpPath> {
        let check = || {
            if connection.close_reason().is_some() || !gate.allows_member_path_upgrade(connection) {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "TCP member scope invalidated",
                ))
            } else {
                Ok(())
            }
        };
        check()?;
        let claim = self.claim(connection).await?;
        check()?;
        let generation = connection.udp_network_generation();
        let mut observation = None;
        // Both peers consume the protected TCP phase even when only one can
        // punch. Unsupported platforms neither query observers nor prime a
        // circuit, and report unavailable before the shared UDP fallback.
        if self.transport.supports_punching() {
            for observer in &self.observers {
                check()?;
                match crate::tcp_observation::observe(connection, &self.transport, observer).await {
                    Ok(result) if result.is_fresh() && result.mapped.is_ipv4() => {
                        observation = Some(result);
                        break;
                    }
                    result => {
                        eprintln!("TCP_OBSERVATION_FAILED observer={observer} result={result:?}")
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
        }
        check()?;
        let exchange = if connection.side().is_client() {
            connection.open_control_exchange().await?
        } else {
            connection.accept_control_exchange().await?
        };
        // Keep both peers in the TCP phase even when only one observer fails.
        // The retained observation socket lives through the path decision.
        let mut candidate = b"MIOTCP01".to_vec();
        if let Some(result) = &observation
            && result.is_fresh()
            && let SocketAddr::V4(local) = result.mapped
        {
            candidate.push(1);
            candidate.extend(local.ip().octets());
            candidate.extend(local.port().to_be_bytes());
        } else {
            candidate.extend([0; 7]);
        }
        exchange
            .send_message(
                iroh::endpoint::ControlMessageFormat::Fixed { length: 15 },
                &candidate,
            )
            .await?;
        let received = exchange
            .recv_message(iroh::endpoint::ControlMessageFormat::Fixed { length: 15 })
            .await?;
        if received[..8] != *b"MIOTCP01" || received[8] > 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid protected TCP candidate",
            ));
        }
        if candidate[8] == 0 || received[8] == 0 {
            exchange.finish_send().await?;
            return Err(io::Error::other("TCP observation unavailable at a peer"));
        }
        let remote = SocketAddr::from((
            std::net::Ipv4Addr::from(<[u8; 4]>::try_from(&received[9..13]).unwrap()),
            u16::from_be_bytes(received[13..].try_into().unwrap()),
        ));
        let route = TcpTransport::address(remote);
        let allowed = self.resources.allows_custom(&route)
            && !remote.ip().is_unspecified()
            && !remote.ip().is_multicast()
            && remote.port() != 0;
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
        let validation = async {
            circuit = Some(self.transport.prime(remote)?);
            println!(
                "TCP_PUNCH_START stable_id={} peer={} remote={remote}",
                connection.stable_id(),
                connection.remote_id()
            );
            let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
            loop {
                check()?;
                if !generation.is_current()
                    || connection.is_closed()
                    || !observation.as_ref().is_some_and(|result| result.is_fresh())
                {
                    return Err(io::Error::other(
                        "TCP observation invalidated during punching",
                    ));
                }
                if connection.side().is_client() {
                    match connection.open_control_path(route.clone()).await {
                        Ok(()) => {}
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                        Err(error) => return Err(error),
                    }
                }
                if connection.paths().iter().any(
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
        }
        .await;
        // A unilateral validation cannot release business while its peer is
        // entering fallback. Both endpoints commit the same phase result.
        exchange
            .send_message(
                iroh::endpoint::ControlMessageFormat::EndTerminated { maximum: 1 },
                &[u8::from(validation.is_ok())],
            )
            .await?;
        let peer_validated = exchange
            .recv_message(iroh::endpoint::ControlMessageFormat::EndTerminated { maximum: 1 })
            .await?;
        if peer_validated.len() != 1 || peer_validated[0] > 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid TCP validation status",
            ));
        }
        if let Err(error) = validation {
            let _ = connection.retire_control_path(route).await;
            return Err(error);
        }
        if peer_validated[0] == 0 {
            let _ = connection.retire_control_path(route).await;
            return Err(io::Error::other("TCP path unavailable at a peer"));
        }
        if !generation.is_current()
            || connection.is_closed()
            || !observation.as_ref().is_some_and(|result| result.is_fresh())
        {
            let _ = connection.retire_control_path(route).await;
            return Err(io::Error::other(
                "TCP observation invalidated before release",
            ));
        }
        Ok(PreparedTcpPath {
            punch: self.clone(),
            claim,
            circuit,
            observation: observation.expect("validated retained TCP observation"),
            generation,
            route,
            original: connection.weak_handle(),
            connection_id: connection.stable_id(),
        })
    }
}
