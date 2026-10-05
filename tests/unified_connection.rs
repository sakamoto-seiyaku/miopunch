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
    let (client, server) = tokio::join!(be.connect(ae.addr(), ADMISSION_REQUEST_ALPN), async {
        ae.accept().await.unwrap().await
    });
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
struct InvalidEnvelope(u8);
impl EndpointHooks for InvalidEnvelope {
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
            _ => unreachable!(),
        }
        send.write_all(&(bytes.len() as u16).to_be_bytes()).await?;
        send.write_all(&bytes).await?;
        send.finish()?;
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
                    .hooks(InvalidEnvelope(fault)),
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
