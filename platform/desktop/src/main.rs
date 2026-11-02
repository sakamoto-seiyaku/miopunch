#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use miopunch_native_probe::{
    client_commands::save_private,
    client_host::{ClientHost, StartMode},
    resources::ResourceConfig,
};
use serde_json::{Value, json};
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
use tauri::{AppHandle, Manager, State, WebviewWindow};
#[cfg(target_os = "linux")]
mod autostart_linux;
#[cfg(windows)]
mod autostart_windows;
use autostart::ManagerExt;
#[cfg(target_os = "linux")]
use autostart_linux as autostart;
#[cfg(windows)]
use autostart_windows as autostart;
#[cfg(not(any(windows, target_os = "linux")))]
use tauri_plugin_autostart as autostart;
use tokio::sync::{Semaphore, oneshot};

#[derive(Default)]
struct Options {
    directory: Option<PathBuf>,
    background: bool,
    verification: Option<PathBuf>,
}

impl Options {
    fn read() -> Result<Self, String> {
        let mut result = Self::default();
        let mut args = std::env::args_os().skip(1);
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--data-dir") => {
                    result.directory = Some(args.next().ok_or("missing data directory")?.into())
                }
                Some("--verification-dir") => {
                    result.verification =
                        Some(args.next().ok_or("missing verification directory")?.into())
                }
                Some("--background") => result.background = true,
                _ => return Err("unknown desktop host argument".into()),
            }
        }
        if let Some(path) = &result.verification {
            let metadata = path
                .metadata()
                .map_err(|_| "verification directory must already exist")?;
            if !metadata.is_dir() {
                return Err("verification directory is not a directory".into());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if metadata.mode() & 0o077 != 0 {
                    return Err("verification directory must be private".into());
                }
            }
        }
        if let Some(path) = &mut result.directory {
            *path = if path.exists() {
                path.canonicalize()
            } else {
                std::path::absolute(&*path)
            }
            .map_err(|_| "invalid data directory")?;
        }
        if let Some(path) = &mut result.verification {
            *path = path
                .canonicalize()
                .map_err(|_| "invalid verification directory")?;
        }
        Ok(result)
    }
}

struct AppState {
    host: Arc<ClientHost>,
    directory: PathBuf,
    normal: Arc<Semaphore>,
    lifecycle: Arc<Semaphore>,
    opening: AtomicBool,
    quitting: AtomicBool,
    exit_ready: AtomicBool,
    startup_error: Mutex<Option<&'static str>>,
    verification: Option<PathBuf>,
    issued: AtomicU64,
    event_lock: Mutex<()>,
    windows_created: AtomicU64,
    windows_ready: AtomicU64,
}

impl AppState {
    fn event(&self, name: &str, detail: Value) {
        let Some(directory) = &self.verification else {
            return;
        };
        let _guard = self.event_lock.lock().unwrap();
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        if let Ok(mut file) = options.open(directory.join("events.jsonl")) {
            let _ = writeln!(
                file,
                "{}",
                json!({"event":name,"pid":std::process::id(),"detail":detail})
            );
            let _ = file.sync_all();
        }
    }

    fn verification_result(&self, id: u64, result: &Value) -> Result<(), String> {
        let directory = self
            .verification
            .as_ref()
            .ok_or("verification is disabled")?;
        if id == 0 || id != self.issued.load(Ordering::SeqCst) {
            return Err("unissued verification request".into());
        }
        let path = directory.join(format!("result-{id}.json"));
        save_private(
            path.to_str().ok_or("invalid result path")?,
            &serde_json::to_vec(result).map_err(|_| "invalid result")?,
        )
        .map_err(|_| "verification result could not be saved")?;
        Ok(())
    }
}

// SDK futures are constructed and polled on bounded Rust-owned 8MiB stacks.
// This also avoids depending on Windows' main/WebView thread stack size.
async fn native_work<T: Send + 'static, F: FnOnce() -> T + Send + 'static>(
    slots: Arc<Semaphore>,
    operation: F,
) -> Result<T, String> {
    let permit = slots
        .acquire_owned()
        .await
        .map_err(|_| "native host closed")?;
    let (sender, receiver) = oneshot::channel();
    std::thread::Builder::new()
        .name("miopunch-desktop-node".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let _permit = permit;
            let _ = sender.send(operation());
        })
        .map_err(|_| "native worker unavailable")?;
    receiver.await.map_err(|_| "native worker failed".into())
}

#[tauri::command]
async fn host_request(state: State<'_, Arc<AppState>>, input: String) -> Result<Value, String> {
    if input.len() > 65536 {
        return Err("command too large".into());
    }
    let lifecycle = serde_json::from_str::<Value>(&input)
        .ok()
        .is_some_and(|v| matches!(v["op"].as_str(), Some("stop" | "replacement_prepare")));
    let slots = if lifecycle {
        state.lifecycle.clone()
    } else {
        state.normal.clone()
    };
    let host = state.host.clone();
    native_work(slots, move || {
        tauri::async_runtime::block_on(host.request(&input))
    })
    .await
}

#[tauri::command]
async fn host_start(
    state: State<'_, Arc<AppState>>,
    mode: Value,
    resources: Option<Value>,
) -> Result<Value, String> {
    let mode: StartMode = serde_json::from_value(mode).map_err(|_| "invalid start mode")?;
    let resources: ResourceConfig = resources
        .map(serde_json::from_value)
        .transpose()
        .map_err(|_| "invalid resources")?
        .unwrap_or_default();
    let (host, path, owner) = (
        state.host.clone(),
        state.directory.clone(),
        state.inner().clone(),
    );
    native_work(state.lifecycle.clone(), move || {
        if owner.quitting.load(Ordering::SeqCst) {
            return Err("host is quitting".to_string());
        }
        tauri::async_runtime::block_on(host.start(path, mode, resources))
            .map_err(|_| "node start failed".to_string())
    })
    .await?
}

#[tauri::command]
async fn host_state(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<Value, String> {
    let host = state.host.clone();
    let node = native_work(state.normal.clone(), move || {
        tauri::async_runtime::block_on(host.request(r#"{"id":0,"op":"status"}"#))
    })
    .await?;
    Ok(json!({
        "pid": std::process::id(), "node": node,
        "opening": state.opening.load(Ordering::SeqCst),
        "startup_error": *state.startup_error.lock().unwrap(),
        "windows": app.webview_windows().keys().cloned().collect::<Vec<_>>(),
        "windows_created": state.windows_created.load(Ordering::SeqCst),
        "windows_ready": state.windows_ready.load(Ordering::SeqCst),
        "autostart": app.autolaunch().is_enabled().map_err(|_| "autostart status unavailable")?,
    }))
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<bool, String> {
    if enabled {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    }
    .map_err(|_| "autostart setting failed")?;
    let actual = app
        .autolaunch()
        .is_enabled()
        .map_err(|_| "autostart status unavailable")?;
    if actual != enabled {
        return Err("autostart setting did not persist".into());
    }
    Ok(actual)
}

#[tauri::command]
fn window_ready(
    window: WebviewWindow,
    state: State<'_, Arc<AppState>>,
    generation: String,
) -> Result<(), String> {
    if window.label() != "main" || generation.len() > 128 {
        return Err("invalid window".into());
    }
    let count = state.windows_ready.fetch_add(1, Ordering::SeqCst) + 1;
    state.event(
        "webview_ready",
        json!({"label":window.label(),"generation":generation,"count":count}),
    );
    Ok(())
}

#[tauri::command]
fn verification_next(state: State<'_, Arc<AppState>>) -> Result<Option<Value>, String> {
    let Some(directory) = &state.verification else {
        return Ok(None);
    };
    let path = directory.join("request.json");
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("verification request unavailable".into()),
    };
    let mut bytes = Vec::new();
    file.take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| "verification request unavailable")?;
    if bytes.len() > 65536 {
        return Err("verification request too large".into());
    }
    let request: Value =
        serde_json::from_slice(&bytes).map_err(|_| "invalid verification request")?;
    let id = request["id"].as_u64().ok_or("invalid verification id")?;
    let previous = state.issued.load(Ordering::SeqCst);
    if id <= previous {
        return Ok(None);
    }
    state
        .issued
        .compare_exchange(previous, id, Ordering::SeqCst, Ordering::SeqCst)
        .map_err(|_| "concurrent verification request")?;
    Ok(Some(request))
}

#[tauri::command]
fn verification_result(
    state: State<'_, Arc<AppState>>,
    id: u64,
    result: Value,
) -> Result<(), String> {
    state.verification_result(id, &result)
}

#[tauri::command]
fn close_window(
    window: WebviewWindow,
    state: State<'_, Arc<AppState>>,
    verification_id: Option<u64>,
) -> Result<(), String> {
    if let Some(id) = verification_id {
        state.verification_result(id, &json!({"ok":true,"close_requested":true}))?;
    }
    window.close().map_err(|_| "window close failed".into())
}

async fn shutdown(
    app: AppHandle,
    state: Arc<AppState>,
    verification_id: Option<u64>,
) -> Result<(), String> {
    if state.quitting.swap(true, Ordering::SeqCst) {
        return Err("quit already requested".into());
    }
    let host = state.host.clone();
    let result = native_work(state.lifecycle.clone(), move || {
        tauri::async_runtime::block_on(host.stop()).map_err(|_| "node close failed".to_string())
    })
    .await
    .and_then(|v| v);
    let receipt = verification_id
        .map(|id| {
            state.verification_result(id, &json!({"ok":result.is_ok(),"stopped":result.is_ok()}))
        })
        .transpose();
    state.event("quit_completed", json!({"sdk_closed":result.is_ok()}));
    state.exit_ready.store(true, Ordering::SeqCst);
    app.exit(if result.is_ok() && receipt.is_ok() {
        0
    } else {
        1
    });
    receipt?;
    result
}

#[tauri::command]
async fn quit(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    verification_id: Option<u64>,
) -> Result<(), String> {
    shutdown(app, state.inner().clone(), verification_id).await
}

fn show_window(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("main") {
        window.show()?;
        window.unminimize()?;
        window.set_focus()?;
    } else {
        tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?
            .on_navigation(|url| {
                (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
                    || (matches!(url.scheme(), "http" | "https")
                        && url.host_str() == Some("tauri.localhost"))
            })
            .build()?;
        let state = app.state::<Arc<AppState>>();
        let count = state.windows_created.fetch_add(1, Ordering::SeqCst) + 1;
        state.event("window_created", json!({"count":count}));
    }
    Ok(())
}

fn initialize_autostart(
    app: &AppHandle,
    directory: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut name = directory
        .file_name()
        .ok_or("node directory has no name")?
        .to_os_string();
    name.push(".host-settings.json");
    let path = directory.with_file_name(name);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let value: Value = serde_json::from_slice(&bytes)?;
            if value != json!({"autostart_initialized":true,"version":1}) {
                return Err("invalid host settings".into());
            }
            // Subsequent starts respect the actual saved OS setting, including
            // disabling it outside this application.
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let previous = app.autolaunch().is_enabled()?;
            app.autolaunch().enable()?;
            let saved = save_private(
                path.to_str().ok_or("invalid settings path")?,
                br#"{"autostart_initialized":true,"version":1}"#,
            );
            if saved.is_err() {
                if !previous {
                    app.autolaunch().disable()?;
                }
                return Err("autostart initialization could not be saved".into());
            }
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn main() {
    let options = Options::read().expect("desktop host options");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(4)
        .thread_stack_size(8 * 1024 * 1024)
        .enable_all()
        .build()
        .expect("native runtime");
    tauri::async_runtime::set(runtime.handle().clone());
    let mut autostart = autostart::Builder::new()
        .app_name("miopunch")
        .arg("--background");
    if let Some(directory) = &options.directory {
        autostart = autostart.args([
            "--data-dir".to_string(),
            directory.to_string_lossy().into_owned(),
        ]);
    }
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _| {
            let background = args.iter().any(|arg| arg == "--background");
            if let Some(state) = app.try_state::<Arc<AppState>>() {
                state.event("secondary_launch", json!({"background":background}));
            }
            if !background {
                let _ = show_window(app);
            }
        }))
        .plugin(autostart.build())
        .invoke_handler(tauri::generate_handler![
            host_request,
            host_start,
            host_state,
            set_autostart,
            window_ready,
            verification_next,
            verification_result,
            close_window,
            quit
        ])
        .setup(move |app| {
            let requested = options
                .directory
                .unwrap_or(app.path().app_data_dir()?.join("node"));
            let directory = if requested.exists() {
                requested.canonicalize()?
            } else {
                let parent = requested
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(Path::new("."));
                std::fs::create_dir_all(parent)?;
                parent
                    .canonicalize()?
                    .join(requested.file_name().ok_or("node directory has no name")?)
            };
            let state = Arc::new(AppState {
                host: Arc::new(ClientHost::default()),
                directory,
                normal: Arc::new(Semaphore::new(16)),
                lifecycle: Arc::new(Semaphore::new(1)),
                opening: AtomicBool::new(false),
                quitting: AtomicBool::new(false),
                exit_ready: AtomicBool::new(false),
                startup_error: Mutex::new(None),
                verification: options.verification,
                issued: AtomicU64::new(0),
                event_lock: Mutex::new(()),
                windows_created: AtomicU64::new(0),
                windows_ready: AtomicU64::new(0),
            });
            app.manage(state.clone());
            initialize_autostart(app.handle(), &state.directory)?;
            state.event(
                "host_started",
                json!({"background":options.background,"autostart":app.autolaunch().is_enabled()?}),
            );
            if state.directory.join("node.key").exists() {
                state.opening.store(true, Ordering::SeqCst);
                let owner = state.clone();
                tauri::async_runtime::spawn(async move {
                    let (host, directory, guard) =
                        (owner.host.clone(), owner.directory.clone(), owner.clone());
                    let result = native_work(owner.lifecycle.clone(), move || {
                        if guard.quitting.load(Ordering::SeqCst) {
                            return Err(());
                        }
                        tauri::async_runtime::block_on(host.start(
                            directory,
                            StartMode::Open,
                            ResourceConfig::default(),
                        ))
                        .map_err(|_| ())
                    })
                    .await;
                    let succeeded = matches!(result, Ok(Ok(_)));
                    if !succeeded {
                        *owner.startup_error.lock().unwrap() = Some("node_open_failed");
                    }
                    owner.opening.store(false, Ordering::SeqCst);
                    owner.event("node_opened", json!({"ok":succeeded}));
                });
            }
            if !options.background {
                show_window(app.handle())?;
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let Some(state) = window.try_state::<Arc<AppState>>() {
                match event {
                    tauri::WindowEvent::CloseRequested { .. } => {
                        state.event("window_close_requested", json!({"label":window.label()}))
                    }
                    tauri::WindowEvent::Destroyed => {
                        state.event("window_destroyed", json!({"label":window.label()}))
                    }
                    _ => {}
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("desktop host setup");
    app.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
            let state = app.state::<Arc<AppState>>();
            if !state.exit_ready.load(Ordering::SeqCst) {
                api.prevent_exit();
                if code.is_some() && !state.quitting.load(Ordering::SeqCst) {
                    let (app, state) = (app.clone(), state.inner().clone());
                    tauri::async_runtime::spawn(async move {
                        let _ = shutdown(app, state, None).await;
                    });
                }
            }
        }
    });
}
