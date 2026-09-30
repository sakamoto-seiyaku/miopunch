//! Legacy single-socket phase semantics shared by mode0/1/3 and later pools.
use crate::{
    nat_mapping::NodeProbeBudget,
    nat_selector::{Behavior, Role},
};
use iroh::endpoint::{UdpTraversalLease, UdpTraversalSender};
use std::{
    io,
    net::{IpAddr, SocketAddr},
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
        let total_budget_ms = behavior.send_delay_ms.max(peer_delay_ms) + 5000;
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
}

/// Raw traversal success remains provisional until original QUIC business succeeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Winner {
    pub remote: SocketAddr,
    pub response: bool,
    pub before_probe: bool,
}
#[derive(Debug, Default)]
pub struct ExecutionTrace {
    pub probes: usize,
    pub responses: usize,
    pub ttl_degraded: usize,
    pub targets: Vec<SocketAddr>,
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

async fn send(
    sender: &UdpTraversalSender,
    target: SocketAddr,
    packet: &[u8],
    ttl: u8,
    budget: &NodeProbeBudget,
    check: &(dyn Fn() -> io::Result<()> + Sync),
) -> io::Result<bool> {
    check()?;
    if !budget.spend_peer_datagram() {
        return Err(io::Error::other("NAT probe budget exhausted"));
    }
    let degraded = sender.send_checked(target, packet, ttl, check).await?;
    println!(
        "LEGACY_DATAGRAM target={target} ttl={ttl} response={} degraded={degraded}",
        packet[0] == 1
    );
    Ok(degraded)
}
async fn probe_round(
    plan: &PhasePlan,
    sender: &UdpTraversalSender,
    auth: &PacketAuth,
    budget: &NodeProbeBudget,
    trace: &mut ExecutionTrace,
    first: bool,
    check: &(dyn Fn() -> io::Result<()> + Sync),
) -> io::Result<()> {
    for target in &plan.explicit {
        trace.ttl_degraded += usize::from(
            send(
                sender,
                *target,
                &auth.request(),
                plan.behavior.ttl,
                budget,
                check,
            )
            .await?,
        );
        trace.probes += 1;
        trace.targets.push(*target);
    }
    if first {
        for target in plan.range_targets() {
            trace.ttl_degraded += usize::from(
                send(
                    sender,
                    target,
                    &auth.request(),
                    plan.behavior.ttl,
                    budget,
                    check,
                )
                .await?,
            );
            trace.probes += 1;
            trace.targets.push(target);
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }
    Ok(())
}
async fn receive(
    lease: &mut UdpTraversalLease,
    sender: &UdpTraversalSender,
    plan: &PhasePlan,
    auth: &PacketAuth,
    budget: &NodeProbeBudget,
    trace: &mut ExecutionTrace,
    check: &(dyn Fn() -> io::Result<()> + Sync),
) -> io::Result<Winner> {
    loop {
        let packet = lease
            .recv()
            .await
            .ok_or_else(|| io::Error::other("NAT round receive stopped"))?;
        if !auth.authenticate(&packet.payload) {
            continue;
        }
        let response = packet.payload[0] == 1;
        if !response {
            if plan.behavior.role == Role::Sender {
                continue;
            }
            let payload = auth.response(&packet.payload);
            for i in 0..3 {
                trace.ttl_degraded +=
                    usize::from(send(sender, packet.remote, &payload, 0, budget, check).await?);
                trace.responses += 1;
                if i < 2 {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }
        return Ok(Winner {
            remote: packet.remote,
            response,
            before_probe: false,
        });
    }
}

/// No detached work: dropping this future drops the receive/probe loops and
/// the caller's round lease is synchronously unregistered. The outer attempt
/// monitors its admission/network scope throughout every delay and burst.
pub async fn run(
    plan: &PhasePlan,
    lease: &mut UdpTraversalLease,
    auth: &PacketAuth,
    budget: &NodeProbeBudget,
    check: &(dyn Fn() -> io::Result<()> + Sync),
) -> io::Result<(Winner, ExecutionTrace)> {
    let sender = lease.sender();
    let mut probes = ExecutionTrace::default();
    let mut responses = ExecutionTrace::default();
    let mut recv = Box::pin(receive(
        lease,
        &sender,
        plan,
        auth,
        budget,
        &mut responses,
        check,
    ));
    if plan.behavior.send_delay_ms > 0 {
        tokio::select! {
            result=&mut recv => {
                let mut winner=result?; winner.before_probe=true;
                drop(recv);
                return Ok((winner,responses));
            }
            _=tokio::time::sleep(Duration::from_millis(plan.behavior.send_delay_ms.into()))=>{}
        }
    }
    // The old first burst completes before the main loop consumes a winner.
    // Reception and response work still starts before sending any packet.
    let mut first = Box::pin(probe_round(
        plan,
        &sender,
        auth,
        budget,
        &mut probes,
        true,
        check,
    ));
    let mut received = None;
    tokio::select! {
        result=&mut recv=> { received=Some(result?); (&mut first).await?; }
        result=&mut first=> { result?; }
    }
    drop(first);
    let winner = if let Some(winner) = received {
        winner
    } else {
        let probe = async {
            let mut ticker = tokio::time::interval_at(
                tokio::time::Instant::now() + Duration::from_millis(200),
                Duration::from_millis(200),
            );
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                probe_round(plan, &sender, auth, budget, &mut probes, false, check).await?;
            }
            #[allow(unreachable_code)]
            Ok::<(), io::Error>(())
        };
        tokio::select! { result=&mut recv=>result?, result=probe=>{ result?; unreachable!() } }
    };
    drop(recv);
    probes.responses = responses.responses;
    probes.ttl_degraded += responses.ttl_degraded;
    Ok((winner, probes))
}
