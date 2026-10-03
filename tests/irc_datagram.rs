use miopunch_native_probe::irc_datagram::{Profile, Reassembly, fragments};

#[test]
fn slow_fragments_deliver_only_a_complete_datagram_before_the_declared_deadline() {
    let own = [1; 16];
    let source = [2; 16];
    let bytes = vec![0x5a; 1200];
    let parts = fragments(own, source, 42, &bytes).unwrap();
    let mut assembly = Reassembly::new(Profile::slow()).unwrap();
    for (index, part) in parts.iter().enumerate() {
        let packet = assembly.receive(part, own, index as u64 * 600);
        if index < 9 {
            assert!(packet.is_none());
        } else {
            assert_eq!(packet.unwrap().1, bytes);
        }
    }
    assert_eq!(assembly.snapshot().pending, 0);
}

#[test]
fn cancellation_retires_the_assembly_generation_even_when_all_fragments_arrive_later() {
    let parts = fragments([1; 16], [2; 16], 42, &[0x5a; 1200]).unwrap();
    let mut assembly = Reassembly::new(Profile::slow()).unwrap();
    assembly.receive(&parts[0], [1; 16], 0);
    assembly.cancel();
    for part in &parts {
        assert!(assembly.receive(part, [1; 16], 1).is_none());
    }
    assert_eq!(assembly.snapshot().pending, 0);
}

#[tokio::test]
async fn native_ping_overtakes_a_slow_fragment_queue_and_cancel_retires_the_driver() {
    use miopunch_native_probe::{
        irc_datagram::{IrcFacility, address},
        irc_transport::IrcTransport,
    };
    use std::time::Duration;
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server = listener.local_addr().unwrap();
    let channel = "#owned-ping";
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let (read, mut write) = socket.into_split();
        let mut lines = BufReader::new(read).lines();
        let nick = lines
            .next_line()
            .await
            .unwrap()
            .unwrap()
            .strip_prefix("NICK ")
            .unwrap()
            .to_string();
        assert!(
            lines
                .next_line()
                .await
                .unwrap()
                .unwrap()
                .starts_with("USER ")
        );
        write
            .write_all(
                format!(
                    ":fixture 001 {nick} :welcome\r\n:fixture 005 {nick} NICKLEN=32 :supported\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        assert_eq!(
            lines.next_line().await.unwrap().unwrap(),
            format!("JOIN {channel}")
        );
        write.write_all(format!(":{nick}!~u@localhost JOIN :{channel}\r\n:fixture 366 {nick} {channel} :names done\r\n").as_bytes()).await.unwrap();
        assert!(
            lines
                .next_line()
                .await
                .unwrap()
                .unwrap()
                .starts_with("PRIVMSG ")
        );
        let at = tokio::time::Instant::now();
        write
            .write_all(b"PING :owned-pacing-check\r\n")
            .await
            .unwrap();
        let pong = tokio::time::timeout(Duration::from_millis(500), lines.next_line())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(pong, "PONG :owned-pacing-check");
        assert!(at.elapsed() < Duration::from_millis(500));
        while let Some(line) = lines.next_line().await.unwrap() {
            if line.starts_with("QUIT ") {
                return;
            }
        }
        panic!("native cancellation must send owned-channel PART and QUIT");
    });
    let facility = IrcFacility {
        logical_id: "owned-native-ping".into(),
        native_url: format!("irc://{server}/"),
        wss_url: "wss://localhost:1/webirc".into(),
        channel: channel.into(),
    };
    let transport = IrcTransport::from_facility(&facility, &[], Profile::slow(), None).unwrap();
    let datagrams = transport.bind_datagrams().unwrap();
    transport.wait_ready().await.unwrap();
    datagrams
        .send(
            &address(facility.service_tag().unwrap(), [2; 16]),
            &[90; 1200],
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while transport.traffic().snapshot()["protocol"]["pong_sent"] != 1 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    transport.close();
    assert!(datagrams.local_addresses().is_empty());
    assert!(transport.wait_ready().await.is_err());
    assert!(
        datagrams
            .send(
                &address(facility.service_tag().unwrap(), [2; 16]),
                &[90; 1200]
            )
            .await
            .is_err()
    );

    tokio::time::timeout(Duration::from_secs(2), peer)
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while transport.traffic().snapshot()["driver_active"] != 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(transport.traffic().pending_assemblies(), 0);
    assert_eq!(
        transport.traffic().snapshot()["protocol"]["queue_pending"],
        0
    );
    assert_eq!(transport.traffic().sent_packets(), 0);
}

#[test]
fn missing_and_conflicting_fragments_never_deliver_and_pending_stays_bounded() {
    let own = [1; 16];
    let source = [2; 16];
    let parts = fragments(own, source, 1, &[90; 1200]).unwrap();
    let mut missing = Reassembly::new(Profile::slow()).unwrap();
    for part in parts.iter().skip(1) {
        assert!(missing.receive(part, own, 0).is_none());
    }
    missing.expire(8000);
    assert_eq!(missing.snapshot().expired, 1);
    assert_eq!(missing.snapshot().pending, 0);
    for part in &parts {
        assert!(missing.receive(part, own, 8001).is_none());
    }
    let mut conflict = Reassembly::new(Profile::slow()).unwrap();
    conflict.receive(&parts[0], own, 0);
    let mut changed = parts[0].clone();
    changed.replace_range(changed.len() - 2.., "ff");
    conflict.receive(&changed, own, 1);
    for part in &parts {
        assert!(conflict.receive(part, own, 2).is_none());
    }
    assert_eq!(conflict.snapshot().rejected, 1);
    assert_eq!(conflict.snapshot().pending, 0);
    let mut bounded = Reassembly::new(Profile::slow()).unwrap();
    for id in 0..40 {
        let parts = fragments(own, source, id, &[90; 1200]).unwrap();
        assert!(bounded.receive(&parts[0], own, 0).is_none());
    }
    assert_eq!(bounded.snapshot().pending, 32);
    assert_eq!(bounded.snapshot().pending_peak, 32);
    assert_eq!(bounded.snapshot().rejected, 8);
    bounded.expire(8000);
    assert_eq!(bounded.snapshot().pending, 0);
    assert_eq!(bounded.snapshot().expired, 32);
}

#[test]
fn default_three_second_profile_does_not_claim_qualification_for_the_slow_datagram() {
    let parts = fragments([1; 16], [2; 16], 1, &[90; 1200]).unwrap();
    let mut assembly = Reassembly::new(Profile::default()).unwrap();
    for (i, part) in parts.iter().enumerate() {
        assert!(assembly.receive(part, [1; 16], i as u64 * 600).is_none());
    }
    assert_eq!(assembly.snapshot().delivered, 0);
    assert_eq!(assembly.snapshot().expired, 1);
    assert_eq!(assembly.snapshot().pending, 0);
}

#[test]
fn facility_entry_aliases_and_channel_names_do_not_bypass_disable_or_service_tags() {
    use miopunch_native_probe::irc_datagram::{IrcFacility, address, route_from_addr};
    let facility = IrcFacility {
        logical_id: "owned".into(),
        native_url: "ircs://localhost:6697/".into(),
        wss_url: "wss://localhost:8097/webirc".into(),
        channel: "#owned".into(),
    };
    for disabled in [
        &facility.logical_id,
        &facility.native_url,
        &facility.wss_url,
    ] {
        assert!(facility.check_allowed(&[disabled.clone()]).is_err());
    }
    assert!(
        facility
            .check_allowed(&["wss://LOCALHOST:8097/other-entry".into()])
            .is_err()
    );
    assert!(
        facility
            .check_allowed(&["IRCS://LOCALHOST:6697/other-entry".into()])
            .is_err()
    );
    let mut other = facility.clone();
    other.channel = "#other".into();
    assert_ne!(
        facility.service_tag().unwrap(),
        other.service_tag().unwrap()
    );
    let target = address(facility.service_tag().unwrap(), [1; 16]);
    assert!(route_from_addr(&target, other.service_tag().unwrap()).is_err());
    other.channel = "#owned\r\nJOIN #human".into();
    assert!(other.validate().is_err());
}

#[test]
fn readiness_requires_registration_isupport_own_join_and_matching_names_completion() {
    use miopunch_native_probe::irc_datagram::{Qualification, parse_line};
    let mut q = Qualification::default();
    for line in [
        ":server 001 mio :welcome",
        ":server 366 mio #owned :names complete",
        ":mio!~u@host JOIN :#owned",
    ] {
        q.receive(&parse_line(line).unwrap(), "mio", "#owned")
            .unwrap();
        assert!(!q.ready);
    }
    q.receive(
        &parse_line(":server 005 mio NICKLEN=32 :supported").unwrap(),
        "mio",
        "#owned",
    )
    .unwrap();
    assert!(q.ready);
    q.retire();
    assert!(!q.ready);
    q.receive(
        &parse_line(":server 005 mio NICKLEN=32 :late event").unwrap(),
        "mio",
        "#owned",
    )
    .unwrap();
    assert!(!q.ready);
}

#[tokio::test]
async fn native_whole_attempt_deadline_closes_a_silent_registration_and_clears_pending() {
    use miopunch_native_probe::{irc_datagram::IrcFacility, irc_transport::IrcTransport};
    use std::time::Duration;
    use tokio::{io::AsyncReadExt, net::TcpListener};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        socket.read_to_end(&mut bytes).await.unwrap();
        assert!(!bytes.is_empty());
    });
    let facility = IrcFacility {
        logical_id: "owned-deadline".into(),
        native_url: format!("irc://{server}/"),
        wss_url: "wss://localhost:1/webirc".into(),
        channel: "#owned".into(),
    };
    let mut profile = Profile::slow();
    profile.attempt_ms = 200;
    let transport = IrcTransport::from_facility(&facility, &[], profile, None).unwrap();
    let mut datagrams = transport.bind_datagrams().unwrap();
    assert!(transport.wait_ready().await.is_err());
    assert!(datagrams.receive().await.is_err());
    assert!(datagrams.local_addresses().is_empty());
    tokio::time::timeout(Duration::from_secs(1), peer)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(transport.traffic().snapshot()["driver_active"], 0);
    assert_eq!(transport.traffic().pending_assemblies(), 0);
    assert_eq!(
        transport.traffic().snapshot()["protocol"]["queue_pending"],
        0
    );
    assert!(
        transport.traffic().snapshot()["protocol"]["failure"]
            .as_str()
            .unwrap()
            .contains("whole attempt deadline")
    );
}

#[tokio::test]
async fn dropping_during_tls_startup_retires_the_socket_without_waiting_for_its_timeout() {
    use miopunch_native_probe::{irc_datagram::IrcFacility, irc_transport::IrcTransport};
    use std::time::Duration;
    use tokio::{io::AsyncReadExt, net::TcpListener, sync::oneshot};
    let _ = rustls::crypto::ring::default_provider().install_default();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server = listener.local_addr().unwrap();
    let (connected, accepted) = oneshot::channel();
    let peer = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        connected.send(()).unwrap();
        let mut data = Vec::new();
        socket.read_to_end(&mut data).await.unwrap();
    });
    let facility = IrcFacility {
        logical_id: "owned-tls-cancel".into(),
        native_url: format!("ircs://{server}/"),
        wss_url: "wss://localhost:1/webirc".into(),
        channel: "#owned".into(),
    };
    let transport = IrcTransport::from_facility(&facility, &[], Profile::slow(), None).unwrap();
    let datagrams = transport.bind_datagrams().unwrap();
    accepted.await.unwrap();
    assert!(datagrams.local_addresses().is_empty());
    drop(datagrams);
    tokio::time::timeout(Duration::from_millis(500), peer)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(transport.traffic().snapshot()["driver_active"], 0);
    assert_eq!(
        transport.traffic().snapshot()["protocol"]["qualification"]["ready"],
        false
    );
}

#[tokio::test]
async fn immediate_rebind_keeps_the_new_route_ready_when_the_old_driver_retires() {
    use miopunch_native_probe::{irc_datagram::IrcFacility, irc_transport::IrcTransport};
    use std::time::Duration;
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let mut workers = Vec::new();
        for _ in 0..2 {
            let (socket, _) = listener.accept().await.unwrap();
            workers.push(tokio::spawn(async move{
                let(read,mut write)=socket.into_split();let mut lines=BufReader::new(read).lines();
                let nick=lines.next_line().await.unwrap().unwrap().strip_prefix("NICK ").unwrap().to_string();
                assert!(lines.next_line().await.unwrap().unwrap().starts_with("USER "));
                write.write_all(format!(":fixture 001 {nick} :welcome\r\n:fixture 005 {nick} NICKLEN=32 :supported\r\n").as_bytes()).await.unwrap();
                assert_eq!(lines.next_line().await.unwrap().unwrap(),"JOIN #owned");
                write.write_all(format!(":{nick}!~u@localhost JOIN :#owned\r\n:fixture 366 {nick} #owned :names done\r\n").as_bytes()).await.unwrap();
                while let Some(line)=lines.next_line().await.unwrap(){if line.starts_with("QUIT "){break;}}
            }));
        }
        for worker in workers {
            worker.await.unwrap();
        }
    });
    let facility = IrcFacility {
        logical_id: "owned-rebind".into(),
        native_url: format!("irc://{server}/"),
        wss_url: "wss://localhost:1/webirc".into(),
        channel: "#owned".into(),
    };
    let transport = IrcTransport::from_facility(&facility, &[], Profile::slow(), None).unwrap();
    let first = transport.bind_datagrams().unwrap();
    transport.wait_ready().await.unwrap();
    let old = first.local_addresses()[0].clone();
    drop(first);
    let second = transport.bind_datagrams().unwrap();
    transport.wait_ready().await.unwrap();
    let current = second.local_addresses()[0].clone();
    assert_ne!(old, current);
    tokio::time::timeout(Duration::from_secs(1), async {
        while transport.traffic().snapshot()["driver_active"] != 1 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let report = transport.traffic().snapshot();
    assert!(report["protocol"]["withdraw_ms"].is_null());
    assert_eq!(report["protocol"]["qualification"]["ready"], true);
    assert_eq!(second.local_addresses(), vec![current]);
    drop(second);
    tokio::time::timeout(Duration::from_secs(1), peer)
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while transport.traffic().snapshot()["driver_active"] != 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
