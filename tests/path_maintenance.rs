//! Public node/Connection/stream behavior for same-session path maintenance.
#[path = "support/delayed_relay.rs"]
mod delayed_relay;
use delayed_relay::delay_relay;

use iroh::{
    EndpointAddr, TransportAddr,
    endpoint::{Connection, ControlConnection, ControlMessageFormat},
};
use miopunch_native_probe::{
    admission_gate::{ADMISSION_REQUEST_ALPN, ProtocolScope},
    manual_admission::{AdmissionStage, ManualExchange},
    native_network::NativeNetwork,
    resources::{ResourceConfig, ResourceEndpoint},
};
use std::{net::Ipv4Addr, sync::Arc, time::Duration};
const REPAIR_MESSAGE: ControlMessageFormat = ControlMessageFormat::EndTerminated { maximum: 4096 };
const BUSINESS: &[u8] = b"miopunch/path-maintenance/1";

#[tokio::test]
async fn repair_uses_the_ninth_candidate_on_the_original_stream() {
    use miopunch_native_probe::connection_recovery::CONNECTION_RECOVERY_ALPN;
    let mut pair = Pair::new(true).await;
    let (client, server) = pair.business().await;
    let ids = (client.stable_id(), server.stable_id());
    let (mut send, mut recv) = client.open_bi().await.unwrap();
    send.write_all(b"before").await.unwrap();
    let (mut reply, mut request) = server.accept_bi().await.unwrap();
    request.read_exact(&mut [0; 6]).await.unwrap();
    let good = TransportAddr::Ip(pair.server.endpoint().bound_sockets()[0]);
    let mut offer = EndpointAddr::new(pair.server.endpoint().id());
    for port in 1..=8 {
        offer
            .addrs
            .insert(TransportAddr::Ip(([127, 0, 0, 1], port).into()));
    }
    offer.addrs.insert(good.clone());
    assert_eq!(
        offer.addrs.iter().position(|address| address == &good),
        Some(8)
    );
    pair.server
        .endpoint()
        .set_alpns(vec![BUSINESS.to_vec(), CONNECTION_RECOVERY_ALPN.to_vec()]);
    let serve = async {
        let control = pair
            .server
            .endpoint()
            .accept()
            .await
            .unwrap()
            .accept_control()
            .unwrap()
            .await
            .unwrap();
        let exchange = control.accept_control_exchange().await.unwrap();
        exchange.recv_message(REPAIR_MESSAGE).await.unwrap();
        exchange
            .send_message(REPAIR_MESSAGE, &serde_json::to_vec(&offer).unwrap())
            .await
            .unwrap();
        control.closed().await;
    };
    let (recovered, ()) = tokio::join!(
        pair.joining.recover_connection_path_until(
            &pair.client,
            &client,
            tcp_target(&pair.server),
            n0_future::time::Instant::now() + Duration::from_secs(2)
        ),
        serve
    );
    let recovered =
        recovered.expect("the ninth offered address must repair the existing member connection");
    assert_eq!(recovered.path, good);
    let path_id = client
        .paths()
        .iter()
        .find(|path| path.remote_addr() == &good)
        .unwrap()
        .id();
    let before_frames = [&client, &server].map(|original| {
        original
            .paths()
            .get(path_id)
            .unwrap()
            .stats()
            .frame_rx
            .stream
    });
    for original in [&client, &server] {
        for path in original.paths().iter().filter(|path| path.id() != path_id) {
            path.close().unwrap();
        }
    }
    send.write_all(b"after").await.unwrap();
    let mut bytes = [0; 5];
    request.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"after");
    reply.write_all(b"there").await.unwrap();
    recv.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"there");
    for (original, before) in [&client, &server].into_iter().zip(before_frames) {
        assert!(
            original
                .paths()
                .get(path_id)
                .unwrap()
                .stats()
                .frame_rx
                .stream
                > before,
            "both old stream directions must really use the ninth candidate after other paths retire"
        );
    }
    assert_eq!((client.stable_id(), server.stable_id()), ids);
    pair.close().await;
}

#[path = "support/relay_contact_broker.rs"]
#[allow(dead_code)]
mod batch_broker;

#[tokio::test]
async fn slow_signaling_repairs_from_a_second_batch_after_three_seconds() {
    delayed_second_batch(BatchTiming::Slow).await;
}

#[tokio::test]
async fn fast_contact_yields_to_slow_signaling_without_restarting_the_repair_clock() {
    delayed_second_batch(BatchTiming::FastThenSlow).await;
}

#[tokio::test]
async fn repair_validates_a_usable_first_batch_before_the_delayed_second_batch() {
    delayed_second_batch(BatchTiming::UsableFirst).await;
}

enum BatchTiming {
    Slow,
    FastThenSlow,
    UsableFirst,
}

async fn delayed_second_batch(timing: BatchTiming) {
    let fast_first = matches!(timing, BatchTiming::FastThenSlow);
    let usable_first = matches!(timing, BatchTiming::UsableFirst);
    use miopunch_native_probe::{
        connection_recovery::CONNECTION_RECOVERY_ALPN,
        mqtt_transport::TRANSPORT_ID,
        resources::{Resource, ResourceUse},
    };
    let broker = batch_broker::Broker::new().await;
    let mut pair = Pair::new(true).await;
    pair.reconfigure(vec![Resource {
        url: format!("mqtt://{}", broker.address),
        uses: [ResourceUse::Signaling].into(),
        limitation: "owned delayed candidate signaling".into(),
    }])
    .await;
    let (client, server) = pair.business().await;
    let ids = (client.stable_id(), server.stable_id());
    let (mut send, mut recv) = client.open_bi().await.unwrap();
    send.write_all(b"before").await.unwrap();
    let (mut reply, mut request) = server.accept_bi().await.unwrap();
    request.read_exact(&mut [0; 6]).await.unwrap();
    pair.server
        .endpoint()
        .set_alpns(vec![BUSINESS.to_vec(), CONNECTION_RECOVERY_ALPN.to_vec()]);
    let route = pair
        .server
        .endpoint()
        .custom_addrs()
        .into_iter()
        .find(|route| route.id() == TRANSPORT_ID)
        .unwrap();
    let mut contact =
        EndpointAddr::new(pair.server.endpoint().id()).with_addrs([TransportAddr::Custom(route)]);
    if fast_first {
        contact.addrs.extend(tcp_target(&pair.server).addrs);
    }
    let good = TransportAddr::Ip(pair.server.endpoint().bound_sockets()[0]);
    let started = tokio::time::Instant::now();
    let frame = ControlMessageFormat::LengthPrefixed16 { maximum: 4096 };
    let serve = async {
        if fast_first {
            let silent = pair
                .server
                .endpoint()
                .accept()
                .await
                .unwrap()
                .accept_control()
                .unwrap()
                .await
                .unwrap();
            let header = silent.accept_control_exchange().await.unwrap();
            header.recv_message(REPAIR_MESSAGE).await.unwrap();
            tokio::time::timeout(Duration::from_millis(3400), silent.closed())
                .await
                .expect("fast contact must end within three seconds");
            assert!(started.elapsed() >= Duration::from_millis(2800));
        }
        let control = pair
            .server
            .endpoint()
            .accept()
            .await
            .unwrap()
            .accept_control()
            .unwrap()
            .await
            .unwrap();
        let slow_started = tokio::time::Instant::now();
        let header = control.accept_control_exchange().await.unwrap();
        let wire: serde_json::Value =
            serde_json::from_slice(&header.recv_message(REPAIR_MESSAGE).await.unwrap()).unwrap();
        assert_eq!(wire["batched"], true);
        assert!(wire["budget_ms"].as_u64().unwrap() > 3000);
        header
            .send_message(
                REPAIR_MESSAGE,
                &serde_json::to_vec(&serde_json::json!({
                    "id":pair.server.endpoint().id(),"addrs":[],"batched":true
                }))
                .unwrap(),
            )
            .await
            .unwrap();
        let frames = control.accept_control_exchange().await.unwrap();
        let first = if usable_first {
            EndpointAddr::new(pair.server.endpoint().id()).with_addrs([good.clone()])
        } else {
            EndpointAddr::new(pair.server.endpoint().id())
                .with_addrs((1..=8).map(|port| TransportAddr::Ip(([127, 0, 0, 1], port).into())))
        };
        frames
            .send_message(
                frame,
                &serde_json::to_vec(&serde_json::json!({"Candidates":first})).unwrap(),
            )
            .await
            .unwrap();
        let receive_ready = async {
            loop {
                let value: serde_json::Value =
                    serde_json::from_slice(&frames.recv_message(frame).await.unwrap()).unwrap();
                if value.get("Ready").is_some() {
                    break;
                }
            }
        };
        tokio::pin!(receive_ready);
        if usable_first {
            tokio::time::timeout(Duration::from_secs(1), &mut receive_ready)
                .await
                .expect("first usable batch validates before the second batch is sent");
        }
        tokio::time::sleep_until(slow_started + Duration::from_millis(3300)).await;
        assert!(client.close_reason().is_none() && server.close_reason().is_none());
        let second = EndpointAddr::new(pair.server.endpoint().id()).with_addrs([good.clone()]);
        frames
            .send_message(
                frame,
                &serde_json::to_vec(&serde_json::json!({"Candidates":second})).unwrap(),
            )
            .await
            .unwrap();
        if !usable_first {
            receive_ready.await;
        }
        let remote = TransportAddr::Ip(pair.client.endpoint().bound_sockets()[0]);
        let probe = server
            .paths()
            .iter()
            .find(|path| path.remote_addr() == &remote)
            .unwrap()
            .start_return_path_probe()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while !probe.is_acknowledged().unwrap() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        frames
            .send_message(
                frame,
                &serde_json::to_vec(&serde_json::json!({"Ready":{
                    "path":remote,"fresh_ack":true,"probe_watermark":probe.watermark()
                }}))
                .unwrap(),
            )
            .await
            .unwrap();
        let committed: serde_json::Value =
            serde_json::from_slice(&frames.recv_message(frame).await.unwrap()).unwrap();
        assert_eq!(committed, "Committed");
        frames.send_message(frame, b"\"Committed\"").await.unwrap();
        frames.finish_send_confirmed().await.unwrap();
        control.closed().await;
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(8), async {
        tokio::join!(
            pair.joining.recover_connection_path_confirmed_until(
                &pair.client,
                &client,
                contact,
                n0_future::time::Instant::now() + Duration::from_secs(7)
            ),
            serve
        )
    })
    .await
    .unwrap();
    let result = result.unwrap();
    let minimum = if fast_first { 6100 } else { 3300 };
    assert!(started.elapsed() >= Duration::from_millis(minimum));
    assert!(
        started.elapsed() < Duration::from_secs(7),
        "all contacts and batches share the original seven-second clock"
    );
    assert_eq!(result.path, good);
    assert_eq!(result.stable_id, ids.0);
    eprintln!(
        "MIO204_BATCH_RECOVERY fast_first={fast_first} elapsed_ms={} stable_ids={ids:?} path={good:?}",
        started.elapsed().as_millis()
    );
    send.write_all(b"after").await.unwrap();
    let mut bytes = [0; 5];
    request.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"after");
    reply.write_all(b"there").await.unwrap();
    recv.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"there");
    assert_eq!((client.stable_id(), server.stable_id()), ids);
    pair.close().await;
    broker.close().await;
}

#[tokio::test]
async fn candidate_specific_refusal_does_not_block_another_permitted_original_path() {
    use miopunch_native_probe::connection_recovery::CONNECTION_RECOVERY_ALPN;
    let mut pair = Pair::new(true).await;
    let stricter = pair
        .joining
        .bind_configured_endpoint_with(
            iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .bind_addr((Ipv4Addr::LOCALHOST, 0))
                .unwrap()
                .clear_relay_transports()
                .restricted_data_custom_transports(vec![]),
        )
        .await
        .unwrap();
    std::mem::replace(&mut pair.client, stricter).close().await;
    let good = TransportAddr::Ip(pair.server.endpoint().bound_sockets()[0]);
    let target = EndpointAddr::new(pair.server.endpoint().id()).with_addrs([good.clone()]);
    let (client, server) = connect(&mut pair.client, &pair.server, target, BUSINESS).await;
    let ids = (client.stable_id(), server.stable_id());
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"before").await.unwrap();
    let (_, mut recv) = server.accept_bi().await.unwrap();
    recv.read_exact(&mut [0; 6]).await.unwrap();
    assert!(
        pair.joining
            .admission_gate()
            .allows(&client, ProtocolScope::Member)
    );
    let refused = tcp_target(&pair.server).addrs.into_iter().next().unwrap();
    assert!(
        !pair
            .joining
            .resource_configuration()
            .unwrap()
            .tcp_direct_disabled
    );
    let refusal = pair
        .client
        .endpoint()
        .open_data_transport_path(&client, refused.clone())
        .await
        .unwrap_err();
    assert!(
        refusal.to_string().contains("forbidden"),
        "actual per-M refusal: {refusal}"
    );
    let offer =
        EndpointAddr::new(pair.server.endpoint().id()).with_addrs([good.clone(), refused.clone()]);
    pair.server
        .endpoint()
        .set_alpns(vec![BUSINESS.to_vec(), CONNECTION_RECOVERY_ALPN.to_vec()]);
    let serve = async {
        let control = pair
            .server
            .endpoint()
            .accept()
            .await
            .unwrap()
            .accept_control()
            .unwrap()
            .await
            .unwrap();
        assert!(
            pair.admin
                .admission_gate()
                .allows_member_path_upgrade(&control)
        );
        let exchange = control.accept_control_exchange().await.unwrap();
        let _: serde_json::Value =
            serde_json::from_slice(&exchange.recv_message(REPAIR_MESSAGE).await.unwrap()).unwrap();
        exchange
            .send_message(REPAIR_MESSAGE, &serde_json::to_vec(&offer).unwrap())
            .await
            .unwrap();
        control.closed().await;
    };
    let (recovered, _) = tokio::join!(
        pair.joining.recover_connection_path_until(
            &pair.client,
            &client,
            tcp_target(&pair.server),
            n0_future::time::Instant::now() + Duration::from_secs(2)
        ),
        serve
    );
    let recovered =
        recovered.expect("one refused candidate must not abort the permitted UDP probe");
    assert_eq!(recovered.path, good);
    assert_eq!(recovered.stable_id, ids.0);
    assert!(client.paths().iter().any(|p| p.is_ip()));
    assert!(client.paths().iter().all(|p| p.remote_addr() != &refused));
    send.write_all(b"after").await.unwrap();
    let mut after = [0; 5];
    recv.read_exact(&mut after).await.unwrap();
    assert_eq!(&after, b"after");
    assert_eq!((client.stable_id(), server.stable_id()), ids);
    client.close(0u32.into(), b"fixture ended");
    server.close(0u32.into(), b"fixture ended");
    pair.close().await;
}

#[tokio::test]
async fn old_literal_repair_request_is_accepted_on_protected_original_member() {
    use miopunch_native_probe::connection_recovery::CONNECTION_RECOVERY_ALPN;
    let mut pair = Pair::new(true).await;
    let sockets: Vec<_> = (0..9)
        .map(|_| std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap())
        .collect();
    let addresses: Vec<_> = sockets
        .iter()
        .map(|socket| socket.local_addr().unwrap())
        .collect();
    let mut builder =
        iroh::Endpoint::builder(iroh::endpoint::presets::Minimal).clear_ip_transports();
    for (index, address) in addresses.iter().enumerate() {
        builder = builder
            .bind_addr_with_opts(
                *address,
                iroh::endpoint::BindOpts::default()
                    .set_is_default_route(index + 1 == addresses.len()),
            )
            .unwrap();
    }
    drop(sockets);
    let replacement = pair
        .admin
        .bind_configured_endpoint_with(builder)
        .await
        .unwrap();
    replacement.endpoint().set_alpns(vec![BUSINESS.to_vec()]);
    std::mem::replace(&mut pair.server, replacement)
        .close()
        .await;
    assert!(
        pair.server.addr().addrs.len() > 8,
        "legacy peer must face a current set larger than eight"
    );
    let (client, server) = pair.business().await;
    pair.server
        .endpoint()
        .set_alpns(vec![BUSINESS.to_vec(), CONNECTION_RECOVERY_ALPN.to_vec()]);
    let (mut original_send, _) = client.open_bi().await.unwrap();
    original_send.write_all(b"before").await.unwrap();
    let (_, mut original_recv) = server.accept_bi().await.unwrap();
    original_recv.read_exact(&mut [0; 6]).await.unwrap();
    let mut binding = [0u8; 32];
    client
        .export_keying_material(
            &mut binding,
            b"miopunch-existing-connection-path-recovery/1",
            &[],
        )
        .unwrap();
    // This literal has exactly the pre-extension wire fields. The protected
    // server must bind it to the actual original M, not merely decode JSON.
    let bytes =
        serde_json::to_vec(&serde_json::json!({"binding":binding,"offer":pair.client.addr()}))
            .unwrap();
    let (control, incoming) = tokio::join!(
        pair.client
            .endpoint()
            .connect_control(tcp_target(&pair.server), CONNECTION_RECOVERY_ALPN),
        async {
            pair.server
                .endpoint()
                .accept()
                .await
                .unwrap()
                .accept_control()
                .unwrap()
                .await
                .unwrap()
        }
    );
    let control = control.unwrap();
    let request = async {
        let exchange = control.open_control_exchange().await.unwrap();
        exchange.send_message(REPAIR_MESSAGE, &bytes).await.unwrap();
        let offer: EndpointAddr =
            serde_json::from_slice(&exchange.recv_message(REPAIR_MESSAGE).await.unwrap()).unwrap();
        assert_eq!(offer.id, pair.server.endpoint().id());
        assert!(
            offer.addrs.len() <= 8,
            "an old requester retains its historical bounded single offer"
        );
        assert!(
            offer
                .addrs
                .iter()
                .any(|address| matches!(address,TransportAddr::Ip(ip) if addresses.contains(ip)))
        );
        control.close(0u32.into(), b"legacy request completed");
    };
    let (_, served) = tokio::join!(
        request,
        pair.admin
            .serve_connection_path_recovery(&pair.server, &incoming)
    );
    served.unwrap();
    incoming.close(0u32.into(), b"test ended");
    original_send.write_all(b"after").await.unwrap();
    let mut after = [0; 5];
    original_recv.read_exact(&mut after).await.unwrap();
    assert_eq!(&after, b"after");
    client.close(0u32.into(), b"test ended");
    server.close(0u32.into(), b"test ended");
    pair.close().await;
}

#[tokio::test]
async fn default_repair_api_preserves_optional_confirmation_on_actual_protected_wire() {
    use miopunch_native_probe::{
        connection_coordinator::ACTIVE_IMPROVEMENT_ALPN,
        connection_recovery::CONNECTION_RECOVERY_ALPN,
    };
    let mut pair = Pair::new(true).await;
    let (client, server) = pair.business().await;
    let Pair {
        _dir,
        admin,
        joining,
        client: client_endpoint,
        server: server_endpoint,
    } = pair;
    let client_endpoint = Arc::new(client_endpoint);
    client_endpoint
        .endpoint()
        .set_alpns(vec![ACTIVE_IMPROVEMENT_ALPN.to_vec()]);
    server_endpoint
        .endpoint()
        .set_alpns(vec![BUSINESS.to_vec(), CONNECTION_RECOVERY_ALPN.to_vec()]);
    let target = tcp_target(&server_endpoint);
    let port = server_endpoint.endpoint().bound_sockets()[0].port();
    let mut full_contact = target.clone();
    for address in 2..10 {
        full_contact
            .addrs
            .insert(TransportAddr::Ip(([127, 0, 0, address], port).into()));
    }
    assert!(full_contact.addrs.len() > 8);
    let deadline = n0_future::time::Instant::now() + Duration::from_secs(3);
    let inspect = async {
        let control = server_endpoint
            .endpoint()
            .accept()
            .await
            .unwrap()
            .accept_control()
            .unwrap()
            .await
            .unwrap();
        assert!(admin.admission_gate().allows_member_path_upgrade(&control));
        let exchange = control.accept_control_exchange().await.unwrap();
        let wire: serde_json::Value =
            serde_json::from_slice(&exchange.recv_message(REPAIR_MESSAGE).await.unwrap()).unwrap();
        assert_eq!(wire.as_object().unwrap().len(), 4);
        assert_eq!(wire["batched"], true);
        assert!(wire["budget_ms"].as_u64().is_some());
        assert!(wire.get("binding").is_some() && wire.get("offer").is_some());
        assert!(wire.get("require_confirmation").is_none());
        let binding: [u8; 32] = serde_json::from_value(wire["binding"].clone()).unwrap();
        let paths: Vec<_> = client.paths().iter().map(|path| path.id()).collect();
        // The public legacy requester has sent on C and holds the original
        // lease while it waits for this offer. The reverse active C is refused.
        assert_active_control_deferred(
            &joining,
            &server_endpoint,
            &client_endpoint,
            &binding,
            deadline,
        )
        .await;
        assert_eq!(
            client
                .paths()
                .iter()
                .map(|path| path.id())
                .collect::<Vec<_>>(),
            paths
        );
        exchange
            .send_message(
                REPAIR_MESSAGE,
                &serde_json::to_vec(&server_endpoint.addr()).unwrap(),
            )
            .await
            .unwrap();
        control.closed().await;
        let next = server_endpoint
            .endpoint()
            .accept()
            .await
            .unwrap()
            .accept_control()
            .unwrap()
            .await
            .unwrap();
        admin
            .serve_connection_path_recovery(&server_endpoint, &next)
            .await
            .unwrap();
    };
    let (result, confirmed, ()) = tokio::time::timeout_at(deadline, async {
        tokio::join!(
            joining.recover_connection_path(&client_endpoint, &client, full_contact),
            joining.recover_connection_path_confirmed_until(
                &client_endpoint,
                &client,
                target.clone(),
                deadline
            ),
            inspect
        )
    })
    .await
    .unwrap();
    let result = result.unwrap();
    let confirmed = confirmed.unwrap();
    assert!(!result.peer_confirmed);
    assert_eq!(result.stable_id, client.stable_id());
    assert!(confirmed.peer_confirmed);
    assert_eq!(confirmed.stable_id, result.stable_id);
    assert_ne!(confirmed.control_stable_id, result.control_stable_id);
    assert!(
        client
            .paths()
            .iter()
            .any(|p| p.id().to_string() == confirmed.path_id && p.remote_addr() == &confirmed.path)
    );
    client.close(0u32.into(), b"test ended");
    server.close(0u32.into(), b"test ended");
    Arc::into_inner(client_endpoint).unwrap().close().await;
    server_endpoint.close().await;
    tokio::join!(admin.close(), joining.close());
}

async fn assert_active_control_deferred(
    node: &NativeNetwork,
    requester: &ResourceEndpoint,
    responder: &Arc<ResourceEndpoint>,
    binding: &[u8; 32],
    deadline: n0_future::time::Instant,
) {
    use iroh::protocol::ControlProtocolHandler;
    use miopunch_native_probe::connection_coordinator::ACTIVE_IMPROVEMENT_ALPN;

    let handler = node.active_improvement_handler(responder.clone());
    let initiate = async {
        let control = requester
            .endpoint()
            .connect_control(tcp_target(responder), ACTIVE_IMPROVEMENT_ALPN)
            .await
            .unwrap();
        let active = control.open_control_exchange().await.unwrap();
        let mut hello = [0u8; 59];
        hello[..11].copy_from_slice(b"MIA1\x01\x01\x01\x01\x01\x01\x00");
        hello[11..27].copy_from_slice(&rand::random::<[u8; 16]>());
        hello[27..].copy_from_slice(binding);
        active
            .send_message(
                ControlMessageFormat::LengthPrefixed16 { maximum: 59 },
                &hello,
            )
            .await
            .unwrap();
        active.finish_send().await.unwrap();
        control.closed().await
    };
    let accept = async {
        let active = responder
            .endpoint()
            .accept()
            .await
            .unwrap()
            .with_caller_deadline(deadline)
            .accept_control()
            .unwrap()
            .await
            .unwrap();
        handler.accept(active).await.unwrap();
    };
    let (reason, ()) = tokio::join!(initiate, accept);
    assert!(
        matches!(reason, iroh::endpoint::ConnectionError::ApplicationClosed(close) if close.error_code == 0x4d49_4f01u32.into())
    );
}

#[tokio::test]
async fn legacy_responder_defers_active_control_on_either_original_side() {
    use miopunch_native_probe::{
        connection_coordinator::ACTIVE_IMPROVEMENT_ALPN,
        connection_recovery::CONNECTION_RECOVERY_ALPN,
    };
    let mut pair = Pair::new(true).await;
    let (client, server) = pair.business().await;
    let Pair {
        _dir,
        admin,
        joining,
        client: client_endpoint,
        server: server_endpoint,
    } = pair;
    let (client_endpoint, server_endpoint) = (Arc::new(client_endpoint), Arc::new(server_endpoint));
    for peer in [&client_endpoint, &server_endpoint] {
        peer.endpoint().set_alpns(vec![
            CONNECTION_RECOVERY_ALPN.to_vec(),
            ACTIVE_IMPROVEMENT_ALPN.to_vec(),
        ]);
    }
    for (requester, responder, original, remote) in [
        (&client_endpoint, &server_endpoint, &client, &server),
        (&server_endpoint, &client_endpoint, &server, &client),
    ] {
        let node = if remote.side().is_client() {
            &joining
        } else {
            &admin
        };
        let (mut send, mut recv) = original.open_bi().await.unwrap();
        send.write_all(b"before").await.unwrap();
        let (mut reply, mut request) = remote.accept_bi().await.unwrap();
        request.read_exact(&mut [0; 6]).await.unwrap();
        let ids = (original.stable_id(), remote.stable_id());
        let deadline = n0_future::time::Instant::now() + Duration::from_secs(2);
        let (out, incoming) = connect_control(
            requester,
            responder,
            tcp_target(responder),
            CONNECTION_RECOVERY_ALPN,
        )
        .await;
        let mutate = async {
            let mut binding = [0; 32];
            original
                .export_keying_material(
                    &mut binding,
                    b"miopunch-existing-connection-path-recovery/1",
                    &[],
                )
                .unwrap();
            let exchange = out.open_control_exchange().await.unwrap();
            let request = serde_json::json!({"binding":binding,"offer":requester.addr()});
            exchange
                .send_message(REPAIR_MESSAGE, &serde_json::to_vec(&request).unwrap())
                .await
                .unwrap();
            let _: EndpointAddr =
                serde_json::from_slice(&exchange.recv_message(REPAIR_MESSAGE).await.unwrap())
                    .unwrap();
            // A complete legacy offer leaves the sole owner waiting for its
            // requester's C to finish. An active C must use its deferral code.
            let paths: Vec<_> = remote.paths().iter().map(|path| path.id()).collect();
            assert_active_control_deferred(node, requester, responder, &binding, deadline).await;
            assert_eq!(
                remote
                    .paths()
                    .iter()
                    .map(|path| path.id())
                    .collect::<Vec<_>>(),
                paths
            );
            out.close(0u32.into(), b"legacy offer received");
        };
        let ((), served) = tokio::time::timeout_at(deadline, async {
            tokio::join!(
                mutate,
                node.serve_connection_path_recovery(responder, &incoming)
            )
        })
        .await
        .unwrap();
        served.unwrap();
        send.write_all(b"after").await.unwrap();
        let mut bytes = [0; 5];
        request.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"after");
        reply.write_all(&bytes).await.unwrap();
        recv.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"after");
        assert_eq!((original.stable_id(), remote.stable_id()), ids);
    }
    client.close(0u32.into(), b"legacy contention fixture ended");
    server.close(0u32.into(), b"legacy contention fixture ended");
    Arc::into_inner(client_endpoint).unwrap().close().await;
    Arc::into_inner(server_endpoint).unwrap().close().await;
    tokio::join!(admin.close(), joining.close());
}

#[tokio::test]
async fn protected_repair_confirms_each_ends_distinct_receiving_address_on_original_stream() {
    use miopunch_native_probe::connection_recovery::CONNECTION_RECOVERY_ALPN;
    let mut pair = Pair::new(true).await;
    let (client, server) = pair.business().await;
    pair.server
        .endpoint()
        .set_alpns(vec![BUSINESS.to_vec(), CONNECTION_RECOVERY_ALPN.to_vec()]);
    let id = client.stable_id();
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"before").await.unwrap();
    let (_, mut recv) = server.accept_bi().await.unwrap();
    recv.read_exact(&mut [0; 6]).await.unwrap();
    let requester_receive = pair.client.addr();
    let responder_receive = pair.server.addr();
    assert!(
        requester_receive
            .addrs
            .iter()
            .all(|address| !responder_receive.addrs.contains(address))
    );
    let serve = async {
        let control = pair
            .server
            .endpoint()
            .accept()
            .await
            .unwrap()
            .accept_control()
            .unwrap()
            .await
            .unwrap();
        let result = pair
            .admin
            .serve_connection_path_recovery(&pair.server, &control)
            .await;
        control.close(0u32.into(), b"serving ended");
        result
    };
    let (result, served) = tokio::join!(
        pair.joining.recover_connection_path_confirmed_until(
            &pair.client,
            &client,
            tcp_target(&pair.server),
            n0_future::time::Instant::now() + Duration::from_secs(2)
        ),
        serve
    );
    assert!(
        result.is_ok(),
        "each endpoint validates its own remote receiving address, not the other endpoint's remote address: {result:?}"
    );
    let result = result.unwrap();
    assert_eq!(result.stable_id, id);
    assert!(result.peer_confirmed);
    assert!(served.is_ok());
    send.write_all(b"after").await.unwrap();
    let mut bytes = [0; 5];
    recv.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"after");
    client.close(0u32.into(), b"test ended");
    server.close(0u32.into(), b"test ended");
    pair.close().await;
}

#[tokio::test]
async fn repair_caller_deadline_does_not_wait_for_the_default_or_close_original_stream() {
    use miopunch_native_probe::connection_recovery::{
        CONNECTION_RECOVERY_ALPN, ConnectionPathRecoveryError,
    };
    let mut pair = Pair::new(false).await;
    let (client, server) = pair.business().await;
    pair.server
        .endpoint()
        .set_alpns(vec![BUSINESS.to_vec(), CONNECTION_RECOVERY_ALPN.to_vec()]);
    let (mut send, mut recv) = client.open_bi().await.unwrap();
    send.write_all(b"before").await.unwrap();
    let (mut reply, mut incoming) = server.accept_bi().await.unwrap();
    incoming.read_exact(&mut [0; 6]).await.unwrap();
    let original_id = client.stable_id();
    let start = std::time::Instant::now();
    // No receiver accepts this new protected repair contact. Its wait must
    // consume this caller's clock, independently of the already admitted M.
    let result = pair
        .joining
        .recover_connection_path_until(
            &pair.client,
            &client,
            tcp_target(&pair.server),
            n0_future::time::Instant::now() + Duration::from_millis(120),
        )
        .await;
    assert!(
        matches!(result, Err(ConnectionPathRecoveryError::Retryable { stable_id, .. }) if stable_id == original_id)
    );
    assert!(start.elapsed() < Duration::from_millis(500));
    assert!(client.close_reason().is_none());
    send.write_all(b"after").await.unwrap();
    let mut after = [0; 5];
    incoming.read_exact(&mut after).await.unwrap();
    assert_eq!(&after, b"after");
    reply.write_all(b"same stream").await.unwrap();
    let mut response = [0; 11];
    recv.read_exact(&mut response).await.unwrap();
    assert_eq!(&response, b"same stream");
    client.close(0u32.into(), b"test finished");
    server.close(0u32.into(), b"test finished");
    pair.close().await;
}

struct Pair {
    _dir: tempfile::TempDir,
    admin: NativeNetwork,
    joining: NativeNetwork,
    client: ResourceEndpoint,
    server: ResourceEndpoint,
}
impl Pair {
    async fn new(udp: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut admin = NativeNetwork::create(dir.path().join("admin"))
            .await
            .unwrap();
        let mut joining = NativeNetwork::join(
            dir.path().join("joining"),
            admin.manual_invitation().unwrap(),
        )
        .await
        .unwrap();
        for node in [&admin, &joining] {
            let mut config = ResourceConfig::default();
            config.replacement = Some(vec![]);
            config.tcp_listen = Some("127.0.0.1:0".parse().unwrap());
            config.udp_direct_disabled = !udp;
            node.save_resource_configuration(&config).unwrap();
            node.admission_gate()
                .register_protocol(BUSINESS, ProtocolScope::Member);
        }
        let server = admin
            .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap();
        server
            .endpoint()
            .set_alpns(vec![ADMISSION_REQUEST_ALPN.to_vec(), BUSINESS.to_vec()]);
        let client = joining
            .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap();
        let (out, incoming) = connect_control(
            &client,
            &server,
            tcp_target(&server),
            ADMISSION_REQUEST_ALPN,
        )
        .await;
        let (request, served) = tokio::join!(
            joining.request_manual_admission(&out),
            admin.serve_manual_admission(&incoming)
        );
        assert!(matches!(request.unwrap(), ManualExchange::AwaitingApproval));
        assert_eq!(served.unwrap(), AdmissionStage::AwaitingApproval);
        let request = admin.manual_requests().await.unwrap().pop().unwrap();
        admin.approve_manual_admission(&request).await.unwrap();
        let receive = async {
            let ManualExchange::Materials(materials) =
                joining.request_manual_admission(&out).await.unwrap()
            else {
                panic!("approved materials")
            };
            joining.complete_manual_admission(materials).await.unwrap()
        };
        let (saved, served) = tokio::join!(receive, admin.serve_manual_admission(&incoming));
        assert_eq!(saved, AdmissionStage::Complete);
        assert_eq!(served.unwrap(), AdmissionStage::Complete);
        out.close(0u32.into(), b"admitted");
        incoming.close(0u32.into(), b"admitted");
        // Rebind the admitted nodes so the initial contact is not replaced by
        // a cached route from the admission fixture.
        client.close().await;
        server.close().await;
        let server = admin
            .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap();
        server.endpoint().set_alpns(vec![BUSINESS.to_vec()]);
        let client = joining
            .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap();
        Self {
            _dir: dir,
            admin,
            joining,
            client,
            server,
        }
    }
    async fn reconfigure(&mut self, resources: Vec<miopunch_native_probe::resources::Resource>) {
        for node in [&self.admin, &self.joining] {
            let mut config = node.resource_configuration().unwrap();
            config.replacement = Some(resources.clone());
            node.save_resource_configuration(&config).unwrap();
        }
        // Only the configured endpoints restart; durable admission is retained.
        let new_server = self
            .admin
            .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap();
        new_server
            .endpoint()
            .set_alpns(vec![ADMISSION_REQUEST_ALPN.to_vec(), BUSINESS.to_vec()]);
        let new_client = self
            .joining
            .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap();
        std::mem::replace(&mut self.client, new_client)
            .close()
            .await;
        std::mem::replace(&mut self.server, new_server)
            .close()
            .await;
    }
    async fn business(&mut self) -> (Connection, Connection) {
        let target = tcp_target(&self.server);
        connect(&mut self.client, &self.server, target, BUSINESS).await
    }
    async fn close(self) {
        self.client.close().await;
        self.server.close().await;
        self.joining.close().await;
        self.admin.close().await;
    }
}
fn tcp_target(endpoint: &ResourceEndpoint) -> EndpointAddr {
    EndpointAddr::new(endpoint.endpoint().id()).with_addrs(
        endpoint
            .addr()
            .addrs
            .into_iter()
            .filter(|a| matches!(a, TransportAddr::Custom(_))),
    )
}
async fn connect_control(
    client: &ResourceEndpoint,
    server: &ResourceEndpoint,
    target: EndpointAddr,
    alpn: &[u8],
) -> (Arc<ControlConnection>, Arc<ControlConnection>) {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (out, incoming) =
            tokio::join!(client.endpoint().connect_control(target, alpn), async {
                server
                    .endpoint()
                    .accept()
                    .await
                    .unwrap()
                    .accept_control()
                    .unwrap()
                    .await
            });
        (Arc::new(out.unwrap()), Arc::new(incoming.unwrap()))
    })
    .await
    .expect("bounded actual Native control pair")
}
async fn connect(
    client: &mut ResourceEndpoint,
    server: &ResourceEndpoint,
    target: EndpointAddr,
    alpn: &[u8],
) -> (Connection, Connection) {
    tokio::time::timeout(Duration::from_secs(15), async {
        let (out, incoming) = tokio::join!(client.connect(target, alpn), async {
            server.endpoint().accept().await.unwrap().await
        });
        (out.unwrap(), incoming.unwrap())
    })
    .await
    .expect("bounded authenticated connection")
}
async fn wait_paths(connection: &Connection, condition: impl Fn(&Connection) -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !condition(connection) {
            assert!(
                connection.close_reason().is_none(),
                "original connection terminated"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|error| {
        panic!(
            "qualified path condition: {error:?}; paths={:?}",
            connection
                .paths()
                .iter()
                .map(|p| (p.id(), p.remote_addr().clone(), p.rtt(), p.is_selected()))
                .collect::<Vec<_>>()
        )
    });
}
async fn chunk(
    send: &mut iroh::endpoint::SendStream,
    receive: &mut iroh::endpoint::RecvStream,
    bytes: &[u8],
) {
    send.write_all(bytes).await.unwrap();
    let mut actual = vec![0; bytes.len()];
    tokio::time::timeout(Duration::from_secs(5), receive.read_exact(&mut actual))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(actual, bytes);
}

#[tokio::test]
async fn first_same_stream_survives_validated_tcp_to_udp_switch() {
    let mut pair = Pair::new(true).await;
    let (client, server) = pair.business().await;
    let ids = (client.stable_id(), server.stable_id());
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"before").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut before = [0; 6];
    receive.read_exact(&mut before).await.unwrap();
    assert_eq!(&before, b"before");
    let udp = pair.server.endpoint().bound_sockets()[0];
    pair.client
        .endpoint()
        .open_data_path(&client, udp)
        .await
        .unwrap();
    wait_paths(&client, |c| c.paths().iter().any(|p| p.is_ip())).await;
    assert!(
        client
            .paths()
            .iter()
            .any(|p| matches!(p.remote_addr(), TransportAddr::Custom(_)))
    );
    for path in client
        .paths()
        .iter()
        .filter(|p| matches!(p.remote_addr(), TransportAddr::Custom(_)))
    {
        path.close().unwrap();
    }
    wait_paths(&client, |c| {
        c.paths().iter().any(|p| p.is_ip() && p.is_selected())
    })
    .await;
    chunk(
        &mut send,
        &mut receive,
        b"same stream after TCP to UDP switch",
    )
    .await;
    assert_eq!((client.stable_id(), server.stable_id()), ids);
    eprintln!(
        "MIO47_FIRST stable_ids={ids:?} same_stream=true content_complete=true paths={:?}",
        client.paths()
    );
    pair.close().await;
}

#[tokio::test]
async fn priority_closed_connection_fails_old_stream_and_reports_new_identity() {
    let mut pair = Pair::new(false).await;
    let (client, server) = pair.business().await;
    let old = (client.stable_id(), server.stable_id());
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"old stream").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut before = [0; 10];
    receive.read_exact(&mut before).await.unwrap();
    assert_eq!(&before, b"old stream");
    client.close(0u32.into(), b"original connection terminated");
    let reason = client.closed().await;
    tokio::time::timeout(Duration::from_secs(3), server.closed())
        .await
        .unwrap();
    assert!(send.write_all(b"must fail").await.is_err());
    let mut after = [0; 1];
    assert!(!matches!(receive.read_exact(&mut after).await, Ok(())));
    let (replacement, replacement_server) = pair.business().await;
    let new = (replacement.stable_id(), replacement_server.stable_id());
    assert_ne!(new.0, old.0);
    assert_ne!(new.1, old.1);
    assert!(client.close_reason().is_some());
    let (mut send, _) = replacement.open_bi().await.unwrap();
    send.write_all(b"new connection").await.unwrap();
    let (_, mut receive) = replacement_server.accept_bi().await.unwrap();
    let mut content = [0; 14];
    receive.read_exact(&mut content).await.unwrap();
    assert_eq!(&content, b"new connection");
    eprintln!("MIO47_PRIORITY old={old:?} new={new:?} old_stream_failed=true close={reason:?}");
    pair.close().await;
}

// Controlled byte carrier: route prefaces contain the proxy address; inner
// QUIC/TLS packets are unchanged. Delay/loss are real socket fault injection.
struct TcpProxy {
    address: std::net::SocketAddr,
    frames: Arc<TcpFrameControl>,
    stop: tokio_util::sync::CancellationToken,
    task: tokio::task::JoinHandle<()>,
}
struct TcpFrameControl {
    delay_ms: std::sync::atomic::AtomicU64,
    blackhole: std::sync::atomic::AtomicBool,
    loss: std::sync::atomic::AtomicBool,
    dropped: std::sync::atomic::AtomicU64,
}
const PATH_LOSS_SEED: u64 = 132;

impl TcpProxy {
    async fn new(upstream: std::net::SocketAddr, delay_ms: u64) -> Self {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let frames = Arc::new(TcpFrameControl {
            delay_ms: delay_ms.into(),
            blackhole: false.into(),
            loss: false.into(),
            dropped: 0.into(),
        });
        let frame_control = frames.clone();
        let stop = tokio_util::sync::CancellationToken::new();
        let cancel = stop.clone();
        let task = tokio::spawn(async move {
            let mut circuits = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    accepted = listener.accept() => {
                        let (mut downstream, _) = accepted.unwrap();
                        let frame_control = frame_control.clone();
                        circuits.spawn(async move {
                            let mut upstream = tokio::net::TcpStream::connect(upstream).await?;
                            downstream.set_nodelay(true)?;
                            upstream.set_nodelay(true)?;
                            let mut preface = [0; 6];
                            downstream.read_exact(&mut preface).await?;
                            let route = miopunch_native_probe::tcp_transport::TcpTransport::address(address);
                            upstream.write_all(route.data()).await?;
                            upstream.read_exact(&mut preface).await?;
                            downstream.write_all(route.data()).await?;
                            let (dr, dw) = downstream.into_split(); let (ur, uw) = upstream.into_split();
                            let _ = tokio::join!(
                                forward_frames(dr, uw, &frame_control, PATH_LOSS_SEED),
                                forward_frames(ur, dw, &frame_control, PATH_LOSS_SEED + 1),
                            );
                            Ok::<(), std::io::Error>(())
                        });
                    }
                    Some(result) = circuits.join_next(), if !circuits.is_empty() => {
                        if let Err(error) = result.unwrap() {
                            // The endpoint may retire a dial during the route
                            // preface, including when the fixture closes it.
                            assert!(matches!(error.kind(), std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe), "proxy preface: {error}");
                        }
                    }
                }
            }
            circuits.shutdown().await;
        });
        Self {
            address,
            frames,
            stop,
            task,
        }
    }
    fn custom(&self) -> iroh_base::CustomAddr {
        miopunch_native_probe::tcp_transport::TcpTransport::address(self.address)
    }
    fn delay(&self, ms: u64) {
        self.frames
            .delay_ms
            .store(ms, std::sync::atomic::Ordering::Relaxed);
    }
    fn blackhole(&self, enabled: bool) {
        self.frames
            .blackhole
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
    }
    fn loss(&self, enabled: bool) {
        self.frames
            .loss
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
    }
    async fn close(self) {
        self.stop.cancel();
        self.task.await.unwrap();
    }
}
async fn forward_frames(
    mut read: tokio::net::tcp::OwnedReadHalf,
    mut write: tokio::net::tcp::OwnedWriteHalf,
    frames: &TcpFrameControl,
    seed: u64,
) -> std::io::Result<()> {
    use rand::{RngExt, SeedableRng};
    use std::sync::atomic::Ordering::Relaxed;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut random = rand::rngs::StdRng::seed_from_u64(seed);
    loop {
        let len = read.read_u16().await?;
        assert!((1..=2048).contains(&len));
        let mut bytes = vec![0; len as usize];
        read.read_exact(&mut bytes).await?;
        if frames.blackhole.load(Relaxed)
            || (frames.loss.load(Relaxed) && random.random_ratio(1, 10))
        {
            frames.dropped.fetch_add(1, Relaxed);
            continue;
        }
        tokio::time::sleep(Duration::from_millis(frames.delay_ms.load(Relaxed))).await;
        if frames.blackhole.load(Relaxed) {
            frames.dropped.fetch_add(1, Relaxed);
            continue;
        }
        write.write_u16(len).await?;
        write.write_all(&bytes).await?;
    }
}

#[tokio::test]
async fn living_paths_are_reevaluated_when_real_latency_changes() {
    let mut pair = Pair::new(false).await;
    let target = tcp_target(&pair.server);
    let TransportAddr::Custom(address) = target.addrs.iter().next().unwrap() else {
        unreachable!()
    };
    let upstream =
        miopunch_native_probe::tcp_transport::TcpTransport::socket_addr(address).unwrap();
    let first = TcpProxy::new(upstream, 2).await;
    let second = TcpProxy::new(upstream, 30).await;
    let target = EndpointAddr::new(pair.server.endpoint().id())
        .with_addrs([TransportAddr::Custom(first.custom())]);
    let (client, server) = connect(&mut pair.client, &pair.server, target, BUSINESS).await;
    let ids = (client.stable_id(), server.stable_id());
    eprintln!(
        "MIO47_FIXTURE first={} second={} initial={:?}",
        first.address,
        second.address,
        client.paths()
    );
    pair.client
        .endpoint()
        .open_data_transport_path(&client, TransportAddr::Custom(second.custom()))
        .await
        .unwrap();
    wait_paths(&client, |c| {
        c.paths()
            .iter()
            .any(|p| p.remote_addr() == &TransportAddr::Custom(second.custom()))
    })
    .await;
    eprintln!("MIO47_FIXTURE both_established={:?}", client.paths());
    let path_ids = client
        .paths()
        .iter()
        .map(|p| p.id())
        .collect::<std::collections::BTreeSet<_>>();
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"before").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut before = [0; 6];
    receive.read_exact(&mut before).await.unwrap();
    assert_eq!(&before, b"before");
    wait_paths(&client, |c| {
        c.paths()
            .iter()
            .any(|p| p.remote_addr() == &TransportAddr::Custom(first.custom()) && p.is_selected())
    })
    .await;
    first.delay(80);
    second.delay(0);
    for _ in 0..12 {
        chunk(&mut send, &mut receive, b"same stream real latency change").await;
    }
    eprintln!(
        "MIO47_QUALITY before_wait paths={:?}",
        client
            .paths()
            .iter()
            .map(|p| (p.id(), p.remote_addr().clone(), p.rtt(), p.is_selected()))
            .collect::<Vec<_>>()
    );
    wait_paths(&client, |c| {
        c.paths()
            .iter()
            .any(|p| p.remote_addr() == &TransportAddr::Custom(second.custom()) && p.is_selected())
    })
    .await;
    assert_eq!(
        client
            .paths()
            .iter()
            .map(|p| p.id())
            .collect::<std::collections::BTreeSet<_>>(),
        path_ids,
        "quality reevaluation must not depend on opening/replacing paths"
    );
    chunk(&mut send, &mut receive, b"after quality switch").await;
    assert_eq!((client.stable_id(), server.stable_id()), ids);
    eprintln!(
        "MIO47_QUALITY stable_ids={ids:?} same_stream=true same_paths=true selected={:?}",
        client
            .paths()
            .iter()
            .filter(|p| p.is_selected())
            .map(|p| p.remote_addr().clone())
            .collect::<Vec<_>>()
    );
    pair.close().await;
    first.close().await;
    second.close().await;
}

#[tokio::test]
async fn restricted_control_paths_do_not_displace_business_quality_selection() {
    use miopunch_native_probe::{
        mqtt_transport::TRANSPORT_ID,
        resources::{Resource, ResourceUse},
    };
    let broker = std::env::var("MIOPUNCH_MQTT_BROKER").expect("owned MQTT broker required");
    let mut pair = Pair::new(false).await;
    pair.reconfigure(vec![Resource {
        url: format!("mqtt://{broker}"),
        uses: std::collections::BTreeSet::from([ResourceUse::Signaling]),
        limitation: "owned controlled test broker".into(),
    }])
    .await;
    let route = pair
        .server
        .endpoint()
        .custom_addrs()
        .into_iter()
        .find(|a| a.id() == TRANSPORT_ID)
        .unwrap();
    let target = EndpointAddr::new(pair.server.endpoint().id())
        .with_addrs([TransportAddr::Custom(route.clone())]);
    let (control, control_server) = tokio::time::timeout(Duration::from_secs(10), async {
        let (client, server) = tokio::join!(
            pair.client
                .endpoint()
                .connect_control(target, ADMISSION_REQUEST_ALPN),
            async {
                pair.server
                    .endpoint()
                    .accept()
                    .await
                    .unwrap()
                    .accept_control()
                    .unwrap()
                    .await
            }
        );
        (client.unwrap(), server.unwrap())
    })
    .await
    .unwrap();
    let target = tcp_target(&pair.server);
    let TransportAddr::Custom(address) = target.addrs.iter().next().unwrap() else {
        unreachable!()
    };
    let upstream =
        miopunch_native_probe::tcp_transport::TcpTransport::socket_addr(address).unwrap();
    let first = TcpProxy::new(upstream, 2).await;
    let second = TcpProxy::new(upstream, 30).await;
    let target = EndpointAddr::new(pair.server.endpoint().id())
        .with_addrs([TransportAddr::Custom(first.custom())]);
    let (client, server) = connect(&mut pair.client, &pair.server, target, BUSINESS).await;
    pair.client
        .endpoint()
        .open_data_transport_path(&client, TransportAddr::Custom(second.custom()))
        .await
        .unwrap();
    wait_paths(&client, |c| {
        c.paths()
            .iter()
            .any(|p| p.remote_addr() == &TransportAddr::Custom(second.custom()))
    })
    .await;
    let ids = (client.stable_id(), server.stable_id());
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"before").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut before = [0; 6];
    receive.read_exact(&mut before).await.unwrap();
    wait_paths(&client, |c| {
        c.paths()
            .iter()
            .any(|p| p.remote_addr() == &TransportAddr::Custom(first.custom()) && p.is_selected())
    })
    .await;
    first.delay(100);
    second.delay(0);
    for _ in 0..12 {
        chunk(
            &mut send,
            &mut receive,
            b"business beside live protected control",
        )
        .await;
    }
    wait_paths(&client, |c| {
        c.paths()
            .iter()
            .any(|p| p.remote_addr() == &TransportAddr::Custom(second.custom()) && p.is_selected())
    })
    .await;
    assert_eq!((client.stable_id(), server.stable_id()), ids);
    assert!(control.close_reason().is_none());
    assert!(control_server.close_reason().is_none());
    assert!(
        client.paths().iter().all(
            |p| !matches!(p.remote_addr(), TransportAddr::Custom(a) if a.id() == TRANSPORT_ID)
        )
    );
    assert!(
        pair.client
            .endpoint()
            .open_data_transport_path(&client, TransportAddr::Custom(route))
            .await
            .is_err()
    );
    chunk(
        &mut send,
        &mut receive,
        b"signaling remains forbidden for business",
    )
    .await;
    eprintln!(
        "MIO47_ISOLATION same_stream=true stable_ids={ids:?} control_alive=true signaling_business_paths=0"
    );
    pair.close().await;
    first.close().await;
    second.close().await;
}

async fn native_relay() -> iroh_relay::server::Server {
    let mut config = iroh_relay::server::ServerConfig::default();
    config.relay = Some(iroh_relay::server::RelayConfig::new((
        Ipv4Addr::LOCALHOST,
        0,
    )));
    iroh_relay::server::Server::spawn(config).await.unwrap()
}
fn relay_resource(url: &iroh::RelayUrl) -> miopunch_native_probe::resources::Resource {
    miopunch_native_probe::resources::Resource {
        url: url.to_string(),
        uses: std::collections::BTreeSet::from([
            miopunch_native_probe::resources::ResourceUse::DataRelay,
        ]),
        limitation: "owned native relay; real endpoint TLS, no public availability claim".into(),
    }
}
async fn open_carrier(endpoint: &ResourceEndpoint, connection: &Connection, remote: TransportAddr) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match endpoint
                .endpoint()
                .open_data_transport_path(connection, remote.clone())
                .await
            {
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    tokio::time::sleep(Duration::from_millis(50)).await
                }
                result => {
                    result.unwrap();
                    break;
                }
            }
        }
    })
    .await
    .unwrap();
    wait_paths(connection, |c| {
        c.paths().iter().any(|p| p.remote_addr() == &remote)
    })
    .await;
}

#[tokio::test]
async fn tcp_blackhole_recovers_original_stream_under_continuous_writes() {
    direct_blackhole_with_relay(LOW_RELAY_DELAY, true, true, false).await;
}

#[tokio::test]
async fn tcp_blackhole_recovers_original_stream_after_one_write() {
    direct_blackhole_with_relay(LOW_RELAY_DELAY, false, false, false).await;
}

#[tokio::test]
async fn tcp_blackhole_recovers_original_stream_with_high_rtt_relay() {
    direct_blackhole_with_relay(HIGH_RELAY_DELAY, true, false, false).await;
    direct_blackhole_with_relay(HIGH_RELAY_DELAY, false, false, false).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "uses iptables inside the UDP NAT lab's disposable network namespace"]
async fn udp_blackhole_recovers_original_stream_in_netns() {
    direct_blackhole_with_relay(LOW_RELAY_DELAY, true, false, true).await;
}

const SEND_INTERVAL: Duration = Duration::from_millis(25);
const REEVALUATION: Duration = Duration::from_secs(1);
const SCHEDULING_TOLERANCE: Duration = Duration::from_millis(150);
const LOW_RELAY_DELAY: Duration = Duration::from_millis(25);
const HIGH_RELAY_DELAY: Duration = Duration::from_millis(350);
// One selection cycle, about 3 s for recovery/backup confirmation, up to 1.8 s
// relay application RTT, plus send cadence and scheduling. This same deadline
// is used for both fault and relay-only control tests at both RTTs.
const BUSINESS_LIMIT: Duration = Duration::from_secs(6);

#[tokio::test]
async fn tcp_blackhole_healthy_relay_controls_use_same_business_deadline() {
    for (delay, min_rtt) in [
        (LOW_RELAY_DELAY, Duration::ZERO),
        (HIGH_RELAY_DELAY, Duration::from_secs(1)),
    ] {
        let service = native_relay().await;
        let (url, stop) = delay_relay(service.http_addr().unwrap(), delay).await;
        let mut pair = Pair::new(false).await;
        pair.reconfigure(vec![relay_resource(&url)]).await;
        let target =
            EndpointAddr::new(pair.server.endpoint().id()).with_addrs([TransportAddr::Relay(url)]);
        let (client, server) = connect(&mut pair.client, &pair.server, target, BUSINESS).await;
        let (mut send, mut answer) = client.open_bi().await.unwrap();
        let started = std::time::Instant::now();
        tokio::time::timeout(BUSINESS_LIMIT, async {
            let ((), ()) = tokio::join!(
                async {
                    send.write_all(b"healthy relay").await.unwrap();
                    let mut data = [0; 13];
                    answer.read_exact(&mut data).await.unwrap();
                    assert_eq!(&data, b"healthy relay");
                },
                async {
                    let (mut reply, mut request) = server.accept_bi().await.unwrap();
                    let mut data = [0; 13];
                    request.read_exact(&mut data).await.unwrap();
                    assert_eq!(&data, b"healthy relay");
                    reply.write_all(&data).await.unwrap();
                }
            );
        })
        .await
        .expect("relay-only comparator shares the fault test business deadline");
        let rtt = started.elapsed();
        assert!(
            rtt >= min_rtt,
            "configured high RTT must reach the business stream: {rtt:?}"
        );
        for conn in [&client, &server] {
            assert!(conn.paths().iter().all(|path| path.is_relay()));
        }
        eprintln!(
            "MIO132 healthy_relay relay_leg_delay={delay:?} app_rtt={rtt:?} deadline={BUSINESS_LIMIT:?}"
        );
        pair.close().await;
        stop.cancel();
        service.shutdown().await.unwrap();
    }
}

async fn direct_blackhole_with_relay(
    relay_delay: Duration,
    continuous: bool,
    seeded_loss: bool,
    udp: bool,
) {
    use std::time::Instant;
    let service = native_relay().await;
    let (relay, delay_stop) = delay_relay(service.http_addr().unwrap(), relay_delay).await;
    let mut pair = Pair::new(udp).await;
    pair.reconfigure(vec![relay_resource(&relay)]).await;
    let (direct, tcp) = if udp {
        (
            TransportAddr::Ip(pair.server.endpoint().bound_sockets()[0]),
            None,
        )
    } else {
        let TransportAddr::Custom(address) =
            tcp_target(&pair.server).addrs.into_iter().next().unwrap()
        else {
            unreachable!();
        };
        let upstream =
            miopunch_native_probe::tcp_transport::TcpTransport::socket_addr(&address).unwrap();
        let tcp = TcpProxy::new(upstream, 1).await;
        (TransportAddr::Custom(tcp.custom()), Some(tcp))
    };
    let target = EndpointAddr::new(pair.server.endpoint().id()).with_addrs([direct]);
    let (client, server) = connect(&mut pair.client, &pair.server, target, BUSINESS).await;
    let ids = (client.stable_id(), server.stable_id());
    assert!(
        pair.joining
            .admission_gate()
            .allows(&client, ProtocolScope::Member)
    );
    let (mut send, mut answers) = client.open_bi().await.unwrap();
    send.write_all(&0u64.to_be_bytes()).await.unwrap();
    let (mut echo, mut requests) = server.accept_bi().await.unwrap();
    let echo_task = tokio::spawn(async move {
        let mut expected = 0u64;
        let mut bytes = [0; 8];
        while requests.read_exact(&mut bytes).await.is_ok() {
            assert_eq!(u64::from_be_bytes(bytes), expected);
            if expected == 1 {
                eprintln!("MIO132 first_original_request at={:?}", Instant::now());
            }
            echo.write_all(&bytes).await.unwrap();
            expected += 1;
        }
    });
    answers.read_exact(&mut [0; 8]).await.unwrap();
    open_carrier(&pair.client, &client, TransportAddr::Relay(relay)).await;
    for connection in [&client, &server] {
        wait_paths(connection, |connection| {
            let paths = connection.paths();
            paths
                .iter()
                .any(|path| path.is_relay() && path.stats().acknowledgment.is_some())
                && paths
                    .iter()
                    .any(|path| !path.is_relay() && path.is_selected())
        })
        .await;
        for path in connection.paths().iter() {
            eprintln!(
                "MIO132 ready side={:?} path={} relay={} rtt={:?} selected={}",
                connection.side(),
                path.id(),
                path.is_relay(),
                path.rtt(),
                path.is_selected()
            );
        }
    }

    let cut = tokio::time::Instant::now();
    let deadline = cut + BUSINESS_LIMIT;
    let released = tokio_util::sync::CancellationToken::new();
    let connections = [&client, &server];
    let original_paths = connections.map(|connection| {
        connection
            .paths()
            .iter()
            .find(|path| !path.is_relay() && path.is_selected())
            .unwrap()
            .id()
    });
    let observed = std::array::from_fn::<_, 2, _>(|side| {
        tokio::spawn(observe_failover(
            connections[side].clone(),
            original_paths[side],
            released.clone(),
        ))
    });
    eprintln!(
        "MIO132 cut at={:?} stable_ids={ids:?} direction=both continuous={continuous} relay_leg_delay={relay_delay:?} send_interval={SEND_INTERVAL:?} deadline={BUSINESS_LIMIT:?}",
        cut.into_std()
    );
    let mut udp_filter = None;
    if let Some(tcp) = &tcp {
        tcp.blackhole(true);
    } else {
        let endpoints = [&client, &server].map(|connection| {
            let paths = connection.paths();
            let path = paths
                .iter()
                .find(|path| path.is_ip() && path.is_selected())
                .unwrap();
            let TransportAddr::Ip(remote) = path.remote_addr() else {
                unreachable!()
            };
            *remote
        });
        udp_filter = Some(UdpBlackhole::new(endpoints));
    }
    eprintln!("MIO132 first_unacknowledged_write at={:?}", Instant::now());
    send.write_all(&1u64.to_be_bytes()).await.unwrap();
    let last_sequence = if continuous {
        (BUSINESS_LIMIT.as_millis() / SEND_INTERVAL.as_millis()) as u64
    } else {
        1
    };
    let writer = async {
        let mut cadence = tokio::time::interval_at(cut + SEND_INTERVAL, SEND_INTERVAL);
        cadence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        for sequence in 2u64..=last_sequence {
            cadence.tick().await;
            send.write_all(&sequence.to_be_bytes()).await.unwrap();
        }
    };
    let reader = async {
        let mut answer = [0; 8];
        let result = tokio::time::timeout_at(deadline, answers.read_exact(&mut answer)).await;
        let first_ack = Instant::now();
        eprintln!(
            "MIO132 first_application_ack result={result:?} at={first_ack:?} elapsed={:?}",
            cut.elapsed()
        );
        result.expect("healthy relay must recover original bytes while the direct path remains blackholed by the fixed business deadline").unwrap();
        assert_eq!(answer, 1u64.to_be_bytes());
        for sequence in 2..=last_sequence {
            tokio::time::timeout_at(deadline + BUSINESS_LIMIT, answers.read_exact(&mut answer))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(answer, sequence.to_be_bytes());
        }
        first_ack
    };
    let ((), first_ack) = tokio::join!(writer, reader);
    tokio::time::sleep_until(tokio::time::Instant::from_std(first_ack) + 3 * REEVALUATION).await;
    let release_at = Instant::now();
    released.cancel();
    for task in observed {
        // Each sampler has independently checked that direct stayed registered
        // and unselected after the switch, including throughout these cycles.
        let (eligible, selected) = task.await.unwrap();
        if let Some(eligible) = eligible {
            assert!(
                selected <= eligible + REEVALUATION + SCHEDULING_TOLERANCE,
                "relay activation must occur within one reevaluation after confirmed failure and backup ACK"
            );
        }
    }
    if let Some(tcp) = &tcp {
        tcp.blackhole(false);
    }
    drop(udp_filter);
    eprintln!("MIO132 release at={release_at:?}");
    for (connection, original) in connections.into_iter().zip(original_paths) {
        wait_paths(connection, |connection| {
            connection.paths().iter().any(|path| {
                path.id() == original
                    && (udp || path.is_selected())
                    && path
                        .stats()
                        .acknowledgment
                        .is_some_and(|ack| ack.sent_at > release_at)
            })
        })
        .await;
    }
    if udp {
        tokio::join!(
            observe_restored_quality(&client, original_paths[0]),
            observe_restored_quality(&server, original_paths[1])
        );
    }
    let next = last_sequence + 1;
    send.write_all(&next.to_be_bytes()).await.unwrap();
    let mut answer = [0; 8];
    tokio::time::timeout(BUSINESS_LIMIT, answers.read_exact(&mut answer))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(answer, next.to_be_bytes());

    if seeded_loss {
        check_seeded_loss(
            tcp.as_ref().unwrap(),
            [&client, &server],
            &mut send,
            &mut answers,
            next + 1,
        )
        .await;
    }
    assert_eq!((client.stable_id(), server.stable_id()), ids);
    echo_task.abort();
    pair.close().await;
    if let Some(tcp) = tcp {
        tcp.close().await;
    }
    delay_stop.cancel();
    service.shutdown().await.unwrap();
}

async fn observe_restored_quality(connection: &Connection, original: noq_proto::PathId) {
    // An authenticated late ACK can leave the recovered path's smoothed RTT
    // above relay for several cycles. A fresh ACK makes it eligible for normal
    // Quality again. It takes over once it earns the five millisecond margin.
    let started = std::time::Instant::now();
    let mut direct_lead = None;
    loop {
        let paths = connection.paths();
        let direct = paths.iter().find(|path| path.id() == original).unwrap();
        let relay = paths.iter().find(|path| path.is_relay()).unwrap();
        let (direct_rtt, relay_rtt) = (direct.rtt(), relay.rtt());
        if direct.is_selected()
            || (started.elapsed() >= Duration::from_secs(15) && direct_lead.is_none())
        {
            eprintln!(
                "MIO132 restored_quality side={:?} direct_rtt={direct_rtt:?} relay_rtt={relay_rtt:?} direct_selected={} direct_ack={:?}",
                connection.side(),
                direct.is_selected(),
                direct.stats().acknowledgment
            );
            break;
        }
        assert!(relay.is_selected());
        if direct_rtt + Duration::from_millis(5) <= relay_rtt {
            let lead = direct_lead.get_or_insert_with(std::time::Instant::now);
            assert!(
                lead.elapsed() <= REEVALUATION + SCHEDULING_TOLERANCE,
                "the recovered direct path won normal Quality but remained demoted"
            );
        } else {
            direct_lead = None;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

struct UdpBlackhole(Vec<(u16, u16)>);

impl UdpBlackhole {
    fn new([left, right]: [std::net::SocketAddr; 2]) -> Self {
        let mut filter = Self(Vec::new());
        for (from, to) in [(left.port(), right.port()), (right.port(), left.port())] {
            assert!(
                Self::rule("-I", from, to),
                "install UDP fault inside the lab namespace"
            );
            filter.0.push((from, to));
        }
        filter
    }

    fn rule(action: &str, from: u16, to: u16) -> bool {
        std::process::Command::new("iptables")
            .args([
                "-w",
                action,
                "OUTPUT",
                "-o",
                "lo",
                "-p",
                "udp",
                "--sport",
                &from.to_string(),
                "--dport",
                &to.to_string(),
                "-j",
                "DROP",
            ])
            .status()
            .unwrap()
            .success()
    }
}

impl Drop for UdpBlackhole {
    fn drop(&mut self) {
        for (from, to) in self.0.drain(..) {
            assert!(
                Self::rule("-D", from, to),
                "remove UDP fault from the lab namespace"
            );
        }
    }
}

async fn observe_failover(
    connection: Connection,
    original: noq_proto::PathId,
    released: tokio_util::sync::CancellationToken,
) -> (Option<std::time::Instant>, std::time::Instant) {
    use std::time::Instant;
    let (mut second_pto, mut backup_ack, mut eligible, mut selected) = (None, None, None, None);
    let mut last = std::collections::BTreeMap::new();
    while !released.is_cancelled() {
        let now = Instant::now();
        let paths = connection.paths();
        let direct = paths
            .iter()
            .find(|path| path.id() == original)
            .expect("fault retains the original direct path");
        let relay = paths
            .iter()
            .find(|path| path.is_relay())
            .expect("validated backup stays present");
        second_pto = second_pto.or_else(|| (direct.stats().pto_count >= 2).then_some(now));
        let relay_stats = relay.stats();
        if let Some(ack) = relay_stats.acknowledgment.filter(|ack| {
            relay_stats.pto_count < 2
                && now.saturating_duration_since(ack.received_at)
                    <= 2 * REEVALUATION.max(relay_stats.probe_timeout)
        }) {
            backup_ack.get_or_insert(ack);
            if let Some(at) = second_pto {
                // A recent ACK for a pre-fault send may already qualify the
                // backup. Anchor to that ACK, even if later probes are pending.
                eligible.get_or_insert(at.max(ack.received_at));
            }
        }
        if relay.is_selected() {
            selected.get_or_insert(now);
        }
        if selected.is_some() {
            assert!(
                relay.is_selected() && !direct.is_selected(),
                "blackholed path was reselected without recovery"
            );
        }
        for path in paths.iter() {
            let stats = path.stats();
            let snapshot = (
                stats.pto_count,
                stats.acknowledgment,
                path.is_selected(),
                stats.frame_tx.ping,
                stats.frame_tx.stream,
            );
            if last.insert(path.id(), snapshot) != Some(snapshot) {
                eprintln!(
                    "MIO132 path sampled_at={now:?} side={:?} id={} relay={} pto_count={} ack={:?} ping_tx={} stream_tx={} selected={}",
                    connection.side(),
                    path.id(),
                    path.is_relay(),
                    stats.pto_count,
                    stats.acknowledgment,
                    stats.frame_tx.ping,
                    stats.frame_tx.stream,
                    path.is_selected()
                );
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let backup_ack = backup_ack.expect("fresh backup ACK during the fault");
    let selected = selected.expect("relay selected during the fault");
    eprintln!(
        "MIO132 switch_summary side={:?} second_pto_observed={second_pto:?} first_recent_backup_ack={backup_ack:?} eligible={eligible:?} selected_observed={selected:?}",
        connection.side()
    );
    (eligible, selected)
}

async fn check_seeded_loss(
    tcp: &TcpProxy,
    connections: [&Connection; 2],
    send: &mut iroh::endpoint::SendStream,
    answers: &mut iroh::endpoint::RecvStream,
    first_sequence: u64,
) {
    use std::{sync::atomic::Ordering::Relaxed, time::Instant};
    let idle = || {
        connections.iter().all(|connection| {
            connection.paths().iter().all(|path| {
                let stats = path.stats();
                stats.ack_pending_since.is_none() && stats.pto_count == 0
            })
        })
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if idle() {
                tokio::time::sleep(Duration::from_millis(200)).await;
                if idle() {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("no outstanding sends and no PTO growth while idle");
    eprintln!(
        "MIO132 idle in_flight=false pto_count=0 at={:?}",
        Instant::now()
    );

    const LOSS_DURATION: Duration = Duration::from_secs(3);
    let loss_started = tokio::time::Instant::now();
    let end = loss_started + LOSS_DURATION;
    let frames = LOSS_DURATION.as_millis() as u64 / SEND_INTERVAL.as_millis() as u64;
    let dropped_before = tcp.frames.dropped.load(Relaxed);
    tcp.loss(true);
    let writer = async {
        let mut cadence = tokio::time::interval_at(loss_started, SEND_INTERVAL);
        cadence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        for sequence in first_sequence..first_sequence + frames {
            cadence.tick().await;
            send.write_all(&sequence.to_be_bytes()).await.unwrap();
        }
    };
    let reader = async {
        let mut answer = [0; 8];
        for sequence in first_sequence..first_sequence + frames {
            answers.read_exact(&mut answer).await.unwrap();
            assert_eq!(answer, sequence.to_be_bytes());
        }
    };
    let observe = async {
        let mut last: [Option<(bool, u32, Option<Instant>)>; 2] = [None; 2];
        while tokio::time::Instant::now() < end {
            for (connection, previous) in connections.iter().zip(&mut last) {
                let paths = connection.paths();
                let direct = paths.iter().find(|path| !path.is_relay()).unwrap();
                let stats = direct.stats();
                if let Some((was_direct, _, ack)) = *previous
                    && !was_direct
                    && direct.is_selected()
                {
                    assert!(
                        stats.acknowledgment.map(|ack| ack.sent_at) > ack,
                        "direct must demonstrate new ACK progress before it is reselected"
                    );
                }
                let now = (
                    direct.is_selected(),
                    stats.pto_count,
                    stats.acknowledgment.map(|ack| ack.sent_at),
                );
                if *previous != Some(now) {
                    eprintln!(
                        "MIO132 seeded_loss at={:?} side={:?} path={} selected={} pto_count={} ack={:?}",
                        Instant::now(),
                        connection.side(),
                        direct.id(),
                        now.0,
                        now.1,
                        stats.acknowledgment
                    );
                    *previous = Some(now);
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tcp.loss(false);
    };
    tokio::time::timeout_at(end + BUSINESS_LIMIT, async {
        tokio::join!(writer, reader, observe)
    })
    .await
    .expect("ordered application bytes through seeded 10 percent loss");
    let dropped = tcp.frames.dropped.load(Relaxed) - dropped_before;
    assert!(dropped > 0);
    eprintln!(
        "MIO132 seeded_loss_complete seeds={PATH_LOSS_SEED}/{} probability=10% duration={LOSS_DURATION:?} dropped={dropped}",
        PATH_LOSS_SEED + 1
    );
}

#[tokio::test]
async fn quality_selects_better_native_relay_and_keeps_tcp_as_existing_backup() {
    let service = native_relay().await;
    let url = service.http_url().unwrap();
    let mut pair = Pair::new(false).await;
    pair.reconfigure(vec![relay_resource(&url)]).await;
    let target = tcp_target(&pair.server);
    let TransportAddr::Custom(address) = target.addrs.iter().next().unwrap() else {
        unreachable!()
    };
    let upstream =
        miopunch_native_probe::tcp_transport::TcpTransport::socket_addr(address).unwrap();
    let proxy = TcpProxy::new(upstream, 30).await;
    let target = EndpointAddr::new(pair.server.endpoint().id())
        .with_addrs([TransportAddr::Custom(proxy.custom())]);
    let (client, server) = tokio::time::timeout(Duration::from_secs(15), async {
        let (out, incoming) =
            tokio::join!(pair.client.endpoint().connect(target, BUSINESS), async {
                pair.server.endpoint().accept().await.unwrap().await
            });
        (out.unwrap(), incoming.unwrap())
    })
    .await
    .unwrap();
    let ids = (client.stable_id(), server.stable_id());
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"TCP").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut before = [0; 3];
    receive.read_exact(&mut before).await.unwrap();
    assert_eq!(&before, b"TCP");
    open_carrier(&pair.client, &client, TransportAddr::Relay(url.clone())).await;
    wait_paths(&client, |c| {
        c.paths().iter().any(|p| p.is_relay() && p.is_selected())
    })
    .await;
    let paths = client
        .paths()
        .iter()
        .map(|p| (p.remote_addr().clone(), p.rtt(), p.is_selected()))
        .collect::<Vec<_>>();
    eprintln!("MIO47_RELAY_QUALITY before_cut={paths:?}");
    assert!(
        client
            .paths()
            .iter()
            .any(|p| matches!(p.remote_addr(), TransportAddr::Custom(_)))
    );
    chunk(
        &mut send,
        &mut receive,
        b"same stream on better permitted native relay",
    )
    .await;
    // Removing the selected carrier retains the already validated TCP backup.
    for path in client.paths().iter().filter(|p| p.is_relay()) {
        path.close().unwrap();
    }
    wait_paths(&client, |c| {
        c.paths()
            .iter()
            .any(|p| matches!(p.remote_addr(), TransportAddr::Custom(_)) && p.is_selected())
    })
    .await;
    chunk(
        &mut send,
        &mut receive,
        b"same stream on surviving TCP backup",
    )
    .await;
    assert_eq!((client.stable_id(), server.stable_id()), ids);
    eprintln!(
        "MIO47_MATRIX TCP->Relay->TCP stable_ids={ids:?} same_stream=true content_complete=true"
    );
    pair.close().await;
    proxy.close().await;
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn control_paths_recover_a_new_tcp_carrier_within_the_original_connection_window() {
    use miopunch_native_probe::{
        connection_recovery::CONNECTION_RECOVERY_ALPN,
        mqtt_transport::TRANSPORT_ID,
        resources::{Resource, ResourceUse},
    };
    let broker = std::env::var("MIOPUNCH_MQTT_BROKER").expect("owned MQTT broker required");
    let mut pair = Pair::new(false).await;
    pair.reconfigure(vec![Resource {
        url: format!("mqtt://{broker}"),
        uses: std::collections::BTreeSet::from([ResourceUse::Signaling]),
        limitation: "owned broker for protected path repair".into(),
    }])
    .await;
    pair.server
        .endpoint()
        .set_alpns(vec![BUSINESS.to_vec(), CONNECTION_RECOVERY_ALPN.to_vec()]);
    let route = pair
        .server
        .endpoint()
        .custom_addrs()
        .into_iter()
        .find(|a| a.id() == TRANSPORT_ID)
        .unwrap();
    let control_target =
        EndpointAddr::new(pair.server.endpoint().id()).with_addrs([TransportAddr::Custom(route)]);
    let target = tcp_target(&pair.server);
    let TransportAddr::Custom(address) = target.addrs.iter().next().unwrap() else {
        unreachable!()
    };
    let upstream =
        miopunch_native_probe::tcp_transport::TcpTransport::socket_addr(address).unwrap();
    let proxy = TcpProxy::new(upstream, 0).await;
    let target = EndpointAddr::new(pair.server.endpoint().id())
        .with_addrs([TransportAddr::Custom(proxy.custom())]);
    let (client, server) = connect(&mut pair.client, &pair.server, target, BUSINESS).await;
    let ids = (client.stable_id(), server.stable_id());
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"before cut").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut before = [0; 10];
    receive.read_exact(&mut before).await.unwrap();
    assert_eq!(&before, b"before cut");
    assert_eq!(client.paths().len(), 1, "no pre-established backup");
    proxy.close().await;
    assert!(
        client.close_reason().is_none(),
        "repair starts only within original QUIC lifetime"
    );
    let started = std::time::Instant::now();
    let served = async {
        let control = pair
            .server
            .endpoint()
            .accept()
            .await
            .unwrap()
            .accept_control()
            .unwrap()
            .await
            .unwrap();
        let result = pair
            .admin
            .serve_connection_path_recovery(&pair.server, &control)
            .await;
        control.close(0u32.into(), b"path repair complete");
        result
    };
    let (recovered, served) = tokio::join!(
        pair.joining
            .recover_connection_path(&pair.client, &client, control_target),
        served
    );
    eprintln!("MIO47_CONTROL_RESULT recovered={recovered:?} served={served:?}");
    let recovered = recovered.unwrap();
    served.unwrap();
    assert_eq!(recovered.stable_id, ids.0);
    chunk(
        &mut send,
        &mut receive,
        b"same original stream after protected path recovery",
    )
    .await;
    assert_eq!((client.stable_id(), server.stable_id()), ids);
    assert!(
        client.paths().iter().all(
            |p| !matches!(p.remote_addr(), TransportAddr::Custom(a) if a.id() == TRANSPORT_ID)
        )
    );
    eprintln!(
        "MIO47_CONTROL_RECOVERY stable_ids={ids:?} same_stream=true new_carrier={:?} elapsed_ms={}",
        recovered.path,
        started.elapsed().as_millis()
    );
    pair.close().await;
}

#[tokio::test]
async fn control_paths_never_recover_a_closed_business_connection() {
    use miopunch_native_probe::{
        connection_recovery::ConnectionPathRecoveryError,
        mqtt_transport::TRANSPORT_ID,
        resources::{Resource, ResourceUse},
    };
    let broker = std::env::var("MIOPUNCH_MQTT_BROKER").expect("owned MQTT broker required");
    let mut pair = Pair::new(false).await;
    pair.reconfigure(vec![Resource {
        url: format!("mqtt://{broker}"),
        uses: std::collections::BTreeSet::from([ResourceUse::Signaling]),
        limitation: "owned control-only broker".into(),
    }])
    .await;
    let route = pair
        .server
        .endpoint()
        .custom_addrs()
        .into_iter()
        .find(|a| a.id() == TRANSPORT_ID)
        .unwrap();
    let contact =
        EndpointAddr::new(pair.server.endpoint().id()).with_addrs([TransportAddr::Custom(route)]);
    let (control, control_server) = tokio::time::timeout(Duration::from_secs(10), async {
        let (out, incoming) = tokio::join!(
            pair.client
                .endpoint()
                .connect_control(contact.clone(), ADMISSION_REQUEST_ALPN),
            async {
                pair.server
                    .endpoint()
                    .accept()
                    .await
                    .unwrap()
                    .accept_control()
                    .unwrap()
                    .await
            }
        );
        (out.unwrap(), incoming.unwrap())
    })
    .await
    .unwrap();
    let (client, server) = pair.business().await;
    let old = client.stable_id();
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"old").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut old_bytes = [0; 3];
    receive.read_exact(&mut old_bytes).await.unwrap();
    client.close(0u32.into(), b"terminal old session");
    tokio::time::timeout(Duration::from_secs(3), server.closed())
        .await
        .unwrap();
    let result = pair
        .joining
        .recover_connection_path(&pair.client, &client, contact)
        .await;
    assert!(
        matches!(result, Err(ConnectionPathRecoveryError::Terminal { stable_id }) if stable_id == old)
    );
    assert!(control.close_reason().is_none());
    assert!(control_server.close_reason().is_none());
    assert!(send.write_all(b"old must fail").await.is_err());
    let (replacement, replacement_server) = pair.business().await;
    assert_ne!(replacement.stable_id(), old);
    let (mut new_send, _) = replacement.open_bi().await.unwrap();
    new_send.write_all(b"new").await.unwrap();
    let (_, mut new_receive) = replacement_server.accept_bi().await.unwrap();
    let mut bytes = [0; 3];
    new_receive.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"new");
    eprintln!(
        "MIO47_CLOSED_WITH_CONTROL old={old} new={} terminal=true control_alive=true old_stream_failed=true",
        replacement.stable_id()
    );
    pair.close().await;
}

#[tokio::test]
async fn explicit_relay_preference_switches_the_same_udp_stream_and_retains_udp_backup() {
    use miopunch_native_probe::path_selection::PathPreference;
    let service = native_relay().await;
    let url = service.http_url().unwrap();
    let mut pair = Pair::new(true).await;
    for node in [&pair.admin, &pair.joining] {
        let mut config = node.resource_configuration().unwrap();
        config.path_selection.preference = PathPreference::PreferRelay;
        node.save_resource_configuration(&config).unwrap();
    }
    pair.reconfigure(vec![relay_resource(&url)]).await;
    let address = pair.server.endpoint().bound_sockets()[0];
    let target =
        EndpointAddr::new(pair.server.endpoint().id()).with_addrs([TransportAddr::Ip(address)]);
    let (client, server) = tokio::time::timeout(Duration::from_secs(15), async {
        let (out, incoming) =
            tokio::join!(pair.client.endpoint().connect(target, BUSINESS), async {
                pair.server.endpoint().accept().await.unwrap().await
            });
        (out.unwrap(), incoming.unwrap())
    })
    .await
    .unwrap();
    let ids = (client.stable_id(), server.stable_id());
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"UDP").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut before = [0; 3];
    receive.read_exact(&mut before).await.unwrap();
    assert_eq!(&before, b"UDP");
    open_carrier(&pair.client, &client, TransportAddr::Relay(url.clone())).await;
    wait_paths(&client, |c| {
        c.paths().iter().any(|p| p.is_relay() && p.is_selected())
    })
    .await;
    assert!(client.paths().iter().any(|p| p.is_ip()));
    chunk(
        &mut send,
        &mut receive,
        b"same UDP stream on preferred relay",
    )
    .await;
    for path in client.paths().iter().filter(|p| p.is_relay()) {
        path.close().unwrap();
    }
    wait_paths(&client, |c| {
        c.paths().iter().any(|p| p.is_ip() && p.is_selected())
    })
    .await;
    chunk(
        &mut send,
        &mut receive,
        b"same stream restored to UDP backup",
    )
    .await;
    assert_eq!((client.stable_id(), server.stable_id()), ids);
    eprintln!(
        "MIO47_MATRIX UDP->Relay->UDP stable_ids={ids:?} preference=PreferRelay same_stream=true content_complete=true"
    );
    pair.close().await;
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn explicit_direct_preference_wins_even_when_permitted_relay_is_faster() {
    use miopunch_native_probe::path_selection::PathPreference;
    let service = native_relay().await;
    let url = service.http_url().unwrap();
    let mut pair = Pair::new(false).await;
    for node in [&pair.admin, &pair.joining] {
        let mut config = node.resource_configuration().unwrap();
        config.path_selection.preference = PathPreference::PreferDirect;
        node.save_resource_configuration(&config).unwrap();
    }
    pair.reconfigure(vec![relay_resource(&url)]).await;
    let target = tcp_target(&pair.server);
    let TransportAddr::Custom(address) = target.addrs.iter().next().unwrap() else {
        unreachable!()
    };
    let proxy = TcpProxy::new(
        miopunch_native_probe::tcp_transport::TcpTransport::socket_addr(address).unwrap(),
        30,
    )
    .await;
    let target = EndpointAddr::new(pair.server.endpoint().id())
        .with_addrs([TransportAddr::Custom(proxy.custom())]);
    let (client, server) = tokio::time::timeout(Duration::from_secs(15), async {
        let (out, incoming) =
            tokio::join!(pair.client.endpoint().connect(target, BUSINESS), async {
                pair.server.endpoint().accept().await.unwrap().await
            });
        (out.unwrap(), incoming.unwrap())
    })
    .await
    .unwrap();
    let ids = (client.stable_id(), server.stable_id());
    open_carrier(&pair.client, &client, TransportAddr::Relay(url)).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let tcp = client
        .paths()
        .iter()
        .find(|p| matches!(p.remote_addr(), TransportAddr::Custom(_)))
        .unwrap()
        .rtt();
    let relay = client.paths().iter().find(|p| p.is_relay()).unwrap().rtt();
    assert!(
        relay < tcp,
        "actual relay must be better in this fixture: {relay:?} vs {tcp:?}"
    );
    assert!(
        client
            .paths()
            .iter()
            .any(|p| matches!(p.remote_addr(), TransportAddr::Custom(_)) && p.is_selected())
    );
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"explicit preference").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut bytes = [0; 19];
    receive.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"explicit preference");
    assert_eq!((client.stable_id(), server.stable_id()), ids);
    eprintln!(
        "MIO47_PREFERENCE direct_rtt={tcp:?} relay_rtt={relay:?} selected=TCP stable_ids={ids:?}"
    );
    pair.close().await;
    proxy.close().await;
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn disabled_relay_is_never_opened_or_selected_even_with_explicit_relay_preference() {
    use miopunch_native_probe::{path_selection::PathPreference, resources::ResourceUse};
    let service = native_relay().await;
    let url = service.http_url().unwrap();
    let mut pair = Pair::new(false).await;
    for node in [&pair.admin, &pair.joining] {
        let mut config = node.resource_configuration().unwrap();
        config.path_selection.preference = PathPreference::PreferRelay;
        config
            .disabled
            .insert((url.to_string(), ResourceUse::DataRelay));
        node.save_resource_configuration(&config).unwrap();
    }
    pair.reconfigure(vec![relay_resource(&url)]).await;
    let (client, server) = pair.business().await;
    let ids = (client.stable_id(), server.stable_id());
    pair.client
        .endpoint()
        .open_data_transport_path(&client, TransportAddr::Relay(url))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(client.paths().iter().all(|p| !p.is_relay()));
    assert_eq!(
        service.metrics().server.accepts.get(),
        0,
        "no access to disabled origin"
    );
    assert_eq!(service.metrics().server.bytes_recv.get(), 0);
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"allowed TCP").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut bytes = [0; 11];
    receive.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"allowed TCP");
    assert_eq!((client.stable_id(), server.stable_id()), ids);
    eprintln!(
        "MIO47_DISABLED preference=PreferRelay actual_relay_accesses=0 relay_paths=0 TCP_stream=complete stable_ids={ids:?}"
    );
    pair.close().await;
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn control_paths_stay_live_while_physical_business_loss_ends_queued_bytes() {
    use miopunch_native_probe::{
        connection_recovery::ConnectionPathRecoveryError,
        mqtt_transport::TRANSPORT_ID,
        resources::{Resource, ResourceUse},
    };
    let broker = std::env::var("MIOPUNCH_MQTT_BROKER").expect("owned MQTT broker required");
    let mut pair = Pair::new(false).await;
    pair.reconfigure(vec![Resource {
        url: format!("mqtt://{broker}"),
        uses: std::collections::BTreeSet::from([ResourceUse::Signaling]),
        limitation: "owned control-only broker".into(),
    }])
    .await;
    let route = pair
        .server
        .endpoint()
        .custom_addrs()
        .into_iter()
        .find(|a| a.id() == TRANSPORT_ID)
        .unwrap();
    let contact =
        EndpointAddr::new(pair.server.endpoint().id()).with_addrs([TransportAddr::Custom(route)]);
    let (control, control_server) = tokio::time::timeout(Duration::from_secs(10), async {
        let (out, incoming) = tokio::join!(
            pair.client
                .endpoint()
                .connect_control(contact.clone(), ADMISSION_REQUEST_ALPN),
            async {
                pair.server
                    .endpoint()
                    .accept()
                    .await
                    .unwrap()
                    .accept_control()
                    .unwrap()
                    .await
            }
        );
        (out.unwrap(), incoming.unwrap())
    })
    .await
    .unwrap();
    let target = tcp_target(&pair.server);
    let TransportAddr::Custom(address) = target.addrs.iter().next().unwrap() else {
        unreachable!()
    };
    let proxy = TcpProxy::new(
        miopunch_native_probe::tcp_transport::TcpTransport::socket_addr(address).unwrap(),
        0,
    )
    .await;
    let target = EndpointAddr::new(pair.server.endpoint().id())
        .with_addrs([TransportAddr::Custom(proxy.custom())]);
    let (client, server) = connect(&mut pair.client, &pair.server, target, BUSINESS).await;
    let old = client.stable_id();
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"before cut").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut before = [0; 10];
    receive.read_exact(&mut before).await.unwrap();
    assert_eq!(client.paths().len(), 1);
    proxy.close().await;
    let started = std::time::Instant::now();
    send.write_all(&vec![0x5a; 65_536]).await.unwrap();
    send.finish().unwrap();
    let reason = tokio::time::timeout(Duration::from_secs(16), client.closed())
        .await
        .expect("actual noq idle/path lifetime ends business");
    assert!(send.stopped().await.is_err());
    let mut bytes = [0; 1];
    assert!(receive.read_exact(&mut bytes).await.is_err());
    assert!(control.close_reason().is_none());
    assert!(control_server.close_reason().is_none());
    let result = pair
        .joining
        .recover_connection_path(&pair.client, &client, contact)
        .await;
    assert!(
        matches!(result, Err(ConnectionPathRecoveryError::Terminal { stable_id }) if stable_id == old)
    );
    let (new, _) = pair.business().await;
    assert_ne!(new.stable_id(), old);
    eprintln!(
        "MIO47_PHYSICAL_TERMINAL old={old} new={} close={reason:?} queued_bytes=65536 control_alive=true elapsed_ms={}",
        new.stable_id(),
        started.elapsed().as_millis()
    );
    pair.close().await;
}

#[tokio::test]
async fn control_paths_reject_an_unknown_original_tls_binding_without_changing_business() {
    use miopunch_native_probe::{
        connection_recovery::CONNECTION_RECOVERY_ALPN,
        mqtt_transport::TRANSPORT_ID,
        resources::{Resource, ResourceUse},
    };
    let broker = std::env::var("MIOPUNCH_MQTT_BROKER").expect("owned MQTT broker required");
    let mut pair = Pair::new(false).await;
    pair.reconfigure(vec![Resource {
        url: format!("mqtt://{broker}"),
        uses: std::collections::BTreeSet::from([ResourceUse::Signaling]),
        limitation: "owned broker for binding rejection".into(),
    }])
    .await;
    pair.server
        .endpoint()
        .set_alpns(vec![BUSINESS.to_vec(), CONNECTION_RECOVERY_ALPN.to_vec()]);
    let (client, server) = pair.business().await;
    let ids = (client.stable_id(), server.stable_id());
    let route = pair
        .server
        .endpoint()
        .custom_addrs()
        .into_iter()
        .find(|a| a.id() == TRANSPORT_ID)
        .unwrap();
    let contact =
        EndpointAddr::new(pair.server.endpoint().id()).with_addrs([TransportAddr::Custom(route)]);
    let (control, control_server) = tokio::time::timeout(Duration::from_secs(10), async {
        let (out, incoming) = tokio::join!(
            pair.client
                .endpoint()
                .connect_control(contact, CONNECTION_RECOVERY_ALPN),
            async {
                pair.server
                    .endpoint()
                    .accept()
                    .await
                    .unwrap()
                    .accept_control()
                    .unwrap()
                    .await
            }
        );
        (out.unwrap(), incoming.unwrap())
    })
    .await
    .unwrap();
    let request = async {
        let exchange = control.open_control_exchange().await.unwrap();
        let request = serde_json::json!({"binding": vec![0u8; 32], "offer": pair.client.addr()});
        exchange
            .send_message(REPAIR_MESSAGE, &serde_json::to_vec(&request).unwrap())
            .await
            .unwrap();
        let result = exchange.recv_message(REPAIR_MESSAGE).await;
        assert!(
            !matches!(result, Ok(ref bytes) if !bytes.is_empty()),
            "invalid binding must not disclose a path offer"
        );
    };
    let serve = async {
        let error = pair
            .admin
            .serve_connection_path_recovery(&pair.server, &control_server)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::NotConnected);
        control_server.close(0u32.into(), b"unknown original binding");
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(request, serve);
    })
    .await
    .unwrap();
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"original still usable").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut bytes = [0; 21];
    receive.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"original still usable");
    assert_eq!((client.stable_id(), server.stable_id()), ids);
    eprintln!(
        "MIO47_BINDING unknown_binding=rejected offered_candidates=0 original_business=usable stable_ids={ids:?}"
    );
    pair.close().await;
}

#[tokio::test]
async fn control_paths_recover_when_the_original_accepting_side_initiates() {
    use miopunch_native_probe::{
        connection_recovery::CONNECTION_RECOVERY_ALPN,
        mqtt_transport::TRANSPORT_ID,
        resources::{Resource, ResourceUse},
    };
    let broker = std::env::var("MIOPUNCH_MQTT_BROKER").expect("owned MQTT broker required");
    let mut pair = Pair::new(false).await;
    pair.reconfigure(vec![Resource {
        url: format!("mqtt://{broker}"),
        uses: std::collections::BTreeSet::from([ResourceUse::Signaling]),
        limitation: "owned broker for accepting-side repair".into(),
    }])
    .await;
    pair.client
        .endpoint()
        .set_alpns(vec![CONNECTION_RECOVERY_ALPN.to_vec()]);
    let route = pair
        .client
        .endpoint()
        .custom_addrs()
        .into_iter()
        .find(|a| a.id() == TRANSPORT_ID)
        .unwrap();
    let contact =
        EndpointAddr::new(pair.client.endpoint().id()).with_addrs([TransportAddr::Custom(route)]);
    let target = tcp_target(&pair.server);
    let TransportAddr::Custom(address) = target.addrs.iter().next().unwrap() else {
        unreachable!()
    };
    let proxy = TcpProxy::new(
        miopunch_native_probe::tcp_transport::TcpTransport::socket_addr(address).unwrap(),
        0,
    )
    .await;
    let target = EndpointAddr::new(pair.server.endpoint().id())
        .with_addrs([TransportAddr::Custom(proxy.custom())]);
    let (client, server) = connect(&mut pair.client, &pair.server, target, BUSINESS).await;
    let ids = (client.stable_id(), server.stable_id());
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"before cut").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut before = [0; 10];
    receive.read_exact(&mut before).await.unwrap();
    assert_eq!(client.paths().len(), 1);
    proxy.close().await;
    let served = async {
        let control = pair
            .client
            .endpoint()
            .accept()
            .await
            .unwrap()
            .accept_control()
            .unwrap()
            .await
            .unwrap();
        pair.joining
            .serve_connection_path_recovery(&pair.client, &control)
            .await
    };
    let (recovered, served) = tokio::join!(
        pair.admin
            .recover_connection_path(&pair.server, &server, contact),
        served
    );
    eprintln!("MIO47_REVERSE_CONTROL_RESULT recovered={recovered:?} served={served:?}");
    let recovered = recovered.unwrap();
    served.unwrap();
    assert_eq!(recovered.stable_id, ids.1);
    chunk(
        &mut send,
        &mut receive,
        b"same stream after accepting-side control coordination",
    )
    .await;
    assert_eq!((client.stable_id(), server.stable_id()), ids);
    eprintln!(
        "MIO47_REVERSE_CONTROL same_stream=true original_stable_ids={ids:?} repaired_by=original_dialing_peer"
    );
    pair.close().await;
}

// This boundary fixture cuts the real TCP listener/circuits while preserving
// the endpoint's advertised address, as happens during an unchanged-IP outage.
#[derive(Debug)]
struct CuttableTcp {
    inner: miopunch_native_probe::tcp_transport::TcpTransport,
    shutdown: std::sync::Mutex<
        Option<std::sync::Arc<dyn iroh::endpoint::transports::CustomEndpointShutdown>>,
    >,
}
impl iroh::endpoint::transports::CustomTransport for CuttableTcp {
    fn bind(&self) -> std::io::Result<Box<dyn iroh::endpoint::transports::CustomEndpoint>> {
        let endpoint = self.inner.bind()?;
        *self.shutdown.lock().unwrap() = endpoint.shutdown_handler();
        Ok(endpoint)
    }
}

#[tokio::test]
async fn control_paths_do_not_report_stale_same_address_as_recovered() {
    use miopunch_native_probe::{
        connection_recovery::{CONNECTION_RECOVERY_ALPN, ConnectionPathRecoveryError},
        mqtt_transport::TRANSPORT_ID,
        resources::{Resource, ResourceUse},
        tcp_transport::{TcpTransport, Traffic},
    };
    let broker = std::env::var("MIOPUNCH_MQTT_BROKER").expect("owned MQTT broker required");
    let mut pair = Pair::new(false).await;
    pair.reconfigure(vec![Resource {
        url: format!("mqtt://{broker}"),
        uses: std::collections::BTreeSet::from([ResourceUse::Signaling]),
        limitation: "owned broker for unchanged-address outage".into(),
    }])
    .await;
    let mut config = pair.admin.resource_configuration().unwrap();
    config.tcp_listen = None;
    pair.admin.save_resource_configuration(&config).unwrap();
    let carrier = std::sync::Arc::new(CuttableTcp {
        inner: TcpTransport::new(Ipv4Addr::LOCALHOST, std::sync::Arc::new(Traffic::default())),
        shutdown: std::sync::Mutex::new(None),
    });
    let new_server = pair
        .admin
        .bind_configured_endpoint_with(
            iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .clear_ip_transports()
                .bind_addr((Ipv4Addr::LOCALHOST, 0))
                .unwrap()
                .add_custom_transport(carrier.clone()),
        )
        .await
        .unwrap();
    std::mem::replace(&mut pair.server, new_server)
        .close()
        .await;
    pair.server
        .endpoint()
        .set_alpns(vec![BUSINESS.to_vec(), CONNECTION_RECOVERY_ALPN.to_vec()]);
    let route = pair
        .server
        .endpoint()
        .custom_addrs()
        .into_iter()
        .find(|a| a.id() == TRANSPORT_ID)
        .unwrap();
    let contact =
        EndpointAddr::new(pair.server.endpoint().id()).with_addrs([TransportAddr::Custom(route)]);
    let (client, server) = pair.business().await;
    let old = client.stable_id();
    let original_address = client.paths().iter().next().unwrap().remote_addr().clone();
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"before outage").await.unwrap();
    let (_, mut receive) = server.accept_bi().await.unwrap();
    let mut before = [0; 13];
    receive.read_exact(&mut before).await.unwrap();
    // First confirm that a living unchanged path really can be acknowledged.
    let serve = async {
        let control = pair
            .server
            .endpoint()
            .accept()
            .await
            .unwrap()
            .accept_control()
            .unwrap()
            .await
            .unwrap();
        pair.admin
            .serve_connection_path_recovery(&pair.server, &control)
            .await
    };
    let (result, served) = tokio::join!(
        pair.joining
            .recover_connection_path(&pair.client, &client, contact.clone()),
        serve
    );
    assert_eq!(result.unwrap().path, original_address);
    served.unwrap();
    chunk(&mut send, &mut receive, b"fresh unchanged path").await;
    let shutdown = carrier.shutdown.lock().unwrap().clone().unwrap();
    shutdown.close().await;
    tokio::time::timeout(Duration::from_secs(1), async {
        while tokio::net::TcpStream::connect(carrier.inner.bound_addr().unwrap())
            .await
            .is_ok()
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("real listener stopped");
    assert!(client.close_reason().is_none());
    assert!(
        client
            .paths()
            .iter()
            .any(|p| p.remote_addr() == &original_address)
    );
    assert!(
        pair.server.addr().addrs.contains(&original_address),
        "unchanged peer offer"
    );
    let serve = async {
        let control = pair
            .server
            .endpoint()
            .accept()
            .await
            .unwrap()
            .accept_control()
            .unwrap()
            .await
            .unwrap();
        assert!(
            matches!(control.paths().iter().next().unwrap().remote_addr(), TransportAddr::Custom(a) if a.id() == TRANSPORT_ID)
        );
        pair.admin
            .serve_connection_path_recovery(&pair.server, &control)
            .await
    };
    let (result, served) = tokio::join!(
        pair.joining
            .recover_connection_path(&pair.client, &client, contact.clone()),
        serve
    );
    eprintln!("MIO47_SAME_ADDRESS result={result:?} served={served:?} control_contacted=true");
    assert!(
        matches!(result, Err(ConnectionPathRecoveryError::Retryable { stable_id, .. }) if stable_id == old),
        "stale listed path must not prove delivery"
    );
    assert!(client.close_reason().is_none());
    send.write_all(b"queued during outage").await.unwrap();
    let reason = tokio::time::timeout(Duration::from_secs(16), client.closed())
        .await
        .expect("actual original lifetime expires");
    let result = pair
        .joining
        .recover_connection_path(&pair.client, &client, contact)
        .await;
    assert!(
        matches!(result, Err(ConnectionPathRecoveryError::Terminal { stable_id }) if stable_id == old)
    );
    assert!(receive.read_exact(&mut [0; 1]).await.is_err());
    eprintln!(
        "MIO47_SAME_ADDRESS_FINAL original={old} close={reason:?} retryable_then_terminal=true false_success=false"
    );
    pair.close().await;
}

#[tokio::test]
async fn a_second_fast_contact_gets_its_attempt_inside_the_original_caller_window() {
    use miopunch_native_probe::connection_recovery::CONNECTION_RECOVERY_ALPN;
    let mut pair = Pair::new(true).await;
    let (client, server) = pair.business().await;
    let ids = (client.stable_id(), server.stable_id());
    let (mut send, mut recv) = client.open_bi().await.unwrap();
    send.write_all(b"before").await.unwrap();
    let (mut reply, mut request) = server.accept_bi().await.unwrap();
    request.read_exact(&mut [0; 6]).await.unwrap();
    let mut contact = tcp_target(&pair.server);
    let TransportAddr::Custom(route) = contact.addrs.first().unwrap() else {
        unreachable!()
    };
    let proxy = TcpProxy::new(
        miopunch_native_probe::tcp_transport::TcpTransport::socket_addr(route).unwrap(),
        0,
    )
    .await;
    contact.addrs.insert(TransportAddr::Custom(proxy.custom()));
    assert_eq!(contact.addrs.len(), 2);
    pair.server
        .endpoint()
        .set_alpns(vec![BUSINESS.to_vec(), CONNECTION_RECOVERY_ALPN.to_vec()]);
    let started = tokio::time::Instant::now();
    let serve = async {
        let first = pair
            .server
            .endpoint()
            .accept()
            .await
            .unwrap()
            .accept_control()
            .unwrap()
            .await
            .unwrap();
        let exchange = first.accept_control_exchange().await.unwrap();
        exchange.recv_message(REPAIR_MESSAGE).await.unwrap();
        tokio::time::timeout(Duration::from_millis(3400), first.closed())
            .await
            .expect("each fast attempt is capped at three seconds");
        assert!(started.elapsed() >= Duration::from_millis(2800));
        let second = pair
            .server
            .endpoint()
            .accept()
            .await
            .unwrap()
            .accept_control()
            .unwrap()
            .await
            .unwrap();
        let exchange = second.accept_control_exchange().await.unwrap();
        exchange.recv_message(REPAIR_MESSAGE).await.unwrap();
        exchange
            .send_message(
                REPAIR_MESSAGE,
                &serde_json::to_vec(&pair.server.addr()).unwrap(),
            )
            .await
            .unwrap();
        second.closed().await;
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(8), async {
        tokio::join!(
            pair.joining.recover_connection_path_until(
                &pair.client,
                &client,
                contact,
                n0_future::time::Instant::now() + Duration::from_secs(7)
            ),
            serve
        )
    })
    .await
    .unwrap();
    assert_eq!(result.unwrap().stable_id, ids.0);
    assert!(
        started.elapsed() >= Duration::from_millis(3000)
            && started.elapsed() < Duration::from_secs(7)
    );
    send.write_all(b"after").await.unwrap();
    let mut bytes = [0; 5];
    request.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"after");
    reply.write_all(b"there").await.unwrap();
    recv.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"there");
    assert_eq!((client.stable_id(), server.stable_id()), ids);
    proxy.close().await;
    pair.close().await;
}

#[tokio::test]
async fn slow_repair_responder_accepts_requester_candidates_after_three_seconds() {
    use miopunch_native_probe::{
        connection_recovery::CONNECTION_RECOVERY_ALPN,
        mqtt_transport::TRANSPORT_ID,
        resources::{Resource, ResourceUse},
    };
    let broker = batch_broker::Broker::new().await;
    let mut pair = Pair::new(true).await;
    pair.reconfigure(vec![Resource {
        url: format!("mqtt://{}", broker.address),
        uses: [ResourceUse::Signaling].into(),
        limitation: "owned delayed requester candidate signaling".into(),
    }])
    .await;
    let (client, server) = pair.business().await;
    let ids = (client.stable_id(), server.stable_id());
    let (mut send, mut recv) = client.open_bi().await.unwrap();
    send.write_all(b"before").await.unwrap();
    let (mut reply, mut request) = server.accept_bi().await.unwrap();
    request.read_exact(&mut [0; 6]).await.unwrap();
    pair.server
        .endpoint()
        .set_alpns(vec![BUSINESS.to_vec(), CONNECTION_RECOVERY_ALPN.to_vec()]);
    let route = pair
        .server
        .endpoint()
        .custom_addrs()
        .into_iter()
        .find(|route| route.id() == TRANSPORT_ID)
        .unwrap();
    let contact =
        EndpointAddr::new(pair.server.endpoint().id()).with_addrs([TransportAddr::Custom(route)]);
    let (out, incoming) = connect_control(
        &pair.client,
        &pair.server,
        contact,
        CONNECTION_RECOVERY_ALPN,
    )
    .await;
    let started = tokio::time::Instant::now();
    let frame = ControlMessageFormat::LengthPrefixed16 { maximum: 4096 };
    let requester = async {
        let header = out.open_control_exchange().await.unwrap();
        let mut binding = [0; 32];
        client
            .export_keying_material(
                &mut binding,
                b"miopunch-existing-connection-path-recovery/1",
                &[],
            )
            .unwrap();
        let initial = EndpointAddr::new(pair.client.endpoint().id())
            .with_addrs((1..=8).map(|port| TransportAddr::Ip(([127, 0, 0, 1], port).into())));
        let wire = serde_json::json!({"binding":binding,"offer":initial,"batched":true,"require_confirmation":true,"budget_ms":7000});
        header
            .send_message(REPAIR_MESSAGE, &serde_json::to_vec(&wire).unwrap())
            .await
            .unwrap();
        let response: serde_json::Value =
            serde_json::from_slice(&header.recv_message(REPAIR_MESSAGE).await.unwrap()).unwrap();
        assert_eq!(response["batched"], true);
        let frames = out.open_control_exchange().await.unwrap();
        // QUIC announces the new stream only when its opener writes. Start
        // the real first candidate batch before awaiting the peer's offer.
        frames
            .send_message(
                frame,
                &serde_json::to_vec(&serde_json::json!({"Candidates":initial})).unwrap(),
            )
            .await
            .unwrap();
        let offered: serde_json::Value =
            serde_json::from_slice(&frames.recv_message(frame).await.unwrap()).unwrap();
        let offered: EndpointAddr = serde_json::from_value(offered["Candidates"].clone()).unwrap();
        let good = TransportAddr::Ip(pair.server.endpoint().bound_sockets()[0]);
        assert!(offered.addrs.contains(&good));
        pair.client
            .endpoint()
            .open_data_transport_path(&client, good.clone())
            .await
            .unwrap();
        let probe = client
            .paths()
            .iter()
            .find(|path| path.remote_addr() == &good)
            .unwrap()
            .start_probe()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while !probe.is_acknowledged().unwrap() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        tokio::time::sleep_until(started + Duration::from_millis(3300)).await;
        let own = EndpointAddr::new(pair.client.endpoint().id())
            .with_addrs([TransportAddr::Ip(pair.client.endpoint().bound_sockets()[0])]);
        frames
            .send_message(
                frame,
                &serde_json::to_vec(&serde_json::json!({"Candidates":own})).unwrap(),
            )
            .await
            .unwrap();
        frames.send_message(frame,&serde_json::to_vec(&serde_json::json!({"Ready":{"path":good,"fresh_ack":true,"probe_watermark":probe.watermark()}})).unwrap()).await.unwrap();
        let ready: serde_json::Value =
            serde_json::from_slice(&frames.recv_message(frame).await.unwrap()).unwrap();
        assert_eq!(ready["Ready"]["fresh_ack"], true);
        frames.send_message(frame, b"\"Committed\"").await.unwrap();
        let committed: serde_json::Value =
            serde_json::from_slice(&frames.recv_message(frame).await.unwrap()).unwrap();
        assert_eq!(committed, "Committed");
        frames.finish_send_confirmed().await.unwrap();
    };
    let ((), served) = tokio::time::timeout(Duration::from_secs(8), async {
        tokio::join!(
            requester,
            pair.admin
                .serve_connection_path_recovery(&pair.server, &incoming)
        )
    })
    .await
    .unwrap();
    served.unwrap();
    assert!(
        started.elapsed() >= Duration::from_millis(3300)
            && started.elapsed() < Duration::from_secs(7)
    );
    send.write_all(b"after").await.unwrap();
    let mut bytes = [0; 5];
    request.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"after");
    reply.write_all(b"there").await.unwrap();
    recv.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"there");
    assert_eq!((client.stable_id(), server.stable_id()), ids);
    out.close(0u32.into(), b"fixture complete");
    incoming.close(0u32.into(), b"fixture complete");
    pair.close().await;
    broker.close().await;
}
