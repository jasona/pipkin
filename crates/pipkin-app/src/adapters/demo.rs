//! `DemoBackend`: a deterministic simulated Pi engine.
//!
//! One worker thread owns all simulated runs. Scripted events are emitted with real sleeps
//! scaled by `speed` (0.0 means never sleep). Between steps the worker polls its request
//! queue, so Cancel, Steer and CheckStatus act on the live script rather than a replay.
//! The clock is never read for content: ids, timestamps and text are functions of the seed.

use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pipkin_core::*;

use super::fixtures::{DEFAULT_BASE_TIME, Fixtures};
use super::rng::Rng;
use super::script::{self, Action, Step, Subst};

/// How long a cancelled run takes to settle before `Cancelled` is emitted (at speed 1.0).
pub const SETTLE_MS: u64 = 350;

#[derive(Clone, Debug)]
pub struct DemoOptions {
    /// 1.0 is real time; 0.0 never sleeps (tests).
    pub speed: f32,
    pub seed: u64,
    /// Scenario that answers the next Submit.
    pub scenario: String,
    /// Fixed "now" for fixture timestamps. Never the wall clock unless the caller passes it.
    pub base_time: i64,
    /// At speed 0, `await_steer` steps wait for a Steer (tests) instead of passing through.
    pub hold_steer: bool,
}

impl Default for DemoOptions {
    fn default() -> Self {
        DemoOptions {
            speed: 1.0,
            seed: 0x5EED,
            scenario: "normal".into(),
            base_time: DEFAULT_BASE_TIME,
            hold_steer: false,
        }
    }
}

/// One Submit as the backend saw it.
#[derive(Clone, Debug, PartialEq)]
pub struct SubmitRecord {
    pub conversation: ConversationId,
    pub op: OperationId,
    pub text: String,
}

struct Shared {
    scenario: Mutex<String>,
    submit_count: AtomicUsize,
    submissions: Mutex<Vec<SubmitRecord>>,
}

pub struct DemoBackend {
    requests: mpsc::Sender<BackendRequest>,
    shared: Arc<Shared>,
    fixtures: Fixtures,
}

impl DemoBackend {
    pub fn new(events: async_channel::Sender<BackendEvent>, options: DemoOptions) -> Self {
        let mut scenario = options.scenario.clone();
        if !script::scenario_names().contains(&scenario.as_str()) {
            log::warn!("unknown demo scenario {scenario:?}; using normal");
            scenario = "normal".into();
        }
        let shared = Arc::new(Shared {
            scenario: Mutex::new(scenario),
            submit_count: AtomicUsize::new(0),
            submissions: Mutex::new(Vec::new()),
        });
        let (requests, inbox) = mpsc::channel();
        let worker = Worker {
            ctx: Ctx {
                events,
                speed: options.speed.max(0.0),
                hold_steer: options.hold_steer,
                closed: Cell::new(false),
            },
            inbox,
            shared: shared.clone(),
            fixtures: Fixtures::new(options.seed, options.base_time),
            seed: options.seed,
            runs: Vec::new(),
            attempts: HashMap::new(),
            seen_ops: HashSet::new(),
            scripts: HashMap::new(),
        };
        std::thread::Builder::new()
            .name("pi-demo-backend".into())
            .spawn(move || worker.run())
            .expect("spawn demo backend thread");
        DemoBackend {
            requests,
            shared,
            fixtures: Fixtures::new(options.seed, options.base_time),
        }
    }

    pub fn bootstrap(&self) -> Bootstrap {
        self.fixtures.bootstrap()
    }

    /// Scenario that will answer the next Submit.
    pub fn scenario(&self) -> String {
        self.shared.scenario.lock().unwrap().clone()
    }

    /// Returns false (and changes nothing) for an unknown scenario name.
    pub fn set_scenario(&self, name: &str) -> bool {
        if script::scenario_names().contains(&name) {
            *self.shared.scenario.lock().unwrap() = name.to_string();
            true
        } else {
            false
        }
    }

    /// Number of Submit requests the backend has received. A repeated Submit is counted, never
    /// folded into the original.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn submit_count(&self) -> usize {
        self.shared.submit_count.load(Ordering::SeqCst)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn submissions(&self) -> Vec<SubmitRecord> {
        self.shared.submissions.lock().unwrap().clone()
    }
}

impl Backend for DemoBackend {
    fn start(&self, sink: LifecycleSink) {
        sink(LifecycleEvent::Catalog(self.bootstrap()));
        sink(LifecycleEvent::Connection(Connection::Ready));
    }

    fn request(&self, request: BackendRequest) {
        // The worker only goes away when the backend is dropped.
        let _ = self.requests.send(request);
    }
}

// ------------------------------------------------------------------------------ worker

#[derive(Clone, Copy, PartialEq)]
enum Block {
    None,
    /// Waiting for a steer, optionally until a deadline.
    Steer(Option<Instant>),
    /// Waiting for CheckStatus.
    Status,
    /// Cancelled; waiting out the settle delay.
    Settling,
}

struct Run {
    conversation: ConversationId,
    generation: u64,
    op: OperationId,
    steps: Vec<Step>,
    next_step: usize,
    queue: VecDeque<Action>,
    due: Instant,
    block: Block,
    steered: bool,
    /// A steer arrived while the script was not waiting; the next `await_steer` consumes it.
    pending_steer: bool,
    subst: Subst,
    rng: Rng,
    ended: bool,
}

struct Ctx {
    events: async_channel::Sender<BackendEvent>,
    speed: f32,
    hold_steer: bool,
    closed: Cell<bool>,
}

impl Ctx {
    fn scale(&self, ms: u64) -> Duration {
        if self.speed <= 0.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(ms as f64 / 1000.0 * self.speed as f64)
        }
    }

    fn emit(
        &self,
        conversation: ConversationId,
        generation: u64,
        op: Option<OperationId>,
        kind: EventKind,
    ) {
        let event = BackendEvent {
            conversation,
            generation,
            op,
            kind,
        };
        if self.events.send_blocking(event).is_err() {
            self.closed.set(true);
        }
    }

    fn runnable(&self, run: &Run, now: Instant) -> bool {
        match run.block {
            Block::None | Block::Settling => self.speed <= 0.0 || now >= run.due,
            Block::Steer(deadline) => deadline.is_some_and(|d| now >= d),
            Block::Status => false,
        }
    }

    fn wake_at(&self, run: &Run) -> Option<Instant> {
        match run.block {
            Block::None | Block::Settling => Some(run.due),
            Block::Steer(deadline) => deadline,
            Block::Status => None,
        }
    }

    /// Advance one run by one action.
    fn step(&self, run: &mut Run) {
        match run.block {
            Block::Settling => {
                self.emit(
                    run.conversation,
                    run.generation,
                    Some(run.op),
                    EventKind::Cancelled,
                );
                run.ended = true;
            }
            Block::Steer(_) => run.block = Block::None,
            Block::Status => {}
            Block::None => {
                let action = loop {
                    if let Some(a) = run.queue.pop_front() {
                        break Some(a);
                    }
                    let Some(step) = run.steps.get(run.next_step).cloned() else {
                        break None;
                    };
                    run.next_step += 1;
                    script::expand(&step, run.steered, &run.subst, &mut run.rng, &mut run.queue);
                };
                match action {
                    None => run.ended = true,
                    Some(Action::Sleep(ms)) => run.due = Instant::now() + self.scale(ms),
                    Some(Action::Emit(kind)) => {
                        let terminal = matches!(
                            kind,
                            EventKind::Completed
                                | EventKind::Failed { .. }
                                | EventKind::Rejected { .. }
                        );
                        self.emit(run.conversation, run.generation, Some(run.op), kind);
                        run.ended |= terminal;
                    }
                    Some(Action::AwaitSteer(timeout)) => {
                        if run.pending_steer {
                            run.pending_steer = false;
                        } else if self.speed > 0.0 {
                            let deadline = timeout.map(|ms| Instant::now() + self.scale(ms));
                            run.block = Block::Steer(deadline);
                        } else if self.hold_steer {
                            run.block = Block::Steer(None);
                        }
                    }
                    Some(Action::AwaitStatus) => run.block = Block::Status,
                }
            }
        }
    }
}

struct Worker {
    ctx: Ctx,
    inbox: mpsc::Receiver<BackendRequest>,
    shared: Arc<Shared>,
    fixtures: Fixtures,
    seed: u64,
    runs: Vec<Run>,
    /// Submits seen per conversation; selects the variant for `failure`.
    attempts: HashMap<ConversationId, usize>,
    seen_ops: HashSet<(ConversationId, OperationId)>,
    scripts: HashMap<String, Vec<Vec<Step>>>,
}

impl Worker {
    fn run(mut self) {
        loop {
            loop {
                match self.inbox.try_recv() {
                    Ok(request) => self.handle(request),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }
            let now = Instant::now();
            let mut progressed = false;
            for run in &mut self.runs {
                if self.ctx.runnable(run, now) {
                    self.ctx.step(run);
                    progressed = true;
                }
            }
            self.runs.retain(|r| !r.ended);
            if self.ctx.closed.get() {
                return;
            }
            if progressed {
                continue;
            }
            let wake = self.runs.iter().filter_map(|r| self.ctx.wake_at(r)).min();
            let request = match wake {
                Some(at) => match self
                    .inbox
                    .recv_timeout(at.saturating_duration_since(Instant::now()))
                {
                    Ok(r) => Some(r),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                },
                None => match self.inbox.recv() {
                    Ok(r) => Some(r),
                    Err(_) => return,
                },
            };
            if let Some(request) = request {
                self.handle(request);
            }
        }
    }

    fn variants(&mut self, scenario: &str) -> &Vec<Vec<Step>> {
        self.scripts.entry(scenario.to_string()).or_insert_with(|| {
            script::load(scenario)
                .or_else(|e| {
                    log::error!("scenario {scenario}: {e}");
                    script::load("normal")
                })
                .unwrap_or_else(|e| {
                    log::error!("builtin scenario failed to load: {e}");
                    vec![vec![]]
                })
        })
    }

    fn find_run(&mut self, conversation: ConversationId, op: OperationId) -> Option<&mut Run> {
        self.runs
            .iter_mut()
            .find(|r| r.conversation == conversation && r.op == op && !r.ended)
    }

    fn handle(&mut self, request: BackendRequest) {
        match request {
            BackendRequest::Open {
                conversation,
                generation,
            } => {
                let (items, has_older, changes) = self.fixtures.open(conversation);
                self.ctx.emit(
                    conversation,
                    generation,
                    None,
                    EventKind::Opened {
                        items,
                        has_older,
                        changes,
                    },
                );
            }
            BackendRequest::LoadOlder {
                conversation,
                generation,
                before,
            } => {
                let (items, has_older) = self.fixtures.older(conversation, before);
                self.ctx.emit(
                    conversation,
                    generation,
                    None,
                    EventKind::OlderPage { items, has_older },
                );
            }
            BackendRequest::Submit {
                conversation,
                generation,
                op,
                text,
                model,
                ..
            } => {
                self.shared.submit_count.fetch_add(1, Ordering::SeqCst);
                self.shared.submissions.lock().unwrap().push(SubmitRecord {
                    conversation,
                    op,
                    text: text.clone(),
                });
                self.seen_ops.insert((conversation, op));
                let attempt = {
                    let n = self.attempts.entry(conversation).or_insert(0);
                    *n += 1;
                    *n
                };
                let scenario = self.shared.scenario.lock().unwrap().clone();
                let variants = self.variants(&scenario);
                let steps = variants[(attempt - 1) % variants.len()].clone();
                self.runs.push(Run {
                    conversation,
                    generation,
                    op,
                    steps,
                    next_step: 0,
                    queue: VecDeque::new(),
                    due: Instant::now(),
                    block: Block::None,
                    steered: false,
                    pending_steer: false,
                    subst: Subst {
                        prompt: single_line(&text, 100),
                        steer: String::new(),
                        model: model.unwrap_or_else(|| "default".into()),
                    },
                    rng: Rng::from_parts(&[self.seed, conversation.0, op.0, attempt as u64]),
                    ended: false,
                });
            }
            // Real-mode requests: the demo's core never issues them.
            BackendRequest::Queue { .. }
            | BackendRequest::OpenSubagent { .. }
            | BackendRequest::SetSubagentsEnabled { .. }
            | BackendRequest::RemoveSignIn(_)
            | BackendRequest::StartSignIn(_)
            | BackendRequest::ReuseSignIn(_)
            | BackendRequest::AnswerSignIn { .. }
            | BackendRequest::CancelSignIn { .. }
            | BackendRequest::CancelQueued { .. }
            | BackendRequest::RefreshModels { .. }
            | BackendRequest::RefreshChanges { .. }
            | BackendRequest::FetchToolOutput { .. }
            | BackendRequest::UiRespond { .. }
            | BackendRequest::UiCancel { .. } => {}
            BackendRequest::Steer {
                conversation,
                generation,
                op,
                text,
            } => {
                let accepted = match self.find_run(conversation, op) {
                    Some(run) if run.block != Block::Settling => {
                        run.steered = true;
                        run.subst.steer = single_line(&text, 100);
                        if matches!(run.block, Block::Steer(_)) {
                            run.block = Block::None;
                        } else {
                            run.pending_steer = true;
                        }
                        true
                    }
                    _ => false,
                };
                if accepted {
                    self.ctx
                        .emit(conversation, generation, Some(op), EventKind::SteerAccepted);
                }
            }
            BackendRequest::Cancel {
                conversation,
                generation,
                op,
            } => {
                let due = Instant::now() + self.ctx.scale(SETTLE_MS);
                match self.find_run(conversation, op) {
                    Some(run) => {
                        run.queue.clear();
                        run.next_step = run.steps.len();
                        run.generation = generation;
                        run.block = Block::Settling;
                        run.due = due;
                    }
                    None => self.runs.push(Run {
                        conversation,
                        generation,
                        op,
                        steps: Vec::new(),
                        next_step: 0,
                        queue: VecDeque::new(),
                        due,
                        block: Block::Settling,
                        steered: false,
                        pending_steer: false,
                        subst: Subst::default(),
                        rng: Rng::from_parts(&[self.seed, op.0]),
                        ended: false,
                    }),
                }
            }
            // The demo creates conversations and chooses models locally in the core, so these
            // are never sent to it.
            BackendRequest::CreateConversation { .. }
            | BackendRequest::SetModel { .. }
            | BackendRequest::SetThinkingLevel { .. } => {}
            BackendRequest::CheckStatus {
                conversation,
                generation,
                op,
                ..
            } => {
                let known = self.seen_ops.contains(&(conversation, op));
                if let Some(run) = self.find_run(conversation, op)
                    && run.block == Block::Status
                {
                    run.block = Block::None;
                }
                self.ctx.emit(
                    conversation,
                    generation,
                    Some(op),
                    EventKind::StatusResolved { accepted: known },
                );
            }
        }
    }
}

/// Collapse whitespace, neutralize backticks and cap the length, for echoing user text as
/// inert, single-line Markdown.
fn single_line(text: &str, max_chars: usize) -> String {
    let collapsed = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('`', "'");
    if collapsed.chars().count() <= max_chars {
        collapsed
    } else {
        let mut s: String = collapsed.chars().take(max_chars).collect();
        s.push('\u{2026}');
        s
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn single_line_is_inert_and_bounded() {
        assert_eq!(single_line("a\n\nb   `rm -rf`", 50), "a b 'rm -rf'");
        assert_eq!(single_line(&"x".repeat(300), 10).chars().count(), 11);
    }
}
