//! Mesh LLM desktop app: starts a local mesh-llm node from a one-screen
//! launcher, shows its web console in a native window, and keeps it running
//! from the system tray.

// No console window behind the app on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod logs;
mod node;
mod readiness;
mod settings;
mod sidecar;
mod tray;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use tauri::{App, AppHandle, Manager, RunEvent, WindowEvent};

use crate::node::{MAIN_WINDOW, NodeController};

/// Whether a tray icon exists; without one, closing the window quits.
struct TrayAvailable(AtomicBool);

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main_window(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::launch_state,
            commands::start_mesh,
            commands::retry_mesh,
            commands::change_mode,
            commands::open_logs,
        ])
        .setup(|app| setup(app).map_err(Into::into))
        .on_window_event(|window, event| {
            if window.label() != MAIN_WINDOW {
                return;
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                let tray = window.app_handle().state::<TrayAvailable>();
                if tray.0.load(Ordering::SeqCst) {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build the Mesh LLM desktop app");

    app.run(handle_run_event);
}

fn setup(app: &mut App) -> tauri::Result<()> {
    let handle = app.handle().clone();
    let paths = app.path();
    let settings_path = paths.app_config_dir()?.join("settings.json");
    let log_dir = paths.app_log_dir()?;
    let node = Arc::new(NodeController::new(handle.clone(), settings_path, log_dir));
    if let Some(window) = app.get_webview_window(MAIN_WINDOW)
        && let Ok(url) = window.url()
    {
        node.remember_launcher_url(url);
    }
    app.manage(Arc::clone(&node));

    // Linux desktops without an appindicator host cannot show a tray; the
    // app then behaves like a normal window that quits on close.
    let tray_ok = tray::install(&handle).is_ok();
    app.manage(TrayAvailable(AtomicBool::new(tray_ok)));

    thread::spawn(move || node.boot());
    Ok(())
}

fn handle_run_event(app: &AppHandle, event: RunEvent) {
    match event {
        RunEvent::Exit => {
            if let Some(node) = app.try_state::<Arc<NodeController>>() {
                node.stop_owned();
            }
        }
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => tray::show_main_window(app),
        _ => {}
    }
}
