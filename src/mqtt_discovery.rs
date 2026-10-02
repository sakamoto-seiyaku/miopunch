//! Bounded, public MQTT candidate exchange for an already joined network.
//! A response is only a route hint. TLS, admission proof, and signed history
//! still decide whether the responding endpoint is a member.

use std::{
    io,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    time::Duration,
};

use iroh_base::EndpointId;
use rumqttc::mqttbytes::v4::SubscribeReasonCode;
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use sha2::{Digest, Sha256};

use crate::admission_proof::AdmissionScope;

const QUERY: &[u8; 4] = b"MDQ1";
const RESPONSE: &[u8; 4] = b"MDR1";
const TAGGED_RESPONSE: &[u8; 4] = b"MDR2";
const MAX_CANDIDATES: usize = 16;
const DISCOVERY_WINDOW: Duration = Duration::from_secs(2);

fn namespace(scope: &AdmissionScope) -> String {
    let mut hash = Sha256::new();
    hash.update(b"miopunch/mqtt-discovery/1\0");
    hash.update(scope.network_id());
    hash.update(scope.secret_generation().to_be_bytes());
    format!("miopunch/bootstrap/1/{}", hex::encode(hash.finalize()))
}

fn client(broker: SocketAddr) -> (AsyncClient, rumqttc::EventLoop) {
    let mut options = MqttOptions::new(
        format!("mio26-{}", rand::random::<u64>()),
        broker.ip().to_string(),
        broker.port(),
    );
    options.set_keep_alive(Duration::from_secs(5));
    options.set_clean_session(true);
    options.set_max_packet_size(1024, 1024);
    AsyncClient::new(options, 32)
}

async fn subscribed(events: &mut rumqttc::EventLoop) -> io::Result<()> {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if let Event::Incoming(Packet::SubAck(ack)) =
                events.poll().await.map_err(io::Error::other)?
            {
                if ack.return_codes != [SubscribeReasonCode::Success(QoS::AtMostOnce)] {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "discovery subscription rejected",
                    ));
                }
                return Ok(());
            }
        }
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "discovery subscribe timeout"))?
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Candidate<const N: usize = 16> {
    pub id: EndpointId,
    pub route: [u8; N],
    pub udp_addr: SocketAddrV4,
}

impl<const N: usize> Candidate<N> {
    fn encode(self, nonce: &[u8; 16]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(58 + N);
        bytes.extend_from_slice(if N == 32 { TAGGED_RESPONSE } else { RESPONSE });
        bytes.extend_from_slice(nonce);
        bytes.extend_from_slice(self.id.as_bytes());
        bytes.extend_from_slice(&self.route);
        bytes.extend_from_slice(&self.udp_addr.ip().octets());
        bytes.extend_from_slice(&self.udp_addr.port().to_be_bytes());
        bytes
    }

    fn decode(bytes: &[u8], nonce: &[u8; 16]) -> Option<Self> {
        let version = if N == 32 { TAGGED_RESPONSE } else { RESPONSE };
        if !matches!(N, 16 | 32)
            || bytes.len() != 58 + N
            || &bytes[..4] != version
            || &bytes[4..20] != nonce
        {
            return None;
        }
        let id = EndpointId::from_bytes(bytes[20..52].try_into().ok()?).ok()?;
        let route = bytes[52..52 + N].try_into().ok()?;
        let udp_addr = SocketAddrV4::new(
            Ipv4Addr::from(<[u8; 4]>::try_from(&bytes[52 + N..56 + N]).ok()?),
            u16::from_be_bytes(bytes[56 + N..58 + N].try_into().ok()?),
        );
        (udp_addr.port() != 0 && !udp_addr.ip().is_unspecified() && !udp_addr.ip().is_multicast())
            .then_some(Self {
                id,
                route,
                udp_addr,
            })
    }
}

/// A responder subscribes before reporting ready. Its task can be aborted
/// when this node stops being available on the selected message resource.
pub async fn respond<const N: usize>(
    broker: SocketAddr,
    scope: AdmissionScope,
    candidate: Candidate<N>,
) -> io::Result<tokio::task::JoinHandle<()>> {
    respond_live(broker, scope, move || Some(candidate)).await
}

pub(crate) async fn respond_live<const N: usize>(
    broker: SocketAddr,
    scope: AdmissionScope,
    candidate: impl Fn() -> Option<Candidate<N>> + Send + 'static,
) -> io::Result<tokio::task::JoinHandle<()>> {
    let prefix = namespace(&scope);
    let query_topic = format!("{prefix}/query");
    let (mqtt, mut events) = client(broker);
    mqtt.subscribe(&query_topic, QoS::AtMostOnce)
        .await
        .map_err(io::Error::other)?;
    subscribed(&mut events).await?;
    Ok(tokio::spawn(async move {
        while let Ok(event) = events.poll().await {
            let Event::Incoming(Packet::Publish(publish)) = event else {
                continue;
            };
            if publish.topic != query_topic
                || publish.retain
                || publish.payload.len() != 20
                || &publish.payload[..4] != QUERY
            {
                continue;
            }
            let Some(candidate) = candidate() else {
                continue;
            };
            let nonce: [u8; 16] = publish.payload[4..20].try_into().unwrap();
            let reply = format!("{prefix}/reply/{}", hex::encode(nonce));
            if mqtt
                .try_publish(reply, QoS::AtMostOnce, false, candidate.encode(&nonce))
                .is_err()
            {
                break;
            }
        }
    }))
}

/// Gather a bounded set of hints. The random reply route expires when this
/// call returns; retained or late replies never become reusable authority.
pub async fn discover(broker: SocketAddr, scope: &AdmissionScope) -> io::Result<Vec<Candidate>> {
    discover_routes::<16>(broker, scope).await
}

pub(crate) async fn discover_routes<const N: usize>(
    broker: SocketAddr,
    scope: &AdmissionScope,
) -> io::Result<Vec<Candidate<N>>> {
    let prefix = namespace(scope);
    let nonce: [u8; 16] = rand::random();
    let reply = format!("{prefix}/reply/{}", hex::encode(nonce));
    let (mqtt, mut events) = client(broker);
    mqtt.subscribe(&reply, QoS::AtMostOnce)
        .await
        .map_err(io::Error::other)?;
    subscribed(&mut events).await?;
    let mut query = [0; 20];
    query[..4].copy_from_slice(QUERY);
    query[4..].copy_from_slice(&nonce);
    mqtt.publish(format!("{prefix}/query"), QoS::AtMostOnce, false, query)
        .await
        .map_err(io::Error::other)?;
    let mut found = Vec::new();
    let deadline = tokio::time::Instant::now() + DISCOVERY_WINDOW;
    while found.len() < MAX_CANDIDATES {
        let Ok(event) = tokio::time::timeout_at(deadline, events.poll()).await else {
            break;
        };
        let Event::Incoming(Packet::Publish(publish)) = event.map_err(io::Error::other)? else {
            continue;
        };
        if publish.topic != reply || publish.retain {
            continue;
        }
        if let Some(candidate) = Candidate::decode(&publish.payload, &nonce) {
            if !found
                .iter()
                .any(|old: &Candidate<N>| old.id == candidate.id && old.route == candidate.route)
            {
                found.push(candidate);
            }
        }
    }
    Ok(found)
}
