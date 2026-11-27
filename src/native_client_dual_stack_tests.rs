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
            // Installed hosts can explicitly report changes on platforms that
            // do not provide an automatic interface event monitor.
            clients[0].runtime().resources.endpoint().network_change().await;
            clients[1].runtime().resources.endpoint().network_change().await;
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
    let guest = clients[0].issue_service_guest(
        clients[0].address(), crate::service_policy::Service::Files,
        crate::service_policy::now() + 300,
    ).await.unwrap();
    assert_eq!(guest.provider.addrs.len(), 32);
    assert!(crate::service_policy::GuestInvitation::decode(&guest.encode().unwrap()).is_ok());
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

/// Driven by the existing two-node namespace topology, with one live forwarding stream.
#[tokio::test]
#[ignore = "requires scripts/check_native_network_change.sh"]
async fn actual_interface_changes_preserve_valid_work_and_the_original_forwarding_stream() {
    use crate::{
        forwarding::Protocol,
        resources::{Resource, ResourceUse},
    };
    use iroh::{TransportAddr, endpoint::ObservationFamily};
    use std::{
        net::IpAddr,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream, UdpSocket};
    let root = std::path::PathBuf::from(std::env::var("MIO205_ROOT").unwrap());
    let role = std::env::var("MIO205_ROLE").unwrap();
    let tcp = std::env::var("MIO205_TRANSPORT").is_ok_and(|mode| mode == "tcp");
    enum SourceObservation {
        Udp(iroh::endpoint::UdpMappingSequence),
        Tcp(crate::resources::TcpStunObservation),
    }
    impl SourceObservation {
        fn is_current(&self) -> bool {
            match self {
                Self::Udp(value) => value.is_current_generation(),
                Self::Tcp(value) => value.is_fresh(),
            }
        }
        fn mapped(&self) -> Vec<SocketAddr> {
            match self {
                Self::Udp(value) => value.samples.iter().map(|sample| sample.mapped).collect(),
                Self::Tcp(value) => vec![value.mapped],
            }
        }
    }
    async fn observe(
        resources: &crate::resources::ResourceEndpoint,
        observers: &[iroh::RelayUrl],
        family: ObservationFamily,
        tcp: bool,
        budget: u64,
    ) -> std::io::Result<SourceObservation> {
        if tcp {
            resources
                .observe_tcp_stun_for_family(observers.last().unwrap(), family)
                .await
                .map(SourceObservation::Tcp)
        } else {
            resources
                .endpoint()
                .observe_udp_mapping_round_for_family_until(
                    &iroh::RelayMap::empty(),
                    observers,
                    family,
                    tokio::time::Instant::now() + Duration::from_secs(budget),
                )
                .await
                .map(SourceObservation::Udp)
        }
    }
    fn candidates(address: &EndpointAddr, tcp: bool) -> Vec<SocketAddr> {
        address
            .addrs
            .iter()
            .filter_map(|address| match address {
                TransportAddr::Ip(value) if !tcp => Some(*value),
                TransportAddr::Custom(value) if tcp => {
                    crate::tcp_transport::TcpTransport::socket_addr(value).ok()
                }
                _ => None,
            })
            .collect()
    }
    fn carrier(address: SocketAddr, tcp: bool) -> TransportAddr {
        if tcp {
            TransportAddr::Custom(crate::tcp_transport::TcpTransport::address(address))
        } else {
            TransportAddr::Ip(address)
        }
    }
    async fn signal(root: &Path, name: &str) {
        timeout(Duration::from_secs(35), async {
            while !root.join(name).exists() {
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|error| panic!("missing {name}: {error}"));
    }
    if role == "relay" {
        fn binding_reply(packet: &[u8], source: SocketAddr) -> Vec<u8> {
            let mut reply = vec![
                1,
                1,
                0,
                if source.is_ipv6() { 24 } else { 12 },
                0x21,
                0x12,
                0xa4,
                0x42,
            ];
            reply.extend_from_slice(&packet[8..20]);
            reply.extend_from_slice(&[
                0,
                0x20,
                0,
                if source.is_ipv6() { 20 } else { 8 },
                0,
                if source.is_ipv6() { 2 } else { 1 },
            ]);
            reply.extend_from_slice(&(source.port() ^ 0x2112).to_be_bytes());
            match source.ip() {
                IpAddr::V4(ip) => {
                    reply.extend_from_slice(&(u32::from(ip) ^ 0x2112a442).to_be_bytes())
                }
                IpAddr::V6(ip) => {
                    for (address, mask) in ip.octets().iter().zip(&packet[4..20]) {
                        reply.push(address ^ mask);
                    }
                }
            }
            reply
        }
        let mut config = iroh_relay::server::ServerConfig::default();
        config.relay = Some(iroh_relay::server::RelayConfig::new((
            Ipv4Addr::UNSPECIFIED,
            0,
        )));
        let relay = iroh_relay::server::Server::spawn(config).await.unwrap();
        for (ip, port) in [
            ("10.205.0.254".parse::<IpAddr>().unwrap(), 20580),
            ("10.205.0.254".parse().unwrap(), 20581),
            ("2001:db8:205::fe".parse().unwrap(), 20582),
            ("2001:db8:205::fe".parse().unwrap(), 20583),
            ("10.205.0.254".parse().unwrap(), 20584),
        ] {
            if port == 20584 && !tcp {
                continue;
            }
            if tcp {
                let listener = TcpListener::bind((ip, port)).await.unwrap();
                let root = root.clone();
                tokio::spawn(async move {
                    loop {
                        let (mut socket, source) = listener.accept().await.unwrap();
                        let root = root.clone();
                        tokio::spawn(async move {
                            let mut packet = [0; 20];
                            socket.read_exact(&mut packet).await.unwrap();
                            std::fs::write(root.join(format!("observed-{}", source.ip())), [])
                                .unwrap();
                            if root.join("hold-work").exists() {
                                std::fs::write(
                                    root.join(format!("work-observed-{}-{port}", source.ip())),
                                    [],
                                )
                                .unwrap();
                                signal(&root, "interface").await;
                            }
                            if socket
                                .write_all(&binding_reply(&packet, source))
                                .await
                                .is_ok()
                            {
                                let mut end = Vec::new();
                                let _ = socket.read_to_end(&mut end).await;
                            }
                        });
                    }
                });
                continue;
            }
            let socket = Arc::new(UdpSocket::bind((ip, port)).await.unwrap());
            let root = root.clone();
            tokio::spawn(async move {
                loop {
                    let mut packet = [0; 1500];
                    let (length, source) = socket.recv_from(&mut packet).await.unwrap();
                    assert_eq!(length, 20);
                    let root = root.clone();
                    let socket = socket.clone();
                    tokio::spawn(async move {
                        std::fs::write(root.join(format!("observed-{}", source.ip())), []).unwrap();
                        if source.is_ipv4() && root.join("hold-v4").exists() {
                            signal(&root, "release-v4").await;
                        }
                        if root.join("hold-work").exists() {
                            std::fs::write(root.join(format!("work-observed-{}", source.ip())), [])
                                .unwrap();
                            signal(&root, "interface").await;
                        }
                        let reply = binding_reply(&packet, source);
                        socket.send_to(&reply, source).await.unwrap();
                    });
                }
            });
        }
        std::fs::write(
            root.join("relay.port"),
            relay.http_addr().unwrap().port().to_string(),
        )
        .unwrap();
        std::future::pending::<()>().await;
        return;
    }
    let port = std::fs::read_to_string(root.join("relay.port")).unwrap();
    let relay: iroh::RelayUrl = format!("http://10.205.0.254:{port}").parse().unwrap();
    let mut stuns: Vec<iroh::RelayUrl> = [
        "stun://10.205.0.254:20580/",
        "stun://10.205.0.254:20581/",
        "stun://[2001:db8:205::fe]:20582/",
        "stun://[2001:db8:205::fe]:20583/",
    ]
    .into_iter()
    .map(|value| {
        if tcp {
            value.replacen("stun:", "stun+tcp:", 1).parse().unwrap()
        } else {
            value.parse().unwrap()
        }
    })
    .collect();
    if tcp {
        stuns.push("stun+tcp://10.205.0.254:20584/".parse().unwrap());
    }
    if role == "prepare" {
        let [a, b] = Box::pin(admitted_pair(&root)).await;
        a.close().await.unwrap();
        b.close().await.unwrap();
        for name in ["provider", "consumer"] {
            let node = NativeNetwork::open(root.join(name)).await.unwrap();
            let mut config = node.resource_configuration().unwrap();
            let mut resources = stuns
                .iter()
                .map(|url| Resource {
                    url: url.to_string(),
                    uses: [ResourceUse::Observation].into(),
                    limitation: "owned actual source observer".into(),
                })
                .collect::<Vec<_>>();
            resources.push(Resource {
                url: relay.to_string(),
                uses: [ResourceUse::DataRelay].into(),
                limitation: "owned unchanged relay".into(),
            });
            config.replacement = Some(resources);
            config.tcp_direct_disabled = !tcp;
            config.udp_direct_disabled = tcp;
            config.maintenance.quality_interval_ms = 30_000;
            config.maintenance.contact_timeout_ms = 12_000;
            node.save_resource_configuration(&config).unwrap();
            node.close().await;
        }
        return;
    }
    let operation = async {
        let provider = role == "provider";
        let moving = std::env::var("MIO205_MOVING").unwrap_or_else(|_| "both".into());
        let local_moves = moving == "both" || provider;
        let peer_moves = moving == "both" || !provider;
        let other = if provider { "consumer" } else { "provider" };
        let number = if provider { 1 } else { 2 };
        let peer_number = 3 - number;
        let client = NativeClient::open(root.join(&role), Ipv4Addr::UNSPECIFIED)
            .await
            .unwrap();
        std::fs::write(
            root.join(format!("{role}.id")),
            client.address().id.to_string(),
        )
        .unwrap();
        signal(&root, &format!("{other}.id")).await;
        let peer = std::fs::read_to_string(root.join(format!("{other}.id")))
            .unwrap()
            .parse()
            .unwrap();
        let original = timeout(Duration::from_secs(15), async {
            loop {
                if let Ok(connection) = client.test_member_connection(peer).await {
                    break connection;
                }
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("normal client startup admits its shared member before the interface scenario");
        let received = Arc::new(AtomicUsize::new(0));
        let mut stream = None;
        let mut echo = None;
        if provider {
            let sink = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let target = sink.local_addr().unwrap();
            client
                .enable_forwarding(Protocol::Tcp, vec![target])
                .await
                .unwrap();
            std::fs::write(root.join("target"), target.to_string()).unwrap();
            let echo_received = received.clone();
            echo = Some(tokio::spawn(async move {
                let (mut socket, _) = sink.accept().await.unwrap();
                loop {
                    let mut packet = [0; 8];
                    if socket.read_exact(&mut packet).await.is_err() {
                        break;
                    }
                    socket
                        .write_all(&(!u64::from_be_bytes(packet)).to_be_bytes())
                        .await
                        .unwrap();
                    echo_received.fetch_add(1, Ordering::SeqCst);
                }
            }));
            timeout(Duration::from_secs(10), async {
                while received.load(Ordering::SeqCst) == 0 {
                    sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        } else {
            signal(&root, "target").await;
            let target = std::fs::read_to_string(root.join("target"))
                .unwrap()
                .parse()
                .unwrap();
            let listener = client
                .start_forwarding(
                    EndpointAddr::new(peer).with_relay_url(relay.clone()),
                    Protocol::Tcp,
                    target,
                    (Ipv4Addr::LOCALHOST, 0).into(),
                )
                .await
                .unwrap();
            let local = serde_json::from_value(listener["listen"].clone()).unwrap();
            let mut socket = TcpStream::connect::<SocketAddr>(local).await.unwrap();
            socket.write_all(&1u64.to_be_bytes()).await.unwrap();
            let mut reply = [0; 8];
            socket.read_exact(&mut reply).await.unwrap();
            assert_eq!(u64::from_be_bytes(reply), !1);
            stream = Some(socket);
        }
        let stable = original.stable_id();
        let runtime = client.runtime();
        let endpoint = runtime.resources.endpoint().clone();
        let fusion = runtime.resources.member_fusion.as_ref().unwrap().0.clone();
        let local = if tcp {
            candidates(&client.address(), true)
        } else {
            endpoint.bound_sockets()
        };
        let v4_port = local.iter().find(|addr| addr.is_ipv4()).unwrap().port();
        let v6_port = local.iter().find(|addr| addr.is_ipv6()).unwrap().port();
        std::fs::write(
            root.join(format!("{role}.ports")),
            format!("{v4_port} {v6_port}"),
        )
        .unwrap();
        signal(&root, &format!("{other}.ports")).await;
        let peer_ports = std::fs::read_to_string(root.join(format!("{other}.ports")))
            .unwrap()
            .split_whitespace()
            .map(|value| value.parse::<u16>().unwrap())
            .collect::<Vec<_>>();
        let peer4: SocketAddr = format!("10.205.0.{peer_number}:{}", peer_ports[0])
            .parse()
            .unwrap();
        fusion
            .learn_path_candidates(EndpointAddr::new(peer).with_addrs([carrier(peer4, tcp)]))
            .await;
        timeout(Duration::from_secs(8), async {
            while !original.paths().iter().any(|path| {
                path.remote_addr() == &carrier(peer4, tcp) && path.stats().acknowledgment.is_some()
            }) {
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let v4_path = original
            .paths()
            .iter()
            .find(|path| path.remote_addr() == &carrier(peer4, tcp))
            .unwrap()
            .id();
        let old4: SocketAddr = format!("10.205.0.{number}:{v4_port}").parse().unwrap();
        let old6: SocketAddr = format!("[2001:db8:205::{number}]:{v6_port}")
            .parse()
            .unwrap();
        let v4_valid = endpoint
            .binding_route_generation(old4, "10.205.0.254:20580".parse().unwrap())
            .unwrap();
        let v6_valid = endpoint
            .binding_route_generation(old6, "[2001:db8:205::fe]:20582".parse().unwrap())
            .unwrap();
        let v6_observation = observe(
            &runtime.resources,
            &stuns[2..4],
            ObservationFamily::Ipv6,
            tcp,
            5,
        )
        .await
        .unwrap();
        assert!(v6_observation.mapped().iter().all(|sample| *sample == old6));
        let initial_v4 = if tcp {
            Some(
                observe(
                    &runtime.resources,
                    &stuns[..2],
                    ObservationFamily::Ipv4,
                    true,
                    5,
                )
                .await
                .unwrap(),
            )
        } else {
            None
        };
        std::fs::write(root.join(format!("{role}.observation-ready")), []).unwrap();
        signal(&root, "start-observer").await;
        let observer = if tcp {
            None
        } else {
            Some(tokio::spawn({
                let resources = runtime.resources.clone();
                let stuns = stuns[..2].to_vec();
                async move { observe(&resources, &stuns, ObservationFamily::Ipv4, false, 8).await }
            }))
        };
        signal(&root, &format!("observed-10.205.0.{number}")).await;
        std::fs::write(root.join(format!("{role}.ready")), []).unwrap();
        signal(&root, "prefix").await;
        if local_moves {
            timeout(Duration::from_secs(3), async {
                while v6_valid.is_current() {
                    sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        } else {
            assert!(v6_valid.is_current());
        }
        assert!(
            v4_valid.is_current(),
            "the other family's exact source and route remain current"
        );
        if local_moves {
            assert!(!v6_observation.is_current());
        }
        assert!(
            original.paths().get(v4_path).is_some(),
            "healthy IPv4 path keeps its actual path identity"
        );
        std::fs::write(root.join(format!("{role}.prefix-refreshed")), []).unwrap();
        let v4_observation = match observer {
            Some(observer) => observer.await.unwrap().unwrap(),
            None => initial_v4.unwrap(),
        };
        assert!(v4_observation.is_current());
        assert!(v4_observation.mapped().iter().all(|sample| *sample == old4));
        let mut held_round: Option<
            tokio::task::JoinHandle<std::io::Result<iroh::endpoint::MemberUdpPlan>>,
        > = None;
        let mut held_v4: Option<tokio::task::JoinHandle<std::io::Result<SourceObservation>>> = None;
        for stage in [1, 2] {
            if stage == 2 {
                signal(&root, "interface").await;
                if local_moves {
                    timeout(Duration::from_secs(3), async {
                        while v4_valid.is_current() {
                            sleep(Duration::from_millis(10)).await;
                        }
                    })
                    .await
                    .unwrap();
                }
                if let Some(work) = held_round.take() {
                    let result: std::io::Result<iroh::endpoint::MemberUdpPlan> =
                        timeout(Duration::from_secs(1), work)
                            .await
                            .unwrap()
                            .unwrap();
                    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::Interrupted);
                    println!(
                        "NETWORK_CHANGE_RELEASED role={role} family=IPv6 physical=held-before-sampling result=Interrupted"
                    );
                }
                if let Some(work) = held_v4.take() {
                    let result = timeout(Duration::from_secs(1), work)
                        .await
                        .unwrap()
                        .unwrap();
                    assert_eq!(
                        result
                            .err()
                            .expect("old source query must be cancelled")
                            .kind(),
                        std::io::ErrorKind::Interrupted
                    );
                }
            }
            let local_prefix = if local_moves { stage } else { 0 };
            let peer_prefix = if peer_moves { stage } else { 0 };
            let expected: IpAddr = format!("2001:db8:205:{local_prefix}::{number}")
                .parse()
                .unwrap();
            timeout(Duration::from_secs(4), async {
                while !candidates(&client.address(), tcp)
                    .iter()
                    .any(|addr| addr.ip() == expected)
                {
                    sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            assert!(
                !local_moves
                    || !candidates(&client.address(), tcp)
                        .iter()
                        .any(|addr| addr.ip() == old6.ip())
            );
            std::fs::write(
                root.join(format!("{role}.addr-{stage}")),
                serde_json::to_vec(&client.address()).unwrap(),
            )
            .unwrap();
            signal(&root, &format!("{other}.addr-{stage}")).await;
            let current: EndpointAddr = serde_json::from_slice(
                &std::fs::read(root.join(format!("{other}.addr-{stage}"))).unwrap(),
            )
            .unwrap();
            assert!(candidates(&current, tcp).iter().any(|address| {
                address.ip()
                    == format!("2001:db8:205:{peer_prefix}::{peer_number}")
                        .parse::<IpAddr>()
                        .unwrap()
            }));
            let peer6: SocketAddr = format!(
                "[2001:db8:205:{peer_prefix}::{peer_number}]:{}",
                peer_ports[1]
            )
            .parse()
            .unwrap();
            timeout(Duration::from_secs(8), async {
                while !original.paths().iter().any(|path| {
                    path.remote_addr() == &carrier(peer6, tcp)
                        && path.stats().acknowledgment.is_some()
                }) {
                    sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap_or_else(|error| {
                panic!("new IPv6 path {peer6}: {error}, {:?}", original.paths())
            });
            let fresh = if tcp && !local_moves {
                assert!(v6_observation.is_current());
                v6_observation.mapped()
            } else if tcp {
                observe(
                    &runtime.resources,
                    &stuns[2..4],
                    ObservationFamily::Ipv6,
                    true,
                    5,
                )
                .await
                .unwrap()
                .mapped()
            } else {
                // Automatic traversal owns its same-family round. Reuse only its
                // accepted, still-current plan when it supersedes this public query.
                let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
                match observe(
                    &runtime.resources,
                    &stuns[2..4],
                    ObservationFamily::Ipv6,
                    false,
                    5,
                )
                .await
                {
                    Ok(samples) => {
                        assert!(samples.is_current());
                        samples.mapped()
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                        tokio::time::timeout_at(deadline, async {
                            loop {
                                if let Some(plan) =
                                    original.test_udp_plan_for_family(ObservationFamily::Ipv6)
                                {
                                    let samples = plan.local_samples();
                                    if !samples.is_empty()
                                        && samples
                                            .iter()
                                            .all(|sample| sample.mapped.ip() == expected)
                                    {
                                        break samples
                                            .iter()
                                            .map(|sample| sample.mapped)
                                            .collect::<Vec<_>>();
                                    }
                                }
                                sleep(Duration::from_millis(10)).await;
                            }
                        })
                        .await
                        .expect("superseding owner must publish current real new-source samples")
                    }
                    Err(error) => panic!("new source observation failed: {error}"),
                }
            };
            assert!(!fresh.is_empty() && fresh.iter().all(|sample| sample.ip() == expected));
            if stage == 2 {
                assert_eq!(v4_valid.is_current(), !local_moves);
                if local_moves {
                    assert!(!v4_observation.is_current());
                }
                let mapped = if tcp && !local_moves {
                    assert!(v4_observation.is_current());
                    v4_observation.mapped()
                } else {
                    let samples = observe(
                        &runtime.resources,
                        &stuns[..2],
                        ObservationFamily::Ipv4,
                        tcp,
                        5,
                    )
                    .await
                    .unwrap();
                    assert!(samples.is_current());
                    samples.mapped()
                };
                assert!(mapped.iter().all(|sample| {
                    sample.ip()
                        == format!("10.205.{}.{number}", usize::from(local_moves))
                            .parse::<IpAddr>()
                            .unwrap()
                }));
            }
            if let Some(socket) = &mut stream {
                socket
                    .write_all(&(stage + 1u64).to_be_bytes())
                    .await
                    .unwrap();
                let mut reply = [0; 8];
                timeout(Duration::from_secs(5), socket.read_exact(&mut reply))
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(u64::from_be_bytes(reply), !(stage + 1));
            } else {
                timeout(Duration::from_secs(8), async {
                    while received.load(Ordering::SeqCst) < stage as usize + 1 {
                        sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .unwrap();
            }
            assert_eq!(
                client
                    .test_member_connection(peer)
                    .await
                    .unwrap()
                    .stable_id(),
                stable
            );
            let flows = client.forwarding_status();
            assert!(
                flows["flows"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|flow| flow["transport"]["connection"] == stable)
            );
            println!(
                "NETWORK_CHANGE_ORIGINAL role={role} stage={stage} original={stable} paths={:?} flows={flows}",
                original.paths()
            );
            std::fs::write(root.join(format!("{role}.stage-{stage}")), []).unwrap();
            if stage == 1 {
                signal(&root, "start-work").await;
                if local_moves && !tcp {
                    let reserved = fusion
                        .test_coordinate_udp_reservation(Arc::new(tokio::sync::Barrier::new(2)));
                    held_round = Some(tokio::spawn({
                        let original = original.clone();
                        async move {
                            original
                                .coordinate_udp_traversal_for_family(
                                    ObservationFamily::Ipv6,
                                    tokio::time::Instant::now() + Duration::from_secs(8),
                                    &tokio_util::sync::CancellationToken::new(),
                                )
                                .await
                        }
                    }));
                    timeout(Duration::from_secs(2), async {
                        while !reserved.load(Ordering::Acquire) {
                            sleep(Duration::from_millis(5)).await;
                        }
                    })
                    .await
                    .unwrap();
                    println!(
                        "NETWORK_CHANGE_HELD role={role} family=IPv6 physical=held-before-sampling"
                    );
                }
                if local_moves {
                    held_v4 = Some(tokio::spawn({
                        let resources = runtime.resources.clone();
                        let stuns = if tcp {
                            stuns[4..].to_vec()
                        } else {
                            stuns[..2].to_vec()
                        };
                        async move { observe(&resources, &stuns, ObservationFamily::Ipv4, tcp, 8).await }
                    }));
                }
                std::fs::write(root.join(format!("{role}.held")), []).unwrap();
            }
        }
        signal(&root, &format!("{other}.stage-2")).await;
        drop(stream);
        if let Some(echo) = echo {
            echo.abort();
        }
        original.test_close(0, b"network-change terminal boundary");
        timeout(Duration::from_secs(3), original.closed())
            .await
            .unwrap();
        assert!(
            original
                .repair_path_until(
                    tokio::time::Instant::now() + Duration::from_secs(1),
                    tokio_util::sync::CancellationToken::new(),
                )
                .await
                .is_err()
        );
        std::fs::write(root.join(format!("{role}.terminal")), []).unwrap();
        signal(&root, &format!("{other}.terminal")).await;
        let reply = client.ping(EndpointAddr::new(peer)).await.unwrap();
        assert_eq!(reply["reply"], "PONG");
        let replacement = client.test_member_connection(peer).await.unwrap();
        assert_ne!(replacement.stable_id(), stable);
        println!(
            "NETWORK_CHANGE_REDIAL role={role} original={stable} replacement={} reply={reply}",
            replacement.stable_id()
        );
        std::fs::write(root.join(format!("{role}.redial")), []).unwrap();
        signal(&root, &format!("{other}.redial")).await;
        drop(runtime);
        client.close().await.unwrap();
    };
    timeout(Duration::from_secs(85), Box::pin(operation))
        .await
        .unwrap();
}
