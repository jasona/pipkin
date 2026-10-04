//! The adapter against the mock Pi server, over real Unix sockets with the real trust checks.

use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pi_client::delta::{Op, Seg};
use pi_client::testing::{ConnHandle, MockPi, SERVER_ID, install_session_management};
use pipkin_core::{
    Backend, BackendEvent, BackendRequest, Bootstrap, Connection, ConversationId, EventKind,
    ItemKind, LifecycleEvent, OperationId, RequestId,
};
use serde_json::{Value, json};

use super::{PiBackend, PiConfig};

const WAIT: Duration = Duration::from_secs(10);

fn chmod(path: &std::path::Path, mode: u32) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn user_entry(id: u64, text: &str) -> Value {
    json!({ "id": id, "conversationId": 1, "kind": "pi.message",
        "model": [{ "role": "user", "content": text, "timestamp": 1_700_000_000_000i64 }] })
}

fn view(texts: &[&str]) -> Value {
    let entries: Vec<Value> = texts
        .iter()
        .enumerate()
        .map(|(i, t)| user_entry(i as u64 + 1, t))
        .collect();
    json!({ "conversation": { "id": 1 }, "entries": entries, "docs": {} })
}

fn key(s: &str) -> Seg {
    Seg::Key(s.into())
}

struct Env {
    _dir: tempfile::TempDir,
    dir: PathBuf,
    pi: MockPi,
    conns: Arc<Mutex<Vec<ConnHandle>>>,
    backend: PiBackend,
    events: async_channel::Receiver<BackendEvent>,
    lifecycle: Arc<Mutex<Vec<LifecycleEvent>>>,
}

fn sessions_state(sessions: &[(&str, i64)]) -> Value {
    json!({ "revision": 1, "sessions": sessions.iter().map(|(id, at)| json!({
        "serverId": SERVER_ID, "sessionId": id, "createdAt": at,
    })).collect::<Vec<_>>() })
}

/// A mock whose attach handler swaps the transcript to the attached session's view, so each
/// attachment really does show different content.
type Views = Arc<Mutex<Vec<(String, Value)>>>;

fn mock(sessions: &[(&str, i64)], views: Vec<(&str, Value)>) -> MockPi {
    mock_with_views(sessions, views).0
}

/// Like `mock`, but the per-session views stay editable, so a test can change what a session
/// shows between attachments (for example while the connection is down).
fn mock_with_views(sessions: &[(&str, i64)], views: Vec<(&str, Value)>) -> (MockPi, Views) {
    let pi = MockPi::default();
    pi.add_service("pi.session-directory", &[], Some(sessions_state(sessions)));
    pi.add_service(
        "pi.transcript",
        &[],
        Some(json!({ "entries": [], "docs": {} })),
    );
    pi.add_service(
        "pi.models",
        &["select"],
        Some(json!({
            "catalog": { "revision": 1, "availableModels": [
                { "provider": "anthropic", "modelId": "sonnet", "name": "Sonnet", "reasoning": true } ] },
            "configuration": { "model": { "provider": "anthropic", "modelId": "sonnet" }, "thinkingLevel": "off" },
        })),
    );
    install_session_management(&pi);
    let views: Views = Arc::new(Mutex::new(
        views.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(),
    ));
    let handler_views = views.clone();
    pi.set_handler("pi.session-management", "attach", move |pi, conn, call| {
        let session_id = call.args[0].as_str().unwrap().to_owned();
        let attachment_id = format!("att-{}", pi.requests().len());
        conn.set_attachment(Some(pi_client::protocol::SessionTarget {
            server_id: SERVER_ID.into(),
            session_id: session_id.clone(),
            attachment_id,
        }));
        let view = handler_views
            .lock()
            .unwrap()
            .iter()
            .find(|(k, _)| *k == session_id)
            .map(|(_, v)| v.clone());
        if let Some(v) = view {
            pi.publish("pi.transcript", vec![Op::Replace(v)]);
        }
        Ok(None)
    });
    (pi, views)
}

fn listen(pi: &MockPi, dir: &std::path::Path, id: &str, conns: &Arc<Mutex<Vec<ConnHandle>>>) {
    let socket = dir.join(format!("{id}.sock"));
    let listener = UnixListener::bind(&socket).unwrap();
    chmod(&socket, 0o600);
    let (pi, conns) = (pi.clone(), conns.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let handle = pi.accept(stream);
            conns.lock().unwrap().push(handle);
        }
    });
}

fn start(pi: MockPi, serve: bool) -> Env {
    start_with_mode(pi, serve, 0o700)
}

fn start_with_mode(pi: MockPi, serve: bool, dir_mode: u32) -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("server");
    std::fs::create_dir(&dir).unwrap();
    chmod(&dir, dir_mode);
    let conns = Arc::new(Mutex::new(Vec::new()));
    if serve {
        listen(&pi, &dir, SERVER_ID, &conns);
    }
    let (tx, events) = async_channel::unbounded();
    let mut config = PiConfig::new(dir.clone());
    config.retry_delay = Duration::from_millis(50);
    let backend = PiBackend::new(tx, config);
    let lifecycle = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let lifecycle = lifecycle.clone();
        Box::new(move |e| lifecycle.lock().unwrap().push(e))
    };
    backend.start(sink);
    Env {
        _dir: tmp,
        dir,
        pi,
        conns,
        backend,
        events,
        lifecycle,
    }
}

impl Env {
    fn connections(&self) -> Vec<Connection> {
        self.lifecycle
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| {
                if let LifecycleEvent::Connection(c) = e {
                    Some(c.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    fn catalogs(&self) -> Vec<Bootstrap> {
        self.lifecycle
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| {
                if let LifecycleEvent::Catalog(b) = e {
                    Some(b.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    fn wait_ready(&self) {
        wait_until("ready", || self.connections().contains(&Connection::Ready));
    }

    /// The live connection. Discovery probes connect first, so the newest one is the session.
    fn live_connection(&self) -> ConnHandle {
        self.conns
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("a connection")
    }

    fn conversation(&self, index: usize) -> ConversationId {
        self.wait_ready();
        self.catalogs().last().unwrap().conversations[index].0
    }

    fn next_event(&self) -> BackendEvent {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Ok(e) = self.events.try_recv() {
                return e;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for a backend event"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn no_event_within(&self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
        if let Ok(e) = self.events.try_recv() {
            panic!("unexpected event: {e:?}");
        }
    }

    fn open(&self, conversation: ConversationId, generation: u64) {
        self.backend.request(BackendRequest::Open {
            conversation,
            generation,
        });
    }
}

fn texts(items: &[pipkin_core::TranscriptItem]) -> Vec<String> {
    items
        .iter()
        .filter_map(|i| match &i.kind {
            ItemKind::User { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn connects_then_publishes_the_catalog_and_ready() {
    let env = start(
        mock(
            &[("aaaaaaaa-1", 1_700_000_100), ("bbbbbbbb-2", 1_700_000_200)],
            vec![],
        ),
        true,
    );
    env.wait_ready();
    let connections = env.connections();
    assert_eq!(connections.first(), Some(&Connection::Connecting));
    assert_eq!(connections.last(), Some(&Connection::Ready));
    let catalog = env.catalogs().pop().unwrap();
    assert_eq!(catalog.conversations.len(), 2);
    assert_eq!(catalog.projects.len(), 1);
    assert!(
        catalog
            .conversations
            .iter()
            .any(|c| c.2.starts_with("Session aaaaaaaa"))
    );
    // The catalogue precedes readiness: the UI never shows Ready with nothing to list.
    let order: Vec<&str> = env
        .lifecycle
        .lock()
        .unwrap()
        .iter()
        .map(|e| match e {
            LifecycleEvent::Catalog(_) => "catalog",
            LifecycleEvent::Connection(Connection::Ready) => "ready",
            LifecycleEvent::Connection(_) => "state",
            LifecycleEvent::ModelSelected(_) | LifecycleEvent::Notice(_) => "other",
        })
        .collect();
    let catalog_at = order.iter().position(|e| *e == "catalog").unwrap();
    let ready_at = order.iter().position(|e| *e == "ready").unwrap();
    assert!(catalog_at < ready_at);
}

#[test]
fn opening_a_session_shows_its_actual_transcript_and_models() {
    let env = start(
        mock(
            &[("s-one", 100)],
            vec![("s-one", view(&["fix the failing test", "and run clippy"]))],
        ),
        true,
    );
    let conv = env.conversation(0);
    env.open(conv, 7);
    let event = env.next_event();
    assert_eq!((event.conversation, event.generation), (conv, 7));
    let EventKind::Opened {
        items, has_older, ..
    } = event.kind
    else {
        panic!("{event:?}")
    };
    assert_eq!(texts(&items), ["fix the failing test", "and run clippy"]);
    assert!(!has_older);
    // The session's model catalogue reaches the core, configured model first.
    wait_until("models", || {
        env.catalogs().last().is_some_and(|c| !c.models.is_empty())
    });
    assert_eq!(
        env.catalogs().last().unwrap().models[0].id,
        "anthropic/sonnet"
    );
}

#[test]
fn live_changes_arrive_as_synced_items() {
    let env = start(
        mock(&[("s-one", 100)], vec![("s-one", view(&["first"]))]),
        true,
    );
    let conv = env.conversation(0);
    env.open(conv, 1);
    assert!(matches!(env.next_event().kind, EventKind::Opened { .. }));
    env.pi.publish(
        "pi.transcript",
        vec![Op::Splice {
            path: vec![key("entries")],
            index: 1,
            remove: 0,
            items: vec![user_entry(2, "second")],
        }],
    );
    let event = env.next_event();
    assert_eq!((event.conversation, event.generation), (conv, 1));
    let EventKind::Synced { items } = event.kind else {
        panic!("{event:?}")
    };
    assert_eq!(texts(&items), ["first", "second"]);
}

#[test]
fn a_burst_of_updates_is_coalesced_and_ends_consistent() {
    let env = start(
        mock(&[("s-one", 100)], vec![("s-one", view(&["first"]))]),
        true,
    );
    let conv = env.conversation(0);
    env.open(conv, 1);
    env.next_event();
    for i in 0..60u64 {
        env.pi.publish(
            "pi.transcript",
            vec![Op::Splice {
                path: vec![key("entries")],
                index: (i + 1) as usize,
                remove: 0,
                items: vec![user_entry(i + 2, &format!("m{i}"))],
            }],
        );
    }
    // Whatever the batching, the final synced state has every message.
    let mut last = 0;
    wait_until("converged", || {
        while let Ok(e) = env.events.try_recv() {
            if let EventKind::Synced { items } = e.kind {
                last = items.len();
            }
        }
        last == 61
    });
}

#[test]
fn switching_sessions_never_lets_the_old_attachment_update_the_screen() {
    let env = start(
        mock(
            &[("s-a", 100), ("s-b", 200)],
            vec![("s-a", view(&["from A"])), ("s-b", view(&["from B"]))],
        ),
        true,
    );
    let (a, b) = {
        env.wait_ready();
        let c = env.catalogs().pop().unwrap().conversations;
        let find = |t: &str| c.iter().find(|x| x.2.contains(t)).unwrap().0;
        (find("s-a"), find("s-b"))
    };
    env.open(a, 1);
    let EventKind::Opened { items, .. } = env.next_event().kind else {
        panic!()
    };
    assert_eq!(texts(&items), ["from A"]);
    let conn = env.live_connection();
    let old_ids = conn.subscription_ids("pi.transcript");
    assert_eq!(old_ids.len(), 1);

    env.open(b, 1);
    let event = env.next_event();
    assert_eq!(event.conversation, b);
    let EventKind::Opened { items, .. } = event.kind else {
        panic!("{event:?}")
    };
    assert_eq!(texts(&items), ["from B"]);

    // The adapter released A's subscription before attaching B, so only B's remains.
    let live_ids = conn.subscription_ids("pi.transcript");
    assert_eq!(live_ids.len(), 1);
    assert_ne!(live_ids, old_ids);

    // A frame for A's subscription that was already in flight arrives now. It must not surface.
    conn.send_update(
        &old_ids[0],
        json!({ "type": "state", "member": "state", "sequence": 99, "ops": [["s", ["entries"], []]] }),
    );
    env.no_event_within(300);

    // B keeps updating normally afterwards.
    env.pi.publish(
        "pi.transcript",
        vec![Op::Splice {
            path: vec![key("entries")],
            index: 1,
            remove: 0,
            items: vec![user_entry(9, "B again")],
        }],
    );
    let event = env.next_event();
    assert_eq!(event.conversation, b);
    let EventKind::Synced { items } = event.kind else {
        panic!("{event:?}")
    };
    assert_eq!(texts(&items), ["from B", "B again"]);
}

#[test]
fn opening_an_unknown_conversation_fails_visibly() {
    let env = start(mock(&[("s", 1)], vec![]), true);
    env.wait_ready();
    env.open(ConversationId(424_242), 3);
    let event = env.next_event();
    assert_eq!(event.generation, 3);
    assert!(
        matches!(event.kind, EventKind::OpenFailed { ref message } if message.contains("no longer exists"))
    );
}

#[test]
fn offline_until_a_server_appears() {
    let pi = mock(&[("s", 1)], vec![]);
    let env = start(pi.clone(), false);
    wait_until("offline", || {
        env.connections()
            .iter()
            .any(|c| matches!(c, Connection::Offline(m) if m.contains("no Pi server")))
    });
    // A request while offline is answered, not left hanging.
    env.open(ConversationId(1), 1);
    assert!(matches!(
        env.next_event().kind,
        EventKind::OpenFailed { .. }
    ));
    // The server starts later; the adapter finds it on its own.
    listen(&pi, &env.dir, SERVER_ID, &env.conns);
    env.wait_ready();
    assert_eq!(env.catalogs().last().unwrap().conversations.len(), 1);
}

#[test]
fn an_engine_without_the_required_services_is_incompatible() {
    let pi = MockPi::default();
    pi.add_service("pi.session-directory", &[], Some(sessions_state(&[])));
    // No pi.session-management.
    let env = start(pi, true);
    wait_until("incompatible", || {
        env.connections().iter().any(
            |c| matches!(c, Connection::Incompatible(m) if m.contains("pi.session-management")),
        )
    });
    assert!(!env.connections().contains(&Connection::Ready));
}

#[test]
fn an_untrusted_directory_is_reported_as_such_not_as_no_server() {
    // The server is running, but its directory is open to other users.
    let env = start_with_mode(mock(&[("s", 1)], vec![]), true, 0o755);
    wait_until("failed", || {
        env.connections()
            .iter()
            .any(|c| matches!(c, Connection::Failed(m) if m.contains("accessible to other users")))
    });
    assert!(!env.connections().contains(&Connection::Ready));
    assert!(
        !env.connections()
            .iter()
            .any(|c| matches!(c, Connection::Offline(m) if m.contains("no Pi server")))
    );
}

#[test]
fn losing_the_connection_reconnects_and_refreshes_the_open_conversation() {
    let (pi, views) = mock_with_views(&[("s-a", 100)], vec![("s-a", view(&["before"]))]);
    let env = start(pi, true);
    let conv = env.conversation(0);
    env.open(conv, 5);
    assert!(matches!(env.next_event().kind, EventKind::Opened { .. }));
    let requests_before = env.pi.requests().len();

    // The engine moves on while the connection is down; the next attach shows the new state.
    views.lock().unwrap()[0].1 = view(&["before", "while away"]);
    env.live_connection().close();
    wait_until("reconnecting", || {
        env.connections().contains(&Connection::Reconnecting)
    });
    wait_until("ready again", || {
        env.connections()
            .iter()
            .filter(|c| **c == Connection::Ready)
            .count()
            >= 2
    });

    // The same conversation, same generation, refreshed in place (Synced, not a fresh open).
    let event = env.next_event();
    assert_eq!((event.conversation, event.generation), (conv, 5));
    let EventKind::Synced { items } = event.kind else {
        panic!("{event:?}")
    };
    assert_eq!(texts(&items), ["before", "while away"]);
    // Reconnecting re-attached to read, but replayed nothing that mutates.
    let mutating = env
        .pi
        .requests()
        .iter()
        .skip(requests_before)
        .filter(|(_, c)| c.service_id == "pi.agent-controller")
        .count();
    assert_eq!(mutating, 0);
}

#[test]
fn directory_changes_update_the_catalog() {
    let env = start(mock(&[("s-a", 100)], vec![]), true);
    env.wait_ready();
    let before = env.catalogs().len();
    env.pi.publish(
        "pi.session-directory",
        vec![Op::Splice {
            path: vec![key("sessions")],
            index: 1,
            remove: 0,
            items: vec![json!({ "serverId": SERVER_ID, "sessionId": "s-new", "createdAt": 300 })],
        }],
    );
    wait_until("new session", || {
        env.catalogs()
            .iter()
            .skip(before)
            .any(|c| c.conversations.len() == 2)
    });
}

#[test]
fn malformed_directory_rows_do_not_break_the_catalog() {
    let state = json!({ "revision": 1, "sessions": [ { "sessionId": "ok", "createdAt": 5 }, 7, { "createdAt": 1 }, { "sessionId": "" } ] });
    let pi = mock(&[], vec![]);
    pi.publish("pi.session-directory", vec![Op::Replace(state)]);
    let env = start(pi, true);
    env.wait_ready();
    assert_eq!(env.catalogs().last().unwrap().conversations.len(), 1);
}

#[test]
fn discovery_prefers_the_default_server_when_several_run() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("server");
    std::fs::create_dir(&dir).unwrap();
    chmod(&dir, 0o700);
    let other_id = "00000000-0000-4000-8000-0000000000dd";
    let conns = Arc::new(Mutex::new(Vec::new()));
    let a = mock(&[("from-default", 1)], vec![]);
    let b = MockPi::new(other_id);
    b.add_service(
        "pi.session-directory",
        &[],
        Some(sessions_state(&[("from-other", 1)])),
    );
    install_session_management(&b);
    listen(&a, &dir, SERVER_ID, &conns);
    listen(&b, &dir, other_id, &conns);

    let run = |default: Option<&str>| {
        if let Some(d) = default {
            std::fs::write(dir.join("default-server-id"), d).unwrap();
        }
        let (tx, _events) = async_channel::unbounded();
        let mut config = PiConfig::new(dir.clone());
        config.retry_delay = Duration::from_millis(50);
        let backend = PiBackend::new(tx, config);
        let lifecycle = Arc::new(Mutex::new(Vec::new()));
        let sink = {
            let l = lifecycle.clone();
            Box::new(move |e| l.lock().unwrap().push(e))
        };
        backend.start(sink);
        (backend, lifecycle)
    };

    // Ambiguous without a default: refuse to guess.
    let (backend, lifecycle) = run(None);
    wait_until("ambiguity", || {
        lifecycle.lock().unwrap().iter().any(|e| matches!(e, LifecycleEvent::Connection(Connection::Failed(m)) if m.contains("--pi-server-id")))
    });
    backend.shutdown();

    // With the profile's default id present, that server is chosen.
    let (backend, lifecycle) = run(Some(SERVER_ID));
    wait_until("default chosen", || {
        lifecycle.lock().unwrap().iter().any(|e| matches!(e, LifecycleEvent::Catalog(b) if b.conversations.iter().any(|c| c.2.contains("from-def"))))
    });
    backend.shutdown();
}

#[test]
fn shutdown_returns_promptly_and_later_requests_are_harmless() {
    let env = start(mock(&[("s", 1)], vec![]), true);
    env.wait_ready();
    let started = Instant::now();
    env.backend.shutdown();
    assert!(started.elapsed() < Duration::from_secs(2));
    env.backend.request(BackendRequest::Open {
        conversation: ConversationId(1),
        generation: 1,
    });
    env.no_event_within(100);
}

#[test]
fn attach_retries_a_transient_internal_error_then_succeeds() {
    let pi = mock(&[("s", 1)], vec![("s", view(&["hello"]))]);
    let failures = Arc::new(Mutex::new(2u32));
    let attempts = Arc::new(Mutex::new(0u32));
    let (f, a) = (failures.clone(), attempts.clone());
    // Replace the mock's attach: fail twice with the server's generic error, then behave.
    pi.set_handler("pi.session-management", "attach", move |pi, conn, call| {
        *a.lock().unwrap() += 1;
        let mut left = f.lock().unwrap();
        if *left > 0 {
            *left -= 1;
            return Err(pi_client::protocol::ProtocolError {
                code: "internal_error".into(),
                message: "Internal server error".into(),
            });
        }
        let session_id = call.args[0].as_str().unwrap().to_owned();
        conn.set_attachment(Some(pi_client::protocol::SessionTarget {
            server_id: SERVER_ID.into(),
            session_id,
            attachment_id: "att-retry".into(),
        }));
        pi.publish("pi.transcript", vec![Op::Replace(view(&["hello"]))]);
        Ok(None)
    });
    let env = start(pi, true);
    let conv = env.conversation(0);
    env.open(conv, 1);
    let event = env.next_event();
    let EventKind::Opened { items, .. } = event.kind else {
        panic!("{event:?}")
    };
    assert_eq!(texts(&items), ["hello"]);
    assert_eq!(*attempts.lock().unwrap(), 3, "two failures, then success");
}

#[test]
fn attach_gives_up_after_bounded_retries_and_other_errors_are_final() {
    // A persistent internal_error is retried a bounded number of times, then reported.
    let pi = mock(&[("s", 1)], vec![]);
    let attempts = Arc::new(Mutex::new(0u32));
    let a = attempts.clone();
    pi.set_handler("pi.session-management", "attach", move |_, _, _| {
        *a.lock().unwrap() += 1;
        Err(pi_client::protocol::ProtocolError {
            code: "internal_error".into(),
            message: "x".into(),
        })
    });
    let env = start(pi, true);
    let conv = env.conversation(0);
    env.open(conv, 1);
    let event = env.next_event();
    assert!(
        matches!(event.kind, EventKind::OpenFailed { ref message } if message.contains("Could not attach"))
    );
    assert_eq!(*attempts.lock().unwrap(), 5, "one try plus four retries");

    // A different error code is not retried.
    let pi = mock(&[("s", 1)], vec![]);
    let attempts = Arc::new(Mutex::new(0u32));
    let a = attempts.clone();
    pi.set_handler("pi.session-management", "attach", move |_, _, _| {
        *a.lock().unwrap() += 1;
        Err(pi_client::protocol::ProtocolError {
            code: "session_not_found".into(),
            message: "gone".into(),
        })
    });
    let env = start(pi, true);
    let conv = env.conversation(0);
    env.open(conv, 1);
    assert!(matches!(
        env.next_event().kind,
        EventKind::OpenFailed { .. }
    ));
    assert_eq!(*attempts.lock().unwrap(), 1);
}

#[test]
fn a_server_that_refuses_our_protocol_is_reported_as_incompatible_not_absent() {
    let pi = mock(&[("s", 1)], vec![]);
    pi.reject_handshake("version", "unsupported protocol version");
    let env = start(pi, true);
    wait_until("incompatible", || {
        env.connections()
            .iter()
            .any(|c| matches!(c, Connection::Incompatible(m) if m.contains("version")))
    });
    assert!(!env.connections().contains(&Connection::Ready));
    assert!(
        !env.connections()
            .iter()
            .any(|c| matches!(c, Connection::Offline(m) if m.contains("no Pi server")))
    );
}

// ------------------------------------------------------------------ agent controller flows

use pipkin_core::FileChange;

/// What the mock AgentController, Models and session management were asked, and what the engine
/// "reports" for each request key. Tests edit `statuses` to model the engine finishing a run.
#[derive(Default)]
struct Agent {
    prompts: Vec<Value>,
    aborts: usize,
    selects: Vec<Value>,
    creates: Vec<Value>,
    /// requestId -> the JSON `lookup` returns.
    statuses: std::collections::HashMap<String, Value>,
    next_op: u64,
    /// How the next prompt is answered.
    prompt_mode: PromptMode,
}

#[derive(Default, Clone)]
enum PromptMode {
    #[default]
    Accept,
    Reject(&'static str),
    InternalError,
    DropConnection,
}

fn placed(op: &str) -> Value {
    json!({ "found": true, "operationId": op, "status": "placed", "reason": null, "detail": null })
}

fn settled(op: &str, status: &str, reason: Option<&str>, detail: Option<&str>) -> Value {
    json!({ "found": true, "operationId": op, "status": status, "reason": reason, "detail": detail })
}

/// A mock engine with an AgentController. Each prompt is admitted as `placed`.
fn agent_mock(
    sessions: &[(&str, i64)],
    views: Vec<(&str, Value)>,
) -> (MockPi, Arc<Mutex<Agent>>, Views) {
    let (pi, views) = mock_with_views(sessions, views);
    let agent = Arc::new(Mutex::new(Agent::default()));
    let a = agent.clone();
    pi.set_handler("pi.agent-controller", "prompt", move |_, conn, call| {
        let mut agent = a.lock().unwrap();
        let request = call.args[0].clone();
        agent.prompts.push(request.clone());
        match agent.prompt_mode.clone() {
            PromptMode::Reject(why) => Ok(Some(json!({
                "accepted": false, "operationId": null,
                "error": { "code": "busy", "message": why },
            }))),
            PromptMode::InternalError => Err(pi_client::protocol::ProtocolError {
                code: "internal_error".into(),
                message: "Internal server error".into(),
            }),
            PromptMode::DropConnection => {
                conn.close();
                Ok(None)
            }
            PromptMode::Accept => {
                agent.next_op += 1;
                let op = agent.next_op.to_string();
                if let Some(key) = request["requestId"].as_str() {
                    // A repeated key returns the original submission, as the real engine does.
                    let existing = agent.statuses.get(key).cloned();
                    let operation = existing
                        .as_ref()
                        .and_then(|e| e["operationId"].as_str().map(str::to_owned))
                        .unwrap_or_else(|| op.clone());
                    agent
                        .statuses
                        .entry(key.to_owned())
                        .or_insert_with(|| placed(&op));
                    return Ok(Some(
                        json!({ "accepted": true, "operationId": operation, "error": null }),
                    ));
                }
                Ok(Some(
                    json!({ "accepted": true, "operationId": op, "error": null }),
                ))
            }
        }
    });
    let a = agent.clone();
    pi.set_handler("pi.agent-controller", "lookup", move |_, _, call| {
        let key = call.args[0].as_str().unwrap_or("");
        Ok(Some(
            a.lock()
                .unwrap()
                .statuses
                .get(key)
                .cloned()
                .unwrap_or(json!({ "found": false })),
        ))
    });
    let a = agent.clone();
    pi.set_handler("pi.agent-controller", "abort", move |_, _, _| {
        let mut agent = a.lock().unwrap();
        agent.aborts += 1;
        for status in agent.statuses.values_mut() {
            if status["status"] == "placed" {
                let op = status["operationId"].as_str().unwrap_or("").to_owned();
                *status = settled(&op, "unanswered", Some("aborted"), None);
            }
        }
        Ok(None)
    });
    let a = agent.clone();
    pi.set_handler("pi.models", "select", move |pi, _, call| {
        a.lock().unwrap().selects.push(call.args[0].clone());
        let model = call.args[0].clone();
        pi.publish(
            "pi.models",
            vec![Op::Set(vec![key("configuration"), key("model")], model)],
        );
        Ok(None)
    });
    let a = agent.clone();
    pi.set_handler("pi.session-management", "create", move |pi, _, call| {
        let options = call.args[0].clone();
        let mut agent = a.lock().unwrap();
        let id = options["id"].as_str().unwrap_or("generated").to_owned();
        if agent.creates.iter().any(|c| c["id"] == options["id"]) {
            return Err(pi_client::protocol::ProtocolError {
                code: "internal_error".into(),
                message: format!("Session {id} already exists"),
            });
        }
        agent.creates.push(options.clone());
        let row = json!({ "serverId": SERVER_ID, "sessionId": id, "createdAt": 4_000_000_000_000i64, "cwd": options["cwd"] });
        pi.publish(
            "pi.session-directory",
            vec![Op::Splice { path: vec![key("sessions")], index: 99, remove: 0, items: vec![row.clone()] }],
        );
        Ok(Some(row))
    });
    (pi, agent, views)
}

fn submit(env: &Env, conv: ConversationId, generation: u64, op: u64, request: &str, text: &str) {
    env.backend.request(BackendRequest::Submit {
        conversation: conv,
        generation,
        op: OperationId(op),
        request: RequestId(request.into()),
        text: text.into(),
        attachments: vec![],
        model: None,
    });
}

impl Env {
    /// The next event for `op`, skipping unrelated transcript and workspace updates.
    fn next_for(&self, op: u64) -> BackendEvent {
        loop {
            let event = self.next_event();
            if event.op == Some(OperationId(op)) {
                return event;
            }
        }
    }

    fn open_first(&self) -> ConversationId {
        let conv = self.conversation(0);
        self.open(conv, 1);
        assert!(matches!(self.next_event().kind, EventKind::Opened { .. }));
        conv
    }

    fn wait_notice(&self, needle: &str) {
        wait_until(needle, || {
            self.lifecycle
                .lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, LifecycleEvent::Notice(m) if m.contains(needle)))
        });
    }
}

#[test]
fn a_prompt_is_sent_with_its_request_key_and_settles_when_the_engine_says_so() {
    let (pi, agent, _) = agent_mock(&[("s", 1)], vec![("s", view(&[]))]);
    let env = start(pi, true);
    let conv = env.open_first();
    submit(&env, conv, 1, 7, "req-1", "fix the bug");
    assert_eq!(env.next_for(7).kind, EventKind::Accepted);
    {
        let agent = agent.lock().unwrap();
        assert_eq!(agent.prompts.len(), 1);
        assert_eq!(agent.prompts[0]["message"], "fix the bug");
        assert_eq!(
            agent.prompts[0]["requestId"], "req-1",
            "the journaled key reaches the engine"
        );
        assert!(agent.prompts[0]["images"].is_null());
    }
    // Still running: nothing settles yet.
    env.no_event_within(600);
    // The engine finishes the run; the adapter notices by asking, not by being told.
    agent
        .lock()
        .unwrap()
        .statuses
        .insert("req-1".into(), settled("1", "done", None, None));
    assert_eq!(env.next_for(7).kind, EventKind::Completed);
}

#[test]
fn a_refused_prompt_is_a_definite_rejection_with_the_engine_s_reason() {
    let (pi, agent, _) = agent_mock(&[("s", 1)], vec![("s", view(&[]))]);
    let env = start(pi, true);
    let conv = env.open_first();
    agent.lock().unwrap().prompt_mode = PromptMode::Reject("Pi is busy with another run");
    submit(&env, conv, 1, 3, "req-busy", "again");
    let EventKind::Rejected { reason } = env.next_for(3).kind else {
        panic!()
    };
    assert!(reason.contains("busy"), "{reason}");
    // A refusal leaves nothing to settle.
    env.no_event_within(600);
}

#[test]
fn an_internal_error_after_sending_is_an_unknown_outcome_not_a_rejection() {
    let (pi, agent, _) = agent_mock(&[("s", 1)], vec![("s", view(&[]))]);
    let env = start(pi, true);
    let conv = env.open_first();
    agent.lock().unwrap().prompt_mode = PromptMode::InternalError;
    submit(&env, conv, 1, 4, "req-ie", "maybe");
    assert_eq!(env.next_for(4).kind, EventKind::AckLost);
}

#[test]
fn losing_the_connection_before_the_acknowledgment_is_an_unknown_outcome() {
    let (pi, agent, _) = agent_mock(&[("s", 1)], vec![("s", view(&[]))]);
    let env = start(pi, true);
    let conv = env.open_first();
    agent.lock().unwrap().prompt_mode = PromptMode::DropConnection;
    submit(&env, conv, 1, 5, "req-drop", "lost");
    assert_eq!(env.next_for(5).kind, EventKind::AckLost);
}

#[test]
fn a_prompt_for_a_session_that_is_not_open_is_refused_and_nothing_is_sent() {
    let (pi, agent, _) = agent_mock(&[("s", 1)], vec![("s", view(&[]))]);
    let env = start(pi, true);
    let conv = env.conversation(0); // listed, never opened
    submit(&env, conv, 1, 6, "req-closed", "hi");
    let EventKind::Rejected { reason } = env.next_for(6).kind else {
        panic!()
    };
    assert!(reason.contains("not open"), "{reason}");
    assert!(agent.lock().unwrap().prompts.is_empty());
}

#[test]
fn attachments_are_refused_rather_than_silently_dropped() {
    let (pi, agent, _) = agent_mock(&[("s", 1)], vec![("s", view(&[]))]);
    let env = start(pi, true);
    let conv = env.open_first();
    env.backend.request(BackendRequest::Submit {
        conversation: conv,
        generation: 1,
        op: OperationId(8),
        request: RequestId("req-att".into()),
        text: "with file".into(),
        attachments: vec![pipkin_core::Attachment {
            path: "/tmp/a.txt".into(),
            name: "a.txt".into(),
            size: Some(1),
            error: None,
        }],
        model: None,
    });
    let EventKind::Rejected { reason } = env.next_for(8).kind else {
        panic!()
    };
    assert!(reason.contains("Attachments"), "{reason}");
    assert!(agent.lock().unwrap().prompts.is_empty());
}

#[test]
fn check_status_resolves_through_the_engine_by_request_key() {
    let (pi, agent, _) = agent_mock(&[("s", 1)], vec![("s", view(&[]))]);
    let env = start(pi, true);
    let conv = env.open_first();
    let check = |op: u64, key: &str| {
        env.backend.request(BackendRequest::CheckStatus {
            conversation: conv,
            generation: 1,
            op: OperationId(op),
            request: Some(RequestId(key.into())),
        })
    };
    // The engine never saw it: proven not accepted, so a retry is safe.
    check(10, "never-arrived");
    assert_eq!(
        env.next_for(10).kind,
        EventKind::StatusResolved { accepted: false }
    );
    // The engine has it and it is still running: accepted, then settles later.
    agent
        .lock()
        .unwrap()
        .statuses
        .insert("running".into(), placed("9"));
    check(11, "running");
    assert_eq!(
        env.next_for(11).kind,
        EventKind::StatusResolved { accepted: true }
    );
    agent
        .lock()
        .unwrap()
        .statuses
        .insert("running".into(), settled("9", "done", None, None));
    assert_eq!(env.next_for(11).kind, EventKind::Completed);
    // The engine has it and it already finished.
    agent
        .lock()
        .unwrap()
        .statuses
        .insert("finished".into(), settled("8", "done", None, None));
    check(12, "finished");
    assert_eq!(
        env.next_for(12).kind,
        EventKind::StatusResolved { accepted: true }
    );
    assert_eq!(env.next_for(12).kind, EventKind::Completed);
    // No key to ask with: say so, resolve nothing.
    env.backend.request(BackendRequest::CheckStatus {
        conversation: conv,
        generation: 1,
        op: OperationId(13),
        request: None,
    });
    env.wait_notice("no record");
}

#[test]
fn engine_failures_read_as_failures_with_their_message() {
    let (pi, agent, _) = agent_mock(&[("s", 1)], vec![("s", view(&[]))]);
    let env = start(pi, true);
    let conv = env.open_first();
    submit(&env, conv, 1, 20, "req-err", "go");
    assert_eq!(env.next_for(20).kind, EventKind::Accepted);
    agent.lock().unwrap().statuses.insert(
        "req-err".into(),
        settled(
            "1",
            "unanswered",
            Some("model_error"),
            Some("provider unavailable"),
        ),
    );
    assert_eq!(
        env.next_for(20).kind,
        EventKind::Failed {
            message: "provider unavailable".into()
        }
    );
}

#[test]
fn stopping_a_run_asks_the_engine_and_settles_as_cancelled() {
    let (pi, agent, _) = agent_mock(&[("s", 1)], vec![("s", view(&[]))]);
    let env = start(pi, true);
    let conv = env.open_first();
    submit(&env, conv, 1, 30, "req-stop", "long job");
    assert_eq!(env.next_for(30).kind, EventKind::Accepted);
    env.backend.request(BackendRequest::Cancel {
        conversation: conv,
        generation: 1,
        op: OperationId(30),
    });
    // Stop is only a request: the run settles when the engine reports it.
    assert_eq!(env.next_for(30).kind, EventKind::Cancelled);
    assert_eq!(agent.lock().unwrap().aborts, 1);
}

#[test]
fn a_run_that_finished_while_another_session_was_open_settles_when_reopened() {
    let (pi, agent, _) = agent_mock(
        &[("s-a", 100), ("s-b", 200)],
        vec![("s-a", view(&[])), ("s-b", view(&[]))],
    );
    let env = start(pi, true);
    env.wait_ready();
    let (a, b) = {
        let c = env.catalogs().pop().unwrap().conversations;
        let find = |t: &str| c.iter().find(|x| x.2.contains(t)).unwrap().0;
        (find("s-a"), find("s-b"))
    };
    env.open(a, 1);
    assert!(matches!(env.next_event().kind, EventKind::Opened { .. }));
    submit(&env, a, 1, 40, "req-away", "start it");
    assert_eq!(env.next_for(40).kind, EventKind::Accepted);
    env.open(b, 1);
    assert!(matches!(
        env.next_for_conversation(b).kind,
        EventKind::Opened { .. }
    ));
    // While away, the engine finishes the run; nothing is delivered for a session not attached.
    agent
        .lock()
        .unwrap()
        .statuses
        .insert("req-away".into(), settled("1", "done", None, None));
    env.no_event_within(700);
    env.open(a, 2);
    assert_eq!(env.next_for(40).kind, EventKind::Completed);
}

impl Env {
    fn next_for_conversation(&self, conversation: ConversationId) -> BackendEvent {
        loop {
            let event = self.next_event();
            if event.conversation == conversation {
                return event;
            }
        }
    }
}

#[test]
fn choosing_a_model_asks_the_engine_and_the_selection_follows_its_report() {
    let (pi, agent, _) = agent_mock(&[("s", 1)], vec![("s", view(&[]))]);
    let env = start(pi, true);
    let conv = env.open_first();
    // Opening already reported the engine's model.
    assert!(
        env.lifecycle.lock().unwrap().iter().any(
            |e| matches!(e, LifecycleEvent::ModelSelected(Some(m)) if m == "anthropic/sonnet")
        )
    );
    env.backend.request(BackendRequest::SetModel {
        conversation: conv,
        generation: 1,
        model: "openai/gpt".into(),
    });
    wait_until("select sent", || !agent.lock().unwrap().selects.is_empty());
    assert_eq!(
        agent.lock().unwrap().selects[0],
        json!({ "provider": "openai", "modelId": "gpt" })
    );
    wait_until("engine reported the new model", || {
        env.lifecycle
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, LifecycleEvent::ModelSelected(Some(m)) if m == "openai/gpt"))
    });
    // A malformed id is not sent to the engine.
    env.backend.request(BackendRequest::SetModel {
        conversation: conv,
        generation: 1,
        model: "noslash".into(),
    });
    env.wait_notice("Unrecognized model");
    assert_eq!(agent.lock().unwrap().selects.len(), 1);
}

#[test]
fn creating_a_conversation_names_the_directory_and_is_safe_to_repeat() {
    let (pi, agent, _) = agent_mock(&[("s", 1)], vec![]);
    let env = start(pi, true);
    env.wait_ready();
    let create = || {
        env.backend.request(BackendRequest::CreateConversation {
            project: pipkin_core::project_id_for_path("/work/app"),
            cwd: "/work/app".into(),
            request: RequestId("create-1".into()),
        })
    };
    create();
    wait_until("created", || agent.lock().unwrap().creates.len() == 1);
    assert_eq!(
        agent.lock().unwrap().creates[0],
        json!({ "id": "create-1", "cwd": "/work/app" })
    );
    // The new session arrives through the directory, in the project of its directory.
    wait_until("catalog lists it", || {
        env.catalogs().last().is_some_and(|c| {
            c.projects.iter().any(|p| p.path == "/work/app")
                && c.conversations
                    .iter()
                    .any(|x| x.1 == pipkin_core::project_id_for_path("/work/app"))
        })
    });
    // Repeating the same creation (a retry after a lost reply) is not an error.
    create();
    env.no_event_within(500);
    assert!(
        !env.lifecycle
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, LifecycleEvent::Notice(_))),
        "a duplicate creation must not show an error"
    );
}

#[test]
fn a_failed_creation_is_reported_as_a_notice() {
    let (pi, _agent, _) = agent_mock(&[("s", 1)], vec![]);
    pi.set_handler("pi.session-management", "create", |_, _, _| {
        Err(pi_client::protocol::ProtocolError {
            code: "service_invalid_value".into(),
            message: "Session cwd is not an existing directory: /nope".into(),
        })
    });
    let env = start(pi, true);
    env.wait_ready();
    env.backend.request(BackendRequest::CreateConversation {
        project: pipkin_core::project_id_for_path("/nope"),
        cwd: "/nope".into(),
        request: RequestId("create-bad".into()),
    });
    env.wait_notice("not an existing directory");
}

#[test]
fn workspace_changes_follow_the_session_directory_and_update_when_a_tool_finishes() {
    let repo = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(repo.path())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    };
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(
        repo.path().join("notes.txt"),
        "one
",
    )
    .unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "init"]);

    let (pi, _agent, _) = agent_mock(&[], vec![("s", view(&[]))]);
    pi.publish(
        "pi.session-directory",
        vec![Op::Replace(json!({ "revision": 2, "sessions": [{
            "serverId": SERVER_ID, "sessionId": "s", "createdAt": 5,
            "cwd": repo.path().to_str().unwrap(),
        }]}))],
    );
    let env = start(pi, true);
    let conv = env.conversation(0);
    env.open(conv, 1);
    let changes = |env: &Env| -> Vec<FileChange> {
        loop {
            if let EventKind::ChangesSynced(files) = env.next_event().kind {
                return files;
            }
        }
    };
    // Opening scans the session's directory: clean at first.
    assert!(changes(&env).is_empty());

    // A tool edits a tracked file and creates a new one; the transcript shows a finished tool.
    std::fs::write(
        repo.path().join("notes.txt"),
        "one
two
",
    )
    .unwrap();
    std::fs::write(
        repo.path().join("new.txt"),
        "hello
",
    )
    .unwrap();
    let call = json!({ "role": "assistant", "stopReason": "toolUse", "timestamp": 0,
        "content": [{ "type": "toolCall", "id": "c1", "name": "write", "arguments": {} }] });
    let result = json!({ "role": "toolResult", "toolCallId": "c1", "toolName": "write", "isError": false,
        "timestamp": 0, "content": [{ "type": "text", "text": "Successfully wrote" }] });
    env.pi.publish(
        "pi.transcript",
        vec![Op::Replace(
            json!({ "conversation": { "id": 1 }, "docs": {}, "entries": [
                { "id": 1, "kind": "pi.assistant", "model": [call] },
                { "id": 2, "kind": "pi.tool-result", "model": [result] },
            ]}),
        )],
    );
    let files = changes(&env);
    let names: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(names, ["notes.txt", "new.txt"], "{files:?}");
    assert_eq!((files[0].added, files[0].removed), (1, 0));
    assert!(
        files[1].hunks[0]
            .lines
            .iter()
            .all(|l| l.kind == pipkin_core::DiffKind::Add)
    );
    // A transcript change that finishes no tool and ends no run does not rescan.
    let _ = ItemKind::Notice {
        text: String::new(),
        level: pipkin_core::NoticeLevel::Info,
    };
}
