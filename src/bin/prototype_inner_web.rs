//! THROWAWAY: admitted browser -> existing member stream -> provider HTTP.
//! This does not implement the product service ACL or a production HTTP proxy.
use std::{error::Error, path::PathBuf, sync::Arc, time::Duration};

use base64::{Engine, engine::general_purpose::STANDARD};
use iroh::{RelayMode, endpoint::presets};
use miopunch_native_probe::{
    admission_gate::{ADMISSION_REQUEST_ALPN, PRECODE_REQUEST_ALPN, ProtocolScope},
    connection_coordinator::MEMBER_ALPN,
    control_protocol::{AuthenticatedControl, OwnedControl},
    native_network::NativeNetwork,
    resources::{Resource, ResourceUse},
};
use serde::Deserialize;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

type Failure = Box<dyn Error + Send + Sync>;

#[derive(Deserialize)]
struct Request {
    path: String,
    method: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    content_type: String,
}

// Deliberately bounded to one explicitly selected, owned test origin. Curl is
// only a throwaway HTTP adapter; argv is never passed through a shell.
async fn fetch(
    request: Request,
    directory: &PathBuf,
    destination: &str,
) -> Result<String, Failure> {
    if !request.path.starts_with('/')
        || request.path.starts_with("//")
        || request.path.chars().any(char::is_control)
        || !matches!(request.method.as_str(), "GET" | "POST")
        || request.content_type.contains(['\r', '\n'])
    {
        return Err("prototype target or method refused".into());
    }
    let body = STANDARD.decode(&request.body)?;
    if body.len() > 16384 {
        return Err("prototype request limit exceeded".into());
    }
    let headers = directory.join("http-headers");
    let cookies = directory.join("http-cookies");
    let mut command = tokio::process::Command::new("curl");
    command.args([
        "--silent",
        "--show-error",
        "--noproxy",
        "*",
        "--max-time",
        "10",
        "--max-filesize",
        "24576",
        "--proto",
        "=http",
        "--request",
        &request.method,
        "--connect-to",
        destination,
        "--dump-header",
    ]);
    command
        .arg(&headers)
        .arg("--cookie")
        .arg(&cookies)
        .arg("--cookie-jar")
        .arg(&cookies);
    command
        .arg("--url")
        .arg(format!("http://node-only.invalid{}", request.path));
    if request.method == "POST" {
        command
            .arg("--header")
            .arg(format!("Content-Type: {}", request.content_type));
        command.args(["--data-binary", "@-"]);
    }
    command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn()?;
    if let Some(mut input) = child.stdin.take() {
        input.write_all(&body).await?;
    }
    let output = child.wait_with_output().await?;
    if !output.status.success() {
        return Err(format!(
            "provider HTTP failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    let text = tokio::fs::read_to_string(headers).await?;
    let block = text
        .trim()
        .rsplit("\r\n\r\n")
        .next()
        .ok_or("missing HTTP headers")?;
    let mut lines = block.lines();
    let status: u16 = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or("missing HTTP status")?
        .parse()?;
    let forwarded: Vec<_> = lines
        .filter_map(|line| line.split_once(':'))
        .filter_map(|(key, value)| {
            let key = key.to_ascii_lowercase();
            matches!(key.as_str(), "content-type" | "location" | "cache-control")
                .then(|| (key, value.trim().to_owned()))
        })
        .collect();
    println!(
        "{}",
        json!({"event":"http","method":request.method,"path":request.path,
        "status":status,"bytes":output.stdout.len()})
    );
    Ok(
        json!({"status":status,"headers":forwarded,"body":STANDARD.encode(output.stdout)})
            .to_string(),
    )
}

#[tokio::main]
async fn main() -> Result<(), Failure> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err("usage: prototype_inner_web SCRATCH_DIR RELAY_URL OWNED_HTTP_PORT".into());
    }
    let directory = PathBuf::from(&args[1]);
    let port: u16 = args[3].parse()?;
    let destination = format!("node-only.invalid:80:127.0.0.1:{port}");
    let mut node = NativeNetwork::create(&directory).await?;
    let mut resources = node.resource_configuration()?;
    resources.import_trusted_replacement(vec![Resource {
        url: args[2].clone(),
        uses: [ResourceUse::DataRelay].into(),
        limitation: "owned local prototype relay".into(),
    }])?;
    resources.udp_direct_disabled = true;
    resources.tcp_direct_disabled = true;
    resources.lan_discovery = miopunch_native_probe::resources::LanDiscovery::Disabled;
    node.save_resource_configuration(&resources)?;
    node.admission_gate()
        .register_protocol(MEMBER_ALPN, ProtocolScope::Member);
    let endpoint = node
        .bind_endpoint_with(
            iroh::Endpoint::builder(presets::Minimal)
                .clear_ip_transports()
                .relay_mode(RelayMode::custom([args[2].parse()?]))
                .alpns(vec![
                    ADMISSION_REQUEST_ALPN.to_vec(),
                    PRECODE_REQUEST_ALPN.to_vec(),
                    MEMBER_ALPN.to_vec(),
                ]),
        )
        .await?;
    tokio::time::timeout(Duration::from_secs(20), endpoint.online()).await?;
    let expires = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs()
        + 3600;
    let code = node.issue_precode(expires, Some(1)).await?;
    println!(
        "{}",
        json!({"event":"ready","id":endpoint.id().to_string(),"code":hex::encode(code.encode())})
    );
    let node = Arc::new(tokio::sync::Mutex::new(node));
    let http_owner = Arc::new(tokio::sync::Mutex::new(()));
    let mut handlers = tokio::task::JoinSet::new();
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = lines.next_line() => break,
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { break };
                let node = node.clone();
                let directory = directory.clone();
                let destination = destination.clone();
                let http_owner = http_owner.clone();
                handlers.spawn(async move {
                    let mut accepting = incoming.accept()?.with_caller_deadline(n0_future::time::Instant::now() + Duration::from_secs(60));
                    let alpn = accepting.alpn().await?;
                    if alpn == PRECODE_REQUEST_ALPN {
                        let control = Arc::new(accepting.into_control().await?).retain_control();
                        let stage = node.lock().await.serve_precode_admission(&control).await?;
                        println!("{}", json!({"event":"admission","stage":format!("{stage:?}")}));
                        control.revocation_handle().closed().await;
                    } else if alpn == MEMBER_ALPN {
                        let connection = accepting.await?;
                        while let Ok((mut send, mut recv)) = connection.accept_bi().await {
                            if !node.lock().await.admission_gate().allows(&connection, ProtocolScope::Member) {
                                connection.close(0u32.into(), b"prototype membership refused");
                                break;
                            }
                            let request = recv.read_to_end(32768).await?;
                            let _owner = http_owner.lock().await;
                            let result = match serde_json::from_slice(&request) {
                                Ok(request) => fetch(request, &directory, &destination).await,
                                Err(error) => Err(error.into()),
                            };
                            let response = result.unwrap_or_else(|error| json!({"error":error.to_string()}).to_string());
                            send.write_all(response.as_bytes()).await?;
                            send.finish()?;
                        }
                    }
                    Ok::<_, Failure>(())
                });
            },
            Some(result) = handlers.join_next(), if !handlers.is_empty() => {
                if let Err(error) = result? { eprintln!("prototype connection: {error}"); }
            }
        }
    }
    endpoint.close().await;
    handlers.abort_all();
    while handlers.join_next().await.is_some() {}
    Arc::try_unwrap(node)
        .map_err(|_| "prototype node retained")?
        .into_inner()
        .close()
        .await;
    Ok(())
}
