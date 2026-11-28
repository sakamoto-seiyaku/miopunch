//! Shared bounded TCP byte-stream and UDP datagram forwarding wire contract.
use crate::{
    data_stream::DataStream,
    service_policy::{Service, ServiceAccess},
};
use p2panda_core::VerifyingKey;
use serde::{Deserialize, Serialize};
use std::{io, net::SocketAddr};

pub const TCP_ALPN: &[u8] = b"miopunch/tcp-forward/1";
pub const UDP_ALPN: &[u8] = b"miopunch/udp-forward/1";
pub const MAX_DATAGRAM: usize = 65507;
pub const MAX_EXCHANGE: usize = 60 * 1024;

/// Limits for retained reliable channels. Send and receive each get a node pool,
/// so simultaneous bidirectional traffic can always release the peer's credit.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ChannelPolicy {
    pub recovery_ms: u64,
    pub channel_bytes: usize,
    pub node_bytes_per_direction: usize,
}
impl Default for ChannelPolicy {
    fn default() -> Self {
        Self {
            recovery_ms: 60_000,
            channel_bytes: 256 * 1024,
            node_bytes_per_direction: 2 * 1024 * 1024,
        }
    }
}
impl ChannelPolicy {
    pub(crate) fn validate(&self) -> io::Result<()> {
        if self.recovery_ms == 0
            || self.channel_bytes == 0
            || self.node_bytes_per_direction < self.channel_bytes
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid business channel limits",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    Tcp,
    Udp,
}
impl Protocol {
    pub fn service(self) -> Service {
        match self {
            Self::Tcp => Service::TcpForward,
            Self::Udp => Service::UdpForward,
        }
    }
    pub fn alpn(self) -> &'static [u8] {
        match self {
            Self::Tcp => TCP_ALPN,
            Self::Udp => UDP_ALPN,
        }
    }
    pub fn from_alpn(alpn: &[u8]) -> Option<Self> {
        match alpn {
            TCP_ALPN => Some(Self::Tcp),
            UDP_ALPN => Some(Self::Udp),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelOrigin {
    pub connection: [u8; 32],
    pub stream: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelRequest {
    pub origin: ChannelOrigin,
    pub id: String,
    pub resume: bool,
    #[serde(default)]
    pub generation: u64,
}

/// Opening result is independent of which carrier currently owns the IO.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelReady {
    Connected { attached: bool },
    Ended,
    OpenOutcomeUnknown,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Open {
    pub target: SocketAddr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<ChannelRequest>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ready {
    pub connected: bool,
}

/// Admission can support either direction. Only the actual provider authorizes
/// target effects; a consumer need not enable its own unrelated provider.
pub fn peer_permitted(access: &ServiceAccess, peer: VerifyingKey, protocol: Protocol) -> bool {
    access.can_contact_service(protocol.service(), peer)
}

pub async fn open(
    connection: &iroh::endpoint::Connection,
    target: SocketAddr,
) -> io::Result<DataStream> {
    open_on_stream(DataStream::open(connection).await?, target).await
}

pub(crate) async fn open_on_stream(mut stream: DataStream, target: SocketAddr) -> io::Result<DataStream> {
    stream.write_control(&Open { target, channel: None }).await?;
    if !stream.read_control::<Ready>().await?.connected {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "forward target unavailable",
        ));
    }
    Ok(stream)
}

/// A bounded one-request/one-response operation useful in a browser, which
/// cannot create a general host listener. TCP callers choose a response length;
/// UDP retains one whole datagram, including an empty datagram.
pub async fn exchange(
    mut stream: DataStream,
    protocol: Protocol,
    payload: &[u8],
    response_bytes: usize,
) -> io::Result<Vec<u8>> {
    if payload.len() > MAX_EXCHANGE || response_bytes > MAX_EXCHANGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "exchange too large",
        ));
    }
    stream.write_chunk(payload).await?;
    let mut response = Vec::new();
    loop {
        let bytes = stream.read_chunk().await?;
        if response.len() + bytes.len() > MAX_EXCHANGE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "response too large",
            ));
        }
        response.extend(bytes);
        if protocol == Protocol::Udp || response.len() >= response_bytes {
            break;
        }
    }
    Ok(response)
}
