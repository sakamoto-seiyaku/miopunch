//! A bounded unauthenticated STUN Binding transaction over a retained TCP
//! socket. RFC 8489 sections 5, 6.2.2 and 14.2; no credentials or member data
//! are disclosed. The result is an untrusted candidate, never reachability.
use std::{
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

/// The caller retains this real observation socket through path validation.
#[derive(Debug)]
pub(crate) struct TcpSocketObservation {
    pub(crate) destination: SocketAddr,
    pub(crate) listener: SocketAddr,
    pub(crate) local_addr: SocketAddr,
    pub(crate) mapped: SocketAddr,
    pub(crate) observed_at: std::time::Instant,
    pub(crate) stream: TcpStream,
    pub(crate) generation: iroh::endpoint::ObservationValidity,
    pub(crate) request: tokio_util::sync::CancellationToken,
}

impl TcpSocketObservation {
    pub(crate) fn is_fresh(&self) -> bool {
        self.generation.is_current()
            && !self.request.is_cancelled()
            && self.observed_at.elapsed() < std::time::Duration::from_secs(40)
            && matches!(self.stream.try_read(&mut [0; 1]), Err(error) if error.kind() == io::ErrorKind::WouldBlock)
    }
}

pub(crate) trait ObservationEndpoint: Sync {
    fn udp_network_generation(&self) -> iroh::endpoint::ObservationValidity;
    fn dns_resolver(&self) -> Result<&iroh::dns::DnsResolver, iroh::endpoint::EndpointError>;
    fn is_closed(&self) -> bool;
}
macro_rules! observation_endpoint {
    ($ty:ty) => {
        impl ObservationEndpoint for $ty {
            fn udp_network_generation(&self) -> iroh::endpoint::ObservationValidity {
                <$ty>::udp_network_generation(self)
            }
            fn dns_resolver(
                &self,
            ) -> Result<&iroh::dns::DnsResolver, iroh::endpoint::EndpointError> {
                <$ty>::dns_resolver(self)
            }
            fn is_closed(&self) -> bool {
                <$ty>::is_closed(self)
            }
        }
    };
}
observation_endpoint!(iroh::Endpoint);
observation_endpoint!(iroh::endpoint::RestrictedEndpoint);
observation_endpoint!(iroh::endpoint::ControlHandle);

/// Resolve within the transaction bound and preserve the actual listener port.
pub(crate) async fn observe(
    endpoint: &(impl ObservationEndpoint + ?Sized),
    transport: &crate::tcp_transport::TcpTransport,
    observer: &iroh::RelayUrl,
) -> io::Result<TcpSocketObservation> {
    use std::time::{Duration, Instant};
    let origin = crate::resources::resource_origin(observer.as_str())
        .ok_or_else(|| io::Error::other("invalid TCP STUN origin"))?;
    let request = transport.begin_observation(&origin);
    // A dropped future cannot later publish a result. Successful callers retain
    // the token/socket and explicitly recheck freshness before candidate use.
    let cancel_on_drop = request.clone().drop_guard();
    let generation = endpoint.udp_network_generation();
    let operation = async {
        let host = observer
            .host_str()
            .ok_or_else(|| io::Error::other("missing STUN host"))?;
        let ip = if let Ok(ip) = host.parse::<Ipv4Addr>() {
            IpAddr::V4(ip)
        } else {
            endpoint
                .dns_resolver()
                .map_err(io::Error::other)?
                .lookup_ipv4(host.to_owned(), Duration::from_secs(2))
                .await
                .map_err(io::Error::other)?
                .next()
                .ok_or_else(|| io::Error::other("STUN has no IPv4 address"))?
        };
        let destination = SocketAddr::new(ip, observer.port().unwrap_or(3478));
        if destination.port() == 0 {
            return Err(io::Error::other("invalid STUN port"));
        }
        let listener = transport
            .bound_socket_addr()
            .ok_or_else(|| io::Error::other("TCP transport not bound"))?;
        let (mapped, stream) = transport.observe(destination).await?;
        let local_addr = stream.local_addr()?;
        if stream.peer_addr()? != destination
            || local_addr.port() != listener.port()
            || endpoint.is_closed()
            || !generation.is_current()
            || request.is_cancelled()
        {
            return Err(io::Error::other(
                "TCP STUN transport changed or source mismatched",
            ));
        }
        Ok(TcpSocketObservation {
            destination,
            listener,
            local_addr,
            mapped,
            observed_at: Instant::now(),
            stream,
            generation,
            request: request.clone(),
        })
    };
    let result = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::select! {
            _ = request.cancelled() => Err(io::Error::new(io::ErrorKind::Interrupted, "TCP STUN query superseded")),
            result = operation => result,
        }
    }).await.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "TCP STUN Binding timed out"))??;
    cancel_on_drop.disarm();
    Ok(result)
}

pub(crate) async fn binding(mut stream: TcpStream) -> io::Result<(SocketAddr, TcpStream)> {
    let transaction: [u8; 12] = rand::random();
    let mut request = vec![0, 1, 0, 0, 0x21, 0x12, 0xa4, 0x42];
    request.extend(transaction);
    stream.write_all(&request).await?;
    let mut header = [0; 20];
    stream.read_exact(&mut header).await?;
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid STUN TCP binding response",
        )
    };
    let length = u16::from_be_bytes(header[2..4].try_into().unwrap()) as usize;
    if header[..2] != [1, 1]
        || header[4..8] != request[4..8]
        || header[8..] != transaction
        || length > 1024
        || length % 4 != 0
    {
        return Err(invalid());
    }
    let mut attributes = vec![0; length];
    stream.read_exact(&mut attributes).await?;
    let mut offset = 0;
    let mut candidate = None;
    while offset < length {
        if offset + 4 > length {
            return Err(invalid());
        }
        let kind = u16::from_be_bytes(attributes[offset..offset + 2].try_into().unwrap());
        let size =
            u16::from_be_bytes(attributes[offset + 2..offset + 4].try_into().unwrap()) as usize;
        let end = offset + 4 + size;
        if offset + 4 + ((size + 3) & !3) > length {
            return Err(invalid());
        }
        let value = &attributes[offset + 4..end];
        if kind == 0x0020 {
            if candidate.is_some() || size < 4 || value[0] != 0 {
                return Err(invalid());
            }
            let port = u16::from_be_bytes(value[2..4].try_into().unwrap()) ^ 0x2112;
            let address = match (value[1], size) {
                (1, 8) => {
                    let ip = Ipv4Addr::from(
                        u32::from_be_bytes(value[4..8].try_into().unwrap()) ^ 0x2112a442,
                    );
                    if ip.is_broadcast() {
                        return Err(invalid());
                    }
                    IpAddr::V4(ip)
                }
                (2, 20) => {
                    let mut bytes = [0; 16];
                    for index in 0..16 {
                        bytes[index] = value[4 + index] ^ request[4 + index];
                    }
                    IpAddr::V6(Ipv6Addr::from(bytes))
                }
                _ => return Err(invalid()),
            };
            if port == 0 || address.is_unspecified() || address.is_multicast() {
                return Err(invalid());
            }
            candidate = Some(SocketAddr::from((address, port)));
        } else if kind < 0x8000 {
            return Err(invalid());
        }
        offset += 4 + ((size + 3) & !3);
    }
    if offset != length {
        return Err(invalid());
    }
    Ok((candidate.ok_or_else(invalid)?, stream))
}
