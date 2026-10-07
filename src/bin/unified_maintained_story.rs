//! M77 fixture only: a real maintained Node and an ordinary continuity stream.
//! JSON stdout is private operator evidence, never a public diagnostic dump.
#[cfg(feature = "rtc-probe")]
mod enabled {
    use iroh::{
        Endpoint,
        endpoint::{Connection, WeakConnectionHandle},
        protocol::{AcceptError, ProtocolHandler},
    };
    use iroh_relay::server::{RelayConfig, Server, ServerConfig};
    use miopunch_native_probe::{
        admission_gate::{AdmissionGate, ProtocolScope},
        connection_coordinator::MEMBER_ALPN,
        mqtt_datagram::MqttFacility,
        mqtt_transport::{MqttTransport, Traffic},
        native_network::NativeNetwork,
        network_maintenance::{MaintainedNetwork, MaintenanceState},
        resources::{LanDiscovery, PortMapping, Resource, ResourceUse},
        rtc_association::CONTINUITY_BINDING_LABEL,
        rtc_ice_profile::RtcIceProfile,
        rtc_invalidation_fixture::ControlCheckpointStage,
        rtc_transport::{RtcTransport, TRANSPORT_ID},
    };
    use n0_watcher::Watcher;
    use serde_json::{Value, json};
    use std::{
        collections::BTreeSet,
        error::Error,
        fmt,
        net::Ipv4Addr,
        path::Path,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };
    use tokio::io::{AsyncBufReadExt, BufReader};

    type Failure = Box<dyn Error + Send + Sync>;

    fn emit(value: Value) {
        println!("{value}");
    }

    fn sockets() -> Result<BTreeSet<String>, Failure> {
        let mut values = BTreeSet::new();
        for entry in std::fs::read_dir("/proc/self/fd")? {
            let target = match std::fs::read_link(entry?.path()) {
                Ok(target) => target,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            if let Some(target) = target.to_str().filter(|text| text.starts_with("socket:[")) {
                values.insert(target.to_owned());
            }
        }
        Ok(values)
    }

    fn binding(connection: &Connection) -> Result<[u8; 32], Failure> {
        let mut value = [0; 32];
        connection
            .export_keying_material(&mut value, CONTINUITY_BINDING_LABEL, &[])
            .map_err(|_| "original M binding unavailable")?;
        Ok(value)
    }

    struct Original {
        weak: WeakConnectionHandle,
        stable: usize,
        binding: [u8; 32],
    }

    #[derive(Default)]
    struct Observed {
        endpoint: Mutex<Option<Endpoint>>,
        original: Mutex<Option<Original>>,
        handlers: AtomicUsize,
        stream_handlers: AtomicUsize,
        accepted_streams: AtomicUsize,
    }
    impl fmt::Debug for Observed {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("M77Observed(private original-M evidence)")
        }
    }

    #[derive(Debug)]
    struct Application {
        gate: AdmissionGate,
        rtc: RtcTransport,
        expected: iroh::EndpointId,
        observed: Arc<Observed>,
        member_data_service: Option<miopunch_native_probe::member_transport::MemberDataRelay>,
        member_data_peer: Option<iroh::EndpointId>,
        member_data_leases:
            Arc<tokio::sync::Mutex<Vec<miopunch_native_probe::member_transport::SignalingLease>>>,
    }

    struct HandlerGuard(Arc<Observed>);
    impl Drop for HandlerGuard {
        fn drop(&mut self) {
            self.0.handlers.fetch_sub(1, Ordering::AcqRel);
        }
    }

    struct StreamGuard(Arc<Observed>);
    impl Drop for StreamGuard {
        fn drop(&mut self) {
            self.0.stream_handlers.fetch_sub(1, Ordering::AcqRel);
        }
    }

    const MAX_APPLICATION_STREAMS: usize = 2;

    fn original_facts(observed: &Observed, gate: &AdmissionGate) -> Value {
        let values = observed.original.lock().unwrap();
        let Some(original) = values.as_ref() else {
            return json!({"observed":false});
        };
        let Some(connection) = original.weak.upgrade() else {
            return json!({"observed":true,"open":false});
        };
        json!({"observed":true,"open":connection.close_reason().is_none(),
            "stableId":connection.stable_id(),"stableUnchanged":connection.stable_id()==original.stable,
            "bindingUnchanged":binding(&connection).is_ok_and(|value|value==original.binding),
            "memberAuthorized":gate.allows(&connection,ProtocolScope::Member),
            "alpn":String::from_utf8_lossy(connection.alpn()),
            "quicRole":if connection.side().is_client(){"client"}else{"server"},
            "paths":connection.paths().iter().map(|path|json!({"pathId":path.id().to_string(),
                "route":path.remote_addr().to_string(),"selected":path.is_selected(),
                "isRelay":path.is_relay(),"isIp":path.is_ip(),
                "isMemberData":matches!(path.remote_addr(),iroh::TransportAddr::Custom(address)
                    if address.id()==miopunch_native_probe::member_transport::DATA_TRANSPORT_ID),
                "isRtc":matches!(path.remote_addr(),iroh::TransportAddr::Custom(address) if address.id()==TRANSPORT_ID),
                "rttMicros":path.stats().rtt.as_micros(),
                "streamSent":path.stats().frame_tx.stream,"streamReceived":path.stats().frame_rx.stream,
                "acksReceived":path.stats().frame_rx.acks})).collect::<Vec<_>>()})
    }

    impl ProtocolHandler for Application {
        async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
            self.observed.handlers.fetch_add(1, Ordering::AcqRel);
            let _handler = HandlerGuard(self.observed.clone());
            if connection.alpn() != MEMBER_ALPN
                || (connection.remote_id() != self.expected
                    && !(self.member_data_service.is_some()
                        && Some(connection.remote_id()) == self.member_data_peer))
                || !self.gate.allows(&connection, ProtocolScope::Member)
            {
                connection.close(0u32.into(), b"fixture expected ordinary member required");
                return Err(AcceptError::from_err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "fixture ordinary member refused",
                )));
            }
            if let Some(service) = &self.member_data_service {
                let result = service
                    .accept_link(&connection, Duration::from_secs(300))
                    .await;
                let permitted = result.is_ok();
                let failure = result.as_ref().err().map(ToString::to_string);
                if let Ok(lease) = result {
                    self.member_data_leases.lock().await.push(lease);
                }
                emit(
                    json!({"event":"member-data-forwarder-link","forwardDataPermitted":permitted,
                    "failure":failure,"outerStableId":connection.stable_id(),
                    "outerStillOpen":connection.close_reason().is_none(),
                    "memberAuthorized":self.gate.allows(&connection,ProtocolScope::Member),
                    "activeLinks":service.active_links()}),
                );
                // Denial of ForwardData does not close the admitted outer M.
                // Central SDK fixture ownership joins forwarding tasks before
                // Node shutdown, including links belonging to the other peer.
                connection.closed().await;
                return Ok(());
            }
            let original = Original {
                weak: connection.weak_handle(),
                stable: connection.stable_id(),
                binding: binding(&connection).map_err(|error| {
                    AcceptError::from_err(std::io::Error::other(error.to_string()))
                })?,
            };
            {
                let mut current = self.observed.original.lock().unwrap();
                if current
                    .as_ref()
                    .is_some_and(|original| original.weak.is_open())
                {
                    connection.close(0u32.into(), b"fixture owns one ordinary M");
                    return Err(AcceptError::from_err(std::io::Error::other(
                        "fixture original M already owned",
                    )));
                }
                *current = Some(original);
            }
            emit(json!({"event":"member-released","identityMatched":true,
                "ordinary":original_facts(&self.observed,&self.gate)}));
            let endpoint = self
                .observed
                .endpoint
                .lock()
                .unwrap()
                .as_ref()
                .cloned()
                .ok_or_else(|| {
                    AcceptError::from_err(std::io::Error::other("fixture endpoint unavailable"))
                })?;
            // Router owns endpoint.accept. Keep each application stream in an
            // owned JoinSet so a long-lived continuity stream cannot block a
            // later independent PING/PONG stream on this same M.
            let mut streams = tokio::task::JoinSet::new();
            loop {
                let next = tokio::select! {
                    _ = connection.closed() => break,
                    stream = connection.accept_bi(), if streams.len() < MAX_APPLICATION_STREAMS => Some(stream),
                    Some(result) = streams.join_next(), if !streams.is_empty() => {
                        if let Err(error) = result {
                            emit(json!({"event":"application-stream-task-ended",
                                "failure":error.to_string(),"originalOpen":connection.close_reason().is_none()}));
                        }
                        None
                    }
                };
                let Some(stream) = next else {
                    continue;
                };
                let (send, recv) = match stream {
                    Ok(stream) => stream,
                    Err(_) => break,
                };
                self.observed
                    .accepted_streams
                    .fetch_add(1, Ordering::AcqRel);
                self.observed.stream_handlers.fetch_add(1, Ordering::AcqRel);
                let observed = self.observed.clone();
                let endpoint = endpoint.clone();
                let gate = self.gate.clone();
                let rtc = self.rtc.clone();
                let connection = connection.clone();
                streams.spawn(async move {
                    let _guard = StreamGuard(observed.clone());
                    let result = miopunch_native_probe::rtc_member::serve_stream(
                        endpoint,
                        connection.clone(),
                        gate,
                        rtc,
                        send,
                        recv,
                    )
                    .await;
                    if let Err(error) = &result {
                        emit(json!({"event":"application-stream-ended",
                            "failure":error.to_string(),
                            "originalOpen":connection.close_reason().is_none()}));
                    }
                    result
                });
            }
            while streams.join_next().await.is_some() {}
            emit(
                json!({"event":"application-ended","ordinary":original_facts(&self.observed,&self.gate)}),
            );
            Ok(())
        }
    }

    fn status(
        owner: &MaintainedNetwork,
        observed: &Observed,
        gate: &AdmissionGate,
        rtc: &RtcTransport,
        traffic: &Traffic,
        publications: u64,
    ) -> Value {
        let value = owner.maintenance_status();
        let home_relay_status = owner
            .endpoint()
            .home_relay_status()
            .get()
            .into_iter()
            .map(|status| {
                json!({"url": status.url().to_string(), "connected": status.is_connected()})
            })
            .collect::<Vec<_>>();
        json!({"state":format!("{:?}",value.state),"contactAttempts":value.contact_attempts,
            "discoveryRounds":value.discovery_rounds,"maxInFlight":value.max_in_flight,
            "neighborCount":value.neighbors.len(),"retryAfterMs":value.retry_after.as_millis(),
            "watchNotifications":publications,"nodeId":owner.endpoint().id().to_string(),
            "contactListener":value.contact_listener,"homeRelayStatus":home_relay_status,
            "activeRtcAssociations":rtc.active_associations(),"activeRtcPumps":rtc.active_pumps(),
            "applicationHandlers":observed.handlers.load(Ordering::Acquire),
            "applicationStreamHandlers":observed.stream_handlers.load(Ordering::Acquire),
            "acceptedApplicationStreams":observed.accepted_streams.load(Ordering::Acquire),
            "traffic":traffic.snapshot(),"ordinary":original_facts(observed,gate)})
    }

    fn active_candidate_failure_state(rtc: &RtcTransport) -> Value {
        let checkpoint = rtc.control_fixture_snapshot();
        json!({
            "checkpointReached": checkpoint.is_some(),
            "held": checkpoint
                .as_ref()
                .and_then(|value| value.get("held"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            "activeRtcAssociations": rtc.active_associations(),
            "activeRtcPumps": rtc.active_pumps(),
        })
    }

    async fn run() -> Result<(), Failure> {
        let args: Vec<_> = std::env::args().collect();
        if args.len() < 6 {
            return Err("usage: unified_maintained_story SAVED_PROVIDER_DIR FACILITY_JSON RTC_PROFILE PERMITTED_RELAY EXPECTED_BROWSER_ID [--controlled-loopback] [--member-data-via MEMBER | --member-data-forwarder TARGET_MEMBER enabled|disabled]".into());
        }
        let mut controlled_loopback = false;
        let mut no_public_data_relay = false;
        let mut native_udp = false;
        let mut path_preference = None;
        let mut member_data_via = None::<iroh::EndpointId>;
        let mut member_data_forwarder = None::<bool>;
        let mut member_data_peer = None::<iroh::EndpointId>;
        let mut flags = args[6..].iter();
        while let Some(flag) = flags.next() {
            match flag.as_str() {
                "--controlled-loopback" if !controlled_loopback => controlled_loopback = true,
                "--no-public-data-relay" if !no_public_data_relay => no_public_data_relay = true,
                "--native-udp" if !native_udp => native_udp = true,
                "--path-preference" if path_preference.is_none() => {
                    path_preference = Some(match flags.next().map(String::as_str) {
                        Some("Quality") => {
                            miopunch_native_probe::path_selection::PathPreference::Quality
                        }
                        Some("PreferDirect") => {
                            miopunch_native_probe::path_selection::PathPreference::PreferDirect
                        }
                        Some("PreferRelay") => {
                            miopunch_native_probe::path_selection::PathPreference::PreferRelay
                        }
                        _ => return Err("existing path preference required".into()),
                    })
                }
                "--member-data-via"
                    if member_data_via.is_none() && member_data_forwarder.is_none() =>
                {
                    member_data_via = Some(flags.next().ok_or("MID1 member missing")?.parse()?)
                }
                "--member-data-forwarder"
                    if member_data_forwarder.is_none() && member_data_via.is_none() =>
                {
                    member_data_peer = Some(
                        flags
                            .next()
                            .ok_or("MID1 second admitted member missing")?
                            .parse()?,
                    );
                    member_data_forwarder = Some(match flags.next().map(String::as_str) {
                        Some("enabled") => true,
                        Some("disabled") => false,
                        _ => return Err("explicit MID1 forwarder enabled|disabled required".into()),
                    });
                }
                _ => return Err("unknown or repeated maintained fixture flag".into()),
            }
        }
        if no_public_data_relay && controlled_loopback {
            return Err("no-public-data-relay cannot create a controlled relay".into());
        }
        if !Path::new(&args[1]).is_dir() {
            return Err("a real saved SDK provider store is required".into());
        }
        let sockets_before = sockets()?.len();
        let node = NativeNetwork::open(&args[1]).await?;
        let expected = args[5].parse::<iroh::EndpointId>()?;
        if !node
            .status()
            .members
            .iter()
            .any(|(member, _)| member.as_bytes() == expected.as_bytes())
        {
            node.close().await;
            return Err("expected Browser is not in the verified saved membership".into());
        }
        for peer in [member_data_via, member_data_peer].into_iter().flatten() {
            if peer == expected
                || peer.as_bytes() == node.status().node_id.as_bytes()
                || !node
                    .status()
                    .members
                    .iter()
                    .any(|(id, _)| id.as_bytes() == peer.as_bytes())
            {
                return Err("MID1 role requires a different independently admitted member".into());
            }
        }
        let facility: MqttFacility = serde_json::from_slice(&std::fs::read(&args[2])?)?;
        facility.validate()?;
        let profile: RtcIceProfile = serde_json::from_slice(&std::fs::read(&args[3])?)?;
        profile.validate()?;
        let mut owned_relay = None;
        let relay = if controlled_loopback {
            let mut relay_config = ServerConfig::default();
            relay_config.relay = Some(RelayConfig::new((Ipv4Addr::LOCALHOST, 0)));
            let server = Server::spawn(relay_config).await?;
            let relay_address = server
                .http_addr()
                .ok_or("owned relay did not expose an HTTP address")?;
            let relay = format!("http://{relay_address}/").parse::<iroh::RelayUrl>()?;
            owned_relay = Some(server);
            relay
        } else {
            args[4].parse::<iroh::RelayUrl>()?
        };
        if !facility.native_url.starts_with("mqtts://")
            || !facility.wss_url.starts_with("wss://")
            || (!controlled_loopback && relay.scheme() != "https")
            || (controlled_loopback && relay.scheme() != "http")
        {
            return Err(
                "normal TLS contact and controlled HTTP or public HTTPS relay required".into(),
            );
        }
        let mut resources = node.resource_configuration()?;
        if !no_public_data_relay
            && resources
                .disabled
                .contains(&(relay.to_string(), ResourceUse::DataRelay))
        {
            return Err("explicit relay remains disabled".into());
        }
        profile.check_allowed(
            &resources
                .disabled
                .iter()
                .filter(|(_, usage)| *usage == ResourceUse::Observation)
                .map(|(url, _)| url.clone())
                .collect::<Vec<_>>(),
        )?;
        let mut selected = vec![Resource {
            url: facility.native_url.clone(),
            uses: [ResourceUse::Signaling].into(),
            limitation: "explicit owned maintained fixture contact".into(),
        }];
        if no_public_data_relay {
            if resources.disabled_uses.contains(&ResourceUse::DataRelay) {
                return Err("saved all-data-relay disable remains decisive for MID1".into());
            }
            resources
                .disabled
                .insert((relay.to_string(), ResourceUse::DataRelay));
        } else {
            selected.push(Resource {
                url: relay.to_string(),
                uses: [ResourceUse::DataRelay].into(),
                limitation: "explicit owned maintained fixture permitted relay".into(),
            });
        }
        for url in profile.observation_resources()? {
            selected.push(Resource {
                url,
                uses: [ResourceUse::Observation].into(),
                limitation: "existing approved RTC profile".into(),
            });
        }
        resources.import_trusted_replacement(selected)?;
        resources.mqtt_facilities = vec![facility.clone()];
        resources.lan_discovery = LanDiscovery::Disabled;
        resources.port_mapping = PortMapping::Disabled;
        resources.udp_direct_disabled = !native_udp;
        resources.tcp_direct_disabled = true;
        resources.member_data_relay_disabled =
            !(member_data_via.is_some() || member_data_forwarder == Some(true));
        resources.disabled_uses.remove(&ResourceUse::DataRelay);
        resources.disabled_uses.remove(&ResourceUse::Observation);
        if let Some(preference) = path_preference {
            resources.path_selection.preference = preference;
        }
        // The default keeps the saved policy; an explicit existing preference
        // changes no RTT scores, hysteresis or reevaluation interval.
        let policy = resources.maintenance.clone();
        let path_policy = resources.path_selection.clone();
        node.save_resource_configuration(&resources)?;
        let gate = node.admission_gate();
        let traffic = Arc::new(Traffic::default());
        let contact = Arc::new(MqttTransport::from_facility(
            &facility,
            &resources
                .disabled
                .iter()
                .filter(|(_, usage)| *usage == ResourceUse::Signaling)
                .map(|(url, _)| url.clone())
                .collect::<Vec<_>>(),
            traffic.clone(),
            None,
        )?);
        let rtc = RtcTransport::with_ice_profile(profile)?;
        let member_data_transport = member_data_via.map(|via| {
            Arc::new(
                miopunch_native_probe::member_transport::MemberDataTransport::new(
                    via,
                    gate.clone(),
                ),
            )
        });
        let member_data_service = member_data_forwarder.map(|enabled| {
            let service =
                miopunch_native_probe::member_transport::MemberDataRelay::new(gate.clone());
            service.set_enabled(
                enabled
                    && !resources.member_data_relay_disabled
                    && !resources.disabled_uses.contains(&ResourceUse::DataRelay),
            );
            service
        });
        let member_data_leases: Arc<
            tokio::sync::Mutex<Vec<miopunch_native_probe::member_transport::SignalingLease>>,
        > = Arc::default();
        let mut member_data_outer = Vec::<Connection>::new();
        let observed = Arc::new(Observed::default());
        let application = Arc::new(Application {
            gate: gate.clone(),
            rtc: rtc.clone(),
            expected,
            observed: observed.clone(),
            member_data_service: member_data_service.clone(),
            member_data_peer,
            member_data_leases: member_data_leases.clone(),
        });
        let owner = node
            .maintain_network_with_rtc_and_contact(
                Ipv4Addr::UNSPECIFIED,
                rtc.clone(),
                contact.clone(),
                |builder| {
                    let mut builder = if native_udp {
                        builder
                    } else {
                        builder.clear_ip_transports()
                    };
                    if let Some(transport) = &member_data_transport {
                        builder = builder.add_custom_transport(transport.clone());
                    }
                    builder
                },
                |router| router.accept(MEMBER_ALPN, application),
            )
            .await?;
        let endpoint = owner.endpoint().clone();
        *observed.endpoint.lock().unwrap() = Some(endpoint.clone());
        let mut priority = Some(owner.business_activity());
        let mut updates = owner.watch_maintenance();
        let mut publications = 0u64;
        let operation = async {
            tokio::time::timeout(Duration::from_secs(10), async {
                contact.wait_ready().await?;
                if !no_public_data_relay { endpoint.online().await; }
                Ok::<_, std::io::Error>(())
            }).await??;
            emit(json!({"event":"ready","id":endpoint.id().to_string(),"route":contact.route()?,
                "normalPublicTrust":true,"maintainedNode":true,"rtcEnabled":true,
                "controlledLoopback":controlled_loopback,
                "permittedRelay":if no_public_data_relay {None}else{Some(relay.to_string())},
                "publicDataRelayEnabled":!no_public_data_relay,"nativeUdpEnabled":native_udp,
                "directAddresses":endpoint.addr().ip_addrs().map(ToString::to_string).collect::<Vec<_>>(),
                "savedMaintenancePolicy":policy,"savedPathSelectionPolicy":path_policy,
                "knownMembers":owner.node_status().members.iter().map(|(id,_)|id.to_string()).collect::<Vec<_>>(),
                "relayAddressCount":endpoint.addr().relay_urls().count(),
                "status":status(&owner,&observed,&gate,&rtc,&traffic,publications)}));
            let mut lines = BufReader::new(tokio::io::stdin()).lines();
            loop {
                tokio::select! {
                    changed = updates.changed() => {
                        changed?;
                        publications=publications.checked_add(1).ok_or("watch count overflow")?;
                        emit(json!({"event":"maintenance-publication",
                            "status":status(&owner,&observed,&gate,&rtc,&traffic,publications)}));
                    }
                    line = lines.next_line() => {
                        let Some(line) = line? else { break; };
                        let request: Value = serde_json::from_str(&line)?;
                        let op = request.get("op").and_then(Value::as_str).ok_or("fixture command op missing")?;
                        match op {
                            "status" => emit(json!({"event":"status","status":status(&owner,&observed,&gate,&rtc,&traffic,publications)})),
                            "member-data-attach" => {
                                if !member_data_outer.is_empty() { return Err("MID1 fixture generation already attached".into()); }
                                let via = member_data_via.ok_or("MID1 consumer not explicitly installed")?;
                                let mut target = iroh::EndpointAddr::new(via);
                                if let Some(addresses) = request.get("directAddresses").and_then(Value::as_array) {
                                    if addresses.len() > 8 || !native_udp { return Err("MID1 outer direct hints unavailable or too many".into()); }
                                    for address in addresses {
                                        target.addrs.insert(iroh::TransportAddr::Ip(address.as_str().ok_or("outer address missing")?.parse()?));
                                    }
                                }
                                for url in endpoint.addr().relay_urls() { target.addrs.insert(iroh::TransportAddr::Relay(url.clone())); }
                                if target.addrs.is_empty() { return Err("admitted outer member needs an actual allowed direct or relay hint".into()); }
                                let deadline = n0_future::time::Instant::now() + Duration::from_secs(60);
                                let result = owner.coordinator().connect(target,
                                    miopunch_native_probe::connection_coordinator::SessionPurpose::MemberBusiness,
                                    miopunch_native_probe::connection_coordinator::AttemptContext::new(deadline,
                                        tokio_util::sync::CancellationToken::new())).await;
                                match result {
                                    Ok(outer) => {
                                        member_data_outer.push(outer.clone());
                                        let result = tokio::time::timeout(
                                            deadline.saturating_duration_since(n0_future::time::Instant::now()),
                                            member_data_transport.as_ref().unwrap().attach(&outer,Duration::from_secs(300)),
                                        ).await.unwrap_or_else(|_| Err(std::io::Error::new(
                                            std::io::ErrorKind::TimedOut,"MID1 attach caller deadline exceeded")));
                                        let attached = result.is_ok();
                                        let failure = result.as_ref().err().map(ToString::to_string);
                                        if let Ok(lease) = result { member_data_leases.lock().await.push(lease); }
                                        emit(json!({"event":"member-data-attached","attached":attached,"failure":failure,
                                            "outerStableId":outer.stable_id(),"outerStillOpen":outer.close_reason().is_none(),
                                            "outerAlpn":String::from_utf8_lossy(outer.alpn()),
                                            "outerPaths":outer.paths().iter().map(|path|json!({"selected":path.is_selected(),"isIp":path.is_ip(),"isRelay":path.is_relay(),
                                                "isMemberData":matches!(path.remote_addr(),iroh::TransportAddr::Custom(address) if address.id()==miopunch_native_probe::member_transport::DATA_TRANSPORT_ID),
                                                "isRtc":matches!(path.remote_addr(),iroh::TransportAddr::Custom(address) if address.id()==TRANSPORT_ID)})).collect::<Vec<_>>(),
                                            "outerMemberAuthorized":gate.allows(&outer,ProtocolScope::Member),
                                            "dynamicRoute":member_data_transport.as_ref().and_then(|transport|transport.address().ok()).map(|address|hex::encode(address.data()))}));
                                    }
                                    Err(error) => emit(json!({"event":"member-data-attached","attached":false,
                                        "failure":error.to_string(),"outerReturned":false})),
                                }
                            }
                            "member-data-status" => {
                                let leases = member_data_leases.lock().await;
                                emit(json!({"event":"member-data-status",
                                    "installed":member_data_transport.is_some(),
                                    "activeForwarderLinks":member_data_service.as_ref().map(|service|service.active_links()),
                                    "liveLeases":leases.iter().filter(|lease|!lease.is_finished()).count(),
                                    "outerConnectionsOpen":member_data_outer.iter().filter(|connection|connection.close_reason().is_none()).count(),
                                    "dynamicRoute":member_data_transport.as_ref().and_then(|transport|transport.address().ok()).map(|address|hex::encode(address.data()))}));
                            }
                            "member-data-disable" => {
                                if let Some(service) = &member_data_service { service.set_enabled(false); }
                                let mut leases = std::mem::take(&mut *member_data_leases.lock().await);
                                for lease in &mut leases { lease.close().await; }
                                emit(json!({"event":"member-data-disabled","ownedLeasesJoined":true,
                                    "outerConnectionsOpen":member_data_outer.iter().filter(|connection|connection.close_reason().is_none()).count(),
                                    "activeForwarderLinks":member_data_service.as_ref().map(|service|service.active_links())}));
                            }
                            "release-business-priority" => {
                                if priority.take().is_none() { return Err("fixture priority already released".into()); }
                                emit(json!({"event":"priority-released","businessPriorityReleased":true,
                                    "status":status(&owner,&observed,&gate,&rtc,&traffic,publications)}));
                            }
                            "disable-contact-signaling" => {
                                let started = std::time::Instant::now();
                                let before = status(&owner,&observed,&gate,&rtc,&traffic,publications);
                                let result = owner.disable_contact_signaling().await;
                                emit(json!({"event":"contact-signaling-disabled",
                                    "completed":result.is_ok(),"error":result.err().map(|error| error.to_string()),
                                    "sdkCallerMs":started.elapsed().as_millis(),"before":before,
                                    "status":status(&owner,&observed,&gate,&rtc,&traffic,publications)}));
                            }
                            "network-changed" => {
                                let started = std::time::Instant::now();
                                owner.network_changed().await?;
                                emit(json!({"event":"network-changed","sdkCallerMs":started.elapsed().as_millis(),
                                    "status":status(&owner,&observed,&gate,&rtc,&traffic,publications)}));
                            }
                            "arm-active-candidate-failure" => {
                                rtc.arm_control_fixture(ControlCheckpointStage::BeforeRelease)?;
                                emit(json!({"event":"active-candidate-failure-armed",
                                    "stage":"before-release"}));
                            }
                            "active-candidate-failure-state" => {
                                for _ in 0..10 {
                                    let state = active_candidate_failure_state(&rtc);
                                    if state.get("held").and_then(Value::as_bool) != Some(true) {
                                        break;
                                    }
                                    tokio::time::sleep(Duration::from_millis(100)).await;
                                }
                                emit(json!({"event":"active-candidate-failure-state",
                                    "state":active_candidate_failure_state(&rtc)}));
                            }
                            "clear-active-candidate-failure" => {
                                rtc.clear_control_fixture();
                                emit(json!({"event":"active-candidate-failure-cleared",
                                    "state":active_candidate_failure_state(&rtc)}));
                            }
                            "stop" => break,
                            _ => return Err("unknown fixture command".into()),
                        }
                    }
                }
            }
            Ok::<_,Failure>(())
        }.await;
        if let Err(error) = &operation {
            emit(json!({"event":"operator-error","failure":error.to_string()}));
        }
        let cleanup_started = std::time::Instant::now();
        drop(priority.take());
        if let Some(service) = &member_data_service {
            service.set_enabled(false);
        }
        let mut leases = std::mem::take(&mut *member_data_leases.lock().await);
        for lease in &mut leases {
            lease.close().await;
        }
        emit(
            json!({"event":"member-data-retired","ownedLeasesJoined":true,
            "activeForwarderLinks":member_data_service.as_ref().map(|service|service.active_links()),
            "liveLeases":leases.iter().filter(|lease|!lease.is_finished()).count()}),
        );
        for outer in member_data_outer {
            outer.close(0u32.into(), b"owned MID1 fixture stopped");
        }
        observed.endpoint.lock().unwrap().take();
        endpoint.close().await;
        owner.close().await;
        if let Some(server) = owned_relay.take() {
            server.shutdown().await?;
        }
        let maintenance_stopped = updates.borrow().state == MaintenanceState::Stopped;
        drop(endpoint);
        tokio::time::timeout(Duration::from_secs(1), contact.wait_driver_closed()).await??;
        let immediate = json!({"socketCount":sockets()?.len(),"activeRtcAssociations":rtc.active_associations(),
            "activeRtcPumps":rtc.active_pumps(),"applicationHandlers":observed.handlers.load(Ordering::Acquire),
            "applicationStreamHandlers":observed.stream_handlers.load(Ordering::Acquire),
            "traffic":traffic.snapshot()});
        tokio::time::sleep(Duration::from_millis(100)).await;
        let sockets_late = sockets()?.len();
        let late = json!({"socketCount":sockets_late,"activeRtcAssociations":rtc.active_associations(),
            "activeRtcPumps":rtc.active_pumps(),"applicationHandlers":observed.handlers.load(Ordering::Acquire),
            "applicationStreamHandlers":observed.stream_handlers.load(Ordering::Acquire),
            "traffic":traffic.snapshot()});
        emit(
            json!({"event":"closed","activeRtcAssociations":rtc.active_associations(),
            "activeRtcPumps":rtc.active_pumps(),"socketsBefore":sockets_before,"socketsLate":sockets_late,
            "immediate":immediate,"late":late,"lateDelayMs":100,
            "cleanupMs":cleanup_started.elapsed().as_millis(),"maintenanceTaskJoined":true,
            "maintenanceStopped":maintenance_stopped}),
        );
        if rtc.active_associations() != 0
            || rtc.active_pumps() != 0
            || observed.handlers.load(Ordering::Acquire) != 0
            || observed.stream_handlers.load(Ordering::Acquire) != 0
            || !maintenance_stopped
        {
            return Err("owned maintained fixture resources remain".into());
        }
        operation
    }

    #[tokio::main]
    pub(super) async fn main() {
        if let Err(error) = run().await {
            emit(json!({"event":"terminal-error","failure":error.to_string()}));
            std::process::exit(1);
        }
    }
}

#[cfg(feature = "rtc-probe")]
fn main() {
    enabled::main();
}
#[cfg(not(feature = "rtc-probe"))]
fn main() {
    eprintln!("unified_maintained_story requires rtc-probe");
    std::process::exit(1);
}
