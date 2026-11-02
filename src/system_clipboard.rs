//! Actual native system text clipboard. No fallback memory clipboard exists.
use crate::clipboard::{ClipboardCapability, ClipboardError, MAX_TEXT_BYTES};

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::{path::PathBuf, process::Stdio, time::Duration};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        process::{Child, Command},
        sync::Mutex,
    };

    enum Backend {
        Wayland { copy: PathBuf, paste: PathBuf },
        X11(PathBuf),
    }
    pub struct SystemClipboard {
        backend: Backend,
        owner: Mutex<Option<Child>>,
    }
    fn executable(name: &str) -> Option<PathBuf> {
        use std::os::unix::fs::PermissionsExt;
        std::env::split_paths(&std::env::var_os("PATH")?).find_map(|directory| {
            let path = directory.join(name);
            path.metadata()
                .ok()
                .filter(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                .map(|_| path)
        })
    }
    impl SystemClipboard {
        pub fn detect() -> Result<Self, ClipboardError> {
            let backend = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
                executable("wl-copy")
                    .zip(executable("wl-paste"))
                    .map(|(copy, paste)| Backend::Wayland { copy, paste })
            } else {
                None
            }
            .or_else(|| {
                std::env::var_os("DISPLAY").and_then(|_| executable("xclip").map(Backend::X11))
            })
            .ok_or(ClipboardError::Unavailable)?;
            Ok(Self {
                backend,
                owner: Mutex::new(None),
            })
        }
        pub fn capability(&self) -> ClipboardCapability {
            ClipboardCapability::available(match self.backend {
                Backend::Wayland { .. } => "wayland/wl-clipboard",
                Backend::X11(_) => "x11/xclip",
            })
        }
        async fn read_selection(
            &self,
            formats_only: bool,
        ) -> Result<Option<String>, ClipboardError> {
            let limit = if formats_only { 4096 } else { MAX_TEXT_BYTES };
            let mut command = match &self.backend {
                Backend::Wayland { paste, .. } => {
                    let mut command = Command::new(paste);
                    if formats_only {
                        command.arg("--list-types");
                    } else {
                        command.args(["--no-newline", "--type", "text"]);
                    }
                    command
                }
                Backend::X11(path) => {
                    let mut command = Command::new(path);
                    command.args([
                        "-selection",
                        "clipboard",
                        "-out",
                        "-target",
                        if formats_only {
                            "TARGETS"
                        } else {
                            "UTF8_STRING"
                        },
                    ]);
                    command
                }
            };
            command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            let mut child = command.spawn().map_err(|_| ClipboardError::Unavailable)?;
            let mut stdout = child.stdout.take().ok_or(ClipboardError::Read)?;
            let mut stderr = child.stderr.take().ok_or(ClipboardError::Read)?;
            let work = async {
                let mut bytes = Vec::new();
                let mut errors = Vec::new();
                let mut bounded_stdout = (&mut stdout).take((limit + 1) as u64);
                let mut bounded_stderr = (&mut stderr).take(4097);
                let (output, error_output) = tokio::join!(
                    bounded_stdout.read_to_end(&mut bytes),
                    bounded_stderr.read_to_end(&mut errors)
                );
                output.map_err(|_| ClipboardError::Read)?;
                error_output.map_err(|_| ClipboardError::Read)?;
                if bytes.len() > limit {
                    return Err(ClipboardError::TooLarge);
                }
                if errors.len() > 4096 {
                    return Err(ClipboardError::Read);
                }
                let status = child.wait().await.map_err(|_| ClipboardError::Read)?;
                if !status.success() {
                    // Tool diagnostics contain no clipboard contents. Inspect
                    // only the documented missing-format/empty-selection cases;
                    // unavailable display/permission is never called empty text.
                    let diagnostic = String::from_utf8_lossy(&errors);
                    if diagnostic.contains("target STRING not available")
                        || diagnostic.contains("target UTF8_STRING not available")
                        || diagnostic.contains("target TARGETS not available")
                        || diagnostic.contains("Nothing is copied")
                        || diagnostic.contains("No selection")
                    {
                        return Ok(None);
                    }
                    return Err(ClipboardError::Unavailable);
                }
                if formats_only {
                    Ok(None)
                } else {
                    Ok(Some(
                        String::from_utf8(bytes).map_err(|_| ClipboardError::Read)?,
                    ))
                }
            };
            let result = match tokio::time::timeout(Duration::from_secs(2), work).await {
                Ok(result) => result,
                Err(_) => Err(ClipboardError::Read),
            };
            if result.is_err() {
                let _ = child.kill().await;
            }
            result
        }
        pub async fn read(&self) -> Result<Option<String>, ClipboardError> {
            self.read_selection(false).await
        }
        /// Probe the live display/tool using format metadata, never its text.
        pub async fn probe(&self) -> Result<(), ClipboardError> {
            self.read_selection(true).await.map(|_| ())
        }
        pub async fn write(&self, text: &str) -> Result<(), ClipboardError> {
            if text.len() > MAX_TEXT_BYTES {
                return Err(ClipboardError::TooLarge);
            }
            let mut command = match &self.backend {
                Backend::Wayland { copy, .. } => {
                    let mut command = Command::new(copy);
                    command.args(["--foreground", "--type", "text/plain;charset=utf-8"]);
                    command
                }
                Backend::X11(path) => {
                    let mut command = Command::new(path);
                    command.args([
                        "-selection",
                        "clipboard",
                        "-in",
                        "-target",
                        "UTF8_STRING",
                        "-quiet",
                    ]);
                    command
                }
            };
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            let mut child = command.spawn().map_err(|_| ClipboardError::Write)?;
            let work = async {
                let mut input = child.stdin.take().ok_or(ClipboardError::Write)?;
                input
                    .write_all(text.as_bytes())
                    .await
                    .map_err(|_| ClipboardError::Write)?;
                input.shutdown().await.map_err(|_| ClipboardError::Write)?;
                drop(input);
                // Foreground selection owners must remain alive. Independent
                // system readback, rather than process spawn, proves publication.
                for _ in 0..10 {
                    if self.read().await?.as_deref() == Some(text) {
                        return Ok(());
                    }
                    if child
                        .try_wait()
                        .map_err(|_| ClipboardError::Write)?
                        .is_some()
                    {
                        return Err(ClipboardError::Write);
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                Err(ClipboardError::Write)
            };
            let result = tokio::time::timeout(Duration::from_secs(3), work)
                .await
                .unwrap_or(Err(ClipboardError::Write));
            if result.is_err() {
                let _ = child.kill().await;
                return result;
            }
            let previous = self.owner.lock().await.replace(child);
            if let Some(mut previous) = previous {
                if previous.try_wait().ok().flatten().is_none() {
                    let _ = previous.kill().await;
                } else {
                    let _ = previous.wait().await;
                }
            }
            Ok(())
        }
        pub async fn close(&self) {
            if let Some(mut owner) = self.owner.lock().await.take() {
                if owner.try_wait().ok().flatten().is_none() {
                    let _ = owner.kill().await;
                } else {
                    let _ = owner.wait().await;
                }
            }
        }
    }
}
#[cfg(target_os = "linux")]
pub use linux::SystemClipboard;

#[cfg(target_os = "windows")]
#[path = "windows_clipboard.rs"]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::SystemClipboard;

#[cfg(target_os = "android")]
#[path = "android_clipboard.rs"]
mod android;
#[cfg(target_os = "android")]
pub use android::SystemClipboard;
#[cfg(target_os = "android")]
pub(crate) use android::{
    install_context as install_android, window_focus as android_window_focus,
};

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "android")))]
pub struct SystemClipboard;
#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "android")))]
impl SystemClipboard {
    pub fn detect() -> Result<Self, ClipboardError> {
        Err(ClipboardError::Unavailable)
    }
    pub fn capability(&self) -> ClipboardCapability {
        ClipboardCapability::unavailable("native", "platform_adapter_unavailable")
    }
    pub async fn read(&self) -> Result<Option<String>, ClipboardError> {
        Err(ClipboardError::Unavailable)
    }
    pub async fn write(&self, _: &str) -> Result<(), ClipboardError> {
        Err(ClipboardError::Unavailable)
    }
    pub async fn probe(&self) -> Result<(), ClipboardError> {
        Err(ClipboardError::Unavailable)
    }
    pub async fn close(&self) {}
}

pub fn capability() -> ClipboardCapability {
    SystemClipboard::detect()
        .map(|system| system.capability())
        .unwrap_or_else(|_| {
            ClipboardCapability::unavailable("native", "desktop_or_clipboard_tool_unavailable")
        })
}

/// Current native platform capability, without reading clipboard text.
pub async fn current_capability() -> ClipboardCapability {
    match SystemClipboard::detect() {
        Ok(platform) => match platform.probe().await {
            Ok(()) => platform.capability(),
            Err(failure) => {
                ClipboardCapability::unavailable(platform.capability().backend, failure.to_string())
            }
        },
        Err(failure) => ClipboardCapability::unavailable("native", failure.to_string()),
    }
}
