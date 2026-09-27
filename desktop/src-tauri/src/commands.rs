//! IPC commands used by the bundled launcher page (`desktop/ui`). The
//! console served by mesh-llm is a remote origin and cannot call these.

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::node::{NodeController, PhaseUpdate};
use crate::settings::Settings;

#[derive(Serialize)]
pub struct LaunchState {
    settings: Settings,
    phase: PhaseUpdate,
    log_dir: String,
}

#[tauri::command]
pub fn launch_state(node: State<'_, Arc<NodeController>>) -> LaunchState {
    LaunchState {
        settings: node.settings(),
        phase: node.phase(),
        log_dir: node.log_dir().display().to_string(),
    }
}

#[tauri::command]
pub fn start_mesh(node: State<'_, Arc<NodeController>>, settings: Settings) -> Result<(), String> {
    node.start_with(settings)
}

#[tauri::command]
pub fn retry_mesh(node: State<'_, Arc<NodeController>>) {
    node.restart();
}

#[tauri::command]
pub fn change_mode(node: State<'_, Arc<NodeController>>) -> Result<(), String> {
    node.change_mode()
}

#[tauri::command]
pub fn open_logs(node: State<'_, Arc<NodeController>>) -> Result<(), String> {
    tauri_plugin_opener::open_path(node.log_dir(), None::<&str>).map_err(|err| err.to_string())
}
