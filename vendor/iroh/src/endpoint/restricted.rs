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

mod application;
use application::ApplicationControl;
pub use application::{ControlExchange, ControlMessageFormat};

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
    /// Control-only admission using the existing application's own confirmation.
    /// Used with [`Builder::restricted_application`].
    ControlOnlyWithConfirmation(ControlConfirmation),
    /// Admit business over a path freshly qualified on this same connection.
    Accept(DataPathPermit),
    /// Business admission with a fresh path and an owned application confirmation.
    /// The confirmation runs only after the path checks and ordinary hooks.
    AcceptWithConfirmation {
        /// Fresh proof for this original connection's allowed data path.
        permit: DataPathPermit,
        /// The existing application's final bilateral delivery agreement.
        confirmation: ControlConfirmation,
    },
    /// Reject the connection with an application close reason.
    Reject {
        /// Application close code sent to the peer.
        error_code: VarInt,
        /// Application close reason sent to the peer.
        reason: Vec<u8>,
    },
}

/// One attempt's owned final control-protocol confirmation.
///
/// This future owns its exchange and prepared protocol resources. Admission,
/// current authorization and the data path are checked before it is polled;
/// business scope also runs ordinary hooks first. A rejected, cancelled, timed
/// out or dropped attempt drops the future even if confirmation never starts.
/// It cannot be cloned or polled through a public method.
pub struct ControlConfirmation {
    operation: ConfirmationOperation,
}

enum ConfirmationOperation {
    Owned(BoxFuture<'static, io::Result<()>>),
    WithControl(
        Box<dyn for<'a> FnOnce(&'a ControlConnection) -> BoxFuture<'a, io::Result<()>> + Send>,
    ),
}

impl std::fmt::Debug for ControlConfirmation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlConfirmation")
            .finish_non_exhaustive()
    }
}

impl ControlConfirmation {
    /// Own the existing application's final bilateral confirmation operation.
    pub fn new(future: impl Future<Output = io::Result<()>> + Send + 'static) -> Self {
        Self {
            operation: ConfirmationOperation::Owned(Box::pin(future)),
        }
    }

    /// Own prepared resources while checking this live control session at confirmation.
    /// The callback receives only the constrained capability, never a Connection.
    pub fn with_control<F>(confirmation: F) -> Self
    where
        F: for<'a> FnOnce(
                &'a ControlConnection,
            ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>>
            + Send
            + 'static,
    {
        Self {
            operation: ConfirmationOperation::WithControl(Box::new(confirmation)),
        }
    }

    async fn run(self, control: &ControlConnection) -> io::Result<()> {
        match self.operation {
            ConfirmationOperation::Owned(future) => future.await,
            ConfirmationOperation::WithControl(confirmation) => confirmation(control).await,
        }
    }
}

/// Application admission policy for [`RestrictedEndpoint`].
///
/// The library authenticates the remote TLS identity before invoking this
/// interface. The application supplies its authorization and coordination
/// protocol through bounded control messages. An accepted data path does not
/// itself grant application authorization: return [`AdmissionOutcome::Accept`]
/// only after the application's checks have succeeded.
///
/// Policies must not retain the endpoint they are installed on, which would
/// create an ownership cycle. The supplied owner cannot be cloned or converted
/// to a connection. Its cloneable [`ControlHandle`] remains confined and its
/// exchanges are retired when this owner is dropped or business is delivered.
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

    /// Observe the final ordinary handoff, after control channels are retired
    /// and all authorization/path checks succeed. Intended for weak registries.
    fn business_released(&self, _connection: &Connection) {}
}

pub(crate) trait DynEndpointAdmission: std::fmt::Debug + Send + Sync {
    fn admit<'a>(
        &'a self,
        connection: &'a ControlConnection,
    ) -> BoxFuture<'a, io::Result<AdmissionOutcome>>;
    fn is_current(&self, connection: &ControlConnection) -> bool;
    fn business_released(&self, connection: &Connection);
    fn application_protocol(&self) -> bool {
        false
    }
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
    fn business_released(&self, connection: &Connection) {
        EndpointAdmission::business_released(self, connection)
    }
}

#[derive(Debug)]
struct ApplicationAdmission<T>(T);

impl<T: EndpointAdmission> DynEndpointAdmission for ApplicationAdmission<T> {
    fn admit<'a>(
        &'a self,
        connection: &'a ControlConnection,
    ) -> BoxFuture<'a, io::Result<AdmissionOutcome>> {
        Box::pin(self.0.admit(connection))
    }

    fn is_current(&self, connection: &ControlConnection) -> bool {
        self.0.is_current(connection)
    }

    fn application_protocol(&self) -> bool {
        true
    }
    fn business_released(&self, connection: &Connection) {
        self.0.business_released(connection)
    }
}

pub(super) fn application_policy(
    policy: impl EndpointAdmission + 'static,
) -> Arc<dyn DynEndpointAdmission> {
    Arc::new(ApplicationAdmission(policy))
}

#[derive(Debug)]
struct CustomDataLimit {
    inner: Arc<dyn DynEndpointAdmission>,
    allowed: Vec<u64>,
}
impl DynEndpointAdmission for CustomDataLimit {
    fn admit<'a>(
        &'a self,
        control: &'a ControlConnection,
    ) -> BoxFuture<'a, io::Result<AdmissionOutcome>> {
        Box::pin(async move {
            let mut outcome = self.inner.admit(control).await?;
            match &mut outcome {
                AdmissionOutcome::Accept(permit)
                | AdmissionOutcome::AcceptWithConfirmation { permit, .. } => {
                    permit
                        .allowed_data_custom_ids
                        .retain(|id| self.allowed.contains(id));
                }
                _ => {}
            }
            Ok(outcome)
        })
    }
    fn is_current(&self, control: &ControlConnection) -> bool {
        self.inner.is_current(control)
    }
    fn application_protocol(&self) -> bool {
        self.inner.application_protocol()
    }
    fn business_released(&self, connection: &Connection) {
        self.inner.business_released(connection)
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
    pub(super) fn new(mut inner: Builder) -> Self {
        if let Some(allowed) = inner.restricted_data_custom_transports.take() {
            let policy = inner
                .admission
                .take()
                .expect("restricted admission selected");
            inner.admission = Some(Arc::new(CustomDataLimit {
                inner: policy,
                allowed,
            }));
        }
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
    pub async fn bind(self) -> Result<RestrictedEndpoint, BindError> {
        if self.inner.admission.is_none() || self.admission_timeout.is_zero() {
            return Err(e!(BindError::InvalidAdmissionConfig));
        }
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
    /// Wait for this endpoint's existing contact registration to become online.
    pub async fn online(&self) {
        self.inner.online().await
    }
    /// Read the endpoint's actual transport metrics.
    #[cfg(feature = "metrics")]
    pub fn metrics(&self) -> &crate::metrics::EndpointMetrics {
        self.inner.metrics()
    }
    /// Update published address metadata without exposing an ordinary endpoint.
    pub fn set_user_data_for_address_lookup(&self, data: Option<crate::address_lookup::UserData>) {
        self.inner.set_user_data_for_address_lookup(data)
    }
    /// Build the standard pkarr resolver with this endpoint's DNS and TLS settings.
    pub fn pkarr_resolver(
        &self,
        builder: crate::address_lookup::PkarrResolverBuilder,
    ) -> Result<
        impl crate::address_lookup::AddressLookup,
        crate::address_lookup::AddressLookupBuilderError,
    > {
        use crate::address_lookup::AddressLookupBuilder;
        builder.into_address_lookup(&self.inner)
    }
    /// Build the standard pkarr publisher with this endpoint's identity and TLS settings.
    pub fn pkarr_publisher(
        &self,
        builder: crate::address_lookup::PkarrPublisherBuilder,
    ) -> Result<
        impl crate::address_lookup::AddressLookup,
        crate::address_lookup::AddressLookupBuilderError,
    > {
        use crate::address_lookup::AddressLookupBuilder;
        builder.into_address_lookup(&self.inner)
    }

    /// Watch actual UDP address observations without granting connection access.
    #[cfg(not(wasm_browser))]
    pub fn udp_observation(
        &self,
    ) -> impl n0_watcher::Watcher<Value = super::UdpObservation> + use<> {
        self.inner.udp_observation()
    }

    /// Observe mappings using this endpoint's real sockets and configured resources.
    #[cfg(not(wasm_browser))]
    pub async fn observe_udp_mapping_sequence(
        &self,
        observers: &crate::RelayMap,
    ) -> io::Result<super::UdpMappingSequence> {
        self.inner.observe_udp_mapping_sequence(observers).await
    }
    /// Observation publication epoch, distinct from the physical network epoch.
    #[cfg(not(wasm_browser))]
    pub fn udp_observation_generation(&self) -> super::ObservationValidity {
        self.inner.udp_observation_generation()
    }

    /// Exercise the existing traversal TTL failure on this endpoint's sockets.
    #[cfg(all(not(wasm_browser), feature = "miopunch-test-hooks"))]
    #[doc(hidden)]
    pub async fn test_force_traversal_ttl_unavailable(&self) {
        self.inner.test_force_traversal_ttl_unavailable().await
    }

    pub(crate) fn as_endpoint(&self) -> &Endpoint {
        &self.inner
    }

    pub(crate) fn admission_timeout(&self) -> Duration {
        self.admission_timeout
    }

    /// Configure the default setup budget for this endpoint view and its clones.
    ///
    /// Apply this before accepting connections or building a router. Incoming
    /// budgets start at receipt; earlier caller deadlines still win. Existing
    /// attempts and successfully delivered connections keep their own lifetimes.
    pub fn with_admission_timeout(mut self, timeout: Duration) -> io::Result<Self> {
        if timeout.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "admission timeout must be positive",
            ));
        }
        self.admission_timeout = timeout;
        Ok(self)
    }

    /// The endpoint's cryptographic identity.
    pub fn id(&self) -> EndpointId {
        self.inner.id()
    }

    /// The current contact addresses of this endpoint.
    pub fn addr(&self) -> EndpointAddr {
        self.inner.addr()
    }

    /// Actual bound socket addresses, without exposing socket ownership.
    #[cfg(not(wasm_browser))]
    pub fn bound_sockets(&self) -> Vec<std::net::SocketAddr> {
        self.inner.bound_sockets()
    }

    /// Notify the shared endpoint of a network change under its existing policy.
    pub async fn network_change(&self) {
        self.inner.network_change().await
    }

    /// Observe an authorized UDP STUN facility using the endpoint's actual socket.
    #[cfg(not(wasm_browser))]
    pub async fn observe_udp_stun(
        &self,
        observer: &crate::RelayUrl,
    ) -> io::Result<super::UdpStunObservation> {
        self.inner.observe_udp_stun(observer).await
    }

    /// Locally installed custom contact addresses; no connection is released.
    pub fn custom_addrs(&self) -> Vec<iroh_base::CustomAddr> {
        self.inner.custom_addrs()
    }

    /// Current remote transport observations, without granting admission.
    pub async fn remote_info(&self, id: EndpointId) -> Option<super::RemoteInfo> {
        self.inner.remote_info(id).await
    }

    /// The endpoint's current relay registration status.
    pub fn home_relay_status(
        &self,
    ) -> impl n0_watcher::Watcher<Value = Vec<super::RelayStatus>> + use<> {
        self.inner.home_relay_status()
    }

    /// Configured address lookup services. Results remain untrusted contacts.
    pub fn address_lookup(
        &self,
    ) -> Result<&crate::address_lookup::AddressLookupServices, super::EndpointError> {
        self.inner.address_lookup()
    }

    /// Narrow a connection already released to this caller for path maintenance.
    /// This never turns a control-only connection into ordinary business.
    pub fn control_handle(&self, connection: &Connection) -> ControlHandle {
        self.inner.control_handle(connection)
    }

    /// Test-only request for an actual custom path on an already released connection.
    #[cfg(feature = "miopunch-test-hooks")]
    #[doc(hidden)]
    pub async fn test_open_admitted_custom_path(
        &self,
        connection: &Connection,
        remote: iroh_base::CustomAddr,
    ) -> io::Result<()> {
        self.inner
            .test_open_admitted_custom_path(connection, remote)
            .await
    }

    /// Open a candidate on an ordinary connection already owned by the caller.
    #[cfg(not(wasm_browser))]
    pub async fn open_data_path(
        &self,
        connection: &Connection,
        remote: std::net::SocketAddr,
    ) -> io::Result<()> {
        self.inner.open_data_path(connection, remote).await
    }

    /// Probe a permitted carrier on an already released connection.
    pub async fn open_data_transport_path(
        &self,
        connection: &Connection,
        remote: TransportAddr,
    ) -> io::Result<()> {
        self.inner
            .open_data_transport_path(connection, remote)
            .await
    }

    /// Apply the existing packet guard to an already released connection.
    pub async fn commit_data_path(&self, connection: &Connection) -> io::Result<()> {
        self.inner.commit_data_path(connection).await
    }

    /// Network epoch for observations on this endpoint's physical UDP sockets.
    #[cfg(not(wasm_browser))]
    pub fn udp_network_generation(&self) -> super::ObservationValidity {
        self.inner.udp_network_generation()
    }

    /// DNS used by this endpoint's configured observation services.
    #[cfg(not(wasm_browser))]
    pub fn dns_resolver(&self) -> Result<&crate::dns::DnsResolver, super::EndpointError> {
        self.inner.dns_resolver()
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
    /// Best-effort Initial ALPN metadata; fragmented ClientHello may be incomplete.
    /// Full classification still requires `RestrictedAccepting::alpn`.
    pub fn initial_alpns(&self) -> Option<Vec<Vec<u8>>> {
        self.inner
            .decrypt()
            .and_then(|initial| initial.alpns())
            .and_then(|alpns| {
                alpns
                    .map(|alpn| alpn.ok().map(|alpn| alpn.to_vec()))
                    .collect()
            })
    }

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
    control: ControlHandle,
    close_on_drop: bool,
}

/// Retain restricted control work without obtaining business I/O or an endpoint.
///
/// Clones share the original control channel and session. Dropping the owning
/// [`ControlConnection`] still closes a pending or control-only session; helpers
/// cannot extend its authorization. Business handoff retires all control I/O.
#[derive(Debug, Clone)]
pub struct ControlHandle {
    connection: Connection,
    endpoint: Endpoint,
    channel: Arc<ControlChannel>,
}

impl std::ops::Deref for ControlConnection {
    type Target = ControlHandle;
    fn deref(&self) -> &Self::Target {
        &self.control
    }
}

impl ControlConnection {
    /// Retain a narrowed helper for owned control or candidate preparation.
    /// It cannot release business, clone this owner, or expose a full endpoint.
    pub fn control_handle(&self) -> ControlHandle {
        self.control.clone()
    }
}

impl Drop for ControlConnection {
    fn drop(&mut self) {
        if self.close_on_drop {
            self.channel.cancel();
            self.connection
                .close(0u32.into(), b"restricted control owner dropped");
        }
    }
}

impl ControlHandle {
    /// Exercise transport-path loss on this original connection only.
    #[cfg(feature = "miopunch-test-hooks")]
    #[doc(hidden)]
    pub async fn test_limit_transport_paths(&self, allowed: Vec<TransportAddr>) -> io::Result<()> {
        self.endpoint
            .test_limit_transport_paths(&self.connection, allowed)
            .await
    }

    /// Read the original connection's packet admission counters.
    pub fn path_filter_stats(&self) -> noq::PathFilterStats {
        self.connection.path_filter_stats()
    }
    /// Inject an old custom-path packet through the existing ingress-gate fixture.
    #[cfg(feature = "miopunch-test-hooks")]
    #[doc(hidden)]
    pub async fn test_reopen_control_path(&self, remote: iroh_base::CustomAddr) -> io::Result<()> {
        self.endpoint
            .test_reopen_control_path(&self.connection, remote)
            .await
    }

    pub(super) fn narrow(endpoint: &Endpoint, connection: &Connection) -> Self {
        Self {
            endpoint: endpoint.clone(),
            connection: connection.clone(),
            channel: Arc::new(ControlChannel::Application(ApplicationControl::new())),
        }
    }

    /// Local endpoint addresses only; this does not return an endpoint capability.
    pub fn addr(&self) -> EndpointAddr {
        self.endpoint.addr()
    }
    /// Local endpoint identity.
    pub fn id(&self) -> EndpointId {
        self.endpoint.id()
    }
    /// Local custom carrier candidates, without an endpoint capability.
    #[cfg(feature = "unstable-custom-transports")]
    pub fn custom_addrs(&self) -> Vec<iroh_base::CustomAddr> {
        self.endpoint.custom_addrs()
    }
    /// Observe the original connection's traffic and frame counters.
    pub fn stats(&self) -> super::ConnectionStats {
        self.connection.stats()
    }
    /// Whether the retained endpoint has closed.
    pub fn is_closed(&self) -> bool {
        self.endpoint.is_closed()
    }
    /// Observe connection path changes without business access.
    pub fn path_events(&self) -> super::PathEventStream {
        self.connection.path_events()
    }
    /// The endpoint's current relay status, with no connection entry points.
    pub fn home_relay_status(
        &self,
    ) -> impl n0_watcher::Watcher<Value = Vec<super::RelayStatus>> + use<> {
        self.endpoint.home_relay_status()
    }
    /// Start validating one custom candidate under the original connection guards.
    pub async fn open_control_path(&self, remote: iroh_base::CustomAddr) -> io::Result<()> {
        self.endpoint
            .open_control_path(&self.connection, remote)
            .await
    }
    /// Validate a permitted carrier on this same already admitted original.
    /// The existing admission and resource checks still govern path creation.
    pub async fn open_data_transport_path(&self, remote: TransportAddr) -> io::Result<()> {
        self.endpoint
            .open_data_transport_path(&self.connection, remote)
            .await
    }
    /// Retain an already validated custom control path under its existing seal.
    pub async fn retain_control_data_transport_path(
        &self,
        remote: iroh_base::CustomAddr,
    ) -> io::Result<()> {
        self.endpoint
            .retain_control_data_transport_path(&self.connection, remote)
            .await
    }
    /// Retain a validated custom path on this already admitted original.
    pub async fn retain_data_transport_path(
        &self,
        remote: iroh_base::CustomAddr,
    ) -> io::Result<()> {
        self.endpoint
            .retain_data_transport_path(&self.connection, remote)
            .await
    }
    /// Start validating a permitted relay on this same original connection.
    pub async fn open_control_relay_path(&self, remote: crate::RelayUrl) -> io::Result<()> {
        self.endpoint
            .open_control_relay_path(&self.connection, remote)
            .await
    }
    /// Retire only this original connection's failed custom candidate.
    pub async fn retire_control_path(&self, remote: iroh_base::CustomAddr) -> io::Result<()> {
        self.endpoint
            .retire_control_path(&self.connection, remote)
            .await
    }
    /// Install the existing data-path packet guard without releasing business I/O.
    pub async fn commit_data_path(&self) -> io::Result<()> {
        self.endpoint.commit_data_path(&self.connection).await
    }
    /// Watch observations made using the endpoint's actual UDP socket.
    pub fn udp_observation(
        &self,
    ) -> impl n0_watcher::Watcher<Value = super::UdpObservation> + use<> {
        self.endpoint.udp_observation()
    }
    /// Resolve observation facilities using this endpoint's configured resolver.
    #[cfg(not(wasm_browser))]
    pub fn dns_resolver(&self) -> Result<&crate::dns::DnsResolver, super::EndpointError> {
        self.endpoint.dns_resolver()
    }
    /// Capture the current physical socket/network generation.
    #[cfg(not(wasm_browser))]
    pub fn udp_network_generation(&self) -> super::ObservationValidity {
        self.endpoint.udp_network_generation()
    }
    /// Actual primary local IPv4 candidates; reflected addresses are not included.
    #[cfg(not(wasm_browser))]
    pub fn udp_local_ipv4_candidates(&self) -> Vec<std::net::SocketAddr> {
        self.endpoint.udp_local_ipv4_candidates()
    }
    /// Use the same actual UDP socket and resource policy for a QAD sequence.
    #[cfg(not(wasm_browser))]
    pub async fn observe_udp_mapping_sequence(
        &self,
        observers: &crate::RelayMap,
    ) -> io::Result<super::UdpMappingSequence> {
        self.endpoint.observe_udp_mapping_sequence(observers).await
    }
    /// Use the same actual UDP socket and resource policy for a STUN sequence.
    #[cfg(not(wasm_browser))]
    pub async fn observe_udp_stun_mapping_sequence(
        &self,
        observers: &[crate::RelayUrl],
    ) -> io::Result<super::UdpMappingSequence> {
        self.endpoint
            .observe_udp_stun_mapping_sequence(observers)
            .await
    }
    /// Validate through the exact retained physical binding of this connection.
    #[cfg(not(wasm_browser))]
    pub async fn open_data_path_from(
        &self,
        local: super::LocalSocketBinding,
        remote: std::net::SocketAddr,
    ) -> io::Result<()> {
        self.endpoint
            .open_data_path_from(&self.connection, local, remote)
            .await
    }
    /// Send a bounded mapping probe through the original connection's actual UDP socket.
    #[cfg(not(wasm_browser))]
    pub async fn prime_udp_path(&self, remote: std::net::SocketAddr) -> io::Result<()> {
        self.endpoint.prime_udp_path(&self.connection, remote).await
    }
    /// Register a bounded traversal round under the original connection guards.
    #[cfg(not(wasm_browser))]
    pub async fn register_udp_traversal(
        &self,
        round: [u8; 16],
    ) -> io::Result<super::UdpTraversalLease> {
        self.endpoint
            .register_udp_traversal(&self.connection, round)
            .await
    }
    /// Retain the exact validated, selected physical winner without business access.
    #[cfg(not(wasm_browser))]
    pub fn retain_udp_winner(&self, lease: &mut super::UdpSocketPathLease) -> io::Result<()> {
        lease.commit(&self.connection)
    }
    /// Exercise the existing bounded path-ID retry fixture on this session.
    #[cfg(all(not(wasm_browser), feature = "miopunch-test-hooks"))]
    #[doc(hidden)]
    pub async fn test_force_path_id_retry_once(&self) -> io::Result<()> {
        self.endpoint
            .test_force_path_id_retry_once(&self.connection)
            .await
    }

    /// Open a bounded exchange in the explicitly selected application protocol.
    ///
    /// Message methods preserve the chosen framing without exposing a stream,
    /// datagram or ordinary connection. They reject after business handoff.
    /// Available only with [`Builder::restricted_application`].
    pub async fn open_control_exchange(&self) -> io::Result<ControlExchange> {
        self.channel.application()?.open(&self.connection).await
    }

    /// Accept the next bounded application control exchange.
    pub async fn accept_control_exchange(&self) -> io::Result<ControlExchange> {
        self.channel.application()?.accept(&self.connection).await
    }

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

    /// Observe the original session's close reason without a business handle.
    pub fn close_reason(&self) -> Option<super::ConnectionError> {
        self.connection.close_reason()
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

    /// Test-only observation of candidates on this original pending connection.
    #[cfg(feature = "miopunch-test-hooks")]
    #[doc(hidden)]
    pub fn test_local_qnt_candidate_count(&self) -> usize {
        self.connection.test_local_qnt_candidate_count()
    }

    /// Observe changes to the original session's paths without a business handle.
    pub fn paths_stream(&self) -> super::PathListStream<'_> {
        self.connection.paths_stream()
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
        if !self
            .endpoint
            .inner
            .qualifies_admission_data_path(
                self.remote_id(),
                self.stable_id(),
                &remote,
                &allowed_data_custom_ids,
            )
            .await
        {
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
                permit.check(self).await?;
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

impl From<super::WeakConnectionHandle> for WeakControlConnection {
    /// Narrow an existing ordinary weak handle to observation and revocation.
    /// This conversion provides no way to upgrade the resulting capability.
    fn from(inner: super::WeakConnectionHandle) -> Self {
        Self { inner }
    }
}

impl WeakControlConnection {
    /// Whether the original session is still open.
    pub fn is_open(&self) -> bool {
        self.inner.is_open()
    }

    /// Observe whether one exact path is currently selected without retaining
    /// or exposing the original connection.
    pub fn path_is_selected(&self, path_id: &str, remote: &TransportAddr) -> bool {
        self.inner.upgrade().is_some_and(|connection| {
            connection.paths().iter().any(|path| {
                path.id().to_string() == path_id
                    && path.is_selected()
                    && path.remote_addr() == remote
            })
        })
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
    async fn check(&self, control: &ControlHandle) -> io::Result<()> {
        if self.connection_id != control.stable_id()
            || control.connection.close_reason().is_some()
            || !control
                .connection
                .paths()
                .iter()
                .any(|path| path.id() == self.path_id && path.remote_addr() == &self.remote)
            || !control
                .endpoint
                .inner
                .qualifies_admission_data_path(
                    control.remote_id(),
                    self.connection_id,
                    &self.remote,
                    &self.allowed_data_custom_ids,
                )
                .await
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
enum ControlChannel {
    Framed(FramedControl),
    Application(ApplicationControl),
}

impl ControlChannel {
    async fn new(connection: &Connection, application: bool) -> io::Result<Self> {
        if application {
            Ok(Self::Application(ApplicationControl::new()))
        } else {
            Ok(Self::Framed(FramedControl::new(connection).await?))
        }
    }

    fn application(&self) -> io::Result<&ApplicationControl> {
        match self {
            Self::Application(control) => Ok(control),
            Self::Framed(_) => Err(io::Error::other(
                "application control protocol was not selected",
            )),
        }
    }

    async fn send_frame(&self, kind: u8, message: &[u8]) -> io::Result<()> {
        match self {
            Self::Framed(channel) => channel.send_frame(kind, message).await,
            Self::Application(_) => Err(io::Error::other(
                "application protocol requires an explicit message format",
            )),
        }
    }

    async fn recv_frame(&self) -> io::Result<(u8, Vec<u8>)> {
        match self {
            Self::Framed(channel) => channel.recv_frame().await,
            Self::Application(_) => Err(io::Error::other(
                "application protocol requires an explicit message format",
            )),
        }
    }

    async fn ready(
        &self,
        control: &ControlConnection,
        confirmation: Option<ControlConfirmation>,
        scope: u8,
    ) -> io::Result<()> {
        match (self, confirmation) {
            (Self::Framed(channel), None) => channel.ready(scope).await,
            (Self::Application(_), Some(confirmation)) => confirmation.run(control).await,
            _ => Err(io::Error::other(
                "control confirmation does not match the selected protocol",
            )),
        }
    }

    async fn retire(&self) -> io::Result<()> {
        match self {
            Self::Framed(channel) => channel.retire().await,
            Self::Application(control) => {
                control.cancel();
                Ok(())
            }
        }
    }

    fn cancel(&self) {
        if let Self::Application(control) = self {
            control.cancel();
        }
    }
}

#[derive(Debug)]
struct FramedControl {
    send: Mutex<Option<SendStream>>,
    recv: Mutex<Option<RecvStream>>,
}

impl FramedControl {
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
            control: ControlHandle {
                channel: Arc::new(
                    ControlChannel::new(&connection, policy.application_protocol()).await?,
                ),
                connection: connection.clone(),
                endpoint: endpoint.clone(),
            },
            close_on_drop: true,
        };
        let (permit, confirmation) = match policy.admit(&control).await? {
            AdmissionOutcome::Accept(permit) => (Some(permit), None),
            AdmissionOutcome::AcceptWithConfirmation {
                permit,
                confirmation,
            } => (Some(permit), Some(confirmation)),
            AdmissionOutcome::ControlOnly => (None, None),
            AdmissionOutcome::ControlOnlyWithConfirmation(confirmation) => {
                (None, Some(confirmation))
            }
            AdmissionOutcome::Pending => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "admission decision missing",
                ));
            }
            AdmissionOutcome::Reject { error_code, reason } => {
                connection.close(error_code, &reason);
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "admission rejected",
                ));
            }
        };
        if policy.application_protocol() != confirmation.is_some() {
            return Err(io::Error::other(
                "application delivery agreement missing or unexpected",
            ));
        }
        let Some(permit) = permit else {
            if !policy.is_current(&control) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "control admission is no longer current",
                ));
            }
            control.channel.ready(&control, confirmation, 1).await?;
            if !policy.is_current(&control) || connection.close_reason().is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "control admission changed before delivery",
                ));
            }
            return Ok(RegisteredConnection::Control(control));
        };
        permit.check(&control).await?;
        if !policy.is_current(&control) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "admission decision is no longer current",
            ));
        }
        // This is deliberately after the new policy and path checks: the old
        // hook receives a full Connection and may use its business methods.
        match endpoint.inner.hooks.after_handshake(&connection).await {
            AfterHandshakeOutcome::Accept => {}
            AfterHandshakeOutcome::Reject { error_code, reason } => {
                connection.close(error_code, &reason);
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "legacy handshake hook rejected",
                ));
            }
        }
        permit.check(&control).await?;
        if !policy.is_current(&control) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "admission changed during legacy handshake checks",
            ));
        }
        control.channel.ready(&control, confirmation, 0).await?;
        permit.check(&control).await?;
        if !policy.is_current(&control) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "admission changed before delivery",
            ));
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
        permit.check(&control).await?;
        if !policy.is_current(&control) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "admission changed during delivery",
            ));
        }
        policy.business_released(&connection);
        control.close_on_drop = false;
        Ok(RegisteredConnection::Business(connection.clone()))
    }
    .await;
    match result {
        Ok(connection) => Ok(connection),
        Err(error) => {
            tracing::debug!(%error, "restricted endpoint admission failed");
            connection.close(0u32.into(), b"restricted endpoint admission failed");
            Err(e!(ConnectingError::AdmissionFailed {
                reason: error.to_string(),
                kind: error.kind()
            }))
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
