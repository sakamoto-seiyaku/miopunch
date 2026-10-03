//! The fork's admitted, restricted stage before ordinary connection release.
use std::{fmt, io, sync::Arc, time::Duration};

use iroh::{
    Endpoint, TransportAddr,
    endpoint::{Connection, EndpointHooks},
};

use crate::{
    admission_gate::AdmissionGate,
    control_session::{CommitError, ControlSession},
    mqtt_transport::TRANSPORT_ID,
    nat_connection::NatConnector,
};

#[derive(Clone, Copy, Debug)]
struct PathCapabilities {
    tcp: bool,
    udp: bool,
    relay: bool,
}

async fn exchange_capabilities(
    connection: &Connection,
    local: PathCapabilities,
) -> io::Result<PathCapabilities> {
    let (mut send, mut recv) = if connection.side().is_client() {
        connection.open_bi().await?
    } else {
        connection.accept_bi().await?
    };
    send.write_all(&[
        2,
        u8::from(local.tcp),
        u8::from(local.udp),
        u8::from(local.relay),
    ])
    .await?;
    send.finish()?;
    let peer = recv.read_to_end(4).await.map_err(io::Error::other)?;
    if peer.len() != 4 || peer[0] != 2 || peer[1..].iter().any(|capability| *capability > 1) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid protected data path capabilities",
        ));
    }
    Ok(PathCapabilities {
        tcp: local.tcp && peer[1] == 1,
        udp: local.udp && peer[2] == 1,
        relay: local.relay && peer[3] == 1,
    })
}

pub(crate) struct MemberPathBootstrap {
    gate: AdmissionGate,
    udp: Arc<NatConnector>,
    tcp: Option<crate::tcp_punch::TcpPunch>,
    resources: crate::resources::ResourceConfig,
}

impl fmt::Debug for MemberPathBootstrap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemberPathBootstrap")
            .finish_non_exhaustive()
    }
}

impl MemberPathBootstrap {
    pub(crate) fn new(
        gate: AdmissionGate,
        udp: Arc<NatConnector>,
        tcp: Option<crate::tcp_punch::TcpPunch>,
        resources: crate::resources::ResourceConfig,
    ) -> Self {
        Self {
            gate,
            udp,
            tcp,
            resources,
        }
    }

    async fn relay_upgrade(&self, endpoint: &Endpoint, connection: &Connection) -> io::Result<()> {
        let check = || {
            if !self.gate.allows_member_path_upgrade(connection)
                || connection.close_reason().is_some()
            {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "relay member scope changed",
                ))
            } else {
                Ok(())
            }
        };
        check()?;
        let local: Vec<_> = endpoint
            .addr()
            .relay_urls()
            .filter(|url| {
                self.resources
                    .allows(url.as_str(), crate::resources::ResourceUse::DataRelay)
            })
            .take(4)
            .cloned()
            .collect();
        let (mut send, mut recv) = if connection.side().is_client() {
            connection.open_bi().await?
        } else {
            connection.accept_bi().await?
        };
        let encoded = serde_json::to_vec(&local).map_err(io::Error::other)?;
        if encoded.len() > 1024 {
            return Err(io::Error::other("relay candidate message too large"));
        }
        send.write_all(&(encoded.len() as u16).to_be_bytes())
            .await?;
        send.write_all(&encoded).await?;
        let mut length = [0; 2];
        recv.read_exact(&mut length)
            .await
            .map_err(io::Error::other)?;
        let length = usize::from(u16::from_be_bytes(length));
        if length > 1024 {
            return Err(io::Error::other("peer relay candidate message too large"));
        }
        let mut encoded = vec![0; length];
        recv.read_exact(&mut encoded)
            .await
            .map_err(io::Error::other)?;
        let candidates: Vec<iroh::RelayUrl> =
            serde_json::from_slice(&encoded).map_err(io::Error::other)?;
        if candidates.len() > 4 {
            return Err(io::Error::other("too many peer relay candidates"));
        }
        let remote = candidates.into_iter().find(|url| {
            self.resources
                .allows(url.as_str(), crate::resources::ResourceUse::DataRelay)
        });
        let validation = async {
            let remote = remote.ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied, "no mutually permitted relay"))?;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
            loop {
                check()?;
                if connection.side().is_client() {
                    match endpoint.open_control_relay_path(connection, remote.clone()).await {
                        Ok(()) => {},
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {},
                        Err(error) => return Err(error),
                    }
                }
                if connection.paths().iter().any(|path| matches!(path.remote_addr(), TransportAddr::Relay(url)
                    if self.resources.allows(url.as_str(), crate::resources::ResourceUse::DataRelay))) {
                    return Ok(());
                }
                if tokio::time::Instant::now() >= deadline { return Err(io::Error::new(io::ErrorKind::TimedOut, "relay data validation timed out")); }
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
        }.await;
        if let Err(error) = &validation {
            eprintln!(
                "MESSAGE_RELAY_VALIDATION_FAILED stable_id={} error={error}",
                connection.stable_id()
            );
        }
        send.write_all(&[u8::from(validation.is_ok())]).await?;
        send.finish()?;
        let peer = recv.read_to_end(1).await.map_err(io::Error::other)?;
        validation?;
        if peer != [1] {
            return Err(io::Error::other("relay validation failed at peer"));
        }
        check()?;
        println!(
            "MESSAGE_RELAY_VALIDATED stable_id={} peer={} paths={:?}",
            connection.stable_id(),
            connection.remote_id(),
            connection.paths()
        );
        Ok(())
    }
}

impl EndpointHooks for MemberPathBootstrap {
    async fn before_data_release(
        &self,
        endpoint: &Endpoint,
        connection: &Connection,
    ) -> io::Result<()> {
        if !connection.paths().iter().any(|path| {
            matches!(path.remote_addr(),
            TransportAddr::Custom(address) if matches!(address.id(), TRANSPORT_ID | crate::member_transport::TRANSPORT_ID | crate::irc_transport::TRANSPORT_ID))
        }) {
            if let Some(tcp)=&self.tcp
                && connection.paths().iter().any(|path|matches!(path.remote_addr(),TransportAddr::Custom(address) if address.id()==crate::tcp_transport::DATA_TRANSPORT_ID)) {
                let control=ControlSession::from_connection(endpoint.clone(),connection.clone());
                let mut cancellation=control.cancellation_guard();
                tcp.retain_existing(endpoint,connection).await?;
                if !self.gate.allows_member_path_upgrade(connection) {return Err(io::Error::new(io::ErrorKind::PermissionDenied,"TCP membership changed before data release"));}
                cancellation.disarm();
            }
            return Ok(());
        }
        let carrier = if connection.paths().iter().any(|path| matches!(path.remote_addr(), TransportAddr::Custom(address) if address.id() == crate::member_transport::TRANSPORT_ID)) { "MEMBER" } else if connection.paths().iter().any(|path| matches!(path.remote_addr(), TransportAddr::Custom(address) if address.id() == crate::irc_transport::TRANSPORT_ID)) { "IRC" } else { "MQTT" };
        let control = ControlSession::from_connection(endpoint.clone(), connection.clone());
        let mut cancellation = control.cancellation_guard();
        println!(
            "{carrier}_BOOTSTRAP_START stable_id={} peer={}",
            connection.stable_id(),
            connection.remote_id()
        );
        let operation = async {
            let capabilities = exchange_capabilities(
                connection,
                PathCapabilities {
                    tcp: self.tcp.is_some(),
                    udp: self.udp.udp_enabled(),
                    relay: !self
                        .resources
                        .relay_map(crate::resources::ResourceUse::DataRelay)?
                        .is_empty(),
                },
            )
            .await?;
            if capabilities.tcp
                && let Some(tcp) = &self.tcp
            {
                match tcp.upgrade(endpoint, connection, &self.gate).await {
                    Ok(()) => return Ok(()),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        return Err(error);
                    }
                    Err(error) => eprintln!(
                        "TCP_PUNCH_FALLBACK stable_id={} error={error}",
                        connection.stable_id()
                    ),
                }
            }
            if capabilities.relay {
                match self.relay_upgrade(endpoint, connection).await {
                    Ok(()) => return Ok(()),
                    Err(error) => eprintln!(
                        "MESSAGE_RELAY_FALLBACK stable_id={} error={error}",
                        connection.stable_id()
                    ),
                }
            }
            if !capabilities.udp {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "no mutually permitted data path capability",
                ));
            }
            let mut upgrade = self
                .udp
                .upgrade(control, &self.gate)
                .await
                .map_err(|error| match error {
                    CommitError::Retryable { error, .. } | CommitError::Terminal { error } => error,
                })?;
            if let Some(attempt) = upgrade.attempt.as_mut() {
                attempt.retain_application_path(&upgrade.data, &self.gate)?;
            }
            if !self.gate.allows_member_path_upgrade(connection)
                || connection.close_reason().is_some()
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "member scope changed before data release",
                ));
            }
            println!(
                "{carrier}_BOOTSTRAP_DATA stable_id={} peer={} paths={:?}",
                connection.stable_id(),
                connection.remote_id(),
                connection.paths()
            );
            Ok(())
        };
        let result = tokio::time::timeout(Duration::from_secs(45), operation)
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "message data path bootstrap timed out",
                )
            })
            .and_then(|result| result);
        if let Err(error) = result {
            eprintln!(
                "{carrier}_BOOTSTRAP_FAILED stable_id={} error={error}",
                connection.stable_id()
            );
            return Err(error);
        }
        cancellation.disarm();
        Ok(())
    }
}
