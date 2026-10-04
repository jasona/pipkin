use crate::protocol::BackendRequest;
use crate::state::Bootstrap;

/// The narrow backend port. A demo implementation exists today; a Pi adapter will implement
/// the same trait later. Events flow back through a channel supplied at construction, never
/// through return values, so slow or lost responses are representable.
pub trait Backend: Send + Sync {
    /// Seed projects, models and conversation summaries.
    fn bootstrap(&self) -> Bootstrap;
    /// Fire a request. Responses arrive as `BackendEvent`s on the adapter's channel.
    fn request(&self, request: BackendRequest);
}
