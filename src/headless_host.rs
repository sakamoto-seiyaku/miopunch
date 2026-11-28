//! Linux/Unix service host. Control connections do not own the node lifetime.
//! The local socket exposes trusted application commands, never a network API.
use crate::{
    client_commands::Request,
    client_host::{ClientHost, StartMode},
    native_client::ClientError,
};
use std::{
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::Semaphore,
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

struct SocketPath {
    path: PathBuf,
    device: u64,
    inode: u64,
}
impl Drop for SocketPath {
    fn drop(&mut self) {
        if let Ok(info) = self.path.symlink_metadata() {
            if info.dev() == self.device && info.ino() == self.inode {
                let _ = std::fs::remove_file(&self.path);
            }
        }
    }
}

/// Open an existing node and serve until explicit stop/replacement or SIGTERM/
/// SIGINT. The caller must provide an existing private control directory.
pub async fn serve(directory: &Path, socket: &Path) -> Result<(), ClientError> {
    let parent = socket
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent_info = parent.metadata()?;
    if !parent_info.is_dir() || parent_info.mode() & 0o077 != 0 {
        return Err("control socket requires a private directory (mode 0700)".into());
    }
    // Binding never replaces an existing path, including stale sockets.
    let listener = UnixListener::bind(socket)?;
    let info = socket.symlink_metadata()?;
    let _socket_path = SocketPath {
        path: socket.to_owned(),
        device: info.dev(),
        inode: info.ino(),
    };
    if parent_info.uid() != info.uid() {
        return Err("control directory must belong to this user".into());
    }
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let host = Arc::new(ClientHost::default());
    let ready = host
        .start(directory, StartMode::Open, Default::default())
        .await?;
    println!("{ready}");
    let shutdown = CancellationToken::new();
    let commands = Arc::new(Semaphore::new(16));
    let mut connections = JoinSet::new();
    let outcome = loop {
        tokio::select! {
            biased;
            _ = shutdown.cancelled() => break Ok(()),
            _ = terminate.recv() => break Ok(()),
            _ = interrupt.recv() => break Ok(()),
            Some(_) = connections.join_next(), if !connections.is_empty() => {},
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    // Filesystem permissions and kernel peer credentials agree.
                    if !stream.peer_cred().is_ok_and(|peer| peer.uid() == info.uid()) || connections.len() >= 32 { continue; }
                    let (host, shutdown, commands) = (host.clone(), shutdown.clone(), commands.clone());
                    connections.spawn(async move { control(stream, host, shutdown, commands).await });
                }
                Err(error) => break Err(error.into()),
            }
        }
    };
    drop(listener);
    shutdown.cancel();
    let close = host.stop().await;
    // Reads/writes and ordinary commands observe shutdown. Lifecycle work is
    // retained by ClientHost until its SDK close/export completes.
    while connections.join_next().await.is_some() {}
    close?;
    outcome
}

async fn control(
    stream: UnixStream,
    host: Arc<ClientHost>,
    shutdown: CancellationToken,
    commands: Arc<Semaphore>,
) -> Result<(), ClientError> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader.take(Request::HOST_INPUT_LIMIT as u64 + 1));
    let mut input = Vec::new();
    tokio::select! {
        _ = shutdown.cancelled() => return Ok(()),
        result = tokio::time::timeout(Duration::from_secs(5), reader.read_until(b'\n', &mut input)) => result??,
    };
    let request = if input.last() == Some(&b'\n') {
        Request::parse_host(&input)
    } else {
        Err(Request::invalid_host_command())
    };
    let lifecycle = request
        .as_ref()
        .is_ok_and(|request| request.ends_host_session());
    let response = match request {
        Ok(request) => {
            // Stop has capacity independent of slow business commands.
            let _permit = if lifecycle {
                None
            } else {
                Some(tokio::select! {
                    _ = shutdown.cancelled() => return Ok(()),
                    permit = commands.acquire() => permit?,
                })
            };
            host.dispatch_request(request).await
        }
        Err(response) => response,
    };
    let stopped = lifecycle && !host.running().await;
    let output = format!("{response}\n");
    let write = tokio::select! {
        _ = shutdown.cancelled() => Ok(()),
        result = tokio::time::timeout(Duration::from_secs(5), writer.write_all(output.as_bytes())) => result.map_err(ClientError::from).and_then(|r| r.map_err(ClientError::from)),
    };
    // A disconnected controller cannot prevent a completed explicit stop.
    if stopped {
        shutdown.cancel();
    }
    write
}
