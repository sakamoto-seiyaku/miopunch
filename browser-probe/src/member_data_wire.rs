//! Existing native MID1 client envelope. This is a port, not a new protocol.
use std::io;

pub(super) const MAGIC: &[u8; 4] = b"MID1";
pub(super) const DATA_TRANSPORT_ID: u64 = 0x6d696f646174;
pub(super) const MAX_DATAGRAM: usize = 2048;
pub(super) const QUEUE_DEPTH: usize = 64;
pub(super) const BYTES_PER_SECOND: usize = 1024 * 1024;
pub(super) const PACKETS_PER_SECOND: usize = 4096;
pub(super) type Route = [u8; 16];

pub(super) fn hello(own: Route) -> [u8; 20] {
    let mut bytes = [0; 20];
    bytes[..4].copy_from_slice(MAGIC);
    bytes[4..].copy_from_slice(&own);
    bytes
}

pub(super) fn address_data(via: &[u8; 32], route: Route) -> [u8; 48] {
    let mut bytes = [0; 48];
    bytes[..32].copy_from_slice(via);
    bytes[32..].copy_from_slice(&route);
    bytes
}

pub(super) fn route(id: u64, bytes: &[u8], via: &[u8; 32]) -> io::Result<Route> {
    if id != DATA_TRANSPORT_ID || bytes.len() != 48 || bytes[..32] != *via {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "wrong MID1 data route",
        ));
    }
    Ok(bytes[32..].try_into().expect("checked MID1 route length"))
}

pub(super) fn header(peer: Route, bytes: usize) -> io::Result<[u8; 18]> {
    if bytes == 0 || bytes > MAX_DATAGRAM {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid MID1 datagram size",
        ));
    }
    let mut header = [0; 18];
    header[..16].copy_from_slice(&peer);
    header[16..].copy_from_slice(&(bytes as u16).to_be_bytes());
    Ok(header)
}

pub(super) fn parse_header(header: &[u8; 18]) -> io::Result<(Route, usize)> {
    let length = usize::from(u16::from_be_bytes(header[16..].try_into().unwrap()));
    if length == 0 || length > MAX_DATAGRAM {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid MID1 datagram size",
        ));
    }
    Ok((header[..16].try_into().unwrap(), length))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_hello_is_exactly_magic_then_one_random_route() {
        let route = [0x71; 16];
        let mut native = b"MID1".to_vec();
        native.extend_from_slice(&route);
        assert_eq!(hello(route).as_slice(), native);
        assert_ne!(&hello(route)[..4], b"MIM1");
    }

    #[test]
    fn native_address_preserves_forwarding_identity_and_route() {
        let via = [0x53; 32];
        let own = [0x27; 16];
        let address = address_data(&via, own);
        assert_eq!(route(DATA_TRANSPORT_ID, &address, &via).unwrap(), own);
        assert!(route(0x6d696f6d656d, &address, &via).is_err());
        assert!(route(DATA_TRANSPORT_ID, &address[..47], &via).is_err());
        assert!(route(DATA_TRANSPORT_ID, &address, &[0x54; 32]).is_err());
    }

    #[test]
    fn native_datagram_header_and_bounded_payload_length_match() {
        let peer = [0x42; 16];
        for length in [1, 1200, MAX_DATAGRAM] {
            let encoded = header(peer, length).unwrap();
            assert_eq!(&encoded[..16], peer);
            assert_eq!(&encoded[16..], &(length as u16).to_be_bytes());
            assert_eq!(parse_header(&encoded).unwrap(), (peer, length));
        }
    }

    #[test]
    fn zero_and_oversized_lengths_cannot_allocate_or_enqueue_payloads() {
        for length in [0, MAX_DATAGRAM + 1, u16::MAX as usize] {
            assert!(header([0; 16], length).is_err());
            let mut raw = [0; 18];
            raw[16..].copy_from_slice(&(length as u16).to_be_bytes());
            assert_eq!(
                parse_header(&raw).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
    }
}
