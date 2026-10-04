//! The Pi engine adapter: maps Pi's replicated services onto Pipkin's core.
//!
//! One worker thread owns the connection. It discovers a trusted local server, mirrors the
//! session directory into the catalogue, and opens a conversation by attaching its session and
//! subscribing to its `Transcript` and `Models` services. Every update is checked against the
//! live subscription, so traffic from an attachment the user has moved away from cannot reach
//! the screen. This slice is read-only: prompts are refused honestly, never simulated.
//!
//! The adapter never replays a request after a disconnect. It reconnects, resubscribes and
//! refreshes what is shown; anything that mutates stays the user's explicit decision.

pub mod session;
pub mod transcript;

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use pi_client::Error;
use pi_client::chord::{Mode, ServiceCall};
use pi_client::client::{Client, ClientEvent, ClientOptions, Subscription};
use pi_client::protocol::RpcTarget;
use pi_client::unix::{self, ServerRoute};
use pipkin_core::{
    Backend, BackendEvent, BackendRequest, Connection, ConversationId, EventKind, LifecycleEvent,
    LifecycleSink, ModelInfo,
};
use serde_json::{Value, json};

use self::session::Session;

const CALL_TIMEOUT: Duration = Duration::from_secs(15);
const ATTACH_TIMEOUT: Duration = Duration::from_secs(5);
/// Messages handled per wakeup before remapping, so a burst becomes one refresh.
const BATCH: usize = 256;
/// Waits before each retry of an attach that failed with `internal_error`; at most this many
/// retries.
const ATTACH_RETRY_DELAYS: [Duration; 4] = [
    Duration::from_millis(150),
    Duration::from_millis(400),
    Duration::from_millis(1000),
    Duration::from_millis(2000),
];
/// A subscription that keeps failing is not worth patching; resync through a reconnect.
const MAX_RESUBSCRIBES: u32 = 3;

#[derive(Clone, Debug)]
pub struct PiConfig {
    /// The Pi server profile directory (sockets and `default-server-id`).
    pub directory: PathBuf,
    /// Use this logical server instead of discovering one.
    pub server_id: Option<String>,
    /// Delay between connection attempts while no server is reachable.
    pub retry_delay: Duration,
}

impl PiConfig {
    pub fn new(directory: PathBuf) -> Self {
        PiConfig {
            directory,
            server_id: None,
            retry_delay: Duration::from_secs(3),
        }
    }
}

/// Where Pi keeps its server profile unless told otherwise: `PI_SERVER_DIR` or `~/.pi/server`.
pub fn default_directory() -> PathBuf {
    if let Some(dir) = std::env::var_os("PI_SERVER_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".pi").join("server")
}

enum Msg {
    Request(BackendRequest),
    Client(ClientEvent),
    Shutdown,
}

pub struct PiBackend {
    config: PiConfig,
    events: async_channel::Sender<BackendEvent>,
    tx: Sender<Msg>,
    rx: Mutex<Option<Receiver<Msg>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl PiBackend {
    pub fn new(events: async_channel::Sender<BackendEvent>, config: PiConfig) -> Self {
        let (tx, rx) = mpsc::channel();
        PiBackend {
            config,
            events,
            tx,
            rx: Mutex::new(Some(rx)),
            worker: Mutex::new(None),
        }
    }
}

impl Backend for PiBackend {
    fn start(&self, sink: LifecycleSink) {
        let Some(rx) = lock(&self.rx).take() else {
            return; // already started
        };
        let worker = Worker {
            config: self.config.clone(),
            events: self.events.clone(),
            sink,
            tx: self.tx.clone(),
            rx,
            last_open: None,
            ever_connected: false,
        };
        let handle = thread::Builder::new()
            .name("pi-backend".into())
            .spawn(move || worker.run())
            .expect("spawn pi backend");
        *lock(&self.worker) = Some(handle);
    }

    fn request(&self, request: BackendRequest) {
        let _ = self.tx.send(Msg::Request(request));
    }

    fn shutdown(&self) {
        let _ = self.tx.send(Msg::Shutdown);
        if let Some(handle) = lock(&self.worker).take() {
            let deadline = Instant::now() + Duration::from_secs(2);
            while !handle.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            // A worker stuck in a call is abandoned rather than blocking application exit.
        }
    }
}

struct Worker {
    config: PiConfig,
    events: async_channel::Sender<BackendEvent>,
    sink: LifecycleSink,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    /// The conversation most recently opened, refreshed after a reconnect.
    last_open: Option<(ConversationId, u64)>,
    ever_connected: bool,
}

enum Outcome {
    Shutdown,
    Retry,
}

impl Worker {
    fn run(mut self) {
        loop {
            match self.connect_and_serve() {
                Outcome::Shutdown => return,
                Outcome::Retry => {
                    if self.wait_to_retry() {
                        return;
                    }
                }
            }
        }
    }

    fn now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64)
    }

    fn report(&self, connection: Connection) {
        (self.sink)(LifecycleEvent::Connection(connection));
    }

    fn emit(&self, conversation: ConversationId, generation: u64, kind: EventKind) {
        let _ = self.events.send_blocking(BackendEvent {
            conversation,
            generation,
            op: None,
            kind,
        });
    }

    /// Sleep out the retry delay, still answering requests so nothing waits on a dead link.
    /// Returns true on shutdown.
    fn wait_to_retry(&mut self) -> bool {
        let deadline = Instant::now() + self.config.retry_delay;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(remaining) {
                Err(mpsc::RecvTimeoutError::Timeout) => return false,
                Err(mpsc::RecvTimeoutError::Disconnected) | Ok(Msg::Shutdown) => return true,
                Ok(Msg::Request(request)) => self.refuse_offline(request),
                Ok(Msg::Client(_)) => {} // from a connection that is already gone
            }
        }
    }

    fn refuse_offline(&self, request: BackendRequest) {
        match request {
            BackendRequest::Open {
                conversation,
                generation,
            } => self.emit(
                conversation,
                generation,
                EventKind::OpenFailed {
                    message: "Not connected to the Pi engine.".into(),
                },
            ),
            other => self.refuse_unsupported(other, "Not connected to the Pi engine."),
        }
    }

    /// Answer a mutating request truthfully instead of leaving it to hang or pretending.
    fn refuse_unsupported(&self, request: BackendRequest, reason: &str) {
        match request {
            BackendRequest::Submit {
                conversation,
                generation,
                op,
                ..
            } => {
                let _ = self.events.send_blocking(BackendEvent {
                    conversation,
                    generation,
                    op: Some(op),
                    kind: EventKind::Rejected {
                        reason: reason.into(),
                    },
                });
            }
            BackendRequest::LoadOlder {
                conversation,
                generation,
                ..
            } => {
                self.emit(
                    conversation,
                    generation,
                    EventKind::OlderPage {
                        items: vec![],
                        has_older: false,
                    },
                );
            }
            other => log::warn!("not supported by the Pi adapter yet: {other:?}"),
        }
    }

    fn resolve(&self) -> Result<ServerRoute, Connection> {
        let dir = &self.config.directory;
        if let Some(id) = &self.config.server_id {
            return Ok(ServerRoute {
                server_id: id.clone(),
                path: dir.join(format!("{id}.sock")),
            });
        }
        let probe = ClientOptions::new("00000000-0000-4000-8000-000000000000");
        let found = unix::discover(dir, &probe).map_err(|e| Connection::Offline(e.to_string()))?;
        let mut routes = found.routes;
        match routes.len() {
            // A server may be running behind permissions we refuse; say so instead of "none".
            0 if !found.untrusted.is_empty() => {
                Err(Connection::Failed(found.untrusted[0].1.to_string()))
            }
            0 => Err(Connection::Offline(format!(
                "no Pi server is running in {}. Start one with the Pi experimental server.",
                dir.display()
            ))),
            1 => Ok(routes.remove(0)),
            _ => {
                // Prefer the server Pi itself would pick.
                let default =
                    std::fs::read_to_string(dir.join("default-server-id")).unwrap_or_default();
                match routes.iter().position(|r| r.server_id == default.trim()) {
                    Some(i) => Ok(routes.remove(i)),
                    None => Err(Connection::Failed(format!(
                        "{} Pi servers are running in {}; choose one with --pi-server-id.",
                        routes.len(),
                        dir.display()
                    ))),
                }
            }
        }
    }

    fn connect_and_serve(&mut self) -> Outcome {
        self.report(if self.ever_connected {
            Connection::Reconnecting
        } else {
            Connection::Connecting
        });
        let route = match self.resolve() {
            Ok(route) => route,
            Err(connection) => {
                self.report(connection);
                return Outcome::Retry;
            }
        };
        let (client, events) =
            match unix::connect(&route.path, ClientOptions::new(&route.server_id)) {
                Ok(pair) => pair,
                Err(error) => {
                    self.report(match &error {
                        Error::Untrusted(_) => Connection::Failed(error.to_string()),
                        Error::Server { code, .. } if code == "version" => {
                            Connection::Incompatible(error.to_string())
                        }
                        _ => Connection::Offline(error.to_string()),
                    });
                    return Outcome::Retry;
                }
            };
        // Forward client events into the one queue this thread reads.
        let forward_tx = self.tx.clone();
        thread::spawn(move || {
            while let Ok(event) = events.recv_blocking() {
                if forward_tx.send(Msg::Client(event)).is_err() {
                    break;
                }
            }
        });
        let mut live = match Live::start(&client, &route, &self.config) {
            Ok(live) => live,
            Err(connection) => {
                client.disconnect();
                self.report(connection);
                return Outcome::Retry;
            }
        };
        self.ever_connected = true;
        (self.sink)(LifecycleEvent::Catalog(live.catalog()));
        self.report(Connection::Ready);
        // After a reconnect, refresh what is on screen instead of leaving it stale.
        if let Some((conversation, generation)) = self.last_open {
            live.open(self, conversation, generation, false);
        }
        self.serve(&mut live)
    }

    fn serve(&mut self, live: &mut Live) -> Outcome {
        let mut deferred: VecDeque<Msg> = VecDeque::new();
        loop {
            let first = match deferred.pop_front() {
                Some(m) => m,
                None => match self.rx.recv() {
                    Ok(m) => m,
                    Err(_) => return Outcome::Shutdown,
                },
            };
            let mut batch = vec![first];
            while batch.len() < BATCH {
                match self.rx.try_recv() {
                    Ok(m) => batch.push(m),
                    Err(_) => break,
                }
            }
            let mut dirty = Dirty::default();
            for message in batch {
                match message {
                    Msg::Shutdown => {
                        live.client.disconnect();
                        return Outcome::Shutdown;
                    }
                    Msg::Request(request) => live.handle_request(self, request),
                    Msg::Client(ClientEvent::Disconnected(error)) => {
                        log::warn!("Pi connection lost: {error}");
                        return Outcome::Retry;
                    }
                    Msg::Client(event) => {
                        if let Some(outcome) = live.handle_event(self, event, &mut dirty) {
                            return outcome;
                        }
                    }
                }
            }
            live.flush(self, dirty);
        }
    }
}

#[derive(Default)]
struct Dirty {
    directory: bool,
    transcript: bool,
    models: bool,
}

struct Current {
    conversation: ConversationId,
    generation: u64,
    transcript: Subscription,
    models: Option<Subscription>,
}

struct Live {
    client: Client,
    location: String,
    dir: Subscription,
    sessions: Vec<(ConversationId, Session)>,
    index: HashMap<ConversationId, String>,
    models: Vec<ModelInfo>,
    current: Option<Current>,
    resubscribes: u32,
}

fn server_target(route: &ServerRoute) -> RpcTarget {
    RpcTarget::Server {
        server_id: route.server_id.clone(),
    }
}

impl Live {
    fn server(&self) -> RpcTarget {
        RpcTarget::Server {
            server_id: self.client.server_id().to_owned(),
        }
    }

    fn start(client: &Client, route: &ServerRoute, config: &PiConfig) -> Result<Live, Connection> {
        let target = server_target(route);
        // The services this milestone needs. A missing one is an incompatible engine, which no
        // amount of retrying fixes, so say exactly what is missing.
        let catalogue = client
            .catalogue(&target, CALL_TIMEOUT)
            .map_err(|e| Connection::Offline(e.to_string()))?;
        for required in ["pi.session-directory", "pi.session-management"] {
            if !catalogue.iter().any(|e| e.service_id == required) {
                return Err(Connection::Incompatible(format!(
                    "the engine does not provide {required}"
                )));
            }
        }
        let dir = client
            .subscribe(
                &target,
                "pi.session-directory",
                Mode::Singleton,
                CALL_TIMEOUT,
            )
            .map_err(|e| Connection::Offline(e.to_string()))?;
        let mut live = Live {
            client: client.clone(),
            location: display_path(&config.directory),
            dir,
            sessions: Vec::new(),
            index: HashMap::new(),
            models: Vec::new(),
            current: None,
            resubscribes: 0,
        };
        live.refresh_directory();
        Ok(live)
    }

    fn refresh_directory(&mut self) {
        let state = self
            .dir
            .read(|r| r.state("state").cloned())
            .flatten()
            .unwrap_or(Value::Null);
        self.sessions = session::assign_ids(&session::parse_directory(&state));
        self.index = session::index(&self.sessions);
    }

    fn catalog(&self) -> pipkin_core::Bootstrap {
        session::catalog(
            &self.sessions,
            self.models.clone(),
            &self.location,
            Worker::now(),
        )
    }

    fn handle_request(&mut self, worker: &mut Worker, request: BackendRequest) {
        match request {
            BackendRequest::Open { conversation, generation } => {
                worker.last_open = Some((conversation, generation));
                self.open(worker, conversation, generation, true);
            }
            other => worker.refuse_unsupported(
                other,
                "Sending prompts is not available yet: this build can read Pi sessions but not run them.",
            ),
        }
    }

    /// Attach a session, subscribe to its services and show its state.
    fn open(
        &mut self,
        worker: &Worker,
        conversation: ConversationId,
        generation: u64,
        initial: bool,
    ) {
        let fail = |message: String| {
            if initial {
                worker.emit(conversation, generation, EventKind::OpenFailed { message });
            } else {
                log::warn!("could not refresh {conversation:?} after reconnecting: {message}");
            }
        };
        let Some(session_id) = self.index.get(&conversation).cloned() else {
            return fail("This session no longer exists on the Pi server.".into());
        };
        // Drop the previous attachment's subscriptions first; the client retires them anyway.
        self.current = None;
        let attach = ServiceCall::new("pi.session-management", "attach", vec![json!(session_id)]);
        // Pi's server can answer `internal_error` when a session is re-attached right after a
        // client switched away from it (observed against the real server: the session's worker
        // is still retiring). Attaching only navigates, it changes no session data, so a short
        // bounded retry is safe. Every other failure is final.
        let mut attempt = 0;
        loop {
            let result = self
                .client
                .request(&self.server(), &attach)
                .and_then(|pending| pending.wait_timeout(CALL_TIMEOUT));
            match result {
                Ok(_) => break,
                Err(Error::Server { ref code, .. })
                    if code == "internal_error" && attempt < ATTACH_RETRY_DELAYS.len() =>
                {
                    log::warn!(
                        "attach of {session_id} failed with internal_error; retrying (attempt {})",
                        attempt + 1
                    );
                    thread::sleep(ATTACH_RETRY_DELAYS[attempt]);
                    attempt += 1;
                }
                Err(error) => return fail(format!("Could not attach the session: {error}")),
            }
        }
        // The route arrives out of band, before or after the response.
        let deadline = Instant::now() + ATTACH_TIMEOUT;
        let target = loop {
            if let Some(a) = self.client.attachment()
                && a.session_id == session_id
            {
                break a.rpc();
            }
            if Instant::now() >= deadline {
                return fail("The Pi server did not publish the session attachment.".into());
            }
            thread::sleep(Duration::from_millis(10));
        };
        let transcript =
            match self
                .client
                .subscribe(&target, "pi.transcript", Mode::Singleton, CALL_TIMEOUT)
            {
                Ok(sub) => sub,
                Err(error) => return fail(format!("Could not read the session: {error}")),
            };
        let models = self
            .client
            .subscribe(&target, "pi.models", Mode::Singleton, CALL_TIMEOUT)
            .ok();
        let Some(view) = transcript.read(|r| r.state("state").cloned()).flatten() else {
            return fail("The session has no transcript.".into());
        };
        let mapped = transcript::map_view(&view);
        let kind = if initial {
            EventKind::Opened {
                items: mapped.items,
                has_older: false,
                changes: vec![],
            }
        } else {
            EventKind::Synced {
                items: mapped.items,
            }
        };
        worker.emit(conversation, generation, kind);
        if let Some(models) = &models {
            self.models = models
                .read(|r| r.state("state").cloned())
                .flatten()
                .map(|s| session::parse_models(&s))
                .unwrap_or_default();
            (worker.sink)(LifecycleEvent::Catalog(self.catalog()));
        }
        self.resubscribes = 0;
        self.current = Some(Current {
            conversation,
            generation,
            transcript,
            models,
        });
    }

    fn handle_event(
        &mut self,
        worker: &mut Worker,
        event: ClientEvent,
        dirty: &mut Dirty,
    ) -> Option<Outcome> {
        match event {
            ClientEvent::SubscriptionChanged { subscription, .. } => {
                if subscription == self.dir.id() {
                    dirty.directory = true;
                } else if let Some(current) = &self.current {
                    // Anything not from the live subscriptions is from an attachment we left.
                    if subscription == current.transcript.id() {
                        dirty.transcript = true;
                    } else if current
                        .models
                        .as_ref()
                        .is_some_and(|m| m.id() == subscription)
                    {
                        dirty.models = true;
                    }
                }
            }
            ClientEvent::SubscriptionFailed {
                subscription,
                error,
            } => {
                let is_current = self.current.as_ref().is_some_and(|c| {
                    c.transcript.id() == subscription
                        || c.models.as_ref().is_some_and(|m| m.id() == subscription)
                });
                if subscription == self.dir.id() || is_current {
                    log::warn!("subscription {subscription} failed: {error}");
                    self.resubscribes += 1;
                    if self.resubscribes > MAX_RESUBSCRIBES {
                        // Stop patching; a reconnect resynchronizes everything.
                        self.client.disconnect();
                        return Some(Outcome::Retry);
                    }
                    if subscription == self.dir.id() {
                        match self.client.subscribe(
                            &self.server(),
                            "pi.session-directory",
                            Mode::Singleton,
                            CALL_TIMEOUT,
                        ) {
                            Ok(dir) => {
                                self.dir = dir;
                                dirty.directory = true;
                            }
                            Err(_) => {
                                self.client.disconnect();
                                return Some(Outcome::Retry);
                            }
                        }
                    } else if let Some((conversation, generation)) = self
                        .current
                        .as_ref()
                        .map(|c| (c.conversation, c.generation))
                    {
                        self.open(worker, conversation, generation, false);
                    }
                }
            }
            ClientEvent::Attachment(_)
            | ClientEvent::SubscriptionRetired { .. }
            | ClientEvent::Disconnected(_) => {}
        }
        None
    }

    /// Turn the batch's changes into one refresh each.
    fn flush(&mut self, worker: &Worker, dirty: Dirty) {
        if dirty.directory {
            self.refresh_directory();
        }
        if dirty.models
            && let Some(current) = &self.current
            && let Some(models) = &current.models
            && let Some(state) = models.read(|r| r.state("state").cloned()).flatten()
        {
            self.models = session::parse_models(&state);
        }
        if dirty.directory || dirty.models {
            (worker.sink)(LifecycleEvent::Catalog(self.catalog()));
        }
        if dirty.transcript
            && let Some(current) = &self.current
            && let Some(view) = current
                .transcript
                .read(|r| r.state("state").cloned())
                .flatten()
        {
            let mapped = transcript::map_view(&view);
            worker.emit(
                current.conversation,
                current.generation,
                EventKind::Synced {
                    items: mapped.items,
                },
            );
        }
    }
}

fn display_path(path: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from)
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod real_pi;
