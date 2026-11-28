//! Path improvement for the RTC member wire protocol.
use super::{AdmissionGate, Candidate, MaintenanceStatus, ResourceEndpoint};
use crate::{
    connection_coordinator::CoordinatorHook,
    resources::ContactClass,
    session_maintenance::{Completion, LocalNatBudgetError, MaintenanceDeferral, MaintenanceError},
};
use iroh::{
    EndpointAddr, EndpointId, TransportAddr,
    endpoint::{Connection, ObservationValidity, WeakConnectionHandle},
};
use std::{collections::BTreeMap, net::SocketAddr, sync::Arc, time::Duration};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

struct Improvement {
    original: WeakConnectionHandle,
    failures: u32,
    next_due: Option<Instant>,
    budget_not_before: Option<Instant>,
    wait_clues: Option<ImprovementClues>,
}
#[derive(Clone, PartialEq)]
struct ImprovementClues {
    target: EndpointAddr,
    local: EndpointAddr,
    direct_capabilities: u8,
    revision: u64,
    udp_hint: Option<SocketAddr>,
}

impl Improvement {
    fn defer(&mut self, next_due: Instant) {
        self.next_due = Some(next_due);
    }

    fn clear_wait(&mut self) {
        self.budget_not_before = None;
        self.wait_clues = None;
    }

    fn ready_at(&self, node_budget_wait: Option<Instant>) -> Option<Instant> {
        let due = self.next_due?;
        Some(self.budget_not_before.map_or(due, |budget| {
            budget.max(due).max(node_budget_wait.unwrap_or(due))
        }))
    }

    fn failed(&mut self, now: Instant, floor: Duration) {
        self.failures = (self.failures + 1).min(5);
        self.defer(now + floor * (1 << self.failures));
    }
}
pub(super) struct RtcMaintenance {
    pub(super) next_due: Instant,
    shutdown: CancellationToken,
    resources: Arc<ResourceEndpoint>,
    gate: AdmissionGate,
    adapter: CoordinatorHook,
    floor: Duration,
    improve_generation: ObservationValidity,
    improvements: BTreeMap<usize, Improvement>,
    last_improved: Option<usize>,
}

impl RtcMaintenance {
    pub(super) fn new(
        resources: Arc<ResourceEndpoint>,
        gate: AdmissionGate,
        adapter: CoordinatorHook,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            next_due: Instant::now(),
            shutdown,
            improve_generation: resources.endpoint().udp_network_generation(),
            floor: Duration::from_millis(resources.maintenance_policy().path_retry_base_ms),
            resources,
            gate,
            adapter,
            improvements: BTreeMap::new(),
            last_improved: None,
        }
    }

    pub(super) async fn run_due(
        &mut self,
        candidates: &BTreeMap<EndpointId, Candidate>,
        status: &mut MaintenanceStatus,
    ) {
        if Instant::now() >= self.next_due {
            let started = self.improve_live_member(candidates, status).await;
            let now = Instant::now();
            let mut improve_at = now + self.floor;
            if !started {
                // Continue observing path loss and generation changes on the
                // same node timer, but wake at an earlier pending retry.
                let node_budget_wait = self.enhanced_budget_wait(now);
                for next in self
                    .improvements
                    .values()
                    .filter_map(|state| state.ready_at(node_budget_wait))
                    .filter(|at| *at > now)
                {
                    improve_at = improve_at.min(next);
                }
            }
            self.next_due = improve_at;
        }
    }

    fn enhanced_budget_wait(&self, now: Instant) -> Option<Instant> {
        if !self
            .improvements
            .values()
            .any(|state| state.budget_not_before.is_some())
        {
            return None;
        }
        let due = Instant::from(self.adapter.udp_round_wait_until(now.into())?);
        // The previous cooldown may already have elapsed while a holder
        // remains active. Recheck on the normal floor without changing it.
        Some(if due <= now { now + self.floor } else { due })
    }
    fn improvement_clues(
        &self,
        original: &Connection,
        local: &EndpointAddr,
        direct_capabilities: u8,
        hint: Option<&Candidate>,
    ) -> Option<ImprovementClues> {
        if !self.gate.allows_member_path_upgrade(original)
            || self.adapter.has_qualified_direct(&self.resources, original)
        {
            return None;
        }
        let qualification = self.gate.path_qualification(original)?;
        let peer = original.remote_id();
        let paths = original.paths();
        let preferred_relay = paths
            .iter()
            .find(|path| {
                path.is_selected()
                    && path.is_relay()
                    && self.resources.allows_data_path(path.remote_addr())
            })
            .map(|path| path.remote_addr().clone());
        let current = paths
            .iter()
            .filter(|path| self.resources.allows_data_path(path.remote_addr()))
            .map(|path| path.remote_addr().clone())
            .filter(|address| Some(address) != preferred_relay.as_ref());
        let learned = self
            .resources
            .contacts
            .get_endpoint_info(peer)
            .map(|info| info.into_endpoint_addr());
        let learned_relays = learned
            .iter()
            .flat_map(|contact| contact.addrs.iter())
            .filter(|address| matches!(address, TransportAddr::Relay(_)));
        let hints = hint
            .into_iter()
            .flat_map(|candidate| &candidate.address.addrs)
            .chain(learned_relays)
            .filter(|address| {
                !matches!(address, TransportAddr::Relay(_))
                    || self.resources.allows_data_path(address)
            });
        let target = EndpointAddr::new(peer).with_addrs(
            preferred_relay
                .iter()
                .cloned()
                .chain(current)
                .chain(hints.cloned())
                .take(8),
        );
        (!target.addrs.is_empty()).then(|| ImprovementClues {
            target,
            local: local.clone(),
            direct_capabilities,
            revision: qualification.revision(),
            udp_hint: hint.and_then(|candidate| candidate.udp_hint),
        })
    }

    async fn improve_live_member(
        &mut self,
        candidates: &BTreeMap<EndpointId, Candidate>,
        status: &mut MaintenanceStatus,
    ) -> bool {
        use crate::connection_coordinator::AttemptContext;

        let active = &self.adapter;
        let generation = self.resources.endpoint().udp_network_generation();
        if generation != self.improve_generation {
            self.improve_generation = generation;
            self.improvements.clear();
        }
        let registry = self.gate.maintenance();
        let live = registry.live();
        self.improvements
            .retain(|_, state| state.original.is_open());
        let direct_capabilities = active.improvement_capabilities();
        if direct_capabilities == 0 || live.is_empty() {
            return false;
        }
        let floor = self.floor;
        let now = Instant::now();
        let node_budget_wait = self.enhanced_budget_wait(now);
        let local = self.resources.addr();
        let mut due = Vec::new();
        for original in live {
            let stable = original.stable_id();
            let Some(clues) = self.improvement_clues(
                &original,
                &local,
                direct_capabilities,
                candidates.get(&original.remote_id()),
            ) else {
                continue;
            };
            let state = self.improvements.entry(stable).or_insert(Improvement {
                original: original.weak_handle(),
                failures: 0,
                next_due: Some(now),
                budget_not_before: None,
                wait_clues: None,
            });
            if state.wait_clues.as_ref().is_some_and(|old| old != &clues) {
                state.next_due.get_or_insert(now);
                state.clear_wait();
            }
            if state.ready_at(node_budget_wait).is_some_and(|at| at <= now) {
                due.push((state.next_due.unwrap(), stable, original, clues));
            }
        }
        due.sort_unstable_by_key(|(at, id, _, _)| {
            (*at, self.last_improved.is_some_and(|last| *id <= last), *id)
        });
        for (_, stable, original, clues) in due {
            let state = self.improvements.get_mut(&stable).unwrap();
            let Ok((deadline, contact)) = self
                .resources
                .begin_contact_attempt(original.remote_id(), ContactClass::Background)
            else {
                state.defer(Instant::now() + floor);
                continue;
            };
            let attempt = AttemptContext::new(deadline, self.shutdown.child_token());
            let mut lease = match registry.acquire_outgoing(&original, &attempt, contact) {
                Ok(lease) => lease,
                Err(MaintenanceError::SessionClosed) => {
                    self.improvements.remove(&stable);
                    continue;
                }
                Err(MaintenanceError::Cancelled) => return false,
                Err(_) => {
                    state.defer(Instant::now() + floor);
                    continue;
                }
            };
            self.last_improved = Some(stable);
            status.contact_attempts += 1;
            let original_handle = self.resources.endpoint().control_handle(&original);
            self.gate
                .with_direct_candidates(&original_handle, |progress| {
                    progress.active_enhanced_only = None;
                });
            let result = lease
                .run_ref(active.improve(
                    self.resources.endpoint(),
                    &original,
                    clues.target.clone(),
                    attempt,
                    || lease.seal_outgoing_hello(),
                ))
                .await;
            let now = Instant::now();
            match self
                .gate
                .with_direct_candidates(&original_handle, |progress| {
                    progress.active_enhanced_only.take()
                })
                .flatten()
            {
                Some(true) => {
                    state.budget_not_before.get_or_insert(now);
                    state.wait_clues = Some(clues.clone());
                }
                Some(false) => state.clear_wait(),
                None => {}
            }
            match result {
                Ok(Ok(path)) => {
                    lease.completed(Completion::Active(path));
                    self.improvements.remove(&stable);
                    status.cause = None;
                }
                Ok(Err(error)) => {
                    status.cause = Some(format!("active improvement: {error:?}"));
                    match MaintenanceDeferral::from_error(&error) {
                        Some(MaintenanceDeferral::LocalBudget { next_due }) => {
                            // The ledger controls when this M can retry, but
                            // a cooldown must not erase its age in the queue.
                            state.budget_not_before =
                                Some((now + floor).max(Instant::from(*next_due)));
                            state.wait_clues = Some(clues);
                        }
                        Some(MaintenanceDeferral::LocalAlternatives) => {
                            state.clear_wait();
                            state.defer(now + floor);
                        }
                        Some(MaintenanceDeferral::Remote) => {
                            state.defer(now + floor);
                        }
                        None if error.kind() == std::io::ErrorKind::Unsupported => {
                            // The protected capability exchange found no
                            // mutually permitted improvement mechanism.
                            state.next_due = None;
                            state.budget_not_before = None;
                            state.wait_clues = Some(clues);
                        }
                        None if LocalNatBudgetError::is_local_limit(&error)
                            || !self.gate.allows_member_path_upgrade(&original)
                            || error.kind() == std::io::ErrorKind::Interrupted =>
                        {
                            state.defer(now + floor);
                        }
                        None => state.failed(now, floor),
                    }
                }
                Err(MaintenanceError::SessionClosed) => {
                    self.improvements.remove(&stable);
                }
                Err(MaintenanceError::Cancelled) => {
                    if !self.shutdown.is_cancelled() {
                        state.defer(now + floor);
                    }
                }
                Err(error) => {
                    status.cause = Some(format!("active improvement: {error:?}"));
                    state.failed(now, floor);
                }
            }
            return true;
        }
        false
    }
}
