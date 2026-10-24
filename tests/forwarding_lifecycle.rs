//! Real public forwarding futures, OS sockets and cancellation/drop ownership.
use miopunch_native_probe::{
    forwarding::Protocol,
    manual_admission::AdmissionStage,
    native_client::{ClientError, NativeClient},
    service_policy,
    resources::{Resource, ResourceConfig, ResourceTable, ResourceUse},
};
use std::{
    future::Future,
    net::Ipv4Addr,
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::Poll,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinSet,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn configured_guest_resources_preserve_local_allow_list_after_reopen() {
    use miopunch_native_probe::{native_network::NativeNetwork, service_policy::Service};
    let root = PathBuf::from(std::env::var_os("M118_FORWARD_PRIVATE").expect("owned root"));
    let relay = std::env::var("M118_LOCAL_RELAY").expect("fresh owned loopback relay");
    let config = local_resources(relay.clone());
    let target = TcpListener::bind("[::1]:0").await.unwrap();
    let provider = NativeClient::create_with_resources(root.join("provider"), Ipv4Addr::LOCALHOST, config.clone()).await.unwrap();
    let mut guest = None;
    let mut reopened = None;
    let result = async {
        provider.export_recovery_card_to(root.join("card"), None).await?;
        provider.acknowledge_recovery_saved().await?;
        provider.enable_forwarding(Protocol::Tcp, vec![target.local_addr()?]).await?;
        let invitation = provider.issue_service_guest(provider.address(), Service::TcpForward, service_policy::now() + 180).await?;
        guest = Some(NativeClient::join_guest_with_resources(root.join("guest"), &invitation, Ipv4Addr::LOCALHOST, config.clone()).await?);
        let active = guest.as_ref().unwrap().redeem_guest().await?;
        let deadline = active["scope"]["expires"].clone();
        if active["stage"] != "GuestActive" { return Err("configured guest did not redeem".into()); }
        guest.take().unwrap().close().await?;
        let node = NativeNetwork::open(root.join("guest")).await?;
        let saved = node.resource_configuration();
        node.close().await;
        if saved? != config { return Err("saved guest resource permissions changed".into()); }
        reopened = Some(NativeClient::open(root.join("guest"), Ipv4Addr::LOCALHOST).await?);
        let retained = reopened.as_ref().unwrap().redeem_guest().await?;
        let only_local_relays = reopened.as_ref().unwrap().address().addrs.iter().all(|address| match address {
            iroh::TransportAddr::Relay(url) => url.as_str() == relay,
            _ => true,
        });
        if retained["scope"]["expires"] != deadline || !only_local_relays { return Err("reopen changed guest deadline or relay allow-list".into()); }
        eprintln!("configured-guest saved_exact_resource_policy=true reopened_same_deadline=true no_public_relay_hint={only_local_relays}");
        Ok::<_, ClientError>(())
    }.await;
    let mut cleanup_errors = Vec::new();
    if let Some(guest) = guest { if let Err(error) = guest.close().await { cleanup_errors.push(format!("guest: {error}")); } }
    if let Some(reopened) = reopened { if let Err(error) = reopened.close().await { cleanup_errors.push(format!("reopened: {error}")); } }
    if let Err(error) = provider.close().await { cleanup_errors.push(format!("provider: {error}")); }
    drop(target);
    assert!(result.is_ok() && cleanup_errors.is_empty(), "configured guest business={result:?}; cleanup={cleanup_errors:?}");
}

fn local_resources(relay: String) -> ResourceConfig {
    let mut config = ResourceConfig::with_defaults(ResourceTable { revision: 2, resources: vec![] });
    config.replacement = Some(vec![Resource { url: relay, uses: BTreeSet::from([ResourceUse::DataRelay]), limitation: "fresh owned local fixture".into() }]);
    config.disabled_uses = BTreeSet::from([ResourceUse::Observation, ResourceUse::Lookup, ResourceUse::Signaling]);
    config
}

fn check_reopened_node(
    context: &str,
    predicates: &[(&str, bool)],
    cleanup: Result<(), ClientError>,
) -> Result<(), ClientError> {
    if predicates.iter().any(|(_, passed)| !passed) || cleanup.is_err() {
        return Err(format!("{context}: predicates={predicates:?}; reopened_cleanup={cleanup:?}").into());
    }
    Ok(())
}

#[test]
fn reopened_failures_retain_independent_invariants_and_cleanup() {
    // Pure result values only: no NativeClient, runtime, network or OS socket.
    let predicates = [("same_identity", false), ("no_target_effect", false), ("owners_zero", true)];
    let error = check_reopened_node("fixture", &predicates, Err("cleanup-sentinel".into())).unwrap_err().to_string();
    assert!(error.contains("same_identity\", false") && error.contains("no_target_effect\", false") && error.contains("owners_zero\", true"));
    assert!(error.contains("cleanup-sentinel"));
    let business = check_reopened_node("fixture", &predicates, Ok(())).unwrap_err().to_string();
    assert!(business.contains("same_identity") && business.contains("reopened_cleanup=Ok(())"));
    let cleanup = check_reopened_node("fixture", &[("same_identity", true)], Err("cleanup-only".into())).unwrap_err().to_string();
    assert!(cleanup.contains("same_identity\", true") && cleanup.contains("cleanup-only"));
    assert!(check_reopened_node("fixture", &[("same_identity", true), ("no_target_effect", true)], Ok(())).is_ok());
}

struct Pair {
    provider: Option<NativeClient>,
    consumer: Option<Arc<NativeClient>>,
}
impl Pair {
    async fn create(root: &std::path::Path) -> Result<Self, ClientError> {
        let config = local_resources(std::env::var("M118_LOCAL_RELAY").expect("fresh owned loopback relay"));
        let mut pair = Self {
            provider: Some(
                NativeClient::create_with_resources(root.join("provider"), Ipv4Addr::LOCALHOST, config.clone()).await?,
            ),
            consumer: None,
        };
        let setup = async {
            let provider = pair.provider.as_ref().unwrap();
            provider
                .export_recovery_card_to(root.join("card"), None)
                .await?;
            provider.acknowledge_recovery_saved().await?;
            let invitation = provider
                .issue_precode(service_policy::now() + 600, Some(1))
                .await?;
            pair.consumer = Some(Arc::new(
                NativeClient::join_with_resources(root.join("consumer"), &invitation, Ipv4Addr::LOCALHOST, config)
                    .await?,
            ));
            if pair
                .consumer
                .as_ref()
                .unwrap()
                .request_admission(&invitation)
                .await?
                != AdmissionStage::Complete
            {
                return Err("real admission incomplete".into());
            }
            provider
                .sync_services(pair.consumer.as_ref().unwrap().address())
                .await?;
            Ok::<_, ClientError>(())
        }
        .await;
        if let Err(primary) = setup {
            let cleanup = pair.close().await;
            return Err(format!("pair setup={primary}; cleanup={cleanup:?}").into());
        }
        Ok(pair)
    }
    async fn close(&mut self) -> Result<(), ClientError> {
        let mut failures = Vec::new();
        if let Some(consumer) = self.consumer.take() {
            match Arc::try_unwrap(consumer) {
                Ok(consumer) => if let Err(error) = consumer.close().await { failures.push(format!("consumer: {error}")); },
                Err(_) => failures.push("consumer owner still shared".to_owned()),
            }
        }
        if let Some(provider) = self.provider.take() {
            if let Err(error) = provider.close().await { failures.push(format!("provider: {error}")); }
        }
        if !failures.is_empty() {
            return Err(format!("owned SDK cleanup: {failures:?}").into());
        }
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_pending_forward_can_cancel_drop_and_reopen_without_late_effect() {
    let root = PathBuf::from(std::env::var_os("M118_FORWARD_PRIVATE").expect("owned private root"));
    let mut pair = Pair::create(&root)
        .await
        .expect("real fresh admitted SDK pair");
    let listener = TcpListener::bind("[::1]:0").await.unwrap();
    let target = listener.local_addr().unwrap();
    let bytes_seen = Arc::new(AtomicUsize::new(0));
    let (finish, mut stop) = tokio::sync::oneshot::channel();
    let server_bytes = bytes_seen.clone();
    let server = tokio::spawn(async move {
        let mut handlers = JoinSet::new();
        loop {
            tokio::select! {
                biased;
                _ = &mut stop => break,
                joined = handlers.join_next(), if !handlers.is_empty() => {
                    if !matches!(joined, Some(Ok(Ok(())))) { return Err("owned target handler failed"); }
                },
                accepted = listener.accept(), if handlers.len() < 16 => {
                    let (mut socket, _) = accepted.map_err(|_| "owned target accept failed")?;
                    let seen = server_bytes.clone();
                    handlers.spawn(async move {
                        let mut bytes = [0u8; 1024];
                        loop {
                            let count = socket.read(&mut bytes).await.map_err(|_| "owned target read failed")?;
                            if count == 0 { return Ok::<_, &'static str>(()); }
                            seen.fetch_add(count, Ordering::SeqCst);
                            socket.write_all(&bytes[..count]).await.map_err(|_| "owned target write failed")?;
                        }
                    });
                }
            }
        }
        handlers.abort_all();
        while handlers.join_next().await.is_some() {}
        Ok(())
    });
    let result = async {
        let provider = pair.provider.as_ref().unwrap();
        let consumer = pair.consumer.as_ref().unwrap().clone();
        provider.enable_forwarding(Protocol::Tcp, vec![target]).await?;
        let address = provider.address();
        let (notice, observed) = tokio::sync::oneshot::channel();
        let starting = {
            let consumer = consumer.clone();
            let address = address.clone();
            tokio::spawn(async move {
                let future = consumer.forward_exchange(address, Protocol::Tcp, target, b"pending-forward", 15);
                tokio::pin!(future);
                let mut notice = Some(notice);
                std::future::poll_fn(|context| {
                    let result = future.as_mut().poll(context);
                    if result.is_pending() { if let Some(notice) = notice.take() { let _ = notice.send(()); } }
                    result
                }).await
            })
        };
        let actual_pending = observed.await.is_ok() && !starting.is_finished();
        let status = consumer.forwarding_status();
        let flow = status["flows"].as_array().and_then(|flows| flows.first()).and_then(|flow| flow["id"].as_str()).map(str::to_owned);
        let cancellation = if let Some(flow) = flow { consumer.cancel_forwarding_flow(&flow).await } else { Err("pending flow not observable".into()) };
        let first = starting.await.map_err(|_| "public exchange task failed")?;
        let cancelled_cleanly = cancellation.is_ok() && first.is_err() && consumer.forwarding_status()["active_flows"] == 0 && bytes_seen.load(Ordering::SeqCst) == 0;
        eprintln!("forward-cancel actual_future_pending={actual_pending} joined_no_late_target_effect={cancelled_cleanly}");
        if !actual_pending || !cancelled_cleanly { return Err("pending public cancellation invariant failed".into()); }
        // Cancellation uses the SDK's existing per-peer contact backoff. Wait
        // its known first/second budget before the next distinct operation.
        tokio::time::sleep(Duration::from_millis(2050)).await;
        let mut future = Box::pin(consumer.forward_exchange(address.clone(), Protocol::Tcp, target, b"dropped-forward", 15));
        let pending_drop = std::future::poll_fn(|context| Poll::Ready(future.as_mut().poll(context).is_pending())).await;
        let owned_before_drop = consumer.forwarding_status()["active_flows"] == 1;
        drop(future);
        let dropped_cleanly = consumer.forwarding_status()["active_flows"] == 0 && bytes_seen.load(Ordering::SeqCst) == 0;
        eprintln!("forward-future-drop actual_future_pending={pending_drop} owned_before_drop={owned_before_drop} no_late_effect={dropped_cleanly}");
        if !pending_drop || !owned_before_drop || !dropped_cleanly { return Err("dropped pending public operation retained an owner".into()); }
        tokio::time::sleep(Duration::from_millis(2050)).await;
        let reply = consumer.forward_exchange(address.clone(), Protocol::Tcp, target, b"after-cancel", 12).await?;
        if reply != b"after-cancel" { return Err("same normal operation did not recover".into()); }
        eprintln!("forward-after-cancel same_operation_exact=true target_bytes={}", bytes_seen.load(Ordering::SeqCst));
        // Exercise dropping the host while a healthy listener flow has an
        // actual Pending application read. The provider stays online/enabled.
        drop(consumer);
        let consumer = pair.consumer.as_ref().unwrap();
        let identity = consumer.status().await.node.node_id;
        let forwarding = consumer.start_forwarding(address, Protocol::Tcp, target, "[::1]:0".parse()?).await?;
        let local: std::net::SocketAddr = forwarding["listen"].as_str().ok_or("missing listen address")?.parse()?;
        let mut app = TcpStream::connect(local).await?;
        app.write_all(b"owned-pending-listener").await?;
        let mut echo = [0u8; 22];
        tokio::time::timeout(Duration::from_secs(75), app.read_exact(&mut echo)).await??;
        if &echo != b"owned-pending-listener" { return Err("active listener precondition failed".into()); }
        let pending_state = consumer.forwarding_status();
        let transport_ready = !pending_state["flows"][0]["transport"].is_null();
        let mut end = [0u8; 1];
        let mut reading = Box::pin(app.read(&mut end));
        let read_pending = std::future::poll_fn(|context| Poll::Ready(reading.as_mut().poll(context).is_pending())).await;
        drop(reading);
        let before = bytes_seen.load(Ordering::SeqCst);
        eprintln!("forward-host-drop-before transport_ready={transport_ready} actual_read_pending={read_pending} target_bytes={before}");
        eprintln!("forward-host-owner strong_before_drop={}", Arc::strong_count(consumer));
        let weak = Arc::downgrade(consumer);
        drop(pair.consumer.take());
        eprintln!("forward-host-owner strong_after_drop={} provider={}", weak.strong_count(), provider.forwarding_status());
        let closed = match tokio::time::timeout(Duration::from_secs(5), app.read(&mut end)).await.map_err(|_| "host drop local application socket remained open")? {
            Ok(0) => true,
            Err(failure) if matches!(failure.kind(), std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted | std::io::ErrorKind::BrokenPipe) => true,
            Err(failure) => return Err(failure.into()),
            Ok(_) => false,
        };
        drop(app);
        eprintln!("forward-host-local-closed provider={}", provider.forwarding_status());
        tokio::time::timeout(Duration::from_secs(5), async {
            while provider.forwarding_status()["active_flows"] != 0 { tokio::task::yield_now().await; }
        }).await.map_err(|_| "host drop provider flow remained active")?;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let reopened = NativeClient::open(root.join("consumer"), Ipv4Addr::UNSPECIFIED).await?;
        let same_identity = reopened.status().await.node.node_id == identity;
        let reopened_status = reopened.forwarding_status();
        let flows_zero = reopened_status["active_flows"] == 0;
        let no_listeners = reopened_status["listeners"].as_array().is_some_and(Vec::is_empty);
        let reopening_cleanup = reopened.close().await;
        let drop_no_effect = bytes_seen.load(Ordering::SeqCst) == before;
        let retained = same_identity && flows_zero && no_listeners;
        eprintln!("forward-host-drop transport_ready={transport_ready} actual_read_pending={read_pending} local_socket_closed={closed} retained_identity_no_listener={retained} no_target_effect={drop_no_effect}");
        check_reopened_node("consumer host drop", &[("transport_ready", transport_ready), ("actual_read_pending", read_pending),
            ("local_socket_closed", closed), ("same_identity", same_identity), ("flows_zero", flows_zero),
            ("no_listeners", no_listeners), ("no_target_effect", drop_no_effect)], reopening_cleanup)?;
        Ok::<_, ClientError>(())
    }.await;
    // Preserve the primary result, then attempt every other owned cleanup.
    let sdk_cleanup = pair.close().await;
    let _ = finish.send(());
    let target_cleanup = server.await;
    assert!(
        result.is_ok() && sdk_cleanup.is_ok() && matches!(target_cleanup, Ok(Ok(()))),
        "public lifecycle business={result:?}; sdk_cleanup={sdk_cleanup:?}; target_cleanup={target_cleanup:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_drop_at_capacity_closes_both_sockets_without_late_effect() {
    let root = PathBuf::from(std::env::var_os("M118_FORWARD_PRIVATE").expect("owned private root"));
    let mut pair = Pair::create(&root).await.expect("real fresh admitted SDK pair");
    let target = TcpListener::bind("[::1]:0").await.unwrap();
    let address = target.local_addr().unwrap();
    let bytes = Arc::new(AtomicUsize::new(0));
    let target_closed = Arc::new(AtomicUsize::new(0));
    let seen = bytes.clone();
    let closed = target_closed.clone();
    let (finish, mut stop) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut handlers = JoinSet::new();
        let mut failed = false;
        loop {
            tokio::select! {
                biased;
                _ = &mut stop => break,
                result = handlers.join_next(), if !handlers.is_empty() => {
                    failed |= !matches!(result, Some(Ok(Ok(()))));
                },
                accepted = target.accept(), if handlers.len() < 16 => {
                    let (mut socket, _) = accepted.map_err(|_| "target accept failed")?;
                    let seen = seen.clone();
                    let closed = closed.clone();
                    handlers.spawn(async move {
                        let mut data = [0u8; 1];
                        socket.read_exact(&mut data).await.map_err(|_| "target initial read failed")?;
                        seen.fetch_add(1, Ordering::SeqCst);
                        socket.write_all(&data).await.map_err(|_| "target initial echo failed")?;
                        match socket.read(&mut data).await {
                            Ok(0) => {},
                            Err(error) if matches!(error.kind(), std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted) => {},
                            _ => return Err("target received a late byte or unexpected closure"),
                        }
                        closed.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, &'static str>(())
                    });
                }
            }
        }
        handlers.abort_all();
        while let Some(result) = handlers.join_next().await {
            if !matches!(result, Ok(Ok(()))) && !matches!(result, Err(ref error) if error.is_cancelled()) { failed = true; }
        }
        if failed { return Err("target handler failed"); }
        Ok::<_, &'static str>(())
    });
    let result = async {
        let provider = pair.provider.as_ref().unwrap();
        let identity = provider.status().await.node.node_id;
        let consumer = pair.consumer.as_ref().unwrap();
        provider.enable_forwarding(Protocol::Tcp, vec![address]).await?;
        let listener = consumer.start_forwarding(provider.address(), Protocol::Tcp, address, "[::1]:0".parse()?).await?;
        let local: std::net::SocketAddr = listener["listen"].as_str().ok_or("missing local listener")?.parse()?;
        let mut applications = Vec::new();
        for index in 0..16u8 {
            let mut app = TcpStream::connect(local).await?;
            app.write_all(&[index]).await?;
            let mut echo = [0u8; 1];
            tokio::time::timeout(Duration::from_secs(75), app.read_exact(&mut echo)).await??;
            if echo != [index] { return Err("capacity application echo changed".into()); }
            let mut end = [0u8; 1];
            let mut reading = Box::pin(app.read(&mut end));
            let pending = std::future::poll_fn(|context| Poll::Ready(reading.as_mut().poll(context).is_pending())).await;
            drop(reading);
            if !pending { return Err("capacity app was not healthy and Pending".into()); }
            applications.push(app);
        }
        let status = provider.forwarding_status();
        if status["active_flows"] != 16 || status["provider_workers"] != 16 || bytes.load(Ordering::SeqCst) != 16 { return Err("full provider receiver premise failed".into()); }
        eprintln!("provider-drop full_real_pending=16 active_flows={} provider_workers={}", status["active_flows"], status["provider_workers"]);
        drop(pair.provider.take());
        let drop_started = std::time::Instant::now();
        let mut application_close = Box::pin(async {
            for app in &mut applications {
                let mut end = [0u8; 1];
                match app.read(&mut end).await {
                    Ok(0) => {},
                    Err(error) if matches!(error.kind(), std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted) => {},
                    _ => return Err::<_, ClientError>("provider drop left a live or nonempty application".into()),
                }
            }
            Ok(())
        });
        match tokio::time::timeout(Duration::from_secs(5), &mut application_close).await {
            Ok(result) => result?,
            Err(_) => {
                eprintln!("provider-drop-at5 same_sockets_retained=true target_closed={} target_bytes={} consumer={}", target_closed.load(Ordering::SeqCst), bytes.load(Ordering::SeqCst), consumer.forwarding_status());
                let late = tokio::time::timeout(Duration::from_secs(65), &mut application_close).await;
                eprintln!("provider-drop-late elapsed={} same_operation={late:?} target_closed={} consumer={}", drop_started.elapsed().as_secs_f64(), target_closed.load(Ordering::SeqCst), consumer.forwarding_status());
                return Err("provider drop application close phase timed out at original five-second bound".into());
            }
        }
        drop(application_close);
        drop(applications);
        tokio::time::timeout(Duration::from_secs(5), async {
            while consumer.forwarding_status()["active_flows"] != 0 || target_closed.load(Ordering::SeqCst) != 16 { tokio::task::yield_now().await; }
        }).await.map_err(|_| "provider drop target/consumer phase timed out")?;
        consumer.stop_forwarding(listener["id"].as_str().ok_or("missing listener id")?).await?;
        let reopened = NativeClient::open(root.join("provider"), Ipv4Addr::LOCALHOST).await?;
        let same_identity = reopened.status().await.node.node_id == identity;
        let reopened_status = reopened.forwarding_status();
        let flows_zero = reopened_status["active_flows"] == 0;
        let workers_zero = reopened_status["provider_workers"] == 0;
        let reopening_cleanup = reopened.close().await;
        check_reopened_node("provider host drop", &[("same_identity", same_identity), ("flows_zero", flows_zero),
            ("workers_zero", workers_zero), ("no_target_effect", bytes.load(Ordering::SeqCst) == 16)], reopening_cleanup)?;
        eprintln!("provider-drop applications_closed=16 targets_closed=16 consumer_flows=0 reopened_same_identity=true reopened_workers=0 target_bytes=16");
        Ok::<_, ClientError>(())
    }.await;
    let sdk_cleanup = pair.close().await;
    let _ = finish.send(());
    let target_cleanup = server.await;
    assert!(result.is_ok() && sdk_cleanup.is_ok() && matches!(target_cleanup, Ok(Ok(()))),
        "capacity drop business={result:?}; sdk_cleanup={sdk_cleanup:?}; target_cleanup={target_cleanup:?}");
}
