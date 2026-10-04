//! The application controller: wires storage, the demo backend and the GPUI `Model`.
//!
//! Effects emitted by the core are executed here, never in render. Backend events arrive on an
//! async channel and are applied on the foreground in coalesced batches.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use desktop_core::*;
use desktop_ui::{DemoControls, Model};
use gpui::{App, AppContext as _, Entity};

use crate::adapters::script;
use crate::adapters::{DemoBackend, DemoOptions};
use crate::platform;
use crate::storage::{DemoConversation, Storage};

const CLOCK_REFRESH: Duration = Duration::from_secs(15);
const EVENT_QUEUE: usize = 1024;
const MAX_BATCH: usize = 512;

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub scenario: String,
    pub data_dir: Option<PathBuf>,
    pub speed: f32,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            scenario: "normal".into(),
            data_dir: None,
            speed: 1.0,
        }
    }
}

impl Options {
    pub fn from_env_args() -> Result<Options, String> {
        Options::parse(std::env::args().skip(1))
    }

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
                }
                "--data-dir" => options.data_dir = Some(PathBuf::from(value("--data-dir")?)),
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

fn open_storage(options: &Options) -> Arc<Storage> {
    let dir = platform::data_dir(options.data_dir.as_deref());
    match Storage::open(&platform::db_path(&dir)) {
        Ok(storage) => Arc::new(storage),
        Err(e) => {
            // Keep the app usable; drafts will not survive this session.
            let fallback =
                std::env::temp_dir().join(format!("pi-desktop-fallback-{}", std::process::id()));
            log::error!(
                "cannot open storage in {}: {e}; using {}",
                dir.display(),
                fallback.display()
            );
            Arc::new(Storage::open(&platform::db_path(&fallback)).expect("fallback storage"))
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
        }
    });

    let now = unix_secs();
    let (event_tx, event_rx) = async_channel::bounded::<BackendEvent>(EVENT_QUEUE);
    let backend = Arc::new(DemoBackend::new(
        event_tx,
        DemoOptions {
            speed: options.speed,
            scenario: options.scenario.clone(),
            // Fixtures are relative to the start of today (UTC), so recency reads naturally.
            base_time: now - now % 86_400,
            ..DemoOptions::default()
        },
    ));

    // Build the complete state before the entity exists.
    let mut state = AppState::new(backend.bootstrap(), loaded.prefs.clone());
    state.set_now(now);
    for c in loaded.conversations {
        state.restore_conversation(c.id, c.project, c.title, c.updated_at);
    }
    for d in loaded.drafts {
        state.restore_draft(d.conversation, d.text);
    }
    if let Some(selected) = loaded
        .prefs
        .selected_conversation
        .filter(|id| state.conversation(*id).is_some())
    {
        state.selected = Some(selected);
    }

    let model = cx.new(|_| Model::new(state));
    let (ack_tx, ack_rx) = async_channel::unbounded::<(ConversationId, u64, Result<(), String>)>();

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

    // Open the previously selected conversation so its history loads.
    model.update(cx, |m, cx| {
        if let Some(id) = m.state.selected {
            m.dispatch(Command::SelectConversation(id), cx);
        }
    });

    cx.set_global(DemoControls {
        scenarios: script::SCENARIOS.iter().map(|s| (s.0, s.1)).collect(),
        current_scenario: Rc::new({
            let backend = backend.clone();
            move || backend.scenario()
        }),
        set_scenario: Rc::new({
            let backend = backend.clone();
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

    // Orderly exit: flush every dirty draft, then drain and join the storage writer.
    {
        let weak = model.downgrade();
        let storage = storage.clone();
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
        let o = parse(&["--demo", "followup", "--data-dir=/tmp/x", "--speed", "0.5"]).unwrap();
        assert_eq!(o.scenario, "followup");
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
