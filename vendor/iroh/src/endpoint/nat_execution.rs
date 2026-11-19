//! Shared frp phase execution on the original endpoint's physical UDP sockets.

use crate::endpoint::{
    LocalSocketId, UdpDatagramSendError, UdpTraversalLease, UdpTraversalSender,
    nat_plan::PhasePlan, nat_selector::Role,
};
use futures_util::{StreamExt, stream::FuturesUnordered};
use std::{
    collections::{HashMap, HashSet},
    io,
    net::SocketAddr,
    sync::Mutex,
    time::{Duration, Instant},
};
/// Raw traversal success remains provisional until original QUIC business succeeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Winner {
    /// Actual receiving socket.
    pub local_socket_id: LocalSocketId,
    /// Actual local destination.
    pub local_addr: SocketAddr,
    /// Actual packet source.
    pub remote: SocketAddr,
    /// The match was a correlated response.
    pub response: bool,
    /// The match preceded the local first probe.
    pub before_probe: bool,
}
/// Per-round socket and packet accounting.
#[derive(Debug, Default, Clone)]
pub struct ExecutionTrace {
    /// Requests submitted to the socket.
    pub probes: usize,
    /// Responses submitted to the socket.
    pub responses: usize,
    /// Sends with degraded TTL metadata.
    pub ttl_degraded: usize,
    /// Actual request destinations.
    pub targets: Vec<SocketAddr>,
    /// Actual request source bindings.
    pub local_sockets: Vec<LocalSocketId>,
    /// Random iterations performed.
    pub random_iterations: usize,
    /// Random draws including duplicates.
    pub random_draws: usize,
    /// Random iterations with no distinct port.
    pub random_skipped: usize,
    /// Extra socket binds attempted.
    pub listener_attempted: usize,
    /// Extra sockets bound.
    pub listener_bound: usize,
    /// Extra socket binds that failed.
    pub listener_failed: usize,
    /// Datagram submissions entered.
    pub send_attempts: usize,
    /// Datagram submissions rejected by I/O.
    pub send_errors: usize,
    /// Datagram submissions cancelled before returning.
    pub send_aborted: usize,
    /// Unauthenticated or uncorrelated receipts.
    pub rejected_receipts: usize,
}

/// One legacy random-send call's port uniqueness and ten-draw limit.
/// The draw receives the exact exclusive-upper-bound legacy interval.
#[derive(Debug, Default)]
pub struct RandomPortSampler {
    used: HashSet<u16>,
    /// Number of draws performed.
    pub draws: usize,
    /// Iterations for which all draws failed.
    pub skipped: usize,
}
impl RandomPortSampler {
    /// Draw one unique port within the old phase limit.
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
    /// Round-specific TLS exporter key.
    pub key: [u8; 32],
    /// Authenticated local member identity.
    pub local: [u8; 32],
    /// Authenticated remote member identity.
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

/// Counts entered submissions that stop before kernel success/error, including
/// cancellation while awaiting authorization or socket readiness.
struct PendingSend<'a> {
    trace: &'a Mutex<ExecutionTrace>,
    completed: bool,
}
impl Drop for PendingSend<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.trace.lock().expect("NAT execution trace").send_aborted += 1;
        }
    }
}

/// One authenticated round's I/O, receipt correlations and complete cost ledger.
struct PhaseSession<'a> {
    auth: &'a PacketAuth,
    spend: &'a (dyn Fn() -> bool + Sync),
    trace: &'a Mutex<ExecutionTrace>,
    check: &'a (dyn Fn() -> io::Result<()> + Sync),
    outstanding: Mutex<HashMap<[u8; 16], LocalSocketId>>,
    started: std::time::Instant,
    event: &'a (dyn Fn(PhaseEvent<'_>, Duration) + Sync),
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
        if !(self.spend)() {
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
                    .is_some_and(|cause| cause.is::<UdpDatagramSendError>()) =>
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
        let first = trace.probes == 0 && trace.responses == 0;
        trace.ttl_degraded += usize::from(degraded);
        if packet[0] == 1 {
            trace.responses += 1;
        } else {
            trace.probes += 1;
            trace.targets.push(target);
            trace.local_sockets.push(sender.local_socket_id());
        }
        if first {
            (self.event)(
                PhaseEvent::FirstSend {
                    response: packet[0] == 1,
                },
                self.started.elapsed(),
            );
        }
        println!(
            "LEGACY_DATAGRAM target={target} ttl={ttl} response={} degraded={degraded} local_socket_id={} elapsed_ms={}",
            packet[0] == 1,
            sender.local_socket_id().get(),
            self.started.elapsed().as_millis()
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
    let mut first = true;
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
                if std::mem::replace(&mut first, false) {
                    let at = Instant::now();
                    (session.event)(PhaseEvent::FirstMatch { winner: &winner, at }, at.duration_since(session.started));
                }
                println!("LEGACY_RECEIPT remote={} response={response} local_socket_id={} elapsed_ms={}", packet.remote, packet.local_socket_id.get(), session.started.elapsed().as_millis());
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

/// Boundaries shared by both consumers of the phase runner.
pub struct PhaseRun<'a> {
    /// Per-send permission and absolute deadline check.
    pub check: &'a (dyn Fn() -> io::Result<()> + Sync),
    /// Spend one datagram from the caller's reserved budget.
    pub spend: &'a (dyn Fn() -> bool + Sync),
    /// Both request and response submissions are recorded, even on failure.
    pub trace: &'a Mutex<ExecutionTrace>,
    /// The delayed receiver can hold its probes while an immediate sender probes.
    pub peer_sends_immediately: bool,
    /// Controlled fixture that disables the receiver's own probes.
    pub receiver_wait: bool,
    /// Optional first-send and first-match observation.
    pub event: &'a (dyn Fn(PhaseEvent<'_>, Duration) + Sync),
}

/// First packet milestones within one phase.
#[derive(Debug, Clone, Copy)]
pub enum PhaseEvent<'a> {
    /// First datagram actually submitted to the socket.
    FirstSend {
        /// Whether the sent datagram was a reply.
        response: bool,
    },
    /// First authenticated and correlated receipt.
    FirstMatch {
        /// The actual receiving binding and source of the matched packet.
        winner: &'a Winner,
        /// Local monotonic receive time.
        at: Instant,
    },
}

/// No work is detached from the lease, including range and random probes.
pub async fn run(
    plan: &PhasePlan,
    lease: &mut UdpTraversalLease,
    auth: &PacketAuth,
    run: PhaseRun<'_>,
) -> io::Result<Winner> {
    let session = PhaseSession {
        auth,
        spend: run.spend,
        trace: run.trace,
        check: run.check,
        outstanding: Mutex::new(HashMap::new()),
        started: std::time::Instant::now(),
        event: run.event,
    };
    let senders = lease
        .local_sockets()
        .iter()
        .map(|binding| lease.sender_for(binding.id))
        .collect::<io::Result<Vec<_>>>()?;
    let mut recv = Box::pin(receive(lease, &senders, plan, &session));
    // Explicit experiment on an isolated endpoint. The driver also checks all
    // UDP traffic in the capture, including the endpoint's native probes.
    if plan.behavior.role == Role::Receiver && run.receiver_wait {
        println!("LEGACY_EXPERIMENT receiver_wait=true");
        return recv.await;
    }
    // A filtering NAT still receives the scheduled probes after this wait.
    let send_delay_ms = if plan.behavior.role == Role::Receiver && run.peer_sends_immediately {
        plan.behavior.send_delay_ms.max(500)
    } else {
        plan.behavior.send_delay_ms
    };
    if send_delay_ms > 0 {
        tokio::select! {
            result = &mut recv => { let mut winner=result?; winner.before_probe=true; return Ok(winner); }
            _ = tokio::time::sleep(Duration::from_millis(send_delay_ms.into())) => {}
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
