//! Legacy adapters for the shared native UDP phase runner.
use crate::{
    connection_coordinator::connection_diagnostic::AttemptLog, nat_mapping::NodeProbeBudget,
};
use iroh::endpoint::{
    UdpTraversalLease,
    nat_execution::{self, PhaseEvent, PhaseRun},
};
use std::{
    io,
    sync::{Arc, Mutex},
};

pub use iroh::endpoint::{
    nat_execution::{ExecutionTrace, PacketAuth, RandomPortSampler, Winner},
    nat_plan::{DEFAULT_LEGACY_PEER_ALLOWANCE, PhasePlan},
};

type SharedTrace = Arc<Mutex<ExecutionTrace>>;

pub(crate) struct TraceSummary {
    pub trace: SharedTrace,
    pub round: String,
    pub connection_id: usize,
    pub log: AttemptLog,
}
impl Drop for TraceSummary {
    fn drop(&mut self) {
        let trace = self.trace.lock().expect("NAT execution trace");
        self.log.event(
            "punch-packets",
            "finished",
            serde_json::json!({
                "round":self.round, "probes":trace.probes, "responses":trace.responses,
                "sendErrors":trace.send_errors, "rejectedReceipts":trace.rejected_receipts,
            }),
        );
        println!(
            "LEGACY_TRACE round={} stable_id={} probes={} responses={} ttl_degraded={} random_iterations={} random_draws={} random_skipped={} listener_attempted={} listener_bound={} listener_failed={} send_attempts={} send_errors={} send_aborted={} rejected_receipts={}",
            self.round,
            self.connection_id,
            trace.probes,
            trace.responses,
            trace.ttl_degraded,
            trace.random_iterations,
            trace.random_draws,
            trace.random_skipped,
            trace.listener_attempted,
            trace.listener_bound,
            trace.listener_failed,
            trace.send_attempts,
            trace.send_errors,
            trace.send_aborted,
            trace.rejected_receipts
        );
    }
}

/// No detached work: dropping this future drops every physical receiver,
/// response burst, range loop and random loop. The caller owns all descriptors.
pub async fn run(
    plan: &PhasePlan,
    lease: &mut UdpTraversalLease,
    auth: &PacketAuth,
    budget: &NodeProbeBudget,
    check: &(dyn Fn() -> io::Result<()> + Sync),
) -> io::Result<(Winner, ExecutionTrace)> {
    let trace = Arc::new(Mutex::new(ExecutionTrace::default()));
    let winner = run_with_trace(plan, lease, auth, budget, &trace, check, false, None).await?;
    let trace = trace.lock().expect("NAT execution trace").clone();
    Ok((winner, trace))
}

/// The Attempt retains the ledger even on timeout, cancellation and scope failure.
pub(crate) async fn run_with_trace(
    plan: &PhasePlan,
    lease: &mut UdpTraversalLease,
    auth: &PacketAuth,
    budget: &NodeProbeBudget,
    trace: &SharedTrace,
    check: &(dyn Fn() -> io::Result<()> + Sync),
    peer_sends_immediately: bool,
    round_log: Option<AttemptLog>,
) -> io::Result<Winner> {
    let receiver_wait = cfg!(feature = "nat-lab-test-hooks")
        && std::env::var("MIOPUNCH_NAT_LAB_RECEIVER_WAIT").as_deref() == Ok("1");
    let event = |event: PhaseEvent<'_>, elapsed: std::time::Duration| {
        if let Some(log) = round_log {
            let (phase, outcome, response) = match event {
                PhaseEvent::FirstSend { response } => ("punch-first-send", "sent", response),
                PhaseEvent::FirstMatch { winner, .. } => {
                    ("punch-first-receipt", "matched", winner.response)
                }
            };
            log.event(
                phase,
                outcome,
                serde_json::json!({
                    "response": response, "elapsedMs": elapsed.as_millis(),
                }),
            );
        }
    };
    nat_execution::run(
        plan,
        lease,
        auth,
        PhaseRun {
            check,
            spend: &|| budget.spend_peer_datagram(),
            trace,
            peer_sends_immediately,
            receiver_wait,
            event: &event,
        },
    )
    .await
}
