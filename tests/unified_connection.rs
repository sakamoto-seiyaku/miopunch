//! Formal SDK seam: durable member material, protected release, real streams.
use iroh::{
    Endpoint,
    endpoint::{Connection, EndpointHooks},
};
use miopunch_native_probe::{
    admission_gate::{ADMISSION_REQUEST_ALPN, ProtocolScope},
    connection_coordinator::{AttemptContext, ConnectError, MEMBER_ALPN, SessionPurpose},
    manual_admission::ManualExchange,
    native_network::NativeNetwork,
};
use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::Duration,
};

async fn admitted_pair() -> (tempfile::TempDir, NativeNetwork, NativeNetwork) {
    let dir = tempfile::tempdir().unwrap();
    let a_path = dir.path().join("a");
    let b_path = dir.path().join("b");
    let mut a = NativeNetwork::create(&a_path).await.unwrap();
    let mut b = NativeNetwork::join(&b_path, a.manual_invitation().unwrap())
        .await
        .unwrap();
    for node in [&a, &b] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(vec![]);
        node.save_resource_configuration(&config).unwrap();
    }
    let ae = a.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let be = b.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    // Both sides consume the same original setup caller context required by
    // the unified restricted admission protocol; no stage restarts this clock.
    let deadline = n0_future::time::Instant::now() + Duration::from_secs(5);
    let options = iroh::endpoint::ConnectOptions::new().with_caller_deadline(deadline);
    let (client, server) = tokio::join!(
        async {
            be.connect_with_opts(ae.addr(), ADMISSION_REQUEST_ALPN, options)
                .await
                .unwrap()
                .await
        },
        async {
            ae.accept()
                .await
                .unwrap()
                .with_caller_deadline(deadline)
                .await
        }
    );
    let (client, server) = (client.unwrap(), server.unwrap());
    let (request, served) = tokio::join!(
        b.request_manual_admission(&client),
        a.serve_manual_admission(&server)
    );
    assert!(matches!(request.unwrap(), ManualExchange::AwaitingApproval));
    served.unwrap();
    let request = a.manual_requests().await.unwrap().pop().unwrap();
    a.approve_manual_admission(&request).await.unwrap();
    let receiver = async {
        let ManualExchange::Materials(material) =
            b.request_manual_admission(&client).await.unwrap()
        else {
            panic!("approved materials")
        };
        b.complete_manual_admission(material).await.unwrap();
    };
    let (_, delivered) = tokio::join!(receiver, a.serve_manual_admission(&server));
    delivered.unwrap();
    client.close(0u32.into(), b"admission done");
    server.close(0u32.into(), b"admission done");
    tokio::join!(ae.close(), be.close());
    tokio::join!(a.close(), b.close());
    let a = NativeNetwork::open(&a_path).await.unwrap();
    let b = NativeNetwork::open(&b_path).await.unwrap();
    (dir, a, b)
}

fn attempt() -> AttemptContext {
    AttemptContext::new(
        n0_future::time::Instant::now() + Duration::from_secs(5),
        tokio_util::sync::CancellationToken::new(),
    )
}

async fn streams(a: &Connection, b: &Connection) {
    let send = async {
        let (mut tx, mut rx) = a.open_bi().await.unwrap();
        tx.write_all(b"durable native member request")
            .await
            .unwrap();
        tx.finish().unwrap();
        assert_eq!(
            rx.read_to_end(1024).await.unwrap(),
            b"durable native member response"
        );
    };
    let receive = async {
        let (mut tx, mut rx) = b.accept_bi().await.unwrap();
        assert_eq!(
            rx.read_to_end(1024).await.unwrap(),
            b"durable native member request"
        );
        tx.write_all(b"durable native member response")
            .await
            .unwrap();
        tx.finish().unwrap();
    };
    tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(send, receive);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn reopened_members_use_common_sdk_without_selecting_platform_or_carrier() {
    let (_dir, a, b) = admitted_pair().await;
    let ae = a
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let be = b
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let (ac, bc) = (ae.coordinator(), be.coordinator());
    let sockets: Vec<SocketAddr> = ae
        .endpoint()
        .addr()
        .ip_addrs()
        .chain(be.endpoint().addr().ip_addrs())
        .copied()
        .collect();
    let started = std::time::Instant::now();
    let (out, incoming) = tokio::join!(
        ac.connect(be.addr(), SessionPurpose::MemberBusiness, attempt()),
        async {
            bc.accept(
                be.endpoint().accept().await.unwrap(),
                SessionPurpose::MemberBusiness,
                attempt(),
            )
            .await
        }
    );
    let (out, incoming) = (out.unwrap(), incoming.unwrap());
    assert_eq!(out.remote_id(), be.endpoint().id());
    assert_eq!(incoming.remote_id(), ae.endpoint().id());
    assert!(a.admission_gate().allows(&out, ProtocolScope::Member));
    assert!(b.admission_gate().allows(&incoming, ProtocolScope::Member));
    assert!(out.paths().iter().any(|p| p.is_ip() && p.is_selected()));
    streams(&out, &incoming).await;
    println!(
        "{}",
        serde_json::json!({"event":"unified-native-success","roles":["initiator","responder"],"identityMatched":[true,true],"callerMs":started.elapsed().as_millis(),"path":"native-udp","paths":out.paths().iter().map(|p|serde_json::json!({"route":p.remote_addr().to_string(),"selected":p.is_selected()})).collect::<Vec<_>>() })
    );
    out.close(0u32.into(), b"done");
    incoming.close(0u32.into(), b"done");
    drop((out, incoming, ac, bc));
    let cleanup = std::time::Instant::now();
    tokio::join!(ae.close(), be.close());
    tokio::join!(a.close(), b.close());
    let immediate = socket_release(&sockets);
    let cleanup_ms = cleanup.elapsed().as_millis();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let late = socket_release(&sockets);
    assert!(
        late.iter().all(|released| *released),
        "owned UDP sockets must eventually be released"
    );
    println!(
        "{}",
        serde_json::json!({"event":"unified-cleanup","cleanupMs":cleanup_ms,"lateObservationMs":cleanup.elapsed().as_millis(),"graceful":true,"socketReleasedImmediately":immediate,"socketReleasedLate":late})
    );
}

fn socket_release(sockets: &[SocketAddr]) -> Vec<bool> {
    sockets
        .iter()
        .map(|address| std::net::UdpSocket::bind(address).is_ok())
        .collect()
}

/// Adversarial wire at the protected control boundary, not a mocked gate or
/// transport. The independent member still uses its durable key/secret.
#[derive(Clone, Debug)]
struct ControlEnvelopeFixture(u8);
impl EndpointHooks for ControlEnvelopeFixture {
    async fn before_data_release(
        &self,
        _endpoint: &Endpoint,
        connection: &Connection,
    ) -> std::io::Result<()> {
        if connection.alpn() != MEMBER_ALPN {
            return Ok(());
        }
        let (mut send, mut recv) = connection.open_bi().await.map_err(std::io::Error::other)?;
        let mut bytes = vec![0; 28];
        bytes[..4].copy_from_slice(b"MUC1");
        bytes[4..10].copy_from_slice(&[1, 1, 1, 1, 1, 1]);
        bytes[12..].copy_from_slice(&[7; 16]);
        match self.0 {
            0 => bytes[4] = 99,
            1 => bytes[9] = 128,
            2 => bytes[7] = 2,
            3 => bytes.resize(29, 0),
            // A well-formed browser initiator supports/permits only RTC.
            // This wire-only negative fixture makes no RTC connectivity claim.
            4 | 5 => {
                bytes[6] = 2;
                bytes[8] = 8;
                bytes[9] = 8;
                bytes[10] = 1;
            }
            _ => unreachable!(),
        }
        send.write_all(&(bytes.len() as u16).to_be_bytes()).await?;
        send.write_all(&bytes).await?;
        send.finish()?;
        if self.0 == 5 {
            // Read the real common capability envelope, then send a stale
            // candidate attempt over another protected stream on this M.
            let mut hello = [0; 30];
            recv.read_exact(&mut hello)
                .await
                .map_err(std::io::Error::other)?;
            let (mut candidate, _) = connection.open_bi().await?;
            let invalid = serde_json::to_vec(&serde_json::json!({
                "remainingMs":1000,"attempt":hex::encode([0;16])
            }))
            .unwrap();
            candidate
                .write_all(&(invalid.len() as u32).to_be_bytes())
                .await?;
            candidate.write_all(&invalid).await?;
            candidate.finish()?;
        }
        let _ = recv.read_to_end(64).await;
        connection.closed().await;
        Err(std::io::Error::other("adversarial member attempt ended"))
    }
}

#[tokio::test]
async fn strict_protocol_refuses_bad_versions_capabilities_roles_and_lengths_without_harming_healthy_m()
 {
    let (_dir, a, b) = admitted_pair().await;
    let ae = a
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let be = b
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let (ac, bc) = (ae.coordinator(), be.coordinator());
    let (healthy, healthy_peer) = tokio::join!(
        ac.connect(be.addr(), SessionPurpose::MemberBusiness, attempt()),
        async {
            bc.accept(
                be.endpoint().accept().await.unwrap(),
                SessionPurpose::MemberBusiness,
                attempt(),
            )
            .await
        }
    );
    let (healthy, healthy_peer) = (healthy.unwrap(), healthy_peer.unwrap());
    for fault in 0..4 {
        let evil = b
            .bind_configured_endpoint_with(
                Endpoint::builder(iroh::endpoint::presets::Minimal)
                    .bind_addr("127.0.0.1:0".parse::<SocketAddr>().unwrap())
                    .unwrap()
                    .hooks(ControlEnvelopeFixture(fault)),
            )
            .await
            .unwrap();
        let started = std::time::Instant::now();
        let (bad, rejected) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(evil.endpoint().connect(ae.addr(), MEMBER_ALPN), async {
                ac.accept(
                    ae.endpoint().accept().await.unwrap(),
                    SessionPurpose::MemberBusiness,
                    attempt(),
                )
                .await
            })
        })
        .await
        .unwrap();
        assert!(bad.is_err());
        assert!(
            match (&rejected, fault) {
                (Err(ConnectError::ProtocolUnsupported), 0) => true,
                (Err(ConnectError::InvalidControl), 1..=3) => true,
                _ => false,
            },
            "explicit strict rejection: {rejected:?}"
        );
        let caller_ms = started.elapsed().as_millis();
        let sockets: Vec<_> = evil.addr().ip_addrs().copied().collect();
        let cleanup = std::time::Instant::now();
        evil.close().await;
        let immediate = socket_release(&sockets);
        let cleanup_ms = cleanup.elapsed().as_millis();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let late = socket_release(&sockets);
        assert!(late.iter().all(|released| *released));
        println!(
            "{}",
            serde_json::json!({"event":"unified-invalid-control","fault":fault,"error":rejected.unwrap_err().to_string(),"ordinaryHandleReturned":false,"callerMs":caller_ms,"cleanupMs":cleanup_ms,"socketReleasedImmediately":immediate,"socketReleasedLate":late})
        );
        streams(&healthy, &healthy_peer).await;
        assert!(a.admission_gate().allows(&healthy, ProtocolScope::Member));
    }
    healthy.close(0u32.into(), b"done");
    healthy_peer.close(0u32.into(), b"done");
    drop((healthy, healthy_peer, ac, bc));
    tokio::join!(ae.close(), be.close());
    tokio::join!(a.close(), b.close());
}

#[derive(Clone, Debug)]
struct PendingControl {
    entered: Arc<Mutex<Option<tokio::sync::oneshot::Sender<iroh::endpoint::WeakConnectionHandle>>>>,
}
impl EndpointHooks for PendingControl {
    async fn before_data_release(
        &self,
        _endpoint: &Endpoint,
        connection: &Connection,
    ) -> std::io::Result<()> {
        if connection.alpn() != MEMBER_ALPN {
            return Ok(());
        }
        let (_send, mut recv) = connection
            .accept_bi()
            .await
            .map_err(std::io::Error::other)?;
        let mut hello = [0; 30];
        recv.read_exact(&mut hello)
            .await
            .map_err(std::io::Error::other)?;
        self.entered
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .send(connection.weak_handle())
            .unwrap();
        connection.closed().await;
        Err(std::io::Error::other("pending protected control closed"))
    }
}

#[tokio::test]
async fn cancel_and_deadline_close_only_the_pending_original_m_and_preserve_healthy_member_streams()
{
    let (_dir, a, b) = admitted_pair().await;
    let ae = a
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let be = b
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let (ac, bc) = (ae.coordinator(), be.coordinator());
    let (healthy, healthy_peer) = tokio::join!(
        ac.connect(be.addr(), SessionPurpose::MemberBusiness, attempt()),
        async {
            bc.accept(
                be.endpoint().accept().await.unwrap(),
                SessionPurpose::MemberBusiness,
                attempt(),
            )
            .await
        }
    );
    let (healthy, healthy_peer) = (healthy.unwrap(), healthy_peer.unwrap());
    for cancel in [true, false] {
        let (entered, receiver) = tokio::sync::oneshot::channel();
        let pending = b
            .bind_configured_endpoint_with(
                Endpoint::builder(iroh::endpoint::presets::Minimal)
                    .bind_addr("127.0.0.1:0".parse::<SocketAddr>().unwrap())
                    .unwrap()
                    .hooks(PendingControl {
                        entered: Arc::new(Mutex::new(Some(entered))),
                    }),
            )
            .await
            .unwrap();
        let token = tokio_util::sync::CancellationToken::new();
        let deadline = n0_future::time::Instant::now() + Duration::from_secs(2);
        let ctx = AttemptContext::new(deadline, token.clone());
        // A fresh endpoint prevents the healthy session's peer address cache
        // from selecting its existing socket instead of this controlled route.
        let pending_client = a
            .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap();
        let caller = pending_client.coordinator();
        let address = pending.addr();
        let started = std::time::Instant::now();
        let out = tokio::spawn(async move {
            caller
                .connect(address, SessionPurpose::MemberBusiness, ctx)
                .await
        });
        let responder = pending.endpoint().clone();
        let server = tokio::spawn(async move { responder.accept().await.unwrap().await });
        let weak = tokio::time::timeout(Duration::from_secs(2), receiver)
            .await
            .unwrap()
            .unwrap();
        if cancel {
            token.cancel();
        }
        let result = tokio::time::timeout(Duration::from_secs(3), out)
            .await
            .unwrap()
            .unwrap();
        assert!(
            if cancel {
                matches!(result, Err(ConnectError::Cancelled))
            } else {
                matches!(result, Err(ConnectError::DeadlineExceeded))
            },
            "{result:?}"
        );
        assert!(
            tokio::time::timeout(Duration::from_secs(1), server)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        assert!(!weak.is_open());
        assert!(ae.endpoint().id() != pending.endpoint().id());
        streams(&healthy, &healthy_peer).await;
        let caller_ms = started.elapsed().as_millis();
        let sockets: Vec<_> = pending
            .addr()
            .ip_addrs()
            .chain(pending_client.addr().ip_addrs())
            .copied()
            .collect();
        let cleanup = std::time::Instant::now();
        tokio::join!(pending.close(), pending_client.close());
        let immediate = socket_release(&sockets);
        let cleanup_ms = cleanup.elapsed().as_millis();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let late = socket_release(&sockets);
        assert!(late.iter().all(|released| *released));
        println!(
            "{}",
            serde_json::json!({"event":"unified-cancel-deadline","cancel":cancel,"error":result.unwrap_err().to_string(),"ordinaryHandleReturned":false,"originalClosed":!weak.is_open(),"healthyStreamMatched":true,"callerMs":caller_ms,"deadlineMs":2000,"cleanupMs":cleanup_ms,"socketReleasedImmediately":immediate,"socketReleasedLate":late})
        );
    }
    healthy.close(0u32.into(), b"done");
    healthy_peer.close(0u32.into(), b"done");
    drop((healthy, healthy_peer, ac, bc));
    tokio::join!(ae.close(), be.close());
    tokio::join!(a.close(), b.close());
}

#[tokio::test]
async fn no_permitted_data_or_signaling_only_hints_never_return_an_ordinary_sdk_handle() {
    let (_dir, a, b) = admitted_pair().await;
    let ae = a
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let be = b
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let ac = ae.coordinator();
    // A contact address is not a data grant; this slice has no qualified
    // custom adapter, even when its peer identity is a current member.
    let signaling =
        iroh::EndpointAddr::new(be.endpoint().id()).with_addrs([iroh::TransportAddr::Custom(
            iroh_base::CustomAddr::from_parts(
                miopunch_native_probe::mqtt_transport::TRANSPORT_ID,
                b"signaling-only",
            ),
        )]);
    assert!(matches!(
        ac.connect(signaling, SessionPurpose::MemberBusiness, attempt())
            .await,
        Err(ConnectError::DataPathUnavailable)
    ));
    let mut config = b.resource_configuration().unwrap();
    config.udp_direct_disabled = true;
    b.save_resource_configuration(&config).unwrap();
    let denied = b
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let started = std::time::Instant::now();
    assert!(matches!(
        denied
            .coordinator()
            .connect(ae.addr(), SessionPurpose::MemberBusiness, attempt())
            .await,
        Err(ConnectError::DataPathUnavailable)
    ));
    // Cancellation wins even a planning refusal, without touching the node.
    let cancelled = attempt();
    cancelled.cancellation.cancel();
    assert!(matches!(
        ac.connect(be.addr(), SessionPurpose::MemberBusiness, cancelled)
            .await,
        Err(ConnectError::Cancelled)
    ));
    let bc = be.coordinator();
    let (out, accepted) = tokio::join!(
        ac.connect(be.addr(), SessionPurpose::MemberBusiness, attempt()),
        async {
            bc.accept(
                be.endpoint().accept().await.unwrap(),
                SessionPurpose::MemberBusiness,
                attempt(),
            )
            .await
        }
    );
    let (out, accepted) = (out.unwrap(), accepted.unwrap());
    streams(&out, &accepted).await;
    let caller_ms = started.elapsed().as_millis();
    let sockets: Vec<_> = ae
        .addr()
        .ip_addrs()
        .chain(be.addr().ip_addrs())
        .copied()
        .collect();
    out.close(0u32.into(), b"done");
    accepted.close(0u32.into(), b"done");
    drop((out, accepted, ac, bc));
    let cleanup = std::time::Instant::now();
    tokio::join!(ae.close(), be.close(), denied.close());
    tokio::join!(a.close(), b.close());
    let immediate = socket_release(&sockets);
    let cleanup_ms = cleanup.elapsed().as_millis();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let late = socket_release(&sockets);
    assert!(late.iter().all(|released| *released));
    println!(
        "{}",
        serde_json::json!({"event":"unified-data-denied","ordinaryHandleReturned":false,"signalingPromoted":false,"disabledUdpRejected":true,"healthyStreamMatched":true,"callerMs":caller_ms,"cleanupMs":cleanup_ms,"socketReleasedImmediately":immediate,"socketReleasedLate":late})
    );
}

#[derive(Clone, Debug)]
struct CaptureRestrictedConnection(Arc<Mutex<Option<iroh::endpoint::WeakConnectionHandle>>>);
impl EndpointHooks for CaptureRestrictedConnection {
    async fn before_data_release(
        &self,
        _: &Endpoint,
        connection: &Connection,
    ) -> std::io::Result<()> {
        if connection.alpn() == MEMBER_ALPN {
            *self.0.lock().unwrap() = Some(connection.weak_handle());
        }
        Ok(())
    }
}

/// The adversarial member delays its valid readiness until the already-probed
/// native route has retired. Real TCP carries only the remaining restricted
/// private control on the same QUIC M, not a replacement handshake.
#[derive(Clone, Debug)]
struct RetireProbedRouteBeforeReady {
    remote_endpoint: Endpoint,
    remote_control: iroh_base::CustomAddr,
    remote_connection: Arc<Mutex<Option<iroh::endpoint::WeakConnectionHandle>>>,
    observation: Arc<Mutex<Option<serde_json::Value>>>,
}
impl EndpointHooks for RetireProbedRouteBeforeReady {
    async fn before_data_release(
        &self,
        endpoint: &Endpoint,
        connection: &Connection,
    ) -> std::io::Result<()> {
        if connection.alpn() != MEMBER_ALPN {
            return Ok(());
        }
        let (mut send, mut recv) = connection.open_bi().await.map_err(std::io::Error::other)?;
        let mut hello = [0; 30];
        hello[..12].copy_from_slice(&[0, 28, b'M', b'U', b'C', b'1', 1, 1, 1, 1, 1, 1]);
        hello[14..].copy_from_slice(&[9; 16]);
        send.write_all(&hello).await?;
        let mut peer = [0; 30];
        recv.read_exact(&mut peer)
            .await
            .map_err(std::io::Error::other)?;
        let mut ready = [0; 17];
        recv.read_exact(&mut ready)
            .await
            .map_err(std::io::Error::other)?;
        assert_eq!(
            ready,
            [1, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9],
            "server already completed its native probe"
        );
        let remote = self
            .remote_connection
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .upgrade()
            .unwrap();
        let local_control = endpoint
            .custom_addrs()
            .into_iter()
            .find(|address| address.id() == miopunch_native_probe::tcp_transport::DATA_TRANSPORT_ID)
            .unwrap();
        endpoint
            .open_control_path(connection, self.remote_control.clone())
            .await?;
        tokio::time::timeout(Duration::from_secs(2), async {
            let mut local_probe = None;
            let mut remote_probe = None;
            loop {
                if local_probe.is_none() {
                    local_probe = connection
                        .paths()
                        .iter()
                        .find(|path| {
                            path.remote_addr()
                                == &iroh::TransportAddr::Custom(self.remote_control.clone())
                        })
                        .and_then(|path| path.start_probe().ok());
                }
                if remote_probe.is_none() {
                    remote_probe = remote
                        .paths()
                        .iter()
                        .find(|path| {
                            path.remote_addr()
                                == &iroh::TransportAddr::Custom(local_control.clone())
                        })
                        .and_then(|path| path.start_probe().ok());
                }
                if local_probe
                    .as_ref()
                    .is_some_and(|probe| probe.is_acknowledged() == Ok(true))
                    && remote_probe
                        .as_ref()
                        .is_some_and(|probe| probe.is_acknowledged() == Ok(true))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("real alternate private control path gets fresh authenticated ACK");
        endpoint
            .test_limit_transport_paths(
                connection,
                vec![iroh::TransportAddr::Custom(self.remote_control.clone())],
            )
            .await?;
        self.remote_endpoint
            .test_limit_transport_paths(&remote, vec![iroh::TransportAddr::Custom(local_control)])
            .await?;
        let retired: Vec<_> = remote
            .paths()
            .iter()
            .filter(|path| path.is_ip())
            .map(|path| {
                let id = path.id().to_string();
                path.close().unwrap();
                id
            })
            .collect();
        for path in connection.paths().iter().filter(|path| path.is_ip()) {
            let _ = path.close();
        }
        let control_live = remote.close_reason().is_none() && connection.close_reason().is_none();
        assert!(
            control_live,
            "retiring a path keeps the restricted original M alive"
        );
        *self.observation.lock().unwrap() =
            Some(serde_json::json!({"event":"unified-retired-before-ready",
            "nativeProbeCompleted":true,"retiredPathIds":retired,"controlStillLive":control_live,
            "controlPath":"tcp-restricted","sameOriginalM":true}));
        let mut valid_ready = [0; 17];
        valid_ready[0] = 1;
        valid_ready[1..].copy_from_slice(&peer[14..]);
        send.write_all(&valid_ready).await?;
        send.finish()?;
        connection.closed().await;
        Err(std::io::Error::other("adversarial readiness attempt ended"))
    }
}

#[tokio::test]
async fn a_retired_native_probe_cannot_release_an_ordinary_handle_through_live_private_control() {
    let (_dir, a, b) = admitted_pair().await;
    let remote_connection = Arc::new(Mutex::new(None));
    let observation = Arc::new(Mutex::new(None));
    let bt = Arc::new(miopunch_native_probe::tcp_transport::Traffic::default());
    let at = Arc::new(miopunch_native_probe::tcp_transport::Traffic::default());
    let btcp =
        miopunch_native_probe::tcp_transport::TcpTransport::new(Ipv4Addr::LOCALHOST, bt.clone());
    let be = b
        .bind_configured_endpoint_with(
            Endpoint::builder(iroh::endpoint::presets::Minimal)
                .bind_addr("127.0.0.1:0".parse::<SocketAddr>().unwrap())
                .unwrap()
                .add_custom_transport(Arc::new(btcp))
                .hooks(CaptureRestrictedConnection(remote_connection.clone())),
        )
        .await
        .unwrap();
    let control = be
        .endpoint()
        .custom_addrs()
        .into_iter()
        .find(|address| address.id() == miopunch_native_probe::tcp_transport::DATA_TRANSPORT_ID)
        .unwrap();
    let atcp =
        miopunch_native_probe::tcp_transport::TcpTransport::new(Ipv4Addr::LOCALHOST, at.clone());
    let ae = a
        .bind_configured_endpoint_with(
            Endpoint::builder(iroh::endpoint::presets::Minimal)
                .bind_addr("127.0.0.1:0".parse::<SocketAddr>().unwrap())
                .unwrap()
                .add_custom_transport(Arc::new(atcp))
                .hooks(RetireProbedRouteBeforeReady {
                    remote_endpoint: be.endpoint().clone(),
                    remote_control: control,
                    remote_connection: remote_connection.clone(),
                    observation: observation.clone(),
                }),
        )
        .await
        .unwrap();
    let (ac, bc) = (ae.coordinator(), be.coordinator());
    let started = std::time::Instant::now();
    let (out, rejected) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(
            ac.connect(be.addr(), SessionPurpose::MemberBusiness, attempt()),
            async {
                let result = bc
                    .accept(
                        be.endpoint().accept().await.unwrap(),
                        SessionPurpose::MemberBusiness,
                        attempt(),
                    )
                    .await;
                if let Ok(connection) = &result {
                    connection.close(0u32.into(), b"fixture caught premature ordinary release");
                }
                result
            }
        )
    })
    .await
    .unwrap();
    let observed = observation
        .lock()
        .unwrap()
        .take()
        .expect("retirement reached after actual native probe");
    println!("{observed}");
    assert!(out.is_err());
    assert!(
        matches!(rejected, Err(ConnectError::DataPathUnavailable)),
        "retired exact native route must refuse ordinary SDK handle; ordinaryHandleReturned={} error={:?}",
        rejected.is_ok(),
        rejected.as_ref().err().map(ToString::to_string)
    );
    assert!(
        !remote_connection
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .is_open()
    );
    let caller_ms = started.elapsed().as_millis();
    let sockets: Vec<_> = ae
        .addr()
        .ip_addrs()
        .chain(be.addr().ip_addrs())
        .copied()
        .collect();
    let tcp_sockets: Vec<_> = ae
        .endpoint()
        .custom_addrs()
        .into_iter()
        .chain(be.endpoint().custom_addrs())
        .filter(|address| address.id() == miopunch_native_probe::tcp_transport::DATA_TRANSPORT_ID)
        .map(|address| {
            miopunch_native_probe::tcp_transport::TcpTransport::socket_addr(&address).unwrap()
        })
        .collect();
    drop((out, rejected, ac, bc));
    let cleanup = std::time::Instant::now();
    tokio::join!(ae.close(), be.close());
    tokio::join!(a.close(), b.close());
    let immediate = socket_release(&sockets);
    let tcp_immediate: Vec<_> = tcp_sockets
        .iter()
        .map(|address| std::net::TcpListener::bind(address).is_ok())
        .collect();
    let cleanup_ms = cleanup.elapsed().as_millis();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let late = socket_release(&sockets);
    let tcp_late: Vec<_> = tcp_sockets
        .iter()
        .map(|address| std::net::TcpListener::bind(address).is_ok())
        .collect();
    assert!(late.iter().all(|released| *released));
    assert!(tcp_late.iter().all(|released| *released));
    use std::sync::atomic::Ordering;
    let outstanding: Vec<_> = [&at, &bt]
        .into_iter()
        .map(|traffic| {
            (
                traffic.outbound_outstanding.load(Ordering::Acquire),
                traffic.inbound_outstanding.load(Ordering::Acquire),
            )
        })
        .collect();
    assert!(
        outstanding.iter().all(|counts| *counts == (0, 0)),
        "control queues drained on owned endpoint close"
    );
    println!(
        "{}",
        serde_json::json!({"event":"unified-retirement-refused","ordinaryHandleReturned":false,
        "callerMs":caller_ms,"cleanupMs":cleanup_ms,"socketReleasedImmediately":immediate,"socketReleasedLate":late,
        "tcpListenerReleasedImmediately":tcp_immediate,"tcpListenerReleasedLate":tcp_late,"controlQueuesOutstanding":outstanding})
    );
}

#[tokio::test]
async fn valid_browser_capabilities_without_mutual_data_are_unavailable_not_invalid_control() {
    let (_directory, native, browser_identity_fixture) = admitted_pair().await;
    let native_endpoint = native
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let browser_wire = browser_identity_fixture
        .bind_configured_endpoint_with(
            Endpoint::builder(iroh::endpoint::presets::Minimal)
                .bind_addr("127.0.0.1:0".parse::<SocketAddr>().unwrap())
                .unwrap()
                .hooks(ControlEnvelopeFixture(4)),
        )
        .await
        .unwrap();
    let coordinator = native_endpoint.coordinator();
    let started = std::time::Instant::now();
    let (client, server) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(
            browser_wire
                .endpoint()
                .connect(native_endpoint.addr(), MEMBER_ALPN),
            async {
                coordinator
                    .accept(
                        native_endpoint.endpoint().accept().await.unwrap(),
                        SessionPurpose::MemberBusiness,
                        attempt(),
                    )
                    .await
            }
        )
    })
    .await
    .unwrap();
    assert!(client.is_err());
    let unavailable = matches!(&server, Err(ConnectError::DataPathUnavailable));
    println!(
        "{}",
        serde_json::json!({"event":"unified-browser-capabilities-negative",
        "scope":"real protected member wire; no browser or RTC runtime qualification",
        "ordinaryHandleReturned":server.is_ok(),"dataPathUnavailable":unavailable,
        "failure":server.as_ref().err().map(ToString::to_string),"callerMs":started.elapsed().as_millis()})
    );
    drop(coordinator);
    tokio::join!(browser_wire.close(), native_endpoint.close());
    tokio::join!(native.close(), browser_identity_fixture.close());
    assert!(
        unavailable,
        "well-formed browser capabilities require stable DataPathUnavailable: {server:?}"
    );
}

#[derive(Clone, Debug)]
struct PeerClosesPendingMember {
    gate: miopunch_native_probe::admission_gate::AdmissionGate,
    close_self: bool,
    close: Arc<tokio::sync::Notify>,
    entered: Arc<Mutex<Option<tokio::sync::oneshot::Sender<bool>>>>,
}
impl EndpointHooks for PeerClosesPendingMember {
    async fn before_data_release(
        &self,
        _endpoint: &Endpoint,
        connection: &Connection,
    ) -> std::io::Result<()> {
        if connection.alpn() != MEMBER_ALPN {
            return Ok(());
        }
        if self.close_self {
            self.close.notified().await;
            connection.close(0u32.into(), b"controlled pending peer ended");
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "controlled pending peer ended",
            ));
        }
        let initially_allowed = self.gate.allows(connection, ProtocolScope::Member);
        self.entered
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .send(initially_allowed)
            .unwrap();
        connection.closed().await;
        Ok(())
    }
}

#[tokio::test]
async fn proved_pending_peer_close_is_data_unavailable_not_identity_rejection() {
    let (_directory, a, b) = admitted_pair().await;
    let close = Arc::new(tokio::sync::Notify::new());
    let (entered, receiver) = tokio::sync::oneshot::channel();
    let entered = Arc::new(Mutex::new(Some(entered)));
    let server = a
        .bind_configured_endpoint_with(
            Endpoint::builder(iroh::endpoint::presets::Minimal)
                .bind_addr("127.0.0.1:0".parse::<SocketAddr>().unwrap())
                .unwrap()
                .hooks(PeerClosesPendingMember {
                    gate: a.admission_gate(),
                    close_self: false,
                    close: close.clone(),
                    entered: entered.clone(),
                }),
        )
        .await
        .unwrap();
    let client = b
        .bind_configured_endpoint_with(
            Endpoint::builder(iroh::endpoint::presets::Minimal)
                .bind_addr("127.0.0.1:0".parse::<SocketAddr>().unwrap())
                .unwrap()
                .hooks(PeerClosesPendingMember {
                    gate: b.admission_gate(),
                    close_self: true,
                    close: close.clone(),
                    entered,
                }),
        )
        .await
        .unwrap();
    let dialer = client.coordinator();
    let listener = server.coordinator();
    let address = server.addr();
    let result = async {
        let dialing = dialer.connect(address, SessionPurpose::MemberBusiness, attempt());
        let listening = async {
            listener
                .accept(
                    server.endpoint().accept().await.unwrap(),
                    SessionPurpose::MemberBusiness,
                    attempt(),
                )
                .await
        };
        let cancellation = async {
            assert!(
                receiver.await.unwrap(),
                "existing TLS/member proof must be valid before the close"
            );
            close.notify_one();
        };
        let (client, server, _) = tokio::join!(dialing, listening, cancellation);
        (client, server)
    };
    let (client_result, server_result) = tokio::time::timeout(Duration::from_secs(6), result)
        .await
        .unwrap();
    let correctly_unavailable = matches!(&server_result, Err(ConnectError::DataPathUnavailable));
    assert!(client_result.is_err());
    println!(
        "{}",
        serde_json::json!({"event":"unified-peer-close-negative","initialMemberGate":true,
        "peerClosedPendingM":true,"dataPathUnavailable":correctly_unavailable,"ordinaryHandleReturned":server_result.is_ok(),
        "failure":server_result.as_ref().err().map(ToString::to_string)})
    );
    drop((dialer, listener));
    tokio::join!(client.close(), server.close());
    tokio::join!(a.close(), b.close());
    assert!(
        correctly_unavailable,
        "closed proved M is not an identity violation: {server_result:?}"
    );
}

#[cfg(feature = "rtc-probe")]
#[tokio::test]
async fn stale_rtc_candidate_attempt_on_protected_member_m_is_rejected_before_allocation() {
    let (_directory, native, peer) = admitted_pair().await;
    let rtc = miopunch_native_probe::rtc_transport::RtcTransport::new();
    let native_endpoint = native
        .bind_configured_endpoint_with_rtc(
            Endpoint::builder(iroh::endpoint::presets::Minimal)
                .bind_addr("127.0.0.1:0".parse::<SocketAddr>().unwrap())
                .unwrap(),
            rtc.clone(),
        )
        .await
        .unwrap();
    let peer_endpoint = peer
        .bind_configured_endpoint_with(
            Endpoint::builder(iroh::endpoint::presets::Minimal)
                .bind_addr("127.0.0.1:0".parse::<SocketAddr>().unwrap())
                .unwrap()
                .hooks(ControlEnvelopeFixture(5)),
        )
        .await
        .unwrap();
    let coordinator = native_endpoint.coordinator();
    let started = std::time::Instant::now();
    let (dialing, accepted) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(
            peer_endpoint
                .endpoint()
                .connect(native_endpoint.addr(), MEMBER_ALPN),
            async {
                coordinator
                    .accept(
                        native_endpoint.endpoint().accept().await.unwrap(),
                        SessionPurpose::MemberBusiness,
                        attempt(),
                    )
                    .await
            }
        )
    })
    .await
    .unwrap();
    let correctly_invalid = matches!(&accepted, Err(ConnectError::InvalidControl));
    assert!(dialing.is_err());
    println!(
        "{}",
        serde_json::json!({"event":"unified-stale-rtc-attempt",
        "scope":"real protected member M and native RTC preparation; no browser connectivity claim",
        "ordinaryHandleReturned":accepted.is_ok(),"invalidControl":correctly_invalid,
        "activeRtcAssociations":rtc.active_associations(),"activeRtcPumps":rtc.active_pumps(),
        "callerMs":started.elapsed().as_millis()})
    );
    drop(coordinator);
    tokio::join!(native_endpoint.close(), peer_endpoint.close());
    tokio::join!(native.close(), peer.close());
    assert!(
        correctly_invalid,
        "stale protected attempt must be InvalidControl: {accepted:?}"
    );
    assert_eq!(rtc.active_associations(), 0);
    assert_eq!(rtc.active_pumps(), 0);
}

#[cfg(feature = "rtc-probe")]
#[derive(Debug)]
struct Gathered(tokio::sync::mpsc::Sender<()>);
#[cfg(feature = "rtc-probe")]
#[async_trait::async_trait]
impl webrtc::peer_connection::PeerConnectionEventHandler for Gathered {
    async fn on_ice_gathering_state_change(
        &self,
        state: webrtc::peer_connection::RTCIceGatheringState,
    ) {
        if state == webrtc::peer_connection::RTCIceGatheringState::Complete {
            let _ = self.0.try_send(());
        }
    }
}

/// A controlled wire peer reaches the real native RTC answer allocation, then
/// holds the valid answer until the SDK caller cancels or the budget expires.
/// This exercises native ownership; it makes no browser-event qualification.
#[cfg(feature = "rtc-probe")]
#[derive(Clone)]
struct DeferredRtcAnswer {
    gate: miopunch_native_probe::admission_gate::AdmissionGate,
    offer: webrtc::peer_connection::RTCSessionDescription,
    entered: Arc<
        Mutex<Option<tokio::sync::oneshot::Sender<webrtc::peer_connection::RTCSessionDescription>>>,
    >,
}
#[cfg(feature = "rtc-probe")]
impl std::fmt::Debug for DeferredRtcAnswer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("DeferredRtcAnswer")
    }
}
#[cfg(feature = "rtc-probe")]
impl EndpointHooks for DeferredRtcAnswer {
    async fn before_data_release(
        &self,
        endpoint: &Endpoint,
        connection: &Connection,
    ) -> std::io::Result<()> {
        use miopunch_native_probe::{rtc_association as protocol, rtc_wire};
        if connection.alpn() != MEMBER_ALPN {
            return Ok(());
        }
        let (mut send, mut recv) = connection.open_bi().await?;
        let nonce = [7; 16];
        let mut frame = [0; 28];
        frame[..4].copy_from_slice(b"MUC1");
        frame[4..11].copy_from_slice(&[1, 1, 2, 1, 8, 8, 1]);
        frame[12..].copy_from_slice(&nonce);
        send.write_all(&28u16.to_be_bytes()).await?;
        send.write_all(&frame).await?;
        let mut peer = [0; 30];
        recv.read_exact(&mut peer)
            .await
            .map_err(std::io::Error::other)?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"miopunch/unified-attempt/1");
        hasher.update(&nonce);
        hasher.update(&peer[14..]);
        let attempt = hex::encode(&hasher.finalize().as_bytes()[..16]);
        let proof = self
            .gate
            .proof_for(connection)
            .ok_or_else(|| std::io::Error::other("fixture member proof absent"))?;
        let context = protocol::attempt_context(
            &attempt,
            proof.scope().network_id(),
            proof.scope().secret_generation(),
            &endpoint.id().to_string(),
            &connection.remote_id().to_string(),
        );
        let binding = miopunch_native_probe::rtc_probe::binding(connection, context.as_bytes())
            .map_err(std::io::Error::other)?;
        let (mut candidate, mut answer) = connection.open_bi().await?;
        rtc_wire::write_frame(&mut candidate, &serde_json::json!({
            "protocol":protocol::UNIFIED_RTC_PROTOCOL,"attempt":attempt,"context":context,"binding":binding,
            "description":self.offer,"channel":protocol::datagram_channel_parameters(),"route":hex::encode([9;16]),
            "quicDialer":endpoint.id().to_string(),"rtcOfferer":endpoint.id().to_string(),"rtcAnswerer":connection.remote_id().to_string(),
            "remainingMs":1000,"authorizationRevision":0,"resourceRevision":0,
        })).await?;
        let value = rtc_wire::read_frame(&mut answer).await?;
        let bound =
            value["kind"] == "answer" && value["attempt"] == attempt && value["binding"] == binding;
        assert!(
            bound,
            "the deferred answer belongs to this current protected attempt"
        );
        let description =
            serde_json::from_value(value["description"].clone()).map_err(std::io::Error::other)?;
        self.entered
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .send(description)
            .unwrap();
        // The caller holds the valid answer outside this hook. Cancelling the
        // SDK attempt drops this future before any continuation can apply it.
        connection.closed().await;
        Err(std::io::Error::other("controlled late RTC answer retired"))
    }
}

#[cfg(feature = "rtc-probe")]
#[tokio::test]
async fn cancel_or_timeout_after_real_rtc_answer_retires_candidate_and_late_answer_cannot_revive_it()
 {
    use miopunch_native_probe::rtc_association as protocol;
    use webrtc::{
        data_channel::RTCDataChannelInit,
        peer_connection::{PeerConnection, PeerConnectionBuilder},
    };
    let (_directory, a, b) = admitted_pair().await;
    // Install the real RTC carrier before creating the healthy M. Adding a
    // carrier later intentionally retires old Member sessions in AdmissionGate.
    let ae = a
        .bind_configured_endpoint_with_rtc(
            Endpoint::builder(iroh::endpoint::presets::Minimal)
                .bind_addr("127.0.0.1:0".parse::<SocketAddr>().unwrap())
                .unwrap(),
            miopunch_native_probe::rtc_transport::RtcTransport::new(),
        )
        .await
        .unwrap();
    let be = b
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let (ac, bc) = (ae.coordinator(), be.coordinator());
    let (healthy, healthy_peer) = tokio::join!(
        ac.connect(be.addr(), SessionPurpose::MemberBusiness, attempt()),
        async {
            bc.accept(
                be.endpoint().accept().await.unwrap(),
                SessionPurpose::MemberBusiness,
                attempt(),
            )
            .await
        }
    );
    let (healthy, healthy_peer) = (healthy.unwrap(), healthy_peer.unwrap());
    for cancel in [true, false] {
        let (gathered, mut gathered_rx) = tokio::sync::mpsc::channel(1);
        let pc: Arc<dyn PeerConnection> = Arc::new(
            PeerConnectionBuilder::new()
                .with_udp_addrs(vec!["127.0.0.1:0".parse::<SocketAddr>().unwrap()])
                .with_handler(Arc::new(Gathered(gathered)))
                .build()
                .await
                .unwrap(),
        );
        let _channel = pc
            .create_data_channel(
                protocol::DATAGRAM_CHANNEL_LABEL,
                Some(RTCDataChannelInit {
                    ordered: false,
                    max_retransmits: Some(0),
                    negotiated: Some(0),
                    protocol: protocol::DATAGRAM_CHANNEL_PROTOCOL.into(),
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        pc.set_local_description(pc.create_offer(None).await.unwrap())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), gathered_rx.recv())
            .await
            .unwrap()
            .unwrap();
        let offer = pc.local_description().await.unwrap();
        let (entered, receiver) = tokio::sync::oneshot::channel();
        let rtc = miopunch_native_probe::rtc_transport::RtcTransport::new();
        let server = a
            .bind_configured_endpoint_with_rtc(
                Endpoint::builder(iroh::endpoint::presets::Minimal)
                    .bind_addr("127.0.0.1:0".parse::<SocketAddr>().unwrap())
                    .unwrap(),
                rtc.clone(),
            )
            .await
            .unwrap();
        let peer = b
            .bind_configured_endpoint_with(
                Endpoint::builder(iroh::endpoint::presets::Minimal)
                    .bind_addr("127.0.0.1:0".parse::<SocketAddr>().unwrap())
                    .unwrap()
                    .hooks(DeferredRtcAnswer {
                        gate: b.admission_gate(),
                        offer,
                        entered: Arc::new(Mutex::new(Some(entered))),
                    }),
            )
            .await
            .unwrap();
        let token = tokio_util::sync::CancellationToken::new();
        let deadline = n0_future::time::Instant::now() + Duration::from_secs(5);
        let client_endpoint = peer.endpoint().clone();
        let address = server.addr();
        let cancellation = token.clone();
        let dialing = tokio::spawn(async move {
            client_endpoint
                .connect_with_opts(
                    address,
                    MEMBER_ALPN,
                    iroh::endpoint::ConnectOptions::new()
                        .with_caller_deadline(deadline)
                        .with_attempt_cancellation(cancellation),
                )
                .await
                .unwrap()
                .await
        });
        let coordinator = server.coordinator();
        let endpoint = server.endpoint().clone();
        let accepting = tokio::spawn(async move {
            coordinator
                .accept(
                    endpoint.accept().await.unwrap(),
                    SessionPurpose::MemberBusiness,
                    AttemptContext::new(deadline, tokio_util::sync::CancellationToken::new()),
                )
                .await
        });
        let late_answer = tokio::time::timeout(Duration::from_secs(3), receiver)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            rtc.active_associations(),
            1,
            "the real answer reserved an owned association"
        );
        assert_eq!(rtc.active_pumps(), 0, "withheld answer cannot open a pump");
        let started = std::time::Instant::now();
        if cancel {
            token.cancel();
        }
        let client_result = tokio::time::timeout(Duration::from_secs(3), dialing)
            .await
            .unwrap()
            .unwrap();
        let server_result = tokio::time::timeout(Duration::from_secs(3), accepting)
            .await
            .unwrap()
            .unwrap();
        assert!(
            client_result.is_err() && server_result.is_err(),
            "ordinary handles must remain unavailable"
        );
        pc.close().await.unwrap();
        let late_rejected = pc.set_remote_description(late_answer).await.is_err();
        assert!(
            late_rejected,
            "a late answer cannot reactivate the closed fixture PC"
        );
        tokio::time::timeout(Duration::from_secs(2), async {
            while rtc.active_associations() != 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(rtc.active_pumps(), 0);
        streams(&healthy, &healthy_peer).await;
        println!(
            "{}",
            serde_json::json!({"event":"unified-rtc-abort-after-answer","cancel":cancel,
            "scope":"real native SDK RTC candidate with controlled wire peer; no browser callback claim",
            "answerBoundToCurrentAttempt":true,"ordinaryHandlesReturned":false,"lateAnswerRejected":true,
            "activeRtcAssociations":rtc.active_associations(),"activeRtcPumps":rtc.active_pumps(),
            "healthyOriginalStreamMatched":true,"afterAnswerMs":started.elapsed().as_millis()})
        );
        pc.close().await.unwrap();
        tokio::join!(server.close(), peer.close());
    }
    healthy.close(0u32.into(), b"done");
    healthy_peer.close(0u32.into(), b"done");
    drop((healthy, healthy_peer, ac, bc));
    tokio::join!(ae.close(), be.close());
    tokio::join!(a.close(), b.close());
}

#[path = "support/contact_broker.rs"]
#[allow(dead_code)]
mod contact_override_broker;

#[tokio::test]
async fn contact_override_cannot_borrow_another_facility_permission_at_same_native_url() {
    use miopunch_native_probe::{
        mqtt_datagram::MqttFacility,
        mqtt_transport::{MqttTransport, Traffic},
        resources::{Resource, ResourceUse},
    };
    use std::{collections::BTreeSet, sync::atomic::Ordering};

    let dir = tempfile::tempdir().unwrap();
    let node = NativeNetwork::create(dir.path().join("node"))
        .await
        .unwrap();
    let broker = contact_override_broker::Broker::new().await;
    let native_url = format!("mqtt://{}/", broker.address);
    let a = MqttFacility::new(
        "facility-a".into(),
        native_url.clone(),
        "wss://a.invalid/mqtt".into(),
    )
    .unwrap();
    let b = MqttFacility::new(
        "facility-b".into(),
        native_url.clone(),
        "wss://b.invalid/mqtt".into(),
    )
    .unwrap();
    let unknown = MqttFacility::new(
        "facility-c".into(),
        native_url.clone(),
        "wss://c.invalid/mqtt".into(),
    )
    .unwrap();
    let remapped_b = MqttFacility::new(
        b.logical_id.clone(),
        native_url.clone(),
        "wss://changed-b.invalid/mqtt".into(),
    )
    .unwrap();
    let mut config = node.resource_configuration().unwrap();
    config.replacement = Some(vec![Resource {
        url: native_url,
        uses: BTreeSet::from([ResourceUse::Signaling]),
        limitation: "owned loopback permission regression".into(),
    }]);
    config.mqtt_facilities = vec![a.clone(), b.clone()];
    let reserve = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let udp_address = reserve.local_addr().unwrap();
    drop(reserve);
    let builder = || {
        Endpoint::builder(iroh::endpoint::presets::Minimal)
            .bind_addr(udp_address)
            .unwrap()
    };
    let mut denied_cases = Vec::new();
    for (name, disabled, facility) in [
        ("wss-disabled", a.wss_url.clone(), Some(&a)),
        ("logical-id-disabled", a.logical_id.clone(), Some(&a)),
        ("unsaved-descriptor", a.wss_url.clone(), Some(&unknown)),
        (
            "same-id-changed-wss-descriptor",
            a.wss_url.clone(),
            Some(&remapped_b),
        ),
        (
            "raw-cannot-borrow-mapped-permission",
            a.wss_url.clone(),
            None,
        ),
    ] {
        config.disabled = BTreeSet::from([(disabled, ResourceUse::Signaling)]);
        node.save_resource_configuration(&config).unwrap();
        let traffic = Arc::new(Traffic::default());
        // Empty constructor policy deliberately defers consent to the actual
        // saved-policy SDK bind entrance rather than testing constructor refusal.
        let contact = Arc::new(match facility {
            Some(facility) => {
                MqttTransport::from_facility(facility, &[], traffic.clone(), None).unwrap()
            }
            None => MqttTransport::new(broker.address, traffic.clone()),
        });
        let before = broker.connections.load(Ordering::SeqCst);
        let result = node
            .bind_configured_endpoint_with_contact(builder(), contact.clone())
            .await;
        let denied = result.as_ref().err().is_some_and(|error| {
            matches!(error, miopunch_native_probe::native_network::NodeError::Io(io) if io.kind() == std::io::ErrorKind::PermissionDenied)
        });
        let no_driver_started = traffic.snapshot()["driver_active"] == 0;
        if let Ok(endpoint) = result {
            // Retire the incorrectly accepted red path before any assertion.
            let _ = tokio::time::timeout(Duration::from_secs(1), contact.wait_ready()).await;
            endpoint.close().await;
            contact.wait_driver_closed().await.unwrap();
        }
        let sockets = broker.no_sockets().await;
        let no_broker_socket = broker.connections.load(Ordering::SeqCst) == before;
        let udp_released = std::net::UdpSocket::bind(udp_address).is_ok();
        println!(
            "{}",
            serde_json::json!({"event":"contact-override-permission","case":name,"denied":denied,"noDriverStarted":no_driver_started,"noBrokerSocket":no_broker_socket,"brokerImmediateEmpty":sockets.immediate_empty,"brokerLateEmpty":sockets.late_empty,"udpReleased":udp_released})
        );
        denied_cases.push((
            name,
            denied && no_driver_started && no_broker_socket && udp_released,
        ));
    }
    config.disabled = BTreeSet::from([(a.wss_url.clone(), ResourceUse::Signaling)]);
    node.save_resource_configuration(&config).unwrap();
    let permitted = Arc::new(
        MqttTransport::from_facility(&b, &[], Arc::new(Traffic::default()), None).unwrap(),
    );
    let endpoint = node
        .bind_configured_endpoint_with_contact(builder(), permitted.clone())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), permitted.wait_ready())
        .await
        .unwrap()
        .unwrap();
    let allowed_b_ready = permitted.route().is_ok();
    endpoint.close().await;
    permitted.wait_driver_closed().await.unwrap();
    let b_retired = broker.no_sockets().await;
    let udp_released_after_b = std::net::UdpSocket::bind(udp_address).is_ok();

    // Preserve the explicit URL-policy legacy path when there is no facility
    // mapping to borrow, and still reject that raw resource once disabled.
    config.mqtt_facilities.clear();
    config.disabled.clear();
    node.save_resource_configuration(&config).unwrap();
    let raw = Arc::new(MqttTransport::new(
        broker.address,
        Arc::new(Traffic::default()),
    ));
    let endpoint = node
        .bind_configured_endpoint_with_contact(builder(), raw.clone())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), raw.wait_ready())
        .await
        .unwrap()
        .unwrap();
    let unmapped_raw_ready = raw.route().is_ok();
    endpoint.close().await;
    raw.wait_driver_closed().await.unwrap();
    let raw_retired = broker.no_sockets().await;
    config.disabled = BTreeSet::from([(
        format!("mqtt://{}/", broker.address),
        ResourceUse::Signaling,
    )]);
    node.save_resource_configuration(&config).unwrap();
    let raw_denied = node
        .bind_configured_endpoint_with_contact(
            builder(),
            Arc::new(MqttTransport::new(
                broker.address,
                Arc::new(Traffic::default()),
            )),
        )
        .await
        .is_err();
    node.close().await;
    broker.close().await;
    println!(
        "{}",
        serde_json::json!({"event":"contact-override-neighbor-policy","allowedBReady":allowed_b_ready,"allowedBRetiredLate":b_retired.late_empty,"udpReleasedAfterB":udp_released_after_b,"unmappedRawReady":unmapped_raw_ready,"unmappedRawRetiredLate":raw_retired.late_empty,"disabledRawDenied":raw_denied})
    );
    assert!(
        denied_cases.iter().all(|(_, passed)| *passed),
        "saved policy bypass: {denied_cases:?}"
    );
    assert!(allowed_b_ready && b_retired.late_empty && udp_released_after_b);
    assert!(unmapped_raw_ready && raw_retired.late_empty && raw_denied);
}

#[path = "support/relay_contact_broker.rs"]
mod relay_contact_broker;

#[tokio::test]
async fn late_home_relay_connection_after_mqtt_ready_preserves_the_same_sdk_member() {
    use miopunch_native_probe::{
        mqtt_transport::{MqttTransport, Traffic},
        resources::{Resource, ResourceUse},
    };
    use n0_watcher::Watcher;
    use std::collections::BTreeSet;
    let mut outcomes = Vec::new();
    for cold in [false, true] {
        let (_directory, a, b) = admitted_pair().await;
        let broker = relay_contact_broker::Broker::new().await;
        let mut relay_config = iroh_relay::server::ServerConfig::default();
        relay_config.relay = Some(iroh_relay::server::RelayConfig::new((
            Ipv4Addr::LOCALHOST,
            0,
        )));
        let relay = iroh_relay::server::Server::spawn(relay_config)
            .await
            .unwrap();
        let backend = relay.http_url().unwrap();
        let gate = relay_contact_broker::RelayGate::new(
            (Ipv4Addr::LOCALHOST, backend.port().unwrap()).into(),
        )
        .await;
        let relay_url: iroh::RelayUrl = format!("http://{}/", gate.address).parse().unwrap();
        for node in [&a, &b] {
            let mut config = node.resource_configuration().unwrap();
            config.replacement = Some(vec![
                Resource {
                    url: format!("mqtt://{}/", broker.address),
                    uses: BTreeSet::from([ResourceUse::Signaling]),
                    limitation: "owned routed MQTT SDK fault fixture".into(),
                },
                Resource {
                    url: relay_url.to_string(),
                    uses: BTreeSet::from([ResourceUse::DataRelay]),
                    limitation: "owned actual Iroh relay timing fixture".into(),
                },
            ]);
            config.udp_direct_disabled = true;
            config.tcp_direct_disabled = true;
            config.member_data_relay_disabled = true;
            config.disabled_uses.insert(ResourceUse::Observation);
            node.save_resource_configuration(&config).unwrap();
        }
        let bt = Arc::new(MqttTransport::new(
            broker.address,
            Arc::new(Traffic::default()),
        ));
        let capture = Arc::new(Mutex::new(None));
        let be = b
            .bind_configured_endpoint_with_contact(
                Endpoint::builder(iroh::endpoint::presets::Minimal)
                    .hooks(CaptureRestrictedConnection(capture.clone())),
                bt.clone(),
            )
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), bt.wait_ready())
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), be.endpoint().online())
            .await
            .unwrap();
        if cold {
            gate.hold_new();
        }
        let at = Arc::new(MqttTransport::new(
            broker.address,
            Arc::new(Traffic::default()),
        ));
        let ae = a
            .bind_configured_endpoint_with_contact(
                Endpoint::builder(iroh::endpoint::presets::Minimal),
                at.clone(),
            )
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), at.wait_ready())
            .await
            .unwrap()
            .unwrap();
        if !cold {
            tokio::time::timeout(Duration::from_secs(2), ae.endpoint().online())
                .await
                .unwrap();
        }
        let a_initial = ae.endpoint().addr().relay_urls().count();
        let b_initial = be.endpoint().addr().relay_urls().count();
        let initial_connected = [ae.endpoint(), be.endpoint()].map(|endpoint| {
            endpoint
                .home_relay_status()
                .get()
                .iter()
                .any(|status| status.is_connected())
        });
        let no_ip_sockets =
            ae.endpoint().bound_sockets().is_empty() && be.endpoint().bound_sockets().is_empty();
        let fixture_matched =
            a_initial == 1 && b_initial == 1 && initial_connected == [!cold, true] && no_ip_sockets;
        let target =
            iroh::EndpointAddr::new(be.endpoint().id()).with_addrs([iroh::TransportAddr::Custom(
                MqttTransport::address(&bt.route().unwrap()).unwrap(),
            )]);
        let started = std::time::Instant::now();
        let deadline = n0_future::time::Instant::now() + Duration::from_secs(2);
        let release = async {
            if cold {
                let _ = tokio::time::timeout(Duration::from_secs(1), async {
                    while capture.lock().unwrap().is_none() {
                        tokio::time::sleep(Duration::from_millis(2)).await;
                    }
                })
                .await;
                tokio::time::sleep(Duration::from_millis(200)).await;
                gate.release();
            }
        };
        let coordinator_a = ae.coordinator();
        let coordinator_b = be.coordinator();
        let (outgoing, incoming, _) = tokio::join!(
            coordinator_a.connect(
                target,
                SessionPurpose::MemberBusiness,
                AttemptContext::new(deadline, tokio_util::sync::CancellationToken::new())
            ),
            async {
                let incoming = tokio::time::timeout(Duration::from_secs(2), be.endpoint().accept())
                    .await
                    .map_err(|_| ConnectError::DeadlineExceeded)?
                    .ok_or(ConnectError::DataPathUnavailable)?;
                coordinator_b
                    .accept(
                        incoming,
                        SessionPurpose::MemberBusiness,
                        AttemptContext::new(deadline, tokio_util::sync::CancellationToken::new()),
                    )
                    .await
            },
            release,
        );
        let caller_ms = started.elapsed().as_millis();
        let success = outgoing.is_ok() && incoming.is_ok();
        let mut matched = false;
        let mut selected_relay = false;
        let mut gate_checked = false;
        if let (Ok(outgoing), Ok(incoming)) = (&outgoing, &incoming) {
            streams(outgoing, incoming).await;
            matched = outgoing.remote_id().as_bytes() == b.status().node_id.as_bytes()
                && incoming.remote_id().as_bytes() == a.status().node_id.as_bytes();
            selected_relay = [outgoing, incoming].iter().all(|connection| {
                connection
                    .paths()
                    .iter()
                    .any(|path| path.is_selected() && path.is_relay())
            });
            gate_checked = a.admission_gate().allows(outgoing, ProtocolScope::Member)
                && b.admission_gate().allows(incoming, ProtocolScope::Member);
            outgoing.close(0u32.into(), b"owned relay regression finished");
            incoming.close(0u32.into(), b"owned relay regression finished");
        }
        println!(
            "{}",
            serde_json::json!({"event":"cold-relay-sdk-result","scope":"two reopened native SDK members with owned MQTT and actual Iroh relay; no browser/public claim","coldHome":cold,"initialHomeCounts":[a_initial,b_initial],"initialHomeConnected":initial_connected,"noIpObservationSockets":no_ip_sockets,"fixtureMatched":fixture_matched,"endHomeCounts":[ae.endpoint().addr().relay_urls().count(),be.endpoint().addr().relay_urls().count()],"memberHandlesReturned":success,"identityMatched":matched,"gateChecked":gate_checked,"selectedRelay":selected_relay,"callerMs":caller_ms,"callerBudgetMs":2000,"clientError":outgoing.as_ref().err().map(ToString::to_string),"serverError":incoming.as_ref().err().map(ToString::to_string)})
        );
        drop((outgoing, incoming));
        capture.lock().unwrap().take();
        let cleanup = std::time::Instant::now();
        tokio::join!(ae.close(), be.close());
        let (driver_a, driver_b) = tokio::join!(at.wait_driver_closed(), bt.wait_driver_closed());
        driver_a.unwrap();
        driver_b.unwrap();
        tokio::join!(a.close(), b.close());
        let retirement = relay_contact_broker::retired(&broker, &gate).await;
        let driver_zero = at.traffic().snapshot()["driver_active"] == 0
            && bt.traffic().snapshot()["driver_active"] == 0;
        println!(
            "{}",
            serde_json::json!({"event":"cold-relay-sdk-cleanup","coldHome":cold,"cleanupMs":cleanup.elapsed().as_millis(),"socketsImmediateEmpty":retirement.0,"socketSettleMs":retirement.1,"socketsLateEmpty":retirement.2,"driversZero":driver_zero,"forced":false})
        );
        gate.close().await;
        broker.close().await;
        relay.shutdown().await.unwrap();
        outcomes.push((
            cold,
            success
                && matched
                && selected_relay
                && gate_checked
                && retirement.2
                && driver_zero
                && fixture_matched,
        ));
    }
    assert!(
        outcomes.iter().all(|(_, passed)| *passed),
        "a relay becoming available within the original caller budget must retain the original SDK member: {outcomes:?}"
    );
}

#[tokio::test]
async fn incoming_receipt_clock_survives_alpn_classification_and_reaches_admission_hook() {
    #[derive(Debug)]
    struct ClockHook(Arc<Mutex<Option<n0_future::time::Instant>>>);
    impl EndpointHooks for ClockHook {
        async fn after_handshake(
            &self,
            connection: &Connection,
        ) -> iroh::endpoint::AfterHandshakeOutcome {
            *self.0.lock().unwrap() = connection.caller_deadline();
            iroh::endpoint::AfterHandshakeOutcome::AcceptRestricted
        }
    }
    const ALPN: &[u8] = b"mio77/incoming-clock-unit/1";
    let observed = Arc::new(Mutex::new(None));
    let client = Endpoint::builder(iroh::endpoint::presets::Minimal)
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .clear_relay_transports()
        .bind()
        .await
        .unwrap();
    let server = Endpoint::builder(iroh::endpoint::presets::Minimal)
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .clear_relay_transports()
        .alpns(vec![ALPN.to_vec()])
        .admission_gate(ClockHook(observed.clone()))
        .bind()
        .await
        .unwrap();
    let expected = Arc::new(Mutex::new(None));
    let captured = expected.clone();
    let (outgoing, incoming) = tokio::join!(client.connect(server.addr(), ALPN), async {
        let incoming = server.accept().await.unwrap();
        let received = incoming.received_at();
        let deadline = received + Duration::from_secs(5);
        *captured.lock().unwrap() = Some(deadline);
        let mut accepting = incoming.with_caller_deadline(deadline).accept().unwrap();
        assert_eq!(accepting.received_at(), received);
        assert_eq!(accepting.alpn().await.unwrap(), ALPN);
        assert_eq!(accepting.received_at(), received);
        let connection = accepting.await.unwrap();
        assert_eq!(connection.caller_deadline(), Some(deadline));
        connection
    });
    let outgoing = outgoing.unwrap();
    assert_eq!(*observed.lock().unwrap(), *expected.lock().unwrap());
    outgoing.close(0u32.into(), b"clock fixture ended");
    incoming.close(0u32.into(), b"clock fixture ended");
    tokio::join!(client.close(), server.close());
}
