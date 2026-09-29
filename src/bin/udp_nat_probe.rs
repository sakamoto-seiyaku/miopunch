//! MIOPUNCH-27: cross-process MQTT bootstrap and real dual-NAT UDP path probe.
use std::{
    error::Error,
    io,
    net::{Ipv4Addr, SocketAddr},
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use iroh::{
    Endpoint, EndpointAddr, RelayConfig, RelayMap, Watcher as _,
    endpoint::{UdpObservation, presets},
};
use iroh_base::{PublicKey, SecretKey, TransportAddr};
use iroh_relay::{RelayQuicConfig, tls::CaTlsConfig};
use miopunch_native_probe::{
    admission_gate::{AdmissionGate, ProtocolScope},
    admission_proof::{AdmissionScope, AdmissionSecret},
    control_session::{CommitError, ControlSession},
    mqtt_transport::{MqttTransport, Traffic},
    nat_mapping::{self, MappingPattern, MappingSample, NodeProbeBudget, PunchRole},
};

const ALPN: &[u8] = b"miopunch/nat-probe/1";
const WAIT: Duration = Duration::from_secs(12);
const COORD_WAIT: Duration = Duration::from_secs(3);
const AUTO_PATH_WAIT: Duration = Duration::from_secs(3);
const NAT3_PROBES: usize = 8;
const NAT3_PROBE_GAP: Duration = Duration::from_millis(75);
const NAT3_NETWORK: [u8; 32] = [56; 32];

struct MappingEvidence {
    sequence: iroh::endpoint::UdpMappingSequence,
    pattern: MappingPattern,
    budget: Arc<NodeProbeBudget>,
    peer_probe_limit: usize,
}

fn mapping_samples(sequence: &iroh::endpoint::UdpMappingSequence) -> Vec<MappingSample> {
    sequence
        .samples
        .iter()
        .map(|sample| MappingSample {
            observer: sample.observer.clone(),
            destination: sample.destination,
            mapped: sample.mapped,
            observed_at: sample.observed_at.into(),
        })
        .collect()
}

impl MappingEvidence {
    fn fresh_pattern(&self) -> Result<MappingPattern, Box<dyn Error>> {
        if !self.sequence.is_fresh() {
            return Err("ordered QAD mapping evidence expired".into());
        }
        let pattern = nat_mapping::classify(&mapping_samples(&self.sequence), Instant::now());
        if pattern != self.pattern || matches!(pattern, MappingPattern::Unknown) {
            return Err("ordered QAD mapping evidence unknown or changed".into());
        }
        Ok(pattern)
    }
}

async fn mapping_evidence(
    endpoint: &Endpoint,
    observers: &RelayMap,
    budget: Arc<NodeProbeBudget>,
    peer_probe_limit: usize,
) -> Result<MappingEvidence, Box<dyn Error>> {
    if !budget.reserve_observations(4) {
        return Err("node maintenance budget exhausted before extra QAD observations".into());
    }
    let sequence = endpoint.observe_udp_mapping_sequence(observers).await?;
    let first = sequence
        .samples
        .first()
        .ok_or("empty mapping sequence")?
        .observed_at;
    for (index, sample) in sequence.samples.iter().enumerate() {
        println!(
            "MAPPING_SAMPLE order={index} observer={} destination={} mapped={} t_ms={} age_ms={}",
            sample.observer,
            sample.destination,
            sample.mapped,
            sample.observed_at.duration_since(first).as_millis(),
            sample.observed_at.elapsed().as_millis()
        );
    }
    let pattern = nat_mapping::classify(&mapping_samples(&sequence), Instant::now());
    println!(
        "MAPPING_CLASS pattern={pattern:?} confidence=consistent_{}/3_transitions fresh={}",
        if matches!(pattern, MappingPattern::Unknown) {
            0
        } else {
            3
        },
        sequence.is_fresh()
    );
    Ok(MappingEvidence {
        sequence,
        pattern,
        budget,
        peer_probe_limit,
    })
}

fn auto_probe_limit() -> Result<usize, Box<dyn Error>> {
    match std::env::var("MIOPUNCH_NAT_LAB_PROBES").as_deref() {
        Ok("1") | Err(_) => Ok(1),
        Ok("2") => Ok(2),
        _ => Err("lab probe count must be 1 or 2".into()),
    }
}

struct ObservedCandidate {
    addr: SocketAddr,
    observation: UdpObservation,
}

impl ObservedCandidate {
    fn fresh_addr(&self) -> Result<SocketAddr, Box<dyn Error>> {
        if self.observation.is_fresh() {
            Ok(self.addr)
        } else {
            Err("observed UDP candidate is stale".into())
        }
    }
}

fn load_or_create_key(path: &Path) -> io::Result<SecretKey> {
    if !path.exists() {
        let key = SecretKey::generate();
        std::fs::write(path, key.to_bytes())?;
    }
    let bytes: [u8; 32] = std::fs::read(path)?
        .try_into()
        .map_err(|_| io::Error::other("bad key length"))?;
    Ok(SecretKey::from_bytes(&bytes))
}

async fn endpoint(
    key: SecretKey,
    peer: PublicKey,
    broker: SocketAddr,
    observer: &str,
    observer_port: u16,
    cert: &Path,
) -> Result<
    (
        Endpoint,
        Arc<MqttTransport>,
        Arc<Traffic>,
        Option<AdmissionGate>,
        Option<RelayMap>,
    ),
    Box<dyn Error>,
> {
    let traffic = Arc::new(Traffic::default());
    let carrier = Arc::new(MqttTransport::new(broker, traffic.clone()));
    let roots = CaTlsConfig::custom_roots(vec![std::fs::read(cert)?.into()]);
    let url = format!("https://{observer}/").parse()?;
    let mapping_observers = if let Some(config) = std::env::var("MIOPUNCH_NAT_OBSERVERS")
        .ok()
        .filter(|value| !value.is_empty())
    {
        let entries = config
            .split(',')
            .map(|entry| {
                let (host, port) = entry.rsplit_once(':').ok_or("invalid QAD observer entry")?;
                let url = format!("https://{host}/").parse()?;
                let port = port.parse()?;
                Ok::<_, Box<dyn Error>>(RelayConfig::new(url, Some(RelayQuicConfig::new(port))))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if entries.len() != 4 {
            return Err("four QAD observers required".into());
        }
        Some(RelayMap::from_iter(entries))
    } else {
        None
    };
    let qad = RelayMap::from(RelayConfig::new(
        url,
        Some(RelayQuicConfig::new(observer_port)),
    ));
    let bind_port: u16 = std::env::var("MIOPUNCH_NAT_BIND_PORT")
        .unwrap_or_else(|_| "0".to_string())
        .parse()?;
    let gate = if let Ok(path) = std::env::var("MIOPUNCH_NAT_SECRET_FILE") {
        let secret = if path == "-" {
            None
        } else {
            let bytes: [u8; 32] = std::fs::read(path)?
                .try_into()
                .map_err(|_| "NAT lab secret must be 32 bytes")?;
            Some(AdmissionSecret::from_saved(bytes))
        };
        let gate = AdmissionGate::new(key.public(), AdmissionScope::new(NAT3_NETWORK, 0), secret);
        gate.register_protocol(ALPN, ProtocolScope::Bootstrap);
        let fault = std::env::var("MIOPUNCH_NAT_AUTH_FAULT").unwrap_or_default();
        if fault == "nonmember" {
            gate.update_authorization([*key.public().as_bytes()], []);
        } else if fault == "revoked" {
            gate.update_authorization([*key.public().as_bytes()], [*peer.as_bytes()]);
        } else {
            gate.update_authorization([*key.public().as_bytes(), *peer.as_bytes()], []);
        }
        Some(gate)
    } else {
        None
    };
    let builder = Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .alpns(vec![ALPN.to_vec()])
        .control_only_alpns(vec![ALPN.to_vec()])
        .bind_addr((Ipv4Addr::UNSPECIFIED, bind_port))?
        .clear_relay_transports()
        .observation_servers(qad)
        .ca_tls_config(roots)
        .add_custom_transport(carrier.clone());
    let endpoint = if let Some(gate) = &gate {
        gate.configure(builder)
    } else {
        builder
    }
    .bind()
    .await?;
    carrier.wait_ready().await?;
    Ok((endpoint, carrier, traffic, gate, mapping_observers))
}

async fn observed_addr(endpoint: &Endpoint) -> Result<ObservedCandidate, Box<dyn Error>> {
    let mut watcher = endpoint.udp_observation();
    let state = tokio::time::timeout(WAIT, async {
        loop {
            let state = watcher.get();
            if !matches!(state, UdpObservation::Pending) {
                return state;
            }
            watcher.updated().await.expect("observation watcher closed");
        }
    })
    .await?;
    let UdpObservation::Fresh { addresses, .. } = &state else {
        return Err(format!("no observed candidate: {state:?}").into());
    };
    if !state.is_fresh() {
        return Err("observation expired before exchange".into());
    }
    let addr = addresses
        .iter()
        .copied()
        .find(SocketAddr::is_ipv4)
        .ok_or("no IPv4 observed candidate")?;
    if !endpoint
        .addr()
        .ip_addrs()
        .any(|candidate| *candidate == addr)
    {
        return Err("observed candidate absent from endpoint address".into());
    }
    println!(
        "CANDIDATE id={} sockets={:?} observed={addr} mapping_varies={:?}",
        endpoint.id(),
        endpoint.bound_sockets(),
        match state {
            UdpObservation::Fresh {
                mapping_varies_by_dest,
                ..
            } => mapping_varies_by_dest,
            _ => unreachable!(),
        }
    );
    Ok(ObservedCandidate {
        addr,
        observation: state,
    })
}

async fn wait_ip(session: &ControlSession, limit: Duration) -> Result<(), Box<dyn Error>> {
    tokio::time::timeout(limit, async {
        while !session.has_data_path() {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await?;
    Ok(())
}

async fn read_candidate(
    recv: &mut iroh::endpoint::RecvStream,
) -> Result<SocketAddr, Box<dyn Error>> {
    let bytes = tokio::time::timeout(WAIT, recv.read_to_end(128)).await??;
    Ok(std::str::from_utf8(&bytes)?.parse()?)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Round([u8; 16]);

impl Round {
    fn fresh() -> Self {
        let random = SecretKey::generate().to_bytes();
        let mut bytes = [0; 16];
        bytes.copy_from_slice(&random[..16]);
        Self(bytes)
    }

    fn parse(value: &str) -> Result<Self, Box<dyn Error>> {
        let bytes: [u8; 16] = hex::decode(value)?
            .try_into()
            .map_err(|_| "invalid NAT3 round length")?;
        Ok(Self(bytes))
    }

    fn encoded(self) -> String {
        hex::encode(self.0)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum CoordFrame {
    Offer(SocketAddr),
    Ready(Round, SocketAddr),
    AutoOffer(MappingPattern),
    AutoReady(Round, MappingPattern),
    Start(Round),
    Ack(Round),
    Cancel(Round),
    SenderPrimed(Round),
    SenderPrimedAck(Round),
    PathFailed(Round),
    PathFailedAck(Round),
    PathFailedDone(Round),
    PathFailedDoneAck(Round),
    Promoted(Round),
    Committed(Round),
}

impl CoordFrame {
    fn encode(&self) -> String {
        match self {
            Self::Offer(addr) => format!("OFFER {addr}"),
            Self::Ready(round, addr) => format!("READY {} {addr}", round.encoded()),
            Self::AutoOffer(pattern) => format!("AUTO_OFFER {}", encode_pattern(*pattern)),
            Self::AutoReady(round, pattern) => format!(
                "AUTO_READY {} {}",
                round.encoded(),
                encode_pattern(*pattern)
            ),
            Self::Start(round) => format!("START {}", round.encoded()),
            Self::Ack(round) => format!("ACK {}", round.encoded()),
            Self::Cancel(round) => format!("CANCEL {}", round.encoded()),
            Self::SenderPrimed(round) => format!("SENDER_PRIMED {}", round.encoded()),
            Self::SenderPrimedAck(round) => format!("SENDER_PRIMED_ACK {}", round.encoded()),
            Self::PathFailed(round) => format!("PATH_FAILED {}", round.encoded()),
            Self::PathFailedAck(round) => format!("PATH_FAILED_ACK {}", round.encoded()),
            Self::PathFailedDone(round) => format!("PATH_FAILED_DONE {}", round.encoded()),
            Self::PathFailedDoneAck(round) => format!("PATH_FAILED_DONE_ACK {}", round.encoded()),
            Self::Promoted(round) => format!("PROMOTED {}", round.encoded()),
            Self::Committed(round) => format!("COMMITTED {}", round.encoded()),
        }
    }

    fn parse(bytes: &[u8]) -> Result<Self, Box<dyn Error>> {
        let text = std::str::from_utf8(bytes)?;
        let fields: Vec<_> = text.split_ascii_whitespace().collect();
        let frame = match fields.as_slice() {
            ["OFFER", addr] => Self::Offer(addr.parse()?),
            ["READY", round, addr] => Self::Ready(Round::parse(round)?, addr.parse()?),
            ["AUTO_OFFER", kind, addr, step] => Self::AutoOffer(parse_pattern(kind, addr, step)?),
            ["AUTO_READY", round, kind, addr, step] => {
                Self::AutoReady(Round::parse(round)?, parse_pattern(kind, addr, step)?)
            }
            ["START", round] => Self::Start(Round::parse(round)?),
            ["ACK", round] => Self::Ack(Round::parse(round)?),
            ["CANCEL", round] => Self::Cancel(Round::parse(round)?),
            ["SENDER_PRIMED", round] => Self::SenderPrimed(Round::parse(round)?),
            ["SENDER_PRIMED_ACK", round] => Self::SenderPrimedAck(Round::parse(round)?),
            ["PATH_FAILED", round] => Self::PathFailed(Round::parse(round)?),
            ["PATH_FAILED_ACK", round] => Self::PathFailedAck(Round::parse(round)?),
            ["PATH_FAILED_DONE", round] => Self::PathFailedDone(Round::parse(round)?),
            ["PATH_FAILED_DONE_ACK", round] => Self::PathFailedDoneAck(Round::parse(round)?),
            ["PROMOTED", round] => Self::Promoted(Round::parse(round)?),
            ["COMMITTED", round] => Self::Committed(Round::parse(round)?),
            _ => return Err("invalid NAT3 control frame".into()),
        };
        let addr = match frame {
            Self::Offer(addr) | Self::Ready(_, addr) => Some(addr),
            _ => None,
        };
        if addr.is_some_and(|addr| !addr.is_ipv4() || addr.port() == 0) {
            return Err("invalid NAT3 candidate".into());
        }
        Ok(frame)
    }
}

fn encode_pattern(pattern: MappingPattern) -> String {
    match pattern {
        MappingPattern::Stable { address } => format!("S {address} 0"),
        MappingPattern::Sequential { last, step } => format!("Q {last} {step}"),
        MappingPattern::Unknown => "U 0.0.0.0:0 0".into(),
    }
}

fn parse_pattern(kind: &str, addr: &str, step: &str) -> Result<MappingPattern, Box<dyn Error>> {
    let address: SocketAddr = addr.parse()?;
    let step: i16 = step.parse()?;
    let pattern = match kind {
        "S" if step == 0 && address.is_ipv4() && address.port() != 0 => {
            MappingPattern::Stable { address }
        }
        "Q" if step != 0
            && step.unsigned_abs() <= nat_mapping::MAX_ABS_STEP as u16
            && address.is_ipv4()
            && address.port() != 0 =>
        {
            let pattern = MappingPattern::Sequential {
                last: address,
                step,
            };
            if nat_mapping::predicted_peer_address(pattern).is_none() {
                return Err("invalid predicted UDP port".into());
            }
            pattern
        }
        "U" if address.port() == 0 && step == 0 => MappingPattern::Unknown,
        _ => return Err("invalid or over-budget mapping claim".into()),
    };
    Ok(pattern)
}

async fn send_frame(
    send: &mut iroh::endpoint::SendStream,
    frame: &CoordFrame,
) -> Result<(), Box<dyn Error>> {
    send.write_all(frame.encode().as_bytes()).await?;
    send.finish()?;
    Ok(())
}

async fn recv_frame(recv: &mut iroh::endpoint::RecvStream) -> Result<CoordFrame, Box<dyn Error>> {
    let bytes = tokio::time::timeout(COORD_WAIT, recv.read_to_end(128)).await??;
    CoordFrame::parse(&bytes)
}

async fn report_auto_path_failure_server(
    control: &ControlSession,
    round: Round,
) -> Result<(), Box<dyn Error>> {
    let (mut reply, mut request) =
        tokio::time::timeout(COORD_WAIT, control.accept_control_bi()).await??;
    if recv_frame(&mut request).await? != CoordFrame::PathFailed(round) {
        return Err("wrong failed-path round".into());
    }
    send_frame(&mut reply, &CoordFrame::PathFailedAck(round)).await?;
    // Do not tear down this one-run endpoint until the peer explicitly
    // confirms it received the first acknowledgment.
    let (mut done_reply, mut done_request) =
        tokio::time::timeout(COORD_WAIT, control.accept_control_bi()).await??;
    if recv_frame(&mut done_request).await? != CoordFrame::PathFailedDone(round) {
        return Err("failed-path receipt missing or wrong round".into());
    }
    send_frame(&mut done_reply, &CoordFrame::PathFailedDoneAck(round)).await?;
    // Keep the one-run endpoint alive until QUIC confirms the peer received
    // the terminal frame; closing immediately can discard buffered data.
    match tokio::time::timeout(COORD_WAIT, done_reply.stopped()).await?? {
        None => {}
        Some(_) => return Err("failed-path terminal stream stopped by peer".into()),
    }
    println!(
        "CONTROL_ALIVE_AFTER_UDP_FAILURE stable_id={}",
        control.stable_id()
    );
    Ok(())
}

async fn report_auto_path_failure_client(
    control: &ControlSession,
    round: Round,
) -> Result<(), Box<dyn Error>> {
    let (mut send, mut recv) =
        tokio::time::timeout(COORD_WAIT, control.open_control_bi()).await??;
    send_frame(&mut send, &CoordFrame::PathFailed(round)).await?;
    if recv_frame(&mut recv).await? != CoordFrame::PathFailedAck(round) {
        return Err("failed-path control acknowledgment mismatch".into());
    }
    let (mut done_send, mut done_recv) =
        tokio::time::timeout(COORD_WAIT, control.open_control_bi()).await??;
    send_frame(&mut done_send, &CoordFrame::PathFailedDone(round)).await?;
    if recv_frame(&mut done_recv).await? != CoordFrame::PathFailedDoneAck(round) {
        return Err("failed-path terminal acknowledgment mismatch".into());
    }
    println!(
        "CONTROL_ALIVE_AFTER_UDP_FAILURE stable_id={}",
        control.stable_id()
    );
    Ok(())
}

fn member_eligible(control: &ControlSession, gate: &Option<AdmissionGate>) -> bool {
    gate.as_ref()
        .is_none_or(|gate| control.may_upgrade_member_path(gate))
}

async fn coordinate_server(
    control: &ControlSession,
    own_candidate: &ObservedCandidate,
) -> Result<(SocketAddr, Round), Box<dyn Error>> {
    let (mut reply, mut request) =
        tokio::time::timeout(COORD_WAIT, control.accept_control_bi()).await??;
    let CoordFrame::Offer(remote) = recv_frame(&mut request).await? else {
        return Err("NAT3 offer expected".into());
    };
    let round = Round::fresh();
    send_frame(
        &mut reply,
        &CoordFrame::Ready(round, own_candidate.fresh_addr()?),
    )
    .await?;
    let (mut reply, mut request) =
        tokio::time::timeout(COORD_WAIT, control.accept_control_bi()).await??;
    match recv_frame(&mut request).await? {
        CoordFrame::Start(value) if value == round => {
            // Only this connection's fresh server challenge starts a probe
            // round. A replay or an old START cannot spend the probe budget.
            send_frame(&mut reply, &CoordFrame::Ack(round)).await?;
            Ok((remote, round))
        }
        CoordFrame::Cancel(value) if value == round => Err("NAT3 round cancelled".into()),
        _ => Err("NAT3 old or replayed round".into()),
    }
}

async fn coordinate_client(
    control: &ControlSession,
    own_candidate: &ObservedCandidate,
) -> Result<(SocketAddr, Round), Box<dyn Error>> {
    let (mut send, mut recv) =
        tokio::time::timeout(COORD_WAIT, control.open_control_bi()).await??;
    send_frame(&mut send, &CoordFrame::Offer(own_candidate.fresh_addr()?)).await?;
    let CoordFrame::Ready(round, remote) = recv_frame(&mut recv).await? else {
        return Err("NAT3 ready expected".into());
    };
    let fault = std::env::var("MIOPUNCH_NAT_FAULT").unwrap_or_default();
    if fault == "late_control" {
        tokio::time::sleep(COORD_WAIT + Duration::from_secs(1)).await;
    }
    let (mut send, mut recv) =
        tokio::time::timeout(COORD_WAIT, control.open_control_bi()).await??;
    let start = match fault.as_str() {
        "cancel" => CoordFrame::Cancel(round),
        "old_round" => {
            CoordFrame::Start(Round::parse(&std::env::var("MIOPUNCH_NAT_REPLAY_ROUND")?)?)
        }
        _ => CoordFrame::Start(round),
    };
    send_frame(&mut send, &start).await?;
    if fault == "cancel" {
        return Err("NAT3 round cancelled".into());
    }
    if recv_frame(&mut recv).await? != CoordFrame::Ack(round) {
        return Err("NAT3 ack mismatch".into());
    }
    if fault == "replay" {
        let (mut replay, _) = tokio::time::timeout(COORD_WAIT, control.open_control_bi()).await??;
        send_frame(&mut replay, &CoordFrame::Start(round)).await?;
        println!("REPLAY_SENT round={}", round.encoded());
        return Err("duplicate NAT3 start submitted".into());
    }
    Ok((remote, round))
}

fn auto_target(
    local: MappingPattern,
    remote: MappingPattern,
) -> Result<(SocketAddr, PunchRole), Box<dyn Error>> {
    let role = nat_mapping::choose_single_sequential(local, remote)
        .ok_or("no supported single-sequential mapping pair")?;
    let target = match remote {
        MappingPattern::Stable { address } => address,
        MappingPattern::Sequential { .. } => {
            nat_mapping::predicted_peer_address(remote).ok_or("predicted mapping port overflow")?
        }
        MappingPattern::Unknown => return Err("unknown peer mapping".into()),
    };
    Ok((target, role))
}

async fn coordinate_auto_server(
    control: &ControlSession,
    evidence: &MappingEvidence,
) -> Result<(SocketAddr, Round, PunchRole), Box<dyn Error>> {
    let (mut reply, mut request) =
        tokio::time::timeout(COORD_WAIT, control.accept_control_bi()).await??;
    let CoordFrame::AutoOffer(remote_pattern) = recv_frame(&mut request).await? else {
        return Err("auto mapping offer expected".into());
    };
    let own_pattern = evidence.fresh_pattern()?;
    let (remote, role) = auto_target(own_pattern, remote_pattern)?;
    let round = Round::fresh();
    send_frame(&mut reply, &CoordFrame::AutoReady(round, own_pattern)).await?;
    let (mut reply, mut request) =
        tokio::time::timeout(COORD_WAIT, control.accept_control_bi()).await??;
    match recv_frame(&mut request).await? {
        CoordFrame::Start(value) if value == round => {
            evidence.fresh_pattern()?;
            send_frame(&mut reply, &CoordFrame::Ack(round)).await?;
            Ok((remote, round, role))
        }
        CoordFrame::Cancel(value) if value == round => Err("mapping round cancelled".into()),
        _ => Err("old or replayed mapping round".into()),
    }
}

async fn coordinate_auto_client(
    control: &ControlSession,
    evidence: &MappingEvidence,
) -> Result<(SocketAddr, Round, PunchRole), Box<dyn Error>> {
    let (mut send, mut recv) =
        tokio::time::timeout(COORD_WAIT, control.open_control_bi()).await??;
    let local = evidence.fresh_pattern()?;
    let fault = std::env::var("MIOPUNCH_NAT_FAULT").unwrap_or_default();
    if fault == "wide_claim" {
        let address = match local {
            MappingPattern::Stable { address } => address,
            MappingPattern::Sequential { last, .. } => last,
            MappingPattern::Unknown => return Err("unknown local mapping".into()),
        };
        send.write_all(format!("AUTO_OFFER Q {address} 100").as_bytes())
            .await?;
        send.finish()?;
    } else {
        send_frame(&mut send, &CoordFrame::AutoOffer(local)).await?;
    }
    let CoordFrame::AutoReady(round, remote_pattern) = recv_frame(&mut recv).await? else {
        return Err("auto mapping ready expected".into());
    };
    let (remote, role) = auto_target(evidence.fresh_pattern()?, remote_pattern)?;
    if fault == "late_control" {
        tokio::time::sleep(COORD_WAIT + Duration::from_secs(1)).await;
    }
    let (mut send, mut recv) =
        tokio::time::timeout(COORD_WAIT, control.open_control_bi()).await??;
    if fault == "cancel" {
        send_frame(&mut send, &CoordFrame::Cancel(round)).await?;
        return Err("mapping round cancelled".into());
    }
    send_frame(&mut send, &CoordFrame::Start(round)).await?;
    if recv_frame(&mut recv).await? != CoordFrame::Ack(round) {
        return Err("mapping round acknowledgment mismatch".into());
    }
    evidence.fresh_pattern()?;
    Ok((remote, round, role))
}

async fn auto_probe_window(
    control: &ControlSession,
    gate: &Option<AdmissionGate>,
    evidence: &MappingEvidence,
    remote: SocketAddr,
    role: PunchRole,
    round: Round,
) -> Result<(), Box<dyn Error>> {
    let started = Instant::now();
    match role {
        PunchRole::MappingSideSender => {
            auto_probe_once(control, gate, evidence, remote).await?;
            // A protected, round-bound signal proves the Hard-side socket
            // actually sent before the Stable side tries its predicted port.
            let (mut send, mut recv) =
                tokio::time::timeout(COORD_WAIT, control.open_control_bi()).await??;
            send_frame(&mut send, &CoordFrame::SenderPrimed(round)).await?;
            if recv_frame(&mut recv).await? != CoordFrame::SenderPrimedAck(round) {
                return Err("sender-prime acknowledgment mismatch".into());
            }
        }
        PunchRole::StableSideReceiver => {
            let (mut reply, mut request) =
                tokio::time::timeout(COORD_WAIT, control.accept_control_bi()).await??;
            if recv_frame(&mut request).await? != CoordFrame::SenderPrimed(round) {
                return Err("sender-prime signal missing or wrong round".into());
            }
            send_frame(&mut reply, &CoordFrame::SenderPrimedAck(round)).await?;
            auto_probe_once(control, gate, evidence, remote).await?;
        }
    }
    if evidence.peer_probe_limit == 2 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        auto_probe_once(control, gate, evidence, remote).await?;
    }
    println!(
        "AUTO_PROBES_SENT count={} distinct_targets=1 role={role:?} target={remote} elapsed_ms={}",
        evidence.peer_probe_limit,
        started.elapsed().as_millis()
    );
    Ok(())
}

async fn auto_probe_once(
    control: &ControlSession,
    gate: &Option<AdmissionGate>,
    evidence: &MappingEvidence,
    remote: SocketAddr,
) -> Result<(), Box<dyn Error>> {
    if !member_eligible(control, gate) {
        return Err("member path upgrade no longer authorized".into());
    }
    evidence.fresh_pattern()?;
    if !evidence.budget.spend_peer_datagram() {
        return Err("node maintenance peer-datagram budget exhausted".into());
    }
    control.prime_udp_path(remote).await?;
    Ok(())
}

async fn probe_window(
    control: &ControlSession,
    gate: &Option<AdmissionGate>,
    own_candidate: &ObservedCandidate,
    remote: SocketAddr,
) -> Result<(), Box<dyn Error>> {
    let started = Instant::now();
    for attempt in 0..NAT3_PROBES {
        if !member_eligible(control, gate) {
            return Err("member path upgrade no longer authorized".into());
        }
        own_candidate.fresh_addr()?;
        control.prime_udp_path(remote).await?;
        if attempt + 1 < NAT3_PROBES {
            tokio::time::sleep(NAT3_PROBE_GAP).await;
        }
    }
    println!(
        "PROBES_SENT count={NAT3_PROBES} distinct_targets=1 elapsed_ms={}",
        started.elapsed().as_millis()
    );
    Ok(())
}

async fn run_server(
    endpoint: Endpoint,
    carrier: Arc<MqttTransport>,
    traffic: Arc<Traffic>,
    gate: Option<AdmissionGate>,
    peer: PublicKey,
    own_candidate: ObservedCandidate,
    mapping: Option<MappingEvidence>,
) -> Result<(), Box<dyn Error>> {
    println!("READY id={} route={}", endpoint.id(), carrier.route()?);
    let control = tokio::time::timeout(WAIT, ControlSession::accept(&endpoint)).await??;
    if control.remote_id() != peer {
        return Err("unexpected authenticated peer".into());
    }
    let stable = control.stable_id();
    if !member_eligible(&control, &gate) {
        println!("ADMISSION_REJECTED peer={peer} stable_id={stable} probes=0");
        endpoint.close().await;
        return Ok(());
    }
    let coordinated = std::env::var("MIOPUNCH_NAT_STRATEGY").as_deref() == Ok("coordinated");
    let automatic = std::env::var("MIOPUNCH_NAT_STRATEGY").as_deref() == Ok("auto");
    let advanced = coordinated || automatic;
    let mut active_round = None;
    let mut auto_role = None;
    let remote = if automatic {
        let evidence = mapping
            .as_ref()
            .ok_or("missing ordered QAD mapping evidence")?;
        match coordinate_auto_server(&control, evidence).await {
            Ok((remote, round, role)) => {
                active_round = Some(round);
                auto_role = Some(role);
                println!(
                    "AUTO_SELECTED role={role:?} target={remote} round={} stable_id={stable}",
                    round.encoded()
                );
                remote
            }
            Err(error) => {
                println!("COORD_UNAVAILABLE reason={error} mqtt={}", traffic.report());
                endpoint.close().await;
                return Ok(());
            }
        }
    } else if coordinated {
        match coordinate_server(&control, &own_candidate).await {
            Ok((remote, round)) => {
                active_round = Some(round);
                println!(
                    "COORD_START peer={peer} round={} stable_id={stable}",
                    round.encoded()
                );
                remote
            }
            Err(error) => {
                println!("COORD_UNAVAILABLE reason={error} mqtt={}", traffic.report());
                endpoint.close().await;
                return Ok(());
            }
        }
    } else {
        let (mut reply, mut request) =
            match tokio::time::timeout(WAIT, control.accept_control_bi()).await {
                Ok(Ok(streams)) => streams,
                other => {
                    println!(
                        "CONTROL_UNAVAILABLE reason={other:?} mqtt={}",
                        traffic.report()
                    );
                    endpoint.close().await;
                    return Ok(());
                }
            };
        let remote = match read_candidate(&mut request).await {
            Ok(remote) => remote,
            Err(error) => {
                println!(
                    "CONTROL_UNAVAILABLE reason={error} mqtt={}",
                    traffic.report()
                );
                endpoint.close().await;
                return Ok(());
            }
        };
        if std::env::var("MIOPUNCH_NAT_SKIP_PRIME").as_deref() == Ok("1") {
            println!("PRIME_SKIPPED peer_candidate={remote}");
        } else {
            control.prime_udp_path(remote).await?;
            println!("PROBES_SENT count=1 distinct_targets=1");
        }
        let own_addr = own_candidate.fresh_addr()?;
        reply.write_all(own_addr.to_string().as_bytes()).await?;
        reply.finish()?;
        remote
    };
    println!("EXCHANGED peer={peer} peer_candidate={remote} stable_id={stable}");
    println!(
        "CONTROL_MSGS sent={} received={}",
        if automatic {
            3
        } else if coordinated {
            2
        } else {
            1
        },
        if automatic {
            3
        } else if coordinated {
            2
        } else {
            1
        }
    );
    let path_started = Instant::now();
    if advanced {
        // The auto branch's next control stream is the authenticated
        // SENDER_PRIMED barrier. A duplicate START fails that frame check
        // without opening another probe window.
        if coordinated {
            let duplicate = tokio::select! {
                biased;
                extra = control.accept_control_bi() => Some(extra),
                _ = tokio::time::sleep(Duration::from_millis(50)) => None,
            };
            if duplicate.is_some() {
                println!(
                    "COORD_UNAVAILABLE reason=duplicate control stream mqtt={}",
                    traffic.report()
                );
                endpoint.close().await;
                return Ok(());
            }
        }
        let result = if automatic {
            auto_probe_window(
                &control,
                &gate,
                mapping.as_ref().unwrap(),
                remote,
                auto_role.unwrap(),
                active_round.unwrap(),
            )
            .await
        } else {
            probe_window(&control, &gate, &own_candidate, remote).await
        };
        if let Err(error) = result {
            println!("PATH_UNAVAILABLE reason={error} mqtt={}", traffic.report());
            endpoint.close().await;
            return Ok(());
        }
    }
    if let Err(error) = wait_ip(&control, if automatic { AUTO_PATH_WAIT } else { WAIT }).await {
        println!("PATH_UNAVAILABLE reason={error} mqtt={}", traffic.report());
        if automatic && let Some(round) = active_round {
            if let Err(reason) = report_auto_path_failure_server(&control, round).await {
                println!("CONTROL_LIVENESS_FAILED reason={reason}");
            }
        }
        endpoint.close().await;
        return Ok(());
    }
    if !member_eligible(&control, &gate) {
        println!("ADMISSION_REJECTED peer={peer} stable_id={stable} before_commit=1");
        endpoint.close().await;
        return Ok(());
    }
    let mut promotion_reply = if let Some(round) = active_round {
        let received = tokio::time::timeout(WAIT, async {
            let (send, mut recv) = control.accept_control_bi().await?;
            let frame = recv_frame(&mut recv).await?;
            Ok::<_, Box<dyn Error>>((send, frame))
        })
        .await;
        match received {
            Ok(Ok((send, CoordFrame::Promoted(value)))) if value == round => {
                println!(
                    "UDP_PROMOTION_RECEIVED round={} stable_id={stable}",
                    round.encoded()
                );
                Some(send)
            }
            other => {
                println!(
                    "PATH_UNAVAILABLE reason=promotion missing or wrong: {other:?} mqtt={}",
                    traffic.report()
                );
                endpoint.close().await;
                return Ok(());
            }
        }
    } else {
        None
    };
    println!("PATH_OBSERVED peer={peer} stable_id={stable}");
    let data = match control.commit_data_path().await {
        Ok(data) => data,
        Err(CommitError::Retryable { error, .. } | CommitError::Terminal { error }) => {
            println!("PATH_UNAVAILABLE reason={error} mqtt={}", traffic.report());
            endpoint.close().await;
            return Ok(());
        }
    };
    if let (Some(mut send), Some(round)) = (promotion_reply.take(), active_round) {
        if let Err(error) = send_frame(&mut send, &CoordFrame::Committed(round)).await {
            println!("PATH_UNAVAILABLE reason={error} mqtt={}", traffic.report());
            endpoint.close().await;
            return Ok(());
        }
        println!(
            "UDP_COMMIT_ACK round={} stable_id={stable}",
            round.encoded()
        );
    }
    let before = traffic.sent_packets();
    let business = tokio::time::timeout(WAIT, async {
        let (_, mut recv) = data.connection().accept_bi().await?;
        recv.read_to_end(64).await.map_err(io::Error::other)
    })
    .await;
    let business = match business {
        Ok(Ok(bytes)) => bytes,
        other => {
            println!(
                "PATH_UNAVAILABLE reason={other:?} mqtt={}",
                traffic.report()
            );
            endpoint.close().await;
            return Ok(());
        }
    };
    if business != b"NAT_DATA" || traffic.sent_packets() != before {
        return Err("business crossed MQTT or had wrong content".into());
    }
    println!(
        "DATA_OK stable_id={stable} peer={peer} path_ms={} paths={:?} mqtt_after_commit={before}",
        path_started.elapsed().as_millis(),
        data.connection().paths()
    );
    endpoint.close().await;
    Ok(())
}

async fn run_client(
    endpoint: Endpoint,
    traffic: Arc<Traffic>,
    gate: Option<AdmissionGate>,
    peer: PublicKey,
    route: &str,
    own_candidate: ObservedCandidate,
    override_candidate: Option<SocketAddr>,
    mapping: Option<MappingEvidence>,
) -> Result<(), Box<dyn Error>> {
    let target =
        EndpointAddr::new(peer).with_addrs([TransportAddr::Custom(MqttTransport::address(route)?)]);
    let mut control =
        tokio::time::timeout(WAIT, ControlSession::connect(&endpoint, target, ALPN)).await??;
    if control.remote_id() != peer {
        return Err("wrong authenticated peer".into());
    }
    let stable = control.stable_id();
    control.queue_business(b"NAT_DATA".to_vec())?;
    if !member_eligible(&control, &gate) {
        println!("ADMISSION_REJECTED peer={peer} stable_id={stable} probes=0");
        endpoint.close().await;
        return Ok(());
    }
    if std::env::var("MIOPUNCH_NAT_FAULT").as_deref() == Ok("stale") {
        endpoint.network_change().await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while own_candidate.observation.is_fresh() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        println!("STALE_CANDIDATE rejected_before_exchange stable_id={stable}");
        endpoint.close().await;
        return Ok(());
    }
    if std::env::var("MIOPUNCH_NAT_FAULT").as_deref() == Ok("control_cut") {
        println!("CONTROL_READY stable_id={stable}");
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    if std::env::var("MIOPUNCH_NAT_FAULT").as_deref() == Ok("expire_mapping") {
        // Lab-only delay: prove the ordered observation expires even though
        // the authenticated control connection and membership remain valid.
        tokio::time::sleep(nat_mapping::MAX_EVIDENCE_AGE + Duration::from_millis(100)).await;
    }
    let coordinated = std::env::var("MIOPUNCH_NAT_STRATEGY").as_deref() == Ok("coordinated");
    let automatic = std::env::var("MIOPUNCH_NAT_STRATEGY").as_deref() == Ok("auto");
    let advanced = coordinated || automatic;
    let mut active_round = None;
    let mut auto_role = None;
    let reported = if automatic {
        let evidence = mapping
            .as_ref()
            .ok_or("missing ordered QAD mapping evidence")?;
        match coordinate_auto_client(&control, evidence).await {
            Ok((remote, round, role)) => {
                active_round = Some(round);
                auto_role = Some(role);
                println!(
                    "AUTO_SELECTED role={role:?} target={remote} round={} stable_id={stable}",
                    round.encoded()
                );
                remote
            }
            Err(error) => {
                println!("COORD_UNAVAILABLE reason={error} mqtt={}", traffic.report());
                endpoint.close().await;
                return Ok(());
            }
        }
    } else if coordinated {
        match coordinate_client(&control, &own_candidate).await {
            Ok((remote, round)) => {
                active_round = Some(round);
                println!(
                    "COORD_START peer={peer} round={} stable_id={stable}",
                    round.encoded()
                );
                remote
            }
            Err(error) => {
                println!("COORD_UNAVAILABLE reason={error} mqtt={}", traffic.report());
                endpoint.close().await;
                return Ok(());
            }
        }
    } else {
        let own_addr = own_candidate.fresh_addr()?;
        match tokio::time::timeout(WAIT, async {
            let (mut send, mut recv) = control.open_control_bi().await?;
            send.write_all(own_addr.to_string().as_bytes()).await?;
            send.finish()?;
            read_candidate(&mut recv).await
        })
        .await
        {
            Ok(Ok(addr)) => addr,
            other => {
                let error = format!("{other:?}");
                println!(
                    "CONTROL_UNAVAILABLE reason={error} mqtt={}",
                    traffic.report()
                );
                endpoint.close().await;
                return Ok(());
            }
        }
    };
    let remote = override_candidate.unwrap_or(reported);
    println!(
        "EXCHANGED peer={peer} peer_candidate={reported} attempted={remote} stable_id={stable}"
    );
    println!(
        "CONTROL_MSGS sent={} received={}",
        if automatic {
            3
        } else if coordinated {
            2
        } else {
            1
        },
        if automatic {
            3
        } else if coordinated {
            2
        } else {
            1
        }
    );
    let path_started = Instant::now();
    if !advanced {
        control.prime_udp_path(remote).await?;
        println!("PROBES_SENT count=1 distinct_targets=1");
    }
    if std::env::var("MIOPUNCH_NAT_FAULT").as_deref() == Ok("path_id_retry") {
        #[cfg(feature = "nat-lab-test-hooks")]
        control.test_force_path_id_retry_once().await?;
        #[cfg(not(feature = "nat-lab-test-hooks"))]
        return Err("path_id_retry requires the nat-lab-test-hooks feature".into());
    }
    let result = if automatic {
        let probes = auto_probe_window(
            &control,
            &gate,
            mapping.as_ref().unwrap(),
            remote,
            auto_role.unwrap(),
            active_round.unwrap(),
        )
        .await;
        if let Err(error) = probes {
            println!("PATH_UNAVAILABLE reason={error} mqtt={}", traffic.report());
            endpoint.close().await;
            return Ok(());
        }
        tokio::time::timeout(AUTO_PATH_WAIT, control.open_data_path_report(remote)).await
    } else if coordinated {
        let (probes, opened) = tokio::join!(
            probe_window(&control, &gate, &own_candidate, remote),
            tokio::time::timeout(WAIT, control.open_data_path_report(remote))
        );
        if let Err(error) = probes {
            println!("PATH_UNAVAILABLE reason={error} mqtt={}", traffic.report());
            endpoint.close().await;
            return Ok(());
        }
        opened
    } else {
        tokio::time::timeout(WAIT, control.open_data_path_report(remote)).await
    };
    match result {
        Ok(Ok(report)) => {
            println!("PATH_OPEN path_id_retries={}", report.path_id_retries);
        }
        other => {
            println!(
                "PATH_UNAVAILABLE reason={other:?} mqtt={}",
                traffic.report()
            );
            if automatic && let Some(round) = active_round {
                if let Err(reason) = report_auto_path_failure_client(&control, round).await {
                    println!("CONTROL_LIVENESS_FAILED reason={reason}");
                }
            }
            endpoint.close().await;
            return Ok(());
        }
    }
    if let Err(error) = wait_ip(&control, if automatic { AUTO_PATH_WAIT } else { WAIT }).await {
        println!("PATH_UNAVAILABLE reason={error} mqtt={}", traffic.report());
        if automatic && let Some(round) = active_round {
            if let Err(reason) = report_auto_path_failure_client(&control, round).await {
                println!("CONTROL_LIVENESS_FAILED reason={reason}");
            }
        }
        endpoint.close().await;
        return Ok(());
    }
    if !member_eligible(&control, &gate) {
        println!("ADMISSION_REJECTED peer={peer} stable_id={stable} before_commit=1");
        endpoint.close().await;
        return Ok(());
    }
    match control.commit_data_path().await {
        Ok(mut data) => {
            if let Some(round) = active_round {
                let promoted = tokio::time::timeout(WAIT, async {
                    let (mut send, mut recv) = data.connection().open_bi().await?;
                    send_frame(&mut send, &CoordFrame::Promoted(round)).await?;
                    let answer = recv_frame(&mut recv).await?;
                    if answer != CoordFrame::Committed(round) {
                        return Err("NAT3 promotion acknowledgment mismatch".into());
                    }
                    Ok::<_, Box<dyn Error>>(())
                })
                .await;
                match promoted {
                    Ok(Ok(())) => {}
                    other => {
                        println!(
                            "PATH_UNAVAILABLE reason={other:?} mqtt={}",
                            traffic.report()
                        );
                        endpoint.close().await;
                        return Ok(());
                    }
                }
                println!(
                    "UDP_COMMIT_ACK round={} stable_id={stable}",
                    round.encoded()
                );
            }
            let queued = data
                .take_pending_business()
                .pop_front()
                .ok_or("missing queued business")?;
            let before = traffic.sent_packets();
            let (mut send, _) = data.connection().open_bi().await?;
            send.write_all(&queued).await?;
            send.finish()?;
            tokio::time::sleep(Duration::from_millis(250)).await;
            if traffic.sent_packets() != before {
                return Err("business used MQTT".into());
            }
            println!(
                "DATA_OK stable_id={stable} peer={peer} path_ms={} paths={:?} mqtt_after_commit={before}",
                path_started.elapsed().as_millis(),
                data.connection().paths()
            );
            endpoint.close().await;
            return Ok(());
        }
        Err(CommitError::Retryable { error, .. }) => {
            println!("PATH_UNAVAILABLE reason={error} mqtt={}", traffic.report());
        }
        Err(CommitError::Terminal { error }) => {
            println!("PATH_UNAVAILABLE reason={error} mqtt={}", traffic.report());
        }
    }
    endpoint.close().await;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if let [mode, path] = args.as_slice() {
        if mode == "identity" {
            println!("{}", load_or_create_key(Path::new(path))?.public());
            return Ok(());
        }
    }
    let [
        mode,
        key_path,
        cert_path,
        broker,
        observer,
        port,
        peer,
        tail @ ..,
    ] = args.as_slice()
    else {
        return Err("usage: udp_nat_probe identity KEY | server|client KEY CERT BROKER OBSERVER_HOST OBSERVER_PORT PEER_ID [ROUTE [PEER_UDP_OVERRIDE]]".into());
    };
    if matches!(
        std::env::var("MIOPUNCH_NAT_STRATEGY").as_deref(),
        Ok("coordinated" | "auto")
    ) && std::env::var("MIOPUNCH_NAT_SECRET_FILE").is_err()
    {
        return Err("enhanced UDP strategy requires an admission-gated control connection".into());
    }
    let peer: PublicKey = peer.parse()?;
    let (endpoint, carrier, traffic, gate, mapping_observers) = endpoint(
        load_or_create_key(Path::new(key_path))?,
        peer,
        broker.parse()?,
        observer,
        port.parse()?,
        Path::new(cert_path),
    )
    .await?;
    let own_candidate = observed_addr(&endpoint).await?;
    let mapping = if std::env::var("MIOPUNCH_NAT_STRATEGY").as_deref() == Ok("auto") {
        // The endpoint's baseline QAD observation remains available to all
        // strategies; this allowance covers only the four additional QAD
        // connections and the selected one/two peer-directed datagrams.
        let probe_limit = auto_probe_limit()?;
        let budget = Arc::new(NodeProbeBudget::new(4, probe_limit));
        Some(
            mapping_evidence(
                &endpoint,
                mapping_observers
                    .as_ref()
                    .ok_or("auto requires four configured observers")?,
                budget,
                probe_limit,
            )
            .await?,
        )
    } else {
        None
    };
    match (mode.as_str(), tail) {
        ("server", []) => {
            run_server(
                endpoint,
                carrier,
                traffic,
                gate,
                peer,
                own_candidate,
                mapping,
            )
            .await
        }
        ("client", [route]) => {
            run_client(
                endpoint,
                traffic,
                gate,
                peer,
                route,
                own_candidate,
                None,
                mapping,
            )
            .await
        }
        ("client", [route, override_addr]) => {
            run_client(
                endpoint,
                traffic,
                gate,
                peer,
                route,
                own_candidate,
                Some(override_addr.parse()?),
                mapping,
            )
            .await
        }
        _ => Err("invalid mode arguments".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::{CoordFrame, Round};
    use miopunch_native_probe::nat_mapping::MappingPattern;

    #[test]
    fn control_frames_are_exact_and_round_bound() {
        let round = Round([7; 16]);
        let addr = "100.64.27.2:42027".parse().unwrap();
        for frame in [
            CoordFrame::Offer(addr),
            CoordFrame::Ready(round, addr),
            CoordFrame::AutoOffer(MappingPattern::Sequential {
                last: addr,
                step: -2,
            }),
            CoordFrame::AutoReady(round, MappingPattern::Stable { address: addr }),
            CoordFrame::Start(round),
            CoordFrame::Ack(round),
            CoordFrame::Cancel(round),
            CoordFrame::SenderPrimed(round),
            CoordFrame::SenderPrimedAck(round),
            CoordFrame::PathFailed(round),
            CoordFrame::PathFailedAck(round),
            CoordFrame::PathFailedDone(round),
            CoordFrame::PathFailedDoneAck(round),
            CoordFrame::Promoted(round),
            CoordFrame::Committed(round),
        ] {
            assert_eq!(CoordFrame::parse(frame.encode().as_bytes()).unwrap(), frame);
        }
        assert!(CoordFrame::parse(b"START 00000000000000000000000000000000 extra").is_err());
        assert!(CoordFrame::parse(b"OFFER 100.64.27.2:0").is_err());
        assert!(CoordFrame::parse(b"READY 00 100.64.27.2:42027").is_err());
        assert!(CoordFrame::parse(b"AUTO_OFFER Q 100.64.27.2:42027 100").is_err());
        assert!(CoordFrame::parse(b"AUTO_OFFER Q 100.64.27.2:42027 3").is_err());
        assert!(CoordFrame::parse(b"AUTO_OFFER Q 100.64.27.2:65535 1").is_err());
        assert!(CoordFrame::parse(b"AUTO_OFFER S 100.64.27.2:42027 1").is_err());
        assert_ne!(CoordFrame::Start(round), CoordFrame::Start(Round([0; 16])));
    }
}
