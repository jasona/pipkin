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
fn prompts_are_refused_honestly_not_simulated() {
    let env = start(mock(&[("s", 1)], vec![]), true);
    let conv = env.conversation(0);
    env.backend.request(BackendRequest::Submit {
        conversation: conv,
        generation: 1,
        op: OperationId(4),
        request: RequestId("r".into()),
        text: "do it".into(),
        attachments: vec![],
        model: None,
    });
    let event = env.next_event();
    assert_eq!(event.op, Some(OperationId(4)));
    let EventKind::Rejected { reason } = event.kind else {
        panic!("{event:?}")
    };
    assert!(reason.contains("not available yet"), "{reason}");
    // Nothing was sent to the engine.
    assert!(
        env.pi
            .requests()
            .iter()
            .all(|(_, call)| call.service_id != "pi.agent-controller")
    );
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
