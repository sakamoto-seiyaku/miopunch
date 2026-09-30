//! MIOPUNCH-27: cross-process MQTT bootstrap and real dual-NAT UDP path probe.
use std::{
    error::Error,
    io,
    net::{Ipv4Addr, SocketAddr},
    path::Path,
    sync::{Arc, Mutex},
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
    nat_connection::{NatConnector, UdpPolicy},
    nat_mapping::{self, MappingPattern, MappingSample, NodeProbeBudget, PunchRole},
    nat_punch::{self, Attempt, ExecutionStatus, MappingSnapshot},
    nat_selector::Analyzer,
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

struct LegacyContext {
    snapshot: MappingSnapshot,
    analyzer: Arc<Mutex<Analyzer>>,
    budget: Arc<NodeProbeBudget>,
}

struct LegacyResources {
    observers: Option<RelayMap>,
    analyzer: Arc<Mutex<Analyzer>>,
    budget: Arc<NodeProbeBudget>,
}

async fn prepare_legacy(
    endpoint: &Endpoint,
    own_candidate: &ObservedCandidate,
    resources: LegacyResources,
) -> Result<LegacyContext, Box<dyn Error>> {
    let snapshot = if let Some(observers) = &resources.observers {
        if !resources.budget.reserve_observations(observers.len()) {
            return Err("legacy observation budget exhausted".into());
        }
        match endpoint.observe_udp_mapping_sequence(observers).await {
            Ok(sequence) => {
                for (index, sample) in sequence.samples.iter().enumerate() {
                    println!(
                        "LEGACY_SAMPLE order={index} destination={} mapped={}",
                        sample.destination, sample.mapped
                    );
                }
                MappingSnapshot::from_sequence(sequence)?
            }
            Err(error) => {
                println!("LEGACY_OBSERVATION_FALLBACK reason={error}");
                MappingSnapshot::fallback(endpoint, own_candidate.observation.clone())?
            }
        }
    } else {
        MappingSnapshot::fallback(endpoint, own_candidate.observation.clone())?
    };
    println!(
        "LEGACY_FEATURE feature={:?} socket_generation_current={}",
        snapshot.feature(),
        snapshot.is_current()
    );
    Ok(LegacyContext {
        snapshot,
        analyzer: resources.analyzer,
        budget: resources.budget,
    })
}

fn trace_legacy(attempt: &Attempt, stable: usize) {
    println!(
        "LEGACY_SELECTED mode={} index={} c={:?} v={:?} scores_before={:?} round={} stable_id={} execution={:?} fallback={}",
        attempt.plan().id.mode,
        attempt.plan().id.index,
        attempt.plan().c,
        attempt.plan().v,
        attempt.plan().scores_before,
        attempt.round().encoded(),
        stable,
        attempt.execution_status(),
        attempt.plan().fallback
    );
}

async fn legacy_probe(
    control: &ControlSession,
    gate: &Option<AdmissionGate>,
    context: &LegacyContext,
    attempt: &mut Attempt,
) -> Result<(), Box<dyn Error>> {
    let status = attempt
        .execute(
            control,
            gate.as_ref().ok_or("legacy requires member gate")?,
            &context.budget,
        )
        .await?;
    println!(
        "LEGACY_EXECUTION status={status:?} target={:?} stable_id={}",
        attempt.remote_target(),
        control.stable_id()
    );
    if status != ExecutionStatus::CompleteExecution {
        return Err(format!("legacy {status:?}").into());
    }
    let path = attempt
        .prepare_data_path(control, gate.as_ref().ok_or("legacy requires member gate")?)
        .await?;
    println!(
        "LEGACY_WINNER_PATH path_id_retries={} stable_id={}",
        path.path_id_retries,
        control.stable_id()
    );
    println!(
        "PROBES_SENT count={} distinct_targets={} complete_execution=1",
        attempt.execution_trace().map_or(0, |trace| trace.probes),
        attempt.execution_trace().map_or(0, |trace| trace
            .targets
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len())
    );
    Ok(())
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

async fn guarded_control<T>(
    attempt: Option<&Attempt>,
    control: &ControlSession,
    gate: &Option<AdmissionGate>,
    operation: impl std::future::Future<Output = Result<T, Box<dyn Error>>>,
) -> Result<T, Box<dyn Error>> {
    if let Some(attempt) = attempt {
        Ok(attempt
            .guard_handoff(
                control,
                gate.as_ref().ok_or("legacy requires member gate")?,
                async {
                    operation
                        .await
                        .map_err(|error| io::Error::other(error.to_string()))
                },
            )
            .await?)
    } else {
        operation.await
    }
}

async fn guarded_data<T>(
    attempt: Option<&Attempt>,
    data: &miopunch_native_probe::control_session::DataSession,
    gate: &Option<AdmissionGate>,
    operation: impl std::future::Future<Output = Result<T, Box<dyn Error>>>,
) -> Result<T, Box<dyn Error>> {
    if let Some(attempt) = attempt {
        Ok(attempt
            .guard_committed(
                data,
                gate.as_ref().ok_or("legacy requires member gate")?,
                async {
                    operation
                        .await
                        .map_err(|error| io::Error::other(error.to_string()))
                },
            )
            .await?)
    } else {
        operation.await
    }
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
        "MAPPING_CLASS pattern={pattern:?} confidence={} fresh={}",
        match pattern {
            MappingPattern::Unknown => "invalid_or_unsupported",
            MappingPattern::Irregular { .. } => "fresh_irregular_no_prediction",
            _ => "consistent_3/3_transitions",
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

fn automatic_strategy(_has_mapping_observers: bool) -> Result<bool, Box<dyn Error>> {
    match std::env::var("MIOPUNCH_NAT_STRATEGY").as_deref() {
        Ok("baseline" | "coordinated" | "legacy" | "auto" | "formal") => Ok(false),
        Ok("single-candidate") => Ok(true),
        Err(_) => Ok(false),
        _ => Err("invalid NAT strategy".into()),
    }
}

fn data_path_wait(automatic: bool) -> Result<Duration, Box<dyn Error>> {
    match std::env::var("MIOPUNCH_NAT_LAB_PATH_WAIT").as_deref() {
        Ok("3") => Ok(AUTO_PATH_WAIT),
        Err(_) => Ok(if automatic { AUTO_PATH_WAIT } else { WAIT }),
        _ => Err("lab path wait must be 3 seconds".into()),
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
        if entries.len() < 2 || entries.len() > 64 {
            return Err("two to 64 QAD observers required".into());
        }
        Some(RelayMap::from_iter(entries))
    } else {
        None
    };
    let qad = RelayMap::from(RelayConfig::new(
        url,
        Some(RelayQuicConfig::new(observer_port)),
    ));
    // A resource-absence fixture, never a selector or behavior override.
    #[cfg(feature = "nat-lab-test-hooks")]
    let (mapping_observers, qad) =
        if std::env::var("MIOPUNCH_NAT_LAB_NO_OBSERVERS").as_deref() == Ok("1") {
            (None, RelayMap::empty())
        } else {
            (mapping_observers, qad)
        };
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
    ReceiverPrimed(Round),
    ReceiverPrimedAck(Round),
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
            Self::ReceiverPrimed(round) => format!("RECEIVER_PRIMED {}", round.encoded()),
            Self::ReceiverPrimedAck(round) => format!("RECEIVER_PRIMED_ACK {}", round.encoded()),
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
            ["RECEIVER_PRIMED", round] => Self::ReceiverPrimed(Round::parse(round)?),
            ["RECEIVER_PRIMED_ACK", round] => Self::ReceiverPrimedAck(Round::parse(round)?),
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
        MappingPattern::Irregular { last, low, high } => format!("I {last} {low}:{high}"),
        MappingPattern::Unknown => "U 0.0.0.0:0 0".into(),
    }
}

fn parse_pattern(kind: &str, addr: &str, step: &str) -> Result<MappingPattern, Box<dyn Error>> {
    let address: SocketAddr = addr.parse()?;
    if kind == "I" {
        let (low, high) = step.split_once(':').ok_or("invalid irregular span")?;
        let pattern = MappingPattern::Irregular {
            last: address,
            low: low.parse()?,
            high: high.parse()?,
        };
        if nat_mapping::irregular_window(pattern).is_none() {
            return Err("invalid irregular mapping claim".into());
        }
        return Ok(pattern);
    }
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
    let encoded = frame.encode();
    send.write_all(encoded.as_bytes()).await?;
    send.finish()?;
    println!(
        "CONTROL_FRAME_SENT kind={} bytes={}",
        encoded.split_whitespace().next().unwrap(),
        encoded.len()
    );
    Ok(())
}

async fn recv_frame(recv: &mut iroh::endpoint::RecvStream) -> Result<CoordFrame, Box<dyn Error>> {
    let bytes = tokio::time::timeout(COORD_WAIT, recv.read_to_end(128)).await??;
    let frame = CoordFrame::parse(&bytes)?;
    println!(
        "CONTROL_FRAME_RECEIVED kind={} bytes={}",
        frame.encode().split_whitespace().next().unwrap(),
        bytes.len()
    );
    Ok(frame)
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

/// A dedicated protected lab stream, opened before fault injection. It proves
/// early rejection leaves this original control connection usable even when
/// the rejected strategy never completed a round. It never carries business.
struct FailureLiveness {
    send: iroh::endpoint::SendStream,
    recv: iroh::endpoint::RecvStream,
    server: bool,
}

impl FailureLiveness {
    async fn open(control: &ControlSession, server: bool) -> Result<Option<Self>, Box<dyn Error>> {
        if std::env::var("MIOPUNCH_NAT_LAB_FAILURE_LIVENESS").as_deref() != Ok("1") {
            return Ok(None);
        }
        let (mut send, mut recv) = if server {
            tokio::time::timeout(COORD_WAIT, control.accept_control_bi()).await??
        } else {
            tokio::time::timeout(COORD_WAIT, control.open_control_bi()).await??
        };
        tokio::time::timeout(COORD_WAIT, async {
            let mut marker = [0; 5];
            if server {
                recv.read_exact(&mut marker).await?;
                if &marker != b"MIO59" {
                    return Err("invalid failure-check stream".into());
                }
                send.write_all(b"READY").await?;
            } else {
                send.write_all(b"MIO59").await?;
                recv.read_exact(&mut marker).await?;
                if &marker != b"READY" {
                    return Err("invalid failure-check acknowledgment".into());
                }
            }
            Ok::<_, Box<dyn Error>>(())
        })
        .await??;
        Ok(Some(Self { send, recv, server }))
    }

    async fn finish(mut self, stable: usize) -> Result<(), Box<dyn Error>> {
        tokio::time::timeout(WAIT, async {
            if self.server {
                if self.recv.read_to_end(4).await? != b"PING" {
                    return Err("invalid failure-check ping".into());
                }
                self.send.write_all(b"PONG").await?;
                self.send.finish()?;
                if self.send.stopped().await?.is_some() {
                    return Err("failure-check reply stopped".into());
                }
            } else {
                self.send.write_all(b"PING").await?;
                self.send.finish()?;
                if self.recv.read_to_end(4).await? != b"PONG" {
                    return Err("invalid failure-check pong".into());
                }
            }
            Ok::<_, Box<dyn Error>>(())
        })
        .await??;
        println!("EARLY_FAILURE_CONTROL_ALIVE stable_id={stable}");
        Ok(())
    }
}

async fn close_failed_lab_attempt(
    endpoint: &Endpoint,
    stable: usize,
    safety: &mut Option<FailureLiveness>,
) -> Result<(), Box<dyn Error>> {
    if let Some(check) = safety.take() {
        check.finish(stable).await?;
    }
    endpoint.close().await;
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
    is_server: bool,
) -> Result<(SocketAddr, PunchRole), Box<dyn Error>> {
    let role = nat_mapping::choose_regular_pair(local, remote, is_server)
        .or_else(|| nat_mapping::choose_irregular_pair(local, remote))
        .ok_or("unsupported mapping pair: bounded fallback, no peer probe")?;
    let target = match remote {
        MappingPattern::Stable { address } => address,
        MappingPattern::Sequential { .. } => {
            nat_mapping::predicted_peer_address(remote).ok_or("predicted mapping port overflow")?
        }
        MappingPattern::Irregular { .. } => {
            let padding = match std::env::var("MIOPUNCH_NAT_LAB_RANDOM_PADDING").as_deref() {
                Ok("0") => 0,
                Ok("4") | Err(_) => 4,
                _ => return Err("lab random padding must be 0 or 4".into()),
            };
            let (last, low, high) = nat_mapping::irregular_window_with_padding(remote, padding)
                .ok_or("invalid irregular evidence")?;
            let port = rand::random_range(low..=high);
            println!(
                "RANDOM_CANDIDATE observed_last={last} low={low} high={high} draws=1 ports=1 sockets=0"
            );
            SocketAddr::new(last.ip(), port)
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
    let (remote, role) = auto_target(own_pattern, remote_pattern, true)?;
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
            MappingPattern::Irregular { last, .. } => last,
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
    let (remote, role) = auto_target(evidence.fresh_pattern()?, remote_pattern, false)?;
    if fault == "late_control" {
        tokio::time::sleep(COORD_WAIT + Duration::from_secs(1)).await;
    }
    let (mut send, mut recv) =
        tokio::time::timeout(COORD_WAIT, control.open_control_bi()).await??;
    if fault == "cancel" {
        send_frame(&mut send, &CoordFrame::Cancel(round)).await?;
        return Err("mapping round cancelled".into());
    }
    let start = if fault == "old_round" {
        CoordFrame::Start(Round::parse(&std::env::var("MIOPUNCH_NAT_REPLAY_ROUND")?)?)
    } else {
        CoordFrame::Start(round)
    };
    send_frame(&mut send, &start).await?;
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
    let dual_first_packets = match std::env::var("MIOPUNCH_NAT_LAB_DUAL_FIRST_PACKETS").as_deref() {
        Ok("1") | Err(_) => 1,
        Ok("2") => 2,
        _ => return Err("lab dual-first packet count must be 1 or 2".into()),
    };
    let signal_round = if std::env::var("MIOPUNCH_NAT_FAULT").as_deref() == Ok("old_receipt") {
        Round([0; 16])
    } else {
        round
    };
    match role {
        PunchRole::MappingSideSender
        | PunchRole::IrregularReceiver
        | PunchRole::IrregularRegularReceiver => {
            auto_probe_once(control, gate, evidence, remote).await?;
            // A protected round-bound barrier orders the irregular/Hard
            // prime before the peer sends to its selected candidate.
            let (mut send, mut recv) =
                tokio::time::timeout(COORD_WAIT, control.open_control_bi()).await??;
            send_frame(&mut send, &CoordFrame::SenderPrimed(signal_round)).await?;
            if recv_frame(&mut recv).await? != CoordFrame::SenderPrimedAck(round) {
                return Err("sender-prime acknowledgment mismatch".into());
            }
        }
        PunchRole::StableSideReceiver
        | PunchRole::RandomTargetSender
        | PunchRole::RegularRandomTargetSender => {
            let (mut reply, mut request) =
                tokio::time::timeout(COORD_WAIT, control.accept_control_bi()).await??;
            if recv_frame(&mut request).await? != CoordFrame::SenderPrimed(round) {
                return Err("sender-prime signal missing or wrong round".into());
            }
            send_frame(&mut reply, &CoordFrame::SenderPrimedAck(round)).await?;
            auto_probe_once(control, gate, evidence, remote).await?;
        }
        PunchRole::DualFirst => {
            auto_probe_once(control, gate, evidence, remote).await?;
            let (mut send, mut recv) =
                tokio::time::timeout(COORD_WAIT, control.open_control_bi()).await??;
            send_frame(&mut send, &CoordFrame::SenderPrimed(round)).await?;
            if recv_frame(&mut recv).await? != CoordFrame::SenderPrimedAck(round) {
                return Err("dual first-prime acknowledgment mismatch".into());
            }
            let (mut reply, mut request) =
                tokio::time::timeout(COORD_WAIT, control.accept_control_bi()).await??;
            if recv_frame(&mut request).await? != CoordFrame::ReceiverPrimed(round) {
                return Err("dual second-prime signal missing or wrong round".into());
            }
            send_frame(&mut reply, &CoordFrame::ReceiverPrimedAck(round)).await?;
            if dual_first_packets == 2 {
                auto_probe_once(control, gate, evidence, remote).await?;
            }
        }
        PunchRole::DualSecond => {
            let (mut reply, mut request) =
                tokio::time::timeout(COORD_WAIT, control.accept_control_bi()).await??;
            if recv_frame(&mut request).await? != CoordFrame::SenderPrimed(round) {
                return Err("dual first-prime signal missing or wrong round".into());
            }
            send_frame(&mut reply, &CoordFrame::SenderPrimedAck(round)).await?;
            auto_probe_once(control, gate, evidence, remote).await?;
            let (mut send, mut recv) =
                tokio::time::timeout(COORD_WAIT, control.open_control_bi()).await??;
            send_frame(&mut send, &CoordFrame::ReceiverPrimed(round)).await?;
            if recv_frame(&mut recv).await? != CoordFrame::ReceiverPrimedAck(round) {
                return Err("dual second-prime acknowledgment mismatch".into());
            }
        }
    }
    if evidence.peer_probe_limit == 2
        && matches!(
            role,
            PunchRole::MappingSideSender | PunchRole::StableSideReceiver
        )
    {
        tokio::time::sleep(Duration::from_millis(50)).await;
        auto_probe_once(control, gate, evidence, remote).await?;
    }
    let probes = match role {
        PunchRole::DualFirst => dual_first_packets,
        PunchRole::DualSecond => 1,
        PunchRole::IrregularReceiver
        | PunchRole::IrregularRegularReceiver
        | PunchRole::RandomTargetSender
        | PunchRole::RegularRandomTargetSender => 1,
        _ => evidence.peer_probe_limit,
    };
    println!(
        "AUTO_PROBES_SENT count={} distinct_targets=1 role={role:?} target={remote} elapsed_ms={}",
        probes,
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
    legacy: Option<LegacyResources>,
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
    let mut safety = FailureLiveness::open(&control, true).await?;
    let legacy = match legacy {
        Some(resources) => Some(prepare_legacy(&endpoint, &own_candidate, resources).await?),
        None => None,
    };
    let coordinated = std::env::var("MIOPUNCH_NAT_STRATEGY").as_deref() == Ok("coordinated");
    let automatic = automatic_strategy(mapping.is_some())?;
    let advanced = coordinated || automatic || legacy.is_some();
    let mut active_round = None;
    let mut auto_role = None;
    let mut legacy_attempt = None;
    let remote = if let Some(context) = &legacy {
        match nat_punch::coordinate_server(
            &control,
            gate.as_ref().ok_or("legacy requires member gate")?,
            &context.snapshot,
            &context.analyzer,
        )
        .await
        {
            Ok(attempt) => {
                trace_legacy(&attempt, stable);
                active_round = Some(Round::parse(&attempt.round().encoded())?);
                let target = attempt.remote_target();
                legacy_attempt = Some(attempt);
                target
            }
            Err(error) => {
                println!(
                    "COORD_UNAVAILABLE reason={error} probes=0 mqtt={}",
                    traffic.report()
                );
                close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
                return Ok(());
            }
        }
    } else if automatic {
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
                Some(remote)
            }
            Err(error) => {
                println!("COORD_UNAVAILABLE reason={error} mqtt={}", traffic.report());
                close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
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
                Some(remote)
            }
            Err(error) => {
                println!("COORD_UNAVAILABLE reason={error} mqtt={}", traffic.report());
                close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
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
                    close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
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
                close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
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
        println!(
            "CONTROL_FRAME_SENT kind=CANDIDATE bytes={}",
            own_addr.to_string().len()
        );
        println!(
            "CONTROL_FRAME_RECEIVED kind=CANDIDATE bytes={}",
            remote.to_string().len()
        );
        Some(remote)
    };
    if let Some(remote) = remote {
        println!("EXCHANGED peer={peer} peer_candidate={remote} stable_id={stable}");
    } else {
        println!("EXCHANGED peer={peer} peer_candidate=none receive_only=1 stable_id={stable}");
    }
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
                close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
                return Ok(());
            }
        }
        let result = if let Some(context) = &legacy {
            legacy_probe(&control, &gate, context, legacy_attempt.as_mut().unwrap()).await
        } else if automatic {
            auto_probe_window(
                &control,
                &gate,
                mapping.as_ref().unwrap(),
                remote.expect("auto target"),
                auto_role.unwrap(),
                active_round.unwrap(),
            )
            .await
        } else {
            probe_window(
                &control,
                &gate,
                &own_candidate,
                remote.expect("coordinated target"),
            )
            .await
        };
        if let Err(error) = result {
            println!("PATH_UNAVAILABLE reason={error} mqtt={}", traffic.report());
            if let Some(attempt) = legacy_attempt.as_mut() {
                attempt.finish_failure();
            }
            if safety.is_none()
                && let Some(round) = active_round
            {
                if let Err(reason) = report_auto_path_failure_server(&control, round).await {
                    println!("CONTROL_LIVENESS_FAILED reason={reason}");
                }
            }
            close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
            return Ok(());
        }
    }
    let waited = if let Some(attempt) = legacy_attempt.as_ref() {
        attempt.verify_winner_path(
            &control,
            gate.as_ref().ok_or("legacy requires member gate")?,
        )
    } else {
        wait_ip(&control, data_path_wait(automatic)?)
            .await
            .map_err(|error| io::Error::other(error.to_string()))
    };
    if let Err(error) = waited {
        println!("PATH_UNAVAILABLE reason={error} mqtt={}", traffic.report());
        if safety.is_none()
            && (automatic || legacy.is_some())
            && let Some(round) = active_round
        {
            if let Err(reason) = report_auto_path_failure_server(&control, round).await {
                println!("CONTROL_LIVENESS_FAILED reason={reason}");
            }
        }
        close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
        return Ok(());
    }
    if !member_eligible(&control, &gate) {
        println!("ADMISSION_REJECTED peer={peer} stable_id={stable} before_commit=1");
        close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
        return Ok(());
    }
    let mut promotion_reply = if let Some(round) = active_round {
        let received = tokio::time::timeout(
            WAIT,
            guarded_control(legacy_attempt.as_ref(), &control, &gate, async {
                let (send, mut recv) = control.accept_control_bi().await?;
                let frame = recv_frame(&mut recv).await?;
                Ok::<_, Box<dyn Error>>((send, frame))
            }),
        )
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
                close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
                return Ok(());
            }
        }
    } else {
        None
    };
    println!("PATH_OBSERVED peer={peer} stable_id={stable}");
    let committed = if let Some(attempt) = legacy_attempt.as_ref() {
        attempt
            .commit_winner_path(control, gate.as_ref().ok_or("legacy requires member gate")?)
            .await
    } else {
        control.commit_data_path().await
    };
    let data = match committed {
        Ok(data) => data,
        Err(CommitError::Retryable { error, .. } | CommitError::Terminal { error }) => {
            println!("PATH_UNAVAILABLE reason={error} mqtt={}", traffic.report());
            if let Some(attempt) = legacy_attempt.as_mut() {
                attempt.finish_failure();
            }
            close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
            return Ok(());
        }
    };
    if let (Some(mut send), Some(round)) = (promotion_reply.take(), active_round) {
        if let Err(error) = guarded_data(
            legacy_attempt.as_ref(),
            &data,
            &gate,
            send_frame(&mut send, &CoordFrame::Committed(round)),
        )
        .await
        {
            println!("PATH_UNAVAILABLE reason={error} mqtt={}", traffic.report());
            close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
            return Ok(());
        }
        println!(
            "UDP_COMMIT_ACK round={} stable_id={stable}",
            round.encoded()
        );
    }
    let before = traffic.sent_packets();
    let business = tokio::time::timeout(
        WAIT,
        guarded_data(legacy_attempt.as_ref(), &data, &gate, async {
            let (_, mut recv) = data.connection().accept_bi().await?;
            Ok::<_, Box<dyn Error>>(recv.read_to_end(64).await?)
        }),
    )
    .await;
    let business = match business {
        Ok(Ok(bytes)) => bytes,
        other => {
            println!(
                "PATH_UNAVAILABLE reason={other:?} mqtt={}",
                traffic.report()
            );
            close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
            return Ok(());
        }
    };
    if business != b"NAT_DATA" || traffic.sent_packets() != before {
        return Err("business crossed MQTT or had wrong content".into());
    }
    if let (Some(context), Some(attempt)) = (&legacy, legacy_attempt.as_mut()) {
        let learned = attempt.confirm_success(&data, gate.as_ref().unwrap(), &context.analyzer);
        println!(
            "LEGACY_RESULT outcome=confirmed_business learned={learned} score_delta={} stable_id={stable}",
            if learned { 2 } else { 0 }
        );
    }
    println!(
        "DATA_OK stable_id={stable} peer={peer} path_ms={} paths={:?} mqtt_after_commit={before}",
        path_started.elapsed().as_millis(),
        data.connection().paths()
    );
    if let Some(binding) = legacy_attempt
        .as_ref()
        .and_then(|attempt| attempt.local_winner())
    {
        if let Some(path) = data
            .connection()
            .paths()
            .iter()
            .find(|path| path.is_selected())
        {
            println!(
                "LEGACY_BUSINESS_BINDING stable_id={stable} peer={peer} path_id={} local_socket_id={} local_addr={} remote={}",
                path.id(),
                binding.id.get(),
                binding.local_addr,
                path.remote_addr()
            );
        }
    }
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
    legacy: Option<LegacyResources>,
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
    let mut safety = FailureLiveness::open(&control, false).await?;
    let legacy = match legacy {
        Some(resources) => Some(prepare_legacy(&endpoint, &own_candidate, resources).await?),
        None => None,
    };
    if std::env::var("MIOPUNCH_NAT_FAULT").as_deref() == Ok("stale") {
        endpoint.network_change().await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while own_candidate.observation.is_fresh() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        println!("STALE_CANDIDATE rejected_before_exchange stable_id={stable}");
        close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
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
    let automatic = automatic_strategy(mapping.is_some())?;
    let advanced = coordinated || automatic || legacy.is_some();
    let mut active_round = None;
    let mut auto_role = None;
    let mut legacy_attempt = None;
    let reported = if let Some(context) = &legacy {
        match nat_punch::coordinate_client(
            &control,
            gate.as_ref().ok_or("legacy requires member gate")?,
            &context.snapshot,
        )
        .await
        {
            Ok(attempt) => {
                trace_legacy(&attempt, stable);
                active_round = Some(Round::parse(&attempt.round().encoded())?);
                let target = attempt.remote_target();
                legacy_attempt = Some(attempt);
                target
            }
            Err(error) => {
                println!(
                    "COORD_UNAVAILABLE reason={error} probes=0 mqtt={}",
                    traffic.report()
                );
                close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
                return Ok(());
            }
        }
    } else if automatic {
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
                Some(remote)
            }
            Err(error) => {
                println!("COORD_UNAVAILABLE reason={error} mqtt={}", traffic.report());
                close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
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
                Some(remote)
            }
            Err(error) => {
                println!("COORD_UNAVAILABLE reason={error} mqtt={}", traffic.report());
                close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
                return Ok(());
            }
        }
    } else {
        let own_addr = own_candidate.fresh_addr()?;
        match tokio::time::timeout(WAIT, async {
            let (mut send, mut recv) = control.open_control_bi().await?;
            send.write_all(own_addr.to_string().as_bytes()).await?;
            send.finish()?;
            println!(
                "CONTROL_FRAME_SENT kind=CANDIDATE bytes={}",
                own_addr.to_string().len()
            );
            let addr = read_candidate(&mut recv).await?;
            println!(
                "CONTROL_FRAME_RECEIVED kind=CANDIDATE bytes={}",
                addr.to_string().len()
            );
            Ok::<SocketAddr, Box<dyn Error>>(addr)
        })
        .await
        {
            Ok(Ok(addr)) => Some(addr),
            other => {
                let error = format!("{other:?}");
                println!(
                    "CONTROL_UNAVAILABLE reason={error} mqtt={}",
                    traffic.report()
                );
                close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
                return Ok(());
            }
        }
    };
    let remote = override_candidate.or(reported);
    if let (Some(reported), Some(remote)) = (reported, remote) {
        println!(
            "EXCHANGED peer={peer} peer_candidate={reported} attempted={remote} stable_id={stable}"
        );
    } else {
        println!("EXCHANGED peer={peer} peer_candidate=none receive_only=1 stable_id={stable}");
    }
    if automatic {
        match std::env::var("MIOPUNCH_NAT_FAULT").as_deref() {
            Ok("network_after_plan") => {
                endpoint.network_change().await;
                // network_change queues a notification; synchronize this
                // fault with actual invalidation, rather than testing the
                // interval before the Endpoint actor processes it.
                tokio::time::timeout(Duration::from_secs(2), async {
                    while mapping.as_ref().unwrap().sequence.is_current_generation() {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await?;
                println!("FAULT_INJECTED network_changed_after_plan stable_id={stable}");
            }
            Ok("revoke_after_plan") => {
                gate.as_ref()
                    .ok_or("missing lab member gate")?
                    .update_authorization([*endpoint.id().as_bytes()], [*peer.as_bytes()]);
                println!("FAULT_INJECTED peer_revoked_after_plan stable_id={stable}");
            }
            _ => {}
        }
    }
    let path_started = Instant::now();
    if !advanced {
        control
            .prime_udp_path(remote.expect("baseline target"))
            .await?;
        println!("PROBES_SENT count=1 distinct_targets=1");
    }
    if std::env::var("MIOPUNCH_NAT_FAULT").as_deref() == Ok("path_id_retry") {
        #[cfg(feature = "nat-lab-test-hooks")]
        control.test_force_path_id_retry_once().await?;
        #[cfg(not(feature = "nat-lab-test-hooks"))]
        return Err("path_id_retry requires the nat-lab-test-hooks feature".into());
    }
    let result = if let Some(context) = &legacy {
        if let Err(error) =
            legacy_probe(&control, &gate, context, legacy_attempt.as_mut().unwrap()).await
        {
            println!("PATH_UNAVAILABLE reason={error} mqtt={}", traffic.report());
            if let Some(attempt) = legacy_attempt.as_mut() {
                attempt.finish_failure();
            }
            if safety.is_none()
                && let Some(round) = active_round
            {
                if let Err(reason) = report_auto_path_failure_client(&control, round).await {
                    println!("CONTROL_LIVENESS_FAILED reason={reason}");
                }
            }
            close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
            return Ok(());
        }
        Ok(Ok(miopunch_native_probe::control_session::DataPathOpen {
            path_id_retries: 0,
        }))
    } else if automatic {
        let probes = auto_probe_window(
            &control,
            &gate,
            mapping.as_ref().unwrap(),
            remote.expect("auto target"),
            auto_role.unwrap(),
            active_round.unwrap(),
        )
        .await;
        if let Err(error) = probes {
            println!("PATH_UNAVAILABLE reason={error} mqtt={}", traffic.report());
            close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
            return Ok(());
        }
        tokio::time::timeout(
            AUTO_PATH_WAIT,
            control.open_data_path_report(remote.expect("outbound target")),
        )
        .await
    } else if coordinated {
        let (probes, opened) = tokio::join!(
            probe_window(
                &control,
                &gate,
                &own_candidate,
                remote.expect("coordinated target")
            ),
            tokio::time::timeout(
                data_path_wait(false)?,
                control.open_data_path_report(remote.expect("outbound target"))
            )
        );
        if let Err(error) = probes {
            println!("PATH_UNAVAILABLE reason={error} mqtt={}", traffic.report());
            close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
            return Ok(());
        }
        opened
    } else {
        tokio::time::timeout(
            data_path_wait(false)?,
            control.open_data_path_report(remote.expect("outbound target")),
        )
        .await
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
            if safety.is_none()
                && (automatic || legacy.is_some())
                && let Some(round) = active_round
            {
                if let Err(reason) = report_auto_path_failure_client(&control, round).await {
                    println!("CONTROL_LIVENESS_FAILED reason={reason}");
                }
            }
            close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
            return Ok(());
        }
    }
    let waited = if let Some(attempt) = legacy_attempt.as_ref() {
        attempt.verify_winner_path(
            &control,
            gate.as_ref().ok_or("legacy requires member gate")?,
        )
    } else {
        wait_ip(&control, data_path_wait(automatic)?)
            .await
            .map_err(|error| io::Error::other(error.to_string()))
    };
    if let Err(error) = waited {
        println!("PATH_UNAVAILABLE reason={error} mqtt={}", traffic.report());
        if safety.is_none()
            && (automatic || legacy.is_some())
            && let Some(round) = active_round
        {
            if let Err(reason) = report_auto_path_failure_client(&control, round).await {
                println!("CONTROL_LIVENESS_FAILED reason={reason}");
            }
        }
        close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
        return Ok(());
    }
    if !member_eligible(&control, &gate) {
        println!("ADMISSION_REJECTED peer={peer} stable_id={stable} before_commit=1");
        close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
        return Ok(());
    }
    let committed = if let Some(attempt) = legacy_attempt.as_ref() {
        attempt
            .commit_winner_path(control, gate.as_ref().ok_or("legacy requires member gate")?)
            .await
    } else {
        control.commit_data_path().await
    };
    match committed {
        Ok(mut data) => {
            if let Some(round) = active_round {
                let promoted = tokio::time::timeout(
                    WAIT,
                    guarded_data(legacy_attempt.as_ref(), &data, &gate, async {
                        let (mut send, mut recv) = data.connection().open_bi().await?;
                        send_frame(&mut send, &CoordFrame::Promoted(round)).await?;
                        let answer = recv_frame(&mut recv).await?;
                        if answer != CoordFrame::Committed(round) {
                            return Err("NAT3 promotion acknowledgment mismatch".into());
                        }
                        Ok::<_, Box<dyn Error>>(())
                    }),
                )
                .await;
                match promoted {
                    Ok(Ok(())) => {}
                    other => {
                        println!(
                            "PATH_UNAVAILABLE reason={other:?} mqtt={}",
                            traffic.report()
                        );
                        close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
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
            let transmitted = guarded_data(legacy_attempt.as_ref(), &data, &gate, async {
                let (mut send, _) = data.connection().open_bi().await?;
                send.write_all(&queued).await?;
                send.finish()?;
                Ok::<_, Box<dyn Error>>(())
            })
            .await;
            if let Err(error) = transmitted {
                println!("PATH_UNAVAILABLE reason={error} mqtt={}", traffic.report());
                close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
                return Ok(());
            }
            // Business was submitted under the exact winner scope. The peer
            // may now close normally; this delay only measures carrier costs.
            tokio::time::sleep(Duration::from_millis(250)).await;
            if traffic.sent_packets() != before {
                return Err("business used MQTT".into());
            }
            println!(
                "DATA_OK stable_id={stable} peer={peer} path_ms={} paths={:?} mqtt_after_commit={before}",
                path_started.elapsed().as_millis(),
                data.connection().paths()
            );
            if let Some(binding) = legacy_attempt
                .as_ref()
                .and_then(|attempt| attempt.local_winner())
            {
                if let Some(path) = data
                    .connection()
                    .paths()
                    .iter()
                    .find(|path| path.is_selected())
                {
                    println!(
                        "LEGACY_BUSINESS_BINDING stable_id={stable} peer={peer} path_id={} local_socket_id={} local_addr={} remote={}",
                        path.id(),
                        binding.id.get(),
                        binding.local_addr,
                        path.remote_addr()
                    );
                }
            }
            if legacy.is_some() {
                println!(
                    "LEGACY_RESULT outcome=business_sent feedback_owner=coordinator stable_id={stable}"
                );
            }
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
    close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
    Ok(())
}

/// The normal entry delegates all selection, bounded rounds and physical path
/// ownership to the node connector. Experiment branches below remain explicit.
async fn run_formal(
    endpoint: Endpoint,
    carrier: Arc<MqttTransport>,
    traffic: Arc<Traffic>,
    gate: Option<AdmissionGate>,
    peer: PublicKey,
    mode: &str,
    route: Option<&str>,
    observers: Option<RelayMap>,
) -> Result<(), Box<dyn Error>> {
    let accepting = mode == "server";
    let control = if accepting {
        println!("READY id={} route={}", endpoint.id(), carrier.route()?);
        tokio::time::timeout(WAIT, ControlSession::accept(&endpoint)).await??
    } else {
        let route = route.ok_or("client requires control route")?;
        let target = EndpointAddr::new(peer)
            .with_addrs([TransportAddr::Custom(MqttTransport::address(route)?)]);
        tokio::time::timeout(WAIT, ControlSession::connect(&endpoint, target, ALPN)).await??
    };
    let stable = control.stable_id();
    if control.remote_id() != peer || !member_eligible(&control, &gate) {
        println!("ADMISSION_REJECTED peer={peer} stable_id={stable} probes=0");
        endpoint.close().await;
        return Ok(());
    }
    let gate = gate.ok_or("formal member gate required")?;
    println!(
        "NODE_SESSION local={} peer={peer} stable_id={stable} dialing={}",
        endpoint.id(),
        !accepting
    );
    let mut safety = FailureLiveness::open(&control, accepting).await?;
    let policy = UdpPolicy::default();
    #[cfg(feature = "nat-lab-test-hooks")]
    let policy = {
        let mut policy = policy;
        if let Ok(rounds) = std::env::var("MIOPUNCH_NAT_LAB_ROUNDS") {
            policy.max_rounds = rounds.parse()?;
            policy.additional_observations =
                usize::from(policy.max_rounds) * observers.as_ref().map_or(0, RelayMap::len);
        }
        match std::env::var("MIOPUNCH_NAT_FAULT").as_deref() {
            Ok("budget_exhausted") => policy.peer_datagrams = 0,
            Ok("udp_disabled") => policy.enabled = false,
            _ => (),
        }
        policy
    };
    let connector = NatConnector::new(observers, policy);
    let upgrade = match connector.upgrade(control, &gate).await {
        Ok(upgrade) => upgrade,
        Err(CommitError::Retryable { error, session }) => {
            println!(
                "PATH_UNAVAILABLE reason={error} stable_id={} stop=bounded_original_session",
                session.stable_id()
            );
            close_failed_lab_attempt(&endpoint, stable, &mut safety).await?;
            return Ok(());
        }
        Err(CommitError::Terminal { error }) => return Err(error.into()),
    };
    let mut upgrade = upgrade;
    let before = traffic.sent_packets();
    let payload: &[u8] = if accepting { b"NAT_ACK" } else { b"NAT_DATA" };
    let received = connector
        .exchange_upgrade_business(&mut upgrade, &gate, payload)
        .await?;
    let mut data = upgrade.data;
    let attempt = upgrade.attempt;
    if received
        != (if accepting {
            b"NAT_DATA".as_slice()
        } else {
            b"NAT_ACK".as_slice()
        })
        || traffic.sent_packets() != before
    {
        return Err("wrong business payload or MQTT business transmission".into());
    }
    if let Some(attempt) = attempt.as_ref() {
        let binding = attempt.local_winner().ok_or("missing physical winner")?;
        let paths = data.connection().paths();
        let path = paths
            .iter()
            .find(|path| path.is_selected())
            .ok_or("missing selected winner path")?;
        println!(
            "LEGACY_BUSINESS_BINDING stable_id={stable} peer={peer} path_id={} local_socket_id={} local_addr={} remote={}",
            path.id(),
            binding.id.get(),
            binding.local_addr,
            path.remote_addr()
        );
    }
    println!(
        "DATA_OK stable_id={stable} peer={peer} kind={} paths={:?} mqtt_after_commit={before}",
        if upgrade.direct.is_some() {
            "direct"
        } else {
            "punching"
        },
        data.connection().paths()
    );
    data.take_pending_business();
    // Both original peers finish their receipts before disposable lab teardown.
    tokio::time::sleep(Duration::from_millis(100)).await;
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
    let has_mapping_observers =
        std::env::var("MIOPUNCH_NAT_OBSERVERS").is_ok_and(|value| !value.is_empty());
    let automatic = automatic_strategy(has_mapping_observers)?;
    let formal = matches!(
        std::env::var("MIOPUNCH_NAT_STRATEGY").as_deref(),
        Err(_) | Ok("auto" | "formal")
    );
    let legacy_selected = std::env::var("MIOPUNCH_NAT_STRATEGY").as_deref() == Ok("legacy");
    if (automatic
        || legacy_selected
        || std::env::var("MIOPUNCH_NAT_STRATEGY").as_deref() == Ok("coordinated"))
        && std::env::var("MIOPUNCH_NAT_SECRET_FILE").is_err()
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
    #[cfg(feature = "nat-lab-test-hooks")]
    if std::env::var("MIOPUNCH_LEGACY_TTL_UNAVAILABLE").as_deref() == Ok("1") {
        endpoint.test_force_traversal_ttl_unavailable().await;
    }
    if formal {
        let route = match (mode.as_str(), tail) {
            ("server", []) => None,
            ("client", [route]) => Some(route.as_str()),
            _ => return Err("normal entry accepts server or client with one control route".into()),
        };
        return run_formal(
            endpoint,
            carrier,
            traffic,
            gate,
            peer,
            mode,
            route,
            mapping_observers,
        )
        .await;
    }
    let own_candidate = observed_addr(&endpoint).await?;
    let legacy = if legacy_selected {
        let budget = Arc::new(NodeProbeBudget::new(
            mapping_observers.as_ref().map_or(0, RelayMap::len),
            miopunch_native_probe::nat_scheduler::DEFAULT_LEGACY_PEER_ALLOWANCE,
        ));
        Some(LegacyResources {
            observers: mapping_observers.clone(),
            analyzer: Arc::new(Mutex::new(Analyzer::new(Instant::now()))),
            budget,
        })
    } else {
        None
    };
    let mapping = if automatic {
        // The endpoint's baseline QAD observation remains available to all
        // strategies; this allowance covers only the four additional QAD
        // connections and the selected one/two peer-directed datagrams.
        let probe_limit = auto_probe_limit()?;
        // Both regular sides use one application prime by default. Reserve a
        // second only for the bounded packet-budget comparison in the lab.
        let dual_limit = match std::env::var("MIOPUNCH_NAT_LAB_DUAL_FIRST_PACKETS").as_deref() {
            Ok("2") => 2,
            Ok("1") | Err(_) => 1,
            _ => return Err("lab dual-first packet count must be 1 or 2".into()),
        };
        let peer_budget =
            if std::env::var("MIOPUNCH_NAT_FAULT").as_deref() == Ok("budget_exhausted") {
                0
            } else {
                probe_limit.max(dual_limit)
            };
        let budget = Arc::new(NodeProbeBudget::new(4, peer_budget));
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
                legacy,
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
                legacy,
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
                legacy,
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
            CoordFrame::AutoOffer(MappingPattern::Irregular {
                last: addr,
                low: 42020,
                high: 42030,
            }),
            CoordFrame::Start(round),
            CoordFrame::Ack(round),
            CoordFrame::Cancel(round),
            CoordFrame::SenderPrimed(round),
            CoordFrame::SenderPrimedAck(round),
            CoordFrame::ReceiverPrimed(round),
            CoordFrame::ReceiverPrimedAck(round),
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
        for invalid in [
            b"AUTO_OFFER I 100.64.27.2:42027 0:42030".as_slice(),
            b"AUTO_OFFER I 100.64.27.2:42027 42030:42020".as_slice(),
            b"AUTO_OFFER I 100.64.27.2:42027 42020:42025".as_slice(),
            b"AUTO_OFFER I [::1]:42027 42020:42030".as_slice(),
            b"AUTO_OFFER I 100.64.27.2:42027 42020:42030 extra".as_slice(),
        ] {
            assert!(CoordFrame::parse(invalid).is_err());
        }
        assert_ne!(CoordFrame::Start(round), CoordFrame::Start(Round([0; 16])));
    }
}
