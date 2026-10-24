//! Public SDK Web cancellation, close, drop and saved-identity reopen.
//! Targets, state, bytes and local relay are created solely for this Web test.
use miopunch_native_probe::{
    manual_admission::AdmissionStage,
    native_client::{ClientError, NativeClient},
    resources::ResourceConfig,
    service_policy,
    web::WebRequest,
};
use std::{
    net::Ipv4Addr,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
    task::{JoinHandle, JoinSet},
};

const BODY: &[u8] = b"owned M119 SDK Web target";

struct Target {
    url: String,
    pending: Arc<AtomicUsize>,
    closed: Arc<AtomicUsize>,
    stop: watch::Sender<bool>,
    task: JoinHandle<Result<(), ClientError>>,
}
impl Target {
    async fn start() -> Result<Self, ClientError> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/", listener.local_addr()?);
        let pending = Arc::new(AtomicUsize::new(0));
        let closed = Arc::new(AtomicUsize::new(0));
        let (stop, mut stopped) = watch::channel(false);
        let (started, ended) = (pending.clone(), closed.clone());
        let task = tokio::spawn(async move {
            let mut children = JoinSet::new();
            let result = loop {
                tokio::select! {
                    _ = stopped.changed() => break Ok::<_, ClientError>(()),
                    accepted = listener.accept() => {
                        let (mut stream, _) = match accepted { Ok(accepted) => accepted, Err(error) => break Err(error.into()) };
                        if children.len() >= 8 { break Err("owned HTTP target capacity exceeded".into()); }
                        let (started, ended, mut child_stop) = (started.clone(), ended.clone(), stopped.clone());
                        children.spawn(async move {
                            let work = async {
                                let mut head = Vec::new();
                                while !head.ends_with(b"\r\n\r\n") {
                                    if head.len() >= 8192 { return Err("owned HTTP header bound".into()); }
                                    let mut byte = [0];
                                    stream.read_exact(&mut byte).await?;
                                    head.push(byte[0]);
                                }
                                if head.starts_with(b"GET /pending HTTP/1.1\r\n") {
                                    started.fetch_add(1, Ordering::SeqCst);
                                    let mut byte = [0];
                                    match stream.read(&mut byte).await {
                                        Ok(0) | Err(_) => { ended.fetch_add(1, Ordering::SeqCst); }
                                        Ok(_) => return Err("unexpected pending HTTP body".into()),
                                    }
                                } else if head.starts_with(b"GET / HTTP/1.1\r\n") {
                                    stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", BODY.len()).as_bytes()).await?;
                                    stream.write_all(BODY).await?;
                                    stream.shutdown().await?;
                                } else { return Err("unexpected owned HTTP request".into()); }
                                Ok::<_, ClientError>(())
                            };
                            tokio::select! { _ = child_stop.changed() => Ok(()), result = work => result }
                        });
                    },
                    result = children.join_next(), if !children.is_empty() => {
                        match result { Some(Ok(Ok(()))) => {}, Some(Ok(Err(error))) => break Err(error),
                            Some(Err(error)) => break Err(error.into()), None => {} }
                    },
                }
            };
            let mut cleanup = Vec::new();
            // Stop is an owned channel, never a process or host signal.
            while let Some(result) = children.join_next().await {
                if !matches!(result, Ok(Ok(()))) {
                    cleanup.push(format!("{result:?}"));
                }
            }
            if result.is_err() || !cleanup.is_empty() {
                return Err(format!("target primary={result:?}; cleanup={cleanup:?}").into());
            }
            Ok(())
        });
        Ok(Self {
            url,
            pending,
            closed,
            stop,
            task,
        })
    }
    async fn wait_count(counter: &AtomicUsize, expected: usize) -> Result<(), ClientError> {
        tokio::time::timeout(Duration::from_secs(5), async {
            while counter.load(Ordering::SeqCst) != expected {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        Ok(())
    }
    async fn close(self) -> Result<(), ClientError> {
        let _ = self.stop.send(true);
        tokio::time::timeout(Duration::from_secs(5), self.task).await???;
        Ok(())
    }
}

struct Pair {
    provider: Option<NativeClient>,
    consumer: Option<NativeClient>,
}
impl Pair {
    async fn create(root: &Path) -> Result<Self, ClientError> {
        let relay = std::env::var("M119_WEB_LOCAL_RELAY")?;
        let config: ResourceConfig = serde_json::from_value(serde_json::json!({
            "defaults":{"revision":2,"resources":[]},"replacement":[{"url":relay,"uses":["DataRelay"],"limitation":"fresh owned Web fixture"}],
            "disabled":[],"disabled_uses":["Observation","Lookup","Signaling"],"lan_discovery":"Disabled","port_mapping":"Disabled"
        }))?;
        let mut pair = Self {
            provider: Some(
                NativeClient::create_with_resources(
                    root.join("provider"),
                    Ipv4Addr::LOCALHOST,
                    config.clone(),
                )
                .await?,
            ),
            consumer: None,
        };
        let result = async {
            let provider = pair.provider.as_ref().unwrap();
            provider
                .export_recovery_card_to(root.join("card"), None)
                .await?;
            provider.acknowledge_recovery_saved().await?;
            let invitation = provider
                .issue_precode(service_policy::now() + 300, Some(1))
                .await?;
            pair.consumer = Some(
                NativeClient::join_with_resources(
                    root.join("consumer"),
                    &invitation,
                    Ipv4Addr::LOCALHOST,
                    config,
                )
                .await?,
            );
            if pair
                .consumer
                .as_ref()
                .unwrap()
                .request_admission(&invitation)
                .await?
                != AdmissionStage::Complete
            {
                return Err("real owned admission incomplete".into());
            }
            provider
                .sync_services(pair.consumer.as_ref().unwrap().address())
                .await?;
            Ok::<_, ClientError>(())
        }
        .await;
        if let Err(primary) = result {
            let cleanup = pair.close().await;
            return Err(format!("pair primary={primary}; cleanup={cleanup:?}").into());
        }
        Ok(pair)
    }
    async fn close(&mut self) -> Result<(), ClientError> {
        let mut errors = Vec::new();
        if let Some(consumer) = self.consumer.take() {
            if let Err(error) = consumer.close().await {
                errors.push(format!("consumer:{error}"));
            }
        }
        if let Some(provider) = self.provider.take() {
            if let Err(error) = provider.close().await {
                errors.push(format!("provider:{error}"));
            }
        }
        if !errors.is_empty() {
            return Err(format!("SDK cleanup:{errors:?}").into());
        }
        Ok(())
    }
}
fn request(url: String) -> WebRequest {
    WebRequest {
        url,
        method: "GET".into(),
        headers: vec![],
        body: vec![],
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owned_web_future_cancel_close_drop_and_reopen_are_bounded() {
    let root =
        std::path::PathBuf::from(std::env::var_os("M119_WEB_PRIVATE").expect("owned private root"));
    let target = Target::start().await.expect("new owned HTTP target");
    let mut pair = Pair::create(&root).await.expect("actual admitted SDK pair");
    let result = async {
        // Independent socket prerequisite before any SDK business request.
        let mut direct = TcpStream::connect(target.url.trim_start_matches("http://").trim_end_matches('/')).await?;
        direct.write_all(b"GET / HTTP/1.1\r\nHost: owned\r\nConnection: close\r\n\r\n").await?;
        let mut received = Vec::new(); direct.read_to_end(&mut received).await?;
        if !received.ends_with(BODY) { return Err("direct target literal mismatch".into()); }
        let provider = pair.provider.as_ref().unwrap();
        provider.enable_web(vec![target.url.clone()], vec![]).await?;
        let consumer = pair.consumer.as_ref().unwrap();
        if consumer.request_web(provider.address(), request(target.url.clone())).await?.body != BODY { return Err("public SDK literal mismatch".into()); }
        let mut pending = Box::pin(consumer.request_web(provider.address(), request(target.url.clone()+"pending")));
        tokio::select! {
            result = &mut pending => return Err(format!("pending request ended before cancellation; success={}", result.is_ok()).into()),
            result = Target::wait_count(&target.pending, 1) => result?,
        }
        drop(pending);
        Target::wait_count(&target.closed, 1).await?;
        if consumer.web_status()["active_flows"] != 0 { return Err("dropped public future retained consumer owner".into()); }
        let _job = consumer.begin_web_request(provider.address(), request(target.url.clone()+"pending")).await?;
        Target::wait_count(&target.pending, 2).await?;
        let identity = consumer.address().id;
        tokio::time::timeout(Duration::from_secs(5), pair.consumer.take().unwrap().close()).await??;
        Target::wait_count(&target.closed, 2).await?;
        pair.consumer = Some(NativeClient::open(root.join("consumer"), Ipv4Addr::LOCALHOST).await?);
        let consumer = pair.consumer.as_ref().unwrap();
        if consumer.address().id != identity || consumer.web_status()["active_flows"] != 0 || consumer.web_status()["request_handles"] != 0 {
            return Err("saved identity reopen retained late Web owners".into());
        }
        if consumer.request_web(provider.address(), request(target.url.clone())).await?.body != BODY { return Err("explicit fresh Web operation after reopen failed".into()); }
        consumer.begin_web_request(provider.address(), request(target.url.clone()+"pending")).await?;
        Target::wait_count(&target.pending, 3).await?;
        drop(pair.consumer.take().unwrap());
        Target::wait_count(&target.closed, 3).await?;
        pair.consumer = Some(NativeClient::open(root.join("consumer"), Ipv4Addr::LOCALHOST).await?);
        let consumer = pair.consumer.as_ref().unwrap();
        let job = consumer.begin_web_request(provider.address(), request(target.url.clone()+"pending")).await?;
        Target::wait_count(&target.pending, 4).await?;
        drop(pair.provider.take().unwrap());
        Target::wait_count(&target.closed, 4).await?;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match consumer.web_request_result(job["id"].as_str().unwrap()) {
                    Ok(value) if value["pending"] == true => tokio::time::sleep(Duration::from_millis(10)).await,
                    Err(_) => return Ok::<_, ClientError>(()),
                    Ok(_) => return Err("provider drop returned false success".into()),
                }
            }
        }).await??;
        eprintln!("owned Web future cancel, close, consumer/provider drop: target EOF observed; saved identity reopen exact; explicit new request succeeds; no false pending success");
        Ok::<_, ClientError>(())
    }.await;
    let sdk_cleanup = pair.close().await;
    let target_cleanup = target.close().await;
    assert!(
        result.is_ok() && sdk_cleanup.is_ok() && target_cleanup.is_ok(),
        "Web primary={result:?}; SDK cleanup={sdk_cleanup:?}; target cleanup={target_cleanup:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owned_web_socket_explicit_close_covers_outgoing_enqueue() {
    use miopunch_native_probe::web::WebSocketMessage;
    let root =
        std::path::PathBuf::from(std::env::var_os("M119_WEB_PRIVATE").expect("owned private root"));
    let target =
        std::env::var("M119_WEB_PRESSURE_TARGET").expect("fresh independently observed target");
    let release = std::path::PathBuf::from(
        std::env::var_os("M119_WEB_PRESSURE_RELEASE").expect("owned target release"),
    );
    let mut pair = Pair::create(&root).await.expect("actual admitted SDK pair");
    let result = async {
        let provider = pair.provider.as_ref().unwrap();
        let consumer = pair.consumer.as_ref().unwrap();
        provider.enable_web(vec![target.clone()], vec![]).await?;
        let socket = consumer.open_web_socket(provider.address(), target.replacen("http", "ws", 1)+"socket?pressure=1", vec![]).await?;
        let id = socket["id"].as_str().ok_or("missing public socket ID")?;
        let greeting = consumer.read_web_socket(id, Duration::from_secs(2)).await?;
        if greeting["message"]["text"] != "owned M119 ready" { return Err("pressure target greeting absent".into()); }
        // Bounded producer: at most 32MiB, normal 32KiB Web frames, no reply.
        // A genuinely Pending public send proves its outgoing queue cannot
        // accept another message; no private queue inspection or API injection.
        for index in 0..1024 {
            let message = WebSocketMessage::Binary {data:vec![0x5a;32768]};
            let mut send = Box::pin(consumer.send_web_socket(id, message));
            tokio::select! {
                sent = &mut send => { sent?; },
                _ = tokio::time::sleep(Duration::from_millis(1000)) => {
                    if consumer.web_status()["active_flows"] != 1 || provider.web_status()["active_flows"] != 1 {
                        return Err("pressure lost the live provider/consumer owner".into());
                    }
                    eprintln!("outgoing_pressure_proved accepted_frames={index} frame_bytes=32768 pending_send_ms=1000 provider_enabled=true");
                    let started = std::time::Instant::now();
                    let close = tokio::time::timeout(Duration::from_millis(6250), consumer.close_web_socket(id)).await;
                    let elapsed = started.elapsed();
                    eprintln!("explicit_close_observation seconds={} observer_timeout={}",elapsed.as_secs_f64(),close.is_err());
                    drop(send);
                    match close {
                        Err(_) => return Err("explicit WebSocket close exceeded total 6.25s bound while outgoing send Pending".into()),
                        Ok(Ok(value)) if value["failed"] == false => return Err("unread target falsely reported normal close".into()),
                        Ok(_) => {},
                    }
                    if elapsed > Duration::from_millis(6250) || consumer.web_status()["active_flows"] != 0 || consumer.web_status()["socket_handles"] != 0 {
                        return Err("explicit close returned before exact consumer owner/handle settled".into());
                    }
                    eprintln!("explicit_close_bounded seconds={} failed_truthfully=true consumer_owners=0 handles=0",elapsed.as_secs_f64());
                    return Ok::<_,ClientError>(());
                },
            }
        }
        Err("bounded producer did not establish outgoing queue pressure".into())
    }.await;
    // Release after classifying the close observation, independently of that
    // primary result and SDK cleanup. A fixture I/O error cannot replace it.
    let release_result = std::fs::write(&release, b"owned scenario cleanup");
    let cleanup = pair.close().await;
    assert!(
        result.is_ok() && cleanup.is_ok() && release_result.is_ok(),
        "Web socket primary={result:?}; SDK cleanup={cleanup:?}; target release={release_result:?}"
    );
}
