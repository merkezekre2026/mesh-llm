//! Launcher choices persisted between runs: which mesh-llm mode to start,
//! which ports to use, and whether to start it without asking next time.

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const DEFAULT_CONSOLE_PORT: u16 = 3131;
pub const DEFAULT_API_PORT: u16 = 9337;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Join a mesh without serving a model (`mesh-llm client`).
    Client,
    /// Share this machine's GPU/memory and serve a model (`mesh-llm serve`).
    Serve,
}

impl Mode {
    pub fn subcommand(self) -> &'static str {
        match self {
            Mode::Client => "client",
            Mode::Serve => "serve",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub mode: Mode,
    pub console_port: u16,
    pub api_port: u16,
    /// Private mesh invite token. `None` joins the best public mesh (`--auto`).
    pub join_token: Option<String>,
    /// Start with these settings on launch instead of showing the launcher.
    pub remember: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Client,
            console_port: DEFAULT_CONSOLE_PORT,
            api_port: DEFAULT_API_PORT,
            join_token: None,
            remember: false,
        }
    }
}

impl Settings {
    /// Reads saved settings. A missing or unreadable file yields `None` so the
    /// launcher is shown instead of failing startup.
    pub fn load(path: &Path) -> Option<Self> {
        let raw = fs::read_to_string(path).ok()?;
        serde_json::from_str::<Settings>(&raw)
            .ok()
            .map(Settings::normalized)
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let body = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        fs::write(path, body)?;
        restrict_to_owner(path)
    }

    /// Trims the invite token, drops an empty one, and replaces zero ports
    /// with defaults.
    pub fn normalized(mut self) -> Self {
        self.join_token = self
            .join_token
            .map(|token| token.trim().to_string())
            .filter(|token| !token.is_empty());
        if self.console_port == 0 {
            self.console_port = DEFAULT_CONSOLE_PORT;
        }
        if self.api_port == 0 {
            self.api_port = DEFAULT_API_PORT;
        }
        self
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.console_port == self.api_port {
            return Err("The console port and the API port must be different.".to_string());
        }
        Ok(())
    }
}

/// The file may hold an invite token, so keep it private to the user.
#[cfg(unix)]
fn restrict_to_owner(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_to_owner(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir()
            .join(format!("mesh-llm-desktop-{}-{name}", std::process::id()))
            .join("settings.json")
    }

    #[test]
    fn save_and_load_round_trip() {
        let path = temp_path("round-trip");
        let settings = Settings {
            mode: Mode::Serve,
            console_port: 4131,
            api_port: 10337,
            join_token: Some("token".to_string()),
            remember: true,
        };
        settings.save(&path).expect("save");
        assert_eq!(Settings::load(&path), Some(settings));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn missing_or_corrupt_file_loads_as_none() {
        let path = temp_path("corrupt");
        assert_eq!(Settings::load(&path), None);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{not json").unwrap();
        assert_eq!(Settings::load(&path), None);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn partial_file_fills_defaults() {
        let parsed: Settings = serde_json::from_str(r#"{"mode":"serve"}"#).unwrap();
        assert_eq!(parsed.mode, Mode::Serve);
        assert_eq!(parsed.console_port, DEFAULT_CONSOLE_PORT);
        assert_eq!(parsed.api_port, DEFAULT_API_PORT);
        assert!(!parsed.remember);
    }

    #[test]
    fn normalized_drops_blank_token_and_zero_ports() {
        let settings = Settings {
            join_token: Some("   ".to_string()),
            console_port: 0,
            api_port: 0,
            ..Settings::default()
        }
        .normalized();
        assert_eq!(settings.join_token, None);
        assert_eq!(settings.console_port, DEFAULT_CONSOLE_PORT);
        assert_eq!(settings.api_port, DEFAULT_API_PORT);

        let trimmed = Settings {
            join_token: Some("  abc \n".to_string()),
            ..Settings::default()
        }
        .normalized();
        assert_eq!(trimmed.join_token.as_deref(), Some("abc"));
    }

    #[test]
    fn validate_rejects_port_collision() {
        let settings = Settings {
            console_port: 5000,
            api_port: 5000,
            ..Settings::default()
        };
        assert!(settings.validate().is_err());
        assert!(Settings::default().validate().is_ok());
    }
}
