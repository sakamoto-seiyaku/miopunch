//! Complete legacy UDP phase semantics on the original Endpoint's physical sockets.
use crate::{
    nat_mapping::NodeProbeBudget,
    nat_selector::{Behavior, Role},
};
use futures_util::{StreamExt, stream::FuturesUnordered};
use iroh::endpoint::{LocalSocketId, UdpTraversalLease, UdpTraversalSender};
use std::{
    collections::{HashMap, HashSet},
    io,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::Duration,
};

/// A radius is converted using the last sample; it is never a next-port guess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhasePlan {
    pub behavior: Behavior,
    pub explicit: Vec<SocketAddr>,
    pub candidates: Vec<SocketAddr>,
    pub range: Option<(u16, u16)>,
    pub total_budget_ms: u32,
    pub read_budget_ms: u32,
}
impl PhasePlan {
    pub fn new(
        behavior: Behavior,
        peer_delay_ms: u32,
        mapped: &[SocketAddr],
        assisted: &[SocketAddr],
        difference: u16,
    ) -> Self {
        let range = mapped
            .last()
            .filter(|_| behavior.ports_range > 0)
            .map(|last| {
                let p = i32::from(last.port());
                let d = i32::from(difference);
                let r = i32::from(behavior.ports_range);
                (
                    (p - d - 5).max(p - r).max(1) as u16,
                    (p + d + 5).min(p + r).min(65535) as u16,
                )
            });
        let mut candidates = mapped.to_vec();
        candidates.dedup();
        let mut explicit = match behavior.role {
            Role::Sender => assisted.iter().chain(mapped).copied().collect::<Vec<_>>(),
            Role::Receiver if range.is_none() => mapped.to_vec(),
            Role::Receiver => Vec::new(),
        };
        explicit.dedup();
        let random_phase = behavior.ports_random > 0 || behavior.listen_random > 0;
        // The old selector extends both partners' read timeout by 30s for
        // random sending/listening, on top of max delay + 5s.
        let total_budget_ms =
            behavior.send_delay_ms.max(peer_delay_ms) + 5000 + if random_phase { 30000 } else { 0 };
        Self {
            behavior,
            explicit,
            candidates,
            range,
            total_budget_ms,
            read_budget_ms: total_budget_ms - behavior.send_delay_ms,
        }
    }
    /// IP order and adjacent-only compaction retain legacy parseIPs semantics.
    pub fn range_targets(&self) -> Vec<SocketAddr> {
        let Some((from, to)) = self.range else {
            return Vec::new();
        };
        let mut ips = self
            .candidates
            .iter()
            .map(SocketAddr::ip)
            .collect::<Vec<IpAddr>>();
        ips.dedup();
        ips.into_iter()
            .flat_map(|ip| (from..=to).map(move |port| SocketAddr::new(ip, port)))
            .collect()
    }

    /// A conservative allowance for the complete phase, without shrinking any
    /// candidate, listener, repeat or random iteration. Wall time may stop it first.
    pub fn peer_datagram_bound(&self, physical_sockets: usize) -> usize {
        let repeats = 1 + (self.total_budget_ms as usize).div_ceil(200);
        let mut ips = self
            .candidates
            .iter()
            .map(SocketAddr::ip)
            .collect::<Vec<_>>();
        ips.dedup();
        physical_sockets
            * (self.explicit.len() * repeats
                + self.range_targets().len()
                + usize::from(self.behavior.ports_random) * ips.len()
                + 3)
    }
}

/// Maximum wire candidates/assistance are 64 each. This deliberately covers
/// the full 38s, 257 physical bindings and all literal legacy parameters.
/// It is an allowance, never a promise to transmit this many datagrams.
pub const DEFAULT_LEGACY_PEER_ALLOWANCE: usize = 257 * (128 * 191 + 64 * 21 + 1000 * 64 + 3);

/// Raw traversal success remains provisional until original QUIC business succeeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Winner {
    pub local_socket_id: LocalSocketId,
    pub local_addr: SocketAddr,
    pub remote: SocketAddr,
    pub response: bool,
    pub before_probe: bool,
}
#[derive(Debug, Default, Clone)]
pub struct ExecutionTrace {
    pub probes: usize,
    pub responses: usize,
    pub ttl_degraded: usize,
    pub targets: Vec<SocketAddr>,
    pub local_sockets: Vec<LocalSocketId>,
    pub random_iterations: usize,
    pub random_draws: usize,
    pub random_skipped: usize,
    pub listener_attempted: usize,
    pub listener_bound: usize,
    pub listener_failed: usize,
    pub send_attempts: usize,
    pub send_errors: usize,
    pub send_aborted: usize,
    pub rejected_receipts: usize,
}

/// One legacy random-send call's port uniqueness and ten-draw limit.
/// The draw receives the exact exclusive-upper-bound legacy interval.
#[derive(Debug, Default)]
pub struct RandomPortSampler {
    used: HashSet<u16>,
    pub draws: usize,
    pub skipped: usize,
}
impl RandomPortSampler {
    pub fn sample(&mut self, mut draw: impl FnMut(std::ops::Range<u16>) -> u16) -> Option<u16> {
        for _ in 0..10 {
            self.draws += 1;
            let port = draw(1024..65535);
            if (1024..65535).contains(&port) && self.used.insert(port) {
                return Some(port);
            }
        }
        self.skipped += 1;
        None
    }
}

/// Keys come from the original connection TLS exporter with this round context.
/// Sender identities prevent reflected local requests from claiming a winner.
#[derive(Clone)]
pub struct PacketAuth {
    pub key: [u8; 32],
    pub local: [u8; 32],
    pub remote: [u8; 32],
}
impl std::fmt::Debug for PacketAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PacketAuth(TLS exporter)")
    }
}
impl PacketAuth {
    fn tag(&self, body: &[u8], outgoing: bool) -> blake3::Hash {
        let mut h = blake3::Hasher::new_keyed(&self.key);
        if outgoing {
            h.update(&self.local);
            h.update(&self.remote);
        } else {
            h.update(&self.remote);
            h.update(&self.local);
        }
        h.update(body);
        h.finalize()
    }
    fn request(&self) -> Vec<u8> {
        let mut body = vec![0];
        body.extend_from_slice(&rand::random::<[u8; 16]>());
        let tag = self.tag(&body, true);
        body.extend_from_slice(tag.as_bytes());
        body
    }
    fn authenticate(&self, payload: &[u8]) -> bool {
        payload.len() == 49
            && payload[0] <= 1
            && self.tag(&payload[..17], false)
                == blake3::Hash::from_bytes(payload[17..].try_into().expect("MAC length"))
    }
    fn response(&self, request: &[u8]) -> Vec<u8> {
        let mut body = request[..17].to_vec();
        body[0] = 1;
        let tag = self.tag(&body, true);
        body.extend_from_slice(tag.as_bytes());
        body
    }
}

type SharedTrace = Arc<Mutex<ExecutionTrace>>;

/// Counts entered submissions that stop before kernel success/error, including
/// cancellation while awaiting authorization or socket readiness.
struct PendingSend<'a> {
    trace: &'a SharedTrace,
    completed: bool,
}
impl Drop for PendingSend<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.trace.lock().expect("NAT execution trace").send_aborted += 1;
        }
    }
}

pub(crate) struct TraceSummary {
    pub trace: SharedTrace,
    pub round: String,
    pub connection_id: usize,
}
impl Drop for TraceSummary {
    fn drop(&mut self) {
        let trace = self.trace.lock().expect("NAT execution trace");
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

/// One authenticated round's I/O, receipt correlations and complete cost ledger.
struct PhaseSession<'a> {
    auth: &'a PacketAuth,
    budget: &'a NodeProbeBudget,
    trace: &'a SharedTrace,
    check: &'a (dyn Fn() -> io::Result<()> + Sync),
    outstanding: Mutex<HashMap<[u8; 16], LocalSocketId>>,
}
impl PhaseSession<'_> {
    async fn send(
        &self,
        sender: &UdpTraversalSender,
        target: SocketAddr,
        packet: &[u8],
        ttl: u8,
    ) -> io::Result<()> {
        (self.check)()?;
        if !self.budget.spend_peer_datagram() {
            return Err(io::Error::other("NAT probe budget exhausted"));
        }
        if packet[0] == 0 {
            let nonce = packet[1..17].try_into().expect("request nonce");
            // Register before actual socket submission: a very fast response
            // must still match the physical source binding of this request.
            self.outstanding
                .lock()
                .expect("NAT receipts")
                .insert(nonce, sender.local_socket_id());
        }
        self.trace
            .lock()
            .expect("NAT execution trace")
            .send_attempts += 1;
        let mut pending = PendingSend {
            trace: self.trace,
            completed: false,
        };
        let degraded = match sender.send_checked(target, packet, ttl, self.check).await {
            Ok(degraded) => {
                pending.completed = true;
                degraded
            }
            Err(error)
                if error
                    .get_ref()
                    .is_some_and(|cause| cause.is::<iroh::endpoint::UdpDatagramSendError>()) =>
            {
                pending.completed = true;
                self.trace.lock().expect("NAT execution trace").send_errors += 1;
                println!(
                    "LEGACY_DATAGRAM_ERROR target={target} ttl={ttl} response={} local_socket_id={} reason={error}",
                    packet[0] == 1,
                    sender.local_socket_id().get()
                );
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let mut trace = self.trace.lock().expect("NAT execution trace");
        trace.ttl_degraded += usize::from(degraded);
        if packet[0] == 1 {
            trace.responses += 1;
        } else {
            trace.probes += 1;
            trace.targets.push(target);
            trace.local_sockets.push(sender.local_socket_id());
        }
        println!(
            "LEGACY_DATAGRAM target={target} ttl={ttl} response={} degraded={degraded} local_socket_id={}",
            packet[0] == 1,
            sender.local_socket_id().get()
        );
        Ok(())
    }
    fn reject(&self, reason: &str, id: LocalSocketId) {
        self.trace
            .lock()
            .expect("NAT execution trace")
            .rejected_receipts += 1;
        println!(
            "LEGACY_RECEIPT_REJECT reason={reason} local_socket_id={}",
            id.get()
        );
    }
    fn accepts_response(&self, payload: &[u8], id: LocalSocketId) -> bool {
        let nonce: [u8; 16] = payload[1..17].try_into().expect("authenticated nonce");
        let mut outstanding = self.outstanding.lock().expect("NAT receipts");
        if outstanding.get(&nonce) != Some(&id) {
            return false;
        }
        outstanding.remove(&nonce);
        true
    }
}

async fn probe_round(
    plan: &PhasePlan,
    senders: &[UdpTraversalSender],
    session: &PhaseSession<'_>,
    first: bool,
) -> io::Result<()> {
    // Old ordering: explicit target first, then every physical listener.
    for target in &plan.explicit {
        for sender in senders {
            session
                .send(sender, *target, &session.auth.request(), plan.behavior.ttl)
                .await?;
        }
    }
    if first {
        for sender in senders {
            for target in plan.range_targets() {
                session
                    .send(sender, target, &session.auth.request(), plan.behavior.ttl)
                    .await?;
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        }
    }
    Ok(())
}

async fn random_probes(
    plan: &PhasePlan,
    sender: &UdpTraversalSender,
    session: &PhaseSession<'_>,
) -> io::Result<()> {
    let mut ips = plan
        .candidates
        .iter()
        .map(SocketAddr::ip)
        .collect::<Vec<_>>();
    ips.dedup();
    let mut sampler = RandomPortSampler::default();
    for _ in 0..plan.behavior.ports_random {
        (session.check)()?;
        let before_draws = sampler.draws;
        let before_skipped = sampler.skipped;
        let port = sampler.sample(rand::random_range);
        {
            let mut trace = session.trace.lock().expect("NAT execution trace");
            trace.random_iterations += 1;
            trace.random_draws += sampler.draws - before_draws;
            trace.random_skipped += sampler.skipped - before_skipped;
        }
        if let Some(port) = port {
            for ip in &ips {
                session
                    .send(
                        sender,
                        SocketAddr::new(*ip, port),
                        &session.auth.request(),
                        plan.behavior.ttl,
                    )
                    .await?;
                tokio::time::sleep(Duration::from_millis(15)).await;
            }
        }
    }
    Ok(())
}

async fn receive(
    lease: &mut UdpTraversalLease,
    senders: &[UdpTraversalSender],
    plan: &PhasePlan,
    session: &PhaseSession<'_>,
) -> io::Result<Winner> {
    let mut handled = HashSet::new();
    let mut replies = FuturesUnordered::new();
    loop {
        tokio::select! {
            packet = lease.recv() => {
                let packet = packet.ok_or_else(|| io::Error::other("NAT round receive stopped"))?;
                if !session.auth.authenticate(&packet.payload) { session.reject("authentication", packet.local_socket_id); continue; }
                if handled.contains(&packet.local_socket_id) { continue; }
                let response = packet.payload[0] == 1;
                if !response && plan.behavior.role == Role::Sender { continue; }
                if response && !session.accepts_response(&packet.payload, packet.local_socket_id) { session.reject("nonce_socket", packet.local_socket_id); continue; }
                (session.check)()?;
                let winner = Winner { local_socket_id: packet.local_socket_id, local_addr: packet.local_addr, remote: packet.remote, response, before_probe: false };
                if response { return Ok(winner); }
                let Some(sender) = senders.iter().find(|s| s.local_socket_id() == packet.local_socket_id) else { continue; };
                let sender = sender.with_source_ip(packet.local_addr.ip())?;
                handled.insert(packet.local_socket_id);
                let payload = session.auth.response(&packet.payload);
                // Independent reply futures preserve first-completed listener
                // behavior and are dropped synchronously with the first winner.
                replies.push(async move {
                    for i in 0..3 {
                        session.send(&sender, packet.remote, &payload, 0).await?;
                        if i < 2 { tokio::time::sleep(Duration::from_millis(50)).await; }
                    }
                    Ok::<Winner, io::Error>(winner)
                });
            }
            result = replies.next(), if !replies.is_empty() => return result.expect("active receiver reply"),
        }
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
    let winner = run_with_trace(plan, lease, auth, budget, &trace, check).await?;
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
) -> io::Result<Winner> {
    let session = PhaseSession {
        auth,
        budget,
        trace,
        check,
        outstanding: Mutex::new(HashMap::new()),
    };
    let senders = lease
        .local_sockets()
        .iter()
        .map(|binding| lease.sender_for(binding.id))
        .collect::<io::Result<Vec<_>>>()?;
    let mut recv = Box::pin(receive(lease, &senders, plan, &session));
    if plan.behavior.send_delay_ms > 0 {
        tokio::select! {
            result = &mut recv => { let mut winner=result?; winner.before_probe=true; return Ok(winner); }
            _ = tokio::time::sleep(Duration::from_millis(plan.behavior.send_delay_ms.into())) => {}
        }
    }
    let mut first = Box::pin(probe_round(plan, &senders, &session, true));
    let mut received = None;
    tokio::select! {
        result = &mut recv => { received=Some(result?); (&mut first).await?; }
        result = &mut first => { result?; }
    }
    drop(first);
    let mut random = FuturesUnordered::new();
    if plan.behavior.ports_random > 0 {
        for sender in &senders {
            random.push(random_probes(plan, sender, &session));
        }
    }
    if let Some(winner) = received {
        return Ok(winner);
    }
    let probe = async {
        let mut ticker = tokio::time::interval_at(
            tokio::time::Instant::now() + Duration::from_millis(200),
            Duration::from_millis(200),
        );
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            probe_round(plan, &senders, &session, false).await?;
        }
        #[allow(unreachable_code)]
        Ok::<(), io::Error>(())
    };
    tokio::pin!(probe);
    loop {
        tokio::select! {
            result = &mut recv => return result,
            result = &mut probe => { result?; unreachable!() }
            result = random.next(), if !random.is_empty() => { result.expect("active random probe")?; }
        }
    }
}
