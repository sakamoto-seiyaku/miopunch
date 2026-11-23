//! Ordinary native application streams on the node-owned member connection.
use std::{
    future::{Future, pending},
    io,
    sync::{Arc, Mutex},
    time::Duration,
};

use futures_util::StreamExt;
use iroh::{
    EndpointAddr, EndpointId,
    endpoint::{
        Connection, ConnectionError, MemberConnection, MemberStream, MemberStreamLease, RecvStream,
        SendStream, StreamControl,
    },
};
use serde_json::{Value, json};
use tokio::{task::JoinSet, time::Instant};

use crate::{data_stream::DataStream, native_client::ClientError, resources::ResourceEndpoint};

#[derive(Clone, Copy)]
#[repr(u8)]
pub(crate) enum MemberService {
    Ping = 0,
    Files = 1,
    Terminal = 2,
    Clipboard = 3,
    TcpForward = 4,
    UdpForward = 5,
    Web = 6,
}

impl MemberService {
    pub(crate) async fn open(self, connection: &MemberConnection) -> io::Result<MemberStream> {
        let mut stream = connection.open_bi().await?;
        stream
            .streams()
            .0
            .write_all(&[self as u8])
            .await
            .map_err(io::Error::other)?;
        Ok(stream)
    }

    const ALL: [Self; 7] = [
        Self::Ping,
        Self::Files,
        Self::Terminal,
        Self::Clipboard,
        Self::TcpForward,
        Self::UdpForward,
        Self::Web,
    ];

    fn legacy_alpn(self) -> &'static [u8] {
        match self {
            Self::Ping => crate::native_client::MEMBER_MESSAGE,
            Self::Files => crate::file_transfer::FILE_ALPN,
            Self::Terminal => crate::terminal::TERMINAL_ALPN,
            Self::Clipboard => crate::clipboard::CLIPBOARD_ALPN,
            Self::TcpForward => crate::forwarding::TCP_ALPN,
            Self::UdpForward => crate::forwarding::UDP_ALPN,
            Self::Web => crate::web::WEB_ALPN,
        }
    }
}

impl From<crate::forwarding::Protocol> for MemberService {
    fn from(protocol: crate::forwarding::Protocol) -> Self {
        match protocol {
            crate::forwarding::Protocol::Tcp => Self::TcpForward,
            crate::forwarding::Protocol::Udp => Self::UdpForward,
        }
    }
}

impl ResourceEndpoint {
    pub(crate) async fn connect_service(
        &self,
        target: EndpointAddr,
        service: MemberService,
    ) -> Result<ServiceTransport, ClientError> {
        self.dial_service(target, service)
            .await?
            .open(service)
            .await
    }

    /// Reuse the node-owned contact backoff through a business recovery round.
    pub(crate) async fn reconnect_service(
        &self,
        target: EndpointAddr,
        service: MemberService,
        deadline: Instant,
    ) -> Result<ServiceTransport, ClientError> {
        tokio::time::timeout_at(deadline, async {
            loop {
                match self.connect_service(target.clone(), service).await {
                    Ok(next) => return Ok(next),
                    Err(error) if crate::business_channel::retryable(&error) => {
                        tokio::time::sleep(self.member_retry_after(target.id, error.to_string()))
                            .await;
                    }
                    Err(error) => return Err(error),
                }
            }
        })
        .await
        .map_err(|_| crate::business_channel::RecoveryExpired)?
    }

    pub(crate) async fn dial_service(
        &self,
        target: EndpointAddr,
        service: MemberService,
    ) -> Result<ServiceContact, ClientError> {
        let (_, gate) = self
            .member_fusion
            .as_ref()
            .expect("native client member owner");
        let alpn = service.legacy_alpn();
        let deadline = Instant::now() + crate::connection_budget::DEFAULT;
        tokio::time::timeout_at(deadline, async {
            if !gate.member_contact_allowed(target.id)
                && gate.scoped_guest_contact_allowed(target.id, alpn)
            {
                return Ok(ServiceContact::Legacy(self.connect(target, alpn).await?));
            }
            match self.connect_member(target.clone(), deadline).await {
                Ok(connection) => Ok(ServiceContact::Member(connection)),
                // Browser nodes still advertise the dedicated application ALPNs.
                Err(error)
                    if error.kind() == io::ErrorKind::Unsupported
                        && matches!(service, MemberService::Files | MemberService::Clipboard) =>
                {
                    Ok(ServiceContact::Legacy(self.connect(target, alpn).await?))
                }
                Err(error) => Err(error.into()),
            }
        })
        .await?
    }
}

pub(crate) enum ServiceContact {
    Legacy(Connection),
    Member(MemberConnection),
}

impl ServiceContact {
    pub(crate) async fn open(
        &self,
        service: MemberService,
    ) -> Result<ServiceTransport, ClientError> {
        match self {
            Self::Legacy(connection) => Ok(connection.clone().into()),
            Self::Member(connection) => Ok(ServiceTransport::member(
                connection,
                service.open(connection).await?,
            )),
        }
    }

    pub(crate) fn status(&self) -> Value {
        match self {
            Self::Legacy(connection) => crate::native_client::path_status(connection),
            Self::Member(connection) => path_status(connection),
        }
    }

    pub(crate) fn end_legacy(&self, reason: &[u8]) {
        if let Self::Legacy(connection) = self {
            connection.close(0u32.into(), reason);
        }
    }
}

#[derive(Clone)]
pub(crate) enum ServiceTransport {
    Legacy(Connection),
    Member(Arc<ServiceStream>),
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum PeerStreamEnd {
    ReleasedReader,
    Disconnected,
}

pub(crate) struct ServiceStream {
    connection: MemberConnection,
    stream: String,
    halves: Mutex<Option<(SendStream, RecvStream)>>,
    control: StreamControl,
    id: (usize, Option<u64>),
    _lease: MemberStreamLease,
}

impl From<Connection> for ServiceTransport {
    fn from(connection: Connection) -> Self {
        Self::Legacy(connection)
    }
}

impl ServiceTransport {
    fn member(connection: &MemberConnection, stream: MemberStream) -> Self {
        let control = stream.control();
        let (send, recv, lease) = stream.into_parts();
        let id = (connection.stable_id(), Some(send.id().into()));
        Self::Member(Arc::new(ServiceStream {
            connection: connection.clone(),
            stream: send.id().to_string(),
            halves: Mutex::new(Some((send, recv))),
            control,
            id,
            _lease: lease,
        }))
    }

    pub(crate) fn is_member(&self) -> bool {
        matches!(self, Self::Member(_))
    }

    pub(crate) fn member_connection(&self) -> Option<MemberConnection> {
        match self {
            Self::Legacy(_) => None,
            Self::Member(stream) => Some(stream.connection.clone()),
        }
    }

    pub(crate) fn network_interrupted(&self) -> bool {
        let reason = match self {
            Self::Legacy(connection) => connection.close_reason(),
            Self::Member(stream) => stream.connection.close_reason(),
        };
        matches!(
            reason,
            Some(ConnectionError::TimedOut | ConnectionError::Reset)
        ) || matches!(self, Self::Member(stream) if stream.connection.was_superseded())
    }

    pub(crate) fn remote_id(&self) -> EndpointId {
        match self {
            Self::Legacy(connection) => connection.remote_id(),
            Self::Member(stream) => stream.connection.remote_id(),
        }
    }

    pub(crate) fn status(&self) -> Value {
        match self {
            Self::Legacy(connection) => crate::native_client::path_status(connection),
            Self::Member(stream) => {
                let mut status = path_status(&stream.connection);
                status["stream"] = json!(stream.stream);
                status
            }
        }
    }

    pub(crate) fn same_flow(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Legacy(a), Self::Legacy(b)) => a.stable_id() == b.stable_id(),
            (Self::Member(a), Self::Member(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    pub(crate) fn flow_id(&self) -> (usize, Option<u64>) {
        match self {
            Self::Legacy(connection) => (connection.stable_id(), None),
            Self::Member(stream) => stream.id,
        }
    }

    pub(crate) async fn stream(&self, outgoing: bool) -> io::Result<DataStream> {
        match self {
            Self::Legacy(connection) if outgoing => DataStream::open(connection).await,
            Self::Legacy(connection) => DataStream::accept(connection).await,
            Self::Member(_) => {
                let (send, recv) = self.stream_halves(outgoing).await?;
                Ok(DataStream::from_parts(send, recv))
            }
        }
    }

    pub(crate) async fn stream_halves(
        &self,
        outgoing: bool,
    ) -> io::Result<(SendStream, RecvStream)> {
        match self {
            Self::Legacy(connection) if outgoing => {
                connection.open_bi().await.map_err(io::Error::other)
            }
            Self::Legacy(connection) => connection.accept_bi().await.map_err(io::Error::other),
            Self::Member(stream) => stream
                .halves
                .lock()
                .unwrap()
                .take()
                .ok_or_else(|| io::Error::other("service stream already opened")),
        }
    }

    pub(crate) fn end(&self, legacy_reason: &[u8]) {
        if let Self::Legacy(connection) = self {
            connection.close(0u32.into(), legacy_reason);
        }
    }

    pub(crate) fn abort(&self, code: u32, legacy_reason: &[u8]) {
        match self {
            Self::Legacy(connection) => connection.close(code.into(), legacy_reason),
            Self::Member(stream) => stream.control.abort(code.into()),
        }
    }

    pub(crate) async fn closed(&self) {
        self.peer_end().await;
    }

    pub(crate) async fn peer_end(&self) -> PeerStreamEnd {
        match self {
            Self::Legacy(connection) => {
                connection.closed().await;
                PeerStreamEnd::Disconnected
            }
            Self::Member(stream) => {
                tokio::select! {
                    _ = stream.control.connection_closed() => PeerStreamEnd::Disconnected,
                    end = async {
                        match stream.control.stopped().await {
                            Ok(Some(code)) if code == 0u32.into() => PeerStreamEnd::ReleasedReader,
                            Ok(Some(_)) | Err(_) => PeerStreamEnd::Disconnected,
                            Ok(None) => pending().await,
                        }
                    } => end,
                }
            }
        }
    }

    pub(crate) async fn received(
        &self,
        legacy_reason: &[u8],
        receipt: impl Future<Output = io::Result<Vec<u8>>>,
    ) -> bool {
        match self {
            Self::Legacy(connection) => matches!(connection.closed().await,
                ConnectionError::ApplicationClosed(reason) if reason.error_code == 0u32.into() && reason.reason.as_ref() == legacy_reason),
            Self::Member(_) => receipt.await.is_ok_and(|bytes| bytes.is_empty()),
        }
    }

    pub(crate) async fn wait_for_drain(&self) {
        match self {
            Self::Legacy(connection) => {
                connection.closed().await;
            }
            Self::Member(stream) => {
                let _ = stream.control.stopped().await;
            }
        }
    }

    pub(crate) fn peer_cancelled(&self) -> bool {
        match self {
            Self::Legacy(connection) => crate::file_transfer::peer_cancelled(connection),
            Self::Member(stream) => stream.control.stop_code() == Some(1u32.into()),
        }
    }
}

impl crate::file_transfer::FileTransport for ServiceTransport {
    fn status(&self) -> Value {
        self.status()
    }
    fn abort(&self, code: u32, reason: &[u8]) {
        self.abort(code, reason);
    }
    fn peer_cancelled(&self) -> bool {
        self.peer_cancelled()
    }
    async fn closed(&self) {
        self.closed().await;
    }
}

#[derive(Clone)]
pub(crate) struct MemberServices {
    pub(crate) member_contacts: Option<Arc<crate::member_contacts::MemberContacts>>,
    pub(crate) terminal: Arc<crate::native_terminal::NativeTerminal>,
    pub(crate) files: Arc<crate::native_file_transfer::NativeFiles>,
    pub(crate) forwarding: Arc<crate::native_forwarding::NativeForwarding>,
    pub(crate) web: Arc<crate::native_web::NativeWeb>,
    pub(crate) clipboard: Arc<crate::native_clipboard::NativeClipboard>,
    pub(crate) node: Arc<tokio::sync::Mutex<crate::native_network::NativeNetwork>>,
    pub(crate) policies: Arc<tokio::sync::Mutex<crate::native_services::NativeServices>>,
}

pub(crate) async fn receive(
    connection: MemberConnection,
    services: MemberServices,
    mut path_events: iroh::endpoint::PathEventStream,
) {
    let learning = async {
        while let Some(event) = path_events.next().await {
            if let Some(contacts) = &services.member_contacts
                && let Err(error) = contacts.observe_event(connection.remote_id(), event).await
            {
                eprintln!("could not save retired member path: {error}");
            }
        }
    };
    let business = async {
        let mut handlers = JoinSet::new();
        loop {
            tokio::select! {
                _ = connection.closed() => break,
                _ = handlers.join_next(), if !handlers.is_empty() => {},
                result = connection.incoming_bi(), if handlers.len() < 100 => {
                    let Ok(incoming) = result else { break };
                    let connection = connection.clone();
                    let services = services.clone();
                    handlers.spawn(async move {
                        let Ok(Ok((service, mut stream))) = tokio::time::timeout(Duration::from_secs(60), async {
                            let mut stream = incoming.accept().await?;
                            if !stream.is_business() { return Err(io::ErrorKind::InvalidData.into()); }
                            let mut header = [0];
                            stream.streams().1.read_exact(&mut header).await.map_err(io::Error::other)?;
                            let service = *MemberService::ALL.get(usize::from(header[0])).ok_or(io::ErrorKind::InvalidData)?;
                            Ok::<_, io::Error>((service, stream))
                        }).await else { return };
                        if let MemberService::Ping = service {
                            let _ = tokio::time::timeout(Duration::from_secs(5), async {
                            let (send, recv) = stream.streams();
                            let request = recv.read_to_end(64).await?;
                            crate::member_message::respond(&request, send).await?;
                            let _ = send.stopped().await?;
                            Ok::<_, crate::native_client::ClientError>(())
                            }).await;
                            return;
                        }
                        let stream = ServiceTransport::member(&connection, stream);
                        match service {
                            MemberService::Files => services.files.receive(stream),
                            MemberService::Terminal => { let _ = services.terminal.receive(stream).await; }
                            MemberService::TcpForward => { let _ = services.forwarding.receive(stream, crate::forwarding::Protocol::Tcp).await; }
                            MemberService::UdpForward => { let _ = services.forwarding.receive(stream, crate::forwarding::Protocol::Udp).await; }
                            MemberService::Web => { let _ = services.web.receive(stream).await; }
                            MemberService::Clipboard => {
                                let request = services.clipboard.state.request();
                                let _ = services.clipboard.run(stream, services.node, services.policies, false, request).await;
                            }
                            MemberService::Ping => unreachable!("handled before service stream dispatch"),
                        }
                    });
                }
            }
        }
    };
    // Connection closure wakes business before the remote actor publishes its
    // final path statistics. Let the existing event stream finish naturally.
    tokio::join!(business, learning);
}

pub(crate) fn path_status(connection: &MemberConnection) -> Value {
    let mut status = json!({"peer":connection.remote_id().to_string(), "connection":connection.stable_id(),
        "initial_data_path":connection.initial_data_path(),
        "paths":connection.paths().iter().map(|path| {
            let stats = path.stats();
            json!({"id":path.id().to_string(),"remote":path.remote_addr(),
            "local":format!("{:?}",path.local_addr()),
            "selected":path.is_selected(),"tx_bytes":stats.udp_tx.bytes,"rx_bytes":stats.udp_rx.bytes,
            "rx_stream_frames":stats.frame_rx.stream,"tx_stream_frames":stats.frame_tx.stream,
            "last_ack_age_ms":stats.acknowledgment.map(|ack| ack.received_at.elapsed().as_millis())})
        }).collect::<Vec<_>>()});
    if let Some(round) = connection.udp_traversal()
        && let Some((matched, matched_at)) = round.first_match
        && let Some((path, ack)) = round.validated
    {
        status["enhanced_udp"] = json!({
            "round":hex::encode(round.round), "connection":round.connection_id,
            "path":path.to_string(), "peer":matched.remote, "binding":matched.local_addr,
            "socket":format!("{:?}", matched.local_socket_id),
            "fresh_ack":ack.sent_at >= matched_at,
            "selected_round":round.outcome == Some(Ok(())),
            "learned":round.learned,
        });
    }
    status
}

impl crate::business_channel::ChannelTransport for ServiceTransport {
    fn network_interrupted(&self) -> bool {
        ServiceTransport::network_interrupted(self)
    }
    async fn closed(&self) {
        ServiceTransport::closed(self).await;
    }
}
