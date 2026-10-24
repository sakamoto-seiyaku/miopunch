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

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Open {
    pub target: SocketAddr,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ready {
    pub connected: bool,
}

/// Admission can support either direction. Only the actual provider authorizes
/// target effects; a consumer need not enable its own unrelated provider.
pub fn peer_permitted(access: &ServiceAccess, peer: VerifyingKey, protocol: Protocol) -> bool {
    access.member(access.local) && access.member(peer)
        || access
            .guest_pairs_for(protocol.service())
            .iter()
            .any(|(id, _)| id == peer.as_bytes())
}

pub async fn open(
    connection: &iroh::endpoint::Connection,
    target: SocketAddr,
) -> io::Result<DataStream> {
    let mut stream = DataStream::open(connection).await?;
    stream.write_control(&Open { target }).await?;
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
