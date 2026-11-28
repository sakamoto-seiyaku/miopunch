//! Shared loopback relay and STUN fixtures for member path arbitration.
#[path = "delayed_relay.rs"]
pub(super) mod delayed_relay;

use miopunch_native_probe::resources::{Resource, ResourceUse};
use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering::Relaxed},
    },
    time::Duration,
};
use tokio::{net::UdpSocket, task::JoinSet};

pub(super) const FLOOR: Duration = Duration::from_millis(250);
pub(super) const DEADLINE: Duration = Duration::from_secs(3);

pub(super) async fn observe_stun(
    observers: &mut JoinSet<()>,
    requests: &Arc<AtomicUsize>,
) -> Vec<Resource> {
    let mut allowed = Vec::new();
    for _ in 0..2 {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        allowed.push(Resource {
            url: format!("stun://{}/", socket.local_addr().unwrap()),
            uses: [ResourceUse::Observation].into(),
            limitation: "isolated contention fixture".into(),
        });
        let requests = requests.clone();
        observers.spawn(async move {
            loop {
                let mut request = [0u8; 1500];
                let (length, source) = socket.recv_from(&mut request).await.unwrap();
                assert_eq!(length, 20);
                assert_eq!(&request[..8], &[0, 1, 0, 0, 0x21, 0x12, 0xa4, 0x42]);
                let SocketAddr::V4(source) = source else {
                    panic!("IPv4 STUN source")
                };
                let mut reply = [0; 32];
                reply[..8].copy_from_slice(&[1, 1, 0, 12, 0x21, 0x12, 0xa4, 0x42]);
                reply[8..20].copy_from_slice(&request[8..20]);
                reply[20..26].copy_from_slice(&[0, 0x20, 0, 8, 0, 1]);
                reply[26..28].copy_from_slice(&(source.port() ^ 0x2112).to_be_bytes());
                reply[28..32]
                    .copy_from_slice(&(u32::from(*source.ip()) ^ 0x2112a442).to_be_bytes());
                requests.fetch_add(1, Relaxed);
                socket.send_to(&reply, source).await.unwrap();
            }
        });
    }
    allowed
}
