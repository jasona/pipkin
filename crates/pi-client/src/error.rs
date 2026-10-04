use std::fmt;

/// A protocol, codec or transport failure. Every variant is fatal for the connection or
/// subscription it was raised on; the client never retries or replays on its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Malformed or out-of-subset CBOR, or a limit exceeded.
    Cbor(String),
    /// Invalid length-prefixed framing.
    Frame(String),
    /// A well-formed message that violates the envelope or Chord grammar.
    Validation(String),
    /// An operation that cannot be applied to the replica it targets.
    Delta(String),
    /// A replicated state update skipped a sequence number.
    SequenceGap { expected: u64, got: u64 },
    /// The remote side returned an error response.
    Server { code: String, message: String },
    /// The connection is not (or no longer) usable.
    Disconnected(String),
    /// A request was cancelled locally before it settled.
    Cancelled,
    /// A wait elapsed. The request itself is still outstanding until it is cancelled.
    Timeout,
    /// The endpoint failed an identity or permission check.
    Untrusted(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Cbor(m) => write!(f, "CBOR error: {m}"),
            Error::Frame(m) => write!(f, "framing error: {m}"),
            Error::Validation(m) => write!(f, "protocol validation error: {m}"),
            Error::Delta(m) => write!(f, "state operation error: {m}"),
            Error::SequenceGap { expected, got } => {
                write!(
                    f,
                    "state update sequence gap: expected {expected}, got {got}"
                )
            }
            Error::Server { code, message } => write!(f, "server error {code}: {message}"),
            Error::Disconnected(m) => write!(f, "disconnected: {m}"),
            Error::Cancelled => f.write_str("request cancelled"),
            Error::Timeout => f.write_str("timed out"),
            Error::Untrusted(m) => write!(f, "untrusted endpoint: {m}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn validation(message: impl Into<String>) -> Error {
    Error::Validation(message.into())
}
