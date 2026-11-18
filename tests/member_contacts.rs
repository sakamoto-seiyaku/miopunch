//! Member address learning through the public Native SDK, including reopening.
#[path = "support/native_connections.rs"]
mod native_connections;
use iroh::{
    EndpointAddr, TransportAddr,
    endpoint::Connection,
    protocol::{AcceptError, ProtocolHandler},
};
use miopunch_native_probe::{
    admission_gate::ADMISSION_REQUEST_ALPN, connection_coordinator::MEMBER_ALPN,
    manual_admission::ManualExchange, native_network::NativeNetwork,
};
use std::{net::Ipv4Addr, time::Duration};

#[derive(Clone, Debug)]
struct Echo;
impl ProtocolHandler for Echo {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        while let Ok((mut send, mut recv)) = connection.accept_bi().await {
            let Ok(bytes) = recv.read_to_end(128).await else {
                break;
            };
            if send.write_all(&bytes).await.is_err() {
                break;
            }
            send.finish()?;
        }
        Ok(())
    }
}
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
fn configure(node: &NativeNetwork) {
    let mut config = node.resource_configuration().unwrap();
    config.replacement = Some(vec![]);
    config.tcp_direct_disabled = true;
    node.save_resource_configuration(&config).unwrap();
}
async fn request(connection: &Connection, sequence: u64) {
    let (mut send, mut recv) = connection.open_bi().await.unwrap();
    let bytes = sequence.to_be_bytes();
    send.write_all(&bytes).await.unwrap();
    send.finish().unwrap();
    assert_eq!(recv.read_to_end(128).await.unwrap(), bytes);
}
#[tokio::test]
async fn member_redial_learns_active_address_and_survives_reopen() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let directory = tempfile::tempdir().unwrap();
        let client_path = directory.path().join("client");
        let mut server = NativeNetwork::create(directory.path().join("server"))
            .await
            .unwrap();
        let client = join(&mut server, &client_path).await;
        for node in [&server, &client] {
            configure(node);
        }
        let server_endpoint = server
            .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap();
        let accepting = server_endpoint.endpoint().clone();
        let router = tokio::spawn(async move {
            while let Some(incoming) = accepting.accept().await {
                tokio::spawn(async move {
                    if let Ok(c) = incoming.await {
                        let _ = Echo.accept(c).await;
                    }
                });
            }
        });
        let client_endpoint = client
            .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap()
            .with_contact_timeout(Duration::from_secs(2))
            .unwrap();
        let actual = server_endpoint.addr();
        let old = EndpointAddr::new(actual.id)
            .with_addrs([TransportAddr::Ip("127.0.0.1:9".parse().unwrap())]);
        let first = client_endpoint
            .connect(actual.clone(), MEMBER_ALPN)
            .await
            .unwrap();
        assert_eq!(first.remote_id(), actual.id);
        request(&first, 1).await;
        assert!(client_endpoint.remember_member_contact(&first, EndpointAddr::new(client_endpoint.endpoint().id())).is_err(), "another identity cannot be substituted for the authenticated member");
        client_endpoint.remember_member_contact(&first, actual.clone()).unwrap();
        let second = client_endpoint
            .connect(old.clone(), MEMBER_ALPN)
            .await
            .expect("known active address participates even when absent from caller target");
        assert_eq!(second.remote_id(), actual.id);
        request(&second, 2).await;
        first.close(0u32.into(), b"fixture reopen");
        second.close(0u32.into(), b"fixture reopen");
        client_endpoint.endpoint().close().await;
        drop(client_endpoint);
        client.close().await;
        let client = NativeNetwork::open(&client_path).await.unwrap();
        let client_endpoint = client
            .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap()
            .with_contact_timeout(Duration::from_secs(2))
            .unwrap();
        let restored = client_endpoint
            .connect(old.clone(), MEMBER_ALPN)
            .await
            .expect("saved member address participates after reopening");
        assert_eq!(restored.remote_id(), actual.id);
        request(&restored, 3).await;
        assert!(
            restored
                .paths()
                .iter()
                .any(|p| actual.addrs.contains(p.remote_addr()))
        );
        // Cancellation and an already expired caller deadline must win even
        // when the persistent cache contains a usable address.
        use miopunch_native_probe::connection_coordinator::{AttemptContext, SessionPurpose, ConnectError};
        let cancelled = tokio_util::sync::CancellationToken::new(); cancelled.cancel();
        assert!(matches!(client_endpoint.coordinator().connect(old.clone(), SessionPurpose::MemberBusiness,
            AttemptContext::new(n0_future::time::Instant::now()+Duration::from_secs(2),cancelled)).await, Err(ConnectError::Cancelled)));
        assert!(matches!(client_endpoint.coordinator().connect(old.clone(), SessionPurpose::MemberBusiness,
            AttemptContext::new(n0_future::time::Instant::now(),tokio_util::sync::CancellationToken::new())).await, Err(ConnectError::DeadlineExceeded)));
        restored.close(0u32.into(), b"fixture stale hint");
        client_endpoint.endpoint().close().await; drop(client_endpoint); client.close().await;
        let file = client_path.join("member.contacts.json");
        let mut saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        for peer in saved.as_array_mut().unwrap() {
            for route in peer["routes"].as_array_mut().unwrap() {
                route["observed_at"] = serde_json::json!(0);
                route["address"] = serde_json::json!({"Ip":"127.0.0.1:9"});
            }
        }
        std::fs::write(&file, serde_json::to_vec(&saved).unwrap()).unwrap();
        let mut client = NativeNetwork::open(&client_path).await.unwrap();
        let client_endpoint = client.bind_configured_endpoint(Ipv4Addr::LOCALHOST).await.unwrap().with_contact_timeout(Duration::from_secs(2)).unwrap();
        let cold = client_endpoint.connect(actual.clone(), MEMBER_ALPN).await.expect("expired wrong hint must be a cold fallback and leave the current address an opportunity");
        request(&cold, 4).await;
        let mut config = client.resource_configuration().unwrap(); config.udp_direct_disabled = true;
        client.save_resource_configuration(&config).unwrap();
        let disabled = client.bind_configured_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
        assert!(disabled.connect(actual.clone(), MEMBER_ALPN).await.is_err(), "cached IP cannot reenable disabled UDP");
        disabled.endpoint().close().await;
        config.udp_direct_disabled = false; client.save_resource_configuration(&config).unwrap();
        let mut management = server.begin_management_session();
        let removed = server.manage(&mut management, miopunch_native_probe::daily_management::ManagementAction::RemoveMember(client.status().node_id)).await.unwrap();
        let record = server.management_record(removed.record).await.unwrap().unwrap();
        client.ingest_management_record(record.header(), record.body()).await.unwrap();
        assert!(client_endpoint.remember_member_contact(&cold, actual.clone()).is_err(), "signed removal invalidates an existing connection's right to update hints");
        assert!(client_endpoint.connect(actual.clone(), MEMBER_ALPN).await.is_err(), "saved hints cannot resurrect removed membership");
        cold.close(0u32.into(), b"fixture removed");
        client_endpoint.endpoint().close().await; drop(client_endpoint);
        server_endpoint.endpoint().close().await;
        router.await.unwrap();
        client.close().await;
        server.close().await;
    })
    .await
    .expect("bounded member-redial fixture");
}

// Fixed-network actors create their identities on their respective hosts.
// Only a one-use invitation crosses hosts; private node state stays local.
fn configure_remote(node: &NativeNetwork, server: bool) {
    use miopunch_native_probe::resources::{Resource, ResourceUse};
    let mut config = node.resource_configuration().unwrap();
    let names = if server {
        vec!["aps1-1"]
    } else {
        vec!["aps1-1", "euc1-1"]
    };
    config.replacement = Some(
        names
            .into_iter()
            .map(|name| Resource {
                url: format!("https://{name}.relay.n0.iroh.link./"),
                uses: std::collections::BTreeSet::from([ResourceUse::DataRelay]),
                limitation: "MIOPUNCH-136 independent fixed-network acceptance".into(),
            })
            .collect(),
    );
    config.tcp_direct_disabled = true;
    node.save_resource_configuration(&config).unwrap();
}

#[derive(Clone, Debug)]
struct ContactEcho(EndpointAddr);
impl ProtocolHandler for ContactEcho {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        while let Ok((mut send, mut recv)) = connection.accept_bi().await {
            let Ok(bytes) = recv.read_to_end(8).await else {
                break;
            };
            let Ok(bytes) = <[u8; 8]>::try_from(bytes) else {
                break;
            };
            let reply = serde_json::json!({"id":self.0.id, "sequence":u64::from_be_bytes(bytes), "address":self.0});
            if send.write_all(reply.to_string().as_bytes()).await.is_err() {
                break;
            }
            if send.finish().is_err() {
                break;
            }
        }
        Ok(())
    }
}
#[tokio::test]
#[ignore = "explicit independently provisioned r_proxy server; stop via its own stop file"]
async fn remote_member_contacts_server() {
    use iroh::{Endpoint, endpoint::presets::Minimal};
    let root = std::path::PathBuf::from(std::env::var("MIO136_ROOT").unwrap());
    std::fs::create_dir_all(&root).unwrap();
    let node = if root.join("server").exists() {
        NativeNetwork::open(root.join("server")).await.unwrap()
    } else {
        NativeNetwork::create(root.join("server")).await.unwrap()
    };
    configure_remote(&node, true);
    let node = std::sync::Arc::new(tokio::sync::Mutex::new(node));
    use miopunch_native_probe::connection_coordinator::PRECODE_REQUEST_ALPN;
    node.lock().await.admission_gate().register_protocol(
        PRECODE_REQUEST_ALPN,
        miopunch_native_probe::admission_gate::ProtocolScope::AdmissionRequest,
    );
    let endpoint = node
        .lock()
        .await
        .bind_configured_endpoint_with(
            Endpoint::builder(Minimal)
                .bind_addr("0.0.0.0:47136")
                .unwrap(),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(60), endpoint.endpoint().online())
        .await
        .unwrap();
    let mut address = endpoint.addr();
    address.addrs = address.addrs.into_iter().take(8).collect();
    std::fs::write(
        root.join("address.json"),
        serde_json::to_vec(&address).unwrap(),
    )
    .unwrap();
    println!(
        "{}",
        serde_json::json!({"event":"mio136-server-ready","pid":std::process::id(),"address":address})
    );
    let code = node
        .lock()
        .await
        .issue_precode(miopunch_native_probe::service_policy::now() + 1800, Some(1))
        .await
        .unwrap();
    let invitation = miopunch_native_probe::client_invitation::ClientInvitation::new(
        code.invitation().encode(),
        address.clone(),
        Some(code.encode()),
        miopunch_native_probe::client_invitation::BootstrapContact {
            relays: vec!["https://aps1-1.relay.n0.iroh.link./".into()],
            ..Default::default()
        },
    )
    .unwrap();
    std::fs::write(
        root.join("invitation.json"),
        serde_json::to_vec(&invitation).unwrap(),
    )
    .unwrap();
    let handler = ContactEcho(address);
    let until = tokio::time::Instant::now() + Duration::from_secs(1800);
    let mut sessions = tokio::task::JoinSet::new();
    while tokio::time::Instant::now() < until && !root.join("stop").exists() {
        tokio::select! {
            incoming = endpoint.endpoint().accept() => {
                let Some(incoming) = incoming else {break};
                let handler = handler.clone();
                let node = node.clone();
                sessions.spawn(async move {
                    let Ok(mut accepting) = incoming.accept() else {return};
                    let Ok(alpn) = accepting.alpn().await else {return};
                    if alpn == PRECODE_REQUEST_ALPN {
                        let Ok(control) = accepting.into_control().await else {return};
                        let control = std::sync::Arc::new(control);
                        let result = node.lock().await.serve_precode_admission(&control).await;
                        println!("{}", serde_json::json!({"event":"mio136-precode-served","ok":result.is_ok()}));
                        // Like the normal demo bootstrap, keep the owner until
                        // the applicant receives DONE and closes its control.
                        // finish_send alone does not acknowledge delivery.
                        if result.is_ok() {
                            let remaining = control.caller_deadline().expect("bounded admission control")
                                .saturating_duration_since(n0_future::time::Instant::now());
                            let _ = tokio::time::timeout(remaining, control.weak_handle().closed()).await;
                        }
                    } else if let Ok(connection) = accepting.await {let _ = handler.accept(connection).await;}
                });
            }
            _ = tokio::time::sleep(Duration::from_millis(200)) => {}
            _ = sessions.join_next(), if !sessions.is_empty() => {}
        }
    }
    endpoint.endpoint().close().await;
    while sessions.join_next().await.is_some() {}
    std::sync::Arc::try_unwrap(node)
        .unwrap_or_else(|_| panic!("server still owned"))
        .into_inner()
        .close()
        .await;
}

#[tokio::test]
#[ignore = "explicit independently provisioned Windows client; modes learn, reopen, ip"]
async fn remote_member_contacts_client() {
    use miopunch_native_probe::resources::ResourceUse;
    let root = std::path::PathBuf::from(std::env::var("MIO136_ROOT").unwrap());
    let mode = std::env::var("MIO136_MODE").unwrap();
    std::fs::create_dir_all(&root).unwrap();
    let exists = root.join("client").exists();
    let admitted = if exists {
        let node = NativeNetwork::open(root.join("client")).await.unwrap();
        let status = node.status();
        let admitted = status.members.iter().any(|(id, _)| id == &status.node_id);
        node.close().await;
        admitted
    } else {
        false
    };
    if !admitted {
        let invitation = miopunch_native_probe::client_invitation::ClientInvitation::decode(
            &std::fs::read(root.join("invitation.json")).unwrap(),
        )
        .unwrap();
        let client = if exists {
            miopunch_native_probe::native_client::NativeClient::resume(
                root.join("client"),
                &invitation,
                Ipv4Addr::UNSPECIFIED,
            )
            .await
            .unwrap()
        } else {
            miopunch_native_probe::native_client::NativeClient::join_with_resources(
                root.join("client"),
                &invitation,
                Ipv4Addr::UNSPECIFIED,
                Default::default(),
            )
            .await
            .unwrap()
        };
        client.request_admission(&invitation).await.unwrap();
        client.close().await.unwrap();
    }
    let node = NativeNetwork::open(root.join("client")).await.unwrap();
    configure_remote(&node, false);
    let mut config = node.resource_configuration().unwrap();
    if mode == "ip" {
        config.disabled_uses.insert(ResourceUse::DataRelay);
    } else {
        config.disabled_uses.remove(&ResourceUse::DataRelay);
    }
    node.save_resource_configuration(&config).unwrap();
    let endpoint = node
        .bind_configured_endpoint(Ipv4Addr::UNSPECIFIED)
        .await
        .unwrap()
        .with_contact_timeout(if mode == "ip" {
            Duration::from_secs(12)
        } else {
            Duration::from_secs(60)
        })
        .unwrap();
    let actual: EndpointAddr =
        serde_json::from_slice(&std::fs::read(root.join("address.json")).unwrap()).unwrap();
    let target = if mode == "learn" {
        actual.clone()
    } else {
        let mut stale = EndpointAddr::new(actual.id)
            .with_addrs([TransportAddr::Ip("127.0.0.1:9".parse().unwrap())]);
        if mode != "ip" {
            stale.addrs.insert(TransportAddr::Relay(
                "https://euc1-1.relay.n0.iroh.link./".parse().unwrap(),
            ));
        }
        stale
    };
    println!(
        "{}",
        serde_json::json!({"event":"mio136-client-start","mode":mode,"pid":std::process::id(),"target":target,"actual":actual})
    );
    let started = std::time::Instant::now();
    let result = endpoint.connect(target, MEMBER_ALPN).await;
    match result {
        Ok(connection) => {
            assert_eq!(connection.remote_id(), actual.id);
            let sequence = match mode.as_str() {
                "learn" => 13601,
                "reopen" => 13602,
                _ => 13603,
            };
            let (mut send, mut recv) = connection.open_bi().await.unwrap();
            send.write_all(&u64::to_be_bytes(sequence)).await.unwrap();
            send.finish().unwrap();
            let reply = tokio::time::timeout(Duration::from_secs(8), recv.read_to_end(8192))
                .await
                .unwrap()
                .unwrap();
            let reply: serde_json::Value = serde_json::from_slice(&reply).unwrap();
            assert_eq!(reply["id"], actual.id.to_string());
            assert_eq!(reply["sequence"], sequence);
            let address: EndpointAddr = serde_json::from_value(reply["address"].clone()).unwrap();
            endpoint
                .remember_member_contact(&connection, address)
                .unwrap();
            println!(
                "{}",
                serde_json::json!({"event":"mio136-valid-reply","mode":mode,"pid":std::process::id(),"sequence":sequence,"peer":connection.remote_id(),"elapsedMs":started.elapsed().as_millis(),"paths":connection.paths().iter().map(|p|p.remote_addr().to_string()).collect::<Vec<_>>()})
            );
            connection.close(0u32.into(), b"MIOPUNCH-136 independent phase complete");
        }
        Err(error) if mode == "ip" => println!(
            "{}",
            serde_json::json!({"event":"mio136-ip-unavailable","pid":std::process::id(),"elapsedMs":started.elapsed().as_millis(),"error":error.to_string()})
        ),
        Err(error) => panic!("{mode} failed: {error}"),
    }
    endpoint.endpoint().close().await;
    node.close().await;
}

#[tokio::test]
async fn recent_unreachable_hint_leaves_time_for_current_target() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let root = tempfile::tempdir().unwrap();
        let client_path = root.path().join("client");
        let mut server = NativeNetwork::create(root.path().join("server")).await.unwrap();
        let client = join(&mut server, &client_path).await;
        configure(&client); configure(&server);
        let serving = server.bind_configured_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
        let endpoint = serving.endpoint().clone();
        let task = tokio::spawn(async move {
            while let Some(incoming) = endpoint.accept().await {
                tokio::spawn(async move {if let Ok(connection) = incoming.await {let _=Echo.accept(connection).await;}});
            }
        });
        let actual = serving.addr();
        let resource = client.bind_configured_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
        let first = resource.connect(actual.clone(), MEMBER_ALPN).await.unwrap();
        request(&first, 10).await;
        first.close(0u32.into(), b"fixture prior address went away");
        resource.endpoint().close().await; drop(resource); client.close().await;
        // A formerly working address can disappear inside the preference
        // window. Preserve its legitimate observation time; replace only the
        // destination to make that failure deterministic without waiting.
        let file = client_path.join("member.contacts.json");
        let mut hints:serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        for peer in hints.as_array_mut().unwrap() {for route in peer["routes"].as_array_mut().unwrap() {
            route["address"] = serde_json::json!({"Ip":"127.0.0.1:9"});
        }}
        std::fs::write(&file,serde_json::to_vec(&hints).unwrap()).unwrap();
        let client = NativeNetwork::open(&client_path).await.unwrap();
        let resource = client.bind_configured_endpoint(Ipv4Addr::LOCALHOST).await.unwrap().with_contact_timeout(Duration::from_secs(2)).unwrap();
        let current = resource.connect(actual, MEMBER_ALPN).await.expect("recent unreachable hint must leave caller's current candidate an opportunity inside its original deadline");
        request(&current,11).await;
        current.close(0u32.into(), b"fixture done");
        resource.endpoint().close().await; serving.endpoint().close().await; task.await.unwrap();
        client.close().await; server.close().await;
    }).await.unwrap();
}

#[tokio::test]
async fn current_announcements_survive_a_full_expired_verified_cache() {
    for reannounced in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let client_path = root.path().join("client");
        let mut server = NativeNetwork::create(root.path().join("server"))
            .await
            .unwrap();
        let client = join(&mut server, &client_path).await;
        configure(&client);
        configure(&server);
        let old_server = server
            .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap();
        let next_server = server
            .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap();
        let old_address = old_server.addr();
        let next_address = next_server.addr();
        let current_route = old_address
            .addrs
            .iter()
            .find(|a| matches!(a, TransportAddr::Ip(_)))
            .unwrap()
            .clone();
        let next_route = next_address
            .addrs
            .iter()
            .find(|a| matches!(a, TransportAddr::Ip(_)))
            .unwrap()
            .clone();
        let mut routes = vec![
            serde_json::json!({"address":current_route,"observed_at":miopunch_native_probe::service_policy::now(),"verified":true}),
        ];
        for index in 0..7 {
            let address = if reannounced && index == 0 {
                next_route.clone()
            } else {
                TransportAddr::Ip(format!("192.0.2.1:{}", 100 + index).parse().unwrap())
            };
            routes.push(serde_json::json!({"address":address,"observed_at":0,"verified":true}));
        }
        std::fs::write(
            client_path.join("member.contacts.json"),
            serde_json::to_vec(&serde_json::json!([{"id":old_address.id,"routes":routes}]))
                .unwrap(),
        )
        .unwrap();
        let resources = client
            .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap();
        let (connection, remote) = native_connections::member_pair(
            resources.endpoint(),
            old_server.endpoint(),
            MEMBER_ALPN,
        )
        .await;
        let mut announcement = next_address.clone();
        if reannounced {
            // Fill all seven announcement positions. A previously verified but
            // expired copy of next_route must become a current unverified hint,
            // or the other fresh announcements would push it out.
            announcement.addrs.extend(
                (0..6)
                    .map(|i| TransportAddr::Ip(format!("193.0.2.1:{}", 100 + i).parse().unwrap())),
            );
        }
        resources
            .remember_member_contact(&connection, announcement)
            .unwrap();
        connection.close(0u32.into(), b"fixture switching endpoint");
        remote.close(0u32.into(), b"fixture switching endpoint");
        resources.endpoint().close().await;
        drop(resources);
        client.close().await;
        old_server.endpoint().close().await;
        let endpoint = next_server.endpoint().clone();
        let accepting = tokio::spawn(async move {
            while let Some(incoming) = endpoint.accept().await {
                tokio::spawn(async move {
                    if let Ok(c) = incoming.await {
                        let _ = Echo.accept(c).await;
                    }
                });
            }
        });
        let client = NativeNetwork::open(&client_path).await.unwrap();
        let resources = client
            .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap()
            .with_contact_timeout(Duration::from_secs(6))
            .unwrap();
        let restored = resources.connect(old_address, MEMBER_ALPN).await.expect(
            "current authenticated announcement must survive expired verified-cache saturation",
        );
        assert_eq!(restored.remote_id(), next_address.id);
        request(&restored, if reannounced { 22 } else { 21 }).await;
        println!(
            "OBSERVED full-expired-cache reannounced={reannounced} actual-route={:?}",
            restored
                .paths()
                .iter()
                .map(|p| p.remote_addr().to_string())
                .collect::<Vec<_>>()
        );
        restored.close(0u32.into(), b"fixture done");
        resources.endpoint().close().await;
        next_server.endpoint().close().await;
        accepting.await.unwrap();
        client.close().await;
        server.close().await;
    }
}
