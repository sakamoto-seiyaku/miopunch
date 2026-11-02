//! Small, bounded service protocol for the Linux/browser demo.
//! Membership is checked by the caller before every RPC. This demo explicitly
//! enables these services for its invited members; it is not the product ACL.
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::{Read, Seek, SeekFrom, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream, UdpSocket},
    process::{Child, ChildStdin, ChildStdout, Command},
    task::JoinHandle,
};

pub type Failure = Box<dyn std::error::Error + Send + Sync>;
const CHUNK: usize = 24000;
const MAX_FILE: u64 = 128 * 1024 * 1024;

fn text<'a>(request: &'a Value, key: &str) -> Result<&'a str, Failure> {
    request[key]
        .as_str()
        .ok_or_else(|| format!("missing {key}").into())
}
fn integer(request: &Value, key: &str) -> Result<u64, Failure> {
    request[key]
        .as_u64()
        .ok_or_else(|| format!("missing {key}").into())
}
fn identifier(value: &str) -> Result<&str, Failure> {
    if value.len() != 32 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid object identifier".into());
    }
    Ok(value)
}
fn unique(value: &str) -> String {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    blake3::hash(format!("{time}:{value}").as_bytes()).to_hex()[..32].to_owned()
}
fn file_chunk(path: &Path, offset: u64) -> Result<Value, Failure> {
    let mut file = std::fs::File::open(path)?;
    let size = file.metadata()?.len();
    if offset > size {
        return Err("offset exceeds saved object".into());
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0; CHUNK];
    let count = file.read(&mut bytes)?;
    bytes.truncate(count);
    Ok(
        json!({"data":STANDARD.encode(bytes),"offset":offset+count as u64,"size":size,"done":offset+count as u64==size}),
    )
}
fn private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_private() || ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_loopback() || (ip.segments()[0] & 0xfe00 == 0xfc00),
    }
}
async fn target(host: &str, port: u16) -> Result<SocketAddr, Failure> {
    if port == 0 || host.len() > 253 || host.contains(['\r', '\n', '/', '@']) {
        return Err("invalid destination".into());
    }
    let addresses: Vec<_> = tokio::time::timeout(
        Duration::from_secs(3),
        tokio::net::lookup_host((host, port)),
    )
    .await??
    .collect();
    addresses
        .into_iter()
        .find(|a| private_ip(a.ip()))
        .ok_or_else(|| "demo targets must resolve to a private or loopback address".into())
}

struct Terminal {
    _child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}
impl Terminal {
    fn start(peer: &str, helper: &Path) -> Result<Self, Failure> {
        let mut command = Command::new("python3");
        command
            .arg(helper)
            .arg(peer)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        command
            .env_clear()
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .env("LANG", "C.UTF-8");
        // The runtime grants only SETUID/SETGID. Drop both before Python/tmux.
        unsafe {
            command.pre_exec(|| {
                unsafe extern "C" {
                    fn setgroups(n: usize, groups: *const u32) -> i32;
                    fn setgid(id: u32) -> i32;
                    fn setuid(id: u32) -> i32;
                }
                if setgroups(0, std::ptr::null()) != 0 || setgid(1000) != 0 || setuid(1000) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn()?;
        Ok(Self {
            input: child.stdin.take().ok_or("terminal stdin unavailable")?,
            output: BufReader::new(child.stdout.take().ok_or("terminal stdout unavailable")?),
            _child: child,
        })
    }
    async fn request(&mut self, request: &Value) -> Result<Value, Failure> {
        let mut line = serde_json::to_vec(request)?;
        line.push(b'\n');
        self.input.write_all(&line).await?;
        self.input.flush().await?;
        let mut reply = String::new();
        let count = tokio::time::timeout(Duration::from_secs(8), self.output.read_line(&mut reply))
            .await??;
        if count == 0 || reply.len() > 60000 {
            return Err("terminal helper ended".into());
        }
        Ok(serde_json::from_str(&reply)?)
    }
}

struct Forward {
    protocol: String,
    destination: SocketAddr,
    local: SocketAddr,
    task: JoinHandle<()>,
}
impl Drop for Forward {
    fn drop(&mut self) {
        self.task.abort();
    }
}
pub struct Services {
    root: PathBuf,
    helper: PathBuf,
    terminals: HashMap<String, Terminal>,
    forwards: HashMap<String, (String, Forward)>,
}
impl Services {
    pub fn new(root: PathBuf, helper: PathBuf) -> Self {
        Self {
            root,
            helper,
            terminals: HashMap::new(),
            forwards: HashMap::new(),
        }
    }
    pub async fn request(&mut self, peer: &str, request: Value) -> Result<Value, Failure> {
        let directory = self.root.join(peer);
        std::fs::create_dir_all(&directory)?;
        let operation = text(&request, "op")?;
        match operation {
            "info" => Ok(
                json!({"name":"r_proxy Linux","services":["files","clipboard-buffer","tcp","udp","web","terminal"],"clipboard":"headless-text-buffer","terminal":"unprivileged container / tmux","fileLimit":MAX_FILE,"webLimit":8*1024*1024}),
            ),
            "file.begin" => {
                let name: String = text(&request, "name")?
                    .chars()
                    .filter(|c| !c.is_control() && !matches!(c, '/' | '\\'))
                    .take(180)
                    .collect();
                let size = integer(&request, "size")?;
                if name.is_empty() || size > MAX_FILE {
                    return Err("invalid file name or file exceeds 128 MiB".into());
                }
                let hash = text(&request, "sha256")?;
                if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err("invalid SHA-256".into());
                }
                let id = unique(&name);
                let meta = json!({"id":id,"name":name,"size":size,"sha256":hash,"complete":false});
                std::fs::write(
                    directory.join(format!("{id}.json")),
                    serde_json::to_vec(&meta)?,
                )?;
                std::fs::File::create(directory.join(format!("{id}.data")))?.sync_all()?;
                Ok(meta)
            }
            "file.write" => {
                let id = identifier(text(&request, "id")?)?;
                let meta: Value =
                    serde_json::from_slice(&std::fs::read(directory.join(format!("{id}.json")))?)?;
                if meta["complete"] == true {
                    return Err("file already complete".into());
                }
                let bytes = STANDARD.decode(text(&request, "data")?)?;
                if bytes.len() > CHUNK {
                    return Err("file chunk too large".into());
                }
                let offset = integer(&request, "offset")?;
                let mut file = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(directory.join(format!("{id}.data")))?;
                let size = file.metadata()?.len();
                if offset > size
                    || offset + bytes.len() as u64
                        > meta["size"].as_u64().ok_or("invalid saved size")?
                {
                    return Err("file offset mismatch".into());
                }
                // Exact retries are accepted. A changed chunk cannot overwrite saved bytes.
                file.seek(SeekFrom::Start(offset))?;
                if offset < size {
                    let mut saved = vec![0; bytes.len()];
                    file.read_exact(&mut saved)?;
                    if saved != bytes {
                        return Err("retry content differs".into());
                    }
                } else {
                    file.write_all(&bytes)?;
                    file.sync_data()?;
                }
                Ok(json!({"offset":file.metadata()?.len()}))
            }
            "file.finish" => {
                let id = identifier(text(&request, "id")?)?;
                let path = directory.join(format!("{id}.json"));
                let mut meta: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
                let mut file = std::fs::File::open(directory.join(format!("{id}.data")))?;
                if file.metadata()?.len() != meta["size"].as_u64().ok_or("invalid saved size")? {
                    return Err("file incomplete".into());
                }
                let mut hash = Sha256::new();
                let mut buffer = [0; 65536];
                loop {
                    let n = file.read(&mut buffer)?;
                    if n == 0 {
                        break;
                    }
                    hash.update(&buffer[..n]);
                }
                let digest = hex::encode(hash.finalize());
                if meta["sha256"] != digest {
                    return Err("saved SHA-256 differs".into());
                }
                file.sync_all()?;
                meta["complete"] = json!(true);
                let temp = directory.join(format!("{id}.json.tmp"));
                std::fs::write(&temp, serde_json::to_vec(&meta)?)?;
                std::fs::File::open(&temp)?.sync_all()?;
                std::fs::rename(temp, &path)?;
                std::fs::File::open(&directory)?.sync_all()?;
                Ok(meta)
            }
            "file.list" => {
                let mut files = vec![];
                for entry in std::fs::read_dir(&directory)? {
                    let path = entry?.path();
                    if path.extension().and_then(|x| x.to_str()) == Some("json") {
                        if let Ok(meta) = serde_json::from_slice::<Value>(&std::fs::read(path)?) {
                            if meta["complete"] == true {
                                files.push(meta);
                            }
                        }
                    }
                }
                Ok(json!({"files":files}))
            }
            "file.read" => {
                let id = identifier(text(&request, "id")?)?;
                let meta: Value =
                    serde_json::from_slice(&std::fs::read(directory.join(format!("{id}.json")))?)?;
                if meta["complete"] != true {
                    return Err("file not durably complete".into());
                }
                file_chunk(
                    &directory.join(format!("{id}.data")),
                    integer(&request, "offset")?,
                )
            }
            "clipboard.set" => {
                let value = text(&request, "text")?;
                if value.len() > 16384 {
                    return Err("clipboard text exceeds 16 KiB".into());
                }
                std::fs::write(directory.join("clipboard.txt"), value)?;
                Ok(
                    json!({"text":value,"version":blake3::hash(value.as_bytes()).to_hex().to_string(),"sink":"headless-text-buffer"}),
                )
            }
            "clipboard.get" => {
                let value =
                    std::fs::read_to_string(directory.join("clipboard.txt")).unwrap_or_default();
                Ok(
                    json!({"text":value,"version":blake3::hash(value.as_bytes()).to_hex().to_string(),"sink":"headless-text-buffer"}),
                )
            }
            "terminal" => {
                if !self.terminals.contains_key(peer) {
                    if self.terminals.len() >= 12 {
                        return Err("demo terminal client limit reached".into());
                    }
                    self.terminals
                        .insert(peer.to_owned(), Terminal::start(peer, &self.helper)?);
                }
                let result = self
                    .terminals
                    .get_mut(peer)
                    .ok_or("terminal unavailable")?
                    .request(&request)
                    .await;
                if result.is_err() {
                    // A dead/timed-out helper must not leave its pipe protocol
                    // out of sync. A fresh helper can reattach retained tmux.
                    self.terminals.remove(peer);
                }
                result
            }
            "forward.create" => {
                if self
                    .forwards
                    .values()
                    .filter(|(owner, _)| owner == peer)
                    .count()
                    >= 8
                {
                    return Err("at most eight forwards per node".into());
                }
                let protocol = text(&request, "protocol")?.to_owned();
                let port = u16::try_from(integer(&request, "port")?)?;
                let destination = target(text(&request, "host")?, port).await?;
                let bind = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
                let (local, task) = match protocol.as_str() {
                    "tcp" => {
                        let listener = TcpListener::bind(bind).await?;
                        let local = listener.local_addr()?;
                        let task = tokio::spawn(async move {
                            // Dropping this JoinSet also terminates accepted
                            // sockets when the user stops the parent forward.
                            let mut streams = tokio::task::JoinSet::new();
                            loop {
                                tokio::select! {
                                incoming = listener.accept() => {
                                    let Ok((mut incoming,_)) = incoming else { break };
                                    if streams.len() >= 16 { continue; }
                                    streams.spawn(async move {
                                    if let Ok(Ok(mut outgoing)) = tokio::time::timeout(
                                        Duration::from_secs(4),
                                        TcpStream::connect(destination),
                                    )
                                    .await
                                    {
                                        let _ = tokio::time::timeout(
                                            Duration::from_secs(120),
                                            tokio::io::copy_bidirectional(
                                                &mut incoming,
                                                &mut outgoing,
                                            ),
                                        )
                                        .await;
                                    }
                                    });
                                },
                                Some(_) = streams.join_next(), if !streams.is_empty() => {}
                                }
                            }
                        });
                        (local, task)
                    }
                    "udp" => {
                        let socket = UdpSocket::bind(bind).await?;
                        let local = socket.local_addr()?;
                        let task = tokio::spawn(async move {
                            let mut buffer = [0; 8192];
                            while let Ok((count, client)) = socket.recv_from(&mut buffer).await {
                                if let Ok(upstream) = UdpSocket::bind(bind).await {
                                    if upstream.connect(destination).await.is_ok()
                                        && upstream.send(&buffer[..count]).await.is_ok()
                                    {
                                        if let Ok(Ok(n)) = tokio::time::timeout(
                                            Duration::from_secs(2),
                                            upstream.recv(&mut buffer),
                                        )
                                        .await
                                        {
                                            let _ = socket.send_to(&buffer[..n], client).await;
                                        }
                                    }
                                }
                            }
                        });
                        (local, task)
                    }
                    _ => return Err("protocol must be tcp or udp".into()),
                };
                let id = unique(&destination.to_string());
                self.forwards.insert(
                    id.clone(),
                    (
                        peer.to_owned(),
                        Forward {
                            protocol: protocol.clone(),
                            destination,
                            local,
                            task,
                        },
                    ),
                );
                Ok(
                    json!({"id":id,"protocol":protocol,"listen":local.to_string(),"target":destination.to_string(),"on":"Linux host loopback"}),
                )
            }
            "forward.list" => Ok(
                json!({"forwards":self.forwards.iter().filter(|(_, (owner,_))|owner==peer).map(|(id,(_,f))|json!({"id":id,"protocol":f.protocol,"listen":f.local.to_string(),"target":f.destination.to_string()})).collect::<Vec<_>>()}),
            ),
            "forward.stop" => {
                let id = text(&request, "id")?;
                if self.forwards.get(id).is_none_or(|(owner, _)| owner != peer) {
                    return Err("unknown forward".into());
                }
                self.forwards.remove(id);
                Ok(json!({"stopped":true}))
            }
            "forward.probe" => {
                let id = text(&request, "id")?;
                let (_, forward) = self
                    .forwards
                    .get(id)
                    .filter(|(owner, _)| owner == peer)
                    .ok_or("unknown forward")?;
                let data = STANDARD.decode(text(&request, "data")?)?;
                if data.len() > 4096 {
                    return Err("probe input too large".into());
                }
                let exchange = async {
                    let mut output = vec![0; 8192];
                    let n = if forward.protocol == "tcp" {
                        use tokio::io::AsyncReadExt;
                        let mut stream = TcpStream::connect(forward.local).await?;
                        stream.write_all(&data).await?;
                        stream.read(&mut output).await?
                    } else {
                        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await?;
                        socket.connect(forward.local).await?;
                        socket.send(&data).await?;
                        socket.recv(&mut output).await?
                    };
                    output.truncate(n);
                    Ok::<_, Failure>(json!({"data":STANDARD.encode(output)}))
                };
                tokio::time::timeout(Duration::from_secs(4), exchange).await?
            }
            "web.open" => {
                let url: iroh_base::RelayUrl = text(&request, "url")?.parse()?;
                if !matches!(url.scheme(), "http" | "https")
                    || !url.username().is_empty()
                    || url.password().is_some()
                {
                    return Err("use an HTTP(S) address without embedded credentials".into());
                }
                let host = url.host_str().ok_or("missing website host")?;
                let destination =
                    target(host, url.port_or_known_default().ok_or("invalid URL port")?).await?;
                let site = unique(url.as_str());
                let origin = url.origin().ascii_serialization();
                let value = json!({"origin":origin,"host":host,"port":destination.port(),"ip":destination.ip().to_string()});
                std::fs::write(
                    directory.join(format!("site-{site}.json")),
                    serde_json::to_vec(&value)?,
                )?;
                Ok(
                    json!({"site":site,"origin":origin,"path":format!("{}{}",url.path(),url.query().map(|x|format!("?{x}")).unwrap_or_default())}),
                )
            }
            "web.request" => {
                let site = identifier(text(&request, "site")?)?;
                let configuration: Value = serde_json::from_slice(&std::fs::read(
                    directory.join(format!("site-{site}.json")),
                )?)?;
                let path = text(&request, "path")?;
                let method = text(&request, "method")?;
                if !path.starts_with('/')
                    || path.starts_with("//")
                    || path.chars().any(char::is_control)
                    || !matches!(
                        method,
                        "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE" | "OPTIONS"
                    )
                {
                    return Err("invalid website request".into());
                }
                let data = STANDARD.decode(request["body"].as_str().unwrap_or_default())?;
                if data.len() > 24000 {
                    return Err("demo website upload limit is 24 KiB".into());
                }
                let id = unique(path);
                let headers = directory.join(format!("web-{id}.headers"));
                let body = directory.join(format!("web-{id}.data"));
                // Keep at most 24 response spools per peer, never more than 8 MiB each.
                let mut old: Vec<_> = std::fs::read_dir(&directory)?
                    .filter_map(Result::ok)
                    .filter(|e| e.file_name().to_string_lossy().starts_with("web-"))
                    .collect();
                old.sort_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
                while old.len() > 46 {
                    let entry = old.remove(0);
                    let _ = std::fs::remove_file(entry.path());
                }
                let cookie = directory.join(format!(
                    "cookies-{}",
                    &blake3::hash(
                        configuration["origin"]
                            .as_str()
                            .ok_or("invalid origin")?
                            .as_bytes()
                    )
                    .to_hex()[..24]
                ));
                let mut command = Command::new("curl");
                command
                    .args([
                        "--silent",
                        "--show-error",
                        "--noproxy",
                        "*",
                        "--connect-timeout",
                        "4",
                        "--max-time",
                        "12",
                        "--max-filesize",
                        "8388608",
                        "--proto",
                        "=http,https",
                        "--request",
                        method,
                        "--resolve",
                    ])
                    .arg(format!(
                        "{}:{}:{}",
                        text(&configuration, "host")?,
                        integer(&configuration, "port")?,
                        text(&configuration, "ip")?
                    ))
                    .arg("--url")
                    .arg(format!("{}{path}", text(&configuration, "origin")?))
                    .arg("--dump-header")
                    .arg(&headers)
                    .arg("--output")
                    .arg(&body)
                    .arg("--cookie")
                    .arg(&cookie)
                    .arg("--cookie-jar")
                    .arg(&cookie)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::piped())
                    .kill_on_drop(true);
                for name in ["content-type", "accept", "x-requested-with"] {
                    if let Some(value) = request["headers"][name].as_str() {
                        if !value.contains(['\r', '\n']) {
                            command.arg("--header").arg(format!("{name}: {value}"));
                        }
                    }
                }
                if !matches!(method, "GET" | "HEAD") {
                    command.args(["--data-binary", "@-"]);
                }
                let mut child = command.spawn()?;
                if let Some(mut input) = child.stdin.take() {
                    input.write_all(&data).await?;
                }
                let output = child.wait_with_output().await?;
                if !output.status.success() {
                    let _ = std::fs::remove_file(body);
                    return Err(format!(
                        "provider HTTP failed: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    )
                    .into());
                }
                let raw = std::fs::read_to_string(&headers)?;
                let block = raw
                    .trim()
                    .rsplit("\r\n\r\n")
                    .next()
                    .ok_or("missing HTTP headers")?;
                let mut lines = block.lines();
                let status: u16 = lines
                    .next()
                    .and_then(|x| x.split_whitespace().nth(1))
                    .ok_or("missing HTTP status")?
                    .parse()?;
                let selected: Vec<_> = lines
                    .filter_map(|l| l.split_once(':'))
                    .filter_map(|(k, v)| {
                        let k = k.to_ascii_lowercase();
                        matches!(
                            k.as_str(),
                            "content-type" | "location" | "cache-control" | "content-language"
                        )
                        .then(|| (k, v.trim().to_owned()))
                    })
                    .collect();
                let size = std::fs::metadata(&body)?.len();
                if size > 8 * 1024 * 1024 {
                    std::fs::remove_file(body)?;
                    return Err("website response too large".into());
                }
                Ok(
                    json!({"id":id,"status":status,"headers":selected,"size":size,"origin":configuration["origin"]}),
                )
            }
            "web.read" => {
                let id = identifier(text(&request, "id")?)?;
                file_chunk(
                    &directory.join(format!("web-{id}.data")),
                    integer(&request, "offset")?,
                )
            }
            _ => Err("unknown demo operation".into()),
        }
    }
}
