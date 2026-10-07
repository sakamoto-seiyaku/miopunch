//! Controlled SDK startup through the actual NodeMaintenance and Router owner.
//! These owned loopback cases do not qualify public facilities or RTC data.
use miopunch_native_probe::{
    admission_gate::{AdmissionGate, CONTROL_ALPN, ProtocolScope},
    connection_coordinator::{AttemptContext, MEMBER_ALPN, SessionPurpose},
    contact_listener::ContactListenerPhase,
    manual_admission::ManualExchange,
    mqtt_datagram::MqttFacility,
    mqtt_transport::{MqttTransport, Traffic},
    native_network::NativeNetwork,
    network_maintenance::{MaintainedNetwork, MaintenanceState},
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

#[derive(Debug)]
struct Control {
    gate: AdmissionGate,
    expected: iroh::EndpointId,
    observed: Arc<Mutex<serde_json::Value>>,
    closed: Arc<tokio::sync::Notify>,
}
impl iroh::protocol::ProtocolHandler for Control {
    async fn accept(
        &self,
        connection: iroh::endpoint::Connection,
    ) -> Result<(), iroh::protocol::AcceptError> {
        *self.observed.lock().unwrap() = serde_json::json!({
            "identityMatched":connection.remote_id()==self.expected,
            "bootstrapGate":self.gate.allows(&connection, ProtocolScope::Bootstrap),
            "ordinaryMemberGate":self.gate.allows(&connection, ProtocolScope::Member),
        });
        let (mut send, mut recv) = connection
            .accept_bi()
            .await
            .map_err(iroh::protocol::AcceptError::from_err)?;
        if recv
            .read_to_end(16)
            .await
            .map_err(iroh::protocol::AcceptError::from_err)?
            != b"CONTROL"
        {
            return Err(std::io::Error::other("unexpected owned control request").into());
        }
        send.write_all(b"CONTROL-ACK")
            .await
            .map_err(iroh::protocol::AcceptError::from_err)?;
        send.finish()?;
        drop(send);
        drop(recv);
        connection.closed().await;
        self.closed.notify_one();
        Ok(())
    }
}
#[derive(Debug)]
struct Echo {
    gate: AdmissionGate,
    expected: iroh::EndpointId,
    observed: Arc<Mutex<serde_json::Value>>,
}
impl iroh::protocol::ProtocolHandler for Echo {
    async fn accept(
        &self,
        connection: iroh::endpoint::Connection,
    ) -> Result<(), iroh::protocol::AcceptError> {
        *self.observed.lock().unwrap() = serde_json::json!({
            "identityMatched":connection.remote_id()==self.expected,
            "memberGate":self.gate.allows(&connection, ProtocolScope::Member),
            "strictV1":connection.alpn()==MEMBER_ALPN,
        });
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

async fn admit(admin: &mut NativeNetwork, directory: &std::path::Path) -> NativeNetwork {
    let mut peer = NativeNetwork::join(directory, admin.manual_invitation().unwrap())
        .await
        .unwrap();
    let server = admin
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let client = peer
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let server_coordinator = server.coordinator();
    let client_coordinator = client.coordinator();
    let deadline = n0_future::time::Instant::now() + Duration::from_secs(12);
    let cancellation = tokio_util::sync::CancellationToken::new();
    let (outgoing, incoming) = tokio::join!(
        client_coordinator.connect(
            server.addr(),
            SessionPurpose::AdmissionRequest,
            AttemptContext::new(deadline, cancellation.clone()),
        ),
        async {
            let incoming = server.endpoint().accept().await.unwrap();
            server_coordinator
                .accept(
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
        peer.request_manual_admission(&outgoing),
        admin.serve_manual_admission(&incoming)
    );
    assert!(matches!(request.unwrap(), ManualExchange::AwaitingApproval));
    served.unwrap();
    let request = admin.manual_requests().await.unwrap().pop().unwrap();
    admin.approve_manual_admission(&request).await.unwrap();
    let receive = async {
        let ManualExchange::Materials(material) =
            peer.request_manual_admission(&outgoing).await.unwrap()
        else {
            panic!("approved materials required")
        };
        peer.complete_manual_admission(material).await.unwrap();
    };
    let (_, served) = tokio::join!(receive, admin.serve_manual_admission(&incoming));
    served.unwrap();
    outgoing.close(0u32.into(), b"owned admission ended");
    incoming.close(0u32.into(), b"owned admission ended");
    drop(outgoing);
    drop(incoming);
    client.close().await;
    server.close().await;
    peer
}

async fn start(
    node: NativeNetwork,
    contact: Arc<MqttTransport>,
    reserved: tokio::net::UdpSocket,
    expected: iroh::EndpointId,
    control_facts: Arc<Mutex<serde_json::Value>>,
    member_facts: Arc<Mutex<serde_json::Value>>,
    control_closed: Arc<tokio::sync::Notify>,
    #[cfg(feature = "rtc-probe")] rtc: Option<miopunch_native_probe::rtc_transport::RtcTransport>,
) -> MaintainedNetwork {
    let gate = node.admission_gate();
    let bind = reserved.local_addr().unwrap();
    let configure = move |builder: iroh::endpoint::Builder| {
        drop(reserved);
        builder.clear_ip_transports().bind_addr(bind).unwrap()
    };
    let routes = move |router: iroh::protocol::RouterBuilder| {
        router
            .accept(
                CONTROL_ALPN,
                Control {
                    gate: gate.clone(),
                    expected,
                    observed: control_facts,
                    closed: control_closed,
                },
            )
            .accept(
                MEMBER_ALPN,
                Echo {
                    gate,
                    expected,
                    observed: member_facts,
                },
            )
    };
    #[cfg(feature = "rtc-probe")]
    let node = if let Some(rtc) = rtc {
        node.maintain_network_with_rtc_and_contact(
            Ipv4Addr::LOCALHOST,
            rtc,
            contact,
            configure,
            routes,
        )
        .await
        .unwrap()
    } else {
        node.maintain_network_with_contact(Ipv4Addr::LOCALHOST, contact, configure, routes)
            .await
            .unwrap()
    };
    #[cfg(not(feature = "rtc-probe"))]
    let node = node
        .maintain_network_with_contact(Ipv4Addr::LOCALHOST, contact, configure, routes)
        .await
        .unwrap();
    assert!(
        node.addr().ip_addrs().any(|value| *value == bind),
        "actual configured socket must belong to the maintained endpoint"
    );
    node
}

async fn startup_story(with_rtc: bool, disable_contact: bool) {
    let directory = tempfile::tempdir().unwrap();
    let broker =
        member_priority_broker::Broker::new(member_contact_observation::Events::default()).await;
    let mut receiver = NativeNetwork::create(directory.path().join("receiver"))
        .await
        .unwrap();
    let mut config = receiver.resource_configuration().unwrap();
    config.replacement = Some(vec![]);
    receiver.save_resource_configuration(&config).unwrap();
    let caller = admit(&mut receiver, &directory.path().join("caller")).await;
    config.replacement = Some(vec![Resource {
        url: format!("mqtt://{}/", broker.address),
        uses: BTreeSet::from([ResourceUse::Signaling]),
        limitation: "owned maintained startup fixture; no public qualification".into(),
    }]);
    let facility = MqttFacility::new(
        "owned-live-disable".into(),
        format!("mqtt://{}/", broker.address),
        "wss://127.0.0.1:443/owned-live-disable".into(),
    )
    .unwrap();
    if disable_contact {
        config.mqtt_facilities = vec![facility.clone()];
    }
    for node in [&receiver, &caller] {
        node.save_resource_configuration(&config).unwrap();
    }
    let receiver_id = receiver.status().node_id;
    let caller_id = caller.status().node_id;
    receiver.close().await;
    caller.close().await;
    let receiver = NativeNetwork::open(directory.path().join("receiver"))
        .await
        .unwrap();
    let caller = NativeNetwork::open(directory.path().join("caller"))
        .await
        .unwrap();
    assert_eq!(receiver.status().node_id, receiver_id);
    assert_eq!(caller.status().node_id, caller_id);
    let caller_gate = caller.admission_gate();
    let contact = || {
        Arc::new(if disable_contact {
            MqttTransport::from_facility(&facility, &[], Arc::new(Traffic::default()), None)
                .unwrap()
        } else {
            MqttTransport::new(broker.address, Arc::new(Traffic::default()))
        })
    };
    let receiver_contact = contact();
    let caller_contact = contact();
    let control_facts = Arc::new(Mutex::new(serde_json::Value::Null));
    let member_facts = Arc::new(Mutex::new(serde_json::Value::Null));
    let control_closed = Arc::new(tokio::sync::Notify::new());
    #[cfg(feature = "rtc-probe")]
    let rtc = || with_rtc.then(miopunch_native_probe::rtc_transport::RtcTransport::new);
    let receiver = start(
        receiver,
        receiver_contact.clone(),
        tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap(),
        iroh::EndpointId::from_bytes(caller_id.as_bytes()).unwrap(),
        control_facts.clone(),
        member_facts.clone(),
        control_closed.clone(),
        #[cfg(feature = "rtc-probe")]
        rtc(),
    )
    .await;
    let caller = start(
        caller,
        caller_contact.clone(),
        tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap(),
        receiver.endpoint().id(),
        Arc::default(),
        Arc::default(),
        Arc::new(tokio::sync::Notify::new()),
        #[cfg(feature = "rtc-probe")]
        rtc(),
    )
    .await;
    for node in [&receiver, &caller] {
        let status = node.maintenance_status().contact_listener.unwrap();
        assert_eq!(status.phase, ContactListenerPhase::Ready);
        assert!(status.generation > 0 && status.ready_routes > 0);
        let observed = node.contact_listener_traffic().unwrap();
        assert_eq!(observed["driver_active"], 1);
        assert_eq!(observed["protocol"]["connack"], 0);
        assert_eq!(observed["protocol"]["suback_codes"], serde_json::json!([0]));
    }
    let receiver_ready = receiver.maintenance_status().contact_listener.unwrap();
    let route = receiver
        .endpoint()
        .custom_addrs()
        .into_iter()
        .find(|route| route.id() == miopunch_native_probe::mqtt_transport::TRANSPORT_ID)
        .unwrap();
    let control = tokio::time::timeout(
        Duration::from_secs(3),
        caller.endpoint().connect(
            iroh::EndpointAddr::new(receiver.endpoint().id())
                .with_addrs([iroh::TransportAddr::Custom(route)]),
            CONTROL_ALPN,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(caller_gate.allows(&control, ProtocolScope::Bootstrap));
    assert!(!caller_gate.allows(&control, ProtocolScope::Member));
    let (mut send, mut recv) = control.open_bi().await.unwrap();
    send.write_all(b"CONTROL").await.unwrap();
    send.finish().unwrap();
    assert_eq!(recv.read_to_end(16).await.unwrap(), b"CONTROL-ACK");
    drop(send);
    drop(recv);
    control.close(0u32.into(), b"owned restricted control ended");
    drop(control);
    tokio::time::timeout(Duration::from_secs(1), control_closed.notified())
        .await
        .unwrap();
    assert_eq!(
        *control_facts.lock().unwrap(),
        serde_json::json!({"identityMatched":true,"bootstrapGate":true,"ordinaryMemberGate":false})
    );
    let caller_activity = caller.business_activity();
    let receiver_activity = receiver.business_activity();
    let coordinator = caller.coordinator();
    let cancel = tokio_util::sync::CancellationToken::new();
    let deadline = n0_future::time::Instant::now() + Duration::from_secs(12);
    let started = tokio::time::Instant::now();
    let member = coordinator
        .connect(
            receiver.addr(),
            SessionPurpose::MemberBusiness,
            AttemptContext::new(deadline, cancel.clone()),
        )
        .await
        .unwrap();
    drop(coordinator);
    let caller_ms = started.elapsed().as_millis();
    assert_eq!(member.remote_id(), receiver.endpoint().id());
    assert!(caller_gate.allows(&member, ProtocolScope::Member));
    let (mut send, mut recv) = member.open_bi().await.unwrap();
    send.write_all(b"PING").await.unwrap();
    send.finish().unwrap();
    assert_eq!(recv.read_to_end(64).await.unwrap(), b"PONG");
    drop(send);
    drop(recv);
    assert_eq!(
        *member_facts.lock().unwrap(),
        serde_json::json!({"identityMatched":true,"memberGate":true,"strictV1":true})
    );
    assert!(!cancel.is_cancelled());
    assert_eq!(
        caller.maintenance_status().state,
        MaintenanceState::BusinessPriority
    );
    let mut disable_facts = serde_json::Value::Null;
    if disable_contact {
        let stable = member.stable_id();
        let mut original_binding = [0; 32];
        member
            .export_keying_material(
                &mut original_binding,
                b"miopunch/test/disable-continuity/1",
                &[],
            )
            .unwrap();
        let before = receiver.contact_listener_traffic().unwrap();
        // Corrupt only the saved facility mapping to ensure the running carrier
        // cannot borrow permission from a native URL with a different WSS entry.
        let saved_path = directory.path().join("receiver/resources.json");
        let mut mismatched = config.clone();
        mismatched.mqtt_facilities[0].wss_url = "wss://127.0.0.1:443/other".into();
        std::fs::write(&saved_path, serde_json::to_vec(&mismatched).unwrap()).unwrap();
        assert!(receiver.disable_contact_signaling().await.is_err());
        assert_eq!(
            receiver
                .maintenance_status()
                .contact_listener
                .unwrap()
                .phase,
            ContactListenerPhase::Ready
        );
        assert_eq!(
            receiver.contact_listener_traffic().unwrap()["driver_active"],
            1
        );
        assert_eq!(
            serde_json::from_slice::<miopunch_native_probe::resources::ResourceConfig>(
                &std::fs::read(&saved_path).unwrap(),
            )
            .unwrap(),
            mismatched
        );
        std::fs::write(&saved_path, serde_json::to_vec(&config).unwrap()).unwrap();
        let disabled_at = tokio::time::Instant::now();
        receiver.disable_contact_signaling().await.unwrap();
        let disabled_ms = disabled_at.elapsed().as_millis();
        let stopped = receiver.maintenance_status().contact_listener.unwrap();
        assert_eq!(stopped.phase, ContactListenerPhase::Stopped);
        assert_eq!(stopped.ready_routes, 0);
        assert_eq!(stopped.generation, receiver_ready.generation);
        assert!(
            !receiver
                .endpoint()
                .custom_addrs()
                .iter()
                .any(|route| { route.id() == miopunch_native_probe::mqtt_transport::TRANSPORT_ID })
        );
        let after = receiver.contact_listener_traffic().unwrap();
        assert_eq!(after["driver_active"], 0);
        assert_eq!(after["queued_packets"], 0);
        // A second completed owner command is a FIFO barrier after the network
        // events, rather than an arbitrary delay or a private listener call.
        receiver.network_changed().await.unwrap();
        receiver.network_changed().await.unwrap();
        receiver.disable_contact_signaling().await.unwrap();
        let settled = receiver.maintenance_status().contact_listener.unwrap();
        assert_eq!(settled.phase, ContactListenerPhase::Stopped);
        assert_eq!(settled.starts, stopped.starts);
        assert_eq!(settled.ready_routes, 0);
        assert!(
            !receiver
                .endpoint()
                .custom_addrs()
                .iter()
                .any(|route| { route.id() == miopunch_native_probe::mqtt_transport::TRANSPORT_ID })
        );
        tokio::time::timeout(Duration::from_secs(1), async {
            while broker.active.load(std::sync::atomic::Ordering::SeqCst) != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        // Retain the original admitted M and its real exporter. Signaling-only
        // retirement must leave its independently permitted native data usable.
        let (mut send, mut recv) = member.open_bi().await.unwrap();
        send.write_all(b"PING").await.unwrap();
        send.finish().unwrap();
        let after_disable_pong = recv.read_to_end(64).await.unwrap() == b"PONG";
        assert!(after_disable_pong);
        let mut current_binding = [0; 32];
        member
            .export_keying_material(
                &mut current_binding,
                b"miopunch/test/disable-continuity/1",
                &[],
            )
            .unwrap();
        assert_eq!(member.stable_id(), stable);
        assert_eq!(current_binding, original_binding);
        assert!(caller_gate.allows(&member, ProtocolScope::Member));
        config
            .disabled
            .insert((facility.native_url.clone(), ResourceUse::Signaling));
        config
            .disabled
            .insert((facility.wss_url.clone(), ResourceUse::Signaling));
        disable_facts = serde_json::json!({
            "before":before,"after":after,"stopped":stopped,"afterNetworkEvents":settled,
            "completedMs":disabled_ms,"remainingBrokerSockets":broker.active.load(std::sync::atomic::Ordering::SeqCst),
            "sameM":member.stable_id()==stable,"bindingMatched":current_binding==original_binding,
            "memberGate":caller_gate.allows(&member, ProtocolScope::Member),"sameMPong":after_disable_pong,
        });
    }
    let bound = [
        *receiver.addr().ip_addrs().next().unwrap(),
        *caller.addr().ip_addrs().next().unwrap(),
    ];
    let receiver_updates = receiver.watch_maintenance();
    let caller_updates = caller.watch_maintenance();
    member.close(0u32.into(), b"owned maintained story ended");
    drop(member);
    drop(caller_activity);
    drop(receiver_activity);
    let cleanup = tokio::time::Instant::now();
    caller.close().await;
    receiver.close().await;
    for updates in [&receiver_updates, &caller_updates] {
        assert_eq!(updates.borrow().state, MaintenanceState::Stopped);
        let status = updates.borrow().contact_listener.clone().unwrap();
        assert_eq!(status.phase, ContactListenerPhase::Stopped);
        assert_eq!(status.ready_routes, 0);
    }
    for contact in [&receiver_contact, &caller_contact] {
        contact.wait_driver_closed().await.unwrap();
        assert_eq!(contact.traffic().snapshot()["driver_active"], 0);
        assert_eq!(contact.traffic().snapshot()["queued_packets"], 0);
    }
    let immediate: Vec<_> = bound
        .iter()
        .map(|addr| std::net::UdpSocket::bind(addr).is_ok())
        .collect();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let late: Vec<_> = bound
        .iter()
        .map(|addr| std::net::UdpSocket::bind(addr).is_ok())
        .collect();
    assert!(immediate.iter().chain(&late).all(|value| *value));
    let mqtt = broker.retired().await;
    assert_eq!(mqtt["lateActiveSockets"], 0);
    assert_eq!(mqtt["lateSubscriptions"], 0);
    let receiver_reopened = NativeNetwork::open(directory.path().join("receiver"))
        .await
        .unwrap();
    assert_eq!(receiver_reopened.status().node_id, receiver_id);
    assert_eq!(receiver_reopened.resource_configuration().unwrap(), config);
    if disable_contact {
        let reopened = receiver_reopened
            .maintain_network(Ipv4Addr::LOCALHOST)
            .await
            .unwrap();
        assert!(reopened.maintenance_status().contact_listener.is_none());
        assert!(
            reopened
                .endpoint()
                .custom_addrs()
                .iter()
                .all(|route| { route.id() != miopunch_native_probe::mqtt_transport::TRANSPORT_ID })
        );
        assert!(reopened.disable_contact_signaling().await.is_err());
        reopened.close().await;
        let fresh_traffic = Arc::new(Traffic::default());
        let fresh_contact = Arc::new(
            MqttTransport::from_facility(&facility, &[], fresh_traffic.clone(), None).unwrap(),
        );
        let reopened = NativeNetwork::open(directory.path().join("receiver"))
            .await
            .unwrap();
        assert!(
            reopened
                .maintain_network_with_contact(
                    Ipv4Addr::LOCALHOST,
                    fresh_contact,
                    |builder| builder,
                    |router| router,
                )
                .await
                .is_err()
        );
        assert_eq!(fresh_traffic.snapshot()["generation"], 0);
        assert_eq!(fresh_traffic.snapshot()["driver_active"], 0);
        assert_eq!(fresh_traffic.snapshot()["queued_packets"], 0);
        let no_sockets = broker.retired().await;
        assert_eq!(no_sockets["lateActiveSockets"], 0);
        assert_eq!(no_sockets["lateSubscriptions"], 0);
        disable_facts["reopenedExplicitTraffic"] = fresh_traffic.snapshot();
        disable_facts["reopenedBrokerRetirement"] = no_sockets;
    } else {
        receiver_reopened.close().await;
    }
    println!(
        "MAINTAINED_STARTUP_RECEIPT {}",
        serde_json::json!({
            "actualNodeMaintenanceOwner":true,"sameConfiguredRouter":true,"independentSavedReopenedMembers":2,
            "canonicalContactObserver":true,"receiverListener":receiver_ready,"controlFacts":*control_facts.lock().unwrap(),
            "memberFacts":*member_facts.lock().unwrap(),"ordinaryHandle":true,"streamMatched":true,"callerMs":caller_ms,
            "savedPolicyUnchanged":!disable_contact,"liveDisable":disable_facts,"foregroundBudgetProjection":false,"optionalRtcInstalled":with_rtc,"rtcDataQualified":false,
            "cleanupMs":cleanup.elapsed().as_millis(),"udpImmediateEmpty":immediate,"udpLate100MsEmpty":late,
            "mqttRetirement":mqtt,"listenerStopped":true,"driverActive":0,"queuedPackets":0,
        })
    );
    broker.close().await;
}

#[tokio::test]
async fn canonical_contact_and_routes_belong_to_real_maintained_members() {
    startup_story(false, false).await;
}

#[tokio::test]
async fn node_owned_disable_retires_ready_contact_and_preserves_same_member_session() {
    startup_story(false, true).await;
}

#[cfg(feature = "rtc-probe")]
#[tokio::test]
async fn optional_rtc_composes_with_same_maintained_contact_and_router() {
    startup_story(true, false).await;
}

#[tokio::test]
async fn maintained_contact_override_preserves_disabled_and_full_facility_checks() {
    let directory = tempfile::tempdir().unwrap();
    let broker =
        member_priority_broker::Broker::new(member_contact_observation::Events::default()).await;
    for mismatch in [false, true] {
        let node = NativeNetwork::create(directory.path().join(if mismatch {
            "mapping"
        } else {
            "disabled"
        }))
        .await
        .unwrap();
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(vec![Resource {
            url: format!("mqtt://{}/", broker.address),
            uses: BTreeSet::from([ResourceUse::Signaling]),
            limitation: "owned denied contact fixture".into(),
        }]);
        let traffic = Arc::new(Traffic::default());
        let contact = if mismatch {
            let saved = MqttFacility::new(
                "owned-map".into(),
                format!("mqtt://{}/", broker.address),
                "wss://127.0.0.1:443/saved".into(),
            )
            .unwrap();
            let other = MqttFacility::new(
                saved.logical_id.clone(),
                saved.native_url.clone(),
                "wss://127.0.0.1:443/other".into(),
            )
            .unwrap();
            config.mqtt_facilities = vec![saved];
            Arc::new(MqttTransport::from_facility(&other, &[], traffic.clone(), None).unwrap())
        } else {
            config.disabled_uses.insert(ResourceUse::Signaling);
            Arc::new(MqttTransport::new(broker.address, traffic.clone()))
        };
        node.save_resource_configuration(&config).unwrap();
        let result = node
            .maintain_network_with_contact(
                Ipv4Addr::LOCALHOST,
                contact,
                |builder| builder,
                |router| router,
            )
            .await;
        assert!(result.is_err());
        assert!(
            result
                .err()
                .unwrap()
                .to_string()
                .contains("explicit contact resource denied")
        );
        assert_eq!(traffic.snapshot()["generation"], 0);
        assert_eq!(traffic.snapshot()["driver_active"], 0);
        assert_eq!(traffic.snapshot()["queued_packets"], 0);
    }
    let retired = broker.retired().await;
    assert_eq!(retired["lateActiveSockets"], 0);
    println!(
        "MAINTAINED_CONTACT_DENIED_RECEIPT {}",
        serde_json::json!({"disabledRejected":true,"completeMappingMismatchRejected":true,"contactNeverBound":true,"mqttRetirement":retired})
    );
    broker.close().await;
}

#[tokio::test]
async fn default_wrapper_reports_absent_explicit_observer_as_unknown() {
    let directory = tempfile::tempdir().unwrap();
    let broker =
        member_priority_broker::Broker::new(member_contact_observation::Events::default()).await;
    let node = NativeNetwork::create(directory.path().join("node"))
        .await
        .unwrap();
    let mut config = node.resource_configuration().unwrap();
    config.replacement = Some(vec![Resource {
        url: format!("mqtt://{}/", broker.address),
        uses: BTreeSet::from([ResourceUse::Signaling]),
        limitation: "owned compatibility contact fixture".into(),
    }]);
    node.save_resource_configuration(&config).unwrap();
    let node = node.maintain_network(Ipv4Addr::LOCALHOST).await.unwrap();
    assert_eq!(
        node.maintenance_status().contact_listener.unwrap().phase,
        ContactListenerPhase::Ready
    );
    assert!(node.contact_listener_traffic().is_none());
    let updates = node.watch_maintenance();
    node.close().await;
    assert_eq!(updates.borrow().state, MaintenanceState::Stopped);
    let retired = broker.retired().await;
    assert_eq!(retired["lateActiveSockets"], 0);
    println!(
        "MAINTAINED_OBSERVER_UNKNOWN_RECEIPT {}",
        serde_json::json!({"defaultWrapperPreserved":true,"realListenerReady":true,"explicitObserverUnknown":true,"mqttRetirement":retired})
    );
    broker.close().await;
}
