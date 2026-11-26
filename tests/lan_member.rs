//! Offline LAN acceptance uses two independently reopened SDK processes.
#![cfg(feature = "nat-lab-test-hooks")]
#[path = "support/native_connections.rs"]
mod native_connections;
use iroh::EndpointAddr;
use miopunch_native_probe::{
    admission_gate::ADMISSION_REQUEST_ALPN,
    manual_admission::ManualExchange,
    native_network::NativeNetwork,
    resources::{LanDiscovery, Resource, ResourceUse},
};
use std::{net::Ipv4Addr, time::Duration};
async fn join(admin: &mut NativeNetwork, path: &std::path::Path) -> NativeNetwork {
    let mut node = NativeNetwork::join(path, admin.manual_invitation().unwrap())
        .await
        .unwrap();
    let ae = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let ne = node.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = native_connections::control_pair(&ne, &ae, ADMISSION_REQUEST_ALPN).await;
    let (request, served) = tokio::join!(
        node.request_manual_admission(&client),
        admin.serve_manual_admission(&server)
    );
    assert!(matches!(request.unwrap(), ManualExchange::AwaitingApproval));
    served.unwrap();
    let request = admin.manual_requests().await.unwrap().pop().unwrap();
    admin.approve_manual_admission(&request).await.unwrap();
    let receive = async {
        let ManualExchange::Materials(material) =
            node.request_manual_admission(&client).await.unwrap()
        else {
            panic!("approval")
        };
        node.complete_manual_admission(material).await.unwrap();
    };
    let (_, served) = tokio::join!(receive, admin.serve_manual_admission(&server));
    served.unwrap();
    ne.close().await;
    ae.close().await;
    node
}

#[tokio::test]
async fn offline_lan_member() {
    let Ok(directory) = std::env::var("MIO_LAN_DIRECTORY") else {
        return;
    };
    let directory = std::path::Path::new(&directory);
    let role = std::env::var("MIO_LAN_ROLE").unwrap();
    if role == "prepare" {
        let mut server = NativeNetwork::create(directory.join("server"))
            .await
            .unwrap();
        let client = join(&mut server, &directory.join("client")).await;
        for (role, node) in [("server", &server), ("client", &client)] {
            let mut config = node.resource_configuration().unwrap();
            config.replacement = Some(vec![Resource {
                url: "http://192.0.2.1:3340".into(),
                uses: [ResourceUse::DataRelay].into(),
                limitation: "offline LAN acceptance unreachable public contact".into(),
            }]);
            config.tcp_direct_disabled = true;
            config.lan_discovery = LanDiscovery::Active;
            node.save_resource_configuration(&config).unwrap();
            let endpoint = node.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
            std::fs::write(
                directory.join(format!("{role}.id")),
                endpoint.id().to_string(),
            )
            .unwrap();
            endpoint.close().await;
        }
        client.close().await;
        server.close().await;
        let admin = miopunch_native_probe::native_client::NativeClient::open(
            directory.join("server"),
            Ipv4Addr::LOCALHOST,
        )
        .await
        .unwrap();
        admin
            .export_recovery_card_to(directory.join("card"), None)
            .await
            .unwrap();
        admin.acknowledge_recovery_saved().await.unwrap();
        admin.close().await.unwrap();
        return;
    }
    tokio::time::timeout(Duration::from_secs(20), async {
        let client = miopunch_native_probe::native_client::NativeClient::open(
            directory.join(&role), Ipv4Addr::UNSPECIFIED,
        ).await.unwrap();
        let ipv6_only = std::env::var("MIO_LAN_IPV6_ONLY").is_ok();
        let other = if role == "server" { "client" } else { "server" };
        let peer = std::fs::read_to_string(directory.join(format!("{other}.id"))).unwrap().parse().unwrap();
        let started = std::time::Instant::now();
        let target = EndpointAddr::new(peer).with_relay_url("http://192.0.2.1:3340".parse().unwrap());
        let reply = client.ping(target.clone()).await.unwrap();
        assert_eq!(reply["reply"], "PONG");
        let connection = client.test_member_connection(peer).await.unwrap();
        assert_eq!(connection.remote_id(), peer);
        let stable_id = connection.stable_id();
        tokio::time::timeout(Duration::from_secs(4), async {
            while !connection.paths().iter().any(|path| path.is_selected() && path.is_ip()) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.unwrap_or_else(|error| panic!("selected LAN path after PONG: {error}; reply={reply}; paths={:?}", connection.paths()));
        let paths = connection.paths();
        if ipv6_only {
            let local_index: u32 = std::env::var("MIO_LAN_IFINDEX").unwrap().parse().unwrap();
            assert!(paths.iter().any(|path| path.is_selected() && matches!(path.remote_addr(), iroh::TransportAddr::Ip(std::net::SocketAddr::V6(addr)) if addr.ip().is_unicast_link_local() && addr.scope_id()==local_index)));
        }
        let again = client.ping(target).await.unwrap();
        assert_eq!(again["reply"], "PONG");
        assert_eq!(client.test_member_connection(peer).await.unwrap().stable_id(), stable_id);
        println!("LAN_DATA_OK role={role} ipv6_only={ipv6_only} elapsed_ms={} paths={paths:?} reply={reply}", started.elapsed().as_millis());
        std::fs::write(directory.join(format!("{role}.done")), b"data received").unwrap();
        while !directory.join(format!("{other}.done")).exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        client.close().await.unwrap();
    }).await.expect("offline LAN member data before original deadline");
}
