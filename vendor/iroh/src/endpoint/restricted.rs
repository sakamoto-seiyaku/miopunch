//! Typed admission on the existing endpoint and connection implementation.

use std::{future::IntoFuture, io, pin::Pin, sync::Arc};

use iroh_base::{EndpointAddr, EndpointId, TransportAddr};
use n0_error::e;
use n0_future::time::{self, Duration, Instant};
use tokio::sync::Mutex;

use super::{
    Accepting, AfterHandshakeOutcome, AlpnError, BindError, Builder, ConnectError, ConnectOptions,
    ConnectWithOptsError, Connecting, ConnectingError, Connection, Endpoint,
    ExportKeyingMaterialError, Incoming, IncomingAddr, LocalTransportAddr, PathId, PathList,
    RecvStream, SendStream, ServerConfig, ServerConfigBuilder, Side, VarInt,
    connection::{AttemptOptions, RegisteredConnection},
};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Admission decision made through a restricted, authenticated control handle.
#[derive(Debug)]
#[non_exhaustive]
pub enum AdmissionOutcome {
    /// No decision has been made. Business delivery is refused.
    Pending,
    /// Accept only the dedicated control protocol, without ordinary business.
    ///
    /// Use [`RestrictedEndpoint::connect_control`] or
    /// [`RestrictedIncoming::accept_control`] to receive this result. Ordinary
    /// connect/accept futures reject this scope instead of exposing Connection.
    ControlOnly,
    /// Admit business over a path freshly qualified on this same connection.
    Accept(DataPathPermit),
    /// Reject the connection with an application close reason.
    Reject {
        /// Application close code sent to the peer.
        error_code: VarInt,
        /// Application close reason sent to the peer.
        reason: Vec<u8>,
    },
}

/// Application admission policy for [`RestrictedEndpoint`].
///
/// The library authenticates the remote TLS identity before invoking this
/// interface. The application supplies its authorization and coordination
/// protocol through bounded control messages. An accepted data path does not
/// itself grant application authorization: return [`AdmissionOutcome::Accept`]
/// only after the application's checks have succeeded.
///
/// Policies must not retain an endpoint, which would create an ownership cycle.
/// The supplied control handle cannot be cloned or converted to a connection.
pub trait EndpointAdmission: std::fmt::Debug + Send + Sync {
    /// Exchange protected control messages and decide admission.
    ///
    /// The default is deliberately undecided. Missing policy work cannot grant
    /// business access. The whole future shares the attempt's original deadline
    /// and cancellation; dropping it closes the pending connection.
    fn admit<'a>(
        &'a self,
        _connection: &'a ControlConnection,
    ) -> impl Future<Output = io::Result<AdmissionOutcome>> + Send + 'a {
        async { Ok(AdmissionOutcome::Pending) }
    }

    /// Whether the application's decision is still valid now.
    ///
    /// Called again after legacy hooks and immediately before handoff. Use the
    /// current authorization/resource revision rather than an admission-time
    /// snapshot. Continuing revocation after handoff remains the application's
    /// policy responsibility, enforced by its existing connection/path guards.
    fn is_current(&self, connection: &ControlConnection) -> bool;
}

pub(crate) trait DynEndpointAdmission: std::fmt::Debug + Send + Sync {
    fn admit<'a>(
        &'a self,
        connection: &'a ControlConnection,
    ) -> BoxFuture<'a, io::Result<AdmissionOutcome>>;
    fn is_current(&self, connection: &ControlConnection) -> bool;
}

impl<T: EndpointAdmission> DynEndpointAdmission for T {
    fn admit<'a>(
        &'a self,
        connection: &'a ControlConnection,
    ) -> BoxFuture<'a, io::Result<AdmissionOutcome>> {
        Box::pin(EndpointAdmission::admit(self, connection))
    }

    fn is_current(&self, connection: &ControlConnection) -> bool {
        EndpointAdmission::is_current(self, connection)
    }
}

/// Builder selected by [`Builder::restricted`].
///
/// Keys, transports and ordinary hooks are configured using the original builder
/// before selecting restricted admission. Both endpoint types bind the same
/// socket, TLS, QUIC and path implementation.
#[derive(Debug)]
pub struct RestrictedBuilder {
    inner: Builder,
    admission_timeout: Duration,
}

impl RestrictedBuilder {
    pub(super) fn new(inner: Builder) -> Self {
        Self {
            inner,
            admission_timeout: Duration::from_secs(60),
        }
    }

    /// Bound restricted setup, including TLS, control, hooks and delivery.
    ///
    /// Defaults to 60 seconds. An earlier caller deadline always wins. Incoming
    /// attempts start this budget at receipt, before ALPN routing; no stage resets
    /// it. Successful connections do not expire with this setup budget.
    pub fn admission_timeout(mut self, timeout: Duration) -> Self {
        self.admission_timeout = timeout;
        self
    }

    /// Bind an endpoint which exposes only restricted establishment entry points.
    pub async fn bind(mut self) -> Result<RestrictedEndpoint, BindError> {
        if self.inner.admission_gated
            || !self.inner.control_only_alpns.is_empty()
            || self.admission_timeout.is_zero()
        {
            return Err(e!(BindError::InvalidAdmissionConfig));
        }
        self.inner.admission_gated = true;
        let inner = self.inner.bind().await?;
        Ok(RestrictedEndpoint {
            inner,
            admission_timeout: self.admission_timeout,
        })
    }
}

/// An endpoint with an explicit authenticated control phase before business.
///
/// This owns an ordinary endpoint internally but exposes no conversion, full
/// object borrow, or early-data entry point. Awaiting connect/accept returns the
/// original [`Connection`] only after admission, a fresh data-path confirmation,
/// legacy hooks, and bilateral delivery readiness have succeeded.
#[derive(Debug, Clone)]
pub struct RestrictedEndpoint {
    pub(super) inner: Endpoint,
    admission_timeout: Duration,
}

impl RestrictedEndpoint {
    pub(crate) fn as_endpoint(&self) -> &Endpoint {
        &self.inner
    }

    pub(crate) fn admission_timeout(&self) -> Duration {
        self.admission_timeout
    }

    /// The endpoint's cryptographic identity.
    pub fn id(&self) -> EndpointId {
        self.inner.id()
    }

    /// The current contact addresses of this endpoint.
    pub fn addr(&self) -> EndpointAddr {
        self.inner.addr()
    }

    /// Configure incoming application protocols, preserving restricted admission.
    pub fn set_alpns(&self, alpns: Vec<Vec<u8>>) {
        self.inner.set_alpns(alpns);
    }

    /// Build a server configuration for [`RestrictedIncoming::accept_with`].
    ///
    /// Custom TLS configuration does not expose early business handles or bypass
    /// the authenticated admission and delivery stages.
    pub fn create_server_config_builder(&self, alpns: Vec<Vec<u8>>) -> ServerConfigBuilder {
        self.inner.create_server_config_builder(alpns)
    }

    /// Connect through restricted admission, then deliver an ordinary connection.
    pub async fn connect(
        &self,
        endpoint_addr: impl Into<EndpointAddr>,
        alpn: &[u8],
    ) -> Result<Connection, ConnectError> {
        Ok(self
            .connect_with_opts(endpoint_addr, alpn, ConnectOptions::default())
            .await?
            .await?)
    }

    /// Start a connection with one deadline shared by setup and admission.
    pub async fn connect_with_opts(
        &self,
        endpoint_addr: impl Into<EndpointAddr>,
        alpn: &[u8],
        mut options: ConnectOptions,
    ) -> Result<RestrictedConnecting, ConnectWithOptsError> {
        set_deadline(
            &mut options.attempt,
            Instant::now() + self.admission_timeout,
        );
        let attempt = options.attempt.clone();
        let inner = bounded(
            &attempt,
            self.inner.connect_with_opts(endpoint_addr, alpn, options),
        )
        .await?;
        Ok(RestrictedConnecting { inner })
    }

    /// Establish a control-only session, with no ordinary connection conversion.
    pub async fn connect_control(
        &self,
        endpoint_addr: impl Into<EndpointAddr>,
        alpn: &[u8],
    ) -> Result<ControlConnection, ConnectError> {
        Ok(self
            .connect_control_with_opts(endpoint_addr, alpn, ConnectOptions::default())
            .await?
            .await?)
    }

    /// Start a control-only attempt with explicit configuration and cancellation.
    pub async fn connect_control_with_opts(
        &self,
        endpoint_addr: impl Into<EndpointAddr>,
        alpn: &[u8],
        options: ConnectOptions,
    ) -> Result<ControlConnecting, ConnectWithOptsError> {
        Ok(ControlConnecting {
            inner: self.connect_with_opts(endpoint_addr, alpn, options).await?,
        })
    }

    /// Receive an incoming attempt without exposing the ordinary incoming type.
    pub async fn accept(&self) -> Option<RestrictedIncoming> {
        let mut inner = self.inner.accept().await?;
        let deadline = inner.received_at() + self.admission_timeout;
        set_deadline(&mut inner.attempt, deadline);
        Some(RestrictedIncoming { inner })
    }

    /// Shut down the shared endpoint, its connections and transports.
    pub async fn close(&self) {
        self.inner.close().await;
    }

    /// Whether endpoint shutdown has completed.
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }
}

fn set_deadline(attempt: &mut AttemptOptions, deadline: Instant) {
    attempt.caller_deadline = Some(
        attempt
            .caller_deadline
            .map_or(deadline, |old| old.min(deadline)),
    );
}

/// A restricted outgoing attempt. It offers no early-data conversion.
#[derive(Debug)]
pub struct RestrictedConnecting {
    inner: Connecting,
}

impl RestrictedConnecting {
    /// The identity being dialed, still subject to TLS authentication.
    pub fn remote_id(&self) -> EndpointId {
        self.inner.remote_id()
    }

    /// Inspect negotiated ALPN under the same attempt budget.
    pub async fn alpn(&mut self) -> Result<Vec<u8>, AlpnError> {
        let attempt = self.inner.attempt.clone();
        bounded(&attempt, self.inner.alpn()).await
    }
}

impl IntoFuture for RestrictedConnecting {
    type Output = Result<Connection, ConnectingError>;
    type IntoFuture = BoxFuture<'static, Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let attempt = self.inner.attempt.clone();
            bounded(&attempt, self.inner).await
        })
    }
}

/// A control-only outgoing attempt, with no early-data or business conversion.
#[derive(Debug)]
pub struct ControlConnecting {
    inner: RestrictedConnecting,
}

impl ControlConnecting {
    /// The identity being dialed, still subject to TLS authentication.
    pub fn remote_id(&self) -> EndpointId {
        self.inner.remote_id()
    }

    /// Inspect negotiated ALPN under the same attempt budget.
    pub async fn alpn(&mut self) -> Result<Vec<u8>, AlpnError> {
        self.inner.alpn().await
    }
}

impl IntoFuture for ControlConnecting {
    type Output = Result<ControlConnection, ConnectingError>;
    type IntoFuture = BoxFuture<'static, Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let attempt = self.inner.inner.attempt.clone();
            bounded(&attempt, self.inner.inner.into_control()).await
        })
    }
}

/// An incoming attempt whose public acceptance paths enforce typed admission.
#[derive(Debug)]
pub struct RestrictedIncoming {
    inner: Incoming,
}

impl RestrictedIncoming {
    /// Original receipt time, also used by the default admission budget.
    pub fn received_at(&self) -> Instant {
        self.inner.received_at()
    }

    /// Add an earlier caller deadline without refreshing the receipt budget.
    pub fn with_caller_deadline(mut self, deadline: Instant) -> Self {
        set_deadline(&mut self.inner.attempt, deadline);
        self
    }

    /// Cancel this attempt independently of the endpoint.
    pub fn with_attempt_cancellation(
        mut self,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Self {
        self.inner = self.inner.with_attempt_cancellation(cancellation);
        self
    }

    /// Accept with the endpoint's TLS configuration and admission policy.
    pub fn accept(self) -> Result<RestrictedAccepting, super::ConnectionError> {
        Ok(RestrictedAccepting {
            inner: self.inner.accept()?,
        })
    }

    /// Accept with custom TLS configuration while retaining admission enforcement.
    pub fn accept_with(
        self,
        server_config: Arc<ServerConfig>,
    ) -> Result<RestrictedAccepting, super::ConnectionError> {
        Ok(RestrictedAccepting {
            inner: self.inner.accept_with(server_config)?,
        })
    }

    /// Accept only a control-only result from the admission policy.
    pub fn accept_control(self) -> Result<ControlAccepting, super::ConnectionError> {
        Ok(self.accept()?.into_control())
    }

    /// Accept a control-only session with a custom server configuration.
    pub fn accept_control_with(
        self,
        server_config: Arc<ServerConfig>,
    ) -> Result<ControlAccepting, super::ConnectionError> {
        Ok(self.accept_with(server_config)?.into_control())
    }

    /// Reject this attempt before accepting it.
    pub fn refuse(self) {
        self.inner.refuse();
    }

    /// Ignore this attempt before accepting it.
    pub fn ignore(self) {
        self.inner.ignore();
    }

    /// The observed remote transport address; this grants no authorization.
    pub fn remote_addr(&self) -> IncomingAddr {
        self.inner.remote_addr()
    }

    /// The local transport address which received this attempt.
    pub fn local_addr(&self) -> LocalTransportAddr {
        self.inner.local_addr()
    }
}

impl IntoFuture for RestrictedIncoming {
    type Output = Result<Connection, ConnectingError>;
    type IntoFuture = BoxFuture<'static, Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.accept()?.await })
    }
}

/// An accepted TLS attempt with no early-data or ordinary connection access.
#[derive(Debug)]
pub struct RestrictedAccepting {
    inner: Accepting,
}

impl RestrictedAccepting {
    pub(crate) fn from_accepting(inner: Accepting) -> Self {
        Self { inner }
    }

    /// Select control-only delivery; this grants no new connection capability.
    pub fn into_control(self) -> ControlAccepting {
        ControlAccepting { inner: self }
    }

    /// Original receipt time, preserved across ALPN routing.
    pub fn received_at(&self) -> Instant {
        self.inner.received_at()
    }

    /// Inspect the negotiated ALPN without running application admission yet.
    pub async fn alpn(&mut self) -> Result<Vec<u8>, AlpnError> {
        let attempt = self.inner.attempt.clone();
        bounded(&attempt, self.inner.alpn()).await
    }

    /// The remote transport address, before application admission.
    pub fn remote_addr(&self) -> IncomingAddr {
        self.inner.remote_addr()
    }

    /// Add an earlier caller deadline before awaiting this attempt.
    pub fn with_caller_deadline(mut self, deadline: Instant) -> Self {
        set_deadline(&mut self.inner.attempt, deadline);
        self
    }

    /// Associate this accepted attempt with the caller's cancellation token.
    pub fn with_attempt_cancellation(
        mut self,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Self {
        self.inner = self.inner.with_attempt_cancellation(cancellation);
        self
    }
}

impl IntoFuture for RestrictedAccepting {
    type Output = Result<Connection, ConnectingError>;
    type IntoFuture = BoxFuture<'static, Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let attempt = self.inner.attempt.clone();
            bounded(&attempt, self.inner).await
        })
    }
}

/// An incoming attempt which can deliver only the dedicated control handle.
#[derive(Debug)]
pub struct ControlAccepting {
    inner: RestrictedAccepting,
}

impl ControlAccepting {
    /// Original receipt time, preserved across routing and scope selection.
    pub fn received_at(&self) -> Instant {
        self.inner.received_at()
    }

    /// Inspect ALPN without exposing the ordinary accepting state.
    pub async fn alpn(&mut self) -> Result<Vec<u8>, AlpnError> {
        self.inner.alpn().await
    }

    /// Attach an earlier caller deadline before awaiting this attempt.
    pub fn with_caller_deadline(mut self, deadline: Instant) -> Self {
        self.inner = self.inner.with_caller_deadline(deadline);
        self
    }

    /// Cancel this control-only attempt independently of the endpoint.
    pub fn with_attempt_cancellation(
        mut self,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Self {
        self.inner = self.inner.with_attempt_cancellation(cancellation);
        self
    }
}

impl IntoFuture for ControlAccepting {
    type Output = Result<ControlConnection, ConnectingError>;
    type IntoFuture = BoxFuture<'static, Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let attempt = self.inner.inner.attempt.clone();
            bounded(&attempt, self.inner.inner.into_control()).await
        })
    }
}

/// Authenticated control capability supplied to [`EndpointAdmission`].
///
/// The sole private bidirectional stream has a versioned admission preface and
/// bounded length-delimited messages. Raw streams and datagrams are not exposed.
/// This is protocol separation, not payload classification: the application
/// must validate its control messages and must not put business payloads in them.
/// A peer sending ordinary stream bytes during control fails the framing check.
/// The control stream is retired before an ordinary connection is handed off.
#[derive(Debug)]
pub struct ControlConnection {
    connection: Connection,
    endpoint: Endpoint,
    channel: ControlChannel,
    close_on_drop: bool,
}

impl Drop for ControlConnection {
    fn drop(&mut self) {
        if self.close_on_drop {
            self.connection
                .close(0u32.into(), b"restricted control owner dropped");
        }
    }
}

impl ControlConnection {
    /// Authenticated remote identity from TLS.
    pub fn remote_id(&self) -> EndpointId {
        self.connection.remote_id()
    }

    /// Identity of this endpoint.
    pub fn local_id(&self) -> EndpointId {
        self.endpoint.id()
    }

    /// Negotiated application protocol.
    pub fn alpn(&self) -> &[u8] {
        self.connection.alpn()
    }

    /// Original connection identity, unchanged across path operations.
    pub fn stable_id(&self) -> usize {
        self.connection.stable_id()
    }

    /// QUIC initiator or acceptor role.
    pub fn side(&self) -> Side {
        self.connection.side()
    }

    /// The single absolute deadline shared by the entire pending attempt.
    pub fn caller_deadline(&self) -> Option<Instant> {
        self.connection.caller_deadline()
    }

    /// The individual attempt's cancellation token, if supplied by the caller.
    pub fn attempt_cancellation(&self) -> Option<tokio_util::sync::CancellationToken> {
        self.connection.attempt_cancellation()
    }

    /// Bind the application control protocol to this authenticated TLS session.
    pub fn export_keying_material(
        &self,
        output: &mut [u8],
        label: &[u8],
        context: &[u8],
    ) -> Result<(), ExportKeyingMaterialError> {
        self.connection
            .export_keying_material(output, label, context)
    }

    /// Snapshot of this connection's actual paths, without connection upgrades.
    pub fn paths(&self) -> PathList<'_> {
        self.connection.paths()
    }

    /// Start validating an authorized UDP candidate on this original session.
    ///
    /// Only the QUIC dialing side can open a path. The existing connection,
    /// socket ownership and transport guards apply; this does not return a
    /// business handle or qualify the candidate. Observe an established path
    /// and obtain a fresh [`DataPathPermit`] before accepting business scope.
    pub async fn open_data_path(&self, remote: std::net::SocketAddr) -> io::Result<()> {
        self.endpoint.open_data_path(&self.connection, remote).await
    }

    /// Send one control message, limited to 64 KiB.
    ///
    /// This never opens an application stream. Cancelling a partially completed
    /// frame invalidates that control exchange; the pending attempt must be dropped.
    pub async fn send_control(&self, message: &[u8]) -> io::Result<()> {
        self.channel.send_frame(CONTROL_MESSAGE, message).await
    }

    /// Receive one complete, bounded control message.
    ///
    /// Receiving a readiness frame here is a control-protocol ordering error.
    /// Cancellation during a frame likewise requires dropping the attempt.
    pub async fn recv_control(&self) -> io::Result<Vec<u8>> {
        let (kind, message) = self.channel.recv_frame().await?;
        if kind != CONTROL_MESSAGE {
            return Err(io::Error::other(
                "peer finished admission before control exchange",
            ));
        }
        Ok(message)
    }

    /// Freshly confirm an explicitly permitted data path on this connection.
    ///
    /// The returned proof cannot be constructed from cached reachability or
    /// another connection. Custom transports must also satisfy their live
    /// connection-owned qualification and resource policy. The final handoff
    /// rechecks the proof, including path generation, after ordinary hooks.
    pub async fn qualify_data_path(
        &self,
        path_id: PathId,
        allowed_data_custom_ids: Vec<u64>,
    ) -> io::Result<DataPathPermit> {
        let paths = self.connection.paths();
        let path = paths
            .iter()
            .find(|path| path.id() == path_id)
            .ok_or_else(|| io::Error::other("data path is not established on this connection"))?;
        let remote = path.remote_addr().clone();
        if !self.endpoint.inner.qualifies_admission_data_path(
            self.stable_id(),
            &remote,
            &allowed_data_custom_ids,
        ) {
            return Err(io::Error::other(
                "data path is not currently permitted and qualified",
            ));
        }
        let probe = path.start_probe()?;
        let permit = DataPathPermit {
            connection_id: self.stable_id(),
            path_id,
            remote,
            probe,
            allowed_data_custom_ids,
        };
        loop {
            if permit.probe.is_acknowledged().map_err(io::Error::other)? {
                permit.check(self)?;
                return Ok(permit);
            }
            time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// Close this pending connection without exposing a business capability.
    pub fn close(&self, error_code: VarInt, reason: &[u8]) {
        self.connection.close(error_code, reason);
    }

    /// Retain liveness/closure authority without retaining or upgrading business.
    ///
    /// This weak handle remains useful after ordinary Connection handoff, so an
    /// admission policy can close the original session when authorization changes.
    pub fn weak_handle(&self) -> WeakControlConnection {
        WeakControlConnection {
            inner: self.connection.weak_handle(),
        }
    }

    /// Wait for this control session to close.
    pub async fn closed(&self) -> super::ConnectionError {
        self.connection.closed().await
    }
}

/// Non-owning liveness and revocation authority for an authenticated session.
///
/// Unlike [`super::WeakConnectionHandle`], this type has no public upgrade.
/// Keeping it does not keep a connection alive or expose ordinary business
/// methods, either during admission or after business handoff.
#[derive(Debug, Clone)]
pub struct WeakControlConnection {
    inner: super::WeakConnectionHandle,
}

impl WeakControlConnection {
    /// Whether the original session is still open.
    pub fn is_open(&self) -> bool {
        self.inner.is_open()
    }

    /// Issue a local close if the original session still exists and is open.
    ///
    /// True means a local close was issued, not that the peer acknowledged it.
    pub fn close(&self, error_code: VarInt, reason: &[u8]) -> bool {
        let Some(connection) = self.inner.upgrade() else {
            return false;
        };
        if connection.close_reason().is_some() {
            return false;
        }
        connection.close(error_code, reason);
        true
    }

    /// Wait for closure without extending the original connection's lifetime.
    pub fn closed(&self) -> impl Future<Output = Option<super::Closed>> + Send + 'static {
        self.inner.closed()
    }
}

/// A fresh authenticated acknowledgment bound to one original data path.
///
/// Created only through [`ControlConnection::qualify_data_path`]. It grants no
/// application authorization by itself and exposes no underlying connection.
#[derive(Debug)]
pub struct DataPathPermit {
    connection_id: usize,
    path_id: PathId,
    remote: TransportAddr,
    probe: noq::PathProbe,
    allowed_data_custom_ids: Vec<u64>,
}

impl DataPathPermit {
    fn check(&self, control: &ControlConnection) -> io::Result<()> {
        if self.connection_id != control.stable_id()
            || control.connection.close_reason().is_some()
            || !control
                .connection
                .paths()
                .iter()
                .any(|path| path.id() == self.path_id && path.remote_addr() == &self.remote)
            || !control.endpoint.inner.qualifies_admission_data_path(
                self.connection_id,
                &self.remote,
                &self.allowed_data_custom_ids,
            )
            || !self.probe.is_acknowledged().map_err(io::Error::other)?
        {
            return Err(io::Error::other(
                "data-path confirmation is no longer current",
            ));
        }
        Ok(())
    }
}

const CONTROL_PREFACE: &[u8] = b"iroh/admission/1\0";
const MAX_CONTROL_MESSAGE: usize = 64 * 1024;
const CONTROL_MESSAGE: u8 = 0;
const DELIVERY_READY: u8 = 1;

#[derive(Debug)]
struct ControlChannel {
    send: Mutex<Option<SendStream>>,
    recv: Mutex<Option<RecvStream>>,
}

impl ControlChannel {
    async fn new(connection: &Connection) -> io::Result<Self> {
        let (mut send, mut recv) = if connection.side().is_client() {
            connection.open_bi().await.map_err(io::Error::other)?
        } else {
            connection.accept_bi().await.map_err(io::Error::other)?
        };
        // This is the first client-initiated bidirectional stream. No caller
        // can allocate an earlier stream through the restricted interface.
        if u64::from(send.id()) != 0 || u64::from(recv.id()) != 0 {
            return Err(io::Error::other("unexpected admission control stream"));
        }
        send.write_all(CONTROL_PREFACE)
            .await
            .map_err(io::Error::other)?;
        let mut preface = [0; CONTROL_PREFACE.len()];
        recv.read_exact(&mut preface)
            .await
            .map_err(io::Error::other)?;
        if preface != CONTROL_PREFACE {
            return Err(io::Error::other("invalid admission control preface"));
        }
        Ok(Self {
            send: Mutex::new(Some(send)),
            recv: Mutex::new(Some(recv)),
        })
    }

    async fn send_frame(&self, kind: u8, message: &[u8]) -> io::Result<()> {
        if message.len() > MAX_CONTROL_MESSAGE {
            return Err(io::Error::other("admission control message exceeds 64 KiB"));
        }
        let mut guard = self.send.lock().await;
        let send = guard
            .as_mut()
            .ok_or_else(|| io::Error::other("control channel retired"))?;
        let mut header = [0; 5];
        header[0] = kind;
        header[1..].copy_from_slice(&(message.len() as u32).to_be_bytes());
        send.write_all(&header).await.map_err(io::Error::other)?;
        send.write_all(message).await.map_err(io::Error::other)
    }

    async fn recv_frame(&self) -> io::Result<(u8, Vec<u8>)> {
        let mut guard = self.recv.lock().await;
        let recv = guard
            .as_mut()
            .ok_or_else(|| io::Error::other("control channel retired"))?;
        let mut header = [0; 5];
        recv.read_exact(&mut header)
            .await
            .map_err(io::Error::other)?;
        let length =
            u32::from_be_bytes(header[1..].try_into().expect("fixed frame header")) as usize;
        if length > MAX_CONTROL_MESSAGE || !matches!(header[0], CONTROL_MESSAGE | DELIVERY_READY) {
            return Err(io::Error::other("invalid admission control frame"));
        }
        let mut message = vec![0; length];
        recv.read_exact(&mut message)
            .await
            .map_err(io::Error::other)?;
        Ok((header[0], message))
    }

    async fn ready(&self, scope: u8) -> io::Result<()> {
        self.send_frame(DELIVERY_READY, &[scope]).await?;
        let (kind, message) = self.recv_frame().await?;
        if kind != DELIVERY_READY || message != [scope] {
            return Err(io::Error::other(
                "peer has not accepted the same delivery scope",
            ));
        }
        Ok(())
    }

    async fn retire(&self) -> io::Result<()> {
        if let Some(mut send) = self.send.lock().await.take() {
            send.finish().map_err(io::Error::other)?;
        }
        if let Some(mut recv) = self.recv.lock().await.take() {
            recv.stop(0u32.into()).map_err(io::Error::other)?;
        }
        Ok(())
    }
}

pub(super) async fn finish_admission(
    connection: Connection,
    endpoint: &Endpoint,
    policy: &dyn DynEndpointAdmission,
) -> Result<RegisteredConnection, ConnectingError> {
    let result = async {
        let mut control = ControlConnection {
            channel: ControlChannel::new(&connection).await?,
            connection: connection.clone(),
            endpoint: endpoint.clone(),
            close_on_drop: true,
        };
        let permit = match policy.admit(&control).await? {
            AdmissionOutcome::Accept(permit) => permit,
            AdmissionOutcome::ControlOnly => {
                if !policy.is_current(&control) {
                    return Err(io::Error::other("control admission is no longer current"));
                }
                control.channel.ready(1).await?;
                if !policy.is_current(&control) || connection.close_reason().is_some() {
                    return Err(io::Error::other(
                        "control admission changed before delivery",
                    ));
                }
                return Ok(RegisteredConnection::Control(control));
            }
            AdmissionOutcome::Pending => {
                return Err(io::Error::other("admission decision missing"));
            }
            AdmissionOutcome::Reject { error_code, reason } => {
                connection.close(error_code, &reason);
                return Err(io::Error::other("admission rejected"));
            }
        };
        permit.check(&control)?;
        if !policy.is_current(&control) {
            return Err(io::Error::other("admission decision is no longer current"));
        }
        // This is deliberately after the new policy and path checks: the old
        // hook receives a full Connection and may use its business methods.
        match endpoint.inner.hooks.after_handshake(&connection).await {
            AfterHandshakeOutcome::Accept => {}
            AfterHandshakeOutcome::Reject { error_code, reason } => {
                connection.close(error_code, &reason);
                return Err(io::Error::other("legacy handshake hook rejected"));
            }
            _ => {
                return Err(io::Error::other(
                    "legacy hook returned a transitional admission scope",
                ));
            }
        }
        permit.check(&control)?;
        if !policy.is_current(&control) {
            return Err(io::Error::other(
                "admission changed during legacy handshake checks",
            ));
        }
        control.channel.ready(0).await?;
        permit.check(&control)?;
        if !policy.is_current(&control) {
            return Err(io::Error::other("admission changed before delivery"));
        }
        endpoint
            .inner
            .authorize_candidates(
                connection.remote_id(),
                connection.stable_id(),
                permit.allowed_data_custom_ids.clone(),
            )
            .await?;
        control.channel.retire().await?;
        permit.check(&control)?;
        if !policy.is_current(&control) {
            return Err(io::Error::other("admission changed during delivery"));
        }
        control.close_on_drop = false;
        Ok(RegisteredConnection::Business(connection.clone()))
    }
    .await;
    match result {
        Ok(connection) => Ok(connection),
        Err(error) => {
            tracing::debug!(%error, "restricted endpoint admission failed");
            connection.close(0u32.into(), b"restricted endpoint admission failed");
            Err(e!(ConnectingError::LocallyRejected))
        }
    }
}

pub(super) trait AttemptError {
    fn cancelled() -> Self;
    fn timed_out() -> Self;
}

macro_rules! attempt_error {
    ($ty:ident) => {
        impl AttemptError for $ty {
            fn cancelled() -> Self {
                e!($ty::AttemptCancelled)
            }
            fn timed_out() -> Self {
                e!($ty::AttemptTimedOut)
            }
        }
    };
}
attempt_error!(ConnectingError);
attempt_error!(ConnectWithOptsError);
attempt_error!(AlpnError);

pub(super) async fn bounded<T, E: AttemptError>(
    attempt: &AttemptOptions,
    work: impl Future<Output = Result<T, E>>,
) -> Result<T, E> {
    let cancelled = async {
        match &attempt.cancellation {
            Some(token) => token.cancelled().await,
            None => std::future::pending().await,
        }
    };
    let expired = async {
        match attempt.caller_deadline {
            Some(deadline) => time::sleep_until(deadline).await,
            None => std::future::pending().await,
        }
    };
    tokio::select! { biased;
        _ = cancelled => Err(E::cancelled()),
        _ = expired => Err(E::timed_out()),
        result = work => result,
    }
}
