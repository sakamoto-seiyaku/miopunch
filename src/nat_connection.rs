//! Node-owned automatic UDP traversal on an admitted original Connection.
//!
//! Each new round observes the real Endpoint and recommends once. The shared
//! selector and maintenance allowance survive rounds and connections; no NAT
//! label, mode override, or experiment environment is consulted here.

use crate::{
    admission_gate::AdmissionGate,
    control_session::{CommitError, ControlSession, DataSession},
    nat_mapping::NodeProbeBudget,
    nat_punch::{self, Attempt, ExecutionStatus, MappingSnapshot},
    nat_selector::Analyzer,
};
use iroh::{RelayMap, Watcher as _};
use std::{
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

/// Local permission and maintenance ceilings. They never shrink a legacy plan.
#[derive(Clone, Debug)]
pub struct UdpPolicy {
    pub enabled: bool,
    pub additional_observations: usize,
    pub peer_datagrams: usize,
    pub max_rounds: u8,
}

impl Default for UdpPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            additional_observations: 64,
            peer_datagrams: crate::nat_scheduler::DEFAULT_LEGACY_PEER_ALLOWANCE,
            max_rounds: 1,
        }
    }
}

/// Reuse one connector per node so learning and allowances cannot refill on retry.
pub struct NatConnector {
    observers: Option<RelayMap>,
    analyzer: Arc<Mutex<Analyzer>>,
    budget: NodeProbeBudget,
    enabled: AtomicBool,
    max_rounds: u8,
}

/// Direct success has no punching recommendation and cannot train its score.
#[derive(Debug)]
pub struct UdpUpgrade {
    pub data: DataSession,
    pub attempt: Option<Attempt>,
    pub direct: Option<std::net::SocketAddr>,
    direct_scope: Option<crate::nat_direct::DirectPath>,
}

enum PreparedUpgrade {
    Direct(crate::nat_direct::DirectPath),
    Punching(Attempt),
}

impl NatConnector {
    pub fn new(observers: Option<RelayMap>, policy: UdpPolicy) -> Self {
        Self {
            observers,
            analyzer: Arc::new(Mutex::new(Analyzer::new(Instant::now()))),
            budget: NodeProbeBudget::new(policy.additional_observations, policy.peer_datagrams),
            enabled: AtomicBool::new(policy.enabled),
            max_rounds: policy.max_rounds.min(32),
        }
    }

    pub fn set_udp_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Release);
    }

    fn check(&self, control: &ControlSession, gate: &AdmissionGate) -> io::Result<()> {
        if !self.enabled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "UDP resource disabled",
            ));
        }
        if !control.may_upgrade_member_path(gate) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "member path upgrade denied",
            ));
        }
        Ok(())
    }

    async fn guard<T>(
        &self,
        control: &ControlSession,
        gate: &AdmissionGate,
        operation: impl std::future::Future<Output = io::Result<T>>,
    ) -> io::Result<T> {
        let scope = control.nat_learning_scope(gate);
        let generation = control.endpoint().udp_network_generation();
        let check = || {
            self.check(control, gate)?;
            if !generation.is_current()
                || control.nat_learning_scope(gate) != scope
                || control.is_closed()
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "NAT connection scope changed",
                ));
            }
            Ok(())
        };
        let polling = async {
            loop {
                check()?;
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            #[allow(unreachable_code)]
            Err::<T, io::Error>(io::Error::other("scope guard stopped"))
        };
        check()?;
        tokio::select! {
            result = operation => { let value = result?; check()?; Ok(value) },
            result = polling => result,
        }
    }

    /// Acquire a fresh ordered mapping sequence only after current admission.
    pub async fn observe(
        &self,
        control: &ControlSession,
        gate: &AdmissionGate,
    ) -> io::Result<MappingSnapshot> {
        self.check(control, gate)?;
        let endpoint = control.endpoint();
        let snapshot = if let Some(observers) = &self.observers {
            if !self.budget.reserve_observations(observers.len()) {
                return Err(io::Error::other("NAT observation budget exhausted"));
            }
            match self
                .guard(
                    control,
                    gate,
                    endpoint.observe_udp_mapping_sequence(observers),
                )
                .await
            {
                Ok(sequence) => {
                    for (index, sample) in sequence.samples.iter().enumerate() {
                        println!(
                            "LEGACY_SAMPLE order={index} destination={} mapped={}",
                            sample.destination, sample.mapped
                        );
                    }
                    MappingSnapshot::from_sequence(sequence)?
                }
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied => return Err(error),
                Err(_) => MappingSnapshot::fallback(endpoint, endpoint.udp_observation().get())?,
            }
        } else {
            MappingSnapshot::fallback(endpoint, endpoint.udp_observation().get())?
        };
        let snapshot = snapshot.with_assisted(endpoint.udp_local_ipv4_candidates())?;
        self.check(control, gate)?;
        println!(
            "LEGACY_FEATURE feature={:?} socket_generation_current={}",
            snapshot.feature(),
            snapshot.is_current()
        );
        Ok(snapshot)
    }

    /// Exactly one Analyze and protected plan exchange for this fresh round.
    pub async fn prepare(
        &self,
        control: &ControlSession,
        gate: &AdmissionGate,
    ) -> io::Result<Attempt> {
        let snapshot = self.observe(control, gate).await?;
        self.coordinate(control, gate, &snapshot).await
    }

    pub async fn coordinate(
        &self,
        control: &ControlSession,
        gate: &AdmissionGate,
        snapshot: &MappingSnapshot,
    ) -> io::Result<Attempt> {
        self.check(control, gate)?;
        let operation = async {
            if control.is_dialing_side() {
                nat_punch::coordinate_client(control, gate, snapshot).await
            } else {
                nat_punch::coordinate_server(control, gate, snapshot, &self.analyzer).await
            }
        };
        let attempt = self.guard(control, gate, operation).await?;
        println!(
            "LEGACY_SELECTED mode={} index={} c={:?} v={:?} scores_before={:?} round={} stable_id={} execution={:?} fallback={}",
            attempt.plan().id.mode,
            attempt.plan().id.index,
            attempt.plan().c,
            attempt.plan().v,
            attempt.plan().scores_before,
            attempt.round().encoded(),
            control.stable_id(),
            attempt.execution_status(),
            attempt.plan().fallback
        );
        println!(
            "LEGACY_CANDIDATES round={} stable_id={} assisted={:?} mapped={:?}",
            attempt.round().encoded(),
            control.stable_id(),
            attempt.assisted_candidates(),
            attempt.mapped_candidates()
        );
        Ok(attempt)
    }

    /// Execute the entire selected plan and validate its original physical path.
    pub async fn execute(
        &self,
        attempt: &mut Attempt,
        control: &ControlSession,
        gate: &AdmissionGate,
    ) -> io::Result<()> {
        self.check(control, gate)?;
        attempt.check_execution(control)?;
        let allowance = self
            .budget
            .reserve_peer_allowance(attempt.peer_datagram_bound())
            .ok_or_else(|| io::Error::other("NAT complete-plan budget insufficient"))?;
        let enabled = || self.check(control, gate);
        let status = attempt
            .execute_checked(control, gate, &allowance, &enabled)
            .await?;
        if status != ExecutionStatus::CompleteExecution {
            return Err(io::Error::other(format!("NAT plan {status:?}")));
        }
        self.check(control, gate)?;
        self.guard(control, gate, attempt.prepare_data_path(control, gate))
            .await?;
        self.check(control, gate)
    }

    /// Bounded retries create fresh rounds and re-Analyze. Retryable errors
    /// return the original restricted session with its queued business intact.
    pub async fn upgrade(
        &self,
        control: ControlSession,
        gate: &AdmissionGate,
    ) -> Result<UdpUpgrade, CommitError> {
        let prepared = match self.prepare_upgrade(&control, gate).await {
            Ok(prepared) => prepared,
            Err(error) => {
                return Err(CommitError::Retryable {
                    error,
                    session: control,
                });
            }
        };
        match prepared {
            PreparedUpgrade::Punching(attempt) => {
                let data = self.commit_checked(control, gate, Some(&attempt)).await?;
                Ok(UdpUpgrade {
                    data,
                    attempt: Some(attempt),
                    direct: None,
                    direct_scope: None,
                })
            }
            PreparedUpgrade::Direct(direct) => {
                let data = self.commit_checked(control, gate, None).await?;
                if !self.enabled.load(Ordering::Acquire)
                    || !direct.generation.is_current()
                    || gate.nat_learning_scope(data.connection()) != Some(direct.scope.clone())
                    || data.connection().stable_id() != direct.connection_id
                {
                    data.connection().close(
                        iroh::endpoint::VarInt::from_u32(0),
                        b"direct commitment invalidated",
                    );
                    return Err(CommitError::Terminal {
                        error: io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "direct commitment invalidated",
                        ),
                    });
                }
                Ok(UdpUpgrade {
                    data,
                    attempt: None,
                    direct: Some(direct.remote),
                    direct_scope: Some(direct),
                })
            }
        }
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
            if !self.enabled.load(Ordering::Acquire)
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
        let polling = async {
            loop {
                if let Err(error) = permitted() {
                    return Err(CommitError::Terminal { error });
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        };
        let operation = async {
            if let Some(attempt) = attempt {
                attempt.commit_winner_path(control, gate).await
            } else {
                control.commit_data_path().await
            }
        };
        let result =
            tokio::select! { result = operation => result, result = polling => return result };
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

    async fn prepare_upgrade(
        &self,
        control: &ControlSession,
        gate: &AdmissionGate,
    ) -> io::Result<PreparedUpgrade> {
        self.check(control, gate)?;
        if self.max_rounds == 0 {
            return Err(io::Error::other("NAT round allowance exhausted"));
        }
        // Direct QUIC validation has a separate bounded maintenance envelope.
        let _direct_allowance = self
            .budget
            .reserve_peer_allowance(64)
            .ok_or_else(|| io::Error::other("NAT direct budget insufficient"))?;
        if let Some(direct) = self
            .guard(control, gate, crate::nat_direct::attempt(control, gate))
            .await?
        {
            direct.check(control, gate)?;
            println!(
                "NAT_DIRECT_SUCCESS stable_id={} target={} punching_recommendations=0",
                control.stable_id(),
                direct.remote
            );
            return Ok(PreparedUpgrade::Direct(direct));
        }
        for round in 0..self.max_rounds {
            println!(
                "NAT_CONNECTION_ROUND number={} stable_id={}",
                round + 1,
                control.stable_id()
            );
            let mut attempt = self.prepare(control, gate).await?;
            match self.execute(&mut attempt, control, gate).await {
                Ok(()) => return Ok(PreparedUpgrade::Punching(attempt)),
                Err(error) => {
                    attempt.finish_failure();
                    if error.kind() != io::ErrorKind::TimedOut || round + 1 == self.max_rounds {
                        return Err(error);
                    }
                }
            }
        }
        Err(io::Error::other("NAT round allowance exhausted"))
    }

    fn confirm_success(
        &self,
        attempt: &mut Attempt,
        data: &DataSession,
        gate: &AdmissionGate,
    ) -> bool {
        self.enabled.load(Ordering::Acquire) && attempt.confirm_success(data, gate, &self.analyzer)
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
        if !self.enabled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "UDP disabled during business confirmation",
            ));
        }
        let disabled = async {
            loop {
                if !self.enabled.load(Ordering::Acquire) {
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
    /// Direct and punching results both confirm bytes on their captured
    /// original scope; direct results never receive a legacy score update.
    pub async fn exchange_upgrade_business(
        &self,
        upgrade: &mut UdpUpgrade,
        gate: &AdmissionGate,
        payload: &[u8],
    ) -> io::Result<Vec<u8>> {
        if let Some(attempt) = upgrade.attempt.as_mut() {
            return self
                .exchange_business(attempt, &upgrade.data, gate, payload)
                .await;
        }
        let direct = upgrade
            .direct_scope
            .as_ref()
            .ok_or_else(|| io::Error::other("missing direct scope"))?;
        let check = || {
            if !self.enabled.load(Ordering::Acquire)
                || !direct.generation.is_current()
                || upgrade.data.connection().stable_id() != direct.connection_id
                || gate.nat_learning_scope(upgrade.data.connection()).as_ref()
                    != Some(&direct.scope)
                || upgrade.data.connection().close_reason().is_some()
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "direct business scope changed",
                ));
            }
            Ok(())
        };
        let polling = async {
            loop {
                check()?;
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            #[allow(unreachable_code)]
            Err::<Vec<u8>, io::Error>(io::Error::other("direct guard stopped"))
        };
        check()?;
        let result = tokio::select! {
            result = tokio::time::timeout(std::time::Duration::from_secs(5), exchange_payload(&upgrade.data, payload)) =>
                result.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "direct business confirmation elapsed"))?,
            result = polling => result,
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
