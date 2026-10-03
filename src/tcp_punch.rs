//! TCP traversal during the fork-owned admitted control stage.
use crate::{admission_gate::AdmissionGate, tcp_transport::TcpTransport};
use iroh::{Endpoint, TransportAddr, endpoint::Connection};
use std::{
    collections::HashMap,
    io,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Debug)]
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
    connection: iroh::endpoint::WeakConnectionHandle,
    released: bool,
}

struct AttemptClaim {
    attempts: Arc<Mutex<HashMap<(iroh::EndpointId, Vec<u8>), PeerAttempt>>>,
    key: (iroh::EndpointId, Vec<u8>),
    token: [u8; 16],
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
    pub(crate) async fn retain_existing(
        &self,
        endpoint: &Endpoint,
        connection: &Connection,
    ) -> io::Result<()> {
        let claim = self.claim(endpoint, connection).await?;
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
            circuits.push(self.transport.prime(TcpTransport::socket_addr(&address)?)?);
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

    fn release_claim(&self, claim: &AttemptClaim, connection: &Connection) -> io::Result<()> {
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

    async fn claim(
        &self,
        endpoint: &Endpoint,
        connection: &Connection,
    ) -> io::Result<AttemptClaim> {
        let key = (connection.remote_id(), connection.alpn().to_vec());
        let preferred = connection.side().is_client() == (endpoint.id() < connection.remote_id());
        let mut token = [0; 16];
        connection
            .export_keying_material(&mut token, b"miopunch-tcp-attempt-choice-v1", b"")
            .map_err(|error| io::Error::other(format!("{error:?}")))?;
        {
            let mut attempts = self.attempts.lock().unwrap();
            if let Some(old) = attempts.get(&key)
                && let Some(old_connection) = old
                    .connection
                    .upgrade()
                    .filter(|conn| conn.close_reason().is_none())
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
                old_connection.close(0u32.into(), b"superseded duplicate TCP peer attempt");
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

    pub(crate) async fn upgrade(
        &self,
        endpoint: &Endpoint,
        connection: &Connection,
        gate: &AdmissionGate,
    ) -> io::Result<()> {
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
        let claim = self.claim(endpoint, connection).await?;
        check()?;
        let generation = endpoint.udp_network_generation();
        let mut observation = None;
        for observer in &self.observers {
            check()?;
            match crate::tcp_observation::observe(endpoint, &self.transport, observer).await {
                Ok(result) if result.is_fresh() && result.mapped.is_ipv4() => {
                    observation = Some(result);
                    break;
                }
                result => eprintln!("TCP_OBSERVATION_FAILED observer={observer} result={result:?}"),
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
        let (mut send, mut recv) = if connection.side().is_client() {
            connection.open_bi().await?
        } else {
            connection.accept_bi().await?
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
        send.write_all(&candidate).await?;
        let mut received = [0; 15];
        recv.read_exact(&mut received)
            .await
            .map_err(io::Error::other)?;
        if received[..8] != *b"MIOTCP01" || received[8] > 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid protected TCP candidate",
            ));
        }
        if candidate[8] == 0 || received[8] == 0 {
            send.finish()?;
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
        send.write_all(&[u8::from(allowed)]).await?;
        let mut peer_allowed = [0];
        recv.read_exact(&mut peer_allowed)
            .await
            .map_err(io::Error::other)?;
        if peer_allowed[0] > 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid TCP permission status",
            ));
        }
        if !allowed || peer_allowed[0] == 0 {
            send.finish()?;
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
                    || endpoint.is_closed()
                    || !observation.as_ref().is_some_and(|result| result.is_fresh())
                {
                    return Err(io::Error::other(
                        "TCP observation invalidated during punching",
                    ));
                }
                if connection.side().is_client() {
                    match endpoint.open_control_path(connection, route.clone()).await {
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
        send.write_all(&[u8::from(validation.is_ok())]).await?;
        send.finish()?;
        let peer_validated = recv.read_to_end(1).await.map_err(io::Error::other)?;
        if peer_validated.len() != 1 || peer_validated[0] > 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid TCP validation status",
            ));
        }
        if let Err(error) = validation {
            let _ = endpoint.retire_control_path(connection, route).await;
            return Err(error);
        }
        if peer_validated[0] == 0 {
            let _ = endpoint.retire_control_path(connection, route).await;
            return Err(io::Error::other("TCP path unavailable at a peer"));
        }
        if !generation.is_current()
            || endpoint.is_closed()
            || !observation.as_ref().is_some_and(|result| result.is_fresh())
        {
            let _ = endpoint.retire_control_path(connection, route).await;
            return Err(io::Error::other(
                "TCP observation invalidated before release",
            ));
        }
        self.release_claim(&claim, connection)?;
        let closed = connection.weak_handle();
        tokio::spawn(async move {
            closed.closed().await;
            drop(circuit);
            drop(claim);
        });
        Ok(())
    }
}
