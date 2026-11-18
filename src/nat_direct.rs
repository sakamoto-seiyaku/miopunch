//! Protected, bounded direct UDP candidates before any punching recommendation.
use crate::{
    admission_gate::AdmissionGate, connection_coordinator::control_error,
    control_session::ControlSession, nat_selector::LearningScope,
};
use iroh::endpoint::{
    ControlExchange, ControlMessageFormat, ObservationValidity, PathId, ReadToEndError,
    UdpCandidatePath,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io,
    net::{IpAddr, SocketAddr},
    time::Duration,
};
use tokio::time::Instant;

const LIMIT: usize = 16 * 1024;
const MAX_ADDRESSES: usize = 64;
const CANDIDATE_WINDOW: Duration = Duration::from_secs(2);
const PATH_RETRY: Duration = Duration::from_millis(50);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum Frame {
    Hello {
        network: [u8; 32],
        generation: u64,
    },
    Offer {
        nonce: [u8; 16],
        addresses: Vec<SocketAddr>,
    },
    Ready {
        nonce: [u8; 16],
    },
    Result {
        nonce: [u8; 16],
        available: bool,
    },
}

async fn send(exchange: &ControlExchange, frame: Frame) -> io::Result<()> {
    let bytes = serde_json::to_vec(&frame).map_err(io::Error::other)?;
    exchange
        .send_message(
            ControlMessageFormat::EndTerminated { maximum: LIMIT },
            &bytes,
        )
        .await
}
async fn receive(exchange: &ControlExchange) -> io::Result<Frame> {
    let bytes = exchange
        .recv_message(ControlMessageFormat::EndTerminated { maximum: LIMIT })
        .await
        .map_err(|error| {
            if error.kind() == io::ErrorKind::InvalidData
                || matches!(
                    error.get_ref().and_then(|source| source.downcast_ref()),
                    Some(ReadToEndError::TooLong)
                )
            {
                control_error("InvalidControl")
            } else {
                error
            }
        })?;
    serde_json::from_slice(&bytes).map_err(|_| control_error("InvalidControl"))
}

async fn receive_result(exchange: &ControlExchange, nonce: [u8; 16]) -> io::Result<bool> {
    match receive(exchange).await? {
        Frame::Result {
            nonce: result,
            available,
        } if result == nonce => Ok(available),
        _ => Err(control_error("InvalidControl")),
    }
}

fn validate(addresses: &[SocketAddr]) -> io::Result<()> {
    if addresses.len() > MAX_ADDRESSES || addresses.iter().any(|address| !valid(*address)) {
        return Err(control_error("InvalidControl"));
    }
    Ok(())
}

fn valid(address: SocketAddr) -> bool {
    address.port() != 0 && !address.ip().is_unspecified() && !address.ip().is_multicast()
}

#[derive(Debug, Default)]
pub(crate) struct CandidateProgress {
    advertised: CandidateCursor,
    received: CandidateCursor,
}

#[derive(Debug, Default)]
struct CandidateCursor {
    first: usize,
    offsets: [usize; 3],
}

impl CandidateCursor {
    fn next_round(
        &mut self,
        addresses: Vec<SocketAddr>,
    ) -> impl Iterator<Item = (usize, SocketAddr)> + use<> {
        let mut groups: [Vec<_>; 3] = Default::default();
        for address in addresses.into_iter().collect::<BTreeSet<_>>() {
            let group = match address.ip() {
                IpAddr::V4(ip) if ip.is_private() || ip.is_loopback() || ip.is_link_local() => 0,
                IpAddr::V6(_) => 1,
                IpAddr::V4(_) => 2,
            };
            groups[group].push(address);
        }
        for (group, offset) in groups.iter_mut().zip(self.offsets) {
            let count = group.len();
            if count > 0 {
                group.rotate_left(offset % count);
            }
        }
        let mut next = self.first;
        self.first = (self.first + 1) % groups.len();
        let mut groups = groups.map(Vec::into_iter);
        std::iter::from_fn(move || {
            for _ in 0..groups.len() {
                let current = next;
                next = (next + 1) % groups.len();
                if let Some(address) = groups[current].next() {
                    return Some((current, address));
                }
            }
            None
        })
    }
}

/// Captured validity remains local. A peer cannot replace its network epoch.
#[derive(Debug)]
pub(crate) struct DirectPath {
    pub remote: SocketAddr,
    pub scope: LearningScope,
    pub generation: ObservationValidity,
    pub connection_id: usize,
    pub id: PathId,
    pub(crate) candidate: Option<UdpCandidatePath>,
}

impl DirectPath {
    pub fn check(&self, control: &ControlSession, gate: &AdmissionGate) -> io::Result<()> {
        if control.stable_id() != self.connection_id
            || !self.generation.is_current()
            || control.nat_learning_scope(gate).as_ref() != Some(&self.scope)
            || control.is_closed()
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "direct path scope changed",
            ));
        }
        Ok(())
    }

    pub fn retain(&mut self) {
        if let Some(path) = self.candidate.take() {
            path.retain();
        }
    }
}

fn ip_path(control: &ControlSession) -> Option<(PathId, SocketAddr)> {
    control
        .endpoint()
        .paths()
        .iter()
        .find_map(|path| match path.remote_addr() {
            iroh_base::TransportAddr::Ip(address) => Some((path.id(), *address)),
            _ => None,
        })
}

fn session_ended() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "direct session ended")
}

async fn probe_candidates(
    control: &ControlSession,
    gate: &AdmissionGate,
    remote: Vec<SocketAddr>,
    deadline: Instant,
    check: impl Fn() -> io::Result<()>,
) -> io::Result<Option<UdpCandidatePath>> {
    if remote.is_empty() || ip_path(control).is_some() {
        return Ok(None);
    }
    let deadline = deadline.min(Instant::now() + CANDIDATE_WINDOW);
    let candidate_order = gate
        .with_direct_candidates(control.endpoint(), |progress| {
            progress.received.next_round(remote)
        })
        .ok_or_else(session_ended)?;
    let mut owned = Vec::new();
    let operation = async {
        for (group, address) in candidate_order {
            println!(
                "NAT_DIRECT_CANDIDATE stable_id={} target={address}",
                control.stable_id()
            );
            loop {
                check()?;
                if ip_path(control).is_some() {
                    return Ok(());
                }
                match control.endpoint().open_udp_candidate(address).await {
                    Ok(path) => owned.extend(path),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        // No address judgment until a usable ID and CID exist.
                        tokio::time::sleep(PATH_RETRY).await;
                        continue;
                    }
                    Err(_) => {}
                }
                gate.with_direct_candidates(control.endpoint(), |progress| {
                    progress.received.offsets[group] += 1;
                })
                .ok_or_else(session_ended)?;
                break;
            }
            tokio::time::sleep(PATH_RETRY).await;
        }
        while ip_path(control).is_none() {
            check()?;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok::<_, io::Error>(())
    };
    if let Ok(result) = tokio::time::timeout_at(deadline, operation).await {
        result?;
    }
    check()?;
    let winner = ip_path(control).map(|(id, _)| id);
    // Drop the round's other allocations before entering enhanced punching.
    Ok(owned.into_iter().find(|path| Some(path.id()) == winner))
}

pub(crate) async fn attempt(
    control: &ControlSession,
    gate: &AdmissionGate,
    deadline: Instant,
) -> io::Result<Option<DirectPath>> {
    let scope = control.nat_learning_scope(gate).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "direct member proof required",
        )
    })?;
    let generation = control.endpoint().udp_network_generation();
    let connection_id = control.stable_id();
    let check = || {
        if !generation.is_current()
            || control.nat_learning_scope(gate).as_ref() != Some(&scope)
            || control.is_closed()
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "direct path scope changed",
            ));
        }
        Ok(())
    };
    let local = control
        .endpoint()
        .addr()
        .ip_addrs()
        .copied()
        .filter(|address| valid(*address))
        .collect::<Vec<_>>();
    let available_addresses = local.len();
    let addresses = gate
        .with_direct_candidates(control.endpoint(), |progress| {
            let cursor = &mut progress.advertised;
            cursor
                .next_round(local)
                .take(MAX_ADDRESSES)
                .map(|(group, address)| {
                    cursor.offsets[group] += 1;
                    address
                })
                .collect::<Vec<_>>()
        })
        .ok_or_else(session_ended)?;
    println!(
        "NAT_DIRECT_LOCAL stable_id={} available={} advertised={addresses:?}",
        control.stable_id(),
        available_addresses
    );
    let operation = async {
        check()?;
        let (nonce, remote) = if control.is_dialing_side() {
            let exchange = control.open_control_exchange().await?;
            send(
                &exchange,
                Frame::Hello {
                    network: scope.network,
                    generation: scope.secret_generation,
                },
            )
            .await?;
            let Frame::Offer {
                nonce,
                addresses: remote,
            } = receive(&exchange).await?
            else {
                return Err(control_error("InvalidControl"));
            };
            validate(&remote)?;
            check()?;
            let exchange = control.open_control_exchange().await?;
            send(&exchange, Frame::Offer { nonce, addresses }).await?;
            if !matches!(receive(&exchange).await?, Frame::Ready { nonce: ack } if ack == nonce) {
                return Err(control_error("InvalidControl"));
            }
            (nonce, remote)
        } else {
            let exchange = control.accept_control_exchange().await?;
            if !matches!(receive(&exchange).await?, Frame::Hello { network, generation }
                if network == scope.network && generation == scope.secret_generation)
            {
                return Err(control_error("InvalidControl"));
            }
            let nonce = rand::random();
            check()?;
            send(&exchange, Frame::Offer { nonce, addresses }).await?;
            let exchange = control.accept_control_exchange().await?;
            let Frame::Offer {
                nonce: offered,
                addresses: remote,
            } = receive(&exchange).await?
            else {
                return Err(control_error("InvalidControl"));
            };
            if offered != nonce {
                return Err(control_error("InvalidControl"));
            }
            validate(&remote)?;
            check()?;
            send(&exchange, Frame::Ready { nonce }).await?;
            (nonce, remote)
        };
        println!(
            "NAT_DIRECT_PEER stable_id={} received={remote:?}",
            control.stable_id()
        );
        let candidate = if control.is_dialing_side() {
            let candidate = probe_candidates(control, gate, remote, deadline, check).await?;
            check()?;
            let exchange = control.open_control_exchange().await?;
            send(
                &exchange,
                Frame::Result {
                    nonce,
                    available: ip_path(control).is_some(),
                },
            )
            .await?;
            if !receive_result(&exchange, nonce).await? {
                return Ok(None);
            }
            candidate
        } else {
            let exchange = control.accept_control_exchange().await?;
            let available = receive_result(&exchange, nonce).await?;
            check()?;
            let available = available && ip_path(control).is_some();
            send(&exchange, Frame::Result { nonce, available }).await?;
            if !available {
                return Ok(None);
            }
            None
        };
        check()?;
        let (id, remote) = ip_path(control).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotConnected,
                "negotiated direct UDP path disappeared",
            )
        })?;
        Ok(Some(DirectPath {
            remote,
            scope: scope.clone(),
            generation: generation.clone(),
            connection_id,
            id,
            candidate: candidate.filter(|path| path.id() == id),
        }))
    };
    let phase = control.diagnostic().phase(
        "direct-candidates",
        serde_json::json!({
            "budgetMs":deadline.saturating_duration_since(Instant::now()).as_millis() as u64,
            "candidateBudgetMs":CANDIDATE_WINDOW.as_millis() as u64,
        }),
    );
    let result = tokio::select! {
        biased;
        _ = tokio::time::sleep_until(deadline) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "unified caller deadline exceeded",
        )),
        result = operation => result,
    };
    phase.finish(match &result {
        Ok(Some(_)) => "succeeded",
        Ok(None) => "unavailable",
        Err(_) => "failed",
    });
    result
}
