//! Public RTC lease and authenticated QUIC path seams; owned loopback fixtures.
use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use futures_util::future::poll_fn;
use iroh::endpoint::transports::CustomTransport;
use iroh::{
    Endpoint,
    endpoint::{AfterHandshakeOutcome, Connection, EndpointHooks, presets},
};
use miopunch_native_probe::rtc_transport::{RtcTransport, TRANSPORT_ID};

const ALPN: &[u8] = b"miopunch/rtc-path-qualification-test/1";

#[derive(Debug)]
struct CarrierGate;
impl EndpointHooks for CarrierGate {
    async fn after_handshake(&self, _: &Connection) -> AfterHandshakeOutcome {
        AfterHandshakeOutcome::AcceptData {
            allowed_data_custom_ids: vec![TRANSPORT_ID],
        }
    }
}

async fn endpoint(transport: RtcTransport) -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .clear_relay_transports()
        .add_custom_transport(Arc::new(transport))
        .admission_gate(CarrierGate)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .unwrap()
}

async fn pair(a: &Endpoint, b: &Endpoint) -> (Connection, Connection) {
    tokio::time::timeout(Duration::from_secs(3), async {
        let (out, incoming) = tokio::join!(a.connect(b.addr(), ALPN), async {
            b.accept().await.unwrap().await
        });
        (out.unwrap(), incoming.unwrap())
    })
    .await
    .expect("owned loopback QUIC connection")
}

#[tokio::test]
async fn maintenance_retirement_closes_only_the_exact_original_member_association() {
    let at = RtcTransport::new();
    let bt = RtcTransport::new();
    let a = endpoint(at.clone()).await;
    let b = endpoint(bt.clone()).await;
    let (first, first_peer) = pair(&a, &b).await;
    let (second, second_peer) = pair(&a, &b).await;
    let first_lease = at.reserve(&first, || true).unwrap();
    let second_lease = at.reserve(&second, || true).unwrap();
    let first_remote = bt.reserve(&first_peer, || true).unwrap();
    let second_remote = bt.reserve(&second_peer, || true).unwrap();
    for (lease, remote) in [
        (&first_lease, &first_remote),
        (&second_lease, &second_remote),
    ] {
        lease.set_remote(remote.local_addr()).unwrap();
        remote.set_remote(lease.local_addr()).unwrap();
        lease.activate().unwrap();
        remote.activate().unwrap();
    }
    assert!(
        !at.retire_connection(&first_peer),
        "foreign endpoint M cannot retire either local association"
    );
    assert_eq!(at.active_associations(), 2);
    assert!(at.retire_connection(&first));
    assert!(!first_lease.io().is_active());
    assert!(second_lease.io().is_active());
    assert_eq!(at.active_associations(), 1);
    assert!(
        !at.retire_connection(&first),
        "retirement is idempotent for only that old M"
    );
    assert!(first.close_reason().is_none() && second.close_reason().is_none());
    assert!(at.retire_connection(&second));
    for c in [&first, &second, &first_peer, &second_peer] {
        c.close(0u32.into(), b"fixture ended");
    }
    tokio::join!(a.close(), b.close());
}

#[tokio::test]
async fn rtc_route_is_owned_by_one_original_live_member_and_retirement_is_terminal() {
    let at = RtcTransport::new();
    let bt = RtcTransport::new();
    let a = endpoint(at.clone()).await;
    let b = endpoint(bt.clone()).await;
    let (first, remote) = pair(&a, &b).await;
    let (second, second_remote) = pair(&a, &b).await;
    let valid = Arc::new(AtomicBool::new(true));
    let current = valid.clone();
    let lease = at
        .reserve(&first, move || current.load(Ordering::Acquire))
        .unwrap();
    assert!(
        at.reserve(&first, || true).is_err(),
        "one association per M"
    );
    let peer = bt.reserve(&remote, || true).unwrap();
    lease.set_remote(peer.local_addr()).unwrap();
    peer.set_remote(lease.local_addr()).unwrap();
    lease.activate().unwrap();
    peer.activate().unwrap();
    let sender = at.bind().unwrap().create_sender();
    assert!(sender.is_valid_connection_addr(&peer.local_addr(), first.stable_id()));
    assert!(
        !sender.can_bootstrap_connection(&peer.local_addr()),
        "a current RTC route cannot bootstrap another connection"
    );
    assert!(
        second.close_reason().is_none(),
        "second Ordinary M remains live"
    );
    assert!(!sender.is_valid_connection_addr(&peer.local_addr(), second.stable_id()));
    assert!(
        !sender.is_data_path_qualified(&peer.local_addr(), first.stable_id()),
        "channel open does not qualify ordinary data"
    );
    valid.store(false, Ordering::Release);
    assert!(!sender.is_valid_send_addr(&peer.local_addr()));
    valid.store(true, Ordering::Release);
    assert!(
        !sender.is_valid_send_addr(&peer.local_addr()),
        "old scope cannot revive"
    );
    first.close(0u32.into(), b"fixture ended");
    second.close(0u32.into(), b"fixture ended");
    second_remote.close(0u32.into(), b"fixture ended");
    remote.close(0u32.into(), b"fixture ended");
    tokio::join!(a.close(), b.close());
}

#[tokio::test]
async fn pending_rtc_path_never_carries_stream_data_and_only_its_fresh_probe_can_retain_it() {
    let at = RtcTransport::new();
    let bt = RtcTransport::new();
    let a = endpoint(at.clone()).await;
    let b = endpoint(bt.clone()).await;
    let (client, server) = pair(&a, &b).await;
    let (other, other_server) = pair(&a, &b).await;
    let (mut send, mut reply) = client.open_bi().await.unwrap();
    send.write_all(b"zero").await.unwrap();
    let (mut response, mut incoming) = server.accept_bi().await.unwrap();
    let mut bytes = [0; 4];
    incoming.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"zero");
    response.write_all(b"zero").await.unwrap();
    reply.read_exact(&mut bytes).await.unwrap();

    let lease = at.reserve(&client, || true).unwrap();
    let peer = bt.reserve(&server, || true).unwrap();
    lease.set_remote(peer.local_addr()).unwrap();
    peer.set_remote(lease.local_addr()).unwrap();
    lease.activate().unwrap();
    peer.activate().unwrap();
    let forward = |from: miopunch_native_probe::rtc_transport::RtcDatagramIo,
                   to: miopunch_native_probe::rtc_transport::RtcDatagramIo| {
        tokio::spawn(async move {
            while let Ok(packet) = poll_fn(|cx| from.poll_outbound(cx)).await {
                if to.receive(packet).is_err() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
    };
    let ab = forward(lease.io(), peer.io());
    let ba = forward(peer.io(), lease.io());
    assert!(
        a.open_data_transport_path(&other, iroh::TransportAddr::Custom(peer.local_addr()))
            .await
            .is_err(),
        "another Ordinary M cannot inherit this association"
    );
    a.open_data_transport_path(&client, iroh::TransportAddr::Custom(peer.local_addr()))
        .await
        .unwrap();
    let (probe, remote_probe) = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let (Ok(probe), Ok(remote_probe)) =
                (lease.start_probe(&client), peer.start_probe(&server))
            {
                break (probe, remote_probe);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        lease.retain(&client, &remote_probe).is_err(),
        "the opposite local probe is not transferable"
    );
    assert!(
        lease.retain(&other, &probe).is_err(),
        "another M cannot use the original probe"
    );
    assert!(
        client
            .paths()
            .iter()
            .filter(|path| path.remote_addr().is_custom())
            .all(|path| !path.is_selected())
    );
    // Isolate loss of the previous carrier. The otherwise permitted QNT engine
    // can relearn the same loopback UDP address after Path::close().
    a.test_limit_transport_paths(
        &client,
        vec![iroh::TransportAddr::Custom(peer.local_addr())],
    )
    .await
    .unwrap();
    b.test_limit_transport_paths(
        &server,
        vec![iroh::TransportAddr::Custom(lease.local_addr())],
    )
    .await
    .unwrap();
    for path in client.paths().iter().filter(|path| path.is_ip()) {
        path.close().unwrap();
    }
    for path in server.paths().iter().filter(|path| path.is_ip()) {
        let _ = path.close();
    }
    send.write_all(b"next").await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(150), incoming.read_exact(&mut bytes))
            .await
            .is_err(),
        "pending RTC cannot be an ordinary-data fallback when all old paths close"
    );
    for path in client
        .paths()
        .iter()
        .filter(|path| path.remote_addr().is_custom())
    {
        assert_eq!(path.stats().frame_tx.stream, 0);
        assert_eq!(path.stats().frame_tx.datagram, 0);
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        while !probe.is_acknowledged().unwrap() || !remote_probe.is_acknowledged().unwrap() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("path validation and ACK work while ordinary data is prohibited");
    lease.retain(&client, &probe).unwrap();
    peer.retain(&server, &remote_probe).unwrap();
    a.retain_data_transport_path(&client, peer.local_addr())
        .await
        .unwrap();
    b.retain_data_transport_path(&server, lease.local_addr())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), incoming.read_exact(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        &bytes, b"next",
        "the original stream resumes only on the qualified path"
    );
    response.write_all(b"next").await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), reply.read_exact(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&bytes, b"next");
    assert!(
        other
            .paths()
            .iter()
            .all(|path| !path.remote_addr().is_custom())
    );
    let (later, later_server) = pair(&a, &b).await;
    assert!(
        later
            .paths()
            .iter()
            .all(|path| !path.remote_addr().is_custom()),
        "fresh M uses peer-wide bootstrap, never original RTC"
    );
    later.close(0u32.into(), b"fixture ended");
    later_server.close(0u32.into(), b"fixture ended");
    lease.close();
    peer.close();
    ab.abort();
    ba.abort();
    for connection in [&client, &server, &other, &other_server] {
        connection.close(0u32.into(), b"fixture ended");
    }
    tokio::join!(a.close(), b.close());
}

#[derive(Clone, Copy)]
enum RetireHeld {
    Resource,
    Qualification,
    OriginalClosed,
}
#[tokio::test]
async fn retiring_a_route_cancels_a_held_datagram_before_channel_capacity_returns() {
    held_datagram_retirement(RetireHeld::Resource).await;
}
#[tokio::test]
async fn qualification_change_wakes_a_backpressured_driver_without_capacity() {
    held_datagram_retirement(RetireHeld::Qualification).await;
}
#[tokio::test]
async fn original_member_close_wakes_a_backpressured_driver_without_capacity() {
    held_datagram_retirement(RetireHeld::OriginalClosed).await;
}
async fn held_datagram_retirement(trigger: RetireHeld) {
    let at = RtcTransport::new();
    let bt = RtcTransport::new();
    let a = endpoint(at.clone()).await;
    let b = endpoint(bt.clone()).await;
    let (client, server) = pair(&a, &b).await;
    let valid = Arc::new(AtomicBool::new(true));
    let current = valid.clone();
    let updates = n0_watcher::Watchable::new(0);
    let lease = at
        .reserve_with_updates(
            &client,
            move || current.load(Ordering::Acquire),
            updates.watch(),
        )
        .unwrap();
    let peer = bt.reserve(&server, || true).unwrap();
    lease.set_remote(peer.local_addr()).unwrap();
    peer.set_remote(lease.local_addr()).unwrap();
    lease.activate().unwrap();
    peer.activate().unwrap();
    a.open_data_transport_path(&client, iroh::TransportAddr::Custom(peer.local_addr()))
        .await
        .unwrap();
    let io = lease.io();
    // Capture a real authenticated QUIC path packet into the pump, then model
    // the channel's bounded send capacity while this packet is held outside IO.
    let packet = tokio::time::timeout(Duration::from_secs(1), poll_fn(|cx| io.poll_outbound(cx)))
        .await
        .unwrap()
        .unwrap();
    assert!(!packet.is_empty());
    let capacity = Arc::new(tokio::sync::Semaphore::new(0));
    let available = capacity.clone();
    let sent = Arc::new(AtomicBool::new(false));
    let observed = sent.clone();
    let task = tokio::spawn(async move {
        let _driver = io.pump_guard();
        io.while_active(async move {
            let _capacity = available.acquire().await.unwrap();
            assert!(!packet.is_empty());
            observed.store(true, Ordering::Release);
        })
        .await
    });
    tokio::task::yield_now().await;
    match trigger {
        RetireHeld::Resource => {
            at.set_resource_allowed(false);
            capacity.add_permits(1);
        }
        RetireHeld::Qualification => {
            valid.store(false, Ordering::Release);
            let _ = updates.set(1);
        }
        RetireHeld::OriginalClosed => {
            client.close(0u32.into(), b"original caller closed");
        }
    }
    let result = tokio::time::timeout(Duration::from_millis(100), task)
        .await
        .expect("retirement promptly wakes a backpressured pump")
        .unwrap();
    assert!(
        result.is_err(),
        "retirement takes priority even when capacity is also ready"
    );
    assert!(
        !sent.load(Ordering::Acquire),
        "held datagram cannot be sent after retirement"
    );
    assert_eq!(at.active_pumps(), 0);
    assert_eq!(lease.report().owned_pump_tasks, 0);
    assert!(!lease.report().active);
    assert_eq!(lease.report().tx_bytes + lease.report().rx_bytes, 0);
    if !matches!(trigger, RetireHeld::OriginalClosed) {
        assert!(
            client.close_reason().is_none(),
            "candidate retirement preserves the original approved carrier"
        );
    }
    client.close(0u32.into(), b"fixture ended");
    server.close(0u32.into(), b"fixture ended");
    tokio::join!(a.close(), b.close());
}

#[tokio::test]
async fn reserved_preparation_retirement_wakes_without_ready_or_pump_and_preserves_original_stream()
{
    for trigger in [
        RetireHeld::Resource,
        RetireHeld::Qualification,
        RetireHeld::OriginalClosed,
    ] {
        let at = RtcTransport::new();
        let bt = RtcTransport::new();
        let a = endpoint(at.clone()).await;
        let b = endpoint(bt.clone()).await;
        let (client, server) = pair(&a, &b).await;
        let (mut send, _) = client.open_bi().await.unwrap();
        send.write_all(b"before").await.unwrap();
        let (_, mut recv) = server.accept_bi().await.unwrap();
        let mut first = [0; 6];
        recv.read_exact(&mut first).await.unwrap();
        assert_eq!(&first, b"before");
        let valid = Arc::new(AtomicBool::new(true));
        let current = valid.clone();
        let updates = n0_watcher::Watchable::new(0);
        let lease = at
            .reserve_with_updates(
                &client,
                move || current.load(Ordering::Acquire),
                updates.watch(),
            )
            .unwrap();
        let io = lease.io();
        let started = Arc::new(tokio::sync::Notify::new());
        let entered = started.clone();
        let task = tokio::spawn(async move {
            io.while_current(async move {
                entered.notify_one();
                std::future::pending::<()>().await;
            })
            .await
        });
        started.notified().await;
        assert!(!lease.report().active);
        assert_eq!(at.active_pumps(), 0);
        match trigger {
            RetireHeld::Resource => at.set_resource_allowed(false),
            RetireHeld::Qualification => {
                valid.store(false, Ordering::Release);
                let _ = updates.set(1);
            }
            RetireHeld::OriginalClosed => client.close(0u32.into(), b"original caller closed"),
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(100), task)
                .await
                .expect("Reserved retirement wakes preparation without channel readiness")
                .unwrap()
                .is_err()
        );
        if !matches!(trigger, RetireHeld::OriginalClosed) {
            assert!(client.close_reason().is_none());
            send.write_all(b"after").await.unwrap();
            let mut after = [0; 5];
            recv.read_exact(&mut after).await.unwrap();
            assert_eq!(&after, b"after");
        }
        drop(lease);
        client.close(0u32.into(), b"fixture ended");
        server.close(0u32.into(), b"fixture ended");
        tokio::join!(a.close(), b.close());
    }
}
