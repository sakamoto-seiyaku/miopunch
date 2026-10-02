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
    send.write_all(&[1, u8::from(local.tcp), u8::from(local.udp)])
        .await?;
    send.finish()?;
    let peer = recv.read_to_end(3).await.map_err(io::Error::other)?;
    if peer.len() != 3 || peer[0] != 1 || peer[1] > 1 || peer[2] > 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid protected data path capabilities",
        ));
    }
    Ok(PathCapabilities {
        tcp: local.tcp && peer[1] == 1,
        udp: local.udp && peer[2] == 1,
    })
}

pub(crate) struct MemberPathBootstrap {
    gate: AdmissionGate,
    udp: Arc<NatConnector>,
    tcp: Option<crate::tcp_punch::TcpPunch>,
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
    ) -> Self {
        Self { gate, udp, tcp }
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
            TransportAddr::Custom(address) if address.id() == TRANSPORT_ID)
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
        let control = ControlSession::from_connection(endpoint.clone(), connection.clone());
        let mut cancellation = control.cancellation_guard();
        println!(
            "MQTT_BOOTSTRAP_START stable_id={} peer={}",
            connection.stable_id(),
            connection.remote_id()
        );
        let operation = async {
            let capabilities = exchange_capabilities(
                connection,
                PathCapabilities {
                    tcp: self.tcp.is_some(),
                    udp: self.udp.udp_enabled(),
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
                "MQTT_BOOTSTRAP_DATA stable_id={} peer={} paths={:?}",
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
                    "MQTT data path bootstrap timed out",
                )
            })
            .and_then(|result| result);
        if let Err(error) = result {
            eprintln!(
                "MQTT_BOOTSTRAP_FAILED stable_id={} error={error}",
                connection.stable_id()
            );
            return Err(error);
        }
        cancellation.disarm();
        Ok(())
    }
}
