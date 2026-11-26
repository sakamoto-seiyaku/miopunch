//! Bounded v1 messages on the dedicated member UDP planning stream.

use std::{
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};

use crate::endpoint::{
    nat_plan::{MAX_OBSERVATIONS, SnapshotWire},
    nat_selector::{BehaviorId, LearningScope},
};

use super::invalid;

const HELLO: u8 = 1;
pub(super) const CHALLENGE: u8 = 2;
const OFFER: u8 = 3;
const PLAN: u8 = 4;
const ACCEPT: u8 = 5;
pub(super) const COMMITTED: u8 = 6;
const ERROR: u8 = 7;
pub(super) const LISTENING: u8 = 8;
pub(super) const WINNER_READY: u8 = 9;
pub(super) const VALIDATED: u8 = 10;
pub(super) const BUSINESS_READY: u8 = 11;
pub(super) const BUSINESS_CONFIRMED: u8 = 12;
pub(super) const OBSERVE: u8 = 13;
pub(super) const SELECTED: u8 = 14;

pub(super) struct Hello {
    network: [u8; 32],
    generation: u64,
    pub budget: Duration,
    pub family: iroh_relay::ObservationFamily,
}

impl Hello {
    pub(super) fn new(
        scope: &LearningScope,
        budget: Duration,
        family: iroh_relay::ObservationFamily,
    ) -> io::Result<Self> {
        let budget = Duration::from_millis(u64::from(millis(budget)?));
        Ok(Self {
            network: scope.network,
            generation: scope.secret_generation,
            budget,
            family,
        })
    }

    pub(super) fn encode(&self) -> Vec<u8> {
        let mut bytes = vec![HELLO];
        bytes.extend(self.network);
        bytes.extend(self.generation.to_be_bytes());
        bytes.extend((self.budget.as_millis() as u32).to_be_bytes());
        if self.family == iroh_relay::ObservationFamily::Ipv6 {
            bytes.push(6);
        }
        bytes
    }

    pub(super) fn decode(mut bytes: &[u8]) -> io::Result<Self> {
        if take(&mut bytes)? != [HELLO] {
            return Err(invalid("UDP planning hello required"));
        }
        let network = take(&mut bytes)?;
        let generation = u64::from_be_bytes(take(&mut bytes)?);
        let budget = duration(&mut bytes)?;
        let family = match bytes {
            [] => iroh_relay::ObservationFamily::Ipv4,
            [6] => {
                bytes = &[];
                iroh_relay::ObservationFamily::Ipv6
            }
            _ => return Err(invalid("invalid UDP address family")),
        };
        end(bytes)?;
        Ok(Self {
            network,
            generation,
            budget,
            family,
        })
    }

    pub(super) fn check(&self, scope: &LearningScope) -> io::Result<()> {
        if self.network != scope.network || self.generation != scope.secret_generation {
            return Err(super::denied(
                "UDP planning hello has a different membership scope",
            ));
        }
        Ok(())
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub(super) struct Round(pub [u8; 16]);

pub(super) struct Proposal {
    pub evidence: SnapshotWire,
    pub id: BehaviorId,
    pub packet_limits: [usize; 2],
    pub budget: Duration,
}

impl Round {
    pub(super) fn message(self, kind: u8) -> Vec<u8> {
        let mut bytes = vec![kind];
        bytes.extend(self.0);
        bytes
    }

    pub(super) fn decode(mut bytes: &[u8], kind: u8) -> io::Result<Self> {
        let round = read_round(&mut bytes, kind)?;
        end(bytes)?;
        Ok(round)
    }

    fn body<'a>(self, mut bytes: &'a [u8], kind: u8) -> io::Result<&'a [u8]> {
        if read_round(&mut bytes, kind)? != self {
            return Err(invalid("UDP plan belongs to a different round"));
        }
        Ok(bytes)
    }

    pub(super) fn business(self, kind: u8, stream: noq::StreamId) -> Vec<u8> {
        let mut bytes = self.message(kind);
        bytes.extend(u64::from(stream).to_be_bytes());
        bytes
    }

    pub(super) fn check(self, bytes: &[u8], kind: u8) -> io::Result<()> {
        end(self.body(bytes, kind)?)
    }

    pub(super) fn offer(self, snapshot: &SnapshotWire) -> Vec<u8> {
        let mut bytes = self.message(OFFER);
        write_evidence(&mut bytes, snapshot);
        bytes
    }

    pub(super) fn evidence(self, bytes: &[u8]) -> io::Result<SnapshotWire> {
        let mut body = self.body(bytes, OFFER)?;
        let snapshot = read_evidence(&mut body)?;
        end(body)?;
        Ok(snapshot)
    }

    pub(super) fn plan(
        self,
        snapshot: &SnapshotWire,
        id: BehaviorId,
        packets: [usize; 2],
        budget: Duration,
    ) -> io::Result<Vec<u8>> {
        let mut bytes = self.message(PLAN);
        bytes.extend([id.mode, id.index]);
        for count in packets {
            bytes.extend(
                u32::try_from(count)
                    .map_err(io::Error::other)?
                    .to_be_bytes(),
            );
        }
        bytes.extend(millis(budget)?.to_be_bytes());
        write_evidence(&mut bytes, snapshot);
        Ok(bytes)
    }

    pub(super) fn proposal(self, bytes: &[u8]) -> io::Result<Proposal> {
        let mut body = self.body(bytes, PLAN)?;
        let [mode, index] = take(&mut body)?;
        let c = u32::from_be_bytes(take(&mut body)?) as usize;
        let v = u32::from_be_bytes(take(&mut body)?) as usize;
        let budget = duration(&mut body)?;
        let evidence = read_evidence(&mut body)?;
        end(body)?;
        Ok(Proposal {
            evidence,
            id: BehaviorId { mode, index },
            packet_limits: [c, v],
            budget,
        })
    }

    pub(super) fn accept(self, budget: Duration) -> io::Result<Vec<u8>> {
        let mut bytes = self.message(ACCEPT);
        bytes.extend(millis(budget)?.to_be_bytes());
        Ok(bytes)
    }

    pub(super) fn accepted(self, bytes: &[u8]) -> io::Result<Duration> {
        let mut body = self.body(bytes, ACCEPT)?;
        let accepted = duration(&mut body)?;
        end(body)?;
        Ok(accepted)
    }
}

fn read_round(bytes: &mut &[u8], kind: u8) -> io::Result<Round> {
    if take(bytes)? != [kind] {
        return Err(invalid("unexpected UDP planning message"));
    }
    Ok(Round(take(bytes)?))
}

fn write_evidence(bytes: &mut Vec<u8>, snapshot: &SnapshotWire) {
    let ipv6 = snapshot
        .mapped
        .iter()
        .chain(&snapshot.assisted)
        .any(SocketAddr::is_ipv6)
        || snapshot.local_ips.iter().any(IpAddr::is_ipv6);
    if ipv6 {
        bytes.push(255);
    }
    fn ip(bytes: &mut Vec<u8>, address: IpAddr) {
        match address {
            IpAddr::V4(address) => bytes.extend(address.octets()),
            IpAddr::V6(address) => bytes.extend(address.octets()),
        }
    }
    fn addresses(bytes: &mut Vec<u8>, addresses: &[SocketAddr]) {
        bytes.push(addresses.len() as u8);
        for address in addresses {
            ip(bytes, address.ip());
            bytes.extend(address.port().to_be_bytes());
        }
    }
    addresses(bytes, &snapshot.mapped);
    bytes.push(snapshot.local_ips.len() as u8);
    for address in &snapshot.local_ips {
        ip(bytes, *address);
    }
    addresses(bytes, &snapshot.assisted);
}

fn read_evidence(bytes: &mut &[u8]) -> io::Result<SnapshotWire> {
    let ipv6 = bytes.first() == Some(&255);
    if ipv6 {
        *bytes = &bytes[1..];
    }
    fn ip(bytes: &mut &[u8], ipv6: bool) -> io::Result<IpAddr> {
        Ok(if ipv6 {
            IpAddr::V6(Ipv6Addr::from(take(bytes)?))
        } else {
            IpAddr::V4(Ipv4Addr::from(take(bytes)?))
        })
    }
    let address = |bytes: &mut &[u8]| {
        let ip = ip(bytes, ipv6)?;
        let port = u16::from_be_bytes(take(bytes)?);
        Ok(SocketAddr::new(ip, port))
    };
    let evidence = SnapshotWire {
        mapped: list(bytes, address)?,
        local_ips: list(bytes, |bytes| ip(bytes, ipv6))?,
        assisted: list(bytes, address)?,
    };
    evidence.validate()?;
    Ok(evidence)
}

fn list<T>(
    bytes: &mut &[u8],
    mut read: impl FnMut(&mut &[u8]) -> io::Result<T>,
) -> io::Result<Vec<T>> {
    let [length] = take(bytes)?;
    if usize::from(length) > MAX_OBSERVATIONS {
        return Err(invalid("oversized UDP observation list"));
    }
    (0..length).map(|_| read(bytes)).collect()
}

fn millis(duration: Duration) -> io::Result<u32> {
    let ms = u32::try_from(duration.as_millis()).map_err(io::Error::other)?;
    if ms == 0 {
        return Err(io::ErrorKind::TimedOut.into());
    }
    Ok(ms)
}

fn duration(bytes: &mut &[u8]) -> io::Result<Duration> {
    let ms = u32::from_be_bytes(take(bytes)?);
    if ms == 0 {
        return Err(invalid("empty UDP round budget"));
    }
    Ok(Duration::from_millis(u64::from(ms)))
}

fn take<const N: usize>(bytes: &mut &[u8]) -> io::Result<[u8; N]> {
    let (value, rest) = bytes
        .split_first_chunk()
        .ok_or_else(|| invalid("truncated UDP planning message"))?;
    *bytes = rest;
    Ok(*value)
}

fn end(bytes: &[u8]) -> io::Result<()> {
    if !bytes.is_empty() {
        return Err(invalid("trailing bytes in UDP planning message"));
    }
    Ok(())
}

pub(super) fn error(kind: io::ErrorKind) -> [u8; 2] {
    let code = match kind {
        io::ErrorKind::PermissionDenied => 1,
        io::ErrorKind::Interrupted => 2,
        io::ErrorKind::TimedOut => 3,
        io::ErrorKind::WouldBlock => 4,
        io::ErrorKind::InvalidData => 5,
        _ => 0,
    };
    [ERROR, code]
}

pub(super) fn check_error(bytes: &[u8]) -> io::Result<()> {
    if let [ERROR, code] = bytes {
        let kind = match code {
            1 => io::ErrorKind::PermissionDenied,
            2 => io::ErrorKind::Interrupted,
            3 => io::ErrorKind::TimedOut,
            4 => io::ErrorKind::WouldBlock,
            5 => io::ErrorKind::InvalidData,
            _ => io::ErrorKind::ConnectionAborted,
        };
        return Err(io::Error::new(kind, "peer declined the enhanced UDP round"));
    }
    Ok(())
}
