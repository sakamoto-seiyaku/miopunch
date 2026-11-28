//! Faults at the final bilateral confirmation on two admitted native members.

use super::{
    admitted_pair, relay, resource,
    udp::{OriginalStream, on_relay, qualified_udp, reply_stun, wait_for},
};
use iroh::{
    Endpoint, EndpointAddr,
    endpoint::{MemberConnection, PathCommitPause, PathCommitPoint, presets},
};
use miopunch_native_probe::{
    admission_gate::ProtocolScope,
    native_network::NativeNetwork,
    path_selection::PathPreference,
    resources::{Resource, ResourceUse},
};
use std::{net::Ipv4Addr, time::Duration};
use tokio::{
    net::UdpSocket,
    time::{Instant, timeout},
};
use tokio_util::sync::CancellationToken;

#[path = "tcp_binding_observer.rs"]
mod tcp_observer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fault {
    ExchangeReset,
    PeerFinished,
    OriginalClosed,
    CredentialChanged,
}

const FAULTS: [Fault; 4] = [
    Fault::ExchangeReset,
    Fault::PeerFinished,
    Fault::OriginalClosed,
    Fault::CredentialChanged,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Carrier {
    Tcp,
    Udp,
}

impl Carrier {
    fn matches(self, path: &iroh::TransportAddr) -> bool {
        match self {
            Self::Tcp => {
                matches!(path, iroh::TransportAddr::Custom(a) if a.id() == miopunch_native_probe::tcp_transport::DATA_TRANSPORT_ID)
            }
            Self::Udp => matches!(path, iroh::TransportAddr::Ip(_)),
        }
    }
}

#[tokio::test]
async fn tcp_commit_preserves_the_original_fault_contracts() {
    for fault in FAULTS {
        timeout(Duration::from_secs(30), scenario(Carrier::Tcp, fault))
            .await
            .expect("TCP commit scenario");
    }
}

#[tokio::test]
async fn udp_commit_preserves_the_original_fault_contracts() {
    for fault in FAULTS {
        timeout(Duration::from_secs(30), scenario(Carrier::Udp, fault))
            .await
            .expect("UDP commit scenario");
    }
}

async fn inject(
    fault: Fault,
    original: &MemberConnection,
    node: &mut NativeNetwork,
    held: PathCommitPause,
    peer: PathCommitPause,
) {
    let peer_exchange = peer.entered.await.expect("peer reached final confirmation");
    if fault == Fault::ExchangeReset {
        let exchange = held
            .entered
            .await
            .expect("tentative candidate reached confirmation");
        exchange.abort(1u32.into());
        let _ = held.resume.send(());
        let _ = peer.resume.send(());
        assert!(held.retained.await.is_err());
        assert!(peer.retained.await.is_err());
        return;
    }

    peer.resume.send(()).unwrap();
    let _local_exchange = held
        .entered
        .await
        .expect("received the actual peer confirmation");
    peer.retained.await.expect("peer committed its candidate");
    // Wait for transport acknowledgement of the peer's finished control stream.
    assert!(peer_exchange.stopped().await.unwrap().is_none());
    match fault {
        Fault::PeerFinished => {}
        Fault::OriginalClosed => original.test_close(7, b"original closed after peer commit"),
        Fault::CredentialChanged => {
            node.invalidate_current_credential_fixture().await.unwrap();
        }
        Fault::ExchangeReset => unreachable!(),
    }
    let _ = held.resume.send(());
    assert_eq!(held.retained.await.is_ok(), fault == Fault::PeerFinished);
}

async fn scenario(carrier: Carrier, fault: Fault) {
    eprintln!("path commit {carrier:?} {fault:?}");
    let root = tempfile::tempdir().unwrap();
    let relay = relay().await;
    let url = relay.http_url().unwrap();
    let (mut admin, _) = admitted_pair(root.path(), &url).await;
    let joining = NativeNetwork::open(root.path().join("joining"))
        .await
        .unwrap();

    // Independent admitted stream with the same gate. Closing the target
    // original must leave this stream usable until credentials are invalidated.
    async fn bind_other(node: &NativeNetwork) -> iroh::endpoint::RestrictedEndpoint {
        let mut config = node.resource_configuration().unwrap();
        config.udp_direct_disabled = false;
        node.save_resource_configuration(&config).unwrap();
        node.admission_gate()
            .register_protocol(crate::BUSINESS, ProtocolScope::Member);
        node.bind_endpoint_with(
            Endpoint::builder(presets::Minimal)
                .clear_ip_transports()
                .bind_addr((Ipv4Addr::LOCALHOST, 0))
                .unwrap()
                .alpns(vec![crate::BUSINESS.to_vec()]),
        )
        .await
        .unwrap()
    }
    let (left, right) = tokio::join!(bind_other(&joining), bind_other(&admin));
    let other_target = EndpointAddr::new(right.id()).with_ip_addr(right.bound_sockets()[0]);
    let (out, inc) = tokio::join!(left.connect(other_target, crate::BUSINESS), async {
        right.accept().await.unwrap().await
    });
    let (out, inc) = (out.unwrap(), inc.unwrap());
    let (mut write, mut read) = out.open_bi().await.unwrap();
    write.write_all(b"a").await.unwrap();
    let (mut back, mut request) = inc.accept_bi().await.unwrap();
    let mut byte = [0];
    request.read_exact(&mut byte).await.unwrap();
    assert_eq!(byte, *b"a");

    let mut observers = Vec::new();
    let mut udp = Vec::new();
    let mut listener = None;
    if carrier == Carrier::Tcp {
        let (addr, task) = tcp_observer::tcp_binding_observer(None, 0).await;
        listener = Some(task);
        observers.push(format!("stun+tcp://{addr}/"));
    } else {
        for _ in 0..2 {
            let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            observers.push(format!("stun://{}/", socket.local_addr().unwrap()));
            udp.push(socket);
        }
    }
    for node in [&joining, &admin] {
        let mut config = node.resource_configuration().unwrap();
        config.replacement = Some(
            std::iter::once(resource(&url))
                .chain(observers.iter().map(|url| Resource {
                    url: url.clone(),
                    uses: [ResourceUse::Observation].into(),
                    limitation: "owned commit observer".into(),
                }))
                .collect(),
        );
        config.udp_direct_disabled = carrier == Carrier::Tcp;
        config.tcp_direct_disabled = carrier == Carrier::Udp;
        config.tcp_listen = Some((Ipv4Addr::UNSPECIFIED, 0).into());
        config.path_selection.preference = PathPreference::PreferDirect;
        config.maintenance.standby_member_limit = Some(0);
        config.maintenance.path_retry_base_ms = 30_000;
        node.save_resource_configuration(&config).unwrap();
    }
    let builder = || {
        Endpoint::builder(presets::Minimal)
            .clear_ip_transports()
            .bind_addr((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .test_suppress_member_direct_announcements()
    };
    let budget = Duration::from_secs(12);
    let client = joining
        .bind_member_fusion_endpoint_with(builder(), budget)
        .await
        .unwrap();
    let server = admin
        .bind_member_fusion_endpoint_with(builder(), budget)
        .await
        .unwrap();
    client.test_disable_background_udp();
    server.test_disable_background_udp();
    let connections = on_relay(&client, &server, &url).await;
    let point = if fault == Fault::ExchangeReset {
        PathCommitPoint::BeforeConfirmation
    } else {
        PathCommitPoint::AfterConfirmation
    };
    let held = connections[1].test_pause_path_commit(point);
    let peer = connections[0].test_pause_path_commit(PathCommitPoint::BeforeConfirmation);
    let mut flow = OriginalStream::open(&connections[0], &connections[1], 1).await;
    let inject = inject(fault, &connections[1], &mut admin, held, peer);
    if carrier == Carrier::Tcp {
        inject.await;
    } else {
        let addresses =
            [&client, &server].map(|ep| *ep.addr().ip_addrs().find(|addr| addr.is_ipv4()).unwrap());
        let started = Instant::now();
        let cancel = CancellationToken::new();
        let (result, (), ()) = tokio::join!(
            connections[0].coordinate_udp_traversal(started + budget, &cancel),
            async {
                tokio::join!(
                    reply_stun(&udp[0], addresses),
                    reply_stun(&udp[1], addresses)
                );
                qualified_udp(&connections, addresses, started, true).await;
                wait_for("both candidates have a fresh ACK", budget, || {
                    connections
                        .iter()
                        .all(|c| c.udp_traversal().is_some_and(|r| r.validated.is_some()))
                })
                .await;
                let round = connections[0].udp_traversal().unwrap().round;
                flow.confirm_round_trip(&connections, round, 2).await;
            },
            inject
        );
        match fault {
            Fault::PeerFinished => {
                result.expect("confirmed UDP round succeeded");
            }
            Fault::ExchangeReset => assert!(result.is_err()),
            // The initiator committed before the responder's injected fault.
            Fault::OriginalClosed | Fault::CredentialChanged => {}
        }
        let report = connections[1].udp_traversal().unwrap();
        assert!(report.validated.is_some());
        assert_eq!(
            report.business.map(|(id, _)| id),
            (fault == Fault::PeerFinished).then_some(flow.stream_id)
        );
        assert_eq!(report.learned, fault == Fault::PeerFinished);
    }

    if matches!(fault, Fault::PeerFinished | Fault::ExchangeReset) {
        if fault == Fault::PeerFinished {
            // Retention completes before the path selector processes its next update.
            wait_for(
                "both committed paths selected",
                Duration::from_secs(6),
                || {
                    connections.iter().all(|c| {
                        c.paths().iter().any(|path| {
                            carrier.matches(path.remote_addr())
                                && path.is_selected()
                                && path.stats().acknowledgment.is_some()
                        })
                    })
                },
            )
            .await;
        } else if carrier == Carrier::Tcp {
            wait_for(
                "both tentative TCP candidates rolled back",
                Duration::from_secs(3),
                || {
                    connections
                        .iter()
                        .all(|c| c.paths().iter().all(|p| !carrier.matches(p.remote_addr())))
                },
            )
            .await;
        }
        flow.round_trip(3).await;
    } else {
        connections[1].closed().await;
    }
    if fault == Fault::CredentialChanged {
        assert!(!admin.admission_gate().allows(&inc, ProtocolScope::Member));
        inc.closed().await;
    } else {
        write.write_all(b"b").await.unwrap();
        request.read_exact(&mut byte).await.unwrap();
        assert_eq!(byte, *b"b");
        back.write_all(b"c").await.unwrap();
        read.read_exact(&mut byte).await.unwrap();
        assert_eq!(byte, *b"c");
    }
    if let Some(listener) = listener {
        listener.abort();
    }
    tokio::join!(client.close(), server.close(), left.close(), right.close());
    tokio::join!(admin.close(), joining.close());
    relay.shutdown().await.unwrap();
}
