//! System tray icon and menu. The app keeps running here when its window is
//! closed, so the local node stays available to other apps.

use std::sync::Arc;

use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::node::{MAIN_WINDOW, NodeController, Phase, PhaseUpdate};

const TRAY_ID: &str = "mesh-llm";
const STATUS_ITEM: &str = "status";
const SHOW: &str = "show";
const OPEN_BROWSER: &str = "open-browser";
const CHANGE_MODE: &str = "change-mode";
const RESTART: &str = "restart";
const OPEN_LOGS: &str = "open-logs";
const QUIT: &str = "quit";

/// Holds the status line so it can be updated as the node changes phase.
pub struct TrayStatus(MenuItem<Wry>);

pub fn install(app: &AppHandle) -> tauri::Result<TrayIcon> {
    let status = MenuItem::with_id(
        app,
        STATUS_ITEM,
        "Mesh LLM: not running",
        false,
        None::<&str>,
    )?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, SHOW, "Show Mesh LLM", true, None::<&str>)?,
            &MenuItem::with_id(
                app,
                OPEN_BROWSER,
                "Open Console in Browser",
                true,
                None::<&str>,
            )?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, CHANGE_MODE, "Change Mode…", true, None::<&str>)?,
            &MenuItem::with_id(app, RESTART, "Restart Node", true, None::<&str>)?,
            &MenuItem::with_id(app, OPEN_LOGS, "Open Logs Folder", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, QUIT, "Quit Mesh LLM", true, None::<&str>)?,
        ],
    )?;
    app.manage(TrayStatus(status));
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Mesh LLM")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(handle_menu)
        .on_tray_icon_event(handle_icon);
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)
}

fn handle_menu(app: &AppHandle, event: MenuEvent) {
    let node = app.state::<Arc<NodeController>>();
    match event.id().as_ref() {
        SHOW => show_main_window(app),
        OPEN_BROWSER => open_console_in_browser(app, &node),
        CHANGE_MODE => {
            let _ = node.change_mode();
            show_main_window(app);
        }
        RESTART => {
            node.restart();
            show_main_window(app);
        }
        OPEN_LOGS => {
            let _ = tauri_plugin_opener::open_path(node.log_dir(), None::<&str>);
        }
        QUIT => app.exit(0),
        _ => {}
    }
}

fn handle_icon(tray: &TrayIcon, event: TrayIconEvent) {
    if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        ..
    } = event
    {
        show_main_window(tray.app_handle());
    }
}

fn open_console_in_browser(app: &AppHandle, node: &NodeController) {
    match node.phase().console_url {
        Some(url) => {
            let _ = tauri_plugin_opener::open_url(url, None::<&str>);
        }
        None => show_main_window(app),
    }
}

pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Mirrors the node phase in the tray status line and tooltip.
pub fn reflect_phase(app: &AppHandle, update: &PhaseUpdate) {
    let text = match update.phase {
        Phase::Idle => "Mesh LLM: not running",
        Phase::Starting => "Mesh LLM: starting…",
        Phase::Ready if update.external => "Mesh LLM: running (external)",
        Phase::Ready => "Mesh LLM: running",
        Phase::Failed => "Mesh LLM: stopped with an error",
    };
    if let Some(status) = app.try_state::<TrayStatus>() {
        let _ = status.0.set_text(text);
    }
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(text));
    }
}
