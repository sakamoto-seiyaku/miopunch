//! Private tmux server operations. No user socket, startup file or OS shell
//! replacement is consulted. Windows executes these operations inside WSL.
use crate::{
    native_client::ClientError,
    terminal::{self, TerminalInput, TerminalProvider},
};
use std::{
    process::Stdio,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
};
use tokio_util::sync::CancellationToken;

const COMMAND_BUDGET: Duration = Duration::from_secs(5);
const MAX_CONTROL_LINE: usize = 128 * 1024;
const MAX_CLOSE_DRAIN: u64 = 2 * 1024 * 1024;
// tmux 3.3a's command parser overflows its yacc stack with one argument
// per byte of a valid 16 KiB record. Keep each command's argument count
// small while preserving byte order and the public record size.
const INPUT_COMMAND_BYTES: usize = 1024;

pub(crate) struct Tmux {
    pub configuration: TerminalProvider,
    operations: Mutex<()>,
    pub live_clients: AtomicUsize,
    pub cleanup_unknown: AtomicBool,
}

impl Tmux {
    fn new(configuration: TerminalProvider) -> Self {
        Self {
            configuration,
            operations: Mutex::new(()),
            live_clients: AtomicUsize::new(0),
            cleanup_unknown: AtomicBool::new(false),
        }
    }
    #[cfg(windows)]
    fn wsl_command() -> Result<Command, ClientError> {
        let system =
            std::env::var_os("SystemRoot").ok_or("Windows system directory unavailable")?;
        let mut command = Command::new(std::path::Path::new(&system).join("System32/wsl.exe"));
        command
            .env_clear()
            .env("SystemRoot", system)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        Ok(command)
    }
    pub async fn environments() -> Result<serde_json::Value, ClientError> {
        #[cfg(windows)]
        {
            let mut command = Self::wsl_command()?;
            command.args(["--list", "--quiet"]);
            let (ok, bytes, _) = Self::output(command, &CancellationToken::new()).await?;
            if !ok {
                return Err("WSL environment discovery failed; no installation attempted".into());
            }
            let text = if bytes.starts_with(&[0xff, 0xfe]) || bytes.contains(&0) {
                if bytes.len() % 2 != 0 {
                    return Err("invalid WSL environment output".into());
                }
                let words = bytes
                    .chunks_exact(2)
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                    .collect::<Vec<_>>();
                String::from_utf16(&words)?
            } else {
                String::from_utf8(bytes)?
            };
            let mut names = text
                .trim_start_matches('\u{feff}')
                .lines()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>();
            if names.len() > 32
                || names
                    .iter()
                    .any(|name| name.len() > 128 || name.chars().any(char::is_control))
            {
                return Err("WSL environment list exceeded bounds".into());
            }
            names.sort();
            names.dedup();
            return Ok(
                serde_json::json!({"platform":"windows_wsl","distributions":names,"runtime_probed":false,"automatic_install":false}),
            );
        }
        #[cfg(target_os = "linux")]
        {
            let backend = Self::new(TerminalProvider {
                directory: String::new(),
                distribution: None,
            });
            let mut command = backend.command("/usr/bin/tmux")?;
            command.arg("-V");
            let (ok, bytes, _) = Self::output(command, &CancellationToken::new()).await?;
            return Ok(
                serde_json::json!({"platform":"linux","tmux_available":ok && bytes.starts_with(b"tmux "),
                "version":String::from_utf8(bytes)?.trim(),"automatic_install":false}),
            );
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        Ok(
            serde_json::json!({"platform":"unsupported","provider_available":false,"automatic_install":false}),
        )
    }
    fn command(&self, executable: &str) -> Result<Command, ClientError> {
        let mut command;
        #[cfg(windows)]
        {
            let distribution = self
                .configuration
                .distribution
                .as_ref()
                .ok_or("WSL distribution required")?;
            command = Self::wsl_command()?;
            // Windows env_clear alone does not clear the selected distro's
            // Linux environment. No user shell startup/history is consulted.
            command.args([
                "--distribution",
                distribution,
                "--exec",
                "/usr/bin/env",
                "-i",
                "PATH=/usr/local/bin:/usr/bin:/bin",
                "LANG=C.UTF-8",
                "TERM=xterm-256color",
                "SHELL=/bin/bash",
                "HISTFILE=/dev/null",
                executable,
            ]);
        }
        #[cfg(not(windows))]
        {
            if self.configuration.distribution.is_some() || !cfg!(target_os = "linux") {
                return Err("terminal provider requires Linux or selected Windows WSL".into());
            }
            command = Command::new(executable);
            command.env_clear();
        }
        command
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .env("LANG", "C.UTF-8")
            .env("TERM", "xterm-256color")
            .env("SHELL", "/bin/bash")
            .env("HISTFILE", "/dev/null")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        Ok(command)
    }
    fn tmux(&self) -> Result<Command, ClientError> {
        let mut command = self.command("/usr/bin/tmux")?;
        command.args([
            "-S",
            &self.socket(),
            "-f",
            &format!("{}/tmux.conf", self.configuration.directory),
        ]);
        Ok(command)
    }
    fn socket(&self) -> String {
        format!("{}/server.sock", self.configuration.directory)
    }

    async fn output(
        mut command: Command,
        cancel: &CancellationToken,
    ) -> Result<(bool, Vec<u8>, Vec<u8>), ClientError> {
        if cancel.is_cancelled() {
            return Err("terminal operation cancelled".into());
        }
        let mut child = command.spawn()?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or("tmux stdout missing")?
            .take(MAX_CONTROL_LINE as u64 + 1);
        let mut stderr = child
            .stderr
            .take()
            .ok_or("tmux stderr missing")?
            .take(MAX_CONTROL_LINE as u64 + 1);
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let result = tokio::time::timeout(COMMAND_BUDGET, async {
            tokio::select! { biased;
            _ = cancel.cancelled() => Err(std::io::Error::other("terminal operation cancelled")),
            result = async {
            let (status, _, _) = tokio::try_join!(child.wait(), stdout.read_to_end(&mut out), stderr.read_to_end(&mut err))?;
            Ok::<_, std::io::Error>(status)
            } => result,
            }
        }).await;
        match result {
            Ok(Ok(status)) if out.len() <= MAX_CONTROL_LINE && err.len() <= MAX_CONTROL_LINE => {
                Ok((status.success(), out, err))
            }
            _ => {
                let _ = child.start_kill();
                let joined = tokio::time::timeout(Duration::from_secs(1), child.wait()).await;
                if !matches!(joined, Ok(Ok(_))) {
                    return Err("tmux command owner did not settle".into());
                }
                Err("tmux command failed or exceeded bounded output/deadline".into())
            }
        }
    }

    pub async fn configure(
        configuration: TerminalProvider,
        owner: String,
    ) -> Result<Arc<Self>, ClientError> {
        let path = &configuration.directory;
        if !path.starts_with('/')
            || path.len() > 70
            || path.chars().any(char::is_control)
            || path.split('/').any(|part| matches!(part, ".." | "."))
            || configuration.distribution.as_ref().is_some_and(|name| {
                name.is_empty() || name.len() > 128 || name.chars().any(char::is_control)
            })
        {
            return Err("invalid private terminal directory or WSL distribution".into());
        }
        let backend = Arc::new(Self::new(configuration));
        let mut version = backend.command("/usr/bin/tmux")?;
        version.arg("-V");
        let configure = CancellationToken::new();
        let (ok, bytes, _) = Self::output(version, &configure).await?;
        if !ok || !bytes.starts_with(b"tmux ") {
            return Err("tmux unavailable in selected terminal environment".into());
        }
        let mut initialize = backend.command("/bin/sh")?;
        // Constant program and positional arguments: configuration never
        // becomes shell syntax. Existing unmarked directories are refused.
        initialize.args(["-c", r#"set -eu
root=$1
owner=$2
if [ -e "$root" ]; then
  [ -d "$root" ] && [ ! -L "$root" ] && [ -f "$root/owner" ] && [ ! -L "$root/owner" ]
  [ "$(cat "$root/owner")" = "$owner" ]
else
  umask 077
  mkdir -- "$root"
  printf '%s' "$owner" > "$root/owner"
  printf '%s\n' 'set -g history-limit 1024' 'set -g set-clipboard off' 'set -g exit-empty on' > "$root/tmux.conf"
fi
test -x /bin/bash
"#, "miopunch-terminal", &backend.configuration.directory, &owner]);
        if !Self::output(initialize, &configure).await?.0 {
            return Err("terminal directory ownership or shell prerequisite failed".into());
        }
        Ok(backend)
    }

    async fn list_unlocked(&self, cancel: &CancellationToken) -> Result<Vec<String>, ClientError> {
        let mut command = self.tmux()?;
        command.args(["list-sessions", "-F", "#{session_name}"]);
        let (ok, out, err) = Self::output(command, cancel).await?;
        if !ok {
            // A last-session exit removes the server, sometimes leaving its
            // socket inode. Other failures are not reported as an empty list.
            let error = String::from_utf8_lossy(&err);
            if error.starts_with("no server running on ")
                || error.starts_with("error connecting to ")
                    && error.contains("No such file or directory")
            {
                return Ok(Vec::new());
            }
            return Err("private tmux session list failed".into());
        }
        let names = String::from_utf8(out)?
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if names.len() > terminal::MAX_WORKSPACES {
            return Err("terminal workspace limit exceeded".into());
        }
        for name in &names {
            terminal::validate_workspace(name)?;
        }
        Ok(names)
    }
    pub async fn list(&self, cancel: &CancellationToken) -> Result<Vec<String>, ClientError> {
        let _operation = self.operations.lock().await;
        self.list_unlocked(cancel).await
    }
    pub async fn create(&self, cancel: &CancellationToken) -> Result<String, ClientError> {
        let _operation = self.operations.lock().await;
        if self.list_unlocked(cancel).await?.len() >= terminal::MAX_WORKSPACES {
            return Err("terminal workspace limit reached".into());
        }
        let workspace = format!("mio-{}", hex::encode(rand::random::<[u8; 16]>()));
        let directory = format!("{}/{}", self.configuration.directory, workspace);
        let mut mkdir = self.command("/bin/mkdir")?;
        mkdir.args(["-m", "700", "--", &directory]);
        if !Self::output(mkdir, cancel).await?.0 {
            return Err("terminal workspace directory creation failed".into());
        }
        let mut command = self.tmux()?;
        command.args([
            "new-session",
            "-d",
            "-s",
            &workspace,
            "-c",
            &directory,
            "/bin/bash",
            "--noprofile",
            "--norc",
        ]);
        if !Self::output(command, cancel).await?.0 {
            return Err("terminal workspace creation failed".into());
        }
        Ok(workspace)
    }
    pub async fn end(
        &self,
        workspace: &str,
        cancel: &CancellationToken,
    ) -> Result<(), ClientError> {
        terminal::validate_workspace(workspace)?;
        let _operation = self.operations.lock().await;
        let mut command = self.tmux()?;
        command.args(["kill-session", "-t", &format!("={workspace}")]);
        if !Self::output(command, cancel).await?.0 {
            return Err("terminal workspace no longer exists or could not end".into());
        }
        if self
            .list_unlocked(cancel)
            .await?
            .iter()
            .any(|name| name == workspace)
        {
            return Err("terminal workspace did not end".into());
        }
        Ok(())
    }
    pub async fn attach(
        self: &Arc<Self>,
        workspace: &str,
        columns: u16,
        rows: u16,
        cancel: &CancellationToken,
    ) -> Result<ControlClient, ClientError> {
        terminal::validate_workspace(workspace)?;
        terminal::validate_size(columns, rows)?;
        let _operation = self.operations.lock().await;
        if !self
            .list_unlocked(cancel)
            .await?
            .iter()
            .any(|name| name == workspace)
        {
            return Err("terminal workspace no longer exists".into());
        }
        if cancel.is_cancelled() {
            return Err("terminal attachment cancelled".into());
        }
        let mut command = self.tmux()?;
        command
            .arg("-C")
            .args(["attach-session", "-t", &format!("={workspace}")])
            .stdin(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn()?;
        let input = child.stdin.take().ok_or("terminal stdin unavailable")?;
        let output = child.stdout.take().ok_or("terminal stdout unavailable")?;
        let mut client = ControlClient {
            child,
            input: Some(input),
            output: Some(BufReader::new(output)),
            workspace: workspace.to_owned(),
            outcome: None,
            backend: Arc::downgrade(self),
            settled: false,
        };
        self.live_clients.fetch_add(1, Ordering::AcqRel);
        if let Err(primary) = client.write(&TerminalInput::Resize { columns, rows }).await {
            let _ = client.close().await;
            return Err(primary);
        }
        Ok(client)
    }
}

pub(crate) struct ControlClient {
    child: Child,
    pub input: Option<ChildStdin>,
    pub output: Option<BufReader<ChildStdout>>,
    workspace: String,
    outcome: Option<Result<(), String>>,
    backend: Weak<Tmux>,
    settled: bool,
}
impl ControlClient {
    fn settled(&mut self) {
        if !self.settled {
            self.settled = true;
            if let Some(backend) = self.backend.upgrade() {
                backend.live_clients.fetch_sub(1, Ordering::AcqRel);
            }
        }
    }
    pub fn encode(workspace: &str, input: &TerminalInput) -> Result<Vec<u8>, ClientError> {
        input.validate()?;
        let command = match input {
            TerminalInput::Bytes { data } if data.is_empty() => return Ok(Vec::new()),
            TerminalInput::Bytes { data } => data
                .chunks(INPUT_COMMAND_BYTES)
                .map(|chunk| {
                    format!(
                        "send-keys -t {workspace}:0.0 -H {}\n",
                        chunk.iter()
                            .map(|byte| format!("{byte:02x}"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    )
                })
                .collect::<String>(),
            TerminalInput::Resize { columns, rows } => {
                format!("resize-window -t {workspace}:0 -x {columns} -y {rows}\n")
            }
            TerminalInput::Detach => "detach-client\n".into(),
        };
        Ok(command.into_bytes())
    }
    async fn write(&mut self, input: &TerminalInput) -> Result<(), ClientError> {
        let bytes = Self::encode(&self.workspace, input)?;
        tokio::time::timeout(
            COMMAND_BUDGET,
            self.input
                .as_mut()
                .ok_or("terminal input closed")?
                .write_all(&bytes),
        )
        .await??;
        Ok(())
    }
    pub async fn close(&mut self) -> Result<(), ClientError> {
        if let Some(outcome) = &self.outcome {
            return outcome.clone().map_err(Into::into);
        }
        // The budget includes writing detach, even when the PTY/control pipe
        // is under legitimate backpressure. Kill only this own client, never
        // its retained tmux session/server.
        let input = self.input.take();
        let output = self.output.take();
        let protocol = tokio::time::timeout(COMMAND_BUDGET, async {
            let detaching = async {
                if let Some(mut input) = input {
                    if let Err(error) = input.write_all(b"detach-client\n").await {
                        if error.kind() != std::io::ErrorKind::BrokenPipe {
                            return Err(error);
                        }
                    }
                }
                Ok::<_, std::io::Error>(())
            };
            let draining = async {
                if let Some(output) = output {
                    let copied = tokio::io::copy(
                        &mut output.take(MAX_CLOSE_DRAIN + 1),
                        &mut tokio::io::sink(),
                    )
                    .await?;
                    if copied > MAX_CLOSE_DRAIN {
                        return Err(std::io::Error::other(
                            "terminal detach drain exceeded bound",
                        ));
                    }
                }
                Ok::<_, std::io::Error>(())
            };
            // tmux's control client must flush its final stdout before it
            // exits. Dropping that pipe before wait can strand normal detach.
            // Drain concurrently with input and wait, also under backpressure.
            let (_, _, status) = tokio::try_join!(detaching, draining, self.child.wait())?;
            Ok::<_, std::io::Error>(status)
        })
        .await;
        let outcome: Result<(), ClientError> = match protocol {
            Ok(Ok(status)) if status.success() => {
                self.settled();
                Ok(())
            }
            _ => {
                #[cfg(windows)]
                if let Some(backend) = self.backend.upgrade() {
                    backend.cleanup_unknown.store(true, Ordering::Release);
                }
                self.input.take();
                self.output.take();
                let _ = self.child.start_kill();
                if !matches!(
                    tokio::time::timeout(Duration::from_secs(1), self.child.wait()).await,
                    Ok(Ok(_))
                ) {
                    if let Some(backend) = self.backend.upgrade() {
                        backend.cleanup_unknown.store(true, Ordering::Release);
                    }
                    self.outcome = Some(Err("terminal attachment process did not settle".into()));
                    return Err("terminal attachment process did not settle".into());
                }
                self.settled();
                Err("terminal attachment process failed or detach exceeded deadline".into())
            }
        };
        self.outcome = Some(outcome.as_ref().map(|_| ()).map_err(ToString::to_string));
        outcome
    }
    pub fn workspace(&self) -> &str {
        &self.workspace
    }
}

impl Drop for ControlClient {
    fn drop(&mut self) {
        if !self.settled {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                self.settled();
            } else if let Some(backend) = self.backend.upgrade() {
                backend.cleanup_unknown.store(true, Ordering::Release);
            }
        }
    }
}

pub(crate) async fn read_output(
    reader: &mut BufReader<ChildStdout>,
) -> Result<Option<Vec<u8>>, ClientError> {
    loop {
        let mut line = Vec::new();
        let count = reader
            .take(MAX_CONTROL_LINE as u64 + 1)
            .read_until(b'\n', &mut line)
            .await?;
        if count == 0 {
            return Ok(None);
        }
        if line.len() > MAX_CONTROL_LINE || line.last() != Some(&b'\n') {
            return Err("terminal control line too large or truncated".into());
        }
        if line.starts_with(b"%exit") {
            return Ok(None);
        }
        if line.starts_with(b"%error") {
            return Err("terminal control operation failed".into());
        }
        let Some(output) = line.strip_prefix(b"%output ") else {
            continue;
        };
        let Some(offset) = output.iter().position(|byte| *byte == b' ') else {
            return Err("invalid tmux output record".into());
        };
        let encoded = &output[offset + 1..output.len() - 1];
        let mut bytes = Vec::with_capacity(encoded.len());
        let mut index = 0;
        while index < encoded.len() {
            if encoded[index] == b'\\' {
                let digits = encoded
                    .get(index + 1..index + 4)
                    .ok_or("truncated tmux output escape")?;
                if !digits.iter().all(|byte| matches!(byte, b'0'..=b'7')) {
                    return Err("invalid tmux output escape".into());
                }
                let number = ((digits[0] - b'0') as u16) * 64
                    + ((digits[1] - b'0') as u16) * 8
                    + (digits[2] - b'0') as u16;
                if number > 255 {
                    return Err("invalid tmux output byte".into());
                }
                bytes.push(number as u8);
                index += 4;
            } else {
                bytes.push(encoded[index]);
                index += 1;
            }
        }
        return Ok(Some(bytes));
    }
}
