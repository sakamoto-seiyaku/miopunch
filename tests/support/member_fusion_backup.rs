use super::*;
use iroh::endpoint::{
    RelayResourceUse,
    transports::{CustomEndpoint, CustomTransport},
};
use iroh_base::CustomAddr;
use miopunch_native_probe::irc_transport::IrcTransport;
use miopunch_native_probe::mqtt_transport::{MqttTransport, Traffic};
use n0_watcher::{Direct, Watcher};
use tokio::net::TcpListener;

#[path = "relay_contact_broker.rs"]
#[allow(dead_code)]
mod broker;

#[derive(Debug)]
struct Route {
    address: TransportAddr,
    started: Instant,
    identity: Option<(Instant, EndpointId, usize)>,
    outcome: Option<Result<usize, ErrorKind>>,
    settled: bool,
}

#[derive(Clone, Default)]
struct Timeline(Arc<Mutex<Vec<Route>>>);

impl Timeline {
    fn observe(&self, options: MemberDialOptions, deadline: Instant) -> MemberDialOptions {
        let starts = self.clone();
        let identities = self.clone();
        options
            .with_route_diagnostics(move |address, caller_deadline| {
                assert_eq!(caller_deadline, deadline);
                let mut routes = starts.0.lock().unwrap();
                let index = routes.len();
                routes.push(Route {
                    address: address.clone(),
                    started: Instant::now(),
                    identity: None,
                    outcome: None,
                    settled: false,
                });
                let report = Settlement(starts.clone(), index);
                ((1, index), move |outcome| report.finish(outcome))
            })
            .with_identity_diagnostics(move |(_, index), peer, stable_id| {
                let mut routes = identities.0.lock().unwrap();
                assert!(routes[index].identity.is_none());
                routes[index].identity = Some((Instant::now(), peer, stable_id));
            })
    }
}

struct Settlement(Timeline, usize);
impl Settlement {
    fn finish(self, outcome: Result<usize, ErrorKind>) {
        self.0.0.lock().unwrap()[self.1].outcome = Some(outcome);
    }
}
impl Drop for Settlement {
    fn drop(&mut self) {
        self.0.0.lock().unwrap()[self.1].settled = true;
    }
}

fn mqtt_port(broker: &broker::Broker) -> Arc<MqttTransport> {
    Arc::new(MqttTransport::new(
        broker.address,
        Arc::new(Traffic::default()),
    ))
}

fn mqtt_address(mqtt: &MqttTransport) -> iroh_base::CustomAddr {
    MqttTransport::address(&mqtt.route().unwrap()).unwrap()
}

async fn reply_once(server: &MemberFusionEndpoint, peer: EndpointId) -> usize {
    let connection = server.accept().await.unwrap().unwrap();
    assert_eq!(connection.remote_id(), peer);
    let mut stream = connection.accept_bi().await.unwrap();
    let (send, recv) = stream.streams();
    assert_eq!(recv.read_to_end(64).await.unwrap(), REQUEST);
    send.write_all(REPLY).await.unwrap();
    send.finish().unwrap();
    assert_eq!(send.stopped().await.unwrap(), None);
    connection.stable_id()
}

async fn refused_relay() -> RelayUrl {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    format!("http://{}", listener.local_addr().unwrap())
        .parse()
        .unwrap()
}

#[derive(Debug)]
struct ObservedIrc {
    transport: Arc<IrcTransport>,
    addresses: Mutex<Option<Direct<Vec<CustomAddr>>>>,
}

impl CustomTransport for ObservedIrc {
    fn resource(&self) -> Option<(RelayUrl, RelayResourceUse)> {
        self.transport.resource()
    }

    fn bind(&self) -> std::io::Result<Box<dyn CustomEndpoint>> {
        let endpoint = self.transport.bind()?;
        *self.addresses.lock().unwrap() = Some(endpoint.watch_local_addrs());
        Ok(endpoint)
    }
}

#[tokio::test]
#[ignore = "requires the owned real IRCd supplied by MIOPUNCH_IRC_SERVER"]
async fn ready_irc_precedes_a_raw_mqtt_sender_whose_subscription_is_pending() {
    use super::super::{Resource, ResourceUse};

    let irc_url: RelayUrl = format!(
        "irc://{}/",
        std::env::var("MIOPUNCH_IRC_SERVER").expect("owned real IRCd required")
    )
    .parse()
    .unwrap();
    let broker = broker::Broker::new().await;
    let gate = broker::RelayGate::new(broker.address).await;
    gate.hold_new();
    let relay = relay().await;
    let dir = tempfile::tempdir().unwrap();
    let (admin, member_id) = admitted_pair(dir.path(), &relay.http_url().unwrap()).await;
    let member = NativeNetwork::open(dir.path().join("joining"))
        .await
        .unwrap();
    let caller_mqtt = Arc::new(MqttTransport::new(
        gate.address,
        Arc::new(Traffic::default()),
    ));
    let receiver_mqtt = mqtt_port(&broker);
    let caller_irc = Arc::new(IrcTransport::from_resource(irc_url.clone()).unwrap());
    let receiver_irc = Arc::new(ObservedIrc {
        transport: Arc::new(IrcTransport::from_resource(irc_url.clone()).unwrap()),
        addresses: Mutex::new(None),
    });
    let signaling = |mqtt| {
        [format!("mqtt://{mqtt}/"), irc_url.to_string()]
            .map(|url| Resource {
                url,
                uses: [ResourceUse::Signaling].into(),
                limitation: "owned signaling fixture".into(),
            })
            .into()
    };
    for (node, mqtt) in [(&admin, broker.address), (&member, gate.address)] {
        let mut policy = node.resource_configuration().unwrap();
        policy.udp_direct_disabled = false;
        policy.replacement = Some(signaling(mqtt));
        node.save_resource_configuration(&policy).unwrap();
    }
    let builder = || {
        Endpoint::builder(presets::Minimal)
            .clear_ip_transports()
            .bind_addr(std::net::SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .unwrap()
    };
    let server = admin
        .bind_member_fusion_endpoint_with(
            builder()
                .add_custom_transport(receiver_mqtt.clone())
                .add_custom_transport(receiver_irc.clone()),
            SETUP,
        )
        .await
        .unwrap();
    let client = member
        .bind_member_fusion_endpoint_with(
            builder()
                .add_custom_transport(caller_mqtt.clone())
                .add_custom_transport(caller_irc.clone()),
            SETUP,
        )
        .await
        .unwrap();
    receiver_mqtt.wait_ready().await.unwrap();
    receiver_irc.transport.wait_ready().await.unwrap();
    caller_irc.wait_ready().await.unwrap();
    time::timeout(SETUP, async {
        while gate.active() == 0 {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the caller opened TCP and is waiting for the broker");
    assert!(!caller_mqtt.diagnostics().1);
    let mqtt = mqtt_address(&receiver_mqtt);
    let irc = receiver_irc
        .addresses
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .get()[0]
        .clone();
    let timeline = Timeline::default();
    let deadline = Instant::now() + SETUP;
    let options = timeline.observe(
        MemberDialOptions::default().with_restricted_contacts(vec![mqtt, irc.clone()]),
        deadline,
    );
    let cancel = CancellationToken::new();
    let (caller, responder) = time::timeout_at(deadline, async {
        tokio::join!(
            async {
                let connection = client
                    .connect_with_routes_until(
                        EndpointAddr::new(server.id()),
                        options,
                        deadline,
                        &cancel,
                    )
                    .await
                    .unwrap();
                assert_eq!(connection.remote_id(), server.id());
                assert!(connection.initial_data_path().is_ip());
                request(&connection).await;
                connection.stable_id()
            },
            reply_once(&server, member_id),
        )
    })
    .await
    .unwrap();
    assert!(!caller_mqtt.diagnostics().1);
    assert_eq!(caller_mqtt.traffic().sent_packets(), 0);
    assert!(caller_irc.traffic().sent_packets() > 0);
    client.close().await;
    server.close().await;
    member.close().await;
    admin.close().await;
    gate.close().await;
    broker.close().await;
    relay.shutdown().await.unwrap();
    let routes = timeline.0.lock().unwrap();
    println!("MIO149 raw MQTT pending; caller={caller} responder={responder} routes={routes:?}");
    assert_eq!(
        routes
            .iter()
            .map(|route| &route.address)
            .collect::<Vec<_>>(),
        vec![&TransportAddr::Custom(irc)],
        "the ready IRC is the only candidate started while MQTT is pending"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn physical_relays_leave_one_contact_opportunity_in_the_original_dial() {
    let healthy_server = relay().await;
    let healthy = healthy_server.http_url().unwrap();
    let registration_only = relay().await;
    let registered = registration_only.http_url().unwrap();
    let mut silent = Vec::new();
    for _ in 0..4 {
        silent
            .push(delayed_relay::delay_relay(healthy_server.http_addr().unwrap(), SETUP * 2).await);
    }
    let [first, second, third, fourth] = std::array::from_fn(|index| silent[index].0.clone());
    let dead_a = refused_relay().await;
    let dead_b = refused_relay().await;
    let denied = refused_relay().await;
    let broker = broker::Broker::new().await;
    let dir = tempfile::tempdir().unwrap();
    let (mut admin, member_id) = admitted_pair(dir.path(), &healthy).await;
    let member = NativeNetwork::open(dir.path().join("joining"))
        .await
        .unwrap();
    let mut server_policy = admin.resource_configuration().unwrap();
    server_policy.udp_direct_disabled = false;
    let mqtt_resource = super::super::Resource {
        url: format!("mqtt://{}/", broker.address),
        uses: [super::super::ResourceUse::Signaling].into(),
        limitation: "owned MQTT contact broker".into(),
    };
    server_policy.defaults.resources.push(mqtt_resource.clone());
    admin.save_resource_configuration(&server_policy).unwrap();
    let normal = Duration::from_secs(4);
    // The short dial reserves much less than the MQTT business qualification
    // took in the production test. Its observed milestones are asserted
    // separately from cases that require a successful business reply.
    let short = Duration::from_millis(960);
    let (slow_admission, stop_admission) = delayed_relay::delay_relay(
        healthy_server.http_addr().unwrap(),
        Duration::from_millis(100),
    )
    .await;
    let late_gate = broker::RelayGate::new(healthy_server.http_addr().unwrap()).await;
    let late_relay: RelayUrl = format!("http://{}", late_gate.address).parse().unwrap();
    late_gate.hold_new();
    let cases = [
        (
            "healthy",
            normal,
            vec![healthy.clone()],
            None,
            Some(healthy.clone()),
        ),
        (
            "silent-first",
            normal,
            vec![first.clone(), healthy.clone()],
            None,
            Some(healthy.clone()),
        ),
        (
            "identity-before-admission",
            normal,
            vec![slow_admission.clone()],
            None,
            Some(slow_admission),
        ),
        (
            "late-relay-wins",
            normal,
            vec![late_relay.clone()],
            Some((500, 60)),
            Some(late_relay),
        ),
        (
            "four-silent",
            normal,
            vec![first.clone(), second.clone(), third.clone(), fourth.clone()],
            Some((1250, 60)),
            None,
        ),
        (
            "two-permitted",
            normal,
            vec![first.clone(), second.clone(), first.clone(), denied.clone()],
            Some((750, 60)),
            None,
        ),
        (
            "two-short",
            short,
            vec![first.clone(), second.clone()],
            Some((720, 60)),
            None,
        ),
        (
            "all-refused",
            normal,
            vec![dead_a, dead_b],
            Some((250, 160)),
            None,
        ),
        (
            "locally-registered",
            normal,
            vec![registered],
            Some((500, 60)),
            None,
        ),
        (
            "expired",
            short,
            vec![first.clone(), second.clone(), third, fourth],
            None,
            None,
        ),
        ("cancelled", normal, vec![first, second], None, None),
        ("rejected", normal, vec![healthy.clone()], None, None),
    ];
    for (label, budget, relays, backup, relay_winner) in cases {
        if label == "rejected" {
            let mut manager = admin.begin_management_session();
            admin
                .manage(
                    &mut manager,
                    ManagementAction::RemoveMember(
                        VerifyingKey::from_bytes(member_id.as_bytes()).unwrap(),
                    ),
                )
                .await
                .unwrap();
        }
        let udp_disabled = relay_winner.is_some() || label == "rejected";
        let mut provider_policy = server_policy.clone();
        provider_policy.udp_direct_disabled = udp_disabled;
        admin.save_resource_configuration(&provider_policy).unwrap();
        let builder = |mqtt: Arc<MqttTransport>| {
            Endpoint::builder(presets::Minimal)
                .clear_ip_transports()
                .bind_addr(std::net::SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
                .unwrap()
                .add_custom_transport(mqtt)
        };
        let receiver_mqtt = mqtt_port(&broker);
        let server = admin
            .bind_member_fusion_endpoint_with(builder(receiver_mqtt.clone()), SETUP)
            .await
            .unwrap();
        time::timeout(SETUP, server.online()).await.unwrap();
        receiver_mqtt.wait_ready().await.unwrap();

        let mut config = server_policy.clone();
        config.udp_direct_disabled = udp_disabled;
        config.replacement = Some(
            relays
                .iter()
                .filter(|url| **url != denied)
                .map(resource)
                .chain([mqtt_resource.clone()])
                .collect(),
        );
        member.save_resource_configuration(&config).unwrap();
        let sender_mqtt = mqtt_port(&broker);
        let client = member
            .bind_member_fusion_endpoint_with(builder(sender_mqtt.clone()), SETUP)
            .await
            .unwrap();
        sender_mqtt.wait_ready().await.unwrap();
        if matches!(label, "locally-registered" | "identity-before-admission") {
            time::timeout(SETUP, client.online()).await.unwrap();
            assert_eq!(
                client.addr().relay_urls().collect::<Vec<_>>(),
                vec![&relays[0]]
            );
        }
        let mqtt_route = if label == "late-relay-wins" {
            MqttTransport::address("00000000000000000000000000000000").unwrap()
        } else {
            mqtt_address(&receiver_mqtt)
        };
        let target = EndpointAddr::new(server.id()).with_relay_url(relays[0].clone());
        let timeline = Timeline::default();
        let deadline = Instant::now() + budget;
        let options = timeline.observe(
            MemberDialOptions::new(Vec::new(), Vec::new(), relays.clone())
                .with_restricted_contacts(vec![mqtt_route.clone()]),
            deadline,
        );
        let cancel = CancellationToken::new();
        let dial = client.connect_with_routes_until(target, options, deadline, &cancel);
        let must_reply = !matches!(label, "expired" | "cancelled" | "two-short" | "rejected");
        let (outgoing, incoming) = if must_reply {
            let reply = async {
                let connection = dial.await.unwrap();
                assert_eq!(connection.remote_id(), server.id());
                if let Some(expected) = &relay_winner {
                    assert_eq!(
                        connection.initial_data_path(),
                        &TransportAddr::Relay(expected.clone())
                    );
                } else {
                    assert!(connection.initial_data_path().is_ip());
                }
                request(&connection).await;
                (connection.stable_id(), Instant::now())
            };
            let (outgoing, incoming, ()) = time::timeout_at(deadline, async {
                tokio::join!(reply, reply_once(&server, member_id), async {
                    if label == "late-relay-wins" {
                        time::sleep_until(deadline - budget + Duration::from_millis(700)).await;
                        assert!(late_gate.active() > 0);
                        late_gate.release();
                    }
                })
            })
            .await
            .unwrap();
            (Some(outgoing), Some(incoming))
        } else if label == "cancelled" {
            let (result, ()) = tokio::join!(dial, async {
                time::sleep(Duration::from_millis(100)).await;
                cancel.cancel();
            });
            assert_eq!(result.unwrap_err().kind(), ErrorKind::Interrupted);
            time::sleep(Duration::from_millis(800)).await;
            (None, None)
        } else {
            let result = dial.await;
            if label == "expired" {
                assert_eq!(result.unwrap_err().kind(), ErrorKind::TimedOut);
                time::sleep(Duration::from_millis(250)).await;
            } else if label == "rejected" {
                assert_eq!(result.unwrap_err().kind(), ErrorKind::PermissionDenied);
            } else if let Err(error) = result {
                assert_eq!(error.kind(), ErrorKind::TimedOut);
            }
            (None, None)
        };
        client.close().await;
        server.close().await;

        let events = timeline.0.lock().unwrap();
        assert!(
            events
                .iter()
                .all(|route| route.started < deadline && route.settled),
            "{events:?}"
        );
        assert!(
            events
                .iter()
                .filter(|route| route.address.is_relay())
                .all(|route| route.address != TransportAddr::Relay(denied.clone())),
            "{events:?}"
        );
        let entries: Vec<_> = events
            .iter()
            .filter(|route| route.address == TransportAddr::Custom(mqtt_route.clone()))
            .collect();
        let anchor = events[0].started;
        if let Some((expected, tolerance)) = backup {
            assert_eq!(entries.len(), 1, "{label}: {events:?}");
            let actual = entries[0].started.duration_since(anchor);
            assert!(
                actual.abs_diff(Duration::from_millis(expected))
                    <= Duration::from_millis(tolerance),
                "{label}: expected {expected}ms, saw {actual:?}, {events:?}"
            );
            if label == "all-refused" {
                assert!(
                    events[..2]
                        .iter()
                        .all(|route| route.outcome == Some(Err(ErrorKind::NotConnected))),
                    "{events:?}"
                );
            }
            if label == "two-short" {
                let measured_stagger = events[1].started.duration_since(anchor);
                assert!(
                    actual.abs_diff(3 * measured_stagger) <= Duration::from_millis(60),
                    "{events:?}"
                );
                assert_eq!(
                    entries[0].identity.map(|(_, id, _)| id),
                    Some(server.id()),
                    "{events:?}"
                );
            }
            if label == "late-relay-wins" {
                assert!(
                    events[0].identity.unwrap().0 > entries[0].started,
                    "{events:?}"
                );
                assert!(entries[0].identity.is_none());
                assert_eq!(entries[0].outcome, None, "the silent contact was cancelled");
            }
        } else {
            assert!(entries.is_empty(), "{label}: {events:?}");
        }
        if label == "cancelled" {
            assert_eq!(events.len(), 1, "{events:?}");
        }
        if label == "rejected" {
            assert_eq!(
                events[0].identity.map(|(_, peer, _)| peer),
                Some(server.id()),
                "{events:?}"
            );
            assert_eq!(events[0].outcome, Some(Err(ErrorKind::PermissionDenied)));
        }
        if let Some((stable_id, replied)) = outgoing {
            assert!(
                events.iter().any(|route| route
                    .identity
                    .is_some_and(|(at, peer, id)| peer == server.id()
                        && id == stable_id
                        && at < replied)),
                "{events:?}"
            );
            if label == "identity-before-admission" {
                assert!(events[0].identity.unwrap().0 - anchor < Duration::from_millis(500));
                assert!(replied - anchor > Duration::from_millis(500));
            }
        }
        println!("MIO149 {label} caller={outgoing:?} responder={incoming:?} routes={events:?}");
    }
    member.close().await;
    admin.close().await;
    for (_, stop) in silent {
        stop.cancel();
    }
    stop_admission.cancel();
    late_gate.close().await;
    registration_only.shutdown().await.unwrap();
    healthy_server.shutdown().await.unwrap();
    broker.close().await;
}
