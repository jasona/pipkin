use std::fmt;

macro_rules! id {
    ($($name:ident),*) => {$(
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub u64);
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    )*};
}

id!(ProjectId, ConversationId, ItemId, OperationId, QueueId);

/// Items created locally (user prompts, notices) start here so they never collide with
/// backend-provided history ids.
pub const LOCAL_ITEM_BASE: u64 = 1 << 40;

/// Client-generated key for one mutation (prompt submission). Unique across restarts, so a
/// backend can deduplicate and a recovered journal entry can be looked up by it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RequestId(pub String);

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
