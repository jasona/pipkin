use crate::model::Connection;
use crate::protocol::BackendRequest;
use crate::state::Bootstrap;

/// Connection and catalog updates, separate from per-conversation `BackendEvent`s. These are
/// authoritative state and must never be dropped, so the sink behind them is unbounded.
#[derive(Clone, Debug, PartialEq)]
pub enum LifecycleEvent {
    Connection(Connection),
    /// Projects, models and conversation summaries known to the backend.
    Catalog(Bootstrap),
}

pub type LifecycleSink = Box<dyn Fn(LifecycleEvent) + Send + Sync>;

/// The narrow backend port. Initialization is asynchronous: `start` returns immediately and the
/// backend reports progress through the sink. Per-conversation events flow back through a
/// channel supplied at construction, never through return values, so slow or lost responses are
/// representable.
pub trait Backend: Send + Sync {
    /// Begin connecting. Must not block; report `Connecting`/`Ready`/`Offline`/... and the
    /// catalog through `sink`, from any thread.
    fn start(&self, sink: LifecycleSink);
    /// Fire a request. Responses arrive as `BackendEvent`s on the adapter's channel.
    fn request(&self, request: BackendRequest);
    /// Orderly stop, called once at application exit. Must not block indefinitely.
    fn shutdown(&self) {}
}
