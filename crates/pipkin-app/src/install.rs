//! The installed layout: where the bundled Pi engine lives, whether it is usable, and the
//! `--version` / `--diagnose` reports that say so.
//!
//! An installation is `<prefix>/bin/pipkin` plus `<prefix>/lib/pipkin/engine/`, a self-contained
//! copy of the Pi engine with an `engine.json` manifest. Nothing here reads a source checkout.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const ENGINE_DIR_ENV: &str = "PIPKIN_ENGINE_DIR";
pub const MANIFEST_FILE: &str = "engine.json";
/// The oldest Node the engine runs on (it loads TypeScript directly).
pub const NODE_MIN: (u32, u32) = (22, 19);
/// The Chord/Pi protocol revision this build speaks. An engine built for another one is refused.
pub const PROTOCOL: u32 = 8;

/// What `engine.json` says about a bundled engine.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub name: String,
    /// The engine's own version (a Pi commit or release).
    pub version: String,
    pub protocol: u32,
    /// The first Pipkin version this engine supports.
    #[serde(default)]
    pub min_client: Option<String>,
    #[serde(default)]
    pub built_at: Option<String>,
}

/// Directories searched for a bundled engine, best first: the environment override, then the
/// layout relative to the running binary (`../lib/pipkin/engine`, so a relocated prefix works),
/// then the system locations.
pub fn engine_search_dirs(exe: Option<&Path>, env: Option<PathBuf>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = env.filter(|d| !d.as_os_str().is_empty()) {
        dirs.push(dir);
    }
    if let Some(prefix) = exe
        .and_then(Path::parent)
        .and_then(Path::parent)
        .map(Path::to_path_buf)
    {
        dirs.push(prefix.join("lib/pipkin/engine"));
    }
    for fixed in ["/usr/lib/pipkin/engine", "/usr/local/lib/pipkin/engine"] {
        let fixed = PathBuf::from(fixed);
        if !dirs.contains(&fixed) {
            dirs.push(fixed);
        }
    }
    dirs
}

/// Why a directory is not a usable engine, or its manifest when it is.
pub fn check_engine(dir: &Path) -> Result<Manifest, String> {
    let manifest_path = dir.join(MANIFEST_FILE);
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let manifest: Manifest = serde_json::from_str(&text)
        .map_err(|e| format!("{} is not a valid manifest: {e}", manifest_path.display()))?;
    if manifest.protocol != PROTOCOL {
        return Err(format!(
            "the engine at {} speaks protocol {}, but this Pipkin needs {PROTOCOL}",
            dir.display(),
            manifest.protocol
        ));
    }
    if let Some(min) = &manifest.min_client
        && version_less(VERSION, min)
    {
        return Err(format!(
            "the engine at {} needs Pipkin {min} or newer; this is {VERSION}",
            dir.display()
        ));
    }
    for needed in ["pi-test.sh", "node_modules"] {
        if !dir.join(needed).exists() {
            return Err(format!(
                "the engine at {} is incomplete: {needed} is missing",
                dir.display()
            ));
        }
    }
    Ok(manifest)
}

/// The first usable bundled engine, or the reasons each candidate was rejected.
pub fn find_engine(candidates: &[PathBuf]) -> Result<(PathBuf, Manifest), Vec<String>> {
    let mut reasons = Vec::new();
    for dir in candidates {
        if !dir.exists() {
            reasons.push(format!("{}: not present", dir.display()));
            continue;
        }
        match check_engine(dir) {
            Ok(manifest) => return Ok((dir.clone(), manifest)),
            Err(reason) => reasons.push(reason),
        }
    }
    Err(reasons)
}

/// Dotted numeric versions compared by component; non-numeric parts count as zero.
fn version_less(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> {
        v.split(['.', '-', '+'])
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    let (mut a, mut b) = (parts(a), parts(b));
    let len = a.len().max(b.len());
    a.resize(len, 0);
    b.resize(len, 0);
    a < b
}

/// `node --version` as `(major, minor, patch)`.
pub fn node_version() -> Result<(u32, u32, u32), String> {
    let output = Command::new("node")
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| {
            format!(
                "node was not found ({e}); install Node.js {}.{} or newer",
                NODE_MIN.0, NODE_MIN.1
            )
        })?;
    parse_node_version(&String::from_utf8_lossy(&output.stdout))
}

pub fn parse_node_version(text: &str) -> Result<(u32, u32, u32), String> {
    let mut nums = text
        .trim()
        .trim_start_matches('v')
        .split('.')
        .map(|p| p.parse::<u32>());
    match (nums.next(), nums.next(), nums.next()) {
        (Some(Ok(a)), Some(Ok(b)), Some(Ok(c))) => Ok((a, b, c)),
        _ => Err(format!(
            "cannot read the Node version from {:?}",
            text.trim()
        )),
    }
}

pub fn node_is_new_enough(version: (u32, u32, u32)) -> bool {
    (version.0, version.1) >= NODE_MIN
}

/// Replace the home directory with `~` so a pasted report does not carry the user name.
pub fn redact_home(text: &str, home: Option<&Path>) -> String {
    match home.and_then(Path::to_str).filter(|h| h.len() > 1) {
        Some(home) => text.replace(home, "~"),
        None => text.to_owned(),
    }
}

/// The last `lines` lines of a log file, or why there are none.
pub fn log_tail(path: &Path, lines: usize) -> String {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let all: Vec<&str> = text.lines().collect();
            all[all.len().saturating_sub(lines)..].join("\n")
        }
        Err(e) => format!("({e})"),
    }
}

/// What the report needs to know; gathered by the caller so the text is a pure function.
pub struct ReportInput {
    pub exe: Option<PathBuf>,
    pub data_dir: PathBuf,
    pub schema_version: u32,
    pub engine: Result<(PathBuf, Manifest), Vec<String>>,
    /// An explicit `--pi-repo` or `PIPKIN_ENGINE_DIR` in use, which wins over the bundled one.
    pub engine_override: Option<PathBuf>,
    pub node: Result<(u32, u32, u32), String>,
    pub wayland_display: bool,
    pub display_vars: Vec<(String, String)>,
    pub database: Result<String, String>,
    pub probe: Option<Result<String, String>>,
    pub engine_log_tail: String,
    pub home: Option<PathBuf>,
}

pub fn report(input: &ReportInput) -> String {
    let mut out = String::new();
    let mut line = |text: String| {
        out.push_str(&text);
        out.push('\n');
    };
    line(format!(
        "Pipkin {VERSION} (protocol {PROTOCOL}, schema {})",
        input.schema_version
    ));
    line(format!(
        "binary: {}",
        input
            .exe
            .as_deref()
            .map_or("unknown".into(), |p| p.display().to_string())
    ));
    line(format!("data directory: {}", input.data_dir.display()));
    match &input.database {
        Ok(summary) => line(format!("database: {summary}")),
        Err(error) => line(format!("database: PROBLEM {error}")),
    }
    if let Some(dir) = &input.engine_override {
        line(format!("engine override: {}", dir.display()));
    }
    match &input.engine {
        Ok((dir, m)) => line(format!(
            "engine: {} {} at {} (protocol {})",
            m.name,
            m.version,
            dir.display(),
            m.protocol
        )),
        Err(reasons) => {
            line("engine: NOT FOUND".into());
            for reason in reasons {
                line(format!("  - {reason}"));
            }
        }
    }
    match &input.node {
        Ok(v) if node_is_new_enough(*v) => line(format!("node: {}.{}.{}", v.0, v.1, v.2)),
        Ok(v) => line(format!(
            "node: PROBLEM {}.{}.{} is older than {}.{}",
            v.0, v.1, v.2, NODE_MIN.0, NODE_MIN.1
        )),
        Err(e) => line(format!("node: PROBLEM {e}")),
    }
    line(format!(
        "display: {}",
        if input.wayland_display {
            "Wayland"
        } else {
            "no WAYLAND_DISPLAY (a window cannot open here)"
        }
    ));
    for (key, value) in &input.display_vars {
        line(format!("  {key}={value}"));
    }
    match &input.probe {
        Some(Ok(text)) => line(format!("engine probe: OK {text}")),
        Some(Err(e)) => line(format!("engine probe: FAILED {e}")),
        None => {
            line("engine probe: not run (add --probe to start the engine and shake hands)".into())
        }
    }
    if !input.engine_log_tail.is_empty() {
        line("engine log (last lines):".into());
        for l in input.engine_log_tail.lines() {
            line(format!("  {l}"));
        }
    }
    redact_home(&out, input.home.as_deref())
}

/// Gather the facts and render the `--diagnose` report. With `probe`, a throwaway engine is
/// started in a scratch profile (offline, no credentials), asked for a handshake, and stopped;
/// the person's own profile and sessions are not touched.
pub fn diagnose(data_dir: &Path, engine_override: Option<&Path>, probe: bool) -> (String, bool) {
    let exe = std::env::current_exe().ok();
    let mut candidates = engine_search_dirs(
        exe.as_deref(),
        std::env::var_os(ENGINE_DIR_ENV).map(PathBuf::from),
    );
    if let Some(dir) = engine_override {
        candidates.insert(0, dir.to_path_buf());
    }
    let engine = find_engine(&candidates);
    let probe_result = probe.then(|| match &engine {
        Ok((dir, _)) => probe_engine(dir),
        Err(_) => Err("no engine to probe".into()),
    });
    let input = ReportInput {
        exe,
        data_dir: data_dir.to_path_buf(),
        schema_version: crate::storage::SCHEMA_VERSION,
        engine,
        engine_override: engine_override.map(Path::to_path_buf),
        node: node_version(),
        wayland_display: std::env::var_os("WAYLAND_DISPLAY").is_some(),
        display_vars: ["WAYLAND_DISPLAY", "XDG_SESSION_TYPE", "XDG_CURRENT_DESKTOP"]
            .iter()
            .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v)))
            .collect(),
        database: crate::storage::inspect(&crate::platform::db_path(data_dir)),
        probe: probe_result,
        engine_log_tail: Some(log_tail(&data_dir.join("engine.log"), 12))
            .filter(|t| !t.starts_with('('))
            .unwrap_or_default(),
        home: std::env::var_os("HOME").map(PathBuf::from),
    };
    let text = report(&input);
    let bad = has_problem(&text);
    (text, bad)
}

/// Start the engine from `dir` in a scratch profile, wait for a trusted handshake, stop it.
fn probe_engine(dir: &Path) -> Result<String, String> {
    use crate::adapters::pi::engine::{Engine, EngineConfig};
    use std::time::{Duration, Instant};
    // Unix socket paths are short; keep the scratch profile directly under the temp directory.
    let root = std::env::temp_dir().join(format!("pipkin-probe-{}", std::process::id()));
    let server_dir = root.join("server");
    let agent_dir = root.join("agent");
    std::fs::create_dir_all(&agent_dir).map_err(|e| e.to_string())?;
    let server_id = std::fs::read_to_string("/proc/sys/kernel/random/uuid")
        .map_err(|e| e.to_string())?
        .trim()
        .to_owned();
    let config = EngineConfig {
        pi_repo: dir.to_path_buf(),
        server_dir,
        server_id,
        agent_dir: Some(agent_dir),
        cwd: root.clone(),
        model: None,
        log_path: root.join("engine.log"),
        env: vec![("PI_OFFLINE".into(), "1".into())],
        extensions: vec![],
    };
    let started = Instant::now();
    let result = Engine::spawn(&config)
        .map_err(|e| format!("{e:?}"))
        .and_then(|mut engine| {
            let ready = engine
                .wait_ready(Duration::from_secs(45))
                .map_err(|e| format!("{e:?}"));
            engine.stop();
            ready
        })
        .map(|()| format!("(ready in {:.1}s)", started.elapsed().as_secs_f32()));
    let _ = std::fs::remove_dir_all(&root);
    result
}

/// Whether the report found anything that stops Pipkin working.
pub fn has_problem(report: &str) -> bool {
    report.contains("PROBLEM") || report.contains("NOT FOUND") || report.contains("FAILED")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine_dir(root: &Path, protocol: u32, min_client: Option<&str>) -> PathBuf {
        let dir = root.join("engine");
        std::fs::create_dir_all(dir.join("node_modules")).unwrap();
        std::fs::write(dir.join("pi-test.sh"), "#!/bin/sh\n").unwrap();
        let min = min_client.map_or(String::new(), |m| format!(r#","minClient":"{m}""#));
        std::fs::write(
            dir.join(MANIFEST_FILE),
            format!(r#"{{"name":"pi","version":"abc123","protocol":{protocol}{min}}}"#),
        )
        .unwrap();
        dir
    }

    #[test]
    fn search_order_is_override_then_relative_to_the_binary_then_system() {
        let dirs = engine_search_dirs(
            Some(Path::new("/opt/p/bin/pipkin")),
            Some(PathBuf::from("/x")),
        );
        assert_eq!(dirs[0], PathBuf::from("/x"));
        assert_eq!(dirs[1], PathBuf::from("/opt/p/lib/pipkin/engine"));
        assert!(dirs.contains(&PathBuf::from("/usr/lib/pipkin/engine")));
        let none = engine_search_dirs(None, Some(PathBuf::new()));
        assert_eq!(
            none[0],
            PathBuf::from("/usr/lib/pipkin/engine"),
            "an empty override is ignored"
        );
    }

    #[test]
    fn a_complete_engine_with_the_right_protocol_is_accepted() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = engine_dir(tmp.path(), PROTOCOL, None);
        let manifest = check_engine(&dir).unwrap();
        assert_eq!(manifest.version, "abc123");
    }

    #[test]
    fn engines_that_cannot_work_are_rejected_with_a_reason() {
        let tmp = tempfile::tempdir().unwrap();
        let wrong = engine_dir(&tmp.path().join("a"), PROTOCOL + 1, None);
        assert!(check_engine(&wrong).unwrap_err().contains("protocol"));
        let newer = engine_dir(&tmp.path().join("b"), PROTOCOL, Some("999.0.0"));
        assert!(
            check_engine(&newer)
                .unwrap_err()
                .contains("needs Pipkin 999.0.0")
        );
        let incomplete = engine_dir(&tmp.path().join("c"), PROTOCOL, None);
        std::fs::remove_dir_all(incomplete.join("node_modules")).unwrap();
        assert!(
            check_engine(&incomplete)
                .unwrap_err()
                .contains("node_modules is missing")
        );
        assert!(check_engine(&tmp.path().join("none")).is_err());
        std::fs::write(tmp.path().join("bad").with_extension("json"), "x").unwrap();
        let broken = tmp.path().join("d");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join(MANIFEST_FILE), "{not json").unwrap();
        assert!(
            check_engine(&broken)
                .unwrap_err()
                .contains("not a valid manifest")
        );
    }

    #[test]
    fn the_first_usable_candidate_wins_and_rejections_are_listed() {
        let tmp = tempfile::tempdir().unwrap();
        let good = engine_dir(&tmp.path().join("g"), PROTOCOL, None);
        let bad = engine_dir(&tmp.path().join("w"), 1, None);
        let missing = tmp.path().join("missing");
        let found = find_engine(&[missing.clone(), bad.clone(), good.clone()]).unwrap();
        assert_eq!(found.0, good);
        let reasons = find_engine(&[missing, bad]).unwrap_err();
        assert_eq!(reasons.len(), 2);
        assert!(reasons[0].contains("not present"));
    }

    #[test]
    fn versions_compare_by_component_not_text() {
        assert!(version_less("0.9.0", "0.10.0"));
        assert!(version_less("0.0.1", "0.0.2"));
        assert!(!version_less("1.0", "1.0.0"));
        assert!(!version_less("2.0.0", "1.9.9"));
    }

    #[test]
    fn node_versions_parse_and_are_compared_to_the_minimum() {
        assert_eq!(parse_node_version("v22.19.0\n").unwrap(), (22, 19, 0));
        assert!(parse_node_version("nonsense").is_err());
        assert!(node_is_new_enough((22, 19, 0)));
        assert!(node_is_new_enough((26, 8, 1)));
        assert!(!node_is_new_enough((22, 18, 9)));
        assert!(!node_is_new_enough((20, 0, 0)));
    }

    #[test]
    fn a_report_hides_the_home_directory_and_flags_problems() {
        let input = ReportInput {
            exe: Some(PathBuf::from("/home/ann/bin/pipkin")),
            data_dir: PathBuf::from("/home/ann/.local/share/pipkin"),
            schema_version: 7,
            engine: Err(vec!["/usr/lib/pipkin/engine: not present".into()]),
            engine_override: None,
            node: Ok((20, 1, 0)),
            wayland_display: false,
            display_vars: vec![],
            database: Ok("ok".into()),
            probe: None,
            engine_log_tail: String::new(),
            home: Some(PathBuf::from("/home/ann")),
        };
        let text = report(&input);
        assert!(!text.contains("/home/ann"), "{text}");
        assert!(text.contains("~/.local/share/pipkin"));
        assert!(text.contains("engine: NOT FOUND"));
        assert!(text.contains("older than 22.19"));
        assert!(has_problem(&text));
        assert!(!has_problem("Pipkin 0.0.1\nengine probe: OK"));
    }

    #[test]
    fn the_log_tail_keeps_only_the_last_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("log");
        std::fs::write(&path, "a\nb\nc\nd\n").unwrap();
        assert_eq!(log_tail(&path, 2), "c\nd");
        assert!(log_tail(&tmp.path().join("none"), 2).starts_with('('));
    }
}
