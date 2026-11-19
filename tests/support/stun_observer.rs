use std::net::SocketAddr;

pub async fn reply_one_stun(socket: &tokio::net::UdpSocket) -> SocketAddr {
    let mut request = [0; 1500];
    let (len, source) = socket.recv_from(&mut request).await.unwrap();
    assert_eq!(len, 20);
    assert_eq!(&request[..8], &[0, 1, 0, 0, 0x21, 0x12, 0xa4, 0x42]);
    let SocketAddr::V4(source) = source else {
        panic!("IPv4 observation required");
    };
    let mut reply = [0; 32];
    reply[..8].copy_from_slice(&[1, 1, 0, 12, 0x21, 0x12, 0xa4, 0x42]);
    reply[8..20].copy_from_slice(&request[8..20]);
    reply[20..26].copy_from_slice(&[0, 0x20, 0, 8, 0, 1]);
    reply[26..28].copy_from_slice(&(source.port() ^ 0x2112).to_be_bytes());
    reply[28..32].copy_from_slice(&(u32::from(*source.ip()) ^ 0x2112_a442).to_be_bytes());
    socket.send_to(&reply, source).await.unwrap();
    SocketAddr::V4(source)
}
