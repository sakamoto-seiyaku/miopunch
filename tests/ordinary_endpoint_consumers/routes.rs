// This consumer uses only APIs present in the fixed official libraries.
use std::{
    future::{IntoFuture, poll_fn},
    io::{self, IoSliceMut},
    net::{Ipv4Addr, SocketAddr},
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};

use iroh::{
    Endpoint, EndpointAddr,
    endpoint::{
        Accepting, AfterHandshakeOutcome, BeforeConnectOutcome, Connection, ConnectingError,
        EndpointHooks, PortmapperConfig, presets,
    },
    protocol::{AcceptError, ProtocolHandler, Router},
};
use noq::{AsyncUdpSocket, Runtime, UdpSender};

const ALPN: &[u8] = b"test/ordinary-routing/1";
const CUSTOM_ALPN: &[u8] = b"test/ordinary-custom-config/1";
type Events = Arc<Mutex<Vec<&'static str>>>;

#[derive(Debug)]
struct Hook {
    name: &'static str,
    events: Events,
    reject_before: bool,
    reject_after: bool,
}

impl EndpointHooks for Hook {
    async fn before_connect(&self, _: &EndpointAddr, _: &[u8]) -> BeforeConnectOutcome {
        self.events.lock().unwrap().push(self.name);
        if self.reject_before {
            BeforeConnectOutcome::Reject
        } else {
            BeforeConnectOutcome::Accept
        }
    }

    async fn after_handshake(&self, connection: &Connection) -> AfterHandshakeOutcome {
        assert!(connection.close_reason().is_none());
        self.events.lock().unwrap().push(self.name);
        if self.reject_after {
            AfterHandshakeOutcome::Reject {
                error_code: 73u32.into(),
                reason: b"ordinary veto".to_vec(),
            }
        } else {
            AfterHandshakeOutcome::Accept
        }
    }
}

async fn endpoint(events: Events) -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .portmapper_config(PortmapperConfig::Disabled)
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .alpns(vec![ALPN.to_vec()])
        .hooks(Hook {
            name: "hook",
            events,
            reject_before: false,
            reject_after: false,
        })
        .bind()
        .await
        .unwrap()
}

fn direct(endpoint: &Endpoint) -> EndpointAddr {
    EndpointAddr::new(endpoint.id()).with_addrs(
        endpoint
            .addr()
            .ip_addrs()
            .copied()
            .map(iroh_base::TransportAddr::Ip),
    )
}

async fn send_ping(connection: &Connection) {
    let (mut send, mut recv) = connection.open_bi().await.unwrap();
    send.write_all(b"ordinary ping").await.unwrap();
    send.finish().unwrap();
    assert_eq!(recv.read_to_end(64).await.unwrap(), b"ordinary pong");
}

async fn answer_ping(connection: &Connection) {
    let (mut send, mut recv) = connection.accept_bi().await.unwrap();
    assert_eq!(recv.read_to_end(64).await.unwrap(), b"ordinary ping");
    send.write_all(b"ordinary pong").await.unwrap();
    send.finish().unwrap();
    send.stopped().await.unwrap();
}

async fn incoming_routes() {
    for mode in 0..3 {
        let server_events = Events::default();
        let client_events = Events::default();
        let server = endpoint(server_events.clone()).await;
        let client = endpoint(client_events.clone()).await;
        let alpn = if mode == 2 { CUSTOM_ALPN } else { ALPN };
        let target = direct(&server);
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(
                async {
                    let connection = client.connect(target, alpn).await.unwrap();
                    assert_eq!(*client_events.lock().unwrap(), ["hook", "hook"]);
                    send_ping(&connection).await;
                    connection.close(0u32.into(), b"complete");
                },
                async {
                    let incoming = server.accept().await.unwrap();
                    assert!(server_events.lock().unwrap().is_empty());
                    let connection = match mode {
                        0 => incoming.into_future().await.unwrap(),
                        1 => incoming.accept().unwrap().into_future().await.unwrap(),
                        _ => incoming
                            .accept_with(Arc::new(
                                server.create_server_config_builder(vec![CUSTOM_ALPN.to_vec()]).build(),
                            ))
                            .unwrap()
                            .into_future()
                            .await
                            .unwrap(),
                    };
                    assert_eq!(connection.alpn(), alpn);
                    assert_eq!(*server_events.lock().unwrap(), ["hook"]);
                    answer_ping(&connection).await;
                    connection.closed().await;
                }
            );
        }).await;
        client.close().await;
        server.close().await;
        result.expect("ordinary accept route is bounded");
    }
}

#[derive(Debug)]
struct EarlyRouter(Events);
impl ProtocolHandler for EarlyRouter {
    async fn on_accepting(&self, accepting: Accepting) -> Result<Connection, AcceptError> {
        assert!(self.0.lock().unwrap().is_empty());
        let early = accepting.into_0rtt();
        let mut greeting = early.open_uni().await?;
        greeting.write_all(b"ordinary router greeting").await.unwrap();
        greeting.finish()?;
        assert!(self.0.lock().unwrap().is_empty());
        Ok(early.handshake_completed().await?)
    }

    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        assert_eq!(*self.0.lock().unwrap(), ["hook"]);
        answer_ping(&connection).await;
        connection.closed().await;
        Ok(())
    }
}

async fn router() {
    let events = Events::default();
    let server = endpoint(events.clone()).await;
    let target = direct(&server);
    let router = Router::builder(server).accept(ALPN, EarlyRouter(events.clone())).spawn();
    let client = endpoint(Events::default()).await;
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        let connection = client.connect(target, ALPN).await.unwrap();
        let mut greeting = connection.accept_uni().await.unwrap();
        assert_eq!(greeting.read_to_end(64).await.unwrap(), b"ordinary router greeting");
        send_ping(&connection).await;
        connection.close(0u32.into(), b"router complete");
    }).await;
    client.close().await;
    router.shutdown().await.unwrap();
    result.expect("ordinary Router early-data exchange is bounded");
    assert_eq!(*events.lock().unwrap(), ["hook"]);
}

async fn veto(before: bool) {
    let events = Events::default();
    let builder = Endpoint::builder(presets::Minimal)
        .portmapper_config(PortmapperConfig::Disabled)
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .alpns(vec![ALPN.to_vec()]);
    let mut builder = builder;
    for (name, rejects) in [("first", false), ("veto", true), ("unreached", false)] {
        builder = builder.hooks(Hook {
            name, events: events.clone(),
            reject_before: before && rejects,
            reject_after: !before && rejects,
        });
    }
    let guarded = builder.bind().await.unwrap();
    let other = endpoint(Events::default()).await;
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        if before {
            assert!(guarded.connect(direct(&other), ALPN).await.is_err());
            assert!(tokio::time::timeout(Duration::from_millis(100), other.accept()).await.is_err());
        } else {
            tokio::join!(
                async {
                    assert!(matches!(guarded.accept().await.unwrap().await, Err(ConnectingError::LocallyRejected { .. })));
                },
                async {
                    if let Ok(connection) = other.connect(direct(&guarded), ALPN).await {
                        let error = connection.closed().await;
                        match error {
                            iroh::endpoint::ConnectionError::ApplicationClosed(close) => {
                                assert_eq!(close.error_code, 73u32.into());
                                assert_eq!(close.reason.as_ref(), b"ordinary veto");
                            }
                            error => panic!("expected hook's application close: {error}"),
                        }
                    }
                }
            );
        }
    }).await;
    guarded.close().await;
    other.close().await;
    result.expect("ordinary hook veto is bounded");
    assert_eq!(*events.lock().unwrap(), ["first", "veto"]);
}

// Implements precisely the original required methods. Actual I/O goes through
// them; no socket-aware additions are needed for ordinary datagrams.
#[derive(Debug)]
struct ExistingSocket(Box<dyn AsyncUdpSocket>);
#[derive(Debug)]
struct ExistingSender(Pin<Box<dyn UdpSender>>);
impl AsyncUdpSocket for ExistingSocket {
    fn create_sender(&self) -> Pin<Box<dyn UdpSender>> {
        Box::pin(ExistingSender(self.0.create_sender()))
    }
    fn poll_recv(&mut self, cx: &mut Context<'_>, bufs: &mut [IoSliceMut<'_>], meta: &mut [noq::udp::RecvMeta]) -> Poll<io::Result<usize>> {
        self.0.poll_recv(cx, bufs, meta)
    }
    fn local_addr(&self) -> io::Result<SocketAddr> { self.0.local_addr() }
}
impl UdpSender for ExistingSender {
    fn poll_send(mut self: Pin<&mut Self>, transmit: &noq::udp::Transmit<'_>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.0.as_mut().poll_send(transmit, cx)
    }
}

async fn existing_socket() {
    let bind = || {
        let socket = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        socket.set_nonblocking(true).unwrap();
        ExistingSocket(noq::TokioRuntime.wrap_udp_socket(socket).unwrap())
    };
    let mut receive = bind();
    let send = bind();
    let mut sender = send.create_sender();
    let transmit = noq::udp::Transmit {
        destination: receive.local_addr().unwrap(),
        ecn: None, contents: b"legacy socket", segment_size: None, src_ip: None,
    };
    let result = tokio::time::timeout(Duration::from_secs(2), async {
        poll_fn(|cx| sender.as_mut().poll_send(&transmit, cx)).await.unwrap();
        let mut bytes = [0u8; 64];
        let mut meta = [noq::udp::RecvMeta::default()];
        let count = poll_fn(|cx| receive.poll_recv(cx, &mut [IoSliceMut::new(&mut bytes)], &mut meta)).await.unwrap();
        assert_eq!(count, 1);
        assert_eq!(&bytes[..meta[0].len], b"legacy socket");
        assert_eq!(meta[0].addr, send.local_addr().unwrap());
    }).await;
    drop(sender);
    drop(send);
    drop(receive);
    result.expect("legacy required socket methods exchange a real datagram");
}

fn main() {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        incoming_routes().await;
        router().await;
        veto(false).await;
        veto(true).await;
        existing_socket().await;
    });
    println!("ORDINARY_ROUTES_PASS incoming=true accepting=true accept_with=true router_early_data=true hook_veto=true legacy_socket=true natural_close=true");
}
