//! A bounded unauthenticated STUN Binding transaction over a retained TCP
//! socket. RFC 8489 sections 5, 6.2.2 and 14.2; no credentials or member data
//! are disclosed. The result is an untrusted candidate, never reachability.
use std::{
    io,
    net::{Ipv4Addr, SocketAddr},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

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
        if end > length {
            return Err(invalid());
        }
        let value = &attributes[offset + 4..end];
        if kind == 0x0020 {
            if candidate.is_some() || size != 8 || value[..2] != [0, 1] {
                return Err(invalid());
            }
            let port = u16::from_be_bytes(value[2..4].try_into().unwrap()) ^ 0x2112;
            let address = u32::from_be_bytes(value[4..8].try_into().unwrap()) ^ 0x2112a442;
            let address = Ipv4Addr::from(address);
            if port == 0
                || address.is_unspecified()
                || address.is_multicast()
                || address.is_broadcast()
            {
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
