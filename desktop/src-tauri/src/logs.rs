//! Sidecar output handling: every line goes to a log file in the app log
//! directory, the most recent lines are kept for the failure screen, and
//! JSON runtime events are condensed into one-line progress messages.

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const LOG_FILE_NAME: &str = "mesh-llm.log";
const ROTATE_BYTES: u64 = 10 * 1024 * 1024;
const RECENT_LINES: usize = 40;

pub struct LogBook {
    dir: PathBuf,
    file: Mutex<Option<File>>,
    recent: Mutex<VecDeque<String>>,
}

impl LogBook {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            file: Mutex::new(None),
            recent: Mutex::new(VecDeque::with_capacity(RECENT_LINES)),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Opens a fresh log for a new sidecar run, rotating the previous file
    /// once it grows past the size limit.
    pub fn begin_run(&self, header: &str) {
        self.recent.lock().expect("log lock").clear();
        let path = self.dir.join(LOG_FILE_NAME);
        let file = fs::create_dir_all(&self.dir).ok().and_then(|()| {
            rotate_if_large(&path);
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .ok()
        });
        *self.file.lock().expect("log lock") = file;
        self.record(header);
    }

    pub fn record(&self, line: &str) {
        if let Some(file) = self.file.lock().expect("log lock").as_mut() {
            let _ = writeln!(file, "{line}");
        }
        let mut recent = self.recent.lock().expect("log lock");
        if recent.len() == RECENT_LINES {
            recent.pop_front();
        }
        recent.push_back(line.to_string());
    }

    pub fn recent(&self) -> Vec<String> {
        self.recent
            .lock()
            .expect("log lock")
            .iter()
            .cloned()
            .collect()
    }
}

fn rotate_if_large(path: &Path) {
    let large = fs::metadata(path).is_ok_and(|meta| meta.len() > ROTATE_BYTES);
    if large {
        let _ = fs::rename(path, path.with_extension("log.1"));
    }
}

/// Turns one sidecar output line into a short progress message for the
/// launcher. Only JSON runtime events (`--log-format json`) qualify; raw
/// stderr diagnostics and periodic health dumps stay in the log file.
pub fn summarize_line(line: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(line.trim()).ok()?;
    let context = value.get("context").and_then(serde_json::Value::as_str);
    if context.is_some_and(|context| QUIET_CONTEXTS.contains(&context)) {
        return None;
    }
    value
        .get("message")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(str::to_string)
}

/// Runtime event contexts that are too noisy to show as startup progress.
const QUIET_CONTEXTS: &[&str] = &["event_system_health"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_json_events_by_message() {
        let line = r#"{"timestamp":"t","level":"info","event":"x","message":"Downloading model"}"#;
        assert_eq!(summarize_line(line).as_deref(), Some("Downloading model"));
        assert_eq!(summarize_line(r#"{"level":"info","message":""}"#), None);
    }

    #[test]
    fn ignores_plain_lines_and_health_dumps() {
        assert_eq!(
            summarize_line("\u{1b}[31mERROR\u{1b}[0m relay failed"),
            None
        );
        assert_eq!(summarize_line("   "), None);
        assert_eq!(summarize_line("{not json"), None);
        let health = r#"{"context":"event_system_health","level":"info","message":"version=0"}"#;
        assert_eq!(summarize_line(health), None);
    }

    #[test]
    fn keeps_only_recent_lines_and_writes_file() {
        let dir =
            std::env::temp_dir().join(format!("mesh-llm-desktop-logs-{}", std::process::id()));
        let book = LogBook::new(dir.clone());
        book.begin_run("start");
        for index in 0..(RECENT_LINES + 5) {
            book.record(&format!("line {index}"));
        }
        let recent = book.recent();
        assert_eq!(recent.len(), RECENT_LINES);
        assert_eq!(
            recent.last().unwrap(),
            &format!("line {}", RECENT_LINES + 4)
        );
        let written = fs::read_to_string(dir.join(LOG_FILE_NAME)).unwrap();
        assert!(written.starts_with("start\n"));
        let _ = fs::remove_dir_all(dir);
    }
}
