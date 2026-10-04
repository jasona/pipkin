//! Launching and owning a local Pi engine (the dev entry point: a Pi checkout's `pi-test.sh`).
//!
//! The engine is started with an explicit executable, argument array, working directory and
//! environment, never through a shell or the user's startup files. Pi spawns its coordinator
//! and session workers as *detached* processes, so a process group is not enough to stop it.
//! Ownership is therefore by identity: every engine process we started carries this profile's
//! exact `PI_SERVER_DIR` and `PI_SERVER_ID`, and only processes of the same user that carry
//! both are ever signalled. An unrelated installed Pi is never touched.
//!
//! Output goes to a log file, not a pipe that could fill. Restarts are capped so a crash loop
//! stops with a diagnosis instead of spinning.

use std::fs::{self, File};
use std::io;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use pi_client::client::ClientOptions;
use pi_client::unix;

/// Restarts allowed within `RESTART_WINDOW` before giving up.
const MAX_RESTARTS: usize = 3;
const RESTART_WINDOW: Duration = Duration::from_secs(60);
const READY_TIMEOUT: Duration = Duration::from_secs(45);
const STOP_GRACE: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub struct EngineConfig {
    /// A Pi checkout containing `pi-test.sh` and installed dependencies.
    pub pi_repo: PathBuf,
    /// `PI_SERVER_DIR`: the profile directory holding sockets.
    pub server_dir: PathBuf,
    /// `PI_SERVER_ID`: the logical server identity.
    pub server_id: String,
    /// `PI_CODING_AGENT_DIR` when set; otherwise Pi's own default applies.
    pub agent_dir: Option<PathBuf>,
    /// The engine process's working directory (the default session cwd).
    pub cwd: PathBuf,
    /// `--provider`/`--model` for new sessions.
    pub model: Option<(String, String)>,
    /// Where stdout and stderr go.
    pub log_path: PathBuf,
    /// Extra environment, for example `PI_OFFLINE=1`.
    pub env: Vec<(String, String)>,
}

#[derive(Debug)]
pub enum EngineError {
    Missing(String),
    Spawn(io::Error),
    /// The engine exited before becoming ready.
    Exited(String),
    NotReady(String),
    /// Crashed too often to keep restarting.
    GaveUp(String),
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::Missing(m) => write!(f, "{m}"),
            EngineError::Spawn(e) => write!(f, "cannot start the Pi engine: {e}"),
            EngineError::Exited(m) => write!(f, "the Pi engine exited during startup: {m}"),
            EngineError::NotReady(m) => write!(f, "the Pi engine did not become ready: {m}"),
            EngineError::GaveUp(m) => write!(f, "the Pi engine keeps crashing: {m}"),
        }
    }
}

impl std::error::Error for EngineError {}

/// One running engine we started.
pub struct Engine {
    child: Child,
    config: EngineConfig,
}

fn socket_path(config: &EngineConfig) -> PathBuf {
    config.server_dir.join(format!("{}.sock", config.server_id))
}

/// The last lines of the engine's log, for an error message a person can act on.
fn log_tail(path: &Path) -> String {
    let text = fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let tail = lines[lines.len().saturating_sub(6)..].join(" | ");
    if tail.is_empty() {
        "no output".into()
    } else {
        tail
    }
}

impl Engine {
    pub fn spawn(config: &EngineConfig) -> Result<Engine, EngineError> {
        let launcher = config.pi_repo.join("pi-test.sh");
        if !launcher.is_file() {
            return Err(EngineError::Missing(format!(
                "{} not found; --pi-repo must be a Pi checkout",
                launcher.display()
            )));
        }
        if !config.pi_repo.join("node_modules").is_dir() {
            return Err(EngineError::Missing(format!(
                "Pi dependencies are not installed in {} (run npm ci there)",
                config.pi_repo.display()
            )));
        }
        fs::create_dir_all(&config.server_dir).map_err(EngineError::Spawn)?;
        // The socket directory must be private; Pi enforces it too, but fail with our own message.
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&config.server_dir, fs::Permissions::from_mode(0o700))
            .map_err(EngineError::Spawn)?;
        if let Some(parent) = config.log_path.parent() {
            fs::create_dir_all(parent).map_err(EngineError::Spawn)?;
        }
        let log = File::create(&config.log_path).map_err(EngineError::Spawn)?;
        let log_err = log.try_clone().map_err(EngineError::Spawn)?;

        let mut args: Vec<String> = vec!["server".into()];
        if let Some((provider, model)) = &config.model {
            args.extend([
                "--provider".into(),
                provider.clone(),
                "--model".into(),
                model.clone(),
            ]);
        }
        let mut command = Command::new(&launcher);
        command
            .args(&args)
            .current_dir(&config.cwd)
            .env("PI_EXPERIMENTAL", "1")
            .env("PI_SERVER_DIR", &config.server_dir)
            .env("PI_SERVER_ID", &config.server_id)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err))
            // Its own group, so signalling the launcher never reaches this application's group.
            .process_group(0);
        if let Some(agent_dir) = &config.agent_dir {
            command.env("PI_CODING_AGENT_DIR", agent_dir);
        }
        for (key, value) in &config.env {
            command.env(key, value);
        }
        let child = command.spawn().map_err(EngineError::Spawn)?;
        log::info!("started the Pi engine launcher (pid {})", child.id());
        Ok(Engine {
            child,
            config: config.clone(),
        })
    }

    /// True while the launcher is alive. (The launcher waits for the server it starts.)
    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Wait until the engine answers a trusted handshake for its server id.
    pub fn wait_ready(&mut self, timeout: Duration) -> Result<(), EngineError> {
        let deadline = Instant::now() + timeout;
        let socket = socket_path(&self.config);
        loop {
            if let Ok(Some(status)) = self.child.try_wait() {
                return Err(EngineError::Exited(format!(
                    "{status}: {}",
                    log_tail(&self.config.log_path)
                )));
            }
            if socket.exists() {
                let mut options = ClientOptions::new(&self.config.server_id);
                options.handshake_timeout = Duration::from_secs(2);
                if let Ok((client, _events)) = unix::connect(&socket, options) {
                    client.disconnect();
                    return Ok(());
                }
            }
            if Instant::now() >= deadline {
                return Err(EngineError::NotReady(format!(
                    "no handshake within {}s: {}",
                    timeout.as_secs(),
                    log_tail(&self.config.log_path)
                )));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Stop everything this engine started: the launcher and the detached server, coordinator
    /// and workers that carry this profile. Only same-user processes with this exact profile
    /// identity are signalled.
    pub fn stop(&mut self) {
        stop_profile(&self.config, STOP_GRACE);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Pids of this user's processes carrying exactly this profile's `PI_SERVER_DIR` and
/// `PI_SERVER_ID`. Reads `/proc`; processes we cannot inspect cannot be ours.
pub fn profile_pids(server_dir: &Path, server_id: &str) -> Vec<u32> {
    let want_dir = format!("PI_SERVER_DIR={}", server_dir.display());
    let want_id = format!("PI_SERVER_ID={server_id}");
    let me = std::process::id();
    let Ok(entries) = fs::read_dir("/proc") else {
        return vec![];
    };
    let mut pids = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == me {
            continue;
        }
        let Ok(environ) = fs::read(entry.path().join("environ")) else {
            continue;
        };
        let mut dir = false;
        let mut id = false;
        for var in environ.split(|b| *b == 0) {
            dir |= var == want_dir.as_bytes();
            id |= var == want_id.as_bytes();
        }
        if dir && id {
            pids.push(pid);
        }
    }
    pids
}

fn signal(pid: u32, sig: libc::c_int) {
    // SAFETY: kill(2) with a pid we just matched by environment identity; failure is ignored.
    unsafe {
        libc::kill(pid as libc::pid_t, sig);
    }
}

fn stop_profile(config: &EngineConfig, grace: Duration) {
    let pids = profile_pids(&config.server_dir, &config.server_id);
    for pid in &pids {
        signal(*pid, libc::SIGTERM);
    }
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        if profile_pids(&config.server_dir, &config.server_id).is_empty() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    for pid in profile_pids(&config.server_dir, &config.server_id) {
        log::warn!("the Pi engine process {pid} ignored SIGTERM; killing it");
        signal(pid, libc::SIGKILL);
    }
}

/// Pi's launcher holds a lock directory (`launcher-<id>.lock`) that a hard crash leaves behind,
/// and a new engine would wait 30 seconds for it to go stale. When no process of this profile
/// exists nobody can be holding it, so removing it is safe and makes a restart immediate. If any
/// process of the profile is alive the lock is left strictly alone.
fn clear_stale_launcher_lock(config: &EngineConfig) {
    if !profile_pids(&config.server_dir, &config.server_id).is_empty() {
        return;
    }
    let lock = config
        .server_dir
        .join(format!("launcher-{}.lock", config.server_id));
    if lock.is_dir() {
        log::info!(
            "removing a stale engine lock left by a crash: {}",
            lock.display()
        );
        let _ = fs::remove_dir(&lock);
    }
}

/// An engine that is restarted when it dies, within a cap.
pub struct EngineHost {
    config: EngineConfig,
    engine: Option<Engine>,
    starts: Vec<Instant>,
}

impl EngineHost {
    pub fn new(config: EngineConfig) -> Self {
        EngineHost {
            config,
            engine: None,
            starts: Vec::new(),
        }
    }

    #[cfg(test)]
    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// Start the engine if it is not running, waiting until it is ready. A crash loop is
    /// refused after `MAX_RESTARTS` starts within `RESTART_WINDOW`.
    pub fn ensure_running(&mut self) -> Result<(), EngineError> {
        if let Some(engine) = self.engine.as_mut()
            && engine.is_running()
        {
            return Ok(());
        }
        if let Some(mut dead) = self.engine.take() {
            dead.stop(); // sweep anything it left behind
        }
        self.starts.retain(|t| t.elapsed() < RESTART_WINDOW);
        if self.starts.len() >= MAX_RESTARTS {
            return Err(EngineError::GaveUp(format!(
                "{} starts in {}s; last output: {}",
                self.starts.len(),
                RESTART_WINDOW.as_secs(),
                log_tail(&self.config.log_path)
            )));
        }
        clear_stale_launcher_lock(&self.config);
        self.starts.push(Instant::now());
        let mut engine = Engine::spawn(&self.config)?;
        match engine.wait_ready(READY_TIMEOUT) {
            Ok(()) => {
                self.engine = Some(engine);
                Ok(())
            }
            Err(e) => {
                engine.stop();
                Err(e)
            }
        }
    }

    /// Stop the engine this host started. Idempotent.
    pub fn stop(&mut self) {
        if let Some(mut engine) = self.engine.take() {
            engine.stop();
        }
    }
}

impl Drop for EngineHost {
    fn drop(&mut self) {
        self.stop();
    }
}
