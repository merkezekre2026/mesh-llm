//! Lifecycle of the bundled `mesh-llm` binary: locating it, building its
//! command line from the launcher settings, streaming its output, and
//! stopping it gracefully so it can clean up its runtime directory.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::settings::Settings;

/// Name of the mesh-llm copy bundled beside the app executable. It differs
/// from the CLI's `mesh-llm` so a Linux package never collides with a
/// separately installed CLI in `/usr/bin`.
const BUNDLED_STEM: &str = "mesh-llm-sidecar";
/// Name of an installed mesh-llm CLI, used when no bundled copy exists.
const CLI_STEM: &str = "mesh-llm";
/// Overrides the sidecar location, for running the app against a dev build.
pub const SIDECAR_OVERRIDE_ENV: &str = "MESH_LLM_DESKTOP_SIDECAR";
/// Environment variable mesh-llm reads for an explicit native runtime bundle.
const NATIVE_RUNTIME_BUNDLE_DIR_ENV: &str = "MESH_LLM_NATIVE_RUNTIME_BUNDLE_DIR";
/// Environment variable mesh-llm reads for an invite token. Using it keeps
/// the token out of the process list, unlike `--join <token>`.
const JOIN_ENV: &str = "MESH_LLM_JOIN";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl LaunchPlan {
    pub fn new(program: PathBuf, settings: &Settings, native_runtimes: Option<&Path>) -> Self {
        Self {
            program,
            args: sidecar_args(settings),
            env: sidecar_env(settings, native_runtimes),
        }
    }

    /// The command line as shown in logs; never includes the invite token.
    pub fn display(&self) -> String {
        let mut parts = vec![self.program.display().to_string()];
        parts.extend(self.args.iter().cloned());
        parts.join(" ")
    }
}

pub fn sidecar_args(settings: &Settings) -> Vec<String> {
    let mut args = vec![
        "--log-format".to_string(),
        "json".to_string(),
        settings.mode.subcommand().to_string(),
    ];
    if settings.join_token.is_none() {
        args.push("--auto".to_string());
    }
    args.extend([
        "--port".to_string(),
        settings.api_port.to_string(),
        "--console".to_string(),
        settings.console_port.to_string(),
    ]);
    args
}

pub fn sidecar_env(settings: &Settings, native_runtimes: Option<&Path>) -> Vec<(String, String)> {
    let mut env = Vec::new();
    if let Some(dir) = native_runtimes {
        env.push((
            NATIVE_RUNTIME_BUNDLE_DIR_ENV.to_string(),
            dir.display().to_string(),
        ));
    }
    if let Some(token) = &settings.join_token {
        env.push((JOIN_ENV.to_string(), token.clone()));
    }
    env
}

fn executable_name(stem: &str) -> String {
    format!("{stem}{}", std::env::consts::EXE_SUFFIX)
}

/// Finds the mesh-llm binary: an explicit override, then the copy bundled
/// beside the app executable, then an installed CLI on `PATH`.
pub fn locate_binary(
    override_path: Option<OsString>,
    exe_dir: Option<&Path>,
    path_var: Option<OsString>,
) -> Option<PathBuf> {
    if let Some(path) = override_path.map(PathBuf::from) {
        return path.is_file().then_some(path);
    }
    let bundled = exe_dir
        .map(|dir| dir.join(executable_name(BUNDLED_STEM)))
        .filter(|path| path.is_file());
    bundled.or_else(|| {
        let cli = executable_name(CLI_STEM);
        path_var
            .iter()
            .flat_map(std::env::split_paths)
            .map(|dir| dir.join(&cli))
            .find(|candidate| candidate.is_file())
    })
}

/// Finds the native runtime bundle shipped as an app resource. An empty
/// staging directory (dev builds) is treated as absent so mesh-llm falls
/// back to its own discovery.
pub fn locate_native_runtimes(resource_dir: Option<&Path>) -> Option<PathBuf> {
    let dir = resource_dir?.join("native-runtimes");
    let populated = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .any(|entry| !entry.file_name().to_string_lossy().starts_with('.'));
    populated.then_some(dir)
}

pub type LineSink = Arc<dyn Fn(String) + Send + Sync>;

pub struct Sidecar {
    child: Mutex<Child>,
}

impl Sidecar {
    pub fn spawn(plan: &LaunchPlan, on_line: LineSink) -> std::io::Result<Self> {
        let mut command = Command::new(&plan.program);
        command
            .args(&plan.args)
            .envs(plan.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        hide_console_window(&mut command);
        stop_with_parent(&mut command);
        let mut child = command.spawn()?;
        if let Some(stdout) = child.stdout.take() {
            forward_lines(stdout, on_line.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            forward_lines(stderr, on_line);
        }
        Ok(Self {
            child: Mutex::new(child),
        })
    }

    pub fn pid(&self) -> u32 {
        self.child.lock().expect("sidecar lock").id()
    }

    /// Returns the exit status once the process has exited.
    pub fn exit_status(&self) -> Option<ExitStatus> {
        self.child
            .lock()
            .expect("sidecar lock")
            .try_wait()
            .ok()
            .flatten()
    }

    /// Asks mesh-llm to shut down, then force-kills it after `grace`.
    pub fn stop(&self, grace: Duration) {
        let mut child = self.child.lock().expect("sidecar lock");
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        request_graceful_exit(&child);
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn forward_lines(stream: impl Read + Send + 'static, on_line: LineSink) {
    thread::spawn(move || {
        for line in BufReader::new(stream).lines() {
            match line {
                Ok(line) => on_line(line),
                Err(_) => break,
            }
        }
    });
}

/// mesh-llm handles SIGTERM as a clean shutdown and removes its runtime
/// directory.
#[cfg(unix)]
fn request_graceful_exit(child: &Child) {
    let Ok(pid) = libc::pid_t::try_from(child.id()) else {
        return;
    };
    // SAFETY: `kill` has no memory-safety preconditions; `pid` is our own
    // child, which has not been reaped yet because we still hold its handle.
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
}

/// A GUI process on Windows has no console to deliver Ctrl+Break through, so
/// the sidecar is terminated after the grace period. Other mesh-llm instances
/// garbage-collect the stale runtime directory later.
#[cfg(not(unix))]
fn request_graceful_exit(_child: &Child) {}

/// If the app is killed without a chance to clean up (session logout,
/// crash), Linux delivers SIGTERM to the sidecar so it does not linger.
#[cfg(target_os = "linux")]
fn stop_with_parent(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: the closure runs between fork and exec and only calls `prctl`,
    // which is async-signal-safe.
    unsafe {
        command.pre_exec(|| {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(target_os = "linux"))]
fn stop_with_parent(_command: &mut Command) {}

#[cfg(windows)]
fn hide_console_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console_window(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Mode;
    use std::fs;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mesh-llm-desktop-sidecar-{}-{name}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn public_client_uses_auto_discovery() {
        let args = sidecar_args(&Settings::default());
        assert_eq!(
            args,
            [
                "--log-format",
                "json",
                "client",
                "--auto",
                "--port",
                "9337",
                "--console",
                "3131"
            ]
        );
    }

    #[test]
    fn private_serve_passes_token_through_env_only() {
        let settings = Settings {
            mode: Mode::Serve,
            console_port: 4000,
            api_port: 4001,
            join_token: Some("secret-token".to_string()),
            remember: false,
        };
        let plan = LaunchPlan::new(PathBuf::from("mesh-llm"), &settings, None);
        assert_eq!(
            plan.args,
            [
                "--log-format",
                "json",
                "serve",
                "--port",
                "4001",
                "--console",
                "4000"
            ]
        );
        assert_eq!(
            plan.env,
            [("MESH_LLM_JOIN".to_string(), "secret-token".to_string())]
        );
        assert!(!plan.display().contains("secret-token"));
    }

    #[test]
    fn native_runtime_dir_is_exported() {
        let env = sidecar_env(&Settings::default(), Some(Path::new("/opt/rt")));
        assert_eq!(
            env,
            [(
                "MESH_LLM_NATIVE_RUNTIME_BUNDLE_DIR".to_string(),
                "/opt/rt".to_string()
            )]
        );
    }

    #[test]
    fn locate_prefers_override_then_bundle_then_path() {
        let bundle = scratch("bundle");
        let on_path = scratch("path");
        let cli = executable_name(CLI_STEM);
        let bundled = executable_name(BUNDLED_STEM);
        fs::write(on_path.join(&cli), b"").unwrap();
        // A CLI-named binary beside the app is not the bundled sidecar.
        fs::write(bundle.join(&cli), b"").unwrap();

        let path_var = std::env::join_paths([&on_path]).unwrap();
        assert_eq!(
            locate_binary(None, Some(&bundle), Some(path_var.clone())),
            Some(on_path.join(&cli))
        );

        fs::write(bundle.join(&bundled), b"").unwrap();
        assert_eq!(
            locate_binary(None, Some(&bundle), Some(path_var.clone())),
            Some(bundle.join(&bundled))
        );

        let custom = on_path.join("custom-mesh");
        fs::write(&custom, b"").unwrap();
        assert_eq!(
            locate_binary(Some(custom.clone().into()), Some(&bundle), Some(path_var)),
            Some(custom)
        );
        assert_eq!(
            locate_binary(Some(on_path.join("missing").into()), Some(&bundle), None),
            None
        );

        let _ = fs::remove_dir_all(bundle);
        let _ = fs::remove_dir_all(on_path);
    }

    #[test]
    fn empty_native_runtime_dir_is_ignored() {
        let resources = scratch("resources");
        let runtimes = resources.join("native-runtimes");
        fs::create_dir_all(&runtimes).unwrap();
        fs::write(runtimes.join(".gitkeep"), b"").unwrap();
        assert_eq!(locate_native_runtimes(Some(&resources)), None);
        fs::create_dir_all(runtimes.join("cpu-x86_64")).unwrap();
        assert_eq!(locate_native_runtimes(Some(&resources)), Some(runtimes));
        let _ = fs::remove_dir_all(resources);
    }

    #[cfg(unix)]
    #[test]
    fn stop_terminates_and_output_is_forwarded() {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let sink_lines = lines.clone();
        let plan = LaunchPlan {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), "echo ready; exec sleep 30".into()],
            env: Vec::new(),
        };
        let sidecar = Sidecar::spawn(
            &plan,
            Arc::new(move |line| sink_lines.lock().unwrap().push(line)),
        )
        .expect("spawn");
        let started = Instant::now();
        while lines.lock().unwrap().is_empty() && started.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(lines.lock().unwrap().as_slice(), ["ready"]);
        assert!(sidecar.exit_status().is_none());
        sidecar.stop(Duration::from_secs(5));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(sidecar.exit_status().is_some());
    }
}
