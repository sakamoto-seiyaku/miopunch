//! Per-user Windows login startup with a correctly quoted command line.
//! Uses only this application's values under HKCU; it never modifies HKLM.
use std::io;
use tauri::{
    Manager, Runtime, State,
    plugin::{Builder as PluginBuilder, TauriPlugin},
};
use windows_registry::CURRENT_USER;

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const NOT_FOUND: i32 = 0x80070002u32 as i32;

pub struct AutoLaunchManager {
    name: String,
    command: String,
}

impl AutoLaunchManager {
    pub fn enable(&self) -> io::Result<()> {
        // Preserve the existing auto-launch 0.6.0 integration with Task
        // Manager, limited to the current user's value for this application.
        let approved = match CURRENT_USER.options().write().open(APPROVED) {
            Ok(key) => Some(key),
            Err(error) if error.code().0 == NOT_FOUND => None,
            Err(error) => return Err(error.into()),
        };
        let run = CURRENT_USER.create(RUN)?;
        let previous = match run.get_string(&self.name) {
            Ok(value) => Some(value),
            Err(error) if error.code().0 == NOT_FOUND => None,
            Err(error) => return Err(error.into()),
        };
        run.set_string(&self.name, &self.command)?;
        if let Some(approved) = approved {
            if let Err(error) = approved.set_bytes(
                &self.name,
                windows_registry::Type::Bytes,
                &[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            ) {
                let rollback = match previous {
                    Some(value) => run.set_string(&self.name, value),
                    None => run.remove_value(&self.name),
                };
                if rollback.is_err() {
                    return Err(io::Error::other(
                        "startup approval failed and registration rollback failed",
                    ));
                }
                return Err(error.into());
            }
        }
        Ok(())
    }

    pub fn disable(&self) -> io::Result<()> {
        match CURRENT_USER
            .options()
            .write()
            .open(RUN)
            .and_then(|key| key.remove_value(&self.name))
        {
            Ok(()) => Ok(()),
            Err(error) if error.code().0 == NOT_FOUND => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn is_enabled(&self) -> io::Result<bool> {
        match CURRENT_USER
            .open(RUN)
            .and_then(|key| key.get_string(&self.name))
        {
            Ok(_) => {}
            Err(error) if error.code().0 == NOT_FOUND => return Ok(false),
            Err(error) => return Err(error.into()),
        }
        match CURRENT_USER
            .open(APPROVED)
            .and_then(|key| key.get_value(&self.name))
        {
            // Same compatibility rule as the pinned auto-launch adapter:
            // disabled entries contain a nonzero timestamp in the last 8 bytes.
            Ok(bytes) if bytes.len() >= 8 => Ok(bytes.iter().rev().take(8).all(|byte| *byte == 0)),
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid Windows startup approval",
            )),
            Err(error) if error.code().0 == NOT_FOUND => Ok(true),
            Err(error) => Err(error.into()),
        }
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

/// Windows argument quoting: backslashes before a quote or the closing quote
/// are doubled. Other characters are literal; no command shell is involved.
fn quote(argument: &str) -> String {
    let mut result = String::from("\"");
    let mut slashes = 0;
    for character in argument.chars() {
        if character == '\\' {
            slashes += 1;
            continue;
        }
        result.extend(std::iter::repeat_n(
            '\\',
            if character == '"' {
                slashes * 2 + 1
            } else {
                slashes
            },
        ));
        slashes = 0;
        result.push(character);
    }
    result.extend(std::iter::repeat_n('\\', slashes * 2));
    result.push('"');
    result
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
                let executable = std::env::current_exe()?;
                let executable = executable.to_str().ok_or("startup path must be Unicode")?;
                let mut arguments = vec![quote(executable)];
                for argument in self.args {
                    if argument.contains('\0') {
                        return Err("invalid startup argument".into());
                    }
                    arguments.push(quote(&argument));
                }
                let command = arguments.join(" ");
                if command.encode_utf16().count() > 260 {
                    return Err("Windows login startup command exceeds the Run key limit".into());
                }
                app.manage(AutoLaunchManager {
                    name: self.name,
                    command,
                });
                Ok(())
            })
            .build()
    }
}
