//! Native gate/candidate adapter for the common member coordinator.
use super::{
    AttemptRole, MemberPathAdapter, PathPlan, Platform, PreparedPath, ProtectedAttempt, RELAY, RTC,
    RtcRole, RuntimeCapabilities, TCP, UDP, control_error, coordinate_member,
};
use crate::{admission_gate::AdmissionGate, control_session::ControlSession};
use iroh::{Endpoint, TransportAddr, endpoint::Connection};
use std::{future::Future, io};

#[derive(Debug, Clone)]
pub(crate) struct CoordinatorHook {
    gate: AdmissionGate,
    udp_allowed: bool,
    relay_allowed: bool,
    resources: Option<crate::resources::ResourceConfig>,
    native: Option<crate::native_path_adapter::NativePathAdapter>,
    #[cfg(feature = "rtc-probe")]
    rtc: Option<crate::rtc_transport::RtcTransport>,
}
impl CoordinatorHook {
    pub(crate) fn new(gate: AdmissionGate, udp_allowed: bool) -> Self {
        Self {
            gate,
            udp_allowed,
            relay_allowed: false,
            resources: None,
            native: None,
            #[cfg(feature = "rtc-probe")]
            rtc: None,
        }
    }
    pub(crate) fn with_resources(mut self, resources: crate::resources::ResourceConfig) -> Self {
        self.resources = Some(resources);
        self
    }
    pub(crate) fn with_native(
        mut self,
        adapter: crate::native_path_adapter::NativePathAdapter,
    ) -> Self {
        self.native = Some(adapter);
        self
    }
    pub(crate) fn with_relay(mut self, allowed: bool) -> Self {
        self.relay_allowed = allowed;
        self
    }
    #[cfg(feature = "rtc-probe")]
    pub(crate) fn with_rtc(mut self, rtc: Option<crate::rtc_transport::RtcTransport>) -> Self {
        self.rtc = rtc;
        self
    }
}
impl MemberPathAdapter for CoordinatorHook {
    fn capabilities(&self, role: AttemptRole) -> RuntimeCapabilities {
        let (rtc_supported, rtc_allowed) = {
            #[cfg(feature = "rtc-probe")]
            {
                (
                    role == AttemptRole::Responder && self.rtc.is_some(),
                    role == AttemptRole::Responder
                        && self.rtc.as_ref().is_some_and(|rtc| rtc.resource_allowed()),
                )
            }
            #[cfg(not(feature = "rtc-probe"))]
            {
                (false, false)
            }
        };
        RuntimeCapabilities {
            runtime: Platform::Native,
            supported: UDP
                | RELAY
                | if self
                    .native
                    .as_ref()
                    .is_some_and(|native| native.tcp_available())
                {
                    TCP
                } else {
                    0
                }
                | if rtc_supported { RTC } else { 0 },
            permitted: if self.udp_allowed { UDP } else { 0 }
                | if self
                    .native
                    .as_ref()
                    .is_some_and(|native| native.tcp_available())
                {
                    TCP
                } else {
                    0
                }
                | if self.relay_allowed { RELAY } else { 0 }
                | if rtc_allowed { RTC } else { 0 },
            rtc_role: RtcRole::for_attempt(role, rtc_supported),
        }
    }
    fn authorization_revision(&self, connection: &Connection) -> io::Result<u64> {
        // A normally closed M has already lost its ephemeral proof entry.
        // That retirement is a network outcome, not evidence of bad identity.
        if connection.close_reason().is_some() {
            return Err(control_error("DataPathUnavailable"));
        }
        self.gate
            .path_qualification(connection)
            .map(|q| q.revision())
            .ok_or_else(|| control_error("IdentityOrAdmissionRejected"))
    }
    fn check<'a>(
        &'a self,
        connection: &'a Connection,
    ) -> impl Future<Output = io::Result<()>> + Send + 'a {
        async move {
            if connection.close_reason().is_some() {
                return Err(control_error("DataPathUnavailable"));
            }
            if self
                .gate
                .allows(connection, crate::admission_gate::ProtocolScope::Member)
            {
                Ok(())
            } else {
                Err(control_error("IdentityOrAdmissionRejected"))
            }
        }
    }
    fn existing_direct(&self, connection: &Connection) -> Option<(TransportAddr, u8)> {
        let paths = connection.paths();
        paths
            .iter()
            .filter_map(|path| {
                let capability = match path.remote_addr() {
                    TransportAddr::Ip(_) if self.udp_allowed => UDP,
                    TransportAddr::Custom(address)
                        if address.id() == crate::tcp_transport::DATA_TRANSPORT_ID
                            && self
                                .native
                                .as_ref()
                                .is_some_and(|native| native.tcp_available()) =>
                    {
                        TCP
                    }
                    _ => return None,
                };
                Some((path.is_selected(), path.remote_addr().clone(), capability))
            })
            .max_by_key(|(selected, _, _)| *selected)
            .map(|(_, route, capability)| (route, capability))
    }
    fn prepare<'a>(
        &'a self,
        endpoint: &'a Endpoint,
        connection: &'a Connection,
        attempt: ProtectedAttempt,
        plan: PathPlan,
    ) -> impl Future<Output = io::Result<PreparedPath>> + Send + 'a {
        async move {
            match plan {
                #[cfg(feature = "rtc-probe")]
                PathPlan::Rtc => {
                    crate::rtc_member::bootstrap_control_unified(
                        endpoint,
                        connection,
                        &self.gate,
                        self.rtc
                            .as_ref()
                            .ok_or_else(|| control_error("DataPathUnavailable"))?,
                        attempt,
                    )
                    .await
                }
                PathPlan::Native { tcp, udp } => self
                    .native
                    .as_ref()
                    .ok_or_else(|| control_error("DataPathUnavailable"))?
                    .prepare_paths(endpoint, connection, &self.gate, tcp, udp)
                    .await
                    .map_err(|error| {
                        if error.kind() == io::ErrorKind::InvalidData {
                            control_error("InvalidControl")
                        } else if !self.gate.allows_member_path_upgrade(connection) {
                            control_error("IdentityOrAdmissionRejected")
                        } else {
                            control_error("DataPathUnavailable")
                        }
                    }),
                PathPlan::Relay => {
                    crate::message_bootstrap::prepare_relay(
                        endpoint,
                        connection,
                        &self.gate,
                        self.resources
                            .as_ref()
                            .ok_or_else(|| control_error("DataPathUnavailable"))?,
                    )
                    .await?;
                    let paths = connection.paths();
                    let path = paths
                        .iter()
                        .filter(|path| path.is_relay())
                        .max_by_key(|path| path.is_selected())
                        .ok_or_else(|| control_error("DataPathUnavailable"))?;
                    Ok(PreparedPath::candidate(
                        connection,
                        path.remote_addr().clone(),
                        || Ok(()),
                    ))
                }
                _ => {
                    let _ = attempt;
                    Err(control_error("DataPathUnavailable"))
                }
            }
        }
    }
}
impl iroh::endpoint::EndpointHooks for CoordinatorHook {
    async fn before_data_release(
        &self,
        endpoint: &Endpoint,
        connection: &Connection,
    ) -> io::Result<()> {
        coordinate_member(endpoint, connection, self).await
    }
}

/// Existing direct-path release, shared by the native bootstrap and the
/// unified coordinator. Retention does not grant membership.
pub(crate) async fn release_existing_data(
    endpoint: &Endpoint,
    connection: &Connection,
    tcp: Option<&crate::tcp_punch::TcpPunch>,
    gate: &AdmissionGate,
) -> io::Result<()> {
    if let Some(tcp) = tcp
        && connection.paths().iter().any(|path| matches!(path.remote_addr(),
            TransportAddr::Custom(address) if address.id() == crate::tcp_transport::DATA_TRANSPORT_ID))
    {
        let control = ControlSession::from_connection(endpoint.clone(), connection.clone());
        let mut cancellation = control.cancellation_guard();
        tcp.retain_existing(endpoint, connection).await?;
        if !gate.allows_member_path_upgrade(connection) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied,
                "TCP membership changed before data release"));
        }
        cancellation.disarm();
    }
    Ok(())
}
