//! The application handoff for a connection bootstrapped over a control-only
//! transport. Ordinary callers receive the QUIC connection only after its
//! data-path gate is installed.

use std::{collections::VecDeque, future::Future, io, net::SocketAddr, pin::Pin, time::Duration};

use iroh::endpoint::{
    Connection, ControlConnection, ControlExchange, ControlHandle, PathEventStream,
};
use iroh_base::PublicKey;

use crate::admission_gate::AdmissionGate;

const MAX_PENDING_BUSINESS_BYTES: usize = 1024 * 1024;
const MAX_PENDING_BUSINESS_MESSAGES: usize = 64;

/// Result of requesting an explicit data path on the current control connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataPathOpen {
    /// How many times the remote QUIC path identifiers were not yet available.
    pub path_id_retries: u32,
}

/// Owns a restricted connection while it can still use the message carrier.
#[derive(Debug)]
pub struct ControlSession {
    connection: ControlHandle,
    _control_owner: Option<ControlConnection>,
    handoff: Option<DataHandoff>,
    close_on_drop: bool,
    pending_business: VecDeque<Vec<u8>>,
    pending_bytes: usize,
}

type HandoffFuture = Pin<Box<dyn Future<Output = io::Result<Connection>> + Send>>;
struct DataHandoff(std::sync::Mutex<Box<dyn FnOnce(ControlHandle) -> HandoffFuture + Send>>);
impl std::fmt::Debug for DataHandoff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DataHandoff")
    }
}

impl Drop for ControlSession {
    fn drop(&mut self) {
        if self.close_on_drop {
            self.connection
                .close(0u32.into(), b"pending data handoff dropped");
        }
    }
}

impl ControlSession {
    /// Own an admitted control-only session without any business handoff.
    pub fn from_control_connection(control: ControlConnection) -> Self {
        Self {
            connection: control.control_handle(),
            _control_owner: Some(control),
            handoff: None,
            close_on_drop: false,
            pending_business: VecDeque::new(),
            pending_bytes: 0,
        }
    }
    /// Own preparation on a restricted original while waiting for Core admission.
    ///
    /// The callback must await that original's pending restricted connect/accept
    /// operation. It runs only after path commitment, and its result is checked
    /// against the original TLS session before any queued business is released.
    /// It must own cancellation of any spawned work it retains. Dropping this
    /// session or its pending commit closes the original, even with other helpers.
    pub fn with_data_handoff<F, Fut>(control: ControlHandle, handoff: F) -> Self
    where
        F: FnOnce(ControlHandle) -> Fut + Send + 'static,
        Fut: Future<Output = io::Result<Connection>> + Send + 'static,
    {
        Self {
            connection: control,
            _control_owner: None,
            handoff: Some(DataHandoff(std::sync::Mutex::new(Box::new(
                move |control| Box::pin(handoff(control)),
            )))),
            close_on_drop: true,
            pending_business: VecDeque::new(),
            pending_bytes: 0,
        }
    }
    pub(crate) fn from_control(connection: ControlHandle) -> Self {
        Self {
            connection,
            _control_owner: None,
            handoff: None,
            close_on_drop: false,
            pending_business: VecDeque::new(),
            pending_bytes: 0,
        }
    }
    pub(crate) fn endpoint(&self) -> &ControlHandle {
        &self.connection
    }
    /// Retain a restricted helper for metadata and original-session path work.
    pub fn control_handle(&self) -> ControlHandle {
        self.connection.clone()
    }

    pub(crate) fn scope_connection(&self) -> ControlHandle {
        self.connection.clone()
    }
    /// Stable local ID for tracing this exact QUIC connection across paths.
    pub fn stable_id(&self) -> usize {
        self.connection.stable_id()
    }

    /// Authenticated peer identity for the restricted bootstrap exchange.
    pub fn remote_id(&self) -> PublicKey {
        self.connection.remote_id()
    }

    pub(crate) fn is_dialing_side(&self) -> bool {
        self.connection.side().is_client()
    }

    pub(crate) fn cancellation_guard(&self) -> CommitCancellationGuard {
        CommitCancellationGuard {
            connection: self.connection.weak_handle(),
            armed: true,
        }
    }

    /// Recheck membership on this exact admitted control connection before
    /// disclosing a candidate or spending a direct-path probe budget.
    pub fn may_upgrade_member_path(&self, gate: &AdmissionGate) -> bool {
        gate.allows_member_path_upgrade(&self.connection)
    }

    pub(crate) fn nat_learning_scope(
        &self,
        gate: &AdmissionGate,
    ) -> Option<crate::nat_selector::LearningScope> {
        gate.nat_learning_scope(&self.connection)
    }

    /// Open a complete, bounded control exchange on this original session.
    pub async fn open_control_exchange(&self) -> io::Result<ControlExchange> {
        self.connection.open_control_exchange().await
    }
    /// Accept a complete, bounded control exchange on this original session.
    pub async fn accept_control_exchange(&self) -> io::Result<ControlExchange> {
        self.connection.accept_control_exchange().await
    }
    /// Install the original connection packet guard without releasing business.
    pub async fn commit_control_path(&self) -> io::Result<()> {
        self.connection.commit_data_path().await
    }
    pub(crate) fn retain_udp_winner(
        &self,
        lease: &mut iroh::endpoint::UdpSocketPathLease,
    ) -> io::Result<()> {
        self.connection.retain_udp_winner(lease)
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
    /// A newly authenticated QUIC connection may still be receiving spare path
    /// identifiers, so retry that one transient error within a two-second bound.
    pub async fn open_data_path(&self, remote: SocketAddr) -> io::Result<()> {
        self.open_data_path_report(remote).await.map(|_| ())
    }

    /// The same bounded attempt with a retry count for diagnostics.
    pub async fn open_data_path_report(&self, remote: SocketAddr) -> io::Result<DataPathOpen> {
        self.open_data_path_on(
            None,
            remote,
            tokio::time::Instant::now() + Duration::from_secs(2),
            &|| Ok(()),
        )
        .await
    }

    /// Validate a path through the winner's actual retained physical UDP socket.
    pub async fn open_data_path_from(
        &self,
        local: iroh::endpoint::LocalSocketBinding,
        remote: SocketAddr,
    ) -> io::Result<()> {
        self.open_data_path_from_report(local, remote)
            .await
            .map(|_| ())
    }

    pub async fn open_data_path_from_report(
        &self,
        local: iroh::endpoint::LocalSocketBinding,
        remote: SocketAddr,
    ) -> io::Result<DataPathOpen> {
        self.open_data_path_on(
            Some(local),
            remote,
            tokio::time::Instant::now() + Duration::from_secs(2),
            &|| Ok(()),
        )
        .await
    }

    pub(crate) async fn open_data_path_from_until(
        &self,
        local: iroh::endpoint::LocalSocketBinding,
        remote: SocketAddr,
        deadline: tokio::time::Instant,
        check: &(dyn Fn() -> io::Result<()> + Sync),
    ) -> io::Result<DataPathOpen> {
        self.open_data_path_on(Some(local), remote, deadline, check)
            .await
    }

    async fn open_data_path_on(
        &self,
        local: Option<iroh::endpoint::LocalSocketBinding>,
        remote: SocketAddr,
        deadline: tokio::time::Instant,
        check: &(dyn Fn() -> io::Result<()> + Sync),
    ) -> io::Result<DataPathOpen> {
        let mut path_id_retries = 0;
        tokio::time::timeout_at(deadline, async {
            loop {
                check()?;
                let result = match local {
                    Some(local) => self.connection.open_data_path_from(local, remote).await,
                    None => self.connection.open_data_path(remote).await,
                };
                match result {
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        path_id_retries += 1;
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                    result => {
                        check()?;
                        return result.map(|_| DataPathOpen { path_id_retries });
                    }
                }
            }
        })
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "UDP path identifiers unavailable"))?
    }

    /// Deterministically exercise the path-ID retry branch in a real data-path lab.
    #[cfg(feature = "nat-lab-test-hooks")]
    #[doc(hidden)]
    pub async fn test_force_path_id_retry_once(&self) -> io::Result<()> {
        self.connection.test_force_path_id_retry_once().await
    }

    /// Send a small UDP mapping probe from the authenticated endpoint's QUIC socket.
    pub async fn prime_udp_path(&self, remote: SocketAddr) -> io::Result<()> {
        self.connection.prime_udp_path(remote).await
    }

    pub(crate) fn traversal_key(&self, round: &[u8; 16]) -> io::Result<[u8; 32]> {
        let mut key = [0; 32];
        self.connection
            .export_keying_material(&mut key, b"miopunch-nat-traversal-v1", round)
            .map_err(|e| io::Error::other(format!("{e:?}")))?;
        Ok(key)
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.connection.close_reason().is_some()
    }

    pub async fn register_udp_traversal(
        &self,
        round: [u8; 16],
    ) -> io::Result<iroh::endpoint::UdpTraversalLease> {
        self.connection.register_udp_traversal(round).await
    }

    /// Whether a validated IP path is visible to this connection.
    pub fn has_data_path(&self) -> bool {
        self.connection
            .paths()
            .iter()
            .any(|path| matches!(path.remote_addr(), iroh_base::TransportAddr::Ip(_)))
    }

    pub(crate) fn connection_paths(&self) -> Vec<iroh_base::TransportAddr> {
        self.connection
            .paths()
            .iter()
            .map(|path| path.remote_addr().clone())
            .collect()
    }

    pub(crate) fn has_data_path_from(
        &self,
        local: iroh::endpoint::LocalSocketBinding,
        remote: SocketAddr,
    ) -> bool {
        self.connection.paths().iter().any(|path| path.local_socket_id() == Some(local.id)
            && matches!(path.remote_addr(), iroh_base::TransportAddr::Ip(addr) if *addr == remote))
    }

    /// Observes path closure before the data handle is released.
    pub fn path_events(&self) -> PathEventStream {
        self.connection.path_events()
    }

    /// Consumes the control stage and releases the normal connection only
    /// after the per-connection path gate has been installed. A transient
    /// failure returns the restricted session with its pending business intact;
    /// a closed connection is terminal and never becomes a business handle.
    pub async fn commit_data_path(mut self) -> Result<DataSession, CommitError> {
        if self.handoff.is_none() {
            return Err(CommitError::Terminal {
                error: io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "business is released only by restricted endpoint admission",
                ),
            });
        }
        if let Err(error) = self.connection.commit_data_path().await {
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
        let handoff = self.handoff.take().expect("checked data handoff");
        let business = {
            let mut original_binding = [0; 32];
            self.connection
                .export_keying_material(&mut original_binding, b"miopunch-control-handoff-v1", b"")
                .map_err(|error| CommitError::Terminal {
                    error: io::Error::other(format!("{error:?}")),
                })?;
            let cancellation = self.connection.attempt_cancellation();
            let deadline = self.connection.caller_deadline();
            let canceled = async {
                match cancellation {
                    Some(token) => token.cancelled().await,
                    None => std::future::pending().await,
                }
            };
            let expired = async {
                match deadline {
                    Some(deadline) => n0_future::time::sleep_until(deadline).await,
                    None => std::future::pending().await,
                }
            };
            let result = tokio::select! {
                biased;
                _ = canceled => Err(io::Error::new(io::ErrorKind::Interrupted,"data handoff canceled")),
                _ = expired => Err(io::Error::new(io::ErrorKind::TimedOut,"data handoff deadline elapsed")),
                reason = self.connection.closed() => Err(io::Error::other(format!("original closed during data handoff: {reason}"))),
                result = (handoff.0.into_inner().expect("owned handoff"))(self.connection.clone()) => result,
            };
            let business = result.map_err(|error| CommitError::Terminal { error })?;
            let mut delivered_binding = [0; 32];
            let binding_matches = business
                .export_keying_material(&mut delivered_binding, b"miopunch-control-handoff-v1", b"")
                .is_ok()
                && original_binding == delivered_binding;
            if business.stable_id() != self.connection.stable_id()
                || business.remote_id() != self.connection.remote_id()
                || !binding_matches
                || business.close_reason().is_some()
                || self.connection.close_reason().is_some()
            {
                business.close(0u32.into(), b"wrong or closed original data handoff");
                return Err(CommitError::Terminal {
                    error: io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "data handoff is not the live original TLS session",
                    ),
                });
            }
            business
        };
        self.close_on_drop = false;
        Ok(DataSession {
            connection: business,
            pending_business: std::mem::take(&mut self.pending_business),
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

/// A canceled actor commitment must close even if live streams retain handles.
pub(crate) struct CommitCancellationGuard {
    connection: iroh::endpoint::WeakControlConnection,
    armed: bool,
}
impl CommitCancellationGuard {
    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}
impl Drop for CommitCancellationGuard {
    fn drop(&mut self) {
        if self.armed {
            self.connection.close(
                iroh::endpoint::VarInt::from_u32(0),
                b"canceled NAT path commitment",
            );
        }
    }
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
