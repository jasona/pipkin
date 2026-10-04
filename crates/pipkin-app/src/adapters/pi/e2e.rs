//! End-to-end tests against a REAL Pi engine with a scripted provider. Opt-in:
//!
//! ```text
//! PIPKIN_PI_REPO=<pi checkout> cargo test -p pipkin-app e2e -- --ignored --nocapture
//! ```
//!
//! The engine, its coding tools, its durable transcript and its session storage are real; only
//! the model's answers are scripted. A harness drives the real core state machine, the real
//! `PiBackend`, and the real SQLite journal. A fault proxy between them can drop the engine's
//! reply to a prompt (or the prompt itself) and sever the connection.

use std::collections::{HashSet, VecDeque};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use pi_client::cbor::{self, Limits};
use pi_client::chord::parse_service_call;
use pi_client::frame::{DEFAULT_MAX_FRAME_LENGTH, FrameDecoder, encode_frame};
use pi_client::protocol::{
    ClientMessage, ServerMessage, parse_client_message, parse_server_message,
};
use pipkin_core::{
    AppState, Backend, BackendEvent, Bootstrap, Command, Connection, ConversationId, Delivery,
    Effect, ItemKind, LifecycleEvent, Mode, Outcome, RequestId, RunState, ToolStatus,
};
use serde_json::{Value, json};

use super::engine::{EngineHost, profile_pids};
use super::testsupport::{Fixture, Prepared, Reply, is_tool_result_turn, prepare};
use super::{PiBackend, PiConfig};
use crate::controller::Restore;
use crate::storage::Storage;

const NAMESPACE: &str = "pi:e2e";
const WAIT: Duration = Duration::from_secs(90);

/// The model's script: edit a tracked file, create a new one, then say what it did.
fn two_edits(request: &Value, n: usize) -> Reply {
    match n {
        0 => Reply::Tool {
            lead: Some("Updating the notes. ".into()),
            name: "write".into(),
            args: json!({ "path": "notes.txt", "content": "one\ntwo\n" }),
        },
        1 => Reply::Tool {
            lead: None,
            name: "write".into(),
            args: json!({ "path": "hello.txt", "content": "hello from the stub\n" }),
        },
        _ => {
            assert!(
                is_tool_result_turn(request),
                "the final turn shows a tool result"
            );
            Reply::Text("Done: updated notes.txt and created hello.txt.".into())
        }
    }
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Process::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .status()
        .expect("git runs")
        .success();
    assert!(ok, "git {args:?}");
}

/// A project with one committed file, so edits show up as a real diff.
fn init_project(project: &Path) {
    git(project, &["init", "-q", "-b", "main"]);
    std::fs::write(project.join("notes.txt"), "one\n").unwrap();
    git(project, &["add", "-A"]);
    git(project, &["commit", "-q", "-m", "init"]);
}

// --------------------------------------------------------------------------- the harness

/// The application as the controller composes it, minus the window: real state machine, real
/// backend, real SQLite storage, with effects executed the way the controller executes them.
struct Harness {
    state: AppState,
    backend: PiBackend,
    storage: Arc<Storage>,
    restore: Restore,
    events: async_channel::Receiver<BackendEvent>,
    life: Arc<Mutex<VecDeque<LifecycleEvent>>>,
    acks: mpsc::Receiver<(ConversationId, RequestId, Result<(), String>)>,
    ack_tx: mpsc::Sender<(ConversationId, RequestId, Result<(), String>)>,
    notices: Vec<String>,
}

fn unique(prefix: &str) -> String {
    static N: AtomicUsize = AtomicUsize::new(0);
    format!("{prefix}{}", N.fetch_add(1, Ordering::SeqCst))
}

impl Harness {
    fn start(config: PiConfig, db: &Path) -> Harness {
        let storage = Arc::new(Storage::open(db, NAMESPACE).expect("storage"));
        let loaded = storage.load_all().expect("load");
        let mut state = AppState::new(
            Bootstrap {
                projects: vec![],
                models: vec![],
                conversations: vec![],
                now: 1,
            },
            loaded.prefs.clone(),
        );
        state.mode = Mode::Real;
        state.can_create = true;
        state.set_request_prefix(unique("e2e"));
        state.set_connection(Connection::Connecting);
        for path in &loaded.projects {
            state.restore_project(path);
        }
        let restore = Restore::new(loaded, false);
        let (tx, events) = async_channel::unbounded();
        let backend = PiBackend::new(tx, config);
        let life = Arc::new(Mutex::new(VecDeque::new()));
        let sink = {
            let life = life.clone();
            Box::new(move |e| life.lock().unwrap().push_back(e))
        };
        backend.start(sink);
        let (ack_tx, acks) = mpsc::channel();
        Harness {
            state,
            backend,
            storage,
            restore,
            events,
            life,
            acks,
            ack_tx,
            notices: vec![],
        }
    }

    fn exec(&mut self, outcome: Outcome) {
        for effect in outcome.effects {
            match effect {
                Effect::Backend(request) => self.backend.request(request),
                Effect::SaveDraft {
                    conversation, rev, ..
                } => self.state.draft_saved(conversation, rev, Ok(())),
                Effect::SavePrefs(prefs) => self.storage.save_prefs(&prefs),
                Effect::SaveProject { path } => self.storage.save_project(path),
                Effect::SaveConversation { .. } => {}
                Effect::JournalIntent {
                    conversation,
                    request,
                    text,
                    attachments,
                    model,
                } => {
                    let tx = self.ack_tx.clone();
                    let key = request.clone();
                    self.storage.journal_intent(
                        conversation,
                        request,
                        &text,
                        &attachments,
                        model.as_deref(),
                        move |r| {
                            let _ = tx.send((conversation, key, r));
                        },
                    );
                }
                Effect::JournalState { request, state, .. } => {
                    self.storage.journal_state(request, state)
                }
            }
        }
    }

    fn dispatch(&mut self, command: Command) {
        let outcome = self.state.dispatch(command);
        self.exec(outcome);
    }

    /// Apply everything that has arrived, once.
    fn pump(&mut self) {
        let lifecycle: Vec<LifecycleEvent> = self.life.lock().unwrap().drain(..).collect();
        for event in lifecycle {
            let outcome = match event {
                LifecycleEvent::Connection(c) => self.state.set_connection(c),
                LifecycleEvent::Catalog(boot) => {
                    let mut out = self.state.apply_catalog(boot);
                    out.merge(self.restore.apply(&mut self.state));
                    out.merge(self.state.select_initial());
                    out
                }
                LifecycleEvent::ModelSelected(m) => self.state.set_engine_model(m),
                LifecycleEvent::Notice(message) => {
                    self.notices.push(message.clone());
                    self.state.set_notice(message)
                }
            };
            self.exec(outcome);
        }
        while let Ok(event) = self.events.try_recv() {
            let outcome = self.state.apply_event(event);
            self.exec(outcome);
        }
        while let Ok((conversation, request, result)) = self.acks.try_recv() {
            let outcome = self.state.intent_persisted(conversation, &request, result);
            self.exec(outcome);
        }
    }

    fn until(&mut self, what: &str, done: impl Fn(&AppState) -> bool) {
        let deadline = Instant::now() + WAIT;
        loop {
            self.pump();
            if done(&self.state) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for: {what}\n  connection: {:?}\n  notices: {:?}\n  run: {:?}",
                self.state.connection,
                self.notices,
                self.state.current().map(|c| c.run.clone()),
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Open `project` as a project, ask for a conversation in it, and wait until it is open.
    fn open_new_conversation(&mut self, project: &Path) {
        self.until("connected", |s| s.connection.is_ready());
        // The engine resolves a session's directory through symlinks, so open the real path.
        let real = std::fs::canonicalize(project).unwrap();
        self.dispatch(Command::AddProject(real.display().to_string()));
        self.dispatch(Command::NewConversation);
        self.until("the new conversation is open", |s| {
            s.current().is_some_and(|c| c.opened)
        });
    }

    fn send(&mut self, text: &str) {
        self.dispatch(Command::EditDraft(text.into()));
        self.dispatch(Command::Submit);
    }

    fn user_messages(&self) -> Vec<String> {
        self.state
            .current()
            .unwrap()
            .items
            .iter()
            .filter_map(|i| match &i.kind {
                ItemKind::User { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// An orderly stop, as the application's quit does.
    fn stop(self) {
        self.backend.shutdown();
        self.storage.shutdown();
    }
}

fn direct_config(server_dir: &Path, server_id: &str) -> PiConfig {
    let mut config = PiConfig::new(server_dir.to_path_buf());
    config.server_id = Some(server_id.to_owned());
    config.retry_delay = Duration::from_millis(200);
    config
}

// ------------------------------------------------------------------------- the fault proxy

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fault {
    None,
    /// Let the prompt reach the engine, then drop its reply and sever the connection.
    DropNextPromptReply,
    /// Sever the connection instead of delivering the prompt to the engine.
    DropNextPromptRequest,
}

struct ProxyState {
    fault: Fault,
    prompt_ids: HashSet<String>,
    fired: usize,
}

/// A Unix-socket proxy in front of the engine that understands the wire well enough to find a
/// prompt call and its reply. Everything else is forwarded byte for byte.
struct FaultProxy {
    dir: PathBuf,
    state: Arc<Mutex<ProxyState>>,
    stop: Arc<AtomicBool>,
}

fn is_prompt(call: &Value) -> bool {
    parse_service_call(call)
        .is_ok_and(|c| c.service_id == "pi.agent-controller" && c.member == "prompt")
}

impl FaultProxy {
    fn start(upstream: PathBuf, dir: PathBuf, server_id: &str) -> FaultProxy {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let socket = dir.join(format!("{server_id}.sock"));
        let listener = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let state = Arc::new(Mutex::new(ProxyState {
            fault: Fault::None,
            prompt_ids: HashSet::new(),
            fired: 0,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let (s, st) = (state.clone(), stop.clone());
        thread::spawn(move || {
            for client in listener.incoming().flatten() {
                if st.load(Ordering::SeqCst) {
                    return;
                }
                let Ok(engine) = UnixStream::connect(&upstream) else {
                    continue;
                };
                Self::pipe(client, engine, s.clone());
            }
        });
        FaultProxy { dir, state, stop }
    }

    fn arm(&self, fault: Fault) {
        self.state.lock().unwrap().fault = fault;
    }

    fn fired(&self) -> usize {
        self.state.lock().unwrap().fired
    }

    fn pipe(client: UnixStream, engine: UnixStream, state: Arc<Mutex<ProxyState>>) {
        let sever = {
            let (c, e) = (client.try_clone().unwrap(), engine.try_clone().unwrap());
            Arc::new(move || {
                let _ = c.shutdown(std::net::Shutdown::Both);
                let _ = e.shutdown(std::net::Shutdown::Both);
            })
        };
        // client -> engine
        {
            let (mut from, mut to, state, sever) = (
                client.try_clone().unwrap(),
                engine.try_clone().unwrap(),
                state.clone(),
                sever.clone(),
            );
            thread::spawn(move || {
                let mut frames = FrameDecoder::new(DEFAULT_MAX_FRAME_LENGTH);
                let mut buf = [0u8; 16 * 1024];
                loop {
                    let n = match from.read(&mut buf) {
                        Ok(0) | Err(_) => return sever(),
                        Ok(n) => n,
                    };
                    let Ok(payloads) = frames.push(&buf[..n]) else {
                        return sever();
                    };
                    for payload in payloads {
                        if let Ok(value) = cbor::decode(&payload, &Limits::default())
                            && let Ok(ClientMessage::Request { id, call, .. }) =
                                parse_client_message(&value)
                            && is_prompt(&call)
                        {
                            let mut s = state.lock().unwrap();
                            if s.fault == Fault::DropNextPromptRequest {
                                s.fault = Fault::None;
                                s.fired += 1;
                                drop(s);
                                return sever();
                            }
                            s.prompt_ids.insert(id);
                        }
                        let frame = encode_frame(&payload).unwrap();
                        if to.write_all(&frame).is_err() {
                            return sever();
                        }
                    }
                }
            });
        }
        // engine -> client
        thread::spawn(move || {
            let (mut from, mut to) = (engine, client);
            let mut frames = FrameDecoder::new(DEFAULT_MAX_FRAME_LENGTH);
            let mut buf = [0u8; 16 * 1024];
            loop {
                let n = match from.read(&mut buf) {
                    Ok(0) | Err(_) => return sever(),
                    Ok(n) => n,
                };
                let Ok(payloads) = frames.push(&buf[..n]) else {
                    return sever();
                };
                for payload in payloads {
                    if let Ok(value) = cbor::decode(&payload, &Limits::default())
                        && let Ok(ServerMessage::Response { id, .. }) = parse_server_message(&value)
                    {
                        let mut s = state.lock().unwrap();
                        if s.fault == Fault::DropNextPromptReply && s.prompt_ids.contains(&id) {
                            s.fault = Fault::None;
                            s.fired += 1;
                            drop(s);
                            return sever();
                        }
                    }
                    let frame = encode_frame(&payload).unwrap();
                    if to.write_all(&frame).is_err() {
                        return sever();
                    }
                }
            }
        });
    }
}

impl Drop for FaultProxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = UnixStream::connect(self.dir.join("wake"));
    }
}

// --------------------------------------------------------------------------------- the tests

fn nothing(_: &Value, _: usize) -> Reply {
    Reply::Text("ok".into())
}

fn handshake(dir: &Path, id: &str) -> bool {
    use pi_client::client::ClientOptions;
    pi_client::unix::connect(&dir.join(format!("{id}.sock")), ClientOptions::new(id)).is_ok()
}

#[test]
#[ignore = "needs a real Pi engine: set PIPKIN_PI_REPO"]
fn the_engine_starts_serves_and_stops_without_leaving_processes() {
    let mut fx = Fixture::start(nothing);
    assert!(handshake(&fx.server_dir, &fx.server_id), "{}", fx.log());
    assert!(!profile_pids(&fx.server_dir, &fx.server_id).is_empty());
    let (dir, id) = (fx.server_dir.clone(), fx.server_id.clone());
    fx.host.stop();
    // Pi's coordinator and workers are detached; every one of them must be gone.
    assert_eq!(
        profile_pids(&dir, &id),
        Vec::<u32>::new(),
        "orphaned engine processes"
    );
    assert!(!handshake(&dir, &id), "nothing should answer after stop");
    fx.host.stop(); // stopping twice is harmless
}

/// M2's exit criterion, first half: in a temporary project a real engine with a scripted
/// provider executes file-changing tools; the app shows the tool output and the real diff; the
/// same history comes back when it is reopened.
#[test]
#[ignore = "needs a real Pi engine: set PIPKIN_PI_REPO"]
fn a_real_run_edits_files_shows_output_and_diff_and_reopens_the_same_history() {
    let fx = Fixture::start(two_edits);
    init_project(&fx.project);
    let db = fx.root.join("app.sqlite3");
    let mut h = Harness::start(direct_config(&fx.server_dir, &fx.server_id), &db);
    h.open_new_conversation(&fx.project);

    // The conversation was created in the project's directory, through the real engine.
    let real = std::fs::canonicalize(&fx.project)
        .unwrap()
        .display()
        .to_string();
    let project = h.state.current_project().expect("a project").clone();
    assert_eq!(project.path, real);
    assert_eq!(project.name, "project");
    // The engine, not a local preference, says which model is selected.
    assert_eq!(h.state.prefs.model.as_deref(), Some("stub/scripted"));
    assert!(h.state.models.iter().any(|m| m.id == "stub/scripted"));

    h.send("please update the notes and add a greeting");
    h.until("the run completes", |s| {
        s.current().is_some_and(|c| c.run == RunState::Idle)
            && s.current().unwrap().items.iter().any(
                |i| matches!(&i.kind, ItemKind::Assistant { text, .. } if text.contains("Done:")),
            )
            && s.current().unwrap().changes.len() == 2
    });

    // The files really changed, in the project.
    assert_eq!(
        std::fs::read_to_string(fx.project.join("notes.txt")).unwrap(),
        "one\ntwo\n"
    );
    assert_eq!(
        std::fs::read_to_string(fx.project.join("hello.txt")).unwrap(),
        "hello from the stub\n"
    );

    // The transcript shows the real exchange: the prompt, both tool calls with their output, the answer.
    let conv = h.state.current().unwrap();
    let tools: Vec<_> = conv
        .items
        .iter()
        .filter_map(|i| {
            if let ItemKind::Tool(t) = &i.kind {
                Some(t)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(tools.len(), 2, "{:?}", conv.items);
    assert!(
        tools
            .iter()
            .all(|t| t.name == "write" && t.status == ToolStatus::Ok),
        "{tools:?}"
    );
    assert!(tools[0].input.contains("notes.txt") && tools[1].input.contains("hello.txt"));
    assert!(
        tools[0].output.contains("Successfully wrote"),
        "{}",
        tools[0].output
    );
    assert_eq!(
        h.user_messages(),
        ["please update the notes and add a greeting"]
    );
    assert!(matches!(
        &conv.items.last().unwrap().kind,
        ItemKind::Assistant { text, streaming: false } if text == "Done: updated notes.txt and created hello.txt."
    ));
    assert!(conv.items.iter().any(|i| matches!(
        &i.kind,
        ItemKind::User {
            delivery: Delivery::Sent,
            ..
        }
    )));

    // The workspace diff is real: a modified tracked file and a new untracked one.
    let names: Vec<&str> = conv.changes.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(names, ["notes.txt", "hello.txt"]);
    assert_eq!((conv.changes[0].added, conv.changes[0].removed), (1, 0));
    assert_eq!((conv.changes[1].added, conv.changes[1].removed), (1, 0));

    // The model was called three times: two tool turns and the final answer.
    let requests = fx.provider.requests();
    assert_eq!(requests.len(), 3);
    assert!(is_tool_result_turn(&requests[1]) && is_tool_result_turn(&requests[2]));

    // Nothing is left unresolved in the journal: the request ran to completion.
    h.storage.shutdown();
    let loaded = Storage::open(&db, NAMESPACE).unwrap().load_all().unwrap();
    assert!(
        loaded.open_requests.is_empty(),
        "{:?}",
        loaded.open_requests
    );
    let items_before = h.state.current().unwrap().items.clone();
    let conversation = h.state.current().unwrap().id;
    h.backend.shutdown();

    // Reopen from scratch: a new application instance against the same engine shows the same history.
    let mut again = Harness::start(direct_config(&fx.server_dir, &fx.server_id), &db);
    again.until("the conversation reopens", |s| {
        s.selected == Some(conversation) && s.current().is_some_and(|c| c.opened)
    });
    let reopened = again.state.current().unwrap();
    assert_eq!(reopened.items.len(), items_before.len());
    assert_eq!(
        again.user_messages(),
        ["please update the notes and add a greeting"]
    );
    assert_eq!(
        reopened.items.iter().map(|i| i.id).collect::<Vec<_>>(),
        items_before.iter().map(|i| i.id).collect::<Vec<_>>(),
        "the same entries keep the same ids across a reopen"
    );
    again.until("the workspace changes load", |s| {
        s.current().unwrap().changes.len() == 2
    });
    again.stop();
}

/// Everything shared by the two "drop the acknowledgment" scenarios.
struct Fault2 {
    fx: Fixture,
    proxy: FaultProxy,
    db: PathBuf,
}

fn fault_setup() -> Fault2 {
    let fx = Fixture::start(two_edits);
    init_project(&fx.project);
    let proxy = FaultProxy::start(
        fx.server_dir.join(format!("{}.sock", fx.server_id)),
        fx.root.join("proxy"),
        &fx.server_id,
    );
    let db = fx.root.join("app.sqlite3");
    Fault2 { fx, proxy, db }
}

/// M2's exit criterion, second half, case 1: the engine accepted the prompt and ran it, but the
/// acknowledgment never reached the app, and the app was killed. After relaunch the outcome is
/// unknown, never resent; asking the engine finds the original, so the prompt ran exactly once.
#[test]
#[ignore = "needs a real Pi engine: set PIPKIN_PI_REPO"]
fn a_dropped_acknowledgment_then_a_relaunch_never_runs_the_prompt_twice() {
    let f = fault_setup();
    let mut h = Harness::start(direct_config(&f.proxy.dir, &f.fx.server_id), &f.db);
    h.open_new_conversation(&f.fx.project);
    let conversation = h.state.current().unwrap().id;

    f.proxy.arm(Fault::DropNextPromptReply);
    h.send("update the notes");
    h.until("the outcome is unknown", |s| {
        matches!(
            s.current().map(|c| &c.run),
            Some(RunState::OutcomeUnknown { .. })
        )
    });
    assert_eq!(f.proxy.fired(), 1);
    // The core refuses to resend on its own: submit is off, retry is off, only a status check.
    let a = h.state.availability();
    assert!(!a.submit && !a.retry && a.check_status);

    // The engine, unaware of any of this, finishes the run it accepted.
    let deadline = Instant::now() + WAIT;
    while f.fx.provider.requests().len() < 3 || !f.fx.project.join("hello.txt").exists() {
        assert!(
            Instant::now() < deadline,
            "the engine never finished the run it accepted"
        );
        thread::sleep(Duration::from_millis(50));
    }

    // Kill the application. The journal already holds the intent; nothing else was saved.
    h.stop();

    // Relaunch against the engine directly. The unresolved request is restored as unknown.
    let mut h2 = Harness::start(direct_config(&f.fx.server_dir, &f.fx.server_id), &f.db);
    h2.until("the conversation reopens as unknown", |s| {
        s.selected == Some(conversation)
            && s.current()
                .is_some_and(|c| c.opened && matches!(c.run, RunState::OutcomeUnknown { .. }))
    });
    assert_eq!(
        f.fx.provider.requests().len(),
        3,
        "relaunching resent nothing"
    );
    assert!(
        !h2.state.availability().submit,
        "no automatic resubmission is offered"
    );

    // The user asks. The engine finds the original submission by its key.
    h2.dispatch(Command::CheckStatus);
    h2.until("the status resolves and the run settles", |s| {
        s.current()
            .is_some_and(|c| c.run == RunState::Idle && c.last_submission.is_some())
    });
    assert_eq!(h2.user_messages(), ["update the notes"], "one prompt, once");
    assert!(
        h2.state.current().unwrap().items.iter().any(|i| matches!(
            &i.kind,
            ItemKind::User {
                delivery: Delivery::Sent,
                ..
            }
        )),
        "the message is shown as delivered"
    );
    assert_eq!(
        f.fx.provider.requests().len(),
        3,
        "the run happened exactly once"
    );
    assert_eq!(
        std::fs::read_to_string(f.fx.project.join("notes.txt")).unwrap(),
        "one\ntwo\n"
    );

    // The journal entry is closed, so a third launch has nothing left to resolve.
    h2.stop();
    let loaded = Storage::open(&f.db, NAMESPACE).unwrap().load_all().unwrap();
    assert!(
        loaded.open_requests.is_empty(),
        "{:?}",
        loaded.open_requests
    );
}

/// Case 2: the prompt never reached the engine (the connection died first), and the app was
/// killed. After relaunch the engine proves it has no such submission, and only then does the
/// user's explicit retry send it, once.
#[test]
#[ignore = "needs a real Pi engine: set PIPKIN_PI_REPO"]
fn a_lost_prompt_is_proven_unsent_and_only_an_explicit_retry_sends_it() {
    let f = fault_setup();
    let mut h = Harness::start(direct_config(&f.proxy.dir, &f.fx.server_id), &f.db);
    h.open_new_conversation(&f.fx.project);
    let conversation = h.state.current().unwrap().id;

    f.proxy.arm(Fault::DropNextPromptRequest);
    h.send("add a greeting");
    h.until("the outcome is unknown", |s| {
        matches!(
            s.current().map(|c| &c.run),
            Some(RunState::OutcomeUnknown { .. })
        )
    });
    assert_eq!(f.proxy.fired(), 1);
    thread::sleep(Duration::from_millis(500));
    assert_eq!(
        f.fx.provider.requests().len(),
        0,
        "the engine never saw the prompt"
    );
    h.stop();

    let mut h2 = Harness::start(direct_config(&f.fx.server_dir, &f.fx.server_id), &f.db);
    h2.until("reopened as unknown", |s| {
        s.selected == Some(conversation)
            && s.current()
                .is_some_and(|c| c.opened && matches!(c.run, RunState::OutcomeUnknown { .. }))
    });
    h2.dispatch(Command::CheckStatus);
    // The engine has no submission under that key: provably never accepted.
    h2.until("proven not accepted", |s| {
        s.current()
            .is_some_and(|c| matches!(c.run, RunState::Failed { .. }))
    });
    assert!(h2.state.availability().retry, "now a retry is offered");
    assert_eq!(f.fx.provider.requests().len(), 0, "still nothing was run");

    // Only the user's explicit retry sends it, and it runs once.
    h2.dispatch(Command::Retry);
    h2.until("the retried run completes", |s| {
        s.current()
            .is_some_and(|c| c.run == RunState::Idle && c.changes.len() == 2)
    });
    assert_eq!(h2.user_messages(), ["add a greeting"]);
    assert_eq!(f.fx.provider.requests().len(), 3, "exactly one run");
    assert_eq!(
        std::fs::read_to_string(f.fx.project.join("hello.txt")).unwrap(),
        "hello from the stub\n"
    );
    h2.stop();
}

/// Engine lifecycle: the app launches and owns the engine, restarts it promptly when it dies,
/// and stops every process it started on quit.
#[test]
#[ignore = "needs a real Pi engine: set PIPKIN_PI_REPO"]
fn the_app_owns_the_engine_restarts_it_after_a_crash_and_stops_it_on_quit() {
    let p: Prepared = prepare(nothing);
    let (dir, id) = (p.server_dir.clone(), p.server_id.clone());
    let db = p.root.join("app.sqlite3");
    let mut config = PiConfig::managed(p.config.clone());
    config.retry_delay = Duration::from_millis(200);
    let mut h = Harness::start(config, &db);

    h.until("the app launched the engine and connected", |s| {
        s.connection.is_ready()
    });
    assert!(!profile_pids(&dir, &id).is_empty(), "the engine is running");

    // The engine crashes hard (every process of it, no cleanup); the app notices, restarts it,
    // and reconnects, without waiting out the launcher lock the crash left behind.
    for pid in profile_pids(&dir, &id) {
        // SAFETY: SIGKILL to processes matched by this test's own profile identity.
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    }
    let crashed = Instant::now();
    h.until("the app sees the loss", |s| !s.connection.is_ready());
    h.until("the app restarted the engine and reconnected", |s| {
        s.connection.is_ready()
    });
    assert!(
        crashed.elapsed() < Duration::from_secs(20),
        "a restart after a crash took {:?}; the stale launcher lock was not cleared",
        crashed.elapsed()
    );

    // Quit: every process the app started is stopped, none orphaned.
    h.stop();
    assert_eq!(
        profile_pids(&dir, &id),
        Vec::<u32>::new(),
        "orphaned engine processes after quit"
    );
    drop(p.provider);
}

/// A crash loop is capped: an engine that cannot start is retried a few times, then the app stops
/// trying and says why, with the engine's own output, instead of spinning.
#[test]
#[ignore = "needs a real Pi engine: set PIPKIN_PI_REPO"]
fn an_engine_that_cannot_start_is_retried_a_few_times_then_reported_with_its_output() {
    let mut p: Prepared = prepare(nothing);
    // Pi rejects a malformed server id as it starts, so the engine exits during startup, every
    // time, within moments.
    p.config.server_id = "not-a-valid-server-id".into();
    let (dir, id) = (p.server_dir.clone(), p.config.server_id.clone());
    let db = p.root.join("app.sqlite3");
    let mut config = PiConfig::managed(p.config.clone());
    config.retry_delay = Duration::from_millis(100);
    let mut h = Harness::start(config, &db);

    h.until(
        "the crash loop is reported",
        |s| matches!(&s.connection, Connection::Failed(m) if m.contains("keeps crashing")),
    );
    let Connection::Failed(message) = h.state.connection.clone() else {
        unreachable!()
    };
    assert!(message.contains("3 starts"), "{message}");
    assert!(
        message.to_lowercase().contains("invalid"),
        "the engine's own output is in the diagnosis: {message}"
    );
    // Once capped, no further engine is started.
    thread::sleep(Duration::from_secs(2));
    assert_eq!(
        profile_pids(&dir, &id),
        Vec::<u32>::new(),
        "no engine left running"
    );
    h.stop();
}

/// A user who has an engine running already must be left alone by the app: connecting to it
/// never starts, restarts or stops anything.
#[test]
#[ignore = "needs a real Pi engine: set PIPKIN_PI_REPO"]
fn an_engine_the_app_did_not_start_is_never_stopped_by_it() {
    let fx = Fixture::start(nothing);
    let db = fx.root.join("app.sqlite3");
    let mut h = Harness::start(direct_config(&fx.server_dir, &fx.server_id), &db);
    h.until("connected", |s| s.connection.is_ready());
    let before = profile_pids(&fx.server_dir, &fx.server_id);
    assert!(!before.is_empty());
    h.stop();
    assert!(
        handshake(&fx.server_dir, &fx.server_id),
        "the engine outlives the app"
    );
    let after = profile_pids(&fx.server_dir, &fx.server_id);
    assert_eq!(
        before, after,
        "no engine process was started or stopped by the app"
    );
    drop(EngineHost::new(fx.host.config().clone())); // a host that never started anything stops nothing
    assert!(handshake(&fx.server_dir, &fx.server_id));
}
