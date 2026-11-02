use std::ffi::{OsStr, OsString};
use std::sync::OnceLock;

pub fn executable() -> &'static OsStr {
    static CLI: OnceLock<OsString> = OnceLock::new();
    CLI.get_or_init(|| {
        std::env::var_os("MIOPUNCH_TEST_CLI")
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_miopunch-native-probe").into())
    })
    .as_os_str()
}
