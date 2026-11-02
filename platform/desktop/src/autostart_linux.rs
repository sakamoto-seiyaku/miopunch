//! Per-user XDG autostart with desktop-entry argument escaping.
use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use tauri::{
    Manager, Runtime, State,
    plugin::{Builder as PluginBuilder, TauriPlugin},
};

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);
const MAX_ENTRY: u64 = 16 * 1024;

pub struct AutoLaunchManager {
    path: PathBuf,
    entry: String,
}

impl AutoLaunchManager {
    pub fn enable(&self) -> io::Result<()> {
        let parent = self.path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "autostart directory missing")
        })?;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
        let mut temporary = None;
        for _ in 0..16 {
            let sequence = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
            let candidate = parent.join(format!(
                ".miopunch-autostart-{}-{sequence}.tmp",
                std::process::id()
            ));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&candidate)
            {
                Ok(file) => {
                    temporary = Some((candidate, file));
                    break;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        let (temporary, mut file) = temporary.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "autostart temporary path busy",
            )
        })?;
        let saved = (|| {
            file.write_all(self.entry.as_bytes())?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, &self.path)
        })();
        if saved.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        saved
    }

    pub fn disable(&self) -> io::Result<()> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    pub fn is_enabled(&self) -> io::Result<bool> {
        let file = match fs::File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        let mut bytes = Vec::new();
        file.take(MAX_ENTRY + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_ENTRY {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "autostart entry too large",
            ));
        }
        let text = String::from_utf8(bytes)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid autostart entry"))?;
        let mut desktop_entry = false;
        let mut found = false;
        let mut hidden = None;
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') {
                desktop_entry = line == "[Desktop Entry]";
                found |= desktop_entry;
                continue;
            }
            if desktop_entry {
                if let Some((key, value)) = line.split_once('=') {
                    if key.trim() == "Hidden" {
                        if hidden.is_some() {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "duplicate autostart Hidden value",
                            ));
                        }
                        hidden = Some(match value.trim() {
                            "true" => true,
                            "false" => false,
                            _ => {
                                return Err(io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    "invalid autostart Hidden value",
                                ));
                            }
                        });
                    }
                }
            }
        }
        if !found {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "autostart Desktop Entry group missing",
            ));
        }
        Ok(!hidden.unwrap_or(false))
    }
}

pub trait ManagerExt<R: Runtime> {
    fn autolaunch(&self) -> State<'_, AutoLaunchManager>;
}
impl<R: Runtime, M: Manager<R>> ManagerExt<R> for M {
    fn autolaunch(&self) -> State<'_, AutoLaunchManager> {
        self.state::<AutoLaunchManager>()
    }
}

fn quoted_argument(argument: &str) -> String {
    // Exec quoting is decoded after the desktop-entry string escape layer.
    let mut quoted = String::from("\"");
    for character in argument.chars() {
        if matches!(character, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(character);
    }
    quoted.push('"');
    let mut encoded = String::new();
    for character in quoted.chars() {
        match character {
            '\\' => encoded.push_str("\\\\"),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            _ => encoded.push(character),
        }
    }
    encoded
}

fn shell_argument(argument: &str) -> String {
    let mut quoted = String::from("'");
    for character in argument.chars() {
        match character {
            '\'' => quoted.push_str("'\\''"),
            '%' => quoted.push_str("'\"$1\"'"),
            _ => quoted.push(character),
        }
    }
    quoted.push('\'');
    quoted
}

#[derive(Default)]
pub struct Builder {
    name: String,
    args: Vec<String>,
}

impl Builder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn app_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }
    pub fn args(mut self, args: impl IntoIterator<Item = String>) -> Self {
        self.args.extend(args);
        self
    }
    pub fn build<R: Runtime>(self) -> TauriPlugin<R> {
        PluginBuilder::new("platform-autostart")
            .setup(move |app, _| {
                if self.name.is_empty()
                    || !self
                        .name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
                {
                    return Err("invalid autostart name".into());
                }
                let executable = std::env::current_exe()?;
                let executable = executable.to_str().ok_or("startup path must be Unicode")?;
                let launcher = "/bin/sh";
                if !Path::new(launcher).is_file() {
                    return Err("Linux login startup requires /bin/sh".into());
                }
                let mut arguments = vec![shell_argument(executable)];
                for argument in self.args {
                    if argument.contains('\0') {
                        return Err("invalid startup argument".into());
                    }
                    arguments.push(shell_argument(&argument));
                }
                // Launchers disagree on expanding %% in Exec. Keep that
                // character out of the entry entirely, using a private shell
                // positional parameter. Every supplied path/argument remains
                // a single-quoted literal; only our fixed printf/exec runs.
                // Unlike command substitution for whole arguments, this also
                // preserves trailing newlines and leaves the environment alone.
                let script = format!(
                    r#"set -- "$(printf '\045')"; exec {}"#,
                    arguments.join(" ")
                );
                let command = [launcher, "-c", script.as_str()]
                    .map(quoted_argument)
                    .join(" ");
                let entry = format!(
                    "[Desktop Entry]\nType=Application\nVersion=1.0\nName={}\nExec={command}\nStartupNotify=false\nTerminal=false\n",
                    self.name
                );
                if entry.len() as u64 > MAX_ENTRY {
                    return Err("autostart entry too large".into());
                }
                let path = app
                    .path()
                    .config_dir()?
                    .join("autostart")
                    .join(format!("{}.desktop", self.name));
                app.manage(AutoLaunchManager { path, entry });
                Ok(())
            })
            .build()
    }
}
