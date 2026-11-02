//! Public Native terminal future/host ownership, limits and real backpressure.
//! All commands, directories, tmux sessions and process observations are owned.
use miopunch_native_probe::{
    manual_admission::AdmissionStage,
    native_client::{ClientError, NativeClient},
    resources::ResourceConfig,
    service_policy,
    terminal::{MAX_BYTES, TerminalInput, TerminalProvider, TerminalRequest},
};
use std::{
    net::Ipv4Addr,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::time::Instant;

struct Pair {
    root: PathBuf,
    provider: Option<NativeClient>,
    consumer: Option<NativeClient>,
}
impl Pair {
    async fn start(name: &str) -> Result<Self, ClientError> {
        let root =
            PathBuf::from(std::env::var_os("M120_TERMINAL_PRIVATE").ok_or("owned root required")?)
                .join(name);
        std::fs::create_dir(&root)?;
        let relay = std::env::var("M120_TERMINAL_LOCAL_RELAY")?;
        let configuration: ResourceConfig = serde_json::from_value(serde_json::json!({
            "defaults":{"revision":2,"resources":[]},"replacement":[{"url":relay,"uses":["DataRelay"],"limitation":"fresh owned terminal SDK fixture"}],
            "disabled":[],"disabled_uses":["Observation","Lookup","Signaling"],"lan_discovery":"Disabled","port_mapping":"Disabled"
        }))?;
        let mut pair = Self {
            root,
            provider: None,
            consumer: None,
        };
        let result = async {
            pair.provider = Some(
                NativeClient::create_with_resources(
                    pair.root.join("provider"),
                    Ipv4Addr::LOCALHOST,
                    configuration.clone(),
                )
                .await?,
            );
            let provider = pair.provider.as_ref().unwrap();
            provider
                .export_recovery_card_to(pair.root.join("card"), None)
                .await?;
            provider.acknowledge_recovery_saved().await?;
            let invitation = provider
                .issue_precode(service_policy::now() + 300, Some(1))
                .await?;
            pair.consumer = Some(
                NativeClient::join_with_resources(
                    pair.root.join("consumer"),
                    &invitation,
                    Ipv4Addr::LOCALHOST,
                    configuration,
                )
                .await?,
            );
            let consumer = pair.consumer.as_ref().unwrap();
            if consumer.request_admission(&invitation).await? != AdmissionStage::Complete {
                return Err("real terminal admission incomplete".into());
            }
            provider.sync_services(consumer.address()).await?;
            if consumer.address().id == provider.address().id
                || consumer.ping(provider.address()).await?["reply"] != "PONG"
            {
                return Err("distinct real member premise failed".into());
            }
            provider
                .enable_terminal(TerminalProvider {
                    directory: pair
                        .root
                        .join("terminal")
                        .to_str()
                        .ok_or("owned path encoding")?
                        .into(),
                    distribution: None,
                })
                .await?;
            Ok::<_, ClientError>(())
        }
        .await;
        if let Err(primary) = result {
            let cleanup = pair.close().await;
            return Err(format!("pair primary={primary}; independent cleanup={cleanup:?}").into());
        }
        Ok(pair)
    }
    fn provider(&self) -> &NativeClient {
        self.provider.as_ref().unwrap()
    }
    fn consumer(&self) -> &NativeClient {
        self.consumer.as_ref().unwrap()
    }
    async fn create(&self) -> Result<String, ClientError> {
        Ok(self
            .consumer()
            .request_terminal(self.provider().address(), TerminalRequest::Create)
            .await?["workspace"]
            .as_str()
            .ok_or("workspace missing")?
            .into())
    }
    async fn attach(&self, workspace: &str) -> Result<String, ClientError> {
        let id = self
            .consumer()
            .attach_terminal(self.provider().address(), workspace.into(), 80, 24)
            .await?["id"]
            .as_str()
            .ok_or("attachment missing")?
            .to_owned();
        send(
            self.consumer(),
            &id,
            "stty -echo; printf 'SDK_'; printf 'READY\\n'\n",
        )
        .await?;
        read_until(self.consumer(), &id, b"SDK_READY").await?;
        Ok(id)
    }
    async fn end(&self, workspace: &str) -> Result<(), ClientError> {
        let response = self
            .consumer()
            .request_terminal(
                self.provider().address(),
                TerminalRequest::End {
                    workspace: workspace.into(),
                },
            )
            .await?;
        if response["ended"] != true {
            return Err("public explicit end not acknowledged".into());
        }
        Ok(())
    }
    fn file(&self, workspace: &str, filename: &str) -> PathBuf {
        self.root.join("terminal").join(workspace).join(filename)
    }
    async fn settle_provider(&self) -> Result<(), ClientError> {
        wait_until(Duration::from_secs(7), || {
            let status = self.provider().terminal_status();
            status["active_operations"] == 0
                && status["attachment_process_owners"] == 0
                && status["process_cleanup_unknown"] == false
        })
        .await
    }
    async fn close(&mut self) -> Result<(), ClientError> {
        let mut failures = Vec::new();
        if let Some(client) = self.consumer.take() {
            match tokio::time::timeout(Duration::from_secs(8), client.close()).await {
                Ok(Ok(())) => {}
                outcome => failures.push(format!("consumer:{outcome:?}")),
            }
        }
        if let Some(client) = self.provider.take() {
            match tokio::time::timeout(Duration::from_secs(8), client.close()).await {
                Ok(Ok(())) => {}
                outcome => failures.push(format!("provider:{outcome:?}")),
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(format!("independent SDK cleanup:{failures:?}").into())
        }
    }
}

async fn send(client: &NativeClient, id: &str, command: &str) -> Result<(), ClientError> {
    client
        .send_terminal(
            id,
            TerminalInput::Bytes {
                data: command.as_bytes().to_vec(),
            },
        )
        .await
}
async fn read_until(client: &NativeClient, id: &str, literal: &[u8]) -> Result<(), ClientError> {
    let started = Instant::now();
    let mut bytes = Vec::new();
    while started.elapsed() < Duration::from_secs(5) {
        let response = client.read_terminal(id, Duration::from_millis(100)).await?;
        if response["message"]["type"] == "bytes" {
            let message: Vec<u8> = serde_json::from_value(response["message"]["data"].clone())?;
            bytes.extend(message);
            if bytes.len() > 128 * 1024 {
                return Err("bounded output oracle exceeded".into());
            }
            if bytes.windows(literal.len()).any(|window| window == literal) {
                return Ok(());
            }
        }
        if response["attached"] != true {
            return Err("terminal closed before literal output".into());
        }
    }
    Err("literal terminal output not observed within five monotonic seconds".into())
}
async fn wait_until(deadline: Duration, predicate: impl Fn() -> bool) -> Result<(), ClientError> {
    let started = Instant::now();
    while started.elapsed() < deadline {
        if predicate() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Err("owned independent observation exceeded monotonic bound".into())
}
async fn exact_file(path: &Path, literal: &[u8]) -> Result<(), ClientError> {
    wait_until(Duration::from_secs(10), || {
        std::fs::read(path).is_ok_and(|bytes| bytes == literal)
    })
    .await
}
fn process_start(pid: u32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields = stat[stat.rfind(')')? + 2..]
        .split_whitespace()
        .collect::<Vec<_>>();
    if fields[0] == "Z" {
        return None;
    }
    Some(fields.get(19)?.to_string())
}
fn finish(product: Result<(), ClientError>, cleanup: Result<(), ClientError>) {
    assert!(
        product.is_ok() && cleanup.is_ok(),
        "terminal primary={product:?}; independent cleanup={cleanup:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owned_terminal_future_cancel_close_drop_and_reopen_are_bounded() {
    let mut pair = Pair::start("lifecycle")
        .await
        .expect("fresh admitted terminal pair");
    let product = async {
        let workspace = pair.create().await?;
        let mut opening = Box::pin(pair.consumer().attach_terminal(pair.provider().address(), workspace.clone(), 80,24));
        tokio::select! {
            result = &mut opening => return Err(format!("future-cancel premise invalid: opening already ended, success={}",result.is_ok()).into()),
            _ = tokio::time::sleep(Duration::from_millis(20)) => {},
        }
        if pair.consumer().terminal_status()["active_operations"] != 1 || pair.consumer().terminal_status()["attachment_handles"] != 0 { return Err("pending attach ownership premise invalid".into()); }
        drop(opening);
        wait_until(Duration::from_secs(1), || pair.consumer().terminal_status()["active_operations"] == 0).await?;
        pair.settle_provider().await?;
        let mut listing = Box::pin(pair.consumer().request_terminal(pair.provider().address(), TerminalRequest::List));
        tokio::select! {
            result = &mut listing => return Err(format!("request-cancel premise invalid: already ended, success={}",result.is_ok()).into()),
            _ = tokio::time::sleep(Duration::from_millis(20)) => {},
        }
        if pair.consumer().terminal_status()["active_operations"] != 1 { return Err("pending request owner premise invalid".into()); }
        drop(listing);
        wait_until(Duration::from_secs(1), || pair.consumer().terminal_status()["active_operations"] == 0).await?;
        pair.settle_provider().await?;
        let id = pair.attach(&workspace).await?;
        if pair.consumer().send_terminal(&id, TerminalInput::Bytes { data:vec![b'X';MAX_BYTES+1] }).await.is_ok() { return Err("oversized terminal input was accepted".into()); }
        send(pair.consumer(), &id, "printf 'started' > close-start; (sleep 1; printf 'close-retained' > close-done) &\n").await?;
        exact_file(&pair.file(&workspace,"close-start"), b"started").await?;
        let identity = pair.consumer().address().id;
        tokio::time::timeout(Duration::from_secs(8), pair.consumer.take().unwrap().close()).await??;
        pair.settle_provider().await?;
        exact_file(&pair.file(&workspace,"close-done"), b"close-retained").await?;
        pair.consumer = Some(NativeClient::open(pair.root.join("consumer"),Ipv4Addr::LOCALHOST).await?);
        if pair.consumer().address().id != identity { return Err("saved terminal consumer identity changed".into()); }
        let id = pair.attach(&workspace).await?;
        send(pair.consumer(), &id, "printf 'started' > drop-start; (sleep 1; printf 'drop-retained' > drop-done) &\n").await?;
        exact_file(&pair.file(&workspace,"drop-start"), b"started").await?;
        drop(pair.consumer.take().unwrap());
        pair.settle_provider().await?;
        exact_file(&pair.file(&workspace,"drop-done"), b"drop-retained").await?;
        pair.consumer = Some(NativeClient::open(pair.root.join("consumer"),Ipv4Addr::LOCALHOST).await?);
        if pair.consumer().address().id != identity { return Err("drop/reopen identity changed".into()); }
        let id = pair.attach(&workspace).await?;
        send(pair.consumer(), &id, "printf '%s' $$ > foreground.pid; exec sleep 300\n").await?;
        wait_until(Duration::from_secs(5), || pair.file(&workspace,"foreground.pid").exists()).await?;
        let pid:u32 = std::fs::read_to_string(pair.file(&workspace,"foreground.pid"))?.parse()?;
        let original = process_start(pid).ok_or("owned foreground process was not live")?;
        pair.end(&workspace).await?;
        wait_until(Duration::from_secs(5), || process_start(pid).as_ref() != Some(&original)).await?;
        pair.consumer().cancel_terminal(&id).await?;
        pair.settle_provider().await?;
        println!("M120_SDK_LIFECYCLE future_cancel=true explicit_close=true drop=true same_identity=true retained_work=true selected_process_ended=true");
        Ok::<_, ClientError>(())
    }.await;
    let cleanup = pair.close().await;
    finish(product, cleanup);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owned_terminal_real_output_and_input_pressure_are_bounded() {
    let mut pair = Pair::start("pressure")
        .await
        .expect("fresh pressure terminal pair");
    let product = async {
        let workspace = pair.create().await?;
        let id = pair.attach(&workspace).await?;
        let failures = pair.provider().terminal_status()["failed_operations"].as_u64().ok_or("failure counter")?;
        // Ordinary finite output and an intentionally absent application reader.
        // This is not an interposed stream, clock or fault implementation.
        send(pair.consumer(), &id, "python3 -c 'import os; open(\"output-start\",\"w\").write(\"started\"); [os.write(1,b\"P\"*16384) for _ in range(1024)]; open(\"output-done\",\"w\").write(\"completed\")'\n").await?;
        exact_file(&pair.file(&workspace,"output-start"),b"started").await?;
        wait_until(Duration::from_secs(10), || pair.consumer().terminal_status()["active_operations"] == 0).await?;
        if pair.provider().terminal_status()["enabled"] != true || pair.provider().terminal_status()["failed_operations"].as_u64().unwrap_or(0) <= failures { return Err("output-pressure failure premise absent".into()); }
        let outcome = pair.consumer().read_terminal(&id,Duration::from_millis(100)).await?;
        if outcome["attached"] != false || outcome["failed"] != true { return Err("output pressure did not report truthful failure".into()); }
        let started = Instant::now();
        let detached = tokio::time::timeout(Duration::from_millis(6250), pair.consumer().detach_terminal(&id)).await??;
        let detach_seconds = started.elapsed().as_secs_f64();
        if detached["workspace_ended"] != false || detached["failed"] != true { return Err("failed output attachment was reported as normal/end".into()); }
        pair.settle_provider().await?;
        exact_file(&pair.file(&workspace,"output-done"), b"completed").await?;
        println!("M120_OUTPUT_PRESSURE produced_bytes=16777216 reader_consumed=false detach_seconds={detach_seconds} retained_job=true failure_truthful=true");
        pair.end(&workspace).await?;

        let workspace = pair.create().await?;
        let id = pair.attach(&workspace).await?;
        send(pair.consumer(), &id, "stty -echo -icanon; printf 'started' > input-start; sleep 20\n").await?;
        exact_file(&pair.file(&workspace,"input-start"),b"started").await?;
        let accepted = std::cell::Cell::new(0usize);
        let failed = std::cell::RefCell::new(Vec::new());
        let mut sends = (0..1024).map(|_| Some(Box::pin(pair.consumer().send_terminal(&id,
            TerminalInput::Bytes { data:vec![b'Q';MAX_BYTES] })))).collect::<Vec<_>>();
        let pending_count = std::cell::Cell::new(0usize);
        let (result, detach_seconds, pending_at_close, accepted_at_close) = {
            let batch = std::future::poll_fn(|context| {
                let mut pending = 0;
                for slot in &mut sends {
                    if let Some(future) = slot {
                        match std::future::Future::poll(future.as_mut(), context) {
                            std::task::Poll::Ready(result) => {
                                match result { Ok(()) => accepted.set(accepted.get()+1), Err(error) => failed.borrow_mut().push(error.to_string()) }
                                *slot = None;
                            }
                            std::task::Poll::Pending => pending += 1,
                        }
                    }
                }
                pending_count.set(pending);
                if pending > 0 { std::task::Poll::Pending } else { std::task::Poll::Ready(()) }
            });
            tokio::pin!(batch);
            // Observe actual public queue pressure, without requiring the
            // product to keep a finite batch Pending for an arbitrary duration.
            std::future::poll_fn(|context| {
                let _ = std::future::Future::poll(batch.as_mut(), context);
                std::task::Poll::Ready(())
            }).await;
            let pending_at_close = pending_count.get();
            let accepted_at_close = accepted.get();
            println!("M120_INPUT_PRESSURE_PREMISE accepted={accepted_at_close} pending={pending_at_close} errors={:?} provider={}",failed.borrow(),pair.provider().terminal_status());
            if pending_at_close == 0 || !failed.borrow().is_empty()
                || pair.provider().terminal_status()["enabled"] != true
                || pair.consumer().terminal_status()["active_operations"] != 1 {
                return Err("input pressure lacked a real pending public queue and enabled/live owner".into());
            }
            let close_started = Instant::now();
            let detaching = pair.consumer().detach_terminal(&id);
            tokio::pin!(detaching);
            let closing = async {
                // Continue polling all send callers during close. Rejections
                // after close are observations, never manufactured pending work.
                tokio::select! {
                    result = &mut detaching => result,
                    _ = &mut batch => (&mut detaching).await,
                }
            };
            let result = tokio::time::timeout(Duration::from_millis(6250), closing).await?;
            // Closing the owner releases every remaining queued caller.
            tokio::time::timeout(Duration::from_secs(1), &mut batch).await?;
            (result, close_started.elapsed().as_secs_f64(), pending_at_close, accepted_at_close)
        };
        drop(sends);
        wait_until(Duration::from_secs(1), || pair.consumer().terminal_status()["active_operations"] == 0).await?;
        pair.settle_provider().await?;
        println!("M120_INPUT_PRESSURE accepted_at_close={accepted_at_close} accepted_final={} errors_final={} bytes_per_frame={MAX_BYTES} public_send_pending_at_close={pending_at_close} detach_seconds={detach_seconds} result={result:?} error_examples={:?}",accepted.get(),failed.borrow().len(),failed.borrow().iter().take(3).collect::<Vec<_>>());
        if accepted.get()+failed.borrow().len() != 1024 { return Err("a public input caller did not settle".into()); }
        if !pair.consumer().request_terminal(pair.provider().address(), TerminalRequest::List).await?["workspaces"].as_array().ok_or("workspace list")?.iter().any(|value| value == &workspace) { return Err("input close ended retained workspace".into()); }
        pair.end(&workspace).await?;
        Ok::<_, ClientError>(())
    }.await;
    let cleanup = pair.close().await;
    finish(product, cleanup);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owned_terminal_attachment_and_workspace_limits_have_no_extra_effect() {
    let mut pair = Pair::start("bounds")
        .await
        .expect("fresh bounded terminal pair");
    let product = async {
        let workspace = pair.create().await?;
        let mut attachments = Vec::new();
        for _ in 0..16 {
            attachments.push(pair.consumer().attach_terminal(pair.provider().address(),workspace.clone(),80,24).await?["id"].as_str().ok_or("attachment id")?.to_owned());
        }
        if pair.consumer().terminal_status()["attachment_handles"] != 16 || pair.provider().terminal_status()["attachment_process_owners"] != 16 { return Err("sixteen actual attachment premise absent".into()); }
        if pair.consumer().attach_terminal(pair.provider().address(),workspace.clone(),80,24).await.is_ok() { return Err("seventeenth attachment accepted".into()); }
        if pair.provider().terminal_status()["attachment_process_owners"] != 16 { return Err("rejected attachment changed provider owner count".into()); }
        for id in attachments { pair.consumer().cancel_terminal(&id).await?; }
        pair.settle_provider().await?;
        pair.end(&workspace).await?;
        let mut workspaces = Vec::new();
        for _ in 0..16 { workspaces.push(pair.create().await?); }
        if pair.consumer().request_terminal(pair.provider().address(),TerminalRequest::Create).await.is_ok() { return Err("seventeenth workspace accepted".into()); }
        let response = pair.consumer().request_terminal(pair.provider().address(),TerminalRequest::List).await?;
        let mut actual = response["workspaces"].as_array().ok_or("actual workspace list")?.iter().map(|value|value.as_str().unwrap().to_owned()).collect::<Vec<_>>();
        actual.sort(); workspaces.sort();
        if actual != workspaces { return Err("workspace limit negative created an extra session".into()); }
        for workspace in workspaces { pair.end(&workspace).await?; }
        pair.settle_provider().await?;
        println!("M120_NATIVE_BOUNDS attachments=16 workspaces=16 extra_no_effect=true all_publicly_ended=true");
        Ok::<_, ClientError>(())
    }.await;
    let cleanup = pair.close().await;
    finish(product, cleanup);
}
