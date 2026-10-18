//! Two actual SDK contact-selection cases; loopback, never physical/public qualification.
use miopunch_native_probe::control_protocol::AuthenticatedControl;
use miopunch_native_probe::{
    admission_gate::{AdmissionGate, ProtocolScope},
    connection_coordinator::{AttemptContext, MEMBER_ALPN, SessionPurpose},
    contact_listener::ContactListenerPhase,
    manual_admission::ManualExchange,
    member_transport::{MemberForwarder, MemberSignalingTransport},
    native_network::NativeNetwork,
    resources::{Resource, ResourceUse},
};
use std::{
    collections::BTreeSet,
    net::Ipv4Addr,
    sync::{Arc, Mutex},
    time::Duration,
};
#[path = "support/member_contact_observation.rs"]
mod member_contact_observation;
#[path = "support/member_priority_broker.rs"]
mod member_priority_broker;
use member_contact_observation::{Events, ObservedMember};

#[derive(Debug, Default)]
struct Received {
    facts: serde_json::Value,
    binding: Option<[u8; 32]>,
}
#[derive(Debug)]
struct Echo {
    gate: AdmissionGate,
    expected: iroh::EndpointId,
    received: Arc<Mutex<Received>>,
}
impl iroh::protocol::ProtocolHandler for Echo {
    async fn accept(
        &self,
        connection: iroh::endpoint::Connection,
    ) -> Result<(), iroh::protocol::AcceptError> {
        let mut binding = [0; 32];
        let exported = connection
            .export_keying_material(&mut binding, b"miopunch/contact-priority-test/1", &[])
            .is_ok();
        *self.received.lock().unwrap() = Received {
            facts: serde_json::json!({"identityMatched":connection.remote_id()==self.expected,
                "memberGate":self.gate.allows(&connection,ProtocolScope::Member),
                "strictV1":connection.alpn()==MEMBER_ALPN,"role":"responder",
                "selectedUdp":connection.paths().iter().any(|path|path.is_selected()&&path.is_ip()),
                "bindingExported":exported}),
            binding: exported.then_some(binding),
        };
        while let Ok((send, recv)) = connection.accept_bi().await {
            if miopunch_native_probe::member_message::serve(send, recv)
                .await
                .is_err()
            {
                break;
            }
        }
        Ok(())
    }
}
#[derive(Debug)]
struct WarmControl {
    gate: AdmissionGate,
    expected: iroh::EndpointId,
    facts: Arc<Mutex<serde_json::Value>>,
    closed: Arc<tokio::sync::Notify>,
}
impl iroh::protocol::ControlProtocolHandler for WarmControl {
    async fn accept(
        &self,
        connection: iroh::endpoint::ControlConnection,
    ) -> Result<(), iroh::protocol::AcceptError> {
        *self.facts.lock().unwrap() = serde_json::json!({
            "identityMatched":connection.remote_id()==self.expected,
            "bootstrapGate":self.gate.allows(&connection,ProtocolScope::Bootstrap),
            "ordinaryMemberGate":self.gate.allows(&connection,ProtocolScope::Member),
        });
        let exchange = connection.accept_control_exchange().await?;
        let format = iroh::endpoint::ControlMessageFormat::EndTerminated { maximum: 16 };
        if exchange.recv_message(format).await? != b"WARM" {
            return Err(std::io::Error::other("unexpected owned control request").into());
        }
        exchange.send_message(format, b"WARMED").await?;
        drop(exchange);
        connection.closed().await;
        self.closed.notify_one();
        Ok(())
    }
}
async fn admit(admin: &mut NativeNetwork, directory: &std::path::Path) -> NativeNetwork {
    let mut member = NativeNetwork::join(directory, admin.manual_invitation().unwrap())
        .await
        .unwrap();
    let server = admin
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let client = member
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let server_coordinator = server.coordinator();
    let client_coordinator = client.coordinator();
    let deadline = n0_future::time::Instant::now() + Duration::from_secs(12);
    let cancellation = tokio_util::sync::CancellationToken::new();
    let (outgoing, incoming) = tokio::join!(
        client_coordinator.connect_control(
            server.addr(),
            SessionPurpose::AdmissionRequest,
            AttemptContext::new(deadline, cancellation.clone()),
        ),
        async {
            let incoming = server.endpoint().accept().await.unwrap();
            server_coordinator
                .accept_control(
                    incoming,
                    SessionPurpose::AdmissionRequest,
                    AttemptContext::new(deadline, cancellation),
                )
                .await
        }
    );
    let outgoing = outgoing.unwrap();
    let incoming = incoming.unwrap();
    let (request, served) = tokio::join!(
        member.request_manual_admission(&outgoing),
        admin.serve_manual_admission(&incoming)
    );
    assert!(matches!(request.unwrap(), ManualExchange::AwaitingApproval));
    served.unwrap();
    let request = admin.manual_requests().await.unwrap().pop().unwrap();
    admin.approve_manual_admission(&request).await.unwrap();
    let receive = async {
        let ManualExchange::Materials(material) =
            member.request_manual_admission(&outgoing).await.unwrap()
        else {
            panic!("approved materials required");
        };
        member.complete_manual_admission(material).await.unwrap();
    };
    let (_, served) = tokio::join!(receive, admin.serve_manual_admission(&incoming));
    served.unwrap();
    outgoing.close(0u32.into(), b"owned admission complete");
    incoming.close(0u32.into(), b"owned admission complete");
    client.close().await;
    server.close().await;
    member
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DataHint {
    None,
    LiveIp,
    ActiveIp,
    StaleIp,
    WarmCacheLiveIp,
}

async fn run_case(member_unavailable: bool, data_hint: DataHint) {
    const LINK_ALPN: &[u8] = b"miopunch/contact-priority-link/1";
    let directory = tempfile::tempdir().unwrap();
    let events = Events::default();
    let broker = member_priority_broker::Broker::new(events.clone()).await;
    let mut forwarder_node = NativeNetwork::create(directory.path().join("forwarder"))
        .await
        .unwrap();
    let mut config = forwarder_node.resource_configuration().unwrap();
    config.replacement = Some(vec![]);
    config.member_data_relay_disabled = true;
    forwarder_node.save_resource_configuration(&config).unwrap();
    let mut caller_node = admit(&mut forwarder_node, &directory.path().join("caller")).await;
    let receiver_node = admit(&mut forwarder_node, &directory.path().join("receiver")).await;
    for record in forwarder_node.management_history().await.unwrap() {
        caller_node
            .ingest_management_record(record.header(), record.body())
            .await
            .unwrap();
    }
    let mut relay_config = iroh_relay::server::ServerConfig::default();
    relay_config.relay = Some(iroh_relay::server::RelayConfig::new((
        Ipv4Addr::LOCALHOST,
        0,
    )));
    let relay = iroh_relay::server::Server::spawn(relay_config)
        .await
        .unwrap();
    let relay_url = relay.http_url().unwrap();
    let mut caller_config = config.clone();
    caller_config.replacement = Some(vec![Resource {
        url: format!("mqtt://{}/", broker.address),
        uses: BTreeSet::from([ResourceUse::Signaling]),
        limitation: "owned actual loopback MQTT contact, never business data".into(),
    }]);
    caller_node
        .save_resource_configuration(&caller_config)
        .unwrap();
    let mut receiver_config = caller_config.clone();
    receiver_config
        .replacement
        .as_mut()
        .unwrap()
        .push(Resource {
            url: relay_url.to_string(),
            uses: BTreeSet::from([ResourceUse::DataRelay]),
            limitation: "owned actual healthy relay while maintained contact accepts other member"
                .into(),
        });
    receiver_node
        .save_resource_configuration(&receiver_config)
        .unwrap();
    let caller_id = caller_node.status().node_id;
    let receiver_id = receiver_node.status().node_id;
    let forwarder_id = forwarder_node.status().node_id;
    assert!(caller_id != receiver_id && caller_id != forwarder_id && receiver_id != forwarder_id);
    caller_node.close().await;
    receiver_node.close().await;
    forwarder_node.close().await;
    let caller_node = NativeNetwork::open(directory.path().join("caller"))
        .await
        .unwrap();
    let receiver_node = NativeNetwork::open(directory.path().join("receiver"))
        .await
        .unwrap();
    let forwarder_node = NativeNetwork::open(directory.path().join("forwarder"))
        .await
        .unwrap();
    assert_eq!(caller_node.status().node_id, caller_id);
    assert_eq!(receiver_node.status().node_id, receiver_id);
    assert_eq!(forwarder_node.status().node_id, forwarder_id);
    // Keep the expected peer's real old endpoint bound until its maintained
    // replacement has a distinct tuple, then retire it before the attempt.
    let old_receiver = if data_hint == DataHint::StaleIp {
        Some(
            receiver_node
                .bind_endpoint(Ipv4Addr::LOCALHOST)
                .await
                .unwrap(),
        )
    } else {
        None
    };
    let old_receiver_ip = old_receiver
        .as_ref()
        .map(|endpoint| *endpoint.addr().ip_addrs().next().unwrap());
    for node in [&caller_node, &receiver_node, &forwarder_node] {
        node.admission_gate()
            .register_protocol(LINK_ALPN, ProtocolScope::Member);
    }
    let forwarder_resources = forwarder_node
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    forwarder_resources
        .endpoint()
        .set_alpns(vec![LINK_ALPN.to_vec()]);
    let caller_transport = Arc::new(MemberSignalingTransport::new(
        forwarder_resources.endpoint().id(),
        caller_node.admission_gate(),
    ));
    let receiver_transport = Arc::new(MemberSignalingTransport::new(
        forwarder_resources.endpoint().id(),
        receiver_node.admission_gate(),
    ));
    let observed = Arc::new(ObservedMember {
        real: caller_transport.clone(),
        events: events.clone(),
    });
    let caller_resources = caller_node
        .bind_configured_endpoint_with(
            iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
                .bind_addr((Ipv4Addr::LOCALHOST, 0))
                .unwrap()
                .add_custom_transport(observed),
        )
        .await
        .unwrap();
    let received = Arc::new(Mutex::new(Received::default()));
    let gate = receiver_node.admission_gate();
    let receiver_observation = received.clone();
    let expected_caller = caller_resources.endpoint().id();
    let warm_facts = Arc::new(Mutex::new(serde_json::Value::Null));
    let warm_closed = Arc::new(tokio::sync::Notify::new());
    let warm_handler = WarmControl {
        gate: gate.clone(),
        expected: expected_caller,
        facts: warm_facts.clone(),
        closed: warm_closed.clone(),
    };
    let receiver = receiver_node
        .maintain_network_with_endpoint(
            Ipv4Addr::LOCALHOST,
            |builder| builder.add_custom_transport(receiver_transport.clone()),
            |router| {
                router
                    .accept_control(
                        miopunch_native_probe::admission_gate::CONTROL_ALPN,
                        warm_handler,
                    )
                    .accept(
                        MEMBER_ALPN,
                        Echo {
                            gate,
                            expected: expected_caller,
                            received: receiver_observation,
                        },
                    )
            },
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), receiver.endpoint().online())
        .await
        .unwrap();
    assert!(receiver.addr().relay_urls().any(|url| url == &relay_url));
    assert!(caller_resources.addr().relay_urls().next().is_none());
    assert_eq!(
        receiver
            .maintenance_status()
            .contact_listener
            .unwrap()
            .phase,
        ContactListenerPhase::Ready
    );
    let live_receiver_ip = *receiver.addr().ip_addrs().next().unwrap();
    let mut actual_stale_tuple_retired = false;
    if let Some(old) = old_receiver {
        assert_eq!(old.id(), receiver.endpoint().id());
        assert_ne!(old_receiver_ip.unwrap(), live_receiver_ip);
        old.close().await;
        drop(old);
        actual_stale_tuple_retired = tokio::net::UdpSocket::bind(old_receiver_ip.unwrap())
            .await
            .is_ok();
        assert!(
            actual_stale_tuple_retired,
            "the same peer's old tuple must actually retire"
        );
    }
    let mut active_udp_control = None;
    let mut actual_active_udp_control = false;
    if matches!(data_hint, DataHint::ActiveIp | DataHint::WarmCacheLiveIp) {
        let direct_target = iroh::EndpointAddr::new(receiver.endpoint().id())
            .with_addrs([iroh::TransportAddr::Ip(live_receiver_ip)]);
        let control = tokio::time::timeout(
            Duration::from_secs(3),
            caller_resources
                .endpoint()
                .connect(direct_target, MEMBER_ALPN),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(control.remote_id(), receiver.endpoint().id());
        assert_eq!(control.alpn(), MEMBER_ALPN);
        assert!(
            caller_node
                .admission_gate()
                .allows(&control, ProtocolScope::Member)
        );
        assert!(
            control
                .paths()
                .iter()
                .any(|path| path.is_selected() && path.is_ip())
        );
        let active = caller_resources
            .endpoint()
            .remote_info(receiver.endpoint().id())
            .await
            .expect("direct control path must publish remote info");
        assert!(active.addrs().any(|entry| {
            entry.addr() == &iroh::TransportAddr::Ip(live_receiver_ip)
                && matches!(entry.usage(), iroh::endpoint::TransportAddrUsage::Active)
        }));
        active_udp_control = Some(control);
        actual_active_udp_control = true;
    }
    let forwarder = MemberForwarder::new(forwarder_node.admission_gate());
    let mut leases = vec![];
    let mut links = vec![];
    for (endpoint, transport) in [
        (caller_resources.endpoint(), &caller_transport),
        (receiver.endpoint(), &receiver_transport),
    ] {
        let (outgoing, incoming) = tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(
                endpoint.connect(forwarder_resources.addr(), LINK_ALPN),
                async { forwarder_resources.endpoint().accept().await.unwrap().await }
            )
        })
        .await
        .unwrap();
        let outgoing = outgoing.unwrap();
        let incoming = incoming.unwrap();
        let (client_lease, server_lease) = tokio::join!(
            transport.attach(&outgoing, Duration::from_secs(30)),
            forwarder.accept_link(&incoming, Duration::from_secs(30))
        );
        leases.extend([client_lease.unwrap(), server_lease.unwrap()]);
        links.extend([outgoing, incoming]);
    }
    assert_eq!(forwarder.active_links(), 2);
    let member_published = || {
        caller_resources
            .endpoint()
            .custom_addrs()
            .iter()
            .any(|route| route.id() == miopunch_native_probe::member_transport::TRANSPORT_ID)
    };
    assert!(
        member_published(),
        "real caller member registration must begin Ready"
    );
    let member_route = receiver_transport.address().unwrap();
    let mqtt_route = receiver
        .endpoint()
        .custom_addrs()
        .into_iter()
        .find(|addr| addr.id() == miopunch_native_probe::mqtt_transport::TRANSPORT_ID)
        .unwrap();
    let mut actual_warm_cached_mqtt = false;
    if data_hint == DataHint::WarmCacheLiveIp {
        let warm_target = iroh::EndpointAddr::new(receiver.endpoint().id())
            .with_addrs([iroh::TransportAddr::Custom(mqtt_route.clone())]);
        let warm = tokio::time::timeout(
            Duration::from_secs(3),
            caller_resources.endpoint().connect_control(
                warm_target,
                miopunch_native_probe::admission_gate::CONTROL_ALPN,
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(warm.remote_id(), receiver.endpoint().id());
        assert!(
            caller_node
                .admission_gate()
                .allows(&warm, ProtocolScope::Bootstrap)
        );
        assert!(
            !caller_node
                .admission_gate()
                .allows(&warm, ProtocolScope::Member)
        );
        let exchange = warm.open_control_exchange().await.unwrap();
        let format = iroh::endpoint::ControlMessageFormat::EndTerminated { maximum: 16 };
        exchange.send_message(format, b"WARM").await.unwrap();
        assert_eq!(exchange.recv_message(format).await.unwrap(), b"WARMED");
        drop(exchange);
        warm.close(0u32.into(), b"owned restricted warm session ended");
        drop(warm);
        tokio::time::timeout(Duration::from_secs(1), warm_closed.notified())
            .await
            .unwrap();
        // Kernel-backed control M ended at both peers; only its learned route
        // remains in the actual library cache. This wait is fixture observation.
        tokio::time::sleep(Duration::from_millis(100)).await;
        actual_warm_cached_mqtt = caller_resources.endpoint().remote_info(receiver.endpoint().id()).await
            .is_some_and(|info|info.addrs().any(|entry|matches!(entry.addr(),
                iroh::TransportAddr::Custom(route) if route.id()==miopunch_native_probe::mqtt_transport::TRANSPORT_ID)));
        assert!(
            actual_warm_cached_mqtt,
            "actual SDK warm session must populate the contact cache"
        );
        let facts = warm_facts.lock().unwrap();
        assert_eq!(facts["identityMatched"], true);
        assert_eq!(facts["bootstrapGate"], true);
        assert_eq!(facts["ordinaryMemberGate"], false);
    }
    let mut target = iroh::EndpointAddr::new(receiver.endpoint().id()).with_addrs([
        iroh::TransportAddr::Custom(member_route),
        iroh::TransportAddr::Custom(mqtt_route),
    ]);
    if data_hint != DataHint::None {
        target.addrs.insert(iroh::TransportAddr::Ip(
            if matches!(
                data_hint,
                DataHint::LiveIp | DataHint::ActiveIp | DataHint::WarmCacheLiveIp
            ) {
                live_receiver_ip
            } else {
                old_receiver_ip.unwrap()
            },
        ));
    }
    let coordinator = caller_resources.coordinator();
    let mut actual_member_link_retired = false;
    if member_unavailable {
        links[0].close(0u32.into(), b"owned unavailable member contact");
        links[1].close(0u32.into(), b"owned unavailable member contact");
        tokio::time::timeout(Duration::from_secs(1), async {
            while !leases[0].is_finished()
                || !leases[1].is_finished()
                || forwarder.active_links() != 1
                || member_published()
            {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        actual_member_link_retired = true;
    }
    let actual_member_registration_withdrawn = !member_published();
    let activity = receiver.business_activity();
    let deadline = n0_future::time::Instant::now() + Duration::from_secs(12);
    let cancel = tokio_util::sync::CancellationToken::new();
    let attempt = AttemptContext::new(deadline, cancel.clone());
    events.begin();
    println!("CONTACT_PRIORITY_ATTEMPT_BEGIN");
    let started = tokio::time::Instant::now();
    let result = coordinator
        .connect(target, SessionPurpose::MemberBusiness, attempt)
        .await;
    // This read-only coordinator owns an Endpoint clone. Retire it with the
    // fixture's other handles before observing kernel socket reclamation.
    drop(coordinator);
    let caller_ms = started.elapsed().as_millis();
    let mut facts = serde_json::json!({"ordinaryHandle":result.is_ok(),"deadlineMs":12000,
        "singleCoordinatorCall":true,"cancellationUnchanged":!cancel.is_cancelled(),
        "callerMs":caller_ms,"actualMemberLinkRetired":actual_member_link_retired,
        "actualMemberRegistrationWithdrawn":actual_member_registration_withdrawn,
        "initialForwarderLinks":2,"remainingForwarderLinks":forwarder.active_links(),
        "independentSavedReopenedMembers":3,"callerHasRelay":false,
        "receiverRelayStillOnline":receiver.addr().relay_urls().any(|url|url==&relay_url),
        "receiverListenerStillReady":receiver.maintenance_status().contact_listener.as_ref().is_some_and(|value|value.phase==ContactListenerPhase::Ready),
        "memberUnavailableCase":member_unavailable});
    facts["dataHintKind"] = serde_json::json!(format!("{data_hint:?}"));
    facts["actualStaleTupleRetiredBeforeAttempt"] = serde_json::json!(actual_stale_tuple_retired);
    facts["actualActiveUdpControl"] = serde_json::json!(actual_active_udp_control);
    facts["actualRestrictedWarmCachedMqtt"] = serde_json::json!(actual_warm_cached_mqtt);
    if let Ok(connection) = &result {
        facts["identityMatched"] =
            serde_json::json!(connection.remote_id().as_bytes() == receiver_id.as_bytes());
        facts["memberGate"] = serde_json::json!(
            caller_node
                .admission_gate()
                .allows(connection, ProtocolScope::Member)
        );
        facts["strictV1"] = serde_json::json!(connection.alpn() == MEMBER_ALPN);
        facts["role"] = serde_json::json!("initiator");
        facts["selectedUdp"] = serde_json::json!(
            connection
                .paths()
                .iter()
                .any(|path| path.is_selected() && path.is_ip())
        );
        facts["originalAbsoluteDeadlinePreserved"] =
            serde_json::json!(connection.caller_deadline() == Some(deadline));
        let mut before = [0; 32];
        let before_ok = connection
            .export_keying_material(&mut before, b"miopunch/contact-priority-test/1", &[])
            .is_ok();
        let business = tokio::time::timeout(Duration::from_secs(1), async {
            let (mut send, mut recv) = connection.open_bi().await.map_err(std::io::Error::other)?;
            send.write_all(b"PING")
                .await
                .map_err(std::io::Error::other)?;
            send.finish().map_err(std::io::Error::other)?;
            Ok::<_, std::io::Error>(
                recv.read_to_end(64).await.map_err(std::io::Error::other)? == b"PONG",
            )
        })
        .await;
        facts["bidirectionalStreamMatched"] = serde_json::json!(matches!(business, Ok(Ok(true))));
        let mut after = [0; 32];
        facts["sameOriginalM"] = serde_json::json!(
            before_ok
                && connection
                    .export_keying_material(&mut after, b"miopunch/contact-priority-test/1", &[])
                    .is_ok()
                && before == after
        );
        let receiver = received.lock().unwrap();
        facts["bothOriginalBindingsMatched"] =
            serde_json::json!(before_ok && receiver.binding == Some(before));
        facts["responder"] = receiver.facts.clone();
    } else {
        facts["failureClass"] = serde_json::json!(match &result {
            Err(miopunch_native_probe::connection_coordinator::ConnectError::DeadlineExceeded) =>
                "DeadlineExceeded",
            Err(
                miopunch_native_probe::connection_coordinator::ConnectError::DataPathUnavailable,
            ) => "DataPathUnavailable",
            Err(miopunch_native_probe::connection_coordinator::ConnectError::Cancelled) =>
                "Cancelled",
            _ => "OtherConnectError",
        });
    }
    let contact_events = events.snapshot();
    facts["contactEvents"] = serde_json::json!(contact_events);
    let mqtt_packets = contact_events
        .iter()
        .filter(|event| event["kind"] == "mqttPublish")
        .count();
    let member_packets = contact_events
        .iter()
        .filter(|event| event["kind"] == "memberSend")
        .count();
    let member_unavailable_observed = contact_events
        .iter()
        .any(|event| event["kind"] == "memberSend" && event["outcome"] == "BrokenPipe");
    facts["mqttQuicPublishPackets"] = serde_json::json!(mqtt_packets);
    facts["memberQuicSendCalls"] = serde_json::json!(member_packets);
    facts["actualMemberUnavailableObserved"] = serde_json::json!(member_unavailable_observed);
    let addresses = [
        caller_resources.endpoint(),
        receiver.endpoint(),
        forwarder_resources.endpoint(),
    ]
    .map(|endpoint| *endpoint.addr().ip_addrs().next().unwrap());
    let updates = receiver.watch_maintenance();
    let cleanup = tokio::time::Instant::now();
    if let Ok(connection) = result {
        connection.close(0u32.into(), b"owned contact selection case ended");
    }
    if let Some(control) = active_udp_control {
        control.close(0u32.into(), b"owned active UDP priority control ended");
    }
    for link in &links {
        link.close(0u32.into(), b"owned member links ended");
    }
    drop(links);
    drop(activity);
    receiver.close().await;
    caller_resources.close().await;
    forwarder_resources.close().await;
    caller_node.close().await;
    forwarder_node.close().await;
    let leases_ended = tokio::time::timeout(Duration::from_secs(1), async {
        while leases.iter().any(|lease| !lease.is_finished()) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .is_ok();
    drop(leases);
    let mut immediate = vec![];
    for address in addresses {
        immediate.push(tokio::net::UdpSocket::bind(address).await.is_ok());
    }
    let mqtt_retirement = broker.retired().await;
    let mut late = vec![];
    for address in addresses {
        late.push(tokio::net::UdpSocket::bind(address).await.is_ok());
    }
    facts["cleanupMs"] = serde_json::json!(cleanup.elapsed().as_millis());
    facts["udpImmediateEmpty"] = serde_json::json!(immediate);
    facts["udpLate100MsEmpty"] = serde_json::json!(late);
    facts["forwardingLeasesEnded"] = serde_json::json!(leases_ended);
    facts["forwardingLinksRemaining"] = serde_json::json!(forwarder.active_links());
    facts["mqttRetirement"] = mqtt_retirement;
    facts["listenerStopped"] = serde_json::json!(
        updates
            .borrow()
            .contact_listener
            .as_ref()
            .is_some_and(|value| value.phase == ContactListenerPhase::Stopped)
    );
    broker.close().await;
    relay.shutdown().await.unwrap();
    println!("CONTACT_PRIORITY_RECEIPT {facts}");
    assert!(
        facts["ordinaryHandle"] == true,
        "actual SDK member connection failed; receipt retained"
    );
    for key in [
        "identityMatched",
        "memberGate",
        "strictV1",
        "selectedUdp",
        "originalAbsoluteDeadlinePreserved",
        "bidirectionalStreamMatched",
        "sameOriginalM",
        "bothOriginalBindingsMatched",
        "receiverRelayStillOnline",
        "receiverListenerStillReady",
        "forwardingLeasesEnded",
        "listenerStopped",
    ] {
        assert_eq!(facts[key], true, "{key} failed; actual receipt retained");
    }
    for key in ["identityMatched", "memberGate", "strictV1", "selectedUdp"] {
        assert_eq!(facts["responder"][key], true, "responder {key} failed");
    }
    assert_eq!(facts["forwardingLinksRemaining"], 0);
    assert_eq!(facts["mqttRetirement"]["lateActiveSockets"], 0);
    assert_eq!(facts["mqttRetirement"]["lateSubscriptions"], 0);
    assert!(late.iter().all(|value| *value));
    if member_unavailable {
        assert!(
            actual_member_registration_withdrawn || member_unavailable_observed,
            "neither actual withdrawn registration nor unavailable sender observed"
        );
        assert!(
            mqtt_packets > 0,
            "no actual configured MQTT fallback datagrams observed"
        );
        if actual_member_registration_withdrawn {
            assert_eq!(
                member_packets, 0,
                "a known withdrawn contact must not start another dial"
            );
        } else {
            let first_member_error = contact_events
                .iter()
                .position(|event| event["kind"] == "memberSend" && event["outcome"] == "BrokenPipe")
                .unwrap();
            let first_mqtt = contact_events
                .iter()
                .position(|event| event["kind"] == "mqttPublish")
                .unwrap();
            assert!(
                first_member_error < first_mqtt,
                "MQTT started before actual member unavailability"
            );
        }
    } else if matches!(data_hint, DataHint::ActiveIp | DataHint::WarmCacheLiveIp) {
        assert_eq!(
            member_packets, 0,
            "live data must carry the attempt before member contact"
        );
        assert_eq!(
            mqtt_packets, 0,
            "live data must carry the attempt before configured MQTT contact"
        );
    } else {
        assert!(
            member_packets > 0,
            "no actual member contact attempt observed"
        );
        assert_eq!(
            mqtt_packets, 0,
            "available member contact must precede MQTT; real QUIC publishes were concurrent"
        );
    }
    if data_hint == DataHint::ActiveIp {
        assert_eq!(facts["actualActiveUdpControl"], true);
    }
}

#[tokio::test]
async fn available_member_contact_precedes_ready_configured_mqtt() {
    run_case(false, DataHint::None).await;
}
#[tokio::test]
async fn unavailable_member_contact_falls_back_with_original_deadline() {
    run_case(true, DataHint::None).await;
}

#[tokio::test]
async fn stale_ip_hint_still_uses_healthy_member_before_configured_mqtt() {
    run_case(false, DataHint::StaleIp).await;
}
#[tokio::test]
async fn announced_ip_without_live_provenance_uses_member_before_configured_mqtt() {
    run_case(false, DataHint::LiveIp).await;
}

#[tokio::test]
async fn active_udp_ip_data_precedes_member_and_configured_mqtt_contact() {
    run_case(false, DataHint::ActiveIp).await;
}

#[tokio::test]
async fn live_ip_bootstrap_ignores_actual_cached_mqtt_control_route() {
    run_case(false, DataHint::WarmCacheLiveIp).await;
}
