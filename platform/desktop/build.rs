fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "host_request",
            "host_start",
            "host_state",
            "set_autostart",
            "window_ready",
            "verification_next",
            "verification_result",
            "close_window",
            "quit",
        ]),
    ))
    .expect("desktop host manifest");
}
