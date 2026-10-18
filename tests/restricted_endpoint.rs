//! Restricted consumers use the same public endpoint/stream seam as applications.

use std::{
    io,
    net::{Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::Duration,
};

use iroh::{
    Endpoint, EndpointAddr,
    endpoint::{
        AdmissionOutcome, AfterHandshakeOutcome, Connection, ControlConnection, EndpointAdmission,
        EndpointHooks, PortmapperConfig, RestrictedEndpoint, presets,
    },
};

const ALPN: &[u8] = b"test/restricted-endpoint/1";
const CONTROL_ALPN: &[u8] = b"test/restricted-control/1";
type Events = Arc<Mutex<Vec<String>>>;

#[path = "support/relay_contact_broker.rs"]
mod owned_broker;

#[derive(Debug)]
struct ExchangeAdmission {
    label: &'static str,
    events: Events,
}

impl EndpointAdmission for ExchangeAdmission {
    async fn admit(&self, connection: &ControlConnection) -> io::Result<AdmissionOutcome> {
        connection.send_control(b"authorized-control").await?;
        if connection.recv_control().await? != b"authorized-control" {
            return Err(io::Error::other("unexpected protected control message"));
        }
        self.events
            .lock()
            .unwrap()
            .push(format!("{}:gate", self.label));
        let path = connection
            .paths()
            .iter()
            .find(|path| path.is_ip())
            .ok_or_else(|| io::Error::other("no UDP candidate"))?
            .id();
        let permit = connection.qualify_data_path(path, Vec::new()).await?;
        self.events
            .lock()
            .unwrap()
            .push(format!("{}:path", self.label));
        Ok(AdmissionOutcome::Accept(permit))
    }

    fn is_current(&self, _connection: &ControlConnection) -> bool {
        true
    }
}

#[derive(Debug)]
struct LegacyCheck {
    label: &'static str,
    events: Events,
}

impl EndpointHooks for LegacyCheck {
    async fn after_handshake(&self, _connection: &Connection) -> AfterHandshakeOutcome {
        let mut events = self.events.lock().unwrap();
        assert!(events.contains(&format!("{}:gate", self.label)));
        assert!(events.contains(&format!("{}:path", self.label)));
        events.push(format!("{}:hook", self.label));
        AfterHandshakeOutcome::Accept
    }
}

async fn endpoint(label: &'static str, events: Events) -> RestrictedEndpoint {
    Endpoint::builder(presets::Minimal)
        .portmapper_config(PortmapperConfig::Disabled)
        .alpns(vec![ALPN.to_vec()])
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .hooks(LegacyCheck {
            label,
            events: events.clone(),
        })
        .restricted(ExchangeAdmission { label, events })
        .bind()
        .await
        .unwrap()
}

fn direct(endpoint: &RestrictedEndpoint) -> EndpointAddr {
    EndpointAddr::new(endpoint.id()).with_addrs(
        endpoint
            .addr()
            .ip_addrs()
            .copied()
            .map(iroh_base::TransportAddr::Ip),
    )
}

#[tokio::test]
async fn control_and_path_checks_precede_legacy_hook_and_original_connection_handoff() {
    let events = Events::default();
    let client = endpoint("client", events.clone()).await;
    let server = endpoint("server", events.clone()).await;
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(
            async {
                let connection: Connection = client.connect(direct(&server), ALPN).await.unwrap();
                events.lock().unwrap().push("client:handoff".to_string());
                let (mut send, mut recv) = connection.open_bi().await.unwrap();
                send.write_all(b"business-ping").await.unwrap();
                send.finish().unwrap();
                let reply = recv.read_to_end(64).await.unwrap();
                (connection, reply)
            },
            async {
                let connection: Connection = server.accept().await.unwrap().await.unwrap();
                events.lock().unwrap().push("server:handoff".to_string());
                let (mut send, mut recv) = connection.accept_bi().await.unwrap();
                let request = recv.read_to_end(64).await.unwrap();
                send.write_all(b"business-pong").await.unwrap();
                send.finish().unwrap();
                (connection, request)
            }
        )
    })
    .await;
    client.close().await;
    server.close().await;
    let ((_, reply), (_, request)) = result.expect("bounded restricted exchange");
    assert_eq!(request, b"business-ping");
    assert_eq!(reply, b"business-pong");
    let events = events.lock().unwrap();
    for label in ["client", "server"] {
        let actual: Vec<_> = events
            .iter()
            .filter(|event| event.starts_with(label))
            .cloned()
            .collect();
        assert_eq!(
            actual,
            ["gate", "path", "hook", "handoff"].map(|stage| format!("{label}:{stage}"))
        );
    }
}

#[derive(Debug)]
struct ControlOnlyAdmission;

impl EndpointAdmission for ControlOnlyAdmission {
    async fn admit(&self, connection: &ControlConnection) -> io::Result<AdmissionOutcome> {
        connection.send_control(b"bootstrap-only").await?;
        if connection.recv_control().await? != b"bootstrap-only" {
            return Err(io::Error::other("incorrect bootstrap proof"));
        }
        Ok(AdmissionOutcome::ControlOnly)
    }

    fn is_current(&self, _connection: &ControlConnection) -> bool {
        true
    }
}

#[derive(Debug)]
struct BusinessHookMustNotRun;

impl EndpointHooks for BusinessHookMustNotRun {
    async fn after_handshake(&self, _connection: &Connection) -> AfterHandshakeOutcome {
        panic!("a control-only scope must not expose Connection to a business hook")
    }
}

#[tokio::test]
async fn control_only_admission_delivers_framed_control_without_business_handoff() {
    let bind = || async {
        Endpoint::builder(presets::Minimal)
            .portmapper_config(PortmapperConfig::Disabled)
            .alpns(vec![ALPN.to_vec()])
            .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .unwrap()
            .hooks(BusinessHookMustNotRun)
            .restricted(ControlOnlyAdmission)
            .bind()
            .await
            .unwrap()
    };
    let client = bind().await;
    let server = bind().await;
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(
            async {
                let connection: ControlConnection =
                    client.connect_control(direct(&server), ALPN).await.unwrap();
                connection.send_control(b"admission-request").await.unwrap();
                let response = connection.recv_control().await.unwrap();
                (connection, response)
            },
            async {
                let connection: ControlConnection = server
                    .accept()
                    .await
                    .unwrap()
                    .accept_control()
                    .unwrap()
                    .await
                    .unwrap();
                let request = connection.recv_control().await.unwrap();
                connection.send_control(b"awaiting-approval").await.unwrap();
                (connection, request)
            }
        )
    })
    .await;
    client.close().await;
    server.close().await;
    let ((_, response), (_, request)) = result.expect("bounded control-only exchange");
    assert_eq!(request, b"admission-request");
    assert_eq!(response, b"awaiting-approval");
}

#[derive(Debug)]
struct RoutingAdmission(ExchangeAdmission);

impl EndpointAdmission for RoutingAdmission {
    async fn admit(&self, connection: &ControlConnection) -> io::Result<AdmissionOutcome> {
        if connection.alpn() == CONTROL_ALPN {
            EndpointAdmission::admit(&ControlOnlyAdmission, connection).await
        } else {
            EndpointAdmission::admit(&self.0, connection).await
        }
    }

    fn is_current(&self, _connection: &ControlConnection) -> bool {
        true
    }
}

#[derive(Debug)]
struct RoutedBusiness;

impl iroh::protocol::ProtocolHandler for RoutedBusiness {
    async fn on_accepting(
        &self,
        _accepting: iroh::endpoint::Accepting,
    ) -> Result<Connection, iroh::protocol::AcceptError> {
        panic!("restricted routing must not expose the ordinary Accepting state")
    }

    async fn accept(&self, connection: Connection) -> Result<(), iroh::protocol::AcceptError> {
        let (mut send, mut recv) = connection.accept_bi().await?;
        let message = recv
            .read_to_end(64)
            .await
            .map_err(iroh::protocol::AcceptError::from_err)?;
        send.write_all(&message)
            .await
            .map_err(iroh::protocol::AcceptError::from_err)?;
        send.finish()?;
        send.stopped()
            .await
            .map_err(iroh::protocol::AcceptError::from_err)?;
        Ok(())
    }
}

#[derive(Debug)]
struct RoutedControl;

impl iroh::protocol::ControlProtocolHandler for RoutedControl {
    async fn accept(
        &self,
        connection: ControlConnection,
    ) -> Result<(), iroh::protocol::AcceptError> {
        let message = connection.recv_control().await?;
        connection.send_control(&message).await?;
        let done = connection.recv_control().await?;
        assert_eq!(done, b"done");
        Ok(())
    }
}

#[tokio::test]
async fn restricted_router_delivers_original_business_and_separate_control_handlers() {
    use iroh::protocol::RestrictedRouter;
    let events = Events::default();
    let bind = |label| {
        let events = events.clone();
        async move {
            Endpoint::builder(presets::Minimal)
                .portmapper_config(PortmapperConfig::Disabled)
                .alpns(vec![ALPN.to_vec(), CONTROL_ALPN.to_vec()])
                .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
                .unwrap()
                .hooks(LegacyCheck {
                    label,
                    events: events.clone(),
                })
                .restricted(RoutingAdmission(ExchangeAdmission { label, events }))
                .bind()
                .await
                .unwrap()
        }
    };
    let client = bind("client").await;
    let server = bind("server").await;
    let router = RestrictedRouter::builder(server.clone())
        .accept(ALPN, RoutedBusiness)
        .accept_control(CONTROL_ALPN, RoutedControl)
        .spawn();
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let connection: Connection = client
            .connect(direct(router.endpoint()), ALPN)
            .await
            .unwrap();
        let (mut send, mut recv) = connection.open_bi().await.unwrap();
        send.write_all(b"routed-business").await.unwrap();
        send.finish().unwrap();
        let business = recv.read_to_end(64).await.unwrap();
        let control = client
            .connect_control(direct(router.endpoint()), CONTROL_ALPN)
            .await
            .unwrap();
        control.send_control(b"routed-control").await.unwrap();
        let bootstrap = control.recv_control().await.unwrap();
        control.send_control(b"done").await.unwrap();
        (connection, control, business, bootstrap)
    })
    .await;
    client.close().await;
    router.shutdown().await.unwrap();
    let (_, _, business, bootstrap) = result.expect("bounded restricted routing");
    assert_eq!(business, b"routed-business");
    assert_eq!(bootstrap, b"routed-control");
}

#[derive(Debug)]
struct WaitingAdmission {
    entered: Arc<tokio::sync::Notify>,
}

impl EndpointAdmission for WaitingAdmission {
    async fn admit(&self, connection: &ControlConnection) -> io::Result<AdmissionOutcome> {
        connection.send_control(b"wait-for-decision").await?;
        if connection.recv_control().await? != b"wait-for-decision" {
            return Err(io::Error::other("incorrect control phase"));
        }
        self.entered.notify_one();
        std::future::pending().await
    }

    fn is_current(&self, _connection: &ControlConnection) -> bool {
        true
    }
}

async fn waiting_endpoint(entered: Arc<tokio::sync::Notify>) -> RestrictedEndpoint {
    Endpoint::builder(presets::Minimal)
        .portmapper_config(PortmapperConfig::Disabled)
        .alpns(vec![ALPN.to_vec()])
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .hooks(BusinessHookMustNotRun)
        .restricted(WaitingAdmission { entered })
        .admission_timeout(Duration::from_secs(10))
        .bind()
        .await
        .unwrap()
}

#[tokio::test]
async fn cancellation_closes_original_attempt_and_releases_the_peers_pending_policy() {
    let client_entered = Arc::new(tokio::sync::Notify::new());
    let server_entered = Arc::new(tokio::sync::Notify::new());
    let client = waiting_endpoint(client_entered.clone()).await;
    let server = waiting_endpoint(server_entered.clone()).await;
    let token = tokio_util::sync::CancellationToken::new();
    let receiver = server.clone();
    let mut incoming = tokio::spawn(async move { receiver.accept().await.unwrap().await });
    let attempt = client
        .connect_with_opts(
            direct(&server),
            ALPN,
            iroh::endpoint::ConnectOptions::new().with_attempt_cancellation(token.clone()),
        )
        .await
        .unwrap();
    let mut outgoing = tokio::spawn(async move { attempt.await });
    let entered = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(client_entered.notified(), server_entered.notified());
    })
    .await;
    token.cancel();
    let outgoing_result = tokio::time::timeout(Duration::from_secs(2), &mut outgoing).await;
    let incoming_result = tokio::time::timeout(Duration::from_secs(2), &mut incoming).await;
    if outgoing_result.is_err() {
        outgoing.abort();
        let _ = outgoing.await;
    }
    if incoming_result.is_err() {
        incoming.abort();
        let _ = incoming.await;
    }
    client.close().await;
    server.close().await;
    assert!(
        entered.is_ok(),
        "both real policies entered the protected wait"
    );
    let outgoing = outgoing_result.expect("cancellation must finish the local attempt");
    let incoming = incoming_result
        .expect("peer closure must release pending policy without waiting for its setup deadline");
    assert!(matches!(
        outgoing.unwrap(),
        Err(iroh::endpoint::ConnectingError::AttemptCancelled { .. })
    ));
    assert!(matches!(
        incoming.unwrap(),
        Err(iroh::endpoint::ConnectingError::ConnectionError { .. })
    ));
}

#[derive(Debug)]
struct RevocableAdmission {
    exchange: ExchangeAdmission,
    retained: Arc<Mutex<Option<iroh::endpoint::WeakControlConnection>>>,
}

impl EndpointAdmission for RevocableAdmission {
    async fn admit(&self, connection: &ControlConnection) -> io::Result<AdmissionOutcome> {
        *self.retained.lock().unwrap() = Some(connection.weak_handle());
        EndpointAdmission::admit(&self.exchange, connection).await
    }

    fn is_current(&self, _connection: &ControlConnection) -> bool {
        true
    }
}

#[tokio::test]
async fn restricted_policy_can_revoke_after_original_connection_handoff_without_a_business_handle()
{
    let events = Events::default();
    let retained = Arc::new(Mutex::new(None));
    let client = endpoint("client", events.clone()).await;
    let server = Endpoint::builder(presets::Minimal)
        .portmapper_config(PortmapperConfig::Disabled)
        .alpns(vec![ALPN.to_vec()])
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .hooks(LegacyCheck {
            label: "server",
            events: events.clone(),
        })
        .restricted(RevocableAdmission {
            exchange: ExchangeAdmission {
                label: "server",
                events,
            },
            retained: retained.clone(),
        })
        .bind()
        .await
        .unwrap();
    let connections = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(client.connect(direct(&server), ALPN), async {
            server.accept().await.unwrap().await
        })
    })
    .await;
    let result = if let Ok((Ok(outgoing), Ok(incoming))) = &connections {
        let weak = retained.lock().unwrap().as_ref().unwrap().clone();
        assert!(weak.is_open());
        assert!(weak.close(7u32.into(), b"authorization revoked"));
        let closed = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(outgoing.closed(), incoming.closed())
        })
        .await;
        let refuses_stream = incoming.open_bi().await.is_err();
        Some((closed, refuses_stream, weak.is_open()))
    } else {
        None
    };
    client.close().await;
    server.close().await;
    let (closed, refuses_stream, still_open) =
        result.expect("ordinary connections were handed off");
    assert!(
        closed.is_ok(),
        "revocation reaches both original connections"
    );
    assert!(refuses_stream);
    assert!(!still_open);
}

async fn configured<P: EndpointAdmission + 'static, H: EndpointHooks + 'static>(
    policy: P,
    hooks: H,
) -> RestrictedEndpoint {
    Endpoint::builder(presets::Minimal)
        .portmapper_config(PortmapperConfig::Disabled)
        .alpns(vec![ALPN.to_vec()])
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .hooks(hooks)
        .restricted(policy)
        .admission_timeout(Duration::from_secs(3))
        .bind()
        .await
        .unwrap()
}

async fn rejected_policy(policy: impl EndpointAdmission + 'static) {
    let client = endpoint("client", Events::default()).await;
    let server = configured(policy, BusinessHookMustNotRun).await;
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(client.connect(direct(&server), ALPN), async {
            server.accept().await.unwrap().await
        })
    })
    .await;
    client.close().await;
    server.close().await;
    let (outgoing, incoming) = result.expect("rejected admission is bounded");
    assert!(
        incoming.is_err(),
        "server must not hand off business: {incoming:?}"
    );
    assert!(
        outgoing.is_err(),
        "peer must not hand off business: {outgoing:?}"
    );
}

#[derive(Debug)]
struct MissingDecision;
impl EndpointAdmission for MissingDecision {
    fn is_current(&self, _connection: &ControlConnection) -> bool {
        true
    }
}

#[tokio::test]
async fn missing_decision_does_not_reach_legacy_hooks_or_business() {
    rejected_policy(MissingDecision).await;
}

#[derive(Debug)]
struct DenyAdmission;
impl EndpointAdmission for DenyAdmission {
    async fn admit(&self, _connection: &ControlConnection) -> io::Result<AdmissionOutcome> {
        Ok(AdmissionOutcome::Reject {
            error_code: 7u32.into(),
            reason: b"not authorized".to_vec(),
        })
    }
    fn is_current(&self, _connection: &ControlConnection) -> bool {
        true
    }
}

#[tokio::test]
async fn explicit_rejection_does_not_reach_legacy_hooks_or_business() {
    rejected_policy(DenyAdmission).await;
}

#[derive(Debug)]
struct MissingDataPath;
impl EndpointAdmission for MissingDataPath {
    async fn admit(&self, connection: &ControlConnection) -> io::Result<AdmissionOutcome> {
        connection.send_control(b"authorized-control").await?;
        if connection.recv_control().await? != b"authorized-control" {
            return Err(io::Error::other("incorrect control proof"));
        }
        connection
            .qualify_data_path(iroh::endpoint::PathId::MAX, Vec::new())
            .await
            .map(AdmissionOutcome::Accept)
    }
    fn is_current(&self, _connection: &ControlConnection) -> bool {
        true
    }
}

#[tokio::test]
async fn unqualified_path_does_not_reach_legacy_hooks_or_business() {
    rejected_policy(MissingDataPath).await;
}

#[derive(Debug)]
struct RejectAfterChecks(LegacyCheck);
impl EndpointHooks for RejectAfterChecks {
    async fn after_handshake(&self, connection: &Connection) -> AfterHandshakeOutcome {
        EndpointHooks::after_handshake(&self.0, connection).await;
        AfterHandshakeOutcome::Reject {
            error_code: 9u32.into(),
            reason: b"legacy veto".to_vec(),
        }
    }
}

#[tokio::test]
async fn legacy_hook_rejects_after_new_checks_before_either_business_handoff() {
    let events = Events::default();
    let client = endpoint("client", events.clone()).await;
    let server = configured(
        ExchangeAdmission {
            label: "server",
            events: events.clone(),
        },
        RejectAfterChecks(LegacyCheck {
            label: "server",
            events: events.clone(),
        }),
    )
    .await;
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(client.connect(direct(&server), ALPN), async {
            server.accept().await.unwrap().await
        })
    })
    .await;
    client.close().await;
    server.close().await;
    let (outgoing, incoming) = result.expect("legacy veto is bounded");
    assert!(incoming.is_err());
    assert!(outgoing.is_err());
    let events = events.lock().unwrap();
    let server_events: Vec<_> = events
        .iter()
        .filter(|event| event.starts_with("server"))
        .cloned()
        .collect();
    assert_eq!(server_events, ["server:gate", "server:path", "server:hook"]);
}

#[tokio::test]
async fn control_only_decision_cannot_be_used_by_business_connect_or_incoming_future() {
    let client = configured(ControlOnlyAdmission, BusinessHookMustNotRun).await;
    let server = configured(ControlOnlyAdmission, BusinessHookMustNotRun).await;
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(client.connect(direct(&server), ALPN), async {
            server.accept().await.unwrap().await
        })
    })
    .await;
    client.close().await;
    server.close().await;
    let (outgoing, incoming) = result.expect("scope mismatch is bounded");
    assert!(incoming.is_err());
    assert!(outgoing.is_err());
}

#[tokio::test]
async fn accept_with_and_alpn_inspection_preserve_restricted_delivery() {
    let events = Events::default();
    let client = endpoint("client", events.clone()).await;
    let server = endpoint("server", events.clone()).await;
    let config = Arc::new(
        server
            .create_server_config_builder(vec![ALPN.to_vec()])
            .build(),
    );
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        let (outgoing, (incoming, alpn, not_admitted_early)) =
            tokio::join!(client.connect(direct(&server), ALPN), async {
                let mut accepting = server.accept().await.unwrap().accept_with(config).unwrap();
                let alpn = accepting.alpn().await.unwrap();
                let not_admitted_early = events.lock().unwrap().is_empty();
                (accepting.await, alpn, not_admitted_early)
            });
        let outgoing = outgoing.unwrap();
        let incoming = incoming.unwrap();
        let (reply, request) = tokio::join!(
            async {
                let (mut send, mut recv) = outgoing.open_bi().await.unwrap();
                send.write_all(b"custom-config-ping").await.unwrap();
                send.finish().unwrap();
                recv.read_to_end(64).await.unwrap()
            },
            async {
                let (mut send, mut recv) = incoming.accept_bi().await.unwrap();
                let request = recv.read_to_end(64).await.unwrap();
                send.write_all(b"custom-config-pong").await.unwrap();
                send.finish().unwrap();
                request
            }
        );
        (outgoing, incoming, alpn, not_admitted_early, reply, request)
    })
    .await;
    client.close().await;
    server.close().await;
    let (_, _, alpn, not_admitted_early, reply, request) =
        result.expect("custom accepting path is bounded");
    assert_eq!(alpn, ALPN);
    assert!(
        not_admitted_early,
        "ALPN inspection cannot run or bypass admission"
    );
    assert_eq!(request, b"custom-config-ping");
    assert_eq!(reply, b"custom-config-pong");
}

#[tokio::test]
async fn dropping_pending_future_closes_original_connection_and_releases_peer() {
    let client_entered = Arc::new(tokio::sync::Notify::new());
    let server_entered = Arc::new(tokio::sync::Notify::new());
    let client = waiting_endpoint(client_entered.clone()).await;
    let server = waiting_endpoint(server_entered.clone()).await;
    let receiver = server.clone();
    let mut incoming = tokio::spawn(async move { receiver.accept().await.unwrap().await });
    let attempt = client
        .connect_with_opts(direct(&server), ALPN, Default::default())
        .await
        .unwrap();
    let outgoing = tokio::spawn(async move { attempt.await });
    let entered = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(client_entered.notified(), server_entered.notified());
    })
    .await;
    // Abort drops the pending future; it does not close either endpoint or
    // manually close the underlying connection being tested.
    outgoing.abort();
    let outgoing = outgoing.await;
    let result = tokio::time::timeout(Duration::from_secs(2), &mut incoming).await;
    if result.is_err() {
        incoming.abort();
        let _ = incoming.await;
    }
    client.close().await;
    server.close().await;
    assert!(entered.is_ok());
    assert!(outgoing.unwrap_err().is_cancelled());
    assert!(matches!(
        result
            .expect("dropping local pending owner closes peer")
            .unwrap(),
        Err(iroh::endpoint::ConnectingError::ConnectionError { .. })
    ));
}

#[tokio::test]
async fn original_caller_deadline_bounds_control_wait_and_releases_peer() {
    let client_entered = Arc::new(tokio::sync::Notify::new());
    let server_entered = Arc::new(tokio::sync::Notify::new());
    let client = waiting_endpoint(client_entered.clone()).await;
    let server = waiting_endpoint(server_entered.clone()).await;
    let deadline = n0_future::time::Instant::now() + Duration::from_millis(500);
    let result = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(
            async {
                client
                    .connect_with_opts(
                        direct(&server),
                        ALPN,
                        iroh::endpoint::ConnectOptions::new().with_caller_deadline(deadline),
                    )
                    .await
                    .unwrap()
                    .await
            },
            async { server.accept().await.unwrap().await }
        )
    })
    .await;
    let entered = tokio::time::timeout(Duration::from_millis(50), async {
        tokio::join!(client_entered.notified(), server_entered.notified());
    })
    .await;
    client.close().await;
    server.close().await;
    assert!(
        entered.is_ok(),
        "both policies reached control before the caller deadline"
    );
    let (outgoing, incoming) = result.expect("original deadline bounds the whole wait");
    assert!(matches!(
        outgoing,
        Err(iroh::endpoint::ConnectingError::AttemptTimedOut { .. })
    ));
    assert!(matches!(
        incoming,
        Err(iroh::endpoint::ConnectingError::ConnectionError { .. })
    ));
}

#[derive(Debug)]
struct CurrentAdmission {
    exchange: ExchangeAdmission,
    current: Arc<std::sync::atomic::AtomicBool>,
}
impl EndpointAdmission for CurrentAdmission {
    async fn admit(&self, connection: &ControlConnection) -> io::Result<AdmissionOutcome> {
        EndpointAdmission::admit(&self.exchange, connection).await
    }
    fn is_current(&self, _connection: &ControlConnection) -> bool {
        self.current.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[derive(Debug)]
struct WithdrawOnHook {
    check: LegacyCheck,
    current: Arc<std::sync::atomic::AtomicBool>,
}
impl EndpointHooks for WithdrawOnHook {
    async fn after_handshake(&self, connection: &Connection) -> AfterHandshakeOutcome {
        EndpointHooks::after_handshake(&self.check, connection).await;
        self.current
            .store(false, std::sync::atomic::Ordering::SeqCst);
        AfterHandshakeOutcome::Accept
    }
}

#[tokio::test]
async fn authorization_withdrawn_by_legacy_hook_does_not_signal_delivery_ready_to_peer() {
    let events = Events::default();
    let current = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let server = configured(
        CurrentAdmission {
            exchange: ExchangeAdmission {
                label: "server",
                events: events.clone(),
            },
            current: current.clone(),
        },
        WithdrawOnHook {
            check: LegacyCheck {
                label: "server",
                events,
            },
            current: current.clone(),
        },
    )
    .await;
    // A plain public iroh consumer independently speaks the documented control
    // wire, so it can observe readiness without using a private test hook.
    let client = Endpoint::builder(presets::Minimal)
        .portmapper_config(PortmapperConfig::Disabled)
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .bind()
        .await
        .unwrap();
    let receiver = server.clone();
    let mut incoming = tokio::spawn(async move { receiver.accept().await.unwrap().await });
    let observed = tokio::time::timeout(Duration::from_secs(2), async {
        let connection = client.connect(direct(&server), ALPN).await.unwrap();
        let (mut send, mut recv) = connection.open_bi().await.unwrap();
        let preface = b"iroh/admission/1\0";
        send.write_all(preface).await.unwrap();
        let mut remote_preface = [0; 17];
        recv.read_exact(&mut remote_preface[..preface.len()])
            .await
            .unwrap();
        assert_eq!(&remote_preface, preface);
        let frame = b"\x00\x00\x00\x00\x12authorized-control";
        let mut received = [0; 23];
        recv.read_exact(&mut received).await.unwrap();
        assert_eq!(&received, frame);
        send.write_all(frame).await.unwrap();
        let mut ready = [0; 6];
        let observed = recv.read_exact(&mut ready).await.ok().map(|_| ready);
        connection.close(0u32.into(), b"observation complete");
        observed
    })
    .await;
    let result = tokio::time::timeout(Duration::from_secs(2), &mut incoming).await;
    if result.is_err() {
        incoming.abort();
        let _ = incoming.await;
    }
    client.close().await;
    server.close().await;
    assert!(
        !current.load(std::sync::atomic::Ordering::SeqCst),
        "old hook withdrew authorization"
    );
    assert!(result.expect("withdrawn attempt closes").unwrap().is_err());
    assert!(
        observed.expect("wire observation is bounded").is_none(),
        "a withdrawn decision must not advertise delivery readiness"
    );
}

#[derive(Debug)]
struct UpgradeAdmission {
    label: &'static str,
    events: Events,
    local: Arc<std::sync::OnceLock<SocketAddr>>,
    original: Arc<Mutex<Option<usize>>>,
}

impl EndpointAdmission for UpgradeAdmission {
    async fn admit(&self, connection: &ControlConnection) -> io::Result<AdmissionOutcome> {
        use iroh_base::TransportAddr;
        *self.original.lock().unwrap() = Some(connection.stable_id());
        if !connection.paths().iter().all(|path| matches!(path.remote_addr(),
            TransportAddr::Custom(address) if address.id() == miopunch_native_probe::mqtt_transport::TRANSPORT_ID))
        {
            return Err(io::Error::other("bootstrap must use only the owned MQTT route"));
        }
        connection.send_control(b"authorized-control").await?;
        if connection.recv_control().await? != b"authorized-control" {
            return Err(io::Error::other("incorrect control proof"));
        }
        self.events
            .lock()
            .unwrap()
            .push(format!("{}:gate", self.label));
        if connection
            .qualify_data_path(iroh::endpoint::PathId::ZERO, Vec::new())
            .await
            .is_ok()
        {
            return Err(io::Error::other(
                "signaling route must not qualify as permitted business",
            ));
        }
        let local = self
            .local
            .get()
            .ok_or_else(|| io::Error::other("local endpoint not ready"))?;
        let std::net::IpAddr::V4(ip) = local.ip() else {
            return Err(io::Error::other("expected local IPv4"));
        };
        let mut candidate = ip.octets().to_vec();
        candidate.extend(local.port().to_be_bytes());
        connection.send_control(&candidate).await?;
        let remote = connection.recv_control().await?;
        if remote.len() != 6 {
            return Err(io::Error::other("invalid owned UDP candidate"));
        }
        let remote = SocketAddr::from((
            Ipv4Addr::new(remote[0], remote[1], remote[2], remote[3]),
            u16::from_be_bytes([remote[4], remote[5]]),
        ));
        if connection.side().is_client() {
            connection.open_data_path(remote).await?;
        }
        let path_id = loop {
            if let Some(path) = connection.paths().iter().find(|path| path.is_ip()) {
                break path.id();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        };
        let permit = connection.qualify_data_path(path_id, Vec::new()).await?;
        self.events
            .lock()
            .unwrap()
            .push(format!("{}:path", self.label));
        Ok(AdmissionOutcome::Accept(permit))
    }

    fn is_current(&self, _connection: &ControlConnection) -> bool {
        true
    }
}

#[tokio::test]
async fn restricted_mqtt_control_builds_udp_on_original_connection_before_business() {
    use miopunch_native_probe::mqtt_transport::{MqttTransport, Traffic};
    let broker = owned_broker::Broker::new().await;
    let events = Events::default();
    let bind = |label| {
        let events = events.clone();
        let broker = broker.address;
        async move {
            let local = Arc::new(std::sync::OnceLock::new());
            let original = Arc::new(Mutex::new(None));
            let traffic = Arc::new(Traffic::default());
            let carrier = Arc::new(MqttTransport::new(broker, traffic.clone()));
            let endpoint = Endpoint::builder(presets::Minimal)
                .portmapper_config(PortmapperConfig::Disabled)
                .alpns(vec![ALPN.to_vec()])
                .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
                .unwrap()
                .clear_relay_transports()
                .add_custom_transport(carrier.clone())
                .hooks(LegacyCheck {
                    label,
                    events: events.clone(),
                })
                .restricted(UpgradeAdmission {
                    label,
                    events,
                    local: local.clone(),
                    original: original.clone(),
                })
                .admission_timeout(Duration::from_secs(8))
                .bind()
                .await
                .unwrap();
            local
                .set(
                    endpoint
                        .addr()
                        .ip_addrs()
                        .find(|addr| addr.is_ipv4() && addr.ip().is_loopback())
                        .copied()
                        .unwrap(),
                )
                .unwrap();
            carrier.wait_ready().await.unwrap();
            (endpoint, carrier, traffic, original)
        }
    };
    let (client, client_carrier, client_traffic, client_original) = bind("client").await;
    let (server, server_carrier, server_traffic, server_original) = bind("server").await;
    let route = MqttTransport::address(&server_carrier.route().unwrap()).unwrap();
    let target =
        EndpointAddr::new(server.id()).with_addrs([iroh_base::TransportAddr::Custom(route)]);
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let (outgoing, incoming) = tokio::join!(client.connect(target, ALPN), async {
            server.accept().await.unwrap().await
        });
        let (outgoing, incoming) = (outgoing.unwrap(), incoming.unwrap());
        let original = *client_original.lock().unwrap() == Some(outgoing.stable_id())
            && *server_original.lock().unwrap() == Some(incoming.stable_id());
        let udp_selected = [&outgoing, &incoming].iter().all(|connection| {
            connection
                .paths()
                .iter()
                .any(|path| path.is_selected() && path.is_ip())
        });
        let (reply, request) = tokio::join!(
            async {
                let (mut send, mut recv) = outgoing.open_bi().await.unwrap();
                send.write_all(b"upgraded-ping").await.unwrap();
                send.finish().unwrap();
                recv.read_to_end(64).await.unwrap()
            },
            async {
                let (mut send, mut recv) = incoming.accept_bi().await.unwrap();
                let request = recv.read_to_end(64).await.unwrap();
                send.write_all(b"upgraded-pong").await.unwrap();
                send.finish().unwrap();
                request
            }
        );
        (outgoing, incoming, original, udp_selected, reply, request)
    })
    .await;
    client.close().await;
    server.close().await;
    let result = result.map(
        |(outgoing, incoming, original, udp_selected, reply, request)| {
            drop(outgoing);
            drop(incoming);
            (original, udp_selected, reply, request)
        },
    );
    // Endpoint::close drains QUIC; the underlying custom endpoint is released
    // only after all endpoint owners are dropped, as with the ordinary type.
    drop(client);
    drop(server);
    let drivers = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(
            client_carrier.wait_driver_closed(),
            server_carrier.wait_driver_closed()
        )
    })
    .await;
    let inactive = tokio::time::timeout(Duration::from_secs(2), async {
        while broker.active() != 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    broker.close().await;
    let (original, udp_selected, reply, request) =
        result.expect("bounded MQTT-to-UDP establishment");
    assert!(original);
    assert!(udp_selected);
    assert_eq!(request, b"upgraded-ping");
    assert_eq!(reply, b"upgraded-pong");
    assert!(client_traffic.sent_packets() > 0 && server_traffic.sent_packets() > 0);
    let drivers = drivers.expect("owned MQTT drivers stopped after their endpoint owners dropped");
    assert!(drivers.0.is_ok() && drivers.1.is_ok());
    assert!(inactive.is_ok(), "owned MQTT sockets retired naturally");
}

#[derive(Debug)]
struct PreviousProof {
    exchange: ExchangeAdmission,
    previous: Arc<Mutex<Option<iroh::endpoint::DataPathPermit>>>,
    originals: Arc<Mutex<Vec<usize>>>,
    reuse: bool,
}
impl EndpointAdmission for PreviousProof {
    async fn admit(&self, connection: &ControlConnection) -> io::Result<AdmissionOutcome> {
        let first = {
            let mut originals = self.originals.lock().unwrap();
            originals.push(connection.stable_id());
            originals.len() == 1
        };
        let AdmissionOutcome::Accept(permit) =
            EndpointAdmission::admit(&self.exchange, connection).await?
        else {
            return Err(io::Error::other(
                "expected a fresh proof from the test policy",
            ));
        };
        if first {
            if self.reuse {
                *self.previous.lock().unwrap() = Some(permit);
            }
            return Ok(AdmissionOutcome::ControlOnly);
        }
        let permit = if self.reuse {
            self.previous
                .lock()
                .unwrap()
                .take()
                .ok_or_else(|| io::Error::other("missing prior proof"))?
        } else {
            permit
        };
        Ok(AdmissionOutcome::Accept(permit))
    }
    fn is_current(&self, _connection: &ControlConnection) -> bool {
        true
    }
}

#[tokio::test]
async fn proof_from_a_live_other_connection_cannot_release_business_on_the_same_peer() {
    let events = Events::default();
    let server_originals = Arc::new(Mutex::new(Vec::new()));
    let client = configured(
        PreviousProof {
            exchange: ExchangeAdmission {
                label: "client",
                events: events.clone(),
            },
            previous: Arc::default(),
            originals: Arc::default(),
            reuse: false,
        },
        LegacyCheck {
            label: "client",
            events: events.clone(),
        },
    )
    .await;
    let server = configured(
        PreviousProof {
            exchange: ExchangeAdmission {
                label: "server",
                events,
            },
            previous: Arc::default(),
            originals: server_originals.clone(),
            reuse: true,
        },
        BusinessHookMustNotRun,
    )
    .await;
    let first = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(client.connect_control(direct(&server), ALPN), async {
            server
                .accept()
                .await
                .unwrap()
                .accept_control()
                .unwrap()
                .await
        })
    })
    .await;
    let second = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(client.connect(direct(&server), ALPN), async {
            server.accept().await.unwrap().await
        })
    })
    .await;
    client.close().await;
    server.close().await;
    let (first_client, first_server) = first.expect("first control scope was bounded");
    assert!(
        first_client.is_ok() && first_server.is_ok(),
        "the proof's original session stayed live"
    );
    let (outgoing, incoming) = second.expect("second original session was bounded");
    assert!(incoming.is_err() && outgoing.is_err());
    let originals = server_originals.lock().unwrap();
    assert_eq!(originals.len(), 2);
    assert_ne!(originals[0], originals[1]);
}

async fn malformed_control_wire(wire: Vec<u8>) {
    let server = configured(
        ExchangeAdmission {
            label: "server",
            events: Events::default(),
        },
        BusinessHookMustNotRun,
    )
    .await;
    let client = Endpoint::builder(presets::Minimal)
        .portmapper_config(PortmapperConfig::Disabled)
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .bind()
        .await
        .unwrap();
    let receiver = server.clone();
    let mut incoming = tokio::spawn(async move { receiver.accept().await.unwrap().await });
    let closed = tokio::time::timeout(Duration::from_secs(2), async {
        let connection = client.connect(direct(&server), ALPN).await.unwrap();
        let (mut send, _recv) = connection.open_bi().await.unwrap();
        send.write_all(&wire).await.unwrap();
        // Keep the stream open without sending a body. Oversize rejection must
        // happen from the declared length, before EOF or the setup deadline.
        let reason = connection.closed().await;
        (connection, reason)
    })
    .await;
    let result = tokio::time::timeout(Duration::from_secs(2), &mut incoming).await;
    if result.is_err() {
        incoming.abort();
        let _ = incoming.await;
    }
    client.close().await;
    server.close().await;
    assert!(
        closed.is_ok(),
        "malformed control must close before waiting for a body/deadline"
    );
    assert!(
        result
            .expect("malformed control is bounded")
            .unwrap()
            .is_err()
    );
}

#[tokio::test]
async fn ordinary_stream_bytes_cannot_stand_in_for_the_control_protocol() {
    malformed_control_wire(b"ordinary-application-bytes".to_vec()).await;
}

#[tokio::test]
async fn oversized_control_message_is_rejected_before_reading_its_body() {
    let mut wire = b"iroh/admission/1\0".to_vec();
    wire.extend([0, 0, 1, 0, 1]); // Message kind 0, declared body 65,537 bytes.
    malformed_control_wire(wire).await;
}

#[derive(Debug)]
struct OrdinaryHandshakeCount(Arc<std::sync::atomic::AtomicUsize>);
impl EndpointHooks for OrdinaryHandshakeCount {
    async fn after_handshake(&self, _connection: &Connection) -> AfterHandshakeOutcome {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        AfterHandshakeOutcome::Accept
    }
}

#[tokio::test]
async fn ordinary_endpoints_still_exchange_early_data_and_resume_zero_rtt() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let server = Endpoint::builder(presets::Minimal)
        .portmapper_config(PortmapperConfig::Disabled)
        .alpns(vec![ALPN.to_vec()])
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .hooks(OrdinaryHandshakeCount(calls.clone()))
        .bind()
        .await
        .unwrap();
    let client = Endpoint::builder(presets::Minimal)
        .portmapper_config(PortmapperConfig::Disabled)
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .bind()
        .await
        .unwrap();
    let target = EndpointAddr::new(server.id()).with_addrs(
        server
            .addr()
            .ip_addrs()
            .copied()
            .map(iroh_base::TransportAddr::Ip),
    );
    let receiver = server.clone();
    let server_calls = calls.clone();
    let mut task = tokio::spawn(async move {
        let mut rounds = Vec::new();
        for _ in 0..2 {
            let early = receiver
                .accept()
                .await
                .unwrap()
                .accept()
                .unwrap()
                .into_0rtt();
            let (mut send, mut recv) = early.accept_bi().await.unwrap();
            let is_zero_rtt = recv.is_0rtt();
            let message = recv.read_to_end(64).await.unwrap();
            send.write_all(&message).await.unwrap();
            send.finish().unwrap();
            let before = server_calls.load(std::sync::atomic::Ordering::SeqCst);
            let connection = early.handshake_completed().await.unwrap();
            let after = server_calls.load(std::sync::atomic::Ordering::SeqCst);
            rounds.push((is_zero_rtt, before, after, message));
            connection.closed().await;
        }
        rounds
    });
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        let first = client
            .connect_with_opts(target.clone(), ALPN, Default::default())
            .await
            .unwrap()
            .into_0rtt()
            .expect_err("fresh client has no resumption ticket")
            .await
            .unwrap();
        let (mut send, mut recv) = first.open_bi().await.unwrap();
        send.write_all(b"ordinary-first").await.unwrap();
        send.finish().unwrap();
        let first_reply = recv.read_to_end(64).await.unwrap();
        first.close(0u32.into(), b"first complete");
        let early = client
            .connect_with_opts(target, ALPN, Default::default())
            .await
            .unwrap()
            .into_0rtt()
            .expect("original ordinary resumption remains enabled");
        let (mut send, mut recv) = early.open_bi().await.unwrap();
        send.write_all(b"ordinary-zero-rtt").await.unwrap();
        send.finish().unwrap();
        let status = early.handshake_completed().await.unwrap();
        let accepted = matches!(&status, iroh::endpoint::ZeroRttStatus::Accepted(_));
        let connection = match status {
            iroh::endpoint::ZeroRttStatus::Accepted(connection)
            | iroh::endpoint::ZeroRttStatus::Rejected(connection) => connection,
        };
        let resumed_reply = recv.read_to_end(64).await.unwrap();
        connection.close(0u32.into(), b"resumed complete");
        (first, connection, accepted, first_reply, resumed_reply)
    })
    .await;
    client.close().await;
    server.close().await;
    let served = tokio::time::timeout(Duration::from_secs(2), &mut task).await;
    if served.is_err() {
        task.abort();
        let _ = task.await;
    }
    let (_, _, accepted, first_reply, resumed_reply) =
        result.expect("ordinary early-data exchange is bounded");
    assert!(accepted);
    assert_eq!(first_reply, b"ordinary-first");
    assert_eq!(resumed_reply, b"ordinary-zero-rtt");
    let rounds = served.expect("ordinary server completed").unwrap();
    assert_eq!(rounds.len(), 2);
    assert_eq!((rounds[0].0, rounds[0].1, rounds[0].2), (false, 0, 1));
    assert_eq!((rounds[1].0, rounds[1].1, rounds[1].2), (true, 1, 2));
}
