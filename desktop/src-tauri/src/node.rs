//! Owns the local mesh-llm node for the app: decides whether to attach to an
//! instance that is already running or start the bundled sidecar, watches it
//! until its console answers, and moves the main window between the launcher
//! page and the console.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Url};

use crate::logs::{LogBook, summarize_line};
use crate::readiness::{console_url, probe_delay, status_ok};
use crate::settings::Settings;
use crate::sidecar::{
    LaunchPlan, SIDECAR_OVERRIDE_ENV, Sidecar, locate_binary, locate_native_runtimes,
};

pub const MAIN_WINDOW: &str = "main";
pub const PHASE_EVENT: &str = "mesh://phase";
pub const LOG_EVENT: &str = "mesh://log";

const PROBE_TIMEOUT: Duration = Duration::from_millis(500);
const CRASH_POLL: Duration = Duration::from_secs(2);
const STOP_GRACE: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Nothing running; the launcher asks which mode to start.
    Idle,
    Starting,
    Ready,
    Failed,
}

#[derive(Clone, Debug, Serialize)]
pub struct PhaseUpdate {
    pub phase: Phase,
    pub message: String,
    /// Set when the app attached to a mesh-llm it did not start.
    pub external: bool,
    pub console_url: Option<String>,
    pub recent_logs: Vec<String>,
}

impl PhaseUpdate {
    fn idle() -> Self {
        Self {
            phase: Phase::Idle,
            message: String::new(),
            external: false,
            console_url: None,
            recent_logs: Vec::new(),
        }
    }
}

enum Run {
    Idle,
    Owned(Arc<Sidecar>),
    External,
}

pub struct NodeController {
    app: AppHandle,
    settings_path: PathBuf,
    settings: Mutex<Settings>,
    logs: LogBook,
    run: Mutex<Run>,
    /// Bumped on every start/stop so monitors of an older run go quiet.
    generation: AtomicU64,
    phase: Mutex<PhaseUpdate>,
    launcher_url: Mutex<Option<Url>>,
}

impl NodeController {
    pub fn new(app: AppHandle, settings_path: PathBuf, log_dir: PathBuf) -> Self {
        let settings = Settings::load(&settings_path).unwrap_or_default();
        Self {
            app,
            settings_path,
            settings: Mutex::new(settings),
            logs: LogBook::new(log_dir),
            run: Mutex::new(Run::Idle),
            generation: AtomicU64::new(0),
            phase: Mutex::new(PhaseUpdate::idle()),
            launcher_url: Mutex::new(None),
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().expect("settings lock").clone()
    }

    pub fn phase(&self) -> PhaseUpdate {
        self.phase.lock().expect("phase lock").clone()
    }

    pub fn log_dir(&self) -> PathBuf {
        self.logs.dir().to_path_buf()
    }

    pub fn remember_launcher_url(&self, url: Url) {
        *self.launcher_url.lock().expect("launcher lock") = Some(url);
    }

    /// Runs once at app start: attach to a running node, auto-start a
    /// remembered mode, or leave the launcher showing.
    pub fn boot(self: &Arc<Self>) {
        let settings = self.settings();
        if status_ok(settings.console_port, PROBE_TIMEOUT) {
            self.attach_external(&settings);
        } else if settings.remember {
            self.start(settings);
        }
    }

    /// Saves the settings and (re)starts the node with them.
    pub fn start_with(self: &Arc<Self>, settings: Settings) -> Result<(), String> {
        let settings = settings.normalized();
        settings.validate()?;
        settings
            .save(&self.settings_path)
            .map_err(|err| format!("Could not save settings: {err}"))?;
        *self.settings.lock().expect("settings lock") = settings.clone();
        self.start(settings);
        Ok(())
    }

    pub fn restart(self: &Arc<Self>) {
        self.start(self.settings());
    }

    /// Stops the node the app owns and returns to the launcher so the user
    /// can pick a different mode. Auto-start is turned off until they pick.
    pub fn change_mode(&self) -> Result<(), String> {
        if let Some(sidecar) = self.detach() {
            thread::spawn(move || sidecar.stop(STOP_GRACE));
        }
        let mut settings = self.settings.lock().expect("settings lock");
        settings.remember = false;
        settings
            .save(&self.settings_path)
            .map_err(|err| format!("Could not save settings: {err}"))?;
        drop(settings);
        self.publish(PhaseUpdate::idle());
        self.show_launcher();
        Ok(())
    }

    /// Stops the sidecar if the app started it, blocking until it exits. An
    /// attached external instance keeps running.
    pub fn stop_owned(&self) {
        if let Some(sidecar) = self.detach() {
            sidecar.stop(STOP_GRACE);
        }
    }

    /// Ends the current run without waiting: monitors of it go quiet at once
    /// and the owned sidecar, if any, is returned for the caller to stop.
    fn detach(&self) -> Option<Arc<Sidecar>> {
        self.generation.fetch_add(1, Ordering::SeqCst);
        let previous = std::mem::replace(&mut *self.run.lock().expect("run lock"), Run::Idle);
        match previous {
            Run::Owned(sidecar) => {
                self.logs.record("[desktop] stopping mesh-llm");
                Some(sidecar)
            }
            Run::Idle | Run::External => None,
        }
    }

    fn start(self: &Arc<Self>, settings: Settings) {
        let previous = self.detach();
        let generation = self.generation.load(Ordering::SeqCst);
        self.publish(self.update(Phase::Starting, "Starting Mesh LLM…", false));
        let this = Arc::clone(self);
        thread::spawn(move || {
            if let Some(sidecar) = previous {
                sidecar.stop(STOP_GRACE);
            }
            this.launch(generation, &settings);
        });
    }

    fn launch(self: &Arc<Self>, generation: u64, settings: &Settings) {
        if !self.is_current(generation) {
            return;
        }
        if status_ok(settings.console_port, PROBE_TIMEOUT) {
            self.attach_external(settings);
            return;
        }
        let Some(plan) = self.launch_plan(settings) else {
            self.fail(
                "Could not find the mesh-llm program. Reinstall the app, or install the \
                 mesh-llm command-line tool and try again.",
            );
            return;
        };
        self.logs
            .begin_run(&format!("[desktop] starting: {}", plan.display()));
        let sidecar = match Sidecar::spawn(&plan, self.line_sink()) {
            Ok(sidecar) => Arc::new(sidecar),
            Err(err) => {
                self.fail(&format!("Could not start mesh-llm: {err}"));
                return;
            }
        };
        self.logs
            .record(&format!("[desktop] mesh-llm pid {}", sidecar.pid()));
        {
            let mut run = self.run.lock().expect("run lock");
            if !self.is_current(generation) {
                drop(run);
                sidecar.stop(STOP_GRACE);
                return;
            }
            *run = Run::Owned(Arc::clone(&sidecar));
        }
        self.watch(generation, &sidecar, settings.console_port);
    }

    fn launch_plan(&self, settings: &Settings) -> Option<LaunchPlan> {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(PathBuf::from));
        let program = locate_binary(
            std::env::var_os(SIDECAR_OVERRIDE_ENV),
            exe_dir.as_deref(),
            std::env::var_os("PATH"),
        )?;
        let resource_dir = self.app.path().resource_dir().ok();
        let runtimes = locate_native_runtimes(resource_dir.as_deref());
        Some(LaunchPlan::new(program, settings, runtimes.as_deref()))
    }

    fn line_sink(self: &Arc<Self>) -> Arc<dyn Fn(String) + Send + Sync> {
        let this = Arc::clone(self);
        Arc::new(move |line: String| {
            this.logs.record(&line);
            if let Some(message) = summarize_line(&line) {
                let _ = this.app.emit(LOG_EVENT, message);
            }
        })
    }

    /// Waits for the console to answer, then keeps watching for a crash.
    fn watch(&self, generation: u64, sidecar: &Sidecar, console_port: u16) {
        let mut attempt = 0;
        while self.is_current(generation) {
            if let Some(status) = sidecar.exit_status() {
                self.fail_run(
                    generation,
                    &format!("mesh-llm stopped unexpectedly ({status})."),
                );
                return;
            }
            if status_ok(console_port, PROBE_TIMEOUT) {
                self.publish(self.ready_update(console_port, false));
                self.show_console(console_port);
                break;
            }
            thread::sleep(probe_delay(attempt));
            attempt += 1;
        }
        while self.is_current(generation) {
            if let Some(status) = sidecar.exit_status() {
                self.fail_run(
                    generation,
                    &format!("mesh-llm stopped unexpectedly ({status})."),
                );
                self.show_launcher();
                return;
            }
            thread::sleep(CRASH_POLL);
        }
    }

    /// Reports a sidecar failure unless the run was already replaced or
    /// stopped on purpose, in which case the exit is expected.
    fn fail_run(&self, generation: u64, message: &str) {
        let mut run = self.run.lock().expect("run lock");
        if !self.is_current(generation) {
            return;
        }
        *run = Run::Idle;
        drop(run);
        self.fail(message);
    }

    fn attach_external(&self, settings: &Settings) {
        *self.run.lock().expect("run lock") = Run::External;
        self.logs.record(&format!(
            "[desktop] attached to mesh-llm already running on console port {}",
            settings.console_port
        ));
        self.publish(self.ready_update(settings.console_port, true));
        self.show_console(settings.console_port);
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == generation
    }

    fn update(&self, phase: Phase, message: &str, external: bool) -> PhaseUpdate {
        PhaseUpdate {
            phase,
            message: message.to_string(),
            external,
            console_url: None,
            recent_logs: Vec::new(),
        }
    }

    fn ready_update(&self, console_port: u16, external: bool) -> PhaseUpdate {
        let message = if external {
            "Connected to a Mesh LLM node that was already running."
        } else {
            "Mesh LLM is running."
        };
        PhaseUpdate {
            console_url: Some(console_url(console_port)),
            ..self.update(Phase::Ready, message, external)
        }
    }

    fn fail(&self, message: &str) {
        self.logs.record(&format!("[desktop] {message}"));
        self.publish(PhaseUpdate {
            recent_logs: self.logs.recent(),
            ..self.update(Phase::Failed, message, false)
        });
    }

    fn publish(&self, update: PhaseUpdate) {
        *self.phase.lock().expect("phase lock") = update.clone();
        crate::tray::reflect_phase(&self.app, &update);
        let _ = self.app.emit(PHASE_EVENT, update);
    }

    fn show_console(&self, console_port: u16) {
        if let Ok(url) = Url::parse(&console_url(console_port)) {
            self.navigate(url);
        }
    }

    pub fn show_launcher(&self) {
        let url = self.launcher_url.lock().expect("launcher lock").clone();
        if let Some(url) = url {
            self.navigate(url);
        }
    }

    fn navigate(&self, url: Url) {
        if let Some(window) = self.app.get_webview_window(MAIN_WINDOW) {
            let _ = window.navigate(url);
        }
    }
}
