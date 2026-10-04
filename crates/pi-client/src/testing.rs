//! A scriptable in-process Pi server that speaks the real wire (framed CBOR, Chord control
//! calls, per-subscription Delta codecs), for tests of this crate and of adapters built on it.
//!
//! It is a test double, not evidence about the real engine: conformance against the actual Pi
//! server is a separate gate.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use serde_json::Value;

use crate::chord::{
    self, CatalogueEntry, ControlCall, InstanceSnapshot, MemberSnapshot, Mode, ProviderUpdate,
    ServiceCall, StateEncoder, SubscriptionSnapshot,
};
use crate::client::{Client, ClientEvent, ClientOptions};
use crate::delta::{self, Op};
use crate::frame::DEFAULT_MAX_FRAME_LENGTH;
use crate::protocol::{
    ClientMessage, ClientMessageDecoder, PROTOCOL_VERSION, ProtocolError, RpcTarget, ServerMessage,
    SessionTarget, encode_server_message,
};

pub const SERVER_ID: &str = "00000000-0000-4000-8000-000000000001";

pub type HandlerResult = std::result::Result<Option<Value>, ProtocolError>;
pub type Handler = Arc<dyn Fn(&MockPi, &ConnHandle, &ServiceCall) -> HandlerResult + Send + Sync>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct MockService {
    id: String,
    methods: Vec<String>,
    /// The single replicated state member (named "state") and its current revision.
    state: Option<(Value, u64)>,
}

struct MockSub {
    service_id: String,
    encoder: StateEncoder,
}

struct ConnState {
    writer: Mutex<UnixStream>,
    subs: Mutex<HashMap<String, MockSub>>,
    attachment: Mutex<Option<SessionTarget>>,
}

struct MockState {
    server_id: String,
    services: Vec<MockService>,
    handlers: HashMap<(String, String), Handler>,
    connections: Vec<Arc<ConnState>>,
    requests: Vec<(RpcTarget, ServiceCall)>,
    next_attachment: u64,
}

#[derive(Clone)]
pub struct MockPi {
    state: Arc<Mutex<MockState>>,
}

/// One accepted connection, as the server sees it.
#[derive(Clone)]
pub struct ConnHandle {
    conn: Arc<ConnState>,
}

pub struct MockClient {
    pub client: Client,
    pub events: async_channel::Receiver<ClientEvent>,
    pub conn: ConnHandle,
}

impl ConnHandle {
    fn send(&self, message: &ServerMessage) {
        if let Ok(frame) = encode_server_message(message, DEFAULT_MAX_FRAME_LENGTH) {
            self.send_raw(&frame);
        }
    }

    /// Write arbitrary bytes, for malformed-frame and fragmentation tests.
    pub fn send_raw(&self, bytes: &[u8]) {
        let mut w = lock(&self.conn.writer);
        let _ = w.write_all(bytes);
        let _ = w.flush();
    }

    pub fn set_attachment(&self, attachment: Option<SessionTarget>) {
        *lock(&self.conn.attachment) = attachment.clone();
        self.send(&ServerMessage::Attachment(attachment));
    }

    pub fn attachment(&self) -> Option<SessionTarget> {
        lock(&self.conn.attachment).clone()
    }

    /// Inject a raw update for a subscription id, even one the client has since retired.
    pub fn send_update(&self, subscription_id: &str, update: Value) {
        self.send(&ServerMessage::ServiceUpdate {
            subscription_id: subscription_id.into(),
            update,
        });
    }

    pub fn subscription_ids(&self, service_id: &str) -> Vec<String> {
        let mut ids: Vec<String> = lock(&self.conn.subs)
            .iter()
            .filter(|(_, s)| s.service_id == service_id)
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();
        ids
    }

    /// Close the socket, as a crashed server would.
    pub fn close(&self) {
        let _ = lock(&self.conn.writer).shutdown(std::net::Shutdown::Both);
    }
}

impl Default for MockPi {
    fn default() -> Self {
        MockPi::new(SERVER_ID)
    }
}

impl MockPi {
    pub fn new(server_id: &str) -> Self {
        MockPi {
            state: Arc::new(Mutex::new(MockState {
                server_id: server_id.into(),
                services: Vec::new(),
                handlers: HashMap::new(),
                connections: Vec::new(),
                requests: Vec::new(),
                next_attachment: 0,
            })),
        }
    }

    /// Register a singleton service. `state` is its initial value (member "state").
    pub fn add_service(&self, id: &str, methods: &[&str], state: Option<Value>) {
        lock(&self.state).services.push(MockService {
            id: id.into(),
            methods: methods.iter().map(|m| m.to_string()).collect(),
            state: state.map(|v| (v, 0)),
        });
    }

    pub fn set_handler(
        &self,
        service: &str,
        member: &str,
        handler: impl Fn(&MockPi, &ConnHandle, &ServiceCall) -> HandlerResult + Send + Sync + 'static,
    ) {
        lock(&self.state)
            .handlers
            .insert((service.into(), member.into()), Arc::new(handler));
    }

    /// Every non-control call received so far, in order.
    pub fn requests(&self) -> Vec<(RpcTarget, ServiceCall)> {
        lock(&self.state).requests.clone()
    }

    /// The current server-side value of a service's state.
    pub fn state_of(&self, service: &str) -> Option<Value> {
        lock(&self.state)
            .services
            .iter()
            .find(|s| s.id == service)
            .and_then(|s| s.state.as_ref().map(|(v, _)| v.clone()))
    }

    /// Apply ops to a service's state and push the resulting update to every subscriber on
    /// every connection, each through its own independent Delta stream.
    pub fn publish(&self, service: &str, ops: Vec<Op>) {
        let (sequence, connections) = {
            let mut state = lock(&self.state);
            let Some(svc) = state.services.iter_mut().find(|s| s.id == service) else {
                panic!("unknown mock service {service}");
            };
            let Some((value, sequence)) = svc.state.as_mut() else {
                panic!("mock service {service} has no state");
            };
            *value = delta::apply(Some(std::mem::take(value)), ops.clone())
                .expect("mock publish applies");
            *sequence += 1;
            (*sequence, state.connections.clone())
        };
        for conn in connections {
            let handle = ConnHandle { conn: conn.clone() };
            let mut subs = lock(&conn.subs);
            for (id, sub) in subs.iter_mut().filter(|(_, s)| s.service_id == service) {
                let update = ProviderUpdate::State {
                    instance: None,
                    member: "state".into(),
                    sequence,
                    ops: ops.clone(),
                };
                let wire = sub.encoder.encode_update(&update).expect("mock encode");
                handle.send(&ServerMessage::ServiceUpdate {
                    subscription_id: id.clone(),
                    update: chord::wire_update_value(&wire),
                });
            }
        }
    }

    /// Open a connection and complete the handshake.
    pub fn connect(&self) -> crate::Result<MockClient> {
        self.connect_with(ClientOptions::new(SERVER_ID))
    }

    /// Serve one already-accepted stream (for tests that bind a real listener).
    pub fn accept(&self, server_end: UnixStream) -> ConnHandle {
        let conn = Arc::new(ConnState {
            writer: Mutex::new(server_end.try_clone().expect("clone")),
            subs: Mutex::new(HashMap::new()),
            attachment: Mutex::new(None),
        });
        lock(&self.state).connections.push(conn.clone());
        let me = self.clone();
        let serving = conn.clone();
        thread::Builder::new()
            .name("mock-pi-conn".into())
            .spawn(move || serve(me, serving, server_end))
            .expect("spawn mock connection");
        ConnHandle { conn }
    }

    pub fn connect_with(&self, options: ClientOptions) -> crate::Result<MockClient> {
        let (client_end, server_end) = UnixStream::pair().expect("socketpair");
        let conn = self.accept(server_end).conn;
        let read = client_end.try_clone().expect("clone");
        let shutdown_end = client_end.try_clone().expect("clone");
        let (client, events) = Client::establish(
            read,
            client_end,
            move || {
                let _ = shutdown_end.shutdown(std::net::Shutdown::Both);
            },
            options,
        )?;
        Ok(MockClient {
            client,
            events,
            conn: ConnHandle { conn },
        })
    }
}

fn serve(pi: MockPi, conn: Arc<ConnState>, mut stream: UnixStream) {
    let handle = ConnHandle { conn: conn.clone() };
    let mut decoder = ClientMessageDecoder::default();
    let mut buffer = vec![0u8; 16 * 1024];
    let mut said_hello = false;
    loop {
        let n = match stream.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        let Ok(messages) = decoder.push(&buffer[..n]) else {
            return;
        };
        for message in messages {
            match message {
                ClientMessage::Hello { version } => {
                    said_hello = true;
                    if version == PROTOCOL_VERSION {
                        let server_id = lock(&pi.state).server_id.clone();
                        handle.send(&ServerMessage::Hello { server_id });
                    } else {
                        handle.send(&ServerMessage::HelloError {
                            error: ProtocolError {
                                code: "version".into(),
                                message: format!("unsupported version {version}"),
                            },
                        });
                    }
                }
                ClientMessage::Request { id, target, call } => {
                    if !said_hello {
                        return;
                    }
                    let outcome = handle_request(&pi, &handle, &target, &call);
                    handle.send(&ServerMessage::Response { id, outcome });
                }
                ClientMessage::Cancel { .. } => {}
            }
        }
    }
}

fn error(code: &str, message: &str) -> ProtocolError {
    ProtocolError {
        code: code.into(),
        message: message.into(),
    }
}

fn handle_request(
    pi: &MockPi,
    conn: &ConnHandle,
    target: &RpcTarget,
    call: &Value,
) -> HandlerResult {
    let Ok(call) = chord::parse_service_call(call) else {
        return Err(error("invalid_call", "malformed service call"));
    };
    // The server fences Session calls to the live attachment, as Pi does.
    if let RpcTarget::Session {
        session_id,
        attachment_id,
        ..
    } = target
    {
        let current = conn.attachment();
        if !current
            .is_some_and(|a| a.session_id == *session_id && a.attachment_id == *attachment_id)
        {
            return Err(error(
                "stale_attachment",
                "the attachment is no longer current",
            ));
        }
    }
    if let Some(control) = chord::decode_control_call(&call) {
        return match control {
            ControlCall::Catalogue => {
                let entries: Vec<CatalogueEntry> = lock(&pi.state)
                    .services
                    .iter()
                    .map(|s| CatalogueEntry {
                        service_id: s.id.clone(),
                        mode: Mode::Singleton,
                    })
                    .collect();
                Ok(Some(chord::catalogue_value(&entries)))
            }
            ControlCall::Subscribe {
                subscription_id,
                service_id,
                mode,
            } => {
                let state = lock(&pi.state);
                let Some(svc) = state.services.iter().find(|s| s.id == service_id) else {
                    return Err(error("service_not_found", &service_id));
                };
                if mode != Mode::Singleton {
                    return Err(error("invalid_call", "mock services are singletons"));
                }
                let mut members: Vec<MemberSnapshot<Op>> = svc
                    .methods
                    .iter()
                    .map(|m| MemberSnapshot::Method { name: m.clone() })
                    .collect();
                if let Some((value, sequence)) = &svc.state {
                    members.push(MemberSnapshot::State {
                        name: "state".into(),
                        sequence: *sequence,
                        ops: vec![Op::Replace(value.clone())],
                    });
                }
                let snapshot = SubscriptionSnapshot {
                    service_id: service_id.clone(),
                    mode,
                    instances: vec![InstanceSnapshot {
                        instance: None,
                        members,
                    }],
                };
                let mut encoder = StateEncoder::new();
                let wire = encoder
                    .encode_snapshot(&snapshot)
                    .expect("mock encode snapshot");
                // Register under the same lock as the snapshot so no publish can slip between.
                lock(&conn.conn.subs).insert(
                    subscription_id,
                    MockSub {
                        service_id,
                        encoder,
                    },
                );
                Ok(Some(chord::wire_snapshot_value(&wire)))
            }
            ControlCall::Unsubscribe { subscription_id } => {
                lock(&conn.conn.subs).remove(&subscription_id);
                Ok(None)
            }
        };
    }
    lock(&pi.state)
        .requests
        .push((target.clone(), call.clone()));
    let handler = lock(&pi.state)
        .handlers
        .get(&(call.service_id.clone(), call.member.clone()))
        .cloned();
    match handler {
        Some(h) => h(pi, conn, &call),
        None => Err(error(
            "method_not_found",
            &format!("{}.{}", call.service_id, call.member),
        )),
    }
}

/// A `pi.session-management` implementation: `attach(sessionId)` publishes a fresh attachment
/// (before the response, as the real server may), `detach()` clears it.
pub fn install_session_management(pi: &MockPi) {
    pi.add_service(
        "pi.session-management",
        &["create", "remove", "attach", "detach"],
        None,
    );
    pi.set_handler("pi.session-management", "attach", |pi, conn, call| {
        let session_id = call
            .args
            .first()
            .and_then(Value::as_str)
            .ok_or_else(|| error("invalid_call", "sessionId"))?;
        let attachment_id = {
            let mut state = lock(&pi.state);
            state.next_attachment += 1;
            format!("attachment-{}", state.next_attachment)
        };
        let server_id = lock(&pi.state).server_id.clone();
        conn.set_attachment(Some(SessionTarget {
            server_id,
            session_id: session_id.into(),
            attachment_id,
        }));
        Ok(None)
    });
    pi.set_handler("pi.session-management", "detach", |_, conn, _| {
        conn.set_attachment(None);
        Ok(None)
    });
}
