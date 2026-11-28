//! Commit a prepared path and confirm it with business on the same connection.
use super::{
    ControlUdpTraversal, PreparedUdpPath, upgrade_cancelled_error, upgrade_deadline_error,
    wait_for_invalidation,
};
use crate::{
    admission_gate::AdmissionGate,
    control_session::{CommitError, ControlSession, DataSession},
    nat_punch::Attempt,
};
use std::io;
use tokio::time::Instant as TokioInstant;

/// Direct success has no punching recommendation and cannot train its score.
pub struct UdpUpgrade {
    pub data: DataSession,
    path: PreparedUdpPath,
    traversal: ControlUdpTraversal,
    gate: AdmissionGate,
}

impl std::fmt::Debug for UdpUpgrade {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UdpUpgrade")
            .field("data", &self.data)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl ControlUdpTraversal {
    /// Bounded retries create fresh rounds and re-Analyze. Retryable errors
    /// return the original restricted session with its queued business intact.
    pub async fn upgrade(
        &self,
        control: ControlSession,
        gate: &AdmissionGate,
    ) -> Result<UdpUpgrade, CommitError> {
        let deadline = crate::connection_budget::deadline(control.endpoint().caller_deadline());
        let cancellation = control.endpoint().attempt_cancellation();
        let cancelled = || async {
            match &cancellation {
                Some(token) => token.cancelled().await,
                None => std::future::pending().await,
            }
        };
        let prepared = tokio::select! {
            biased;
            _ = cancelled() => Err(upgrade_cancelled_error()),
            result = Box::pin(self.prepare_upgrade(&control, gate, deadline)) => result,
        };
        let prepared = match prepared {
            Ok(prepared) => prepared,
            Err(error) if control.is_closed() => return Err(CommitError::Terminal { error }),
            Err(error) => {
                return Err(CommitError::Retryable {
                    error,
                    session: control,
                });
            }
        };
        if TokioInstant::now() >= TokioInstant::from(deadline) {
            return Err(CommitError::Retryable {
                error: upgrade_deadline_error(),
                session: control,
            });
        }
        // Once commitment starts, the handoff owns the restricted session and
        // its queue. Dropping that work closes the original through its guard.
        tokio::select! {
            biased;
            _ = cancelled() => Err(CommitError::Terminal {
                error: upgrade_cancelled_error(),
            }),
            _ = tokio::time::sleep_until(deadline.into()) => Err(CommitError::Terminal {
                error: upgrade_deadline_error(),
            }),
            result = self.commit_prepared(control, gate, prepared) => result,
        }
    }

    /// Preserve the existing commit boundary while a common coordinator owns
    /// capability negotiation, fresh path proof and bilateral readiness.
    async fn commit_prepared(
        &self,
        control: ControlSession,
        gate: &AdmissionGate,
        mut path: PreparedUdpPath,
    ) -> Result<UdpUpgrade, CommitError> {
        let data = match &mut path {
            PreparedUdpPath::Punching(attempt) => {
                self.commit_checked(control, gate, Some(attempt)).await?
            }
            PreparedUdpPath::Direct(direct) => {
                let data = self.commit_checked(control, gate, None).await?;
                if !self.config.udp_enabled() || !direct.is_current_on(&data, gate) {
                    data.connection()
                        .close(0u32.into(), b"direct commitment invalidated");
                    return Err(CommitError::Terminal {
                        error: io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "direct commitment invalidated",
                        ),
                    });
                }
                direct.retain();
                data
            }
        };
        Ok(UdpUpgrade {
            data,
            path,
            traversal: self.clone(),
            gate: gate.clone(),
        })
    }

    async fn commit_checked(
        &self,
        control: ControlSession,
        gate: &AdmissionGate,
        attempt: Option<&Attempt>,
    ) -> Result<DataSession, CommitError> {
        let connection = control.scope_connection();
        let scope = control.nat_learning_scope(gate);
        let generation = control.endpoint().udp_network_generation();
        let mut cancellation = control.cancellation_guard();
        let permitted = || {
            if !self.config.udp_enabled()
                || !generation.is_current()
                || scope.is_none()
                || gate.nat_learning_scope(&connection) != scope
                || connection.close_reason().is_some()
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "NAT commitment scope changed",
                ));
            }
            Ok(())
        };
        let operation = async {
            if let Some(attempt) = attempt {
                attempt.commit_winner_path(control, gate).await
            } else {
                control.commit_data_path().await
            }
        };
        let result = tokio::select! {
            result = operation => result,
            error = wait_for_invalidation(permitted) => return Err(CommitError::Terminal { error }),
        };
        match result {
            Ok(data) => {
                if let Err(error) = permitted() {
                    return Err(CommitError::Terminal { error });
                }
                cancellation.disarm();
                Ok(data)
            }
            Err(error) => {
                cancellation.disarm();
                Err(error)
            }
        }
    }

    fn confirm_success(
        &self,
        attempt: &mut Attempt,
        data: &DataSession,
        gate: &AdmissionGate,
    ) -> bool {
        self.config.udp_enabled() && attempt.confirm_success(data, gate, &self.rounds().analyzer)
    }

    /// Real authorized bytes and an exact echo over the committed original
    /// data path precede the one-time score update and retained-winner ownership.
    pub async fn exchange_business(
        &self,
        attempt: &mut Attempt,
        data: &DataSession,
        gate: &AdmissionGate,
        payload: &[u8],
    ) -> io::Result<Vec<u8>> {
        // Check before polling either IO future; select's polling order must
        // not create a stream when permission was already withdrawn.
        if !self.config.udp_enabled() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "UDP disabled during business confirmation",
            ));
        }
        let disabled = async {
            loop {
                if !self.config.udp_enabled() {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "UDP disabled during business confirmation",
                    ));
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        };
        let exchange =
            attempt.guard_committed(data, gate, async { exchange_payload(data, payload).await });
        let received = tokio::select! { result = exchange => result, result = disabled => result }?;
        let learned = self.confirm_success(attempt, data, gate);
        println!(
            "LEGACY_RESULT outcome=confirmed_business learned={learned} score_delta={} stable_id={}",
            if learned { 2 } else { 0 },
            data.connection().stable_id()
        );
        Ok(received)
    }
}

impl UdpUpgrade {
    pub fn attempt(&self) -> Option<&Attempt> {
        match &self.path {
            PreparedUdpPath::Punching(attempt) => Some(attempt),
            PreparedUdpPath::Direct(_) => None,
        }
    }

    pub fn direct(&self) -> Option<std::net::SocketAddr> {
        match &self.path {
            PreparedUdpPath::Direct(path) => Some(path.remote),
            PreparedUdpPath::Punching(_) => None,
        }
    }

    /// Confirm actual bytes with the same node permission, gate and learning state that committed the path.
    pub async fn exchange_business(&mut self, payload: &[u8]) -> io::Result<Vec<u8>> {
        let direct = match &mut self.path {
            PreparedUdpPath::Punching(attempt) => {
                return self
                    .traversal
                    .exchange_business(attempt, &self.data, &self.gate, payload)
                    .await;
            }
            PreparedUdpPath::Direct(path) => path,
        };
        let check = || {
            if !self.traversal.config.udp_enabled() || !direct.is_current_on(&self.data, &self.gate)
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "direct business scope changed",
                ));
            }
            Ok(())
        };
        check()?;
        let result = tokio::select! {
            result = tokio::time::timeout(std::time::Duration::from_secs(5), exchange_payload(&self.data, payload)) =>
                result.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "direct business confirmation elapsed"))?,
            error = wait_for_invalidation(check) => Err(error),
        }?;
        check()?;
        Ok(result)
    }
}

async fn exchange_payload(data: &DataSession, payload: &[u8]) -> io::Result<Vec<u8>> {
    if payload.is_empty() || payload.len() > 16 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid business payload",
        ));
    }
    let outbound = async {
        let (mut send, mut recv) = data.connection().open_bi().await?;
        send.write_all(payload).await?;
        send.finish()?;
        if recv
            .read_to_end(16 * 1024)
            .await
            .map_err(io::Error::other)?
            != payload
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "business echo mismatch",
            ));
        }
        Ok::<(), io::Error>(())
    };
    let inbound = async {
        let (mut send, mut recv) = data.connection().accept_bi().await?;
        let received = recv
            .read_to_end(16 * 1024)
            .await
            .map_err(io::Error::other)?;
        if received.is_empty() {
            return Err(io::Error::other("empty peer business"));
        }
        send.write_all(&received).await?;
        send.finish()?;
        Ok::<_, io::Error>(received)
    };
    let (_, received) = tokio::try_join!(outbound, inbound)?;
    Ok(received)
}
