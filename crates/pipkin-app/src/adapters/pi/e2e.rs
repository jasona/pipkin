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
    Effect, ItemKind, LifecycleEvent, Mode, Outcome, QueueMode, RequestId, RunState, ToolStatus,
    describe_attachment,
};
use serde_json::{Value, json};

use super::engine::{EngineHost, profile_pids};
use super::testsupport::{Fixture, Gate, Prepared, Reply, is_tool_result_turn, prepare};
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
                    conversation,
                    text,
                    attachments,
                    rev,
                } => {
                    // Really written, so a restart restores it; waited for, so the test is
                    // deterministic.
                    let (tx, rx) = mpsc::channel();
                    self.storage
                        .save_draft(conversation, text, attachments, rev, move |r| {
                            let _ = tx.send(r);
                        });
                    let result = rx.recv_timeout(Duration::from_secs(10)).unwrap();
                    self.state.draft_saved(conversation, rev, result);
                }
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
                LifecycleEvent::StorageIssue(message) => self.state.set_storage_issue(message),
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

    /// Keep the application running until the engine has made `n` model requests.
    fn until_asked(&mut self, provider: &super::testsupport::StubProvider, n: usize) {
        self.until("the engine asked the model", |_| {
            provider.requests().len() >= n
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
    /// Let the next steer or follow-up reach the engine, then drop its reply and sever.
    DropNextQueueReply,
    /// Sever the connection instead of delivering the next steer or follow-up.
    DropNextQueueRequest,
}

type Sever = Arc<dyn Fn() + Send + Sync>;

struct ProxyState {
    fault: Fault,
    prompt_ids: HashSet<String>,
    queue_ids: HashSet<String>,
    fired: usize,
    /// Every live connection, so a test can cut them all at once.
    severs: Vec<Sever>,
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

fn is_queue(call: &Value) -> bool {
    parse_service_call(call).is_ok_and(|c| {
        c.service_id == "pi.agent-controller" && matches!(c.member.as_str(), "steer" | "followUp")
    })
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
            queue_ids: HashSet::new(),
            fired: 0,
            severs: Vec::new(),
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

    /// Cut every connection through the proxy, as a network failure would.
    fn sever_all(&self) {
        let severs = std::mem::take(&mut self.state.lock().unwrap().severs);
        for sever in severs {
            sever();
        }
    }

    fn pipe(client: UnixStream, engine: UnixStream, state: Arc<Mutex<ProxyState>>) {
        let sever: Sever = {
            let (c, e) = (client.try_clone().unwrap(), engine.try_clone().unwrap());
            Arc::new(move || {
                let _ = c.shutdown(std::net::Shutdown::Both);
                let _ = e.shutdown(std::net::Shutdown::Both);
            })
        };
        state.lock().unwrap().severs.push(sever.clone());
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
                        {
                            let mut s = state.lock().unwrap();
                            if is_prompt(&call) {
                                if s.fault == Fault::DropNextPromptRequest {
                                    s.fault = Fault::None;
                                    s.fired += 1;
                                    drop(s);
                                    return sever();
                                }
                                s.prompt_ids.insert(id);
                            } else if is_queue(&call) {
                                if s.fault == Fault::DropNextQueueRequest {
                                    s.fault = Fault::None;
                                    s.fired += 1;
                                    drop(s);
                                    return sever();
                                }
                                s.queue_ids.insert(id);
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
                        let drop_reply = (s.fault == Fault::DropNextPromptReply
                            && s.prompt_ids.contains(&id))
                            || (s.fault == Fault::DropNextQueueReply && s.queue_ids.contains(&id));
                        if drop_reply {
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

// ------------------------------------------------------------------------- M3: run control

fn assistant_texts(s: &AppState) -> Vec<String> {
    s.current()
        .map(|c| {
            c.items
                .iter()
                .filter_map(|i| match &i.kind {
                    ItemKind::Assistant { text, .. } => Some(text.clone()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn queued(s: &AppState) -> Vec<(QueueMode, String)> {
    s.current()
        .map(|c| c.queue.iter().map(|q| (q.mode, q.text.clone())).collect())
        .unwrap_or_default()
}

fn is_running(s: &AppState) -> bool {
    matches!(s.current().map(|c| &c.run), Some(RunState::Running { .. }))
}

fn is_idle(s: &AppState) -> bool {
    s.current().is_some_and(|c| c.run == RunState::Idle)
}

/// The text of every user message in a model request, oldest first.
fn user_texts(request: &Value) -> Vec<String> {
    request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "user")
        .map(|m| match &m["content"] {
            Value::String(s) => s.clone(),
            Value::Array(parts) => parts
                .iter()
                .filter_map(|p| p["text"].as_str())
                .collect::<Vec<_>>()
                .join(""),
            other => other.to_string(),
        })
        .collect()
}

/// The first model turn is held until the gate opens; later turns answer at once.
fn held_first(
    gate: &Gate,
    first: Reply,
    rest: Vec<Reply>,
) -> impl Fn(&Value, usize) -> Reply + use<> {
    let gate = gate.clone();
    move |_, n| match n {
        0 => Reply::Gated(gate.clone(), Box::new(first.clone())),
        n => rest
            .get(n - 1)
            .cloned()
            .unwrap_or_else(|| Reply::Text(format!("extra turn {n}"))),
    }
}

/// Stopping a run that is in flight: the stop is a request, the run ends when the engine says
/// so, and the conversation takes the next prompt afterwards.
#[test]
#[ignore = "needs a real Pi engine: set PIPKIN_PI_REPO"]
fn stopping_a_run_waits_for_the_engine_and_the_conversation_takes_the_next_prompt() {
    let gate = Gate::new();
    let fx = Fixture::start(held_first(
        &gate,
        Reply::Text("never delivered".into()),
        vec![Reply::Text("second answer".into())],
    ));
    init_project(&fx.project);
    let db = fx.root.join("app.sqlite3");
    let mut h = Harness::start(direct_config(&fx.server_dir, &fx.server_id), &db);
    h.open_new_conversation(&fx.project);

    h.send("a long task");
    h.until_asked(&fx.provider, 1);
    h.until("the run is in flight", is_running);
    assert!(h.state.availability().cancel);

    h.dispatch(Command::Cancel);
    assert!(matches!(
        h.state.current().unwrap().run,
        RunState::Stopping { .. }
    ));
    assert!(
        !h.state.availability().submit,
        "no new prompt until the engine confirms"
    );
    h.until("the engine confirms the stop", is_idle);
    assert!(
        !assistant_texts(&h.state)
            .iter()
            .any(|t| t.contains("never"))
    );

    // The conversation is usable again, and the stopped prompt is closed in the journal.
    h.send("try again");
    h.until("the next prompt is answered", |s| {
        is_idle(s) && assistant_texts(s).contains(&"second answer".to_owned())
    });
    assert_eq!(h.user_messages(), ["a long task", "try again"]);
    h.storage.shutdown();
    let loaded = Storage::open(&db, NAMESPACE).unwrap().load_all().unwrap();
    assert!(
        loaded.open_requests.is_empty(),
        "{:?}",
        loaded.open_requests
    );
    gate.open();
    h.backend.shutdown();
}

/// A follow-up waits behind the run in the engine's own queue, then runs on its own.
#[test]
#[ignore = "needs a real Pi engine: set PIPKIN_PI_REPO"]
fn a_follow_up_waits_in_the_engines_queue_then_runs_by_itself() {
    let gate = Gate::new();
    let fx = Fixture::start(held_first(
        &gate,
        Reply::Text("first done".into()),
        vec![Reply::Text("second done".into())],
    ));
    init_project(&fx.project);
    let db = fx.root.join("app.sqlite3");
    let mut h = Harness::start(direct_config(&fx.server_dir, &fx.server_id), &db);
    h.open_new_conversation(&fx.project);

    h.send("first");
    h.until_asked(&fx.provider, 1);
    h.until("running", is_running);
    h.dispatch(Command::EditDraft("second".into()));
    assert!(h.state.availability().queue);
    h.dispatch(Command::QueueFollowUp);
    h.until("the draft moved on, to the queue", |s| {
        s.current().unwrap().draft.text.is_empty()
    });
    h.until("the engine's queue shows it", |s| {
        queued(s) == [(QueueMode::FollowUp, "second".to_owned())]
            && s.current().unwrap().pending_queue.is_empty()
    });
    assert!(is_running(&h.state));

    gate.open();
    h.until("both ran, in order", |s| {
        is_idle(s) && assistant_texts(s) == ["first done".to_owned(), "second done".to_owned()]
    });
    assert!(h.state.current().unwrap().queue.is_empty());
    assert_eq!(h.user_messages(), ["first", "second"]);
    let requests = fx.provider.requests();
    assert_eq!(requests.len(), 2, "one model turn each");
    assert_eq!(user_texts(&requests[1]), ["first", "second"]);

    // Both journal entries are closed: nothing to reconcile on the next launch.
    h.storage.shutdown();
    let loaded = Storage::open(&db, NAMESPACE).unwrap().load_all().unwrap();
    assert!(
        loaded.open_requests.is_empty(),
        "{:?}",
        loaded.open_requests
    );
    h.backend.shutdown();
}

/// Removing a queued input withdraws it from the engine: it never reaches the model.
#[test]
#[ignore = "needs a real Pi engine: set PIPKIN_PI_REPO"]
fn a_queued_input_removed_before_it_runs_never_reaches_the_model() {
    let gate = Gate::new();
    let fx = Fixture::start(held_first(
        &gate,
        Reply::Text("first done".into()),
        vec![Reply::Text("kept done".into())],
    ));
    init_project(&fx.project);
    let db = fx.root.join("app.sqlite3");
    let mut h = Harness::start(direct_config(&fx.server_dir, &fx.server_id), &db);
    h.open_new_conversation(&fx.project);

    h.send("first");
    h.until_asked(&fx.provider, 1);
    h.until("running", is_running);
    for text in ["queued-drop-xq7", "queued-keep-xq7"] {
        h.dispatch(Command::EditDraft(text.into()));
        h.dispatch(Command::QueueFollowUp);
        let want = text.to_owned();
        h.until("queued", |s| {
            queued(s).iter().any(|(_, t)| *t == want)
                && s.current().unwrap().pending_queue.is_empty()
        });
    }
    assert_eq!(queued(&h.state).len(), 2);

    let second = h.state.current().unwrap().queue[0].id;
    h.dispatch(Command::RemoveQueued(second));
    h.until("the engine dropped it", |s| {
        queued(s) == [(QueueMode::FollowUp, "queued-keep-xq7".to_owned())]
    });

    gate.open();
    h.until("first and the kept input ran", |s| {
        is_idle(s) && assistant_texts(s) == ["first done".to_owned(), "kept done".to_owned()]
    });
    let requests = fx.provider.requests();
    assert_eq!(requests.len(), 2);
    for request in &requests {
        assert!(
            !request.to_string().contains("queued-drop-xq7"),
            "a removed input must never be sent to the model: {request}"
        );
    }
    assert_eq!(user_texts(&requests[1]), ["first", "queued-keep-xq7"]);
    h.backend.shutdown();
}

/// Steering joins the run in flight: the model's next turn in the same run sees it.
#[test]
#[ignore = "needs a real Pi engine: set PIPKIN_PI_REPO"]
fn a_steer_reaches_the_next_model_turn_of_the_active_run() {
    let gate = Gate::new();
    let fx = Fixture::start(held_first(
        &gate,
        Reply::Tool {
            lead: Some("Working. ".into()),
            name: "write".into(),
            args: json!({ "path": "notes.txt", "content": "one\ntwo\n" }),
        },
        vec![Reply::Text("done, with tabs".into())],
    ));
    init_project(&fx.project);
    let db = fx.root.join("app.sqlite3");
    let mut h = Harness::start(direct_config(&fx.server_dir, &fx.server_id), &db);
    h.open_new_conversation(&fx.project);

    h.send("edit the notes");
    h.until_asked(&fx.provider, 1);
    h.until("running", is_running);
    h.dispatch(Command::EditDraft("use tabs".into()));
    assert!(h.state.availability().steer);
    h.dispatch(Command::Steer);
    h.until("the engine holds the steer", |s| {
        queued(s) == [(QueueMode::Steer, "use tabs".to_owned())]
    });

    gate.open();
    h.until("the run finished", |s| {
        is_idle(s) && assistant_texts(s).iter().any(|t| t.contains("with tabs"))
    });
    let requests = fx.provider.requests();
    assert_eq!(requests.len(), 2, "one run, two model turns: {requests:?}");
    assert_eq!(
        user_texts(&requests[1]),
        ["edit the notes", "use tabs"],
        "the second turn of the same run saw the steer"
    );
    assert_eq!(h.user_messages(), ["edit the notes", "use tabs"]);
    assert!(h.state.current().unwrap().queue.is_empty());
    h.backend.shutdown();
}

/// Closing the app mid-run and opening it again picks the run up where it is: the unanswered
/// prompt is looked up (never resent), the engine's queue is shown, and the run finishes.
#[test]
#[ignore = "needs a real Pi engine: set PIPKIN_PI_REPO"]
fn a_run_and_its_queue_are_picked_up_again_after_the_app_restarts() {
    let gate = Gate::new();
    let fx = Fixture::start(held_first(
        &gate,
        Reply::Text("first done".into()),
        vec![Reply::Text("second done".into())],
    ));
    init_project(&fx.project);
    let db = fx.root.join("app.sqlite3");
    let mut h = Harness::start(direct_config(&fx.server_dir, &fx.server_id), &db);
    h.open_new_conversation(&fx.project);
    let conversation = h.state.current().unwrap().id;

    h.send("first");
    h.until_asked(&fx.provider, 1);
    h.until("running", is_running);
    h.dispatch(Command::EditDraft("second".into()));
    h.dispatch(Command::QueueFollowUp);
    h.until("queued", |s| {
        queued(s) == [(QueueMode::FollowUp, "second".to_owned())]
    });
    h.stop(); // the window closes; the engine and its run carry on

    let mut h2 = Harness::start(direct_config(&fx.server_dir, &fx.server_id), &db);
    // The prompt's journal entry was still open, so it comes back in doubt, is looked up
    // automatically, and is found running.
    h2.until("the run is found running again", |s| {
        s.selected == Some(conversation) && is_running(s) && !queued(s).is_empty()
    });
    assert_eq!(
        queued(&h2.state),
        [(QueueMode::FollowUp, "second".to_owned())],
        "the queue is the engine's, shown after a restart"
    );
    assert_eq!(fx.provider.requests().len(), 1, "nothing was resent");
    assert_eq!(h2.user_messages(), ["first"], "one prompt, once");

    gate.open();
    h2.until("everything finishes", |s| {
        is_idle(s) && assistant_texts(s) == ["first done".to_owned(), "second done".to_owned()]
    });
    assert_eq!(fx.provider.requests().len(), 2);
    assert_eq!(h2.user_messages(), ["first", "second"]);
    h2.stop();
    let loaded = Storage::open(&db, NAMESPACE).unwrap().load_all().unwrap();
    assert!(
        loaded.open_requests.is_empty(),
        "{:?}",
        loaded.open_requests
    );
}

/// A run found busy that this window did not start (its prompt was already settled in the
/// journal) is still shown and can be stopped.
#[test]
#[ignore = "needs a real Pi engine: set PIPKIN_PI_REPO"]
fn a_run_started_elsewhere_is_shown_and_can_be_stopped() {
    let gate = Gate::new();
    let fx = Fixture::start(held_first(&gate, Reply::Text("never".into()), vec![]));
    init_project(&fx.project);
    let db = fx.root.join("app.sqlite3");
    let mut h = Harness::start(direct_config(&fx.server_dir, &fx.server_id), &db);
    h.open_new_conversation(&fx.project);
    let conversation = h.state.current().unwrap().id;
    h.send("started here");
    h.until_asked(&fx.provider, 1);
    h.until("running", is_running);
    h.stop();

    // Forget the journal entry, as if another client had started the run.
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute("UPDATE request_journal SET state = 'completed'", [])
            .unwrap();
    }
    let mut h2 = Harness::start(direct_config(&fx.server_dir, &fx.server_id), &db);
    h2.until("the busy run is shown", |s| {
        s.selected == Some(conversation) && is_running(s)
    });
    assert!(h2.state.availability().cancel);
    h2.dispatch(Command::Cancel);
    h2.until("it stops, on the engine's word", is_idle);
    gate.open();
    h2.stop();
}

/// Moving between conversations keeps each one's run: an engine attached to one session at a
/// time still shows the right state when you come back, even when the run ended while away.
#[test]
#[ignore = "needs a real Pi engine: set PIPKIN_PI_REPO"]
fn switching_conversations_keeps_each_ones_run_and_settles_what_ended_while_away() {
    let gate = Gate::new();
    let fx = Fixture::start(held_first(
        &gate,
        Reply::Text("a is done".into()),
        vec![Reply::Text("b is done".into())],
    ));
    init_project(&fx.project);
    let db = fx.root.join("app.sqlite3");
    let mut h = Harness::start(direct_config(&fx.server_dir, &fx.server_id), &db);
    h.open_new_conversation(&fx.project);
    let a = h.state.current().unwrap().id;
    h.send("work in A");
    h.until_asked(&fx.provider, 1);
    h.until("A is running", is_running);

    // A second conversation in the same project; the first keeps running behind it.
    h.dispatch(Command::NewConversation);
    h.until("B is open", |s| {
        s.selected.is_some_and(|id| id != a) && s.current().is_some_and(|c| c.opened)
    });
    let b = h.state.selected.unwrap();
    assert!(is_idle(&h.state), "B has no run");
    assert!(matches!(
        h.state.conversation(a).unwrap().run,
        RunState::Running { .. }
    ));
    // B works while A is held.
    h.send("work in B");
    h.until("B is answered", |s| {
        is_idle(s) && assistant_texts(s).contains(&"b is done".to_owned())
    });

    // A finishes while the user is looking at B.
    gate.open();
    h.until_asked(&fx.provider, 2);
    thread::sleep(Duration::from_millis(500));
    h.dispatch(Command::SelectConversation(a));
    h.until("A is attached again and settled", |s| {
        s.selected == Some(a)
            && s.current().is_some_and(|c| c.opened)
            && is_idle(s)
            && assistant_texts(s).contains(&"a is done".to_owned())
    });
    assert_eq!(h.user_messages(), ["work in A"]);
    // And B is still its own.
    h.dispatch(Command::SelectConversation(b));
    h.until("B shows its own history", |s| {
        s.selected == Some(b)
            && s.current().is_some_and(|c| c.opened)
            && assistant_texts(s).contains(&"b is done".to_owned())
    });
    assert_eq!(h.user_messages(), ["work in B"]);
    h.backend.shutdown();
}

// ------------------------------------------------------------------------------ M3: attachments

/// A valid 1x1 PNG: Pi decodes images before storing them, so the bytes must be real.
const TINY_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

#[test]
#[ignore = "needs a real Pi engine: set PIPKIN_PI_REPO"]
fn attachments_reach_the_model_as_text_and_image_and_come_back_as_chips() {
    use base64::Engine as _;
    let fx = Fixture::start(|_, _| Reply::Text("I read them.".into()));
    init_project(&fx.project);
    let db = fx.root.join("app.sqlite3");
    let mut h = Harness::start(direct_config(&fx.server_dir, &fx.server_id), &db);
    h.open_new_conversation(&fx.project);

    let outside = fx.root.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let notes = outside.join("spec.md");
    std::fs::write(&notes, "# Spec\nUse tabs.\n").unwrap();
    let image = outside.join("shot.png");
    std::fs::write(
        &image,
        base64::engine::general_purpose::STANDARD
            .decode(TINY_PNG)
            .unwrap(),
    )
    .unwrap();
    h.dispatch(Command::AddAttachments(vec![
        describe_attachment(&notes),
        describe_attachment(&image),
    ]));
    h.dispatch(Command::EditDraft("review these".into()));
    h.dispatch(Command::Submit);
    h.until("answered", |s| {
        is_idle(s) && assistant_texts(s).contains(&"I read them.".to_owned())
    });

    // The model was given the file's text and the image.
    let requests = fx.provider.requests();
    let body = requests[0].to_string();
    assert!(
        body.contains("review these") && body.contains("# Spec\\nUse tabs."),
        "{body}"
    );
    assert!(body.contains("data:image/png;base64,"), "{body}");

    // The transcript shows what the person typed and two chips, not the file's text.
    let c = h.state.current().unwrap();
    let ItemKind::User {
        text, attachments, ..
    } = &c
        .items
        .iter()
        .find(|i| matches!(i.kind, ItemKind::User { .. }))
        .unwrap()
        .kind
    else {
        unreachable!()
    };
    assert_eq!(text, "review these");
    let names: Vec<_> = attachments.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["spec.md", "Image"]);
    assert!(c.draft.attachments.is_empty(), "the draft was cleared");
    h.backend.shutdown();
}

#[test]
#[ignore = "needs a real Pi engine: set PIPKIN_PI_REPO"]
fn a_draft_with_attachments_survives_a_restart_and_a_missing_file_blocks_sending() {
    let fx = Fixture::start(|_, _| Reply::Text("ok".into()));
    init_project(&fx.project);
    let db = fx.root.join("app.sqlite3");
    let mut h = Harness::start(direct_config(&fx.server_dir, &fx.server_id), &db);
    h.open_new_conversation(&fx.project);
    let conversation = h.state.current().unwrap().id;

    let keep = fx.root.join("keep.txt");
    let gone = fx.root.join("gone.txt");
    std::fs::write(&keep, "kept").unwrap();
    std::fs::write(&gone, "will vanish").unwrap();
    h.dispatch(Command::AddAttachments(vec![
        describe_attachment(&keep),
        describe_attachment(&gone),
    ]));
    h.dispatch(Command::EditDraft("unsent thought".into()));
    h.dispatch(Command::FlushDraft(conversation));
    h.stop();

    std::fs::remove_file(&gone).unwrap();
    let mut h2 = Harness::start(direct_config(&fx.server_dir, &fx.server_id), &db);
    h2.until("the draft is back", |s| {
        s.selected == Some(conversation)
            && s.current().is_some_and(|c| {
                c.opened && c.draft.text == "unsent thought" && c.draft.attachments.len() == 2
            })
    });
    let c = h2.state.current().unwrap();
    assert!(c.draft.attachments[0].error.is_none());
    assert_eq!(
        c.draft.attachments[1].error.as_deref(),
        Some("File not found"),
        "a file that went away is flagged on reopen"
    );
    assert!(
        !h2.state.availability().submit,
        "a flagged attachment blocks sending until it is removed"
    );
    h2.dispatch(Command::RemoveAttachment(1));
    assert!(h2.state.availability().submit);
    h2.dispatch(Command::Submit);
    h2.until("sent and answered", |s| {
        is_idle(s) && assistant_texts(s).contains(&"ok".to_owned())
    });
    assert!(fx.provider.requests()[0].to_string().contains("kept"));
    h2.backend.shutdown();
}

// ------------------------------------------------------------------------ M3: the fault matrix

/// A held run behind a fault proxy: the engine, the app's database, and the project.
struct Held {
    fx: Fixture,
    proxy: FaultProxy,
    gate: Gate,
    db: PathBuf,
}

fn held_setup(rest: Vec<Reply>) -> Held {
    let gate = Gate::new();
    let fx = Fixture::start(held_first(&gate, Reply::Text("first done".into()), rest));
    init_project(&fx.project);
    let proxy = FaultProxy::start(
        fx.server_dir.join(format!("{}.sock", fx.server_id)),
        fx.root.join("proxy"),
        &fx.server_id,
    );
    let db = fx.root.join("app.sqlite3");
    Held {
        fx,
        proxy,
        gate,
        db,
    }
}

fn journal_is_clean(db: &Path) {
    if std::env::var_os("PIPKIN_DEBUG_JOURNAL").is_some() {
        let conn = rusqlite::Connection::open(db).unwrap();
        let mut stmt = conn
            .prepare("SELECT request_id, state, payload FROM request_journal ORDER BY rowid")
            .unwrap();
        for row in stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .unwrap()
        {
            eprintln!("journal: {:?}", row.unwrap());
        }
    }
    let loaded = Storage::open(db, NAMESPACE).unwrap().load_all().unwrap();
    assert!(
        loaded.open_requests.is_empty(),
        "unresolved journal entries: {:?}",
        loaded.open_requests
    );
}

/// The engine admitted a follow-up but the acknowledgment never arrived. The app reconnects,
/// asks the engine about the key, finds it, and shows it once: no duplicate, no resend.
#[test]
#[ignore = "needs a real Pi engine: set PIPKIN_PI_REPO"]
fn a_follow_up_whose_acknowledgment_is_lost_is_admitted_exactly_once() {
    let f = held_setup(vec![Reply::Text("second done".into())]);
    let mut h = Harness::start(direct_config(&f.proxy.dir, &f.fx.server_id), &f.db);
    h.open_new_conversation(&f.fx.project);
    h.send("first");
    h.until_asked(&f.fx.provider, 1);
    h.until("running", is_running);

    f.proxy.arm(Fault::DropNextQueueReply);
    h.dispatch(Command::EditDraft("second".into()));
    h.dispatch(Command::QueueFollowUp);
    h.until("the acknowledgment was lost", |_| f.proxy.fired() == 1);
    h.until("the app resolves it by asking the engine", |s| {
        s.connection.is_ready()
            && queued(s) == [(QueueMode::FollowUp, "second".to_owned())]
            && s.current().unwrap().pending_queue.is_empty()
    });
    assert!(is_running(&h.state));

    f.gate.open();
    h.until("both ran, once", |s| {
        is_idle(s) && assistant_texts(s) == ["first done".to_owned(), "second done".to_owned()]
    });
    assert_eq!(h.user_messages(), ["first", "second"]);
    assert_eq!(f.fx.provider.requests().len(), 2);
    h.storage.shutdown();
    journal_is_clean(&f.db);
    h.backend.shutdown();
}

/// The follow-up never reached the engine. The same key is sent again, and it is admitted once.
#[test]
#[ignore = "needs a real Pi engine: set PIPKIN_PI_REPO"]
fn a_follow_up_that_never_reached_the_engine_is_sent_again_under_its_key_once() {
    let f = held_setup(vec![Reply::Text("second done".into())]);
    let mut h = Harness::start(direct_config(&f.proxy.dir, &f.fx.server_id), &f.db);
    h.open_new_conversation(&f.fx.project);
    h.send("first");
    h.until_asked(&f.fx.provider, 1);
    h.until("running", is_running);

    f.proxy.arm(Fault::DropNextQueueRequest);
    h.dispatch(Command::EditDraft("second".into()));
    h.dispatch(Command::QueueFollowUp);
    h.until("the request was cut", |_| f.proxy.fired() == 1);
    h.until("it was sent again and admitted", |s| {
        s.connection.is_ready()
            && queued(s) == [(QueueMode::FollowUp, "second".to_owned())]
            && s.current().unwrap().pending_queue.is_empty()
    });

    f.gate.open();
    h.until("both ran, once", |s| {
        is_idle(s) && assistant_texts(s) == ["first done".to_owned(), "second done".to_owned()]
    });
    assert_eq!(h.user_messages(), ["first", "second"]);
    assert_eq!(f.fx.provider.requests().len(), 2);
    h.storage.shutdown();
    journal_is_clean(&f.db);
    h.backend.shutdown();
}

/// The acknowledgment is lost and the app is killed before it can ask. After relaunch the
/// journal's open entry is looked up, never resent, and the follow-up still runs exactly once.
#[test]
#[ignore = "needs a real Pi engine: set PIPKIN_PI_REPO"]
fn a_lost_follow_up_acknowledgment_then_a_kill_and_relaunch_runs_it_once() {
    let f = held_setup(vec![Reply::Text("second done".into())]);
    let mut config = direct_config(&f.proxy.dir, &f.fx.server_id);
    config.retry_delay = Duration::from_secs(30); // the app is killed before it reconnects
    let mut h = Harness::start(config, &f.db);
    h.open_new_conversation(&f.fx.project);
    let conversation = h.state.current().unwrap().id;
    h.send("first");
    h.until_asked(&f.fx.provider, 1);
    h.until("running", is_running);

    f.proxy.arm(Fault::DropNextQueueReply);
    h.dispatch(Command::EditDraft("second".into()));
    h.dispatch(Command::QueueFollowUp);
    h.until("the acknowledgment was lost", |s| {
        f.proxy.fired() == 1
            && s.current()
                .unwrap()
                .pending_queue
                .iter()
                .any(|p| p.state == pipkin_core::QueueSend::Unknown)
    });
    h.stop();

    let mut h2 = Harness::start(direct_config(&f.fx.server_dir, &f.fx.server_id), &f.db);
    h2.until("the engine's side of it is found", |s| {
        s.selected == Some(conversation)
            && is_running(s)
            && queued(s) == [(QueueMode::FollowUp, "second".to_owned())]
    });
    assert_eq!(f.fx.provider.requests().len(), 1, "nothing was resent");

    f.gate.open();
    h2.until("both ran, once", |s| {
        is_idle(s) && assistant_texts(s) == ["first done".to_owned(), "second done".to_owned()]
    });
    assert_eq!(h2.user_messages(), ["first", "second"]);
    assert_eq!(f.fx.provider.requests().len(), 2);
    h2.stop();
    journal_is_clean(&f.db);
}

/// The link to the engine drops while a run is in flight and the engine carries on: the app
/// reconnects, keeps showing the run and its queue, and the run settles on the engine's word.
#[test]
#[ignore = "needs a real Pi engine: set PIPKIN_PI_REPO"]
fn losing_the_connection_mid_run_reconnects_and_the_run_still_settles() {
    let f = held_setup(vec![Reply::Text("second done".into())]);
    let mut h = Harness::start(direct_config(&f.proxy.dir, &f.fx.server_id), &f.db);
    h.open_new_conversation(&f.fx.project);
    h.send("first");
    h.until_asked(&f.fx.provider, 1);
    h.until("running", is_running);
    h.dispatch(Command::EditDraft("second".into()));
    h.dispatch(Command::QueueFollowUp);
    h.until("queued", |s| {
        queued(s) == [(QueueMode::FollowUp, "second".to_owned())]
    });

    f.proxy.sever_all();
    h.until("the loss is noticed", |s| !s.connection.is_ready());
    // While the engine is unreachable nothing can be sent, and nothing is claimed.
    assert!(!h.state.availability().submit && !h.state.availability().cancel);
    h.until("reconnected", |s| s.connection.is_ready());
    assert!(is_running(&h.state), "the run is still shown");
    assert_eq!(queued(&h.state).len(), 1, "and so is its queue");

    f.gate.open();
    h.until("everything finished", |s| {
        is_idle(s) && assistant_texts(s) == ["first done".to_owned(), "second done".to_owned()]
    });
    assert_eq!(f.fx.provider.requests().len(), 2, "no turn was repeated");
    assert_eq!(h.user_messages(), ["first", "second"]);
    h.storage.shutdown();
    journal_is_clean(&f.db);
    h.backend.shutdown();
}

/// Storage refuses the journal write (disk full, say): nothing is sent, the text stays in the
/// composer, and once storage works again the same text sends, once.
#[test]
#[ignore = "needs a real Pi engine: set PIPKIN_PI_REPO"]
fn a_prompt_that_cannot_be_journaled_is_not_sent_and_sends_once_storage_recovers() {
    let fx = Fixture::start(|_, _| Reply::Text("ok".into()));
    init_project(&fx.project);
    let db = fx.root.join("app.sqlite3");
    let mut h = Harness::start(direct_config(&fx.server_dir, &fx.server_id), &db);
    h.open_new_conversation(&fx.project);

    h.storage.set_write_failure(true);
    h.send("important words");
    h.until("the refusal is shown", |s| {
        s.current().unwrap().intent_error.is_some()
    });
    let c = h.state.current().unwrap();
    assert_eq!(c.draft.text, "important words", "nothing was lost");
    assert!(is_idle(&h.state));
    thread::sleep(Duration::from_millis(500));
    assert_eq!(fx.provider.requests().len(), 0, "nothing reached the model");
    assert!(h.user_messages().is_empty());

    h.storage.set_write_failure(false);
    h.dispatch(Command::DismissFailure);
    h.dispatch(Command::Submit);
    h.until("it sends once", |s| {
        is_idle(s) && assistant_texts(s) == ["ok".to_owned()]
    });
    assert_eq!(h.user_messages(), ["important words"]);
    assert_eq!(fx.provider.requests().len(), 1);
    h.backend.shutdown();
}

/// The engine process dies hard while a run is in flight. The app restarts it and reconnects;
/// the engine's own durable recovery resumes the interrupted run. The app resends nothing: the
/// prompt exists once, in the transcript, in the model's context, and in the journal.
#[test]
#[ignore = "needs a real Pi engine: set PIPKIN_PI_REPO"]
fn an_engine_crash_mid_run_is_recovered_by_the_engine_and_the_prompt_is_not_duplicated() {
    let gate = Gate::new();
    let g = gate.clone();
    let p: Prepared = prepare(move |_, n| match n {
        0 => Reply::Gated(g.clone(), Box::new(Reply::Text("never delivered".into()))),
        n => Reply::Text(format!("recovered answer {n}")),
    });
    init_project(&p.project);
    let (dir, id) = (p.server_dir.clone(), p.server_id.clone());
    let db = p.root.join("app.sqlite3");
    let mut config = PiConfig::managed(p.config.clone());
    config.retry_delay = Duration::from_millis(200);
    let mut h = Harness::start(config, &db);
    h.open_new_conversation(&p.project);
    h.send("first");
    h.until_asked(&p.provider, 1);
    h.until("running", is_running);

    for pid in profile_pids(&dir, &id) {
        // SAFETY: SIGKILL to processes matched by this test's own profile identity.
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    }
    h.until("the app sees the loss", |s| !s.connection.is_ready());
    assert!(
        !h.state.availability().submit,
        "nothing can be sent while the engine is down"
    );
    h.until("the app restarted the engine and reconnected", |s| {
        s.connection.is_ready()
    });
    h.until("the engine resumed the run and finished it", |s| {
        is_idle(s)
            && assistant_texts(s)
                .iter()
                .any(|t| t.starts_with("recovered answer"))
    });

    assert_eq!(h.user_messages(), ["first"], "one prompt, once");
    for request in p.provider.requests() {
        assert_eq!(user_texts(&request), ["first"], "never a second copy");
    }
    h.storage.shutdown();
    journal_is_clean(&db);
    gate.open();
    h.backend.shutdown();
    drop(p.provider);
}

/// Credentials are Pi's: the app shows "no model is ready" when the engine has none, and a
/// refresh picks up a provider that was configured after the engine started.
#[test]
#[ignore = "needs a real Pi engine: set PIPKIN_PI_REPO"]
fn a_provider_configured_after_start_appears_after_a_refresh_and_can_then_be_used() {
    let mut p: Prepared = prepare(|_, _| Reply::Text("hello from the new provider".into()));
    init_project(&p.project);
    // The engine starts with no provider at all.
    let agent_dir = p.config.agent_dir.clone().unwrap();
    let configured = std::fs::read_to_string(agent_dir.join("models.json")).unwrap();
    std::fs::write(agent_dir.join("models.json"), r#"{"providers":{}}"#).unwrap();
    p.config.model = None;
    let db = p.root.join("app.sqlite3");
    let mut h = Harness::start(PiConfig::managed(p.config.clone()), &db);
    h.open_new_conversation(&p.project);

    assert!(h.state.models.is_empty(), "{:?}", h.state.models);
    assert!(
        h.state.availability().refresh_models,
        "a refresh is offered when there is no model"
    );

    // The user adds the provider (Pi's own configuration), then refreshes.
    std::fs::write(agent_dir.join("models.json"), configured).unwrap();
    h.dispatch(Command::RefreshModels);
    h.until("the provider's model appears", |s| {
        s.models.iter().any(|m| m.id == "stub/scripted")
    });

    h.dispatch(Command::SetModel("stub/scripted".into()));
    h.until("the engine reports the selection", |s| {
        s.prefs.model.as_deref() == Some("stub/scripted")
    });
    h.send("hello");
    h.until("it answers", |s| {
        is_idle(s) && assistant_texts(s) == ["hello from the new provider".to_owned()]
    });
    h.stop();
    drop(p.provider);
}
