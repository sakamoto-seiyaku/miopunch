//! Platform-host ownership through real shared commands, SDK connections and files.
use miopunch_native_probe::{
    client_host::{ClientHost, StartMode},
    native_client::ClientError,
    resources::ResourceConfig,
};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::io::AsyncReadExt;

fn ensure(condition: bool, message: &'static str) -> Result<(), ClientError> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}
async fn call(host: &ClientHost, mut request: Value) -> Result<Value, ClientError> {
    request["id"] = 1.into();
    let response = host.request(&request.to_string()).await;
    ensure(response["ok"] == true, "shared host command failed")?;
    Ok(response["result"].clone())
}

#[tokio::test]
async fn native_host_retains_identity_excludes_duplicate_and_closes_pending_work()
-> Result<(), ClientError> {
    let root = PathBuf::from(std::env::var_os("M121_HOST_PRIVATE").ok_or("owned root required")?);
    let relay = std::env::var("M121_HOST_LOCAL_RELAY")?;
    let config: ResourceConfig = serde_json::from_value(json!({
        "defaults":{"revision":2,"resources":[]},
        "replacement":[{"url":relay,"uses":["DataRelay"],"limitation":"owned platform host fixture"}],
        "disabled":[],"disabled_uses":["Observation","Lookup","Signaling"],
        "lan_discovery":"Disabled","port_mapping":"Disabled"
    }))?;
    let source = Arc::new(ClientHost::default());
    let member = Arc::new(ClientHost::default());
    let duplicate = ClientHost::default();
    let outcome = async {
        let ready = source.start(root.join("source"), StartMode::Create, config.clone()).await?;
        let source_address = ready["address"].clone();
        let source_id = ready["status"]["node"].clone();
        ensure(ready["status"]["stage"] == "AwaitingRecoverySave", "creation bypassed save gate")?;
        ensure(duplicate.start(root.join("source"), StartMode::Open, config.clone()).await.is_err(), "second host opened same directory")?;
        call(&source, json!({"op":"export_recovery_card","path":root.join("card")})).await?;
        call(&source, json!({"op":"acknowledge_recovery_saved"})).await?;
        let invite = root.join("invite");
        call(&source, json!({"op":"issue_precode","path":invite,"seconds":300,"max_uses":1})).await?;
        let joined = member.start(root.join("member"), StartMode::Join { invitation: std::fs::read_to_string(&invite)? }, config.clone()).await?;
        let identity = joined["status"]["node"].clone();
        ensure(identity != source_id, "new member reused source identity")?;
        ensure(call(&member, json!({"op":"request","path":invite})).await?["stage"] == "Complete", "member admission incomplete")?;
        ensure(call(&member, json!({"op":"ping","address":source_address})).await?["reply"] == "PONG", "member connection premise")?;
        let directory = root.join("receive");
        call(&source, json!({"op":"files_enable","directory":directory})).await?;
        let input = root.join("actual-file.bin");
        let bytes = b"M121 actual SDK saved file\0\xff";
        std::fs::write(&input, bytes)?;
        let task = call(&member, json!({"op":"file_send","address":source_address,"path":input})).await?["task"].clone();
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let status = call(&member, json!({"op":"files_status"})).await?;
                if status["tasks"].as_array().ok_or("task list missing")?.iter().any(|t| t["id"]==task && t["state"]=="complete") { break; }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Ok::<_,ClientError>(())
        }).await??;
        let files = std::fs::read_dir(&directory)?.collect::<Result<Vec<_>,_>>()?;
        ensure(files.len()==1 && std::fs::read(files[0].path())?==bytes, "independent saved file differs")?;
        // A real HTTP request is held by an owned target before host stop.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let target = format!("http://{}",listener.local_addr()?);
        call(&source, json!({"op":"web_enable","targets":[target]})).await?;
        call(&member, json!({"op":"service_sync","address":source_address})).await?;
        let pending_host = member.clone();
        let pending = tokio::spawn(async move {
            pending_host.request(&json!({"id":72,"op":"web_open","address":source_address,"url":format!("{target}/held")}).to_string()).await
        });
        let (mut socket,_) = tokio::time::timeout(Duration::from_secs(15),listener.accept()).await??;
        let mut request = [0;4096];
        let count = tokio::time::timeout(Duration::from_secs(5),socket.read(&mut request)).await??;
        ensure(request[..count].starts_with(b"GET /held "), "actual HTTP prerequisite missing")?;
        ensure(!pending.is_finished(), "request was not held before stop")?;
        let start = tokio::time::Instant::now();
        tokio::time::timeout(Duration::from_secs(10),member.stop()).await??;
        let pending_result = tokio::time::timeout(Duration::from_secs(2),pending).await??;
        ensure(pending_result["ok"]==false && !member.running().await, "stop did not settle pending request")?;
        println!("M121_HOST_CANCEL {}",json!({"actual_http_pending":true,"stop_seconds":start.elapsed().as_secs_f64(),"caller_failed_truthfully":true}));
        drop(socket); drop(listener);
        let reopened = member.start(root.join("member"),StartMode::Open,config.clone()).await?;
        ensure(reopened["status"]["node"]==identity && reopened["status"]["stage"]=="Complete", "reopen lost identity or membership")?;
        ensure(member.request("bad json").await["error"]=="invalid_command", "malformed control accepted")?;
        ensure(member.request(&" ".repeat(65537)).await["error"]=="invalid_command", "oversized control accepted")?;
        ensure(call(&member,json!({"op":"stop"})).await?["stopped"]==true, "stop response preceded close")?;
        println!("M121_HOST_CHAIN {}",json!({"duplicate_host_denied":true,"independent_saved_bytes":bytes.len(),"same_identity_reopened":true,"membership_retained":true,"explicit_stop_completed":true}));
        Ok::<_,ClientError>(())
    }.await;
    // Cleanup always executes, including assertion/error paths.
    let member_close = member.stop().await;
    let source_close = source.stop().await;
    let duplicate_close = duplicate.stop().await;
    outcome?;
    member_close?;
    source_close?;
    duplicate_close?;
    Ok(())
}
