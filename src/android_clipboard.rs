//! Installed-app clipboard access, gated by the app's actual focused Activity.
use std::sync::{
    OnceLock,
    atomic::{AtomicU64, Ordering},
};

use jni::{
    JNIEnv, JavaVM,
    objects::{GlobalRef, JObject, JString, JValue},
};

use crate::clipboard::{ClipboardCapability, ClipboardError, MAX_TEXT_BYTES};

struct ApplicationContext {
    vm: JavaVM,
    application: GlobalRef,
}
static CONTEXT: OnceLock<ApplicationContext> = OnceLock::new();
static FOCUSED_WINDOW: AtomicU64 = AtomicU64::new(0);
const BACKEND: &str = "android/ClipboardManager";

pub(crate) fn install_context(
    env: &mut JNIEnv<'_>,
    application: &JObject<'_>,
) -> jni::errors::Result<()> {
    if CONTEXT.get().is_none() {
        let context = ApplicationContext {
            vm: env.get_java_vm()?,
            application: env.new_global_ref(application)?,
        };
        let _ = CONTEXT.set(context);
    }
    Ok(())
}

pub(crate) fn window_focus(owner: u64, focused: bool) {
    if owner == 0 {
        return;
    }
    if focused {
        FOCUSED_WINDOW.store(owner, Ordering::SeqCst);
    } else {
        // A retiring Activity must not clear a newer window's focus.
        let _ = FOCUSED_WINDOW.compare_exchange(owner, 0, Ordering::SeqCst, Ordering::SeqCst);
    }
}

fn require_foreground() -> Result<(), ClipboardError> {
    if FOCUSED_WINDOW.load(Ordering::SeqCst) == 0 {
        Err(ClipboardError::Unavailable)
    } else {
        Ok(())
    }
}

enum BridgeError {
    Jni,
    Public(ClipboardError),
}
impl From<jni::errors::Error> for BridgeError {
    fn from(_: jni::errors::Error) -> Self {
        Self::Jni
    }
}

pub struct SystemClipboard {
    context: &'static ApplicationContext,
}

impl SystemClipboard {
    pub fn detect() -> Result<Self, ClipboardError> {
        Ok(Self {
            context: CONTEXT.get().ok_or(ClipboardError::Unavailable)?,
        })
    }

    pub fn capability(&self) -> ClipboardCapability {
        if require_foreground().is_ok() {
            ClipboardCapability::available(BACKEND)
        } else {
            ClipboardCapability::unavailable(BACKEND, "focused_activity_required")
        }
    }

    fn with_manager<T>(
        &self,
        failure: ClipboardError,
        operation: impl FnOnce(&mut JNIEnv<'_>, JObject<'_>) -> Result<T, BridgeError>,
    ) -> Result<T, ClipboardError> {
        require_foreground()?;
        let mut env = self
            .context
            .vm
            .attach_current_thread()
            .map_err(|_| failure)?;
        let result = env.with_local_frame(32, |env| {
            let name = env.new_string("clipboard")?;
            let manager = env
                .call_method(
                    self.context.application.as_obj(),
                    "getSystemService",
                    "(Ljava/lang/String;)Ljava/lang/Object;",
                    &[JValue::Object(name.as_ref())],
                )?
                .l()?;
            if manager.is_null() {
                return Err(BridgeError::Public(ClipboardError::Unavailable));
            }
            require_foreground().map_err(BridgeError::Public)?;
            operation(env, manager)
        });
        // A denied framework call must not poison this attached SDK worker or
        // put exception details (potentially including data) into diagnostics.
        if env.exception_check().unwrap_or(true) {
            let _ = env.exception_clear();
            return Err(failure);
        }
        result.map_err(|error| match error {
            BridgeError::Public(error) => error,
            BridgeError::Jni => failure,
        })
    }

    pub async fn probe(&self) -> Result<(), ClipboardError> {
        self.with_manager(ClipboardError::Unavailable, |_, _| Ok(()))
    }

    pub async fn read(&self) -> Result<Option<String>, ClipboardError> {
        let text = self.with_manager(ClipboardError::Read, |env, manager| {
            let clip = env
                .call_method(
                    &manager,
                    "getPrimaryClip",
                    "()Landroid/content/ClipData;",
                    &[],
                )?
                .l()?;
            if clip.is_null() || env.call_method(&clip, "getItemCount", "()I", &[])?.i()? != 1 {
                return Ok(None);
            }
            let item = env
                .call_method(
                    &clip,
                    "getItemAt",
                    "(I)Landroid/content/ClipData$Item;",
                    &[JValue::Int(0)],
                )?
                .l()?;
            let text = env
                .call_method(item, "getText", "()Ljava/lang/CharSequence;", &[])?
                .l()?;
            if text.is_null() {
                return Ok(None);
            }
            let units = env.call_method(&text, "length", "()I", &[])?.i()?;
            if units < 0 || units as usize > MAX_TEXT_BYTES {
                return Err(BridgeError::Public(ClipboardError::TooLarge));
            }
            let string = JString::from(
                env.call_method(text, "toString", "()Ljava/lang/String;", &[])?
                    .l()?,
            );
            let text: String = env.get_string(&string)?.into();
            if text.len() > MAX_TEXT_BYTES {
                return Err(BridgeError::Public(ClipboardError::TooLarge));
            }
            Ok(Some(text))
        })?;
        // Do not publish a sample obtained while the Activity lost focus.
        require_foreground()?;
        Ok(text)
    }

    pub async fn write(&self, text: &str) -> Result<(), ClipboardError> {
        if text.len() > MAX_TEXT_BYTES {
            return Err(ClipboardError::TooLarge);
        }
        self.with_manager(ClipboardError::Write, |env, manager| {
            let label = env.new_string("miopunch")?;
            let value = env.new_string(text)?;
            let clip = env
                .call_static_method(
                    "android/content/ClipData",
                    "newPlainText",
                    "(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/ClipData;",
                    &[
                        JValue::Object(label.as_ref()),
                        JValue::Object(value.as_ref()),
                    ],
                )?
                .l()?;
            require_foreground().map_err(BridgeError::Public)?;
            env.call_method(
                manager,
                "setPrimaryClip",
                "(Landroid/content/ClipData;)V",
                &[JValue::Object(&clip)],
            )?;
            Ok(())
        })
    }

    pub async fn close(&self) {}
}
