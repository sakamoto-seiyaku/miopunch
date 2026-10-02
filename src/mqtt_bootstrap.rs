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

pub(crate) struct MemberPathBootstrap {
    gate: AdmissionGate,
    udp: Arc<NatConnector>,
}

impl fmt::Debug for MemberPathBootstrap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemberPathBootstrap")
            .finish_non_exhaustive()
    }
}

impl MemberPathBootstrap {
    pub(crate) fn new(gate: AdmissionGate, udp: Arc<NatConnector>) -> Self {
        Self { gate, udp }
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
