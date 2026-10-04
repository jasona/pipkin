//! The application controller: wires storage, the demo backend and the GPUI `Model`.
//!
//! Effects emitted by the core are executed here, never in render. Backend events arrive on an
//! async channel and are applied on the foreground in coalesced batches.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui::{App, AppContext as _, Entity};
use pipkin_core::*;
use pipkin_ui::{DemoControls, Model};

use crate::adapters::pi::{PiBackend, PiConfig};
use crate::adapters::script;
use crate::adapters::{DemoBackend, DemoOptions};
use crate::platform;
use crate::storage::{DEMO_NAMESPACE, DemoConversation, Storage};

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

/// Desktop data is namespaced by backend so identities from one never apply to another.
/// The real namespace becomes `pi:<profile>` once the engine host defines profiles.
fn namespace(mode: Mode) -> &'static str {
    match mode {
        Mode::Demo => DEMO_NAMESPACE,
        Mode::Real => "pi:local",
    }
}

fn open_storage(options: &Options) -> Arc<Storage> {
    let dir = platform::data_dir(options.data_dir.as_deref());
    let ns = namespace(options.mode);
    match Storage::open(&platform::db_path(&dir), ns) {
        Ok(storage) => Arc::new(storage),
        Err(e) => {
            // Keep the app usable; drafts will not survive this session.
            let fallback =
                std::env::temp_dir().join(format!("pipkin-fallback-{}", std::process::id()));
            log::error!(
                "cannot open storage in {}: {e}; using {}",
                dir.display(),
                fallback.display()
            );
            Arc::new(Storage::open(&platform::db_path(&fallback), ns).expect("fallback storage"))
        }
    }
}

pub fn start(cx: &mut App, options: Options) -> Entity<Model> {
    let storage = open_storage(&options);
    let loaded = storage.load_all().unwrap_or_else(|e| {
        log::error!("cannot read stored state: {e}");
        crate::storage::Loaded {
            prefs: Prefs::default(),
            drafts: vec![],
            conversations: vec![],
            open_requests: vec![],
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
            let mut config = PiConfig::new(
                options
                    .pi_dir
                    .clone()
                    .unwrap_or_else(crate::adapters::pi::default_directory),
            );
            config.server_id = options.pi_server_id.clone();
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
    state.set_now(now);
    state.set_request_prefix(request_prefix(now));
    state.set_connection(Connection::Connecting);
    if options.mode == Mode::Real {
        state.read_only = Some(
            "This version can read Pi sessions but cannot run them yet, so prompts cannot be sent."
                .into(),
        );
    }
    // Stored demo conversations and numeric-ID drafts must not leak into real mode. A real
    // backend reports its own connection state through `start`.
    let mut restore = (options.mode == Mode::Demo).then_some(loaded);

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
                    rev,
                } => {
                    let ack_tx = ack_tx.clone();
                    storage.save_draft(conversation, text, rev, move |result| {
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
        let weak = model.downgrade();
        cx.spawn(async move |cx| {
            while let Ok(event) = life_rx.recv().await {
                let restore = &mut restore;
                let alive = weak.update(cx, |m, cx| match event {
                    LifecycleEvent::Connection(c) => m.set_connection(c, cx),
                    LifecycleEvent::Catalog(boot) => {
                        let stored = restore.take();
                        m.mutate(
                            |state| {
                                let mut out = state.apply_catalog(boot);
                                if let Some(stored) = stored {
                                    for c in stored.conversations {
                                        state.restore_conversation(
                                            c.id,
                                            c.project,
                                            c.title,
                                            c.updated_at,
                                        );
                                    }
                                    for d in stored.drafts {
                                        state.restore_draft(d.conversation, d.text);
                                    }
                                    // Never resend: unresolved requests become "outcome unknown".
                                    for r in stored.open_requests {
                                        out.merge(state.restore_unresolved(
                                            r.conversation,
                                            r.request,
                                            r.text,
                                            r.attachments,
                                        ));
                                    }
                                }
                                out.merge(state.select_initial());
                                out
                            },
                            cx,
                        );
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
            backend.shutdown();
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
            let background = cx.background_executor().clone();
            async move {
                background
                    .spawn(async move {
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
