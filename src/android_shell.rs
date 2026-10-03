//! Thin foreground test entry. JNI's real Android Context supplies system DNS;
//! all networking, authorization and persistence remain in the shared node.
use jni::{
    JNIEnv,
    objects::{GlobalRef, JClass, JObject, JString},
    sys::jint,
};
use std::sync::OnceLock;

// ndk-context stores raw pointers for process-wide DNS workers. Keep its Java
// object alive for the entire foreground shell process, including runtime drop.
static ANDROID_CONTEXT: OnceLock<GlobalRef> = OnceLock::new();

#[unsafe(no_mangle)]
pub extern "system" fn Java_MioPublicShell_session(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    context: JObject<'_>,
    directory: JString<'_>,
    bind_ip: JString<'_>,
) -> jint {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let directory: String = env.get_string(&directory)?.into();
        let bind_ip: String = env.get_string(&bind_ip)?.into();
        ANDROID_CONTEXT
            .set(env.new_global_ref(context)?)
            .map_err(|_| "Android shell context already installed")?;
        let context = ANDROID_CONTEXT.get().expect("installed Android context");
        let vm = env.get_java_vm()?;
        // The VM is owned by app_process. The global object remains valid until
        // the Java launcher exits immediately after this function returns.
        unsafe {
            iroh::dns::install_android_jni_context(
                vm.get_java_vm_pointer().cast(),
                context.as_obj().as_raw().cast(),
            );
        }
        let runtime = tokio::runtime::Runtime::new()?;
        runtime.block_on(crate::android_desktop_cli::session(
            &directory,
            bind_ip.parse()?,
        ))?;
        Ok::<_, Box<dyn std::error::Error>>(())
    }));
    match result {
        Ok(Ok(())) => 0,
        Ok(Err(error)) => {
            eprintln!("ANDROID_SHELL_FAILED error={error}");
            1
        }
        Err(_) => {
            eprintln!("ANDROID_SHELL_FAILED panic=true");
            1
        }
    }
}
