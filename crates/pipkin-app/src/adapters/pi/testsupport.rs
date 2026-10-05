//! Test support for end-to-end runs against a REAL Pi engine: a deterministic provider that
//! speaks the OpenAI chat-completions streaming protocol on localhost, and a fixture that
//! launches the engine against it. The engine, its tools and its transcript are real; only the
//! model's answers are scripted. Opt-in tests need `PIPKIN_PI_REPO` (a Pi checkout with
//! dependencies installed).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

use super::engine::{EngineConfig, EngineHost};

/// What the scripted model does for one chat-completions request.
#[derive(Clone, Debug)]
pub enum Reply {
    /// Stream this text, then stop.
    Text(String),
    /// Optionally some text, then one tool call.
    Tool {
        lead: Option<String>,
        name: String,
        args: Value,
    },
    /// Accept the request and never answer, until the stub stops.
    #[allow(dead_code)]
    Hang,
    /// Hold the request until the gate opens (or the stub stops), then answer with the reply.
    /// The engine sees a model that is slow to respond, so a run stays in flight on demand.
    Gated(Gate, Box<Reply>),
}

/// A latch a test opens to let a held model response through.
#[derive(Clone, Debug, Default)]
pub struct Gate(Arc<(Mutex<bool>, Condvar)>);

impl Gate {
    pub fn new() -> Gate {
        Gate::default()
    }

    pub fn open(&self) {
        *self.0.0.lock().unwrap() = true;
        self.0.1.notify_all();
    }

    /// Wait until open; false if `stop` was set first.
    fn wait(&self, stop: &AtomicBool) -> bool {
        let mut open = self.0.0.lock().unwrap();
        while !*open {
            if stop.load(Ordering::SeqCst) {
                return false;
            }
            open = self
                .0
                .1
                .wait_timeout(open, Duration::from_millis(100))
                .unwrap()
                .0;
        }
        true
    }
}

type Script = dyn Fn(&Value, usize) -> Reply + Send + Sync;

pub struct StubProvider {
    port: u16,
    requests: Arc<Mutex<Vec<Value>>>,
    stop: Arc<AtomicBool>,
}

fn chunk(delta: Value, finish: Option<&str>, usage: Option<Value>) -> String {
    let mut c = json!({
        "id": "stub-1", "object": "chat.completion.chunk", "created": 0, "model": "scripted",
        "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }],
    });
    if let Some(usage) = usage {
        c["usage"] = usage;
    }
    format!("data: {c}\n\n")
}

fn usage() -> Value {
    json!({ "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 })
}

/// The SSE events for a reply, as the real provider would stream them.
fn events(reply: &Reply) -> Vec<String> {
    let mut out = vec![chunk(
        json!({ "role": "assistant", "content": "" }),
        None,
        None,
    )];
    match reply {
        Reply::Text(text) => {
            out.push(chunk(json!({ "content": text }), None, None));
            out.push(chunk(json!({}), Some("stop"), Some(usage())));
        }
        Reply::Tool { lead, name, args } => {
            if let Some(lead) = lead {
                out.push(chunk(json!({ "content": lead }), None, None));
            }
            let call = json!({ "tool_calls": [{
                "index": 0, "id": "call_1", "type": "function",
                "function": { "name": name, "arguments": args.to_string() },
            }]});
            out.push(chunk(call, None, None));
            out.push(chunk(json!({}), Some("tool_calls"), Some(usage())));
        }
        Reply::Hang => {}
        Reply::Gated(_, inner) => return events(inner),
    }
    out.push("data: [DONE]\n\n".into());
    out
}

fn read_request(stream: &mut TcpStream) -> Option<(String, String, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.lines();
    let mut first = lines.next()?.split_whitespace();
    let (method, path) = (first.next()?.to_owned(), first.next()?.to_owned());
    let length = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = buf[header_end..].to_vec();
    while body.len() < length {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    Some((method, path, body))
}

fn serve(
    mut stream: TcpStream,
    script: Arc<Script>,
    requests: Arc<Mutex<Vec<Value>>>,
    count: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
) {
    let Some((method, path, body)) = read_request(&mut stream) else {
        return;
    };
    if method == "GET" {
        let body = r#"{"object":"list","data":[{"id":"scripted","object":"model"}]}"#;
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        return;
    }
    let request: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let n = count.fetch_add(1, Ordering::SeqCst);
    requests
        .lock()
        .unwrap()
        .push(json!({ "path": path, "body": request.clone() }));
    let mut reply = script(&request, n);
    while let Reply::Gated(gate, inner) = reply {
        if !gate.wait(&stop) {
            return;
        }
        reply = *inner;
    }
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n"
    );
    for event in events(&reply) {
        if stream
            .write_all(event.as_bytes())
            .and_then(|_| stream.flush())
            .is_err()
        {
            return;
        }
    }
    if matches!(reply, Reply::Hang) {
        // Keep the connection open until the stub stops or the client gives up.
        while !stop.load(Ordering::SeqCst) {
            if stream
                .write_all(b": keep-alive\n\n")
                .and_then(|_| stream.flush())
                .is_err()
            {
                return;
            }
            thread::sleep(Duration::from_millis(200));
        }
    }
}

impl StubProvider {
    /// Start on an ephemeral localhost port. `script` receives each chat request body and its
    /// zero-based index.
    pub fn start(script: impl Fn(&Value, usize) -> Reply + Send + Sync + 'static) -> StubProvider {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub provider");
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let count = Arc::new(AtomicUsize::new(0));
        let script: Arc<Script> = Arc::new(script);
        let (r, s) = (requests.clone(), stop.clone());
        thread::spawn(move || {
            for stream in listener.incoming() {
                if s.load(Ordering::SeqCst) {
                    return;
                }
                let Ok(stream) = stream else { continue };
                let (script, r, c, s) = (script.clone(), r.clone(), count.clone(), s.clone());
                thread::spawn(move || serve(stream, script, r, c, s));
            }
        });
        StubProvider {
            port,
            requests,
            stop,
        }
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}/v1", self.port)
    }

    /// Drop the recorded requests (they carry whole histories), for long runs that measure memory.
    pub fn forget_requests(&self) {
        self.requests.lock().unwrap().clear();
    }

    /// Every chat-completions request the engine has made, in order.
    pub fn requests(&self) -> Vec<Value> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| {
                r["path"]
                    .as_str()
                    .is_some_and(|p| p.ends_with("/chat/completions"))
            })
            .map(|r| r["body"].clone())
            .collect()
    }
}

impl Drop for StubProvider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop so it can exit.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

/// True when the newest message is a tool result, i.e. the model is being shown a tool's output.
pub fn is_tool_result_turn(request: &Value) -> bool {
    request["messages"]
        .as_array()
        .and_then(|m| m.last())
        .is_some_and(|m| m["role"] == "tool")
}

/// A fresh canonical UUIDv4 from the kernel.
fn uuid() -> String {
    std::fs::read_to_string("/proc/sys/kernel/random/uuid")
        .expect("kernel uuid")
        .trim()
        .to_owned()
}

/// A throwaway profile and a scripted provider, with the engine described but not yet started.
pub struct Prepared {
    pub root_dir: tempfile::TempDir,
    pub root: PathBuf,
    pub project: PathBuf,
    pub server_dir: PathBuf,
    pub server_id: String,
    pub provider: StubProvider,
    pub config: EngineConfig,
}

pub fn pi_repo() -> PathBuf {
    PathBuf::from(
        std::env::var("PIPKIN_PI_REPO").expect(
            "PIPKIN_PI_REPO: a Pi checkout with dependencies installed (this test is opt-in)",
        ),
    )
}

/// Lay out a short temp root (Unix socket paths are limited to about 108 bytes), a project
/// directory, an agent directory whose `models.json` points at the scripted provider, and the
/// engine configuration. Nothing is started.
pub fn prepare(script: impl Fn(&Value, usize) -> Reply + Send + Sync + 'static) -> Prepared {
    let root_dir = tempfile::Builder::new()
        .prefix("pk")
        .tempdir_in("/tmp")
        .expect("temp root");
    let root = root_dir.path().to_path_buf();
    let (project, server_dir, agent_dir) = (
        root.join("project"),
        root.join("server"),
        root.join("agent"),
    );
    for dir in [&project, &server_dir, &agent_dir] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let provider = StubProvider::start(script);
    std::fs::write(
        agent_dir.join("models.json"),
        json!({ "providers": { "stub": {
            "baseUrl": provider.base_url(), "api": "openai-completions", "apiKey": "stub",
            "models": [{ "id": "scripted", "name": "Scripted stub", "input": ["text", "image"] }],
        }}})
        .to_string(),
    )
    .unwrap();
    let server_id = uuid();
    let config = EngineConfig {
        pi_repo: pi_repo(),
        server_dir: server_dir.clone(),
        server_id: server_id.clone(),
        agent_dir: Some(agent_dir),
        // The engine's own directory; every conversation passes its project as its cwd.
        cwd: root.clone(),
        model: Some(("stub".into(), "scripted".into())),
        log_path: root.join("engine.log"),
        env: vec![("PI_OFFLINE".into(), "1".into())],
        extensions: vec![],
    };
    Prepared {
        root_dir,
        root,
        project,
        server_dir,
        server_id,
        provider,
        config,
    }
}

/// A real engine on a throwaway profile, answering with a scripted provider.
pub struct Fixture {
    _root: tempfile::TempDir,
    pub root: PathBuf,
    pub project: PathBuf,
    pub server_dir: PathBuf,
    pub server_id: String,
    pub provider: StubProvider,
    pub host: EngineHost,
}

impl Fixture {
    /// Start the provider and the engine and wait until the engine is ready.
    pub fn start(script: impl Fn(&Value, usize) -> Reply + Send + Sync + 'static) -> Fixture {
        Fixture::start_with(script, |_| {})
    }

    /// Like `start`, after `tweak` has adjusted how the engine is launched.
    pub fn start_with(
        script: impl Fn(&Value, usize) -> Reply + Send + Sync + 'static,
        tweak: impl FnOnce(&mut EngineConfig),
    ) -> Fixture {
        let mut p = prepare(script);
        tweak(&mut p.config);
        let mut host = EngineHost::new(p.config);
        host.ensure_running().expect("the engine starts");
        Fixture {
            _root: p.root_dir,
            root: p.root,
            project: p.project,
            server_dir: p.server_dir,
            server_id: p.server_id,
            provider: p.provider,
            host,
        }
    }

    /// The engine's captured output, for diagnosing a failure.
    pub fn log(&self) -> String {
        std::fs::read_to_string(self.root.join("engine.log")).unwrap_or_default()
    }
}
