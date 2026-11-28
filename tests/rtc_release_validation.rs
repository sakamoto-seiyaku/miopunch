//! Exact-path release renewal through public authenticated QUIC/RTC lease seams.
//! The owned queue bridge is a component fixture, not browser RTC qualification.
#[path = "support/admitted_ip.rs"]
mod admitted_ip;

use futures_util::future::poll_fn;
use iroh::{Endpoint, EndpointAddr, TransportAddr, endpoint::presets};
use miopunch_native_probe::{
    rtc_association::RtcDeadline,
    rtc_transport::{RtcDatagramIo, RtcTransport, TRANSPORT_ID},
};
use std::{
    io,
    net::{Ipv4Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const ALPN: &[u8] = b"miopunch/rtc-release-renewal-test/1";

async fn endpoint(transport: &RtcTransport) -> iroh::endpoint::RestrictedEndpoint {
    Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .clear_relay_transports()
        .add_custom_transport(Arc::new(transport.clone()))
        .alpns(vec![ALPN.to_vec()])
        .restricted_application(admitted_ip::IpAdmission(vec![TRANSPORT_ID]))
        .bind()
        .await
        .unwrap()
}

fn bridge(source: RtcDatagramIo, destination: RtcDatagramIo) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Ok(packet) = poll_fn(|cx| source.poll_outbound(cx)).await {
            if destination.receive(packet).is_err() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
}

fn budget(duration: Duration) -> RtcDeadline {
    RtcDeadline::from_caller(n0_future::time::Instant::now() + duration)
}

#[tokio::test]
async fn release_needs_a_new_exact_path_ack_even_when_old_probe_and_other_carrier_remain_valid() {
    let at = RtcTransport::new();
    let bt = RtcTransport::new();
    let a = endpoint(&at).await;
    let b = endpoint(&bt).await;
    // Keep the actual original UDP carrier live and Available while its
    // owned forwarding socket can temporarily drop every datagram.
    let proxy = Arc::new(
        tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap(),
    );
    let proxy_address = proxy.local_addr().unwrap();
    let upstream = b.bound_sockets()[0];
    let forwarding = Arc::new(AtomicBool::new(true));
    let forward = forwarding.clone();
    let udp_proxy = tokio::spawn(async move {
        let mut client_address = None;
        let mut packet = vec![0; 65536];
        loop {
            let (length, source) = proxy.recv_from(&mut packet).await.unwrap();
            let destination = if source == upstream {
                let Some(client) = client_address else {
                    continue;
                };
                client
            } else {
                client_address = Some(source);
                upstream
            };
            if forward.load(Ordering::Acquire) {
                proxy.send_to(&packet[..length], destination).await.unwrap();
            }
        }
    });
    let target = EndpointAddr::new(b.id()).with_addrs([TransportAddr::Ip(proxy_address)]);
    let (client, server) = tokio::time::timeout(Duration::from_secs(3), async {
        let (client, server) = tokio::join!(a.connect(target, ALPN), async {
            b.accept().await.unwrap().await
        });
        (client.unwrap(), server.unwrap())
    })
    .await
    .unwrap();
    let lease = at.reserve(&client, || true).unwrap();
    let peer = bt.reserve(&server, || true).unwrap();
    lease.set_remote(peer.local_addr()).unwrap();
    peer.set_remote(lease.local_addr()).unwrap();
    lease.activate().unwrap();
    peer.activate().unwrap();
    a.control_handle(&client)
        .test_limit_transport_paths(vec![
            TransportAddr::Ip(proxy_address),
            TransportAddr::Custom(peer.local_addr()),
        ])
        .await
        .unwrap();
    b.control_handle(&server)
        .test_limit_transport_paths(vec![
            TransportAddr::Ip(proxy_address),
            TransportAddr::Custom(lease.local_addr()),
        ])
        .await
        .unwrap();
    for connection in [&client, &server] {
        for path in connection
            .paths()
            .iter()
            .filter(|path| path.is_ip() && path.remote_addr() != &TransportAddr::Ip(proxy_address))
        {
            path.close().unwrap();
        }
    }
    let ab = bridge(lease.io(), peer.io());
    let ba = bridge(peer.io(), lease.io());
    a.open_data_transport_path(&client, TransportAddr::Custom(peer.local_addr()))
        .await
        .unwrap();
    let initial = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match lease.start_probe(&client) {
                Ok(probe) => break probe,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    tokio::task::yield_now().await
                }
                Err(error) => panic!("initial exact path: {error}"),
            }
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while !initial.is_acknowledged().unwrap() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    lease.retain(&client, &initial).unwrap();
    // A peer need not start a probe before the first unilateral validation.
    // Both finite checks can nevertheless renew over RTC after the Available
    // UDP carrier silently stops forwarding, before QUIC retires that path.
    let remote_initial = peer.start_probe(&server).unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while !remote_initial.is_acknowledged().unwrap() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    for connection in [&client, &server] {
        assert_eq!(
            connection.paths().len(),
            2,
            "exactly the original UDP and RTC paths"
        );
        assert!(
            connection
                .paths()
                .iter()
                .any(|path| path.is_ip() && path.is_selected())
        );
        assert!(
            connection
                .paths()
                .iter()
                .any(|path| path.remote_addr().is_custom() && !path.is_selected())
        );
    }
    forwarding.store(false, Ordering::Release);
    let renewed = lease
        .validate_release(&client, budget(Duration::from_secs(2)))
        .await
        .unwrap();
    assert_eq!(renewed.path_id(), initial.path_id());
    assert!(
        renewed.watermark() > initial.watermark(),
        "release renews the opaque packet watermark"
    );
    assert!(renewed.is_acknowledged().unwrap());
    assert!(
        client
            .paths()
            .iter()
            .any(|path| path.is_ip() && path.is_selected()),
        "renewal works before the old Available carrier is retired"
    );
    forwarding.store(true, Ordering::Release);

    // Stop only this test's RTC queue bridge. The real original UDP carrier,
    // SDK-independent qualification predicate, permission and both Ms stay live.
    ab.abort();
    ba.abort();
    let _ = tokio::join!(ab, ba);
    let before = lease.report().sent_packets;
    let started = std::time::Instant::now();
    let failure = lease
        .validate_release(&client, budget(Duration::from_millis(150)))
        .await;
    assert_eq!(
        failure
            .err()
            .expect("old ACK cannot release unavailable exact RTC path")
            .kind(),
        io::ErrorKind::TimedOut
    );
    assert!(
        started.elapsed() < Duration::from_millis(400),
        "original caller budget owns renewal"
    );
    assert!(
        initial.is_acknowledged().unwrap() && renewed.is_acknowledged().unwrap(),
        "older proofs genuinely remain ACKed"
    );
    assert!(
        lease.report().sent_packets > before,
        "renewal submitted a new real QUIC packet"
    );
    assert!(lease.io().is_active() && peer.io().is_active());
    assert!(at.resource_allowed() && bt.resource_allowed());
    assert_eq!(at.resource_revision() + bt.resource_revision(), 0);
    assert!(client.close_reason().is_none() && server.close_reason().is_none());
    assert!(
        client
            .paths()
            .iter()
            .any(|path| path.is_selected() && path.is_ip()),
        "other actual carrier stays selected"
    );
    let (mut send, mut reply) = client.open_bi().await.unwrap();
    send.write_all(b"live original UDP M").await.unwrap();
    send.finish().unwrap();
    let (mut response, mut incoming) = server.accept_bi().await.unwrap();
    assert_eq!(
        incoming.read_to_end(64).await.unwrap(),
        b"live original UDP M"
    );
    response.write_all(b"live original UDP M").await.unwrap();
    response.finish().unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), reply.read_to_end(64))
            .await
            .unwrap()
            .unwrap(),
        b"live original UDP M"
    );
    // Successful and cancelled validations must stop preferring the RTC
    // backup for ordinary ACKs after their last proof is released.
    drop(initial);
    drop(renewed);
    drop(remote_initial);
    let ab = bridge(lease.io(), peer.io());
    let ba = bridge(peer.io(), lease.io());
    tokio::time::sleep(Duration::from_millis(80)).await;
    let ack_counts = || {
        server
            .paths()
            .iter()
            .fold((0, 0), |(available, backup), path| {
                let acks = path.stats().frame_tx.path_acks;
                if path.remote_addr().is_custom() {
                    assert!(
                        !path.is_selected(),
                        "RTC remains a backup to the live UDP carrier"
                    );
                    (available, backup + acks)
                } else {
                    (available + acks, backup)
                }
            })
    };
    let before = ack_counts();
    let ordinary = client
        .paths()
        .iter()
        .find(|path| path.remote_addr().is_custom())
        .unwrap()
        .start_fresh_probe()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while !ordinary.is_acknowledged().unwrap() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let after = ack_counts();
    assert!(
        after.0 > before.0,
        "ordinary backup PING ACK returns on Available UDP"
    );
    assert_eq!(
        after.1, before.1,
        "RTC probe holders leave no persistent Backup ACK preference"
    );
    ab.abort();
    ba.abort();
    let _ = tokio::join!(ab, ba);
    lease.close();
    peer.close();
    client.close(0u32.into(), b"fixture ended");
    server.close(0u32.into(), b"fixture ended");
    tokio::join!(a.close(), b.close());
    udp_proxy.abort();
    let _ = udp_proxy.await;
}
