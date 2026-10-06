//! Private operator for the MIO83 SDK story. Never persist its raw stdout.
//! Admission and replica remain explicitly unqualified until their common
//! purpose seam is released; this operator supplies no replacement protocol.
#[cfg(feature = "rtc-probe")]
mod enabled {
    use iroh::{Endpoint, EndpointAddr, endpoint::presets};
    use miopunch_native_probe::{
        admission_gate::{ADMISSION_REQUEST_ALPN, ProtocolScope},
        daily_management::ManagementAction,
        manual_admission::{AdmissionRequest, ManualExchange},
        mqtt_datagram::MqttFacility,
        mqtt_transport::{MqttTransport, Traffic},
        native_network::NativeNetwork,
        resources::{LanDiscovery, PortMapping, Resource, ResourceUse},
        rtc_ice_profile::RtcIceProfile,
        rtc_transport::RtcTransport,
    };
    use serde_json::{Value, json};
    use std::{
        collections::BTreeSet, error::Error, net::Ipv4Addr, path::Path, sync::Arc, time::Duration,
    };
    use tokio::io::{AsyncBufReadExt, BufReader};

    type Failure = Box<dyn Error + Send + Sync>;
    fn emit(value: Value) {
        println!("{value}");
    }
    fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, Failure> {
        value
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("missing {key}").into())
    }
    fn socket_inodes() -> Result<BTreeSet<String>, Failure> {
        let mut observed = BTreeSet::new();
        for entry in std::fs::read_dir("/proc/self/fd")? {
            let target = match std::fs::read_link(entry?.path()) {
                Ok(target) => target,
                // The directory iterator's own FD may retire during the scan.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            if let Some(target) = target
                .to_str()
                .filter(|target| target.starts_with("socket:["))
            {
                observed.insert(target.to_owned());
            }
        }
        Ok(observed)
    }
    fn endpoint_socket_inodes(endpoint: &Endpoint) -> Result<BTreeSet<String>, Failure> {
        let ports: BTreeSet<_> = endpoint
            .addr()
            .ip_addrs()
            .map(|address| address.port())
            .collect();
        if ports.is_empty() {
            return Err("owned endpoint local port was not observed".into());
        }
        let mut observed = BTreeSet::new();
        for table in ["/proc/self/net/udp", "/proc/self/net/udp6"] {
            for line in std::fs::read_to_string(table)?.lines().skip(1) {
                let fields: Vec<_> = line.split_whitespace().collect();
                if fields.len() < 10 {
                    continue;
                }
                let Some((_, port)) = fields[1].split_once(':') else {
                    continue;
                };
                if u16::from_str_radix(port, 16).is_ok_and(|port| ports.contains(&port)) {
                    observed.insert(format!("socket:[{}]", fields[9]));
                }
            }
        }
        let owned: BTreeSet<_> = observed.intersection(&socket_inodes()?).cloned().collect();
        if owned.is_empty() {
            return Err("owned UDP socket inode was not observed".into());
        }
        Ok(owned)
    }
    async fn connect(
        client: &Endpoint,
        provider: &Endpoint,
    ) -> Result<(iroh::endpoint::Connection, iroh::endpoint::Connection), Failure> {
        let address = provider
            .addr()
            .ip_addrs()
            .find(|address| address.is_ipv4())
            .copied()
            .ok_or("owned loopback IPv4 unavailable")?;
        let deadline = n0_future::time::Instant::now() + Duration::from_secs(12);
        let (out, incoming) = tokio::join!(
            async {
                client
                    .connect_with_opts(
                        EndpointAddr::new(provider.id()).with_ip_addr(address),
                        ADMISSION_REQUEST_ALPN,
                        iroh::endpoint::ConnectOptions::new().with_caller_deadline(deadline),
                    )
                    .await
                    .map_err(|cause| cause.to_string())?
                    .await
                    .map_err(|cause| cause.to_string())
            },
            async {
                provider
                    .accept()
                    .await
                    .ok_or("owned provider stopped")?
                    .with_caller_deadline(deadline)
                    .await
                    .map_err(|error| error.to_string())
            }
        );
        Ok((out?, incoming?))
    }

    // Only this owned fixture store is changed. The live public assembly keeps
    // its immutable resource copy; restore the saved configuration even on failure.
    async fn dataset_endpoint(node: &NativeNetwork) -> Result<Endpoint, Failure> {
        let original = node.resource_configuration()?;
        let mut local = original.clone();
        local.import_trusted_replacement(Vec::new())?;
        local.mqtt_facilities.clear();
        local.disabled_uses.extend([
            ResourceUse::Observation,
            ResourceUse::DataRelay,
            ResourceUse::Signaling,
            ResourceUse::Lookup,
        ]);
        local.udp_direct_disabled = false;
        local.tcp_direct_disabled = true;
        local.member_data_relay_disabled = true;
        local.lan_discovery = LanDiscovery::Disabled;
        local.port_mapping = PortMapping::Disabled;
        node.save_resource_configuration(&local)?;
        let result = node.bind_endpoint(Ipv4Addr::LOCALHOST).await;
        let restored = node.save_resource_configuration(&original);
        if let Err(error) = restored {
            if let Ok(endpoint) = result {
                endpoint.close().await;
            }
            return Err(error.into());
        }
        Ok(result?)
    }

    /// Independent identity/store admitted by the real SDK. Only public signed
    /// records pass between nodes. No key file, custom signer or copied secret.
    async fn offline_author(
        provider: &mut NativeNetwork,
        directory: &Path,
    ) -> Result<Value, Failure> {
        if directory.exists() {
            return Err("independent author directory already exists".into());
        }
        let mut author = NativeNetwork::join(directory, provider.manual_invitation()?).await?;
        let author_id = author.status().node_id;
        if author_id == provider.status().node_id {
            return Err("author identity is not independent".into());
        }
        let pe = dataset_endpoint(provider).await?;
        let ae = match dataset_endpoint(&author).await {
            Ok(endpoint) => endpoint,
            Err(error) => {
                pe.close().await;
                author.close().await;
                return Err(error);
            }
        };
        pe.set_alpns(vec![ADMISSION_REQUEST_ALPN.to_vec()]);
        let owned = endpoint_socket_inodes(&ae).and_then(|author| {
            Ok(author
                .union(&endpoint_socket_inodes(&pe)?)
                .cloned()
                .collect::<BTreeSet<_>>())
        });
        let result = async {
        owned.as_ref().map_err(|error|error.to_string())?;
        let (client, server) = connect(&ae, &pe).await?;
        let (request, received) = tokio::join!(author.request_manual_admission(&client), provider.serve_manual_admission(&server));
        if !matches!(request?, ManualExchange::AwaitingApproval) { return Err("unexpected author admission phase".into()); }
        received?;
        let requests = provider.manual_requests().await?;
        let exact: Vec<_> = requests.iter().filter(|request| request.applicant == author_id).collect();
        if exact.len() != 1 { return Err("exact independent author request unavailable".into()); }
        provider.approve_manual_admission(exact[0]).await?;
        let delivery = async {
            let ManualExchange::Materials(materials) = author.request_manual_admission(&client).await? else {
                return Err("author materials absent".into());
            };
            author.complete_manual_admission(materials).await?;
            Ok::<_, Failure>(())
        };
        let (saved, delivered) = tokio::join!(delivery, provider.serve_manual_admission(&server));
        saved?; delivered?;
        client.close(0u32.into(), b"author admission complete");
        server.close(0u32.into(), b"author admission complete");
        let mut session = author.begin_management_session();
        let managed = author.manage(&mut session, ManagementAction::RemoveMember(author_id)).await;
        session.end();
        let managed = managed?;
        let record = author.management_record(managed.record).await?.ok_or("public author record absent")?;
        let decoded = p2panda_core::Header::<miopunch_native_probe::management_auth::ManagementExtension>::decode(record.header())?;
        if decoded.verifying_key != author_id || record.body().is_some() { return Err("unexpected public author record".into()); }
        let target = provider.ingest_management_record(record.header(), record.body()).await?;
        if target != managed.record || provider.status().members.iter().any(|(id, _)| *id == author_id) {
            return Err("public author removal was not verified and applied".into());
        }
        Ok::<_, Failure>(json!({"event":"offline-author", "record":target.to_string(),
            "independentIdentity":true,"author":author_id.to_string(),"source":"public NativeNetwork.manage(RemoveMember(self))",
            "serializedRecordBytes":record.header().len(),"bodyPresent":false,"bodyBytes":0,"providerVerifiedApplied":true,
            "datasetAdmission":"actual SDK over owned loopback only; no public qualification",
            "savedProviderResourceConfigurationRestored":true}))
    }.await;
        ae.close().await;
        pe.close().await;
        author.close().await;
        drop(ae);
        drop(pe);
        let owned = owned?;
        let immediate = owned.is_disjoint(&socket_inodes()?);
        tokio::time::sleep(Duration::from_millis(100)).await;
        let late = owned.is_disjoint(&socket_inodes()?);
        let mut facts = result?;
        facts["authorCleanup"] = json!({"coResidentSdkNode":true,"endpointCloseReturned":true,
        "nodeCloseReturned":true,"observedOwnedSocketCount":owned.len(),"ownedSocketsAbsent":immediate,
        "ownedSocketsAbsentLate":late,"lateDelayMs":100,"separateProcessExit":null});
        if !immediate || !late {
            return Err("independent author sockets remain".into());
        }
        Ok(facts)
    }

    #[tokio::main]
    pub(super) async fn main() -> Result<(), Failure> {
        let sockets_before = socket_inodes()?.len();
        let args: Vec<_> = std::env::args().collect();
        if args.len() < 5 || args[4] != "--allow-unqualified-preparation" {
            return Err("usage: unified_admission_story DATA_DIR FACILITY_JSON ICE_PROFILE --allow-unqualified-preparation [--relay URL] [--rtc-disabled]".into());
        }
        let mut permitted_relay = None;
        let mut rtc_enabled = true;
        let mut options = args[5..].iter();
        while let Some(option) = options.next() {
            match option.as_str() {
                "--relay" if permitted_relay.is_none() => {
                    permitted_relay = Some(
                        options
                            .next()
                            .ok_or("missing permitted relay")?
                            .parse::<iroh::RelayUrl>()?,
                    );
                }
                "--rtc-disabled" if rtc_enabled => rtc_enabled = false,
                _ => return Err("unknown or duplicate owned fixture option".into()),
            }
        }
        let node = if Path::new(&args[1]).exists() {
            NativeNetwork::open(&args[1]).await?
        } else {
            NativeNetwork::create(&args[1]).await?
        };
        let facility: MqttFacility = serde_json::from_slice(&std::fs::read(&args[2])?)?;
        let profile: RtcIceProfile = serde_json::from_slice(&std::fs::read(&args[3])?)?;
        profile.validate()?;
        let mut resources = node.resource_configuration()?;
        resources.mqtt_facilities = vec![facility.clone()];
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
            limitation: "explicit owned admission SDK fixture".into(),
        }];
        if let Some(relay) = &permitted_relay {
            if !resources.allows(relay.as_str(), ResourceUse::DataRelay) {
                // A selected resource may be new; explicit URL disables still win.
                if resources
                    .disabled
                    .contains(&(relay.to_string(), ResourceUse::DataRelay))
                {
                    return Err("explicit relay resource is disabled".into());
                }
            }
            selected.push(Resource {
                url: relay.to_string(),
                uses: [ResourceUse::DataRelay].into(),
                limitation: "explicit permitted story data relay".into(),
            });
        }
        for url in if rtc_enabled {
            profile.observation_resources()?
        } else {
            Vec::new()
        } {
            selected.push(Resource {
                url,
                uses: [ResourceUse::Observation].into(),
                limitation: "actual SDK ICE setup only".into(),
            });
        }
        resources.import_trusted_replacement(selected)?;
        resources.lan_discovery = LanDiscovery::Disabled;
        resources.port_mapping = PortMapping::Disabled;
        resources.udp_direct_disabled = true;
        resources.tcp_direct_disabled = true;
        resources.member_data_relay_disabled = true;
        if permitted_relay.is_some() {
            resources.disabled_uses.remove(&ResourceUse::DataRelay);
        } else {
            resources.disabled_uses.insert(ResourceUse::DataRelay);
        }
        if !rtc_enabled || profile.stun_servers.is_empty() {
            resources.disabled_uses.insert(ResourceUse::Observation);
        }
        node.save_resource_configuration(&resources)?;
        let traffic = Arc::new(Traffic::default());
        let transport = Arc::new(MqttTransport::from_facility(
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
        rtc.set_resource_allowed(rtc_enabled);
        let builder = Endpoint::builder(presets::Minimal).clear_ip_transports();
        let assembly = Arc::new(if rtc_enabled {
            node.bind_configured_endpoint_with_rtc_and_contact(
                builder,
                rtc.clone(),
                transport.clone(),
            )
            .await?
        } else {
            node.bind_configured_endpoint_with_contact(builder, transport.clone())
                .await?
        });
        let endpoint = assembly.endpoint().clone();
        let mut alpns = vec![
            ADMISSION_REQUEST_ALPN.to_vec(),
            miopunch_native_probe::connection_coordinator::MEMBER_ALPN.to_vec(),
            miopunch_native_probe::connection_recovery::CONNECTION_RECOVERY_ALPN.to_vec(),
        ];
        alpns.extend([
            miopunch_native_probe::admission_gate::PRECODE_REQUEST_ALPN.to_vec(),
            miopunch_native_probe::connection_coordinator::REPLICA_ALPN.to_vec(),
        ]);
        endpoint.set_alpns(alpns);
        tokio::time::timeout(Duration::from_secs(10), transport.wait_ready()).await??;
        emit(
            json!({"event":"ready","id":endpoint.id().to_string(),"route":transport.route()?,
        "invitation":hex::encode(node.manual_invitation()?.encode()),"normalPublicTrust":true,"newWireQualified":false,
        "sourceContract":"strict common purpose SDK candidate; physical qualification requires full receipt",
        "rtcEnabled":rtc_enabled,"permittedRelay":permitted_relay.map(|relay|relay.to_string())}),
        );
        let node = Arc::new(tokio::sync::Mutex::new(node));
        let precode = Arc::new(tokio::sync::Mutex::new(None));
        let mut input = BufReader::new(tokio::io::stdin()).lines();
        let mut handlers = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                line = input.next_line() => {
                    let Some(line) = line? else { break; };
                    if line == "stop" { break; }
                    let command: Value = serde_json::from_str(&line)?;
                    let operation = text(&command, "op")?;
                    let result: Result<Value, Failure> = async {
                        let mut node = node.lock().await;
                        match operation {
                            "admission-status" => {
                                let mut requests = Vec::new();
                                for request in node.manual_requests().await? {
                                    let (approved, approval_status) = match node.approved_admission(&request).await {
                                        Ok(Some(approved)) => (Some(approved), "Valid"),
                                        Ok(None) => (None, "Unapproved"),
                                        Err(miopunch_native_probe::manual_admission::ManualAdmissionError::Rejected) => (None, "Rejected"),
                                        Err(error) => return Err(error.into()),
                                    };
                                    requests.push(json!({"applicant":request.applicant.to_string(),"request":hex::encode(request.request_id),
                                        "authorization":approved.as_ref().map(|value|value.authorization.to_string()),
                                        "stage":approved.map(|value|format!("{:?}",value.stage)),"approvalStatus":approval_status}));
                                }
                                Ok(json!({"event":operation,"requests":requests,"appliedCount":node.status().applied_count}))
                            }
                            "approve-request" => {
                                let request = AdmissionRequest { applicant:text(&command,"applicant")?.parse()?,
                                    request_id:hex::decode(text(&command,"request")?)?.try_into().map_err(|_|"request length")? };
                                let before = node.status().applied_count;
                                let approved = node.approve_manual_admission(&request).await?;
                                Ok(json!({"event":"approved","applicant":request.applicant.to_string(),"request":hex::encode(request.request_id),
                                    "authorization":approved.authorization.to_string(),"before":before,"after":node.status().applied_count}))
                            }
                            "offline-author" => offline_author(&mut node, &Path::new(&args[1]).with_file_name("independent-author")).await,
                            "remove-member" => {
                                let target = text(&command,"target")?.parse()?;
                                let mut session = node.begin_management_session();
                                let result = node.manage(&mut session,ManagementAction::RemoveMember(target)).await;
                                session.end(); let result = result?;
                                Ok(json!({"event":"removed","record":result.record.to_string(),"currentAccessAbsent":result.effective_access.is_none()}))
                            }
                            "issue-precode" => {
                                let mut issued = precode.lock().await;
                                if issued.is_some() { return Err("one owned precode already issued".into()); }
                                let expires = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs()+3600;
                                let code = node.issue_precode(expires,Some(1)).await?;
                                let result = json!({"event":"issued","code":hex::encode(code.encode())});
                                *issued = Some(code); Ok(result)
                            }
                            "precode-usage" => {
                                let issued = precode.lock().await;
                                let code = issued.as_ref().ok_or("no owned precode")?;
                                let usage = node.precode_usage_status(code).await?;
                                let redemptions = node.precode_redemptions(code).await?;
                                Ok(json!({"event":"usage","uses":node.precode_usage(code).await?,"trusted":usage.count_trusted,
                                    "redemptionCount":redemptions.len(),"appliedCount":node.status().applied_count}))
                            }
                            _ => Err("unknown private operator command".into()),
                        }
                    }.await;
                    match result { Ok(value)=>emit(value), Err(error)=>emit(json!({"event":"operator-error","operation":operation,"failureClass":"SdkOperation","failure":error.to_string()})) }
                }
                incoming = endpoint.accept() => {
                    let Some(incoming) = incoming else { break; };
                    let operation_started = n0_future::time::Instant::now();
                    let deadline = operation_started+Duration::from_secs(12);
                    let incoming = incoming.with_caller_deadline(deadline);
                    let node = node.clone(); let assembly = assembly.clone(); let rtc = rtc.clone();
                    let serving = endpoint.clone(); let precode = precode.clone();
                    handlers.spawn(async move {
                        // The installed SDK admission hook owns all TLS and path
                        // validation. No parallel handshake is introduced here.
                        let connection = incoming.await?;
                        if miopunch_native_probe::connection_coordinator::SessionPurpose::from_alpn(connection.alpn())
                            == Some(miopunch_native_probe::connection_coordinator::SessionPurpose::AdmissionRequest) {
                            let gate = node.lock().await.admission_gate();
                            emit(json!({"event":"admission-accepted","alpn":String::from_utf8_lossy(connection.alpn()),
                            "stableId":connection.stable_id(),"callerDeadlinePresent":connection.caller_deadline().is_some(),
                            "callerElapsedMs":operation_started.elapsed().as_millis(),"callerBudgetMs":12000,
                            "restricted":gate.allows(&connection,ProtocolScope::AdmissionRequest),
                            "businessHandle":gate.allows(&connection,ProtocolScope::Member),"newWireQualified":false}));
                            let mut node = node.lock().await;
                            let result = if connection.alpn() == miopunch_native_probe::admission_gate::PRECODE_REQUEST_ALPN { node.serve_precode_admission(&connection).await }
                                else { node.serve_manual_admission(&connection).await };
                            let provider_elapsed = operation_started.elapsed();
                            drop(node);
                            let retirement_started = n0_future::time::Instant::now();
                            let retirement = if result.is_ok() {
                                // finish() queues the response. The consumer closes A
                                // after processing it; an immediate provider close can
                                // discard even received-but-unprocessed stream data.
                                let original_deadline = connection.caller_deadline().ok_or("owned admission deadline absent")?;
                                let cancelled = async {
                                    if let Some(cancellation) = connection.attempt_cancellation() {
                                        cancellation.cancelled().await;
                                    } else {
                                        std::future::pending::<()>().await;
                                    }
                                };
                                n0_future::time::timeout(original_deadline.saturating_duration_since(n0_future::time::Instant::now()),async {
                                    tokio::select! {
                                        biased;
                                        _ = cancelled => "Cancelled",
                                        _ = connection.closed() => "ConnectionEnded",
                                    }
                                }).await.unwrap_or("DeadlineExceeded")
                            } else { "RejectedWithoutWait" };
                            emit(json!({"event":"admission-ended","alpn":String::from_utf8_lossy(connection.alpn()),
                                "stableId":connection.stable_id(),"succeeded":result.is_ok(),
                                "providerFailureClass":result.as_ref().err().map(|error|match error {
                                    miopunch_native_probe::manual_admission::ManualAdmissionError::Rejected => "Rejected",
                                    miopunch_native_probe::manual_admission::ManualAdmissionError::PathUnavailable => "PathUnavailable",
                                    miopunch_native_probe::manual_admission::ManualAdmissionError::Store(_) => "Store",
                                    miopunch_native_probe::manual_admission::ManualAdmissionError::Node(_) => "Node",
                                }),
                                "callerElapsedMs":provider_elapsed.as_millis(),"callerBudgetMs":12000,
                                "operatorRetirementElapsedMs":retirement_started.elapsed().as_millis(),
                                "operatorRetirementReason":retirement,
                                "stage":result.as_ref().ok().map(|stage|format!("{stage:?}"))}));
                            connection.close(0u32.into(),b"owned admission ended");
                        } else if connection.alpn() == miopunch_native_probe::connection_coordinator::REPLICA_ALPN {
                            let original = connection.stable_id();
                            let gate = node.lock().await.admission_gate();
                            let member_before = gate.allows(&connection, ProtocolScope::Member);
                            let mut binding = [0;32];
                            let binding_observed = connection.export_keying_material(&mut binding,
                                b"miopunch/replica-session-observation/1", &[]).is_ok();
                            let result = node.lock().await.serve_replica_with_deadline(&connection,deadline).await;
                            let mut current = [0;32];
                            let binding_unchanged = if binding_observed {
                                Some(connection.export_keying_material(&mut current,
                                    b"miopunch/replica-session-observation/1", &[]).is_ok() && current==binding)
                            } else {None};
                            emit(json!({"event":"replica-finished","alpn":String::from_utf8_lossy(connection.alpn()),
                                "stableId":connection.stable_id(),"stableUnchanged":connection.stable_id()==original,
                                "memberAuthorizedBefore":member_before,"memberAuthorizedAfter":gate.allows(&connection,ProtocolScope::Member),
                                "originalBindingObserved":binding_observed,"bindingUnchanged":binding_unchanged,
                                "callerDeadlinePresent":connection.caller_deadline().is_some(),
                                "durableReceiptAccepted":result.is_ok(),"newWireQualified":false,
                                "callerElapsedMs":operation_started.elapsed().as_millis(),"callerBudgetMs":12000}));
                            connection.close(0u32.into(),b"owned replica ended");
                        } else if connection.alpn() == miopunch_native_probe::connection_recovery::CONNECTION_RECOVERY_ALPN {
                            node.lock().await.serve_connection_path_recovery(&assembly,&connection).await?;
                            connection.close(0u32.into(),b"owned recovery ended");
                        } else {
                            let gate = node.lock().await.admission_gate();
                            emit(json!({"event":"member-released","stableId":connection.stable_id(),
                                "remoteId":connection.remote_id().to_string(),"alpn":String::from_utf8_lossy(connection.alpn()),
                                "memberAuthorized":gate.allows(&connection,ProtocolScope::Member),
                                "newProtocol":connection.alpn()==miopunch_native_probe::connection_coordinator::MEMBER_ALPN,
                                "callerElapsedMs":operation_started.elapsed().as_millis(),"callerBudgetMs":12000,
                                "quicRole":if connection.side().is_client(){"client"}else{"server"},
                                "paths":connection.paths().iter().map(|path|json!({"isRelay":path.is_relay(),"isIp":path.is_ip(),
                                    "selected":path.is_selected(),"isSignal":matches!(path.remote_addr(),iroh::TransportAddr::Custom(address)
                                        if address.id()==miopunch_native_probe::mqtt_transport::TRANSPORT_ID)})).collect::<Vec<_>>()}));
                            let (send,recv) = connection.accept_bi().await?;
                            miopunch_native_probe::rtc_member::serve_stream(serving,connection.clone(),gate,rtc,send,recv).await?;
                            // The consumer closes M after reading the queued reply.
                            // An ordinary M outlives its completed setup deadline;
                            // closing the owned endpoint also releases this waiter.
                            connection.closed().await;
                            connection.close(0u32.into(),b"owned business ended");
                        }
                        Ok::<_, Failure>(())
                    });
                }
                Some(result) = handlers.join_next(), if !handlers.is_empty() => {
                    match result {
                        Ok(Ok(())) => {},
                        Ok(Err(error)) => emit(json!({"event":"handler-error","succeeded":false,
                            "failureClass":"SdkHandler","failure":error.to_string()})),
                        Err(error) => emit(json!({"event":"handler-error","succeeded":false,
                            "failureClass":"HandlerJoin","failure":error.to_string()})),
                    }
                }
            }
        }
        let cleanup_started = std::time::Instant::now();
        let handlers_at_close = handlers.len();
        // Closing the owned endpoint lets every handler leave through its real
        // stream/connection failure or original operation deadline.
        endpoint.close().await;
        while handlers.join_next().await.is_some() {}
        Arc::try_unwrap(assembly)
            .map_err(|_| "owned assembly still referenced")?
            .close()
            .await;
        drop(endpoint);
        tokio::time::timeout(Duration::from_secs(1), transport.wait_closed()).await??;
        let node = Arc::try_unwrap(node)
            .map_err(|_| "owned node still referenced")?
            .into_inner();
        node.close().await;
        let rtc_retirement_started = std::time::Instant::now();
        let sockets_immediate = socket_inodes()?.len();
        let immediate = json!({"activeRtcAssociations":rtc.active_associations(),"activeRtcPumps":rtc.active_pumps(),"traffic":traffic.snapshot(),"socketCount":sockets_immediate});
        while (rtc.active_associations() != 0 || rtc.active_pumps() != 0)
            && rtc_retirement_started.elapsed() < Duration::from_secs(2)
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        let sockets_late = socket_inodes()?.len();
        emit(
            json!({"event":"closed","handlersDrained":true,"handlersAtClose":handlers_at_close,"abortedHandlers":0,
        "nodeCloseReturned":true,"immediate":immediate,
        "late":{"activeRtcAssociations":rtc.active_associations(),"activeRtcPumps":rtc.active_pumps(),"traffic":traffic.snapshot(),"socketCount":sockets_late},"lateDelayMs":100,
        "activeRtcAssociations":rtc.active_associations(),"activeRtcPumps":rtc.active_pumps(),
        "socketsBefore":sockets_before,"socketsImmediate":sockets_immediate,"socketsLate":sockets_late,
        "cleanupMs":cleanup_started.elapsed().as_millis()}),
        );
        if rtc.active_associations() != 0 || rtc.active_pumps() != 0 {
            return Err("owned RTC retirement incomplete".into());
        }
        Ok(())
    }
}

#[cfg(feature = "rtc-probe")]
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    enabled::main()
}

#[cfg(not(feature = "rtc-probe"))]
fn main() {
    eprintln!("unified_admission_story requires the explicit rtc-probe build feature");
    std::process::exit(2);
}
