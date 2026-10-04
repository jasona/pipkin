//! The connection state machine: handshake, request correlation and cancellation, service
//! subscriptions with hydration buffering, and ordered attachment fencing.
//!
//! Transport-agnostic: it drives any `Read`/`Write` pair. One reader thread owns decoding and
//! all state transitions; API calls only register work and write frames. The client never
//! reconnects or replays on its own: after a disconnect every request fails, every
//! subscription is retired, and the caller decides what is safe to repeat.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use serde_json::Value;

use crate::chord::{self, CatalogueEntry, Change, Mode, Replica, ServiceCall, StateDecoder};
use crate::error::{Error, Result, validation};
use crate::frame::DEFAULT_MAX_FRAME_LENGTH;
use crate::protocol::{
    ClientMessage, PROTOCOL_VERSION, RpcTarget, ServerMessage, ServerMessageDecoder, SessionTarget,
    encode_client_message,
};

/// Updates buffered for a subscription before its snapshot arrives. A server that exceeds this
/// is misbehaving; failing the connection is safer than unbounded memory.
const MAX_QUEUED_UPDATES: usize = 10_000;

#[derive(Clone, Debug)]
pub struct ClientOptions {
    /// The logical server identity expected at the physical endpoint.
    pub server_id: String,
    pub max_frame_length: usize,
    pub handshake_timeout: Duration,
    /// Capacity of the event queue. A full queue backpressures the reader thread (and so the
    /// peer) rather than dropping an authoritative update.
    pub event_capacity: usize,
}

impl ClientOptions {
    pub fn new(server_id: impl Into<String>) -> Self {
        ClientOptions {
            server_id: server_id.into(),
            max_frame_length: DEFAULT_MAX_FRAME_LENGTH,
            handshake_timeout: Duration::from_secs(5),
            event_capacity: 4096,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ClientEvent {
    /// This presentation's selected Session route changed (or cleared).
    Attachment(Option<SessionTarget>),
    SubscriptionChanged {
        subscription: String,
        change: Change,
    },
    /// The subscription's replica is invalid (sequence gap, unapplicable op). It is retired;
    /// resubscribe to obtain a fresh snapshot.
    SubscriptionFailed {
        subscription: String,
        error: Error,
    },
    /// A Session-bound subscription ended because the attachment moved on.
    SubscriptionRetired {
        subscription: String,
    },
    Disconnected(Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Connecting,
    Connected,
    Disconnected,
}

struct PendingEntry {
    tx: SyncSender<Result<Option<Value>>>,
    subscription: Option<String>,
}

/// What a subscription's holder can read.
pub struct SubState {
    pub replica: Option<Replica>,
    pub error: Option<Error>,
    pub retired: bool,
}

struct Sub {
    target: RpcTarget,
    service_id: String,
    mode: Mode,
    decoder: StateDecoder,
    shared: Arc<Mutex<SubState>>,
    hydrated: bool,
    queued: Vec<Value>,
}

struct State {
    phase: Phase,
    hello_server_id: Option<String>,
    attachment: Option<SessionTarget>,
    next_request: u64,
    next_subscription: u64,
    pending: HashMap<String, PendingEntry>,
    subs: HashMap<String, Sub>,
    handshake: Option<SyncSender<Result<String>>>,
    disconnect: Option<Error>,
    disconnect_reported: bool,
    stale_updates_dropped: u64,
}

struct Inner {
    options: ClientOptions,
    state: Mutex<State>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    shutdown: Box<dyn Fn() + Send + Sync>,
    events: async_channel::Sender<ClientEvent>,
}

#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Client {
    /// Establish a connection over an already-connected byte stream and complete the handshake.
    /// `shutdown` must unblock a pending `read` (for sockets, `shutdown(Both)`).
    pub fn establish(
        read: impl Read + Send + 'static,
        write: impl Write + Send + 'static,
        shutdown: impl Fn() + Send + Sync + 'static,
        options: ClientOptions,
    ) -> Result<(Client, async_channel::Receiver<ClientEvent>)> {
        if !crate::protocol::is_server_id(&options.server_id) {
            return Err(validation("serverId must be a canonical lowercase UUIDv4"));
        }
        let (events, event_rx) = async_channel::bounded(options.event_capacity.max(1));
        let (hs_tx, hs_rx) = mpsc::sync_channel(1);
        let inner = Arc::new(Inner {
            state: Mutex::new(State {
                phase: Phase::Connecting,
                hello_server_id: None,
                attachment: None,
                next_request: 0,
                next_subscription: 0,
                pending: HashMap::new(),
                subs: HashMap::new(),
                handshake: Some(hs_tx),
                disconnect: None,
                disconnect_reported: false,
                stale_updates_dropped: 0,
            }),
            writer: Mutex::new(Some(Box::new(write))),
            shutdown: Box::new(shutdown),
            events,
            options,
        });
        let client = Client {
            inner: inner.clone(),
        };

        let reader_inner = inner.clone();
        thread::Builder::new()
            .name("pi-client-reader".into())
            .spawn(move || reader_loop(reader_inner, read))
            .map_err(|e| Error::Disconnected(format!("cannot start reader: {e}")))?;

        let hello = ClientMessage::Hello {
            version: PROTOCOL_VERSION,
        };
        let frame = encode_client_message(&hello, inner.options.max_frame_length)?;
        client.write_frame(&frame)?;
        match hs_rx.recv_timeout(inner.options.handshake_timeout) {
            Ok(Ok(_)) => Ok((client, event_rx)),
            Ok(Err(e)) => Err(e),
            Err(_) => {
                client.fail(Error::Disconnected("handshake timed out".into()));
                Err(Error::Disconnected("handshake timed out".into()))
            }
        }
    }

    pub fn server_id(&self) -> &str {
        &self.inner.options.server_id
    }

    pub fn is_connected(&self) -> bool {
        lock(&self.inner.state).phase == Phase::Connected
    }

    pub fn attachment(&self) -> Option<SessionTarget> {
        lock(&self.inner.state).attachment.clone()
    }

    /// Updates that arrived for a subscription that had already been retired or removed.
    pub fn stale_updates_dropped(&self) -> u64 {
        lock(&self.inner.state).stale_updates_dropped
    }

    pub fn disconnect(&self) {
        self.fail(Error::Disconnected("Client disconnected".into()));
    }

    fn write_frame(&self, frame: &[u8]) -> Result<()> {
        let mut writer = lock(&self.inner.writer);
        let Some(w) = writer.as_mut() else {
            return Err(Error::Disconnected("connection closed".into()));
        };
        match w.write_all(frame).and_then(|_| w.flush()) {
            Ok(()) => Ok(()),
            Err(e) => {
                let error = Error::Disconnected(format!("write failed: {e}"));
                drop(writer);
                self.fail(error.clone());
                Err(error)
            }
        }
    }

    /// Transition to disconnected and unblock everything. Idempotent. The reader thread
    /// reports the event once it observes the closed stream.
    fn fail(&self, error: Error) {
        fail_inner(&self.inner, error);
    }

    /// Whether `target` is the live route for this connection.
    fn target_is_current(state: &State, target: &RpcTarget, server_id: &str) -> bool {
        match target {
            RpcTarget::Server { server_id: s } => s == server_id,
            RpcTarget::Session {
                server_id: s,
                session_id,
                attachment_id,
            } => {
                s == server_id
                    && state.attachment.as_ref().is_some_and(|a| {
                        a.server_id == *s
                            && a.session_id == *session_id
                            && a.attachment_id == *attachment_id
                    })
            }
        }
    }

    /// Send one routed call. Calls to a Session attachment that is no longer current are
    /// refused locally: the route fence a delayed caller must not cross.
    pub fn request(&self, target: &RpcTarget, call: &ServiceCall) -> Result<PendingRequest> {
        self.start_request(target, call, None)
    }

    fn start_request(
        &self,
        target: &RpcTarget,
        call: &ServiceCall,
        subscription: Option<String>,
    ) -> Result<PendingRequest> {
        let (tx, rx) = mpsc::sync_channel(1);
        let id = {
            let mut state = lock(&self.inner.state);
            match state.phase {
                Phase::Connected => {}
                _ => {
                    return Err(state
                        .disconnect
                        .clone()
                        .unwrap_or(Error::Disconnected("Client is disconnected".into())));
                }
            }
            if !Self::target_is_current(&state, target, &self.inner.options.server_id) {
                return Err(Error::Disconnected(
                    "stale route: the target is not the current attachment".into(),
                ));
            }
            state.next_request += 1;
            let id = format!("request-{}", state.next_request);
            state
                .pending
                .insert(id.clone(), PendingEntry { tx, subscription });
            id
        };
        let message = ClientMessage::Request {
            id: id.clone(),
            target: target.clone(),
            call: call.to_value(),
        };
        let frame = match encode_client_message(&message, self.inner.options.max_frame_length) {
            Ok(f) => f,
            Err(e) => {
                lock(&self.inner.state).pending.remove(&id);
                return Err(e);
            }
        };
        if let Err(e) = self.write_frame(&frame) {
            lock(&self.inner.state).pending.remove(&id);
            return Err(e);
        }
        Ok(PendingRequest {
            id,
            target: target.clone(),
            rx,
            client: self.clone(),
        })
    }

    pub fn catalogue(&self, target: &RpcTarget, timeout: Duration) -> Result<Vec<CatalogueEntry>> {
        let result = self
            .request(target, &chord::catalogue_call())?
            .wait_timeout(timeout)?;
        match chord::parse_catalogue(&result.unwrap_or(Value::Null)) {
            Ok(entries) => Ok(entries),
            Err(e) => {
                self.fail(e.clone());
                Err(e)
            }
        }
    }

    /// Subscribe to a service. Returns once the snapshot is installed; updates that arrived
    /// while hydrating are applied in order before this returns.
    pub fn subscribe(
        &self,
        target: &RpcTarget,
        service_id: &str,
        mode: Mode,
        timeout: Duration,
    ) -> Result<Subscription> {
        let (id, shared) = {
            let mut state = lock(&self.inner.state);
            if state.phase != Phase::Connected {
                return Err(state
                    .disconnect
                    .clone()
                    .unwrap_or(Error::Disconnected("Client is disconnected".into())));
            }
            if !Self::target_is_current(&state, target, &self.inner.options.server_id) {
                return Err(Error::Disconnected(
                    "stale route: the target is not the current attachment".into(),
                ));
            }
            state.next_subscription += 1;
            let id = format!("service-{}", state.next_subscription);
            let shared = Arc::new(Mutex::new(SubState {
                replica: None,
                error: None,
                retired: false,
            }));
            state.subs.insert(
                id.clone(),
                Sub {
                    target: target.clone(),
                    service_id: service_id.into(),
                    mode,
                    decoder: StateDecoder::new(),
                    shared: shared.clone(),
                    hydrated: false,
                    queued: Vec::new(),
                },
            );
            (id, shared)
        };
        let pending = match self.start_request(
            target,
            &chord::subscribe_call(&id, service_id, mode),
            Some(id.clone()),
        ) {
            Ok(p) => p,
            Err(e) => {
                lock(&self.inner.state).subs.remove(&id);
                return Err(e);
            }
        };
        match pending.wait_timeout(timeout) {
            Ok(_) => Ok(Subscription {
                id,
                target: target.clone(),
                shared,
                client: self.clone(),
                disposed: false,
            }),
            Err(e) => {
                lock(&self.inner.state).subs.remove(&id);
                Err(e)
            }
        }
    }

    fn unsubscribe(&self, id: &str, target: &RpcTarget) {
        let still_current = {
            let mut state = lock(&self.inner.state);
            state.subs.remove(id);
            state.phase == Phase::Connected
                && Self::target_is_current(&state, target, &self.inner.options.server_id)
        };
        if still_current {
            // Best effort: the response is not awaited, and a failure here changes nothing.
            if let Ok(p) = self.request(target, &chord::unsubscribe_call(id)) {
                drop(p);
            }
        }
    }
}

/// One outstanding call.
pub struct PendingRequest {
    id: String,
    target: RpcTarget,
    rx: Receiver<Result<Option<Value>>>,
    client: Client,
}

impl PendingRequest {
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Block until the response arrives or the connection fails.
    pub fn wait(self) -> Result<Option<Value>> {
        self.rx
            .recv()
            .unwrap_or_else(|_| Err(Error::Disconnected("connection closed".into())))
    }

    /// Wait up to `timeout`. On `Error::Timeout` the request is still outstanding on the
    /// server; call `cancel` (transport cancellation does not undo accepted work).
    pub fn wait_timeout(&self, timeout: Duration) -> Result<Option<Value>> {
        match self.rx.recv_timeout(timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => Err(Error::Timeout),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err(Error::Disconnected("connection closed".into()))
            }
        }
    }

    /// Stop waiting and ask the server to cancel. Accepted work may still complete remotely.
    pub fn cancel(self) {
        // The entry stays registered: the server may still answer (often with a `cancelled`
        // error), and that late response must match something rather than read as a protocol
        // violation. Dropping our receiver makes the eventual delivery a no-op.
        let known = lock(&self.client.inner.state)
            .pending
            .contains_key(&self.id);
        if !known {
            return;
        }
        let message = ClientMessage::Cancel {
            id: self.id.clone(),
            target: self.target.clone(),
        };
        if let Ok(frame) =
            encode_client_message(&message, self.client.inner.options.max_frame_length)
        {
            let _ = self.client.write_frame(&frame);
        }
    }
}

/// A live service subscription. Dropping it unsubscribes (best effort).
pub struct Subscription {
    id: String,
    target: RpcTarget,
    shared: Arc<Mutex<SubState>>,
    client: Client,
    disposed: bool,
}

impl Subscription {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn target(&self) -> &RpcTarget {
        &self.target
    }

    /// Read the replica. `None` once the subscription failed or was retired.
    pub fn read<R>(&self, f: impl FnOnce(&Replica) -> R) -> Option<R> {
        lock(&self.shared).replica.as_ref().map(f)
    }

    pub fn error(&self) -> Option<Error> {
        lock(&self.shared).error.clone()
    }

    pub fn is_retired(&self) -> bool {
        lock(&self.shared).retired
    }

    pub fn dispose(mut self) {
        self.dispose_inner();
    }

    fn dispose_inner(&mut self) {
        if !self.disposed {
            self.disposed = true;
            self.client.unsubscribe(&self.id, &self.target);
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.dispose_inner();
    }
}

// ----------------------------------------------------------------------------- reader

fn fail_inner(inner: &Arc<Inner>, error: Error) {
    let (pending, handshake, subs) = {
        let mut state = lock(&inner.state);
        if state.phase == Phase::Disconnected {
            return;
        }
        state.phase = Phase::Disconnected;
        state.disconnect = Some(error.clone());
        state.attachment = None;
        (
            std::mem::take(&mut state.pending),
            state.handshake.take(),
            std::mem::take(&mut state.subs),
        )
    };
    for (_, entry) in pending {
        let _ = entry.tx.send(Err(error.clone()));
    }
    if let Some(handshake) = handshake {
        let _ = handshake.send(Err(error.clone()));
    }
    for (_, sub) in subs {
        let mut shared = lock(&sub.shared);
        shared.replica = None;
        shared.retired = true;
        shared.error.get_or_insert(error.clone());
    }
    *lock(&inner.writer) = None;
    (inner.shutdown)();
}

fn reader_loop(inner: Arc<Inner>, mut read: impl Read) {
    let mut decoder = ServerMessageDecoder::new(inner.options.max_frame_length);
    let mut buffer = vec![0u8; 64 * 1024];
    let error = loop {
        let n = match read.read(&mut buffer) {
            Ok(0) => {
                break match decoder.end() {
                    Ok(()) => Error::Disconnected("Byte transport closed".into()),
                    Err(e) => e,
                };
            }
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => break Error::Disconnected(format!("read failed: {e}")),
        };
        let messages = match decoder.push(&buffer[..n]) {
            Ok(m) => m,
            Err(e) => break e,
        };
        let mut failure = None;
        for message in messages {
            match handle_message(&inner, message) {
                Ok(events) => {
                    for event in events {
                        // Blocking is the backpressure: a slow consumer slows the peer.
                        if inner.events.send_blocking(event).is_err() {
                            // No consumer remains; keep serving requests and subscriptions.
                        }
                    }
                }
                Err(e) => {
                    failure = Some(e);
                    break;
                }
            }
        }
        if let Some(e) = failure {
            break e;
        }
    };
    fail_inner(&inner, error);
    let report = {
        let mut state = lock(&inner.state);
        if state.disconnect_reported {
            None
        } else {
            state.disconnect_reported = true;
            state.disconnect.clone()
        }
    };
    if let Some(error) = report {
        let _ = inner.events.send_blocking(ClientEvent::Disconnected(error));
    }
}

fn handle_message(inner: &Arc<Inner>, message: ServerMessage) -> Result<Vec<ClientEvent>> {
    let mut state = lock(&inner.state);
    match state.phase {
        Phase::Disconnected => return Ok(vec![]),
        Phase::Connecting => {
            return match message {
                ServerMessage::HelloError { error } => Err(Error::Server {
                    code: error.code,
                    message: error.message,
                }),
                ServerMessage::Hello { server_id } => {
                    if server_id != inner.options.server_id {
                        return Err(validation(format!(
                            "Connected server {server_id:?} does not match {:?}",
                            inner.options.server_id
                        )));
                    }
                    state.phase = Phase::Connected;
                    state.hello_server_id = Some(server_id.clone());
                    if let Some(handshake) = state.handshake.take() {
                        let _ = handshake.send(Ok(server_id));
                    }
                    Ok(vec![])
                }
                _ => Err(validation("Expected server hello as first message")),
            };
        }
        Phase::Connected => {}
    }
    match message {
        ServerMessage::Hello { .. } | ServerMessage::HelloError { .. } => {
            Err(validation("Unexpected handshake message"))
        }
        ServerMessage::Attachment(attachment) => {
            if let Some(a) = &attachment
                && a.server_id != inner.options.server_id
            {
                return Err(validation("Attachment update belongs to another server"));
            }
            if state.attachment == attachment {
                return Ok(vec![]);
            }
            state.attachment = attachment.clone();
            let mut events = Vec::new();
            // Subscriptions bound to any other attachment are over: their late updates must
            // never reach a consumer that has moved on.
            let stale: Vec<String> = state
                .subs
                .iter()
                .filter(|(_, sub)| match &sub.target {
                    RpcTarget::Server { .. } => false,
                    RpcTarget::Session {
                        session_id,
                        attachment_id,
                        ..
                    } => !attachment.as_ref().is_some_and(|a| {
                        a.session_id == *session_id && a.attachment_id == *attachment_id
                    }),
                })
                .map(|(id, _)| id.clone())
                .collect();
            for id in stale {
                if let Some(sub) = state.subs.remove(&id) {
                    let mut shared = lock(&sub.shared);
                    shared.replica = None;
                    shared.retired = true;
                }
                events.push(ClientEvent::SubscriptionRetired { subscription: id });
            }
            events.insert(0, ClientEvent::Attachment(attachment));
            Ok(events)
        }
        ServerMessage::Response { id, outcome } => {
            let Some(entry) = state.pending.remove(&id) else {
                return Err(validation("Response has no matching request"));
            };
            match outcome {
                Err(error) => {
                    if let Some(sub_id) = &entry.subscription {
                        state.subs.remove(sub_id);
                    }
                    let _ = entry.tx.send(Err(Error::Server {
                        code: error.code,
                        message: error.message,
                    }));
                    Ok(vec![])
                }
                Ok(result) => match entry.subscription {
                    None => {
                        let _ = entry.tx.send(Ok(result));
                        Ok(vec![])
                    }
                    Some(sub_id) => {
                        let events = hydrate(&mut state, &sub_id, result)?;
                        let _ = entry.tx.send(Ok(None));
                        Ok(events)
                    }
                },
            }
        }
        ServerMessage::ServiceUpdate {
            subscription_id,
            update,
        } => {
            let Some(sub) = state.subs.get_mut(&subscription_id) else {
                state.stale_updates_dropped += 1;
                return Ok(vec![]);
            };
            if !sub.hydrated {
                if sub.queued.len() >= MAX_QUEUED_UPDATES {
                    return Err(validation(
                        "Too many service updates before the subscription snapshot",
                    ));
                }
                sub.queued.push(update);
                return Ok(vec![]);
            }
            apply_update(&mut state, &subscription_id, &update)
        }
    }
}

/// Install a subscription's snapshot and drain the updates buffered while it was in flight.
fn hydrate(state: &mut State, sub_id: &str, result: Option<Value>) -> Result<Vec<ClientEvent>> {
    let Some(sub) = state.subs.get_mut(sub_id) else {
        return Ok(vec![]); // disposed while in flight
    };
    // An invalid snapshot is a protocol violation: it fails the connection, as in Pi's client.
    let wire = chord::parse_wire_snapshot(
        &result.ok_or_else(|| validation("Subscription response has no snapshot"))?,
    )?;
    if wire.service_id != sub.service_id || wire.mode != sub.mode {
        return Err(validation(format!(
            "Subscription to {} returned the wrong snapshot",
            sub.service_id
        )));
    }
    let snapshot = sub.decoder.decode_snapshot(&wire)?;
    let replica = Replica::hydrate(snapshot)?;
    lock(&sub.shared).replica = Some(replica);
    sub.hydrated = true;
    let queued = std::mem::take(&mut sub.queued);
    let mut events = Vec::new();
    for update in queued {
        events.extend(apply_update(state, sub_id, &update)?);
    }
    Ok(events)
}

fn apply_update(state: &mut State, sub_id: &str, update: &Value) -> Result<Vec<ClientEvent>> {
    let Some(sub) = state.subs.get_mut(sub_id) else {
        return Ok(vec![]);
    };
    // Malformed or undecodable operation streams fail the connection.
    let wire = chord::parse_wire_update(update)?;
    let decoded = sub.decoder.decode_update(&wire)?;
    let mut shared = lock(&sub.shared);
    let Some(replica) = shared.replica.as_mut() else {
        return Ok(vec![]);
    };
    match replica.apply(decoded) {
        Ok(change) => Ok(vec![ClientEvent::SubscriptionChanged {
            subscription: sub_id.into(),
            change,
        }]),
        Err(error) => {
            // A gap or unapplicable op invalidates only this replica.
            shared.replica = None;
            shared.retired = true;
            shared.error = Some(error.clone());
            drop(shared);
            state.subs.remove(sub_id);
            Ok(vec![ClientEvent::SubscriptionFailed {
                subscription: sub_id.into(),
                error,
            }])
        }
    }
}
