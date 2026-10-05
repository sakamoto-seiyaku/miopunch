//! Existing native traversal algorithms behind an owned candidate boundary.
//! The common coordinator owns capability exchange, fresh proof and readiness;
//! this adapter never grants membership or starts another handshake/scheduler.
use std::{
    io,
    sync::{Arc, Mutex},
};

use iroh::{Endpoint, TransportAddr, endpoint::Connection};

use crate::{
    admission_gate::{AdmissionGate, PathQualification},
    connection_coordinator::PreparedPath,
    control_session::{CommitError, ControlSession},
    nat_connection::{NatConnector, PreparedUdpPath},
    tcp_punch::{PreparedTcpPath, TcpPunch},
};

#[derive(Clone, Copy, Debug)]
pub(crate) enum NativeMechanism {
    Udp,
    Tcp,
}

#[derive(Clone)]
pub(crate) struct NativePathAdapter {
    udp: Arc<NatConnector>,
    tcp: Option<TcpPunch>,
}
impl std::fmt::Debug for NativePathAdapter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativePathAdapter")
            .field("udp_available", &self.udp_available())
            .field("tcp_available", &self.tcp_available())
            .finish_non_exhaustive()
    }
}
impl NativePathAdapter {
    pub(crate) fn new(udp: Arc<NatConnector>, tcp: Option<TcpPunch>) -> Self {
        Self { udp, tcp }
    }

    pub(crate) fn udp_available(&self) -> bool {
        self.udp.udp_enabled()
    }
    pub(crate) fn tcp_available(&self) -> bool {
        self.tcp
            .as_ref()
            .is_some_and(|tcp| !tcp.resources.tcp_direct_disabled)
    }

    pub(crate) async fn prepare(
        &self,
        kind: NativeMechanism,
        endpoint: &Endpoint,
        original: &Connection,
        gate: &AdmissionGate,
    ) -> io::Result<NativeCandidate> {
        let qualification = gate.path_qualification(original).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "native member qualification missing",
            )
        })?;
        let prepared = match kind {
            NativeMechanism::Udp if self.udp_available() => {
                let control = ControlSession::from_connection(endpoint.clone(), original.clone());
                PreparedNative::Udp(self.udp.prepare_upgrade(&control, gate).await?)
            }
            NativeMechanism::Tcp if self.tcp_available() => {
                let tcp = self.tcp.as_ref().unwrap();
                // Installed direct TCP and enhanced simultaneous-open support
                // are independent. Preparation must still exchange the same
                // protected unavailable status when local enhancement is
                // absent, so the peer does not consume UDP frames as TCP.
                PreparedNative::Tcp(tcp.prepare(endpoint, original, gate).await?)
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "native mechanism disabled",
                ));
            }
        };
        let (route, local_socket) = match &prepared {
            PreparedNative::Udp(PreparedUdpPath::Direct(path)) => {
                (TransportAddr::Ip(path.remote), None)
            }
            PreparedNative::Udp(PreparedUdpPath::Punching(attempt)) => {
                let remote = attempt
                    .remote_target()
                    .ok_or_else(|| io::Error::other("native UDP winner missing route"))?;
                let local = attempt
                    .local_winner()
                    .ok_or_else(|| io::Error::other("native UDP winner missing socket"))?;
                (TransportAddr::Ip(remote), Some(local.id))
            }
            PreparedNative::Tcp(path) => (TransportAddr::Custom(path.route().clone()), None),
        };
        let candidate = NativeCandidate {
            adapter: self.clone(),
            kind,
            prepared,
            route,
            local_socket,
            qualification,
            original: original.weak_handle(),
            stable_id: original.stable_id(),
        };
        candidate.check(endpoint, original, gate)?;
        Ok(candidate)
    }

    /// Preserve the existing native TCP-first/UDP fallback order. Each
    /// mechanism consumes only its already-owned node allowance and the outer
    /// caller deadline; there is no new round or reconnect loop here.
    pub(crate) async fn prepare_paths(
        &self,
        endpoint: &Endpoint,
        original: &Connection,
        gate: &AdmissionGate,
        tcp: bool,
        udp: bool,
    ) -> io::Result<PreparedPath> {
        if tcp {
            match self
                .prepare(NativeMechanism::Tcp, endpoint, original, gate)
                .await
            {
                Ok(candidate) => return candidate.into_shared(endpoint, original, gate),
                Err(error) if error.kind() == io::ErrorKind::InvalidData => return Err(error),
                Err(error) => {
                    if !gate.allows_member_path_upgrade(original)
                        || original.close_reason().is_some()
                    {
                        return Err(error);
                    }
                    eprintln!("NATIVE_CANDIDATE_FAILED mechanism=tcp error={error}");
                }
            }
        }
        if udp {
            return self
                .prepare(NativeMechanism::Udp, endpoint, original, gate)
                .await?
                .into_shared(endpoint, original, gate);
        }
        Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "no prepared native data path",
        ))
    }
}

enum PreparedNative {
    Udp(PreparedUdpPath),
    Tcp(PreparedTcpPath),
}

pub(crate) struct NativeCandidate {
    adapter: NativePathAdapter,
    kind: NativeMechanism,
    prepared: PreparedNative,
    route: TransportAddr,
    local_socket: Option<iroh::endpoint::LocalSocketId>,
    qualification: PathQualification,
    original: iroh::endpoint::WeakConnectionHandle,
    stable_id: usize,
}
impl NativeCandidate {
    fn check(
        &self,
        endpoint: &Endpoint,
        original: &Connection,
        gate: &AdmissionGate,
    ) -> io::Result<()> {
        if original.stable_id() != self.stable_id
            || !self.original.is_open()
            || !self.qualification.is_current()
            || !gate.allows_member_path_upgrade(original)
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "native original qualification changed",
            ));
        }
        match &self.prepared {
            PreparedNative::Udp(prepared) => {
                if !self.adapter.udp_available() {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "UDP resource disabled",
                    ));
                }
                let control = ControlSession::from_connection(endpoint.clone(), original.clone());
                match prepared {
                    PreparedUdpPath::Direct(path) => path.check(&control, gate)?,
                    PreparedUdpPath::Punching(attempt) => {
                        attempt.verify_winner_path(&control, gate)?
                    }
                }
            }
            PreparedNative::Tcp(path) => path.check(endpoint, original, gate)?,
        }
        self.path_id(original).map(|_| ())
    }

    fn path_id(&self, original: &Connection) -> io::Result<String> {
        original
            .paths()
            .iter()
            .find(|path| {
                path.remote_addr() == &self.route
                    && self
                        .local_socket
                        .is_none_or(|local| path.local_socket_id() == Some(local))
            })
            .map(|path| path.id().to_string())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotConnected,
                    "exact native candidate path disappeared",
                )
            })
    }

    async fn retain_on_original(
        self,
        endpoint: &Endpoint,
        original: &Connection,
        gate: &AdmissionGate,
    ) -> io::Result<()> {
        self.check(endpoint, original, gate)?;
        match self.prepared {
            PreparedNative::Udp(prepared) => {
                let control = ControlSession::from_connection(endpoint.clone(), original.clone());
                let mut retained =
                    self.adapter
                        .udp
                        .commit_prepared(control, gate, prepared)
                        .await
                        .map_err(|error| match error {
                            CommitError::Retryable { error, .. }
                            | CommitError::Terminal { error } => error,
                        })?;
                if let Some(attempt) = retained.attempt.as_mut() {
                    attempt.retain_application_path(&retained.data, gate)?;
                }
            }
            PreparedNative::Tcp(path) => path.retain(endpoint, original, gate)?,
        }
        Ok(())
    }

    fn into_shared(
        self,
        endpoint: &Endpoint,
        original: &Connection,
        gate: &AdmissionGate,
    ) -> io::Result<PreparedPath> {
        let path_id = self.path_id(original)?;
        let route = self.route.clone();
        let kind = self.kind;
        let local_socket = self.local_socket;
        let qualification = self.qualification.clone();
        let generation = endpoint.udp_network_generation();
        let adapter = self.adapter.clone();
        let state = Arc::new(Mutex::new(Some(self)));
        let current_state = state.clone();
        let current_endpoint = endpoint.clone();
        let current_original = original.weak_handle();
        let current_gate = gate.clone();
        let current_route = route.clone();
        let retain_state = state.clone();
        let retain_endpoint = endpoint.clone();
        let retain_original = original.weak_handle();
        let retain_gate = gate.clone();
        let rollback_endpoint = endpoint.clone();
        let rollback_original = original.weak_handle();
        let rollback_route = route.clone();
        let rollback_path_id = path_id.clone();
        Ok(PreparedPath::candidate(original, route, move || {
            let original = current_original
                .upgrade()
                .ok_or_else(|| io::Error::other("native original ended"))?;
            if !qualification.is_current()
                || !generation.is_current()
                || !current_gate.allows_member_path_upgrade(&original)
                || original.close_reason().is_some()
                || (matches!(kind, NativeMechanism::Udp) && !adapter.udp_available())
                || (matches!(kind, NativeMechanism::Tcp) && !adapter.tcp_available())
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "native qualification changed",
                ));
            }
            if let Some(candidate) = current_state.lock().unwrap().as_ref() {
                candidate.check(&current_endpoint, &original, &current_gate)?;
            }
            if !original.paths().iter().any(|path| {
                path.remote_addr() == &current_route
                    && local_socket.is_none_or(|id| path.local_socket_id() == Some(id))
            }) {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "retained native path disappeared",
                ));
            }
            Ok(())
        })
        .with_exact_path_id(path_id)
        .with_retain(move || {
            Box::pin(async move {
                let candidate = { retain_state.lock().unwrap().take() }
                    .ok_or_else(|| io::Error::other("native candidate already consumed"))?;
                let original = retain_original
                    .upgrade()
                    .ok_or_else(|| io::Error::other("native original ended before retention"))?;
                candidate
                    .retain_on_original(&retain_endpoint, &original, &retain_gate)
                    .await
            })
        })
        .with_rollback(move || {
            let candidate = { state.lock().unwrap().take() };
            // A retained candidate already moved its physical owners to M.
            if candidate.is_some() {
                if let Some(original) = rollback_original.upgrade() {
                    for path in original.paths().iter().filter(|path| {
                        path.id().to_string() == rollback_path_id
                            && path.remote_addr() == &rollback_route
                            && local_socket.is_none_or(|id| path.local_socket_id() == Some(id))
                    }) {
                        let _ = path.close();
                    }
                    if let TransportAddr::Custom(address) = rollback_route {
                        tokio::spawn(async move {
                            let _ = rollback_endpoint
                                .retire_control_path(&original, address)
                                .await;
                        });
                    }
                }
                drop(candidate);
            }
        }))
    }
}
