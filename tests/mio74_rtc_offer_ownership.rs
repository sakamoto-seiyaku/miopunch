//! Actual SDK/TLS boundary: a proved wire peer advertises the wrong RTC role.
//! This protects Native offerer admission and ownership, not browser callbacks.
#![cfg(feature = "rtc-probe")]

use iroh::{
    Endpoint,
    endpoint::{AdmissionOutcome, ControlConnection, ControlMessageFormat, EndpointAdmission},
};
use miopunch_native_probe::{
    admission_gate::{ADMISSION_REQUEST_ALPN, AdmissionGate, ProtocolScope},
    connection_coordinator::{AttemptContext, ConnectError, MEMBER_ALPN, SessionPurpose},
    manual_admission::ManualExchange,
    native_network::NativeNetwork,
    resources::{LanDiscovery, PortMapping, ResourceUse},
    rtc_transport::RtcTransport,
};
use std::{
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    sync::{Arc, Mutex},
    time::Duration,
};

fn attempt() -> AttemptContext {
    AttemptContext::new(
        n0_future::time::Instant::now() + Duration::from_secs(12),
        tokio_util::sync::CancellationToken::new(),
    )
}

async fn saved_pair() -> (tempfile::TempDir, NativeNetwork, NativeNetwork) {
    let directory = tempfile::tempdir().unwrap();
    let a_path = directory.path().join("a");
    let b_path = directory.path().join("b");
    let mut a = NativeNetwork::create(&a_path).await.unwrap();
    let mut b = NativeNetwork::join(&b_path, a.manual_invitation().unwrap())
        .await
        .unwrap();
    let ids = (a.status().node_id, b.status().node_id);
    assert_ne!(ids.0, ids.1);
    for node in [&a, &b] {
        let mut resources = node.resource_configuration().unwrap();
        resources.replacement = Some(vec![]);
        resources.lan_discovery = LanDiscovery::Disabled;
        resources.port_mapping = PortMapping::Disabled;
        resources.tcp_direct_disabled = true;
        resources.member_data_relay_disabled = true;
        resources.disabled_uses.insert(ResourceUse::Observation);
        node.save_resource_configuration(&resources).unwrap();
    }
    let ae = a.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let be = b.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = tokio::join!(
        be.connect_control(ae.addr(), ADMISSION_REQUEST_ALPN),
        async { ae.accept().await.unwrap().accept_control().unwrap().await }
    );
    let (client, server) = (Arc::new(client.unwrap()), Arc::new(server.unwrap()));
    let (request, provided) = tokio::join!(
        b.request_manual_admission(&client),
        a.serve_manual_admission(&server)
    );
    assert!(matches!(request.unwrap(), ManualExchange::AwaitingApproval));
    provided.unwrap();
    let request = a.manual_requests().await.unwrap().pop().unwrap();
    a.approve_manual_admission(&request).await.unwrap();
    let receive = async {
        let ManualExchange::Materials(material) =
            b.request_manual_admission(&client).await.unwrap()
        else {
            panic!("independent member materials required");
        };
        b.complete_manual_admission(material).await.unwrap();
    };
    let (_, provided) = tokio::join!(receive, a.serve_manual_admission(&server));
    provided.unwrap();
    client.close(0u32.into(), b"owned admission complete");
    server.close(0u32.into(), b"owned admission complete");
    tokio::join!(ae.close(), be.close());
    tokio::join!(a.close(), b.close());
    let a = NativeNetwork::open(&a_path).await.unwrap();
    let b = NativeNetwork::open(&b_path).await.unwrap();
    assert_eq!((a.status().node_id, b.status().node_id), ids);
    (directory, a, b)
}

#[derive(Default)]
struct Observation {
    tls_peer_matched: bool,
    current_member_proof: bool,
    native_rtc_offer_advertised: bool,
    candidate_stream_observed: bool,
    original: Option<iroh::endpoint::WeakControlConnection>,
}

#[derive(Clone)]
struct WrongResponderRole {
    gate: AdmissionGate,
    observation: Arc<Mutex<Observation>>,
}

impl std::fmt::Debug for WrongResponderRole {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("WrongResponderRole")
    }
}

impl EndpointAdmission for WrongResponderRole {
    async fn admit(&self, connection: &ControlConnection) -> std::io::Result<AdmissionOutcome> {
        let _authenticated = self.gate.admit(connection).await?;
        let proof = self
            .gate
            .proof_for(connection)
            .ok_or_else(|| std::io::Error::other("actual SDK member proof missing"))?;
        let exchange = connection.accept_control_exchange().await?;
        let mut native = [0; 28];
        native.copy_from_slice(
            &exchange
                .recv_message(ControlMessageFormat::LengthPrefixed16 { maximum: 28 })
                .await?,
        );
        if &native[..4] != b"MUC1"
            || native[4] != 1
            || native[5] != 1
            || native[6] != 1
            || native[7] != 1
            || native[8] & 8 == 0
            || native[10] != 1
        {
            return Err(std::io::Error::other(
                "native SDK did not advertise the current offerer role",
            ));
        }
        {
            let mut observed = self.observation.lock().unwrap();
            observed.tls_peer_matched = proof.peer() == connection.remote_id();
            observed.current_member_proof = self.gate.allows(connection, ProtocolScope::Member);
            observed.native_rtc_offer_advertised = true;
            observed.original = Some(connection.weak_handle());
        }
        // Literal protocol input: Browser runtime, responder, RTC-only support
        // and permission. Only its RTC role is wrong: Offerer instead of Answerer.
        let mut frame = [0; 28];
        frame[..4].copy_from_slice(b"MUC1");
        frame[4..11].copy_from_slice(&[1, 1, 2, 2, 8, 8, 1]);
        frame[12..].copy_from_slice(&[0x74; 16]);
        exchange
            .send_message(
                ControlMessageFormat::LengthPrefixed16 { maximum: 28 },
                &frame,
            )
            .await?;
        tokio::select! {
            candidate = connection.accept_control_exchange() => {
                self.observation.lock().unwrap().candidate_stream_observed = candidate.is_ok();
            }
            _ = connection.closed() => {}
        }
        Err(std::io::Error::other("owned malformed role peer retired"))
    }
    fn is_current(&self, connection: &ControlConnection) -> bool {
        self.gate.is_current(connection)
    }
}

async fn stream_step(
    local_send: &mut iroh::endpoint::SendStream,
    local_recv: &mut iroh::endpoint::RecvStream,
    remote_send: &mut iroh::endpoint::SendStream,
    remote_recv: &mut iroh::endpoint::RecvStream,
    text: &[u8; 3],
) {
    let local = async {
        local_send.write_all(text).await.unwrap();
        let mut received = [0; 3];
        local_recv.read_exact(&mut received).await.unwrap();
        assert_eq!(&received, text);
    };
    let remote = async {
        let mut received = [0; 3];
        remote_recv.read_exact(&mut received).await.unwrap();
        assert_eq!(&received, text);
        remote_send.write_all(text).await.unwrap();
    };
    tokio::join!(local, remote);
}

#[tokio::test]
async fn native_sdk_rejects_the_wrong_rtc_responder_role_and_preserves_an_existing_member_stream() {
    let (_directory, a, b) = saved_pair().await;
    let rtc = RtcTransport::new();
    let native = a
        .bind_configured_endpoint_with_rtc(
            Endpoint::builder(iroh::endpoint::presets::Minimal)
                .bind_addr("127.0.0.1:0".parse::<SocketAddr>().unwrap())
                .unwrap(),
            rtc.clone(),
        )
        .await
        .unwrap();
    let healthy_a = a
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let healthy_b = b
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let healthy_coordinator = healthy_a.coordinator();
    let (hc, hp) = tokio::join!(
        healthy_coordinator.connect(healthy_b.addr(), SessionPurpose::MemberBusiness, attempt()),
        async {
            healthy_b
                .coordinator()
                .accept(
                    healthy_b.endpoint().accept().await.unwrap(),
                    SessionPurpose::MemberBusiness,
                    attempt(),
                )
                .await
        }
    );
    let (healthy, remote) = (hc.unwrap(), hp.unwrap());
    let healthy_stable = healthy.stable_id();
    let (mut send, mut recv) = healthy.open_bi().await.unwrap();
    send.write_all(b"one").await.unwrap();
    let (mut peer_send, mut peer_recv) = remote.accept_bi().await.unwrap();
    let mut first = [0; 3];
    peer_recv.read_exact(&mut first).await.unwrap();
    assert_eq!(&first, b"one");
    peer_send.write_all(b"one").await.unwrap();
    recv.read_exact(&mut first).await.unwrap();
    assert_eq!(&first, b"one");
    let stream_id = send.id();
    let observation = Arc::new(Mutex::new(Observation::default()));
    let key: [u8; 32] = std::fs::read(_directory.path().join("b/node.key"))
        .unwrap()
        .try_into()
        .unwrap();
    let peer = b
        .resource_configuration()
        .unwrap()
        .configure(
            Endpoint::builder(iroh::endpoint::presets::Minimal)
                .bind_addr("127.0.0.1:0".parse::<SocketAddr>().unwrap())
                .unwrap(),
        )
        .unwrap()
        .secret_key(iroh_base::SecretKey::from_bytes(&key))
        .alpns(vec![MEMBER_ALPN.to_vec()])
        .restricted_application(WrongResponderRole {
            gate: b.admission_gate(),
            observation: observation.clone(),
        })
        .bind()
        .await
        .unwrap();
    let owned_addresses: Vec<_> = [
        healthy_a.addr(),
        healthy_b.addr(),
        native.addr(),
        peer.addr(),
    ]
    .into_iter()
    .flat_map(|addr| addr.ip_addrs().copied().collect::<Vec<_>>())
    .collect();
    let caller = attempt();
    let deadline = caller.deadline;
    let started = std::time::Instant::now();
    let native_coordinator = native.coordinator();
    let (result, peer_result) = tokio::join!(
        native_coordinator.connect(peer.addr(), SessionPurpose::MemberBusiness, caller),
        async {
            peer.accept()
                .await
                .unwrap()
                .with_caller_deadline(deadline)
                .await
        }
    );
    assert!(
        matches!(result, Err(ConnectError::InvalidControl)),
        "actual wrong role must be InvalidControl: {result:?}"
    );
    assert!(peer_result.is_err());
    let caller_ms = started.elapsed().as_millis();
    let observed = observation.lock().unwrap();
    assert!(
        observed.tls_peer_matched
            && observed.current_member_proof
            && observed.native_rtc_offer_advertised
    );
    assert!(!observed.candidate_stream_observed);
    assert!(!observed.original.as_ref().unwrap().is_open());
    drop(observed);
    assert_eq!(rtc.active_associations(), 0);
    assert_eq!(rtc.active_pumps(), 0);
    stream_step(&mut send, &mut recv, &mut peer_send, &mut peer_recv, b"two").await;
    assert_eq!(send.id(), stream_id);
    assert_eq!(healthy.stable_id(), healthy_stable);
    assert!(healthy.close_reason().is_none());
    send.finish().unwrap();
    peer_send.finish().unwrap();
    healthy.close(0u32.into(), b"owned healthy stream complete");
    remote.close(0u32.into(), b"owned healthy stream complete");
    drop((
        send,
        recv,
        peer_send,
        peer_recv,
        healthy,
        remote,
        healthy_coordinator,
        native_coordinator,
    ));
    let cleanup = std::time::Instant::now();
    tokio::join!(
        healthy_a.close(),
        healthy_b.close(),
        native.close(),
        peer.close()
    );
    drop(peer);
    tokio::join!(a.close(), b.close());
    let immediate: Vec<_> = owned_addresses
        .iter()
        .map(|address| UdpSocket::bind(address).is_ok())
        .collect();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let late: Vec<_> = owned_addresses
        .iter()
        .map(|address| UdpSocket::bind(address).is_ok())
        .collect();
    assert!(late.iter().all(|released| *released));
    println!(
        "{}",
        serde_json::json!({"event":"mio74-native-wrong-role",
        "scope":"actual native SDK and proved protected wire peer; no browser callback qualification",
        "independentSavedReopenedMembers":2,"invalidControl":true,"ordinaryHandleReturned":false,
        "nativeOffererAdvertised":true,"existingOriginalStreamMatched":true,
        "activeRtcAssociations":rtc.active_associations(),"activeRtcPumps":rtc.active_pumps(),
        "callerBudgetMs":12000,"callerMs":caller_ms,"cleanupMs":cleanup.elapsed().as_millis(),
        "ownedUdpReleasedImmediately":immediate,"ownedUdpReleasedLate100Ms":late})
    );
}
