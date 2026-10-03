//! Explicit member-path wire versions; application payloads retain their format.
//! Negotiation runs on the already authenticated original QUIC connection.
use std::io;

use iroh::{
    RelayUrl,
    endpoint::{ConnectOptions, Connection, RecvStream, SendStream},
};

/// Existing legacy relay validation wire, shared by native and browser hooks.
/// Its private stream belongs to the already authenticated original M.
pub struct RelayValidation {
    pub candidates: Vec<RelayUrl>,
    pub stream_id: String,
    send: SendStream,
    recv: RecvStream,
    stable_id: usize,
    binding: [u8; 32],
}
pub struct RelayContinuity {
    pub stable_unchanged: bool,
    pub binding_unchanged: bool,
}

fn relay_binding(connection: &Connection) -> io::Result<[u8; 32]> {
    let mut binding = [0; 32];
    connection
        .export_keying_material(&mut binding, b"miopunch/member-path-continuity/1", b"")
        .map_err(|_| io::Error::other("original member exporter unavailable"))?;
    Ok(binding)
}

impl RelayValidation {
    pub async fn begin(connection: &Connection, local: Vec<RelayUrl>) -> io::Result<Self> {
        let stable_id = connection.stable_id();
        let binding = relay_binding(connection)?;
        let (mut send, mut recv) = if connection.side().is_client() {
            connection.open_bi().await?
        } else {
            connection.accept_bi().await?
        };
        let encoded = serde_json::to_vec(&local).map_err(io::Error::other)?;
        if local.len() > 4 || encoded.len() > 1024 {
            return Err(io::Error::other("relay candidate message too large"));
        }
        send.write_all(&(encoded.len() as u16).to_be_bytes())
            .await?;
        send.write_all(&encoded).await?;
        let mut length = [0; 2];
        recv.read_exact(&mut length)
            .await
            .map_err(io::Error::other)?;
        let length = usize::from(u16::from_be_bytes(length));
        if length > 1024 {
            return Err(io::Error::other("peer relay candidate message too large"));
        }
        let mut encoded = vec![0; length];
        recv.read_exact(&mut encoded)
            .await
            .map_err(io::Error::other)?;
        let candidates: Vec<RelayUrl> =
            serde_json::from_slice(&encoded).map_err(io::Error::other)?;
        if candidates.len() > 4 {
            return Err(io::Error::other("too many peer relay candidates"));
        }
        Ok(Self {
            candidates,
            stream_id: send.id().to_string(),
            send,
            recv,
            stable_id,
            binding,
        })
    }

    /// Strict legacy one-byte bilateral validation; no wire format upgrade.
    /// Only the boolean continuity result may enter diagnostics.
    pub async fn finish(
        mut self,
        connection: &Connection,
        validation: io::Result<()>,
    ) -> io::Result<RelayContinuity> {
        self.send.write_all(&[u8::from(validation.is_ok())]).await?;
        self.send.finish()?;
        let peer = self.recv.read_to_end(1).await.map_err(io::Error::other)?;
        validation?;
        if peer != [1] {
            return Err(io::Error::other("relay validation failed at peer"));
        }
        let stable_unchanged = connection.stable_id() == self.stable_id;
        let binding_unchanged = relay_binding(connection)? == self.binding;
        if !stable_unchanged || !binding_unchanged {
            return Err(io::Error::other("relay original member continuity changed"));
        }
        Ok(RelayContinuity {
            stable_unchanged,
            binding_unchanged,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemberProtocol {
    Business,
    Replica,
}

impl MemberProtocol {
    pub const fn legacy_alpn(self) -> &'static [u8] {
        match self {
            Self::Business => b"miopunch/browser-member/1",
            Self::Replica => b"miopunch/control-replica/1",
        }
    }

    pub const fn upgraded_alpn(self) -> &'static [u8] {
        match self {
            Self::Business => b"miopunch/browser-member/rtc-path/3",
            Self::Replica => b"miopunch/control-replica/rtc-path/3",
        }
    }

    /// Explicit server preference, independent of identifier sorting.
    /// A Router registering both handlers applies this list with set_alpns.
    pub fn preferred_alpns(self) -> Vec<Vec<u8>> {
        vec![self.upgraded_alpn().to_vec(), self.legacy_alpn().to_vec()]
    }

    /// Keep the original application ALPN as the before-connect gate input;
    /// TLS chooses the server's explicit preference on this same connection.
    pub fn connect_options(self) -> ConnectOptions {
        ConnectOptions::new().with_additional_alpns(vec![self.upgraded_alpn().to_vec()])
    }

    pub fn from_wire(alpn: &[u8]) -> Option<(Self, CapabilitiesVersion)> {
        [Self::Business, Self::Replica]
            .into_iter()
            .find_map(|protocol| {
                if alpn == protocol.upgraded_alpn() {
                    Some((protocol, CapabilitiesVersion::Rtc3))
                } else if alpn == protocol.legacy_alpn() {
                    Some((protocol, CapabilitiesVersion::Legacy2))
                } else {
                    None
                }
            })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilitiesVersion {
    Legacy2,
    Rtc3,
}

impl CapabilitiesVersion {
    pub const fn number(self) -> u8 {
        match self {
            Self::Legacy2 => 2,
            Self::Rtc3 => 3,
        }
    }
}

/// A capability of this M's actual role/installed adapter, never a node flag.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RtcRole {
    None = 0,
    Offerer = 1,
    Answerer = 2,
}

impl RtcRole {
    pub fn browser_adapter(connection: &Connection, installed_and_allowed: bool) -> Self {
        if installed_and_allowed && connection.side().is_client() {
            Self::Offerer
        } else {
            Self::None
        }
    }

    pub fn native_adapter(connection: &Connection, installed_and_allowed: bool) -> Self {
        if installed_and_allowed && connection.side().is_server() {
            Self::Answerer
        } else {
            Self::None
        }
    }

    fn decode(value: u8) -> io::Result<Self> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::Offerer),
            2 => Ok(Self::Answerer),
            _ => Err(invalid_capabilities()),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PathCapabilities {
    pub tcp: bool,
    pub udp: bool,
    pub relay: bool,
    pub rtc: RtcRole,
}

#[derive(Clone, Copy, Debug)]
pub struct NegotiatedPaths {
    pub version: CapabilitiesVersion,
    pub tcp: bool,
    pub udp: bool,
    pub relay: bool,
    pub rtc: bool,
    pub local_rtc: RtcRole,
    pub peer_rtc: RtcRole,
}

impl NegotiatedPaths {
    pub fn require_usable(self) -> io::Result<Self> {
        if self.tcp || self.udp || self.relay || self.rtc {
            Ok(self)
        } else {
            Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "temporarily unreachable: no mutually permitted member data path",
            ))
        }
    }
}

fn invalid_capabilities() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid protected data path capabilities",
    )
}

/// Use the negotiated TLS ALPN to choose the wire version before writing.
/// Other existing native application ALPNs retain their original v2 exchange.
/// The caller owns the single attempt deadline and connection cancellation.
pub async fn exchange_capabilities(
    connection: &Connection,
    local: PathCapabilities,
) -> io::Result<NegotiatedPaths> {
    let version = MemberProtocol::from_wire(connection.alpn())
        .map_or(CapabilitiesVersion::Legacy2, |(_, version)| version);
    let local_rtc = if version == CapabilitiesVersion::Rtc3 {
        local.rtc
    } else {
        RtcRole::None
    };
    let mut bytes = vec![
        version.number(),
        u8::from(local.tcp),
        u8::from(local.udp),
        u8::from(local.relay),
    ];
    if version == CapabilitiesVersion::Rtc3 {
        bytes.push(local_rtc as u8);
    }
    let (mut send, mut recv) = if connection.side().is_client() {
        connection.open_bi().await?
    } else {
        connection.accept_bi().await?
    };
    send.write_all(&bytes).await?;
    send.finish()?;
    let peer = recv
        .read_to_end(bytes.len())
        .await
        .map_err(io::Error::other)?;
    if peer.len() != bytes.len()
        || peer[0] != version.number()
        || peer[1..4].iter().any(|value| *value > 1)
    {
        return Err(invalid_capabilities());
    }
    let peer_rtc = if version == CapabilitiesVersion::Rtc3 {
        RtcRole::decode(peer[4])?
    } else {
        RtcRole::None
    };
    Ok(NegotiatedPaths {
        version,
        tcp: local.tcp && peer[1] == 1,
        udp: local.udp && peer[2] == 1,
        relay: local.relay && peer[3] == 1,
        rtc: matches!(
            (local_rtc, peer_rtc),
            (RtcRole::Offerer, RtcRole::Answerer) | (RtcRole::Answerer, RtcRole::Offerer)
        ),
        local_rtc,
        peer_rtc,
    })
}
