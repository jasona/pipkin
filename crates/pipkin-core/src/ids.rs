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

/// A stable 52-bit id for a string key (FNV-1a), never zero. 52 bits survive JSON numbers and
/// make a collision between two live keys astronomically unlikely; callers that must be certain
/// still probe (see the Pi adapter's session ids).
pub fn stable_id(key: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in key.bytes() {
        h ^= byte as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    (h & ((1 << 52) - 1)).max(1)
}

/// The id of a project, derived from its directory so the same folder is always the same project.
pub fn project_id_for_path(path: &str) -> ProjectId {
    ProjectId(stable_id(path))
}

/// A short display name for a project directory: its last component, or the whole path.
pub fn project_name_for_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    trimmed
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or(path)
        .to_owned()
}
