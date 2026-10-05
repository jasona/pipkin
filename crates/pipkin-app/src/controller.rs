//! The application controller: wires storage, the demo backend and the GPUI `Model`.
//!
//! Effects emitted by the core are executed here, never in render. Backend events arrive on an
//! async channel and are applied on the foreground in coalesced batches.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui::{App, AppContext as _, Entity};
use pipkin_core::*;
use pipkin_ui::{DemoControls, Model};

use crate::adapters::pi::engine::EngineConfig;
use crate::adapters::pi::{PiBackend, PiConfig};
use crate::adapters::script;
use crate::adapters::{DemoBackend, DemoOptions};
use crate::platform;
use crate::storage::{
    DEMO_NAMESPACE, DemoConversation, Loaded, OpenRequest, Startup, Storage, StoredDraft,
};

const CLOCK_REFRESH: Duration = Duration::from_secs(15);
const EVENT_QUEUE: usize = 1024;
const MAX_BATCH: usize = 512;

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub mode: Mode,
    pub scenario: String,
    pub data_dir: Option<PathBuf>,
    pub speed: f32,
    /// Real mode: the Pi server profile directory (default `PI_SERVER_DIR` or `~/.pi/server`).
    pub pi_dir: Option<PathBuf>,
    /// Real mode: connect to this logical server instead of discovering one.
    pub pi_server_id: Option<String>,
    /// Real mode: a Pi checkout to launch and own an engine from (the development entry point).
    /// Without it, an engine must already be running.
    pub pi_repo: Option<PathBuf>,
    /// Real mode: the agent directory (credentials, models, sessions) for a launched engine.
    pub pi_agent_dir: Option<PathBuf>,
    /// Real mode: project folders to add at startup (repeatable); the last one is selected.
    pub projects: Vec<PathBuf>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            mode: Mode::Real,
            scenario: "normal".into(),
            data_dir: None,
            speed: 1.0,
            pi_dir: None,
            pi_server_id: None,
            pi_repo: None,
            pi_agent_dir: None,
            projects: vec![],
        }
    }
}

impl Options {
    pub fn from_env_args() -> Result<Options, String> {
        Options::parse(std::env::args().skip(1))
    }

    /// Real mode is the default; `--demo <scenario>` selects the simulated backend.
    /// Parse `--demo <scenario>`, `--data-dir <path>` and `--speed <factor>`
    /// (each also accepted as `--flag=value`).
    pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Options, String> {
        let mut options = Options::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let (flag, inline) = match arg.split_once('=') {
                Some((f, v)) => (f.to_string(), Some(v.to_string())),
                None => (arg, None),
            };
            let mut value = |name: &str| {
                inline
                    .clone()
                    .or_else(|| args.next())
                    .ok_or_else(|| format!("{name} needs a value"))
            };
            match flag.as_str() {
                "--demo" => {
                    let name = value("--demo")?;
                    if !script::scenario_names().contains(&name.as_str()) {
                        return Err(format!(
                            "unknown scenario {name:?}; choose one of: {}",
                            script::scenario_names().join(", ")
                        ));
                    }
                    options.scenario = name;
                    options.mode = Mode::Demo;
                }
                "--data-dir" => options.data_dir = Some(PathBuf::from(value("--data-dir")?)),
                "--pi-dir" => options.pi_dir = Some(PathBuf::from(value("--pi-dir")?)),
                "--pi-repo" => options.pi_repo = Some(PathBuf::from(value("--pi-repo")?)),
                "--project" => options.projects.push(PathBuf::from(value("--project")?)),
                "--pi-agent-dir" => {
                    options.pi_agent_dir = Some(PathBuf::from(value("--pi-agent-dir")?))
                }
                "--pi-server-id" => {
                    let id = value("--pi-server-id")?;
                    if !pi_client::protocol::is_server_id(&id) {
                        return Err(format!(
                            "--pi-server-id must be a lowercase UUIDv4, got {id:?}"
                        ));
                    }
                    options.pi_server_id = Some(id);
                }
                "--speed" => {
                    let raw = value("--speed")?;
                    let speed: f32 = raw
                        .parse()
                        .map_err(|_| format!("invalid --speed {raw:?}"))?;
                    if !speed.is_finite() || speed < 0.0 {
                        return Err(format!("--speed must be a non-negative number, got {raw}"));
                    }
                    options.speed = speed;
                }
                other => return Err(format!("unknown argument {other:?}")),
            }
        }
        Ok(options)
    }
}

/// Entropy for request IDs so they stay unique across restarts. The core takes it as data.
fn request_prefix(now: i64) -> String {
    use std::hash::{BuildHasher, Hasher};
    let random = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    format!("{now:x}{random:016x}")
}

fn unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Merge consecutive token deltas for the same operation so one paint covers a whole burst.
fn coalesce(batch: Vec<BackendEvent>) -> Vec<BackendEvent> {
    let mut out: Vec<BackendEvent> = Vec::with_capacity(batch.len());
    for event in batch {
        if let (EventKind::Token(next), Some(last)) = (&event.kind, out.last_mut())
            && let EventKind::Token(prev) = &mut last.kind
            && last.conversation == event.conversation
            && last.generation == event.generation
            && last.op == event.op
        {
            prev.push_str(next);
            continue;
        }
        out.push(event);
    }
    out
}

/// The Pi server profile directory this run talks to.
fn profile_dir(options: &Options) -> PathBuf {
    options
        .pi_dir
        .clone()
        .unwrap_or_else(crate::adapters::pi::default_directory)
}

/// Desktop data is namespaced by backend so identities from one never apply to another. A real
/// backend is identified by its profile directory, so two engines never share drafts, journals
/// or selections.
fn namespace(options: &Options) -> String {
    match options.mode {
        Mode::Demo => DEMO_NAMESPACE.to_owned(),
        Mode::Real => format!(
            "pi:{:x}",
            stable_id(&profile_dir(options).display().to_string())
        ),
    }
}

/// The engine to launch and own, for `--pi-repo`. Its server identity is the one given, else the
/// profile's `default-server-id` (the identity Pi itself would use), else a fresh one that is
/// remembered there so the next launch is the same logical server.
fn managed_engine(options: &Options, pi_repo: &Path) -> Result<EngineConfig, String> {
    let server_dir = profile_dir(options);
    let id_file = server_dir.join("default-server-id");
    let server_id = match &options.pi_server_id {
        Some(id) => id.clone(),
        None => match std::fs::read_to_string(&id_file) {
            Ok(text) if pi_client::protocol::is_server_id(text.trim()) => text.trim().to_owned(),
            _ => {
                let id = std::fs::read_to_string("/proc/sys/kernel/random/uuid")
                    .map_err(|e| format!("cannot create a server id: {e}"))?
                    .trim()
                    .to_owned();
                std::fs::create_dir_all(&server_dir)
                    .and_then(|_| std::fs::write(&id_file, &id))
                    .map_err(|e| {
                        format!(
                            "cannot remember the server id in {}: {e}",
                            id_file.display()
                        )
                    })?;
                id
            }
        },
    };
    let cwd = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| PathBuf::from("/"));
    Ok(EngineConfig {
        pi_repo: pi_repo.to_path_buf(),
        log_path: platform::data_dir(options.data_dir.as_deref()).join("engine.log"),
        server_dir,
        server_id,
        agent_dir: options.pi_agent_dir.clone(),
        cwd,
        model: None,
        env: vec![],
    })
}

/// Open the desktop database, and say when it is not the one the user expects: a damaged file
/// that was set aside, or no usable database at all. In the second case Pipkin keeps working
/// against a throwaway database so a prompt can still be journaled, and says plainly that
/// nothing survives the session.
fn open_storage(options: &Options) -> (Arc<Storage>, Option<String>) {
    let dir = platform::data_dir(options.data_dir.as_deref());
    let ns = namespace(options);
    match Storage::open_recovering(&platform::db_path(&dir), &ns) {
        Ok((storage, Startup::Clean)) => (Arc::new(storage), None),
        Ok((storage, Startup::Recovered(message))) => {
            log::error!("{message}");
            (Arc::new(storage), Some(message))
        }
        Err(e) => {
            let fallback =
                std::env::temp_dir().join(format!("pipkin-fallback-{}", std::process::id()));
            log::error!(
                "cannot open storage in {}: {e}; using {}",
                dir.display(),
                fallback.display()
            );
            let storage = Storage::open(&platform::db_path(&fallback), &ns)
                .expect("a throwaway database in the temporary directory");
            let message = format!(
                "Pipkin cannot use its saved data ({e}). Your drafts and unfinished sends will \
                 not survive closing the app, and anything saved earlier is not shown. Copy \
                 anything you need to keep."
            );
            (Arc::new(storage), Some(message))
        }
    }
}

/// Stored state waiting for its conversation to appear in a catalog. Real sessions come from
/// the engine, possibly after startup or a reconnect, so each catalog restores whatever now has
/// a home and keeps the rest for later.
pub(crate) struct Restore {
    /// Demo-only conversations kept in the desktop database.
    conversations: Vec<DemoConversation>,
    drafts: Vec<StoredDraft>,
    requests: Vec<OpenRequest>,
}

impl Restore {
    pub(crate) fn new(loaded: Loaded, include_demo_conversations: bool) -> Restore {
        Restore {
            conversations: if include_demo_conversations {
                loaded.conversations
            } else {
                vec![]
            },
            drafts: loaded.drafts,
            requests: loaded.open_requests,
        }
    }

    pub(crate) fn apply(&mut self, state: &mut AppState) -> Outcome {
        let mut out = Outcome::default();
        for c in self.conversations.drain(..) {
            state.restore_conversation(c.id, c.project, c.title, c.updated_at);
        }
        self.drafts.retain(|d| {
            if state.conversation(d.conversation).is_none() {
                return true;
            }
            // The files may have moved or changed since the draft was saved.
            let attachments = d.attachments.iter().map(revalidate_attachment).collect();
            state.restore_draft(d.conversation, d.text.clone(), attachments);
            false
        });
        // Never resend: an unresolved request becomes "outcome unknown" for the user to resolve.
        self.requests.retain(|r| {
            if state.conversation(r.conversation).is_none() {
                return true;
            }
            out.merge(state.restore_unresolved(
                r.conversation,
                r.request.clone(),
                r.text.clone(),
                r.attachments.clone(),
            ));
            false
        });
        out
    }
}

pub fn start(cx: &mut App, options: Options) -> Entity<Model> {
    let (storage, mut storage_issue) = open_storage(&options);
    let loaded = storage.load_all().unwrap_or_else(|e| {
        log::error!("cannot read stored state: {e}");
        storage_issue.get_or_insert(format!(
            "Pipkin could not read its saved data ({e}), so earlier drafts and unfinished \
             sends are not shown."
        ));
        crate::storage::Loaded {
            prefs: Prefs::default(),
            drafts: vec![],
            conversations: vec![],
            open_requests: vec![],
            projects: vec![],
        }
    });

    let now = unix_secs();
    let (event_tx, event_rx) = async_channel::bounded::<BackendEvent>(EVENT_QUEUE);
    let demo = (options.mode == Mode::Demo).then(|| {
        Arc::new(DemoBackend::new(
            event_tx.clone(),
            DemoOptions {
                speed: options.speed,
                scenario: options.scenario.clone(),
                // Fixtures are relative to the start of today (UTC), so recency reads naturally.
                base_time: now - now % 86_400,
                ..DemoOptions::default()
            },
        ))
    });
    let backend: Arc<dyn Backend> = match &demo {
        Some(demo) => demo.clone(),
        None => {
            let config = match &options.pi_repo {
                Some(repo) => match managed_engine(&options, repo) {
                    Ok(engine) => PiConfig::managed(engine),
                    Err(error) => {
                        // Say why the engine cannot be launched; do not quietly fall back.
                        eprintln!("pipkin: cannot manage a Pi engine: {error}");
                        std::process::exit(2);
                    }
                },
                None => {
                    let mut config = PiConfig::new(profile_dir(&options));
                    config.server_id = options.pi_server_id.clone();
                    config
                }
            };
            Arc::new(PiBackend::new(event_tx, config))
        }
    };

    // State starts empty and `Connecting`; the backend's catalog arrives asynchronously.
    let mut state = AppState::new(
        Bootstrap {
            projects: vec![],
            models: vec![],
            conversations: vec![],
            now,
        },
        loaded.prefs.clone(),
    );
    state.mode = options.mode;
    if let Some(issue) = storage_issue {
        state.set_storage_issue(issue);
    }
    state.set_now(now);
    state.set_request_prefix(request_prefix(now));
    state.set_connection(Connection::Connecting);
    if options.mode == Mode::Real {
        state.can_create = true;
        // Folders the user opened are known before the engine answers, so the project list
        // is usable while connecting.
        for path in &loaded.projects {
            state.restore_project(path);
        }
        // Folders named on the command line: resolved like the folder picker's, remembered, and
        // the last one is where new conversations go.
        for path in &options.projects {
            match std::fs::canonicalize(path) {
                Ok(real) if real.is_dir() => {
                    let real = real.display().to_string();
                    state.restore_project(&real);
                    storage.save_project(real.clone());
                    state.prefs.selected_project = Some(project_id_for_path(&real));
                }
                _ => eprintln!(
                    "pipkin: --project {} is not a folder; ignoring it",
                    path.display()
                ),
            }
        }
    }
    // Stored demo conversations must not leak into real mode. Drafts and unresolved requests
    // are namespaced per backend, so they are restored in either mode, as their conversations
    // appear. A real backend reports its own connection state through `start`.
    let mut restore = Restore::new(loaded, options.mode == Mode::Demo);

    let model = cx.new(|_| Model::new(state));
    let (ack_tx, ack_rx) = async_channel::unbounded::<(ConversationId, u64, Result<(), String>)>();
    let (intent_tx, intent_rx) =
        async_channel::unbounded::<(ConversationId, RequestId, Result<(), String>)>();

    {
        let backend = backend.clone();
        let storage = storage.clone();
        model.update(cx, |m, _| {
            m.set_effect_handler(Box::new(move |effect, cx| match effect {
                Effect::Backend(request) => backend.request(request),
                Effect::SaveDraft {
                    conversation,
                    text,
                    attachments,
                    rev,
                } => {
                    let ack_tx = ack_tx.clone();
                    storage.save_draft(conversation, text, attachments, rev, move |result| {
                        let _ = ack_tx.send_blocking((conversation, rev, result));
                    });
                }
                Effect::JournalIntent {
                    conversation,
                    request,
                    text,
                    attachments,
                    model,
                } => {
                    let intent_tx = intent_tx.clone();
                    let key = request.clone();
                    storage.journal_intent(
                        conversation,
                        request,
                        &text,
                        &attachments,
                        model.as_deref(),
                        move |result| {
                            let _ = intent_tx.send_blocking((conversation, key, result));
                        },
                    );
                }
                Effect::JournalState { request, state, .. } => {
                    storage.journal_state(request, state)
                }
                Effect::SavePrefs(prefs) => storage.save_prefs(&prefs),
                Effect::SaveProject { path } => storage.save_project(path),
                Effect::SaveConversation { conversation } => {
                    // The model is mid-update here; read the conversation once it settles.
                    let storage = storage.clone();
                    cx.spawn(async move |this, cx| {
                        let snapshot = this
                            .read_with(cx, |m, _| {
                                m.state
                                    .conversation(conversation)
                                    .map(|c| DemoConversation {
                                        id: c.id,
                                        project: c.project,
                                        title: c.title.clone(),
                                        updated_at: c.updated_at,
                                    })
                            })
                            .ok()
                            .flatten();
                        if let Some(snapshot) = snapshot {
                            storage.save_conversation(snapshot);
                        }
                    })
                    .detach();
                }
            }));
        });
    }

    // Backend events: drain everything queued per wakeup and apply as one batch.
    {
        let weak = model.downgrade();
        cx.spawn(async move |cx| {
            while let Ok(first) = event_rx.recv().await {
                let mut batch = vec![first];
                while batch.len() < MAX_BATCH {
                    match event_rx.try_recv() {
                        Ok(event) => batch.push(event),
                        Err(_) => break,
                    }
                }
                let batch = coalesce(batch);
                let alive = weak.update(cx, |m, cx| {
                    for event in batch {
                        m.apply_event(event, cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    // Draft save acknowledgments (delivered by the storage writer after commit).
    {
        let weak = model.downgrade();
        cx.spawn(async move |cx| {
            while let Ok(first) = ack_rx.recv().await {
                let mut batch = vec![first];
                while let Ok(next) = ack_rx.try_recv() {
                    batch.push(next);
                }
                let alive = weak.update(cx, |m, cx| {
                    for (conversation, rev, result) in batch {
                        m.draft_saved(conversation, rev, result, cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    // Journal commit acknowledgments: only now may a submission be sent.
    {
        let weak = model.downgrade();
        cx.spawn(async move |cx| {
            while let Ok((conversation, request, result)) = intent_rx.recv().await {
                let alive = weak.update(cx, |m, cx| {
                    m.mutate(
                        |state| state.intent_persisted(conversation, &request, result),
                        cx,
                    );
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    // Wall clock for the core, refreshed periodically (the core never reads a clock itself).
    {
        let weak = model.downgrade();
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(CLOCK_REFRESH).await;
                let now = unix_secs();
                if weak.update(cx, |m, _| m.state.set_now(now)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    // Lifecycle updates: connection state and the catalog. The sink never drops an update.
    let (life_tx, life_rx) = async_channel::unbounded::<LifecycleEvent>();
    {
        let tx = life_tx.clone();
        storage.set_error_sink(Arc::new(move |message| {
            let _ = tx.send_blocking(LifecycleEvent::StorageIssue(message));
        }));
    }
    {
        let weak = model.downgrade();
        cx.spawn(async move |cx| {
            while let Ok(event) = life_rx.recv().await {
                let restore = &mut restore;
                let alive = weak.update(cx, |m, cx| match event {
                    LifecycleEvent::Connection(c) => m.set_connection(c, cx),
                    LifecycleEvent::Catalog(boot) => {
                        m.mutate(
                            |state| {
                                let mut out = state.apply_catalog(boot);
                                out.merge(restore.apply(state));
                                out.merge(state.select_initial());
                                out
                            },
                            cx,
                        );
                    }
                    LifecycleEvent::ModelSelected(model) => {
                        m.mutate(|state| state.set_engine_model(model), cx)
                    }
                    LifecycleEvent::Notice(message) => {
                        m.mutate(|state| state.set_notice(message), cx)
                    }
                    LifecycleEvent::StorageIssue(message) => {
                        m.mutate(|state| state.set_storage_issue(message), cx)
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }
    backend.start(Box::new(move |event| {
        let _ = life_tx.send_blocking(event);
    }));

    if let Some(demo) = demo {
        cx.set_global(DemoControls {
            scenarios: script::SCENARIOS.iter().map(|s| (s.0, s.1)).collect(),
            current_scenario: Rc::new({
                let backend = demo.clone();
                move || backend.scenario()
            }),
            set_scenario: Rc::new({
                let backend = demo.clone();
                move |name| {
                    backend.set_scenario(name);
                }
            }),
            save_failure: Rc::new({
                let storage = storage.clone();
                move || storage.write_failure()
            }),
            set_save_failure: Rc::new({
                let storage = storage.clone();
                move |fail| storage.set_write_failure(fail)
            }),
        });
    }

    // Orderly exit: flush every dirty draft, then drain and join the storage writer.
    {
        let weak = model.downgrade();
        let storage = storage.clone();
        let backend = backend.clone();
        cx.on_app_quit(move |cx| {
            let _ = weak.update(cx, |m, cx| {
                let dirty: Vec<ConversationId> = m
                    .state
                    .conversations
                    .iter()
                    .filter(|c| c.draft.save == SaveState::Dirty)
                    .map(|c| c.id)
                    .collect();
                for id in dirty {
                    m.dispatch(Command::FlushDraft(id), cx);
                }
            });
            let storage = storage.clone();
            let backend = backend.clone();
            let background = cx.background_executor().clone();
            async move {
                background
                    .spawn(async move {
                        // Stopping an engine this app launched can take a few seconds; it must
                        // not freeze the window, and it must finish before the process exits.
                        backend.shutdown();
                        storage.shutdown();
                    })
                    .await;
            }
        })
        .detach();
    }

    model
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options, String> {
        Options::parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn defaults_and_flags() {
        assert_eq!(parse(&[]).unwrap(), Options::default());
        assert_eq!(Options::default().mode, Mode::Real);
        let o = parse(&["--demo", "followup", "--data-dir=/tmp/x", "--speed", "0.5"]).unwrap();
        assert_eq!(o.scenario, "followup");
        assert_eq!(o.mode, Mode::Demo);
        assert_eq!(o.data_dir, Some(PathBuf::from("/tmp/x")));
        assert_eq!(o.speed, 0.5);
    }

    #[test]
    fn real_mode_flags() {
        let o = parse(&[
            "--pi-repo=/pi",
            "--pi-dir",
            "/srv",
            "--pi-agent-dir=/agent",
            "--project",
            "/work/a",
            "--project=/work/b",
        ])
        .unwrap();
        assert_eq!(o.mode, Mode::Real);
        assert_eq!(o.pi_repo, Some(PathBuf::from("/pi")));
        assert_eq!(o.pi_dir, Some(PathBuf::from("/srv")));
        assert_eq!(o.pi_agent_dir, Some(PathBuf::from("/agent")));
        assert_eq!(
            o.projects,
            [PathBuf::from("/work/a"), PathBuf::from("/work/b")]
        );
        assert!(parse(&["--pi-server-id", "nope"]).is_err());
        assert!(parse(&["--project"]).is_err());
    }

    #[test]
    fn the_namespace_names_the_backend_and_the_profile() {
        let demo = parse(&["--demo", "normal"]).unwrap();
        assert_eq!(namespace(&demo), DEMO_NAMESPACE);
        let a = parse(&["--pi-dir", "/srv/a"]).unwrap();
        let b = parse(&["--pi-dir", "/srv/b"]).unwrap();
        assert!(namespace(&a).starts_with("pi:"));
        assert_ne!(
            namespace(&a),
            namespace(&b),
            "two profiles never share drafts or journals"
        );
        assert_eq!(
            namespace(&a),
            namespace(&parse(&["--pi-dir", "/srv/a"]).unwrap())
        );
    }

    #[test]
    fn rejects_bad_input() {
        assert!(
            parse(&["--demo", "nope"])
                .unwrap_err()
                .contains("unknown scenario")
        );
        assert!(parse(&["--speed", "-1"]).is_err());
        assert!(parse(&["--speed"]).is_err());
        assert!(parse(&["--bogus"]).is_err());
    }

    #[test]
    fn coalesce_merges_only_matching_tokens() {
        let ev = |op: u64, kind| BackendEvent {
            conversation: ConversationId(1),
            generation: 1,
            op: Some(OperationId(op)),
            kind,
        };
        let batch = vec![
            ev(1, EventKind::Token("a ".into())),
            ev(1, EventKind::Token("b ".into())),
            ev(2, EventKind::Token("c".into())),
            ev(2, EventKind::Completed),
        ];
        let out = coalesce(batch);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].kind, EventKind::Token("a b ".into()));
    }
}
