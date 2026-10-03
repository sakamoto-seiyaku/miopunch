//! Protected wire-version negotiation on real authenticated QUIC connections.
//! These narrow wire tests do not claim admission or a browser RTC data path.
use std::{
    net::{Ipv4Addr, SocketAddr},
    time::Duration,
};

use iroh::{
    Endpoint,
    endpoint::{ConnectOptions, Connection, presets},
};
use miopunch_native_probe::member_path_protocol::{
    MemberProtocol, PathCapabilities, RtcRole, exchange_capabilities,
};

async fn endpoint(alpns: Vec<Vec<u8>>) -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .clear_relay_transports()
        .alpns(alpns)
        .bind()
        .await
        .unwrap()
}

fn binding(connection: &Connection) -> [u8; 32] {
    let mut bytes = [0; 32];
    connection
        .export_keying_material(&mut bytes, b"member-version-test", &[])
        .unwrap();
    bytes
}

#[tokio::test]
async fn upgraded_caller_preserves_legacy_four_bytes_on_the_same_connection() {
    for protocol in [MemberProtocol::Business, MemberProtocol::Replica] {
        let client = endpoint(Vec::new()).await;
        let server = endpoint(vec![protocol.legacy_alpn().to_vec()]).await;
        let (local, remote) = tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(
                async {
                    client
                        .connect_with_opts(
                            server.addr(),
                            protocol.upgraded_alpn(),
                            ConnectOptions::new()
                                .with_additional_alpns(vec![protocol.legacy_alpn().to_vec()]),
                        )
                        .await
                        .unwrap()
                        .await
                        .unwrap()
                },
                async { server.accept().await.unwrap().await.unwrap() }
            )
        })
        .await
        .unwrap();
        let stable = (local.stable_id(), remote.stable_id());
        let before = (binding(&local), binding(&remote));
        assert!(before.0 == before.1, "TLS binding comparison must match");
        assert_eq!(local.alpn(), protocol.legacy_alpn());
        assert_eq!(remote.alpn(), protocol.legacy_alpn());
        let (capabilities, ()) = tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(
                async {
                    exchange_capabilities(
                        &local,
                        PathCapabilities {
                            tcp: false,
                            udp: false,
                            relay: true,
                            rtc: RtcRole::Offerer,
                        },
                    )
                    .await
                    .unwrap()
                },
                async {
                    // An independent legacy peer: its original exact four-byte
                    // limit/version check rejects any appended RTC extension.
                    let (mut send, mut recv) = remote.accept_bi().await.unwrap();
                    send.write_all(&[2, 0, 0, 1]).await.unwrap();
                    send.finish().unwrap();
                    assert_eq!(recv.read_to_end(4).await.unwrap(), [2, 0, 0, 1]);
                }
            )
        })
        .await
        .unwrap();
        assert!(capabilities.relay);
        assert!(!capabilities.rtc);
        assert_eq!((local.stable_id(), remote.stable_id()), stable);
        assert!(
            (binding(&local), binding(&remote)) == before,
            "same M binding comparison must remain true"
        );
        let (mut send, mut recv) = local.open_bi().await.unwrap();
        send.write_all(b"legacy same session").await.unwrap();
        send.finish().unwrap();
        let (mut reply, mut request) = remote.accept_bi().await.unwrap();
        let request = request.read_to_end(64).await.unwrap();
        reply.write_all(&request).await.unwrap();
        reply.finish().unwrap();
        assert_eq!(recv.read_to_end(64).await.unwrap(), b"legacy same session");
        local.close(0u32.into(), b"test done");
        remote.close(0u32.into(), b"test done");
        client.close().await;
        server.close().await;
    }
}

#[tokio::test]
async fn both_upgraded_peers_negotiate_v3_and_only_the_qualified_m_roles() {
    for protocol in [MemberProtocol::Business, MemberProtocol::Replica] {
        let client = endpoint(Vec::new()).await;
        let server = endpoint(protocol.preferred_alpns()).await;
        let (local, remote) = tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(
                async {
                    client
                        .connect_with_opts(
                            server.addr(),
                            protocol.legacy_alpn(),
                            protocol.connect_options(),
                        )
                        .await
                        .unwrap()
                        .await
                        .unwrap()
                },
                async { server.accept().await.unwrap().await.unwrap() }
            )
        })
        .await
        .unwrap();
        assert_eq!(local.alpn(), protocol.upgraded_alpn());
        assert_eq!(remote.alpn(), protocol.upgraded_alpn());
        assert_eq!(
            RtcRole::native_adapter(&local, true),
            RtcRole::None,
            "native dialing role has no qualified RTC adapter"
        );
        assert_eq!(
            RtcRole::browser_adapter(&remote, true),
            RtcRole::None,
            "browser accepting role has no qualified RTC adapter"
        );
        assert_eq!(
            RtcRole::browser_adapter(&local, false),
            RtcRole::None,
            "an absent adapter cannot advertise RTC"
        );
        let (left, right) = tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(
                exchange_capabilities(
                    &local,
                    PathCapabilities {
                        tcp: false,
                        udp: false,
                        relay: false,
                        rtc: RtcRole::browser_adapter(&local, true),
                    }
                ),
                exchange_capabilities(
                    &remote,
                    PathCapabilities {
                        tcp: false,
                        udp: false,
                        relay: false,
                        rtc: RtcRole::native_adapter(&remote, true),
                    }
                )
            )
        })
        .await
        .unwrap();
        let (left, right) = (left.unwrap(), right.unwrap());
        assert_eq!(left.version.number(), 3);
        assert_eq!(right.version.number(), 3);
        assert!(
            left.rtc && right.rtc,
            "complementary supported roles negotiate RTC"
        );
        assert_eq!(left.local_rtc, RtcRole::Offerer);
        assert_eq!(left.peer_rtc, RtcRole::Answerer);
        assert!(
            binding(&local) == binding(&remote),
            "same TLS M remains bound"
        );
        local.close(0u32.into(), b"test done");
        remote.close(0u32.into(), b"test done");
        client.close().await;
        server.close().await;
    }
}

#[tokio::test]
async fn legacy_caller_on_new_server_has_exact_v2_and_no_joint_path_is_unreachable() {
    for protocol in [MemberProtocol::Business, MemberProtocol::Replica] {
        let client = endpoint(Vec::new()).await;
        let server = endpoint(protocol.preferred_alpns()).await;
        let (local, remote) = tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(
                client.connect(server.addr(), protocol.legacy_alpn()),
                async { server.accept().await.unwrap().await }
            )
        })
        .await
        .unwrap();
        let (local, remote) = (local.unwrap(), remote.unwrap());
        let before = (local.stable_id(), remote.stable_id());
        assert_eq!(local.alpn(), protocol.legacy_alpn());
        assert_eq!(remote.alpn(), protocol.legacy_alpn());
        let ((), negotiated) = tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(
                async {
                    let (mut send, mut recv) = local.open_bi().await.unwrap();
                    send.write_all(&[2, 0, 0, 0]).await.unwrap();
                    send.finish().unwrap();
                    assert_eq!(recv.read_to_end(4).await.unwrap(), [2, 0, 0, 0]);
                },
                exchange_capabilities(
                    &remote,
                    PathCapabilities {
                        tcp: false,
                        udp: false,
                        relay: false,
                        rtc: RtcRole::native_adapter(&remote, true),
                    }
                )
            )
        })
        .await
        .unwrap();
        let error = negotiated.unwrap().require_usable().unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::NotConnected);
        assert!(error.to_string().contains("temporarily unreachable"));
        assert_eq!((local.stable_id(), remote.stable_id()), before);
        local.close(0u32.into(), b"test done");
        remote.close(0u32.into(), b"test done");
        client.close().await;
        server.close().await;
    }
}
