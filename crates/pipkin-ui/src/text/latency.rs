//! Input-to-frame instrumentation: time from the platform input-handler call to the end of
//! the composer's paint for the frame that shows the edit. This is CPU frame submission, not
//! display presentation, and excludes the compositor's own input delivery time.

use std::sync::Mutex;
use std::time::{Duration, Instant};

const CAP: usize = 2048;

struct State {
    pending: Option<Instant>,
    samples: Vec<Duration>,
}

static STATE: Mutex<State> = Mutex::new(State {
    pending: None,
    samples: Vec::new(),
});

/// Called when text input reaches the editor.
pub fn mark() {
    if let Ok(mut s) = STATE.lock() {
        s.pending.get_or_insert_with(Instant::now);
    }
}

/// Called at the end of the composer's paint.
pub fn painted() {
    if let Ok(mut s) = STATE.lock()
        && let Some(t) = s.pending.take()
    {
        if s.samples.len() >= CAP {
            s.samples.remove(0);
        }
        s.samples.push(t.elapsed());
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LatencyStats {
    pub count: usize,
    pub p50: Duration,
    pub p95: Duration,
    pub max: Duration,
}

pub fn stats() -> Option<LatencyStats> {
    let s = STATE.lock().ok()?;
    if s.samples.is_empty() {
        return None;
    }
    let mut v = s.samples.clone();
    v.sort();
    let at = |q: f64| v[(((v.len() - 1) as f64) * q).round() as usize];
    Some(LatencyStats {
        count: v.len(),
        p50: at(0.5),
        p95: at(0.95),
        max: *v.last().unwrap(),
    })
}
