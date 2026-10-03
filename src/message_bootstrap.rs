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

use crate::member_path_protocol::{PathCapabilities, RtcRole, exchange_capabilities};

pub(crate) struct MemberPathBootstrap {
    gate: AdmissionGate,
    udp: Arc<NatConnector>,
    tcp: Option<crate::tcp_punch::TcpPunch>,
    resources: crate::resources::ResourceConfig,
    #[cfg(feature = "rtc-probe")]
    rtc: Option<crate::rtc_transport::RtcTransport>,
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
            #[cfg(feature = "rtc-probe")]
            rtc: None,
        }
    }

    #[cfg(feature = "rtc-probe")]
    pub(crate) fn with_rtc(mut self, rtc: Option<crate::rtc_transport::RtcTransport>) -> Self {
        self.rtc = rtc;
        self
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
        let exchange =
            crate::member_path_protocol::RelayValidation::begin(connection, local).await?;
        let stream_id = exchange.stream_id.clone();
        let remote = exchange
            .candidates
            .iter()
            .find(|url| {
                self.resources
                    .allows(url.as_str(), crate::resources::ResourceUse::DataRelay)
            })
            .cloned();
        let validation = async {
            let remote = remote.ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied, "no mutually permitted relay"))?;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
            let started = tokio::time::Instant::now();
            let mut fresh = None;
            loop {
                check()?;
                if connection.side().is_client() {
                    match endpoint.open_control_relay_path(connection, remote.clone()).await {
                        Ok(()) => {},
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {},
                        Err(error) => return Err(error),
                    }
                }
                if let Some(path) = connection.paths().iter().find(|path| matches!(path.remote_addr(), TransportAddr::Relay(url)
                    if self.resources.allows(url.as_str(), crate::resources::ResourceUse::DataRelay))) {
                    if fresh.is_none() { fresh = Some((path.id().to_string(), path.start_probe()?, started.elapsed().as_millis())); }
                    let (path_id, probe, probe_started) = fresh.as_ref().unwrap();
                    if path.id().to_string() != *path_id { return Err(io::Error::other("relay probe path changed")); }
                    if probe.is_acknowledged().map_err(io::Error::other)? {
                        return Ok(serde_json::json!({"pathId":path_id,"freshProbeStartedMs":probe_started,"freshAckMs":started.elapsed().as_millis(),"probeWatermark":probe.watermark(),"freshAck":true}));
                    }
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
        let observation = validation.as_ref().ok().cloned();
        let continuity = exchange.finish(connection, validation.map(|_| ())).await?;
        check()?;
        println!(
            "{}",
            serde_json::json!({"event":"relay-path", "stableId":connection.stable_id(),"privateStreamId":stream_id,
            "stableUnchanged":continuity.stable_unchanged,"bindingUnchanged":continuity.binding_unchanged,"probe":observation,
            "relayFramesAfterConfirmation":connection.paths().iter().filter(|path|path.is_relay()).map(|path|serde_json::json!({"pathId":path.id().to_string(),"selected":path.is_selected(),"streamSent":path.stats().frame_tx.stream,"streamReceived":path.stats().frame_rx.stream})).collect::<Vec<_>>()})
        );
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
                    rtc: {
                        #[cfg(feature = "rtc-probe")]
                        {
                            RtcRole::native_adapter(
                                connection,
                                carrier == "MQTT"
                                    && self.rtc.as_ref().is_some_and(|rtc| rtc.resource_allowed())
                                    && self.gate.path_qualification(connection).is_some(),
                            )
                        }
                        #[cfg(not(feature = "rtc-probe"))]
                        {
                            RtcRole::None
                        }
                    },
                },
            )
            .await?;
            println!(
                "{}",
                serde_json::json!({"event":"path-capabilities", "stableId":connection.stable_id(),
                "alpn":String::from_utf8_lossy(connection.alpn()), "version":capabilities.version.number(),
                "localRtc":format!("{:?}", capabilities.local_rtc), "peerRtc":format!("{:?}", capabilities.peer_rtc),
                "rtc":capabilities.rtc,"tcp":capabilities.tcp,"udp":capabilities.udp,"relay":capabilities.relay,
                "initialPaths":connection.paths().iter().map(|path|serde_json::json!({"isRelay":path.is_relay(),"isIp":path.is_ip(),"isMqtt":matches!(path.remote_addr(),TransportAddr::Custom(addr) if addr.id()==TRANSPORT_ID),"selected":path.is_selected()})).collect::<Vec<_>>()})
            );
            let capabilities = capabilities.require_usable()?;
            #[cfg(feature = "rtc-probe")]
            if capabilities.rtc {
                return crate::rtc_member::bootstrap_control(
                    endpoint,
                    connection,
                    &self.gate,
                    self.rtc
                        .as_ref()
                        .ok_or_else(|| io::Error::other("RTC adapter absent"))?,
                )
                .await;
            }
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
        let budget = connection
            .caller_deadline()
            .map_or(Duration::from_secs(45), |deadline| {
                deadline.saturating_duration_since(n0_future::time::Instant::now())
            });
        let result = tokio::time::timeout(budget, operation)
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
