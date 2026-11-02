//! Application/Service-owned JNI adapter. Activity recreation does not own a node.
use std::sync::{Arc, Mutex, OnceLock};

use jni::{
    JNIEnv,
    objects::{GlobalRef, JClass, JObject, JString},
    sys::{jboolean, jlong, jstring},
};
use serde_json::{Value, json};
use tokio::sync::Semaphore;

use crate::client_host::ClientHost;

const WORKER_STACK: usize = 8 * 1024 * 1024;
const ACTIVE_COMMANDS: usize = 16;

struct ApplicationRuntime {
    runtime: tokio::runtime::Runtime,
    host: ClientHost,
    commands: Arc<Semaphore>,
    stopping: Arc<Semaphore>,
}
static APPLICATION: OnceLock<Mutex<Option<Arc<ApplicationRuntime>>>> = OnceLock::new();
// ndk-context requires both raw pointers to outlive every process-wide DNS worker,
// including a failed Runtime construction followed by another initialize call.
static CONTEXT: OnceLock<GlobalRef> = OnceLock::new();

enum Operation {
    Start {
        directory: String,
        mode: String,
        resources: String,
    },
    Request(String),
    Stop,
}

impl ApplicationRuntime {
    fn execute(self: Arc<Self>, operation: Operation) -> Result<Value, Box<dyn std::error::Error>> {
        let slots = if matches!(operation, Operation::Stop) {
            &self.stopping
        } else {
            &self.commands
        };
        let permit = slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| "host is busy")?;
        // Constructing as well as polling a large SDK future must happen on
        // the owned Rust stack. Merely spawning a future constructed on the
        // ~1 MiB ART caller stack still leaves the original overflow possible.
        let worker = std::thread::Builder::new()
            .name("miopunch-command".into())
            .stack_size(WORKER_STACK)
            .spawn(move || {
                let _permit = permit;
                let result: Result<Value, crate::native_client::ClientError> =
                    self.runtime.block_on(async {
                        match operation {
                            Operation::Start {
                                directory,
                                mode,
                                resources,
                            } => {
                                let mode = serde_json::from_str(&mode)?;
                                let resources = serde_json::from_str::<
                                    Option<crate::resources::ResourceConfig>,
                                >(&resources)?
                                .unwrap_or_default();
                                let value = self.host.start(directory, mode, resources).await?;
                                Ok(json!({"ok":true,"result":value}))
                            }
                            Operation::Request(input) => Ok(self.host.request(&input).await),
                            Operation::Stop => {
                                self.host.stop().await?;
                                Ok(json!({"ok":true,"stopped":true}))
                            }
                        }
                    });
                result
            })?;
        worker
            .join()
            .map_err(|_| "host worker panicked")?
            .map_err(|_| "host operation failed".into())
    }
}

fn application() -> Result<Arc<ApplicationRuntime>, Box<dyn std::error::Error>> {
    APPLICATION
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "application runtime lock unavailable")?
        .clone()
        .ok_or_else(|| "application context is not initialized".into())
}

fn reply(
    env: &mut JNIEnv<'_>,
    operation: impl FnOnce(&mut JNIEnv<'_>) -> Result<Value, Box<dyn std::error::Error>>,
) -> jstring {
    let value = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| operation(env))) {
        Ok(Ok(value)) => value,
        _ => json!({"ok":false,"error":"host_operation_failed"}),
    };
    match env.new_string(value.to_string()) {
        Ok(text) => text.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_miopunch_host_NativeHost_initialize(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    context: JObject<'_>,
) -> jstring {
    reply(&mut env, |env| {
        let mut application = APPLICATION
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| "application runtime lock unavailable")?;
        if application.is_none() {
            if CONTEXT.get().is_none() {
                CONTEXT
                    .set(env.new_global_ref(context)?)
                    .map_err(|_| "context already initialized")?;
                let vm = env.get_java_vm()?;
                // Android owns the VM; CONTEXT retains the application object
                // until process exit. SDK worker threads attach through JNI.
                unsafe {
                    iroh::dns::install_android_jni_context(
                        vm.get_java_vm_pointer().cast(),
                        CONTEXT
                            .get()
                            .expect("installed context")
                            .as_obj()
                            .as_raw()
                            .cast(),
                    );
                }
            }
            crate::system_clipboard::install_android(
                env,
                CONTEXT.get().expect("installed context").as_obj(),
            )?;
            *application = Some(Arc::new(ApplicationRuntime {
                runtime: tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .max_blocking_threads(4)
                    .thread_stack_size(WORKER_STACK)
                    .enable_all()
                    .build()?,
                host: ClientHost::default(),
                commands: Arc::new(Semaphore::new(ACTIVE_COMMANDS)),
                // Normal commands cannot consume the explicit-stop slot.
                stopping: Arc::new(Semaphore::new(1)),
            }));
        }
        Ok(json!({"ok":true,"initialized":true}))
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_miopunch_host_NativeHost_clipboardFocus(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    owner: jlong,
    focused: jboolean,
) {
    if owner > 0 {
        crate::system_clipboard::android_window_focus(owner as u64, focused != 0);
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_miopunch_host_NativeHost_start(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    directory: JString<'_>,
    mode: JString<'_>,
    resources: JString<'_>,
) -> jstring {
    reply(&mut env, |env| {
        let directory: String = env.get_string(&directory)?.into();
        let mode: String = env.get_string(&mode)?.into();
        let resources: String = env.get_string(&resources)?.into();
        if directory.len() > 4096 || mode.len() > 65536 || resources.len() > 65536 {
            return Err("host input too large".into());
        }
        let app = application()?;
        app.execute(Operation::Start {
            directory,
            mode,
            resources,
        })
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_miopunch_host_NativeHost_request(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    input: JString<'_>,
) -> jstring {
    reply(&mut env, |env| {
        let input: String = env.get_string(&input)?.into();
        if input.len() > 65536 {
            return Ok(json!({"id":null,"ok":false,"error":"invalid_command"}));
        }
        let app = application()?;
        app.execute(Operation::Request(input))
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_miopunch_host_NativeHost_stop(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
) -> jstring {
    reply(&mut env, |_| {
        let app = application()?;
        app.execute(Operation::Stop)
    })
}
