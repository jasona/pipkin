//! Conformance against a REAL Pi experimental server. Opt in: these tests are ignored unless
//! asked for, and need a throwaway server started with `scripts/pi-test-server.sh`:
//!
//! ```text
//! scripts/pi-test-server.sh ROOT &                       # needs `npm ci` in the Pi checkout
//! eval "$(scripts/pi-test-server.sh ROOT --print-env)"
//! cargo test -p pipkin-app real_pi -- --ignored --nocapture
//! ```
//!
//! They create sessions on that throwaway server only. No provider credentials are involved.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use pi_client::chord::{Mode, ServiceCall};
use pi_client::client::ClientOptions;
use pi_client::protocol::RpcTarget;
use pi_client::unix;
use pipkin_core::{Backend, BackendEvent, BackendRequest, Connection, EventKind, LifecycleEvent};
use serde_json::{Value, json};

use super::{PiBackend, PiConfig};

const WAIT: Duration = Duration::from_secs(30);

fn environment() -> (PathBuf, String) {
    let dir = std::env::var("PIPKIN_REAL_PI_DIR")
        .expect("PIPKIN_REAL_PI_DIR: start scripts/pi-test-server.sh and eval its --print-env");
    let id = std::env::var("PIPKIN_REAL_PI_SERVER_ID").expect("PIPKIN_REAL_PI_SERVER_ID");
    (PathBuf::from(dir), id)
}

fn server(id: &str) -> RpcTarget {
    RpcTarget::Server {
        server_id: id.into(),
    }
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "needs a real Pi server: see the module docs"]
fn real_server_speaks_the_protocol_pipkin_implements() {
    let (dir, id) = environment();
    let (client, _events) = unix::connect(&dir.join(format!("{id}.sock")), ClientOptions::new(&id))
        .expect("trusted handshake");
    assert!(client.is_connected());

    // The catalogue is the capability inventory: everything M1 binds must be present.
    let catalogue = client.catalogue(&server(&id), WAIT).expect("catalogue");
    let ids: Vec<&str> = catalogue.iter().map(|e| e.service_id.as_str()).collect();
    println!("server services: {ids:?}");
    for required in ["pi.session-directory", "pi.session-management"] {
        assert!(ids.contains(&required), "missing {required}: {ids:?}");
    }

    // Hydrate the directory: a real snapshot through the real Delta codec.
    let directory = client
        .subscribe(&server(&id), "pi.session-directory", Mode::Singleton, WAIT)
        .expect("directory snapshot");
    let state = directory
        .read(|r| r.state("state").cloned())
        .flatten()
        .expect("directory state");
    println!("directory state: {state}");
    assert!(state.get("sessions").is_some_and(Value::is_array));
}

#[test]
#[ignore = "needs a real Pi server: see the module docs"]
fn real_server_session_lifecycle_through_the_adapter() {
    let (dir, id) = environment();
    // Create a session directly (the adapter is read-only this milestone) and let the adapter
    // discover it through the replicated directory.
    let (client, _events) =
        unix::connect(&dir.join(format!("{id}.sock")), ClientOptions::new(&id)).expect("handshake");
    let created = client
        .request(
            &server(&id),
            &ServiceCall::new("pi.session-management", "create", vec![json!({})]),
        )
        .expect("create sent")
        .wait_timeout(WAIT)
        .expect("create result")
        .expect("created session summary");
    println!("created session: {created}");
    let session_id = created["sessionId"].as_str().expect("sessionId").to_owned();

    let (tx, events) = async_channel::unbounded::<BackendEvent>();
    let mut config = PiConfig::new(dir);
    config.server_id = Some(id);
    let backend = PiBackend::new(tx, config);
    let lifecycle = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = {
        let lifecycle = lifecycle.clone();
        Box::new(move |e| lifecycle.lock().unwrap().push(e))
    };
    backend.start(sink);
    wait_until("ready", || {
        lifecycle
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, LifecycleEvent::Connection(Connection::Ready)))
    });
    let conversation = {
        let guard = lifecycle.lock().unwrap();
        let catalog = guard
            .iter()
            .rev()
            .find_map(|e| {
                if let LifecycleEvent::Catalog(b) = e {
                    Some(b.clone())
                } else {
                    None
                }
            })
            .expect("a catalog");
        println!("conversations: {:?}", catalog.conversations);
        catalog
            .conversations
            .iter()
            .find(|c| c.2.contains(&session_id[..8.min(session_id.len())]))
            .expect("the created session is listed")
            .0
    };

    // Open it: attach, subscribe to Transcript and Models, map the real ConversationView.
    backend.request(BackendRequest::Open {
        conversation,
        generation: 1,
    });
    let deadline = Instant::now() + WAIT;
    let opened = loop {
        if let Ok(e) = events.try_recv() {
            break e;
        }
        assert!(Instant::now() < deadline, "no open result");
        std::thread::sleep(Duration::from_millis(20));
    };
    println!("open result: {:?}", opened.kind);
    match opened.kind {
        EventKind::Opened { items, .. } => println!("opened with {} items", items.len()),
        other => panic!("the real session did not open: {other:?}"),
    }
    backend.shutdown();
}
