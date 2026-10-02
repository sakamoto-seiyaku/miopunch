use std::{collections::BTreeSet, pin::Pin};

use iroh_base::EndpointAddr;

use crate::{
    Endpoint,
    endpoint::{connection::Connection, quic::VarInt},
};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Outcome of [`EndpointHooks::before_connect`]
#[derive(Debug)]
pub enum BeforeConnectOutcome {
    /// Accept the connect attempt.
    Accept,
    /// Reject the connect attempt.
    Reject,
}

/// Outcome of [`EndpointHooks::after_handshake`]
#[derive(Debug)]
pub enum AfterHandshakeOutcome {
    /// Accept the connection.
    Accept,
    /// Accept normal protocol use while allowing only the listed custom
    /// carrier IDs for member data. IP and relay paths remain eligible.
    AcceptData {
        /// Custom transport IDs eligible for member data.
        allowed_data_custom_ids: Vec<u64>,
    },
    /// Accept only the restricted protocol; keep NAT candidates undisclosed.
    AcceptRestricted,
    /// Reject and close the connection.
    ///
    /// See [`Connection::close`] for details on `error_code` and `reason`.
    ///
    /// [`Connection::close`]: crate::endpoint::Connection::close
    Reject {
        /// Error code to send with the connection close frame.
        error_code: VarInt,
        /// Close reason to send with the connection close frame.
        reason: Vec<u8>,
    },
}

impl AfterHandshakeOutcome {
    /// Returns [`Self::Accept`].
    pub fn accept() -> Self {
        Self::Accept
    }

    /// Accept a restricted bootstrap connection without enabling QNT.
    pub fn accept_restricted() -> Self {
        Self::AcceptRestricted
    }

    /// Returns [`Self::Reject`].
    pub fn reject(&self, error_code: VarInt, reason: &[u8]) -> Self {
        Self::Reject {
            error_code,
            reason: reason.to_vec(),
        }
    }
}

/// EndpointHooks intercept the connection establishment process of an [`Endpoint`].
///
/// Use [`Builder::hooks`] to install hooks onto an endpoint.
///
/// For each hook, all installed hooks are invoked in the order they were installed on
/// the endpoint builder. If a hook returns `Accept`, processing continues with the next
/// hook. If a hook returns `Reject`, processing is aborted and further hooks
/// are not invoked for this hook point.
///
/// ## Notes to implementers
///
/// As hooks are stored on the endpoint, you must make sure to never store an [`Endpoint`]
/// on the hook struct itself, as this would create reference counting loop and cause the
/// endpoint to never be dropped, leaking memory.
///
/// [`Endpoint`]: crate::Endpoint
/// [`Builder::hooks`]: crate::endpoint::Builder::hooks
pub trait EndpointHooks: std::fmt::Debug + Send + Sync {
    /// Intercept outgoing connections before they are started.
    ///
    /// This is called whenever a new outgoing connection is initiated via [`Endpoint::connect`]
    /// or [`Endpoint::connect_with_opts`].
    ///
    /// If any hook returns [`BeforeConnectOutcome::Reject`], the connection attempt is aborted
    /// before any packets are sent to the remote.
    ///
    /// [`Endpoint::connect`]: crate::Endpoint::connect
    /// [`Endpoint::connect_with_opts`]: crate::Endpoint::connect_with_opts
    fn before_connect<'a>(
        &'a self,
        _remote_addr: &'a EndpointAddr,
        _alpn: &'a [u8],
    ) -> impl Future<Output = BeforeConnectOutcome> + Send + 'a {
        async { BeforeConnectOutcome::Accept }
    }

    /// Intercept both incoming and outgoing connections once the TLS handshake has completed.
    ///
    /// At this point in time, we know the remote's endpoint id and ALPN. If any hook returns
    /// [`AfterHandshakeOutcome::Reject`], the connection is closed with the provided error code
    /// and reason.
    ///
    /// The hook receives the [`Connection`] by reference so that implementations can read any
    /// information they need synchronously. Do not clone the [`Connection`] out of the hook:
    /// holding a strong [`Connection`] handle keeps the connection alive and disables
    /// close-on-drop, which the primary use sites of the connection might rely on. If you need to
    /// keep a reference for later (for example to wait for the connection to close, or to
    /// look up the connection in a map), call [`Connection::weak_handle`] and store the
    /// resulting [`WeakConnectionHandle`] instead.
    ///
    /// [`WeakConnectionHandle`]: crate::endpoint::WeakConnectionHandle
    fn after_handshake<'a>(
        &'a self,
        _conn: &'a Connection,
    ) -> impl Future<Output = AfterHandshakeOutcome> + Send + 'a {
        async { AfterHandshakeOutcome::accept() }
    }

    /// Finish any protected path bootstrap after admission has accepted normal
    /// scope, while the connection is still restricted and no application
    /// handle has been returned. Both connect and accept await this stage.
    ///
    /// A signaling carrier can coordinate a data path here, then commit its
    /// packet gate on this same connection. An error rejects the connection.
    /// Implementations must bound their work and close retained stream handles
    /// on cancellation. Do not store the supplied Endpoint on the hook.
    fn before_data_release<'a>(
        &'a self,
        _endpoint: &'a Endpoint,
        _connection: &'a Connection,
    ) -> impl Future<Output = std::io::Result<()>> + Send + 'a {
        async { Ok(()) }
    }
}

pub(crate) trait DynEndpointHooks: std::fmt::Debug + Send + Sync {
    fn before_connect<'a>(
        &'a self,
        remote_addr: &'a EndpointAddr,
        alpn: &'a [u8],
    ) -> BoxFuture<'a, BeforeConnectOutcome>;
    fn after_handshake<'a>(&'a self, conn: &'a Connection) -> BoxFuture<'a, AfterHandshakeOutcome>;
    fn before_data_release<'a>(
        &'a self,
        endpoint: &'a Endpoint,
        conn: &'a Connection,
    ) -> BoxFuture<'a, std::io::Result<()>>;
}

impl<T: EndpointHooks> DynEndpointHooks for T {
    fn before_connect<'a>(
        &'a self,
        remote_addr: &'a EndpointAddr,
        alpn: &'a [u8],
    ) -> BoxFuture<'a, BeforeConnectOutcome> {
        Box::pin(EndpointHooks::before_connect(self, remote_addr, alpn))
    }

    fn after_handshake<'a>(&'a self, conn: &'a Connection) -> BoxFuture<'a, AfterHandshakeOutcome> {
        Box::pin(EndpointHooks::after_handshake(self, conn))
    }

    fn before_data_release<'a>(
        &'a self,
        endpoint: &'a Endpoint,
        conn: &'a Connection,
    ) -> BoxFuture<'a, std::io::Result<()>> {
        Box::pin(EndpointHooks::before_data_release(self, endpoint, conn))
    }
}

#[derive(Debug, Default)]
pub(crate) struct EndpointHooksList {
    inner: Vec<Box<dyn DynEndpointHooks>>,
}

impl EndpointHooksList {
    pub(super) fn push(&mut self, hook: impl EndpointHooks + 'static) {
        let hook: Box<dyn DynEndpointHooks> = Box::new(hook);
        self.inner.push(hook);
    }

    pub(super) async fn before_connect(
        &self,
        remote_addr: &EndpointAddr,
        alpn: &[u8],
    ) -> BeforeConnectOutcome {
        for hook in self.inner.iter() {
            match hook.before_connect(remote_addr, alpn).await {
                BeforeConnectOutcome::Accept => continue,
                reject @ BeforeConnectOutcome::Reject => {
                    return reject;
                }
            }
        }
        BeforeConnectOutcome::Accept
    }

    pub(super) async fn after_handshake(&self, conn: &Connection) -> AfterHandshakeOutcome {
        let mut restricted = false;
        let mut data_policy = false;
        let mut allowed_data_custom_ids = BTreeSet::new();
        for hook in self.inner.iter() {
            match hook.after_handshake(conn).await {
                AfterHandshakeOutcome::Accept => continue,
                AfterHandshakeOutcome::AcceptData {
                    allowed_data_custom_ids: ids,
                } => {
                    let ids: BTreeSet<_> = ids.into_iter().collect();
                    if data_policy {
                        allowed_data_custom_ids.retain(|id| ids.contains(id));
                    } else {
                        allowed_data_custom_ids = ids;
                    }
                    data_policy = true;
                }
                AfterHandshakeOutcome::AcceptRestricted => restricted = true,
                reject @ AfterHandshakeOutcome::Reject { .. } => {
                    return reject;
                }
            }
        }
        if restricted && data_policy {
            AfterHandshakeOutcome::Reject {
                error_code: 0u32.into(),
                reason: b"conflicting admission scopes".to_vec(),
            }
        } else if restricted {
            AfterHandshakeOutcome::AcceptRestricted
        } else if data_policy {
            AfterHandshakeOutcome::AcceptData {
                allowed_data_custom_ids: allowed_data_custom_ids.into_iter().collect(),
            }
        } else {
            AfterHandshakeOutcome::Accept
        }
    }

    pub(super) async fn before_data_release(
        &self,
        endpoint: &Endpoint,
        conn: &Connection,
    ) -> std::io::Result<()> {
        for hook in &self.inner {
            hook.before_data_release(endpoint, conn).await?;
        }
        Ok(())
    }
}
