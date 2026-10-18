//! M77 fixture only: a real maintained Node and an ordinary continuity stream.
//! JSON stdout is private operator evidence, never a public diagnostic dump.
#[cfg(feature = "rtc-probe")]
mod enabled {
    use iroh::{
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
        endpoint: Mutex<Option<iroh::endpoint::RestrictedEndpoint>>,
        original: Mutex<Option<Original>>,
        allow_next_member: std::sync::atomic::AtomicBool,
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
        member_facts(original, gate)
    }

    fn member_facts(original: &Original, gate: &AdmissionGate) -> Value {
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
                "datagramSent":path.stats().frame_tx.datagram,"datagramReceived":path.stats().frame_rx.datagram,
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
            let admitted = member_facts(&original, &self.gate);
            let primary = {
                let mut current = self.observed.original.lock().unwrap();
                let additional = current.is_some()
                    && self
                        .observed
                        .allow_next_member
                        .swap(false, Ordering::AcqRel);
                if current
                    .as_ref()
                    .is_some_and(|original| original.weak.is_open())
                {
                    if !additional {
                        connection.close(0u32.into(), b"fixture owns one ordinary M");
                        return Err(AcceptError::from_err(std::io::Error::other(
                            "fixture original M already owned",
                        )));
                    }
                    // Preserve the old peer half for honest Terminal observations.
                    // This opt-in admits only one separate M, with its own streams.
                    false
                } else {
                    *current = Some(original);
                    true
                }
            };
            emit(json!({"event":"member-released","identityMatched":true,
                "ordinary":admitted,"fixturePrimary":primary}));
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
                        endpoint.id(),
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

    fn rtc_resource_facts(rtc: &RtcTransport) -> Value {
        json!({"resourceAllowed":rtc.resource_allowed(),"resourceRevision":rtc.resource_revision(),
            "activeAssociations":rtc.active_associations(),"activePumpTasks":rtc.active_pumps(),
            "publishedRoutes":rtc.published_routes_fixture(),
            "pumpObservations":rtc.pump_observations_fixture()})
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
            "rtcResource":rtc_resource_facts(rtc),
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
        let mut private_packet_witness = None::<std::path::PathBuf>;
        let mut flags = args[6..].iter();
        while let Some(flag) = flags.next() {
            match flag.as_str() {
                "--private-packet-witness" if private_packet_witness.is_none() => {
                    private_packet_witness = Some(
                        flags
                            .next()
                            .ok_or("owned private packet capture path required")?
                            .into(),
                    );
                }
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
        let mut packet_capture = private_packet_witness.as_deref()
            .map(miopunch_native_probe::rtc_packet_witness_fixture::PrivatePacketCaptureFixture::start).transpose()?;
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
            // This fixture keeps the caller's exact old handle across Terminal
            // observations and a later ordinary connect. It is not a registry.
            let mut retained_repair_original: Option<Connection> = None;
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
                            "stop-owned-data-relay" => {
                                let before=original_facts(&observed,&gate);
                                if !controlled_loopback || before["open"]!=true
                                    || before["memberAuthorized"]!=true || before["bindingUnchanged"]!=true
                                    || before["stableId"]!=request["originalStableId"]
                                    || rtc.active_associations()!=0 || rtc.active_pumps()!=0 {
                                    return Err("live sole-relay original M and exact owned loopback fault scope required".into());
                                }
                                let server=owned_relay.take().ok_or("owned relay server already stopped or absent")?;
                                let started=std::time::Instant::now();
                                server.shutdown().await?;
                                emit(json!({"event":"owned-data-relay-stopped",
                                    "actualPublicOperation":"iroh_relay::server::Server::shutdown",
                                    "actualOwnedRelayShutdownCompleted":true,"faultWallMs":started.elapsed().as_millis(),
                                    "beforeOriginal":before,"afterOriginal":original_facts(&observed,&gate),
                                    "contactNodeStatus":status(&owner,&observed,&gate,&rtc,&traffic,publications)}));
                            }
                            "retain-original-member" => {
                                let before=original_facts(&observed,&gate);
                                if retained_repair_original.is_some() || before["open"]!=true
                                    || before["memberAuthorized"]!=true || before["bindingUnchanged"]!=true
                                    || before["stableId"]!=request["originalStableId"]
                                    || request["allowOneSubsequentMember"]!=true {
                                    return Err("exact first live original caller handle and explicit one-subsequent fixture scope required".into());
                                }
                                retained_repair_original=observed.original.lock().unwrap().as_ref()
                                    .and_then(|original|original.weak.upgrade());
                                let original=retained_repair_original.as_ref().ok_or("original caller handle unavailable")?;
                                observed.allow_next_member.store(true,Ordering::Release);
                                emit(json!({"event":"original-member-retained","originalStableId":original.stable_id(),
                                    "retainedCallerHandle":true,"originalOpen":original.close_reason().is_none(),
                                    "oneSubsequentMemberPermitted":observed.allow_next_member.load(Ordering::Acquire)}));
                            }
                            "retire-original-relay-paths" => {
                                let before = original_facts(&observed,&gate);
                                if before["open"]!=true || before["bindingUnchanged"]!=true
                                    || before["memberAuthorized"]!=true
                                    || before["stableId"]!=request["originalStableId"] {
                                    return Err("exact qualified original M required for relay retirement".into());
                                }
                                let original = observed.original.lock().unwrap().as_ref()
                                    .and_then(|original|original.weak.upgrade())
                                    .ok_or("original M handle unavailable")?;
                                let rtc_ready = original.paths().iter().any(|path| {
                                    if let iroh::TransportAddr::Custom(address) = path.remote_addr()
                                        && address.id()==TRANSPORT_ID {
                                        return iroh::endpoint::transports::CustomSender::is_data_path_qualified(
                                            &rtc,address,original.stable_id());
                                    }
                                    false
                                });
                                let paths = original.paths();
                                let relays = paths.iter().filter(|path|path.is_relay()).collect::<Vec<_>>();
                                if !rtc_ready || relays.is_empty() {
                                    return Err("live qualified RTC and an old relay path required".into());
                                }
                                let count = relays.len();
                                for path in relays { path.close()?; }
                                emit(json!({"event":"original-relay-paths-retired",
                                    "actualPublicOperation":"Connection::paths/Path::close",
                                    "actualCloseCalls":count,"before":before,
                                    "after":original_facts(&observed,&gate),
                                    "rtcResource":rtc_resource_facts(&rtc)}));
                            }
                            "repair-original-member" => {
                                let caller_ms = request.get("callerMs").and_then(Value::as_u64)
                                    .filter(|value|*value>0 && *value<=60000)
                                    .ok_or("explicit bounded repair caller required")?;
                                if retained_repair_original.is_none() {
                                    retained_repair_original = observed.original.lock().unwrap().as_ref()
                                        .and_then(|original|original.weak.upgrade());
                                }
                                let original = retained_repair_original.as_ref()
                                    .ok_or("original SDK caller handle unavailable")?;
                                if request["originalStableId"].as_u64()!=Some(original.stable_id() as u64) {
                                    return Err("retained repair handle differs from expected original".into());
                                }
                                let mut contact = iroh::EndpointAddr::new(original.remote_id());
                                for url in endpoint.addr().relay_urls() {
                                    contact.addrs.insert(iroh::TransportAddr::Relay(url.clone()));
                                }
                                let started = std::time::Instant::now();
                                let deadline = n0_future::time::Instant::now()+Duration::from_millis(caller_ms);
                                let result = owner.repair_member_connection_until(original,contact,deadline).await;
                                let terminal = matches!(result.as_ref().err(),Some(
                                    miopunch_native_probe::connection_recovery::ConnectionPathRecoveryError::Terminal{..}));
                                emit(json!({"event":"original-member-repair-result",
                                    "outcome":if result.is_ok(){"recovered"}else if terminal{"terminal"}else{"unreachable"},
                                    "actualPublicOperation":"MaintainedNetwork::repair_member_connection_until",
                                    "callerBudgetMs":caller_ms,"sdkCallerMs":started.elapsed().as_millis(),
                                    "originalStableId":original.stable_id(),"originalOpen":original.close_reason().is_none(),
                                    "controlAlpn":String::from_utf8_lossy(miopunch_native_probe::connection_coordinator::ACTIVE_IMPROVEMENT_ALPN),
                                    "failure":result.err().map(|cause|cause.to_string()),
                                    "original":original_facts(&observed,&gate),
                                    "rtcResource":rtc_resource_facts(&rtc)}));
                            }
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
                            "set-rtc-resource-allowed" => {
                                let allowed = request.get("allowed").and_then(Value::as_bool)
                                    .ok_or("explicit RTC resource boolean required")?;
                                let stable = request.get("originalStableId").and_then(Value::as_u64)
                                    .ok_or("exact original M required")?;
                                let started = std::time::Instant::now();
                                let before = rtc_resource_facts(&rtc);
                                let before_original = original_facts(&observed,&gate);
                                let current_original = before_original["stableId"].as_u64() == Some(stable)
                                    && before_original["open"] == true
                                    && before_original["memberAuthorized"] == true
                                    && before_original["stableUnchanged"] == true
                                    && before_original["bindingUnchanged"] == true;
                                let actual_idle = before["pumpObservations"].as_array().is_some_and(|values| {
                                    values.iter().any(|value|value["originalStableId"].as_u64()==Some(stable)
                                        && value["originalOpen"]==true && value["routeReady"]==true
                                        && value["routeRetired"]==false && value["resourceAllowed"]==true
                                        && value["capturedResourceRevision"]==value["currentResourceRevision"]
                                        && value["observation"]["version"]==1
                                        && value["observation"]["phase"]=="actual-outbound-poll-pending"
                                        && value["observation"]["pendingLength"]==0
                                        && value["observation"]["blockedCause"].is_null())
                                });
                                let exact_path = request.get("pathId").and_then(Value::as_str)
                                    .is_some_and(|path_id|before_original["paths"].as_array().is_some_and(|paths| {
                                        paths.iter().any(|path|path["pathId"]==path_id && path["isRtc"]==true
                                            && ["streamSent","streamReceived","datagramSent","datagramReceived"]
                                                .iter().all(|key|path[*key].as_u64()==Some(0)))
                                    }));
                                let fresh_transition = before["resourceAllowed"].as_bool()==Some(!allowed);
                                let permitted = current_original && fresh_transition &&
                                    if allowed {
                                        before["activeAssociations"]==0 && before["activePumpTasks"]==0
                                            && before["publishedRoutes"].as_array().is_some_and(Vec::is_empty)
                                    } else { actual_idle && exact_path };
                                if permitted { rtc.set_resource_allowed(allowed); }
                                emit(json!({"event":"rtc-resource-permission-updated","applied":permitted,
                                    "requestedAllowed":allowed,"actualPublicOperation":"RtcTransport.set_resource_allowed",
                                    "before":before,"after":rtc_resource_facts(&rtc),
                                    "beforeOriginal":before_original,"afterOriginal":original_facts(&observed,&gate),
                                    "sdkCallerMs":started.elapsed().as_millis()}));
                            }
                            "arm-common-ready-input" => {
                                let stable = request.get("originalStableId").and_then(Value::as_u64)
                                    .ok_or("exact original M required")?;
                                let original = original_facts(&observed,&gate);
                                if original["stableId"].as_u64()!=Some(stable) || original["open"]!=true
                                    || original["memberAuthorized"]!=true || original["bindingUnchanged"]!=true
                                    || rtc.active_associations()!=0 {
                                    return Err("Common Ready fixture requires the healthy original M before improvement".into());
                                }
                                rtc.arm_common_ready_input_fixture(usize::try_from(stable)?)?;
                                emit(json!({"event":"common-ready-input-armed","originalStableId":stable}));
                            }
                            "arm-actual-dequeue" => {
                                let stable=request["originalStableId"].as_u64().ok_or("exact original M required")?;
                                let path=request["pathId"].as_str().ok_or("actual selected RTC path required")?;
                                let remaining=request["remainingMs"].as_u64().filter(|n|*n>0 && *n<=5000)
                                    .ok_or("explicit <=5000ms fixture remaining budget required")?;
                                let facts=original_facts(&observed,&gate);
                                if facts["stableId"].as_u64()!=Some(stable) || facts["open"]!=true
                                    || facts["memberAuthorized"]!=true || facts["bindingUnchanged"]!=true {
                                    return Err("actual dequeue fixture needs the same healthy authorized M".into());
                                }
                                let connection={let original=observed.original.lock().unwrap();
                                    original.as_ref().and_then(|o|o.weak.upgrade()).ok_or("original M ended")?};
                                let matcher=packet_capture.as_ref().ok_or("owned private packet capture must be enabled")?
                                    .matcher(usize::try_from(stable)?)?;
                                rtc.arm_actual_dequeue_fixture(&connection,path,
                                    n0_future::time::Instant::now()+Duration::from_millis(remaining),matcher)?;
                                emit(json!({"event":"actual-dequeue-armed","observation":rtc.actual_dequeue_fixture_snapshot()}));
                            }
                            "actual-dequeue-state" => {
                                let actual=rtc.actual_dequeue_fixture_snapshot();
                                let proof=actual.as_ref().and_then(|a|Some((
                                    usize::try_from(a["originalStableId"].as_u64()?).ok()?,
                                    a["actualCiphertextSha256"].as_str()?)))
                                    .and_then(|(stable,digest)|packet_capture.as_ref()?.proof(stable,digest));
                                emit(json!({"event":"actual-dequeue-state","observation":actual,"encoderProof":proof,
                                    "rtcResource":rtc_resource_facts(&rtc),"ordinary":original_facts(&observed,&gate)}));
                            }
                            "retire-matched-rtc-resource" => {
                                let actual=rtc.actual_dequeue_fixture_snapshot().ok_or("actual dequeue fixture not armed")?;
                                let stable=request["originalStableId"].as_u64().ok_or("exact original M required")?;
                                let before=rtc_resource_facts(&rtc);let ordinary=original_facts(&observed,&gate);
                                let matches=actual["matched"]==true && actual["waiting"]==true
                                    && actual["originalStableId"].as_u64()==Some(stable)
                                    && actual["actualCiphertextSha256"]==request["ciphertextSha256"]
                                    && actual["actualRouteAtMatch"]["qualified"]==true
                                    && ordinary["stableId"].as_u64()==Some(stable) && ordinary["open"]==true
                                    && ordinary["memberAuthorized"]==true && ordinary["bindingUnchanged"]==true
                                    && before["resourceAllowed"]==true;
                                if !matches {return Err("actual matched selected business pending packet required before retirement".into());}
                                let started=std::time::Instant::now();
                                rtc.set_resource_allowed(false);
                                emit(json!({"event":"matched-rtc-resource-retired","actualPublicOperation":"RtcTransport.set_resource_allowed(false)",
                                    "before":before,"after":rtc_resource_facts(&rtc),"pendingBeforeRetirement":actual,
                                    "ordinaryBefore":ordinary,"ordinaryAfter":original_facts(&observed,&gate),
                                    "sdkCallerMs":started.elapsed().as_millis()}));
                            }
                            "common-ready-input-state" => {
                                emit(json!({"event":"common-ready-input-state",
                                    "observation":rtc.common_ready_input_fixture_snapshot(),
                                    "rtcResource":rtc_resource_facts(&rtc),
                                    "ordinary":original_facts(&observed,&gate)}));
                            }
                            "release-common-ready-input" => {
                                let actual = rtc.common_ready_input_fixture_snapshot()
                                    .ok_or("actual protected Ready input was not read")?;
                                if actual["held"]!=true || actual["originalStableId"]!=request["originalStableId"]
                                    || actual["controlStableId"]!=request["controlStableId"]
                                    || actual["frameBlake3"]!=request["frameBlake3"] {
                                    return Err("Common Ready fixture release identity mismatch".into());
                                }
                                rtc.release_common_ready_input_fixture();
                                emit(json!({"event":"common-ready-input-released","actual":actual,
                                    "rtcResource":rtc_resource_facts(&rtc),
                                    "observation":rtc.common_ready_input_fixture_snapshot()}));
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
        rtc.clear_common_ready_input_fixture();
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
        let dequeue_terminal = rtc.clear_actual_dequeue_fixture()?;
        let packet_capture_terminal = packet_capture
            .as_mut()
            .map(|capture| capture.finish())
            .transpose()?;
        if let Some(result) = &packet_capture_terminal {
            emit(
                json!({"event":"private-packet-capture-ended","result":result,"dequeueTerminal":dequeue_terminal}),
            );
            if result["adapterLost"] != 0
                || result["observerLost"] != 0
                || result["writerNaturallyJoined"] != true
            {
                return Err("owned private packet capture lost actual records".into());
            }
        }
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
