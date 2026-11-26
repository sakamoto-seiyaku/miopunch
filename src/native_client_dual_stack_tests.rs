use super::*;
use crate::resources::{LanDiscovery, ResourceConfig};
use std::net::{Ipv6Addr, SocketAddr};
use tokio::time::{sleep, timeout};

#[path = "../tests/support/udp_blackhole.rs"]
mod udp_blackhole;

async fn admitted_pair(root: &Path) -> [NativeClient; 2] {
    admitted_pair_with_config(
        root,
        ResourceConfig {
            replacement: Some(Vec::new()),
            tcp_direct_disabled: true,
            lan_discovery: LanDiscovery::Disabled,
            ..Default::default()
        },
    )
    .await
}

async fn admitted_pair_with_config(root: &Path, config: ResourceConfig) -> [NativeClient; 2] {
    let provider = NativeClient::create_with_resources(
        root.join("provider"),
        Ipv4Addr::LOCALHOST,
        config.clone(),
    )
    .await
    .unwrap();
    provider
        .export_recovery_card_to(root.join("card"), None)
        .await
        .unwrap();
    provider.acknowledge_recovery_saved().await.unwrap();
    let invitation = provider
        .issue_precode(crate::service_policy::now() + 300, Some(1))
        .await
        .unwrap();
    let consumer = NativeClient::join_with_resources(
        root.join("consumer"),
        &invitation,
        Ipv4Addr::LOCALHOST,
        config,
    )
    .await
    .unwrap();
    assert_eq!(
        consumer.request_admission(&invitation).await.unwrap(),
        AdmissionStage::Complete
    );
    consumer.close().await.unwrap();
    provider.close().await.unwrap();
    for name in ["provider", "consumer"] {
        let contacts = root.join(name).join("member.contacts.json");
        if contacts.exists() {
            std::fs::remove_file(contacts).unwrap();
        }
    }
    let (provider, consumer) = tokio::join!(
        NativeClient::open(root.join("provider"), Ipv4Addr::LOCALHOST),
        NativeClient::open(root.join("consumer"), Ipv4Addr::LOCALHOST),
    );
    [provider.unwrap(), consumer.unwrap()]
}

fn sockets(client: &NativeClient) -> [SocketAddr; 2] {
    let bound = client.runtime().resources.endpoint().bound_sockets();
    [false, true].map(|ipv6| {
        let port = bound
            .iter()
            .find(|addr| addr.is_ipv6() == ipv6)
            .unwrap()
            .port();
        if ipv6 {
            SocketAddr::from((Ipv6Addr::LOCALHOST, port))
        } else {
            SocketAddr::from((Ipv4Addr::LOCALHOST, port))
        }
    })
}

#[tokio::test]
#[ignore = "requires a disposable network namespace for per-family UDP faults"]
async fn first_member_path_is_independent_of_the_other_address_family() {
    for case in ["ipv4", "ipv6", "dual", "blackhole-v4", "blackhole-v6"] {
        let root = tempfile::tempdir().unwrap();
        let clients = admitted_pair(root.path()).await;
        let addresses = clients.each_ref().map(sockets);
        let blocked = match case {
            "blackhole-v4" => Some(udp_blackhole::UdpBlackhole::new(
                addresses.map(|both| both[0]),
            )),
            "blackhole-v6" => Some(udp_blackhole::UdpBlackhole::new(
                addresses.map(|both| both[1]),
            )),
            _ => None,
        };
        let mut target = EndpointAddr::new(clients[0].address().id);
        for address in addresses[0] {
            if (case != "ipv4" || address.is_ipv4()) && (case != "ipv6" || address.is_ipv6()) {
                target = target.with_ip_addr(address);
            }
        }
        let started = std::time::Instant::now();
        let first = timeout(Duration::from_secs(3), clients[1].ping(target))
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "{case}: the available family must deliver before the dead family's dial slice"
                )
            })
            .unwrap();
        assert_eq!(first["reply"], "PONG");
        let connection = clients[1]
            .test_member_connection(clients[0].address().id)
            .await
            .unwrap();
        let original = connection.stable_id();
        if case == "blackhole-v4" || case == "ipv6" {
            assert!(
                matches!(connection.initial_data_path(), iroh::TransportAddr::Ip(addr) if addr.is_ipv6())
            );
        }
        if case == "blackhole-v6" || case == "ipv4" {
            assert!(
                matches!(connection.initial_data_path(), iroh::TransportAddr::Ip(addr) if addr.is_ipv4())
            );
        }
        let reverse = clients[0]
            .ping(EndpointAddr::new(clients[1].address().id))
            .await
            .unwrap();
        assert_eq!(reverse["reply"], "PONG");
        assert_eq!(
            clients[1]
                .test_member_connection(clients[0].address().id)
                .await
                .unwrap()
                .stable_id(),
            original
        );
        println!(
            "DUAL_STACK_FIRST_DATA case={case} elapsed_ms={} path={:?} reverse={}",
            started.elapsed().as_millis(),
            connection.initial_data_path(),
            reverse["reply"]
        );
        drop(blocked);
        let [provider, consumer] = clients;
        consumer.close().await.unwrap();
        provider.close().await.unwrap();
        sleep(Duration::from_millis(20)).await;
    }
}

/// The shell driver gives each member an independent IPv6 conntrack firewall.
#[tokio::test]
#[ignore = "requires scripts/check_native_udp_dual_stack.sh"]
async fn ipv6_qnt_crosses_both_unsolicited_inbound_filters_on_the_original_member() {
    use crate::resources::{Resource, ResourceUse};
    let root = std::path::PathBuf::from(std::env::var("MIO197_ROOT").unwrap());
    let role = std::env::var("MIO197_ROLE").unwrap();
    if role == "relay" {
        let mut config = iroh_relay::server::ServerConfig::default();
        config.relay = Some(iroh_relay::server::RelayConfig::new((
            Ipv4Addr::UNSPECIFIED,
            0,
        )));
        let server = iroh_relay::server::Server::spawn(config).await.unwrap();
        std::fs::write(
            root.join("relay.port"),
            server.http_addr().unwrap().port().to_string(),
        )
        .unwrap();
        std::future::pending::<()>().await;
        return;
    }
    let port = std::fs::read_to_string(root.join("relay.port")).unwrap();
    let relay: iroh::RelayUrl = format!("http://10.197.0.254:{port}").parse().unwrap();
    if role == "prepare" {
        let [a, b] = admitted_pair(&root).await;
        a.close().await.unwrap();
        b.close().await.unwrap();
        for name in ["provider", "consumer"] {
            let node = NativeNetwork::open(root.join(name)).await.unwrap();
            let mut config = node.resource_configuration().unwrap();
            config.replacement = Some(vec![Resource {
                url: relay.to_string(),
                uses: [ResourceUse::DataRelay].into(),
                limitation: "owned two-gateway dual-stack acceptance relay".into(),
            }]);
            node.save_resource_configuration(&config).unwrap();
            node.close().await;
        }
        return;
    }
    timeout(Duration::from_secs(30), async {
        let other = if role == "provider" { "consumer" } else { "provider" };
        let client = NativeClient::open(root.join(&role), Ipv4Addr::UNSPECIFIED).await.unwrap();
        std::fs::write(root.join(format!("{role}.id")), client.address().id.to_string()).unwrap();
        while !root.join(format!("{other}.id")).exists() { sleep(Duration::from_millis(10)).await; }
        let peer = std::fs::read_to_string(root.join(format!("{other}.id"))).unwrap().parse().unwrap();
        let reply = client.ping(EndpointAddr::new(peer).with_relay_url(relay)).await.unwrap();
        assert_eq!(reply["reply"], "PONG");
        let original = client.test_member_connection(peer).await.unwrap();
        assert!(original.initial_data_path().is_relay());
        let stable = original.stable_id();
        let runtime = client.runtime();
        let fusion = &runtime.resources.member_fusion.as_ref().unwrap().0;
        let port = runtime.resources.endpoint().bound_sockets().into_iter().find(|addr| addr.is_ipv6()).unwrap().port();
        for number in 1..=40 {
            let v4 = SocketAddr::from((Ipv4Addr::new(192, 0, 2, number), port));
            let v6: SocketAddr = format!("[2001:db8::{number:x}]:{port}").parse().unwrap();
            fusion.test_add_external_addr(v4).await;
            fusion.test_add_external_addr(v6).await;
        }
        std::fs::write(root.join(format!("{role}.relay")), b"PONG on original relay").unwrap();
        while !root.join("release").exists() { sleep(Duration::from_millis(10)).await; }
        let local_ip: Ipv6Addr = if role == "provider" { "fd00:197:1::1" } else { "fd00:197:1::2" }.parse().unwrap();
        let peer_ip: Ipv6Addr = if role == "provider" { "fd00:197:1::2" } else { "fd00:197:1::1" }.parse().unwrap();
        fusion.test_add_external_addr(SocketAddr::from((local_ip, port))).await;
        drop(runtime);
        timeout(Duration::from_secs(18), async {
            while !original.paths().iter().any(|path| path.is_selected()
                && matches!(path.remote_addr(), iroh::TransportAddr::Ip(addr) if addr.ip() == peer_ip)
                && path.stats().acknowledgment.is_some()) {
                sleep(Duration::from_millis(20)).await;
            }
        }).await.unwrap_or_else(|error| panic!("IPv6 ordinary QNT must cross both filters: {error}; paths={:?}", original.paths()));
        let reply = client.ping(EndpointAddr::new(peer)).await.unwrap();
        assert_eq!(reply["reply"], "PONG");
        assert_eq!(client.test_member_connection(peer).await.unwrap().stable_id(), stable);
        println!("DUAL_STACK_FILTERED_DATA role={role} original={stable} paths={:?} reach_out={} reply={reply}", original.paths(), original.stats().frame_tx.reach_out);
        std::fs::write(root.join(format!("{role}.done")), b"PONG on original IPv6").unwrap();
        while !root.join(format!("{other}.done")).exists() { sleep(Duration::from_millis(10)).await; }
        client.close().await.unwrap();
    }).await.unwrap();
}

#[tokio::test]
async fn default_tcp_listeners_deliver_both_families_without_udp() {
    for ipv6 in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let clients = admitted_pair_with_config(
            root.path(),
            ResourceConfig {
                replacement: Some(Vec::new()),
                udp_direct_disabled: true,
                lan_discovery: LanDiscovery::Disabled,
                ..Default::default()
            },
        )
        .await;
        let target = timeout(Duration::from_secs(3), async {
            loop {
                let addr = clients[0].address();
                if let Some(route) = addr.addrs.iter().find(|route| {
                    matches!(route, iroh::TransportAddr::Custom(custom) if
                        crate::tcp_transport::TcpTransport::socket_addr(custom)
                            .is_ok_and(|address| address.is_ipv6() == ipv6 && !address.ip().is_unspecified()))
                }) {
                    break EndpointAddr::new(addr.id).with_addrs([route.clone()]);
                }
                sleep(Duration::from_millis(10)).await;
            }
        }).await.expect("default TCP listener advertises this family's actual addresses");
        let reply = timeout(Duration::from_secs(6), clients[1].ping(target))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reply["reply"], "PONG");
        let original = clients[1]
            .test_member_connection(clients[0].address().id)
            .await
            .unwrap();
        assert!(
            matches!(original.initial_data_path(), iroh::TransportAddr::Custom(address)
            if crate::tcp_transport::TcpTransport::socket_addr(address).unwrap().is_ipv6() == ipv6)
        );
        let reverse = clients[0]
            .ping(EndpointAddr::new(clients[1].address().id))
            .await
            .unwrap();
        assert_eq!(reverse["reply"], "PONG");
        println!(
            "TCP_FIRST_DATA ipv6={ipv6} original={} path={:?}",
            original.stable_id(),
            original.initial_data_path()
        );
        let [a, b] = clients;
        b.close().await.unwrap();
        a.close().await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires disposable network namespace and TCP receive filters"]
async fn tcp_later_batches_and_updates_improve_the_original_relay() {
    use crate::resources::{Resource, ResourceUse};
    use crate::tcp_transport::TcpTransport;
    use std::process::Command;
    for number in 1..=5 {
        assert!(
            Command::new("ip")
                .args([
                    "-6",
                    "addr",
                    "add",
                    &format!("fd00:198::{number}/128"),
                    "dev",
                    "lo",
                    "nodad"
                ])
                .status()
                .unwrap()
                .success()
        );
    }
    for case in ["third", "later-batch", "update"] {
        let later_update = case == "update";
        let mut relay_config = iroh_relay::server::ServerConfig::default();
        relay_config.relay = Some(iroh_relay::server::RelayConfig::new((
            Ipv4Addr::LOCALHOST,
            0,
        )));
        let relay = iroh_relay::server::Server::spawn(relay_config)
            .await
            .unwrap();
        let url = relay.http_url().unwrap();
        let root = tempfile::tempdir().unwrap();
        let clients = admitted_pair_with_config(
            root.path(),
            ResourceConfig {
                replacement: Some(vec![Resource {
                    url: url.to_string(),
                    uses: [ResourceUse::DataRelay].into(),
                    limitation: "owned TCP advertisement acceptance relay".into(),
                }]),
                udp_direct_disabled: true,
                lan_discovery: LanDiscovery::Disabled,
                ..Default::default()
            },
        )
        .await;
        let routes = timeout(Duration::from_secs(3), async {
            loop {
                let addresses: Vec<_> = clients[0]
                    .address()
                    .addrs
                    .iter()
                    .filter_map(|addr| match addr {
                        iroh::TransportAddr::Custom(addr) => TcpTransport::socket_addr(addr).ok(),
                        _ => None,
                    })
                    .collect();
                if addresses.iter().filter(|addr| addr.is_ipv6()).count() >= 6 {
                    break addresses;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let v6_port = routes.iter().find(|addr| addr.is_ipv6()).unwrap().port();
        let v4_port = routes.iter().find(|addr| addr.is_ipv4()).unwrap().port();
        let winning_ip: Ipv6Addr = match case {
            "third" => "fd00:198::1",
            "later-batch" => "fd00:198::5",
            _ => "fd00:198::99",
        }
        .parse()
        .unwrap();
        let expected = SocketAddr::from((winning_ip, v6_port));
        if !later_update {
            assert!(routes.iter().position(|addr| *addr == expected).unwrap() >= 2);
        }
        // TCP SYN reaches the kernel, then the receiving firewall drops it.
        assert!(
            Command::new("iptables")
                .args([
                    "-I",
                    "INPUT",
                    "-p",
                    "tcp",
                    "--dport",
                    &v4_port.to_string(),
                    "-j",
                    "DROP"
                ])
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new("ip6tables")
                .args([
                    "-I",
                    "INPUT",
                    "-p",
                    "tcp",
                    "--dport",
                    &v6_port.to_string(),
                    "!",
                    "-d",
                    &winning_ip.to_string(),
                    "-j",
                    "DROP"
                ])
                .status()
                .unwrap()
                .success()
        );
        let reply = clients[1]
            .ping(EndpointAddr::new(clients[0].address().id).with_relay_url(url))
            .await
            .unwrap();
        assert_eq!(reply["reply"], "PONG");
        let original = clients[1]
            .test_member_connection(clients[0].address().id)
            .await
            .unwrap();
        assert!(original.initial_data_path().is_relay());
        let stable = original.stable_id();
        if later_update {
            assert!(!routes.contains(&expected));
            assert!(
                Command::new("ip")
                    .args([
                        "-6",
                        "addr",
                        "add",
                        "fd00:198::99/128",
                        "dev",
                        "lo",
                        "nodad"
                    ])
                    .status()
                    .unwrap()
                    .success()
            );
        }
        timeout(Duration::from_secs(28), async {
            while !original.paths().iter().any(|path| {
                path.is_selected()
                    && path.remote_addr()
                        == &iroh::TransportAddr::Custom(TcpTransport::address(expected))
                    && path.stats().acknowledgment.is_some()
            }) {
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|error| {
            panic!(
                "TCP later_update={later_update}: {error}; paths={:?}",
                original.paths()
            )
        });
        assert_eq!(
            clients[1]
                .ping(EndpointAddr::new(clients[0].address().id))
                .await
                .unwrap()["reply"],
            "PONG"
        );
        assert_eq!(
            clients[0]
                .ping(EndpointAddr::new(clients[1].address().id))
                .await
                .unwrap()["reply"],
            "PONG"
        );
        assert_eq!(
            clients[1]
                .test_member_connection(clients[0].address().id)
                .await
                .unwrap()
                .stable_id(),
            stable
        );
        println!(
            "TCP_LATER_DATA case={case} update={later_update} original={stable} expected={expected} advertised={routes:?}"
        );
        assert!(
            Command::new("iptables")
                .args([
                    "-D",
                    "INPUT",
                    "-p",
                    "tcp",
                    "--dport",
                    &v4_port.to_string(),
                    "-j",
                    "DROP"
                ])
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new("ip6tables")
                .args([
                    "-D",
                    "INPUT",
                    "-p",
                    "tcp",
                    "--dport",
                    &v6_port.to_string(),
                    "!",
                    "-d",
                    &winning_ip.to_string(),
                    "-j",
                    "DROP"
                ])
                .status()
                .unwrap()
                .success()
        );
        let [a, b] = clients;
        b.close().await.unwrap();
        a.close().await.unwrap();
        relay.shutdown().await.unwrap();
        if later_update {
            assert!(
                Command::new("ip")
                    .args(["-6", "addr", "del", "fd00:198::99/128", "dev", "lo"])
                    .status()
                    .unwrap()
                    .success()
            );
        }
    }
}

#[tokio::test]
#[ignore = "requires disposable namespace for forty real local TCP addresses"]
async fn many_tcp_interfaces_preserve_bounded_invitation_bootstrap() {
    use crate::resources::{Resource, ResourceUse};
    use crate::tcp_transport::TcpTransport;
    for number in 1..=40 {
        assert!(
            std::process::Command::new("ip")
                .args(["addr", "add", &format!("10.198.0.{number}/32"), "dev", "lo"])
                .status()
                .unwrap()
                .success()
        );
    }
    let mut relay_config = iroh_relay::server::ServerConfig::default();
    relay_config.relay = Some(iroh_relay::server::RelayConfig::new((
        Ipv4Addr::LOCALHOST,
        0,
    )));
    let relay = iroh_relay::server::Server::spawn(relay_config)
        .await
        .unwrap();
    let url = relay.http_url().unwrap();
    let root = tempfile::tempdir().unwrap();
    let clients = admitted_pair_with_config(
        root.path(),
        ResourceConfig {
            replacement: Some(vec![Resource {
                url: url.to_string(),
                uses: [ResourceUse::DataRelay].into(),
                limitation: "owned many-interface bootstrap relay".into(),
            }]),
            udp_direct_disabled: true,
            lan_discovery: LanDiscovery::Disabled,
            ..Default::default()
        },
    )
    .await;
    timeout(Duration::from_secs(3), async {
        while clients[0].address().addrs.len() <= 32 {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let invitation = clients[0].invitation().await.unwrap();
    assert_eq!(invitation.address().addrs.len(), 32);
    assert!(invitation.address().relay_urls().any(|relay| *relay == url));
    for ipv6 in [false, true] {
        assert!(invitation.address().addrs.iter().any(|route| matches!(route,
            iroh::TransportAddr::Custom(custom) if TcpTransport::socket_addr(custom).is_ok_and(|addr| addr.is_ipv6() == ipv6))));
    }
    assert!(
        crate::client_invitation::ClientInvitation::decode(&invitation.encode().unwrap()).is_ok()
    );
    println!(
        "TCP_INVITATION_BOUND full={} invitation={}",
        clients[0].address().addrs.len(),
        invitation.address().addrs.len()
    );
    let [a, b] = clients;
    b.close().await.unwrap();
    a.close().await.unwrap();
    relay.shutdown().await.unwrap();
}
