//! The application handoff for a connection bootstrapped over a control-only
//! transport. Ordinary callers receive the QUIC connection only after its
//! data-path gate is installed.

use std::{collections::VecDeque, io, net::SocketAddr};

use iroh::{
    Endpoint, EndpointAddr,
    endpoint::{AcceptBi, ConnectError, Connection, OpenBi, PathEventStream},
};

const MAX_PENDING_BUSINESS_BYTES: usize = 1024 * 1024;
const MAX_PENDING_BUSINESS_MESSAGES: usize = 64;

/// Owns a restricted connection while it can still use the message carrier.
#[derive(Debug)]
pub struct ControlSession {
    endpoint: Endpoint,
    connection: Connection,
    pending_business: VecDeque<Vec<u8>>,
    pending_bytes: usize,
}

impl ControlSession {
    /// Accepts a full-handshake control connection without exposing its raw
    /// application handle. The endpoint must be configured for a control-only
    /// ALPN and must not enable early connection handles.
    pub async fn accept(endpoint: &Endpoint) -> io::Result<Self> {
        let incoming = endpoint
            .accept()
            .await
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "endpoint closed"))?;
        let connection = incoming.await.map_err(io::Error::other)?;
        Ok(Self {
            endpoint: endpoint.clone(),
            connection,
            pending_business: VecDeque::new(),
            pending_bytes: 0,
        })
    }

    /// Connects through the explicit control route; the normal business handle
    /// is not returned to the caller.
    pub async fn connect(
        endpoint: &Endpoint,
        target: EndpointAddr,
        alpn: &[u8],
    ) -> Result<Self, ConnectError> {
        let connection = endpoint.connect(target, alpn).await?;
        Ok(Self {
            endpoint: endpoint.clone(),
            connection,
            pending_business: VecDeque::new(),
            pending_bytes: 0,
        })
    }

    /// Stable local ID for tracing this exact QUIC connection across paths.
    pub fn stable_id(&self) -> usize {
        self.connection.stable_id()
    }

    /// Opens a stream reserved for the bootstrap control protocol.
    pub fn open_control_bi(&self) -> OpenBi<'_> {
        self.connection.open_bi()
    }

    /// Accepts a stream reserved for the bootstrap control protocol.
    pub fn accept_control_bi(&self) -> AcceptBi<'_> {
        self.connection.accept_bi()
    }

    /// Retains ordinary business bytes locally until the data path is committed.
    pub fn queue_business(&mut self, bytes: Vec<u8>) -> io::Result<()> {
        if bytes.is_empty()
            || self.pending_business.len() >= MAX_PENDING_BUSINESS_MESSAGES
            || bytes.len() > MAX_PENDING_BUSINESS_BYTES.saturating_sub(self.pending_bytes)
        {
            return Err(io::Error::other(
                "empty business payload or pending queue full",
            ));
        }
        self.pending_bytes += bytes.len();
        self.pending_business.push_back(bytes);
        Ok(())
    }

    /// Begins validating a known, authorized UDP data address on this connection.
    pub async fn open_data_path(&self, remote: SocketAddr) -> io::Result<()> {
        self.endpoint.open_data_path(&self.connection, remote).await
    }

    /// Whether a validated IP path is visible to this connection.
    pub fn has_data_path(&self) -> bool {
        self.connection
            .paths()
            .iter()
            .any(|path| matches!(path.remote_addr(), iroh_base::TransportAddr::Ip(_)))
    }

    /// Observes path closure before the data handle is released.
    pub fn path_events(&self) -> PathEventStream {
        self.connection.path_events()
    }

    /// Consumes the control stage and releases the normal connection only
    /// after the per-connection path gate has been installed. A transient
    /// failure returns the restricted session with its pending business intact;
    /// a closed connection is terminal and never becomes a business handle.
    pub async fn commit_data_path(self) -> Result<DataSession, CommitError> {
        if let Err(error) = self.endpoint.commit_data_path(&self.connection).await {
            return if self.connection.close_reason().is_some() {
                Err(CommitError::Terminal { error })
            } else {
                Err(CommitError::Retryable {
                    error,
                    session: self,
                })
            };
        }
        if let Some(reason) = self.connection.close_reason() {
            return Err(CommitError::Terminal {
                error: io::Error::other(format!("connection closed during promotion: {reason}")),
            });
        }
        Ok(DataSession {
            connection: self.connection,
            pending_business: self.pending_business,
        })
    }
}

/// A failed promotion, separated by whether the original connection survives.
#[derive(Debug)]
pub enum CommitError {
    /// The restricted session and its queue are intact; retry is possible.
    Retryable {
        /// Why this promotion attempt failed.
        error: io::Error,
        /// The original restricted session.
        session: ControlSession,
    },
    /// The original connection closed; queued business must fail.
    Terminal {
        /// Why the connection could not be promoted.
        error: io::Error,
    },
}

/// A connection whose custom control paths have been forbidden.
#[derive(Debug)]
pub struct DataSession {
    connection: Connection,
    pending_business: VecDeque<Vec<u8>>,
}

impl DataSession {
    /// Public QUIC handle for ordinary business after path commitment.
    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Business bytes queued before a data path existed.
    pub fn take_pending_business(&mut self) -> VecDeque<Vec<u8>> {
        std::mem::take(&mut self.pending_business)
    }
}
