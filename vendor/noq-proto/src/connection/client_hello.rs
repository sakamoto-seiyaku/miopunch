use std::ops::Range;

/// Locate the latter half of the SNI hostname in the first TLS ClientHello.
///
/// QUIC CRYPTO frames carry raw handshake messages, without TLS record headers.
/// The sending order follows quic-go #5107.
pub(super) fn sni_tail(data: &[u8]) -> Option<Range<usize>> {
    let [1, high, middle, low, body @ ..] = data else {
        return None;
    };
    let hello_len = u32::from_be_bytes([0, *high, *middle, *low]) as usize;
    let mut hello = body.get(..hello_len)?.get(34..)?; // version and random
    take_vector::<1>(&mut hello)?; // session ID
    take_vector::<2>(&mut hello)?; // cipher suites
    take_vector::<1>(&mut hello)?; // compression methods
    let mut extensions = take_vector::<2>(&mut hello)?;
    let extensions_end = 4 + hello_len - hello.len();
    while !extensions.is_empty() {
        let (kind, next) = extensions.split_at_checked(2)?;
        extensions = next;
        let extension = take_vector::<2>(&mut extensions)?;
        if kind != [0, 0] {
            continue;
        }

        let mut extension = extension;
        let mut names = take_vector::<2>(&mut extension)?;
        let names_end = extensions_end - extensions.len() - extension.len();
        while let Some((&name_type, rest)) = names.split_first() {
            names = rest;
            let name = take_vector::<2>(&mut names)?;
            if name_type == 0 && name.len() >= 2 {
                let end = names_end - names.len();
                return Some(end - name.len().div_ceil(2)..end);
            }
        }
        return None;
    }
    None
}

fn take_vector<'a, const WIDTH: usize>(input: &mut &'a [u8]) -> Option<&'a [u8]> {
    let (length, rest) = input.split_at_checked(WIDTH)?;
    let length = length
        .iter()
        .fold(0usize, |length, &byte| (length << 8) | usize::from(byte));
    let (value, remaining) = rest.split_at_checked(length)?;
    *input = remaining;
    Some(value)
}

#[cfg(all(test, feature = "ring"))]
mod tests {
    use std::{collections::VecDeque, sync::Arc};

    use bytes::Bytes;
    use rustls::{RootCertStore, crypto::ring, quic};

    use crate::{TokenMemoryCache, connection::ConnectionSide};

    #[test]
    fn rustls_client_hello_is_reassembled_from_original_offsets() {
        let host = "qad.example.test";
        let mut provider = ring::default_provider();
        provider.kx_groups = vec![ring::kx_group::X25519];
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(provider))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_root_certificates(RootCertStore::empty())
            .with_no_client_auth();
        let mut tls = quic::ClientConnection::new(
            Arc::new(config),
            quic::Version::V1,
            host.try_into().unwrap(),
            vec![],
        )
        .unwrap();
        let mut hello = Vec::new();
        tls.write_hs(&mut hello);
        let hello = Bytes::from(hello);
        // Rustls's supported_groups vector contains one group, X25519 (29).
        assert!(
            hello
                .windows(8)
                .any(|bytes| bytes == [0, 10, 0, 4, 0, 2, 0, 29])
        );

        let mut pending = VecDeque::new();
        let side = |enabled| ConnectionSide::Client {
            token: Bytes::new(),
            token_store: Arc::new(TokenMemoryCache::default()),
            server_name: host.into(),
            initial_sni_fragmentation: enabled,
        };
        side(false).queue_client_hello(&mut pending, hello.clone());
        assert_eq!(pending.len(), 1);
        let whole = pending.pop_front().unwrap();
        assert_eq!(whole.offset, 0);
        assert_eq!(whole.data, hello);

        side(true).queue_client_hello(&mut pending, hello.clone());
        assert_eq!(pending.len(), 3);
        let tail = pending.back().unwrap();
        let half = host.len() / 2;
        assert_eq!(&tail.data[..], &host.as_bytes()[half..]);
        assert_eq!(pending[0].offset, 0);
        assert_eq!(pending[0].data.len(), tail.offset as usize);
        assert_eq!(pending[1].offset, tail.offset + tail.data.len() as u64);
        let mut reconstructed = vec![0; hello.len()];
        for fragment in pending {
            let offset = fragment.offset as usize;
            reconstructed[offset..offset + fragment.data.len()].copy_from_slice(&fragment.data);
        }
        assert_eq!(reconstructed, hello);
    }
}
