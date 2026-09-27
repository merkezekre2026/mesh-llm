const COMMANDS: &[&str] = &[
    "launch_state",
    "start_mesh",
    "retry_mesh",
    "change_mode",
    "open_logs",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run tauri-build");
}
